use crate::config::NodeRuntimeConfig;

#[derive(Debug, Clone)]
pub struct P2pConfig {
    pub tcp_port: u16,
    pub udp_port: u16,
}

impl From<&NodeRuntimeConfig> for P2pConfig {
    fn from(config: &NodeRuntimeConfig) -> Self {
        Self {
            tcp_port: config
                .p2p_tcp_port
                .as_deref()
                .unwrap_or("4001")
                .parse::<u16>()
                .unwrap_or(4001),
            udp_port: config
                .p2p_udp_port
                .as_deref()
                .unwrap_or("4002")
                .parse::<u16>()
                .unwrap_or(4002),
        }
    }
}
