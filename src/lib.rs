pub mod basket;
pub mod security;

use bytes::Bytes;
use hardy_bpa::async_trait;
use hardy_bpa::cla::{Cla as BpaCla, ForwardBundleResult, Sink as ClaSink};
use hardy_bpv7::eid::NodeId;
use tokio::sync::OnceCell;

/// Normalize a user-supplied EID string so the BPv7 parser accepts it.
///
/// The BPv7 `dtn`-scheme parser requires a `/` separating the node name from
/// the (possibly empty) service name.  Users commonly omit the trailing slash
/// when they only intend to address a node (e.g. `dtn://beacon` instead of
/// `dtn://beacon/`).  This function detects that pattern and appends the
/// missing `/` before the string is handed to the parser.
pub fn normalize_eid(s: &str) -> String {
    let s = s.trim();
    if let Some(rest) = s
        .strip_prefix("dtn://")
        .filter(|r| !r.is_empty() && !r.contains('/'))
    {
        return format!("dtn://{}/", rest);
    }
    s.to_string()
}

/// Resolve the gRPC port based on environment variables and a CLI option or fallback.
///
/// Priority order:
/// 1. `HARDY_GRPC_PORT` env var
/// 2. `DTN_WEB_PORT` env var
/// 3. The provided option (CLI arg)
/// 4. Default fallback port (50051)
pub fn resolve_grpc_port(cli_port: Option<u16>) -> String {
    if let Ok(env_port) = std::env::var("HARDY_GRPC_PORT") {
        env_port
    } else if let Ok(env_port) = std::env::var("DTN_WEB_PORT") {
        env_port
    } else if let Some(port) = cli_port {
        port.to_string()
    } else {
        "50051".to_string()
    }
}

pub struct NoopSenderCla {
    pub sink: OnceCell<Box<dyn ClaSink>>,
}

impl Default for NoopSenderCla {
    fn default() -> Self {
        Self {
            sink: OnceCell::new(),
        }
    }
}

#[async_trait]
impl BpaCla for NoopSenderCla {
    async fn on_register(&self, sink: Box<dyn ClaSink>, _node_ids: &[NodeId]) {
        let _ = self.sink.set(sink);
    }

    async fn on_unregister(&self) {}

    async fn forward(
        &self,
        _queue: Option<u32>,
        _cla_addr: &hardy_bpa::cla::ClaAddress,
        _bundle_id: &hardy_bpv7::bundle::Id,
        _bundle: Bytes,
    ) -> hardy_bpa::cla::Result<ForwardBundleResult> {
        Ok(ForwardBundleResult::Sent)
    }
}
