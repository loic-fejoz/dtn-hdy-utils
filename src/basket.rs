use hardy_cbor::decode::{self, FromCbor, Value};
use hardy_cbor::encode::{self, Encoder, ToCbor};
use serde::Serialize;

pub mod coap_status {
    pub const CHANGED: u8 = 66; // 2.02 Changed
    pub const CONTENT: u8 = 69; // 2.05 Content
    pub const BAD_REQUEST: u8 = 128; // 4.00 Bad Request
    pub const FORBIDDEN: u8 = 131; // 4.03 Forbidden
    pub const NOT_FOUND: u8 = 132; // 4.04 Not Found
    pub const NOT_ACCEPTABLE: u8 = 134; // 4.06 Not Acceptable
    pub const REQUEST_ENTITY_TOO_LARGE: u8 = 141; // 4.13 Request Entity Too Large
    pub const INTERNAL_SERVER_ERROR: u8 = 160; // 5.00 Internal Server Error
}

/// Maximum recursion depth allowed when skipping unknown/unsupported CBOR values.
/// This prevents malicious deeply nested inputs from causing stack overflow during deserialization.
const CBOR_SKIP_DEPTH_LIMIT: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestItem {
    pub op: u64, // default 0 (0=GET, 1=CHECK, 2=SEARCH, 3=CANCEL, 4=LIST)
    pub uri: String,
    pub max_size: Option<u64>,
    pub accepted_formats: Option<Vec<String>>,
    #[serde(serialize_with = "serialize_bytes_opt_vec")]
    pub have_hashes: Option<Vec<Vec<u8>>>,
    pub if_modified_since: Option<u64>,
    pub lifetime_override: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BasketRequest {
    pub experiment_tag: Option<u64>,
    pub version: u64,
    pub req_id: String,
    pub reply_to: Option<String>,
    pub default_lifetime: Option<u64>,
    pub items: Vec<RequestItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemMetadata {
    #[serde(serialize_with = "serialize_bytes")]
    pub hash: Vec<u8>,
    pub size: Option<u64>,
    pub mime_type: Option<String>,
    pub uri: Option<String>,
    pub last_modified: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemResponse {
    pub item_idx: u64,
    pub coap_status: u8,
    pub metadata: Option<ItemMetadata>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BasketResponse {
    pub experiment_tag: Option<u64>,
    pub version: u64,
    pub req_id: String,
    pub items: Vec<ItemResponse>,
}

fn serialize_bytes<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&hex::encode(bytes))
}

fn serialize_bytes_opt_vec<S>(
    opt_vec: &Option<Vec<Vec<u8>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match opt_vec {
        Some(vec) => {
            use serde::ser::SerializeSeq;
            let mut seq = serializer.serialize_seq(Some(vec.len()))?;
            for val in vec {
                seq.serialize_element(&hex::encode(val))?;
            }
            seq.end()
        }
        None => serializer.serialize_none(),
    }
}

// CBOR decoding helpers

pub struct CborBytes(pub Vec<u8>);

impl FromCbor for CborBytes {
    type Error = decode::Error;

    fn from_cbor(data: &[u8]) -> Result<(Self, bool, usize), Self::Error> {
        decode::parse_value(data, |v, shortest, tags| {
            if !tags.is_empty() {
                return Err(decode::Error::IncorrectType(
                    "Untagged Bytes".to_string(),
                    "Tagged".to_string(),
                ));
            }
            match v {
                Value::Bytes(r) => Ok((CborBytes(data[r].to_vec()), shortest)),
                Value::ByteStream(chunks) => {
                    let mut bytes = Vec::new();
                    for r in chunks {
                        bytes.extend_from_slice(&data[r]);
                    }
                    Ok((CborBytes(bytes), shortest))
                }
                _ => Err(decode::Error::IncorrectType(
                    "Bytes".to_string(),
                    v.type_name(false),
                )),
            }
        })
        .map(|((v, s), len)| (v, s, len))
    }
}

fn parse_cbor_text_value(
    v: &Value,
    shortest: bool,
    tags: &[u64],
) -> Result<(String, bool), decode::Error> {
    if !tags.is_empty() {
        return Err(decode::Error::IncorrectType(
            "Untagged Text".to_string(),
            "Tagged".to_string(),
        ));
    }
    match v {
        Value::Text(t) => Ok((t.to_string(), shortest)),
        Value::TextStream(parts) => {
            let combined = parts.iter().fold(String::new(), |mut acc, s| {
                acc.push_str(s);
                acc
            });
            Ok((combined, shortest))
        }
        _ => Err(decode::Error::IncorrectType(
            "Text".to_string(),
            v.type_name(!tags.is_empty()),
        )),
    }
}

pub struct CborString(pub String);

impl FromCbor for CborString {
    type Error = decode::Error;

    fn from_cbor(data: &[u8]) -> Result<(Self, bool, usize), Self::Error> {
        decode::parse_value(data, |v, shortest, tags| {
            parse_cbor_text_value(&v, shortest, tags).map(|(s, shortest)| (CborString(s), shortest))
        })
        .map(|((v, s), len)| (v, s, len))
    }
}

pub struct CborFormat(pub String);

impl FromCbor for CborFormat {
    type Error = decode::Error;

    fn from_cbor(data: &[u8]) -> Result<(Self, bool, usize), Self::Error> {
        decode::parse_value(data, |v, shortest, _tags| match v {
            Value::UnsignedInteger(n) => Ok((CborFormat(n.to_string()), shortest)),
            Value::Text(t) => Ok((CborFormat(t.to_string()), shortest)),
            _ => Err(decode::Error::IncorrectType(
                "Uint or Text".to_string(),
                v.type_name(false),
            )),
        })
        .map(|((v, s), len)| (v, s, len))
    }
}

pub struct CborReqId(pub String);

impl FromCbor for CborReqId {
    type Error = decode::Error;

    fn from_cbor(data: &[u8]) -> Result<(Self, bool, usize), Self::Error> {
        decode::parse_value(data, |v, shortest, tags| match v {
            Value::Text(_) | Value::TextStream(_) => parse_cbor_text_value(&v, shortest, tags)
                .map(|(s, shortest)| (CborReqId(s), shortest)),
            Value::Bytes(r) => Ok((CborReqId(hex::encode(&data[r.clone()])), shortest)),
            Value::ByteStream(chunks) => {
                let mut bytes = Vec::new();
                for r in chunks {
                    bytes.extend_from_slice(&data[r.clone()]);
                }
                Ok((CborReqId(hex::encode(&bytes)), shortest))
            }
            _ => Err(decode::Error::IncorrectType(
                "Text or Bytes".to_string(),
                v.type_name(!tags.is_empty()),
            )),
        })
        .map(|((v, s), len)| (v, s, len))
    }
}

impl FromCbor for RequestItem {
    type Error = decode::Error;

    fn from_cbor(data: &[u8]) -> Result<(Self, bool, usize), Self::Error> {
        decode::parse_map(data, |m, shortest, _tags| {
            let mut op = 0;
            let mut uri = String::new();
            let mut max_size = None;
            let mut accepted_formats = None;
            let mut have_hashes = None;
            let mut if_modified_since = None;
            let mut lifetime_override = None;

            while !m.at_end()? {
                let key = m.parse::<u64>()?;
                match key {
                    0 => {
                        op = m.parse::<u64>()?;
                    }
                    1 => {
                        uri = m.parse::<CborString>()?.0;
                    }
                    2 => {
                        max_size = Some(m.parse::<u64>()?);
                    }
                    3 => {
                        let mut formats = Vec::new();
                        m.parse_value(|val, _, _| match val {
                            Value::Array(arr) => {
                                while !arr.at_end()? {
                                    let fmt = arr.parse::<CborFormat>()?.0;
                                    formats.push(fmt);
                                }
                                Ok(())
                            }
                            _ => Err(decode::Error::IncorrectType(
                                "Array".to_string(),
                                val.type_name(false),
                            )),
                        })?;
                        accepted_formats = Some(formats);
                    }
                    4 => {
                        let mut hashes = Vec::new();
                        m.parse_value(|val, _, _| match val {
                            Value::Array(arr) => {
                                while !arr.at_end()? {
                                    let hash = arr.parse::<CborBytes>()?.0;
                                    hashes.push(hash);
                                }
                                Ok(())
                            }
                            _ => Err(decode::Error::IncorrectType(
                                "Array".to_string(),
                                val.type_name(false),
                            )),
                        })?;
                        have_hashes = Some(hashes);
                    }
                    5 => {
                        if_modified_since = Some(m.parse::<u64>()?);
                    }
                    6 => {
                        lifetime_override = Some(m.parse::<u64>()?);
                    }
                    _ => {
                        m.skip_value(CBOR_SKIP_DEPTH_LIMIT)?;
                    }
                }
            }

            Ok((
                RequestItem {
                    op,
                    uri,
                    max_size,
                    accepted_formats,
                    have_hashes,
                    if_modified_since,
                    lifetime_override,
                },
                shortest,
            ))
        })
        .map(|((v, s), len)| (v, s, len))
    }
}

impl FromCbor for BasketRequest {
    type Error = decode::Error;

    fn from_cbor(data: &[u8]) -> Result<(Self, bool, usize), Self::Error> {
        decode::parse_map(data, |m, shortest, tags| {
            let experiment_tag = if tags.contains(&44444) {
                Some(44444)
            } else {
                None
            };
            let mut version = None;
            let mut req_id = None;
            let mut reply_to = None;
            let mut default_lifetime = None;
            let mut items = Vec::new();

            while !m.at_end()? {
                let key = m.parse::<i64>()?;
                match key {
                    -1 => {
                        let _val = m.parse::<u64>()?;
                    }
                    0 => {
                        version = Some(m.parse::<u64>()?);
                    }
                    1 => {
                        req_id = Some(m.parse::<CborReqId>()?.0);
                    }
                    2 => {
                        reply_to = Some(m.parse::<CborString>()?.0);
                    }
                    3 => {
                        default_lifetime = Some(m.parse::<u64>()?);
                    }
                    4 => {
                        m.parse_value(|val, _, _| match val {
                            Value::Array(arr) => {
                                while !arr.at_end()? {
                                    let item = arr.parse::<RequestItem>()?;
                                    items.push(item);
                                }
                                Ok(())
                            }
                            _ => Err(decode::Error::IncorrectType(
                                "Array".to_string(),
                                val.type_name(false),
                            )),
                        })?;
                    }
                    _ => {
                        m.skip_value(CBOR_SKIP_DEPTH_LIMIT)?;
                    }
                }
            }

            let version = version.ok_or_else(|| {
                decode::Error::IncorrectType("version field".to_string(), "missing".to_string())
            })?;
            let req_id = req_id.ok_or_else(|| {
                decode::Error::IncorrectType("req_id field".to_string(), "missing".to_string())
            })?;

            Ok((
                BasketRequest {
                    experiment_tag,
                    version,
                    req_id,
                    reply_to,
                    default_lifetime,
                    items,
                },
                shortest,
            ))
        })
        .map(|((v, s), len)| (v, s, len))
    }
}

// CBOR encoding implementations

impl ToCbor for RequestItem {
    type Result = ();

    fn to_cbor(&self, encoder: &mut Encoder) -> Self::Result {
        let mut count = 2; // op and uri are required
        if self.max_size.is_some() {
            count += 1;
        }
        if self.accepted_formats.is_some() {
            count += 1;
        }
        if self.have_hashes.is_some() {
            count += 1;
        }
        if self.if_modified_since.is_some() {
            count += 1;
        }
        if self.lifetime_override.is_some() {
            count += 1;
        }

        encoder.emit_map(Some(count), |m| {
            m.emit(&0u64);
            m.emit(&self.op);

            m.emit(&1u64);
            m.emit(self.uri.as_str());

            if let Some(max_size) = self.max_size {
                m.emit(&2u64);
                m.emit(&max_size);
            }
            if let Some(ref formats) = self.accepted_formats {
                m.emit(&3u64);
                m.emit_array(Some(formats.len()), |a| {
                    for fmt in formats {
                        a.emit(fmt.as_str());
                    }
                });
            }
            if let Some(ref hashes) = self.have_hashes {
                m.emit(&4u64);
                m.emit_array(Some(hashes.len()), |a| {
                    for hash in hashes {
                        a.emit(&encode::Bytes(hash));
                    }
                });
            }
            if let Some(if_modified) = self.if_modified_since {
                m.emit(&5u64);
                m.emit(&if_modified);
            }
            if let Some(lifetime) = self.lifetime_override {
                m.emit(&6u64);
                m.emit(&lifetime);
            }
        });
    }
}

impl ToCbor for BasketRequest {
    type Result = ();

    fn to_cbor(&self, encoder: &mut Encoder) -> Self::Result {
        let mut count = 3; // version, req_id, items are required
        if self.experiment_tag.is_some() {
            count += 1;
        }
        if self.reply_to.is_some() {
            count += 1;
        }
        if self.default_lifetime.is_some() {
            count += 1;
        }

        encoder.emit_map(Some(count), |m| {
            if let Some(tag) = self.experiment_tag {
                m.emit(&-1i64);
                m.emit(&tag);
            }
            m.emit(&0u64);
            m.emit(&self.version);

            m.emit(&1u64);
            m.emit(self.req_id.as_str());

            if let Some(ref reply) = self.reply_to {
                m.emit(&2u64);
                m.emit(reply.as_str());
            }
            if let Some(lifetime) = self.default_lifetime {
                m.emit(&3u64);
                m.emit(&lifetime);
            }
            m.emit(&4u64);
            m.emit_array(Some(self.items.len()), |a| {
                for item in &self.items {
                    a.emit(item);
                }
            });
        });
    }
}

impl ToCbor for ItemMetadata {
    type Result = ();

    fn to_cbor(&self, encoder: &mut Encoder) -> Self::Result {
        let mut count = 1; // hash is required
        if self.size.is_some() {
            count += 1;
        }
        if self.mime_type.is_some() {
            count += 1;
        }
        if self.uri.is_some() {
            count += 1;
        }
        if self.last_modified.is_some() {
            count += 1;
        }

        encoder.emit_map(Some(count), |m| {
            m.emit(&0u64);
            m.emit(&encode::Bytes(&self.hash));

            if let Some(size) = self.size {
                m.emit(&1u64);
                m.emit(&size);
            }
            if let Some(ref mime) = self.mime_type {
                m.emit(&2u64);
                m.emit(mime.as_str());
            }
            if let Some(ref uri) = self.uri {
                m.emit(&3u64);
                m.emit(uri.as_str());
            }
            if let Some(last_modified) = self.last_modified {
                m.emit(&4u64);
                m.emit(&last_modified);
            }
        });
    }
}

impl ToCbor for ItemResponse {
    type Result = ();

    fn to_cbor(&self, encoder: &mut Encoder) -> Self::Result {
        let mut count = 2; // item_idx and coap_status are required
        if self.metadata.is_some() {
            count += 1;
        }
        if self.diagnostic.is_some() {
            count += 1;
        }

        encoder.emit_map(Some(count), |m| {
            m.emit(&0u64);
            m.emit(&self.item_idx);

            m.emit(&1u64);
            m.emit(&(self.coap_status as u64));

            if let Some(ref metadata) = self.metadata {
                m.emit(&2u64);
                m.emit(metadata);
            }
            if let Some(ref diagnostic) = self.diagnostic {
                m.emit(&3u64);
                m.emit(diagnostic.as_str());
            }
        });
    }
}

impl ToCbor for BasketResponse {
    type Result = ();

    fn to_cbor(&self, encoder: &mut Encoder) -> Self::Result {
        let mut count = 3; // version, req_id, and items are required
        if self.experiment_tag.is_some() {
            count += 1;
        }

        encoder.emit_map(Some(count), |m| {
            if let Some(tag) = self.experiment_tag {
                m.emit(&-1i64);
                m.emit(&tag);
            }
            m.emit(&0u64);
            m.emit(&self.version);

            m.emit(&1u64);
            m.emit(self.req_id.as_str());

            m.emit(&2u64);
            m.emit_array(Some(self.items.len()), |a| {
                for item in &self.items {
                    a.emit(item);
                }
            });
        });
    }
}

impl FromCbor for ItemMetadata {
    type Error = decode::Error;

    fn from_cbor(data: &[u8]) -> Result<(Self, bool, usize), Self::Error> {
        decode::parse_map(data, |m, shortest, _tags| {
            let mut hash = Vec::new();
            let mut size = None;
            let mut mime_type = None;
            let mut uri = None;
            let mut last_modified = None;

            while !m.at_end()? {
                let key = m.parse::<u64>()?;
                match key {
                    0 => {
                        hash = m.parse::<CborBytes>()?.0;
                    }
                    1 => {
                        size = Some(m.parse::<u64>()?);
                    }
                    2 => {
                        mime_type = Some(m.parse::<CborString>()?.0);
                    }
                    3 => {
                        uri = Some(m.parse::<CborString>()?.0);
                    }
                    4 => {
                        last_modified = Some(m.parse::<u64>()?);
                    }
                    _ => {
                        m.skip_value(CBOR_SKIP_DEPTH_LIMIT)?;
                    }
                }
            }

            Ok((
                ItemMetadata {
                    hash,
                    size,
                    mime_type,
                    uri,
                    last_modified,
                },
                shortest,
            ))
        })
        .map(|((v, s), len)| (v, s, len))
    }
}

impl FromCbor for ItemResponse {
    type Error = decode::Error;

    fn from_cbor(data: &[u8]) -> Result<(Self, bool, usize), Self::Error> {
        decode::parse_map(data, |m, shortest, _tags| {
            let mut item_idx = 0;
            let mut coap_status = 0;
            let mut metadata = None;
            let mut diagnostic = None;

            while !m.at_end()? {
                let key = m.parse::<u64>()?;
                match key {
                    0 => {
                        item_idx = m.parse::<u64>()?;
                    }
                    1 => {
                        let val = m.parse::<u64>()?;
                        coap_status = u8::try_from(val).map_err(|_| {
                            decode::Error::IncorrectType(
                                "u8 CoAP status".to_string(),
                                val.to_string(),
                            )
                        })?;
                    }
                    2 => {
                        metadata = Some(m.parse::<ItemMetadata>()?);
                    }
                    3 => {
                        diagnostic = Some(m.parse::<CborString>()?.0);
                    }
                    _ => {
                        m.skip_value(CBOR_SKIP_DEPTH_LIMIT)?;
                    }
                }
            }

            Ok((
                ItemResponse {
                    item_idx,
                    coap_status,
                    metadata,
                    diagnostic,
                },
                shortest,
            ))
        })
        .map(|((v, s), len)| (v, s, len))
    }
}

impl FromCbor for BasketResponse {
    type Error = decode::Error;

    fn from_cbor(data: &[u8]) -> Result<(Self, bool, usize), Self::Error> {
        decode::parse_map(data, |m, shortest, tags| {
            let experiment_tag = if tags.contains(&44444) {
                Some(44444)
            } else {
                None
            };
            let mut version = None;
            let mut req_id = None;
            let mut items = Vec::new();

            while !m.at_end()? {
                let key = m.parse::<i64>()?;
                match key {
                    -1 => {
                        let _val = m.parse::<u64>()?;
                    }
                    0 => {
                        version = Some(m.parse::<u64>()?);
                    }
                    1 => {
                        req_id = Some(m.parse::<CborReqId>()?.0);
                    }
                    2 => {
                        m.parse_value(|val, _, _| match val {
                            Value::Array(arr) => {
                                while !arr.at_end()? {
                                    let item = arr.parse::<ItemResponse>()?;
                                    items.push(item);
                                }
                                Ok(())
                            }
                            _ => Err(decode::Error::IncorrectType(
                                "Array".to_string(),
                                val.type_name(false),
                            )),
                        })?;
                    }
                    _ => {
                        m.skip_value(CBOR_SKIP_DEPTH_LIMIT)?;
                    }
                }
            }

            let version = version.ok_or_else(|| {
                decode::Error::IncorrectType("version field".to_string(), "missing".to_string())
            })?;
            let req_id = req_id.ok_or_else(|| {
                decode::Error::IncorrectType("req_id field".to_string(), "missing".to_string())
            })?;

            Ok((
                BasketResponse {
                    experiment_tag,
                    version,
                    req_id,
                    items,
                },
                shortest,
            ))
        })
        .map(|((v, s), len)| (v, s, len))
    }
}
