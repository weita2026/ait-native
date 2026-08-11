use crate::json_support::{encode_value_or, parse_value};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const PRIMARY_GATE_SECTION_ORDER: [&str; 6] = [
    "attestation",
    "ci",
    "policy",
    "review",
    "freshness",
    "other",
];

pub trait TelegramWorkflowNotificationFormatter {
    fn format_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultTelegramWorkflowNotificationFormatter;

impl TelegramWorkflowNotificationFormatter for DefaultTelegramWorkflowNotificationFormatter {
    fn format_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        format_telegram_workflow_notification_json(request)
    }
}

pub fn agent_telegram_workflow_notification_format_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    format_with_telegram_workflow_notification_formatter(
        &DefaultTelegramWorkflowNotificationFormatter,
        request,
    )
}

pub fn format_with_telegram_workflow_notification_formatter<F>(
    formatter: &F,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    F: TelegramWorkflowNotificationFormatter + ?Sized,
{
    formatter.format_json(request)
}

fn format_telegram_workflow_notification_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())?;
    let kind = clean_text(object.get("kind"))
        .or_else(|| clean_text(object.get("format_kind")))
        .unwrap_or_else(|| "workflow_notification".to_string());
    let config = object.get("config").and_then(JsonValue::as_object);
    let payload = object
        .get("payload")
        .or_else(|| object.get("detail"))
        .unwrap_or(&JsonValue::Null);
    match kind.as_str() {
        "queue_digest" => {
            let payload_object = payload.as_object().unwrap_or(object);
            let digest = queue_digest(payload_object);
            Ok(json!({
                "kind": kind,
                "digest": digest,
                "actionable": queue_digest_actionable_raw(Some(&digest)),
            }))
        }
        "queue_digest_actionable" => {
            let raw = clean_text(object.get("raw")).or_else(|| clean_text(object.get("digest")));
            Ok(json!({
                "kind": kind,
                "actionable": queue_digest_actionable_raw(raw.as_deref()),
            }))
        }
        "queue_summary" => {
            let payload_object = payload.as_object().unwrap_or(object);
            Ok(text_result(
                &kind,
                format_queue_summary(config, payload_object),
            ))
        }
        "attention_summary" => {
            let payload_object = payload.as_object().unwrap_or(object);
            Ok(text_result(
                &kind,
                format_attention_summary(config, payload_object),
            ))
        }
        "ready_summary" => {
            let payload_object = payload.as_object().unwrap_or(object);
            Ok(text_result(
                &kind,
                format_ready_summary(config, payload_object),
            ))
        }
        "task_summary" => {
            let detail_object = payload.as_object().unwrap_or(object);
            Ok(text_result(
                &kind,
                format_task_summary(config, detail_object),
            ))
        }
        "change_summary" => {
            let detail_object = payload.as_object().unwrap_or(object);
            Ok(text_result(
                &kind,
                format_change_summary(config, detail_object),
            ))
        }
        "task_audit_summary" => {
            let detail_object = payload.as_object().unwrap_or(object);
            Ok(text_result(
                &kind,
                format_task_audit_summary(config, detail_object),
            ))
        }
        "change_land_summary" => {
            let detail_object = payload.as_object().unwrap_or(object);
            Ok(text_result(
                &kind,
                format_change_land_summary(config, detail_object),
            ))
        }
        "workflow_notification" => {
            let payload_object = payload.as_object().unwrap_or(object);
            Ok(text_result(
                &kind,
                format_workflow_notification(config, payload_object),
            ))
        }
        other => Err(format!(
            "unsupported Telegram workflow notification format kind `{other}`"
        )),
    }
}

fn text_result(kind: &str, text: String) -> JsonValue {
    json!({
        "kind": kind,
        "text": text,
    })
}

fn config_text(config: Option<&Map<String, JsonValue>>, field: &str) -> Option<String> {
    config.and_then(|config| clean_text(config.get(field)))
}

fn task_url(config: Option<&Map<String, JsonValue>>, task_id: Option<String>) -> Option<String> {
    let ait_web_url = config_text(config, "ait_web_url")?;
    let task_id = task_id.filter(|value| !value.trim().is_empty())?;
    Some(format!(
        "{}/tasks/{}",
        ait_web_url.trim_end_matches('/'),
        task_id
    ))
}

fn change_url(
    config: Option<&Map<String, JsonValue>>,
    change_id: Option<String>,
) -> Option<String> {
    let ait_web_url = config_text(config, "ait_web_url")?;
    let change_id = change_id.filter(|value| !value.trim().is_empty())?;
    Some(format!(
        "{}/changes/{}",
        ait_web_url.trim_end_matches('/'),
        change_id
    ))
}

fn task_queue_items(payload: &Map<String, JsonValue>, states: &[&str]) -> Vec<JsonValue> {
    let allowed: Vec<&str> = states.to_vec();
    payload
        .get("items")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    if allowed.is_empty() {
                        return true;
                    }
                    let state = item
                        .as_object()
                        .and_then(|item| item.get("workflow"))
                        .and_then(JsonValue::as_object)
                        .and_then(|workflow| clean_text(workflow.get("state")))
                        .unwrap_or_default();
                    allowed.iter().any(|allowed_state| *allowed_state == state)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Copy)]
struct TaskListStyle {
    include_state: bool,
    include_next_action: bool,
    include_updated_date: bool,
    prefer_action_code: bool,
    suppress_detail_with_action_code: bool,
}

const PROACTIVE_TASK_LIST_STYLE: TaskListStyle = TaskListStyle {
    include_state: false,
    include_next_action: true,
    include_updated_date: true,
    prefer_action_code: true,
    suppress_detail_with_action_code: true,
};

const QUERY_TASK_LIST_STYLE: TaskListStyle = TaskListStyle {
    include_state: true,
    include_next_action: true,
    include_updated_date: false,
    prefer_action_code: false,
    suppress_detail_with_action_code: false,
};

const LOCAL_CURRENT_TASK_LIST_STYLE: TaskListStyle = TaskListStyle {
    include_state: true,
    include_next_action: true,
    include_updated_date: true,
    prefer_action_code: true,
    suppress_detail_with_action_code: true,
};

fn task_updated_date(item: Option<&Map<String, JsonValue>>) -> Option<String> {
    let updated_at = clean_text(field(item, "updated_at"))?;
    let date = updated_at.get(..10)?;
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some(date.to_string())
}

fn task_next_action(
    next_action: Option<&Map<String, JsonValue>>,
    prefer_action_code: bool,
) -> Option<String> {
    let code = clean_text(field(next_action, "code"));
    let label = clean_text(field(next_action, "label"));
    if prefer_action_code {
        code.or(label)
    } else {
        label.or(code)
    }
}

fn task_list_lines(items: &[JsonValue], limit: usize, style: TaskListStyle) -> Vec<String> {
    let mut lines = Vec::new();
    for item in items.iter().take(limit) {
        let item_object = item.as_object();
        let task = object_field(item_object, "task");
        let workflow = object_field(item_object, "workflow");
        let next_action = object_field(item_object, "next_action");
        let action_code = clean_text(field(next_action, "code"));
        let detail = clean_text(field(workflow, "reason"))
            .or_else(|| clean_text(field(next_action, "detail")))
            .unwrap_or_default();
        lines.push(format!(
            "• {} · {}",
            display_field(task, "task_id"),
            display_field(task, "title")
        ));
        let mut context = Vec::new();
        if style.include_state {
            context.push(format!("state={}", display_field(workflow, "state")));
        }
        if style.include_updated_date {
            if let Some(date) = task_updated_date(item_object) {
                context.push(format!("updated={date}"));
            }
        }
        if style.include_next_action {
            let next_action = task_next_action(next_action, style.prefer_action_code)
                .unwrap_or_else(|| "inspect".to_string());
            context.push(format!("next={next_action}"));
        }
        if !context.is_empty() {
            lines.push(format!("  {}", context.join(" · ")));
        }
        if let Some(ci_summary_line) = task_ci_summary_line(item_object) {
            lines.push(format!("  {ci_summary_line}"));
        }
        if !detail.is_empty() && (!style.suppress_detail_with_action_code || action_code.is_none())
        {
            lines.push(format!("  {detail}"));
        }
    }
    if items.len() > limit {
        lines.push(format!("… and {} more", items.len() - limit));
    }
    lines
}

fn primary_gate_key(item: Option<&Map<String, JsonValue>>) -> &'static str {
    let gate = clean_text(field(item, "primary_gate"))
        .unwrap_or_default()
        .to_lowercase();
    match gate.as_str() {
        "attestation" => "attestation",
        "ci" => "ci",
        "policy" => "policy",
        "review" => "review",
        "freshness" => "freshness",
        _ => "other",
    }
}

fn primary_gate_title(gate: &str) -> &'static str {
    match gate {
        "attestation" => "Attestation",
        "ci" => "CI",
        "policy" => "Policy",
        "review" => "Review",
        "freshness" => "Stale base",
        _ => "Other blockers",
    }
}

fn tg1_count_label(summary: Option<&Map<String, JsonValue>>) -> Option<String> {
    let live_count = field(summary, "live_count");
    let minimum_count = field(summary, "minimum_count");
    if live_count.is_none() && minimum_count.is_none() {
        return None;
    }
    Some(format!(
        "{}/{}",
        live_count
            .map(display_value)
            .unwrap_or_else(|| "?".to_string()),
        minimum_count
            .map(display_value)
            .unwrap_or_else(|| "?".to_string())
    ))
}

fn task_ci_summary_line(item: Option<&Map<String, JsonValue>>) -> Option<String> {
    let ci_summary = object_field(item, "ci_summary");
    let focus_change = object_field(item, "focus_change");
    let mut parts = Vec::new();
    let patchset_id = clean_text(field(ci_summary, "patchset_id"))
        .or_else(|| clean_text(field(focus_change, "patchset_id")))
        .unwrap_or_default();
    if !patchset_id.is_empty() {
        parts.push(format!("patchset={patchset_id}"));
    }
    let tg1_required = object_field(ci_summary, "tg1_required");
    let tg1_status = clean_text(field(tg1_required, "status")).unwrap_or_default();
    if !tg1_status.is_empty() {
        let mut tg1_label = format!("TG-1={tg1_status}");
        if let Some(count_label) = tg1_count_label(tg1_required) {
            tg1_label.push(' ');
            tg1_label.push_str(&count_label);
        }
        parts.push(tg1_label);
    } else {
        let tests_status = clean_text(field(ci_summary, "tests_status")).unwrap_or_default();
        if !tests_status.is_empty() && tests_status != "not_required" {
            parts.push(format!("CI={tests_status}"));
        }
    }
    let remote_land_gate = clean_text(field(ci_summary, "remote_land_gate")).unwrap_or_default();
    if !remote_land_gate.is_empty() {
        parts.push(format!("land={remote_land_gate}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn attention_sections(
    attention_items: &[JsonValue],
    style: TaskListStyle,
) -> Vec<(String, Vec<String>)> {
    let mut sections = Vec::new();
    for gate in PRIMARY_GATE_SECTION_ORDER {
        let gate_items: Vec<JsonValue> = attention_items
            .iter()
            .filter(|item| primary_gate_key(item.as_object()) == gate)
            .cloned()
            .collect();
        if !gate_items.is_empty() {
            sections.push((
                primary_gate_title(gate).to_string(),
                task_list_lines(&gate_items, 3, style),
            ));
        }
    }
    sections
}

fn workflow_notification_body_lines(payload: &Map<String, JsonValue>) -> Vec<String> {
    if notification_source(payload).as_deref() == Some("local_current") {
        return local_current_workflow_body_lines(payload);
    }
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let attention_items = task_queue_items(payload, &["attention_required"]);
    let ready_land_items = task_queue_items(payload, &["ready_to_land"]);
    let ready_complete_items = task_queue_items(payload, &["ready_to_complete"]);
    if !attention_items.is_empty() {
        sections.extend(attention_sections(
            &attention_items,
            PROACTIVE_TASK_LIST_STYLE,
        ));
    }
    if !ready_land_items.is_empty() {
        sections.push((
            "Ready to land".to_string(),
            task_list_lines(&ready_land_items, 3, PROACTIVE_TASK_LIST_STYLE),
        ));
    }
    if !ready_complete_items.is_empty() {
        sections.push((
            "Ready to complete".to_string(),
            task_list_lines(&ready_complete_items, 3, PROACTIVE_TASK_LIST_STYLE),
        ));
    }

    let mut lines = Vec::new();
    for (index, (title, item_lines)) in sections.into_iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.push(title);
        lines.extend(item_lines);
    }
    lines
}

fn local_current_workflow_body_lines(payload: &Map<String, JsonValue>) -> Vec<String> {
    let current = task_queue_items(payload, &[])
        .into_iter()
        .take(1)
        .collect::<Vec<_>>();
    if current.is_empty() {
        return Vec::new();
    }
    let mut lines = vec!["Current workflow".to_string()];
    let mut item_lines = task_list_lines(&current, 1, LOCAL_CURRENT_TASK_LIST_STYLE);
    if let Some(change) = current[0]
        .as_object()
        .and_then(|item| object_field(Some(item), "focus_change"))
    {
        if let Some(change_id) = clean_text(field(Some(change), "change_id")) {
            let status =
                clean_text(field(Some(change), "status")).unwrap_or_else(|| "active".to_string());
            item_lines.push(format!("  change={change_id} · status={status}"));
        }
    }
    lines.extend(item_lines);
    lines
}

fn notification_source(payload: &Map<String, JsonValue>) -> Option<String> {
    clean_text(payload.get("notification_source"))
}

pub(crate) fn queue_digest(payload: &Map<String, JsonValue>) -> String {
    let body_lines = workflow_notification_body_lines(payload);
    let mut digest = json!({
        "actionable": !body_lines.is_empty(),
        "lines": body_lines,
    });
    if notification_source(payload).as_deref() == Some("local_current") {
        digest["source"] = json!("local_current");
    }
    encode_value_or(&digest, "{\"actionable\":false,\"lines\":[]}")
}

pub(crate) fn queue_digest_actionable_raw(raw: Option<&str>) -> bool {
    let Some(raw) = raw.filter(|value| !value.trim().is_empty()) else {
        return false;
    };
    parse_value(raw, "failed to parse Telegram workflow notification digest")
        .ok()
        .and_then(|payload| {
            payload
                .get("actionable")
                .and_then(|value| optional_bool(Some(value)))
        })
        .unwrap_or(false)
}

pub(crate) fn format_queue_summary(
    config: Option<&Map<String, JsonValue>>,
    payload: &Map<String, JsonValue>,
) -> String {
    let summary = object_field(Some(payload), "summary");
    let attention_items = task_queue_items(payload, &["attention_required"]);
    let ready_land_items = task_queue_items(payload, &["ready_to_land"]);
    let ready_complete_items = task_queue_items(payload, &["ready_to_complete"]);
    let other_items: Vec<JsonValue> = task_queue_items(payload, &[])
        .into_iter()
        .filter(|item| {
            let state = item
                .as_object()
                .and_then(|item| object_field(Some(item), "workflow"))
                .and_then(|workflow| clean_text(field(Some(workflow), "state")))
                .unwrap_or_default();
            !matches!(
                state.as_str(),
                "attention_required" | "ready_to_land" | "ready_to_complete"
            )
        })
        .collect();
    let mut lines = vec![
        format!(
            "ait queue · repo={}",
            config_text(config, "repo_name").unwrap_or_default()
        ),
        format!(
            "active={} attention={} ready_to_land={} ready_to_complete={}",
            display_or_default(field(summary, "active"), "0"),
            display_or_default(field(summary, "attention_required"), "0"),
            display_or_default(field(summary, "ready_to_land"), "0"),
            display_or_default(field(summary, "ready_to_complete"), "0"),
        ),
    ];
    if attention_items.is_empty()
        && ready_land_items.is_empty()
        && ready_complete_items.is_empty()
        && other_items.is_empty()
    {
        lines.push("No active tasks.".to_string());
        return lines.join("\n");
    }
    if !attention_items.is_empty() {
        for (title, item_lines) in attention_sections(&attention_items, QUERY_TASK_LIST_STYLE) {
            lines.push(String::new());
            lines.push(title);
            lines.extend(item_lines);
        }
    }
    if !ready_land_items.is_empty() {
        lines.push(String::new());
        lines.push("Ready to land".to_string());
        lines.extend(task_list_lines(&ready_land_items, 3, QUERY_TASK_LIST_STYLE));
    }
    if !ready_complete_items.is_empty() {
        lines.push(String::new());
        lines.push("Ready to complete".to_string());
        lines.extend(task_list_lines(
            &ready_complete_items,
            3,
            QUERY_TASK_LIST_STYLE,
        ));
    }
    if !other_items.is_empty() {
        lines.push(String::new());
        lines.push("Other active tasks".to_string());
        lines.extend(task_list_lines(&other_items, 2, QUERY_TASK_LIST_STYLE));
    }
    lines.join("\n")
}

pub(crate) fn format_attention_summary(
    config: Option<&Map<String, JsonValue>>,
    payload: &Map<String, JsonValue>,
) -> String {
    let items = task_queue_items(payload, &["attention_required"]);
    let mut lines = vec![
        format!(
            "ait attention · repo={}",
            config_text(config, "repo_name").unwrap_or_default()
        ),
        format!("attention={}", items.len()),
    ];
    if items.is_empty() {
        lines.push("No active tasks currently need attention.".to_string());
        return lines.join("\n");
    }
    for (title, item_lines) in attention_sections(&items, QUERY_TASK_LIST_STYLE) {
        lines.push(String::new());
        lines.push(title);
        lines.extend(item_lines);
    }
    lines.join("\n")
}

pub(crate) fn format_ready_summary(
    config: Option<&Map<String, JsonValue>>,
    payload: &Map<String, JsonValue>,
) -> String {
    let ready_land_items = task_queue_items(payload, &["ready_to_land"]);
    let ready_complete_items = task_queue_items(payload, &["ready_to_complete"]);
    let mut lines = vec![
        format!(
            "ait ready · repo={}",
            config_text(config, "repo_name").unwrap_or_default()
        ),
        format!(
            "ready_to_land={} ready_to_complete={}",
            ready_land_items.len(),
            ready_complete_items.len()
        ),
    ];
    if ready_land_items.is_empty() && ready_complete_items.is_empty() {
        lines.push("No active tasks are ready to land or complete.".to_string());
        return lines.join("\n");
    }
    if !ready_land_items.is_empty() {
        lines.push(String::new());
        lines.push("Ready to land".to_string());
        lines.extend(task_list_lines(&ready_land_items, 3, QUERY_TASK_LIST_STYLE));
    }
    if !ready_complete_items.is_empty() {
        lines.push(String::new());
        lines.push("Ready to complete".to_string());
        lines.extend(task_list_lines(
            &ready_complete_items,
            3,
            QUERY_TASK_LIST_STYLE,
        ));
    }
    lines.join("\n")
}

pub(crate) fn format_task_summary(
    config: Option<&Map<String, JsonValue>>,
    detail: &Map<String, JsonValue>,
) -> String {
    let task = object_field(Some(detail), "task");
    let workflow = object_field(Some(detail), "workflow");
    let changes = detail
        .get("changes")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let next_action = object_field(Some(detail), "next_action");
    let mut lines = vec![
        format!(
            "{} · {}",
            display_field(task, "task_id"),
            display_field(task, "title")
        ),
        format!(
            "status={} workflow={}",
            display_field(task, "status"),
            display_field(workflow, "state")
        ),
        format!("intent={}", display_field(task, "intent")),
        format!(
            "linked_changes={} · next={}",
            changes.len(),
            clean_text(field(next_action, "code")).unwrap_or_else(|| "open_task".to_string())
        ),
    ];
    if let Some(url) = task_url(config, clean_text(field(task, "task_id"))) {
        lines.push(url);
    }
    lines.join("\n")
}

pub(crate) fn format_change_summary(
    config: Option<&Map<String, JsonValue>>,
    detail: &Map<String, JsonValue>,
) -> String {
    let change = object_field(Some(detail), "change");
    let task = object_field(Some(detail), "task");
    let current_patchset = object_field(Some(detail), "current_patchset");
    let policy = object_field(Some(detail), "policy_summary");
    let reviews = object_field(Some(detail), "review_summary");
    let mut lines = vec![
        format!(
            "{} · {}",
            display_field(change, "change_id"),
            display_field(change, "title")
        ),
        format!("status={}", display_field(change, "status")),
        format!(
            "task={} · patchset={} · policy={}",
            display_field(task, "task_id"),
            clean_text(field(current_patchset, "patchset_id"))
                .unwrap_or_else(|| "none".to_string()),
            clean_text(field(policy, "decision")).unwrap_or_else(|| "pending".to_string())
        ),
        format!(
            "approvals={} blocking={} comments={}",
            display_or_default(field(reviews, "approvals"), "0"),
            display_or_default(field(reviews, "blocking"), "0"),
            display_or_default(field(reviews, "comments"), "0"),
        ),
    ];
    if let Some(url) = change_url(config, clean_text(field(change, "change_id"))) {
        lines.push(url);
    }
    lines.join("\n")
}

pub(crate) fn format_task_audit_summary(
    config: Option<&Map<String, JsonValue>>,
    detail: &Map<String, JsonValue>,
) -> String {
    let task = object_field(Some(detail), "task");
    let workflow = object_field(Some(detail), "workflow");
    let summary = object_field(Some(detail), "summary");
    let target = object_field(Some(detail), "target");
    let recommended = object_field(Some(detail), "recommended_action");
    let changes = detail
        .get("changes")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut lines = vec![
        format!(
            "{} · {}",
            display_field(task, "task_id"),
            display_field(task, "title")
        ),
        format!(
            "workflow={} verdict={} target={}",
            display_field(workflow, "state"),
            display_field(summary, "verdict"),
            clean_text(field(target, "line_name")).unwrap_or_else(|| "main".to_string())
        ),
        format!(
            "open_changes={} landed={} on_target={}",
            display_or_default(field(summary, "open_change_count"), "0"),
            display_or_default(field(summary, "landed_change_count"), "0"),
            display_or_default(field(summary, "effective_on_target_change_count"), "0"),
        ),
        format!(
            "recommended={}",
            clean_text(field(recommended, "label"))
                .or_else(|| clean_text(field(recommended, "code")))
                .unwrap_or_else(|| "inspect".to_string())
        ),
    ];
    let detail_text = clean_text(field(recommended, "detail"))
        .or_else(|| clean_text(field(workflow, "reason")))
        .unwrap_or_default();
    if !detail_text.is_empty() {
        lines.push(detail_text);
    }
    if !changes.is_empty() {
        lines.push(String::new());
        lines.push("Linked changes".to_string());
        for row in changes.iter().take(3).filter_map(JsonValue::as_object) {
            let change = object_field(Some(row), "change");
            lines.push(format!(
                "• {} · status={} · target={}",
                display_field(change, "change_id"),
                display_field(change, "status"),
                display_field(Some(row), "target_state")
            ));
        }
    }
    if let Some(url) = task_url(config, clean_text(field(task, "task_id"))) {
        lines.push(url);
    }
    lines.join("\n")
}

fn change_land_readiness(detail: &Map<String, JsonValue>) -> (String, String) {
    let change = object_field(Some(detail), "change");
    let current_patchset = object_field(Some(detail), "current_patchset");
    let policy = object_field(Some(detail), "policy_summary");
    let reviews = object_field(Some(detail), "review_summary");
    let freshness = object_field(Some(detail), "freshness");
    if clean_text(field(change, "status")).unwrap_or_default() == "landed" {
        return (
            "landed".to_string(),
            "Change is already landed.".to_string(),
        );
    }
    if current_patchset
        .map(|value| value.is_empty())
        .unwrap_or(true)
    {
        return (
            "no_patchset".to_string(),
            "Publish and select a patchset before landing.".to_string(),
        );
    }
    if !optional_bool(field(freshness, "base_is_fresh")).unwrap_or(false) {
        return (
            "stale_base".to_string(),
            "Refresh or restack onto the current base head before landing.".to_string(),
        );
    }
    if clean_text(field(change, "status")).unwrap_or_default() == "blocked"
        || optional_i64(field(reviews, "blocking")).unwrap_or(0) > 0
    {
        return (
            "blocked".to_string(),
            "Resolve blocking review feedback before landing.".to_string(),
        );
    }
    if clean_text(field(policy, "decision")).unwrap_or_else(|| "pending".to_string()) != "pass" {
        return (
            "policy_pending".to_string(),
            "Wait for required policy or validation checks to pass.".to_string(),
        );
    }
    if matches!(
        clean_text(field(change, "status"))
            .unwrap_or_default()
            .as_str(),
        "review" | "gated" | "approved" | "landable"
    ) {
        return (
            "ready_to_land".to_string(),
            "Selected patchset looks landable on the current base.".to_string(),
        );
    }
    (
        "not_ready".to_string(),
        "Move the change toward review and approval first.".to_string(),
    )
}

pub(crate) fn format_change_land_summary(
    config: Option<&Map<String, JsonValue>>,
    detail: &Map<String, JsonValue>,
) -> String {
    let change = object_field(Some(detail), "change");
    let task = object_field(Some(detail), "task");
    let current_patchset = object_field(Some(detail), "current_patchset");
    let reviews = object_field(Some(detail), "review_summary");
    let policy = object_field(Some(detail), "policy_summary");
    let freshness = object_field(Some(detail), "freshness");
    let (readiness, reason) = change_land_readiness(detail);
    let mut lines = vec![
        format!(
            "{} · {}",
            display_field(change, "change_id"),
            display_field(change, "title")
        ),
        format!(
            "land_state={} status={} task={}",
            readiness,
            display_field(change, "status"),
            display_field(task, "task_id")
        ),
        format!(
            "patchset={} policy={} base_fresh={}",
            clean_text(field(current_patchset, "patchset_id"))
                .unwrap_or_else(|| "none".to_string()),
            clean_text(field(policy, "decision")).unwrap_or_else(|| "pending".to_string()),
            optional_bool(field(freshness, "base_is_fresh")).unwrap_or(false)
        ),
        format!(
            "approvals={} blocking={} comments={}",
            display_or_default(field(reviews, "approvals"), "0"),
            display_or_default(field(reviews, "blocking"), "0"),
            display_or_default(field(reviews, "comments"), "0"),
        ),
        reason,
    ];
    if let Some(url) = change_url(config, clean_text(field(change, "change_id"))) {
        lines.push(url);
    }
    lines.join("\n")
}

pub(crate) fn format_workflow_notification(
    config: Option<&Map<String, JsonValue>>,
    payload: &Map<String, JsonValue>,
) -> String {
    let local_current = notification_source(payload).as_deref() == Some("local_current");
    let suffix = if local_current { " · local" } else { "" };
    let mut lines = vec![format!(
        "workflow ({}){suffix}",
        config_text(config, "repo_name").unwrap_or_default(),
    )];
    let body_lines = workflow_notification_body_lines(payload);
    lines.push(String::new());
    if body_lines.is_empty() {
        lines.push(if local_current {
            "No active local workflow.".to_string()
        } else {
            "Complete".to_string()
        });
    } else {
        lines.extend(body_lines);
    }
    lines.join("\n")
}

fn field<'a>(object: Option<&'a Map<String, JsonValue>>, key: &str) -> Option<&'a JsonValue> {
    object.and_then(|object| object.get(key))
}

fn object_field<'a>(
    object: Option<&'a Map<String, JsonValue>>,
    key: &str,
) -> Option<&'a Map<String, JsonValue>> {
    field(object, key).and_then(JsonValue::as_object)
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if !python_truthy(value) {
        return None;
    }
    let text = display_value(value).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn display_field(object: Option<&Map<String, JsonValue>>, key: &str) -> String {
    field(object, key)
        .map(display_value)
        .unwrap_or_else(|| "None".to_string())
}

fn display_or_default(value: Option<&JsonValue>, default: &str) -> String {
    value
        .map(display_value)
        .unwrap_or_else(|| default.to_string())
}

fn display_value(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "None".to_string(),
        JsonValue::Bool(true) => "True".to_string(),
        JsonValue::Bool(false) => "False".to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::String(value) => value.clone(),
        JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
    }
}

fn python_truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(value) => *value,
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0).unwrap_or_else(|| {
            number
                .as_u64()
                .map(|value| value != 0)
                .unwrap_or_else(|| number.as_f64().map(|value| value != 0.0).unwrap_or(true))
        }),
        JsonValue::String(value) => !value.is_empty(),
        JsonValue::Array(value) => !value.is_empty(),
        JsonValue::Object(value) => !value.is_empty(),
    }
}

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::Number(value) => value.as_i64().map(|value| value != 0),
        JsonValue::String(value) => {
            let normalized = value.trim().to_lowercase();
            match normalized.as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" | "" => Some(false),
                _ => None,
            }
        }
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().map(|value| value as i64)),
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        JsonValue::Bool(value) => Some(i64::from(*value)),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

#[cfg(test)]
mod tests;
