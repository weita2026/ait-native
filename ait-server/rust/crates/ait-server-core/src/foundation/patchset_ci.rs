use serde::Deserialize;
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::BTreeSet;

const TG1_REQUIRED_SUITE_ID: &str = "tg1_required";
const TG1_DEFAULT_MINIMUM_COUNT: i64 = 33;
const TG1_REQUIRED_CPU_TOKENS: i64 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PatchsetSuiteManifest {
    pub suite_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub plane: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub default_blocking: bool,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default, rename = "_artifact_path")]
    pub artifact_path: Option<String>,
    #[serde(default)]
    pub runner: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchsetCiAggregationPlan {
    pub stage: String,
    pub suite_ids: Vec<String>,
    pub updates_tests_status: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchsetCiJobPlan {
    pub job_type: String,
    pub suite_id: Option<String>,
    pub suite_ids: Vec<String>,
    pub stage: Option<String>,
    pub workflow_ready_foreground: bool,
    pub updates_tests_status: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchsetCiPlan {
    pub selected_suite_ids: Vec<String>,
    pub blocking_suite_ids: Vec<String>,
    pub informational_suite_ids: Vec<String>,
    pub ready_critical_suite_ids: Vec<String>,
    pub background_suite_ids: Vec<String>,
    pub ready_aggregation: PatchsetCiAggregationPlan,
    pub informational_aggregation: Option<PatchsetCiAggregationPlan>,
    pub workflow_ready_foreground_jobs: Vec<PatchsetCiJobPlan>,
    pub background_jobs: Vec<PatchsetCiJobPlan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatchsetCiDispatchJob {
    pub job_id: String,
    pub job: PatchsetCiJobPlan,
    pub payload: JsonMap<String, JsonValue>,
    pub queued_ordinal: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatchsetCiBlockedJob {
    pub job_id: String,
    pub job: PatchsetCiJobPlan,
    pub payload: JsonMap<String, JsonValue>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatchsetCiDispatchPlan {
    pub plan: PatchsetCiPlan,
    pub scope: String,
    pub queued_jobs: Vec<PatchsetCiDispatchJob>,
    pub blocked_jobs: Vec<PatchsetCiBlockedJob>,
}

pub fn plan_patchset_ci_from_manifest_values(
    manifests: &[JsonValue],
) -> Result<PatchsetCiPlan, String> {
    plan_patchset_ci(&parse_manifest_values(manifests)?)
}

pub fn plan_patchset_ci_dispatch_from_manifest_values(
    manifests: &[JsonValue],
    request: &JsonMap<String, JsonValue>,
) -> Result<PatchsetCiDispatchPlan, String> {
    plan_patchset_ci_dispatch(&parse_manifest_values(manifests)?, request)
}

pub fn workflow_ready_server_evidence_from_manifest_values(
    manifests: &[JsonValue],
    suite_evidence: &JsonValue,
) -> Result<JsonValue, String> {
    workflow_ready_server_evidence(&parse_manifest_values(manifests)?, suite_evidence)
}

fn parse_manifest_values(manifests: &[JsonValue]) -> Result<Vec<PatchsetSuiteManifest>, String> {
    let mut parsed = Vec::with_capacity(manifests.len());
    for manifest in manifests {
        parsed.push(
            serde_json::from_value::<PatchsetSuiteManifest>(manifest.clone())
                .map_err(|err| format!("patchset CI suite manifest is invalid: {err}"))?,
        );
    }
    Ok(parsed)
}

pub fn plan_patchset_ci(manifests: &[PatchsetSuiteManifest]) -> Result<PatchsetCiPlan, String> {
    let mut selected = Vec::new();
    for manifest in manifests {
        if !is_patchset_gate(manifest) {
            continue;
        }
        let suite_id = manifest.suite_id.trim();
        if suite_id.is_empty() {
            return Err("patchset CI suite manifest requires `suite_id`.".to_string());
        }
        let ready_blocking = manifest.default_blocking || suite_id == TG1_REQUIRED_SUITE_ID;
        selected.push((suite_id.to_string(), ready_blocking));
    }
    selected.sort_by(|left, right| left.0.cmp(&right.0));

    let selected_suite_ids = selected
        .iter()
        .map(|(suite_id, _)| suite_id.clone())
        .collect::<Vec<_>>();
    let blocking_suite_ids = selected
        .iter()
        .filter(|(_, default_blocking)| *default_blocking)
        .map(|(suite_id, _)| suite_id.clone())
        .collect::<Vec<_>>();
    let informational_suite_ids = selected
        .iter()
        .filter(|(_, default_blocking)| !*default_blocking)
        .map(|(suite_id, _)| suite_id.clone())
        .collect::<Vec<_>>();

    let ready_aggregation = PatchsetCiAggregationPlan {
        stage: "ready_blocking".to_string(),
        suite_ids: blocking_suite_ids.clone(),
        updates_tests_status: true,
    };
    let informational_aggregation = if informational_suite_ids.is_empty() {
        None
    } else {
        Some(PatchsetCiAggregationPlan {
            stage: "informational".to_string(),
            suite_ids: informational_suite_ids.clone(),
            updates_tests_status: false,
        })
    };
    let workflow_ready_foreground_jobs =
        workflow_ready_foreground_jobs(&blocking_suite_ids, &ready_aggregation);
    let background_jobs =
        background_jobs(&informational_suite_ids, informational_aggregation.as_ref());

    Ok(PatchsetCiPlan {
        selected_suite_ids,
        blocking_suite_ids: blocking_suite_ids.clone(),
        informational_suite_ids: informational_suite_ids.clone(),
        ready_critical_suite_ids: blocking_suite_ids.clone(),
        background_suite_ids: informational_suite_ids.clone(),
        ready_aggregation,
        informational_aggregation,
        workflow_ready_foreground_jobs,
        background_jobs,
    })
}

pub fn plan_patchset_ci_dispatch(
    manifests: &[PatchsetSuiteManifest],
    request: &JsonMap<String, JsonValue>,
) -> Result<PatchsetCiDispatchPlan, String> {
    let plan = plan_patchset_ci(manifests)?;
    let patchset_id = required_text(request, "patchset_id")?;
    let snapshot_id = optional_text(request, "revision_snapshot_id")
        .or_else(|| optional_text(request, "snapshot_id"))
        .unwrap_or_else(|| "unknown-snapshot".to_string());
    let scope = optional_text(request, "scope").unwrap_or_else(|| {
        if optional_text(request, "execution_profile").as_deref() == Some("background") {
            "background".to_string()
        } else {
            "workflow_ready_foreground".to_string()
        }
    });
    let completed_suite_ids = completed_suite_ids(request)?;

    let scoped_jobs = patchset_ci_jobs_for_scope(&plan, &scope)?;
    let mut queued_jobs = Vec::new();
    let mut blocked_jobs = Vec::new();
    for job in scoped_jobs {
        if is_completed_suite_job(job, &completed_suite_ids) {
            continue;
        }
        let payload = patchset_ci_job_payload(job, request)?;
        let job_id = patchset_ci_dispatch_job_id(job, &patchset_id, &snapshot_id);
        let missing_suite_ids = missing_aggregate_suite_results(job, &completed_suite_ids);
        if missing_suite_ids.is_empty() {
            queued_jobs.push(PatchsetCiDispatchJob {
                job_id,
                job: job.clone(),
                payload,
                queued_ordinal: queued_jobs.len(),
            });
        } else {
            blocked_jobs.push(PatchsetCiBlockedJob {
                job_id,
                job: job.clone(),
                payload,
                reason: format!("waits for suite results: {}", missing_suite_ids.join(",")),
            });
        }
    }

    Ok(PatchsetCiDispatchPlan {
        plan,
        scope,
        queued_jobs,
        blocked_jobs,
    })
}

pub fn patchset_ci_job_payload(
    job: &PatchsetCiJobPlan,
    request: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut payload = JsonMap::new();
    payload.insert(
        "patchset_id".to_string(),
        json!(required_text(request, "patchset_id")?),
    );
    copy_optional_payload_field(&mut payload, request, "repo_name");
    copy_optional_payload_field(&mut payload, request, "repo_id");
    copy_optional_payload_field(&mut payload, request, "change_id");
    copy_optional_payload_field(&mut payload, request, "change_seq");
    copy_optional_payload_field(&mut payload, request, "patchset_number");
    copy_optional_payload_field(&mut payload, request, "revision_snapshot_id");
    copy_optional_payload_field(&mut payload, request, "snapshot_id");

    match job.job_type.as_str() {
        "patchset.ci" => {
            copy_optional_payload_field(&mut payload, request, "trigger");
            if let Some(execution_profile) = optional_text(request, "execution_profile") {
                payload.insert("execution_profile".to_string(), json!(execution_profile));
            } else if job.workflow_ready_foreground {
                payload.insert(
                    "execution_profile".to_string(),
                    json!("workflow_ready_foreground"),
                );
            } else {
                payload.insert("execution_profile".to_string(), json!("background"));
            }
            let suite_id = job
                .suite_id
                .as_deref()
                .ok_or_else(|| "patchset.ci dispatch job requires `suite_id`.".to_string())?;
            payload.insert("suite_id".to_string(), json!(suite_id));
        }
        "patchset.ci.aggregate" => {
            payload.insert("suite_ids".to_string(), json!(&job.suite_ids));
            if let Some(stage) = &job.stage {
                payload.insert("stage".to_string(), json!(stage));
            }
        }
        _ => {
            return Err(format!(
                "Unsupported patchset CI dispatch job type `{}`.",
                &job.job_type
            ));
        }
    }

    Ok(payload)
}

pub fn workflow_ready_server_evidence(
    manifests: &[PatchsetSuiteManifest],
    suite_evidence: &JsonValue,
) -> Result<JsonValue, String> {
    let plan = plan_patchset_ci(manifests)?;
    let evidence_by_suite = suite_evidence
        .as_object()
        .ok_or_else(|| "`suite_evidence` must be a JSON object keyed by suite_id.".to_string())?;
    let manifest_by_suite = manifests
        .iter()
        .filter(|manifest| is_patchset_gate(manifest))
        .map(|manifest| (manifest.suite_id.trim().to_string(), manifest))
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut suite_results = Vec::new();
    let mut blocking_failures = Vec::new();

    for suite_id in &plan.ready_critical_suite_ids {
        let manifest = manifest_by_suite
            .get(suite_id)
            .ok_or_else(|| format!("Ready suite `{suite_id}` has no suite manifest."))?;
        let evidence = evidence_by_suite
            .get(suite_id)
            .ok_or_else(|| format!("Ready suite `{suite_id}` has no server CI evidence."))?;
        let result = normalize_server_suite_result(manifest, evidence)?;
        if result.get("status").and_then(JsonValue::as_str) != Some("pass") {
            blocking_failures.push(json!({
                "suite_id": suite_id,
                "status": result.get("status").cloned().unwrap_or_else(|| json!("fail")),
                "reason": result.get("failure_reason").cloned().unwrap_or_else(|| json!("suite did not pass")),
            }));
        }
        suite_results.push(result);
    }

    let tests_status = if blocking_failures.is_empty() {
        "pass"
    } else {
        "fail"
    };
    Ok(json!({
        "execution_profile": "workflow_ready_foreground",
        "tests_status": tests_status,
        "selected_suite_ids": plan.ready_critical_suite_ids,
        "blocking_suite_ids": plan.blocking_suite_ids,
        "all_patchset_suite_ids": plan.selected_suite_ids,
        "blocking_failures": blocking_failures,
        "suite_results": suite_results,
        "server_ci_gate": {
            "component": "ait-server-core",
            "execution_profile": "workflow_ready_foreground",
            "python_server_ci_executor": false,
            "python_foreground": false,
            "legacy_runner_foreground": false,
            "tg1_required": plan.ready_critical_suite_ids.contains(&TG1_REQUIRED_SUITE_ID.to_string()),
        },
    }))
}

fn normalize_server_suite_result(
    manifest: &PatchsetSuiteManifest,
    evidence: &JsonValue,
) -> Result<JsonValue, String> {
    let evidence_obj = evidence.as_object().ok_or_else(|| {
        format!(
            "Server CI evidence for `{}` must be an object.",
            manifest.suite_id.trim()
        )
    })?;
    let runner_kind = required_text(evidence_obj, "runner_kind")?;
    reject_python_runner(&runner_kind)?;
    let status = optional_text(evidence_obj, "status").unwrap_or_else(|| "fail".to_string());
    let mut result = JsonMap::new();
    result.insert("suite_id".to_string(), json!(manifest.suite_id.trim()));
    result.insert(
        "display_name".to_string(),
        optional_json_text(manifest.display_name.as_deref()),
    );
    result.insert(
        "artifact_path".to_string(),
        optional_json_text(manifest.artifact_path.as_deref()),
    );
    result.insert("plane".to_string(), json!(manifest.plane.trim()));
    result.insert("mode".to_string(), json!(manifest.mode.trim()));
    result.insert("blocking".to_string(), json!(true));
    result.insert(
        "purpose".to_string(),
        optional_json_text(manifest.purpose.as_deref()),
    );
    result.insert("runner_kind".to_string(), json!(runner_kind));
    result.insert("status".to_string(), json!(status));
    result.insert(
        "artifacts".to_string(),
        evidence_obj
            .get("artifacts")
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    result.insert(
        "server_ci_gate".to_string(),
        json!({
            "component": "ait-server-core",
            "python_server_ci_executor": false,
            "python_foreground": false,
            "legacy_runner_foreground": false,
        }),
    );
    if let Some(reason) = optional_text(evidence_obj, "failure_reason") {
        result.insert("failure_reason".to_string(), json!(reason));
    }
    if manifest.suite_id.trim() == TG1_REQUIRED_SUITE_ID {
        if manifest_runner_kind(manifest) != "server_tg1_required" {
            return Err(
                "TG-1 workflow-ready evidence must be produced by server_tg1_required.".to_string(),
            );
        }
        let summary = evidence_obj.get("tg1_required_summary").ok_or_else(|| {
            "TG-1 server CI evidence requires `tg1_required_summary`.".to_string()
        })?;
        validate_tg1_required_summary(summary)?;
        result.insert("tg1_required_summary".to_string(), summary.clone());
    }
    Ok(JsonValue::Object(result))
}

fn validate_tg1_required_summary(summary: &JsonValue) -> Result<(), String> {
    let obj = summary
        .as_object()
        .ok_or_else(|| "`tg1_required_summary` must be an object.".to_string())?;
    let status = required_text(obj, "status")?;
    let validation_status =
        optional_text(obj, "validation_status").unwrap_or_else(|| status.clone());
    let live_count = required_i64(obj, "live_count")?;
    let minimum_count = optional_i64(obj, "minimum_count").unwrap_or(TG1_DEFAULT_MINIMUM_COUNT);
    let floor = std::cmp::max(minimum_count, TG1_DEFAULT_MINIMUM_COUNT);
    if status != "pass" || validation_status != "pass" {
        return Err("TG-1 server CI evidence is not passing.".to_string());
    }
    if live_count < floor {
        return Err(format!(
            "TG-1 server CI evidence has {live_count} live case(s); expected at least {floor}."
        ));
    }

    let scheduler = required_json_object(obj, "scheduler")?;
    let authority = required_text(scheduler, "authority")?;
    let thread_pool_owner = required_text(scheduler, "thread_pool_owner")?;
    let requested_cpu_tokens = required_i64(scheduler, "requested_cpu_tokens")?;
    let admitted_cpu_tokens = required_i64(scheduler, "admitted_cpu_tokens")?;
    let runner_parallelism_source = required_text(scheduler, "runner_parallelism_source")?;
    if authority != "server_scheduler" {
        return Err("TG-1 server CI evidence must use server_scheduler authority.".to_string());
    }
    if thread_pool_owner != "server" {
        return Err("TG-1 server CI evidence must use server-owned thread pool.".to_string());
    }
    if requested_cpu_tokens != TG1_REQUIRED_CPU_TOKENS {
        return Err(format!(
            "TG-1 server CI evidence must request exactly {TG1_REQUIRED_CPU_TOKENS} CPU tokens."
        ));
    }
    if !(1..=requested_cpu_tokens).contains(&admitted_cpu_tokens) {
        return Err(format!(
            "TG-1 server CI evidence admitted CPU tokens must be between 1 and the requested {requested_cpu_tokens}; got {admitted_cpu_tokens}."
        ));
    }
    if runner_parallelism_source != "scheduler_admitted_cpu_tokens" {
        return Err(
            "TG-1 server CI evidence must source parallelism from scheduler_admitted_cpu_tokens."
                .to_string(),
        );
    }

    let thread_pool_shards = required_json_object(obj, "thread_pool_shards")?;
    let shard_count = required_i64(thread_pool_shards, "shard_count")?;
    if shard_count != admitted_cpu_tokens {
        return Err(format!(
            "TG-1 thread pool shard count {shard_count} must match admitted CPU tokens {admitted_cpu_tokens}."
        ));
    }

    let lifecycle = required_json_object(obj, "lifecycle")?;
    if required_text(lifecycle, "init_policy")? != "once_per_run" {
        return Err("TG-1 init/prewarm policy must be once_per_run.".to_string());
    }
    if required_text(lifecycle, "finish_policy")? != "once_per_run" {
        return Err("TG-1 finish policy must be once_per_run.".to_string());
    }
    if required_i64(lifecycle, "finish_report_count")? != 1 {
        return Err("TG-1 finish report must be emitted exactly once.".to_string());
    }
    if required_text(lifecycle, "cleanup_policy")? != "all_tests_reclaimed_no_dirty" {
        return Err("TG-1 cleanup policy must reclaim all tests without dirty state.".to_string());
    }

    let cleanup = required_json_object(obj, "cleanup")?;
    if required_text(cleanup, "status")? != "cleaned" {
        return Err("TG-1 cleanup evidence must be cleaned.".to_string());
    }
    if optional_text(cleanup, "policy").as_deref() != Some("all_tests_reclaimed_no_dirty") {
        return Err("TG-1 cleanup evidence must use all_tests_reclaimed_no_dirty.".to_string());
    }
    Ok(())
}

fn manifest_runner_kind(manifest: &PatchsetSuiteManifest) -> &str {
    manifest
        .runner
        .get("kind")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .unwrap_or("")
}

fn workflow_ready_foreground_jobs(
    blocking_suite_ids: &[String],
    ready_aggregation: &PatchsetCiAggregationPlan,
) -> Vec<PatchsetCiJobPlan> {
    let mut jobs = blocking_suite_ids
        .iter()
        .map(|suite_id| suite_job(suite_id, true))
        .collect::<Vec<_>>();
    if !ready_aggregation.suite_ids.is_empty() {
        jobs.push(aggregate_job(ready_aggregation, true));
    }
    jobs
}

fn background_jobs(
    informational_suite_ids: &[String],
    informational_aggregation: Option<&PatchsetCiAggregationPlan>,
) -> Vec<PatchsetCiJobPlan> {
    let mut jobs = informational_suite_ids
        .iter()
        .map(|suite_id| suite_job(suite_id, false))
        .collect::<Vec<_>>();
    if let Some(aggregation) = informational_aggregation {
        jobs.push(aggregate_job(aggregation, false));
    }
    jobs
}

fn suite_job(suite_id: &str, workflow_ready_foreground: bool) -> PatchsetCiJobPlan {
    PatchsetCiJobPlan {
        job_type: "patchset.ci".to_string(),
        suite_id: Some(suite_id.to_string()),
        suite_ids: Vec::from([suite_id.to_string()]),
        stage: None,
        workflow_ready_foreground,
        updates_tests_status: false,
    }
}

fn aggregate_job(
    aggregation: &PatchsetCiAggregationPlan,
    workflow_ready_foreground: bool,
) -> PatchsetCiJobPlan {
    PatchsetCiJobPlan {
        job_type: "patchset.ci.aggregate".to_string(),
        suite_id: None,
        suite_ids: aggregation.suite_ids.clone(),
        stage: Some(aggregation.stage.clone()),
        workflow_ready_foreground,
        updates_tests_status: aggregation.updates_tests_status,
    }
}

fn patchset_ci_jobs_for_scope<'a>(
    plan: &'a PatchsetCiPlan,
    scope: &str,
) -> Result<Vec<&'a PatchsetCiJobPlan>, String> {
    match scope {
        "workflow_ready_foreground" => Ok(plan.workflow_ready_foreground_jobs.iter().collect()),
        "background" => Ok(plan.background_jobs.iter().collect()),
        "all" => Ok(plan
            .workflow_ready_foreground_jobs
            .iter()
            .chain(plan.background_jobs.iter())
            .collect()),
        _ => Err(format!(
            "Unsupported patchset CI dispatch scope `{scope}`. Expected workflow_ready_foreground, background, or all."
        )),
    }
}

fn patchset_ci_dispatch_job_id(
    job: &PatchsetCiJobPlan,
    patchset_id: &str,
    snapshot_id: &str,
) -> String {
    match job.job_type.as_str() {
        "patchset.ci" => format!(
            "patchset.ci:{patchset_id}:{}:{snapshot_id}",
            job.suite_id.as_deref().unwrap_or("unknown-suite")
        ),
        "patchset.ci.aggregate" => format!(
            "patchset.ci.aggregate:{patchset_id}:{}:{}:{snapshot_id}",
            job.stage.as_deref().unwrap_or("unknown-stage"),
            job.suite_ids.join("+")
        ),
        _ => format!("{}:{patchset_id}:{snapshot_id}", job.job_type),
    }
}

fn completed_suite_ids(request: &JsonMap<String, JsonValue>) -> Result<BTreeSet<String>, String> {
    let mut completed = BTreeSet::new();
    match request.get("completed_suite_ids") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Array(values)) => {
            for value in values {
                let suite_id = value
                    .as_str()
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .ok_or_else(|| {
                        "`completed_suite_ids` must contain non-empty strings.".to_string()
                    })?;
                completed.insert(suite_id.to_string());
            }
        }
        Some(JsonValue::String(value)) if !value.trim().is_empty() => {
            completed.insert(value.trim().to_string());
        }
        Some(_) => return Err("`completed_suite_ids` must be a string array.".to_string()),
    }
    Ok(completed)
}

fn missing_aggregate_suite_results(
    job: &PatchsetCiJobPlan,
    completed_suite_ids: &BTreeSet<String>,
) -> Vec<String> {
    if job.job_type != "patchset.ci.aggregate" {
        return Vec::new();
    }
    job.suite_ids
        .iter()
        .filter(|suite_id| !completed_suite_ids.contains(*suite_id))
        .cloned()
        .collect()
}

fn is_completed_suite_job(job: &PatchsetCiJobPlan, completed_suite_ids: &BTreeSet<String>) -> bool {
    job.job_type == "patchset.ci"
        && job
            .suite_id
            .as_ref()
            .map(|suite_id| completed_suite_ids.contains(suite_id))
            .unwrap_or(false)
}

fn copy_optional_payload_field(
    target: &mut JsonMap<String, JsonValue>,
    source: &JsonMap<String, JsonValue>,
    field: &str,
) {
    if let Some(value) = source.get(field).filter(|value| !value.is_null()) {
        target.insert(field.to_string(), value.clone());
    }
}

fn is_patchset_gate(manifest: &PatchsetSuiteManifest) -> bool {
    manifest.plane.trim().eq_ignore_ascii_case("patchset")
        && manifest.mode.trim().eq_ignore_ascii_case("gate")
}

fn reject_python_runner(runner_kind: &str) -> Result<(), String> {
    let lowered = runner_kind.to_ascii_lowercase();
    if lowered.contains("python") {
        return Err(format!(
            "Server CI runner `{runner_kind}` is not allowed for workflow-ready evidence."
        ));
    }
    Ok(())
}

fn optional_json_text(value: Option<&str>) -> JsonValue {
    match value.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => JsonValue::String(text.to_string()),
        None => JsonValue::Null,
    }
}

fn required_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Result<String, String> {
    optional_text(obj, field).ok_or_else(|| format!("`{field}` is required."))
}

fn optional_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    let text = obj.get(field)?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn required_i64(obj: &JsonMap<String, JsonValue>, field: &str) -> Result<i64, String> {
    optional_i64(obj, field).ok_or_else(|| format!("`{field}` is required."))
}

fn optional_i64(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<i64> {
    obj.get(field)?.as_i64()
}

fn required_json_object<'a>(
    obj: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    obj.get(field)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("`{field}` is required."))
}
