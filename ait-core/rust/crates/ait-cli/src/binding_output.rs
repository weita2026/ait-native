//! Pure native presentation shared with language frontends; no operation executes here.
use ait_core::json_support::JsonValue;

pub const SNAPSHOT_QUERY_ERROR_EXIT_CODE: u8 = 2;

pub fn format_auth_bindings(payload: &JsonValue) -> Result<String, String> {
    let rows = payload
        .as_array()
        .ok_or("auth bindings payload must decode to a list.")?;
    Ok(crate::render::list_text(
        rows,
        &["actor_identity", "role", "created_at"],
    ))
}

pub fn format_patchset_show(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::result_text(
        "ait-cli patchset show",
        &crate::main_app::patchset_show_display_payload(payload),
        &[
            "patchset_id",
            "patchset_number",
            "author_mode",
            "base_snapshot_id",
            "revision_snapshot_id",
            "evaluation_state",
            "publish_state",
            "summary",
        ],
    )
}

pub fn format_patchset_ci_status(payload: &JsonValue) -> Result<String, String> {
    let display = crate::main_app::patchset_ci_status_display_payload(payload);
    crate::main_app::result_text(
        "ait-cli patchset ci-status",
        &display,
        &crate::main_app::patchset_ci_status_display_fields(&display),
    )
}

pub fn format_patchset_list(payload: &JsonValue) -> Result<String, String> {
    let rows = payload
        .as_array()
        .ok_or("Patchset inventory must be an array.")?;
    Ok(crate::render::list_text(
        rows,
        &[
            "patchset_id",
            "patchset_number",
            "base_snapshot_id",
            "revision_snapshot_id",
            "publish_state",
            "evaluation_state",
        ],
    ))
}

pub fn project_change_list(payload: &JsonValue, all: bool) -> JsonValue {
    crate::main_app::project_change_inventory(payload, all)
}

pub fn format_change_list(
    payload: &JsonValue,
    all: bool,
    scope: &crate::binding_services::PlanScope,
) -> Result<String, String> {
    crate::main_app::format_change_inventory(payload, all, scope)
}

pub fn format_snapshot_query_error(message: &str) -> String {
    format!("Error: {message}")
}

pub fn project_snapshot_list(payload: &JsonValue, all: bool) -> JsonValue {
    crate::main_app::project_snapshot_inventory(payload, all)
}

pub fn format_snapshot_list(payload: &JsonValue, all: bool) -> Result<String, String> {
    crate::main_app::format_snapshot_inventory(payload, all)
}

pub fn format_snapshot_show(inspection: &JsonValue, files: bool) -> Result<String, String> {
    let payload = inspection
        .get("payload")
        .ok_or("Snapshot inspection is missing payload.")?;
    let delta = inspection.get("delta").filter(|value| !value.is_null());
    let mut text = crate::main_app::format_snapshot_identity_text(payload, delta, !files)?;
    if files {
        if let Some(rows) = payload.get("files").and_then(JsonValue::as_array) {
            text.push_str("\n\nfiles\n");
            text.push_str(&crate::render::list_text(
                rows,
                &["path", "blob_id", "size_bytes", "mode"],
            ));
        }
    }
    Ok(text)
}

pub fn format_snapshot_ancestry(payload: &JsonValue, all: bool) -> Result<String, String> {
    crate::main_app::format_snapshot_ancestry_text(payload, all)
}

pub fn format_snapshot_is_ancestor(payload: &JsonValue) -> Result<String, String> {
    match payload.get("is_ancestor").and_then(JsonValue::as_bool) {
        Some(true) => Ok("ancestor".into()),
        Some(false) => Ok("not ancestor".into()),
        None => Err("Snapshot ancestry result is missing is_ancestor.".into()),
    }
}

pub fn format_snapshot_merge_base(payload: &JsonValue) -> Result<String, String> {
    let ids = payload
        .get("merge_base_snapshot_ids")
        .and_then(JsonValue::as_array)
        .ok_or("Snapshot merge-base result is missing merge_base_snapshot_ids.")?;
    Ok(ids
        .iter()
        .filter_map(JsonValue::as_str)
        .collect::<Vec<_>>()
        .join("\n"))
}

pub fn format_remote_add(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::render_remote_add_text(payload)
}

pub fn format_remote_list(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::remote_list_text(payload)
}

pub fn format_queue_summary(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::queue_summary_text(payload)
}

pub fn format_doctor_memory_root(payload: &JsonValue) -> Result<String, String> {
    crate::doctor_surface::render_doctor_text("ait-cli doctor memory-root", payload)
}

pub fn format_doctor_runtime_root(payload: &JsonValue) -> Result<String, String> {
    crate::doctor_surface::render_doctor_text("ait-cli doctor runtime-root", payload)
}

pub fn format_doctor_plan_authority(payload: &JsonValue) -> Result<String, String> {
    crate::doctor_surface::render_doctor_text("ait-cli doctor plan-authority", payload)
}

pub fn format_initialize_repository(payload: &JsonValue) -> String {
    let mut text = crate::init_surface::human_init_text(payload);
    if let Some(team) = payload.get("agent_team_bootstrap") {
        let line = if let Some(activation) = crate::agent_teams::render_activation(team) {
            activation
        } else if team["status"] == "disabled" {
            "Agent Teams assets synchronized; sessions disabled.".into()
        } else if let Some(id) = team["session_id"].as_str() {
            format!("有專案需求或問題，請找 Newton\nSession ID: {id}\nResume: codex resume {id}")
        } else {
            format!("Newton session setup pending: {}", team["detail"])
        };
        text.push_str(&format!("\n{line}"));
        if let Some(condition) = team["condition"].as_str() {
            text.push_str(&format!("\n{condition}"));
        }
    }
    text
}

pub fn format_configuration_show(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::config_show_text(payload)
}

pub fn format_configuration(payload: &JsonValue, request: &JsonValue) -> Result<String, String> {
    format_configuration_with_keys(
        payload,
        &crate::config_surface::configuration_updated_keys(request)?,
    )
}

pub(crate) fn format_configuration_with_keys(
    payload: &JsonValue,
    keys: &[&str],
) -> Result<String, String> {
    let mut text = crate::main_app::config_mutation_text("ait-cli config set", payload, keys)?;
    if let Some(deployment) = payload.get("team_bootstrap") {
        text.push_str(&format!(
            "\n\nTeam sessions: {}",
            deployment["status"].as_str().unwrap_or("blocked")
        ));
        if let Some(activation) = crate::agent_teams::render_activation(deployment) {
            text.push_str(&format!("\n{activation}"));
        } else if let Some(id) = deployment["session_id"].as_str() {
            text.push_str(&format!(
                "\n有專案需求或問題，請找 Newton\nSession ID: {id}\nResume: codex resume {id}"
            ));
        }
        if let Some(detail) = deployment["detail"].as_str() {
            text.push_str(&format!(
                "\nConfiguration saved; session setup requires attention: {detail}"
            ));
        }
        if let Some(condition) = deployment["condition"].as_str() {
            text.push_str(&format!("\n{condition}"));
        }
    }
    Ok(text)
}

pub fn format_configuration_unset(payload: &JsonValue, key: &str) -> Result<String, String> {
    let key = crate::config_surface::config_unset_key(key)?;
    crate::main_app::config_mutation_text("ait-cli config unset", payload, &[key.as_str()])
}

pub fn plan_sync_error(payload: &JsonValue) -> Option<String> {
    crate::main_app::plan_sync_terminal_error(payload)
}

pub fn project_workflow_ready(
    repo: &crate::runtime::RepoRuntime,
    payload: &JsonValue,
    task_id: &str,
    remote_name: Option<&str>,
    full: bool,
) -> JsonValue {
    crate::main_app::project_workflow_json(
        repo,
        &mut payload.clone(),
        "ready",
        task_id,
        remote_name,
        full,
    )
}

pub fn project_workflow_finish(
    repo: &crate::runtime::RepoRuntime,
    payload: &JsonValue,
    task_id: &str,
    remote_name: Option<&str>,
    full: bool,
) -> JsonValue {
    crate::main_app::project_workflow_json(
        repo,
        &mut payload.clone(),
        "finish",
        task_id,
        remote_name,
        full,
    )
}

pub fn format_workflow_ready(payload: &JsonValue, task_id: &str) -> Result<String, String> {
    crate::main_app::render_requested_workflow_text(payload, "ready", task_id)
}

pub fn format_workflow_finish(payload: &JsonValue, task_id: &str) -> Result<String, String> {
    crate::main_app::render_requested_workflow_text(payload, "finish", task_id)
}

pub fn format_workflow_reconcile(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::render_workflow_reconcile_text(payload)
}

pub fn format_workflow_guide(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::render_workflow_guide_text(payload)
}

pub fn format_plan_list(
    payload: &JsonValue,
    show_all: bool,
    scope: &crate::binding_services::PlanScope,
) -> Result<String, String> {
    crate::main_app::format_plan_list_text(payload, show_all, scope)
}

pub fn format_plan_revisions(
    payload: &JsonValue,
    plan_id: &str,
    show_all: bool,
    scope: &crate::binding_services::PlanScope,
) -> Result<String, String> {
    crate::main_app::format_plan_revisions_text(payload, plan_id, show_all, scope)
}

pub fn format_plan_show(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::format_plan_show_like("ait-cli plan show", payload)
}

pub fn format_plan_items(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::format_plan_items_like("ait-cli plan items", payload)
}

pub fn format_plan_candidates(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::format_plan_candidates_like(payload)
}

pub fn format_plan_inspect(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::format_plan_inspect_like(payload)
}

pub fn format_plan_sync(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::format_plan_sync_like(payload)
}

pub fn project_status(payload: &JsonValue, full: bool) -> JsonValue {
    if full {
        payload.clone()
    } else {
        crate::main_app::compact_status_payload(payload)
    }
}

pub fn format_status(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::status_result_text(payload)
}

pub fn project_task_start(payload: &JsonValue, full: bool) -> JsonValue {
    if full {
        payload.clone()
    } else {
        crate::main_app::compact_task_start_payload(payload)
    }
}

pub fn format_task_start(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::task_start_result_text(payload)
}

pub fn project_task_list(payload: &JsonValue, all: bool) -> JsonValue {
    crate::main_app::project_task_inventory(payload, all)
}

pub fn format_task_list(
    payload: &JsonValue,
    all: bool,
    scope: &crate::binding_services::PlanScope,
) -> Result<String, String> {
    crate::main_app::format_task_inventory(payload, all, scope)
}

pub fn format_task_show(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::result_text(
        "ait-cli task show",
        payload,
        &[
            "task_id",
            "title",
            "status",
            "publication_state",
            "published_task_id",
        ],
    )
}

pub fn format_line_list(
    inventory: &JsonValue,
    show_all: bool,
    remote_name: Option<&str>,
) -> Result<String, String> {
    let payload = inventory
        .get("payload")
        .ok_or("Line inventory is missing payload.")?;
    let current_line = inventory
        .get("current_line")
        .and_then(JsonValue::as_str)
        .ok_or("Line inventory is missing current_line.")?;
    crate::main_app::line_list_text(payload, show_all, remote_name, current_line)
}

pub fn format_line_cleanup(
    payload: &JsonValue,
    all: bool,
    include_protected: bool,
    idle_for: &str,
    cleanup_kind: Option<&str>,
    limit: Option<usize>,
) -> Result<String, String> {
    crate::main_app::line_cleanup_text(
        payload,
        all,
        include_protected,
        idle_for,
        cleanup_kind,
        limit,
    )
}

pub fn format_task_quick(payload: &JsonValue) -> Result<String, String> {
    let mut text = format_task_start(payload)?;
    if let Some(card) = payload
        .get("quick_card")
        .and_then(|card| card.get("artifact_path"))
        .and_then(JsonValue::as_str)
    {
        text.push_str(&format!("\ncard: {card}"));
    }
    Ok(text)
}

pub fn format_task_audit(payload: &JsonValue, task_id: &str) -> Result<String, String> {
    crate::main_app::task_audit_result_text(task_id, payload)
}

pub fn format_task_abandon(payload: &JsonValue, task_id: &str) -> Result<String, String> {
    let fields: &[&str] = if task_id.starts_with('@') {
        &["state", "expected_identity", "detail"]
    } else {
        &["task_id", "status", "published_task_id"]
    };
    crate::main_app::result_text("ait-cli task abandon", payload, fields)
}

pub fn project_task_finish(payload: &JsonValue, full: bool) -> JsonValue {
    if full {
        payload.clone()
    } else {
        crate::main_app::compact_task_finish_payload(payload)
    }
}

pub fn format_task_finish(payload: &JsonValue, requested: &str) -> Result<String, String> {
    crate::main_app::render_task_finish_text(&crate::main_app::task_workflow_display_payload(
        payload, requested,
    ))
}

pub fn project_snapshot_capture(payload: &JsonValue, full: bool) -> JsonValue {
    let mut output = payload.clone();
    if let Some(object) = output.as_object_mut() {
        object.remove("phase_timings_ms");
    }
    if full {
        output
    } else {
        crate::main_app::compact_snapshot_create_payload(&output)
    }
}

pub fn format_snapshot_capture(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::result_text(
        "ait-cli snapshot create",
        payload,
        &[
            "snapshot_id",
            "task_id",
            "line_name",
            "parent_snapshot_id",
            "message",
        ],
    )
}

macro_rules! fixed_result_text {
    ($(($name:ident, $title:literal, $fields:expr)),* $(,)?) => {$(
        pub fn $name(payload: &JsonValue) -> Result<String, String> { crate::main_app::result_text($title, payload, $fields) }
    )*};
}

fixed_result_text! {
    (format_line_create, "ait-cli line create", &[
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "current_line",
                    "switched",
                ]),
    (format_line_switch, "ait-cli line switch", &[
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "current_line",
                ]),
    (format_line_show, "ait-cli line show", &[
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "archived_at",
                    "created_at",
                    "updated_at",
                ]),
    (format_line_archive, "ait-cli line archive", &[
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "archived_at",
                ]),
    (format_line_rename, "ait-cli line rename", &[
                    "contract",
                    "operation",
                    "line_id",
                    "old_line_name",
                    "new_line_name",
                    "head_snapshot_id",
                    "remote",
                ]),
    (format_line_delete, "ait-cli line delete", &[
                    "contract",
                    "operation",
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "history_preserved_by",
                    "tombstone",
                    "snapshots_deleted",
                    "remote",
                ]),
    (format_line_merge, "ait-cli line merge", &[
                    "status",
                    "target_line_name",
                    "source_line_name",
                    "merge_snapshot_id",
                    "merge_snapshot_created",
                    "conflict_count",
                    "conflict_paths",
                ]),
}

fixed_result_text! {
    (format_change_create, "ait-cli change create", &[
                    "task_id",
                    "title",
                    "base_line",
                    "fork_snapshot_id",
                    "status",
                ]),
    (format_change_show, "ait-cli change show", &[
                    "task_id",
                    "change_ref",
                    "change_id",
                    "title",
                    "base_line",
                    "fork_snapshot_id",
                    "status",
                    "publication_state",
                ]),
    (format_change_revert, "ait-cli change revert", &[
                    "fork_snapshot_id",
                    "latest_change_snapshot_id",
                    "current_line",
                    "applied",
                    "affected_path_count",
                    "conflict_paths",
                ]),
    (format_change_replay, "ait-cli change replay", &[
                    "fork_snapshot_id",
                    "latest_change_snapshot_id",
                    "onto_line",
                    "applied",
                    "affected_path_count",
                    "conflict_paths",
                ]),
    (format_change_close, "ait-cli change close", &["task_id", "status", "publication_state"]),
    (format_change_publish, "ait-cli change publish", &[
                    "task_id",
                    "publication_state",
                    "published_remote_name",
                ]),
    (format_patchset_publish, "ait-cli patchset publish", &[
                    "patchset_id",
                    "current_line",
                    "base_snapshot_id",
                    "revision_snapshot_id",
                    "author_mode",
                ]),
    (format_patchset_select, "ait-cli patchset select", &["current_patchset_id", "selection_changed"]),
    (format_patchset_rerun_ci, "ait-cli patchset rerun-ci", &["patchset_id", "queued", "trigger"]),
}

fixed_result_text! {
    (format_snapshot_restore_lines, "ait-cli snapshot restore-lines", &[
                    "mode",
                    "snapshot_id",
                    "path",
                    "blob_id",
                    "selected_range",
                    "selected_line_count",
                    "source_line_count",
                    "workspace_line_count",
                    "changed_line_count",
                    "would_overwrite_selected_local_edits",
                    "unchanged_outside_selected_range",
                    "creates_snapshot",
                    "applied",
                ]),
    (format_snapshot_revert, "ait-cli snapshot revert", &[
                    "snapshot_id",
                    "parent_snapshot_id",
                    "current_line",
                    "applied",
                    "affected_path_count",
                ]),
    (format_snapshot_replay, "ait-cli snapshot replay", &[
                    "snapshot_id",
                    "parent_snapshot_id",
                    "onto_line",
                    "applied",
                    "affected_path_count",
                    "conflict_paths",
                ]),
}

pub fn format_snapshot_diff(payload: &JsonValue) -> Result<String, String> {
    let rows = payload
        .get("files")
        .and_then(JsonValue::as_array)
        .ok_or("Snapshot diff result requires files.")?;
    Ok(crate::render::list_text(rows, &["status", "path", "diff"]))
}

fixed_result_text! {
    (format_auth_whoami, "ait-cli auth whoami", &["identity", "mode", "repo_name"]),
    (format_auth_grant, "ait-cli auth grant", &["repo_name", "actor_identity", "roles"]),
}

pub fn format_stash_list(payload: &JsonValue) -> Result<String, String> {
    let rows = payload
        .as_array()
        .ok_or("stash list payload must decode to a list.")?;
    Ok(crate::render::list_text(
        rows,
        &[
            "stash_id",
            "source_line_name",
            "snapshot_id",
            "file_count",
            "created_at",
            "message",
        ],
    ))
}
fixed_result_text! {
    (format_stash_drop, "ait-cli stash drop", &["stash_id", "snapshot_id", "dropped", "snapshot_deleted"]),
}

pub fn format_stash_save(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::stash_record_text("ait-cli stash save", payload)
}

pub fn format_stash_show(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::stash_record_text("ait-cli stash show", payload)
}

pub fn format_stash_apply(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::stash_restore_text("ait-cli stash apply", payload)
}

pub fn format_stash_pop(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::stash_restore_text("ait-cli stash pop", payload)
}

fixed_result_text! {
    (format_tag_create, "ait-cli tag create", &["name", "snapshot_id", "message", "created_at", "source_line"]),
    (format_tag_show, "ait-cli tag show", &["name", "snapshot_id", "message", "created_at"]),
    (format_tag_delete, "ait-cli tag delete", &["name", "snapshot_id", "message", "deleted"]),
}

pub fn format_tag_list(payload: &JsonValue) -> Result<String, String> {
    let rows = payload
        .as_array()
        .ok_or("tag list payload must decode to a list.")?;
    Ok(crate::render::list_text(
        rows,
        &["name", "snapshot_id", "message", "created_at"],
    ))
}

pub fn project_review_task_operation(operation: &JsonValue) -> Result<JsonValue, String> {
    operation
        .get("payload")
        .cloned()
        .ok_or("Review operation requires payload.".into())
}

fn review_task_operation_text(title: &str, operation: &JsonValue) -> Result<String, String> {
    let payload = operation
        .get("payload")
        .ok_or("Review operation requires payload.")?;
    let package = operation
        .get("package_review")
        .and_then(JsonValue::as_bool)
        .ok_or("Review operation requires package_review.")?;
    if package {
        crate::main_app::result_text(
            "ait-cli review task",
            payload,
            &["task_id", "reviewer", "action", "recorded_at_s"],
        )
    } else {
        crate::main_app::result_text(title, payload, &["patchset_id", "reviewer", "action"])
    }
}

pub fn format_review_task_approve(operation: &JsonValue) -> Result<String, String> {
    review_task_operation_text("ait-cli review task approve", operation)
}

pub fn format_review_task_request_changes(operation: &JsonValue) -> Result<String, String> {
    review_task_operation_text("ait-cli review task request-changes", operation)
}

pub fn format_review_task_comment(operation: &JsonValue) -> Result<String, String> {
    review_task_operation_text("ait-cli review task comment", operation)
}

pub fn format_review_task_defer(operation: &JsonValue) -> Result<String, String> {
    review_task_operation_text("ait-cli review task defer", operation)
}

fixed_result_text! {
    (format_review_team_request, "ait-cli review team request", &["patchset_id", "requested_groups", "status"]),
    (format_review_team_approve, "ait-cli review team approve", &["patchset_id", "reviewer", "action"]),
    (format_review_team_request_changes, "ait-cli review team request-changes", &["patchset_id", "reviewer", "action"]),
    (format_review_team_comment, "ait-cli review team comment", &["patchset_id", "reviewer", "action"]),
    (format_review_team_defer, "ait-cli review team defer", &["patchset_id", "reviewer", "action"]),
}

pub fn project_review_show(payload: &JsonValue) -> JsonValue {
    crate::main_app::compact_review_show_payload(payload)
}

pub fn format_review_show(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::review_show_text(payload)
}

pub fn format_review_code_submit(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::review_code_submit_text(payload)
}

pub fn format_review_code_template(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::review_code_template_text(payload)
}

fixed_result_text! {
    (format_attest_put, "ait-cli attest put", &["patchset_id", "author_mode", "verification_state", "revoked", "require_tests_pass", "require_lint_pass", "require_human_review", "ci_backed"]),
    (format_attest_show, "ait-cli attest show", &["attestation_id", "patchset_id", "author_mode", "verification_state", "revoked", "require_tests_pass", "require_lint_pass", "require_human_review", "ci_backed"]),
    (format_policy_eval, "ait-cli policy eval", &["patchset_id", "lane", "decision", "evaluated_at"]),
    (format_policy_waive, "ait-cli policy waive", &["waiver_id", "patchset_id", "rule_name", "expires_at"]),
}

pub fn format_policy_show(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::policy_show_text(payload)
}
pub fn project_policy_show(payload: &JsonValue) -> JsonValue {
    crate::main_app::compact_policy_show_payload(payload)
}

pub fn format_blame(payload: &JsonValue) -> String {
    crate::blame_surface::human_blame_text(payload)
}

pub fn render_workspace_diff_names(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::workspace_diff_name_only_stdout(payload)
}

pub fn render_workspace_diff_stat(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::workspace_diff_stat_stdout(payload)
}

pub fn render_workspace_diff_text(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::workspace_diff_text_stdout(payload)
}

fixed_result_text! {
    (format_pull, "ait-cli pull", &[
            "remote",
            "repo_name",
            "mode",
            "line",
            "relationship",
            "action",
            "local_line_present",
            "local_line_head_snapshot_id",
            "imported_snapshots",
            "head_snapshot_id",
            "line_head_updated",
            "workspace_restored",
            "restore_applied",
        ]),
    (format_push, "ait-cli push", &[
            "remote",
            "repo_name",
            "line",
            "pushed_snapshots",
            "checked_snapshots",
            "uploaded_snapshots",
            "skipped_snapshots",
            "head_snapshot_id",
            "sync_scope",
            "sync_reason",
            "bounded_by_snapshot_id",
        ]),
}

pub fn project_remote_sync(payload: &JsonValue) -> JsonValue {
    let mut output = payload.clone();
    if let Some(object) = output.as_object_mut() {
        object.remove("phase_timings_ms");
        object.remove("remote_sync_metrics");
    }
    output
}

pub fn format_worktree_status(payload: &JsonValue, verbose: bool) -> Result<String, String> {
    crate::main_app::worktree_status_text(payload, verbose)
}

pub fn project_worktree_path(payload: &JsonValue) -> Result<JsonValue, String> {
    crate::main_app::worktree_path_payload(payload)
}

pub fn format_worktree_path(payload: &JsonValue, shell_output: bool) -> Result<String, String> {
    if shell_output {
        crate::main_app::required_object_string_field(payload, "shell_command")
    } else {
        crate::main_app::worktree_open_path_text(payload)
    }
}

macro_rules! worktree_text {
    ($($name:ident => $target:ident),* $(,)?) => {$(
        pub fn $name(payload: &JsonValue) -> Result<String, String> { crate::main_app::$target(payload) }
    )*};
}

worktree_text! {
    format_worktree_list => worktree_list_text,
    format_worktree_doctor => worktree_doctor_text,
    format_worktree_cleanup_candidates => worktree_cleanup_candidates_text,
    format_worktree_sync => worktree_sync_text,
    format_worktree_prune_stale => worktree_prune_text,
}

pub fn format_worktree_cleanup(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::worktree_cleanup_rows_text(
        "ait-cli worktree cleanup",
        payload,
        "planned_count",
        "planned_rows",
        "removed_count",
        "removed_rows",
    )
}

pub fn format_worktree_remove(
    payload: &JsonValue,
    all_stale: bool,
    dry_run: bool,
    multiple: bool,
) -> Result<String, String> {
    if all_stale {
        return format_worktree_prune_stale(payload);
    }
    if dry_run || multiple {
        return crate::main_app::worktree_cleanup_rows_text(
            "ait-cli worktree remove",
            payload,
            "planned_count",
            "planned_rows",
            "removed_count",
            "removed_rows",
        );
    }
    let row = payload
        .get("removed_rows")
        .and_then(JsonValue::as_array)
        .and_then(|rows| rows.first())
        .ok_or("worktree remove payload is missing removed_rows[0].")?;
    crate::main_app::result_text(
        "ait-cli worktree remove",
        row,
        &[
            "name",
            "path",
            "deleted_path",
            "alias_path",
            "target_base_line",
        ],
    )
}

fixed_result_text! {
    (format_worktree_restore, "ait-cli worktree restore", &["worktree_name", "workspace_root", "current_line", "line_name", "line_head_snapshot_id", "target_snapshot_id", "applied"]),
    (format_worktree_show, "ait-cli worktree show", &["name", "path", "alias_path", "current_line", "registered_line_name", "workspace_status", "changed_count", "bound_task_id", "target_base_line", "cleanup_policy", "rebase_state", "merge_state", "merge_conflict_count"]),
    (format_worktree_recreate, "ait-cli worktree recreate", &["name", "path", "status", "current_line", "target_base_line", "fork_snapshot_id"]),
    (format_worktree_recover_task, "ait-cli worktree recover-task", &["status", "task_id", "name", "path", "current_line"]),
    (format_worktree_restore_owned_head, "ait-cli worktree restore-owned-head", &["name", "path", "status", "restored_snapshot_id", "owned_head_snapshot_id", "current_line"]),
    (format_worktree_rebase, "ait-cli worktree rebase", &["name", "path", "status", "current_line", "target_base_line", "rebase_state", "conflict_count"]),
}

fixed_result_text! {
    (format_git_migrate, "ait-cli git migrate", &["status", "mutated", "staging_root"]),
    (format_git_recover, "ait-cli git recover", &["status", "mutated", "repair_files"]),
    (format_git_import, "ait-cli git import", &[
                    "status",
                    "operation_id",
                    "generation_id",
                    "source_repository_fingerprint",
                    "git_object_format",
                    "commit_count",
                    "imported_commit_count",
                    "reused_commit_count",
                    "line_count",
                    "tag_count",
                    "mutated",
                    "workspace_bootstrap",
                ]),
    (format_git_export, "ait-cli git export", &[
                    "status",
                    "operation_id",
                    "generation_id",
                    "target_repository_fingerprint",
                    "git_object_format",
                    "snapshot_count",
                    "exact_git_object_reuse_count",
                    "native_commit_count",
                    "ref_count",
                    "compare_and_swap",
                    "force_updated",
                    "fsck",
                    "mutated",
                ]),
    (format_git_restore, "ait-cli git restore", &[
                    "status",
                    "generation_id",
                    "git_object_id",
                    "file_count",
                    "markdown_count",
                    "target",
                ]),
    (format_git_mirror, "ait-cli git mirror", &[
                    "status",
                    "operation_id",
                    "generation_id",
                    "direction",
                    "endpoint_repository_fingerprint",
                    "state",
                    "equal_count",
                    "inbound_only_count",
                    "outbound_only_count",
                    "divergent_count",
                    "compare_and_swap",
                    "force_updated",
                    "mutated",
                ]),
}

pub fn format_gc_stats(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::gc_payload_text("ait-cli gc stats", payload)
}

pub fn format_gc_validate(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::gc_payload_text("ait-cli gc validate", payload)
}

pub fn format_gc_prune(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::gc_payload_text("ait-cli gc prune", payload)
}

pub fn format_current_source_cache_contract(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text("ait-cli current-source-cache contract", payload)
}

pub fn format_current_source_cache_mark_building(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text(
        "ait-cli current-source-cache mark-building",
        payload,
    )
}

pub fn format_current_source_cache_activate(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text("ait-cli current-source-cache activate", payload)
}

pub fn format_current_source_cache_release_lease(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text(
        "ait-cli current-source-cache release-lease",
        payload,
    )
}

pub fn format_current_source_cache_prune(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text("ait-cli current-source-cache prune", payload)
}

pub fn format_current_source_cache_seed_canonical(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text(
        "ait-cli current-source-cache seed-canonical",
        payload,
    )
}

pub fn format_current_source_cache_extension_fresh(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text(
        "ait-cli current-source-cache extension-fresh",
        payload,
    )
}

pub fn format_current_source_cache_binary_fresh(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text("ait-cli current-source-cache binary-fresh", payload)
}

pub fn format_current_source_cache_core_fingerprint(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text(
        "ait-cli current-source-cache core-fingerprint",
        payload,
    )
}

pub fn format_current_source_cache_server_fingerprint(
    payload: &JsonValue,
) -> Result<String, String> {
    crate::main_app::current_source_cache_text(
        "ait-cli current-source-cache server-fingerprint",
        payload,
    )
}

pub fn format_current_source_cache_core_mtime(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text("ait-cli current-source-cache core-mtime", payload)
}

pub fn format_current_source_cache_server_mtime(payload: &JsonValue) -> Result<String, String> {
    crate::main_app::current_source_cache_text("ait-cli current-source-cache server-mtime", payload)
}

fixed_result_text! {
    (format_remote_recover_head, "ait-cli remote recover-head", &["apply", "remote", "repo_name", "line_name", "snapshot_id", "source_parent_snapshot_id", "history_mode",
"reachable_snapshot_count", "object_pack_count", "tree_pack_count", "downloaded_object_packs", "downloaded_tree_packs",
"captured_file_count", "content_fingerprint", "authority_root", "pack_root", "activation_strategy", "single_syscall_atomic",
"activation_lock_protected", "staging_removed"]),
}
