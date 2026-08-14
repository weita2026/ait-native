use super::*;

fn workflow_ready_apply_output(
    mut output: JsonMap<String, JsonValue>,
    final_snapshot_promotion: Option<&JsonValue>,
) -> JsonValue {
    if let Some(promotion) = final_snapshot_promotion {
        output.insert(
            "mode".to_string(),
            JsonValue::String("solo_local_history_promotion".to_string()),
        );
        output.insert("final_snapshot_promotion".to_string(), promotion.clone());
        for key in [
            "local_task_id",
            "local_change_id",
            "remote_task_id",
            "remote_change_id",
            "base_snapshot_id",
            "revision_snapshot_id",
            "aggregate_snapshot_count",
            "history_entry_count",
            "plan_artifact_paths",
        ] {
            if let Some(value) = promotion.get(key) {
                output.insert(key.to_string(), value.clone());
            }
        }
    }
    JsonValue::Object(output)
}

#[expect(
    clippy::too_many_arguments,
    reason = "ready orchestration keeps remote, policy, and progress ports explicit"
)]
pub fn workflow_ready_apply<F>(
    repo: &RepoRuntime,
    change_id: &str,
    snapshot_message: Option<&str>,
    summary: Option<&str>,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: Option<&str>,
    model: Option<&str>,
    remote_name: Option<&str>,
    mut progress: Option<F>,
) -> Result<JsonValue, String>
where
    F: FnMut(&JsonValue) -> Result<(), String>,
{
    let _apply_range = perfetto_range!("ait.workflow_ready.apply");
    let mut final_snapshot_promotion = None;
    let mut ready_patchset_is_authoritative = false;
    let mut effective_change_id = change_id.to_string();
    let mut applied_actions = Vec::new();
    let mut mutation_receipts = Vec::new();
    let mut seen_signatures = BTreeSet::new();
    let mut attempted_pending_waits = BTreeSet::new();
    let helper_started = Instant::now();
    workflow_progress_emit(
        &mut progress,
        "probing",
        "authoritative_state",
        Some(change_id),
        None,
        None,
        Some("Reading authoritative workflow state before applying helper mutations."),
        Some("authoritative_read"),
        None,
        None,
        None,
    )?;
    if let Some(candidate) =
        workflow_final_snapshot_promotion_candidate(repo, change_id, remote_name)?
    {
        ready_patchset_is_authoritative = true;
        let remote_change_id = workflow_final_snapshot_promotion_remote_change_id(&candidate)?;
        let local_change_published = candidate
            .get("state")
            .and_then(|state| state.get("change"))
            .and_then(|change| change.get("publication_state"))
            .and_then(JsonValue::as_str)
            == Some("published");
        let remote_is_prepared = local_change_published
            && workflow_ready_remote_payload_with_patchset_authority(
                repo,
                &remote_change_id,
                remote_name,
                true,
            )
            .ok()
            .is_some_and(|state| {
                workflow_nested_text(&state, "patchset", "patchset_id").is_some()
                    || workflow_nested_text(&state, "change", "status").as_deref() == Some("landed")
            });
        effective_change_id = remote_change_id;
        if remote_is_prepared {
            final_snapshot_promotion = Some(candidate);
        } else {
            workflow_progress_emit(
                &mut progress,
                "starting",
                "prepare_final_snapshot_promotion",
                Some(change_id),
                None,
                Some(1),
                Some("Publishing the consecutive local workflow history and one aggregate Patchset from the remote target head to the final local snapshot."),
                Some("mutation_started"),
                None,
                None,
                None,
            )?;
            let prepared = workflow_prepare_final_snapshot_promotion(
                repo,
                change_id,
                summary,
                author_mode,
                remote_name,
            )?;
            effective_change_id = required_string_field(&prepared, "remote_change_id")?;
            workflow_progress_emit(
                &mut progress,
                "completed",
                "prepare_final_snapshot_promotion",
                Some(&effective_change_id),
                string_field(&prepared, "patchset_id").as_deref(),
                Some(1),
                None,
                Some("mutation_accepted"),
                None,
                None,
                Some("published local history and selected its aggregate Patchset"),
            )?;
            final_snapshot_promotion = Some(prepared);
        }
    }
    loop {
        let mut state = {
            let _range = perfetto_range!("ait.workflow_ready.authoritative_state");
            workflow_ready_remote_payload_with_patchset_authority(
                repo,
                &effective_change_id,
                remote_name,
                ready_patchset_is_authoritative,
            )?
        };
        let mut code = workflow_nested_text(&state, "next_action", "code").unwrap_or_default();
        if code == "waiting_for_ci" && !attempted_pending_waits.contains("waiting_for_ci") {
            attempted_pending_waits.insert("waiting_for_ci".to_string());
            let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
            let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
            let pending_state = state.clone();
            state = {
                let _range = perfetto_range!("ait.workflow_ready.wait_for_ci");
                workflow_wait_for_pending_state(repo, &state, "waiting_for_ci", || {
                    workflow_ready_ci_poll_payload_with_closeout_remote(
                        repo,
                        &mut closeout_remote,
                        &repo_name,
                        &pending_state,
                        &effective_change_id,
                        remote_name,
                    )
                })?
            };
            code = workflow_nested_text(&state, "next_action", "code").unwrap_or_default();
        }
        let (current_change_id, current_patchset_id) = workflow_current_ids(&state);
        if code.is_empty() || code == "done" {
            let detail = if applied_actions.is_empty() {
                "Authoritative state already satisfies `workflow ready --apply`; no new mutation was needed."
            } else {
                "Workflow ready apply completed."
            };
            if applied_actions.is_empty() {
                workflow_progress_emit(
                    &mut progress,
                    "resumed",
                    "done",
                    current_change_id.as_deref(),
                    current_patchset_id.as_deref(),
                    None,
                    Some(detail),
                    Some("authoritative_resume"),
                    None,
                    None,
                    None,
                )?;
            }
            workflow_maybe_record_ready_wait_hint_sample(
                repo,
                &state,
                &applied_actions,
                helper_started.elapsed().as_secs_f64(),
            )?;
            let mut output = state.as_object().cloned().unwrap_or_default();
            output.insert(
                "applied_actions".to_string(),
                JsonValue::Array(applied_actions),
            );
            output.insert(
                "mutation_receipts".to_string(),
                JsonValue::Array(mutation_receipts),
            );
            output.insert(
                "apply_status".to_string(),
                JsonValue::String("done".to_string()),
            );
            output.insert(
                "apply_phase".to_string(),
                workflow_apply_phase_payload_json(
                    if output
                        .get("applied_actions")
                        .and_then(JsonValue::as_array)
                        .is_some_and(|rows| rows.is_empty())
                    {
                        "authoritative_resume"
                    } else {
                        "done"
                    },
                    "done",
                    Some(detail),
                    output
                        .get("applied_actions")
                        .and_then(JsonValue::as_array)
                        .is_some_and(|rows| rows.is_empty()),
                ),
            );
            return Ok(workflow_ready_apply_output(
                output,
                final_snapshot_promotion.as_ref(),
            ));
        }
        if code == "waiting_for_ci" {
            let detail = workflow_nested_text(&state, "next_action", "detail")
                .or_else(|| workflow_nested_text(&state, "next_action", "summary"))
                .unwrap_or_else(|| "Patchset CI is still pending.".to_string());
            let resumed = applied_actions.is_empty();
            workflow_progress_emit(
                &mut progress,
                if resumed { "resumed" } else { "waiting" },
                "waiting_for_ci",
                current_change_id.as_deref(),
                current_patchset_id.as_deref(),
                None,
                Some(&detail),
                Some(if resumed {
                    "authoritative_resume"
                } else {
                    "pending_gate"
                }),
                None,
                None,
                None,
            )?;
            let mut output = state.as_object().cloned().unwrap_or_default();
            output.insert(
                "applied_actions".to_string(),
                JsonValue::Array(applied_actions),
            );
            output.insert(
                "mutation_receipts".to_string(),
                JsonValue::Array(mutation_receipts),
            );
            output.insert(
                "apply_status".to_string(),
                JsonValue::String("waiting_for_ci".to_string()),
            );
            output.insert(
                "apply_stopped_reason".to_string(),
                JsonValue::String(detail.clone()),
            );
            output.insert(
                "apply_phase".to_string(),
                workflow_apply_phase_payload_json(
                    if resumed {
                        "authoritative_resume"
                    } else {
                        "pending_gate"
                    },
                    "waiting_for_ci",
                    Some(&detail),
                    resumed,
                ),
            );
            return Ok(workflow_ready_apply_output(
                output,
                final_snapshot_promotion.as_ref(),
            ));
        }
        let signature = format!(
            "{}|{}|{}|{}",
            code,
            workflow_nested_text(&state, "patchset", "patchset_id").unwrap_or_default(),
            workflow_nested_text(&state, "change", "status").unwrap_or_default(),
            workflow_nested_text(&state, "attestation", "attestation_id").unwrap_or_default(),
        );
        if seen_signatures.contains(&signature) {
            let stopped_reason =
                format!("Workflow ready apply made no further progress at `{code}`.");
            workflow_progress_emit(
                &mut progress,
                "stopped",
                &code,
                current_change_id.as_deref(),
                current_patchset_id.as_deref(),
                Some(applied_actions.len() + 1),
                None,
                Some("stopped"),
                Some(&stopped_reason),
                None,
                None,
            )?;
            let mut output = state.as_object().cloned().unwrap_or_default();
            output.insert(
                "applied_actions".to_string(),
                JsonValue::Array(applied_actions),
            );
            output.insert(
                "mutation_receipts".to_string(),
                JsonValue::Array(mutation_receipts),
            );
            output.insert(
                "apply_status".to_string(),
                JsonValue::String("stopped".to_string()),
            );
            output.insert(
                "apply_stopped_reason".to_string(),
                JsonValue::String(stopped_reason.clone()),
            );
            output.insert(
                "apply_phase".to_string(),
                workflow_apply_phase_payload_json("stopped", &code, Some(&stopped_reason), false),
            );
            return Ok(workflow_ready_apply_output(
                output,
                final_snapshot_promotion.as_ref(),
            ));
        }
        seen_signatures.insert(signature);
        workflow_progress_emit(
            &mut progress,
            "starting",
            &code,
            current_change_id.as_deref(),
            current_patchset_id.as_deref(),
            Some(applied_actions.len() + 1),
            None,
            Some("mutation_started"),
            None,
            None,
            None,
        )?;
        let action = {
            let _range = perfetto_range!("ait.workflow_ready.action");
            workflow_ready_apply_action(
                repo,
                &code,
                &state,
                &effective_change_id,
                snapshot_message,
                summary,
                tests,
                lint,
                security,
                license,
                author_mode,
                model,
                remote_name,
            )?
        };
        if let Some(stopped_reason) = workflow_root_text(&action, "stopped_reason") {
            workflow_progress_emit(
                &mut progress,
                "stopped",
                &code,
                current_change_id.as_deref(),
                current_patchset_id.as_deref(),
                Some(applied_actions.len() + 1),
                None,
                Some("stopped"),
                Some(&stopped_reason),
                None,
                None,
            )?;
            let mut output = state.as_object().cloned().unwrap_or_default();
            output.insert(
                "applied_actions".to_string(),
                JsonValue::Array(applied_actions),
            );
            output.insert(
                "mutation_receipts".to_string(),
                JsonValue::Array(mutation_receipts),
            );
            output.insert(
                "apply_status".to_string(),
                JsonValue::String("stopped".to_string()),
            );
            output.insert(
                "apply_stopped_reason".to_string(),
                JsonValue::String(stopped_reason.clone()),
            );
            output.insert(
                "apply_phase".to_string(),
                workflow_apply_phase_payload_json("stopped", &code, Some(&stopped_reason), false),
            );
            return Ok(workflow_ready_apply_output(
                output,
                final_snapshot_promotion.as_ref(),
            ));
        }
        let result = action
            .get("result")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let receipts = workflow_remote_action_mutation_receipts(&code, &result)
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        mutation_receipts.extend(receipts);
        let summary = workflow_applied_action_summary(&json!({"code": code, "result": result}))
            .unwrap_or_else(|_| format!("completed `{code}`"));
        workflow_progress_emit(
            &mut progress,
            "completed",
            &code,
            current_change_id.as_deref(),
            action
                .get("patchset_id")
                .and_then(JsonValue::as_str)
                .or(current_patchset_id.as_deref()),
            Some(applied_actions.len() + 1),
            None,
            Some("mutation_accepted"),
            None,
            None,
            Some(&summary),
        )?;
        applied_actions.push(json!({"code": code, "result": result}));
    }
}
