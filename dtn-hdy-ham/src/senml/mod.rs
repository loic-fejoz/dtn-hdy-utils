//! SenML IoT telemetry gateway, normalization, storage and Open MCT integration.

pub mod manager;
pub mod parser;
pub mod service;
pub mod storage;

pub use manager::SenmlManager;
pub use parser::{NormalizedSenmlRecord, parse_senml_payload};
pub use service::SenmlService;
pub use storage::{HistoryPoint, LatestMetricEntry, MetricEntry, SenmlStorage, TelemetryPoint};
