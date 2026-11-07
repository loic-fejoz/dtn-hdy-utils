//! SQLite persistence, dictionary caching and query engine for SenML telemetry.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use super::parser::NormalizedSenmlRecord;
use crate::config::DashboardMetricConfig;

/// Metadata dictionary entry for a known telemetry metric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricEntry {
    pub id: i64,
    pub name: String,
    pub unit: Option<String>,
    pub custom_label: Option<String>,
    pub display_order: Option<i64>,
    pub is_visible: bool,
}

/// Latest known metric point with metadata and values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestMetricEntry {
    pub id: i64,
    pub name: String,
    pub unit: Option<String>,
    pub custom_label: Option<String>,
    pub display_order: Option<i64>,
    pub is_visible: bool,
    pub utc: Option<i64>,
    pub value: Option<f64>,
    pub str_value: Option<String>,
}

/// Open MCT compatible historical telemetry point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPoint {
    pub utc: i64,
    pub value: serde_json::Value,
}

/// Real-time telemetry broadcast event payload for SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryPoint {
    pub id: String,
    pub utc: i64,
    pub value: serde_json::Value,
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// SQLite-backed SenML telemetry storage manager.
#[derive(Clone)]
pub struct SenmlStorage {
    conn: Arc<Mutex<Connection>>,
    cache: Arc<RwLock<HashMap<String, i64>>>,
    config_path: Arc<RwLock<Option<PathBuf>>>,
}

impl SenmlStorage {
    /// Opens or creates the SQLite database and initializes tables.
    pub fn new(db_path: Option<&Path>, config_path: Option<PathBuf>) -> Result<Self> {
        let conn = match db_path {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                Connection::open(path).with_context(|| {
                    format!("Failed to open SenML database at {}", path.display())
                })?
            }
            None => Connection::open_in_memory()
                .context("Failed to open in-memory SenML SQLite database")?,
        };

        // Initialize pragmas
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            "#,
        )
        .context("Failed to set SQLite PRAGMAs for SenML storage")?;

        // Create tables and index
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS metric_dictionary (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                unit TEXT,
                custom_label TEXT,
                display_order INTEGER,
                is_visible INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS telemetry_history (
                metric_id INTEGER NOT NULL,
                timestamp INTEGER NOT NULL,
                value REAL,
                str_value TEXT,
                ingest_time INTEGER NOT NULL,
                PRIMARY KEY (metric_id, timestamp) ON CONFLICT REPLACE,
                FOREIGN KEY (metric_id) REFERENCES metric_dictionary (id)
            ) WITHOUT ROWID;

            CREATE INDEX IF NOT EXISTS idx_telemetry_history 
            ON telemetry_history (metric_id, timestamp ASC);
            "#,
        )
        .context("Failed to create SenML SQLite tables")?;

        // Preload memory cache
        let mut cache = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT name, id FROM metric_dictionary")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let name: String = row.get(0)?;
                let id: i64 = row.get(1)?;
                cache.insert(name, id);
            }
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            cache: Arc::new(RwLock::new(cache)),
            config_path: Arc::new(RwLock::new(config_path)),
        })
    }

    /// Sets or updates the config file path for synchronization.
    pub fn set_config_path(&self, path: PathBuf) {
        if let Ok(mut p) = self.config_path.write() {
            *p = Some(path);
        }
    }

    /// Synchronizes initial configured dashboard metrics from YAML.
    pub fn sync_initial_config(&self, metrics: &[DashboardMetricConfig]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let mut cache = self.cache.write().unwrap();

        for (idx, m) in metrics.iter().enumerate() {
            let order = (idx + 1) as i64;
            conn.execute(
                r#"
                INSERT INTO metric_dictionary (name, custom_label, display_order, is_visible)
                VALUES (?1, ?2, ?3, 1)
                ON CONFLICT(name) DO UPDATE SET
                    custom_label = COALESCE(?2, custom_label),
                    display_order = ?3,
                    is_visible = 1;
                "#,
                params![m.name, m.label, order],
            )?;

            let id: i64 = conn.query_row(
                "SELECT id FROM metric_dictionary WHERE name = ?1",
                params![m.name],
                |row| row.get(0),
            )?;
            cache.insert(m.name.clone(), id);
        }

        Ok(())
    }

    /// Ingests a batch of normalized SenML records atomically in a single transaction.
    /// Returns the inserted telemetry points for real-time SSE broadcasting.
    pub fn insert_records(&self, records: &[NormalizedSenmlRecord]) -> Result<Vec<TelemetryPoint>> {
        if records.is_empty() {
            return Ok(Vec::new());
        }

        let now_ms = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let mut points_to_broadcast = Vec::new();

        for record in records {
            // 1. Resolve or insert metric_id via cache / dictionary
            let (metric_id, label, unit) = {
                let cache = self.cache.read().unwrap();
                if let Some(&id) = cache.get(&record.name) {
                    // Update unit if new unit provided and none stored
                    if let Some(ref u) = record.unit {
                        tx.execute(
                            "UPDATE metric_dictionary SET unit = ?1 WHERE id = ?2 AND (unit IS NULL OR unit = '')",
                            params![u, id],
                        )?;
                    }
                    let (custom_label, cur_unit): (Option<String>, Option<String>) = tx.query_row(
                        "SELECT custom_label, unit FROM metric_dictionary WHERE id = ?1",
                        params![id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )?;
                    (id, custom_label, cur_unit)
                } else {
                    drop(cache);
                    // Insert new discovered metric (hidden from dashboard by default)
                    tx.execute(
                        "INSERT OR IGNORE INTO metric_dictionary (name, unit, is_visible) VALUES (?1, ?2, 0)",
                        params![record.name, record.unit],
                    )?;
                    let (id, custom_label, cur_unit): (i64, Option<String>, Option<String>) = tx
                        .query_row(
                            "SELECT id, custom_label, unit FROM metric_dictionary WHERE name = ?1",
                            params![record.name],
                            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                        )?;
                    let mut cache_mut = self.cache.write().unwrap();
                    cache_mut.insert(record.name.clone(), id);
                    (id, custom_label, cur_unit)
                }
            };

            // 2. Insert into telemetry_history
            tx.execute(
                r#"
                INSERT INTO telemetry_history (metric_id, timestamp, value, str_value, ingest_time)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(metric_id, timestamp) DO UPDATE SET
                    value = excluded.value,
                    str_value = excluded.str_value,
                    ingest_time = excluded.ingest_time;
                "#,
                params![
                    metric_id,
                    record.timestamp_ms,
                    record.value,
                    record.str_value,
                    now_ms
                ],
            )?;

            let val_json = match (record.value, &record.str_value) {
                (Some(v), _) => serde_json::json!(v),
                (None, Some(s)) => serde_json::json!(s),
                _ => serde_json::json!(null),
            };

            points_to_broadcast.push(TelemetryPoint {
                id: record.name.clone(),
                utc: record.timestamp_ms,
                value: val_json,
                unit: record.unit.clone().or(unit),
                label,
            });
        }

        tx.commit()?;
        Ok(points_to_broadcast)
    }

    /// Lists all known metrics in the dictionary.
    pub fn list_metrics(&self) -> Result<Vec<MetricEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r#"
            SELECT id, name, unit, custom_label, display_order, is_visible
            FROM metric_dictionary
            ORDER BY COALESCE(display_order, 999999), name ASC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            let is_vis_int: i64 = row.get(5)?;
            Ok(MetricEntry {
                id: row.get(0)?,
                name: row.get(1)?,
                unit: row.get(2)?,
                custom_label: row.get(3)?,
                display_order: row.get(4)?,
                is_visible: is_vis_int != 0,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// Returns the latest known point for each metric (optionally filtered by name/id or dashboard_only).
    pub fn get_latest_metrics(
        &self,
        filter_ids: Option<&[String]>,
        dashboard_only: bool,
    ) -> Result<Vec<LatestMetricEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            r#"
            SELECT m.id, m.name, m.unit, m.custom_label, m.display_order, m.is_visible,
                   t.timestamp, t.value, t.str_value
            FROM metric_dictionary m
            LEFT JOIN telemetry_history t ON t.metric_id = m.id
                AND t.timestamp = (
                    SELECT MAX(sub.timestamp)
                    FROM telemetry_history sub
                    WHERE sub.metric_id = m.id
                )
            WHERE 1=1
            "#,
        );

        if dashboard_only {
            sql.push_str(" AND m.is_visible = 1 ");
        }

        sql.push_str(" ORDER BY COALESCE(m.display_order, 999999), m.name ASC");

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let is_vis_int: i64 = row.get(5)?;
            Ok(LatestMetricEntry {
                id: row.get(0)?,
                name: row.get(1)?,
                unit: row.get(2)?,
                custom_label: row.get(3)?,
                display_order: row.get(4)?,
                is_visible: is_vis_int != 0,
                utc: row.get(6)?,
                value: row.get(7)?,
                str_value: row.get(8)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            let entry = r?;
            if let Some(filters) = filter_ids
                && !filters.is_empty()
            {
                let id_str = entry.id.to_string();
                if !filters.iter().any(|f| f == &entry.name || f == &id_str) {
                    continue;
                }
            }
            list.push(entry);
        }
        Ok(list)
    }

    /// Queries historical telemetry points for a given metric between start and end (in ms).
    /// Returns points sorted chronologically (timestamp ASC) compatible with Open MCT.
    pub fn get_history(
        &self,
        metric_name_or_id: &str,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<HistoryPoint>> {
        let conn = self.conn.lock().unwrap();

        let metric_id: Option<i64> = if let Ok(num_id) = metric_name_or_id.parse::<i64>() {
            Some(num_id)
        } else {
            conn.query_row(
                "SELECT id FROM metric_dictionary WHERE name = ?1",
                params![metric_name_or_id],
                |row| row.get(0),
            )
            .ok()
        };

        let Some(m_id) = metric_id else {
            return Ok(Vec::new());
        };

        let mut stmt = conn.prepare(
            r#"
            SELECT timestamp, value, str_value
            FROM telemetry_history
            WHERE metric_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3
            ORDER BY timestamp ASC
            "#,
        )?;

        let rows = stmt.query_map(params![m_id, start_ms, end_ms], |row| {
            let utc: i64 = row.get(0)?;
            let v_num: Option<f64> = row.get(1)?;
            let v_str: Option<String> = row.get(2)?;

            let val_json = match (v_num, v_str) {
                (Some(v), _) => serde_json::json!(v),
                (None, Some(s)) => {
                    // Try parsing as boolean or number if stored as text
                    if let Ok(b) = s.parse::<bool>() {
                        serde_json::json!(b)
                    } else if let Ok(n) = s.parse::<f64>() {
                        serde_json::json!(n)
                    } else {
                        serde_json::json!(s)
                    }
                }
                _ => serde_json::json!(null),
            };

            Ok(HistoryPoint {
                utc,
                value: val_json,
            })
        })?;

        let mut history = Vec::new();
        for r in rows {
            history.push(r?);
        }
        Ok(history)
    }

    /// Adds a metric to the active dashboard.
    pub fn add_dashboard_card(&self, name: &str, label: Option<String>) -> Result<MetricEntry> {
        let conn = self.conn.lock().unwrap();

        let max_order: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(display_order), 0) FROM metric_dictionary WHERE is_visible = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let next_order = max_order + 1;

        conn.execute(
            r#"
            INSERT INTO metric_dictionary (name, custom_label, display_order, is_visible)
            VALUES (?1, ?2, ?3, 1)
            ON CONFLICT(name) DO UPDATE SET
                custom_label = COALESCE(?2, custom_label),
                display_order = ?3,
                is_visible = 1;
            "#,
            params![name, label, next_order],
        )?;

        let entry: MetricEntry = conn.query_row(
            "SELECT id, name, unit, custom_label, display_order, is_visible FROM metric_dictionary WHERE name = ?1",
            params![name],
            |row| {
                let vis: i64 = row.get(5)?;
                Ok(MetricEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    unit: row.get(2)?,
                    custom_label: row.get(3)?,
                    display_order: row.get(4)?,
                    is_visible: vis != 0,
                })
            },
        )?;

        drop(conn);
        self.persist_to_yaml_if_needed()?;
        Ok(entry)
    }

    /// Updates label and/or display order for a dashboard card.
    pub fn update_dashboard_card(
        &self,
        id: i64,
        label: Option<String>,
        order: Option<i64>,
    ) -> Result<MetricEntry> {
        let conn = self.conn.lock().unwrap();

        if let Some(ref l) = label {
            conn.execute(
                "UPDATE metric_dictionary SET custom_label = ?1 WHERE id = ?2",
                params![l, id],
            )?;
        }

        if let Some(o) = order {
            conn.execute(
                "UPDATE metric_dictionary SET display_order = ?1, is_visible = 1 WHERE id = ?2",
                params![o, id],
            )?;
        }

        let entry: MetricEntry = conn.query_row(
            "SELECT id, name, unit, custom_label, display_order, is_visible FROM metric_dictionary WHERE id = ?1",
            params![id],
            |row| {
                let vis: i64 = row.get(5)?;
                Ok(MetricEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    unit: row.get(2)?,
                    custom_label: row.get(3)?,
                    display_order: row.get(4)?,
                    is_visible: vis != 0,
                })
            },
        )?;

        drop(conn);
        self.persist_to_yaml_if_needed()?;
        Ok(entry)
    }

    /// Removes a metric card from the dashboard (sets `is_visible = 0`).
    pub fn remove_dashboard_card(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE metric_dictionary SET is_visible = 0, display_order = NULL WHERE id = ?1",
            params![id],
        )?;
        drop(conn);
        self.persist_to_yaml_if_needed()?;
        Ok(())
    }

    /// Clears all cards from the dashboard (sets `is_visible = 0` on all).
    pub fn clear_dashboard(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE metric_dictionary SET is_visible = 0, display_order = NULL",
            [],
        )?;
        drop(conn);
        self.persist_to_yaml_if_needed()?;
        Ok(())
    }

    /// Persists visible dashboard cards back to YAML config file if configured.
    fn persist_to_yaml_if_needed(&self) -> Result<()> {
        let cfg_path = {
            let p = self.config_path.read().unwrap();
            p.clone()
        };

        let Some(path) = cfg_path else {
            return Ok(());
        };

        if !path.exists() {
            return Ok(());
        }

        let latest = self.get_latest_metrics(None, true)?;
        let metric_configs: Vec<DashboardMetricConfig> = latest
            .into_iter()
            .map(|m| DashboardMetricConfig {
                name: m.name,
                label: m.custom_label,
            })
            .collect();

        let yaml_str = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };

        let mut val: serde_yaml::Value = match serde_yaml::from_str(&yaml_str) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };

        if let Some(map) = val.as_mapping_mut() {
            let senml_key = serde_yaml::Value::String("senml".to_string());
            let senml_val = serde_yaml::to_value(crate::config::SenmlConfig {
                metrics: metric_configs,
            })?;
            map.insert(senml_key, senml_val);

            let new_yaml = serde_yaml::to_string(&val)?;
            let tmp_path = path.with_extension("tmp");
            std::fs::write(&tmp_path, new_yaml)?;
            std::fs::rename(tmp_path, path)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_senml_storage_crud_and_idempotence() {
        let storage = SenmlStorage::new(None, None).unwrap();

        // 1. Initial config sync
        let initial_metrics = vec![
            DashboardMetricConfig {
                name: "urn:dtn:node1:power/bus_voltage".to_string(),
                label: Some("Tension Bus".to_string()),
            },
            DashboardMetricConfig {
                name: "urn:dtn:node1:temp/ambient".to_string(),
                label: Some("Temp. Ambiante".to_string()),
            },
        ];
        storage.sync_initial_config(&initial_metrics).unwrap();

        let metrics = storage.list_metrics().unwrap();
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].name, "urn:dtn:node1:power/bus_voltage");
        assert_eq!(metrics[0].custom_label.as_deref(), Some("Tension Bus"));
        assert_eq!(metrics[0].display_order, Some(1));
        assert!(metrics[0].is_visible);

        // 2. Insert records (including a new discovered metric)
        let records = vec![
            NormalizedSenmlRecord {
                name: "urn:dtn:node1:power/bus_voltage".to_string(),
                value: Some(27.8),
                str_value: Some("27.8".to_string()),
                unit: Some("V".to_string()),
                timestamp_ms: 1731000000000,
            },
            NormalizedSenmlRecord {
                name: "urn:dtn:node1:power/bus_voltage".to_string(),
                value: Some(27.9),
                str_value: Some("27.9".to_string()),
                unit: Some("V".to_string()),
                timestamp_ms: 1731000060000,
            },
            NormalizedSenmlRecord {
                name: "urn:dtn:node1:solar/watts".to_string(),
                value: Some(150.0),
                str_value: Some("150".to_string()),
                unit: Some("W".to_string()),
                timestamp_ms: 1731000060000,
            },
        ];

        let points = storage.insert_records(&records).unwrap();
        assert_eq!(points.len(), 3);

        // 3. Idempotence test: inserting same records again must NOT create duplicate rows
        let points2 = storage.insert_records(&records).unwrap();
        assert_eq!(points2.len(), 3);

        let history = storage
            .get_history("urn:dtn:node1:power/bus_voltage", 0, 2000000000000)
            .unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].utc, 1731000000000);
        assert_eq!(history[0].value, serde_json::json!(27.8));
        assert_eq!(history[1].utc, 1731000060000);
        assert_eq!(history[1].value, serde_json::json!(27.9));

        // 4. Latest metrics: verify latest values resolved correctly
        let latest = storage.get_latest_metrics(None, false).unwrap();
        assert_eq!(latest.len(), 3);
        let bus_v = latest
            .iter()
            .find(|m| m.name == "urn:dtn:node1:power/bus_voltage")
            .unwrap();
        assert_eq!(bus_v.utc, Some(1731000060000));
        assert_eq!(bus_v.value, Some(27.9));
        assert_eq!(bus_v.unit.as_deref(), Some("V"));

        // 5. Dashboard-only filtering: newly discovered solar metric is NOT visible on dashboard by default
        let dashboard_latest = storage.get_latest_metrics(None, true).unwrap();
        assert_eq!(dashboard_latest.len(), 2);
        assert!(
            !dashboard_latest
                .iter()
                .any(|m| m.name == "urn:dtn:node1:solar/watts")
        );

        // 6. Add solar to dashboard
        storage
            .add_dashboard_card(
                "urn:dtn:node1:solar/watts",
                Some("Panneau Solaire".to_string()),
            )
            .unwrap();
        let dashboard_after = storage.get_latest_metrics(None, true).unwrap();
        assert_eq!(dashboard_after.len(), 3);
    }
}
