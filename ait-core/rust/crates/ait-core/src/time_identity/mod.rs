use crate::json_support::{JsonMap, JsonValue};
use chrono::{DateTime, SecondsFormat, Utc};
use getrandom::getrandom;

use crate::json_support::JsonCodec;
use crate::shared_foundation::TimeIdentityProvider;
use crate::workflow_primitives::{generate_namespaced_sequence_id, workflow_id_token};

const CROCKFORD_BASE32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

pub struct TimeIdentityJson<S> {
    store: S,
}

impl<S> TimeIdentityJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl TimeIdentityJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> TimeIdentityJson<S> {
    pub fn normalize_timestamp_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan timestamp request")?;
        normalize_plan_timestamp_request_payload_map(payload)
    }

    pub fn normalize_timestamp_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan timestamp")?;
        normalize_plan_timestamp_payload_map(payload)
    }

    pub fn build_timestamp_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan timestamp build request")?;
        build_plan_timestamp_payload_map(payload)
    }

    pub fn normalize_sequence_identity_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan sequence identity request")?;
        normalize_plan_sequence_identity_request_payload_map(payload)
    }

    pub fn normalize_sequence_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan sequence identity")?;
        normalize_plan_sequence_identity_payload_map(payload)
    }

    pub fn build_sequence_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan sequence identity build request")?;
        build_plan_sequence_identity_payload_map(payload)
    }

    pub fn normalize_workflow_id_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan workflow id request")?;
        normalize_plan_workflow_id_request_payload_map(payload)
    }

    pub fn normalize_workflow_id_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan workflow id")?;
        normalize_plan_workflow_id_payload_map(payload)
    }

    pub fn build_workflow_id_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan workflow id build request")?;
        build_plan_workflow_id_payload_map(payload)
    }

    pub fn normalize_temporal_ordering_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "plan temporal ordering")?;
        normalize_plan_temporal_ordering_payload_map(payload)
    }

    pub fn build_temporal_ordering_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan temporal ordering build request")?;
        build_plan_temporal_ordering_payload_map(payload)
    }

    fn parse_object_payload(
        &self,
        payload_json: &str,
        label: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        JsonCodec::parse_object_with_error_prefix(
            payload_json,
            &format!("Failed to parse {label} JSON"),
            &format!("{label} payload must be an object."),
        )
        .map_err(String::from)
    }
}

#[derive(Default)]
pub struct TimeIdentityFoundation;

impl TimeIdentityProvider for TimeIdentityFoundation {
    fn normalize_timestamp_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        TimeIdentityJson::stateless().normalize_timestamp_request_payload_json(payload_json)
    }

    fn normalize_timestamp_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        TimeIdentityJson::stateless().normalize_timestamp_payload_json(payload_json)
    }

    fn build_timestamp_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        TimeIdentityJson::stateless().build_timestamp_payload_json(payload_json)
    }

    fn normalize_sequence_identity_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        TimeIdentityJson::stateless().normalize_sequence_identity_request_payload_json(payload_json)
    }

    fn normalize_sequence_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        TimeIdentityJson::stateless().normalize_sequence_identity_payload_json(payload_json)
    }

    fn build_sequence_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        TimeIdentityJson::stateless().build_sequence_identity_payload_json(payload_json)
    }

    fn normalize_workflow_id_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        TimeIdentityJson::stateless().normalize_workflow_id_request_payload_json(payload_json)
    }

    fn normalize_workflow_id_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        TimeIdentityJson::stateless().normalize_workflow_id_payload_json(payload_json)
    }

    fn build_workflow_id_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        TimeIdentityJson::stateless().build_workflow_id_payload_json(payload_json)
    }
}

pub fn normalize_plan_timestamp_request_with_time_identity_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: TimeIdentityProvider + ?Sized,
{
    provider.normalize_timestamp_request_payload_json(payload_json)
}

pub fn normalize_plan_timestamp_with_time_identity_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: TimeIdentityProvider + ?Sized,
{
    provider.normalize_timestamp_payload_json(payload_json)
}

pub fn build_plan_timestamp_with_time_identity_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: TimeIdentityProvider + ?Sized,
{
    provider.build_timestamp_payload_json(payload_json)
}

pub fn normalize_plan_sequence_identity_request_with_time_identity_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: TimeIdentityProvider + ?Sized,
{
    provider.normalize_sequence_identity_request_payload_json(payload_json)
}

pub fn normalize_plan_sequence_identity_with_time_identity_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: TimeIdentityProvider + ?Sized,
{
    provider.normalize_sequence_identity_payload_json(payload_json)
}

pub fn build_plan_sequence_identity_with_time_identity_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: TimeIdentityProvider + ?Sized,
{
    provider.build_sequence_identity_payload_json(payload_json)
}

pub fn normalize_plan_workflow_id_request_with_time_identity_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: TimeIdentityProvider + ?Sized,
{
    provider.normalize_workflow_id_request_payload_json(payload_json)
}

pub fn normalize_plan_workflow_id_with_time_identity_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: TimeIdentityProvider + ?Sized,
{
    provider.normalize_workflow_id_payload_json(payload_json)
}

pub fn build_plan_workflow_id_with_time_identity_provider<P>(
    provider: &P,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    P: TimeIdentityProvider + ?Sized,
{
    provider.build_workflow_id_payload_json(payload_json)
}

pub fn normalize_plan_timestamp_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().normalize_timestamp_request_payload_json(payload_json)
}

fn normalize_plan_timestamp_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    Ok(JsonValue::Object(JsonMap::from_iter([maybe_json_entry(
        "now",
        optional_text(payload.get("now"))?,
    )])))
}

pub fn normalize_plan_timestamp_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().normalize_timestamp_payload_json(payload_json)
}

fn normalize_plan_timestamp_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let timestamp = require_nonempty_text(payload.get("timestamp"), "timestamp")?;
    let source = require_nonempty_text(payload.get("source"), "source")?;
    if !matches!(source.as_str(), "system" | "injected") {
        return Err("Plan timestamp payload source must be `system` or `injected`.".to_string());
    }
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("timestamp".to_string(), JsonValue::String(timestamp)),
        ("source".to_string(), JsonValue::String(source)),
        (
            "epoch_ms".to_string(),
            JsonValue::from(require_nonnegative_i64(
                payload.get("epoch_ms"),
                "epoch_ms",
            )?),
        ),
    ])))
}

pub fn build_plan_timestamp_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().build_timestamp_payload_json(payload_json)
}

fn build_plan_timestamp_payload_map(
    request: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let now = optional_text(request.get("now"))?;
    let timestamp = now.clone().unwrap_or_else(current_timestamp_string);
    let epoch_ms = parse_iso_timestamp_to_epoch_ms(&timestamp)?;
    normalize_plan_timestamp_payload_map(JsonMap::from_iter([
        ("timestamp".to_string(), JsonValue::String(timestamp)),
        (
            "source".to_string(),
            JsonValue::String(if now.is_some() { "injected" } else { "system" }.to_string()),
        ),
        ("epoch_ms".to_string(), JsonValue::from(epoch_ms)),
    ]))
}

pub fn normalize_plan_sequence_identity_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().normalize_sequence_identity_request_payload_json(payload_json)
}

fn normalize_plan_sequence_identity_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "family".to_string(),
            JsonValue::String(require_nonempty_text(payload.get("family"), "family")?),
        ),
        (
            "number".to_string(),
            JsonValue::from(require_nonnegative_i64(payload.get("number"), "number")?),
        ),
        maybe_json_entry(
            "namespace_prefix",
            optional_text_allow_empty(payload.get("namespace_prefix"))?,
        ),
        (
            "width".to_string(),
            JsonValue::from(require_positive_i64(payload.get("width"), "width", 4)?),
        ),
    ])))
}

pub fn normalize_plan_sequence_identity_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().normalize_sequence_identity_payload_json(payload_json)
}

fn normalize_plan_sequence_identity_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let request = match normalize_plan_sequence_identity_request_payload_map(payload.clone())? {
        JsonValue::Object(map) => map,
        _ => return Err("Plan sequence identity normalization must return an object.".to_string()),
    };
    let generated_id = require_nonempty_text(payload.get("generated_id"), "generated_id")?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "family".to_string(),
            request.get("family").cloned().unwrap_or(JsonValue::Null),
        ),
        (
            "number".to_string(),
            request.get("number").cloned().unwrap_or(JsonValue::Null),
        ),
        (
            "namespace_prefix".to_string(),
            request
                .get("namespace_prefix")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "width".to_string(),
            request.get("width").cloned().unwrap_or(JsonValue::Null),
        ),
        ("generated_id".to_string(), JsonValue::String(generated_id)),
    ])))
}

pub fn build_plan_sequence_identity_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().build_sequence_identity_payload_json(payload_json)
}

fn build_plan_sequence_identity_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let request = match normalize_plan_sequence_identity_request_payload_map(payload)? {
        JsonValue::Object(map) => map,
        _ => {
            return Err(
                "Plan sequence identity request normalization must return an object.".to_string(),
            )
        }
    };
    let family = require_nonempty_text(request.get("family"), "family")?;
    let number = require_nonnegative_i64(request.get("number"), "number")?;
    let namespace_prefix = optional_text_allow_empty(request.get("namespace_prefix"))?;
    let width = require_positive_i64(request.get("width"), "width", 4)? as usize;
    let generated_id =
        generate_namespaced_sequence_id(&family, number, namespace_prefix.as_deref(), width)?;
    normalize_plan_sequence_identity_payload_map(JsonMap::from_iter([
        ("family".to_string(), JsonValue::String(family)),
        ("number".to_string(), JsonValue::from(number)),
        maybe_json_entry("namespace_prefix", namespace_prefix),
        ("width".to_string(), JsonValue::from(width as i64)),
        ("generated_id".to_string(), JsonValue::String(generated_id)),
    ]))
}

pub fn normalize_plan_workflow_id_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().normalize_workflow_id_request_payload_json(payload_json)
}

fn normalize_plan_workflow_id_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let timestamp_ms = match payload.get("timestamp_ms") {
        None | Some(JsonValue::Null) => None,
        Some(value) => Some(require_nonnegative_i64(Some(value), "timestamp_ms")?),
    };
    let randomness_hex =
        optional_text(payload.get("randomness_hex"))?.map(|value| value.to_lowercase());
    if let Some(text) = &randomness_hex {
        if text.len() != 20 || !text.chars().all(|char| char.is_ascii_hexdigit()) {
            return Err(
                "Plan workflow-id request randomness_hex must be exactly 20 hexadecimal characters."
                    .to_string(),
            );
        }
    }
    if timestamp_ms.is_some() ^ randomness_hex.is_some() {
        return Err(
            "Plan workflow-id request must provide timestamp_ms and randomness_hex together when overriding generation."
                .to_string(),
        );
    }
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "family".to_string(),
            JsonValue::String(require_nonempty_text(payload.get("family"), "family")?),
        ),
        maybe_json_entry(
            "namespace_prefix",
            optional_text_allow_empty(payload.get("namespace_prefix"))?,
        ),
        (
            "timestamp_ms".to_string(),
            timestamp_ms.map(JsonValue::from).unwrap_or(JsonValue::Null),
        ),
        maybe_json_entry("randomness_hex", randomness_hex),
    ])))
}

pub fn normalize_plan_workflow_id_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().normalize_workflow_id_payload_json(payload_json)
}

fn normalize_plan_workflow_id_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let request = match normalize_plan_workflow_id_request_payload_map(payload.clone())? {
        JsonValue::Object(map) => map,
        _ => return Err("Plan workflow-id normalization must return an object.".to_string()),
    };
    let generated_id = require_nonempty_text(payload.get("generated_id"), "generated_id")?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "family".to_string(),
            request.get("family").cloned().unwrap_or(JsonValue::Null),
        ),
        (
            "namespace_prefix".to_string(),
            request
                .get("namespace_prefix")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "timestamp_ms".to_string(),
            request
                .get("timestamp_ms")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "randomness_hex".to_string(),
            request
                .get("randomness_hex")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        ("generated_id".to_string(), JsonValue::String(generated_id)),
    ])))
}

pub fn build_plan_workflow_id_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().build_workflow_id_payload_json(payload_json)
}

fn build_plan_workflow_id_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let request = match normalize_plan_workflow_id_request_payload_map(payload)? {
        JsonValue::Object(map) => map,
        _ => {
            return Err("Plan workflow-id request normalization must return an object.".to_string())
        }
    };
    let family = require_nonempty_text(request.get("family"), "family")?;
    let namespace_prefix = optional_text_allow_empty(request.get("namespace_prefix"))?;
    let (timestamp_ms, randomness_hex) = match request.get("timestamp_ms") {
        Some(JsonValue::Number(number)) => {
            let timestamp_ms = number.as_i64().ok_or_else(|| {
                "Plan workflow-id request timestamp_ms must be an integer.".to_string()
            })?;
            let randomness_hex =
                require_nonempty_text(request.get("randomness_hex"), "randomness_hex")?;
            (timestamp_ms, randomness_hex)
        }
        _ => {
            let timestamp_ms = Utc::now().timestamp_millis();
            let mut randomness = [0u8; 10];
            getrandom(&mut randomness)
                .map_err(|exc| format!("Failed to generate workflow id randomness: {exc}"))?;
            (timestamp_ms, hex_string(&randomness))
        }
    };
    let generated_id = build_workflow_id(
        &family,
        namespace_prefix.as_deref(),
        timestamp_ms,
        &randomness_hex,
    )?;
    normalize_plan_workflow_id_payload_map(JsonMap::from_iter([
        ("family".to_string(), JsonValue::String(family)),
        maybe_json_entry("namespace_prefix", namespace_prefix),
        ("timestamp_ms".to_string(), JsonValue::from(timestamp_ms)),
        (
            "randomness_hex".to_string(),
            JsonValue::String(randomness_hex),
        ),
        ("generated_id".to_string(), JsonValue::String(generated_id)),
    ]))
}

pub fn normalize_plan_temporal_ordering_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().normalize_temporal_ordering_payload_json(payload_json)
}

fn normalize_plan_temporal_ordering_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let created_at = require_nonempty_text(payload.get("created_at"), "created_at")?;
    let published_at = optional_text(payload.get("published_at"))?;
    let effective_timestamp =
        require_nonempty_text(payload.get("effective_timestamp"), "effective_timestamp")?;
    let published = require_bool(payload.get("published"), "published")?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("created_at".to_string(), JsonValue::String(created_at)),
        maybe_json_entry("published_at", published_at),
        (
            "effective_timestamp".to_string(),
            JsonValue::String(effective_timestamp),
        ),
        ("published".to_string(), JsonValue::Bool(published)),
    ])))
}

pub fn build_plan_temporal_ordering_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    TimeIdentityJson::stateless().build_temporal_ordering_payload_json(payload_json)
}

fn build_plan_temporal_ordering_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let created_at = require_nonempty_text(payload.get("created_at"), "created_at")?;
    let published_at = optional_text(payload.get("published_at"))?;
    let effective_timestamp = published_at.clone().unwrap_or_else(|| created_at.clone());
    normalize_plan_temporal_ordering_payload_map(JsonMap::from_iter([
        ("created_at".to_string(), JsonValue::String(created_at)),
        maybe_json_entry("published_at", published_at.clone()),
        (
            "effective_timestamp".to_string(),
            JsonValue::String(effective_timestamp),
        ),
        (
            "published".to_string(),
            JsonValue::Bool(published_at.is_some()),
        ),
    ]))
}

fn build_workflow_id(
    family: &str,
    namespace_prefix: Option<&str>,
    timestamp_ms: i64,
    randomness_hex: &str,
) -> Result<String, String> {
    let token = workflow_id_token(family, namespace_prefix)?;
    let randomness_bytes = decode_hex(randomness_hex)?;
    if randomness_bytes.len() != 10 {
        return Err(
            "Plan workflow-id request randomness_hex must decode to exactly 10 bytes.".to_string(),
        );
    }
    let randomness = randomness_bytes
        .iter()
        .fold(0u128, |acc, byte| (acc << 8) | (*byte as u128));
    Ok(format!(
        "{token}-{}{}",
        encode_crockford_base32(timestamp_ms as u128, 10)?,
        encode_crockford_base32(randomness, 16)?,
    ))
}

fn parse_iso_timestamp_to_epoch_ms(value: &str) -> Result<i64, String> {
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|exc| {
        format!("Plan timestamp payload carried invalid timestamp `{value}`: {exc}")
    })?;
    Ok(parsed.timestamp_millis())
}

fn current_timestamp_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn encode_crockford_base32(value: u128, length: usize) -> Result<String, String> {
    let mut remaining = value;
    let mut chars = vec!['0'; length];
    for index in (0..length).rev() {
        chars[index] = CROCKFORD_BASE32[(remaining & 0b11111) as usize] as char;
        remaining >>= 5;
    }
    if remaining != 0 {
        return Err("Value does not fit requested Crockford base32 length".to_string());
    }
    Ok(chars.into_iter().collect())
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("Hex payload must have even length.".to_string());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let pair = std::str::from_utf8(&bytes[index..index + 2])
            .map_err(|exc| format!("Invalid UTF-8 in hex payload: {exc}"))?;
        let byte =
            u8::from_str_radix(pair, 16).map_err(|exc| format!("Invalid hex payload: {exc}"))?;
        out.push(byte);
        index += 2;
    }
    Ok(out)
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn require_nonempty_text(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    optional_text(value)?
        .ok_or_else(|| format!("Plan time/identity payload must include {field_name}."))
}

fn optional_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => {
            let trimmed = text.trim();
            Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
        }
        Some(_) => Err("Plan time/identity text fields must be strings.".to_string()),
    }
}

fn optional_text_allow_empty(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.trim().to_string())),
        Some(_) => Err("Plan time/identity text fields must be strings.".to_string()),
    }
}

fn require_nonnegative_i64(value: Option<&JsonValue>, field_name: &str) -> Result<i64, String> {
    let Some(value) = value else {
        return Err(format!(
            "Plan time/identity payload field `{field_name}` must be an integer."
        ));
    };
    let number = value.as_i64().ok_or_else(|| {
        format!("Plan time/identity payload field `{field_name}` must be an integer.")
    })?;
    if number < 0 {
        return Err(format!(
            "Plan time/identity payload field `{field_name}` must be >= 0."
        ));
    }
    Ok(number)
}

fn require_positive_i64(
    value: Option<&JsonValue>,
    field_name: &str,
    default: i64,
) -> Result<i64, String> {
    let number = match value {
        None | Some(JsonValue::Null) => default,
        _ => require_nonnegative_i64(value, field_name)?,
    };
    if number < 1 {
        return Err(format!(
            "Plan time/identity payload field `{field_name}` must be >= 1."
        ));
    }
    Ok(number)
}

fn require_bool(value: Option<&JsonValue>, field_name: &str) -> Result<bool, String> {
    let Some(value) = value else {
        return Err(format!(
            "Plan time/identity payload must include boolean `{field_name}`."
        ));
    };
    value
        .as_bool()
        .ok_or_else(|| format!("Plan time/identity payload must include boolean `{field_name}`."))
}

fn maybe_json_entry(key: &str, value: Option<String>) -> (String, JsonValue) {
    (
        key.to_string(),
        value.map(JsonValue::String).unwrap_or(JsonValue::Null),
    )
}

#[cfg(test)]
mod tests;
