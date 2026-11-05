//! Active DTN service tracker and BpaRegistration interceptor.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use hardy_async::async_trait;
use hardy_bpa::{
    bpa::BpaRegistration,
    cla::{Cla, ClaInit},
    policy::FlowControllerFactory,
    routing::RoutingAgent,
    services::{self, Application, ApplicationSink, Service, ServiceSink, StatusNotify},
    stream::{Receiver, Segment},
};
use hardy_bpv7::{bundle::Id, eid::Eid, status_report::ReasonCode};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::broadcast;
use tracing::info;

/// Information about an active registered DTN service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceInfo {
    /// Canonical resolved EID (e.g. `dtn://f4jxq/echo` or `dtn://f4jxq/incoming`)
    pub eid: String,
    /// Type / Origin of the service (e.g. "Built-in Echo" or "gRPC Client")
    #[serde(rename = "type")]
    pub service_type: String,
    /// Registration date formatted as ISO 8601 with timezone offset
    pub registered_at: String,
}

/// Real-time service events broadcast over SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServiceEvent {
    #[serde(rename = "services_update")]
    ServicesUpdate { services: Vec<ServiceInfo> },
    #[serde(rename = "bundles_update")]
    BundlesUpdate,
    #[serde(rename = "chat_message")]
    ChatMessage { message: serde_json::Value },
}

/// Generates an ISO 8601 timestamp with local timezone offset (or UTC fallback).
fn now_iso() -> String {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| now.to_string())
}

struct InnerTracker {
    services: RwLock<BTreeMap<String, ServiceInfo>>,
    tx: broadcast::Sender<ServiceEvent>,
}

impl InnerTracker {
    fn list_services(&self) -> Vec<ServiceInfo> {
        let map = self.services.read().unwrap();
        map.values().cloned().collect()
    }

    fn record_service(&self, eid: &Eid, service_type: &str) {
        let eid_str = eid.to_string();
        info!("Registering active service: {} ({})", eid_str, service_type);
        let info = ServiceInfo {
            eid: eid_str.clone(),
            service_type: service_type.to_string(),
            registered_at: now_iso(),
        };

        {
            let mut map = self.services.write().unwrap();
            map.insert(eid_str, info);
        }

        self.broadcast_update();
    }

    fn remove_service(&self, eid: &Eid) {
        let eid_str = eid.to_string();
        info!("Unregistering active service: {}", eid_str);
        {
            let mut map = self.services.write().unwrap();
            map.remove(&eid_str);
        }

        self.broadcast_update();
    }

    fn broadcast_update(&self) {
        let current = self.list_services();
        let event = ServiceEvent::ServicesUpdate { services: current };
        let _ = self.tx.send(event);
    }
}

/// Central tracker for all active DTN services on the BPA.
#[derive(Clone)]
pub struct ServiceTracker {
    inner_bpa: Arc<dyn BpaRegistration>,
    tracker: Arc<InnerTracker>,
}

impl ServiceTracker {
    /// Creates a new `ServiceTracker` wrapping the given BPA registration provider.
    #[must_use]
    pub fn new(inner_bpa: Arc<dyn BpaRegistration>) -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            inner_bpa,
            tracker: Arc::new(InnerTracker {
                services: RwLock::new(BTreeMap::new()),
                tx,
            }),
        }
    }

    /// Returns a snapshot of all currently active services.
    #[must_use]
    pub fn list_services(&self) -> Vec<ServiceInfo> {
        self.tracker.list_services()
    }

    /// Subscribes to service change events (useful for SSE streams).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ServiceEvent> {
        self.tracker.tx.subscribe()
    }

    /// Broadcasts a bundles changed notification over SSE.
    pub fn broadcast_bundles_update(&self) {
        let _ = self.tracker.tx.send(ServiceEvent::BundlesUpdate);
    }

    /// Broadcasts an incoming or outgoing chat message event over SSE.
    pub fn broadcast_chat_message(&self, message: impl Serialize) {
        if let Ok(val) = serde_json::to_value(message) {
            let _ = self
                .tracker
                .tx
                .send(ServiceEvent::ChatMessage { message: val });
        }
    }

    /// Helper to register a service with a custom type label.
    pub async fn register_service_with_type(
        &self,
        service_id: hardy_bpv7::eid::Service,
        service: Arc<dyn Service>,
        service_type: impl Into<String>,
    ) -> services::Result<Eid> {
        let tracked = Arc::new(TrackedService {
            inner: service,
            service_type: service_type.into(),
            tracker: self.tracker.clone(),
            registered_eid: std::sync::Mutex::new(None),
        });

        self.inner_bpa.register_service(service_id, tracked).await
    }
}

// Low-level Service wrapper that intercepts on_register and on_unregister.
struct TrackedService {
    inner: Arc<dyn Service>,
    service_type: String,
    tracker: Arc<InnerTracker>,
    registered_eid: std::sync::Mutex<Option<Eid>>,
}

#[async_trait]
impl Service for TrackedService {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        *self.registered_eid.lock().unwrap() = Some(source.clone());
        self.tracker.record_service(source, &self.service_type);
        self.inner.on_register(source, sink).await;
    }

    async fn on_unregister(&self) {
        if let Some(eid) = self.registered_eid.lock().unwrap().take() {
            self.tracker.remove_service(&eid);
        }
        self.inner.on_unregister().await;
    }

    async fn on_deliver(
        &self,
        bundle_id: &Id,
        expiry: OffsetDateTime,
        total_len: u64,
        stream: &mut dyn Receiver<Segment>,
    ) -> services::Result<()> {
        self.inner
            .on_deliver(bundle_id, expiry, total_len, stream)
            .await
    }

    async fn on_status_notify(
        &self,
        bundle_id: &Id,
        from: &Eid,
        kind: StatusNotify,
        reason: ReasonCode,
        timestamp: Option<OffsetDateTime>,
    ) {
        self.inner
            .on_status_notify(bundle_id, from, kind, reason, timestamp)
            .await;
    }
}

// High-level Application wrapper that intercepts on_register and on_unregister.
struct TrackedApplication {
    inner: Arc<dyn Application>,
    service_type: String,
    tracker: Arc<InnerTracker>,
    registered_eid: std::sync::Mutex<Option<Eid>>,
}

#[async_trait]
impl Application for TrackedApplication {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ApplicationSink>) {
        *self.registered_eid.lock().unwrap() = Some(source.clone());
        self.tracker.record_service(source, &self.service_type);
        self.inner.on_register(source, sink).await;
    }

    async fn on_unregister(&self) {
        if let Some(eid) = self.registered_eid.lock().unwrap().take() {
            self.tracker.remove_service(&eid);
        }
        self.inner.on_unregister().await;
    }

    async fn on_deliver(
        &self,
        bundle_id: &Id,
        expiry: OffsetDateTime,
        ack_requested: bool,
        total_len: u64,
        stream: &mut dyn Receiver<Segment>,
    ) -> services::Result<()> {
        self.inner
            .on_deliver(bundle_id, expiry, ack_requested, total_len, stream)
            .await
    }

    async fn on_status_notify(
        &self,
        bundle_id: &Id,
        from: &Eid,
        kind: StatusNotify,
        reason: ReasonCode,
        timestamp: Option<OffsetDateTime>,
    ) {
        self.inner
            .on_status_notify(bundle_id, from, kind, reason, timestamp)
            .await;
    }
}

#[async_trait]
impl BpaRegistration for ServiceTracker {
    async fn register_cla(
        &self,
        name: String,
        cla: Arc<dyn Cla>,
        policy: Option<Arc<dyn FlowControllerFactory>>,
        init: ClaInit,
    ) -> hardy_bpa::cla::Result<Vec<hardy_bpv7::eid::NodeId>> {
        self.inner_bpa.register_cla(name, cla, policy, init).await
    }

    async fn register_service(
        &self,
        service_id: hardy_bpv7::eid::Service,
        service: Arc<dyn Service>,
    ) -> services::Result<Eid> {
        let tracked = Arc::new(TrackedService {
            inner: service,
            service_type: "gRPC Client".to_string(),
            tracker: self.tracker.clone(),
            registered_eid: std::sync::Mutex::new(None),
        });
        self.inner_bpa.register_service(service_id, tracked).await
    }

    async fn register_application(
        &self,
        service_id: hardy_bpv7::eid::Service,
        application: Arc<dyn Application>,
    ) -> services::Result<Eid> {
        let tracked = Arc::new(TrackedApplication {
            inner: application,
            service_type: "gRPC Client".to_string(),
            tracker: self.tracker.clone(),
            registered_eid: std::sync::Mutex::new(None),
        });
        self.inner_bpa
            .register_application(service_id, tracked)
            .await
    }

    async fn register_dynamic_service(&self, service: Arc<dyn Service>) -> services::Result<Eid> {
        let tracked = Arc::new(TrackedService {
            inner: service,
            service_type: "gRPC Client".to_string(),
            tracker: self.tracker.clone(),
            registered_eid: std::sync::Mutex::new(None),
        });
        self.inner_bpa.register_dynamic_service(tracked).await
    }

    async fn register_dynamic_application(
        &self,
        application: Arc<dyn Application>,
    ) -> services::Result<Eid> {
        let tracked = Arc::new(TrackedApplication {
            inner: application,
            service_type: "gRPC Client".to_string(),
            tracker: self.tracker.clone(),
            registered_eid: std::sync::Mutex::new(None),
        });
        self.inner_bpa.register_dynamic_application(tracked).await
    }

    async fn register_routing_agent(
        &self,
        name: String,
        agent: Arc<dyn RoutingAgent>,
    ) -> hardy_bpa::routing::Result<Vec<hardy_bpv7::eid::NodeId>> {
        self.inner_bpa.register_routing_agent(name, agent).await
    }
}
