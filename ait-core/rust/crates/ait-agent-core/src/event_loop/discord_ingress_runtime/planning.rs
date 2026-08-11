use crate::agent_telegram_event_trigger_plan_json;
use crate::json_support::parse_value;
use crate::transport::agent_transport_event_envelope_json;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use ring::signature::{UnparsedPublicKey, ED25519};

const MIGRATION_STAGE: &str = "rust_agent_discord_ingress_runtime";
const INGRESS_RUNTIME_CONTRACT: &str = "ait_agent_core.event_loop.DiscordIngressRuntime.v1";
const PING_INTERACTION_TYPE: i64 = 1;
const PONG_RESPONSE_TYPE: i64 = 1;
const APPLICATION_COMMAND_INTERACTION_TYPE: i64 = 2;
const CHANNEL_MESSAGE_WITH_SOURCE_TYPE: i64 = 4;
const DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE_TYPE: i64 = 5;
const DISCORD_REPLY_MODE_INTERACTION: &str = "interaction";
const DISCORD_REPLY_MODE_CHANNEL_MESSAGE: &str = "channel_message";
const DEFAULT_RECOVERY_ATTEMPTS: i64 = 3;
const DEFAULT_RECOVERY_DELAY_SECONDS: &[f64] = &[1.0, 3.0, 7.0];
const DEFAULT_WATCH_MAX_WAIT_SECONDS: f64 = 120.0;
const DEFAULT_WATCH_POLL_INTERVAL_SECONDS: f64 = 2.0;

pub trait DiscordIngressRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDiscordIngressRuntimePlanner;

impl DiscordIngressRuntimePlanner for DefaultDiscordIngressRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_ingress_runtime_json(request)
    }
}

pub fn agent_discord_ingress_runtime_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_discord_ingress_runtime_planner(&DefaultDiscordIngressRuntimePlanner, request)
}

pub fn plan_with_discord_ingress_runtime_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: DiscordIngressRuntimePlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_ingress_runtime_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "interaction".to_string());

    match stage.as_str() {
        "interaction_http_request" | "http_interaction_request" => {
            plan_interaction_http_request(object)
        }
        "parse_interaction_payload" | "interaction_payload" => {
            plan_parse_interaction_payload(object)
        }
        "interaction" | "interaction_ingress" => plan_interaction(object),
        "message" | "message_ingress" => plan_message(object),
        other => Err(format!(
            "unsupported Discord ingress runtime stage: {other}"
        )),
    }
}

fn plan_interaction_http_request(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let public_key = clean_text(object.get("public_key")).unwrap_or_default();
    if public_key.is_empty() {
        return Ok(interaction_http_error_payload(
            "missing_public_key",
            400,
            "config_error",
            "Missing Discord public key for interaction verification.",
            false,
        ));
    }

    let raw_payload = required_string(object.get("raw_payload"), "raw_payload")?;
    let signature = clean_text(object.get("signature"));
    let signature_timestamp = clean_text(object.get("signature_timestamp"))
        .or_else(|| clean_text(object.get("timestamp")));

    if let Err(error) = verify_discord_interaction_signature(
        &raw_payload,
        signature.as_deref(),
        signature_timestamp.as_deref(),
        &public_key,
    ) {
        return Ok(interaction_http_error_payload(
            error.state,
            error.http_status,
            error.error_kind,
            error.message,
            false,
        ));
    }

    let (payload, interaction_type) = match parse_discord_interaction_payload(&raw_payload) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Ok(interaction_http_error_payload(
                error.state,
                400,
                "invalid_payload",
                error.message,
                true,
            ));
        }
    };

    Ok(interaction_http_payload(
        "payload_valid",
        json!({
            "ok": true,
            "accepted": true,
            "http_status": 200,
            "write_json_response": false,
            "response": JsonValue::Null,
            "error_kind": JsonValue::Null,
            "error": JsonValue::Null,
            "should_handle_interaction": true,
            "should_parse_payload": true,
            "signature_verified": true,
            "payload": payload,
            "interaction_type": interaction_type,
            "next_ingress_request": {
                "stage": "interaction",
                "payload": payload,
            },
            "actions": [
                {
                    "kind": "verify_interaction_signature",
                },
                {
                    "kind": "parse_interaction_payload",
                    "interaction_type": interaction_type,
                }
            ],
        }),
    ))
}

fn plan_parse_interaction_payload(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let raw_payload = clean_text(object.get("raw_payload"))
        .ok_or_else(|| "No Discord interaction payload provided.".to_string())?;
    let (payload, interaction_type) =
        parse_discord_interaction_payload(raw_payload.as_str()).map_err(|error| error.message)?;

    Ok(base_payload(
        "parse_interaction_payload",
        "payload_valid",
        json!({
            "ok": true,
            "payload": payload,
            "interaction_type": interaction_type,
            "actions": [
                {
                    "kind": "parse_interaction_payload",
                    "interaction_type": interaction_type,
                }
            ],
        }),
    ))
}

fn parse_discord_interaction_payload(
    raw_payload: &str,
) -> Result<(JsonValue, i64), DiscordPayloadError> {
    if raw_payload.trim().is_empty() {
        return Err(DiscordPayloadError::new(
            "empty_payload",
            "No Discord interaction payload provided.",
        ));
    }
    let payload =
        parse_value(raw_payload, "failed to parse Discord interaction payload").map_err(|_| {
            DiscordPayloadError::new(
                "invalid_json",
                "Discord interaction payload must be valid JSON.",
            )
        })?;
    let payload_object = payload.as_object().ok_or_else(|| {
        DiscordPayloadError::new(
            "invalid_payload",
            "Discord interaction payload must be a JSON object.",
        )
    })?;
    let interaction_type = normalize_positive_i64(payload_object.get("type")).ok_or_else(|| {
        DiscordPayloadError::new(
            "missing_type",
            "Discord interaction payload must include a numeric type.",
        )
    })?;
    Ok((payload, interaction_type))
}

fn verify_discord_interaction_signature(
    raw_payload: &str,
    signature: Option<&str>,
    timestamp: Option<&str>,
    public_key: &str,
) -> Result<(), DiscordSignatureError> {
    let Some(normalized_signature) = signature.and_then(clean_text_str) else {
        return Err(DiscordSignatureError::signature(
            "missing_signature",
            "Missing Discord interaction signature header.",
        ));
    };
    let Some(normalized_timestamp) = timestamp.and_then(clean_text_str) else {
        return Err(DiscordSignatureError::signature(
            "missing_signature_timestamp",
            "Missing Discord interaction timestamp header.",
        ));
    };
    let Some(normalized_public_key) = clean_text_str(public_key) else {
        return Err(DiscordSignatureError::config(
            "missing_public_key",
            "Missing Discord public key for interaction verification.",
        ));
    };

    let signature_bytes = decode_hex_exact(&normalized_signature, 64).map_err(|_| {
        DiscordSignatureError::signature(
            "invalid_signature_encoding",
            "Invalid Discord interaction signature encoding.",
        )
    })?;
    let public_key_bytes = decode_hex_exact(&normalized_public_key, 32).map_err(|_| {
        DiscordSignatureError::config("invalid_public_key", "Invalid Discord public key encoding.")
    })?;
    let message = format!("{normalized_timestamp}{raw_payload}");
    let public_key = UnparsedPublicKey::new(&ED25519, public_key_bytes.as_slice());
    public_key
        .verify(message.as_bytes(), signature_bytes.as_slice())
        .map_err(|_| {
            DiscordSignatureError::signature(
                "invalid_signature",
                "Invalid Discord interaction signature.",
            )
        })
}

fn plan_interaction(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let payload = payload_object(object, "interaction_payload", "payload")?;
    let interaction_type = normalize_positive_i64(payload.get("type")).unwrap_or(0);

    if interaction_type == PING_INTERACTION_TYPE {
        return Ok(base_payload(
            "interaction",
            "pong",
            json!({
                "ok": true,
                "accepted": true,
                "interaction_type": interaction_type,
                "response": {"type": PONG_RESPONSE_TYPE},
                "should_submit_turn": false,
                "should_start_background_reply": false,
                "actions": [
                    {
                        "kind": "send_interaction_pong",
                    }
                ],
            }),
        ));
    }

    if interaction_type != APPLICATION_COMMAND_INTERACTION_TYPE {
        return Ok(base_payload(
            "interaction",
            "unsupported_interaction_type",
            json!({
                "ok": true,
                "accepted": false,
                "interaction_type": interaction_type,
                "response": interaction_message_response(
                    "Unsupported Discord interaction type for the current ait Discord slice.",
                ),
                "should_submit_turn": false,
                "should_start_background_reply": false,
                "actions": [
                    {
                        "kind": "respond_to_interaction",
                        "reason": "unsupported_interaction_type",
                    }
                ],
            }),
        ));
    }

    let data = payload
        .get("data")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Discord interaction payload is missing command data.".to_string())?;
    let text = interaction_text(data);
    if text.is_empty() {
        return Ok(base_payload(
            "interaction",
            "missing_text",
            json!({
                "ok": true,
                "accepted": false,
                "interaction_type": interaction_type,
                "response": interaction_message_response("Discord command must include text content."),
                "should_submit_turn": false,
                "should_start_background_reply": false,
                "actions": [
                    {
                        "kind": "respond_to_interaction",
                        "reason": "missing_text",
                    }
                ],
            }),
        ));
    }

    let user = discord_actor_user(payload).ok_or_else(|| {
        "Discord interaction payload is missing a usable user object.".to_string()
    })?;
    let interaction_id = required_clean_text(
        payload.get("id"),
        "Discord interaction payload is missing an interaction id.",
    )?;
    let interaction_token = required_clean_text(
        payload.get("token"),
        "Discord interaction payload is missing an interaction token.",
    )?;
    let channel_id = required_clean_text(
        payload.get("channel_id"),
        "Discord interaction payload is missing a channel id.",
    )?;
    let application_id = clean_text(payload.get("application_id"))
        .or_else(|| clean_text(object.get("application_id")))
        .or_else(|| clean_text(object.get("config_application_id")))
        .unwrap_or_default();
    let guild_id = clean_text(payload.get("guild_id"));
    let channel_kind = discord_channel_kind(payload);
    let channel_title = discord_channel_title(&channel_id, guild_id.as_deref());
    let command_name = clean_text(data.get("name"));
    let command_type = normalize_positive_i64(data.get("type"));
    let source_user_id = clean_text(user.get("id"));

    if is_duplicate(
        object,
        &["duplicate", "already_processed", "processed_interaction"],
    ) {
        return Ok(base_payload(
            "interaction",
            "duplicate_ignored",
            json!({
                "ok": true,
                "accepted": false,
                "duplicate": true,
                "interaction_type": interaction_type,
                "event_id": interaction_id,
                "channel_id": channel_id,
                "response": interaction_message_response("Duplicate Discord interaction ignored."),
                "should_submit_turn": false,
                "should_start_background_reply": false,
                "actions": [],
            }),
        ));
    }

    if let Some(fresh_topic) = fresh_topic_trigger(object, &text) {
        let confirmation_text = fresh_topic_confirmation_text(&fresh_topic);
        let conversation_key = format!("discord:{channel_id}:topic:{interaction_id}");
        return Ok(base_payload(
            "interaction",
            "fresh_topic_conversation_planned",
            json!({
                "ok": true,
                "accepted": true,
                "fresh_topic": true,
                "interaction_type": interaction_type,
                "event_id": interaction_id,
                "channel_id": channel_id,
                "channel_title": channel_title,
                "channel_kind": channel_kind,
                "guild_id": optional_string_json(guild_id.as_deref()),
                "application_id": application_id,
                "source_user_id": optional_string_json(source_user_id.as_deref()),
                "conversation_key": conversation_key,
                "response": interaction_message_response(&confirmation_text),
                "should_submit_turn": false,
                "should_start_background_reply": false,
                "actions": [
                    {
                        "kind": "create_fresh_binding",
                        "channel_id": channel_id,
                        "channel_title": channel_title,
                        "channel_kind": channel_kind,
                        "source_user_id": optional_string_json(source_user_id.as_deref()),
                        "guild_id": optional_string_json(guild_id.as_deref()),
                        "application_id": application_id,
                        "rotation_reason": "fresh_topic_event_trigger",
                    },
                    {
                        "kind": "remember_interaction",
                        "channel_id": channel_id,
                        "interaction_id": interaction_id,
                        "source_user_id": optional_string_json(source_user_id.as_deref()),
                        "guild_id": optional_string_json(guild_id.as_deref()),
                        "command_name": optional_string_json(command_name.as_deref()),
                    },
                    {
                        "kind": "respond_to_interaction",
                        "response": interaction_message_response(&confirmation_text),
                    }
                ],
            }),
        ));
    }

    let conversation_key = format!("discord:{channel_id}");
    let actor_identity = discord_actor_identity(user);
    let actor_display_name = discord_actor_display_name(user);
    let actor_username = clean_text(user.get("username"));
    let actor_is_bot = optional_bool(user.get("bot"));
    let occurred_at = clean_text(object.get("occurred_at"));
    let metadata = json!({
        "command_name": optional_string_json(command_name.as_deref()),
        "guild_id": optional_string_json(guild_id.as_deref()),
        "application_id": application_id,
        "command_type": optional_i64_json(command_type),
    });
    let transport_envelope = agent_transport_event_envelope_json(
        "discord",
        &actor_identity,
        &channel_id,
        &text,
        source_user_id.as_deref(),
        actor_username.as_deref(),
        actor_display_name.as_deref(),
        actor_is_bot,
        Some(&channel_title),
        Some(&channel_kind),
        None,
        None,
        None,
        occurred_at.as_deref(),
        Some(&interaction_id),
        Some(&interaction_id),
        None,
        Some(&metadata),
    );
    let watch_spec = discord_watch_spec(
        &conversation_key,
        &channel_id,
        &channel_title,
        &channel_kind,
        &application_id,
        &interaction_id,
        "interaction",
        DISCORD_REPLY_MODE_INTERACTION,
        Some(&interaction_token),
        object,
    );
    let pending_reply = pending_discord_reply(
        &conversation_key,
        &channel_id,
        &channel_title,
        &channel_kind,
        &application_id,
        &interaction_id,
        "interaction",
        DISCORD_REPLY_MODE_INTERACTION,
        Some(&interaction_token),
        &actor_identity,
        actor_display_name.as_deref(),
        &text,
        &transport_envelope,
        source_user_id.as_deref(),
        guild_id.as_deref(),
        command_name.as_deref(),
        &watch_spec,
    );
    let defer_replies = optional_bool(object.get("defer_replies")).unwrap_or(true);
    let response = if defer_replies {
        json!({"type": DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE_TYPE})
    } else {
        interaction_message_response(
            clean_text(object.get("reply_text"))
                .unwrap_or_else(|| "(pending Discord reply)".to_string())
                .as_str(),
        )
    };

    Ok(base_payload(
        "interaction",
        "turn_submission_planned",
        json!({
            "ok": true,
            "accepted": true,
            "fresh_topic": false,
            "duplicate": false,
            "interaction_type": interaction_type,
            "event_id": interaction_id,
            "dedupe_key": interaction_id,
            "channel_id": channel_id,
            "channel_title": channel_title,
            "channel_kind": channel_kind,
            "guild_id": optional_string_json(guild_id.as_deref()),
            "application_id": application_id,
            "source_user_id": optional_string_json(source_user_id.as_deref()),
            "actor_identity": actor_identity,
            "actor_display_name": optional_string_json(actor_display_name.as_deref()),
            "text": text,
            "conversation_key": conversation_key,
            "transport_envelope": transport_envelope,
            "pending_reply": pending_reply,
            "watch_spec": watch_spec,
            "response": response,
            "should_submit_turn": true,
            "should_start_background_reply": defer_replies,
            "should_execute_pending_turn": !defer_replies,
            "actions": [
                {
                    "kind": "upsert_binding",
                    "channel_id": channel_id,
                    "channel_title": channel_title,
                    "channel_kind": channel_kind,
                    "source_user_id": optional_string_json(source_user_id.as_deref()),
                    "guild_id": optional_string_json(guild_id.as_deref()),
                    "application_id": application_id,
                },
                {
                    "kind": "remember_interaction",
                    "channel_id": channel_id,
                    "interaction_id": interaction_id,
                    "source_user_id": optional_string_json(source_user_id.as_deref()),
                    "guild_id": optional_string_json(guild_id.as_deref()),
                    "command_name": optional_string_json(command_name.as_deref()),
                },
                {
                    "kind": if defer_replies { "start_background_reply" } else { "execute_pending_turn" },
                    "pending_reply": pending_reply,
                }
            ],
        }),
    ))
}

fn plan_message(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let payload = payload_object(object, "message_payload", "payload")?;
    let author = discord_message_author(payload)
        .ok_or_else(|| "Discord message payload is missing a usable author object.".to_string())?;

    if optional_bool(author.get("bot")).unwrap_or(false) {
        return Ok(ignored_message_payload("bot_author", json!([])));
    }
    if clean_text(payload.get("webhook_id")).is_some() {
        return Ok(ignored_message_payload("webhook_message", json!([])));
    }

    let attachments = discord_message_attachments(payload);
    let mut text = discord_message_text(payload);
    if text.is_empty() && attachments.as_array().is_some_and(Vec::is_empty) {
        return Ok(ignored_message_payload("empty_text", json!([])));
    }
    if text.is_empty() {
        text = discord_attachment_message_text(&attachments);
    }

    let message_id = required_clean_text(
        payload.get("id"),
        "Discord message payload is missing a message id.",
    )?;
    let channel_id = required_clean_text(
        payload.get("channel_id"),
        "Discord message payload is missing a channel id.",
    )?;
    let application_id = clean_text(object.get("application_id"))
        .or_else(|| clean_text(object.get("config_application_id")))
        .unwrap_or_default();
    let guild_id = clean_text(payload.get("guild_id"));
    let channel_kind = discord_channel_kind(payload);
    let channel_title = discord_channel_title(&channel_id, guild_id.as_deref());
    let message_type = normalize_non_negative_i64(payload.get("type"));
    let source_user_id = clean_text(author.get("id"));

    if is_duplicate(
        object,
        &["duplicate", "already_processed", "processed_message"],
    ) {
        return Ok(base_payload(
            "message",
            "duplicate_ignored",
            json!({
                "ok": true,
                "accepted": false,
                "ignored": true,
                "duplicate": true,
                "ignore_reason": "duplicate_message",
                "event_id": message_id,
                "channel_id": channel_id,
                "should_submit_turn": false,
                "should_start_background_reply": false,
                "actions": [],
            }),
        ));
    }

    if let Some(fresh_topic) = fresh_topic_trigger(object, &text) {
        let confirmation_text = fresh_topic_confirmation_text(&fresh_topic);
        let conversation_key = format!("discord:{channel_id}:topic:{message_id}");
        return Ok(base_payload(
            "message",
            "fresh_topic_conversation_planned",
            json!({
                "ok": true,
                "accepted": true,
                "fresh_topic": true,
                "event_id": message_id,
                "channel_id": channel_id,
                "channel_title": channel_title,
                "channel_kind": channel_kind,
                "guild_id": optional_string_json(guild_id.as_deref()),
                "application_id": application_id,
                "source_user_id": optional_string_json(source_user_id.as_deref()),
                "conversation_key": conversation_key,
                "send_channel_message": {
                    "channel_id": channel_id,
                    "text": confirmation_text,
                },
                "should_submit_turn": false,
                "should_start_background_reply": false,
                "actions": [
                    {
                        "kind": "create_fresh_binding",
                        "channel_id": channel_id,
                        "channel_title": channel_title,
                        "channel_kind": channel_kind,
                        "source_user_id": optional_string_json(source_user_id.as_deref()),
                        "guild_id": optional_string_json(guild_id.as_deref()),
                        "application_id": application_id,
                        "rotation_reason": "fresh_topic_event_trigger",
                    },
                    {
                        "kind": "remember_message",
                        "channel_id": channel_id,
                        "message_id": message_id,
                        "source_user_id": optional_string_json(source_user_id.as_deref()),
                        "guild_id": optional_string_json(guild_id.as_deref()),
                    },
                    {
                        "kind": "send_channel_message",
                        "channel_id": channel_id,
                        "text": confirmation_text,
                    }
                ],
            }),
        ));
    }

    let conversation_key = format!("discord:{channel_id}");
    let actor_identity = discord_actor_identity(author);
    let actor_display_name = discord_actor_display_name(author);
    let actor_username = clean_text(author.get("username"));
    let occurred_at = clean_text(object.get("occurred_at"));
    let metadata = json!({
        "guild_id": optional_string_json(guild_id.as_deref()),
        "application_id": application_id,
        "message_type": optional_i64_json(message_type),
        "attachment_count": attachments
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
    });
    let transport_envelope = agent_transport_event_envelope_json(
        "discord",
        &actor_identity,
        &channel_id,
        &text,
        source_user_id.as_deref(),
        actor_username.as_deref(),
        actor_display_name.as_deref(),
        Some(false),
        Some(&channel_title),
        Some(&channel_kind),
        None,
        None,
        None,
        occurred_at.as_deref(),
        Some(&message_id),
        Some(&message_id),
        Some(&attachments),
        Some(&metadata),
    );
    let watch_spec = discord_watch_spec(
        &conversation_key,
        &channel_id,
        &channel_title,
        &channel_kind,
        &application_id,
        &message_id,
        "message",
        DISCORD_REPLY_MODE_CHANNEL_MESSAGE,
        None,
        object,
    );
    let pending_reply = pending_discord_reply(
        &conversation_key,
        &channel_id,
        &channel_title,
        &channel_kind,
        &application_id,
        &message_id,
        "message",
        DISCORD_REPLY_MODE_CHANNEL_MESSAGE,
        None,
        &actor_identity,
        actor_display_name.as_deref(),
        &text,
        &transport_envelope,
        source_user_id.as_deref(),
        guild_id.as_deref(),
        None,
        &watch_spec,
    );

    Ok(base_payload(
        "message",
        "turn_submission_planned",
        json!({
            "ok": true,
            "accepted": true,
            "fresh_topic": false,
            "duplicate": false,
            "event_id": message_id,
            "dedupe_key": message_id,
            "channel_id": channel_id,
            "channel_title": channel_title,
            "channel_kind": channel_kind,
            "guild_id": optional_string_json(guild_id.as_deref()),
            "application_id": application_id,
            "source_user_id": optional_string_json(source_user_id.as_deref()),
            "actor_identity": actor_identity,
            "actor_display_name": optional_string_json(actor_display_name.as_deref()),
            "text": text,
            "conversation_key": conversation_key,
            "transport_envelope": transport_envelope,
            "pending_reply": pending_reply,
            "watch_spec": watch_spec,
            "should_submit_turn": true,
            "should_start_background_reply": true,
            "actions": [
                {
                    "kind": "upsert_binding",
                    "channel_id": channel_id,
                    "channel_title": channel_title,
                    "channel_kind": channel_kind,
                    "source_user_id": optional_string_json(source_user_id.as_deref()),
                    "guild_id": optional_string_json(guild_id.as_deref()),
                    "application_id": application_id,
                },
                {
                    "kind": "remember_message",
                    "channel_id": channel_id,
                    "message_id": message_id,
                    "source_user_id": optional_string_json(source_user_id.as_deref()),
                    "guild_id": optional_string_json(guild_id.as_deref()),
                },
                {
                    "kind": "start_background_reply",
                    "pending_reply": pending_reply,
                }
            ],
        }),
    ))
}

fn discord_message_attachments(payload: &Map<String, JsonValue>) -> JsonValue {
    let attachments = payload
        .get("attachments")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_object)
        .filter_map(|attachment| {
            let file_name = clean_text(attachment.get("filename"));
            let url = clean_text(attachment.get("url"));
            if file_name.is_none() && url.is_none() {
                return None;
            }
            let mime_type = clean_text(attachment.get("content_type"));
            let kind = match mime_type.as_deref() {
                Some(value) if value.starts_with("image/") => "photo",
                Some(value) if value.starts_with("audio/") => "audio",
                Some(value) if value.starts_with("video/") => "video",
                _ => "document",
            };
            Some(json!({
                "kind": kind,
                "file_name": file_name.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "mime_type": mime_type.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "file_size_bytes": normalize_positive_i64(attachment.get("size"))
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null),
                "caption": clean_text(attachment.get("description"))
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null),
                "url": url.map(JsonValue::from).unwrap_or(JsonValue::Null),
            }))
        })
        .collect();
    JsonValue::Array(attachments)
}

fn discord_attachment_message_text(attachments: &JsonValue) -> String {
    let names = attachments
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|attachment| clean_text(attachment.get("file_name")))
        .collect::<Vec<_>>();
    if names.is_empty() {
        "Shared a Discord attachment.".to_string()
    } else {
        format!("Shared Discord attachment(s): {}", names.join(", "))
    }
}

fn ignored_message_payload(reason: &str, actions: JsonValue) -> JsonValue {
    base_payload(
        "message",
        "ignored",
        json!({
            "ok": true,
            "accepted": false,
            "ignored": true,
            "ignore_reason": reason,
            "should_submit_turn": false,
            "should_start_background_reply": false,
            "actions": actions,
        }),
    )
}

#[allow(clippy::too_many_arguments)]
fn pending_discord_reply(
    conversation_key: &str,
    channel_id: &str,
    channel_title: &str,
    channel_kind: &str,
    application_id: &str,
    event_id: &str,
    event_kind: &str,
    reply_mode: &str,
    interaction_token: Option<&str>,
    actor_identity: &str,
    actor_display_name: Option<&str>,
    text: &str,
    transport_envelope: &JsonValue,
    source_user_id: Option<&str>,
    guild_id: Option<&str>,
    command_name: Option<&str>,
    watch_spec: &JsonValue,
) -> JsonValue {
    json!({
        "conversation_key": conversation_key,
        "channel_id": channel_id,
        "channel_title": channel_title,
        "channel_kind": channel_kind,
        "application_id": application_id,
        "event_id": event_id,
        "event_kind": event_kind,
        "reply_mode": reply_mode,
        "interaction_token": optional_string_json(interaction_token),
        "actor_identity": actor_identity,
        "actor_display_name": optional_string_json(actor_display_name),
        "text": text,
        "transport_envelope": transport_envelope,
        "source_user_id": optional_string_json(source_user_id),
        "guild_id": optional_string_json(guild_id),
        "command_name": optional_string_json(command_name),
        "watch_spec": watch_spec,
    })
}

#[allow(clippy::too_many_arguments)]
fn discord_watch_spec(
    conversation_key: &str,
    channel_id: &str,
    channel_title: &str,
    channel_kind: &str,
    application_id: &str,
    event_id: &str,
    event_kind: &str,
    reply_mode: &str,
    interaction_token: Option<&str>,
    object: &Map<String, JsonValue>,
) -> JsonValue {
    json!({
        "transport": "discord",
        "conversation_key": conversation_key,
        "channel_id": channel_id,
        "channel_title": channel_title,
        "channel_kind": channel_kind,
        "application_id": application_id,
        "event_id": event_id,
        "event_kind": event_kind,
        "reply_mode": reply_mode,
        "interaction_token": optional_string_json(interaction_token),
        "recovery_attempts": normalize_non_negative_i64(object.get("recovery_attempts"))
            .unwrap_or(DEFAULT_RECOVERY_ATTEMPTS),
        "recovery_delay_seconds": object
            .get("recovery_delay_seconds")
            .cloned()
            .unwrap_or_else(|| json!(DEFAULT_RECOVERY_DELAY_SECONDS)),
        "watch_max_wait_seconds": optional_f64(object.get("watch_max_wait_seconds"))
            .unwrap_or(DEFAULT_WATCH_MAX_WAIT_SECONDS),
        "watch_poll_interval_seconds": optional_f64(object.get("watch_poll_interval_seconds"))
            .unwrap_or(DEFAULT_WATCH_POLL_INTERVAL_SECONDS),
    })
}

fn base_payload(stage: &str, state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "discord_ingress_runtime_contract".to_string(),
        JsonValue::String(INGRESS_RUNTIME_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "transport".to_string(),
        JsonValue::String("discord".to_string()),
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

fn interaction_http_payload(state: &str, payload: JsonValue) -> JsonValue {
    let mut payload = base_payload("interaction_http_request", state, payload);
    let object = payload
        .as_object_mut()
        .expect("interaction HTTP payload must be backed by an object");
    object.insert(
        "interaction_http_ingress_state".to_string(),
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
    payload
}

fn interaction_http_error_payload(
    state: &str,
    status: u16,
    error_kind: &str,
    error: impl Into<String>,
    signature_verified: bool,
) -> JsonValue {
    let error = error.into();
    interaction_http_payload(
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
            "should_handle_interaction": false,
            "should_parse_payload": false,
            "signature_verified": signature_verified,
            "payload": JsonValue::Null,
            "interaction_type": JsonValue::Null,
            "next_ingress_request": JsonValue::Null,
            "actions": [],
        }),
    )
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
        .ok_or_else(|| "Discord ingress runtime payload must be a JSON object.".to_string())
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "Discord ingress runtime request must be an object.".to_string())
}

fn required_string(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("Discord ingress runtime request requires `{field_name}`."))
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

fn clean_text_str(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn decode_hex_exact(value: &str, expected_len: usize) -> Result<Vec<u8>, ()> {
    let text = value.trim();
    if text.len() != expected_len * 2 {
        return Err(());
    }
    let mut decoded = Vec::with_capacity(expected_len);
    for pair in text.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        decoded.push((high << 4) | low);
    }
    Ok(decoded)
}

fn hex_nibble(value: u8) -> Result<u8, ()> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(()),
    }
}

#[derive(Debug)]
struct DiscordSignatureError {
    state: &'static str,
    http_status: u16,
    error_kind: &'static str,
    message: &'static str,
}

impl DiscordSignatureError {
    fn signature(state: &'static str, message: &'static str) -> Self {
        Self {
            state,
            http_status: 401,
            error_kind: "invalid_signature",
            message,
        }
    }

    fn config(state: &'static str, message: &'static str) -> Self {
        Self {
            state,
            http_status: 400,
            error_kind: "config_error",
            message,
        }
    }
}

#[derive(Debug)]
struct DiscordPayloadError {
    state: &'static str,
    message: String,
}

impl DiscordPayloadError {
    fn new(state: &'static str, message: impl Into<String>) -> Self {
        Self {
            state,
            message: message.into(),
        }
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

fn normalize_positive_i64(value: Option<&JsonValue>) -> Option<i64> {
    optional_i64(value).filter(|value| *value > 0)
}

fn normalize_non_negative_i64(value: Option<&JsonValue>) -> Option<i64> {
    optional_i64(value).filter(|value| *value >= 0)
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::String(text) => text.trim().parse::<f64>().ok(),
        JsonValue::Bool(true) => Some(1.0),
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

fn optional_i64_json(value: Option<i64>) -> JsonValue {
    value.map(JsonValue::from).unwrap_or(JsonValue::Null)
}

fn discord_actor_user(payload: &Map<String, JsonValue>) -> Option<&Map<String, JsonValue>> {
    payload
        .get("member")
        .and_then(JsonValue::as_object)
        .and_then(|member| member.get("user"))
        .and_then(JsonValue::as_object)
        .or_else(|| payload.get("user").and_then(JsonValue::as_object))
}

fn discord_message_author(payload: &Map<String, JsonValue>) -> Option<&Map<String, JsonValue>> {
    payload.get("author").and_then(JsonValue::as_object)
}

fn discord_actor_identity(user: &Map<String, JsonValue>) -> String {
    format!(
        "discord:{}",
        clean_text(user.get("id")).unwrap_or_else(|| "unknown".to_string())
    )
}

fn discord_actor_display_name(user: &Map<String, JsonValue>) -> Option<String> {
    clean_text(user.get("global_name"))
        .or_else(|| clean_text(user.get("username")))
        .or_else(|| clean_text(user.get("id")))
}

fn discord_channel_kind(payload: &Map<String, JsonValue>) -> String {
    if clean_text(payload.get("guild_id")).is_some() {
        "guild_channel".to_string()
    } else {
        "dm".to_string()
    }
}

fn discord_channel_title(channel_id: &str, guild_id: Option<&str>) -> String {
    if guild_id.is_some() {
        format!("Discord channel · {channel_id}")
    } else {
        format!("Discord DM · {channel_id}")
    }
}

fn discord_message_text(payload: &Map<String, JsonValue>) -> String {
    clean_text(payload.get("content")).unwrap_or_default()
}

fn interaction_text(data: &Map<String, JsonValue>) -> String {
    if let Some(text) = clean_text(data.get("text")) {
        return text;
    }

    let values = data
        .get("options")
        .and_then(JsonValue::as_array)
        .map(|options| flatten_string_options(options.as_slice()))
        .unwrap_or_default();
    if let Some((_, value)) = values
        .iter()
        .find(|(name, _)| name.as_deref() == Some("text"))
    {
        return value.clone();
    }
    values
        .iter()
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn flatten_string_options(options: &[JsonValue]) -> Vec<(Option<String>, String)> {
    let mut values = Vec::new();
    for option in options {
        let Some(option) = option.as_object() else {
            continue;
        };
        let name = clean_text(option.get("name"));
        if let Some(value) = clean_text(option.get("value")) {
            values.push((name.clone(), value));
        }
        if let Some(nested) = option.get("options").and_then(JsonValue::as_array) {
            values.extend(flatten_string_options(nested));
        }
    }
    values
}

fn interaction_message_response(text: &str) -> JsonValue {
    let content = text.trim();
    let content = if content.is_empty() {
        "(empty)"
    } else {
        content
    };
    json!({
        "type": CHANNEL_MESSAGE_WITH_SOURCE_TYPE,
        "data": {
            "content": content,
            "allowed_mentions": {"parse": []},
        },
    })
}

fn is_duplicate(object: &Map<String, JsonValue>, keys: &[&str]) -> bool {
    keys.iter()
        .any(|key| optional_bool(object.get(*key)).unwrap_or(false))
}

fn fresh_topic_trigger(object: &Map<String, JsonValue>, text: &str) -> Option<JsonValue> {
    if let Some(trigger) = object
        .get("fresh_topic_trigger")
        .and_then(JsonValue::as_object)
    {
        return Some(JsonValue::Object(trigger.clone()));
    }
    if let Some(config) = discord_fresh_topic_config(object) {
        if let Some(trigger) = rust_fresh_topic_trigger(text, config) {
            return Some(trigger);
        }
    }
    if text.trim() == "換個話題" {
        return Some(json!({
            "mode": "clear",
            "display_trigger": "換個話題",
        }));
    }
    None
}

fn discord_fresh_topic_config(object: &Map<String, JsonValue>) -> Option<&Map<String, JsonValue>> {
    object
        .get("fresh_topic_config")
        .or_else(|| object.get("fresh_topic"))
        .and_then(JsonValue::as_object)
        .or_else(|| {
            object
                .get("event_trigger_registry")
                .and_then(JsonValue::as_object)
                .and_then(|registry| registry.get("fresh_topic"))
                .and_then(JsonValue::as_object)
        })
}

fn rust_fresh_topic_trigger(text: &str, config: &Map<String, JsonValue>) -> Option<JsonValue> {
    let planned = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "fresh_topic_match",
        "text": text,
        "config": JsonValue::Object(config.clone()),
    }))
    .ok()?;
    planned
        .get("trigger")
        .filter(|trigger| trigger.is_object())
        .cloned()
}

fn fresh_topic_confirmation_text(fresh_topic: &JsonValue) -> String {
    let trigger = fresh_topic.as_object();
    let mode = trigger
        .and_then(|trigger| clean_text(trigger.get("mode")))
        .unwrap_or_else(|| "clear".to_string())
        .to_ascii_lowercase();
    let topic = trigger.and_then(|trigger| clean_text(trigger.get("topic")));
    let trigger_label = trigger
        .and_then(|trigger| clean_text(trigger.get("display_trigger")))
        .unwrap_or_else(|| "換個話題".to_string());
    let mut lines = vec![
        "Started a fresh Discord conversation.".to_string(),
        format!("Trigger: {trigger_label}."),
    ];
    if mode == "topic" {
        if let Some(topic) = topic {
            lines.push(format!("Topic hint: {topic}"));
        }
    }
    lines.join("\n")
}
