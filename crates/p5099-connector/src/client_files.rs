use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

use p5099_core::client_manifest::{
    ClientManifest, ManifestEntry, is_excluded_client_file, is_keep_client_file,
    is_safe_relative_path,
};
use sha2::{Digest, Sha256};

pub fn hash_file(path: &Path) -> io::Result<(u64, [u8; 32])> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    let mut size = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((size, hasher.finalize().into()))
}

fn collect(directory: &Path, prefix: &str, paths: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err(io::Error::other(format!(
                "non-UTF-8 file name in {}",
                directory.display()
            )));
        };
        let relative = format!("{prefix}{name}");
        if is_excluded_client_file(&relative) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&entry.path(), &format!("{relative}/"), paths)?;
        } else if kind.is_file() {
            if !is_safe_relative_path(&relative) {
                return Err(io::Error::other(format!("unsafe file name {relative:?}")));
            }
            paths.push(relative);
        }
    }
    Ok(())
}

pub fn build_client_manifest(
    client_directory: &Path,
    root: &str,
    mut progress: impl FnMut(&str),
) -> io::Result<ClientManifest> {
    let root = root.trim_matches('/');
    let root = if root.is_empty() {
        String::new()
    } else if is_safe_relative_path(root) {
        format!("{root}/")
    } else {
        return Err(io::Error::other(format!("unsafe storage folder {root:?}")));
    };
    let mut paths = Vec::new();
    collect(client_directory, "", &mut paths)?;
    paths.sort();
    let mut entries = Vec::with_capacity(paths.len());
    for path in paths {
        progress(&path);
        let (size, sha256) = hash_file(&client_directory.join(&path))?;
        entries.push(ManifestEntry {
            keep: is_keep_client_file(&path),
            path,
            size,
            sha256,
        });
    }
    let revision = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    Ok(ClientManifest {
        root,
        revision,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn builds_a_sorted_manifest_without_connector_outputs() {
        let client = tempdir().unwrap();
        let root = client.path();
        fs::create_dir_all(root.join("Data")).unwrap();
        fs::create_dir_all(root.join("Profile/kr")).unwrap();
        fs::write(root.join("KartRider.exe"), b"exe").unwrap();
        fs::write(root.join("KartRider.pin"), b"pin").unwrap();
        fs::write(root.join("KartRider.pin.pristine.bak"), b"pin").unwrap();
        fs::write(root.join("KartRider.xml"), b"xml").unwrap();
        fs::write(root.join("Data/kart.rho"), b"rho").unwrap();
        fs::write(root.join("Profile/kr/launcher.xml"), b"me").unwrap();

        let mut seen = Vec::new();
        let manifest =
            build_client_manifest(root, "/client/", |path| seen.push(path.to_owned())).unwrap();
        assert_eq!(manifest.root, "client/");
        let paths: Vec<_> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["Data/kart.rho", "KartRider.exe", "KartRider.pin"]);
        assert_eq!(seen, paths);
        assert!(manifest.entries[2].keep);
        assert_eq!(manifest.entries[0].size, 3);
        assert_eq!(
            manifest.entries[0].sha256,
            <[u8; 32]>::from(Sha256::digest(b"rho"))
        );
    }
}
