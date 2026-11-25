use std::{
    ffi::OsString,
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
};

use anyhow::{Context as _, Result, anyhow, bail};
use ed25519_dalek::{Signature, VerifyingKey};
use p5099_core::{account_protocol::from_hex, client_manifest::is_safe_relative_path};
use serde::Deserialize;

use crate::updater::{self, RemoteFile};

/// Signed release index uploaded to every GitHub release. GitHub redirects
/// `latest/download/<asset>` to the newest non-prerelease release.
pub const RELEASE_KEY: &str = "latest/download/latest.txt";
/// `owner/repo` whose GitHub releases carry launcher updates. Without it (a
/// local build) the launcher never offers an update.
const RELEASE_REPO: Option<&str> = match option_env!("P5099_RELEASE_REPO") {
    Some(repo) if !repo.is_empty() => Some(repo),
    _ => None,
};
const RELEASE_PUBLIC_KEY: Option<&str> = match option_env!("P5099_RELEASE_PUBLIC_KEY") {
    Some(key) if !key.is_empty() => Some(key),
    _ => None,
};
const MAX_RELEASE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl Version {
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
        let mut next = || -> Result<u64> {
            parts
                .next()
                .and_then(|part| part.parse().ok())
                .ok_or_else(|| anyhow!("버전 형식이 올바르지 않습니다: {text}"))
        };
        let version = Self(next()?, next()?, next()?);
        if parts.next().is_some() {
            bail!("버전 형식이 올바르지 않습니다: {text}");
        }
        Ok(version)
    }

    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("the package version is major.minor.patch")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    version: String,
    #[serde(default)]
    min_version: Option<String>,
    #[serde(default)]
    windows: Option<ReleaseAsset>,
    #[serde(default)]
    macos: Option<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    key: String,
    size: u64,
    sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: Version,
    pub required: bool,
    pub asset: Option<Asset>,
}

pub(crate) fn release_key() -> Result<VerifyingKey> {
    RELEASE_PUBLIC_KEY
        .and_then(|key| from_hex::<32>(key.trim()).ok())
        .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
        .ok_or_else(|| anyhow!("이 실행기에는 배포 서명 키가 없습니다"))
}

fn releases_url(repo: &str) -> String {
    format!("https://github.com/{}/releases", repo.trim_matches('/'))
}

fn releases_base() -> Result<String> {
    RELEASE_REPO
        .map(releases_url)
        .ok_or_else(|| anyhow!("이 실행기에는 업데이트 저장소가 지정되지 않았습니다"))
}

pub fn check() -> Result<Option<Update>> {
    let (Ok(key), Ok(base_url)) = (release_key(), releases_base()) else {
        return Ok(None);
    };
    let text = updater::agent()
        .get(&updater::url(&base_url, RELEASE_KEY))
        .call()
        .context("실행기 버전 정보를 받지 못했습니다")?
        .body_mut()
        .with_config()
        .limit(MAX_RELEASE_BYTES)
        .read_to_string()
        .context("실행기 버전 정보를 읽지 못했습니다")?;
    parse_release(&text, &key, Version::current())
}

fn parse_release(text: &str, key: &VerifyingKey, current: Version) -> Result<Option<Update>> {
    let (signature, body) = text
        .split_once('\n')
        .ok_or_else(|| anyhow!("실행기 버전 정보가 올바르지 않습니다"))?;
    let signature = from_hex::<64>(signature.trim_end_matches('\r'))
        .map_err(|_| anyhow!("실행기 버전 정보의 서명이 올바르지 않습니다"))?;
    key.verify_strict(body.as_bytes(), &Signature::from_bytes(&signature))
        .map_err(|_| anyhow!("실행기 버전 정보의 서명을 확인하지 못했습니다"))?;
    let release: Release =
        serde_json::from_str(body).context("실행기 버전 정보가 올바르지 않습니다")?;
    let version = Version::parse(&release.version)?;
    if version <= current {
        return Ok(None);
    }
    let min_version = release
        .min_version
        .as_deref()
        .map(Version::parse)
        .transpose()?
        .unwrap_or_default();
    let asset = if cfg!(windows) {
        release.windows
    } else if cfg!(target_os = "macos") {
        release.macos
    } else {
        None
    };
    let asset = asset
        .map(|asset| -> Result<Asset> {
            if !is_safe_relative_path(&asset.path) {
                bail!("안전하지 않은 경로: {}", asset.path);
            }
            Ok(Asset {
                sha256: from_hex(&asset.sha256)
                    .map_err(|_| anyhow!("실행기 SHA-256이 올바르지 않습니다"))?,
                key: asset.path,
                size: asset.size,
            })
        })
        .transpose()?;
    Ok(Some(Update {
        version,
        required: current < min_version,
        asset,
    }))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut text = OsString::from(path.as_os_str());
    text.push(suffix);
    PathBuf::from(text)
}

fn swap_in(current: &Path, replacement: &Path, backup: &Path) -> Result<()> {
    if backup.is_dir() {
        let _ = fs::remove_dir_all(backup);
    } else {
        let _ = fs::remove_file(backup);
    }
    fs::rename(current, backup).context("실행기를 교체하지 못했습니다")?;
    if let Err(error) = fs::rename(replacement, current) {
        let _ = fs::rename(backup, current);
        return Err(error).context("실행기를 교체하지 못했습니다");
    }
    Ok(())
}

fn current_app() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("실행기 위치를 알 수 없습니다")?;
    executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(Path::to_owned)
        .ok_or_else(|| anyhow!("실행기가 .app 안에 있지 않습니다"))
}

fn macos_staging(app: &Path) -> PathBuf {
    app.parent().unwrap_or(app).join(".p5099-update")
}

fn old_macos_staging(app: &Path) -> PathBuf {
    app.parent().unwrap_or(app).join(".kartrider-p5099-update")
}

pub fn clean_up() {
    if cfg!(windows) {
        if let Ok(executable) = std::env::current_exe() {
            for suffix in [".old", ".new", ".new.p5099-download"] {
                let _ = fs::remove_file(with_suffix(&executable, suffix));
            }
        }
    } else if cfg!(target_os = "macos")
        && let Ok(app) = current_app()
    {
        let _ = fs::remove_dir_all(macos_staging(&app));
        let _ = fs::remove_dir_all(old_macos_staging(&app));
    }
}

pub fn install(
    update: &Update,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<PathBuf> {
    let asset = update
        .asset
        .as_ref()
        .ok_or_else(|| anyhow!("이 플랫폼에서는 실행기를 자동으로 업데이트할 수 없습니다"))?;
    let file = RemoteFile {
        key: &asset.key,
        label: "실행기",
        size: asset.size,
        sha256: asset.sha256,
    };
    let report = |done| progress(done, asset.size);
    let base_url = releases_base()?;
    let base_url = base_url.as_str();
    let agent = updater::agent();
    if cfg!(windows) {
        let executable = std::env::current_exe().context("실행기 위치를 알 수 없습니다")?;
        let replacement = with_suffix(&executable, ".new");
        updater::fetch_verified(&agent, base_url, &file, &replacement, cancel, &report)?;
        swap_in(&executable, &replacement, &with_suffix(&executable, ".old"))?;
        Ok(executable)
    } else if cfg!(target_os = "macos") {
        let app = current_app()?;
        let staging = macos_staging(&app);
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging).with_context(|| {
            format!(
                "{}에 쓸 수 없습니다. 새 DMG를 받아 설치하세요",
                staging.parent().unwrap_or(&staging).display()
            )
        })?;
        let archive = staging.join("launcher.zip");
        updater::fetch_verified(&agent, base_url, &file, &archive, cancel, &report)?;
        let unpacked = staging.join("new");
        let status = Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(&archive)
            .arg(&unpacked)
            .status()
            .context("실행기 압축을 풀지 못했습니다")?;
        if !status.success() {
            bail!("실행기 압축을 풀지 못했습니다 ({status})");
        }
        let replacement = fs::read_dir(&unpacked)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
            .ok_or_else(|| anyhow!("받은 파일에 실행기 앱이 없습니다"))?;
        swap_in(&app, &replacement, &staging.join("old.app"))?;
        Ok(app)
    } else {
        bail!("이 플랫폼에서는 실행기를 자동으로 업데이트할 수 없습니다")
    }
}

pub fn relaunch(target: &Path) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-n").arg(target);
        command
    } else {
        let mut command = Command::new(target);
        if let Some(directory) = target.parent() {
            command.current_dir(directory);
        }
        command
    };
    command
        .spawn()
        .map(drop)
        .with_context(|| format!("새 실행기를 시작하지 못했습니다: {}", target.display()))
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer as _, SigningKey};
    use p5099_core::account_protocol::to_hex;
    use tempfile::tempdir;

    use super::*;

    const CURRENT: Version = Version(1, 2, 0);

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn signed(body: &str) -> String {
        let signature = signing_key().sign(body.as_bytes());
        format!("{}\n{body}", to_hex(&signature.to_bytes()))
    }

    fn release(version: &str, min_version: &str) -> String {
        let asset = format!(
            r#"{{"path": "download/v{version}/p5099.exe", "size": 3, "sha256": "{}"}}"#,
            "ab".repeat(32)
        );
        format!(
            r#"{{"version": "{version}", "min_version": "{min_version}", "windows": {asset}, "macos": {asset}}}"#
        )
    }

    fn parse(text: &str) -> Result<Option<Update>> {
        parse_release(text, &signing_key().verifying_key(), CURRENT)
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(Version::parse("1.10.0").unwrap() > Version::parse("v1.9.3").unwrap());
        assert!(Version::parse("1.2").is_err());
        assert!(Version::parse("1.2.3.4").is_err());
        assert!(Version::parse("1.2.3-rc1").is_err());
        Version::current();
    }

    #[test]
    fn a_newer_signed_release_is_offered() {
        let update = parse(&signed(&release("1.3.0", "1.0.0"))).unwrap().unwrap();
        assert_eq!(update.version, Version(1, 3, 0));
        assert!(!update.required);
        if cfg!(any(windows, target_os = "macos")) {
            let asset = update.asset.unwrap();
            assert_eq!(asset.key, "download/v1.3.0/p5099.exe");
            assert_eq!(asset.sha256, [0xAB; 32]);
        } else {
            assert_eq!(update.asset, None);
        }
    }

    #[test]
    fn min_version_makes_the_update_required() {
        let update = parse(&signed(&release("1.3.0", "1.3.0"))).unwrap().unwrap();
        assert!(update.required);
    }

    #[test]
    fn the_same_or_an_older_release_changes_nothing() {
        assert_eq!(parse(&signed(&release("1.2.0", "1.2.0"))).unwrap(), None);
        assert_eq!(parse(&signed(&release("1.1.9", "9.0.0"))).unwrap(), None);
    }

    #[test]
    fn a_tampered_or_foreign_release_is_rejected() {
        let text = signed(&release("1.3.0", "1.0.0")).replace("1.3.0", "1.4.0");
        assert!(format!("{:#}", parse(&text).unwrap_err()).contains("서명"));

        let foreign = SigningKey::from_bytes(&[8; 32]);
        let body = release("1.3.0", "1.0.0");
        let text = format!(
            "{}\n{body}",
            to_hex(&foreign.sign(body.as_bytes()).to_bytes())
        );
        assert!(parse(&text).is_err());
        assert!(parse(&body).is_err());
    }

    #[test]
    fn an_unsafe_asset_path_is_rejected() {
        let body = release("1.3.0", "1.0.0").replace("download/v1.3.0", "../escape");
        let result = parse(&signed(&body));
        if cfg!(any(windows, target_os = "macos")) {
            assert!(result.is_err());
        }
    }

    #[test]
    fn accepts_a_release_signed_by_the_script() {
        const PUBLIC_KEY: &str = "649ba6e75a625f80e1c5661a684c6820dd9a493a60c3107ebf6acad064239876";
        const LATEST: &str = r#"a74f6569e212df90f12621afd2acc9cd63fb47c6fe9f2aa3327d6628aaa87a340b64a60a34a57d9b0d630230c1728ee3fdb6cdd77672f724346b3bcfc9ea7c02
{
  "version": "9.9.9",
  "min_version": "0.0.0",
  "windows": {"path": "download/v9.9.9/p5099.exe", "size": 4, "sha256": "0105fb9b4e2a56d8047bf4f9a02c87ee598ef4364f92d7eec0712f9791a0d1fd"},
  "macos": {"path": "download/v9.9.9/p5099.app.zip", "size": 4, "sha256": "92412659cfa124853189031fb120bbcf67ffd836db3ba52b21c2b40b03c1dd7b"}
}
"#;
        let key = VerifyingKey::from_bytes(&from_hex(PUBLIC_KEY).unwrap()).unwrap();
        let update = parse_release(LATEST, &key, Version(1, 0, 0))
            .unwrap()
            .unwrap();
        assert_eq!(update.version, Version(9, 9, 9));
        if cfg!(any(windows, target_os = "macos")) {
            assert!(
                update
                    .asset
                    .unwrap()
                    .key
                    .starts_with("download/v9.9.9/p5099.")
            );
        }
        assert!(!update.required);
    }

    #[test]
    fn updates_come_from_the_repository_releases() {
        assert_eq!(
            updater::url(&releases_url("ruoh42/P5099_Rust"), RELEASE_KEY),
            "https://github.com/ruoh42/P5099_Rust/releases/latest/download/latest.txt"
        );
        assert_eq!(
            updater::url(
                &releases_url("ruoh42/P5099_Rust"),
                "download/v1.3.0/p5099.app.zip"
            ),
            "https://github.com/ruoh42/P5099_Rust/releases/download/v1.3.0/p5099.app.zip"
        );
    }

    #[test]
    fn swapping_keeps_a_backup_and_restores_on_failure() {
        let folder = tempdir().unwrap();
        let current = folder.path().join("launcher");
        let replacement = folder.path().join("launcher.new");
        let backup = folder.path().join("launcher.old");
        fs::write(&current, b"v1").unwrap();
        fs::write(&replacement, b"v2").unwrap();
        fs::write(&backup, b"v0").unwrap();
        swap_in(&current, &replacement, &backup).unwrap();
        assert_eq!(fs::read(&current).unwrap(), b"v2");
        assert_eq!(fs::read(&backup).unwrap(), b"v1");
        assert!(!replacement.exists());

        swap_in(&current, &replacement, &backup).unwrap_err();
        assert_eq!(fs::read(&current).unwrap(), b"v2");
    }
}
