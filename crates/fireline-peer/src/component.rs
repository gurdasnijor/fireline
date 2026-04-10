//! [`PeerComponent`] — the [`sacp::component::Component`] implementation
//! that provides cross-agent calls for Fireline.

use std::path::PathBuf;

use sacp::{Conductor, ConnectTo, Proxy};

/// Compile-safe placeholder for the peer layer.
///
/// The ACP-native peering spike will replace this passthrough proxy with the
/// real MCP tool injection and lineage propagation logic.
#[derive(Debug, Clone)]
pub struct PeerComponent {
    #[allow(dead_code)]
    directory_path: PathBuf,
}

impl PeerComponent {
    pub fn new(directory_path: impl Into<PathBuf>) -> Self {
        Self {
            directory_path: directory_path.into(),
        }
    }
}

impl ConnectTo<Conductor> for PeerComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        Proxy
            .builder()
            .name("fireline-peer")
            .connect_to(client)
            .await
    }
}
