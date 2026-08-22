use serde_json::{json, Value as JsonValue};
use std::collections::BTreeSet;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;

use super::artifacts::{report_passed, write_artifacts};
use super::config::{CommandAdapterConfig, DiscoveryShardedConfig};
use super::paths::{duration_seconds, path_string};
use super::process::{command_env, run_process, run_process_with_output, EnvMode, ProcessContext};
use crate::foundation::ci_process_env::ci_process_environment_report;

const CONTRACT: &str = "ait.server.ci_test_discovery_sharded_run.v1";
const COMMAND_DISCOVERY_CONTRACT: &str = "ait.server.ci_command_test_discovery.v1";
const COMMAND_SHARDS_CONTRACT: &str = "ait.server.ci_command_test_case_shards.v1";
const MAX_COMMAND_DISCOVERED_TEST_CASES: usize = 100_000;
const MAX_COMMAND_TEST_IDENTIFIER_BYTES: usize = 4 * 1024;
const MAX_COMMAND_TEST_INVENTORY_BYTES: usize = 128 * 1024;

struct CommandDiscoveryInventory {
    test_cases: Vec<String>,
    excluded_test_cases: Vec<String>,
}

struct CommandShardCompletion {
    shard_id: String,
    index: usize,
    result: JsonValue,
}

pub(super) fn run_command_adapter(
    config: &DiscoveryShardedConfig,
    check_reports: Vec<JsonValue>,
    checks_duration_seconds: f64,
    started: Instant,
) -> Result<JsonValue, String> {
    let adapter = config
        .command_adapter
        .as_ref()
        .ok_or_else(|| "Command adapter configuration is missing.".to_string())?;
    let checks_pass = check_reports.iter().all(report_passed);
    let mut status = if checks_pass { "pass" } else { "fail" };
    let mut failure = if checks_pass {
        JsonValue::Null
    } else {
        json!({
            "stage": "checks",
            "message": "One or more pre-discovery checks failed."
        })
    };
    let mut discovery = JsonValue::Null;
    let mut test_cases = Vec::new();
    let mut discovery_duration_seconds = JsonValue::Null;

    if checks_pass {
        let discovery_started = Instant::now();
        let output_dir = config.output_dir.join("command_discovery");
        let env = command_env(config, EnvMode::TestList);
        let run = run_process_with_output(
            "command_discovery",
            1,
            &adapter.discovery_program,
            &adapter.discovery_args,
            &config.adapter_working_dir(),
            ProcessContext {
                output_dir: &output_dir,
                env: &env,
                timeout_seconds: config.timeout_seconds,
            },
        )?;
        discovery_duration_seconds = json!(duration_seconds(discovery_started));
        let mut report = run.report.to_json();
        report["output_format"] = json!(adapter.discovery_output_format);
        let mut discovery_status = run.report.status;
        let mut discovery_failure = JsonValue::Null;
        let mut excluded_test_cases = Vec::new();

        if run.report.status != "pass" {
            status = "fail";
            discovery_failure = run.report.failure_json("command_discovery");
            failure = discovery_failure.clone();
        } else {
            match parse_command_discovery(
                &run.stdout,
                &adapter.discovery_output_format,
                &config.exclude_test_cases,
            ) {
                Ok(inventory) => {
                    test_cases = inventory.test_cases;
                    excluded_test_cases = inventory.excluded_test_cases;
                    report["test_case_count"] = json!(test_cases.len());
                    report["excluded_test_case_count"] = json!(excluded_test_cases.len());
                }
                Err(message) => {
                    status = "fail";
                    discovery_status = "fail";
                    discovery_failure = json!({
                        "stage": "command_discovery_output",
                        "message": message,
                        "log_path": path_string(&run.report.log_path),
                    });
                    failure = discovery_failure.clone();
                }
            }
        }

        discovery = json!({
            "status": discovery_status,
            "executable_count": 0,
            "executables": [],
            "test_case_count": test_cases.len(),
            "fallback_executable_count": 0,
            "excluded_test_case_count": excluded_test_cases.len(),
            "excluded_test_cases": excluded_test_cases,
            "test_case_discovery": {
                "contract": COMMAND_DISCOVERY_CONTRACT,
                "status": discovery_status,
                "duration_seconds": discovery_duration_seconds,
                "output_format": adapter.discovery_output_format,
                "maximum_test_case_count": MAX_COMMAND_DISCOVERED_TEST_CASES,
                "maximum_test_identifier_bytes": MAX_COMMAND_TEST_IDENTIFIER_BYTES,
                "maximum_test_inventory_bytes": MAX_COMMAND_TEST_INVENTORY_BYTES,
                "test_case_count": test_cases.len(),
                "fallback_executable_count": 0,
                "excluded_test_case_count": excluded_test_cases.len(),
                "excluded_test_cases": excluded_test_cases,
                "unit_count": test_cases.len(),
                "reports": [report],
                "units": test_cases.iter().enumerate().map(|(index, test_case)| json!({
                    "index": index + 1,
                    "unit_kind": "test_case",
                    "name": test_case,
                    "kind": "command",
                })).collect::<Vec<_>>(),
                "failure": discovery_failure,
            },
            "build_report": JsonValue::Null,
        });
    }

    let mut shard_summary = JsonValue::Null;
    let mut test_shards_duration_seconds = JsonValue::Null;
    if status == "pass" {
        let shards = run_command_test_case_shards(config, adapter, &test_cases)?;
        if shards
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or("fail")
            != "pass"
        {
            status = "fail";
            failure = shards
                .get("failure")
                .cloned()
                .unwrap_or_else(|| json!({"stage": "command_test_shards"}));
        }
        test_shards_duration_seconds = shards
            .get("duration_seconds")
            .cloned()
            .unwrap_or(JsonValue::Null);
        shard_summary = shards;
    }

    let mut summary = json!({
        "contract": CONTRACT,
        "status": status,
        "duration_seconds": duration_seconds(started),
        "suite_id": config.suite_id,
        "job_type": config.job_type,
        "job_id": config.job_id,
        "workspace_path": path_string(&config.workspace_path),
        "output_dir": path_string(&config.output_dir),
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "command",
            "shard_by": "test_case",
            "discovery_phase": "command_once",
            "run_phase": "command_test_case_shards",
            "discovery_output_format": adapter.discovery_output_format,
            "append_test_items": adapter.append_test_items,
            "working_directory": if adapter.working_directory.as_os_str().is_empty() {
                ".".to_string()
            } else {
                path_string(&adapter.working_directory)
            },
            "doc_tests": false,
            "timeout_seconds": config.timeout_seconds,
            "language_adapter_contract": "ait.server.ci_language_adapter.v1"
        },
        "checks": {
            "status": if checks_pass { "pass" } else { "fail" },
            "reports": check_reports
        },
        "discovery": discovery,
        "test_shards": shard_summary,
        "doc_tests": JsonValue::Null,
        "failure": failure,
        "environment": {
            "process_policy": ci_process_environment_report(),
            "shared_cargo_target_dir": config.shared_cargo_target_dir.as_ref().map(|path| path_string(path)),
            "shared_cargo_build_dir": config.shared_cargo_build_dir.as_ref().map(|path| path_string(path)),
            "temp_dir": config.temp_dir.as_ref().map(|path| path_string(path)),
            "output_dir": path_string(&config.output_dir),
            "workspace_path": path_string(&config.workspace_path),
            "adapter_working_dir": path_string(&config.adapter_working_dir()),
            "runner_parallelism": config.runner_parallelism,
            "admitted_cpu_tokens": config.runner_parallelism,
            "parallelism_source": if config.runner_parallelism.is_some() { "scheduler" } else { "unspecified" },
        },
        "phase_durations_seconds": {
            "checks": checks_duration_seconds,
            "snapshot_materialization": config.snapshot_materialization_duration_seconds.clone(),
            "snapshot_materialization_phases": config.snapshot_materialization_phase_durations.clone(),
            "discovery_build": JsonValue::Null,
            "command_discovery": discovery_duration_seconds,
            "test_shards": test_shards_duration_seconds,
            "doc_tests": JsonValue::Null,
        },
        "diagnostics": {
            "generic_runner_contract": true,
            "language_adapter": "command",
            "language_neutral_command_adapter": true,
            "command_discovers_once": true,
            "cargo_compiles_once": false,
            "cargo_build_cache_policy": "not_applicable",
            "test_execution_sharded": true,
            "test_case_shards": true,
            "test_executable_shards": false,
            "test_executable_fallback": false,
            "python_command_runner": false,
            "python_glue": false,
            "shell_command_bundle": false,
            "full_logs_retained": true,
            "json_output_uses_tail": true
        }
    });
    let artifacts = write_artifacts(config, &summary)?;
    summary["artifacts"] = artifacts;
    Ok(summary)
}

fn parse_command_discovery(
    stdout: &str,
    output_format: &str,
    excluded: &BTreeSet<String>,
) -> Result<CommandDiscoveryInventory, String> {
    let raw_test_cases = match output_format {
        "json_array" => {
            let value = serde_json::from_str::<JsonValue>(stdout).map_err(|exc| {
                format!("Command discovery stdout must be a JSON string array: {exc}")
            })?;
            let values = value.as_array().ok_or_else(|| {
                "Command discovery stdout must be a JSON string array.".to_string()
            })?;
            values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        "Command discovery JSON array must contain only strings.".to_string()
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        "lines" => stdout.lines().map(str::to_string).collect(),
        value => {
            return Err(format!(
                "Unsupported command discovery output format `{value}`."
            ));
        }
    };
    if raw_test_cases.len() > MAX_COMMAND_DISCOVERED_TEST_CASES {
        return Err(format!(
            "Command discovery returned {} test cases; maximum is {MAX_COMMAND_DISCOVERED_TEST_CASES}.",
            raw_test_cases.len()
        ));
    }
    if raw_test_cases.is_empty() {
        return Err("Command discovery returned no test cases.".to_string());
    }

    let mut seen = BTreeSet::new();
    let mut normalized = Vec::with_capacity(raw_test_cases.len());
    let mut inventory_bytes = 0usize;
    for (index, raw) in raw_test_cases.into_iter().enumerate() {
        let test_case = raw.trim();
        if test_case.is_empty() {
            return Err(format!(
                "Command discovery returned a blank test identifier at index {index}."
            ));
        }
        if test_case.chars().any(char::is_control) {
            return Err(format!(
                "Command discovery returned a test identifier containing control characters at index {index}."
            ));
        }
        if test_case.len() > MAX_COMMAND_TEST_IDENTIFIER_BYTES {
            return Err(format!(
                "Command discovery returned a test identifier with {} bytes; maximum is {MAX_COMMAND_TEST_IDENTIFIER_BYTES}.",
                test_case.len()
            ));
        }
        inventory_bytes = inventory_bytes.saturating_add(test_case.len() + 1);
        if inventory_bytes > MAX_COMMAND_TEST_INVENTORY_BYTES {
            return Err(format!(
                "Command discovery returned a test inventory with more than {MAX_COMMAND_TEST_INVENTORY_BYTES} normalized bytes."
            ));
        }
        if !seen.insert(test_case.to_string()) {
            return Err(format!(
                "Command discovery returned duplicate test identifier `{test_case}`."
            ));
        }
        normalized.push(test_case.to_string());
    }

    let unmatched_exclusions = excluded
        .iter()
        .filter(|name| !seen.contains(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unmatched_exclusions.is_empty() {
        return Err(format!(
            "Configured exclude_test_cases did not match discovered tests: {}",
            unmatched_exclusions.join(", ")
        ));
    }

    let mut test_cases = Vec::with_capacity(normalized.len());
    let mut excluded_test_cases = Vec::new();
    for test_case in normalized {
        if excluded.contains(&test_case) {
            excluded_test_cases.push(test_case);
        } else {
            test_cases.push(test_case);
        }
    }
    if test_cases.is_empty() {
        return Err("Command discovery left no runnable test cases after exclusions.".to_string());
    }

    Ok(CommandDiscoveryInventory {
        test_cases,
        excluded_test_cases,
    })
}

fn run_command_test_case_shards(
    config: &DiscoveryShardedConfig,
    adapter: &CommandAdapterConfig,
    test_cases: &[String],
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let shard_count = config.shard_count(test_cases.len());
    let mut shards = (0..shard_count)
        .map(|_| Vec::<String>::with_capacity(test_cases.len() / shard_count + 1))
        .collect::<Vec<_>>();
    for (index, test_case) in test_cases.iter().cloned().enumerate() {
        shards[index % shard_count].push(test_case);
    }

    let config = Arc::new(config.clone());
    let adapter = Arc::new(adapter.clone());
    let (tx, rx) = mpsc::channel::<CommandShardCompletion>();
    for (index, shard_test_cases) in shards.into_iter().enumerate() {
        let tx = tx.clone();
        let config = Arc::clone(&config);
        let adapter = Arc::clone(&adapter);
        thread::spawn(move || {
            let shard_id = format!("shard-{index}");
            let result = run_one_command_test_case_shard(
                &config,
                &adapter,
                index,
                &shard_id,
                shard_test_cases,
            )
            .unwrap_or_else(|message| {
                json!({
                    "status": "fail",
                    "failure": {
                        "stage": "command_test_shard",
                        "message": message,
                    }
                })
            });
            let _ = tx.send(CommandShardCompletion {
                shard_id,
                index,
                result,
            });
        });
    }
    drop(tx);

    let mut completed = rx.into_iter().collect::<Vec<_>>();
    completed.sort_by_key(|left| left.index);
    let shard_values = completed
        .into_iter()
        .map(|completion| {
            let mut result = completion.result;
            result["shard_id"] = json!(completion.shard_id);
            result["index"] = json!(completion.index);
            result
        })
        .collect::<Vec<_>>();
    let status = if shard_values
        .iter()
        .all(|value| value.get("status").and_then(JsonValue::as_str) == Some("pass"))
    {
        "pass"
    } else {
        "fail"
    };
    let failure = if status == "pass" {
        JsonValue::Null
    } else {
        shard_values
            .iter()
            .find_map(|value| value.get("failure").cloned())
            .unwrap_or_else(|| json!({"stage": "command_test_shards"}))
    };

    Ok(json!({
        "contract": COMMAND_SHARDS_CONTRACT,
        "status": status,
        "duration_seconds": duration_seconds(started),
        "shard_count": shard_count,
        "distribution": "stable_round_robin_by_test_case",
        "shard_by": "test_case",
        "test_case_count": test_cases.len(),
        "fallback_executable_count": 0,
        "unit_count": test_cases.len(),
        "shards": shard_values,
        "failure": failure,
    }))
}

fn run_one_command_test_case_shard(
    config: &DiscoveryShardedConfig,
    adapter: &CommandAdapterConfig,
    index: usize,
    shard_id: &str,
    test_cases: Vec<String>,
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let output_dir = config.output_dir.join("command_test_shards").join(shard_id);
    let mut env = command_env(
        config,
        EnvMode::TestShard {
            shard_id,
            shard_index: index,
        },
    );
    env.insert("AIT_SHARD_ID".to_string(), shard_id.to_string());
    env.insert(
        "AIT_SHARD_REPO_DIR".to_string(),
        path_string(&config.adapter_working_dir()),
    );
    env.insert("AIT_SHARD_OUTPUT_DIR".to_string(), path_string(&output_dir));
    env.insert("AIT_TEST_ITEMS".to_string(), test_cases.join("\n"));
    env.insert(
        "AIT_TEST_ITEMS_JSON".to_string(),
        serde_json::to_string(&test_cases).map_err(|exc| exc.to_string())?,
    );
    env.insert(
        "AIT_CI_TEST_CASE_COUNT".to_string(),
        test_cases.len().to_string(),
    );
    let mut args = adapter.run_args.clone();
    if adapter.append_test_items {
        args.extend(test_cases.iter().cloned());
    }
    let report = run_process(
        "command_test_shard",
        1,
        &adapter.run_program,
        &args,
        &config.adapter_working_dir(),
        ProcessContext {
            output_dir: &output_dir,
            env: &env,
            timeout_seconds: config.timeout_seconds,
        },
    )?;
    let status = report.status;
    let failure = if status == "pass" {
        JsonValue::Null
    } else {
        report.failure_json("command_test_shard")
    };

    Ok(json!({
        "shard_id": shard_id,
        "index": index,
        "status": status,
        "duration_seconds": duration_seconds(started),
        "unit_count": test_cases.len(),
        "test_case_count": test_cases.len(),
        "fallback_executable_count": 0,
        "executable_count": 0,
        "executables": [],
        "test_cases": test_cases,
        "units": test_cases.iter().enumerate().map(|(unit_index, test_case)| json!({
            "index": unit_index + 1,
            "unit_kind": "test_case",
            "name": test_case,
            "kind": "command",
        })).collect::<Vec<_>>(),
        "reports": [report.to_json()],
        "failure": failure,
    }))
}
