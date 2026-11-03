//! Server initialization and lifecycle management for `dtn-hdy-ham`.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use hardy_async::TaskPool;
use hardy_bpa::{
    bpa::Bpa,
    cla::{Cla, ClaInit},
    node_ids::NodeIds,
    routing::{RouteAction, StaticRoutingAgent},
    storage::{BundleMemStorage, BundleStorage, MetadataMemStorage, MetadataStorage},
};
use hardy_localdisk_storage::LocalDiskStorage;
use hardy_proto::server::GrpcServer;
use hardy_sqlite_storage::SqliteStorage;
use tracing::{info, warn};

use crate::config::{HamConfig, cla::ClaType};

/// The standalone BPA server instance.
pub struct HamServer {
    bpa: Arc<Bpa>,
    grpc_config: Option<hardy_proto::server::Config>,
    tcp_peers: Vec<(Arc<hardy_tcpclv4::Tcpclv4>, Vec<SocketAddr>)>,
    tasks: TaskPool,
}

impl HamServer {
    /// Constructs a new `HamServer` from the provided configuration.
    pub async fn new(config: HamConfig) -> anyhow::Result<Self> {
        let tasks = TaskPool::new();

        // 1. Storage setup
        let (metadata_storage, bundle_storage): (Arc<dyn MetadataStorage>, Arc<dyn BundleStorage>) =
            if config.storage.in_memory {
                info!("Using in-memory bundle and metadata storage");
                (
                    Arc::new(MetadataMemStorage::new(None)),
                    Arc::new(BundleMemStorage::new(None, None)),
                )
            } else {
                let db_path = config.storage.sqlite_path.unwrap_or_else(|| {
                    if let Some(base_dirs) = directories::BaseDirs::new() {
                        base_dirs
                            .data_local_dir()
                            .join("hardy-ham")
                            .join("bundles.db")
                    } else {
                        PathBuf::from("bundles.db")
                    }
                });

                let parent_dir = db_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                let _ = std::fs::create_dir_all(parent_dir);

                let bundle_dir = parent_dir.join("payloads");
                let _ = std::fs::create_dir_all(&bundle_dir);

                info!("Using SQLite metadata storage at {}", db_path.display());
                info!(
                    "Using localdisk bundle payload storage at {}",
                    bundle_dir.display()
                );

                let db_dir = Some(parent_dir.to_path_buf());
                let db_name = db_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string());

                let meta_store = Arc::new(SqliteStorage::new(db_dir, db_name, true));
                let bundle_store =
                    Arc::new(LocalDiskStorage::new(Some(bundle_dir), Some(false), true));
                (meta_store, bundle_store)
            };

        // 2. Parse Node IDs
        let mut endpoints = Vec::new();
        endpoints.push(config.node_id.parse().context("Invalid primary node-id")?);
        for ep in &config.admin_endpoints {
            endpoints.push(ep.parse().context("Invalid admin-endpoint")?);
        }
        let node_ids =
            NodeIds::try_from(endpoints.as_slice()).context("Invalid node-ids configuration")?;

        let mut builder = Bpa::builder()
            .node_ids(node_ids)
            .metadata_storage(metadata_storage)
            .bundle_storage(bundle_storage);

        // 3. Register Static Routes if configured
        if !config.routes.is_empty() {
            let mut routes = Vec::new();
            for r in &config.routes {
                let pattern = r.pattern.parse().context("Invalid route pattern")?;
                let via_eid = if let Some(ref hop) = r.next_hop {
                    hop.parse().context("Invalid next_hop EID")?
                } else {
                    hardy_bpv7::eid::Eid::Null
                };
                routes.push((pattern, RouteAction::Via(via_eid), 100));
            }
            let routing_agent = Arc::new(StaticRoutingAgent::new(&routes));
            builder = builder.routing_agent("ham_static_routes".to_string(), routing_agent);
        }

        // 4. Configure CLAs
        let mut tcp_peers = Vec::new();
        for cla_config in config.clas {
            let (cla, init): (Arc<dyn Cla>, ClaInit) = match cla_config.cla_type {
                ClaType::TcpClv4(tcp_cfg) => {
                    let mut b = hardy_tcpclv4::Tcpclv4::builder();
                    if let Some(listeners) = tcp_cfg.listeners {
                        for addr in listeners {
                            b = b.listen(addr);
                        }
                    } else if let Some(addr) = tcp_cfg.address {
                        b = b.listen(addr);
                    } else {
                        b = b.listen_default();
                    }

                    if tcp_cfg.require_tls == Some(false) {
                        info!(
                            "TCPCLv4 '{}': TLS disabled for amateur radio compliance",
                            cla_config.name
                        );
                    } else if tcp_cfg.require_tls == Some(true) {
                        warn!("TCPCLv4 '{}': TLS enabled", cla_config.name);
                    }

                    if let Some(mru) = tcp_cfg.segment_mru {
                        b = b.segment_mru(mru);
                    }
                    if let Some(mru) = tcp_cfg.transfer_mru {
                        b = b.transfer_mru(mru);
                    }

                    let cla = Arc::new(b.build().context("Failed to build TCPCLv4 CLA")?);
                    let init = cla.cla_init();

                    if let Some(peers) = tcp_cfg.peers {
                        let mut resolved_peers = Vec::new();
                        for peer_str in peers {
                            match peer_str.to_socket_addrs() {
                                Ok(addrs) => resolved_peers.extend(addrs),
                                Err(e) => {
                                    warn!(
                                        "Failed to resolve TCPCL peer address '{}': {}",
                                        peer_str, e
                                    );
                                }
                            }
                        }
                        if !resolved_peers.is_empty() {
                            tcp_peers.push((cla.clone(), resolved_peers));
                        }
                    }

                    (cla, init)
                }
                ClaType::Ltp(ltp_cfg) => {
                    info!(
                        "Initializing LTP CLA '{}' (Engine ID {})",
                        cla_config.name, ltp_cfg.engine_id
                    );
                    let cla = hardy_ltp_cla::LtpCla::new(ltp_cfg);
                    let init = cla.cla_init();
                    (Arc::new(cla), init)
                }
                ClaType::Hqfbp(hqfbp_cfg) => {
                    info!("Initializing HQFBP CLA '{}'", cla_config.name);
                    let cla = hardy_hqfbp_cla::HqfbpCla::new(hqfbp_cfg);
                    let init = cla.cla_init();
                    (Arc::new(cla), init)
                }
                ClaType::Other { cla_type, .. } => {
                    warn!(
                        "Ignoring CLA '{}' with unsupported type '{}'",
                        cla_config.name, cla_type
                    );
                    continue;
                }
            };

            builder = builder.cla(cla_config.name, cla, None, init);
        }

        let bpa = Arc::new(builder.build().await.map_err(anyhow::Error::from_boxed)?);

        let grpc_config = if config.grpc.enabled {
            Some(hardy_proto::server::Config {
                address: config.grpc.address,
                services: vec![
                    "cla".into(),
                    "service".into(),
                    "application".into(),
                    "routing".into(),
                ],
            })
        } else {
            None
        };

        Ok(Self {
            bpa,
            grpc_config,
            tcp_peers,
            tasks,
        })
    }

    /// Runs the BPA server to completion.
    pub async fn run(self) -> anyhow::Result<()> {
        info!("Starting Hamradio BPA server");
        self.bpa.start(true).await;

        for (cla, addrs) in &self.tcp_peers {
            for peer_addr in addrs {
                let peer_addr = *peer_addr;
                let cla = cla.clone();
                let cancel = self.tasks.cancel_token().clone();
                hardy_async::spawn!(self.tasks, "tcpcl_peer_connector", async move {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    loop {
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            res = cla.connect(&peer_addr) => {
                                match res {
                                    Ok(()) => {
                                        info!("Successfully connected to TCPCL peer {}", peer_addr);
                                        break;
                                    }
                                    Err(e) => {
                                        tracing::debug!(
                                            "TCPCL connect to {} failed ({}), retrying in 1s...",
                                            peer_addr,
                                            e
                                        );
                                    }
                                }
                            }
                        }
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                        }
                    }
                });
            }
        }

        if let Some(ref grpc_config) = self.grpc_config {
            let server = GrpcServer::new(grpc_config, self.bpa.clone())
                .map_err(|e| anyhow::anyhow!("Failed to create gRPC server: {e}"))?;
            let cancel = self.tasks.cancel_token().clone();
            hardy_async::spawn!(self.tasks, "grpc_server", async move {
                if let Err(e) = server.serve(cancel).await {
                    tracing::error!("gRPC server failed: {e}");
                }
            });
            info!(
                "gRPC application service listening on {}",
                grpc_config.address
            );
        }

        info!("Hardy Ham BPA started successfully. Press Ctrl+C to terminate.");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Shutdown signal received");
            }
            _ = self.tasks.cancel_token().cancelled() => {}
        }

        self.shutdown().await;
        info!("Hardy Ham BPA server stopped.");
        Ok(())
    }

    /// Gracefully shuts down background tasks and the BPA engine.
    pub async fn shutdown(&self) {
        self.tasks.shutdown().await;
        self.bpa.shutdown().await;
    }

    /// Accessor for the underlying `Bpa` instance.
    #[must_use]
    pub fn bpa(&self) -> Arc<Bpa> {
        self.bpa.clone()
    }
}
