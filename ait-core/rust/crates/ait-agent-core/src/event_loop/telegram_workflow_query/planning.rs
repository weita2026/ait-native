use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_telegram_workflow_query";
const WORKFLOW_QUERY_CONTRACT: &str = "ait_agent_core.event_loop.TelegramWorkflowQuery.v1";

pub trait TelegramWorkflowQueryPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramWorkflowQueryPlanner;

impl TelegramWorkflowQueryPlanner for DefaultTelegramWorkflowQueryPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_workflow_query_json(request)
    }
}

pub fn agent_telegram_workflow_query_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_telegram_workflow_query_planner(&DefaultTelegramWorkflowQueryPlanner, request)
}

pub fn plan_with_telegram_workflow_query_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramWorkflowQueryPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_workflow_query_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())?;
    let kind = clean_text(object.get("kind"))
        .or_else(|| clean_text(object.get("stage")))
        .unwrap_or_else(|| "detect_workflow_query".to_string());
    match kind.as_str() {
        "parse_command" => Ok(plan_parse_command(object)),
        "detect_workflow_query" => Ok(plan_detect_workflow_query(object)),
        "chat_title" => Ok(plan_chat_title(object)),
        "actor_identity" => Ok(plan_actor_identity(object)),
        "user_display_name" => Ok(plan_user_display_name(object)),
        "message_entrypoint" => Ok(plan_message_entrypoint(object)),
        other => Err(format!(
            "unsupported Telegram workflow query plan kind `{other}`"
        )),
    }
}

fn plan_parse_command(object: &Map<String, JsonValue>) -> JsonValue {
    let raw = clean_text(object.get("text")).unwrap_or_default();
    let username = clean_text(object.get("username")).unwrap_or_default();
    let parsed = parse_command(&raw, &username);
    base_result(
        "parse_command",
        json!({
            "matched": parsed.is_some(),
            "command_name": parsed.as_ref().map(|(name, _)| name.clone()),
            "command_args": parsed.as_ref().map(|(_, args)| args.clone()),
            "command": parsed.map(|(name, args)| json!([name, args])),
        }),
    )
}

fn plan_detect_workflow_query(object: &Map<String, JsonValue>) -> JsonValue {
    let text = clean_text(object.get("text")).unwrap_or_default();
    let detected = detect_workflow_query(&text);
    base_result(
        "detect_workflow_query",
        json!({
            "matched": detected.is_some(),
            "query_kind": detected.as_ref().map(|(kind, _)| kind.clone()),
            "query_ref": detected.as_ref().and_then(|(_, reference)| reference.clone()),
            "workflow_query": detected.map(|(kind, reference)| json!([kind, reference])),
        }),
    )
}

fn plan_chat_title(object: &Map<String, JsonValue>) -> JsonValue {
    let chat = object.get("chat").and_then(JsonValue::as_object);
    base_result("chat_title", json!({ "text": chat_title(chat) }))
}

fn plan_actor_identity(object: &Map<String, JsonValue>) -> JsonValue {
    let from_user = object.get("from_user").and_then(JsonValue::as_object);
    let chat_id = clean_text(object.get("chat_id")).unwrap_or_default();
    base_result(
        "actor_identity",
        json!({ "text": actor_identity(from_user, &chat_id) }),
    )
}

fn plan_user_display_name(object: &Map<String, JsonValue>) -> JsonValue {
    let from_user = object.get("from_user").and_then(JsonValue::as_object);
    base_result(
        "user_display_name",
        json!({ "text": user_display_name(from_user) }),
    )
}

fn plan_message_entrypoint(object: &Map<String, JsonValue>) -> JsonValue {
    let raw_text = clean_text(object.get("raw_text")).unwrap_or_default();
    let username = clean_text(object.get("username")).unwrap_or_default();
    let normalized_text = clean_text(object.get("normalized_text"))
        .unwrap_or_else(|| normalize_user_text(&raw_text, &username));
    let chat = object.get("chat").and_then(JsonValue::as_object);
    let attachments_present = bool_field(object, "attachments_present")
        || object
            .get("attachments")
            .and_then(JsonValue::as_array)
            .is_some_and(|attachments| !attachments.is_empty());

    let command = if attachments_present {
        None
    } else {
        parse_command(&raw_text, &username)
    };
    let workflow_query = if attachments_present {
        None
    } else {
        detect_workflow_query(&normalized_text)
    };

    let (action_kind, dispatch_command_name, dispatch_command_args, message_text) =
        if let Some((name, args)) = command.as_ref() {
            (
                "dispatch_command",
                Some(name.clone()),
                Some(args.clone()),
                None,
            )
        } else if let Some((kind, reference)) = workflow_query.as_ref() {
            workflow_query_dispatch(kind, reference.as_deref())
        } else if normalized_text.trim().is_empty() {
            (
                "send_empty_text_help",
                None,
                None,
                Some("Send a message after the bot mention, or use /help.".to_string()),
            )
        } else {
            ("normal_text_turn", None, None, None)
        };

    base_result(
        "message_entrypoint",
        json!({
            "matched": true,
            "chat_title": chat_title(chat),
            "raw_text": raw_text,
            "normalized_text": normalized_text,
            "attachments_present": attachments_present,
            "command_name": command.as_ref().map(|(name, _)| name.clone()),
            "command_args": command.as_ref().map(|(_, args)| args.clone()).unwrap_or_default(),
            "command": command.map(|(name, args)| json!([name, args])),
            "query_kind": workflow_query.as_ref().map(|(kind, _)| kind.clone()),
            "query_ref": workflow_query.as_ref().and_then(|(_, reference)| reference.clone()),
            "workflow_query": workflow_query.map(|(kind, reference)| json!([kind, reference])),
            "action_kind": action_kind,
            "dispatch_command_name": dispatch_command_name,
            "dispatch_command_args": dispatch_command_args.unwrap_or_default(),
            "message_text": message_text,
        }),
    )
}

fn workflow_query_dispatch(
    kind: &str,
    reference: Option<&str>,
) -> (&'static str, Option<String>, Option<String>, Option<String>) {
    if matches!(kind, "queue" | "attention" | "ready") {
        return (
            "dispatch_command",
            Some(kind.to_string()),
            Some(String::new()),
            None,
        );
    }
    if matches!(kind, "task" | "audit" | "change" | "land") {
        if let Some(reference) = reference.filter(|value| !value.trim().is_empty()) {
            return (
                "dispatch_command",
                Some(kind.to_string()),
                Some(reference.to_string()),
                None,
            );
        }
    }
    ("normal_text_turn", None, None, None)
}

fn base_result(kind: &str, mut fields: JsonValue) -> JsonValue {
    let mut base = json!({
        "migration_stage": MIGRATION_STAGE,
        "workflow_query_contract": WORKFLOW_QUERY_CONTRACT,
        "kind": kind,
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_workflow_query_allowed": false,
    });
    if let (Some(base), Some(fields)) = (base.as_object_mut(), fields.as_object_mut()) {
        for (key, value) in std::mem::take(fields) {
            base.insert(key, value);
        }
    }
    base
}

fn bool_field(object: &Map<String, JsonValue>, key: &str) -> bool {
    object
        .get(key)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn parse_command(text: &str, username: &str) -> Option<(String, String)> {
    let raw = text.trim();
    if !raw.starts_with('/') {
        return None;
    }
    let (first, rest) = match raw.split_once(char::is_whitespace) {
        Some((first, rest)) => (first, rest.trim()),
        None => (raw, ""),
    };
    let mut command = first.trim_start_matches('/').trim();
    if command.is_empty() {
        return None;
    }
    if let Some((command_name, target)) = command.split_once('@') {
        if !username.is_empty() && !target.is_empty() && !target.eq_ignore_ascii_case(username) {
            return None;
        }
        command = command_name;
    }
    let command = command.trim().to_ascii_lowercase();
    (!command.is_empty()).then(|| (command, rest.to_string()))
}

fn detect_workflow_query(text: &str) -> Option<(String, Option<String>)> {
    let normalized = text.trim();
    let lowered = normalized.to_lowercase();
    if matches!(
        lowered.as_str(),
        "queue" | "task queue" | "queue summary" | "what remains" | "what should land next"
    ) {
        return Some(("queue".to_string(), None));
    }
    if matches!(
        lowered.as_str(),
        "attention" | "needs attention" | "what needs attention" | "what is blocked"
    ) {
        return Some(("attention".to_string(), None));
    }
    if matches!(
        lowered.as_str(),
        "ready" | "ready to land" | "ready to complete" | "what can land" | "what can complete"
    ) {
        return Some(("ready".to_string(), None));
    }
    if let Some(task_id) = find_workflow_id(normalized, 'T') {
        if starts_with_any(&lowered, &["task", "aitt-", "lt-", "rt-", "t-", "任務"]) {
            return Some(("task".to_string(), Some(task_id)));
        }
        if starts_with_any(&lowered, &["audit", "task audit"]) {
            return Some(("audit".to_string(), Some(task_id)));
        }
    }
    if let Some(change_id) = find_workflow_id(normalized, 'C') {
        if starts_with_any(&lowered, &["change", "aitc-", "lc-", "rc-", "c-", "變更"]) {
            return Some(("change".to_string(), Some(change_id)));
        }
        if starts_with_any(&lowered, &["land", "change land", "land readiness"]) {
            return Some(("land".to_string(), Some(change_id)));
        }
    }
    None
}

fn chat_title(chat: Option<&Map<String, JsonValue>>) -> String {
    if let Some(title) = field_text(chat, "title") {
        return title;
    }
    let first = field_text(chat, "first_name").unwrap_or_default();
    let last = field_text(chat, "last_name").unwrap_or_default();
    let username = field_text(chat, "username").unwrap_or_default();
    let full = [first.as_str(), last.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !full.is_empty() {
        return full;
    }
    if !username.is_empty() {
        return format!("@{username}");
    }
    field_text(chat, "id").unwrap_or_else(|| "telegram-chat".to_string())
}

fn actor_identity(from_user: Option<&Map<String, JsonValue>>, chat_id: &str) -> String {
    let user_id = field_text(from_user, "id").unwrap_or_else(|| chat_id.to_string());
    let username = field_text(from_user, "username").unwrap_or_default();
    if username.is_empty() {
        format!("telegram:{user_id}")
    } else {
        format!("telegram:{user_id}:@{username}")
    }
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

fn normalize_user_text(text: &str, username: &str) -> String {
    let without_mention = strip_leading_bot_mention(text, username)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    collapse_newlines(&collapse_spaces_and_tabs(&without_mention))
        .trim()
        .to_string()
}

fn strip_leading_bot_mention(text: &str, username: &str) -> String {
    let trimmed = text.trim();
    let username = username.trim();
    if username.is_empty() {
        return trimmed.to_string();
    }
    let mention = format!("@{username}");
    if !trimmed
        .get(..mention.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&mention))
    {
        return trimmed.to_string();
    }
    let suffix = &trimmed[mention.len()..];
    let mut chars = suffix.char_indices();
    let Some((_, first)) = chars.next() else {
        return trimmed.to_string();
    };
    if first.is_whitespace() {
        let end = suffix
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(suffix.len());
        return suffix[end..].to_string();
    }
    if is_mention_separator(first) {
        let end = suffix
            .char_indices()
            .find(|(_, ch)| !is_mention_separator(*ch))
            .map(|(idx, _)| idx)
            .unwrap_or(suffix.len());
        return suffix[end..].to_string();
    }
    trimmed.to_string()
}

fn is_mention_separator(ch: char) -> bool {
    matches!(ch, ':' | ',' | '-' | '，' | '：')
}

fn collapse_spaces_and_tabs(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        if matches!(ch, ' ' | '\t') {
            if !in_space {
                output.push(' ');
                in_space = true;
            }
        } else {
            output.push(ch);
            in_space = false;
        }
    }
    output
}

fn collapse_newlines(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut newline_count = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                output.push(ch);
            }
        } else {
            newline_count = 0;
            output.push(ch);
        }
    }
    output
}

fn starts_with_any(value: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| value.starts_with(prefix))
}

fn find_workflow_id(text: &str, suffix_kind: char) -> Option<String> {
    for token in workflow_tokens(text) {
        let upper = token.to_ascii_uppercase();
        let Some((prefix, tail)) = upper.split_once('-') else {
            continue;
        };
        if tail.is_empty() || !tail.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            continue;
        }
        let valid = match suffix_kind {
            'T' => matches!(prefix, "T" | "AITT" | "LT" | "RT"),
            'C' => matches!(prefix, "C" | "AITC" | "LC" | "RC"),
            _ => false,
        };
        if valid {
            return Some(upper);
        }
    }
    None
}

fn workflow_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            current.push(ch);
            continue;
        }
        if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
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

#[cfg(test)]
mod tests;
