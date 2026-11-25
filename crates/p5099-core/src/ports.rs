use thiserror::Error;

pub const DEFAULT_CONFIGURED_PORT: u16 = 39_311;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[error("configured port {configured_port} cannot support offset +{required_offset}")]
pub struct PortOverflow {
    pub configured_port: u16,
    pub required_offset: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortTopology {
    configured_port: u16,
}

impl PortTopology {
    pub fn new(configured_port: u16) -> Result<Self, PortOverflow> {
        configured_port.checked_add(7).ok_or(PortOverflow {
            configured_port,
            required_offset: 7,
        })?;
        Ok(Self { configured_port })
    }

    #[must_use]
    pub fn configured(self) -> u16 {
        self.configured_port
    }

    #[must_use]
    pub fn login_tcp(self) -> u16 {
        self.configured_port + 1
    }

    #[must_use]
    pub fn game_udp(self) -> u16 {
        self.configured_port
    }

    #[must_use]
    pub fn p2p_udp(self) -> u16 {
        self.configured_port + 1
    }

    #[must_use]
    pub fn messenger_tcp(self) -> u16 {
        self.configured_port + 2
    }

    #[must_use]
    pub fn xun_sidecar_tcp(self) -> u16 {
        self.configured_port + 3
    }

    #[must_use]
    pub fn nmco_tcp(self) -> u16 {
        self.configured_port + 4
    }

    #[must_use]
    pub fn account_tcp(self) -> u16 {
        self.configured_port + 5
    }

    #[must_use]
    pub fn download_tcp(self) -> u16 {
        self.configured_port + 6
    }

    #[must_use]
    pub fn admin_tcp(self) -> u16 {
        self.configured_port + 7
    }
}

impl Default for PortTopology {
    fn default() -> Self {
        Self::new(DEFAULT_CONFIGURED_PORT).expect("the P5099 default topology is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_CONFIGURED_PORT, PortTopology};

    #[test]
    fn reproduces_p5099_port_offsets() {
        let ports = PortTopology::default();
        assert_eq!(ports.configured(), DEFAULT_CONFIGURED_PORT);
        assert_eq!(ports.login_tcp(), 39_312);
        assert_eq!(ports.game_udp(), 39_311);
        assert_eq!(ports.p2p_udp(), 39_312);
        assert_eq!(ports.messenger_tcp(), 39_313);
        assert_eq!(ports.xun_sidecar_tcp(), 39_314);
        assert_eq!(ports.nmco_tcp(), 39_315);
        assert_eq!(ports.account_tcp(), 39_316);
        assert_eq!(ports.download_tcp(), 39_317);
        assert_eq!(ports.admin_tcp(), 39_318);
    }

    #[test]
    fn rejects_a_base_that_would_wrap() {
        assert!(PortTopology::new(u16::MAX - 2).is_err());
        assert!(PortTopology::new(u16::MAX - 1).is_err());
        assert!(PortTopology::new(u16::MAX - 6).is_err());
        assert!(PortTopology::new(u16::MAX - 7).is_ok());
    }
}
