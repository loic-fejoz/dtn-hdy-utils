//! Server initialization and lifecycle management for `dtn-hdy-ham`.

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;

use anyhow::Context;
use hardy_async::TaskPool;
use hardy_bpa::{
    bpa::Bpa,
    cla::{Cla, ClaInit},
    node_ids::NodeIds,
    storage::{BundleStorage, MetadataStorage},
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
    cla_manager: crate::cla_manager::ClaManager,
    chat_manager: crate::chat_manager::ChatManager,
    senml_manager: crate::senml::SenmlManager,
    route_manager: Arc<crate::routing::RouteManager>,
    keystore: Arc<crate::security::DirectionalKeyStore>,
    _ephemeral_dir: Option<tempfile::TempDir>,
    tasks: TaskPool,
}

impl HamServer {
    /// Constructs a new `HamServer` from the provided configuration.
    pub async fn new(mut config: HamConfig) -> anyhow::Result<Self> {
        let tasks = TaskPool::new();
        let cla_manager = crate::cla_manager::ClaManager::new();
        let mut ephemeral_dir = None;

        // 1. Storage setup
        let (metadata_storage, bundle_storage, chat_manager): (
            Arc<dyn MetadataStorage>,
            Arc<dyn BundleStorage>,
            crate::chat_manager::ChatManager,
        ) = if config.storage.in_memory {
            info!("Using ephemeral SQLite bundle and metadata storage for in-memory mode");
            let temp = tempfile::Builder::new()
                .prefix("dtn-ham-mem-")
                .tempdir()
                .context("Failed to create temporary directory for in-memory storage")?;
            let data_dir = temp.path().to_path_buf();
            let db_path = data_dir.join("bundles.db");
            let bundle_dir = data_dir.join("payloads");
            let _ = std::fs::create_dir_all(&data_dir);
            let _ = std::fs::create_dir_all(&bundle_dir);

            config.storage.data_dir = Some(data_dir.clone());
            config.storage.sqlite_path = Some(db_path);

            let meta_store = Arc::new(SqliteStorage::new(
                Some(data_dir),
                Some("bundles.db".to_string()),
                true,
            ));
            let bundle_store = Arc::new(LocalDiskStorage::new(Some(bundle_dir), Some(false), true));
            ephemeral_dir = Some(temp);
            let chat_mgr = crate::internal_storage::InternalStorage::new_in_memory_with_quota(
                config.application.quota.max_storage,
            )?;
            (meta_store, bundle_store, chat_mgr)
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

            let db_dir = Some(data_dir.clone());
            let db_name = db_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string());

            let meta_store = Arc::new(SqliteStorage::new(db_dir, db_name, true));
            let bundle_store = Arc::new(LocalDiskStorage::new(Some(bundle_dir), Some(false), true));
            let chat_mgr = crate::internal_storage::InternalStorage::new_disk_with_quota(
                &data_dir,
                config.application.quota.max_storage,
            )?;
            (meta_store, bundle_store, chat_mgr)
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

        let rfc9171 = config.rfc9171_validity.clone().unwrap_or_default();
        if let Some(enabled) = rfc9171.primary_block_integrity {
            builder = builder.primary_block_integrity(enabled);
        }
        if let Some(enabled) = rfc9171.bundle_age_required {
            builder = builder.bundle_age_required(enabled);
        }

        // 3. Register Dynamic Route Manager
        let route_manager = Arc::new(crate::routing::RouteManager::new(
            config.routes.clone(),
            config.config_path.clone(),
        ));
        builder = builder.routing_agent("ham_routes".to_string(), route_manager.clone());

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
                                Ok(addrs) => {
                                    for addr in addrs {
                                        resolved_peers.push(addr);
                                        cla_manager.register_tcp_peer(cla.clone(), addr).await;

                                        // Register ContactCoordinator for TCPCL if schedule configured
                                        if let Some(ref sched) = cla_config.schedule {
                                            match sched {
                                                ScheduleConfig::Recurring { cron, duration } => {
                                                    match crate::coordinator::cron::CronExpr::parse(
                                                        cron,
                                                    ) {
                                                        Ok(cron_expr) => {
                                                            let dur = *duration;
                                                            let endpoint = Arc::new(
                                                                crate::coordinator::endpoint::TcpclContactEndpoint::new(
                                                                    cla_manager.clone(),
                                                                    addr,
                                                                ),
                                                            );
                                                            let target = ContactTarget {
                                                                callsign: addr.to_string(),
                                                                node_id: None,
                                                                schedule:
                                                                    ContactSchedule::Recurring {
                                                                        cron: cron_expr,
                                                                        duration: dur,
                                                                    },
                                                                grace_period:
                                                                    std::time::Duration::from_secs(
                                                                        30,
                                                                    ),
                                                            };
                                                            let coordinator =
                                                                ContactCoordinator::new(
                                                                    endpoint,
                                                                    vec![target],
                                                                );
                                                            coordinators.push(coordinator);
                                                        }
                                                        Err(e) => {
                                                            warn!(
                                                                "Failed to parse recurring contact cron expression '{}' for TCPCL peer {}: {}",
                                                                cron, addr, e
                                                            );
                                                        }
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
                                                    let endpoint = Arc::new(
                                                        crate::coordinator::endpoint::TcpclContactEndpoint::new(
                                                            cla_manager.clone(),
                                                            addr,
                                                        ),
                                                    );
                                                    let target = ContactTarget {
                                                        callsign: addr.to_string(),
                                                        node_id: None,
                                                        schedule: ContactSchedule::OneShot {
                                                            start: start_dt,
                                                            end: end_dt,
                                                        },
                                                        grace_period:
                                                            std::time::Duration::from_secs(30),
                                                    };
                                                    let coordinator = ContactCoordinator::new(
                                                        endpoint,
                                                        vec![target],
                                                    );
                                                    coordinators.push(coordinator);
                                                }
                                            }
                                        }
                                    }
                                }
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
                                node_id: p.effective_node_id().parse().ok(),
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
                            let peer_node = match p.effective_node_id().parse() {
                                Ok(nid) => Some(nid),
                                Err(e) => {
                                    warn!(
                                        "Invalid peer node ID '{}' for peer '{}': {}",
                                        p.effective_node_id(),
                                        p.callsign,
                                        e
                                    );
                                    None
                                }
                            };
                            targets.push(ContactTarget {
                                callsign: p.callsign.clone(),
                                node_id: peer_node,
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

        // Keystore setup with 0600 file permissions
        let keystore = if config.storage.in_memory {
            Arc::new(crate::security::DirectionalKeyStore::empty())
        } else {
            let data_dir = config.storage.effective_data_dir();
            let keystore_path = data_dir.join("keystore.toml");
            Arc::new(crate::security::DirectionalKeyStore::load_or_init(
                &keystore_path,
            )?)
        };

        // SenML telemetry storage setup
        let senml_storage = if config.storage.in_memory {
            crate::senml::SenmlStorage::new(None, config.config_path.clone())?
        } else {
            let data_dir = config.storage.effective_data_dir();
            let senml_db_path = data_dir.join("senml.db");
            crate::senml::SenmlStorage::new(Some(&senml_db_path), config.config_path.clone())?
        };

        if let Some(ref senml_cfg) = config.senml {
            senml_storage.sync_initial_config(&senml_cfg.metrics)?;
        }

        let senml_manager = crate::senml::SenmlManager::new(senml_storage);

        Ok(Self {
            config,
            bpa,
            service_tracker,
            grpc_config,
            tcp_peers,
            coordinators,
            web_injector,
            cla_manager,
            chat_manager,
            senml_manager,
            route_manager,
            keystore,
            _ephemeral_dir: ephemeral_dir,
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

        // Register permanent built-in Chat service
        let chat_service = Arc::new(crate::chat_service::ChatService::new(
            self.chat_manager.clone(),
            self.service_tracker.clone(),
            self.keystore.clone(),
        ));
        let chat_service_id = hardy_bpv7::eid::Service::Dtn("chat".into());
        match self
            .service_tracker
            .register_service_with_type(chat_service_id, chat_service, "Built-in Chat")
            .await
        {
            Ok(eid) => {
                info!("Registered permanent built-in Chat service at EID: {}", eid);
            }
            Err(e) => {
                warn!("Failed to register built-in Chat service: {}", e);
            }
        }

        // Register permanent built-in Files service
        let files_service = Arc::new(crate::files_service::FilesService::new(
            self.chat_manager.clone(),
            self.service_tracker.clone(),
            self.keystore.clone(),
        ));
        let files_service_id = hardy_bpv7::eid::Service::Dtn("files".into());
        match self
            .service_tracker
            .register_service_with_type(files_service_id, files_service, "Built-in Files")
            .await
        {
            Ok(eid) => {
                info!(
                    "Registered permanent built-in Files service at EID: {}",
                    eid
                );
            }
            Err(e) => {
                warn!("Failed to register built-in Files service: {}", e);
            }
        }

        // Register permanent built-in SenML service
        let senml_service = Arc::new(crate::senml::SenmlService::new(
            self.senml_manager.clone(),
            self.service_tracker.clone(),
            self.keystore.clone(),
        ));
        let senml_service_id = hardy_bpv7::eid::Service::Dtn("senml".into());
        match self
            .service_tracker
            .register_service_with_type(senml_service_id, senml_service, "Built-in SenML")
            .await
        {
            Ok(eid) => {
                info!(
                    "Registered permanent built-in SenML service at EID: {}",
                    eid
                );
            }
            Err(e) => {
                warn!("Failed to register built-in SenML service: {}", e);
            }
        }

        info!(
            "Application storage quota active: max {} bytes (visual lifetime expiration enabled)",
            self.chat_manager.max_storage_bytes()
        );

        let coordinators = std::mem::take(&mut self.coordinators);
        for coordinator in coordinators {
            let cancel = self.tasks.cancel_token().clone();
            coordinator.spawn(cancel);
        }

        for (cla, addrs) in &self.tcp_peers {
            for peer_addr in addrs {
                let peer_addr = *peer_addr;
                let cla = cla.clone();
                let cla_manager = self.cla_manager.clone();
                let cancel = self.tasks.cancel_token().clone();
                hardy_async::spawn!(self.tasks, "tcpcl_peer_connector", async move {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let mut retry_delay = std::time::Duration::from_secs(1);
                    const MAX_STARTUP_RETRIES: usize = 5;
                    let mut attempts = 0;

                    loop {
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            res = cla.connect(&peer_addr) => {
                                match res {
                                    Ok(()) => {
                                        info!("Successfully connected to TCPCL peer {}", peer_addr);
                                        cla_manager
                                            .set_peer_status(
                                                peer_addr,
                                                crate::cla_manager::PeerConnectionStatus::Connected,
                                            )
                                            .await;
                                        break;
                                    }
                                    Err(e) => {
                                        attempts += 1;
                                        cla_manager
                                            .set_peer_status(
                                                peer_addr,
                                                crate::cla_manager::PeerConnectionStatus::Disconnected,
                                            )
                                            .await;
                                        if attempts >= MAX_STARTUP_RETRIES {
                                            tracing::info!(
                                                "TCPCL startup dialing to {} reached max attempts ({}); deferring to coordinator schedule.",
                                                peer_addr,
                                                attempts
                                            );
                                            break;
                                        }
                                        tracing::debug!(
                                            "TCPCL connect to {} failed ({}), retrying in {:?} (attempt {}/{})...",
                                            peer_addr,
                                            e,
                                            retry_delay,
                                            attempts,
                                            MAX_STARTUP_RETRIES
                                        );
                                    }
                                }
                            }
                        }
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            _ = tokio::time::sleep(retry_delay) => {
                                retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(15));
                            }
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
            let injector = self.web_injector.clone().ok_or_else(|| {
                anyhow::anyhow!("Web injector not initialized despite web config being enabled")
            })?;
            let state = AppState {
                config: Arc::new(self.config.clone()),
                injector,
                service_tracker: self.service_tracker.clone(),
                cla_manager: self.cla_manager.clone(),
                chat_manager: self.chat_manager.clone(),
                internal_storage: self.chat_manager.clone(),
                route_manager: self.route_manager.clone(),
                log_buffer: crate::log_buffer::LogBuffer::global(),
                keystore: self.keystore.clone(),
                senml_manager: self.senml_manager.clone(),
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

    /// Accessor for the dynamic route manager.
    #[must_use]
    pub fn route_manager(&self) -> Arc<crate::routing::RouteManager> {
        self.route_manager.clone()
    }

    /// Accessor for the active configuration.
    #[must_use]
    pub fn config(&self) -> &HamConfig {
        &self.config
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

    /// Accessor for the CLA manager.
    #[must_use]
    pub fn cla_manager(&self) -> crate::cla_manager::ClaManager {
        self.cla_manager.clone()
    }

    /// Accessor for the Chat manager.
    #[must_use]
    pub fn chat_manager(&self) -> crate::chat_manager::ChatManager {
        self.chat_manager.clone()
    }

    /// Accessor for the station internal storage.
    #[must_use]
    pub fn internal_storage(&self) -> crate::internal_storage::InternalStorage {
        self.chat_manager.clone()
    }

    /// Accessor for the station directional keystore.
    #[must_use]
    pub fn keystore(&self) -> Arc<crate::security::DirectionalKeyStore> {
        self.keystore.clone()
    }

    /// Accessor for the SenML telemetry manager.
    #[must_use]
    pub fn senml_manager(&self) -> crate::senml::SenmlManager {
        self.senml_manager.clone()
    }
}
