use std::{
    collections::HashMap,
    fmt::Write as _,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow, bail};
use ed25519_dalek::{Signature, VerifyingKey};
use p5099_connector::hash_file;
use p5099_core::{
    account_protocol::{from_hex, to_hex},
    client_manifest::{
        CLIENT_MANIFEST_FILE_NAME, CLIENT_MANIFEST_SIGNATURE_FILE_NAME, ClientManifest,
        ManifestEntry, encode_key, is_safe_relative_path,
    },
};
use sha2::{Digest, Sha256};

const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 1024;
const CACHE_DIRECTORY: &str = ".p5099-update";
const REVISIONS_FILE: &str = "revisions.txt";
const CACHE_FILE: &str = "cache.txt";
const PARTIAL_SUFFIX: &str = ".p5099-download";
const ATTEMPTS: u32 = 6;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

fn retry_delay(attempt: u32) -> Duration {
    if cfg!(test) {
        return Duration::from_millis(10);
    }
    Duration::from_secs(1 << attempt.min(5)).min(MAX_RETRY_DELAY)
}
const MIN_BYTES_PER_SECOND: u64 = 32 * 1024;

#[derive(Debug, Clone)]
pub enum SyncProgress {
    FetchingManifest,
    Checking {
        done: usize,
        total: usize,
    },
    Downloading {
        file: String,
        done_bytes: u64,
        total_bytes: u64,
        files_left: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    pub downloaded_files: usize,
    pub downloaded_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedHash {
    size: u64,
    modified: u128,
    sha256: [u8; 32],
}

struct HashCache {
    path: PathBuf,
    entries: HashMap<String, CachedHash>,
}

impl HashCache {
    fn load(game_directory: &Path) -> Self {
        let path = game_directory.join(CACHE_DIRECTORY).join(CACHE_FILE);
        let entries = fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(4, '\t');
                let size = fields.next()?.parse().ok()?;
                let modified = fields.next()?.parse().ok()?;
                let sha256 = from_hex(fields.next()?).ok()?;
                let path = fields.next()?.to_owned();
                Some((
                    path,
                    CachedHash {
                        size,
                        modified,
                        sha256,
                    },
                ))
            })
            .collect();
        Self { path, entries }
    }

    fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut text = String::new();
        for (path, cached) in &self.entries {
            let _ = writeln!(
                text,
                "{}\t{}\t{}\t{path}",
                cached.size,
                cached.modified,
                to_hex(&cached.sha256)
            );
        }
        let temporary = self.path.with_extension("tmp");
        fs::write(&temporary, text)?;
        fs::rename(temporary, &self.path)
    }
}

fn modified_nanos(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_nanos())
}

fn is_current(game_directory: &Path, entry: &ManifestEntry, cache: &mut HashCache) -> bool {
    let local = game_directory.join(&entry.path);
    let Ok(metadata) = fs::metadata(&local) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    if entry.keep {
        return true;
    }
    if metadata.len() != entry.size {
        return false;
    }
    let modified = modified_nanos(&metadata);
    if let Some(cached) = cache.entries.get(&entry.path)
        && cached.size == entry.size
        && cached.modified == modified
    {
        return cached.sha256 == entry.sha256;
    }
    let Ok((size, sha256)) = hash_file(&local) else {
        return false;
    };
    cache.entries.insert(
        entry.path.clone(),
        CachedHash {
            size,
            modified,
            sha256,
        },
    );
    size == entry.size && sha256 == entry.sha256
}

pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into()
}

pub(crate) fn url(base_url: &str, key: &str) -> String {
    format!("{}/{}", base_url.trim_end_matches('/'), encode_key(key))
}

pub const PLATFORM_FOLDER: &str = if cfg!(windows) { "windows" } else { "macos" };

fn fetch_text(
    agent: &ureq::Agent,
    base_url: &str,
    key: &str,
    limit: u64,
    missing: &str,
) -> Result<String> {
    agent
        .get(&url(base_url, key))
        .call()
        .map_err(|error| match error {
            ureq::Error::StatusCode(404) => anyhow!("{missing} ({key} 없음)"),
            error => anyhow::Error::new(error).context("클라이언트 파일 목록을 받지 못했습니다"),
        })?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_string()
        .context("클라이언트 파일 목록을 읽지 못했습니다")
}

fn fetch_manifest(
    agent: &ureq::Agent,
    base_url: &str,
    folder: &str,
    key: &VerifyingKey,
) -> Result<ClientManifest> {
    let folder = folder.trim_matches('/');
    let prefix = if folder.is_empty() {
        String::new()
    } else {
        format!("{folder}/")
    };
    let text = fetch_text(
        agent,
        base_url,
        &format!("{prefix}{CLIENT_MANIFEST_FILE_NAME}"),
        MAX_MANIFEST_BYTES,
        "서버에 아직 게임 파일이 준비되지 않았습니다",
    )?;
    let signature = fetch_text(
        agent,
        base_url,
        &format!("{prefix}{CLIENT_MANIFEST_SIGNATURE_FILE_NAME}"),
        MAX_SIGNATURE_BYTES,
        "클라이언트 파일 목록에 서명이 없습니다",
    )?;
    let signature = from_hex::<64>(signature.trim())
        .map_err(|_| anyhow!("클라이언트 파일 목록의 서명이 올바르지 않습니다"))?;
    key.verify_strict(text.as_bytes(), &Signature::from_bytes(&signature))
        .map_err(|_| anyhow!("클라이언트 파일 목록의 서명을 확인하지 못했습니다"))?;
    let mut manifest =
        ClientManifest::parse(&text).context("클라이언트 파일 목록이 올바르지 않습니다")?;
    if manifest.root.is_empty() {
        manifest.root = prefix;
    }
    Ok(manifest)
}

pub(crate) struct RemoteFile<'a> {
    pub key: &'a str,
    pub label: &'a str,
    pub size: u64,
    pub sha256: [u8; 32],
}

fn download(
    agent: &ureq::Agent,
    base_url: &str,
    file: &RemoteFile,
    destination: &Path,
    cancel: &AtomicBool,
    mut advance: impl FnMut(u64),
) -> Result<()> {
    let budget = Duration::from_secs(120 + file.size / MIN_BYTES_PER_SECOND);
    let response = agent
        .get(&url(base_url, file.key))
        .config()
        .timeout_recv_body(Some(budget))
        .build()
        .call()
        .with_context(|| format!("{}을(를) 받지 못했습니다", file.label))?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = PathBuf::from(format!("{}{PARTIAL_SUFFIX}", destination.display()));
    let result = (|| {
        let mut reader = response.into_body().into_reader();
        let mut output = File::create(&partial)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 256 * 1024];
        let mut received = 0_u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                bail!("취소했습니다");
            }
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            received += read as u64;
            if received > file.size {
                bail!("{}이(가) 목록보다 큽니다", file.label);
            }
            hasher.update(&buffer[..read]);
            output.write_all(&buffer[..read])?;
            advance(read as u64);
        }
        output.sync_all()?;
        drop(output);
        if received != file.size || <[u8; 32]>::from(hasher.finalize()) != file.sha256 {
            bail!("{}이(가) 손상되었습니다", file.label);
        }
        fs::rename(&partial, destination).with_context(|| {
            format!(
                "{}을(를) 교체하지 못했습니다 (게임이 실행 중인가요?)",
                file.label
            )
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

pub(crate) fn fetch_verified(
    agent: &ureq::Agent,
    base_url: &str,
    file: &RemoteFile,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<()> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut done = 0_u64;
        progress(done);
        let result = download(agent, base_url, file, destination, cancel, |read| {
            done += read;
            progress(done);
        });
        match result {
            Ok(()) => return Ok(()),
            Err(error) if attempt >= ATTEMPTS || cancel.load(Ordering::Relaxed) => {
                return Err(error);
            }
            Err(_) => {
                let delay = retry_delay(attempt);
                let mut waited = Duration::ZERO;
                while waited < delay && !cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(250));
                    waited += Duration::from_millis(250);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateCheck {
    pub files: usize,
    pub bytes: u64,
    pub installed: bool,
}

struct Plan {
    agent: ureq::Agent,
    manifest: ClientManifest,
    needed: Vec<usize>,
    cache: HashCache,
}

fn revisions_path(game_directory: &Path) -> PathBuf {
    game_directory.join(CACHE_DIRECTORY).join(REVISIONS_FILE)
}

fn load_revisions(game_directory: &Path) -> HashMap<String, u64> {
    fs::read_to_string(revisions_path(game_directory))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (revision, source) = line.split_once('\t')?;
            Some((source.to_owned(), revision.parse().ok()?))
        })
        .collect()
}

fn save_revisions(game_directory: &Path, revisions: &HashMap<String, u64>) -> io::Result<()> {
    let path = revisions_path(game_directory);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut text = String::new();
    for (source, revision) in revisions {
        let _ = writeln!(text, "{revision}\t{source}");
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, text)?;
    fs::rename(temporary, path)
}

fn plan(
    game_directory: &Path,
    base_url: &str,
    folder: &str,
    key: &VerifyingKey,
    cancel: &AtomicBool,
    progress: &dyn Fn(SyncProgress),
) -> Result<Plan> {
    let agent = agent();
    progress(SyncProgress::FetchingManifest);
    let manifest = fetch_manifest(&agent, base_url, folder, key)?;
    let source = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        folder.trim_matches('/')
    );
    let mut revisions = load_revisions(game_directory);
    if manifest.revision < revisions.get(&source).copied().unwrap_or(0) {
        bail!(
            "서버가 전에 받은 것보다 오래된 클라이언트 파일 목록을 보냈습니다. 잠시 후 다시 시도하세요"
        );
    }
    if game_directory.is_dir() && revisions.get(&source) != Some(&manifest.revision) {
        revisions.insert(source, manifest.revision);
        let _ = save_revisions(game_directory, &revisions);
    }
    if let Some(entry) = manifest
        .entries
        .iter()
        .find(|entry| !is_safe_relative_path(&entry.path))
    {
        return Err(anyhow!("안전하지 않은 경로: {}", entry.path));
    }
    let mut cache = HashCache::load(game_directory);
    let total = manifest.entries.len();
    let mut needed = Vec::new();
    for (index, entry) in manifest.entries.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            bail!("취소했습니다");
        }
        progress(SyncProgress::Checking { done: index, total });
        if !is_current(game_directory, entry, &mut cache) {
            needed.push(index);
        }
    }
    if game_directory.is_dir() {
        let _ = cache.save();
    }
    Ok(Plan {
        agent,
        manifest,
        needed,
        cache,
    })
}

pub fn check_client(
    game_directory: &Path,
    base_url: &str,
    folder: &str,
    key: &VerifyingKey,
    cancel: &AtomicBool,
    progress: &dyn Fn(SyncProgress),
) -> Result<UpdateCheck> {
    let plan = plan(game_directory, base_url, folder, key, cancel, progress)?;
    Ok(UpdateCheck {
        files: plan.needed.len(),
        bytes: plan
            .needed
            .iter()
            .map(|&index| plan.manifest.entries[index].size)
            .sum(),
        installed: game_directory.join("KartRider.exe").is_file(),
    })
}

pub fn sync_client(
    game_directory: &Path,
    base_url: &str,
    folder: &str,
    key: &VerifyingKey,
    cancel: &AtomicBool,
    progress: &dyn Fn(SyncProgress),
) -> Result<SyncReport> {
    fs::create_dir_all(game_directory).with_context(|| {
        format!(
            "게임 폴더를 만들지 못했습니다: {}",
            game_directory.display()
        )
    })?;
    let Plan {
        agent,
        manifest,
        needed,
        mut cache,
    } = plan(game_directory, base_url, folder, key, cancel, progress)?;
    let needed: Vec<&ManifestEntry> = needed
        .into_iter()
        .map(|index| &manifest.entries[index])
        .collect();

    let total_bytes = needed.iter().map(|entry| entry.size).sum();
    let mut done_bytes = 0_u64;
    for (index, entry) in needed.iter().enumerate() {
        let destination = game_directory.join(&entry.path);
        let key = manifest.key(entry);
        let file = RemoteFile {
            key: &key,
            label: &entry.path,
            size: entry.size,
            sha256: entry.sha256,
        };
        let start = done_bytes;
        let result = fetch_verified(&agent, base_url, &file, &destination, cancel, &|done| {
            progress(SyncProgress::Downloading {
                file: entry.path.clone(),
                done_bytes: start + done,
                total_bytes,
                files_left: needed.len() - index,
            });
        });
        if let Err(error) = result {
            let _ = cache.save();
            return Err(error);
        }
        done_bytes += entry.size;
        if let Ok(metadata) = fs::metadata(&destination) {
            cache.entries.insert(
                entry.path.clone(),
                CachedHash {
                    size: entry.size,
                    modified: modified_nanos(&metadata),
                    sha256: entry.sha256,
                },
            );
        }
    }
    cache
        .save()
        .context("업데이트 기록을 저장하지 못했습니다")?;
    Ok(SyncReport {
        downloaded_files: needed.len(),
        downloaded_bytes: total_bytes,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        sync::Arc,
        thread,
    };

    use ed25519_dalek::{Signer as _, SigningKey};
    use p5099_connector::build_client_manifest;
    use tempfile::tempdir;

    use super::*;

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn publish(folder: &Path, manifest: &ClientManifest) {
        let text = manifest.to_text();
        let signature = signing_key().sign(text.as_bytes());
        fs::write(folder.join(CLIENT_MANIFEST_FILE_NAME), &text).unwrap();
        fs::write(
            folder.join(CLIENT_MANIFEST_SIGNATURE_FILE_NAME),
            to_hex(&signature.to_bytes()),
        )
        .unwrap();
    }

    fn serve(root: PathBuf) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let path = line
                    .split(' ')
                    .nth(1)
                    .unwrap()
                    .trim_start_matches('/')
                    .to_owned();
                log.lock().unwrap().push(path.clone());
                match fs::read(root.join(&path)) {
                    Ok(body) => {
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(&body).unwrap();
                    }
                    Err(_) => {
                        stream
                            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .unwrap();
                    }
                }
            }
        });
        (format!("http://{address}"), requests)
    }

    #[test]
    fn installs_then_only_replaces_changed_files() {
        let storage = tempdir().unwrap();
        let client = storage.path().join("windows");
        fs::create_dir_all(client.join("Data")).unwrap();
        fs::write(client.join("KartRider.exe"), b"exe-v1").unwrap();
        fs::write(client.join("KartRider.pin"), b"pin").unwrap();
        fs::write(client.join("Data/kart.rho"), vec![7_u8; 300_000]).unwrap();
        let manifest = build_client_manifest(&client, "", |_| {}).unwrap();
        publish(&client, &manifest);
        let (base_url, requests) = serve(storage.path().to_owned());

        let key = signing_key().verifying_key();
        let game = tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let check = check_client(
            &game.path().join("new"),
            &base_url,
            "windows",
            &key,
            &cancel,
            &|_| {},
        )
        .unwrap();
        assert_eq!(
            check,
            UpdateCheck {
                files: 3,
                bytes: 300_009,
                installed: false
            }
        );
        let report =
            sync_client(game.path(), &base_url, "windows", &key, &cancel, &|_| {}).unwrap();
        assert_eq!(report.downloaded_files, 3);
        assert_eq!(
            fs::read(game.path().join("KartRider.exe")).unwrap(),
            b"exe-v1"
        );
        assert_eq!(
            fs::read(game.path().join("Data/kart.rho")).unwrap().len(),
            300_000
        );

        fs::write(game.path().join("KartRider.pin"), b"patched").unwrap();
        fs::write(game.path().join("KartRider.exe"), b"exe-xx").unwrap();
        requests.lock().unwrap().clear();
        let report =
            sync_client(game.path(), &base_url, "windows", &key, &cancel, &|_| {}).unwrap();
        assert_eq!(report.downloaded_files, 1);
        assert_eq!(
            *requests.lock().unwrap(),
            [
                "windows/p5099-manifest.txt",
                "windows/p5099-manifest.sig",
                "windows/KartRider.exe"
            ]
        );
        assert_eq!(
            fs::read(game.path().join("KartRider.exe")).unwrap(),
            b"exe-v1"
        );
        assert_eq!(
            fs::read(game.path().join("KartRider.pin")).unwrap(),
            b"patched"
        );

        let report =
            sync_client(game.path(), &base_url, "windows", &key, &cancel, &|_| {}).unwrap();
        assert_eq!(report.downloaded_files, 0);
    }

    #[test]
    fn an_older_file_list_is_refused() {
        let storage = tempdir().unwrap();
        fs::write(storage.path().join("a.dll"), b"good").unwrap();
        let mut manifest = build_client_manifest(storage.path(), "", |_| {}).unwrap();
        manifest.revision = 20;
        publish(storage.path(), &manifest);
        let (base_url, _) = serve(storage.path().to_owned());
        let game = tempdir().unwrap();
        let key = signing_key().verifying_key();
        let sync = || {
            sync_client(
                game.path(),
                &base_url,
                "",
                &key,
                &AtomicBool::new(false),
                &|_| {},
            )
        };
        sync().unwrap();
        manifest.revision = 10;
        publish(storage.path(), &manifest);
        assert!(format!("{:#}", sync().unwrap_err()).contains("오래된"));
        manifest.revision = 30;
        publish(storage.path(), &manifest);
        sync().unwrap();
    }

    #[test]
    fn a_corrupt_download_is_not_installed() {
        let storage = tempdir().unwrap();
        fs::write(storage.path().join("a.dll"), b"good").unwrap();
        let mut manifest = build_client_manifest(storage.path(), "", |_| {}).unwrap();
        manifest.entries[0].sha256 = [0; 32];
        publish(storage.path(), &manifest);
        let (base_url, _) = serve(storage.path().to_owned());

        let game = tempdir().unwrap();
        let key = signing_key().verifying_key();
        let error = sync_client(
            game.path(),
            &base_url,
            "",
            &key,
            &AtomicBool::new(false),
            &|_| {},
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("손상"));
        assert!(!game.path().join("a.dll").exists());
        assert!(!game.path().join(format!("a.dll{PARTIAL_SUFFIX}")).exists());
    }

    #[test]
    fn accepts_a_manifest_signed_by_the_script() {
        const PUBLIC_KEY: &str = "780faee4d4a7ad0da696bb14ee684610f46c8df9326ecb1d086cca48f58bd7f5";
        const SIGNATURE: &str = "041d0304a879f9610b82a0f142e0c9c77937948ebc1116da2978012aa44b15fe1ac34de85a2917bf250811ba3376c6d9f625b10d902a2e73721ff7469d1c5d0d";
        const MANIFEST: &str = "p5099-manifest 1\nroot \n2bb8bf7259c4ea0edaef5b8552247e06dde993959d3d33e4a2b32f32e1144c0a\t5\t-\tData/a b.rho\n9095bdb859308b62acf04036ffd4adfe366d7f737d276eb6c46ae434f3816c9b\t3\t-\tKartRider.exe\n";
        let storage = tempdir().unwrap();
        fs::write(storage.path().join(CLIENT_MANIFEST_FILE_NAME), MANIFEST).unwrap();
        fs::write(
            storage.path().join(CLIENT_MANIFEST_SIGNATURE_FILE_NAME),
            format!("{SIGNATURE}\n"),
        )
        .unwrap();
        let (base_url, _) = serve(storage.path().to_owned());
        let key = VerifyingKey::from_bytes(&from_hex(PUBLIC_KEY).unwrap()).unwrap();
        let manifest = fetch_manifest(&agent(), &base_url, "", &key).unwrap();
        assert_eq!(manifest.entries.len(), 2);
    }

    #[test]
    fn an_unsigned_or_tampered_manifest_installs_nothing() {
        let storage = tempdir().unwrap();
        fs::write(storage.path().join("a.dll"), b"good").unwrap();
        let manifest = build_client_manifest(storage.path(), "", |_| {}).unwrap();
        let (base_url, _) = serve(storage.path().to_owned());
        let game = tempdir().unwrap();
        let key = signing_key().verifying_key();
        let sync = || {
            let error = sync_client(
                game.path(),
                &base_url,
                "",
                &key,
                &AtomicBool::new(false),
                &|_| {},
            )
            .unwrap_err();
            format!("{error:#}")
        };

        fs::write(
            storage.path().join(CLIENT_MANIFEST_FILE_NAME),
            manifest.to_text(),
        )
        .unwrap();
        assert!(sync().contains("서명이 없습니다"), "{}", sync());

        publish(storage.path(), &manifest);
        let mut forged = manifest.clone();
        forged.entries[0].sha256 = [0xEE; 32];
        fs::write(
            storage.path().join(CLIENT_MANIFEST_FILE_NAME),
            forged.to_text(),
        )
        .unwrap();
        assert!(sync().contains("서명을 확인하지 못했습니다"), "{}", sync());
        assert!(!game.path().join("a.dll").exists());
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore = "needs network, the release key and P5099_SERVER_HOST"]
    fn the_server_offers_a_signed_manifest() {
        let key = crate::self_update::release_key().unwrap();
        let server = crate::BUILT_IN_SERVER.expect("build with P5099_SERVER_HOST");
        super::fetch_manifest(
            &super::agent(),
            &crate::server::ServerAddress::parse(server)
                .unwrap()
                .download_url(),
            super::PLATFORM_FOLDER,
            &key,
        )
        .unwrap();
    }
}
