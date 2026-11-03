use super::*;

pub(in crate::primitives) fn team_finish_log_enabled(
    repo: &RepoRuntime,
    output: &JsonValue,
    local: bool,
) -> bool {
    !local
        && repo.team_review_enabled()
        && repo.sprint_enabled()
        && output["task"]["plan_id"].as_str().is_some()
        && output["task"]["plan_item_ref"].as_str().is_some()
        && output["apply_status"] == "done"
        && output
            .get("task_status")
            .or_else(|| output.get("task").and_then(|t| t.get("status")))
            .is_some_and(|s| s == "completed")
}

pub(in crate::primitives) fn append_team_finish_log(
    repo: &RepoRuntime,
    output: &JsonValue,
    remote: &str,
) -> Result<JsonValue, String> {
    let task = output.get("task").ok_or("Missing completed Task binding")?;
    let plan_id = required_string_field(task, "plan_id")?;
    let item_ref = required_string_field(task, "plan_item_ref")?;
    let origin_id = required_string_field(task, "origin_plan_revision_id")?;
    let patchset = required_string_field(output, "patchset_id")?;
    let change = patchset
        .rsplit_once("/P-")
        .ok_or("Invalid Patchset identity")?
        .0;
    let (remote_row, repo_name) = remote_context(repo, Some(remote), None)?;
    let mut client = http_closeout_remote(repo, &remote_row)?;
    let reviews = client
        .list_reviews(change, Some(&repo_name), true)
        .map_err(|e| e.to_string())?;
    let review_id = reviewed_candidate(&reviews, &patchset)?;
    let line = log_line(&patchset, &item_ref, &review_id)?;
    let origin = execute_plan_show_command_request_json(
        &plan_show_request(repo, &plan_id, Some(&origin_id), Some(remote))?.to_string(),
    )?;
    let origin = shown_plan_revision(&origin, &plan_id, "origin")?;
    if string_field(origin, "plan_revision_id").as_deref() != Some(&origin_id) {
        return Err("Origin Plan revision mismatch".into());
    }
    let head = execute_plan_show_command_request_json(
        &plan_show_request(repo, &plan_id, None, Some(remote))?.to_string(),
    )?;
    let head = shown_plan_revision(&head, &plan_id, "head")?;
    let path = required_string_field(head, "artifact_path")?;
    let selector = string_field(head, "artifact_selector");
    if !path.starts_with("docs/sprints/")
        || !is_markdown_artifact_path(&path)
        || string_field(origin, "artifact_path").as_deref() != Some(&path)
        || string_field(origin, "artifact_selector") != selector
    {
        return Err("Sprint artifact binding changed or is not a sprint Markdown file".into());
    }
    for revision in [origin, head] {
        let count = revision
            .get("items")
            .and_then(JsonValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        string_field(item, "plan_item_ref").as_deref() == Some(&item_ref)
                    })
                    .count()
            })
            .unwrap_or(0);
        if count != 1 {
            return Err("Sprint ref is missing or ambiguous".into());
        }
    }
    let resolved = resolve_repo_artifact_path(
        repo.authoritative_repo_root().to_string_lossy().as_ref(),
        &path,
        false,
    )
    .map_err(|err| format!("Sprint path: {err:?}"))?;
    let file = required_string_field(&resolved, "resolved_path")?;
    let markdown = fs::read_to_string(&file).map_err(|e| e.to_string())?;
    if close_plan_item_checkbox(&markdown, &item_ref).status
        != PlanChecklistCloseoutStatus::AlreadyDone
    {
        return Err(
            "Bound sprint item is not uniquely completed; resolve checklist closeout first".into(),
        );
    }
    let expected = required_string_field(head, "artifact_blob_id")?;
    let result = append_and_sync(&file, &expected, &line, || {
        let sync = execute_plan_sync_command_request_json(
            &plan_sync_request(repo, &path, selector.as_deref(), Some(remote), false)?.to_string(),
        )?;
        if sync["status"] != "ok" {
            return Err(format!("Plan sync failed: {sync}"));
        }
        Ok(sync)
    })?;
    Ok(
        json!({"status":"synced", "bound_item_completed":true, "artifact_path":path, "review_id":review_id,
        "patchset_id":patchset, "plan_item_ref":item_ref, "sync":result}),
    )
}

fn reviewed_candidate(reviews: &JsonValue, patchset: &str) -> Result<String, String> {
    let rows = reviews
        .get("reviews")
        .and_then(JsonValue::as_array)
        .ok_or("Missing Review evidence")?;
    rows.iter()
        .rev()
        .find_map(|row| {
            (string_field(row, "patchset_id").as_deref() == Some(patchset)
                && row["action"] == "code_review_summary"
                && row["blocking"] != true)
                .then(|| string_field(row, "review_id"))
                .flatten()
        })
        .ok_or_else(|| "No code Review for completed candidate".into())
}

fn log_line(patchset: &str, item_ref: &str, review: &str) -> Result<String, String> {
    if [patchset, item_ref, review]
        .iter()
        .any(|s| s.is_empty() || s.contains(['\n', '\r', '|']))
    {
        return Err("Invalid finish log identity".into());
    }
    Ok(format!(
        "- Reviewed and remote finished: {patchset} | ref: {item_ref} | review: {review}"
    ))
}

// Called under the existing repository workspace lock. Keep a pending append after
// network failure so the same authoritative receipt can resume without duplication.
fn append_and_sync<F>(file: &str, expected: &str, line: &str, sync: F) -> Result<JsonValue, String>
where
    F: FnOnce() -> Result<JsonValue, String>,
{
    let current = fs::read_to_string(file).map_err(|e| e.to_string())?;
    let suffix = format!("\n{line}\n");
    let present = current.lines().filter(|l| *l == line).count();
    if present > 1 {
        return Err("Duplicate finish log requires reconciliation".into());
    }
    let retry_base = current.strip_suffix(&suffix);
    if artifact_blob_id(&current) != expected
        && retry_base.is_none_or(|base| artifact_blob_id(base) != expected)
    {
        return Err("Sprint has unrelated unsynced drift; reconcile before retry".into());
    }
    if present == 0 {
        fs::write(file, format!("{current}{suffix}")).map_err(|e| e.to_string())?;
    }
    sync()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn only_completed_work_with_teams_enabled_is_eligible() {
        let mut repo = super::super::tests::repo(true);
        for key in [
            "workflow_default_scope",
            "task_default_scope",
            "change_default_scope",
        ] {
            repo.config.insert(key.into(), json!("remote"));
        }
        let completed = json!({"apply_status":"done","task":{"status":"completed","plan_id":"PR-1","plan_item_ref":"ship"}});
        assert!(team_finish_log_enabled(&repo, &completed, false));
        assert!(!team_finish_log_enabled(&repo, &completed, true));
        for output in [
            json!({}),
            json!({"apply_status":"pending","task_status":"completed"}),
            json!({"apply_status":"done","task_status":"active"}),
        ] {
            assert!(!team_finish_log_enabled(&repo, &output, false));
        }
        repo.config.insert("agent_teams".into(), json!(false));
        for scope in ["local", "remote"] {
            for key in [
                "workflow_default_scope",
                "task_default_scope",
                "change_default_scope",
            ] {
                repo.config.insert(key.into(), json!(scope));
            }
            assert!(!team_finish_log_enabled(&repo, &completed, false));
        }
    }
    #[test]
    fn retry_preserves_checked_item_and_does_not_duplicate_log() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.md");
        let original = "# Sprint\n- [x] Done [ref: done]\n";
        fs::write(&path, original).unwrap();
        let path = path.to_str().unwrap();
        let line = log_line("RT-1/C-01/P-01", "done", "R-1").unwrap();
        assert!(
            append_and_sync(path, &artifact_blob_id(original), &line, || Err(
                "offline".into()
            ))
            .is_err()
        );
        let pending = fs::read_to_string(path).unwrap();
        append_and_sync(path, &artifact_blob_id(original), &line, || {
            Ok(json!({"status":"ok"}))
        })
        .unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), pending);
        append_and_sync(path, &artifact_blob_id(&pending), &line, || Ok(json!({}))).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), pending);
    }
    #[test]
    fn concurrent_prose_is_not_published_or_overwritten() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("s.md");
        fs::write(&path, "other agent edit").unwrap();
        assert!(append_and_sync(
            path.to_str().unwrap(),
            &artifact_blob_id("old"),
            "log",
            || panic!("must not sync")
        )
        .is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "other agent edit");
    }
    #[test]
    fn evidence_must_match_candidate_and_be_code_review() {
        let reviews = json!({"reviews":[{"patchset_id":"P1","action":"task_approve","review_id":"R1"},{"patchset_id":"P2","action":"code_review_summary","review_id":"R2"}]});
        assert!(reviewed_candidate(&reviews, "P1").is_err());
        assert_eq!(reviewed_candidate(&reviews, "P2").unwrap(), "R2");
        assert!(log_line("P1", "x\ninjected", "R1").is_err());
    }
}
