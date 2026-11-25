use std::net::{IpAddr, Ipv4Addr};

use anyhow::{Context as _, Result, anyhow, bail};
use p5099_core::ports::{DEFAULT_CONFIGURED_PORT, PortTopology};

const MAX_HOST_CHARS: usize = 253;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    host: String,
    ports: PortTopology,
}

impl ServerAddress {
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let text = text
            .strip_prefix("http://")
            .or_else(|| text.strip_prefix("https://"))
            .unwrap_or(text)
            .trim_end_matches('/');
        if text.is_empty() {
            bail!("서버 주소를 입력하세요");
        }
        let (host, port) = match text.rsplit_once(':') {
            Some((host, port)) => {
                let port = port
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| anyhow!("서버 포트가 올바르지 않습니다: {port}"))?;
                (host, port)
            }
            None => (text, DEFAULT_CONFIGURED_PORT),
        };
        let valid = !host.is_empty()
            && host.len() <= MAX_HOST_CHARS
            && !host.starts_with(['.', '-'])
            && host.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-')
            });
        if !valid {
            bail!("서버 주소가 올바르지 않습니다: {host}");
        }
        let ports =
            PortTopology::new(port).map_err(|_| anyhow!("서버 포트가 너무 큽니다: {port}"))?;
        Ok(Self {
            host: host.to_ascii_lowercase(),
            ports,
        })
    }

    pub const fn ports(&self) -> PortTopology {
        self.ports
    }

    pub fn download_url(&self) -> String {
        format!("http://{}:{}", self.host, self.ports.download_tcp())
    }

    pub async fn resolve(&self) -> Result<Ipv4Addr> {
        tokio::net::lookup_host((self.host.as_str(), self.ports.login_tcp()))
            .await
            .with_context(|| format!("서버 주소 {}을(를) 찾지 못했습니다", self.host))?
            .find_map(|address| match address.ip() {
                IpAddr::V4(address) => Some(address),
                IpAddr::V6(_) => None,
            })
            .ok_or_else(|| anyhow!("서버 주소 {}에 IPv4 주소(A 레코드)가 없습니다", self.host))
    }
}

impl std::fmt::Display for ServerAddress {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.host)?;
        if self.ports.configured() != DEFAULT_CONFIGURED_PORT {
            write!(formatter, ":{}", self.ports.configured())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hosts_and_base_ports() {
        let server = ServerAddress::parse(" Kart.Example.com ").unwrap();
        assert_eq!(server.to_string(), "kart.example.com");
        assert_eq!(server.ports(), PortTopology::default());
        assert_eq!(server.download_url(), "http://kart.example.com:39317");

        let server = ServerAddress::parse("http://192.0.2.10:40000/").unwrap();
        assert_eq!(server.to_string(), "192.0.2.10:40000");
        assert_eq!(server.ports().account_tcp(), 40_005);

        assert_eq!(
            ServerAddress::parse("kart.example.com:39311").unwrap(),
            ServerAddress::parse("KART.example.com").unwrap()
        );
    }

    #[test]
    fn refuses_what_is_not_a_host() {
        for bad in [
            "",
            "   ",
            "kart example.com",
            "kart.example.com:0",
            "kart.example.com:70000",
            "kart.example.com:65530",
            "kart.example.com:port",
            "[2001:db8::1]",
            "user@kart.example.com",
            "kart.example.com/path",
            "-kart.example.com",
        ] {
            assert!(ServerAddress::parse(bad).is_err(), "{bad}");
        }
    }
}
