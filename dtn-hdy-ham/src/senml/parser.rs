//! RFC 8428 SenML parsing and normalization for JSON and CBOR payloads.

use serde::{Deserialize, Serialize};

/// Normalized SenML record representing a single measurement point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedSenmlRecord {
    /// Full resolved metric name `(bn || "") + (n || "")`.
    pub name: String,
    /// Numeric measurement value (if present).
    pub value: Option<f64>,
    /// Textual / boolean measurement representation (if present, or formatted numeric).
    pub str_value: Option<String>,
    /// Engineering unit `u || bu || ""`.
    pub unit: Option<String>,
    /// Absolute POSIX timestamp in milliseconds.
    pub timestamp_ms: i64,
}

/// Parses a raw payload into normalized SenML records.
/// Automatically detects JSON (`application/senml+json`) or CBOR (`application/senml+cbor`).
/// Relative times ($t \le 2^{28}$) are anchored to `fallback_timestamp_ms` (bundle creation time).
pub fn parse_senml_payload(
    data: &[u8],
    fallback_timestamp_ms: i64,
) -> anyhow::Result<Vec<NormalizedSenmlRecord>> {
    let trimmed = match std::str::from_utf8(data) {
        Ok(s) => s.trim_start(),
        Err(_) => "",
    };

    if (trimmed.starts_with('[') || trimmed.starts_with('{'))
        && let Ok(records) = parse_json_senml(data, fallback_timestamp_ms)
        && !records.is_empty()
    {
        return Ok(records);
    }

    // Try CBOR decoding
    if let Ok(records) = parse_cbor_senml(data, fallback_timestamp_ms)
        && !records.is_empty()
    {
        return Ok(records);
    }

    // Fallback: try JSON if not tried yet
    if !trimmed.is_empty()
        && let Ok(records) = parse_json_senml(data, fallback_timestamp_ms)
    {
        return Ok(records);
    }

    anyhow::bail!("Payload could not be parsed as RFC 8428 SenML JSON or CBOR")
}

fn calculate_time_ms(time_sec: f64, fallback_timestamp_ms: i64) -> i64 {
    // RFC 8428 Section 4.5.3: Values > 2**28 (~1978) represent absolute seconds since 1970 UTC.
    // Values <= 2**28 represent relative seconds from base time / current time.
    if !time_sec.is_finite() {
        return fallback_timestamp_ms;
    }
    let delta = (time_sec * 1000.0).round();
    if !delta.is_finite() || delta > (i64::MAX as f64) || delta < (i64::MIN as f64) {
        return fallback_timestamp_ms;
    }
    let delta_ms = delta as i64;
    if time_sec > 268_435_456.0 {
        delta_ms
    } else {
        fallback_timestamp_ms.saturating_add(delta_ms)
    }
}

fn parse_json_senml(
    data: &[u8],
    fallback_timestamp_ms: i64,
) -> anyhow::Result<Vec<NormalizedSenmlRecord>> {
    let root: serde_json::Value = serde_json::from_slice(data)?;
    let array = match root {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(_) => vec![root],
        _ => anyhow::bail!("Expected JSON array or object for SenML"),
    };

    let mut records = Vec::new();
    let mut base_name = String::new();
    let mut base_time = 0.0f64;
    let mut base_unit: Option<String> = None;
    let mut base_value = 0.0f64;

    for item in array {
        let Some(map) = item.as_object() else {
            continue;
        };

        if let Some(bn) = map.get("bn").and_then(|v| v.as_str()) {
            base_name = bn.to_string();
        }
        if let Some(bt) = map.get("bt").and_then(|v| v.as_f64()) {
            base_time = bt;
        }
        if let Some(bu) = map.get("bu").and_then(|v| v.as_str()) {
            base_unit = Some(bu.to_string());
        }
        if let Some(bv) = map.get("bv").and_then(|v| v.as_f64()) {
            base_value = bv;
        }

        let n = map.get("n").and_then(|v| v.as_str());
        let full_name = format!("{}{}", base_name, n.unwrap_or_default());
        if full_name.is_empty() {
            continue;
        }

        let u = map
            .get("u")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| base_unit.clone());

        let t = map.get("t").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let timestamp_ms = calculate_time_ms(base_time + t, fallback_timestamp_ms);

        let (value, str_value) = if let Some(v) = map.get("v").and_then(|v| v.as_f64()) {
            let total_v = v + base_value;
            (Some(total_v), Some(format_float(total_v)))
        } else if let Some(vs) = map.get("vs").and_then(|v| v.as_str()) {
            (None, Some(vs.to_string()))
        } else if let Some(vb) = map.get("vb").and_then(|v| v.as_bool()) {
            (Some(if vb { 1.0 } else { 0.0 }), Some(vb.to_string()))
        } else if let Some(vd) = map.get("vd").and_then(|v| v.as_str()) {
            (None, Some(vd.to_string()))
        } else if n.is_some() {
            // Entry with name only (e.g. metadata ping or 0 value)
            (Some(base_value), Some(format_float(base_value)))
        } else {
            continue;
        };

        records.push(NormalizedSenmlRecord {
            name: full_name,
            value,
            str_value,
            unit: u,
            timestamp_ms,
        });
    }

    Ok(records)
}

fn format_float(val: f64) -> String {
    if (val.fract() == 0.0) && (val.abs() < 1e15) {
        format!("{}", val as i64)
    } else {
        format!("{val}")
    }
}

fn parse_cbor_senml(
    data: &[u8],
    fallback_timestamp_ms: i64,
) -> anyhow::Result<Vec<NormalizedSenmlRecord>> {
    let cbor_val: ciborium::value::Value = ciborium::from_reader(data)?;
    let items = match cbor_val {
        ciborium::value::Value::Array(arr) => arr,
        ciborium::value::Value::Map(_) => vec![cbor_val],
        _ => anyhow::bail!("Expected CBOR array or map for SenML"),
    };

    let mut records = Vec::new();
    let mut base_name = String::new();
    let mut base_time = 0.0f64;
    let mut base_unit: Option<String> = None;
    let mut base_value = 0.0f64;

    for item in items {
        let ciborium::value::Value::Map(entries) = item else {
            continue;
        };

        let get_str = |str_key: &str, int_key: i128| -> Option<String> {
            for (k, v) in &entries {
                let matches = match k {
                    ciborium::value::Value::Text(s) => s == str_key,
                    ciborium::value::Value::Integer(i) => {
                        let k_int: i128 = (*i).into();
                        k_int == int_key
                    }
                    _ => false,
                };
                if matches {
                    match v {
                        ciborium::value::Value::Text(s) => return Some(s.clone()),
                        ciborium::value::Value::Bytes(b) => {
                            return String::from_utf8(b.clone()).ok();
                        }
                        _ => return None,
                    }
                }
            }
            None
        };

        let get_f64 = |str_key: &str, int_key: i128| -> Option<f64> {
            for (k, v) in &entries {
                let matches = match k {
                    ciborium::value::Value::Text(s) => s == str_key,
                    ciborium::value::Value::Integer(i) => {
                        let k_int: i128 = (*i).into();
                        k_int == int_key
                    }
                    _ => false,
                };
                if matches {
                    match v {
                        ciborium::value::Value::Float(f) => return Some(*f),
                        ciborium::value::Value::Integer(i) => {
                            let val: i128 = (*i).into();
                            return Some(val as f64);
                        }
                        _ => return None,
                    }
                }
            }
            None
        };

        let get_bool = |str_key: &str, int_key: i128| -> Option<bool> {
            for (k, v) in &entries {
                let matches = match k {
                    ciborium::value::Value::Text(s) => s == str_key,
                    ciborium::value::Value::Integer(i) => {
                        let k_int: i128 = (*i).into();
                        k_int == int_key
                    }
                    _ => false,
                };
                if matches && let ciborium::value::Value::Bool(b) = v {
                    return Some(*b);
                }
            }
            None
        };

        // RFC 8428 Section 6 integer labels:
        // bn: -2, bt: -3, bu: -4, bv: -5
        if let Some(bn) = get_str("bn", -2) {
            base_name = bn;
        }
        if let Some(bt) = get_f64("bt", -3) {
            base_time = bt;
        }
        if let Some(bu) = get_str("bu", -4) {
            base_unit = Some(bu);
        }
        if let Some(bv) = get_f64("bv", -5) {
            base_value = bv;
        }

        // n: 0, u: 1, v: 2, vs: 3, vb: 4, vd: 8, t: 6
        let n = get_str("n", 0);
        let full_name = format!("{}{}", base_name, n.as_deref().unwrap_or_default());
        if full_name.is_empty() {
            continue;
        }

        let u = get_str("u", 1).or_else(|| base_unit.clone());
        let t = get_f64("t", 6).unwrap_or(0.0);
        let timestamp_ms = calculate_time_ms(base_time + t, fallback_timestamp_ms);

        let v_opt = get_f64("v", 2);
        let vs_opt = get_str("vs", 3);
        let vb_opt = get_bool("vb", 4);
        let vd_opt = get_str("vd", 8);

        let (value, str_value) = if let Some(v) = v_opt {
            let total = v + base_value;
            (Some(total), Some(format_float(total)))
        } else if let Some(vs) = vs_opt {
            (None, Some(vs))
        } else if let Some(vb) = vb_opt {
            (Some(if vb { 1.0 } else { 0.0 }), Some(vb.to_string()))
        } else if let Some(vd) = vd_opt {
            (None, Some(vd))
        } else if n.is_some() {
            (Some(base_value), Some(format_float(base_value)))
        } else {
            continue;
        };

        records.push(NormalizedSenmlRecord {
            name: full_name,
            value,
            str_value,
            unit: u,
            timestamp_ms,
        });
    }

    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_senml_pack() {
        let json_data = r#"[
            {"bn": "urn:dtn:node1:", "bt": 1731000000, "bu": "V", "n": "bus_voltage", "v": 27.9},
            {"n": "temp/cpu", "u": "Cel", "v": 42.5},
            {"n": "status/online", "vb": true},
            {"n": "state/mode", "vs": "STANDBY"}
        ]"#;

        let records = parse_senml_payload(json_data.as_bytes(), 1700000000000).unwrap();
        assert_eq!(records.len(), 4);

        assert_eq!(records[0].name, "urn:dtn:node1:bus_voltage");
        assert_eq!(records[0].value, Some(27.9));
        assert_eq!(records[0].unit.as_deref(), Some("V"));
        assert_eq!(records[0].timestamp_ms, 1731000000000);

        assert_eq!(records[1].name, "urn:dtn:node1:temp/cpu");
        assert_eq!(records[1].value, Some(42.5));
        assert_eq!(records[1].unit.as_deref(), Some("Cel"));
        assert_eq!(records[1].timestamp_ms, 1731000000000);

        assert_eq!(records[2].name, "urn:dtn:node1:status/online");
        assert_eq!(records[2].value, Some(1.0));
        assert_eq!(records[2].str_value.as_deref(), Some("true"));

        assert_eq!(records[3].name, "urn:dtn:node1:state/mode");
        assert_eq!(records[3].value, None);
        assert_eq!(records[3].str_value.as_deref(), Some("STANDBY"));
    }

    #[test]
    fn test_parse_relative_time_anchoring() {
        let json_data = r#"[
            {"bn": "urn:sensor:", "n": "hum", "v": 55.0, "t": -10}
        ]"#;

        let fallback_ms = 1731000000000i64;
        let records = parse_senml_payload(json_data.as_bytes(), fallback_ms).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].timestamp_ms, fallback_ms - 10000);
    }

    #[test]
    fn test_parse_cbor_senml_pack() {
        // Build a CBOR pack manually using ciborium
        let entries = vec![ciborium::value::Value::Map(vec![
            (
                ciborium::value::Value::Integer((-2).into()),
                ciborium::value::Value::Text("urn:cbor:station:".to_string()),
            ),
            (
                ciborium::value::Value::Integer((-3).into()),
                ciborium::value::Value::Float(1731000050.0),
            ),
            (
                ciborium::value::Value::Integer(0.into()),
                ciborium::value::Value::Text("solar_amps".to_string()),
            ),
            (
                ciborium::value::Value::Integer(1.into()),
                ciborium::value::Value::Text("A".to_string()),
            ),
            (
                ciborium::value::Value::Integer(2.into()),
                ciborium::value::Value::Float(4.8),
            ),
        ])];

        let mut cbor_bytes = Vec::new();
        ciborium::into_writer(&ciborium::value::Value::Array(entries), &mut cbor_bytes).unwrap();

        let records = parse_senml_payload(&cbor_bytes, 1700000000000).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "urn:cbor:station:solar_amps");
        assert_eq!(records[0].unit.as_deref(), Some("A"));
        assert_eq!(records[0].value, Some(4.8));
        assert_eq!(records[0].timestamp_ms, 1731000050000);
    }

    #[test]
    fn test_calculate_time_ms_resilience() {
        let fallback = 1_700_000_000_000;
        assert_eq!(calculate_time_ms(f64::NAN, fallback), fallback);
        assert_eq!(calculate_time_ms(f64::INFINITY, fallback), fallback);
        assert_eq!(calculate_time_ms(f64::NEG_INFINITY, fallback), fallback);
        assert_eq!(calculate_time_ms(f64::MAX, fallback), fallback);
        assert_eq!(calculate_time_ms(10.0, fallback), fallback + 10_000);
        assert_eq!(
            calculate_time_ms(1_731_000_000.0, fallback),
            1_731_000_000_000
        );
    }
}
