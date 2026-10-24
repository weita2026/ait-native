use super::*;
pub use ait_core::workflow_expected_identity::WorkflowExpectedIdentity;

pub fn workflow_identity_payload(repo: &RepoRuntime, state: &JsonValue) -> JsonValue {
    json!({
        "repository_index": repo.repository_index().map(|index| index.get()),
        "patchset_id": state["patchset"]["patchset_id"],
        "revision_snapshot_id": state["patchset"]["revision_snapshot_id"],
        "base_snapshot_id": state["patchset"]["base_snapshot_id"],
        "dirty": state["workspace"]["clean"].as_bool().map(|clean| !clean),
        "workspace_matches_patchset": state["workspace"]["workspace_matches_patchset"],
        "base_is_fresh": state["freshness"]["base_is_fresh"],
        "invalidated": state["patchset"]["invalidated"],
        "withdrawn": state["patchset"]["withdrawn"],
    })
}

pub fn validate_workflow_expected_identity(
    repo: &RepoRuntime,
    state: &JsonValue,
    expected: Option<&WorkflowExpectedIdentity>,
) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let repository_index = repo
        .repository_index()
        .ok_or_else(|| "Guarded workflow requires a numeric Repository authority.".to_string())?
        .get();
    expected.validate_patchset(repository_index, &state["patchset"])?;
    validate_guarded_workspace_with_reader(state, || workflow_workspace_status(repo, None, None))
}

fn validate_guarded_workspace_with_reader<F>(
    state: &JsonValue,
    read_workspace: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<JsonValue, String>,
{
    validate_sealed_workspace(state)?;
    if state["workspace"]["clean"].as_bool().is_none()
        && read_workspace()?["clean"].as_bool() != Some(true)
    {
        return Err("EXPECTED_CANDIDATE_MISMATCH: authoritative Patchset cannot bypass a dirty or unverified local workspace.".to_string());
    }
    Ok(())
}

fn validate_sealed_workspace(state: &JsonValue) -> Result<(), String> {
    if state["workspace"]["clean"].as_bool() == Some(false)
        || state["workspace"]["workspace_matches_patchset"].as_bool() == Some(false)
        || state["freshness"]["base_is_fresh"].as_bool() == Some(false)
        || state["freshness"]["recovery_required"].as_bool() == Some(true)
    {
        return Err("EXPECTED_CANDIDATE_MISMATCH: dirty, mismatched or stale sealed candidate requires a new handoff and review.".to_string());
    }
    Ok(())
}

pub(super) fn validate_guarded_action(
    expected: Option<&WorkflowExpectedIdentity>,
    code: &str,
) -> Result<(), String> {
    if expected.is_some() && code.is_empty() {
        return Err(
            "EXPECTED_CANDIDATE_MISMATCH: no authoritative native next action.".to_string(),
        );
    }
    if expected.is_some()
        && matches!(
            code,
            "snapshot_create"
                | "publish_patchset"
                | "refresh_patchset"
                | "prepare_final_snapshot_promotion"
                | "rebase"
                | "workflow_ready"
        )
    {
        return Err(format!("EXPECTED_CANDIDATE_MISMATCH: {code} would replace or prepare the sealed candidate; obtain a new handoff."));
    }
    Ok(())
}

pub(super) fn validate_guarded_ready_ci_action(
    expected: Option<&WorkflowExpectedIdentity>,
    state: &JsonValue,
    code: &str,
) -> Result<(), String> {
    if expected.is_some() && code == "run_patchset_ci" {
        let ci = &state["patchset_ci_status"];
        if ci["latest_job"]["state"].as_str() == Some("failed")
            || ci["blocking_failure_count"].as_u64().unwrap_or_default() > 0
            || (ci["ci_completed_at_s"].as_u64().unwrap_or_default() > 0
                && matches!(
                    ci["tests_status"].as_str(),
                    Some("fail" | "failed" | "hard_fail" | "soft_fail")
                ))
        {
            return Err("GUARDED_CI_FAILED: exact-candidate CI failed; inspect evidence and explicitly request any rerun instead of repeating ready.".to_string());
        }
    }
    Ok(())
}

pub(super) fn refresh_guarded_ready_ci_state(
    repo: &RepoRuntime,
    state: &mut JsonValue,
    expected: Option<&WorkflowExpectedIdentity>,
    remote_name: Option<&str>,
) -> Result<bool, String> {
    let Some(expected) = expected else {
        return Ok(false);
    };
    let ci = patchset_ci_status(repo, &expected.patchset_id, remote_name)?;
    let quoted = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
    let suffix = remote_name
        .map(|name| format!(" --remote {}", quoted(name)))
        .unwrap_or_default();
    let command = format!(
        "ait patchset ci-status {}{suffix}",
        quoted(&expected.patchset_id)
    );
    merge_guarded_ci_state(state, expected, ci, &command)
}

fn merge_guarded_ci_state(
    state: &mut JsonValue,
    expected: &WorkflowExpectedIdentity,
    ci: JsonValue,
    command: &str,
) -> Result<bool, String> {
    let mismatch =
        || "CI_RUN_IDENTITY_MISMATCH: full native CI receipt differs or is incomplete.".to_string();
    if ci["available"].as_bool() != Some(true)
        || ci["repository_index"].as_u64() != Some(u64::from(expected.repository_index))
        || expected.validate_patchset_reference(&ci).is_err()
    {
        return Err(mismatch());
    }
    let run_seq = ci["ci_run_seq"].as_u64().ok_or_else(mismatch)?;
    if run_seq
        < state["patchset_ci_status"]["ci_run_seq"]
            .as_u64()
            .unwrap_or_default()
    {
        return Err(mismatch());
    }
    let job = &ci["latest_job"];
    let mut pending = false;
    if !job.is_null() {
        if run_seq == 0
            || job["repository_index"].as_u64() != Some(u64::from(expected.repository_index))
            || ci["patchset_index"].as_u64().is_none()
            || job["patchset_index"] != ci["patchset_index"]
            || job["patchset_ci"]["ci_run_seq"].as_u64() != Some(run_seq)
            || job["worker_job_index"].as_u64().is_none()
            || job["patchset_ci"]["ci_worker_job_index"] != job["worker_job_index"]
        {
            return Err(mismatch());
        }
        pending = job["retry_pending"].as_bool() == Some(true)
            || matches!(job["state"].as_str(), Some("queued" | "running"));
    } else if run_seq > 0 && ci["ci_completed_at_s"].as_u64().unwrap_or_default() == 0 {
        // Missing job evidence must not authorize another CI enqueue.
        return Err(mismatch());
    }
    if pending {
        state["next_action"] = json!({"code":"waiting_for_ci","command":command,"summary":"Keep the current native CI request and runner.","detail":"Wait for this exact Patchset CI job; do not enqueue another run."});
    } else if job["state"].as_str() == Some("failed")
        || ci["overall_status"].as_str() == Some("fail")
        || ci["blocking_failure_count"].as_u64().unwrap_or_default() > 0
        || matches!(
            ci["tests_status"].as_str(),
            Some("fail" | "failed" | "hard_fail" | "soft_fail")
        )
    {
        return Err(
            "GUARDED_CI_FAILED: current native CI job failed; no automatic rerun.".to_string(),
        );
    }
    let refresh_phase = !pending
        && state["next_action"]["code"].as_str() == Some("run_patchset_ci")
        && ci["overall_status"].as_str() == Some("pass")
        && ci["tests_status"].as_str() == Some("pass")
        && ci["has_runnable_evidence"].as_bool() == Some(true);
    state["patchset_ci_status"] = ci;
    Ok(refresh_phase)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guarded_full_ci_keeps_pending_runner_and_rejects_mixed_run_identity() {
        let expected = WorkflowExpectedIdentity::from_value(&json!({"repository_index":7,"patchset_id":"RT-1/P-01","revision_snapshot_id":"R","base_snapshot_id":"B"})).unwrap();
        let base = json!({"patchset_ci_status":{"ci_run_seq":1,"latest_job":null,"overall_status":"pass"},"next_action":{"code":"done"}});
        let ci = json!({"available":true,"repository_index":7,"patchset_id":"RT-1/C-01/P-01","task_id":"RT-1","change_id":"C-01","change_ref":"RT-1/C-01","patchset_index":3,"ci_run_seq":2,"ci_completed_at_s":10,"overall_status":"pass","tests_status":"pass","latest_job":{"repository_index":7,"patchset_index":3,"state":"running","worker_job_index":8,"patchset_ci":{"ci_run_seq":2,"ci_worker_job_index":8}}});
        let mut state = base.clone();
        merge_guarded_ci_state(
            &mut state,
            &expected,
            ci.clone(),
            "ait patchset ci-status RT-1/P-01 --remote review",
        )
        .unwrap();
        assert_eq!(state["next_action"]["code"], "waiting_for_ci");
        assert!(state["next_action"]["command"]
            .as_str()
            .unwrap()
            .contains("ci-status"));
        for field in [
            "repository_index",
            "patchset_index",
            "ci_run_seq",
            "ci_worker_job_index",
        ] {
            let mut mixed = ci.clone();
            if field == "ci_run_seq" || field == "ci_worker_job_index" {
                mixed["latest_job"]["patchset_ci"][field] = json!(99);
            } else {
                mixed["latest_job"][field] = json!(99);
            }
            assert!(
                merge_guarded_ci_state(&mut base.clone(), &expected, mixed, "status").is_err(),
                "{field}"
            );
        }
        let mut foreign = ci.clone();
        foreign["task_id"] = json!("RT-2");
        assert!(merge_guarded_ci_state(&mut base.clone(), &expected, foreign, "status").is_err());
        let mut missing_owner = ci.clone();
        missing_owner.as_object_mut().unwrap().remove("task_id");
        assert!(
            merge_guarded_ci_state(&mut base.clone(), &expected, missing_owner, "status").is_err()
        );
        let mut missing = ci.clone();
        missing["latest_job"] = JsonValue::Null;
        missing["ci_completed_at_s"] = json!(0);
        assert!(merge_guarded_ci_state(&mut base.clone(), &expected, missing, "status").is_err());
        let mut pass = ci.clone();
        pass["latest_job"]["state"] = json!("succeeded");
        let mut passed = base.clone();
        merge_guarded_ci_state(&mut passed, &expected, pass, "status").unwrap();
        assert_eq!(passed["next_action"]["code"], "done");
        let mut fail = ci;
        fail["latest_job"]["state"] = json!("failed");
        assert!(
            merge_guarded_ci_state(&mut base.clone(), &expected, fail, "status")
                .unwrap_err()
                .contains("GUARDED_CI_FAILED")
        );
    }
    #[test]
    fn guarded_skipped_workspace_reads_real_status_and_never_reuses_dirty_handoff() {
        let skipped = json!({"workspace":{"clean":null,"evaluation":"skipped","reason":"ready_patchset_is_authoritative"}});
        assert!(
            validate_guarded_workspace_with_reader(&skipped, || Ok(json!({"clean":false})))
                .is_err()
        );
        validate_guarded_workspace_with_reader(&skipped, || Ok(json!({"clean":true}))).unwrap();
        validate_guarded_workspace_with_reader(&json!({"workspace":{"clean":true}}), || {
            panic!("already evaluated workspace need not be read again")
        })
        .unwrap();
    }
    #[test]
    fn sealed_candidate_checks_workspace_and_blocks_candidate_replacement() {
        for state in [
            json!({"workspace":{"clean":false}}),
            json!({"workspace":{"workspace_matches_patchset":false}}),
            json!({"freshness":{"base_is_fresh":false}}),
        ] {
            assert!(validate_sealed_workspace(&state).is_err());
        }
        let expected = WorkflowExpectedIdentity::from_value(&json!({"repository_index":0,"patchset_id":"RT-1/P-01","revision_snapshot_id":"R","base_snapshot_id":"B"})).unwrap();
        assert!(validate_guarded_action(Some(&expected), "refresh_patchset").is_err());
        validate_guarded_action(Some(&expected), "record_attestation").unwrap();
        validate_guarded_action(None, "refresh_patchset").unwrap();
        assert!(validate_guarded_action(Some(&expected), "").is_err());
        let failed = json!({"patchset_ci_status":{"ci_completed_at_s":10,"tests_status":"fail"}});
        assert!(
            validate_guarded_ready_ci_action(Some(&expected), &failed, "run_patchset_ci").is_err()
        );
        validate_guarded_ready_ci_action(None, &failed, "run_patchset_ci").unwrap();
        validate_guarded_ready_ci_action(Some(&expected), &failed, "waiting_for_ci").unwrap();
    }
}
