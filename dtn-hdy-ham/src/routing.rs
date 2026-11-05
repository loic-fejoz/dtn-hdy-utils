//! Dynamic static routing agent and manager for Hardy BPA.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use anyhow::Context;
use hardy_async::async_trait;
use hardy_bpa::routing::{RouteAction, RoutingAgent, RoutingSink};
use hardy_bpv7::eid::{Eid, NodeId};
use hardy_eid_patterns::EidPattern;
use tracing::{info, warn};

use crate::config::RouteConfig;
use crate::internal_storage::normalize_peer_node;

/// Dynamic static routing agent and manager for Hardy BPA.
pub struct RouteManager {
    routes: Arc<RwLock<Vec<RouteConfig>>>,
    sink: Arc<RwLock<Option<Box<dyn RoutingSink>>>>,
    config_path: Arc<RwLock<Option<PathBuf>>>,
}

impl RouteManager {
    /// Creates a new `RouteManager` with initial routes and optional configuration path.
    pub fn new(initial_routes: Vec<RouteConfig>, config_path: Option<PathBuf>) -> Self {
        Self {
            routes: Arc::new(RwLock::new(initial_routes)),
            sink: Arc::new(RwLock::new(None)),
            config_path: Arc::new(RwLock::new(config_path)),
        }
    }

    /// Sets or updates the configuration file path used for route persistence.
    pub fn set_config_path(&self, path: PathBuf) {
        if let Ok(mut p) = self.config_path.try_write() {
            *p = Some(path);
        }
    }

    /// Helper to convert a RouteConfig into a route action and priority.
    fn parse_route(route: &RouteConfig) -> anyhow::Result<(EidPattern, RouteAction, u32)> {
        let pattern_str = match route.pattern.as_str() {
            "dtn://**" => "dtn:**",
            "ipn://**" => "ipn:**",
            other => other,
        };
        let pattern: EidPattern = pattern_str
            .parse()
            .with_context(|| format!("Invalid EID route pattern '{}'", route.pattern))?;

        let via_eid = if let Some(ref hop) = route.next_hop {
            let normalized = if hop.starts_with("dtn:") || hop.starts_with("ipn:") {
                hop.clone()
            } else {
                normalize_peer_node(hop)
            };
            normalized
                .parse()
                .with_context(|| format!("Invalid next_hop EID '{}'", hop))?
        } else {
            Eid::Null
        };

        // Default routes (dtn:**, dtn://**, ipn:**, *:**) get lower priority (10)
        // so specific destinations take precedence (100).
        let priority = if route.pattern == "dtn://**"
            || route.pattern == "dtn:**"
            || route.pattern == "ipn://**"
            || route.pattern == "ipn:**"
            || route.pattern == "*:**"
        {
            10
        } else {
            100
        };

        Ok((pattern, RouteAction::Via(via_eid), priority))
    }

    /// Pushes a single route into the Hardy BPA RoutingSink.
    async fn push_route_to_sink(
        sink: &dyn RoutingSink,
        route: &RouteConfig,
    ) -> anyhow::Result<bool> {
        let (pattern, action, priority) = Self::parse_route(route)?;
        let inserted = sink
            .add_route(pattern, action, priority)
            .await
            .map_err(|e| anyhow::anyhow!("BPA rejected route {}: {e}", route.pattern))?;
        Ok(inserted)
    }

    /// Withdraws a single route from the Hardy BPA RoutingSink.
    async fn remove_route_from_sink(
        sink: &dyn RoutingSink,
        route: &RouteConfig,
    ) -> anyhow::Result<bool> {
        let (pattern, action, priority) = Self::parse_route(route)?;
        let removed = sink
            .remove_route(&pattern, &action, priority)
            .await
            .map_err(|e| anyhow::anyhow!("BPA rejected route removal {}: {e}", route.pattern))?;
        Ok(removed)
    }

    /// Returns a snapshot of all currently active static routes.
    pub async fn list_routes(&self) -> Vec<RouteConfig> {
        self.routes.read().await.clone()
    }

    /// Dynamically adds a new static route and pushes it immediately into the BPA RIB.
    pub async fn add_route(&self, route: RouteConfig) -> anyhow::Result<()> {
        // Validate route syntax first
        let _ = Self::parse_route(&route)?;

        // Immediately push to BPA RIB if registered
        if let Some(ref sink) = *self.sink.read().await {
            Self::push_route_to_sink(&**sink, &route).await?;
            info!(
                "Dynamic route added and pushed to BPA: pattern='{}', next_hop='{:?}', cla='{}'",
                route.pattern, route.next_hop, route.cla
            );
        } else {
            info!(
                "Route added to memory (BPA not yet registered): pattern='{}'",
                route.pattern
            );
        }

        // Add to active routes list (updating existing if matching pattern)
        {
            let mut list = self.routes.write().await;
            list.retain(|r| r.pattern != route.pattern);
            list.push(route);
        }

        // Persist to configuration file
        self.persist().await;

        Ok(())
    }

    /// Dynamically deletes a static route and withdraws it immediately from the BPA RIB.
    pub async fn delete_route(&self, pattern_str: &str) -> anyhow::Result<bool> {
        let mut list = self.routes.write().await;
        let Some(pos) = list.iter().position(|r| r.pattern == pattern_str) else {
            return Ok(false);
        };
        let route = list.remove(pos);
        drop(list);

        if let Some(ref sink) = *self.sink.read().await {
            if let Err(e) = Self::remove_route_from_sink(&**sink, &route).await {
                warn!("Failed to withdraw route {} from BPA RIB: {e}", pattern_str);
            } else {
                info!("Dynamic route withdrawn from BPA RIB: '{}'", pattern_str);
            }
        }

        self.persist().await;
        Ok(true)
    }

    /// Persists the current list of static routes to the YAML configuration file.
    async fn persist(&self) {
        let path_opt = self.config_path.read().await.clone();
        let Some(path) = path_opt else {
            return;
        };

        let routes = self.routes.read().await.clone();

        if let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(mut doc) = serde_yaml::from_str::<serde_yaml::Value>(&content)
            && let Ok(routes_val) = serde_yaml::to_value(&routes)
        {
            if let serde_yaml::Value::Mapping(ref mut map) = doc {
                map.insert(serde_yaml::Value::String("routes".to_string()), routes_val);
                if let Ok(new_yaml) = serde_yaml::to_string(&doc) {
                    if let Err(e) = std::fs::write(&path, new_yaml) {
                        warn!("Failed to persist routes to {}: {e}", path.display());
                    } else {
                        info!(
                            "Persisted {} static route(s) to configuration {}",
                            routes.len(),
                            path.display()
                        );
                    }
                }
            }
        }
    }
}

#[async_trait]
impl RoutingAgent for RouteManager {
    async fn on_register(&self, sink: Box<dyn RoutingSink>, _node_ids: &[NodeId]) {
        info!("HamRoutingAgent registered with Hardy BPA");
        let initial_routes = self.routes.read().await.clone();
        for r in &initial_routes {
            if let Err(e) = Self::push_route_to_sink(&*sink, r).await {
                warn!("Failed to install initial static route {}: {e}", r.pattern);
            } else {
                info!(
                    "Installed initial static route: pattern='{}', next_hop='{:?}'",
                    r.pattern, r.next_hop
                );
            }
        }
        *self.sink.write().await = Some(sink);
    }

    async fn on_unregister(&self) {
        info!("HamRoutingAgent unregistered from Hardy BPA");
        *self.sink.write().await = None;
    }
}
