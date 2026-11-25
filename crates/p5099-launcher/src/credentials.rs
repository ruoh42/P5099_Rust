use anyhow::Result;
use p5099_core::account_protocol::{AccountKey, from_hex, to_hex};

use crate::server::ServerAddress;

pub const SUPPORTED: bool = cfg!(any(windows, target_os = "macos"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedLogin {
    pub server: String,
    pub nickname: String,
    pub key: AccountKey,
}

impl SavedLogin {
    fn encode(&self) -> String {
        format!("{}\n{}\n{}", to_hex(&self.key), self.nickname, self.server)
    }

    fn decode(secret: &str, legacy_server: Option<&str>) -> Option<Self> {
        let mut lines = secret.splitn(3, '\n');
        let key = from_hex(lines.next()?).ok()?;
        let nickname = lines.next()?.to_owned();
        let server = ServerAddress::parse(lines.next().or(legacy_server)?)
            .ok()?
            .to_string();
        Some(Self {
            server,
            nickname,
            key,
        })
    }
}

pub fn load(legacy_server: Option<&str>) -> Option<SavedLogin> {
    store::load()
        .as_deref()
        .and_then(|secret| SavedLogin::decode(secret, legacy_server))
}

pub fn save(login: &SavedLogin) -> Result<()> {
    store::save(&login.encode())
}

pub fn clear() {
    store::clear();
}

#[cfg(any(windows, target_os = "macos"))]
mod store {
    use anyhow::{Context as _, Result};

    const SERVICE: &str = "p5099";
    const OLD_SERVICE: &str = "kartrider-p5099";
    const USER: &str = "saved-login";

    fn entry(service: &str) -> keyring::Result<keyring::Entry> {
        keyring::Entry::new(service, USER)
    }

    fn get(service: &str) -> Option<String> {
        entry(service).ok()?.get_password().ok()
    }

    fn delete(service: &str) {
        if let Ok(entry) = entry(service) {
            let _ = entry.delete_credential();
        }
    }

    pub fn load() -> Option<String> {
        if let Some(secret) = get(SERVICE) {
            return Some(secret);
        }
        let secret = get(OLD_SERVICE)?;
        if save(&secret).is_ok() {
            delete(OLD_SERVICE);
        }
        Some(secret)
    }

    pub fn save(secret: &str) -> Result<()> {
        entry(SERVICE)
            .and_then(|entry| entry.set_password(secret))
            .context("비밀번호를 저장하지 못했습니다")
    }

    pub fn clear() {
        delete(SERVICE);
        delete(OLD_SERVICE);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod store {
    use anyhow::{Result, bail};

    pub const fn load() -> Option<String> {
        None
    }

    pub fn save(_secret: &str) -> Result<()> {
        bail!("이 운영체제에서는 비밀번호를 저장할 수 없습니다")
    }

    pub const fn clear() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_login_round_trips() {
        let login = SavedLogin {
            server: "kart.example.com:40000".to_owned(),
            nickname: "Rider".to_owned(),
            key: [7; 32],
        };
        assert_eq!(
            SavedLogin::decode(&login.encode(), None),
            Some(login.clone())
        );
        assert_eq!(
            SavedLogin::decode("not hex\nRider\nkart.example.com", None),
            None
        );
        let bad = format!("{}\nRider\nnot a host", to_hex(&[7; 32]));
        assert_eq!(SavedLogin::decode(&bad, None), None);
    }

    #[test]
    fn an_older_login_belongs_to_the_built_in_server() {
        let old = format!("{}\nRider", to_hex(&[7; 32]));
        let login = SavedLogin::decode(&old, Some("Kart.Example.com")).unwrap();
        assert_eq!(login.server, "kart.example.com");
        assert_eq!(SavedLogin::decode(&old, None), None);
    }
}
