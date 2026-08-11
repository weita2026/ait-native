use crate::event_loop::telegram_workflow_notifications::{
    format_attention_summary, format_change_land_summary, format_change_summary,
    format_queue_summary, format_ready_summary, format_task_audit_summary, format_task_summary,
    format_workflow_notification, queue_digest, queue_digest_actionable_raw,
};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_telegram_command_runtime";
const COMMAND_RUNTIME_CONTRACT: &str = "ait_agent_core.event_loop.TelegramCommandRuntime.v1";
const MISSING_BINDING_TEXT: &str =
    "No conversation binding exists yet. Send a message to start a Codex thread first.";
const DEFAULT_UNKNOWN_COMMAND_NAME: &str = "unknown";

#[derive(Debug, Clone, Copy)]
struct CommandSpec {
    name: &'static str,
    usage: &'static str,
    description: &'static str,
    category: &'static str,
    handler_name: &'static str,
    aliases: &'static [&'static str],
}

const COMMAND_SPECS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        usage: "/start / /help",
        description: "show workflow guidance and realistic examples",
        category: "Learn",
        handler_name: "handle_help_command",
        aliases: &["start"],
    },
    CommandSpec {
        name: "queue",
        usage: "/queue",
        description: "show the active task queue summary",
        category: "Workflow queries",
        handler_name: "handle_queue_command",
        aliases: &[],
    },
    CommandSpec {
        name: "attention",
        usage: "/attention",
        description: "show active tasks that currently need attention",
        category: "Workflow queries",
        handler_name: "handle_attention_command",
        aliases: &[],
    },
    CommandSpec {
        name: "ready",
        usage: "/ready",
        description: "show tasks that are ready to land or complete",
        category: "Workflow queries",
        handler_name: "handle_ready_command",
        aliases: &[],
    },
    CommandSpec {
        name: "task",
        usage: "/task RT-...",
        description: "show task detail for a task id",
        category: "Workflow queries",
        handler_name: "handle_task_command",
        aliases: &[],
    },
    CommandSpec {
        name: "audit",
        usage: "/audit RT-...",
        description: "show task readiness and target-line audit summary",
        category: "Workflow queries",
        handler_name: "handle_audit_command",
        aliases: &[],
    },
    CommandSpec {
        name: "change",
        usage: "/change RC-...",
        description: "show change detail for a change id",
        category: "Workflow queries",
        handler_name: "handle_change_command",
        aliases: &[],
    },
    CommandSpec {
        name: "land",
        usage: "/land RC-...",
        description: "show change land-readiness summary",
        category: "Workflow queries",
        handler_name: "handle_land_command",
        aliases: &[],
    },
    CommandSpec {
        name: "notify",
        usage: "/notify on|off|status",
        description: "toggle optional workflow queue notifications for this chat",
        category: "Workflow notifications",
        handler_name: "handle_notify_command",
        aliases: &[],
    },
    CommandSpec {
        name: "ping",
        usage: "/ping",
        description: "health check",
        category: "Utility",
        handler_name: "handle_ping_command",
        aliases: &[],
    },
];

const WORKFLOW_QUERY_EXAMPLES: &[&str] = &[
    "queue",
    "attention",
    "ready",
    "task RT-0010",
    "audit RT-0010",
    "change RC-0011",
    "land RC-0011",
    "what should land next",
];

pub trait TelegramCommandRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramCommandRuntimePlanner;

impl TelegramCommandRuntimePlanner for DefaultTelegramCommandRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_command_runtime_json(request)
    }
}

pub fn agent_telegram_command_runtime_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_telegram_command_runtime_planner(&DefaultTelegramCommandRuntimePlanner, request)
}

pub fn plan_with_telegram_command_runtime_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramCommandRuntimePlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_command_runtime_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage =
        clean_text(object.get("stage")).unwrap_or_else(|| "missing_binding_text".to_string());

    match stage.as_str() {
        "command_specs" => Ok(command_specs_plan()),
        "command_usage" => Ok(command_usage_plan(object)),
        "dispatch_command" => Ok(dispatch_command_plan(object)),
        "help_text" => Ok(help_text_plan(object)),
        "unknown_command_text" => Ok(unknown_command_text_plan(object)),
        "missing_binding_text" => Ok(text_plan(
            "missing_binding_text",
            "missing_binding",
            MISSING_BINDING_TEXT,
        )),
        "notification_status_text" => Ok(notification_status_text_plan(object)),
        "notification_enabled_text" => Ok(notification_enabled_text_plan(object)),
        "queue_summary_command" => Ok(queue_summary_command_plan(object)),
        "workflow_detail_command" => Ok(workflow_detail_command_plan(object)),
        "notify_command" => Ok(notify_command_plan(object)),
        other => Err(format!(
            "unsupported Telegram command runtime stage: {other}"
        )),
    }
}

fn command_specs_plan() -> JsonValue {
    json!({
        "stage": "command_specs",
        "migration_stage": MIGRATION_STAGE,
        "command_runtime_contract": COMMAND_RUNTIME_CONTRACT,
        "execution_kind": "telegram_command_runtime",
        "ok": true,
        "text_kind": "command_specs",
        "command_specs": default_command_specs_json(),
        "workflow_query_examples": WORKFLOW_QUERY_EXAMPLES,
        "actions": [
            {
                "kind": "command_runtime_specs",
                "command_count": COMMAND_SPECS.len(),
            }
        ],
    })
}

fn command_usage_plan(object: &Map<String, JsonValue>) -> JsonValue {
    let requested_name = clean_text(object.get("command_name"))
        .or_else(|| clean_text(object.get("name")))
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_ascii_lowercase();
    let (command_name, usage, ok) = command_spec_for_name(&requested_name)
        .map(|spec| (spec.name.to_string(), spec.usage.to_string(), true))
        .unwrap_or_else(|| (requested_name.clone(), String::new(), false));
    json!({
        "stage": "command_usage",
        "migration_stage": MIGRATION_STAGE,
        "command_runtime_contract": COMMAND_RUNTIME_CONTRACT,
        "execution_kind": "telegram_command_runtime",
        "ok": ok,
        "mode": "usage",
        "command_name": command_name,
        "requested_name": requested_name,
        "text_kind": "command_usage",
        "text": usage,
        "message_text": usage,
        "actions": [
            {
                "kind": "command_usage",
                "command_name": command_name,
                "usage": usage,
            }
        ],
    })
}

fn dispatch_command_plan(object: &Map<String, JsonValue>) -> JsonValue {
    let chat_id = chat_id_text(object.get("chat_id"));
    let requested_name = clean_text(object.get("name"))
        .or_else(|| clean_text(object.get("command_name")))
        .unwrap_or_else(|| DEFAULT_UNKNOWN_COMMAND_NAME.to_string())
        .trim_start_matches('/')
        .to_ascii_lowercase();
    let args = clean_text(object.get("args")).unwrap_or_default();

    let Some(spec) = command_spec_for_name(&requested_name) else {
        let text = unknown_command_message(
            &requested_name,
            &command_specs_from_request(object.get("command_specs")),
        );
        return dispatch_action_plan(
            "unknown",
            &requested_name,
            &requested_name,
            "",
            &args,
            "unknown_command",
            &text,
            json!([send_message_action(&chat_id, &text)]),
        );
    };

    if spec.name == "ping" {
        return dispatch_action_plan(
            "ping",
            &requested_name,
            spec.name,
            spec.handler_name,
            &args,
            "ping",
            "pong",
            json!([send_message_action(&chat_id, "pong")]),
        );
    }

    dispatch_action_plan(
        "stage",
        &requested_name,
        spec.name,
        spec.handler_name,
        &args,
        "command_stage_dispatch",
        "",
        json!([command_stage_dispatch_action(
            &chat_id,
            &requested_name,
            spec.name,
            &args
        )]),
    )
}

#[allow(clippy::too_many_arguments)]
fn dispatch_action_plan(
    mode: &str,
    requested_name: &str,
    command_name: &str,
    handler_name: &str,
    args: &str,
    text_kind: &str,
    text: &str,
    actions: JsonValue,
) -> JsonValue {
    let action_count = actions.as_array().map(Vec::len).unwrap_or(0);
    json!({
        "stage": "dispatch_command",
        "migration_stage": MIGRATION_STAGE,
        "command_runtime_contract": COMMAND_RUNTIME_CONTRACT,
        "execution_kind": "telegram_command_runtime",
        "ok": true,
        "mode": mode,
        "requested_name": requested_name,
        "command_name": command_name,
        "handler_name": handler_name,
        "args": args,
        "text_kind": text_kind,
        "text": text,
        "message_text": text,
        "actions": actions,
        "action_count": action_count,
    })
}

fn command_stage_dispatch_action(
    chat_id: &str,
    requested_name: &str,
    command_name: &str,
    args: &str,
) -> JsonValue {
    let mut stage_request = Map::new();
    stage_request.insert("chat_id".to_string(), json!(chat_id));

    let mut binding_policy = "none";
    let mut include_chat = false;
    let mut include_username = false;
    let include_config = true;
    let mut include_observed_at = false;
    let mut target_ref_query: Option<String> = None;
    let mut target_ref_expected_kind: Option<String> = None;

    let stage = match command_name {
        "help" => {
            binding_policy = "read_existing";
            include_chat = true;
            include_username = true;
            "help_text"
        }
        "queue" | "attention" | "ready" => {
            stage_request.insert("summary_kind".to_string(), json!(command_name));
            "queue_summary_command"
        }
        "task" | "audit" | "change" | "land" => {
            stage_request.insert("command_name".to_string(), json!(command_name));
            target_ref_query = Some(workflow_detail_dispatch_query(command_name, args));
            target_ref_expected_kind = Some(command_name.to_string());
            "workflow_detail_command"
        }
        "notify" => {
            binding_policy = "read_existing";
            include_observed_at = true;
            stage_request.insert("args".to_string(), json!(args));
            "notify_command"
        }
        _ => "missing_binding_text",
    };

    stage_request.insert("stage".to_string(), json!(stage));

    json!({
        "kind": "run_command_runtime_stage",
        "command_name": command_name,
        "requested_name": requested_name,
        "args": args,
        "stage": stage,
        "stage_request": JsonValue::Object(stage_request),
        "binding_policy": binding_policy,
        "include_chat": include_chat,
        "include_username": include_username,
        "include_config": include_config,
        "include_observed_at": include_observed_at,
        "target_ref_query": target_ref_query,
        "target_ref_expected_kind": target_ref_expected_kind,
    })
}

fn workflow_detail_dispatch_query(command_name: &str, args: &str) -> String {
    let args = args.trim();
    if args.is_empty() {
        command_name.to_string()
    } else {
        format!("{command_name} {args}")
    }
}

fn help_text_plan(object: &Map<String, JsonValue>) -> JsonValue {
    let chat_type = object
        .get("chat")
        .and_then(JsonValue::as_object)
        .and_then(|chat| clean_text(chat.get("type")));
    let username = clean_text(object.get("username"));
    let binding = object.get("binding").and_then(JsonValue::as_object);
    let command_specs = command_specs_from_request(object.get("command_specs"));
    let workflow_query_examples = string_list_from_value(object.get("workflow_query_examples"))
        .unwrap_or_else(|| {
            WORKFLOW_QUERY_EXAMPLES
                .iter()
                .map(|item| item.to_string())
                .collect()
        });

    let mut lines = vec![
        "ait Telegram bot".to_string(),
        "".to_string(),
        "Thin Telegram gateway for Codex conversations and ait workflow queries.".to_string(),
        "".to_string(),
        "Conversation history stays in the bound Codex thread; the gateway stores no transcript."
            .to_string(),
        "Runtime-only state keeps the Telegram conversation key, Codex thread binding, delivery cursor, and notification settings."
            .to_string(),
        "Optional workflow notifications are scoped per conversation binding.".to_string(),
    ];

    let mut current_category: Option<String> = None;
    for spec in command_specs {
        if current_category.as_deref() != Some(spec.category.as_str()) {
            lines.push(String::new());
            lines.push(spec.category.clone());
            current_category = Some(spec.category);
        }
        lines.push(format!("{} - {}", spec.usage, spec.description));
    }

    lines.push(String::new());
    lines.push("Workflow query examples".to_string());
    lines.extend(workflow_query_examples);

    if matches!(chat_type.as_deref(), Some("group" | "supergroup")) {
        let mention_target = username
            .filter(|value| !value.is_empty())
            .map(|value| format!("@{value}"))
            .unwrap_or_else(|| "@your_bot".to_string());
        lines.push(String::new());
        lines.push(format!(
            "Group chat tip: start free text with {mention_target} so the bot knows the turn is for ait."
        ));
        lines.push(format!(
            "Example: {mention_target} summarize what should land next"
        ));
    }

    lines.push(String::new());
    lines.push(
        "Any other text is sent directly to Codex and resumes the bound Codex thread when available."
            .to_string(),
    );
    lines.push(String::new());
    lines.push("Current conversation binding".to_string());
    lines.push(if binding.is_some() {
        "Ready; ait-server is not required for replies.".to_string()
    } else {
        MISSING_BINDING_TEXT.to_string()
    });

    let text = lines.join("\n");
    let chat_id = chat_id_text(object.get("chat_id"));
    if chat_id.is_empty() {
        text_plan("help_text", "help", &text)
    } else {
        text_with_actions_plan(
            "help_text",
            "help",
            &text,
            json!([send_message_action(&chat_id, &text)]),
        )
    }
}

fn unknown_command_text_plan(object: &Map<String, JsonValue>) -> JsonValue {
    let name =
        clean_text(object.get("name")).unwrap_or_else(|| DEFAULT_UNKNOWN_COMMAND_NAME.to_string());
    let specs = command_specs_from_request(object.get("command_specs"));
    text_plan(
        "unknown_command_text",
        "unknown_command",
        &unknown_command_message(&name, &specs),
    )
}

fn notification_status_text_plan(object: &Map<String, JsonValue>) -> JsonValue {
    let text = notification_status_text(object);
    text_plan("notification_status_text", "notification_status", &text)
}

fn notification_status_text(object: &Map<String, JsonValue>) -> String {
    let config = object.get("config").and_then(JsonValue::as_object);
    let binding = object.get("binding").and_then(JsonValue::as_object);
    let notification_mode_label = clean_text(object.get("notification_mode_label"))
        .unwrap_or_else(|| notification_mode_label(config, binding));
    let last_queue_notification_at =
        binding.and_then(|binding| clean_text(binding.get("last_queue_notification_at")));
    let background_sync_enabled = bool_value(object.get("background_sync_enabled"))
        || bool_value(config.and_then(|config| config.get("background_sync_enabled")));
    let mut lines = vec![format!("workflow_notifications={notification_mode_label}")];
    if let Some(value) = last_queue_notification_at {
        lines.push(format!("last_queue_notification_at={value}"));
    } else {
        lines.push("No workflow queue notification delivered yet.".to_string());
    }
    if !background_sync_enabled {
        lines.push(
            "Background sync is disabled, so automatic delivery is currently paused.".to_string(),
        );
    }
    lines.join("\n")
}

fn notification_enabled_text_plan(object: &Map<String, JsonValue>) -> JsonValue {
    let text = notification_enabled_text(object);
    text_plan("notification_enabled_text", "notification_enabled", &text)
}

fn notification_enabled_text(object: &Map<String, JsonValue>) -> String {
    let background_sync_enabled = bool_value(object.get("background_sync_enabled"));
    let workflow_notification_text = clean_text(object.get("workflow_notification_text"));
    let mut lines = vec!["Workflow notifications enabled for this chat.".to_string()];
    if background_sync_enabled {
        lines.push(
            "Background sync is active, so queue updates can arrive automatically.".to_string(),
        );
    } else {
        lines.push(
            "Background sync is currently disabled, so automatic delivery will wait until it is enabled."
                .to_string(),
        );
    }
    if let Some(text) = workflow_notification_text {
        lines.push(String::new());
        lines.push(text);
    } else {
        lines.push("Complete".to_string());
    }
    lines.join("\n")
}

fn queue_summary_command_plan(object: &Map<String, JsonValue>) -> JsonValue {
    let chat_id = chat_id_text(object.get("chat_id"));
    let requested_kind = clean_text(object.get("summary_kind"))
        .or_else(|| clean_text(object.get("command_name")))
        .unwrap_or_else(|| "queue".to_string())
        .to_ascii_lowercase();
    let summary_kind = match requested_kind.as_str() {
        "queue" | "queue_summary" => "queue",
        "attention" | "attention_summary" => "attention",
        "ready" | "ready_summary" => "ready",
        _ => {
            let usage = clean_text(object.get("usage"))
                .unwrap_or_else(|| "/queue | /attention | /ready".to_string());
            let text = format!("Usage: {usage}");
            return queue_summary_action_plan(
                "usage",
                "queue_summary_usage",
                &text,
                json!([send_message_action(&chat_id, &text)]),
                false,
            );
        }
    };

    let Some(queue_payload) = object
        .get("queue_payload")
        .or_else(|| object.get("payload"))
        .and_then(JsonValue::as_object)
    else {
        let pending_text_kind = format!("{summary_kind}_summary_pending_queue");
        return queue_summary_action_plan(
            summary_kind,
            &pending_text_kind,
            "",
            json!([{"kind": "read_task_queue"}]),
            true,
        );
    };

    let config = object.get("config").and_then(JsonValue::as_object);
    let (text_kind, text) = match summary_kind {
        "attention" => (
            "attention_summary",
            format_attention_summary(config, queue_payload),
        ),
        "ready" => ("ready_summary", format_ready_summary(config, queue_payload)),
        _ => ("queue_summary", format_queue_summary(config, queue_payload)),
    };
    queue_summary_action_plan(
        summary_kind,
        text_kind,
        &text,
        json!([send_message_action(&chat_id, &text)]),
        false,
    )
}

fn queue_summary_action_plan(
    mode: &str,
    text_kind: &str,
    text: &str,
    actions: JsonValue,
    needs_queue: bool,
) -> JsonValue {
    let action_count = actions.as_array().map(Vec::len).unwrap_or(0);
    json!({
        "stage": "queue_summary_command",
        "migration_stage": MIGRATION_STAGE,
        "command_runtime_contract": COMMAND_RUNTIME_CONTRACT,
        "execution_kind": "telegram_command_runtime",
        "ok": true,
        "mode": mode,
        "summary_kind": mode,
        "text_kind": text_kind,
        "text": text,
        "message_text": text,
        "needs_queue": needs_queue,
        "queue_request": if needs_queue { json!({"kind": "read_task_queue"}) } else { JsonValue::Null },
        "queue_request_count": if needs_queue { 1 } else { 0 },
        "actions": actions,
        "action_count": action_count,
    })
}

fn workflow_detail_command_plan(object: &Map<String, JsonValue>) -> JsonValue {
    let chat_id = chat_id_text(object.get("chat_id"));
    let requested_name = clean_text(object.get("command_name"))
        .unwrap_or_else(|| "task".to_string())
        .to_ascii_lowercase();
    let command_name = match requested_name.as_str() {
        "task" | "audit" | "change" | "land" => requested_name.as_str(),
        _ => {
            let text = "Usage: /task RT-... | /audit RT-... | /change RC-... | /land RC-...";
            return workflow_detail_action_plan(
                "usage",
                "",
                "workflow_detail_usage",
                text,
                json!([send_message_action(&chat_id, text)]),
                false,
            );
        }
    };
    let usage = clean_text(object.get("usage"))
        .or_else(|| {
            command_usage(
                &command_specs_from_request(object.get("command_specs")),
                command_name,
            )
        })
        .unwrap_or_else(|| default_workflow_detail_usage(command_name).to_string());
    let Some(target_ref) = clean_text(object.get("target_ref")) else {
        let text = format!("Usage: {usage}");
        return workflow_detail_action_plan(
            command_name,
            "",
            "workflow_detail_usage",
            &text,
            json!([send_message_action(&chat_id, &text)]),
            false,
        );
    };
    let Some(detail_payload) = object
        .get("detail_payload")
        .or_else(|| object.get("payload"))
        .and_then(JsonValue::as_object)
    else {
        let pending_text_kind = format!("{command_name}_detail_pending_read");
        return workflow_detail_action_plan(
            command_name,
            &target_ref,
            &pending_text_kind,
            "",
            json!([{
                "kind": workflow_detail_read_action_kind(command_name),
                "target_ref": target_ref,
            }]),
            true,
        );
    };

    let config = object.get("config").and_then(JsonValue::as_object);
    let (text_kind, text) = match command_name {
        "audit" => (
            "task_audit_summary",
            format_task_audit_summary(config, detail_payload),
        ),
        "change" => (
            "change_summary",
            format_change_summary(config, detail_payload),
        ),
        "land" => (
            "change_land_summary",
            format_change_land_summary(config, detail_payload),
        ),
        _ => ("task_summary", format_task_summary(config, detail_payload)),
    };
    workflow_detail_action_plan(
        command_name,
        &target_ref,
        text_kind,
        &text,
        json!([send_message_action(&chat_id, &text)]),
        false,
    )
}

fn workflow_detail_action_plan(
    mode: &str,
    target_ref: &str,
    text_kind: &str,
    text: &str,
    actions: JsonValue,
    needs_detail: bool,
) -> JsonValue {
    let action_count = actions.as_array().map(Vec::len).unwrap_or(0);
    json!({
        "stage": "workflow_detail_command",
        "migration_stage": MIGRATION_STAGE,
        "command_runtime_contract": COMMAND_RUNTIME_CONTRACT,
        "execution_kind": "telegram_command_runtime",
        "ok": true,
        "mode": mode,
        "command_name": mode,
        "target_ref": target_ref,
        "text_kind": text_kind,
        "text": text,
        "message_text": text,
        "needs_detail": needs_detail,
        "detail_request_count": if needs_detail { 1 } else { 0 },
        "actions": actions,
        "action_count": action_count,
    })
}

fn workflow_detail_read_action_kind(command_name: &str) -> &'static str {
    match command_name {
        "audit" => "read_task_audit",
        "change" => "read_change",
        "land" => "read_change_land",
        _ => "read_task",
    }
}

fn default_workflow_detail_usage(command_name: &str) -> &'static str {
    match command_name {
        "audit" => "/audit RT-...",
        "change" => "/change RC-...",
        "land" => "/land RC-...",
        _ => "/task RT-...",
    }
}

fn notify_command_plan(object: &Map<String, JsonValue>) -> JsonValue {
    let chat_id = chat_id_text(object.get("chat_id"));
    let binding = object.get("binding").and_then(JsonValue::as_object);
    let mode = clean_text(object.get("args"))
        .unwrap_or_else(|| "status".to_string())
        .to_ascii_lowercase();
    let usage = clean_text(object.get("usage"))
        .or_else(|| {
            command_usage(
                &command_specs_from_request(object.get("command_specs")),
                "notify",
            )
        })
        .unwrap_or_else(|| "/notify [on|off|status]".to_string());

    if binding.is_none() {
        return notify_action_plan(
            "missing_binding",
            "missing_binding",
            MISSING_BINDING_TEXT,
            json!([send_message_action(&chat_id, MISSING_BINDING_TEXT)]),
            false,
            JsonValue::Null,
            false,
        );
    }

    if matches!(mode.as_str(), "status" | "show") {
        let text = notification_status_text(object);
        return notify_action_plan(
            "status",
            "notification_status",
            &text,
            json!([send_message_action(&chat_id, &text)]),
            false,
            JsonValue::Null,
            false,
        );
    }

    if matches!(mode.as_str(), "on" | "enable" | "enabled") {
        let Some(queue_payload) = object
            .get("queue_payload")
            .or_else(|| object.get("payload"))
            .and_then(JsonValue::as_object)
        else {
            return notify_action_plan(
                "on",
                "notification_enable_pending_queue",
                "",
                json!([{"kind": "read_task_queue"}]),
                true,
                JsonValue::Null,
                false,
            );
        };
        let digest = queue_digest(queue_payload);
        let actionable = queue_digest_actionable_raw(Some(&digest));
        let observed_at = clean_text(object.get("observed_at"));
        let mut patch = Map::new();
        patch.insert("workflow_notifications_enabled".to_string(), json!(true));
        patch.insert("last_queue_summary_digest".to_string(), json!(digest));
        patch.insert(
            "last_queue_notification_at".to_string(),
            if actionable {
                observed_at.map(JsonValue::from).unwrap_or(JsonValue::Null)
            } else {
                JsonValue::Null
            },
        );

        let config = object.get("config").and_then(JsonValue::as_object);
        let mut text_object = object.clone();
        if actionable {
            text_object.insert(
                "workflow_notification_text".to_string(),
                JsonValue::String(format_workflow_notification(config, queue_payload)),
            );
        } else {
            text_object.remove("workflow_notification_text");
        }
        let text = notification_enabled_text(&text_object);
        return notify_action_plan(
            "on",
            "notification_enabled",
            &text,
            json!([
                patch_chat_action(&chat_id, JsonValue::Object(patch)),
                send_message_action(&chat_id, &text),
            ]),
            false,
            JsonValue::Null,
            actionable,
        );
    }

    if matches!(mode.as_str(), "off" | "disable" | "disabled") {
        let patch = json!({
            "workflow_notifications_enabled": false,
            "last_queue_summary_digest": JsonValue::Null,
            "last_queue_notification_at": JsonValue::Null,
        });
        let text = "Workflow notifications disabled for this chat.";
        return notify_action_plan(
            "off",
            "notification_disabled",
            text,
            json!([
                patch_chat_action(&chat_id, patch),
                send_message_action(&chat_id, text),
            ]),
            false,
            JsonValue::Null,
            false,
        );
    }

    let text = format!("Usage: {usage}");
    notify_action_plan(
        "usage",
        "notify_usage",
        &text,
        json!([send_message_action(&chat_id, &text)]),
        false,
        JsonValue::Null,
        false,
    )
}

fn notify_action_plan(
    mode: &str,
    text_kind: &str,
    text: &str,
    actions: JsonValue,
    needs_queue: bool,
    queue_request: JsonValue,
    queue_digest_actionable: bool,
) -> JsonValue {
    let action_count = actions.as_array().map(Vec::len).unwrap_or(0);
    json!({
        "stage": "notify_command",
        "migration_stage": MIGRATION_STAGE,
        "command_runtime_contract": COMMAND_RUNTIME_CONTRACT,
        "execution_kind": "telegram_command_runtime",
        "ok": true,
        "mode": mode,
        "text_kind": text_kind,
        "text": text,
        "message_text": text,
        "needs_queue": needs_queue,
        "queue_request": queue_request,
        "queue_request_count": if needs_queue { 1 } else { 0 },
        "queue_digest_actionable": queue_digest_actionable,
        "actions": actions,
        "action_count": action_count,
    })
}

fn patch_chat_action(chat_id: &str, patch: JsonValue) -> JsonValue {
    json!({
        "kind": "patch_chat",
        "chat_id": chat_id,
        "patch": patch,
    })
}

fn send_message_action(chat_id: &str, text: &str) -> JsonValue {
    json!({
        "kind": "send_message",
        "chat_id": chat_id,
        "message_text": text,
    })
}

fn chat_id_text(value: Option<&JsonValue>) -> String {
    match value {
        Some(JsonValue::String(value)) => value.trim().to_string(),
        Some(JsonValue::Number(value)) => value.to_string(),
        Some(JsonValue::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn text_plan(stage: &str, text_kind: &str, text: &str) -> JsonValue {
    text_with_actions_plan(
        stage,
        text_kind,
        text,
        json!([
            {
                "kind": "command_runtime_text",
                "text_kind": text_kind,
                "text": text,
            }
        ]),
    )
}

fn text_with_actions_plan(
    stage: &str,
    text_kind: &str,
    text: &str,
    actions: JsonValue,
) -> JsonValue {
    let action_count = actions.as_array().map(Vec::len).unwrap_or(0);
    json!({
        "stage": stage,
        "migration_stage": MIGRATION_STAGE,
        "command_runtime_contract": COMMAND_RUNTIME_CONTRACT,
        "execution_kind": "telegram_command_runtime",
        "ok": true,
        "text_kind": text_kind,
        "text": text,
        "message_text": text,
        "actions": actions,
        "action_count": action_count,
    })
}

#[derive(Debug, Clone)]
struct OwnedCommandSpec {
    name: String,
    usage: String,
    description: String,
    category: String,
}

fn default_command_specs_json() -> Vec<JsonValue> {
    COMMAND_SPECS
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "usage": spec.usage,
                "description": spec.description,
                "category": spec.category,
                "handler_name": spec.handler_name,
                "aliases": spec.aliases,
            })
        })
        .collect()
}

fn command_specs_from_request(value: Option<&JsonValue>) -> Vec<OwnedCommandSpec> {
    let Some(items) = value.and_then(JsonValue::as_array) else {
        return COMMAND_SPECS
            .iter()
            .map(|spec| OwnedCommandSpec {
                name: spec.name.to_string(),
                usage: spec.usage.to_string(),
                description: spec.description.to_string(),
                category: spec.category.to_string(),
            })
            .collect();
    };

    let parsed = items
        .iter()
        .filter_map(JsonValue::as_object)
        .filter_map(|object| {
            Some(OwnedCommandSpec {
                name: clean_text(object.get("name"))?,
                usage: clean_text(object.get("usage"))?,
                description: clean_text(object.get("description"))?,
                category: clean_text(object.get("category"))?,
            })
        })
        .collect::<Vec<_>>();
    if parsed.is_empty() {
        command_specs_from_request(None)
    } else {
        parsed
    }
}

fn command_usage(specs: &[OwnedCommandSpec], name: &str) -> Option<String> {
    specs
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.usage.clone())
}

fn command_spec_for_name(name: &str) -> Option<&'static CommandSpec> {
    COMMAND_SPECS
        .iter()
        .find(|spec| spec.name == name || spec.aliases.contains(&name))
}

fn unknown_command_message(name: &str, specs: &[OwnedCommandSpec]) -> String {
    let suggestions = [
        "help",
        "queue",
        "attention",
        "ready",
        "task",
        "change",
        "notify",
    ]
    .into_iter()
    .filter_map(|name| command_usage(specs, name))
    .collect::<Vec<_>>();
    format!(
        "Unknown command /{name}. Send /help for examples like {}.",
        suggestions.join(", ")
    )
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "Telegram command runtime request must be an object.".to_string())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = value?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn string_list_from_value(value: Option<&JsonValue>) -> Option<Vec<String>> {
    let values = value?
        .as_array()?
        .iter()
        .filter_map(|item| clean_text(Some(item)))
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn bool_value(value: Option<&JsonValue>) -> bool {
    value.and_then(JsonValue::as_bool).unwrap_or(false)
}

fn notification_mode_label(
    config: Option<&Map<String, JsonValue>>,
    binding: Option<&Map<String, JsonValue>>,
) -> String {
    let enabled = binding
        .and_then(|value| value.get("workflow_notifications_enabled"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let background_enabled = config
        .and_then(|value| value.get("background_sync_enabled"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    match (enabled, background_enabled) {
        (true, true) => "enabled".to_string(),
        (true, false) => "enabled (background delivery paused)".to_string(),
        (false, _) => "disabled".to_string(),
    }
}

#[cfg(test)]
mod tests;
