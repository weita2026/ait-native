use super::*;

pub(super) fn run_server_tg1_required_suite(
    config: &PatchsetCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
    admitted_cpu_tokens: i64,
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let runner = suite.runner.as_object().cloned().unwrap_or_default();
    let source_repo_root = optional_path(&runner, "source_repo_root")
        .or_else(|| optional_path(&config.tg1, "source_repo_root"))
        .ok_or_else(|| "server_tg1_required requires tg1.source_repo_root.".to_string())?;
    let target_seed_root = optional_path(&runner, "target_seed_root")
        .or_else(|| optional_path(&runner, "target_repo_root"))
        .or_else(|| optional_path(&runner, "main_seed_path"))
        .or_else(|| optional_path(&config.tg1, "target_seed_root"))
        .or_else(|| optional_path(&config.tg1, "target_repo_root"))
        .or_else(|| optional_path(&config.tg1, "main_seed_path"))
        .unwrap_or_else(|| config.workspace_path.clone());
    let test_group_id =
        optional_text(&runner, "test_group_id").unwrap_or_else(|| "TG-1".to_string());
    let target_repo = optional_text(&runner, "target_repo")
        .or_else(|| optional_text(&config.tg1, "target_repo"))
        .unwrap_or_else(|| config.repo_name.clone());
    let static_contract =
        load_ait_test_static_tg1_contract(&source_repo_root, &target_repo, &test_group_id)?;
    let mut node_ids = string_array(&runner, "node_ids")?;
    let mut membership_source = "runner_manifest".to_string();
    if node_ids.is_empty() {
        node_ids = string_array(&config.tg1, "node_ids")?;
        membership_source = "request_tg1_payload".to_string();
    }
    if node_ids.is_empty() {
        if let Some(contract) = &static_contract {
            node_ids = contract.node_ids.clone();
            membership_source = contract.membership_source.clone();
        } else {
            membership_source = "empty".to_string();
        }
    }
    let live_count = node_ids.len() as i64;
    let minimum_count = optional_i64(&runner, "minimum_count")
        .or_else(|| optional_i64(&config.tg1, "minimum_count"))
        .or_else(|| {
            static_contract
                .as_ref()
                .map(|contract| contract.minimum_count)
        })
        .unwrap_or(TG1_DEFAULT_MINIMUM_COUNT);
    let requested_cpu_tokens = tg1_requested_cpu_tokens(config, &runner);
    let thread_pool_tokens = admitted_cpu_tokens.max(1);
    let output_dir = config.output_dir.join(suite.suite_id.trim());
    let repo_name = optional_text(&runner, "repo_name")
        .or_else(|| optional_text(&config.tg1, "repo_name"))
        .unwrap_or_else(|| config.repo_name.clone());
    let normalized_node_ids = normalize_tg1_node_ids(&source_repo_root, &node_ids);
    let mut summary = json!({
        "status": "fail",
        "validation_status": "fail",
        "repo_name": repo_name,
        "test_group_id": test_group_id,
        "membership_source": membership_source,
        "target_repo": target_repo,
        "catalog_source_root": path_string(&source_repo_root),
        "target_seed_root": path_string(&target_seed_root),
        "minimum_count": minimum_count,
        "live_count": live_count,
        "scheduler": tg1_scheduler_evidence(requested_cpu_tokens, thread_pool_tokens),
        "lifecycle": tg1_lifecycle_evidence(1, "not_started"),
        "runner": {
            "status": "fail",
            "workers": thread_pool_tokens,
            "distribution": "server_thread_pool_shards",
            "node_count": live_count,
            "normalized_node_count": normalized_node_ids.len(),
            "exit_code": JsonValue::Null
        },
        "thread_pool_shards": {
            "shard_count": 0,
            "shards": []
        },
        "cleanup": {
            "status": "not_started",
            "policy": "all_tests_reclaimed_no_dirty"
        }
    });
    if live_count < minimum_count {
        let failure_reason = format!(
            "TG-1 live membership has {live_count} case(s); expected at least {minimum_count}."
        );
        summary["failure_reason"] = json!(failure_reason);
        return Ok(json!({
            "status": "fail",
            "duration_seconds": 0.0,
            "failure_reason": failure_reason,
            "artifacts": write_tg1_artifacts(&output_dir, suite, &summary, "")?,
            "tg1_required_summary": summary,
        }));
    }

    let shard_run = ci_test_shard_run_json(&tg1_shard_runner_request(
        config,
        &runner,
        suite,
        &source_repo_root,
        &target_seed_root,
        &output_dir,
        &normalized_node_ids,
        thread_pool_tokens,
    )?)?;
    let status = if shard_run["status"].as_str() == Some("pass") {
        "pass"
    } else {
        "fail"
    };
    summary["status"] = json!(status);
    summary["validation_status"] = json!(status);
    summary["runner"]["status"] = json!(status);
    summary["runner"]["duration_seconds"] = shard_run["duration_seconds"].clone();
    summary["runner"]["details"] = shard_run["runner"].clone();
    summary["main_seed"] = shard_run["main_seed"].clone();
    summary["thread_pool_shards"] = shard_run["thread_pool_shards"].clone();
    summary["cleanup"] = tg1_cleanup_evidence(&shard_run);
    summary["lifecycle"] = tg1_lifecycle_evidence(1, "cleaned");
    if status != "pass" {
        summary["failure_reason"] = json!("TG-1 shard runner failed.");
    }
    let artifacts = tg1_artifacts_with_thread_pool(
        &output_dir,
        suite,
        &summary,
        &tg1_thread_pool_log_text(&shard_run),
        &shard_run,
    )?;
    let mut result = json!({
        "status": status,
        "duration_seconds": duration_seconds(started),
        "artifacts": artifacts,
        "tg1_required_summary": summary,
        "server_ci_gate": {
            "component": "ait-server-core",
            "capability": "server.patchset_ci.workflow_ready_evidence",
            "python_command_runner": false,
            "python_command_bundle": false,
            "rust_thread_pool_shard_runner": true,
            "scheduler_authority": "server_scheduler",
            "thread_pool_owner": "server",
        }
    });
    if status != "pass" {
        result["failure_reason"] = json!("TG-1 shard runner failed.");
    }
    Ok(result)
}

fn normalize_tg1_node_ids(repo_root: &Path, node_ids: &[String]) -> Vec<String> {
    node_ids
        .iter()
        .map(|node_id| {
            let (path_part, suffix) = node_id
                .split_once("::")
                .map(|(path, suffix)| (path, Some(suffix)))
                .unwrap_or((node_id.as_str(), None));
            if !path_part.is_empty() && !repo_root.join(path_part).exists() {
                let tests_path = format!("tests/{path_part}");
                if repo_root.join(&tests_path).exists() {
                    return match suffix {
                        Some(suffix) => format!("{tests_path}::{suffix}"),
                        None => tests_path,
                    };
                }
                if let Some(stripped_path) = path_part.strip_prefix("tests/") {
                    if repo_root.join(stripped_path).exists() {
                        return match suffix {
                            Some(suffix) => format!("{stripped_path}::{suffix}"),
                            None => stripped_path.to_string(),
                        };
                    }
                }
            }
            node_id.clone()
        })
        .collect()
}

#[derive(Debug, Clone)]
struct StaticTg1Contract {
    node_ids: Vec<String>,
    minimum_count: i64,
    membership_source: String,
}

fn load_ait_test_static_tg1_contract(
    source_repo_root: &Path,
    target_repo: &str,
    test_group_id: &str,
) -> Result<Option<StaticTg1Contract>, String> {
    let descriptor_path = source_repo_root.join("descriptors/suites/ait.tg1.toml");
    if !descriptor_path.is_file() {
        return Ok(None);
    }
    let descriptor = fs::read_to_string(&descriptor_path).map_err(|exc| {
        format!(
            "Failed to read ait-test TG1 descriptor `{}`: {exc}",
            path_string(&descriptor_path)
        )
    })?;
    if descriptor_text_value(&descriptor, "target_repo").as_deref() != Some(target_repo) {
        return Ok(None);
    }
    if descriptor_text_value(&descriptor, "test_group_id").as_deref() != Some(test_group_id) {
        return Ok(None);
    }
    let minimum_count =
        descriptor_i64_value(&descriptor, "minimum_count").unwrap_or(TG1_DEFAULT_MINIMUM_COUNT);
    let source_rel = descriptor_text_value(&descriptor, "formal_members_source")
        .unwrap_or_else(|| "crates/ait-test-contract/src/test_groups.rs".to_string());
    let source_path = source_repo_root.join(source_rel);
    let members_source = fs::read_to_string(&source_path).map_err(|exc| {
        format!(
            "Failed to read ait-test TG1 formal member source `{}`: {exc}",
            path_string(&source_path)
        )
    })?;
    let node_ids = parse_corpus_node_ids(&members_source);
    if node_ids.len() < minimum_count as usize {
        return Err(format!(
            "ait-test TG1 static contract has {} corpus node id(s); expected at least {minimum_count}.",
            node_ids.len()
        ));
    }
    Ok(Some(StaticTg1Contract {
        node_ids,
        minimum_count,
        membership_source: "ait_test_static_descriptor".to_string(),
    }))
}

fn descriptor_text_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let value = trimmed.strip_prefix(key)?.trim_start();
        let value = value.strip_prefix('=')?.trim();
        let value = value.strip_prefix('"')?;
        let (value, _) = value.split_once('"')?;
        Some(value.to_string())
    })
}

fn descriptor_i64_value(text: &str, key: &str) -> Option<i64> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let value = trimmed.strip_prefix(key)?.trim_start();
        let value = value.strip_prefix('=')?.trim();
        value.parse::<i64>().ok()
    })
}

fn parse_corpus_node_ids(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let (_, value) = line.split_once("corpus_node_id: \"")?;
            let (node_id, _) = value.split_once('"')?;
            Some(node_id.to_string())
        })
        .collect()
}

pub(super) fn tg1_requested_cpu_tokens(
    config: &PatchsetCiRuntimeConfig,
    runner: &JsonMap<String, JsonValue>,
) -> i64 {
    if config.flow.is_tg1_patchset_ci() {
        return config
            .flow
            .fixed_cpu_tokens
            .unwrap_or(TG1_REQUIRED_CPU_TOKENS)
            .max(1);
    }
    optional_i64(runner, "requested_cpu_tokens")
        .or_else(|| optional_i64(runner, "cpu_tokens"))
        .or_else(|| optional_i64(&config.tg1, "requested_cpu_tokens"))
        .or_else(|| optional_i64(&config.tg1, "cpu_tokens"))
        .or_else(|| optional_i64(runner, "workers"))
        .or_else(|| optional_i64(&config.tg1, "workers"))
        .unwrap_or(TG1_DEFAULT_REQUESTED_CPU_TOKENS)
        .max(1)
}

fn tg1_scheduler_evidence(requested_cpu_tokens: i64, admitted_cpu_tokens: i64) -> JsonValue {
    json!({
        "authority": "server_scheduler",
        "thread_pool_owner": "server",
        "requested_cpu_tokens": requested_cpu_tokens.max(1),
        "admitted_cpu_tokens": admitted_cpu_tokens.max(1),
        "runner_parallelism_source": "scheduler_admitted_cpu_tokens",
    })
}

fn tg1_lifecycle_evidence(finish_report_count: i64, cleanup_status: &str) -> JsonValue {
    json!({
        "init_policy": "once_per_run",
        "prewarm_policy": "main_seed_once_per_run",
        "prewarm_once": true,
        "finish_policy": "once_per_run",
        "finish_report_count": finish_report_count,
        "cleanup_policy": "all_tests_reclaimed_no_dirty",
        "cleanup_status": cleanup_status,
    })
}

fn tg1_cleanup_evidence(shard_run: &JsonValue) -> JsonValue {
    let cleanup = shard_run
        .get("cleanup")
        .cloned()
        .unwrap_or_else(|| json!({"operation": "missing"}));
    let cleaned = cleanup.get("operation").and_then(JsonValue::as_str) == Some("cleanup");
    json!({
        "status": if cleaned { "cleaned" } else { "unknown" },
        "policy": "all_tests_reclaimed_no_dirty",
        "all_shards_completed": true,
        "outputs_merged": true,
        "raw": cleanup,
    })
}

#[allow(clippy::too_many_arguments)]
fn tg1_shard_runner_request(
    config: &PatchsetCiRuntimeConfig,
    runner: &JsonMap<String, JsonValue>,
    suite: &PatchsetSuiteManifest,
    source_repo_root: &Path,
    target_seed_root: &Path,
    output_dir: &Path,
    normalized_node_ids: &[String],
    admitted_cpu_tokens: i64,
) -> Result<JsonValue, String> {
    Ok(json!({
        "job_type": "patchset.ci",
        "job_id": config.patchset_id,
        "main_seed_path": path_string(target_seed_root),
        "shard_root": path_string(&output_dir.join("thread-pool-shards")),
        "merged_output_dir": path_string(&output_dir.join("thread-pool-merged")),
        "admitted_cpu_tokens": admitted_cpu_tokens.max(1),
        "pool_id": format!("patchset-ci-{}", suite.suite_id.trim()),
        "payload": {
            "repo_name": config.repo_name,
            "patchset_id": config.patchset_id,
            "revision_snapshot_id": config.revision_snapshot_id,
            "suite_id": suite.suite_id.trim(),
        },
        "test_items": normalized_node_ids,
        "runner": explicit_tg1_shard_runner_payload(
            config,
            runner,
            source_repo_root,
            target_seed_root,
            normalized_node_ids,
            admitted_cpu_tokens,
        )?,
        "artifacts": {
            "summary_json": "thread-pool-summary.json",
            "log_path": "thread-pool.log"
        },
        "cleanup": true
    }))
}

fn explicit_tg1_shard_runner_payload(
    config: &PatchsetCiRuntimeConfig,
    runner: &JsonMap<String, JsonValue>,
    source_repo_root: &Path,
    target_seed_root: &Path,
    normalized_node_ids: &[String],
    admitted_cpu_tokens: i64,
) -> Result<JsonValue, String> {
    let (program, args) = tg1_native_runner_command(config, runner)?;
    reject_python_tg1_runner(&program)?;
    let append_test_items = optional_bool(runner, "append_test_items")?
        .or(optional_bool(&config.tg1, "append_test_items")?)
        .unwrap_or(true);
    Ok(json!({
        "kind": "command",
        "program": program,
        "args": args,
        "append_test_items": append_test_items,
        "env": tg1_runner_env(
            config,
            runner,
            source_repo_root,
            target_seed_root,
            normalized_node_ids,
            admitted_cpu_tokens,
        ),
    }))
}

fn tg1_native_runner_command(
    config: &PatchsetCiRuntimeConfig,
    runner: &JsonMap<String, JsonValue>,
) -> Result<(String, Vec<String>), String> {
    let mut program = optional_text(runner, "native_runner_program")
        .or_else(|| optional_text(runner, "program"))
        .or_else(|| optional_text(&config.tg1, "native_runner_program"))
        .or_else(|| optional_text(&config.tg1, "program"));
    let mut args = string_array(runner, "native_runner_args")?;
    if args.is_empty() {
        args = string_array(runner, "args")?;
    }
    if args.is_empty() {
        args = string_array(&config.tg1, "native_runner_args")?;
    }
    if args.is_empty() {
        args = string_array(&config.tg1, "args")?;
    }
    if program.is_none() && args.is_empty() {
        if let Some(command) = optional_text(runner, "native_runner_command")
            .or_else(|| optional_text(&config.tg1, "native_runner_command"))
        {
            let parts = command
                .split_whitespace()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if let Some((head, tail)) = parts.split_first() {
                program = Some(head.clone());
                args = tail.to_vec();
            }
        }
    }
    let program = program
        .or_else(|| native_ait_cli_from_env(&config.env))
        .or_else(|| {
            config
                .shared_cargo_target_dir
                .as_ref()
                .map(|path| path.join("release").join(native_ait_cli_binary_name()))
                .filter(|path| path.is_file())
                .map(|path| path_string(&path))
        })
        .unwrap_or_else(|| "ait-cli".to_string());
    if args.is_empty() {
        args = TG1_NATIVE_DEFAULT_ARGS
            .iter()
            .map(|value| value.to_string())
            .collect();
    }
    Ok((program, args))
}

fn native_ait_cli_from_env(env: &JsonMap<String, JsonValue>) -> Option<String> {
    for key in [
        "AIT_NATIVE_AIT_CLI_BIN",
        "AIT_RUST_AIT_CLI_BIN",
        "AIT_CLI_NATIVE_BIN",
    ] {
        if let Some(value) = optional_text(env, key) {
            return Some(value);
        }
    }
    None
}

fn native_ait_cli_binary_name() -> &'static str {
    if cfg!(windows) {
        "ait-cli.exe"
    } else {
        "ait-cli"
    }
}

fn reject_python_tg1_runner(program: &str) -> Result<(), String> {
    let name = Path::new(program)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(program)
        .trim()
        .to_ascii_lowercase();
    if name == "pytest" || name.starts_with("python") {
        return Err(
            "server_tg1_required requires a native Rust runner; pytest/python programs are forbidden."
                .to_string(),
        );
    }
    Ok(())
}

fn tg1_runner_env(
    config: &PatchsetCiRuntimeConfig,
    runner: &JsonMap<String, JsonValue>,
    source_repo_root: &Path,
    target_seed_root: &Path,
    normalized_node_ids: &[String],
    admitted_cpu_tokens: i64,
) -> JsonValue {
    let mut env = JsonMap::new();
    for (key, value) in &config.env {
        if let Some(text) = value.as_str() {
            env.insert(key.clone(), json!(text));
        }
    }
    for source in [&config.tg1, runner] {
        if let Some(raw) = source.get("env").and_then(JsonValue::as_object) {
            for (key, value) in raw {
                if let Some(text) = value.as_str() {
                    env.insert(key.clone(), json!(text));
                }
            }
        }
    }
    env.remove("PYTHONPATH");
    env.insert(
        "AIT_TG1_CATALOG_ROOT".to_string(),
        json!(path_string(source_repo_root)),
    );
    env.insert(
        "AIT_TG1_TARGET_ROOT".to_string(),
        json!(path_string(target_seed_root)),
    );
    env.insert("AIT_TG1_NATIVE_RUNNER".to_string(), json!("1"));
    env.insert("AIT_TG1_RUNNER_AUTHORITY".to_string(), json!("rust"));
    env.insert(
        "AIT_TG1_THREAD_POOL_SHARDS".to_string(),
        json!(admitted_cpu_tokens.max(1).to_string()),
    );
    env.insert(
        "AIT_TG1_CASE_COUNT".to_string(),
        json!(normalized_node_ids.len().to_string()),
    );
    if let Some(path) = &config.shared_cargo_target_dir {
        let text = path_string(path);
        env.insert("CARGO_TARGET_DIR".to_string(), json!(text.clone()));
        env.insert("AIT_SHARED_CARGO_TARGET_DIR".to_string(), json!(text));
    }
    if let Some(path) = &config.shared_cargo_build_dir {
        let text = path_string(path);
        env.insert("CARGO_BUILD_BUILD_DIR".to_string(), json!(text.clone()));
        env.insert("AIT_SHARED_CARGO_BUILD_DIR".to_string(), json!(text));
    }
    enforce_tg1_native_runtime_env(&mut env);
    JsonValue::Object(env)
}

fn enforce_tg1_native_runtime_env(env: &mut JsonMap<String, JsonValue>) {
    env.insert("AIT_PATCHSET_CI_PREWARMED".to_string(), json!("1"));
    env.insert(
        "AIT_PATCHSET_CI_PREWARM_POLICY".to_string(),
        json!("once_per_run"),
    );
    env.insert(
        "AIT_PATCHSET_CI_CARGO_CACHE_MODE".to_string(),
        json!("prewarmed_readonly"),
    );
    env.insert("AIT_RUST_PREWARM_COMPACT".to_string(), json!("0"));
    env.insert("CARGO_BUILD_JOBS".to_string(), json!("1"));
}
