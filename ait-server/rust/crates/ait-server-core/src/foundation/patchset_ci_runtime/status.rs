use super::*;

const CI_EVIDENCE_MAX_LIST_ITEMS: usize = 32;
const CI_EVIDENCE_MAX_TEXT_CHARS: usize = 4096;

fn compact_ci_evidence(value: &JsonValue, remaining_depth: usize) -> JsonValue {
    match value {
        JsonValue::String(text) => {
            JsonValue::String(truncate_ci_evidence_text(text, CI_EVIDENCE_MAX_TEXT_CHARS))
        }
        JsonValue::Array(values) if values.len() > CI_EVIDENCE_MAX_LIST_ITEMS => json!({
            "item_count": values.len(),
            "detail_omitted": true,
        }),
        JsonValue::Array(values) if remaining_depth > 0 => JsonValue::Array(
            values
                .iter()
                .map(|value| compact_ci_evidence(value, remaining_depth - 1))
                .collect(),
        ),
        JsonValue::Object(values) if remaining_depth > 0 => {
            let mut out = JsonMap::new();
            for (key, value) in values {
                out.insert(key.clone(), compact_ci_evidence(value, remaining_depth - 1));
            }
            JsonValue::Object(out)
        }
        JsonValue::Array(values) => json!({
            "item_count": values.len(),
            "detail_omitted": true,
        }),
        JsonValue::Object(values) => json!({
            "field_count": values.len(),
            "detail_omitted": true,
        }),
        _ => value.clone(),
    }
}

fn truncate_ci_evidence_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

fn compact_snapshot_materialization_evidence(value: &JsonValue) -> JsonValue {
    let mut compact = compact_ci_evidence(value, 5);
    if let Some(object) = compact.as_object_mut() {
        object.insert(
            "detail_policy".to_string(),
            json!("bounded_runtime_evidence"),
        );
    }
    compact
}

pub(super) fn build_patchset_ci_detail(
    config: &PatchsetCiRuntimeConfig,
    all_patchset_suites: &[PatchsetSuiteManifest],
    suite_results: &[JsonValue],
    native_prewarm: Option<JsonValue>,
    suite_pool: Option<&JsonValue>,
    tests_status: &str,
) -> JsonValue {
    let blocking_failures = suite_results
        .iter()
        .filter(|suite| {
            suite.get("blocking").and_then(JsonValue::as_bool) == Some(true)
                && suite.get("status").and_then(JsonValue::as_str) != Some("pass")
        })
        .filter_map(|suite| {
            suite
                .get("suite_id")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    let mut detail = json!({
        "trigger": config.trigger,
        "execution_profile": config.execution_profile,
        "patchset_id": config.patchset_id,
        "change_id": config.change_id,
        "base_snapshot_id": config.base_snapshot_id,
        "revision_snapshot_id": config.revision_snapshot_id,
        "selected_suite_ids": suite_results.iter().filter_map(|suite| suite.get("suite_id").and_then(JsonValue::as_str)).collect::<Vec<_>>(),
        "blocking_suite_ids": suite_results.iter().filter(|suite| suite.get("blocking").and_then(JsonValue::as_bool) == Some(true)).filter_map(|suite| suite.get("suite_id").and_then(JsonValue::as_str)).collect::<Vec<_>>(),
        "all_patchset_suite_ids": all_patchset_suites.iter().map(|suite| suite.suite_id.trim()).collect::<Vec<_>>(),
        "blocking_failures": blocking_failures,
        "tests_status": tests_status,
        "suite_results": suite_results,
        "native_prewarm": native_prewarm,
        "scheduler": patchset_ci_scheduler_evidence(config),
        "server_ci_gate": {
            "component": "ait-server-core",
            "python_server_ci_executor": false,
            "python_foreground": false,
            "legacy_runner_foreground": false,
            "rust_patchset_ci_runtime": true,
        }
    });
    if config.flow.is_tg1_patchset_ci() {
        detail["flow"] = tg1_flow_evidence(config);
    }
    if let Some(materialization) = &config.snapshot_materialization_result {
        detail["snapshot_materialization"] =
            compact_snapshot_materialization_evidence(materialization);
    }
    if let Some(admission) = &config.scheduler_admission {
        detail["scheduler_admission"] = admission.clone();
    }
    if let Some(suite_pool) = suite_pool {
        detail["suite_pool"] = suite_pool.clone();
    }
    detail
}

pub(super) fn patchset_ci_scheduler_evidence(config: &PatchsetCiRuntimeConfig) -> JsonValue {
    json!({
        "authority": "server_scheduler",
        "admitted_cpu_tokens": config.suite_pool_tokens.max(1),
        "runner_parallelism": config.suite_pool_tokens.max(1),
        "runner_parallelism_source": "scheduler_admitted_cpu_tokens",
        "scheduler_admission": config.scheduler_admission.clone().unwrap_or(JsonValue::Null),
    })
}

pub(super) fn attach_flow_finish_evidence(
    config: &PatchsetCiRuntimeConfig,
    detail: &mut JsonValue,
    completed_suite_count: usize,
) {
    if !config.flow.is_tg1_patchset_ci() {
        return;
    }
    let selected_suite_count = detail["selected_suite_ids"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(completed_suite_count);
    detail["flow_finish"] = json!({
        "contract": "ait.server.patchset_ci.finish.v1",
        "policy": if config.flow.finish_after_all_suites { "aggregate_after_all_suites" } else { "legacy_inline" },
        "finish_policy": "once_per_run",
        "finish_report_count": 1,
        "completed_suite_count": completed_suite_count,
        "selected_suite_count": selected_suite_count,
        "all_selected_suites_completed": completed_suite_count == selected_suite_count,
    });
}

pub(super) fn tg1_flow_evidence(config: &PatchsetCiRuntimeConfig) -> JsonValue {
    json!({
        "contract": config.flow.contract.as_str(),
        "kind": config.flow.kind.as_str(),
        "suite_selection": {
            "plane": "patchset",
            "include_modes": &config.flow.include_modes,
        },
        "prewarm": {
            "policy": if config.main_seed_prewarm.is_some() { "once_per_main_seed_generation" } else { "once_per_run" },
            "required": config.flow.prewarm_required,
            "main_seed_prewarm": config.main_seed_prewarm.is_some(),
        },
        "parallelism": {
            "policy": if config.flow.require_exact_cpu_tokens { "fixed" } else { "bounded" },
            "cpu_tokens": config.flow.fixed_cpu_tokens.unwrap_or(config.suite_pool_tokens),
            "actual_suite_pool_tokens": config.suite_pool_tokens,
            "source": "server_scheduler",
        },
        "runner_authority": {
            "rust_only": config.flow.rust_runner_only,
            "python_command_bundle_allowed": false,
        },
        "cargo": {
            "shared_target_required": config.flow.shared_cargo_target_required,
            "shared_cargo_target_dir": config.shared_cargo_target_dir.as_ref().map(|path| path_string(path)),
            "shared_cargo_build_dir": config.shared_cargo_build_dir.as_ref().map(|path| path_string(path)),
        },
        "finish": {
            "policy": if config.flow.finish_after_all_suites { "aggregate_after_all_suites" } else { "legacy_inline" },
            "finish_policy": "once_per_run",
        }
    })
}

pub(super) fn build_patchset_ci_completion(
    config: &PatchsetCiRuntimeConfig,
    selected_suite_count: usize,
    patchset_ci_detail: &JsonValue,
    tests_status: &str,
) -> JsonValue {
    let suite_results = patchset_ci_detail
        .get("suite_results")
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let blocking_failure_count = patchset_ci_detail
        .get("blocking_failures")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let mut lint_status = "none";
    for result in suite_results {
        let suite_id = result
            .get("suite_id")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        if !matches!(suite_id, "cargo_fmt" | "rustfmt") {
            continue;
        }
        lint_status = match result.get("status").and_then(JsonValue::as_str) {
            Some("pass" | "passed" | "success" | "succeeded") if lint_status == "none" => "pass",
            Some("pass" | "passed" | "success" | "succeeded") => lint_status,
            Some("fail" | "failed" | "failure") if lint_status != "error" => "fail",
            Some("fail" | "failed" | "failure") => lint_status,
            Some(_) | None => "error",
        };
    }
    let compact_tests_status = match tests_status {
        "pass" | "passed" | "success" | "succeeded" => "pass",
        "fail" | "failed" | "failure" => "fail",
        "none" | "pending" | "queued" | "running" => "none",
        _ => "error",
    };
    let overall_status = match compact_tests_status {
        "pass" => "pass",
        "fail" => "fail",
        _ => "error",
    };
    json!({
        "patchset_id": config.patchset_id,
        "ci_run_seq": config.ci_run_seq,
        "selected_suite_count": selected_suite_count,
        "suite_result_count": suite_results.len(),
        "blocking_failure_count": blocking_failure_count,
        "overall_status": overall_status,
        "tests_status": compact_tests_status,
        "lint_status": lint_status,
    })
}

pub(super) fn build_result(
    config: &PatchsetCiRuntimeConfig,
    detail: JsonValue,
    suite_results: Vec<JsonValue>,
    native_prewarm: Option<JsonValue>,
    policy_job_payload: Option<JsonValue>,
) -> JsonValue {
    let blocking_failures = detail["blocking_failures"].clone();
    let suite_pool = detail.get("suite_pool").cloned().unwrap_or(JsonValue::Null);
    json!({
        "contract": "ait.server.patchset_ci.run.v1",
        "patchset_id": config.patchset_id,
        "change_id": config.change_id,
        "repo_name": config.repo_name,
        "trigger": config.trigger,
        "execution_profile": config.execution_profile,
        "admitted_cpu_tokens": config.suite_pool_tokens.max(1),
        "runner_parallelism": config.suite_pool_tokens.max(1),
        "scheduler_admission": config.scheduler_admission.clone().unwrap_or(JsonValue::Null),
        "tests_status": detail["tests_status"].clone(),
        "blocking_suite_ids": detail["blocking_suite_ids"].clone(),
        "all_patchset_suite_ids": detail["all_patchset_suite_ids"].clone(),
        "blocking_failures": blocking_failures,
        "suite_results": suite_results,
        "native_prewarm": native_prewarm,
        "suite_pool": suite_pool,
        "patchset_ci_detail": detail,
        "policy_job_payload": policy_job_payload,
        "server_ci_gate": {
            "component": "ait-server-core",
            "python_server_ci_executor": false,
            "rust_patchset_ci_runtime": true
        }
    })
}

pub(super) fn attach_workflow_ready_evidence(
    config: &PatchsetCiRuntimeConfig,
    detail: &mut JsonValue,
    tests_status: &mut String,
) -> Result<(), String> {
    if config.execution_profile != PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND {
        return Ok(());
    }
    match validate_workflow_ready_evidence(config, detail) {
        Ok(evidence) => {
            detail["workflow_ready_evidence"] = evidence;
        }
        Err(error) => {
            detail["workflow_ready_evidence_error"] = json!(error);
            if tests_status == "pass" {
                *tests_status = "fail".to_string();
                detail["tests_status"] = json!("fail");
                let mut blocking_failures = detail["blocking_failures"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if !blocking_failures
                    .iter()
                    .any(|value| value.as_str() == Some("workflow_ready_evidence"))
                {
                    blocking_failures.push(json!("workflow_ready_evidence"));
                }
                detail["blocking_failures"] = JsonValue::Array(blocking_failures);
            }
        }
    }
    Ok(())
}

pub(super) fn policy_job_payload(config: &PatchsetCiRuntimeConfig) -> JsonValue {
    let mut payload = JsonMap::new();
    payload.insert("patchset_id".to_string(), json!(config.patchset_id));
    payload.insert("repo_name".to_string(), json!(config.repo_name));
    if let Some(repo_id) = &config.repo_id {
        payload.insert("repo_id".to_string(), json!(repo_id));
    }
    payload.insert("change_id".to_string(), json!(config.change_id));
    if let Some(change_seq) = &config.change_seq {
        payload.insert("change_seq".to_string(), change_seq.clone());
    }
    if let Some(patchset_number) = &config.patchset_number {
        payload.insert("patchset_number".to_string(), patchset_number.clone());
    }
    JsonValue::Object(payload)
}

pub(super) fn validate_workflow_ready_evidence(
    config: &PatchsetCiRuntimeConfig,
    detail: &JsonValue,
) -> Result<JsonValue, String> {
    let mut evidence = JsonMap::new();
    for suite in detail["suite_results"].as_array().into_iter().flatten() {
        if let Some(suite_id) = suite.get("suite_id").and_then(JsonValue::as_str) {
            evidence.insert(suite_id.to_string(), suite.clone());
        }
    }
    workflow_ready_server_evidence_from_manifest_values(
        &config.suite_values,
        &JsonValue::Object(evidence),
    )
}

pub(super) fn suite_value<'a>(
    config: &'a PatchsetCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
) -> Option<&'a JsonValue> {
    config.suite_values.iter().find(|value| {
        value
            .get("suite_id")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            == Some(suite.suite_id.trim())
    })
}

pub(super) fn suite_is_blocking(suite: &PatchsetSuiteManifest) -> bool {
    suite.default_blocking || suite.suite_id.trim() == TG1_REQUIRED_SUITE_ID
}

pub(super) fn rust_runner_kind(kind: &str) -> &'static str {
    match kind {
        "server_tg1_required" => "rust_server_tg1_required",
        "test_discovery_sharded" => "rust_test_discovery_sharded",
        _ => "rust_server_ci",
    }
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn snapshot_materialization_evidence_bounds_repeated_link_lists() {
        let immutable_links = (0..836)
            .map(|index| {
                json!({
                    "relative_path": format!("src/file-{index}.rs"),
                    "source": format!("/seed/src/file-{index}.rs"),
                    "destination": format!("/shard/src/file-{index}.rs"),
                    "linked": true,
                    "immutable": true,
                })
            })
            .collect::<Vec<_>>();
        let shards = (0..9)
            .map(|index| {
                json!({
                    "shard_id": format!("shard-{index}"),
                    "repo_dir": format!("/ram/shard-{index}/repo"),
                    "materialization": {"immutable_links": immutable_links},
                })
            })
            .collect::<Vec<_>>();
        let compact = compact_snapshot_materialization_evidence(&json!({
            "thread_pool_shards": {"shard_count": 9, "shards": shards},
        }));
        let encoded = serde_json::to_vec(&compact).unwrap();

        assert_eq!(
            compact
                .pointer("/thread_pool_shards/shards/0/materialization/immutable_links/item_count"),
            Some(&json!(836))
        );
        assert!(
            encoded.len() < 64 * 1024,
            "bounded materialization evidence was {} bytes",
            encoded.len()
        );
    }
}
