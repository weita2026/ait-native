use super::*;

#[derive(Debug, Clone)]
struct SuitePoolJob {
    index: usize,
    suite: PatchsetSuiteManifest,
    suite_id: String,
    runner_kind: String,
    cpu_tokens: i64,
}

struct SuitePoolCompletion {
    index: usize,
    suite_id: String,
    runner_kind: String,
    cpu_tokens: i64,
    duration_seconds: f64,
    result: Result<JsonValue, String>,
}

pub(super) fn run_suites_with_bounded_pool(
    config: &PatchsetCiRuntimeConfig,
    suites: &[PatchsetSuiteManifest],
) -> Result<(Vec<JsonValue>, JsonValue), String> {
    let started = Instant::now();
    validate_runner_authority(config, suites)?;
    let max_tokens = config.suite_pool_tokens.max(1);
    let mut jobs = suites
        .iter()
        .enumerate()
        .map(|(index, suite)| build_suite_pool_job(config, index, suite, max_tokens, suites.len()))
        .collect::<Result<VecDeque<_>, _>>()?;
    let scheduled_suites = jobs
        .iter()
        .map(|job| {
            json!({
                "index": job.index,
                "suite_id": job.suite_id,
                "runner_kind": job.runner_kind,
                "cpu_tokens": job.cpu_tokens,
            })
        })
        .collect::<Vec<_>>();

    let config = Arc::new(config.clone());
    let (tx, rx) = mpsc::channel::<SuitePoolCompletion>();
    let mut results = vec![JsonValue::Null; suites.len()];
    let mut completions = vec![JsonValue::Null; suites.len()];
    let mut running = 0usize;
    let mut used_tokens = 0i64;
    let mut errors = Vec::new();

    while !jobs.is_empty() || running > 0 {
        let mut index = 0usize;
        while index < jobs.len() {
            let job = jobs
                .get(index)
                .expect("suite pool job index should be in bounds");
            if used_tokens + job.cpu_tokens > max_tokens {
                index += 1;
                continue;
            }
            let job = jobs
                .remove(index)
                .expect("suite pool job should be removable");
            used_tokens += job.cpu_tokens;
            running += 1;
            let tx = tx.clone();
            let config = Arc::clone(&config);
            thread::spawn(move || {
                let started = Instant::now();
                let suite_id = job.suite_id.clone();
                let runner_kind = job.runner_kind.clone();
                let result = catch_unwind(AssertUnwindSafe(|| {
                    run_one_suite(config.as_ref(), &job.suite, job.cpu_tokens)
                }))
                .unwrap_or_else(|_| Err(format!("patchset CI suite `{suite_id}` panicked.")));
                let _ = tx.send(SuitePoolCompletion {
                    index: job.index,
                    suite_id,
                    runner_kind,
                    cpu_tokens: job.cpu_tokens,
                    duration_seconds: duration_seconds(started),
                    result,
                });
            });
        }

        if running == 0 {
            continue;
        }
        let completion = rx
            .recv()
            .map_err(|exc| format!("patchset CI suite pool failed to receive completion: {exc}"))?;
        running -= 1;
        used_tokens = (used_tokens - completion.cpu_tokens).max(0);
        let status = match &completion.result {
            Ok(result) => result
                .get("status")
                .and_then(JsonValue::as_str)
                .unwrap_or("fail")
                .to_string(),
            Err(_) => "error".to_string(),
        };
        completions[completion.index] = json!({
            "index": completion.index,
            "suite_id": completion.suite_id,
            "runner_kind": completion.runner_kind,
            "cpu_tokens": completion.cpu_tokens,
            "status": status,
            "duration_seconds": completion.duration_seconds,
        });
        match completion.result {
            Ok(result) => results[completion.index] = result,
            Err(error) => errors.push(error),
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("; "));
    }

    Ok((
        results,
        json!({
            "contract": "ait.server.patchset_ci.suite_pool.v1",
            "mode": "bounded_parallel",
            "prewarm_barrier": true,
            "prewarm_once": !config.prewarm_commands.is_empty() || config.main_seed_prewarm.is_some(),
            "prewarm_scope": if config.main_seed_prewarm.is_some() { "main_seed_generation" } else { "patchset_ci_run" },
            "max_cpu_tokens": max_tokens,
            "finish_policy": if config.flow.finish_after_all_suites { "aggregate_after_all_suites" } else { "legacy_inline" },
            "finish_report_count": 1,
            "suite_count": suites.len(),
            "scheduled_suites": scheduled_suites,
            "completed_suites": completions,
            "duration_seconds": duration_seconds(started),
            "server_ci_gate": {
                "component": "ait-server-core",
                "python_server_ci_executor": false,
                "rust_suite_pool": true,
            }
        }),
    ))
}

fn validate_runner_authority(
    config: &PatchsetCiRuntimeConfig,
    suites: &[PatchsetSuiteManifest],
) -> Result<(), String> {
    if !config.flow.rust_runner_only {
        return Ok(());
    }
    for suite in suites {
        let kind = suite
            .runner
            .get("kind")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .unwrap_or("");
        if kind == "pytest" {
            return Err(format!(
                "tg1_patchset_ci flow requires native Rust runners only; suite `{}` uses `{kind}`. Use `command_bundle`, `test_discovery_sharded`, or `server_tg1_required`.",
                suite.suite_id.trim()
            ));
        }
        if !matches!(
            kind,
            "command_bundle" | "test_discovery_sharded" | "server_tg1_required"
        ) {
            return Err(format!(
                "tg1_patchset_ci flow requires Rust-owned suite runners; suite `{}` uses `{kind}`.",
                suite.suite_id.trim()
            ));
        }
    }
    Ok(())
}

fn build_suite_pool_job(
    config: &PatchsetCiRuntimeConfig,
    index: usize,
    suite: &PatchsetSuiteManifest,
    max_tokens: i64,
    suite_count: usize,
) -> Result<SuitePoolJob, String> {
    let kind = suite
        .runner
        .get("kind")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .unwrap_or("");
    let cpu_tokens = suite_cpu_tokens(config, suite, kind, max_tokens, suite_count)?;
    Ok(SuitePoolJob {
        index,
        suite: suite.clone(),
        suite_id: suite.suite_id.trim().to_string(),
        runner_kind: rust_runner_kind(kind).to_string(),
        cpu_tokens,
    })
}

fn suite_cpu_tokens(
    config: &PatchsetCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    kind: &str,
    max_tokens: i64,
    suite_count: usize,
) -> Result<i64, String> {
    let runner = suite.runner.as_object().cloned().unwrap_or_default();
    let requested = match kind {
        "server_tg1_required" => tg1_requested_cpu_tokens(config, &runner),
        "command_bundle" | "test_discovery_sharded" => optional_i64(&runner, "cpu_tokens")
            .or_else(|| optional_i64(&runner, "workers"))
            .unwrap_or_else(|| {
                if suite_count == 1 {
                    max_tokens.max(1)
                } else {
                    1
                }
            }),
        "pytest" => {
            return Err(format!(
                "patchset CI runner kind `pytest` is no longer supported. Use native Rust runners only (`command_bundle`, `test_discovery_sharded`, `server_tg1_required`) for suite `{}`.",
                suite.suite_id.trim()
            ))
        }
        value => {
            return Err(format!(
                "Unsupported patchset CI runner kind `{value}` for suite `{}`.",
                suite.suite_id
            ));
        }
    };
    Ok(requested.max(1).min(max_tokens.max(1)))
}

pub(super) fn run_one_suite(
    config: &PatchsetCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    admitted_cpu_tokens: i64,
) -> Result<JsonValue, String> {
    let kind = suite
        .runner
        .get("kind")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .unwrap_or("");
    if suite.suite_id.trim() == TG1_REQUIRED_SUITE_ID && kind != "server_tg1_required" {
        return Err(
            "TG-1 required must use server_tg1_required; all other runners are forbidden for TG1."
                .to_string(),
        );
    }
    let result = match kind {
        "command_bundle" => run_command_bundle_suite(config, suite, admitted_cpu_tokens)?,
        "test_discovery_sharded" => {
            run_test_discovery_sharded_suite(config, suite, admitted_cpu_tokens)?
        }
        "server_tg1_required" => run_server_tg1_required_suite(config, suite, admitted_cpu_tokens)?,
        value => {
            return Err(format!(
                "Unsupported patchset CI runner kind `{value}` for suite `{}`.",
                suite.suite_id
            ));
        }
    };
    let status = result
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("fail")
        .to_string();
    let mut payload = JsonMap::new();
    payload.insert("suite_id".to_string(), json!(suite.suite_id.trim()));
    payload.insert(
        "display_name".to_string(),
        optional_json_text(suite.display_name.as_deref()),
    );
    payload.insert(
        "artifact_path".to_string(),
        optional_json_text(suite.artifact_path.as_deref()),
    );
    payload.insert("plane".to_string(), json!(suite.plane.trim()));
    payload.insert("mode".to_string(), json!(suite.mode.trim()));
    payload.insert("blocking".to_string(), json!(suite_is_blocking(suite)));
    payload.insert(
        "purpose".to_string(),
        optional_json_text(suite.purpose.as_deref()),
    );
    payload.insert("runner_kind".to_string(), json!(rust_runner_kind(kind)));
    payload.insert(
        "runner_parallelism".to_string(),
        json!(admitted_cpu_tokens.max(1)),
    );
    payload.insert(
        "runner_parallelism_source".to_string(),
        json!("scheduler_admitted_cpu_tokens"),
    );
    payload.insert("status".to_string(), json!(status));
    payload.insert(
        "artifacts".to_string(),
        result
            .get("artifacts")
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    payload.insert(
        "duration_seconds".to_string(),
        result
            .get("duration_seconds")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    if let Some(reports) = result.get("command_reports") {
        payload.insert("command_reports".to_string(), reports.clone());
    }
    if let Some(shards) = result.get("command_bundle_shards") {
        payload.insert("command_bundle_shards".to_string(), shards.clone());
    }
    if let Some(shards) = result.get("test_shards") {
        payload.insert(
            "test_shards".to_string(),
            compact_test_shards_evidence(shards),
        );
    }
    if let Some(discovery) = result.get("discovery") {
        payload.insert(
            "discovery".to_string(),
            compact_test_discovery_evidence(discovery),
        );
    }
    if let Some(checks) = result.get("checks") {
        payload.insert("checks".to_string(), checks.clone());
    }
    if let Some(doc_tests) = result.get("doc_tests") {
        payload.insert("doc_tests".to_string(), doc_tests.clone());
    }
    if let Some(shards) = result.get("thread_pool_shards") {
        payload.insert("thread_pool_shards".to_string(), shards.clone());
    }
    if let Some(failure) = result.get("failure") {
        payload.insert("failure".to_string(), failure.clone());
    }
    if let Some(reason) = result.get("failure_reason") {
        payload.insert("failure_reason".to_string(), reason.clone());
    }
    if let Some(summary) = result.get("tg1_required_summary") {
        payload.insert("tg1_required_summary".to_string(), summary.clone());
    }
    let mut server_ci_gate = result
        .get("server_ci_gate")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    server_ci_gate.insert("component".to_string(), json!("ait-server-core"));
    server_ci_gate.insert("python_server_ci_executor".to_string(), json!(false));
    server_ci_gate.insert("python_foreground".to_string(), json!(false));
    server_ci_gate.insert("legacy_runner_foreground".to_string(), json!(false));
    server_ci_gate.insert("scheduler_authority".to_string(), json!("server_scheduler"));
    server_ci_gate.insert("thread_pool_owner".to_string(), json!("server"));
    server_ci_gate.insert(
        "runner_parallelism_source".to_string(),
        json!("scheduler_admitted_cpu_tokens"),
    );
    server_ci_gate.insert(
        "runner_parallelism".to_string(),
        json!(admitted_cpu_tokens.max(1)),
    );
    payload.insert(
        "server_ci_gate".to_string(),
        JsonValue::Object(server_ci_gate),
    );
    Ok(JsonValue::Object(payload))
}

fn selected_object_fields(value: &JsonValue, fields: &[&str]) -> JsonMap<String, JsonValue> {
    let Some(source) = value.as_object() else {
        return JsonMap::new();
    };
    fields
        .iter()
        .filter_map(|field| {
            source
                .get(*field)
                .cloned()
                .map(|value| ((*field).to_string(), value))
        })
        .collect()
}

fn compact_process_report_evidence(value: &JsonValue) -> JsonValue {
    let mut out = selected_object_fields(
        value,
        &[
            "status",
            "command",
            "exit_code",
            "duration_seconds",
            "log_path",
            "stdout_bytes",
            "stderr_bytes",
        ],
    );
    out.insert(
        "detail_policy".to_string(),
        json!("bounded_runtime_evidence"),
    );
    JsonValue::Object(out)
}

fn compact_test_discovery_evidence(value: &JsonValue) -> JsonValue {
    let mut out = selected_object_fields(
        value,
        &[
            "status",
            "executable_count",
            "test_case_count",
            "fallback_executable_count",
            "excluded_test_case_count",
        ],
    );
    if let Some(report) = value.get("test_case_discovery") {
        let mut report = selected_object_fields(
            report,
            &[
                "contract",
                "status",
                "duration_seconds",
                "test_case_count",
                "fallback_executable_count",
                "excluded_test_case_count",
                "unit_count",
            ],
        );
        report.insert("detail_omitted".to_string(), json!(true));
        out.insert("test_case_discovery".to_string(), JsonValue::Object(report));
    }
    if let Some(report) = value.get("build_report") {
        out.insert(
            "build_report".to_string(),
            compact_process_report_evidence(report),
        );
    }
    out.insert("detail_omitted".to_string(), json!(true));
    out.insert(
        "detail_policy".to_string(),
        json!("bounded_runtime_evidence"),
    );
    JsonValue::Object(out)
}

fn compact_test_shards_evidence(value: &JsonValue) -> JsonValue {
    let mut out = selected_object_fields(
        value,
        &[
            "contract",
            "status",
            "duration_seconds",
            "shard_count",
            "distribution",
            "shard_by",
            "test_case_count",
            "fallback_executable_count",
            "unit_count",
            "failure",
        ],
    );
    let shard_result_count = value
        .get("shards")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    out.insert("shard_result_count".to_string(), json!(shard_result_count));
    out.insert("detail_omitted".to_string(), json!(true));
    out.insert(
        "detail_policy".to_string(),
        json!("bounded_runtime_evidence"),
    );
    JsonValue::Object(out)
}

#[cfg(test)]
mod bounded_evidence_tests {
    use super::*;

    #[test]
    fn discovery_and_shard_runtime_bodies_are_not_persisted_in_suite_results() {
        let units = (0..836)
            .map(|index| {
                json!({
                    "index": index,
                    "unit_kind": "test_case",
                    "name": format!("tests::case_{index}"),
                    "executable": {"path": "/tmp/test-bin"},
                })
            })
            .collect::<Vec<_>>();
        let reports = (0..9)
            .map(|index| json!({"index": index, "units": units}))
            .collect::<Vec<_>>();
        let discovery = compact_test_discovery_evidence(&json!({
            "status": "pass",
            "executable_count": 1,
            "test_case_count": 836,
            "fallback_executable_count": 0,
            "excluded_test_case_count": 0,
            "test_case_discovery": {
                "contract": "ait.server.ci_test_case_discovery.v1",
                "status": "pass",
                "test_case_count": 836,
                "unit_count": 836,
                "reports": reports,
                "units": units,
            },
            "build_report": {
                "status": "pass",
                "command": "cargo test --no-run --message-format=json",
                "stdout_tail": "x".repeat(8_000),
                "stderr_tail": "y".repeat(8_000),
                "combined_tail": "z".repeat(12_000),
            },
        }));
        let shard_bodies = (0..9)
            .map(|index| json!({"shard_id": format!("shard-{index}"), "units": units}))
            .collect::<Vec<_>>();
        let shards = compact_test_shards_evidence(&json!({
            "contract": "ait.server.ci_test_case_shards.v1",
            "status": "pass",
            "shard_count": 9,
            "test_case_count": 836,
            "unit_count": 836,
            "shards": shard_bodies,
        }));

        assert!(
            serde_json::to_vec(&discovery).unwrap().len() < 8 * 1024,
            "discovery summary must remain bounded"
        );
        assert!(
            serde_json::to_vec(&shards).unwrap().len() < 4 * 1024,
            "shard summary must remain bounded"
        );
        assert!(discovery.pointer("/test_case_discovery/units").is_none());
        assert!(shards.get("shards").is_none());
        assert_eq!(shards["shard_result_count"], json!(9));
    }
}

#[derive(Clone, Debug)]
pub(super) struct CommandBundleShard {
    shard_id: String,
    repo_dir: PathBuf,
    output_dir: PathBuf,
}

#[derive(Debug)]
struct CommandBundleShardCompletion {
    shard: CommandBundleShard,
    command_indices: Vec<usize>,
    result: Result<JsonValue, String>,
}

pub(super) fn run_command_bundle_sharded_suite(
    config: &PatchsetCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    admitted_cpu_tokens: i64,
    commands: Vec<String>,
    prepared_shards: Vec<CommandBundleShard>,
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let suite_id = suite.suite_id.trim();
    let output_dir = config.output_dir.join(suite_id);
    let shard_count = (admitted_cpu_tokens.max(1) as usize)
        .min(prepared_shards.len())
        .min(commands.len())
        .max(1);
    let shards = prepared_shards
        .into_iter()
        .take(shard_count)
        .collect::<Vec<_>>();
    let mut assignments = (0..shard_count).map(|_| Vec::new()).collect::<Vec<_>>();
    for (index, command) in commands.into_iter().enumerate() {
        assignments[index % shard_count].push((index, command));
    }

    let (tx, rx) = mpsc::channel::<CommandBundleShardCompletion>();
    let mut spawned = 0usize;
    for (shard_index, command_group) in assignments.into_iter().enumerate() {
        if command_group.is_empty() {
            continue;
        }
        spawned += 1;
        let tx = tx.clone();
        let config = config.clone();
        let suite = suite.clone();
        let shard = shards[shard_index].clone();
        thread::spawn(move || {
            let command_indices = command_group
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>();
            let shard_commands = command_group
                .into_iter()
                .map(|(_, command)| command)
                .collect::<Vec<_>>();
            let result = run_one_command_bundle_shard(&config, &suite, &shard, shard_commands);
            let _ = tx.send(CommandBundleShardCompletion {
                shard,
                command_indices,
                result,
            });
        });
    }
    drop(tx);

    let mut completions = Vec::new();
    for _ in 0..spawned {
        completions.push(
            rx.recv()
                .map_err(|exc| format!("command-bundle shard executor failed: {exc}"))?,
        );
    }
    completions.sort_by(|left, right| left.shard.shard_id.cmp(&right.shard.shard_id));

    let mut status = "pass";
    let mut failure = JsonValue::Null;
    let mut command_reports = Vec::new();
    let mut shard_reports = Vec::new();
    for completion in completions {
        match completion.result {
            Ok(result) => {
                if result.get("status").and_then(JsonValue::as_str) != Some("pass") {
                    status = "fail";
                    if failure.is_null() {
                        failure = result.get("failure").cloned().unwrap_or_else(|| {
                            json!({
                                "stage": "command_bundle_shard",
                                "shard_id": completion.shard.shard_id,
                            })
                        });
                    }
                }
                let reports = result
                    .get("command_reports")
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut shard_command_reports = Vec::new();
                for (local_index, report) in reports.into_iter().enumerate() {
                    let original_index = completion
                        .command_indices
                        .get(local_index)
                        .copied()
                        .unwrap_or(local_index);
                    let mut object = report.as_object().cloned().unwrap_or_default();
                    object.insert("index".to_string(), json!(original_index + 1));
                    object.insert("shard_command_index".to_string(), json!(local_index + 1));
                    object.insert("shard_id".to_string(), json!(completion.shard.shard_id));
                    object.insert(
                        "workspace_path".to_string(),
                        json!(path_string(&completion.shard.repo_dir)),
                    );
                    object.insert(
                        "shard_output_dir".to_string(),
                        json!(path_string(&completion.shard.output_dir)),
                    );
                    let report = JsonValue::Object(object);
                    shard_command_reports.push(report.clone());
                    command_reports.push(report);
                }
                shard_reports.push(json!({
                    "shard_id": completion.shard.shard_id,
                    "status": result["status"].clone(),
                    "command_indices": completion.command_indices.iter().map(|index| json!(index + 1)).collect::<Vec<_>>(),
                    "repo_dir": path_string(&completion.shard.repo_dir),
                    "output_dir": path_string(&completion.shard.output_dir),
                    "artifacts": result["artifacts"].clone(),
                    "command_reports": shard_command_reports,
                    "duration_seconds": result["duration_seconds"].clone(),
                }));
            }
            Err(message) => {
                status = "fail";
                if failure.is_null() {
                    failure = json!({
                        "stage": "command_bundle_shard",
                        "shard_id": completion.shard.shard_id,
                        "message": message,
                    });
                }
                shard_reports.push(json!({
                    "shard_id": completion.shard.shard_id,
                    "status": "fail",
                    "command_indices": completion.command_indices.iter().map(|index| json!(index + 1)).collect::<Vec<_>>(),
                    "repo_dir": path_string(&completion.shard.repo_dir),
                    "output_dir": path_string(&completion.shard.output_dir),
                    "failure": message,
                }));
            }
        }
    }
    command_reports.sort_by_key(|report| {
        report
            .get("index")
            .and_then(JsonValue::as_u64)
            .unwrap_or(u64::MAX)
    });

    let mut summary = json!({
        "contract": "ait.server.patchset_ci.command_bundle_shards.v1",
        "status": status,
        "duration_seconds": duration_seconds(started),
        "suite_id": suite_id,
        "job_type": "patchset.ci",
        "job_id": config.patchset_id,
        "workspace_path": path_string(&config.workspace_path),
        "output_dir": path_string(&output_dir),
        "runner": {
            "kind": "command_bundle",
            "command_count": command_reports.len(),
            "distribution": "server_thread_pool_shards",
            "per_shard_runner_parallelism": 1,
        },
        "environment": {
            "runner_parallelism": admitted_cpu_tokens.max(1),
            "admitted_cpu_tokens": admitted_cpu_tokens.max(1),
            "per_shard_runner_parallelism": 1,
            "parallelism_source": "scheduler",
        },
        "command_reports": command_reports,
        "command_bundle_shards": {
            "shard_count": shard_reports.len(),
            "admitted_cpu_tokens": admitted_cpu_tokens.max(1),
            "distribution": "commands_partitioned_across_server_worktrees",
            "shards": shard_reports.clone(),
        },
        "thread_pool_shards": {
            "shard_count": shard_reports.len(),
            "admitted_cpu_tokens": admitted_cpu_tokens.max(1),
            "shards": shard_reports,
        },
        "failure": failure,
        "server_ci_gate": {
            "component": "ait-server-core",
            "python_command_runner": false,
            "python_command_bundle": false,
            "rust_command_bundle_shards": true,
            "scheduler_authority": "server_scheduler",
            "thread_pool_owner": "server",
        }
    });
    let artifacts = write_command_bundle_shard_artifacts(&output_dir, suite, &summary)?;
    summary["artifacts"] = artifacts;
    Ok(summary)
}

pub(super) fn run_one_command_bundle_shard(
    config: &PatchsetCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    shard: &CommandBundleShard,
    commands: Vec<String>,
) -> Result<JsonValue, String> {
    let mut runner = suite.runner.as_object().cloned().unwrap_or_default();
    runner.insert("commands".to_string(), json!(commands));
    let output_dir = config
        .output_dir
        .join(suite.suite_id.trim())
        .join("command-shards")
        .join(&shard.shard_id);
    let mut payload = command_bundle_base_payload(config, output_dir);
    payload.insert(
        "workspace_path".to_string(),
        json!(path_string(&shard.repo_dir)),
    );
    payload.insert(
        "temp_dir".to_string(),
        json!(path_string(&shard.repo_dir.join(".tmp"))),
    );
    payload.insert("suite_id".to_string(), json!(suite.suite_id.trim()));
    payload.insert("runner_parallelism".to_string(), json!(1));
    payload.insert("admitted_cpu_tokens".to_string(), json!(1));
    payload.insert("runner".to_string(), JsonValue::Object(runner));
    payload.insert(
        "artifacts".to_string(),
        json!({"summary_json": "summary.json", "log_path": "run.log"}),
    );
    ci_command_bundle_run_json(&JsonValue::Object(payload))
}

pub(super) fn prepared_command_bundle_shards(
    config: &PatchsetCiRuntimeConfig,
) -> Result<Vec<CommandBundleShard>, String> {
    let Some(materialization) = &config.snapshot_materialization_result else {
        return Ok(Vec::new());
    };
    let shard_values = materialization
        .get("thread_pool_shards")
        .and_then(|value| value.get("shards"))
        .and_then(JsonValue::as_array)
        .or_else(|| {
            materialization
                .get("shard_prepare")
                .and_then(|value| value.get("thread_pool_shards"))
                .and_then(|value| value.get("shards"))
                .and_then(JsonValue::as_array)
        });
    let Some(shard_values) = shard_values else {
        return Ok(Vec::new());
    };
    shard_values
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or_else(|| "thread_pool_shards.shards[] must be an object.".to_string())?;
            Ok(CommandBundleShard {
                shard_id: required_text(object, "shard_id")?,
                repo_dir: PathBuf::from(required_text(object, "repo_dir")?),
                output_dir: PathBuf::from(required_text(object, "output_dir")?),
            })
        })
        .collect()
}

pub(super) fn write_command_bundle_shard_artifacts(
    output_dir: &Path,
    suite: &PatchsetSuiteManifest,
    summary: &JsonValue,
) -> Result<JsonValue, String> {
    fs::create_dir_all(output_dir).map_err(|exc| {
        format!(
            "Failed to create command-bundle shard output dir `{}`: {exc}",
            path_string(output_dir)
        )
    })?;
    let summary_path = output_dir.join("command_bundle_shards.json");
    let log_path = output_dir.join("command_bundle_shards.log");
    fs::write(
        &summary_path,
        serde_json::to_string_pretty(summary).map_err(|exc| exc.to_string())? + "\n",
    )
    .map_err(|exc| format!("Failed to write `{}`: {exc}", path_string(&summary_path)))?;
    fs::write(&log_path, command_bundle_shard_log_text(suite, summary))
        .map_err(|exc| format!("Failed to write `{}`: {exc}", path_string(&log_path)))?;
    Ok(json!({
        "summary_json": artifact_payload(&summary_path),
        "log_path": artifact_payload(&log_path),
    }))
}

pub(super) fn command_bundle_shard_log_text(
    suite: &PatchsetSuiteManifest,
    summary: &JsonValue,
) -> String {
    let mut lines = vec![
        format!("suite={}", suite.suite_id.trim()),
        format!(
            "status={}",
            summary
                .get("status")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
        ),
        format!(
            "shard_count={}",
            summary["command_bundle_shards"]["shard_count"]
                .as_i64()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ),
    ];
    for report in summary["command_reports"].as_array().into_iter().flatten() {
        lines.push(format!(
            "command[{}] shard={} status={} exit_code={} log_path={}",
            report
                .get("index")
                .and_then(JsonValue::as_u64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string()),
            report
                .get("shard_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("?"),
            report
                .get("status")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown"),
            report
                .get("exit_code")
                .and_then(JsonValue::as_i64)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string()),
            report
                .get("log_path")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
        ));
    }
    lines.join("\n") + "\n"
}
