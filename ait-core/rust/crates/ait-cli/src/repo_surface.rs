use crate::repository_retirement::{
    restore_repository, retire_repository, RepoRestoreRequest, RepoRetireRequest,
};
use crate::runtime::{RemoteRow, RepoRuntime};
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use ait_core::repository_pack_json::repository_payload_with_pack_storage_default;
use ait_core::server_operational::{WorkerJobIndex, WorkerJobKey};

#[derive(Clone, Debug)]
pub struct RepoCommandRequest {
    pub command: String,
    pub remote_name: Option<String>,
    pub json_output: bool,
    pub args: JsonMap<String, JsonValue>,
}

pub fn repo_command(repo: &RepoRuntime, request: &RepoCommandRequest) -> Result<JsonValue, String> {
    let command = normalize_required_text(&request.command, "repo command")?;
    if command == "retire" {
        return retire_repository(
            repo,
            &RepoRetireRequest {
                remote_name: request.remote_name.clone(),
                abort: bool_value(request.args.get("abort"), "abort", false)?,
                replace_export: bool_value(
                    request.args.get("replace_export"),
                    "replace_export",
                    false,
                )?,
            },
        );
    }
    if command == "restore" {
        return restore_repository(
            repo,
            &RepoRestoreRequest {
                remote_name: request.remote_name.clone(),
            },
        );
    }
    let (remote_row, repo_name) = remote_context(repo, request.remote_name.as_deref())?;
    let mut client = PlanHttpClientManager::new(http_config(repo, &remote_row))
        .map_err(|err| err.to_string())?;

    match command.as_str() {
        "show" => client
            .get_repository(&repo_name)
            .and_then(|payload| {
                repo_show_payload_with_pack_storage(payload)
                    .map_err(ait_core::plan_http_client::PlanHttpClientError::Invalid)
            })
            .map_err(|err| err.to_string()),
        "jobs" => repo_jobs(repo, &mut client, request),
        "run-ci" => repo_run_ci(&mut client, &remote_row, &repo_name, request),
        "ci-capabilities" => repo_ci_capabilities(&mut client, &remote_row, &repo_name),
        "ci-runs" => client
            .read_repository_ci_runs(
                &repo_name,
                i64_arg(request, "limit", 20)?,
                optional_string_arg(request, "plane")?.as_deref(),
                optional_string_arg(request, "suite_id")?.as_deref(),
            )
            .map_err(|err| err.to_string()),
        _ => Err(format!("Unknown repo command `{command}`.")),
    }
}

pub fn repo_command_from_payload(
    repo: &RepoRuntime,
    payload: &JsonValue,
) -> Result<JsonValue, String> {
    let object = payload
        .as_object()
        .ok_or_else(|| "repo command payload must decode to an object.".to_string())?;
    let args = object
        .get("args")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_else(JsonMap::new);
    repo_command(
        repo,
        &RepoCommandRequest {
            command: string_field(object, "command")?,
            remote_name: optional_string_field(object, "remote_name")?,
            json_output: bool_value(object.get("json_output"), "json_output", false)?,
            args,
        },
    )
}

const REPO_TEXT_EVIDENCE_LIMIT: usize = 20;

pub fn render_repo_text(command: &str, payload: &JsonValue) {
    println!("{}", repo_text(command, payload, None));
}

pub fn render_repo_command_text(request: &RepoCommandRequest, payload: &JsonValue) {
    println!(
        "{}",
        repo_text(request.command.as_str(), payload, Some(request))
    );
}

fn repo_text(command: &str, payload: &JsonValue, request: Option<&RepoCommandRequest>) -> String {
    if payload.get("surface").and_then(JsonValue::as_str) == Some("ait.test.status") {
        return test_status_text(payload, request);
    }
    match command {
        "show" => repo_show_text(payload, request),
        "ci-capabilities" => repo_ci_capabilities_text(payload, request),
        "jobs" => repo_jobs_text("ait repo jobs", payload, request),
        "ci-runs" => repo_jobs_text("ait repo ci-runs", payload, request),
        "run-ci" => repo_run_ci_text(payload, request),
        _ => {
            let title = match command {
                "retire" => "repo retirement",
                "restore" => "repo restore",
                _ => "repo",
            };
            let mut output = vec![title.to_string()];
            append_scalar_payload(&mut output, "", payload);
            output.join("\n")
        }
    }
}

fn repo_show_text(payload: &JsonValue, request: Option<&RepoCommandRequest>) -> String {
    let repository = payload.get("repository").unwrap_or(&JsonValue::Null);
    let storage = payload.get("pack_storage").unwrap_or(&JsonValue::Null);
    let validation = storage.get("validation").unwrap_or(&JsonValue::Null);
    let capabilities = payload.get("ci_capabilities").unwrap_or(&JsonValue::Null);
    let runner = capabilities
        .get("native_runner")
        .unwrap_or(&JsonValue::Null);
    let remote_sync = capabilities
        .get("remote_sync_capabilities")
        .unwrap_or(&JsonValue::Null);
    let repository_name = value_text(repository.get("repository_name"));
    let repository_index = value_text(repository.get("repository_index"));
    let repository_label = if repository_index.is_empty() {
        repository_name
    } else {
        format!("{repository_name} (#{repository_index})")
    };
    let tombstoned = repository
        .get("tombstoned")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let validation_state = value_text(validation.get("state"));
    let error_count = validation
        .get("error_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let object_packs = storage
        .get("object_pack_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let tree_packs = storage
        .get("tree_pack_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let zstd_verified = storage
        .get("zstd_only_verified")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let sync_checks = ["zstd_pack_bulk", "zstd_pack_bulk_download"];
    let ready_sync = sync_checks
        .iter()
        .filter(|name| remote_sync.get(**name).and_then(JsonValue::as_bool) == Some(true))
        .count();
    let runner_contract = value_text(runner.get("contract"));
    let runner_entrypoint = value_text(runner.get("repository_entrypoint"));
    let mut output = vec!["ait repo show".to_string()];
    push_key_value(&mut output, "repository", repository_label);
    push_key_value(
        &mut output,
        "state",
        if tombstoned { "tombstoned" } else { "active" },
    );
    push_key_value(
        &mut output,
        "storage",
        format!("{validation_state} ({error_count} errors)"),
    );
    push_key_value(
        &mut output,
        "packs",
        format!(
            "{object_packs} object, {tree_packs} tree; zstd-only {}",
            if zstd_verified {
                "verified"
            } else {
                "not verified"
            }
        ),
    );
    if !runner_contract.is_empty() || !runner_entrypoint.is_empty() {
        push_key_value(
            &mut output,
            "runner",
            format_contract_entrypoint(&runner_contract, &runner_entrypoint),
        );
    }
    push_key_value(
        &mut output,
        "remote sync",
        format!(
            "{ready_sync}/{} required ready; pull manifest {}",
            sync_checks.len(),
            if remote_sync
                .get("zstd_pull_manifest")
                .and_then(JsonValue::as_bool)
                == Some(true)
            {
                "available"
            } else {
                "unavailable (optional)"
            }
        ),
    );
    if tombstoned {
        output.push("blocker: repository authority is tombstoned".to_string());
        output.push(format!(
            "recovery: ait repo restore{}",
            request_remote_suffix(request)
        ));
    } else if validation_state != "valid" || error_count > 0 || !zstd_verified {
        output.push("blocker: repository pack storage is not fully valid".to_string());
        output.push(format!(
            "next: ait repo show{} --json",
            request_remote_suffix(request)
        ));
    } else if ready_sync < sync_checks.len() {
        output.push("attention: required zstd remote-sync capability is missing".to_string());
        output.push(format!(
            "next: ait repo ci-capabilities{}",
            request_remote_suffix(request)
        ));
    }
    output.push(format!(
        "details: ait repo show{} --json",
        request_remote_suffix(request)
    ));
    output.join("\n")
}

fn repo_ci_capabilities_text(payload: &JsonValue, request: Option<&RepoCommandRequest>) -> String {
    let handshake = payload.get("handshake").unwrap_or(&JsonValue::Null);
    let capabilities = payload.get("ci_capabilities").unwrap_or(&JsonValue::Null);
    let runner = capabilities
        .get("native_runner")
        .unwrap_or(&JsonValue::Null);
    let remote_sync = capabilities
        .get("remote_sync_capabilities")
        .unwrap_or(&JsonValue::Null);
    let server_ready = handshake
        .get("ready")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let runner_contract = value_text(runner.get("contract"));
    let runner_entrypoint = value_text(runner.get("repository_entrypoint"));
    let async_job_count = handshake
        .get("supported_async_job_types")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let sync_checks = ["zstd_pack_bulk", "zstd_pack_bulk_download"];
    let ready_sync = sync_checks
        .iter()
        .filter(|name| remote_sync.get(**name).and_then(JsonValue::as_bool) == Some(true))
        .count();
    let mut output = vec!["ait repo ci-capabilities".to_string()];
    push_key_value(
        &mut output,
        "repository",
        value_text(payload.get("repo_name")),
    );
    push_key_value(&mut output, "remote", value_text(payload.get("remote")));
    push_key_value(
        &mut output,
        "server",
        format!(
            "{}; backend {}; protocol {}",
            if server_ready { "ready" } else { "not ready" },
            value_text(handshake.get("authority_backend")),
            value_text(handshake.get("contract_version"))
        ),
    );
    push_key_value(
        &mut output,
        "runner",
        if runner_contract.is_empty() {
            "missing".to_string()
        } else {
            format_contract_entrypoint(&runner_contract, &runner_entrypoint)
        },
    );
    push_key_value(
        &mut output,
        "async jobs",
        format!("{async_job_count} supported types"),
    );
    push_key_value(
        &mut output,
        "remote sync",
        format!(
            "{ready_sync}/{} required ready; pull manifest {}",
            sync_checks.len(),
            if remote_sync
                .get("zstd_pull_manifest")
                .and_then(JsonValue::as_bool)
                == Some(true)
            {
                "available"
            } else {
                "unavailable (optional)"
            }
        ),
    );
    if !server_ready || runner_contract.is_empty() || ready_sync < sync_checks.len() {
        output.push("blocker: native CI prerequisites are incomplete".to_string());
        output.push(format!(
            "next: ait repo ci-capabilities{} --json",
            request_remote_suffix(request)
        ));
    } else {
        output.push("decision: native CI submission and zstd remote sync are ready".to_string());
    }
    if server_ready && !runner_contract.is_empty() && ready_sync == sync_checks.len() {
        output.push(format!(
            "details: ait repo ci-capabilities{} --json",
            request_remote_suffix(request)
        ));
    }
    output.join("\n")
}

fn repo_jobs_text(
    title: &str,
    payload: &JsonValue,
    request: Option<&RepoCommandRequest>,
) -> String {
    if let Some(job) = payload.get("job").filter(|value| value.is_object()) {
        return single_job_text(title, job, request);
    }
    if payload.get("worker_job_index").is_some() {
        return single_job_text(title, payload, request);
    }
    let jobs = payload
        .get("jobs")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut states = std::collections::BTreeMap::<String, usize>::new();
    for job in &jobs {
        *states.entry(job_state(job)).or_default() += 1;
    }
    let failed = states.get("failed").copied().unwrap_or(0);
    let running = states.get("running").copied().unwrap_or(0);
    let queued = states.get("queued").copied().unwrap_or(0);
    let retrying = jobs
        .iter()
        .filter(|job| job.get("retry_pending").and_then(JsonValue::as_bool) == Some(true))
        .count();
    let outcome_failures = jobs
        .iter()
        .filter(|job| job_state(job) != "failed" && job_has_failure(job))
        .count();
    let total = payload
        .get("count")
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(jobs.len())
        .max(jobs.len());
    let requested_limit = repo_jobs_request_limit(payload, request);
    let query_bound_reached = total > 0 && total as u64 >= requested_limit;
    let mut output = vec![title.to_string()];
    push_key_value(
        &mut output,
        "state",
        if jobs.is_empty() {
            "no matching jobs returned"
        } else if running + queued + retrying > 0 {
            "active work present"
        } else if failed + outcome_failures > 0 {
            "failures present in returned history"
        } else {
            "all returned jobs succeeded"
        },
    );
    push_key_value(
        &mut output,
        "jobs",
        format_job_state_counts(total, &states, retrying, outcome_failures),
    );
    if let Some(latest) = jobs.iter().max_by_key(|job| job_index(job)) {
        push_key_value(&mut output, "latest", job_summary(latest));
    }
    if query_bound_reached {
        push_key_value(
            &mut output,
            "query bound",
            format!("limit {requested_limit} reached; older records may exist"),
        );
    }
    if jobs.is_empty() {
        output.push("evidence: no matching Worker Jobs returned".to_string());
        output.push(format!(
            "details: {}",
            repo_jobs_json_command(title, payload, request)
        ));
        return output.join("\n");
    }

    let mut ranked = jobs;
    ranked.sort_by(|left, right| {
        job_attention_priority(left)
            .cmp(&job_attention_priority(right))
            .then_with(|| job_index(right).cmp(&job_index(left)))
    });
    let shown = ranked.len().min(REPO_TEXT_EVIDENCE_LIMIT);
    output.push(String::new());
    output.push("evidence (failed/running/queued first, then newest)".to_string());
    output.push("job\ttype\tstate\tresult\tattempts\tfailures\tupdated".to_string());
    for job in ranked.iter().take(shown) {
        output.push(project_job_row(job));
    }
    if shown < ranked.len() {
        output.push(format!("shown: {shown}/{total}"));
        output.push(format!(
            "more: {}",
            repo_jobs_json_command(title, payload, request)
        ));
    } else {
        output.push(format!(
            "details: {}",
            repo_jobs_json_command(title, payload, request)
        ));
    }
    if query_bound_reached {
        output.push(format!(
            "older: {}",
            repo_jobs_json_command_at_limit(
                title,
                request,
                requested_limit.saturating_mul(2).max(requested_limit + 1),
            )
        ));
    }
    output.join("\n")
}

fn single_job_text(title: &str, job: &JsonValue, request: Option<&RepoCommandRequest>) -> String {
    let index = job_index(job);
    let patchset_ci = job.get("patchset_ci").unwrap_or(&JsonValue::Null);
    let mut output = vec![title.to_string()];
    push_key_value(&mut output, "job", format!("#{index}"));
    push_key_value(&mut output, "type", value_text(job.get("job_type")));
    push_key_value(&mut output, "state", job_state(job));
    push_key_value(&mut output, "result", job_result(job));
    push_key_value(&mut output, "attempts", job_attempts(job));
    push_key_value(
        &mut output,
        "patchset",
        value_text(job.get("patchset_index")),
    );
    push_key_value(
        &mut output,
        "snapshot",
        value_text(job.get("snapshot_index")),
    );
    if patchset_ci.is_object() {
        push_key_value(
            &mut output,
            "CI",
            format!(
                "{} blocking failures; lint {}; tests {}",
                value_text(patchset_ci.get("blocking_failure_count")),
                value_text(patchset_ci.get("lint_status")),
                value_text(patchset_ci.get("tests_status"))
            ),
        );
    }
    push_key_value(&mut output, "updated", epoch_text(job.get("updated_at_s")));
    output.push(format!(
        "{}: ait repo jobs --worker-job-index {index}{} --json",
        if job_has_failure(job) || matches!(job_state(job).as_str(), "queued" | "running") {
            "next"
        } else {
            "details"
        },
        request_remote_suffix(request)
    ));
    output.join("\n")
}

fn repo_run_ci_text(payload: &JsonValue, request: Option<&RepoCommandRequest>) -> String {
    let job = payload.get("job").unwrap_or(payload);
    let index = job_index(job);
    let mut output = vec!["ait repo run-ci".to_string()];
    push_key_value(
        &mut output,
        "submission",
        if payload.get("queued").and_then(JsonValue::as_bool) == Some(true) {
            "queued"
        } else {
            "accepted"
        },
    );
    push_key_value(&mut output, "job", format!("#{index}"));
    push_key_value(&mut output, "type", value_text(job.get("job_type")));
    push_key_value(&mut output, "state", job_state(job));
    push_key_value(
        &mut output,
        "snapshot",
        value_text(payload.get("snapshot_id")),
    );
    if index > 0 {
        output.push(format!(
            "next: ait repo jobs --worker-job-index {index}{}",
            request_remote_suffix(request)
        ));
    }
    output.join("\n")
}

fn test_status_text(payload: &JsonValue, request: Option<&RepoCommandRequest>) -> String {
    let suite = value_text(payload.get("suite_id"));
    let plane = value_text(payload.get("plane"));
    let status = value_text(payload.get("status"));
    let limit = payload
        .get("limit")
        .and_then(JsonValue::as_i64)
        .unwrap_or(5)
        .max(1);
    let latest = payload.get("latest").filter(|value| value.is_object());
    let decision_status = latest
        .map(|job| {
            if job_has_failure(job) {
                "failed".to_string()
            } else {
                job_state(job)
            }
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if status.is_empty() {
                "unknown".to_string()
            } else {
                status.clone()
            }
        });
    let mut output = vec!["ait test status".to_string()];
    push_key_value(&mut output, "suite", suite.clone());
    push_key_value(&mut output, "plane", plane.clone());
    push_key_value(&mut output, "status", decision_status);
    output.push(
        "authority: latest repo.ci Worker Job; Binary v0 history does not persist plane/suite selectors"
            .to_string(),
    );
    let returned_count = payload
        .get("runs")
        .and_then(|runs| runs.get("count"))
        .and_then(JsonValue::as_u64)
        .or_else(|| {
            payload
                .get("runs")
                .and_then(|runs| runs.get("jobs"))
                .and_then(JsonValue::as_array)
                .map(|jobs| jobs.len() as u64)
        })
        .unwrap_or(0);
    let query_bound_reached = returned_count > 0 && returned_count >= limit as u64;
    if let Some(latest) = latest {
        let index = job_index(latest);
        push_key_value(&mut output, "latest job", format!("#{index}"));
        push_key_value(&mut output, "result", job_result(latest));
        push_key_value(
            &mut output,
            "updated",
            epoch_text(latest.get("updated_at_s")),
        );
        match job_state(latest).as_str() {
            _ if job_has_failure(latest) => {
                output.push("blocker: latest available repository CI evidence failed".to_string());
                output.push(format!(
                    "next: {}",
                    test_rerun_command(&suite, &plane, request)
                ));
            }
            "queued" | "running" => output.push(format!(
                "next: ait repo jobs --worker-job-index {index}{}",
                request_remote_suffix(request)
            )),
            "succeeded" => output.push(
                "decision: latest available repository CI job passed (suite/plane unverified)"
                    .to_string(),
            ),
            other => output.push(format!(
                "attention: latest repository CI run reports {other}"
            )),
        }
    } else if query_bound_reached {
        let next_limit = (limit as u64).saturating_mul(2).max(limit as u64 + 1);
        output.push(format!(
            "attention: no repo.ci test run was found in the latest {limit} Worker Jobs; older records may exist"
        ));
        output.push(format!(
            "next: {}",
            test_status_command(&suite, &plane, next_limit, request, false)
        ));
    } else {
        output.push(format!(
            "blocker: no repo.ci test run was found in the latest {limit} Worker Jobs"
        ));
        output.push(format!(
            "next: {}",
            test_rerun_command(&suite, &plane, request)
        ));
    }

    let mut runs = payload
        .get("runs")
        .and_then(|runs| runs.get("jobs"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if latest.is_some() {
        runs.retain(|job| job.get("job_type").and_then(JsonValue::as_str) == Some("repo.ci"));
    }
    if !runs.is_empty() {
        let mut ranked = runs;
        ranked.sort_by(|left, right| {
            job_attention_priority(left)
                .cmp(&job_attention_priority(right))
                .then_with(|| job_index(right).cmp(&job_index(left)))
        });
        let shown = ranked.len().min(5);
        output.push(String::new());
        output.push(if latest.is_some() {
            "recent repository CI evidence".to_string()
        } else {
            "returned CI evidence (no matching repo.ci run)".to_string()
        });
        output.push("job\ttype\tstate\tresult\tattempts\tfailures\tupdated".to_string());
        for job in ranked.iter().take(shown) {
            output.push(project_job_row(job));
        }
        if shown < ranked.len() {
            output.push(format!("shown: {shown}/{}", ranked.len()));
        }
    }
    output.push(format!(
        "details: {}",
        test_status_command(&suite, &plane, limit as u64, request, true)
    ));
    output.join("\n")
}

fn test_status_command(
    suite: &str,
    plane: &str,
    limit: u64,
    request: Option<&RepoCommandRequest>,
    json_output: bool,
) -> String {
    format!(
        "ait test status --suite-id {suite} --plane {plane} --limit {limit}{}{}",
        request_remote_suffix(request),
        if json_output { " --json" } else { "" }
    )
}

fn test_rerun_command(suite: &str, plane: &str, request: Option<&RepoCommandRequest>) -> String {
    let command = match suite {
        "full_repo" => format!("ait test run --full --plane {plane} --target-line main"),
        "full_repo_zstd_only" => {
            format!("ait test run --full --variant zstd_only --plane {plane} --target-line main")
        }
        _ => format!("ait repo run-ci --suite {suite} --plane {plane} --target-line main"),
    };
    format!("{command}{}", request_remote_suffix(request))
}

fn project_job_row(job: &JsonValue) -> String {
    format!(
        "#{}\t{}\t{}\t{}\t{}\t{}\t{}",
        job_index(job),
        value_text(job.get("job_type")),
        job_state(job),
        job_result(job),
        job_attempts(job),
        job.get("patchset_ci")
            .and_then(|ci| ci.get("blocking_failure_count"))
            .map(|value| value_text(Some(value)))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "0".to_string()),
        epoch_text(job.get("updated_at_s")),
    )
}

fn job_summary(job: &JsonValue) -> String {
    let mut summary = format!(
        "#{} {} — {} / {}",
        job_index(job),
        value_text(job.get("job_type")),
        job_state(job),
        job_result(job)
    );
    let updated = epoch_text(job.get("updated_at_s"));
    if !updated.is_empty() {
        summary.push_str(&format!(" @ {updated}"));
    }
    summary
}

fn format_job_state_counts(
    total: usize,
    states: &std::collections::BTreeMap<String, usize>,
    retrying: usize,
    outcome_failures: usize,
) -> String {
    let mut parts = ["failed", "running", "queued", "succeeded"]
        .iter()
        .filter_map(|state| {
            states
                .get(*state)
                .copied()
                .filter(|count| *count > 0)
                .map(|count| format!("{count} {state}"))
        })
        .collect::<Vec<_>>();
    if retrying > 0 {
        parts.push(format!("{retrying} retry pending"));
    }
    if outcome_failures > 0 {
        parts.push(format!(
            "{outcome_failures} failed CI outcome{}",
            if outcome_failures == 1 { "" } else { "s" }
        ));
    }
    if parts.is_empty() {
        format!("{total} returned")
    } else {
        format!("{total} returned ({})", parts.join(", "))
    }
}

fn job_attention_priority(job: &JsonValue) -> u8 {
    let state = job_state(job);
    if job_has_failure(job) {
        0
    } else if state == "running" {
        1
    } else if state == "queued"
        || job.get("retry_pending").and_then(JsonValue::as_bool) == Some(true)
    {
        2
    } else {
        3
    }
}

fn job_has_failure(job: &JsonValue) -> bool {
    let state = job_state(job);
    let ci_status = value_text(
        job.get("patchset_ci")
            .and_then(|ci| ci.get("overall_status")),
    );
    let blocking_failures = job
        .get("patchset_ci")
        .and_then(|ci| ci.get("blocking_failure_count"))
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let error_kind = job
        .get("error_kind")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    state == "failed"
        || matches!(ci_status.as_str(), "fail" | "failed" | "error")
        || blocking_failures > 0
        || error_kind > 0
}

fn job_state(job: &JsonValue) -> String {
    let state = value_text(job.get("state"));
    if state.is_empty() {
        value_text(job.get("diagnostic_status"))
    } else {
        state
    }
}

fn job_result(job: &JsonValue) -> String {
    let state = job_state(job);
    let ci_status = value_text(
        job.get("patchset_ci")
            .and_then(|ci| ci.get("overall_status")),
    );
    if !ci_status.is_empty() && ci_status != "none" {
        if state == "failed" && !matches!(ci_status.as_str(), "fail" | "failed" | "error") {
            return format!("failed (CI {ci_status})");
        }
        if state == "succeeded" && matches!(ci_status.as_str(), "fail" | "failed" | "error") {
            return format!("CI {ci_status}");
        }
        return ci_status;
    }
    for value in [
        job.get("overall_status"),
        job.get("diagnostic_status"),
        job.get("outcome"),
    ] {
        let rendered = value_text(value);
        if !rendered.is_empty() {
            return rendered;
        }
    }
    job_state(job)
}

fn job_attempts(job: &JsonValue) -> String {
    let attempts = value_text(job.get("attempt_count"));
    let max_attempts = value_text(job.get("max_attempts"));
    match (attempts.is_empty(), max_attempts.is_empty()) {
        (false, false) => format!("{attempts}/{max_attempts}"),
        (false, true) => attempts,
        _ => String::new(),
    }
}

fn job_index(job: &JsonValue) -> u64 {
    job.get("worker_job_index")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0)
}

fn repo_jobs_json_command(
    title: &str,
    payload: &JsonValue,
    request: Option<&RepoCommandRequest>,
) -> String {
    repo_jobs_json_command_at_limit(title, request, repo_jobs_request_limit(payload, request))
}

fn repo_jobs_request_limit(payload: &JsonValue, request: Option<&RepoCommandRequest>) -> u64 {
    request
        .and_then(|request| request.args.get("limit"))
        .and_then(JsonValue::as_u64)
        .or_else(|| payload.get("count").and_then(JsonValue::as_u64))
        .unwrap_or(REPO_TEXT_EVIDENCE_LIMIT as u64)
        .max(1)
}

fn repo_jobs_json_command_at_limit(
    title: &str,
    request: Option<&RepoCommandRequest>,
    requested_limit: u64,
) -> String {
    let mut command = title.to_string();
    if let Some(request) = request {
        if let Some(state) = request.args.get("state").and_then(JsonValue::as_str) {
            if !state.trim().is_empty() {
                command.push_str(&format!(" --state {}", state.trim()));
            }
        }
        if let Some(plane) = request.args.get("plane").and_then(JsonValue::as_str) {
            if !plane.trim().is_empty() {
                command.push_str(&format!(" --plane {}", plane.trim()));
            }
        }
        if let Some(suite) = request.args.get("suite_id").and_then(JsonValue::as_str) {
            if !suite.trim().is_empty() {
                command.push_str(&format!(" --suite-id {}", suite.trim()));
            }
        }
    }
    command.push_str(&format!(" --limit {requested_limit}"));
    command.push_str(&request_remote_suffix(request));
    command.push_str(" --json");
    command
}

fn request_remote_suffix(request: Option<&RepoCommandRequest>) -> String {
    request
        .and_then(|request| request.remote_name.as_deref())
        .map(str::trim)
        .filter(|remote| !remote.is_empty())
        .map(|remote| format!(" --remote {remote}"))
        .unwrap_or_default()
}

fn format_contract_entrypoint(contract: &str, entrypoint: &str) -> String {
    match (contract.is_empty(), entrypoint.is_empty()) {
        (false, false) => format!("{contract} via {entrypoint}"),
        (false, true) => contract.to_string(),
        (true, false) => entrypoint.to_string(),
        (true, true) => String::new(),
    }
}

fn push_key_value(output: &mut Vec<String>, key: &str, value: impl Into<String>) {
    let value = value.into();
    if !value.trim().is_empty() {
        output.push(format!("{key}: {value}"));
    }
}

fn value_text(value: Option<&JsonValue>) -> String {
    match value {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::Bool(value)) => value.to_string(),
        Some(JsonValue::Number(value)) => value.to_string(),
        Some(JsonValue::String(value)) => value.clone(),
        Some(JsonValue::Array(values)) => values
            .iter()
            .map(|value| value_text(Some(value)))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Some(JsonValue::Object(_)) => String::new(),
    }
}

fn epoch_text(value: Option<&JsonValue>) -> String {
    value
        .and_then(JsonValue::as_i64)
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0))
        .map(|value| value.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

fn append_scalar_payload(output: &mut Vec<String>, prefix: &str, payload: &JsonValue) {
    match payload {
        JsonValue::Object(object) => {
            for (key, value) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                append_scalar_payload(output, &path, value);
            }
        }
        JsonValue::Array(values) if values.iter().all(|value| !value.is_object()) => {
            push_key_value(output, prefix, value_text(Some(payload)));
        }
        JsonValue::Array(values) => {
            push_key_value(output, prefix, format!("{} records", values.len()));
            for (index, value) in values.iter().enumerate() {
                append_scalar_payload(output, &format!("{prefix}[{index}]"), value);
            }
        }
        _ => push_key_value(output, prefix, value_text(Some(payload))),
    }
}

fn repo_show_payload_with_pack_storage(payload: JsonValue) -> Result<JsonValue, String> {
    repository_payload_with_pack_storage_default(payload)
}

fn repo_jobs(
    repo: &RepoRuntime,
    client: &mut PlanHttpClientManager,
    request: &RepoCommandRequest,
) -> Result<JsonValue, String> {
    let repository_index = repo.require_repository_index()?;
    if let Some(worker_job_index) = optional_u32_arg(request, "worker_job_index")? {
        return client
            .get_worker_job(WorkerJobKey::new(
                repository_index,
                WorkerJobIndex::new(worker_job_index),
            ))
            .map_err(|err| err.to_string());
    }
    client
        .list_worker_jobs(
            repository_index,
            worker_job_state_kind(optional_string_arg(request, "state")?.as_deref())?,
            u32_arg(request, "limit", 50)?,
        )
        .map_err(|err| err.to_string())
}

fn worker_job_state_kind(state: Option<&str>) -> Result<Option<u8>, String> {
    let Some(state) = normalize_optional_text(state) else {
        return Ok(None);
    };
    match state.as_str() {
        "queued" | "1" => Ok(Some(1)),
        "running" | "2" => Ok(Some(2)),
        "succeeded" | "3" => Ok(Some(3)),
        "failed" | "4" => Ok(Some(4)),
        _ => Err(format!(
            "Worker Job state must be queued, running, succeeded, failed, or its canonical 1..4 value; received `{state}`."
        )),
    }
}

fn repo_run_ci(
    client: &mut PlanHttpClientManager,
    remote_row: &RemoteRow,
    repo_name: &str,
    request: &RepoCommandRequest,
) -> Result<JsonValue, String> {
    let result = client
        .run_repo_ci(
            repo_name,
            &string_list_arg(request, "suite_ids")?,
            optional_string_arg(request, "plane")?.as_deref(),
            &string_arg(request, "target_line", "main")?,
            &string_arg(request, "trigger", "manual_rerun")?,
            optional_string_arg(request, "selector")?.as_deref(),
            &string_list_arg(request, "task_ids")?,
            optional_string_arg(request, "curated_corpus")?.as_deref(),
            optional_i64_arg(request, "count")?,
            optional_i64_arg(request, "window_days")?,
            &string_list_arg(request, "dependency_evidence")?,
            &string_list_arg(request, "compliance_evidence")?,
        )
        .map_err(|err| err.to_string());
    match result {
        Ok(payload) => Ok(payload),
        Err(message) if matches!(remote_error_status_code(&message), Some(404 | 405)) => {
            let cli_hint = format!(
                "ait repo ci-capabilities --remote {}",
                remote_row.name.as_str()
            );
            Err(ci_route_mismatch_guidance(
                client,
                &remote_row.url,
                "repo_run_ci_route",
                &cli_hint,
                &message,
            ))
        }
        Err(message) => Err(message),
    }
}

fn repo_ci_capabilities(
    client: &mut PlanHttpClientManager,
    remote_row: &RemoteRow,
    repo_name: &str,
) -> Result<JsonValue, String> {
    let handshake = client
        .get_server_handshake()
        .map_err(|err| err.to_string())?;
    let ci_capabilities = handshake
        .get("ci_capabilities")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(JsonValue::Null);
    Ok(json!({
        "repo_name": repo_name,
        "remote": remote_row.name.clone(),
        "handshake": handshake,
        "ci_capabilities": ci_capabilities,
    }))
}

fn remote_context(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
) -> Result<(RemoteRow, String), String> {
    let remote_row = repo.remote_row(remote_name)?;
    let repo_name = normalize_optional_text(remote_row.repo_name.as_deref())
        .unwrap_or_else(|| repo.repo_name());
    Ok((remote_row, repo_name))
}

fn http_config(repo: &RepoRuntime, remote_row: &RemoteRow) -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: remote_row.url.clone(),
        repository_index: repo.repository_index(),
        headers: repo.auth_headers(),
        ..PlanHttpClientConfig::default()
    }
}

fn ci_route_mismatch_guidance(
    client: &mut PlanHttpClientManager,
    base_url: &str,
    route_label: &str,
    cli_hint: &str,
    original_error: &str,
) -> String {
    let Some(status_code) = remote_error_status_code(original_error) else {
        return original_error.to_string();
    };
    let capability_hint = match client.get_server_health() {
        Err(_) => {
            "Could not read /healthz from the live runtime, so treat this as a stale or partially updated ait-server process and restart/update it before retrying.".to_string()
        }
        Ok(healthz) => {
            let capabilities = healthz
                .get("ci_capabilities")
                .and_then(JsonValue::as_object);
            let readiness = healthz.get("ci_readiness").and_then(JsonValue::as_object);
            let runtime_root = healthz
                .get("runtime_root")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let mut hint = if capabilities.is_none() {
                "The live runtime /healthz payload does not advertise ci_capabilities, so this ait-server process likely predates the native CI routes. Restart/update the live runtime, then retry.".to_string()
            } else {
                let route_supported = capabilities
                    .and_then(|value| value.get(route_label))
                    .and_then(JsonValue::as_bool);
                let generation = readiness
                    .and_then(|value| value.get("runtime_generation"))
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .trim();
                let mut route_hint = if route_supported == Some(false) {
                    format!("/healthz reports ci_capabilities.{route_label}=false, so the running ait-server process does not support this CI route yet. Restart/update the live runtime, then retry.")
                } else {
                    format!("/healthz advertises ci_capabilities for {route_label}, but the live runtime still returned {status_code}. Treat this as a stale or partially updated server process and restart/update it before retrying.")
                };
                if !generation.is_empty() {
                    route_hint.push_str(&format!(" runtime_generation={generation}."));
                }
                route_hint
            };
            if !runtime_root.is_empty() {
                hint.push_str(&format!(" runtime_root={runtime_root}."));
            }
            hint
        }
    };
    format!(
        "Live runtime rejected the {route_label} CI route with HTTP {status_code}. {capability_hint} Verify support with `{cli_hint}`. Original error: {original_error} base_url={base_url}"
    )
}

fn remote_error_status_code(message: &str) -> Option<i64> {
    message
        .split(" failed: ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse::<i64>()
        .ok()
}

fn i64_arg(request: &RepoCommandRequest, key: &str, default: i64) -> Result<i64, String> {
    i64_value(request.args.get(key), key, default)
}

fn optional_i64_arg(request: &RepoCommandRequest, key: &str) -> Result<Option<i64>, String> {
    optional_i64_value(request.args.get(key), key)
}

fn u32_arg(request: &RepoCommandRequest, key: &str, default: u32) -> Result<u32, String> {
    optional_u32_arg(request, key).map(|value| value.unwrap_or(default))
}

fn optional_u32_arg(request: &RepoCommandRequest, key: &str) -> Result<Option<u32>, String> {
    match request.args.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| format!("repo command arg `{key}` must be an unsigned 32-bit integer.")),
        Some(_) => Err(format!(
            "repo command arg `{key}` must be an unsigned 32-bit integer."
        )),
    }
}

fn string_arg(request: &RepoCommandRequest, key: &str, default: &str) -> Result<String, String> {
    match request.args.get(key) {
        None | Some(JsonValue::Null) => Ok(default.to_string()),
        Some(JsonValue::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("repo command arg `{key}` must be a string.")),
    }
}

fn optional_string_arg(request: &RepoCommandRequest, key: &str) -> Result<Option<String>, String> {
    optional_string_value(request.args.get(key), key)
}

fn string_list_arg(request: &RepoCommandRequest, key: &str) -> Result<Vec<String>, String> {
    match request.args.get(key) {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(values)) => values
            .iter()
            .map(|value| match value {
                JsonValue::String(text) => Ok(text.clone()),
                _ => Err(format!(
                    "repo command arg `{key}` must be a list of strings."
                )),
            })
            .collect(),
        Some(_) => Err(format!(
            "repo command arg `{key}` must be a list of strings."
        )),
    }
}

fn bool_value(value: Option<&JsonValue>, key: &str, default: bool) -> Result<bool, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(default),
        Some(JsonValue::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("repo command arg `{key}` must be a boolean.")),
    }
}

fn i64_value(value: Option<&JsonValue>, key: &str, default: i64) -> Result<i64, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(default),
        Some(JsonValue::Number(value)) => value
            .as_i64()
            .ok_or_else(|| format!("repo command arg `{key}` must be an integer.")),
        Some(_) => Err(format!("repo command arg `{key}` must be an integer.")),
    }
}

fn optional_i64_value(value: Option<&JsonValue>, key: &str) -> Result<Option<i64>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("repo command arg `{key}` must be an integer.")),
        Some(_) => Err(format!("repo command arg `{key}` must be an integer.")),
    }
}

fn string_field(object: &JsonMap<String, JsonValue>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("repo command payload requires `{key}`."))
}

fn optional_string_field(
    object: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<String>, String> {
    optional_string_value(object.get(key), key)
}

fn optional_string_value(value: Option<&JsonValue>, key: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => Ok(normalize_optional_text(Some(value))),
        Some(_) => Err(format!("repo command arg `{key}` must be a string.")),
    }
}

fn normalize_required_text(value: &str, field: &str) -> Result<String, String> {
    normalize_optional_text(Some(value)).ok_or_else(|| format!("{field} is required."))
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_repo_show_exposes_pack_storage_payload() {
        let payload = repo_show_payload_with_pack_storage(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "repo",
                "tombstoned": false
            }
        }))
        .expect("repo show payload should normalize");

        assert_eq!(
            payload["pack_storage"]["contract"],
            json!("ait.repository.pack_storage.v1")
        );
        assert_eq!(payload["pack_storage"]["zstd_only_verified"], json!(true));
        assert_eq!(
            payload["pack_storage"]["object_pack_format"],
            json!("ait-pack-v3-zstd-chunked")
        );
        assert_eq!(
            payload["pack_storage"]["tree_pack_format"],
            json!("ait-tree-pack-v2-zstd-chunked")
        );
    }

    #[test]
    fn worker_job_state_filters_use_the_fixed_binary_v0_values() {
        assert_eq!(worker_job_state_kind(None), Ok(None));
        assert_eq!(worker_job_state_kind(Some("queued")), Ok(Some(1)));
        assert_eq!(worker_job_state_kind(Some("running")), Ok(Some(2)));
        assert_eq!(worker_job_state_kind(Some("succeeded")), Ok(Some(3)));
        assert_eq!(worker_job_state_kind(Some("failed")), Ok(Some(4)));
        assert_eq!(worker_job_state_kind(Some("4")), Ok(Some(4)));
        assert!(worker_job_state_kind(Some("canceled")).is_err());
    }

    #[test]
    fn repo_show_text_projects_decision_fields_without_inline_objects() {
        let payload = json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "repo",
                "tombstoned": false
            },
            "pack_storage": {
                "object_pack_count": 2,
                "tree_pack_count": 3,
                "zstd_only_verified": true,
                "validation": {"state": "valid", "error_count": 0}
            },
            "ci_capabilities": {
                "native_runner": {
                    "contract": "ait.runner.native-job.v3",
                    "repository_entrypoint": "ci/run"
                },
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true,
                    "zstd_pack_bulk_download": true,
                    "zstd_pull_manifest": true
                }
            }
        });

        let rendered = repo_text("show", &payload, None);
        assert!(rendered.contains("repository: repo (#7)"));
        assert!(rendered.contains("storage: valid (0 errors)"));
        assert!(rendered.contains("remote sync: 2/2 required ready; pull manifest available"));
        assert!(rendered.contains("details: ait repo show --json"));
        assert!(!rendered.contains("{\""));
    }

    #[test]
    fn repo_ci_capabilities_text_distinguishes_ready_and_incomplete() {
        let ready = json!({
            "repo_name": "repo",
            "remote": "origin",
            "handshake": {
                "ready": true,
                "authority_backend": "binary_v0",
                "contract_version": "ait.agent_server_protocol.v2",
                "supported_async_job_types": ["repo.ci"]
            },
            "ci_capabilities": {
                "native_runner": {
                    "contract": "ait.runner.native-job.v3",
                    "repository_entrypoint": "ci/run"
                },
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true,
                    "zstd_pack_bulk_download": true,
                    "zstd_pull_manifest": true
                }
            }
        });
        let rendered = repo_text("ci-capabilities", &ready, None);
        assert!(rendered.contains("decision: native CI submission and zstd remote sync are ready"));

        let mut optional_manifest_missing = ready.clone();
        optional_manifest_missing["ci_capabilities"]["remote_sync_capabilities"]
            ["zstd_pull_manifest"] = json!(false);
        let rendered = repo_text("ci-capabilities", &optional_manifest_missing, None);
        assert!(rendered.contains("pull manifest unavailable (optional)"));
        assert!(rendered.contains("decision: native CI submission and zstd remote sync are ready"));

        let mut incomplete = ready;
        incomplete["ci_capabilities"]["remote_sync_capabilities"]["zstd_pack_bulk_download"] =
            json!(false);
        let rendered = repo_text("ci-capabilities", &incomplete, None);
        assert!(rendered.contains("blocker: native CI prerequisites are incomplete"));
        assert!(rendered.contains("next: ait repo ci-capabilities --json"));
    }

    #[test]
    fn repo_jobs_text_prioritizes_failures_and_exposes_truncation() {
        let mut jobs = (1_u64..=21)
            .map(|index| {
                json!({
                    "worker_job_index": index,
                    "job_type": "patchset.ci",
                    "state": "succeeded",
                    "diagnostic_status": "succeeded",
                    "attempt_count": 1,
                    "max_attempts": 3,
                    "updated_at_s": 1_700_000_000 + index,
                    "patchset_ci": {
                        "overall_status": "pass",
                        "blocking_failure_count": 0
                    }
                })
            })
            .collect::<Vec<_>>();
        jobs[0]["state"] = json!("failed");
        jobs[0]["diagnostic_status"] = json!("failed");
        let payload = json!({"count": 21, "jobs": jobs});

        let rendered = repo_text("jobs", &payload, None);
        assert!(rendered.contains("latest: #21 patchset.ci — succeeded / pass"));
        let failed = rendered.find("#1\tpatchset.ci\tfailed").unwrap();
        let newest = rendered.find("#21\tpatchset.ci\tsucceeded").unwrap();
        assert!(failed < newest);
        assert!(rendered.contains("shown: 20/21"));
        assert!(rendered.contains("more: ait repo jobs --limit 21 --json"));
        assert!(!rendered.contains("{\""));
    }

    #[test]
    fn repo_jobs_text_handles_empty_and_single_job_payloads() {
        let empty = repo_text("jobs", &json!({"count": 0, "jobs": []}), None);
        assert!(empty.contains("state: no matching jobs returned"));
        assert!(empty.contains("no matching Worker Jobs returned"));

        let single = repo_text(
            "jobs",
            &json!({
                "contract": "ait.server.worker-job.service.v1",
                "job": {
                    "worker_job_index": 9,
                    "job_type": "repo.ci",
                    "state": "running",
                    "attempt_count": 1,
                    "max_attempts": 3
                }
            }),
            None,
        );
        assert!(single.contains("job: #9"));
        assert!(single.contains("state: running"));
        assert!(single.contains("ait repo jobs --worker-job-index 9 --json"));
        assert!(!single.contains("{\""));
    }

    #[test]
    fn test_status_text_reports_unknown_authority_and_exact_recovery() {
        let payload = json!({
            "surface": "ait.test.status",
            "suite_id": "full_repo",
            "plane": "nightly",
            "limit": 20,
            "status": "unknown",
            "latest": null,
            "runs": {"count": 0, "jobs": []}
        });

        let rendered = repo_text("ci-runs", &payload, None);
        assert!(rendered.contains("blocker: no repo.ci test run was found"));
        assert!(rendered.contains("next: ait test run --full --plane nightly --target-line main"));
        assert!(rendered.contains(
            "details: ait test status --suite-id full_repo --plane nightly --limit 20 --json"
        ));
        assert!(rendered.contains("does not persist plane/suite selectors"));
    }

    #[test]
    fn test_status_text_preserves_remote_and_searches_past_a_bounded_query() {
        let jobs = (1_u64..=20)
            .map(|index| {
                json!({
                    "worker_job_index": index,
                    "job_type": "patchset.ci",
                    "state": "succeeded"
                })
            })
            .collect::<Vec<_>>();
        let payload = json!({
            "surface": "ait.test.status",
            "suite_id": "full_repo",
            "plane": "nightly",
            "limit": 20,
            "status": "unknown",
            "latest": null,
            "runs": {"count": 20, "jobs": jobs}
        });
        let request = RepoCommandRequest {
            command: "ci-runs".to_string(),
            remote_name: Some("origin".to_string()),
            json_output: false,
            args: JsonMap::new(),
        };

        let rendered = repo_text("ci-runs", &payload, Some(&request));
        assert!(rendered.contains("attention: no repo.ci test run was found"));
        assert!(rendered.contains(
            "next: ait test status --suite-id full_repo --plane nightly --limit 40 --remote origin"
        ));
        assert!(!rendered.contains("ait test run --full"));
        assert!(rendered.contains(
            "details: ait test status --suite-id full_repo --plane nightly --limit 20 --remote origin --json"
        ));
    }

    #[test]
    fn test_status_text_treats_failed_ci_outcome_as_a_blocker() {
        let latest = json!({
            "worker_job_index": 8,
            "job_type": "repo.ci",
            "state": "succeeded",
            "patchset_ci": {
                "overall_status": "fail",
                "blocking_failure_count": 1
            }
        });
        let payload = json!({
            "surface": "ait.test.status",
            "suite_id": "full_repo",
            "plane": "nightly",
            "limit": 20,
            "status": "succeeded",
            "latest": latest,
            "runs": {"count": 1, "jobs": [latest]}
        });

        let rendered = repo_text("ci-runs", &payload, None);
        assert!(rendered.contains("status: failed"));
        assert!(rendered.contains("blocker: latest available repository CI evidence failed"));
        assert!(rendered.contains("next: ait test run --full --plane nightly --target-line main"));
        assert!(!rendered.contains("CI job passed"));
    }
}
