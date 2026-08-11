use crate::transport::agent_transport_event_envelope_json;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_slack_ingress_runtime";
const INGRESS_RUNTIME_CONTRACT: &str = "ait_agent_core.event_loop.SlackIngressRuntime.v1";
const DEFAULT_ACK_TEXT: &str = "ait is thinking...";
const DEFAULT_RESPONSE_TYPE: &str = "in_channel";
const DEFAULT_RECENT_COMMAND_LIMIT: usize = 64;
const SLACK_USER_ACTOR_TYPE: &str = "slack_user";

pub trait SlackIngressRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackIngressRuntimePlanner;

impl SlackIngressRuntimePlanner for DefaultSlackIngressRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_ingress_runtime_json(request)
    }
}

pub fn agent_slack_ingress_runtime_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_slack_ingress_runtime_planner(&DefaultSlackIngressRuntimePlanner, request)
}

pub fn plan_with_slack_ingress_runtime_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: SlackIngressRuntimePlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_ingress_runtime_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "command".to_string());

    match stage.as_str() {
        "command" | "command_ingress" | "slash_command" => plan_command(object),
        "socket_envelope" | "socket_mode_envelope" => plan_socket_envelope(object),
        other => Err(format!("unsupported Slack ingress runtime stage: {other}")),
    }
}

fn plan_command(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let payload = payload_object(object, "command_payload", "payload")?;

    if optional_bool(payload.get("ssl_check")).unwrap_or(false) {
        return Ok(base_payload(
            "command",
            "ssl_check",
            json!({
                "ok": true,
                "accepted": true,
                "ssl_check": true,
                "response": command_message_response("ok", "ephemeral"),
                "should_submit_turn": false,
                "should_create_turn": false,
                "should_start_background_reply": false,
                "should_execute_inline_reply": false,
                "actions": [
                    {
                        "kind": "respond_to_ssl_check",
                        "response": command_message_response("ok", "ephemeral"),
                    }
                ],
            }),
        ));
    }

    let text = clean_text(payload.get("text")).unwrap_or_default();
    if text.is_empty() {
        return Ok(base_payload(
            "command",
            "missing_text",
            json!({
                "ok": true,
                "accepted": false,
                "response": command_message_response(
                    "Slack command must include text content.",
                    "ephemeral",
                ),
                "should_submit_turn": false,
                "should_create_turn": false,
                "should_start_background_reply": false,
                "should_execute_inline_reply": false,
                "actions": [
                    {
                        "kind": "respond_to_command",
                        "reason": "missing_text",
                    }
                ],
            }),
        ));
    }

    let channel_id = required_clean_text(
        payload.get("channel_id"),
        "Slack command payload is missing a channel id.",
    )?;
    let response_url = required_clean_text(
        payload.get("response_url"),
        "Slack command payload is missing a response_url.",
    )?;
    let source_user_id = required_clean_text(
        payload.get("user_id"),
        "Slack command payload is missing a user id.",
    )?;
    let command_name = clean_text(payload.get("command"));
    let team_id = clean_text(payload.get("team_id"));
    let channel_name = clean_text(payload.get("channel_name"));
    let thread_id = clean_text(payload.get("thread_ts"));
    let request_id = slack_request_id(payload);
    let binding = object
        .get("binding")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let existing_recent_request_ids = recent_request_ids(object, &binding);

    if is_duplicate(object)
        || existing_recent_request_ids
            .iter()
            .any(|recent| recent == &request_id)
    {
        return Ok(base_payload(
            "command",
            "duplicate_ignored",
            json!({
                "ok": true,
                "accepted": false,
                "duplicate": true,
                "event_id": request_id,
                "dedupe_key": request_id,
                "channel_id": channel_id,
                "thread_id": optional_string_json(thread_id.as_deref()),
                "response": command_message_response(
                    "Duplicate Slack command ignored.",
                    "ephemeral",
                ),
                "should_submit_turn": false,
                "should_create_turn": false,
                "should_start_background_reply": false,
                "should_execute_inline_reply": false,
                "actions": [],
            }),
        ));
    }

    let channel_kind = slack_channel_kind(channel_name.as_deref());
    let channel_title = slack_channel_title(&channel_id, channel_name.as_deref());
    let actor_identity = slack_actor_identity(&source_user_id);
    let actor_username = clean_text(payload.get("user_name"));
    let actor_display_name = actor_username
        .clone()
        .unwrap_or_else(|| source_user_id.clone());
    let repo_name = clean_text(object.get("repo_name")).unwrap_or_default();
    let conversation_key = slack_conversation_key(&channel_id, thread_id.as_deref());
    let reply_target = slack_reply_target(
        &channel_id,
        &channel_kind,
        team_id.as_deref(),
        &response_url,
        thread_id.as_deref(),
        Some(&source_user_id),
    );
    let recent_command_patch = recent_command_patch(
        &existing_recent_request_ids,
        &request_id,
        &source_user_id,
        team_id.as_deref(),
        command_name.as_deref(),
        recent_command_limit(object),
    );
    let occurred_at =
        clean_text(object.get("occurred_at")).or_else(|| clean_text(object.get("now_iso")));
    let metadata = json!({
        "command_name": optional_string_json(command_name.as_deref()),
        "team_id": optional_string_json(team_id.as_deref()),
        "response_url_present": true,
    });
    let transport_envelope = agent_transport_event_envelope_json(
        "slack",
        &actor_identity,
        &channel_id,
        &text,
        Some(&source_user_id),
        actor_username.as_deref(),
        Some(&actor_display_name),
        None,
        Some(&channel_title),
        Some(&channel_kind),
        thread_id.as_deref(),
        None,
        None,
        occurred_at.as_deref(),
        Some(&request_id),
        Some(&request_id),
        None,
        Some(&metadata),
    );
    let pending_reply = pending_slack_reply(
        &conversation_key,
        &channel_id,
        &channel_title,
        &channel_kind,
        &response_url,
        &request_id,
        &actor_identity,
        &actor_display_name,
        &text,
        &transport_envelope,
        &source_user_id,
        team_id.as_deref(),
        command_name.as_deref(),
        thread_id.as_deref(),
    );
    let defer_replies = optional_bool(object.get("defer_replies")).unwrap_or(true);
    let ack_text =
        clean_text(object.get("ack_text")).unwrap_or_else(|| DEFAULT_ACK_TEXT.to_string());
    let response_type = clean_text(object.get("response_type"))
        .unwrap_or_else(|| DEFAULT_RESPONSE_TYPE.to_string());
    let response = if defer_replies {
        command_message_response(&ack_text, "ephemeral")
    } else {
        command_message_response(
            clean_text(object.get("reply_text"))
                .unwrap_or_else(|| "(pending Slack reply)".to_string())
                .as_str(),
            &response_type,
        )
    };
    let mut actions = vec![json!({
        "kind": "upsert_binding",
        "transport": "slack",
        "surface_id": channel_id,
        "thread_id": optional_string_json(thread_id.as_deref()),
        "conversation_key": conversation_key,
        "repo_name": repo_name,
        "channel_title": channel_title,
        "channel_kind": channel_kind,
        "slack_source_user_id": source_user_id,
        "slack_team_id": optional_string_json(team_id.as_deref()),
        "slack_reply_target": reply_target,
    })];
    actions.push(json!({
        "kind": "remember_command",
        "channel_id": channel_id,
        "thread_id": optional_string_json(thread_id.as_deref()),
        "request_id": request_id,
        "source_user_id": source_user_id,
        "team_id": optional_string_json(team_id.as_deref()),
        "command_name": optional_string_json(command_name.as_deref()),
        "patch": recent_command_patch,
    }));
    actions.push(json!({
        "kind": if defer_replies { "start_background_reply" } else { "execute_inline_reply" },
        "pending_reply": pending_reply,
    }));

    Ok(base_payload(
        "command",
        "turn_submission_planned",
        json!({
            "ok": true,
            "accepted": true,
            "duplicate": false,
            "event_id": request_id,
            "dedupe_key": request_id,
            "request_id": request_id,
            "channel_id": channel_id,
            "channel_title": channel_title,
            "channel_kind": channel_kind,
            "thread_id": optional_string_json(thread_id.as_deref()),
            "team_id": optional_string_json(team_id.as_deref()),
            "source_user_id": source_user_id,
            "command_name": optional_string_json(command_name.as_deref()),
            "actor_identity": actor_identity,
            "actor_type": SLACK_USER_ACTOR_TYPE,
            "actor_display_name": actor_display_name,
            "text": text,
            "conversation_key": conversation_key,
            "binding": binding,
            "transport_envelope": transport_envelope,
            "pending_reply": pending_reply,
            "recent_command_patch": recent_command_patch,
            "response": response,
            "response_type": response_type,
            "defer_replies": defer_replies,
            "should_submit_turn": true,
            "should_create_turn": true,
            "should_start_background_reply": defer_replies,
            "should_execute_inline_reply": !defer_replies,
            "actions": actions,
        }),
    ))
}

fn plan_socket_envelope(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let envelope = socket_envelope_object(object)?;
    let envelope_id = required_clean_text(
        envelope.get("envelope_id"),
        "Slack Socket Mode envelope is missing an envelope id.",
    )?;
    let envelope_type = clean_text(envelope.get("type"));
    if envelope_type.as_deref() != Some("slash_commands") {
        return Ok(base_payload(
            "socket_envelope",
            "ignored_envelope",
            json!({
                "ok": true,
                "accepted": false,
                "envelope_id": envelope_id,
                "envelope_type": optional_string_json(envelope_type.as_deref()),
                "response": {"envelope_id": envelope_id},
                "should_handle_command": false,
                "actions": [
                    {
                        "kind": "ack_socket_envelope",
                        "envelope_id": envelope_id,
                    }
                ],
            }),
        ));
    }

    let payload = envelope
        .get("payload")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            "Slack Socket Mode envelope is missing a slash-command payload object.".to_string()
        })?;
    let mut command_request = object.clone();
    command_request.insert(
        "stage".to_string(),
        JsonValue::String("command".to_string()),
    );
    command_request.insert("payload".to_string(), JsonValue::Object(payload.clone()));
    let command_plan = plan_command(&command_request)?;
    let accepts_response_payload =
        optional_bool(envelope.get("accepts_response_payload")).unwrap_or(false);
    let mut response = Map::new();
    response.insert(
        "envelope_id".to_string(),
        JsonValue::String(envelope_id.clone()),
    );
    if accepts_response_payload {
        response.insert(
            "payload".to_string(),
            command_plan
                .get("response")
                .cloned()
                .unwrap_or_else(|| JsonValue::Object(Map::new())),
        );
    }

    Ok(base_payload(
        "socket_envelope",
        "command_ack_planned",
        json!({
            "ok": true,
            "accepted": true,
            "envelope_id": envelope_id,
            "envelope_type": "slash_commands",
            "accepts_response_payload": accepts_response_payload,
            "command_plan": command_plan,
            "response": JsonValue::Object(response),
            "should_handle_command": true,
            "actions": [
                {
                    "kind": "handle_slash_command",
                    "envelope_id": envelope_id,
                },
                {
                    "kind": "ack_socket_envelope",
                    "envelope_id": envelope_id,
                    "include_response_payload": accepts_response_payload,
                }
            ],
        }),
    ))
}

fn base_payload(stage: &str, state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "slack_ingress_runtime_contract".to_string(),
        JsonValue::String(INGRESS_RUNTIME_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "transport".to_string(),
        JsonValue::String("slack".to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert("python_ingress_allowed".to_string(), JsonValue::Bool(false));
    object.insert(
        "ingress_runtime_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    JsonValue::Object(object)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "Slack ingress runtime request must be an object.".to_string())
}

fn payload_object<'a>(
    object: &'a Map<String, JsonValue>,
    primary_key: &str,
    fallback_key: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    object
        .get(primary_key)
        .or_else(|| object.get(fallback_key))
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Slack ingress runtime payload must be a JSON object.".to_string())
}

fn socket_envelope_object(
    object: &Map<String, JsonValue>,
) -> Result<&Map<String, JsonValue>, String> {
    object
        .get("envelope")
        .or_else(|| object.get("payload"))
        .and_then(JsonValue::as_object)
        .or_else(|| {
            if object.contains_key("envelope_id") {
                Some(object)
            } else {
                None
            }
        })
        .ok_or_else(|| "Slack Socket Mode envelope must be a JSON object.".to_string())
}

fn required_clean_text(value: Option<&JsonValue>, error: &str) -> Result<String, String> {
    clean_text(value).ok_or_else(|| error.to_string())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = match value? {
        JsonValue::String(text) => text.trim().to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => return None,
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" | "" => Some(false),
            _ => None,
        },
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_usize(value: Option<&JsonValue>) -> Option<usize> {
    match value? {
        JsonValue::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok()),
        JsonValue::String(text) => text.trim().parse::<usize>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .filter(|text| !text.trim().is_empty())
        .map(|text| JsonValue::String(text.to_string()))
        .unwrap_or(JsonValue::Null)
}

fn is_duplicate(object: &Map<String, JsonValue>) -> bool {
    ["duplicate", "already_processed", "processed_command"]
        .iter()
        .any(|key| optional_bool(object.get(*key)).unwrap_or(false))
}

fn slack_actor_identity(user_id: &str) -> String {
    format!("slack:{user_id}")
}

fn slack_channel_kind(channel_name: Option<&str>) -> String {
    if channel_name
        .map(|name| name.eq_ignore_ascii_case("directmessage"))
        .unwrap_or(false)
    {
        "dm".to_string()
    } else {
        "channel".to_string()
    }
}

fn slack_channel_title(channel_id: &str, channel_name: Option<&str>) -> String {
    let Some(channel_name) = channel_name.filter(|name| !name.trim().is_empty()) else {
        return format!("Slack channel · {channel_id}");
    };
    if channel_name.eq_ignore_ascii_case("directmessage") {
        format!("Slack DM · {channel_id}")
    } else {
        format!("Slack channel · #{channel_name}")
    }
}

fn slack_request_id(payload: &Map<String, JsonValue>) -> String {
    if let Some(trigger_id) = clean_text(payload.get("trigger_id")) {
        return trigger_id;
    }
    let channel_id = clean_text(payload.get("channel_id")).unwrap_or_else(|| "channel".to_string());
    let user_id = clean_text(payload.get("user_id")).unwrap_or_else(|| "user".to_string());
    let command_name = clean_text(payload.get("command")).unwrap_or_else(|| "/ait".to_string());
    let command_text = clean_text(payload.get("text")).unwrap_or_else(|| "command".to_string());
    format!("slack:{channel_id}:{user_id}:{command_name}:{command_text}")
}

fn command_message_response(text: &str, response_type: &str) -> JsonValue {
    let text = text.trim();
    let text = if text.is_empty() { "(empty)" } else { text };
    json!({
        "response_type": response_type,
        "text": text,
    })
}

fn slack_reply_target(
    channel_id: &str,
    channel_kind: &str,
    team_id: Option<&str>,
    response_url: &str,
    thread_id: Option<&str>,
    source_user_id: Option<&str>,
) -> JsonValue {
    let mut target = Map::new();
    target.insert(
        "channel_id".to_string(),
        JsonValue::String(channel_id.to_string()),
    );
    target.insert(
        "channel_kind".to_string(),
        JsonValue::String(channel_kind.to_string()),
    );
    target.insert("team_id".to_string(), optional_string_json(team_id));
    target.insert(
        "response_url".to_string(),
        JsonValue::String(response_url.to_string()),
    );
    if let Some(thread_id) = thread_id {
        target.insert(
            "thread_id".to_string(),
            JsonValue::String(thread_id.to_string()),
        );
    }
    if let Some(source_user_id) = source_user_id {
        target.insert(
            "source_user_id".to_string(),
            JsonValue::String(source_user_id.to_string()),
        );
    }
    JsonValue::Object(target)
}

#[allow(clippy::too_many_arguments)]
fn pending_slack_reply(
    conversation_key: &str,
    channel_id: &str,
    channel_title: &str,
    channel_kind: &str,
    response_url: &str,
    request_id: &str,
    actor_identity: &str,
    actor_display_name: &str,
    text: &str,
    transport_envelope: &JsonValue,
    source_user_id: &str,
    team_id: Option<&str>,
    command_name: Option<&str>,
    thread_id: Option<&str>,
) -> JsonValue {
    json!({
        "conversation_key": conversation_key,
        "channel_id": channel_id,
        "channel_title": channel_title,
        "channel_kind": channel_kind,
        "response_url": response_url,
        "request_id": request_id,
        "actor_identity": actor_identity,
        "actor_type": SLACK_USER_ACTOR_TYPE,
        "actor_display_name": actor_display_name,
        "text": text,
        "transport_envelope": transport_envelope,
        "source_user_id": source_user_id,
        "team_id": optional_string_json(team_id),
        "command_name": optional_string_json(command_name),
        "thread_id": optional_string_json(thread_id),
    })
}

fn recent_request_ids(object: &Map<String, JsonValue>, binding: &JsonValue) -> Vec<String> {
    let mut values = Vec::new();
    collect_text_values(object.get("recent_request_ids"), &mut values);
    collect_text_values(object.get("processed_request_ids"), &mut values);
    collect_text_values(object.get("slack_recent_request_ids"), &mut values);
    if let Some(binding) = binding.as_object() {
        collect_text_values(binding.get("recent_request_ids"), &mut values);
        collect_text_values(binding.get("processed_request_ids"), &mut values);
        collect_text_values(binding.get("slack_recent_request_ids"), &mut values);
    }
    values
}

fn slack_conversation_key(channel_id: &str, thread_id: Option<&str>) -> String {
    match thread_id {
        Some(thread_id) => format!("slack:{channel_id}:thread:{thread_id}"),
        None => format!("slack:{channel_id}"),
    }
}

fn collect_text_values(value: Option<&JsonValue>, values: &mut Vec<String>) {
    match value {
        Some(JsonValue::Array(items)) => {
            for item in items {
                if let Some(text) = clean_text(Some(item)) {
                    values.push(text);
                }
            }
        }
        Some(value) => {
            if let Some(text) = clean_text(Some(value)) {
                values.push(text);
            }
        }
        None => {}
    }
}

fn recent_command_limit(object: &Map<String, JsonValue>) -> usize {
    optional_usize(object.get("recent_command_limit")).unwrap_or(DEFAULT_RECENT_COMMAND_LIMIT)
}

fn recent_command_patch(
    existing_recent_request_ids: &[String],
    request_id: &str,
    source_user_id: &str,
    team_id: Option<&str>,
    command_name: Option<&str>,
    limit: usize,
) -> JsonValue {
    let mut recent = existing_recent_request_ids
        .iter()
        .filter_map(|value| {
            let value = value.trim();
            if value.is_empty() || value == request_id {
                None
            } else {
                Some(value.to_string())
            }
        })
        .collect::<Vec<_>>();
    recent.push(request_id.to_string());
    if limit > 0 && recent.len() > limit {
        recent = recent.split_off(recent.len() - limit);
    }
    json!({
        "slack_recent_request_ids": recent,
        "slack_last_request_id": request_id,
        "slack_last_source_user_id": source_user_id,
        "slack_last_team_id": optional_string_json(team_id),
        "slack_last_command_name": optional_string_json(command_name),
    })
}
