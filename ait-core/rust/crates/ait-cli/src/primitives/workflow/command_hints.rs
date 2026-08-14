use super::*;
use crate::json_support::parse_value_option;

fn workflow_command_with_remote_scope(
    command: impl Into<String>,
    remote_name: Option<&str>,
) -> String {
    let command = command.into();
    normalized_text(remote_name)
        .map(|remote_name| format!("{command} --remote {remote_name}"))
        .unwrap_or(command)
}

pub(in crate::primitives) fn workflow_patchset_ci_contract_exists(repo: &RepoRuntime) -> bool {
    workflow_patchset_ci_catalog_path(&repo.workspace_root()).is_some()
}

pub(in crate::primitives) fn workflow_patchset_ci_catalog_path(root: &Path) -> Option<PathBuf> {
    let default_catalog = root.join("ci").join("patch_ci.json");
    if default_catalog.is_file() {
        return Some(default_catalog);
    }

    let contract_path = root.join("ci").join("config.contract.json");
    let text = fs::read_to_string(contract_path).ok()?;
    let contract = parse_value_option(&text)?;
    let suite_manifest_path = contract
        .get("ci")
        .and_then(JsonValue::as_object)
        .and_then(|ci| ci.get("suite_manifest_path"))
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))?;
    let catalog_path = root.join(suite_manifest_path);
    catalog_path.is_file().then_some(catalog_path)
}

pub(in crate::primitives) fn workflow_land_patchset_command(
    change_id: &str,
    remote_name: Option<&str>,
    base_line_name: &str,
    worktree_retarget: Option<&JsonValue>,
) -> String {
    let publish_command = workflow_command_with_remote_scope(
        format!("ait patchset publish --change {change_id} --summary \"review summary\""),
        remote_name,
    );
    let Some(worktree_retarget) = worktree_retarget.and_then(JsonValue::as_object) else {
        return publish_command;
    };
    if worktree_retarget
        .get("rebase_state")
        .and_then(JsonValue::as_str)
        .map(|value| value == "conflicted")
        .unwrap_or(false)
    {
        return "ait worktree rebase --continue".to_string();
    }
    if worktree_retarget
        .get("needs_retarget")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return format!("ait worktree rebase --onto {base_line_name}");
    }
    publish_command
}

pub(in crate::primitives) fn workflow_ready_command_hints(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
    patchset: Option<&JsonValue>,
    base_line_name: &str,
    worktree_retarget: Option<&JsonValue>,
) -> JsonValue {
    let patchset_id = patchset.and_then(|value| string_field(value, "patchset_id"));
    let publish_command =
        workflow_land_patchset_command(change_id, remote_name, base_line_name, worktree_retarget);
    let patchset_ci_command = if patchset_id.is_some() && workflow_patchset_ci_contract_exists(repo)
    {
        patchset_id
            .as_ref()
            .map(|value| {
                JsonValue::String(workflow_command_with_remote_scope(
                    format!("ait patchset rerun-ci {value}"),
                    remote_name,
                ))
            })
            .unwrap_or(JsonValue::Null)
    } else {
        JsonValue::Null
    };
    let attest_command = patchset_id
        .as_ref()
        .map(|value| {
            JsonValue::String(workflow_command_with_remote_scope(
                format!("ait attest put {value} --tests pass"),
                remote_name,
            ))
        })
        .unwrap_or(JsonValue::Null);
    let task_review_enabled = workflow_task_review_enabled(repo);
    let auto_review_reviewer = if task_review_enabled {
        None
    } else {
        repo.task_review_auto_approval_reviewer_identity(None)
    };
    let review_command = if let Some(value) = patchset_id.as_ref() {
        format!("ait review task approve {change_id} --patchset {value}")
    } else {
        format!("ait review task approve {change_id}")
    };
    let review_command = workflow_command_with_remote_scope(review_command, remote_name);
    json!({
        "apply_command": workflow_command_with_remote_scope(format!("ait workflow ready {change_id} --apply"), remote_name),
        "publish_command": publish_command,
        "patchset_ci_command": patchset_ci_command,
        "attest_command": attest_command,
        "attestation_command": if !patchset_ci_command.is_null() { patchset_ci_command.clone() } else { attest_command.clone() },
        "review_command": review_command,
        "auto_review_reviewer": auto_review_reviewer,
        "policy_command": patchset_id.as_ref().map(|value| JsonValue::String(workflow_command_with_remote_scope(format!("ait policy eval {value}"), remote_name))).unwrap_or(JsonValue::Null),
        "land_command": workflow_command_with_remote_scope(format!("ait task land {change_id}"), remote_name),
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "command hints keep each independently available workflow command explicit"
)]
pub(in crate::primitives) fn workflow_land_command_hints(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
    _task_id: &str,
    patchset: Option<&JsonValue>,
    base_line_name: &str,
    _target_line: &str,
    worktree_retarget: Option<&JsonValue>,
    review_blocking: i64,
    requires_code_review_summary: bool,
) -> JsonValue {
    let team_review_enabled = repo.team_review_enabled();
    let patchset_id = patchset.and_then(|value| string_field(value, "patchset_id"));
    let apply_command =
        workflow_command_with_remote_scope(format!("ait task land {change_id}"), remote_name);
    let publish_command =
        workflow_land_patchset_command(change_id, remote_name, base_line_name, worktree_retarget);
    let patchset_ci_command = if patchset_id.is_some() && workflow_patchset_ci_contract_exists(repo)
    {
        patchset_id
            .as_ref()
            .map(|value| {
                JsonValue::String(workflow_command_with_remote_scope(
                    format!("ait patchset rerun-ci {value}"),
                    remote_name,
                ))
            })
            .unwrap_or(JsonValue::Null)
    } else {
        JsonValue::Null
    };
    let attest_command = patchset_id
        .as_ref()
        .map(|value| {
            JsonValue::String(workflow_command_with_remote_scope(
                format!("ait attest put {value} --tests pass"),
                remote_name,
            ))
        })
        .unwrap_or(JsonValue::Null);
    let code_review_summary_command = patchset_id
        .as_ref()
        .map(|value| {
            JsonValue::String(workflow_command_with_remote_scope(
                format!(
                    "ait review code submit {change_id} --patchset {value} --verdict pass --message \"{CODE_REVIEW_SUMMARY_TEMPLATE}\""
                ),
                remote_name,
            ))
        })
        .unwrap_or(JsonValue::Null);
    let task_review_enabled = workflow_task_review_enabled(repo);
    let auto_review_reviewer = if task_review_enabled {
        None
    } else {
        repo.task_review_auto_approval_reviewer_identity(None)
    };
    let manual_review_command = if let Some(value) = patchset_id.as_ref() {
        format!("ait review task approve {change_id} --patchset {value}")
    } else {
        format!("ait review task approve {change_id}")
    };
    let manual_review_command =
        workflow_command_with_remote_scope(manual_review_command, remote_name);
    let team_review_command = if let Some(value) = patchset_id.as_ref() {
        if team_review_enabled {
            JsonValue::String(workflow_command_with_remote_scope(
                format!("ait review team approve {change_id} --patchset {value}"),
                remote_name,
            ))
        } else {
            JsonValue::Null
        }
    } else if team_review_enabled {
        JsonValue::String(workflow_command_with_remote_scope(
            format!("ait review team approve {change_id}"),
            remote_name,
        ))
    } else {
        JsonValue::Null
    };
    let review_command = if review_blocking > 0 {
        workflow_command_with_remote_scope(format!("ait review show {change_id}"), remote_name)
    } else if auto_review_reviewer.is_some() {
        apply_command.clone()
    } else {
        manual_review_command.clone()
    };
    let land_command = apply_command.clone();
    json!({
        "publish_command": publish_command,
        "apply_command": apply_command,
        "ready_command": workflow_command_with_remote_scope(format!("ait workflow ready {change_id} --apply"), remote_name),
        "patchset_ci_command": patchset_ci_command,
        "attest_command": attest_command,
        "attestation_command": if !patchset_ci_command.is_null() { patchset_ci_command.clone() } else { attest_command.clone() },
        "code_review_summary_command": code_review_summary_command,
        "code_review_template_command": if patchset_id.is_some() && requires_code_review_summary {
            JsonValue::String(CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND.to_string())
        } else {
            JsonValue::Null
        },
        "review_command": review_command,
        "manual_review_command": manual_review_command,
        "team_review_command": team_review_command,
        "auto_review_reviewer": auto_review_reviewer,
        "policy_command": patchset_id.as_ref().map(|value| JsonValue::String(workflow_command_with_remote_scope(format!("ait policy eval {value}"), remote_name))).unwrap_or(JsonValue::Null),
        "land_command": land_command,
        "task_complete_command": workflow_command_with_remote_scope(format!("ait task land {change_id}"), remote_name),
    })
}
