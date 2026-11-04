//! Server initialization and lifecycle management for `dtn-hdy-ham`.

use std::net::{SocketAddr, ToSocketAddrs};
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

use crate::config::{HamConfig, cla::ClaType, cla::ScheduleConfig};
use crate::coordinator::{ContactCoordinator, ContactSchedule, ContactTarget};
use crate::web::AppState;
use crate::web::injector::WebBundleInjector;
use std::num::NonZeroU32;

/// The standalone BPA server instance.
pub struct HamServer {
    config: HamConfig,
    bpa: Arc<Bpa>,
    service_tracker: crate::service_tracker::ServiceTracker,
    grpc_config: Option<hardy_proto::server::Config>,
    tcp_peers: Vec<(Arc<hardy_tcpclv4::Tcpclv4>, Vec<SocketAddr>)>,
    coordinators: Vec<ContactCoordinator>,
    web_injector: Option<WebBundleInjector>,
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
                let data_dir = config.storage.effective_data_dir();
                let db_path = config.storage.effective_db_path();
                let bundle_dir = config.storage.effective_payload_dir();

                let _ = std::fs::create_dir_all(&data_dir);
                let _ = std::fs::create_dir_all(&bundle_dir);

                info!("Using SQLite metadata storage at {}", db_path.display());
                info!(
                    "Using localdisk bundle payload storage at {}",
                    bundle_dir.display()
                );

                let db_dir = Some(data_dir);
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
        let mut coordinators = Vec::new();
        for cla_config in config.clas.clone() {
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
                ClaType::MercuryArq(mercury_cfg) => {
                    info!(
                        "Initializing Mercury Connected ARQ CLA '{}' (mycall={:?}, ctl={}, data={})",
                        cla_config.name,
                        mercury_cfg.my_callsign,
                        mercury_cfg.control_addr,
                        mercury_cfg.data_addr
                    );

                    let m_cfg = hardy_mercury_cla::Config {
                        my_callsign: mercury_cfg.my_callsign.unwrap_or_else(|| {
                            config
                                .callsign
                                .clone()
                                .unwrap_or_else(|| "N0CALL".to_string())
                        }),
                        control_addr: mercury_cfg.control_addr,
                        data_addr: mercury_cfg.data_addr,
                        bandwidth: mercury_cfg.bandwidth,
                        connect_timeout: mercury_cfg.connect_timeout,
                        grace_period: mercury_cfg.grace_period,
                        idle_timeout: mercury_cfg.idle_timeout,
                        peers: mercury_cfg
                            .peers
                            .iter()
                            .map(|p| hardy_mercury_cla::PeerEntry {
                                callsign: p.callsign.clone(),
                                node_id: Some(
                                    p.effective_node_id().parse().expect("valid peer node ID"),
                                ),
                            })
                            .collect(),
                    };

                    let cla = Arc::new(hardy_mercury_cla::MercuryCla::new(m_cfg));
                    let init = cla.cla_init();

                    let mut targets = Vec::new();
                    for p in &mercury_cfg.peers {
                        if let Some(ref sched) = p.schedule {
                            let schedule = match sched {
                                ScheduleConfig::Recurring { cron, duration } => {
                                    let cron_expr = crate::coordinator::cron::CronExpr::parse(cron)
                                        .map_err(|e| {
                                            anyhow::anyhow!(
                                                "Invalid cron for peer {}: {e}",
                                                p.callsign
                                            )
                                        })?;
                                    ContactSchedule::Recurring {
                                        cron: cron_expr,
                                        duration: *duration,
                                    }
                                }
                                ScheduleConfig::OneShot { start, end } => {
                                    let start_dt = start.as_ref().and_then(|s| {
                                        time::OffsetDateTime::parse(
                                            s,
                                            &time::format_description::well_known::Rfc3339,
                                        )
                                        .ok()
                                    });
                                    let end_dt = end.as_ref().and_then(|s| {
                                        time::OffsetDateTime::parse(
                                            s,
                                            &time::format_description::well_known::Rfc3339,
                                        )
                                        .ok()
                                    });
                                    ContactSchedule::OneShot {
                                        start: start_dt,
                                        end: end_dt,
                                    }
                                }
                            };
                            targets.push(ContactTarget {
                                callsign: p.callsign.clone(),
                                node_id: p.effective_node_id().parse().expect("valid peer node ID"),
                                schedule,
                                grace_period: mercury_cfg
                                    .grace_period
                                    .unwrap_or(std::time::Duration::from_secs(30)),
                            });
                        }
                    }

                    if !targets.is_empty() {
                        let coordinator = ContactCoordinator::new(cla.clone(), targets);
                        coordinators.push(coordinator);
                    }

                    (cla, init)
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

        let web_injector = if config.web.is_some() {
            let injector = WebBundleInjector::new();
            builder = builder.cla(
                "web-sender",
                Arc::new(injector.clone()),
                None,
                ClaInit {
                    address_type: None,
                    lane_count: NonZeroU32::new(1),
                    max_bundle_size: None,
                },
            );
            Some(injector)
        } else {
            None
        };

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

        let service_tracker = crate::service_tracker::ServiceTracker::new(bpa.clone());

        Ok(Self {
            config,
            bpa,
            service_tracker,
            grpc_config,
            tcp_peers,
            coordinators,
            web_injector,
            tasks,
        })
    }

    /// Runs the BPA server to completion.
    pub async fn run(mut self) -> anyhow::Result<()> {
        info!("Starting Hamradio BPA server");
        self.bpa.start(true).await;

        // Register permanent built-in Echo service
        let echo_service = Arc::new(hardy_echo_service::EchoService::new());
        let echo_service_id = hardy_bpv7::eid::Service::Dtn("echo".into());
        match self
            .service_tracker
            .register_service_with_type(echo_service_id, echo_service, "Built-in Echo")
            .await
        {
            Ok(eid) => {
                info!("Registered permanent built-in Echo service at EID: {}", eid);
            }
            Err(e) => {
                warn!("Failed to register built-in Echo service: {}", e);
            }
        }

        let coordinators = std::mem::take(&mut self.coordinators);
        for coordinator in coordinators {
            let cancel = self.tasks.cancel_token().clone();
            coordinator.spawn(cancel);
        }

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
            let server = GrpcServer::new(grpc_config, Arc::new(self.service_tracker.clone()))
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

        if let Some(ref web_config) = self.config.web {
            let state = AppState {
                config: Arc::new(self.config.clone()),
                injector: self.web_injector.clone().expect("web_injector initialized"),
                service_tracker: self.service_tracker.clone(),
            };
            let cancel = self.tasks.cancel_token().clone();
            let addr = web_config.address;
            let web_handle = crate::web::start_web_server(state, addr).await?;
            let abort_handle = web_handle.abort_handle();
            hardy_async::spawn!(self.tasks, "web_server", async move {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        abort_handle.abort();
                    }
                    _ = web_handle => {}
                }
            });
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

    /// Accessor for the web bundle injector if configured.
    #[must_use]
    pub fn web_injector(&self) -> Option<WebBundleInjector> {
        self.web_injector.clone()
    }

    /// Accessor for the active service tracker.
    #[must_use]
    pub fn service_tracker(&self) -> crate::service_tracker::ServiceTracker {
        self.service_tracker.clone()
    }
}
