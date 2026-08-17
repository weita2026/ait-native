use crate::repository_retirement::{
    restore_repository, retire_repository, RepoRestoreRequest, RepoRetireRequest,
};
use crate::runtime::{RemoteRow, RepoRuntime};
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use ait_core::repository_pack_json::repository_payload_with_pack_storage_default;
use ait_core::server_operational::{
    validate_worker_job_list_limit, WorkerJobIndex, WorkerJobKey, WORKER_JOB_LIST_LIMIT_DEFAULT,
    WORKER_JOB_LIST_LIMIT_MAX, WORKER_JOB_LIST_LIMIT_MIN,
};

#[derive(Clone, Debug)]
pub struct RepoCommandRequest {
    pub command: String,
    pub remote_name: Option<String>,
    pub json_output: bool,
    pub args: JsonMap<String, JsonValue>,
}

pub fn repo_command(repo: &RepoRuntime, request: &RepoCommandRequest) -> Result<JsonValue, String> {
    let command = normalize_required_text(&request.command, "repo command")?;
    validate_repo_command_request(&command, request)?;
    if command == "retire" {
        return retire_repository(
            repo,
            &RepoRetireRequest {
                remote_name: request.remote_name.clone(),
                abort: bool_value(request.args.get("abort"), "abort", false)?,
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
        "ci-capabilities" => repo_ci_capabilities(&mut client, &remote_row, &repo_name),
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
    reject_unknown_fields(
        "repo command payload",
        object,
        &["command", "remote_name", "json_output", "args"],
    )?;
    let args = match object.get("args") {
        None | Some(JsonValue::Null) => JsonMap::new(),
        Some(JsonValue::Object(args)) => args.clone(),
        Some(_) => return Err("repo command payload `args` must be an object.".to_string()),
    };
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
    match command {
        "show" => repo_show_text(payload, request),
        "ci-capabilities" => repo_ci_capabilities_text(payload, request),
        "jobs" => repo_jobs_text("ait repo jobs", payload, request),
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
        output.push("decision: Patchset CI submission and zstd remote sync are ready".to_string());
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
        if requested_limit < u64::from(WORKER_JOB_LIST_LIMIT_MAX) {
            output.push(format!(
                "older: {}",
                repo_jobs_json_command_at_limit(
                    title,
                    request,
                    requested_limit
                        .saturating_mul(2)
                        .max(requested_limit + 1)
                        .min(u64::from(WORKER_JOB_LIST_LIMIT_MAX)),
                )
            ));
        } else {
            output.push(format!(
                "older: server query maximum {} reached; narrow the inventory with --state",
                WORKER_JOB_LIST_LIMIT_MAX
            ));
        }
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
        .clamp(
            u64::from(WORKER_JOB_LIST_LIMIT_MIN),
            u64::from(WORKER_JOB_LIST_LIMIT_MAX),
        )
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
    let limit = u32_arg(request, "limit", WORKER_JOB_LIST_LIMIT_DEFAULT)?;
    validate_worker_job_list_limit(limit)?;
    client
        .list_worker_jobs(
            repository_index,
            worker_job_state_kind(optional_string_arg(request, "state")?.as_deref())?,
            limit,
        )
        .map_err(|err| err.to_string())
}

fn worker_job_state_kind(state: Option<&str>) -> Result<Option<u8>, String> {
    let Some(state) = normalize_optional_text(state) else {
        return Ok(None);
    };
    match state.as_str() {
        "queued" => Ok(Some(1)),
        "running" => Ok(Some(2)),
        "succeeded" => Ok(Some(3)),
        "failed" => Ok(Some(4)),
        _ => Err(format!(
            "Worker Job state must be queued, running, succeeded, or failed; received `{state}`."
        )),
    }
}

fn validate_repo_command_request(
    command: &str,
    request: &RepoCommandRequest,
) -> Result<(), String> {
    let allowed_args = match command {
        "show" | "restore" | "ci-capabilities" => &[][..],
        "retire" => &["abort"][..],
        "jobs" => &["worker_job_index", "state", "limit"][..],
        _ => return Err(format!("Unknown repo command `{command}`.")),
    };
    reject_unknown_fields("repo command args", &request.args, allowed_args)?;

    match command {
        "retire" => {
            bool_value(request.args.get("abort"), "abort", false)?;
        }
        "jobs" => {
            let worker_job_index = optional_u32_arg(request, "worker_job_index")?;
            let state = optional_string_arg(request, "state")?;
            let limit = optional_u32_arg(request, "limit")?;
            if worker_job_index.is_some() && (state.is_some() || limit.is_some()) {
                return Err(
                    "repo jobs `worker_job_index` cannot be combined with `state` or `limit`."
                        .to_string(),
                );
            }
            if worker_job_index.is_none() {
                worker_job_state_kind(state.as_deref())?;
                validate_worker_job_list_limit(limit.unwrap_or(WORKER_JOB_LIST_LIMIT_DEFAULT))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn reject_unknown_fields(
    context: &str,
    object: &JsonMap<String, JsonValue>,
    allowed: &[&str],
) -> Result<(), String> {
    let mut unsupported = object
        .keys()
        .filter(|field| !allowed.contains(&field.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    unsupported.sort();
    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{context} contains retired or unknown field(s): {}.",
            unsupported.join(", ")
        ))
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

fn optional_string_arg(request: &RepoCommandRequest, key: &str) -> Result<Option<String>, String> {
    optional_string_value(request.args.get(key), key)
}

fn bool_value(value: Option<&JsonValue>, key: &str, default: bool) -> Result<bool, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(default),
        Some(JsonValue::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("repo command arg `{key}` must be a boolean.")),
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
        for storage_encoding in ["1", "2", "3", "4"] {
            assert!(worker_job_state_kind(Some(storage_encoding)).is_err());
        }
        assert!(worker_job_state_kind(Some("canceled")).is_err());
    }

    #[test]
    fn repo_payload_rejects_retired_unknown_and_ambiguous_args_before_mutation() {
        let temp = tempfile::tempdir().expect("temporary repo");
        let ait_dir = temp.path().join(".ait");
        std::fs::create_dir(&ait_dir).expect("create .ait");
        let config_path = ait_dir.join("config.json");
        std::fs::write(&config_path, "{\"repo_name\":\"fixture\"}\n").expect("write config");
        let config_before = std::fs::read(&config_path).expect("read config before validation");
        let repo = RepoRuntime::discover_from_path(temp.path()).expect("discover repo");

        let rejected = [
            json!({
                "command": "retire",
                "args": {"replace_export": true}
            }),
            json!({
                "command": "retire",
                "args": {"abort": false},
                "retired_field": true
            }),
            json!({
                "command": "jobs",
                "args": {"worker_job_index": 7, "state": "failed"}
            }),
            json!({
                "command": "jobs",
                "args": {"worker_job_index": 7, "limit": 50}
            }),
            json!({
                "command": "jobs",
                "args": {"state": "4"}
            }),
            json!({
                "command": "jobs",
                "args": {"limit": 0}
            }),
            json!({
                "command": "jobs",
                "args": {"limit": 1001}
            }),
        ];
        for payload in rejected {
            let error = repo_command_from_payload(&repo, &payload)
                .expect_err("invalid payload must fail before remote selection");
            assert!(
                error.contains("retired or unknown")
                    || error.contains("cannot be combined")
                    || error.contains("state must be")
                    || error.contains("list limit must be"),
                "{error}"
            );
        }

        assert_eq!(
            std::fs::read(&config_path).expect("read config after validation"),
            config_before
        );
        assert!(!ait_dir.join("remote").exists());
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
                "supported_async_job_types": ["patchset.ci"]
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
        assert!(
            rendered.contains("decision: Patchset CI submission and zstd remote sync are ready")
        );

        let mut optional_manifest_missing = ready.clone();
        optional_manifest_missing["ci_capabilities"]["remote_sync_capabilities"]
            ["zstd_pull_manifest"] = json!(false);
        let rendered = repo_text("ci-capabilities", &optional_manifest_missing, None);
        assert!(rendered.contains("pull manifest unavailable (optional)"));
        assert!(
            rendered.contains("decision: Patchset CI submission and zstd remote sync are ready")
        );

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
    fn repo_jobs_text_never_recommends_an_out_of_range_limit() {
        let request = RepoCommandRequest {
            command: "jobs".to_string(),
            remote_name: None,
            json_output: false,
            args: JsonMap::from_iter([
                ("state".to_string(), JsonValue::Null),
                ("limit".to_string(), json!(WORKER_JOB_LIST_LIMIT_MAX)),
            ]),
        };
        let rendered = repo_text(
            "jobs",
            &json!({
                "count": WORKER_JOB_LIST_LIMIT_MAX,
                "jobs": [{
                    "worker_job_index": 9,
                    "job_type": "patchset.ci",
                    "state": "succeeded"
                }]
            }),
            Some(&request),
        );
        assert!(
            rendered.contains("server query maximum 1000 reached"),
            "{rendered}"
        );
        assert!(!rendered.contains("--limit 1001"), "{rendered}");
        assert!(!rendered.contains("--limit 2000"), "{rendered}");
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
                    "job_type": "patchset.ci",
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
}
