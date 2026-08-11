use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_telegram_owner_bootstrap";
const OWNER_BOOTSTRAP_CONTRACT: &str = "ait_agent_core.event_loop.TelegramOwnerBootstrap.v1";

pub trait TelegramOwnerBootstrapPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramOwnerBootstrapPlanner;

impl TelegramOwnerBootstrapPlanner for DefaultTelegramOwnerBootstrapPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_telegram_owner_bootstrap_json(request)
    }
}

pub fn agent_telegram_owner_bootstrap_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_telegram_owner_bootstrap_planner(&DefaultTelegramOwnerBootstrapPlanner, request)
}

pub fn plan_with_telegram_owner_bootstrap_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramOwnerBootstrapPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_telegram_owner_bootstrap_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())?;
    let kind = clean_text(object.get("kind"))
        .or_else(|| clean_text(object.get("stage")))
        .unwrap_or_else(|| "handle".to_string());
    match kind.as_str() {
        "handle" | "gate" => Ok(plan_handle(object)),
        "state_dependencies" | "dependencies" => Ok(plan_state_dependencies(object)),
        other => Err(format!(
            "unsupported Telegram owner bootstrap plan kind `{other}`"
        )),
    }
}

fn plan_state_dependencies(object: &Map<String, JsonValue>) -> JsonValue {
    if !object
        .get("owner_bootstrap_enabled")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return base_result(
            "state_dependencies",
            "disabled",
            false,
            false,
            json!({
                "load_auth_state": false,
                "load_existing_binding": false,
            }),
        );
    }

    let from_user = object.get("from_user").and_then(JsonValue::as_object);
    let user_id = field_text(from_user, "id").unwrap_or_default();
    if user_id.is_empty() {
        return base_result(
            "state_dependencies",
            "missing_user_id",
            false,
            false,
            json!({
                "load_auth_state": false,
                "load_existing_binding": false,
            }),
        );
    }

    let auth_state = object.get("auth_state").and_then(JsonValue::as_object);
    let load_existing_binding = auth_state
        .map(|auth_state| {
            existing_private_binding_lookup_can_be_requested(object, auth_state, &user_id)
        })
        .unwrap_or(false);
    let decision = if auth_state.is_none() {
        "auth_state_required"
    } else if load_existing_binding {
        "existing_private_binding_candidate"
    } else {
        "existing_private_binding_not_needed"
    };

    base_result(
        "state_dependencies",
        decision,
        false,
        false,
        json!({
            "load_auth_state": true,
            "load_existing_binding": load_existing_binding,
        }),
    )
}

fn plan_handle(object: &Map<String, JsonValue>) -> JsonValue {
    if !object
        .get("owner_bootstrap_enabled")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return base_result("handle", "disabled", false, false, json!({}));
    }

    let chat_id = clean_text(object.get("chat_id")).unwrap_or_default();
    let chat_title = clean_text(object.get("chat_title")).unwrap_or_default();
    let chat = object.get("chat").and_then(JsonValue::as_object);
    let from_user = object.get("from_user").and_then(JsonValue::as_object);
    let user_id = field_text(from_user, "id").unwrap_or_default();
    if user_id.is_empty() {
        return base_result("handle", "missing_user_id", true, true, json!({}));
    }

    let auth_state = object.get("auth_state").and_then(JsonValue::as_object);
    if let Some(owner_user_id) = field_text(auth_state, "owner_user_id") {
        let blocked = owner_user_id != user_id;
        return base_result(
            "handle",
            if blocked {
                "owner_mismatch"
            } else {
                "owner_verified"
            },
            blocked,
            blocked,
            json!({ "owner_user_id": owner_user_id }),
        );
    }

    let mut blacklist = bootstrap_blacklist(auth_state);
    if blacklist.contains_key(&user_id) {
        return base_result("handle", "blacklisted_user", true, true, json!({}));
    }

    let pending_user_id = field_text(auth_state, "pending_user_id");
    if let Some(pending_user_id) = pending_user_id.as_deref() {
        if pending_user_id != user_id {
            return base_result(
                "handle",
                "pending_other_user",
                true,
                true,
                json!({ "pending_user_id": pending_user_id }),
            );
        }
    }

    if pending_user_id.is_none()
        && existing_private_binding_can_adopt_owner(chat, object.get("existing_binding"))
    {
        let save_auth_state = compact_json(json!({
            "owner_user_id": user_id,
            "owner_username": field_text(from_user, "username"),
            "owner_display_name": user_display_name(from_user),
            "owner_chat_id": chat_id,
            "owner_chat_title": chat_title,
            "owner_chat_type": field_text(chat, "type"),
            "owner_claimed_at": now_iso(object),
            "owner_claim_reason": "existing_private_conversation_binding",
            "failed_attempts": bootstrap_failed_attempts(auth_state),
            "blacklist": blacklist,
        }))
        .unwrap_or_else(|| json!({}));
        return base_result(
            "handle",
            "adopt_existing_private_binding",
            false,
            false,
            json!({
                "adopted_owner": true,
                "save_auth_state": save_auth_state,
            }),
        );
    }

    let command_name = clean_text(object.get("command_name"))
        .or_else(|| command_name_from_command(object.get("command")));
    let message_text = clean_text(object.get("raw_text")).unwrap_or_default();
    let attachments_present = object
        .get("attachments_present")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);

    if command_name.as_deref() == Some("start") {
        let now = now_iso(object);
        let mut save_auth_state = auth_state.cloned().unwrap_or_default();
        save_auth_state.insert("pending_user_id".to_string(), json!(user_id));
        save_auth_state.insert("pending_chat_id".to_string(), json!(chat_id));
        save_auth_state.insert("pending_chat_title".to_string(), json!(chat_title));
        if pending_user_id.as_deref() != Some(user_id.as_str()) {
            save_auth_state.insert("pending_started_at".to_string(), json!(now.clone()));
        }
        save_auth_state.insert("pending_prompted_at".to_string(), json!(now));
        let save_auth_state =
            compact_json(JsonValue::Object(save_auth_state)).unwrap_or_else(|| json!({}));
        return base_result(
            "handle",
            "prompt_start",
            true,
            true,
            json!({
                "save_auth_state": save_auth_state,
                "send_message_text": prompt_text(),
            }),
        );
    }

    if pending_user_id.is_none() {
        return base_result("handle", "awaiting_start", true, true, json!({}));
    }

    if command_name.is_some() || attachments_present || message_text.is_empty() {
        return base_result(
            "handle",
            "plain_text_required",
            true,
            true,
            json!({ "send_message_text": plain_text_required_text() }),
        );
    }

    let expected_password = clean_text(object.get("expected_password"))
        .or_else(|| {
            object
                .get("config")
                .and_then(JsonValue::as_object)
                .and_then(|config| clean_text(config.get("repo_name")))
        })
        .unwrap_or_default();
    let mut failed_attempts = bootstrap_failed_attempts(auth_state);
    if message_text == expected_password {
        failed_attempts.remove(&user_id);
        let save_auth_state = compact_json(json!({
            "owner_user_id": user_id,
            "owner_username": field_text(from_user, "username"),
            "owner_display_name": user_display_name(from_user),
            "owner_chat_id": chat_id,
            "owner_chat_title": chat_title,
            "owner_chat_type": field_text(chat, "type"),
            "owner_claimed_at": now_iso(object),
            "failed_attempts": failed_attempts,
            "blacklist": blacklist,
        }))
        .unwrap_or_else(|| json!({}));
        return base_result(
            "handle",
            "owner_verified",
            true,
            true,
            json!({
                "save_auth_state": save_auth_state,
                "send_message_text": success_text(),
            }),
        );
    }

    let failures = failed_attempts
        .get(&user_id)
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        + 1;
    if failures >= 3 {
        blacklist.insert(
            user_id.clone(),
            compact_json(json!({
                "attempt_count": failures,
                "blacklisted_at": now_iso(object),
                "chat_id": chat_id,
                "chat_title": chat_title,
                "username": field_text(from_user, "username"),
                "display_name": user_display_name(from_user),
            }))
            .unwrap_or_else(|| json!({})),
        );
        failed_attempts.remove(&user_id);
        let save_auth_state = compact_json(json!({
            "failed_attempts": failed_attempts,
            "blacklist": blacklist,
        }))
        .unwrap_or_else(|| json!({}));
        return base_result(
            "handle",
            "blacklist_after_failures",
            true,
            true,
            json!({
                "save_auth_state": save_auth_state,
                "send_message_text": locked_text(),
            }),
        );
    }

    failed_attempts.insert(user_id.clone(), json!(failures));
    let mut save_auth_state = auth_state.cloned().unwrap_or_default();
    save_auth_state.insert("pending_user_id".to_string(), json!(user_id));
    save_auth_state.insert("pending_chat_id".to_string(), json!(chat_id));
    save_auth_state.insert("pending_chat_title".to_string(), json!(chat_title));
    let pending_started_at =
        field_text(auth_state, "pending_started_at").unwrap_or_else(|| now_iso(object));
    save_auth_state.insert("pending_started_at".to_string(), json!(pending_started_at));
    save_auth_state.insert("pending_prompted_at".to_string(), json!(now_iso(object)));
    save_auth_state.insert(
        "failed_attempts".to_string(),
        JsonValue::Object(failed_attempts),
    );
    save_auth_state.insert("blacklist".to_string(), JsonValue::Object(blacklist));
    let save_auth_state =
        compact_json(JsonValue::Object(save_auth_state)).unwrap_or_else(|| json!({}));
    base_result(
        "handle",
        "incorrect_password",
        true,
        true,
        json!({
            "save_auth_state": save_auth_state,
            "send_message_text": failure_text(3 - failures),
            "remaining_attempts": 3 - failures,
        }),
    )
}

fn base_result(
    kind: &str,
    decision: &str,
    handled: bool,
    blocked: bool,
    mut fields: JsonValue,
) -> JsonValue {
    let mut base = json!({
        "migration_stage": MIGRATION_STAGE,
        "owner_bootstrap_contract": OWNER_BOOTSTRAP_CONTRACT,
        "kind": kind,
        "decision": decision,
        "transport": "telegram",
        "handled": handled,
        "blocked": blocked,
        "adopted_owner": false,
        "save_auth_state": JsonValue::Null,
        "send_message_text": JsonValue::Null,
        "rust_event_loop_required": true,
        "python_owner_bootstrap_allowed": false,
    });
    if let (Some(base), Some(fields)) = (base.as_object_mut(), fields.as_object_mut()) {
        for (key, value) in std::mem::take(fields) {
            base.insert(key, value);
        }
    }
    base
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    let text = match value {
        JsonValue::String(text) => text.trim().to_string(),
        JsonValue::Null => return None,
        other => other.to_string().trim().to_string(),
    };
    (!text.is_empty()).then_some(text)
}

fn field_text(object: Option<&Map<String, JsonValue>>, key: &str) -> Option<String> {
    object.and_then(|object| clean_text(object.get(key)))
}

fn command_name_from_command(value: Option<&JsonValue>) -> Option<String> {
    match value? {
        JsonValue::Array(items) => items.first().and_then(|value| clean_text(Some(value))),
        JsonValue::Object(object) => clean_text(object.get("name")),
        JsonValue::String(text) => text.split_whitespace().next().map(str::to_string),
        _ => None,
    }
    .map(|value| value.trim().trim_start_matches('/').to_ascii_lowercase())
    .filter(|value| !value.is_empty())
}

fn bootstrap_failed_attempts(
    auth_state: Option<&Map<String, JsonValue>>,
) -> Map<String, JsonValue> {
    let mut attempts = Map::new();
    let Some(raw) = auth_state
        .and_then(|state| state.get("failed_attempts"))
        .and_then(JsonValue::as_object)
    else {
        return attempts;
    };
    for (user_id, value) in raw {
        let count = match value {
            JsonValue::Number(number) => number.as_i64(),
            JsonValue::String(text) => text.trim().parse::<i64>().ok(),
            _ => None,
        };
        if let Some(count) = count.filter(|value| *value > 0) {
            attempts.insert(user_id.clone(), json!(count));
        }
    }
    attempts
}

fn bootstrap_blacklist(auth_state: Option<&Map<String, JsonValue>>) -> Map<String, JsonValue> {
    let mut blacklist = Map::new();
    let Some(raw) = auth_state
        .and_then(|state| state.get("blacklist"))
        .and_then(JsonValue::as_object)
    else {
        return blacklist;
    };
    for (user_id, value) in raw {
        blacklist.insert(
            user_id.clone(),
            value
                .as_object()
                .map(|object| JsonValue::Object(object.clone()))
                .unwrap_or_else(|| json!({})),
        );
    }
    blacklist
}

fn existing_private_binding_can_adopt_owner(
    chat: Option<&Map<String, JsonValue>>,
    binding: Option<&JsonValue>,
) -> bool {
    if field_text(chat, "type").as_deref() != Some("private") {
        return false;
    }
    let Some(binding) = binding.and_then(JsonValue::as_object) else {
        return false;
    };
    if field_text(Some(binding), "conversation_key").is_none() {
        return false;
    }
    if let Some(linked_chat_type) = field_text(Some(binding), "surface_kind")
        .or_else(|| field_text(Some(binding), "chat_type"))
        .or_else(|| field_text(chat, "type"))
    {
        if linked_chat_type != "private" {
            return false;
        }
    }
    true
}

fn existing_private_binding_lookup_can_be_requested(
    object: &Map<String, JsonValue>,
    auth_state: &Map<String, JsonValue>,
    user_id: &str,
) -> bool {
    let chat = object.get("chat").and_then(JsonValue::as_object);
    if field_text(chat, "type").as_deref() != Some("private") {
        return false;
    }
    if field_text(Some(auth_state), "owner_user_id").is_some() {
        return false;
    }
    if field_text(Some(auth_state), "pending_user_id").is_some() {
        return false;
    }
    !bootstrap_blacklist(Some(auth_state)).contains_key(user_id)
}

fn user_display_name(from_user: Option<&Map<String, JsonValue>>) -> Option<String> {
    let first = field_text(from_user, "first_name").unwrap_or_default();
    let last = field_text(from_user, "last_name").unwrap_or_default();
    let full = [first.as_str(), last.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !full.is_empty() {
        return Some(full);
    }
    field_text(from_user, "username").map(|username| format!("@{username}"))
}

fn now_iso(object: &Map<String, JsonValue>) -> String {
    clean_text(object.get("now_iso")).unwrap_or_default()
}

fn prompt_text() -> &'static str {
    "Telegram bootstrap is locked. Send the repository-name password as plain text."
}

fn plain_text_required_text() -> &'static str {
    "Send the bootstrap password as plain text."
}

fn success_text() -> &'static str {
    "Owner verified. Telegram access is now bound to this user id. Send /help or a normal message to continue."
}

fn failure_text(remaining_attempts: i64) -> String {
    if remaining_attempts <= 1 {
        "Incorrect password. 1 attempt remaining.".to_string()
    } else {
        format!("Incorrect password. {remaining_attempts} attempts remaining.")
    }
}

fn locked_text() -> &'static str {
    "Incorrect password. This Telegram user id is now blocked until local reset clears the runtime auth state."
}

fn compact_json(value: JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null => None,
        JsonValue::Object(object) => {
            let mut compact = Map::new();
            for (key, value) in object {
                if let Some(value) = compact_json(value) {
                    compact.insert(key, value);
                }
            }
            (!compact.is_empty()).then_some(JsonValue::Object(compact))
        }
        JsonValue::Array(items) => (!items.is_empty()).then_some(JsonValue::Array(items)),
        other => Some(other),
    }
}

#[cfg(test)]
mod tests;
