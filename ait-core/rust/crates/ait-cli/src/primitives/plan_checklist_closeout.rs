use super::*;
use crate::primitives::sprint_card_retention::apply_completed_sprint_card_retention;
use ait_core::object_diff::artifact_blob_id;
use ait_core::plan_command_execution::execute_plan_show_command_request_json;
use ait_core::plan_filesystem::{is_markdown_artifact_path, resolve_repo_artifact_path};
use ait_core::plan_items::{close_plan_item_checkbox, PlanChecklistCloseoutStatus};
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;

const PLAN_BINARY_DB_WRITE_LAYOUT: u32 = 1;

pub(in crate::primitives) fn close_task_plan_checklist_item(
    repo: &RepoRuntime,
    task: &JsonValue,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let Some(plan_id) = string_field(task, "plan_id") else {
        return Ok(skipped("no_plan_binding"));
    };
    let Some(plan_item_ref) = string_field(task, "plan_item_ref") else {
        return Ok(skipped("no_plan_item_ref"));
    };
    let Some(origin_plan_revision_id) = string_field(task, "origin_plan_revision_id") else {
        return Ok(json!({
            "status": "skipped",
            "reason": "no_origin_plan_revision_id",
            "plan_id": plan_id,
            "plan_item_ref": plan_item_ref,
        }));
    };
    let use_local_scope = remote_name.is_none();
    let origin_plan = execute_plan_show_command_request_json(
        &plan_show_request(repo, &plan_id, Some(&origin_plan_revision_id), remote_name)?
            .to_string(),
    )?;
    let origin_revision = shown_plan_revision(&origin_plan, &plan_id, "bound origin")?;
    if string_field(origin_revision, "plan_revision_id").as_deref()
        != Some(origin_plan_revision_id.as_str())
    {
        return Ok(json!({
            "status": "skipped",
            "reason": "origin_plan_revision_mismatch",
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "resolved_plan_revision_id": string_field(origin_revision, "plan_revision_id"),
            "plan_item_ref": plan_item_ref,
        }));
    }
    if let Some(result) = validate_origin_plan_item(
        origin_revision,
        &plan_id,
        &origin_plan_revision_id,
        &plan_item_ref,
    ) {
        return Ok(result);
    }

    let plan = execute_plan_show_command_request_json(
        &plan_show_request(repo, &plan_id, None, remote_name)?.to_string(),
    )?;
    let revision = shown_plan_revision(&plan, &plan_id, "head")?;
    let head_plan_revision_id = string_field(revision, "plan_revision_id")
        .ok_or_else(|| format!("Plan {plan_id} head revision has no revision id."))?;
    let artifact_path = string_field(revision, "artifact_path")
        .ok_or_else(|| format!("Plan {plan_id} has no Markdown artifact path."))?;
    if !is_markdown_artifact_path(&artifact_path) {
        return Ok(json!({
            "status": "skipped",
            "reason": "artifact_is_not_markdown",
            "plan_id": plan_id,
            "plan_item_ref": plan_item_ref,
            "artifact_path": artifact_path,
        }));
    }
    let artifact_selector = string_field(revision, "artifact_selector");
    let origin_artifact_path = string_field(origin_revision, "artifact_path");
    let origin_artifact_selector = string_field(origin_revision, "artifact_selector");
    if origin_artifact_path.as_deref() != Some(artifact_path.as_str())
        || origin_artifact_selector != artifact_selector
    {
        return Ok(json!({
            "status": "skipped",
            "reason": "bound_artifact_changed",
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "head_plan_revision_id": head_plan_revision_id,
            "plan_item_ref": plan_item_ref,
            "origin_artifact_path": origin_artifact_path,
            "origin_artifact_selector": origin_artifact_selector,
            "artifact_path": artifact_path,
            "artifact_selector": artifact_selector,
        }));
    }
    let root = repo.authoritative_repo_root();
    let resolved =
        resolve_repo_artifact_path(root.to_string_lossy().as_ref(), &artifact_path, false)
            .map_err(|err| {
                format!("Could not resolve bound plan artifact {artifact_path}: {err:?}")
            })?;
    let resolved_path = resolved
        .get("resolved_path")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "Resolved plan artifact payload is missing `resolved_path`.".to_string())?;
    let markdown = fs::read_to_string(resolved_path)
        .map_err(|err| format!("Failed to read bound plan artifact {artifact_path}: {err}"))?;
    let current_artifact_blob_id = artifact_blob_id(&markdown);
    let Some(head_artifact_blob_id) = string_field(revision, "artifact_blob_id") else {
        return Ok(json!({
            "status": "skipped",
            "reason": "head_artifact_blob_id_missing",
            "plan_id": plan_id,
            "head_plan_revision_id": head_plan_revision_id,
            "plan_item_ref": plan_item_ref,
            "artifact_path": artifact_path,
        }));
    };
    if current_artifact_blob_id != head_artifact_blob_id {
        return Ok(json!({
            "status": "skipped",
            "reason": "artifact_has_unsynced_drift",
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "head_plan_revision_id": head_plan_revision_id,
            "plan_item_ref": plan_item_ref,
            "artifact_path": artifact_path,
            "expected_artifact_blob_id": head_artifact_blob_id,
            "current_artifact_blob_id": current_artifact_blob_id,
        }));
    }
    let closeout = close_plan_item_checkbox(&markdown, &plan_item_ref);
    if matches!(
        closeout.status,
        PlanChecklistCloseoutStatus::Missing
            | PlanChecklistCloseoutStatus::Ambiguous
            | PlanChecklistCloseoutStatus::NotCheckbox
    ) {
        return Ok(json!({
            "status": "skipped",
            "reason": closeout.status.as_str(),
            "plan_id": plan_id,
            "plan_item_ref": plan_item_ref,
            "artifact_path": artifact_path,
            "line_number": closeout.line_number,
        }));
    }
    let updated = closeout.status == PlanChecklistCloseoutStatus::Updated;
    if updated {
        fs::write(resolved_path, &closeout.markdown).map_err(|err| {
            format!("Failed to close bound checklist item in {artifact_path}: {err}")
        })?;
    }
    let sync_result = execute_plan_sync_command_request_json(
        &plan_sync_request(
            repo,
            &artifact_path,
            artifact_selector.as_deref(),
            remote_name,
            false,
        )?
        .to_string(),
    );
    let sync = match sync_result {
        Ok(sync) if sync.get("status").and_then(JsonValue::as_str) == Some("ok") => sync,
        Ok(sync) => {
            let error = string_field(&sync, "error")
                .unwrap_or_else(|| "plan sync returned a non-ok result".to_string());
            return Err(restore_markdown_after_sync_failure(
                resolved_path,
                &markdown,
                updated,
                &artifact_path,
                &error,
            ));
        }
        Err(error) => {
            return Err(restore_markdown_after_sync_failure(
                resolved_path,
                &markdown,
                updated,
                &artifact_path,
                &error,
            ));
        }
    };
    let retention = apply_completed_sprint_card_retention(repo, &artifact_path, remote_name)
        .map_err(|error| {
            format!(
                "Task landed and bound checklist sync succeeded, but sprint-card retention failed: {error}"
            )
        })?;
    Ok(json!({
        "status": "synced",
        "reason": if updated { "checklist_closed" } else { "already_done_resynced" },
        "scope": if use_local_scope { "local" } else { "remote" },
        "remote": remote_name,
        "plan_id": plan_id,
        "origin_plan_revision_id": origin_plan_revision_id,
        "validated_head_plan_revision_id": head_plan_revision_id,
        "plan_item_ref": plan_item_ref,
        "artifact_path": artifact_path,
        "artifact_selector": artifact_selector,
        "line_number": closeout.line_number,
        "updated": updated,
        "sync": sync,
        "retention": retention,
    }))
}

pub(in crate::primitives) fn inspect_task_plan_checklist_item(
    repo: &RepoRuntime,
    task: &JsonValue,
    remote_name: &str,
) -> Result<JsonValue, String> {
    let Some(plan_id) = string_field(task, "plan_id") else {
        return Ok(json!({
            "status": "unbound",
            "reason": "no_plan_binding",
            "scope": "remote",
            "remote": remote_name,
        }));
    };
    if string_field(task, "plan_item_ref").is_none() {
        return Ok(json!({
            "status": "invalid",
            "reason": "no_plan_item_ref",
            "scope": "remote",
            "remote": remote_name,
            "plan_id": plan_id,
        }));
    }
    let plan = execute_plan_show_command_request_json(
        &plan_show_request(repo, &plan_id, None, Some(remote_name))?.to_string(),
    )?;
    task_plan_checklist_evidence_from_shown_plan(task, &plan, remote_name)
}

fn task_plan_checklist_evidence_from_shown_plan(
    task: &JsonValue,
    plan: &JsonValue,
    remote_name: &str,
) -> Result<JsonValue, String> {
    let plan_id = required_string_field(task, "plan_id")?;
    let plan_item_ref = required_string_field(task, "plan_item_ref")?;
    let revision = shown_plan_revision(plan, &plan_id, "head")?;
    let head_plan_revision_id = string_field(revision, "plan_revision_id")
        .ok_or_else(|| format!("Plan {plan_id} head revision has no revision id."))?;
    let matches = revision
        .get("items")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    string_field(item, "plan_item_ref").as_deref() == Some(plan_item_ref.as_str())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let common = json!({
        "scope": "remote",
        "remote": remote_name,
        "plan_id": plan_id,
        "plan_item_ref": plan_item_ref,
        "head_plan_revision_id": head_plan_revision_id,
        "artifact_path": string_field(revision, "artifact_path"),
        "artifact_selector": string_field(revision, "artifact_selector"),
    });
    if matches.len() != 1 {
        let mut evidence = common;
        let object = evidence
            .as_object_mut()
            .ok_or_else(|| "Plan checklist evidence must be an object.".to_string())?;
        object.insert("status".to_string(), json!("invalid"));
        object.insert(
            "reason".to_string(),
            json!(if matches.is_empty() {
                "head_plan_item_missing"
            } else {
                "head_plan_item_ambiguous"
            }),
        );
        object.insert("match_count".to_string(), json!(matches.len()));
        return Ok(evidence);
    }
    let item = matches[0];
    let checkbox_state = string_field(item, "checkbox_state").unwrap_or_default();
    let (status, reason) = match checkbox_state.as_str() {
        "done" => ("done", "head_plan_item_done"),
        "open" => ("pending", "head_plan_item_open"),
        "none" => ("invalid", "head_plan_item_not_checkbox"),
        _ => ("invalid", "head_plan_item_checkbox_state_unknown"),
    };
    let mut evidence = common;
    let object = evidence
        .as_object_mut()
        .ok_or_else(|| "Plan checklist evidence must be an object.".to_string())?;
    object.insert("status".to_string(), json!(status));
    object.insert("reason".to_string(), json!(reason));
    object.insert("checkbox_state".to_string(), json!(checkbox_state));
    object.insert(
        "line_number".to_string(),
        item.get("line_number").cloned().unwrap_or(JsonValue::Null),
    );
    Ok(evidence)
}

fn plan_show_request(
    repo: &RepoRuntime,
    plan_id: &str,
    revision: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if let Some(remote_name) = remote_name {
        let remote = repo.remote_row(Some(remote_name))?;
        return Ok(json!({
            "scope": "remote",
            "base_url": remote.url,
            "headers": repo.auth_headers(),
            "repository_index": repo.repository_index(),
            "repo_name": remote.repo_name.unwrap_or_else(|| repo.repo_name()),
            "remote": remote.name,
            "plan_id": plan_id,
            "revision": revision,
        }));
    }
    Ok(json!({
        "scope": "local",
        "repository_index": repo.repository_index(),
        "repo_name": repo.repo_name(),
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
        "plan_id": plan_id,
        "revision": revision,
    }))
}

fn shown_plan_revision<'a>(
    payload: &'a JsonValue,
    plan_id: &str,
    label: &str,
) -> Result<&'a JsonValue, String> {
    payload
        .get("revision")
        .or_else(|| payload.get("head_revision"))
        .or_else(|| {
            payload
                .get("plan")
                .and_then(|value| value.get("head_revision"))
        })
        .ok_or_else(|| format!("Plan {plan_id} has no {label} revision for checklist closeout."))
}

fn validate_origin_plan_item(
    revision: &JsonValue,
    plan_id: &str,
    origin_plan_revision_id: &str,
    plan_item_ref: &str,
) -> Option<JsonValue> {
    let matches = revision
        .get("items")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    string_field(item, "plan_item_ref").as_deref() == Some(plan_item_ref)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if matches.len() != 1 {
        return Some(json!({
            "status": "skipped",
            "reason": if matches.is_empty() {
                "origin_plan_item_missing"
            } else {
                "origin_plan_item_ambiguous"
            },
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
            "match_count": matches.len(),
        }));
    }
    let item = matches[0];
    if string_field(item, "checkbox_state").as_deref() == Some("none") {
        return Some(json!({
            "status": "skipped",
            "reason": "origin_plan_item_not_checkbox",
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        }));
    }
    if item.get("taskable_hint").and_then(JsonValue::as_bool) != Some(true) {
        return Some(json!({
            "status": "skipped",
            "reason": "origin_plan_item_not_taskable",
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        }));
    }
    None
}

fn restore_markdown_after_sync_failure(
    resolved_path: &str,
    original_markdown: &str,
    updated: bool,
    artifact_path: &str,
    sync_error: &str,
) -> String {
    if !updated {
        return format!("Task landed, but plan sync failed for {artifact_path}: {sync_error}");
    }
    match fs::write(resolved_path, original_markdown) {
        Ok(()) => format!(
            "Task landed, but plan sync failed for {artifact_path}; the automatic checkbox edit was restored: {sync_error}"
        ),
        Err(restore_error) => format!(
            "Task landed and plan sync failed for {artifact_path}; restoring the automatic checkbox edit also failed ({restore_error}): {sync_error}"
        ),
    }
}

pub(super) fn plan_sync_request(
    repo: &RepoRuntime,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    remote_name: Option<&str>,
    prune: bool,
) -> Result<JsonValue, String> {
    let mut payload = json!({
        "root_path": repo.authoritative_repo_root(),
        "repo_name": repo.repo_name(),
        "repository_index": repo.repository_index(),
        "id_namespace_prefix": repo.id_namespace_prefix(),
        "created_by": repo.actor_identity(),
        "target": artifact_path,
        "plan_ref": artifact_selector,
        "prune": prune,
        "local": remote_name.is_none(),
        "remote_name": JsonValue::Null,
        "remote_repo_name": JsonValue::Null,
        "base_url": JsonValue::Null,
        "rebase": remote_name.is_some(),
        "reconcile": false,
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    });
    if let Some(remote_name) = remote_name {
        let remote = repo.remote_row(Some(remote_name))?;
        let object = payload
            .as_object_mut()
            .ok_or_else(|| "Plan checklist sync request must be an object.".to_string())?;
        object.insert("remote_name".to_string(), JsonValue::String(remote.name));
        object.insert(
            "remote_repo_name".to_string(),
            JsonValue::String(remote.repo_name.unwrap_or_else(|| repo.repo_name())),
        );
        object.insert("base_url".to_string(), JsonValue::String(remote.url));
    }
    Ok(payload)
}

fn skipped(reason: &str) -> JsonValue {
    json!({
        "status": "skipped",
        "reason": reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_surface::{init_repo, InitRequest};
    use ait_core::json_support::JsonMap;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn repo(mode: &str) -> RepoRuntime {
        let mut config = JsonMap::new();
        config.insert("repo_name".to_string(), json!("demo"));
        config.insert("default_line".to_string(), json!("main"));
        config.insert("workflow_mode".to_string(), json!(mode));
        config.insert("default_remote".to_string(), json!("origin"));
        RepoRuntime {
            root: PathBuf::from("/repo"),
            ait_dir: PathBuf::from("/repo/.ait"),
            config,
            worktree_config_path: None,
        }
    }

    #[test]
    fn unbound_tasks_skip_without_plan_reads_or_writes() {
        assert_eq!(
            close_task_plan_checklist_item(&repo("solo_local"), &json!({}), None).unwrap()
                ["reason"],
            "no_plan_binding"
        );
        assert_eq!(
            close_task_plan_checklist_item(&repo("solo_local"), &json!({"plan_id": "PL-1"}), None,)
                .unwrap()["reason"],
            "no_plan_item_ref"
        );
        assert_eq!(
            close_task_plan_checklist_item(
                &repo("solo_local"),
                &json!({
                    "plan_id": "PL-1",
                    "plan_item_ref": "card/item"
                }),
                None,
            )
            .unwrap()["reason"],
            "no_origin_plan_revision_id"
        );
    }

    #[test]
    fn closeout_requests_follow_the_explicit_land_scope() {
        let local = plan_sync_request(
            &repo("solo_local"),
            "docs/sprint.md",
            Some("root"),
            None,
            false,
        )
        .unwrap();
        assert_eq!(local["local"], true);
        assert!(local["remote_name"].is_null());
        assert_eq!(local["plan_ref"], "root");
    }

    #[test]
    fn remote_audit_evidence_distinguishes_done_open_and_invalid_items() {
        let task = json!({
            "plan_id": "PR-8",
            "plan_item_ref": "release/fix",
        });
        let shown = |checkbox_state: &str| {
            json!({
                "head_revision": {
                    "plan_revision_id": "plan-revision:9",
                    "artifact_path": "docs/sprints/release.md",
                    "artifact_selector": "release/root",
                    "items": [{
                        "plan_item_ref": "release/fix",
                        "checkbox_state": checkbox_state,
                        "line_number": 12,
                    }]
                }
            })
        };

        let done =
            task_plan_checklist_evidence_from_shown_plan(&task, &shown("done"), "origin").unwrap();
        assert_eq!(done["status"], "done");
        assert_eq!(done["artifact_path"], "docs/sprints/release.md");

        let pending =
            task_plan_checklist_evidence_from_shown_plan(&task, &shown("open"), "origin").unwrap();
        assert_eq!(pending["status"], "pending");

        let invalid =
            task_plan_checklist_evidence_from_shown_plan(&task, &shown("none"), "origin").unwrap();
        assert_eq!(invalid["status"], "invalid");
        assert_eq!(invalid["reason"], "head_plan_item_not_checkbox");
    }

    #[test]
    fn local_closeout_updates_and_syncs_the_exact_bound_item() {
        let temp = tempdir().unwrap();
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("demo".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        let artifact_path = "docs/sprints/closeout.md";
        fs::create_dir_all(temp.path().join("docs/sprints")).unwrap();
        fs::write(
            temp.path().join(artifact_path),
            "# Closeout [plan-ref: closeout/root]\n\n- [ ] Keep open [ref: closeout/open]\n- [ ] Close me [ref: closeout/done]\n",
        )
        .unwrap();
        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        let initial = execute_plan_sync_command_request_json(
            &plan_sync_request(&repo, artifact_path, Some("closeout/root"), None, false)
                .unwrap()
                .to_string(),
        )
        .unwrap();
        assert_eq!(initial["status"], "ok");
        let plan_id = initial["results"][0]["plan_id"].as_str().unwrap();
        let origin_plan_revision_id = initial["results"][0]["plan_revision_id"]
            .as_str()
            .unwrap()
            .to_string();
        let closeout = close_task_plan_checklist_item(
            &repo,
            &json!({
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id.clone(),
                "plan_item_ref": "closeout/done",
            }),
            None,
        )
        .unwrap();
        assert_eq!(closeout["status"], "synced");
        assert_eq!(closeout["reason"], "checklist_closed");
        assert_eq!(closeout["retention"]["status"], "unchanged");
        assert_eq!(closeout["retention"]["retention_limit"], 20);
        let markdown = fs::read_to_string(temp.path().join(artifact_path)).unwrap();
        assert!(markdown.contains("- [ ] Keep open [ref: closeout/open]"));
        assert!(markdown.contains("- [x] Close me [ref: closeout/done]"));

        let resynced = close_task_plan_checklist_item(
            &repo,
            &json!({
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id,
                "plan_item_ref": "closeout/done",
            }),
            None,
        )
        .unwrap();
        assert_eq!(resynced["reason"], "already_done_resynced");
    }

    #[test]
    fn local_closeout_refuses_unsynced_markdown_drift() {
        let temp = tempdir().unwrap();
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("demo".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        let artifact_path = "docs/sprints/drift.md";
        fs::create_dir_all(temp.path().join("docs/sprints")).unwrap();
        let original = "# Drift [plan-ref: drift/root]\n\n- [ ] Close me [ref: drift/done]\n";
        fs::write(temp.path().join(artifact_path), original).unwrap();
        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        let initial = execute_plan_sync_command_request_json(
            &plan_sync_request(&repo, artifact_path, Some("drift/root"), None, false)
                .unwrap()
                .to_string(),
        )
        .unwrap();
        let plan_id = initial["results"][0]["plan_id"].as_str().unwrap();
        let origin_plan_revision_id = initial["results"][0]["plan_revision_id"].as_str().unwrap();
        let drifted = format!("{original}\nUnrelated unsynced note.\n");
        fs::write(temp.path().join(artifact_path), &drifted).unwrap();

        let closeout = close_task_plan_checklist_item(
            &repo,
            &json!({
                "plan_id": plan_id,
                "origin_plan_revision_id": origin_plan_revision_id,
                "plan_item_ref": "drift/done",
            }),
            None,
        )
        .unwrap();

        assert_eq!(closeout["status"], "skipped");
        assert_eq!(closeout["reason"], "artifact_has_unsynced_drift");
        assert_eq!(
            fs::read_to_string(temp.path().join(artifact_path)).unwrap(),
            drifted
        );
    }
}
