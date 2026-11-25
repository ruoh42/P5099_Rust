use std::fmt::Write as _;

use thiserror::Error;

use crate::account_protocol::{from_hex, to_hex};

pub const CLIENT_MANIFEST_FILE_NAME: &str = "p5099-manifest.txt";
pub const CLIENT_MANIFEST_SIGNATURE_FILE_NAME: &str = "p5099-manifest.sig";
const HEADER_V1: &str = "p5099-manifest 1";
const HEADER: &str = "p5099-manifest 2";
const MAX_PATH_CHARS: usize = 260;

const KEEP_FILES: [&str; 2] = ["KartRider.pin", "Data/DataPack1_00014.rho5"];
const EXCLUDED_TOP_LEVEL: [&str; 9] = [
    "Profile",
    "logs",
    "docs",
    "KartRider.xml",
    "p5099-xun-session.ini",
    "p5099-xun.ini",
    CLIENT_MANIFEST_FILE_NAME,
    CLIENT_MANIFEST_SIGNATURE_FILE_NAME,
    "DxError.log",
];
const EXCLUDED_SUFFIXES: [&str; 6] = [
    ".log",
    ".bak",
    ".pristine.absent",
    ".p5099-download",
    ".DS_Store",
    "Thumbs.db",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub path: String,
    pub size: u64,
    pub sha256: [u8; 32],
    pub keep: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientManifest {
    pub root: String,
    pub revision: u64,
    pub entries: Vec<ManifestEntry>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ManifestError {
    #[error("not a P5099 client manifest")]
    BadHeader,
    #[error("manifest line {line} is malformed")]
    BadLine { line: usize },
    #[error("manifest path {0:?} is not a safe relative path")]
    UnsafePath(String),
    #[error("manifest lists {0:?} twice")]
    Duplicate(String),
}

#[must_use]
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.chars().count() <= MAX_PATH_CHARS
        && !path.starts_with('/')
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

#[must_use]
pub fn is_excluded_client_file(path: &str) -> bool {
    let top = path.split('/').next().unwrap_or_default();
    EXCLUDED_TOP_LEVEL
        .iter()
        .any(|excluded| top.eq_ignore_ascii_case(excluded))
        || path.split('/').any(|segment| segment.starts_with('.'))
        || EXCLUDED_SUFFIXES
            .iter()
            .any(|suffix| path.ends_with(suffix))
        || path.contains(".pristine")
}

#[must_use]
pub fn is_keep_client_file(path: &str) -> bool {
    KEEP_FILES
        .iter()
        .any(|keep| path.eq_ignore_ascii_case(keep))
}

impl ClientManifest {
    #[must_use]
    pub fn key(&self, entry: &ManifestEntry) -> String {
        format!("{}{}", self.root, entry.path)
    }

    #[must_use]
    pub fn total_size(&self) -> u64 {
        self.entries.iter().map(|entry| entry.size).sum()
    }

    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = format!("{HEADER}\nroot {}\nrevision {}\n", self.root, self.revision);
        for entry in &self.entries {
            let flags = if entry.keep { "keep" } else { "-" };
            let _ = writeln!(
                text,
                "{}\t{}\t{flags}\t{}",
                to_hex(&entry.sha256),
                entry.size,
                entry.path
            );
        }
        text
    }

    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let mut lines = text.lines().enumerate();
        let versioned = match lines.next().map(|(_, line)| line.trim()) {
            Some(HEADER) => true,
            Some(HEADER_V1) => false,
            _ => return Err(ManifestError::BadHeader),
        };
        let root = match lines.next() {
            Some((_, line)) => line
                .strip_prefix("root")
                .map(str::trim)
                .ok_or(ManifestError::BadHeader)?
                .to_owned(),
            None => return Err(ManifestError::BadHeader),
        };
        if !(root.is_empty()
            || (root.ends_with('/') && is_safe_relative_path(&root[..root.len() - 1])))
        {
            return Err(ManifestError::UnsafePath(root));
        }
        let revision = if versioned {
            lines
                .next()
                .and_then(|(_, line)| line.strip_prefix("revision "))
                .and_then(|value| value.trim().parse().ok())
                .ok_or(ManifestError::BadHeader)?
        } else {
            0
        };
        let mut entries: Vec<ManifestEntry> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (index, line) in lines {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let bad = || ManifestError::BadLine { line: index + 1 };
            let mut fields = line.splitn(4, '\t');
            let (Some(hash), Some(size), Some(flags), Some(path)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return Err(bad());
            };
            if !is_safe_relative_path(path) {
                return Err(ManifestError::UnsafePath(path.to_owned()));
            }
            if !seen.insert(path.to_ascii_lowercase()) {
                return Err(ManifestError::Duplicate(path.to_owned()));
            }
            entries.push(ManifestEntry {
                path: path.to_owned(),
                size: size.parse().map_err(|_| bad())?,
                sha256: from_hex(hash).map_err(|_| bad())?,
                keep: match flags {
                    "keep" => true,
                    "-" => false,
                    _ => return Err(bad()),
                },
            });
        }
        Ok(Self {
            root,
            revision,
            entries,
        })
    }
}

#[must_use]
pub fn encode_key(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len());
    for byte in key.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[must_use]
pub fn decode_key(encoded: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips() {
        let manifest = ClientManifest {
            root: "client/".to_owned(),
            revision: 1_795_000_000,
            entries: vec![
                ManifestEntry {
                    path: "KartRider.pin".to_owned(),
                    size: 10,
                    sha256: [1; 32],
                    keep: true,
                },
                ManifestEntry {
                    path: "Data/kart.rho".to_owned(),
                    size: 3_000_000_000,
                    sha256: [2; 32],
                    keep: false,
                },
            ],
        };
        let parsed = ClientManifest::parse(&manifest.to_text()).unwrap();
        assert_eq!(parsed, manifest);
        assert_eq!(parsed.key(&parsed.entries[1]), "client/Data/kart.rho");
        assert_eq!(parsed.total_size(), 3_000_000_010);
    }

    #[test]
    fn reads_unversioned_manifests_as_revision_zero() {
        let text = format!(
            "p5099-manifest 1\nroot client/\n{}\t1\t-\tKartRider.pin\n",
            "00".repeat(32)
        );
        let parsed = ClientManifest::parse(&text).unwrap();
        assert_eq!(parsed.revision, 0);
        assert_eq!(parsed.entries.len(), 1);
        assert!(ClientManifest::parse("p5099-manifest 2\nroot client/\n").is_err());
    }

    #[test]
    fn escaping_paths_are_rejected() {
        for path in [
            "../x",
            "/etc/passwd",
            "Data/../../x",
            "C:/x",
            "a\\b",
            "a//b",
            "",
        ] {
            assert!(!is_safe_relative_path(path), "{path}");
        }
        let text = format!(
            "{HEADER}\nroot \nrevision 1\n{}\t1\t-\t../evil.dll\n",
            "00".repeat(32)
        );
        assert!(matches!(
            ClientManifest::parse(&text),
            Err(ManifestError::UnsafePath(_))
        ));
        assert!(matches!(
            ClientManifest::parse(&format!("{HEADER}\nroot ../\n")),
            Err(ManifestError::UnsafePath(_))
        ));
    }

    #[test]
    fn connector_outputs_and_backups_are_not_distributed() {
        for path in [
            "KartRider.xml",
            "Profile/kr/launcher.xml",
            "p5099-xun-session.ini",
            "KartRider.pin.pristine.bak",
            "Data/DataPack1_00014.rho5.pristine.bak",
            "KartRider.xml.pristine.absent",
            ".p5099-asset-import/report.json",
            ".cache/tool/account.json",
            "client.log",
            "logs/a.txt",
        ] {
            assert!(is_excluded_client_file(path), "{path}");
        }
        assert!(!is_excluded_client_file("Data/DataPack1_00000.rho5"));
        assert!(!is_excluded_client_file("KartRider.exe"));
        assert!(is_keep_client_file("KartRider.pin"));
    }

    #[test]
    fn keys_are_percent_encoded() {
        assert_eq!(
            encode_key("client/Data/a b+.rho"),
            "client/Data/a%20b%2B.rho"
        );
        assert_eq!(encode_key("트랙.rho"), "%ED%8A%B8%EB%9E%99.rho");
    }

    #[test]
    fn keys_decode_back() {
        for key in ["client/Data/a b+.rho", "트랙.rho", "plain/path.txt"] {
            assert_eq!(decode_key(&encode_key(key)).as_deref(), Some(key));
        }
        assert_eq!(decode_key("a%2"), None);
        assert_eq!(decode_key("a%zz"), None);
        assert_eq!(decode_key("%FF"), None);
    }
}
