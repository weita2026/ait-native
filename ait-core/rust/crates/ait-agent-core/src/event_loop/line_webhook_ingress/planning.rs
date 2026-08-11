use crate::json_support::parse_value;
use crate::transport::agent_transport_event_envelope_json;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{TimeZone, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

const MIGRATION_STAGE: &str = "rust_agent_line_webhook_ingress";
const WEBHOOK_INGRESS_CONTRACT: &str = "ait_agent_core.event_loop.LineWebhookIngress.v1";
const DEFAULT_WEBHOOK_PATH: &str = "/callback";

type HmacSha256 = Hmac<Sha256>;

pub trait LineWebhookIngressPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultLineWebhookIngressPlanner;

impl LineWebhookIngressPlanner for DefaultLineWebhookIngressPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_webhook_ingress_json(request)
    }
}

pub fn agent_line_webhook_ingress_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_line_webhook_ingress_planner(&DefaultLineWebhookIngressPlanner, request)
}

pub fn plan_with_line_webhook_ingress_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: LineWebhookIngressPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_webhook_ingress_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let request_path =
        clean_text(object.get("request_path")).or_else(|| clean_text(object.get("path")));
    let webhook_path = clean_text(object.get("webhook_path"))
        .map(|value| normalize_webhook_path(&value))
        .unwrap_or_else(|| DEFAULT_WEBHOOK_PATH.to_string());

    if let Some(path) = request_path.as_deref() {
        if path != webhook_path {
            return Ok(base_payload(
                "not_found",
                json!({
                    "ok": false,
                    "accepted": false,
                    "http_status": 404,
                    "write_json_response": false,
                    "response": JsonValue::Null,
                    "error_kind": "not_found",
                    "error": "LINE webhook endpoint not found.",
                    "webhook_path": webhook_path,
                    "request_path": path,
                    "should_handle_webhook": false,
                    "should_parse_payload": false,
                    "signature_verified": false,
                    "payload": JsonValue::Null,
                    "event_plans": [],
                    "accepted_event_count": 0,
                }),
            ));
        }
    }

    let channel_secret = clean_text(object.get("channel_secret")).unwrap_or_default();
    if channel_secret.is_empty() {
        return Ok(error_payload(
            "missing_channel_secret",
            400,
            "config_error",
            "Missing LINE channel secret for webhook verification.",
        ));
    }

    let raw_payload = required_string(object.get("raw_payload"), "raw_payload")?;
    let signature = clean_text(object.get("signature"));
    if let Err(error) = verify_line_signature(&raw_payload, signature.as_deref(), &channel_secret) {
        return Ok(error_payload(
            error.state,
            401,
            "invalid_signature",
            error.message,
        ));
    }

    let payload = match parse_line_webhook_payload(&raw_payload) {
        Ok(payload) => payload,
        Err(error) => {
            return Ok(error_payload(
                error.state,
                400,
                "invalid_payload",
                error.message,
            ));
        }
    };
    let events = payload
        .get("events")
        .and_then(JsonValue::as_array)
        .expect("validated LINE webhook payload must have an events array");
    let mut event_plans = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        let event_object = event
            .as_object()
            .expect("validated LINE webhook events must be JSON objects");
        event_plans.push(plan_line_event(object, event_object, index)?);
    }
    let accepted_event_count = event_plans
        .iter()
        .filter(|event| optional_bool(event.get("should_submit_turn")).unwrap_or(false))
        .count();

    Ok(base_payload(
        "accepted",
        json!({
            "ok": true,
            "accepted": true,
            "http_status": 200,
            "write_json_response": true,
            "response": {
                "ok": true,
                "processed_events": accepted_event_count,
            },
            "error_kind": JsonValue::Null,
            "error": JsonValue::Null,
            "webhook_path": webhook_path,
            "request_path": request_path,
            "should_handle_webhook": true,
            "should_parse_payload": true,
            "signature_verified": true,
            "payload": payload,
            "event_count": events.len(),
            "event_plans": event_plans,
            "accepted_event_count": accepted_event_count,
        }),
    ))
}

fn plan_line_event(
    request: &Map<String, JsonValue>,
    event: &Map<String, JsonValue>,
    index: usize,
) -> Result<JsonValue, String> {
    let event_type = clean_text(event.get("type")).unwrap_or_default();
    if event_type != "message" {
        return Ok(event_payload(
            "ignored_unsupported_event_type",
            index,
            json!({
                "ok": true,
                "accepted": false,
                "event_type": event_type,
                "message_type": JsonValue::Null,
                "ignore_reason": "unsupported_event_type",
                "should_submit_turn": false,
                "pending_reply": JsonValue::Null,
            }),
        ));
    }

    let message = event
        .get("message")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            "LINE webhook message event is missing a valid message object.".to_string()
        })?;
    let message_type = clean_text(message.get("type")).unwrap_or_default();
    if message_type != "text" {
        return Ok(event_payload(
            "ignored_unsupported_message_type",
            index,
            json!({
                "ok": true,
                "accepted": false,
                "event_type": event_type,
                "message_type": message_type,
                "ignore_reason": "unsupported_message_type",
                "should_submit_turn": false,
                "pending_reply": JsonValue::Null,
            }),
        ));
    }

    let text = clean_text(message.get("text")).unwrap_or_default();
    if text.is_empty() {
        return Ok(event_payload(
            "ignored_empty_text",
            index,
            json!({
                "ok": true,
                "accepted": false,
                "event_type": event_type,
                "message_type": message_type,
                "ignore_reason": "empty_text",
                "should_submit_turn": false,
                "pending_reply": JsonValue::Null,
            }),
        ));
    }

    let source = event
        .get("source")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            "LINE webhook message event is missing a valid source object.".to_string()
        })?;
    let channel_id = line_channel_id(source)
        .ok_or_else(|| "LINE webhook message event is missing a usable channel id.".to_string())?;
    let channel_kind = clean_text(source.get("type"));
    let channel_title = line_channel_title(&channel_id, channel_kind.as_deref());
    let source_user_id = clean_text(source.get("userId"));
    let actor_identity = line_actor_identity(source, &channel_id);
    let actor_display_name = source_user_id
        .as_deref()
        .unwrap_or(channel_id.as_str())
        .to_string();
    let message_id = clean_text(message.get("id"));
    let reply_token = clean_text(event.get("replyToken"));
    let webhook_event_id = clean_text(event.get("webhookEventId"))
        .unwrap_or_else(|| fallback_event_id(&channel_id, message, event, request));
    let occurred_at = timestamp_to_iso(event.get("timestamp"));
    let metadata = line_event_metadata(event, reply_token.as_deref());
    let message_id_value = message_id
        .as_deref()
        .and_then(normalize_positive_i64)
        .map(JsonValue::from);
    let message_ids = match message_id.as_deref() {
        Some(value) => json!([value]),
        None => json!([]),
    };
    let transport_envelope = agent_transport_event_envelope_json(
        "line",
        &actor_identity,
        &channel_id,
        &text,
        source_user_id.as_deref(),
        None,
        Some(&actor_display_name),
        None,
        Some(&channel_title),
        channel_kind.as_deref(),
        None,
        message_id_value.as_ref(),
        Some(&message_ids),
        occurred_at.as_deref(),
        Some(&webhook_event_id),
        Some(&webhook_event_id),
        None,
        Some(&metadata),
    );
    let conversation_key = format!("line:{channel_id}");
    let binding_request = json!({
        "transport": "line",
        "surface_id": channel_id,
        "conversation_key": conversation_key,
        "channel_id": channel_id,
        "channel_title": channel_title,
        "channel_kind": optional_string_json(channel_kind.as_deref()),
        "source_user_id": optional_string_json(source_user_id.as_deref()),
    });
    let pending_reply = json!({
        "conversation_key": conversation_key,
        "channel_id": channel_id,
        "channel_title": channel_title,
        "channel_kind": optional_string_json(channel_kind.as_deref()),
        "reply_token": optional_string_json(reply_token.as_deref()),
        "actor_identity": actor_identity,
        "actor_display_name": actor_display_name,
        "text": text,
        "transport_envelope": transport_envelope,
        "source_user_id": optional_string_json(source_user_id.as_deref()),
        "message_id": optional_string_json(message_id.as_deref()),
        "webhook_event_id": webhook_event_id,
    });

    Ok(event_payload(
        "text_message_ready",
        index,
        json!({
            "ok": true,
            "accepted": true,
            "event_type": event_type,
            "message_type": message_type,
            "ignore_reason": JsonValue::Null,
            "should_submit_turn": true,
            "should_upsert_binding": true,
            "conversation_key": conversation_key,
            "channel_id": channel_id,
            "channel_title": channel_title,
            "channel_kind": optional_string_json(channel_kind.as_deref()),
            "source_user_id": optional_string_json(source_user_id.as_deref()),
            "message_id": optional_string_json(message_id.as_deref()),
            "reply_token": optional_string_json(reply_token.as_deref()),
            "webhook_event_id": webhook_event_id,
            "actor_identity": actor_identity,
            "actor_display_name": actor_display_name,
            "text": text,
            "occurred_at": optional_string_json(occurred_at.as_deref()),
            "transport_envelope": transport_envelope,
            "pending_reply": pending_reply,
            "binding_request": binding_request,
        }),
    ))
}

fn verify_line_signature(
    raw_payload: &str,
    signature: Option<&str>,
    channel_secret: &str,
) -> Result<(), LineSignatureError> {
    let Some(normalized_signature) = signature.and_then(clean_text_str) else {
        return Err(LineSignatureError::new(
            "missing_signature",
            "Missing LINE webhook signature header.",
        ));
    };
    let expected = build_line_signature(raw_payload, channel_secret)?;
    if !constant_time_eq(expected.as_bytes(), normalized_signature.as_bytes()) {
        return Err(LineSignatureError::new(
            "invalid_signature",
            "Invalid LINE webhook signature.",
        ));
    }
    Ok(())
}

fn build_line_signature(
    raw_payload: &str,
    channel_secret: &str,
) -> Result<String, LineSignatureError> {
    let mut mac = HmacSha256::new_from_slice(channel_secret.as_bytes()).map_err(|_| {
        LineSignatureError::new("invalid_channel_secret", "Invalid LINE channel secret.")
    })?;
    mac.update(raw_payload.as_bytes());
    let digest = mac.finalize().into_bytes();
    Ok(STANDARD.encode(digest))
}

fn parse_line_webhook_payload(raw_payload: &str) -> Result<JsonValue, LinePayloadError> {
    if raw_payload.trim().is_empty() {
        return Err(LinePayloadError::new(
            "empty_payload",
            "No LINE webhook payload provided.",
        ));
    }
    let payload =
        parse_value(raw_payload, "failed to parse LINE webhook payload").map_err(|_| {
            LinePayloadError::new("invalid_json", "LINE webhook payload must be valid JSON.")
        })?;
    let Some(object) = payload.as_object() else {
        return Err(LinePayloadError::new(
            "invalid_payload",
            "LINE webhook payload must be a JSON object.",
        ));
    };
    let Some(events) = object.get("events").and_then(JsonValue::as_array) else {
        return Err(LinePayloadError::new(
            "missing_events",
            "LINE webhook payload must include an events list.",
        ));
    };
    for (index, event) in events.iter().enumerate() {
        if !event.is_object() {
            return Err(LinePayloadError::new(
                "invalid_event",
                format!("LINE webhook event #{index} must be a JSON object."),
            ));
        }
    }
    Ok(payload)
}

fn base_payload(state: &str, mut payload: JsonValue) -> JsonValue {
    let object = payload
        .as_object_mut()
        .expect("base payload must be backed by an object");
    object.insert(
        "stage".to_string(),
        JsonValue::String("line_webhook_request".to_string()),
    );
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "line_webhook_ingress_contract".to_string(),
        JsonValue::String(WEBHOOK_INGRESS_CONTRACT.to_string()),
    );
    object.insert(
        "transport".to_string(),
        JsonValue::String("line".to_string()),
    );
    object.insert(
        "webhook_ingress_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "python_signature_verification_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_json_parsing_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_event_planning_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "webhook_runtime_required".to_string(),
        JsonValue::Bool(true),
    );
    payload
}

fn event_payload(state: &str, index: usize, mut payload: JsonValue) -> JsonValue {
    let object = payload
        .as_object_mut()
        .expect("event payload must be backed by an object");
    object.insert("event_index".to_string(), JsonValue::from(index));
    object.insert(
        "event_ingress_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "transport".to_string(),
        JsonValue::String("line".to_string()),
    );
    payload
}

fn error_payload(
    state: &str,
    status: u16,
    error_kind: &str,
    error: impl Into<String>,
) -> JsonValue {
    let error = error.into();
    base_payload(
        state,
        json!({
            "ok": false,
            "accepted": false,
            "http_status": status,
            "write_json_response": true,
            "response": {
                "ok": false,
                "error": error,
            },
            "error_kind": error_kind,
            "error": error,
            "should_handle_webhook": false,
            "should_parse_payload": false,
            "signature_verified": false,
            "payload": JsonValue::Null,
            "event_count": 0,
            "event_plans": [],
            "accepted_event_count": 0,
        }),
    )
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "LINE webhook ingress request must be an object.".to_string())
}

fn required_string(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("LINE webhook ingress request requires `{field_name}`."))
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value.and_then(|item| {
        if let Some(text) = item.as_str() {
            clean_text_str(text)
        } else if item.is_null() {
            None
        } else {
            clean_text_str(&item.to_string())
        }
    })
}

fn clean_text_str(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn normalize_webhook_path(value: &str) -> String {
    let text = value.trim();
    if text.is_empty() {
        DEFAULT_WEBHOOK_PATH.to_string()
    } else if text.starts_with('/') {
        text.to_string()
    } else {
        format!("/{text}")
    }
}

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" | "" => Some(false),
            _ => None,
        },
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}

fn line_channel_id(source: &Map<String, JsonValue>) -> Option<String> {
    clean_text(source.get("groupId"))
        .or_else(|| clean_text(source.get("roomId")))
        .or_else(|| clean_text(source.get("userId")))
}

fn line_channel_title(channel_id: &str, channel_kind: Option<&str>) -> String {
    let kind = channel_kind.unwrap_or("chat");
    format!("LINE {kind} · {channel_id}")
}

fn line_actor_identity(source: &Map<String, JsonValue>, channel_id: &str) -> String {
    let actor_id = clean_text(source.get("userId")).unwrap_or_else(|| channel_id.to_string());
    format!("line:{actor_id}")
}

fn fallback_event_id(
    channel_id: &str,
    message: &Map<String, JsonValue>,
    event: &Map<String, JsonValue>,
    request: &Map<String, JsonValue>,
) -> String {
    let message_id = clean_text(message.get("id")).unwrap_or_else(|| "message".to_string());
    let timestamp = clean_text(event.get("timestamp"))
        .or_else(|| clean_text(request.get("now_iso")))
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    format!("line:{channel_id}:{message_id}:{timestamp}")
}

fn timestamp_to_iso(value: Option<&JsonValue>) -> Option<String> {
    let timestamp_ms = match value? {
        JsonValue::Number(number) => number.as_i64()?,
        JsonValue::String(text) => text.trim().parse::<i64>().ok()?,
        _ => return None,
    };
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|datetime| datetime.to_rfc3339())
}

fn normalize_positive_i64(value: &str) -> Option<i64> {
    let parsed = value.trim().parse::<i64>().ok()?;
    (parsed > 0).then_some(parsed)
}

fn line_event_metadata(event: &Map<String, JsonValue>, reply_token: Option<&str>) -> JsonValue {
    let mut metadata = Map::new();
    metadata.insert(
        "delivery_mode".to_string(),
        optional_string_json(clean_text(event.get("mode")).as_deref()),
    );
    let is_redelivery = event
        .get("deliveryContext")
        .and_then(JsonValue::as_object)
        .and_then(|context| optional_bool(context.get("isRedelivery")))
        .unwrap_or(false);
    metadata.insert("is_redelivery".to_string(), JsonValue::Bool(is_redelivery));
    if let Some(reply_token) = reply_token {
        metadata.insert(
            "reply_token".to_string(),
            JsonValue::String(reply_token.to_string()),
        );
    }
    JsonValue::Object(metadata)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in left.iter().zip(right.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

#[derive(Debug)]
struct LineSignatureError {
    state: &'static str,
    message: &'static str,
}

impl LineSignatureError {
    fn new(state: &'static str, message: &'static str) -> Self {
        Self { state, message }
    }
}

#[derive(Debug)]
struct LinePayloadError {
    state: &'static str,
    message: String,
}

impl LinePayloadError {
    fn new(state: &'static str, message: impl Into<String>) -> Self {
        Self {
            state,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests;
