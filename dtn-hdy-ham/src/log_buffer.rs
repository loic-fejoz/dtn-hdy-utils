//! In-memory circular log buffer and tracing Layer for live web streaming.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::broadcast;
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// A single structured log record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    /// Monotonically increasing sequential ID
    pub id: u64,
    /// ISO 8601 / RFC 3339 formatted timestamp
    pub timestamp: String,
    /// Log severity level (TRACE, DEBUG, INFO, WARN, ERROR)
    pub level: String,
    /// Target module or component emitting the log
    pub target: String,
    /// Human-readable log message
    pub message: String,
}

static GLOBAL_LOG_BUFFER: OnceLock<Arc<LogBuffer>> = OnceLock::new();

/// Thread-safe in-memory circular buffer with live broadcast channel.
pub struct LogBuffer {
    max_capacity: usize,
    counter: AtomicU64,
    entries: RwLock<VecDeque<LogEntry>>,
    tx: broadcast::Sender<LogEntry>,
}

impl LogBuffer {
    /// Create a new log buffer instance with specified maximum capacity.
    pub fn new(max_capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(1024);
        Arc::new(Self {
            max_capacity: max_capacity.max(1),
            counter: AtomicU64::new(1),
            entries: RwLock::new(VecDeque::with_capacity(max_capacity.max(1))),
            tx,
        })
    }

    /// Initialize the global singleton log buffer. Returns existing if already initialized.
    pub fn init_global(max_capacity: usize) -> Arc<Self> {
        GLOBAL_LOG_BUFFER
            .get_or_init(|| Self::new(max_capacity))
            .clone()
    }

    /// Access the global log buffer (or initializes a default 500-capacity instance).
    pub fn global() -> Arc<Self> {
        Self::init_global(500)
    }

    /// Inserts a new log entry, discarding the oldest if at capacity, and broadcasts to subscribers.
    pub fn push(&self, level: String, target: String, message: String) -> LogEntry {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        let timestamp = now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| now.to_string());

        let entry = LogEntry {
            id,
            timestamp,
            level,
            target,
            message,
        };

        {
            let mut lock = self.entries.write().unwrap_or_else(|e| e.into_inner());
            if lock.len() >= self.max_capacity {
                lock.pop_front();
            }
            lock.push_back(entry.clone());
        }

        let _ = self.tx.send(entry.clone());
        entry
    }

    /// Returns a snapshot of all stored entries in chronological order.
    pub fn list_entries(&self) -> Vec<LogEntry> {
        let lock = self.entries.read().unwrap_or_else(|e| e.into_inner());
        lock.iter().cloned().collect()
    }

    /// Returns the current number of buffered entries.
    pub fn count(&self) -> usize {
        let lock = self.entries.read().unwrap_or_else(|e| e.into_inner());
        lock.len()
    }

    /// Returns the maximum capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.max_capacity
    }

    /// Subscribe to real-time incoming log entries.
    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.tx.subscribe()
    }

    /// Clear all buffered entries.
    pub fn clear(&self) {
        let mut lock = self.entries.write().unwrap_or_else(|e| e.into_inner());
        lock.clear();
    }
}

/// Tracing subscriber Layer that captures events into a `LogBuffer`.
pub struct LogBufferLayer {
    buffer: Arc<LogBuffer>,
}

impl LogBufferLayer {
    pub fn new(buffer: Arc<LogBuffer>) -> Self {
        Self { buffer }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let mut s = format!("{value:?}");
            if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
                s = s[1..s.len() - 1].to_string();
            }
            self.message = s;
        } else {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }
}

impl<S> Layer<S> for LogBufferLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let level = meta.level().to_string();
        let target = meta.target().to_string();

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let mut msg = visitor.message;
        if msg.is_empty() && !visitor.fields.is_empty() {
            msg = visitor
                .fields
                .into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ");
        }

        self.buffer.push(level, target, msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer_push_and_capacity_eviction() {
        let buffer = LogBuffer::new(5);
        assert_eq!(buffer.capacity(), 5);
        assert_eq!(buffer.count(), 0);

        for i in 1..=7 {
            buffer.push(
                "INFO".to_string(),
                "test::module".to_string(),
                format!("Message {i}"),
            );
        }

        assert_eq!(buffer.count(), 5);
        let entries = buffer.list_entries();
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].message, "Message 3");
        assert_eq!(entries[4].message, "Message 7");
    }

    #[test]
    fn test_log_buffer_subscribe() {
        let buffer = LogBuffer::new(10);
        let mut rx = buffer.subscribe();

        let entry = buffer.push(
            "WARN".to_string(),
            "test::warn".to_string(),
            "Warning test".to_string(),
        );
        let received = rx.try_recv().expect("receives entry");
        assert_eq!(received.id, entry.id);
        assert_eq!(received.message, "Warning test");
    }
}
