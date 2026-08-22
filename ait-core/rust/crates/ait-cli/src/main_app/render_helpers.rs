use crate::json_support::encode_value_or;

fn string_field(value: Option<&JsonValue>) -> String {
    match value {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::String(text)) => text.clone(),
        Some(JsonValue::Bool(flag)) => flag.to_string(),
        Some(JsonValue::Number(number)) => number.to_string(),
        Some(other) => encode_value_or(other, ""),
    }
}

fn string_field_or_default(value: Option<&JsonValue>, default: &str) -> String {
    let rendered = string_field(value);
    if rendered.is_empty() {
        default.to_string()
    } else {
        rendered
    }
}

const DEFAULT_AGENT_TEXT_LIST_LIMIT: usize = 20;
const AGENT_ACTION_JSON_CONTRACT: &str = "ait-agent-action/v1";

fn cloned_field(payload: &JsonValue, field: &str) -> JsonValue {
    payload.get(field).cloned().unwrap_or(JsonValue::Null)
}

fn compact_status_payload(payload: &JsonValue) -> JsonValue {
    let changed_count = payload
        .get("workspace_changed_count")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let next_action = if changed_count > 0 {
        json!({
            "code": "inspect_workspace",
            "command": "ait diff",
        })
    } else {
        let command = payload
            .get("reconciliation")
            .map(|reconciliation| {
                let safe = reconciliation
                    .get("safe_finding_count")
                    .and_then(JsonValue::as_i64)
                    .unwrap_or(0);
                let manual = reconciliation
                    .get("manual_resolution_count")
                    .and_then(JsonValue::as_i64)
                    .unwrap_or(0);
                let protected = reconciliation
                    .get("protected_count")
                    .and_then(JsonValue::as_i64)
                    .unwrap_or(0);
                status_reconciliation_next(reconciliation, safe, manual, protected)
            })
            .unwrap_or_default();
        if command.is_empty() {
            JsonValue::Null
        } else {
            json!({
                "code": "reconcile",
                "command": command,
            })
        }
    };
    let worktree = if payload
        .get("is_worktree")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        json!({
            "name": cloned_field(payload, "worktree_name"),
        })
    } else {
        JsonValue::Null
    };
    json!({
        "contract": AGENT_ACTION_JSON_CONTRACT,
        "command": "status",
        "ok": true,
        "repo_name": cloned_field(payload, "repo_name"),
        "line_name": cloned_field(payload, "current_line"),
        "head_snapshot_id": cloned_field(payload, "head_snapshot_id"),
        "workspace": {
            "status": cloned_field(payload, "workspace_status"),
            "dirty": cloned_field(payload, "workspace_dirty"),
            "changed_count": cloned_field(payload, "workspace_changed_count"),
            "modified_count": cloned_field(payload, "workspace_modified_count"),
            "missing_count": cloned_field(payload, "workspace_missing_count"),
            "untracked_count": cloned_field(payload, "workspace_untracked_count"),
        },
        "worktree": worktree,
        "next_action": next_action,
    })
}

fn shell_quote_text(text: &str) -> String {
    if text
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '/' | '-' | '_' | '.' | ':'))
    {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\"'\"'"))
    }
}

fn compact_task_start_payload(payload: &JsonValue) -> JsonValue {
    let worktree = payload.get("worktree").unwrap_or(&JsonValue::Null);
    let edit_root = worktree.get("path").and_then(JsonValue::as_str);
    let next_action = edit_root.map_or(JsonValue::Null, |path| {
        json!({
            "code": "enter_worktree",
            "command": format!("cd {}", shell_quote_text(path)),
        })
    });
    let change_ref = payload
        .get("change")
        .map(task_scoped_change_ref)
        .unwrap_or_default();
    json!({
        "contract": AGENT_ACTION_JSON_CONTRACT,
        "command": "task.start",
        "ok": true,
        "task_id": cloned_field(payload, "task_id"),
        "change_ref": change_ref,
        "line_name": cloned_field(worktree, "current_line"),
        "head_snapshot_id": cloned_field(worktree, "head_snapshot_id"),
        "worktree_name": cloned_field(worktree, "name"),
        "edit_root": edit_root,
        "next_action": next_action,
    })
}

fn compact_snapshot_create_payload(payload: &JsonValue) -> JsonValue {
    json!({
        "contract": AGENT_ACTION_JSON_CONTRACT,
        "command": "snapshot.create",
        "ok": true,
        "snapshot_id": cloned_field(payload, "snapshot_id"),
        "line_name": cloned_field(payload, "line_name"),
        "parent_snapshot_id": cloned_field(payload, "parent_snapshot_id"),
        "message": cloned_field(payload, "message"),
    })
}

fn compact_nested_status(payload: &JsonValue, field: &str) -> JsonValue {
    payload
        .get(field)
        .and_then(|value| value.get("status"))
        .cloned()
        .unwrap_or(JsonValue::Null)
}

fn compact_task_land_payload(payload: &JsonValue) -> JsonValue {
    let recovery = payload
        .get("closeout_recovery")
        .filter(|value| value.is_object())
        .map(|value| {
            json!({
                "code": cloned_field(value, "code"),
                "command": cloned_field(value, "command"),
            })
        })
        .unwrap_or(JsonValue::Null);
    let change_ref = string_field(payload.get("change_ref"));
    let change_ref = if change_ref.is_empty() {
        string_field(payload.get("change_id"))
    } else {
        change_ref
    };
    let mode = payload
        .get("mode")
        .cloned()
        .or_else(|| {
            payload
                .get("task_land_contract")
                .and_then(|contract| contract.get("scope"))
                .cloned()
        })
        .unwrap_or(JsonValue::Null);
    json!({
        "contract": AGENT_ACTION_JSON_CONTRACT,
        "command": "task.land",
        "ok": task_land_exit_code(payload) == 0,
        "mode": mode,
        "task_id": cloned_field(payload, "task_id"),
        "change_ref": change_ref,
        "patchset_id": cloned_field(payload, "patchset_id"),
        "target_line": cloned_field(payload, "target_line"),
        "landed_snapshot_id": cloned_field(payload, "landed_snapshot_id"),
        "closeout": {
            "status": cloned_field(payload, "closeout_status"),
            "task_status": cloned_field(payload, "task_status"),
            "change_status": cloned_field(payload, "change_status"),
            "worktree_status": compact_nested_status(payload, "bound_worktree_cleanup"),
            "line_status": compact_nested_status(payload, "bound_line_closeout"),
            "plan_status": compact_nested_status(payload, "plan_checklist_closeout"),
        },
        "next_action": recovery,
    })
}

fn print_bounded_evidence(
    rows: &[JsonValue],
    columns: &[&str],
    show_all: bool,
    limit: usize,
    all_command: &str,
) {
    let shown = if show_all || rows.len() <= limit {
        rows
    } else {
        &rows[..limit]
    };
    print_list(shown, columns);
    if shown.len() < rows.len() {
        println!("shown: {}/{}", shown.len(), rows.len());
        println!("more: {all_command}");
    }
}

fn bounded_ordered_json_payload(payload: &JsonValue, show_all: bool, limit: usize) -> JsonValue {
    let Some(rows) = payload.as_array() else {
        return payload.clone();
    };
    if show_all || rows.len() <= limit {
        return payload.clone();
    }
    JsonValue::Array(rows.iter().take(limit).cloned().collect())
}

fn row_recency_key(row: &JsonValue) -> String {
    for field in [
        "updated_at",
        "created_at",
        "task_seq",
        "change_seq",
        "head_revision_number",
    ] {
        let value = string_field(row.get(field));
        if !value.is_empty() {
            return value;
        }
    }
    String::new()
}

fn row_identity_key(row: &JsonValue) -> String {
    for field in [
        "change_ref",
        "task_id",
        "plan_id",
        "snapshot_id",
        "line_id",
    ] {
        let value = string_field(row.get(field));
        if !value.is_empty() {
            return value;
        }
    }
    String::new()
}

fn row_agent_priority(row: &JsonValue) -> i64 {
    if let Some(priority) = row.get("_agent_priority").and_then(JsonValue::as_i64) {
        return priority;
    }
    if row.get("publication_state").and_then(JsonValue::as_str) == Some("local_draft") {
        return 1;
    }
    0
}

fn print_agent_list(
    rows: &[JsonValue],
    columns: &[&str],
    show_all: bool,
    terminal_statuses: &[&str],
    matching_label: Option<&str>,
    all_command: &str,
) {
    let (matching, matching_count, total_count) =
        select_agent_list_rows(rows, show_all, terminal_statuses);
    print_list(&matching, columns);

    if show_all || (matching.len() == total_count && matching_count == total_count) {
        return;
    }
    match matching_label {
        Some(label) if matching_count < total_count => {
            if matching.len() < matching_count {
                println!(
                    "shown: {}/{} {label} ({} total)",
                    matching.len(),
                    matching_count,
                    total_count
                );
            } else {
                println!(
                    "shown: {} {label} ({} total)",
                    matching.len(),
                    total_count
                );
            }
        }
        _ => println!("shown: {}/{}", matching.len(), total_count),
    }
    println!("more: {all_command}");
}

fn select_agent_list_rows(
    rows: &[JsonValue],
    show_all: bool,
    terminal_statuses: &[&str],
) -> (Vec<JsonValue>, usize, usize) {
    let total_count = rows.len();
    let mut matching = rows.to_vec();
    matching.sort_by(|left, right| {
        row_agent_priority(right)
            .cmp(&row_agent_priority(left))
            .then_with(|| row_recency_key(right).cmp(&row_recency_key(left)))
            .then_with(|| row_identity_key(right).cmp(&row_identity_key(left)))
    });
    if !show_all && !terminal_statuses.is_empty() {
        matching.retain(|row| {
            let status = string_field(row.get("status"));
            !terminal_statuses.iter().any(|terminal| status == *terminal)
        });
    }
    let matching_count = matching.len();
    if !show_all {
        matching.truncate(DEFAULT_AGENT_TEXT_LIST_LIMIT);
    }
    (matching, matching_count, total_count)
}

fn agent_list_json_payload(
    payload: &JsonValue,
    show_all: bool,
    terminal_statuses: &[&str],
) -> JsonValue {
    let Some(rows) = payload.as_array() else {
        return payload.clone();
    };
    let (selected, _, _) = select_agent_list_rows(rows, show_all, terminal_statuses);
    JsonValue::Array(selected)
}

fn scoped_all_command(base: &str, local: bool, remote: Option<&str>) -> String {
    let mut command = format!("{base} --all");
    if local {
        command.push_str(" --local");
    } else if let Some(remote) = remote.filter(|value| !value.trim().is_empty()) {
        command.push_str(" --remote ");
        command.push_str(remote);
    }
    command
}

fn task_scoped_change_ref(change: &JsonValue) -> String {
    let change_ref = string_field(change.get("change_ref"));
    if !change_ref.is_empty() {
        return change_ref;
    }
    let task_id = string_field(change.get("task_id"));
    let change_id = string_field(change.get("change_id"));
    if !task_id.is_empty() && !change_id.is_empty() && !change_id.contains('/') {
        format!("{task_id}/{change_id}")
    } else {
        change_id
    }
}

fn project_change_text_rows(rows: &[JsonValue]) -> Vec<JsonValue> {
    rows.iter()
        .map(|row| {
            let mut projected = row.clone();
            if let Some(object) = projected.as_object_mut() {
                object.insert(
                    "change".to_string(),
                    JsonValue::String(task_scoped_change_ref(row)),
                );
            }
            projected
        })
        .collect()
}

fn compact_review_show_payload(payload: &JsonValue) -> JsonValue {
    let Some(obj) = payload.as_object() else {
        return payload.clone();
    };

    let mut compact = ait_core::json_support::JsonMap::new();
    for field in [
        "change_id",
        "current_patchset_id",
        "approvals",
        "blocking",
        "comments",
        "task_approvals",
        "team_approvals",
        "human_approvals",
        "human_task_approvals",
        "independent_human_approvals",
        "independent_task_approvals",
        "code_review_summaries",
        "code_review_summary_reviewers",
    ] {
        if let Some(value) = obj.get(field) {
            compact.insert(field.to_string(), value.clone());
        }
    }

    if let Some(rows) = obj.get("review_requests").and_then(JsonValue::as_array) {
        let compact_rows = rows
            .iter()
            .filter_map(JsonValue::as_object)
            .map(|row| {
                let mut compact_row = ait_core::json_support::JsonMap::new();
                for field in ["patchset_id", "reviewer_group"] {
                    if let Some(value) = row.get(field) {
                        compact_row.insert(field.to_string(), value.clone());
                    }
                }
                JsonValue::Object(compact_row)
            })
            .collect::<Vec<_>>();
        compact.insert(
            "review_requests".to_string(),
            JsonValue::Array(compact_rows),
        );
    }

    JsonValue::Object(compact)
}

fn compact_policy_show_payload(payload: &JsonValue) -> JsonValue {
    let Some(obj) = payload.as_object() else {
        return payload.clone();
    };

    let mut compact = ait_core::json_support::JsonMap::new();
    for field in [
        "policy_id",
        "patchset_id",
        "decision",
        "content_class",
        "author_class",
        "effective_requirements",
        "evaluated_at",
    ] {
        if let Some(value) = obj.get(field) {
            compact.insert(field.to_string(), value.clone());
        }
    }

    if let Some(rows) = obj.get("checks").and_then(JsonValue::as_array) {
        let compact_rows = rows
            .iter()
            .filter_map(JsonValue::as_object)
            .map(|row| {
                let mut compact_row = ait_core::json_support::JsonMap::new();
                for field in ["name", "status"] {
                    if let Some(value) = row.get(field) {
                        compact_row.insert(field.to_string(), value.clone());
                    }
                }
                JsonValue::Object(compact_row)
            })
            .collect::<Vec<_>>();
        compact.insert("checks".to_string(), JsonValue::Array(compact_rows));
    }

    JsonValue::Object(compact)
}

fn emit_result(
    title: &str,
    payload: &JsonValue,
    json_output: bool,
    fields: &[&str],
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| format!("{title} payload must decode to an object."))?;
    let rows = fields
        .iter()
        .map(|field| (*field, string_field(obj.get(*field))))
        .collect::<Vec<_>>();
    print_key_values(title, &rows);
    Ok(())
}

fn emit_auth_bindings_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let rows = payload
        .as_array()
        .ok_or_else(|| "auth bindings payload must decode to a list.".to_string())?;
    print_list(rows, &["actor_identity", "role", "created_at"]);
    Ok(())
}

fn emit_status_result(
    payload: &JsonValue,
    json_output: bool,
    full_output: bool,
) -> Result<(), String> {
    if json_output {
        return if full_output {
            print_json(payload)
        } else {
            print_json(&compact_status_payload(payload))
        };
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "status payload must decode to an object.".to_string())?;
    let current_line = string_field(obj.get("current_line"));
    let head_snapshot = string_field(obj.get("head_snapshot_id"));
    let line = if current_line.is_empty() {
        head_snapshot
    } else if head_snapshot.is_empty() {
        current_line
    } else {
        format!("{current_line} @ {head_snapshot}")
    };
    let workspace_status = string_field_or_default(obj.get("workspace_status"), "unknown");
    let changed_count = obj
        .get("workspace_changed_count")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let workspace = if changed_count > 0 {
        format!("{workspace_status} ({changed_count} changed)")
    } else {
        workspace_status
    };
    let mut rows = vec![
        ("repo", string_field(obj.get("repo_name"))),
        ("worktree", string_field(obj.get("worktree_name"))),
        ("line", line),
        ("workspace", workspace),
    ];

    if let Some(worktree_hygiene) = obj.get("worktree_hygiene") {
        let cleanup = worktree_hygiene
            .get("cleanup_candidate_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let manual = worktree_hygiene
            .get("manual_review_candidate_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let stale = worktree_hygiene
            .get("stale_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        if cleanup + manual + stale > 0 {
            rows.push((
                "worktrees",
                format!("{cleanup} cleanup, {manual} manual, {stale} stale"),
            ));
        }
    }
    if let Some(line_hygiene) = obj.get("line_hygiene") {
        let cleanup = line_hygiene
            .get("candidate_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let protected = line_hygiene
            .get("protected_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        if cleanup + protected > 0 {
            rows.push((
                "lines",
                format!("{cleanup} cleanup, {protected} protected"),
            ));
        }
    }
    let mut reconciliation_next = String::new();
    if let Some(reconciliation) = obj.get("reconciliation") {
        let safe = reconciliation
            .get("safe_finding_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let manual = reconciliation
            .get("manual_resolution_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let protected = reconciliation
            .get("protected_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        let mut findings = Vec::new();
        if safe > 0 {
            findings.push(format!("{safe} safe"));
        }
        if manual > 0 {
            findings.push(format!("{manual} manual"));
        }
        if protected > 0 {
            findings.push(format!("{protected} protected"));
        }
        reconciliation_next =
            status_reconciliation_next(reconciliation, safe, manual, protected);
        if !findings.is_empty() {
            rows.push(("reconciliation", findings.join(", ")));
        }
    }
    print_key_values("ait status", &rows);
    if changed_count > 0 {
        let sample = json_string_array(obj.get("workspace_changed_paths_sample"));
        if !sample.is_empty() {
            println!("changed: {}", sample.join(", "));
        }
        if sample.len() < changed_count as usize {
            println!("shown: {}/{} changed paths", sample.len(), changed_count);
        }
        println!("next: ait diff");
    } else if !reconciliation_next.is_empty() {
        println!("next: {reconciliation_next}");
    }

    Ok(())
}

fn status_reconciliation_next(
    reconciliation: &JsonValue,
    safe: i64,
    manual: i64,
    protected: i64,
) -> String {
    if safe > 0 {
        let command = string_field(reconciliation.get("next_command"));
        if command.is_empty() {
            "ait workflow reconcile --dry-run".to_string()
        } else {
            command
        }
    } else if manual + protected > 0 {
        "ait workflow reconcile --dry-run".to_string()
    } else {
        String::new()
    }
}

fn emit_remote_list_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let rows = payload
        .as_array()
        .ok_or_else(|| "remote list payload must decode to a list.".to_string())?;
    print_list(
        rows,
        &[
            "name",
            "url",
            "repo_name",
            "is_default_push",
            "is_default_pull",
        ],
    );
    Ok(())
}

fn emit_remote_add_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    println!("{}", render_remote_add_text(payload)?);
    Ok(())
}

fn render_remote_add_text(payload: &JsonValue) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "remote add payload must decode to an object.".to_string())?;
    let mut lines = vec![
        "ait-cli remote add".to_string(),
        format!("name: {}", string_field(obj.get("name"))),
        format!("url: {}", string_field(obj.get("url"))),
        format!("repo_name: {}", string_field(obj.get("repo_name"))),
        format!(
            "is_default_push: {}",
            string_field(obj.get("is_default_push"))
        ),
        format!(
            "is_default_pull: {}",
            string_field(obj.get("is_default_pull"))
        ),
        format!("created_at: {}", string_field(obj.get("created_at"))),
    ];
    if let Some(patch_ci) = obj.get("patch_ci").and_then(JsonValue::as_object) {
        let suites = patch_ci
            .get("blocking_suite_ids")
            .and_then(JsonValue::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        lines.extend([
            String::new(),
            "Patchset CI".to_string(),
            format!("status: {}", string_field(patch_ci.get("status"))),
            format!(
                "manifest: {}",
                string_field(patch_ci.get("manifest_path"))
            ),
            format!("blocking_suites: {suites}"),
        ]);
        if patch_ci
            .get("required")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            lines.extend([
                "Configure test commands at `suites[].runner.commands`.".to_string(),
                "Keep `plane: patchset`, `mode: gate`, and `default_blocking: true`."
                    .to_string(),
                "After future CI changes, create a new Snapshot before pushing.".to_string(),
            ]);
        } else {
            lines.push(
                "The effective local policy does not require Patchset tests; no manifest was generated."
                    .to_string(),
            );
        }
    }
    Ok(lines.join("\n"))
}

fn emit_doctor_result(title: &str, payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    println!("{}", render_doctor_text(title, payload)?);
    Ok(())
}

fn json_string_array(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn emit_queue_summary_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "queue summary payload must decode to an object.".to_string())?;
    let summary = obj
        .get("summary")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "queue summary payload is missing summary.".to_string())?;
    let remote = obj
        .get("remote")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "queue summary payload is missing remote.".to_string())?;
    print_key_values(
        "ait-cli queue summary",
        &[
            ("repo", string_field(obj.get("repo_name"))),
            ("remote", {
                let remote_name = string_field(remote.get("remote_name"));
                if remote_name.is_empty() {
                    string_field(remote.get("repo_name"))
                } else {
                    remote_name
                }
            }),
            (
                "shared tasks",
                string_field(summary.get("shared_task_count")),
            ),
            (
                "ready to land",
                string_field(summary.get("ready_to_land_count")),
            ),
            (
                "review inbox",
                string_field(summary.get("reviewer_inbox_count")),
            ),
            (
                "local draft tasks",
                string_field(summary.get("local_draft_task_count")),
            ),
            (
                "local draft changes",
                string_field(summary.get("local_draft_change_count")),
            ),
            (
                "workspace changed",
                string_field(summary.get("workspace_changed_count")),
            ),
            (
                "dirty worktrees",
                string_field(summary.get("dirty_worktree_count")),
            ),
            (
                "stale worktrees",
                string_field(summary.get("stale_worktree_count")),
            ),
        ],
    );
    if let Some(error) = remote.get("error").and_then(JsonValue::as_str) {
        if !error.trim().is_empty() {
            println!();
            println!("remote summary unavailable");
            println!("- {error}");
        }
    }
    Ok(())
}

fn emit_task_audit_result(
    task_id: &str,
    payload: &JsonValue,
    json_output: bool,
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "task audit payload must decode to an object.".to_string())?;
    let task = obj
        .get("task")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "task audit task must decode to an object.".to_string())?;
    let summary = obj
        .get("summary")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "task audit summary must decode to an object.".to_string())?;
    let workflow = obj
        .get("workflow")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "task audit workflow must decode to an object.".to_string())?;
    let target = obj
        .get("target")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "task audit target must decode to an object.".to_string())?;
    let task_land_closeout = obj
        .get("task_land_closeout")
        .and_then(JsonValue::as_object);
    let task_status = string_field(task.get("status"));
    let workflow_state = string_field(workflow.get("state"));
    let action = obj
        .get("recommended_action")
        .or_else(|| obj.get("next_action"))
        .and_then(JsonValue::as_object);
    let action_code = action
        .map(|value| string_field(value.get("code")))
        .unwrap_or_default();
    let state = match (task_status.is_empty(), workflow_state.is_empty()) {
        (false, false) if task_status != workflow_state => {
            format!("{task_status} / {workflow_state}")
        }
        (false, _) => task_status,
        (_, false) => workflow_state,
        _ => String::new(),
    };
    let mut rows = vec![
        ("title", string_field(task.get("title"))),
        ("state", state),
        ("result", string_field(summary.get("verdict"))),
        ("target", string_field(target.get("line_name"))),
    ];
    let workflow_reason = string_field(workflow.get("reason"));
    if let Some(label) = task_audit_reason_label(&action_code) {
        rows.push((label, workflow_reason));
    }
    if let Some(closeout) = task_land_closeout {
        let closeout_status = string_field(closeout.get("status"));
        if !closeout_status.is_empty() && closeout_status != "pending_task_land" {
            rows.push(("closeout", closeout_status));
        }
    }
    print_key_values(&format!("ait task audit {task_id}"), &rows);
    if let Some(action) = action {
        let label = string_field(action.get("label"));
        let detail = string_field(action.get("detail"));
        if action_code != "none" && (!label.is_empty() || !detail.is_empty()) {
            let rendered = match (label.is_empty(), detail.is_empty()) {
                (false, false) => format!("{label} — {detail}"),
                (false, true) => label,
                (true, false) => detail,
                (true, true) => String::new(),
            };
            if !rendered.is_empty() {
                println!("action: {rendered}");
            }
        }
    }
    if let Some(recovery) = task_land_closeout
        .and_then(|closeout| closeout.get("recovery"))
        .and_then(JsonValue::as_object)
    {
        let detail = string_field(recovery.get("detail"));
        let command = string_field(recovery.get("command"));
        if !detail.is_empty() || !command.is_empty() {
            if !detail.is_empty() {
                println!("recovery: {detail}");
            }
            if !command.is_empty() {
                println!("next: {command}");
            }
        }
    }
    if let Some(change_rows) = obj.get("changes").and_then(JsonValue::as_array) {
        let (projected, has_target_state) = project_task_audit_change_text_rows(change_rows);
        if !projected.is_empty() {
            println!();
            println!("changes");
            let columns = if has_target_state {
                &["change", "status", "target_state"][..]
            } else {
                &["change", "status"][..]
            };
            print_list(&projected, columns);
        }
    }
    Ok(())
}

fn project_task_audit_change_text_rows(change_rows: &[JsonValue]) -> (Vec<JsonValue>, bool) {
    let has_target_state = !change_rows.is_empty()
        && change_rows
            .iter()
            .all(|row| !string_field(row.get("target_state")).is_empty());
    let projected = change_rows
        .iter()
        .map(|row| {
            let change = row
                .get("change")
                .filter(|value| value.is_object())
                .unwrap_or(row);
            json!({
                "change": task_scoped_change_ref(change),
                "status": string_field(change.get("status")),
                "target_state": string_field(row.get("target_state")),
            })
        })
        .collect();
    (projected, has_target_state)
}

fn task_audit_reason_label(action_code: &str) -> Option<&'static str> {
    match action_code {
        "continue_task_work" | "create_change" => Some("pending"),
        "none" => None,
        code if code.contains("blocked") => Some("blocker"),
        _ => Some("attention"),
    }
}

fn emit_review_record_result(
    title: &str,
    payload: JsonValue,
    json_output: bool,
) -> Result<(), String> {
    emit_result(
        title,
        &payload,
        json_output,
        &["change_id", "patchset_id", "reviewer", "action"],
    )
}

fn emit_review_code_submit_result(
    payload: &JsonValue,
    json_output: bool,
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let task_review = payload
        .get("task_review")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "review code submit payload is missing task_review.".to_string())?;
    print_key_values(
        "ait-cli review code submit",
        &[
            ("change_id", string_field(payload.get("change_id"))),
            ("patchset_id", string_field(payload.get("patchset_id"))),
            ("code_reviewer", string_field(payload.get("reviewer"))),
            ("code_action", string_field(payload.get("action"))),
            (
                "task_review_mode",
                string_field(task_review.get("mode")),
            ),
            (
                "task_review_status",
                string_field(task_review.get("status")),
            ),
            (
                "task_reviewer",
                string_field(task_review.get("reviewer")),
            ),
        ],
    );
    Ok(())
}

fn emit_review_code_template_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "review code template payload must decode to an object.".to_string())?;
    println!("{}", string_field(obj.get("template")));
    Ok(())
}

fn emit_review_show_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        let compact = compact_review_show_payload(payload);
        return print_json(&compact);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "review show payload must decode to an object.".to_string())?;
    print_key_values(
        "ait-cli review show",
        &[
            ("change_id", string_field(obj.get("change_id"))),
            (
                "current_patchset_id",
                string_field(obj.get("current_patchset_id")),
            ),
            (
                "approvals",
                string_field_or_default(obj.get("approvals"), "0"),
            ),
            (
                "blocking",
                string_field_or_default(obj.get("blocking"), "0"),
            ),
            (
                "comments",
                string_field_or_default(obj.get("comments"), "0"),
            ),
        ],
    );
    if let Some(rows) = obj.get("review_requests").and_then(JsonValue::as_array) {
        if !rows.is_empty() {
            println!();
            print_list(rows, &["patchset_id", "reviewer_group", "note"]);
        }
    }
    if let Some(rows) = obj.get("reviews").and_then(JsonValue::as_array) {
        if !rows.is_empty() {
            println!();
            print_list(
                rows,
                &["reviewer", "patchset_id", "action", "blocking", "comment"],
            );
        }
    }
    Ok(())
}

fn emit_policy_show_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        let compact = compact_policy_show_payload(payload);
        return print_json(&compact);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "policy show payload must decode to an object.".to_string())?;
    print_key_values(
        "ait-cli policy show",
        &[
            ("policy_id", string_field(obj.get("policy_id"))),
            ("patchset_id", string_field(obj.get("patchset_id"))),
            ("decision", string_field(obj.get("decision"))),
            ("evaluated_at", string_field(obj.get("evaluated_at"))),
        ],
    );
    if let Some(rows) = obj.get("checks").and_then(JsonValue::as_array) {
        if !rows.is_empty() {
            println!();
            print_list(rows, &["name", "status", "message"]);
        }
    }
    Ok(())
}

fn touch_worktree_payload(repo: &RepoRuntime, name: Option<&str>) -> Result<JsonValue, String> {
    let payload = worktree_get(repo, name, true)?;
    let touched = worktree_touch_usage(repo, name)?;
    if touched.is_null() {
        Ok(payload)
    } else {
        Ok(touched)
    }
}

fn required_object_string_field(payload: &JsonValue, key: &str) -> Result<String, String> {
    let value = payload
        .as_object()
        .and_then(|obj| obj.get(key))
        .ok_or_else(|| format!("worktree payload is missing `{key}`."))?;
    let rendered = string_field(Some(value));
    if rendered.is_empty() {
        Err(format!("worktree payload has an empty `{key}` field."))
    } else {
        Ok(rendered)
    }
}

fn worktree_open_path_text(payload: &JsonValue) -> Result<String, String> {
    let open_path = string_field(payload.get("open_path"));
    if !open_path.is_empty() {
        return Ok(open_path);
    }
    let alias_path = string_field(payload.get("alias_path"));
    if !alias_path.is_empty() {
        return Ok(alias_path);
    }
    required_object_string_field(payload, "path")
}

fn task_start_progress_line(payload: &JsonValue) -> Option<String> {
    let phase = payload.get("phase").and_then(JsonValue::as_str)?.trim();
    let open_path = payload
        .get("open_path")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match phase {
        "plan_sync_started" => Some(format!(
            "synchronizing Plan source: {} ({})",
            payload
                .get("artifact_path")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown"),
            payload
                .get("scope")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
        )),
        "plan_synced" => Some(format!(
            "Plan synchronized: {}",
            payload
                .get("plan_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
        )),
        "plan_item_validated" => Some(format!(
            "Plan item taskable: {} (title source: {})",
            payload
                .get("plan_item_ref")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown"),
            payload
                .get("title_source")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
        )),
        "task_created" => Some(format!(
            "task created: {}",
            payload
                .get("task_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
        )),
        "change_created" => Some(format!(
            "change created: {}",
            payload
                .get("change_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
        )),
        "worktree_bootstrap_started" => {
            let worktree_name = payload
                .get("worktree_name")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown");
            Some(match open_path {
                Some(path) => {
                    format!("starting bound worktree bootstrap: {worktree_name} at {path}")
                }
                None => format!("starting bound worktree bootstrap: {worktree_name}"),
            })
        }
        "aligning_main_seed" => Some(format!(
            "aligning main-seed to {}@{}",
            payload
                .get("line_name")
                .and_then(JsonValue::as_str)
                .unwrap_or("main"),
            payload
                .get("seed_snapshot_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
        )),
        "materializing_worktree" => {
            let worktree_name = payload
                .get("worktree_name")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown");
            let source = payload.get("source").and_then(JsonValue::as_str);
            Some(match (source, open_path) {
                (Some("main_seed_mirror"), Some(path)) => format!(
                    "materializing worktree {worktree_name} at {path} from main-seed (not ready yet)"
                ),
                (Some("main_seed_mirror"), None) => {
                    format!("materializing worktree {worktree_name} from main-seed (not ready yet)")
                }
                (_, Some(path)) => {
                    format!("materializing worktree {worktree_name} at {path} (not ready yet)")
                }
                _ => format!("materializing worktree {worktree_name} (not ready yet)"),
            })
        }
        "worktree_ready" => {
            let worktree_name = payload
                .get("worktree_name")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown");
            Some(match open_path {
                Some(path) => format!("worktree ready: {worktree_name} at {path}"),
                None => format!("worktree ready: {worktree_name}"),
            })
        }
        _ => None,
    }
}

fn emit_task_start_result(
    payload: &JsonValue,
    json_output: bool,
    full_output: bool,
) -> Result<(), String> {
    if json_output {
        return if full_output {
            print_json(payload)
        } else {
            print_json(&compact_task_start_payload(payload))
        };
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "task start payload must decode to an object.".to_string())?;
    let change_ref = obj
        .get("change")
        .map(task_scoped_change_ref)
        .unwrap_or_default();
    let cd_command = string_field(obj.get("cd_command"));
    print_key_values(
        "ait task start",
        &[
            ("task", string_field(obj.get("task_id"))),
            ("change", change_ref),
            ("next", cd_command),
        ],
    );
    if let Some(error) = obj
        .get("automatic_reconciliation")
        .and_then(|value| value.get("error"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        println!("warning: reconciliation: {error}");
    }
    Ok(())
}

fn emit_snapshot_create_result(
    payload: &JsonValue,
    json_output: bool,
    full_output: bool,
) -> Result<(), String> {
    if json_output {
        return if full_output {
            print_json(payload)
        } else {
            print_json(&compact_snapshot_create_payload(payload))
        };
    }
    emit_result(
        "ait-cli snapshot create",
        payload,
        false,
        &[
            "snapshot_id",
            "line_name",
            "parent_snapshot_id",
            "message",
        ],
    )
}

fn emit_task_land_result(
    payload: &JsonValue,
    json_output: bool,
    full_output: bool,
) -> Result<(), String> {
    if json_output {
        return if full_output {
            print_json(payload)
        } else {
            print_json(&compact_task_land_payload(payload))
        };
    }
    println!("{}", render_task_land_text(payload)?);
    Ok(())
}

fn worktree_path_payload(payload: &JsonValue) -> Result<JsonValue, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "worktree payload must decode to an object.".to_string())?;
    Ok(json!({
        "name": string_field(obj.get("name")),
        "path": string_field(obj.get("path")),
        "open_path": string_field(obj.get("open_path")),
        "alias_path": string_field(obj.get("alias_path")),
        "cd_command": string_field(obj.get("cd_command")),
        "shell_command": string_field(obj.get("shell_command")),
        "current_line": string_field(obj.get("current_line")),
        "workspace_status": string_field(obj.get("workspace_status")),
        "changed_count": obj.get("changed_count").cloned().unwrap_or(JsonValue::Null),
        "src_path": obj.get("src_path").cloned().unwrap_or(JsonValue::Null),
        "venv_path": obj.get("venv_path").cloned().unwrap_or(JsonValue::Null),
        "venv_bin_path": obj.get("venv_bin_path").cloned().unwrap_or(JsonValue::Null),
        "cargo_target_dir": obj.get("cargo_target_dir").cloned().unwrap_or(JsonValue::Null),
        "cargo_build_dir": obj.get("cargo_build_dir").cloned().unwrap_or(JsonValue::Null),
    }))
}

fn emit_worktree_status_result(
    payload: &JsonValue,
    json_output: bool,
    verbose: bool,
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "worktree status payload must decode to an object.".to_string())?;
    let changed_count = obj
        .get("changed_count")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let current_line = string_field(obj.get("current_line"));
    let baseline_snapshot = string_field(obj.get("baseline_snapshot_id"));
    let line = if current_line.is_empty() {
        baseline_snapshot
    } else if baseline_snapshot.is_empty() {
        current_line
    } else {
        format!("{current_line} @ {baseline_snapshot}")
    };
    let mut rows = vec![
        ("worktree", string_field(obj.get("worktree_name"))),
        ("line", line),
        (
            "workspace",
            if changed_count == 0 {
                "clean".to_string()
            } else {
                format!("dirty ({changed_count} changed)")
            },
        ),
    ];
    if verbose {
        rows.extend([
            ("root", string_field(obj.get("workspace_root"))),
            ("baseline source", string_field(obj.get("baseline_source"))),
            ("baseline line", string_field(obj.get("baseline_line_name"))),
        ]);
    }
    print_key_values("ait worktree status", &rows);

    if changed_count > 0 {
        let changed_paths = json_string_array(obj.get("changed_paths"));
        for path in changed_paths.iter().take(DEFAULT_AGENT_TEXT_LIST_LIMIT) {
            println!("changed: {path}");
        }
        if changed_paths.len() > DEFAULT_AGENT_TEXT_LIST_LIMIT {
            println!(
                "shown: {}/{} changed paths",
                DEFAULT_AGENT_TEXT_LIST_LIMIT,
                changed_paths.len()
            );
            println!("more: ait worktree status --json");
        }
        println!("next: ait diff");
    }
    Ok(())
}

fn emit_line_list_result(
    payload: &JsonValue,
    json_output: bool,
    show_all: bool,
    remote: Option<&str>,
    current_line: &str,
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let rows = payload
        .as_array()
        .ok_or_else(|| "line list payload must decode to an array.".to_string())?;
    let mut rows = rows.to_vec();
    for row in &mut rows {
        let priority = if row.get("line_name").and_then(JsonValue::as_str) == Some(current_line) {
            2
        } else if string_field(row.get("head_snapshot_id")).is_empty() {
            1
        } else {
            0
        };
        if priority > 0 {
            if let Some(object) = row.as_object_mut() {
                object.insert("_agent_priority".to_string(), JsonValue::from(priority));
            }
        }
    }
    let all_command = scoped_all_command("ait line list", false, remote);
    print_agent_list(
        &rows,
        &["line_name", "status", "head_snapshot_id"],
        show_all,
        &[],
        None,
        &all_command,
    );
    Ok(())
}

fn emit_line_cleanup_result(
    payload: &JsonValue,
    json_output: bool,
    show_all: bool,
    include_protected: bool,
    idle_for: &str,
    cleanup_kind: Option<&str>,
    limit: Option<usize>,
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "line cleanup payload must decode to an object.".to_string())?;
    print_key_values(
        "ait-cli line cleanup",
        &[
            ("mode", string_field(obj.get("mode"))),
            ("idle_for", string_field(obj.get("idle_for"))),
            ("cleanup_kind", string_field(obj.get("cleanup_kind"))),
            ("inspected_count", string_field(obj.get("inspected_count"))),
            ("candidate_count", string_field(obj.get("candidate_count"))),
            ("protected_count", string_field(obj.get("protected_count"))),
            ("planned_count", string_field(obj.get("planned_count"))),
            ("archived_count", string_field(obj.get("archived_count"))),
        ],
    );
    let mut all_command = format!("ait line cleanup --idle-for {idle_for}");
    if let Some(kind) = cleanup_kind.filter(|value| !value.trim().is_empty()) {
        all_command.push_str(&format!(" --kind {kind}"));
    }
    if let Some(limit) = limit {
        all_command.push_str(&format!(" --limit {limit}"));
    }
    if include_protected {
        all_command.push_str(" --include-protected");
    }
    all_command.push_str(" --all");

    let applied = obj
        .get("applied")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let rows_key = if applied {
        "archived_rows"
    } else {
        "planned_rows"
    };
    if let Some(rows) = obj.get(rows_key).and_then(JsonValue::as_array) {
        if !rows.is_empty() {
            println!();
            println!(
                "{}",
                if applied {
                    "archived lines"
                } else {
                    "cleanup candidates"
                }
            );
            print_bounded_evidence(
                rows,
                if applied {
                    &["line_name", "status", "head_snapshot_id", "archived_at"]
                } else {
                    &[
                        "line_name",
                        "lifecycle_kind",
                        "cleanup_policy",
                        "last_activity_at",
                        "cleanup_reason",
                    ]
                },
                show_all,
                DEFAULT_AGENT_TEXT_LIST_LIMIT,
                &all_command,
            );
        }
    }
    if let Some(rows) = obj.get("protected").and_then(JsonValue::as_array) {
        if !rows.is_empty() {
            let mut reason_counts = std::collections::BTreeMap::<String, usize>::new();
            for row in rows {
                *reason_counts
                    .entry(string_field_or_default(
                        row.get("protected_reason"),
                        "unspecified",
                    ))
                    .or_default() += 1;
            }
            let mut reason_rows = reason_counts
                .into_iter()
                .map(|(reason, count)| json!({"protected_reason": reason, "count": count}))
                .collect::<Vec<_>>();
            reason_rows.sort_by(|left, right| {
                right
                    .get("count")
                    .and_then(JsonValue::as_u64)
                    .cmp(&left.get("count").and_then(JsonValue::as_u64))
                    .then_with(|| {
                        string_field(left.get("protected_reason"))
                            .cmp(&string_field(right.get("protected_reason")))
                    })
            });
            println!();
            println!("protected reasons");
            print_list(&reason_rows, &["count", "protected_reason"]);

            let mut protected_rows = rows.to_vec();
            protected_rows.sort_by(|left, right| {
                cleanup_protected_priority(left)
                    .cmp(&cleanup_protected_priority(right))
                    .then_with(|| {
                        string_field(left.get("line_name"))
                            .cmp(&string_field(right.get("line_name")))
                    })
            });
            println!();
            println!("protected examples");
            let columns = [
                "line_name",
                "lifecycle_kind",
                "cleanup_policy",
                "protected_reason",
            ];
            if show_all {
                print_list(&protected_rows, &columns);
            } else {
                let mut seen_reasons = std::collections::BTreeSet::new();
                let examples = protected_rows
                    .iter()
                    .filter(|row| {
                        seen_reasons.insert(string_field_or_default(
                            row.get("protected_reason"),
                            "unspecified",
                        ))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                print_list(&examples, &columns);
                if examples.len() < protected_rows.len() {
                    println!(
                        "shown: {}/{} representative rows",
                        examples.len(),
                        protected_rows.len()
                    );
                    println!("more: {all_command}");
                }
            }
        }
    }
    let protected_count = obj
        .get("protected_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    if protected_count > 0 && !include_protected {
        let mut inspect_command =
            format!("ait line cleanup --idle-for {idle_for} --include-protected");
        if let Some(kind) = cleanup_kind.filter(|value| !value.trim().is_empty()) {
            inspect_command.push_str(&format!(" --kind {kind}"));
        }
        if let Some(limit) = limit {
            inspect_command.push_str(&format!(" --limit {limit}"));
        }
        inspect_command.push_str(" --all");
        println!("protected detail: {inspect_command}");
    }
    Ok(())
}

fn cleanup_protected_priority(row: &JsonValue) -> u8 {
    match row
        .get("protected_reason")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
    {
        "current line" => 0,
        "default line" => 1,
        "line is still used by a worktree" => 2,
        "line is still used by an active change" => 3,
        "line is already archived" => 4,
        "line lifecycle is manual_only" => 5,
        _ => 6,
    }
}

fn emit_worktree_list_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let rows = payload
        .as_array()
        .ok_or_else(|| "worktree list payload must decode to an array.".to_string())?;
    print_list(
        rows,
        &[
            "name",
            "path",
            "current_line",
            "workspace_status",
            "cleanup_class",
            "cleanup_policy",
            "exists",
            "is_current",
        ],
    );
    Ok(())
}

fn emit_worktree_show_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    emit_result(
        "ait-cli worktree show",
        payload,
        json_output,
        &[
            "name",
            "path",
            "alias_path",
            "current_line",
            "registered_line_name",
            "workspace_status",
            "changed_count",
            "bound_task_id",
            "bound_change_id",
            "target_base_line",
            "cleanup_policy",
            "rebase_state",
            "merge_state",
            "merge_conflict_count",
        ],
    )
}

fn emit_worktree_doctor_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "worktree doctor payload must decode to an object.".to_string())?;
    print_key_values(
        "ait-cli worktree doctor",
        &[
            (
                "status_mode",
                string_field_or_default(obj.get("status_mode"), "metadata"),
            ),
            (
                "refresh_status",
                string_field_or_default(obj.get("refresh_status"), "false"),
            ),
            ("total_count", string_field(obj.get("total_count"))),
            ("current_count", string_field(obj.get("current_count"))),
            ("clean_count", string_field(obj.get("clean_count"))),
            ("dirty_count", string_field(obj.get("dirty_count"))),
            ("missing_count", string_field(obj.get("missing_count"))),
            ("detached_count", string_field(obj.get("detached_count"))),
            ("protected_count", string_field(obj.get("protected_count"))),
            (
                "safe_auto_remove_count",
                string_field(obj.get("safe_auto_remove_count")),
            ),
            (
                "safe_cleanup_candidate_count",
                string_field(obj.get("safe_cleanup_candidate_count")),
            ),
            (
                "manual_review_candidate_count",
                string_field(obj.get("manual_review_candidate_count")),
            ),
        ],
    );
    for (label, key) in [
        ("cleanup_candidate_rows", "cleanup_candidate_rows"),
        ("manual_review_rows", "manual_review_rows"),
        ("stale_rows", "stale_rows"),
    ] {
        if let Some(rows) = obj.get(key).and_then(JsonValue::as_array) {
            if !rows.is_empty() {
                println!();
                println!("{label}");
                print_list(
                    rows,
                    &[
                        "name",
                        "path",
                        "current_line",
                        "workspace_status",
                        "cleanup_class",
                        "cleanup_policy",
                    ],
                );
            }
        }
    }
    if obj
        .get("status_mode")
        .and_then(JsonValue::as_str)
        .unwrap_or("metadata")
        == "metadata"
    {
        println!();
        println!("Run `ait worktree doctor --refresh` to verify live workspace status.");
    }
    Ok(())
}

fn emit_worktree_cleanup_candidates_result(
    payload: &JsonValue,
    json_output: bool,
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload.as_object().ok_or_else(|| {
        "worktree cleanup-candidates payload must decode to an object.".to_string()
    })?;
    print_key_values(
        "ait-cli worktree cleanup-candidates",
        &[
            ("older_than", string_field(obj.get("older_than"))),
            ("cleanup_policy", string_field(obj.get("cleanup_policy"))),
            (
                "allow_manual_only",
                string_field(obj.get("allow_manual_only")),
            ),
            ("inspected_count", string_field(obj.get("inspected_count"))),
            ("candidate_count", string_field(obj.get("candidate_count"))),
            ("protected_count", string_field(obj.get("protected_count"))),
            ("stale_count", string_field(obj.get("stale_count"))),
        ],
    );
    if let Some(rows) = obj.get("candidates").and_then(JsonValue::as_array) {
        if !rows.is_empty() {
            println!();
            print_list(
                rows,
                &[
                    "name",
                    "cleanup_class",
                    "cleanup_policy",
                    "last_used_at",
                    "cleanup_reason",
                ],
            );
        }
    }
    if let Some(rows) = obj.get("protected").and_then(JsonValue::as_array) {
        if !rows.is_empty() {
            println!();
            print_list(rows, &["name", "cleanup_policy", "protected_reason"]);
        }
    }
    Ok(())
}

fn emit_worktree_cleanup_rows(
    title: &str,
    payload: &JsonValue,
    count_key: &str,
    rows_key: &str,
    fallback_count_key: &str,
    fallback_rows_key: &str,
) -> Result<(), String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| format!("{title} payload must decode to an object."))?;
    let primary_rows = obj.get(rows_key).and_then(JsonValue::as_array);
    let primary_count = obj.get(count_key);
    let rows = primary_rows.or_else(|| obj.get(fallback_rows_key).and_then(JsonValue::as_array));
    let row_count = if primary_rows.is_some() {
        string_field(primary_count)
    } else {
        string_field(obj.get(fallback_count_key))
    };
    print_key_values(
        title,
        &[
            ("candidate_count", string_field(obj.get("candidate_count"))),
            ("row_count", row_count),
            ("dry_run", string_field(obj.get("dry_run"))),
        ],
    );
    if let Some(rows) = rows {
        if !rows.is_empty() {
            println!();
            print_list(
                rows,
                &[
                    "name",
                    "path",
                    "current_line",
                    "workspace_status",
                    "cleanup_reason",
                    "deleted_path",
                ],
            );
        }
    }
    Ok(())
}

fn emit_worktree_cleanup_report_result(
    payload: &JsonValue,
    json_output: bool,
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    emit_worktree_cleanup_rows(
        "ait-cli worktree cleanup",
        payload,
        "planned_count",
        "planned_rows",
        "removed_count",
        "removed_rows",
    )
}

fn emit_worktree_sync_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "worktree sync payload must decode to an object.".to_string())?;
    if obj.get("requested_count").is_some() {
        print_key_values(
            "ait-cli worktree sync",
            &[
                ("requested_count", string_field(obj.get("requested_count"))),
                ("synced_count", string_field(obj.get("synced_count"))),
                ("skipped_count", string_field(obj.get("skipped_count"))),
                ("error_count", string_field(obj.get("error_count"))),
                ("ok", string_field(obj.get("ok"))),
            ],
        );
        for (label, key) in [
            ("synced_rows", "synced_rows"),
            ("skipped_rows", "skipped_rows"),
            ("error_rows", "error_rows"),
        ] {
            if let Some(rows) = obj.get(key).and_then(JsonValue::as_array) {
                if !rows.is_empty() {
                    println!();
                    println!("{label}");
                    print_list(
                        rows,
                        &["name", "path", "current_line", "workspace_status", "error"],
                    );
                }
            }
        }
        return Ok(());
    }
    emit_result(
        "ait-cli worktree sync",
        payload,
        false,
        &[
            "name",
            "path",
            "current_line",
            "workspace_status",
            "status",
            "changed_count",
        ],
    )
}

fn emit_worktree_prune_result(payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "worktree prune payload must decode to an object.".to_string())?;
    print_key_values(
        "ait-cli worktree prune-stale",
        &[
            ("dry_run", string_field(obj.get("dry_run"))),
            ("pruned_count", string_field(obj.get("pruned_count"))),
        ],
    );
    if let Some(rows) = obj.get("pruned_rows").and_then(JsonValue::as_array) {
        if !rows.is_empty() {
            println!();
            print_list(
                rows,
                &["name", "path", "current_line", "workspace_status", "exists"],
            );
        }
    }
    Ok(())
}
