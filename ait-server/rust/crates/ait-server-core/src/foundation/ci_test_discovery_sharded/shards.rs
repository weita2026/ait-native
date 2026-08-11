use serde_json::{json, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;

use super::cargo::TestExecutable;
use super::config::DiscoveryShardedConfig;
use super::paths::{duration_seconds, path_string};
use super::process::{command_env, run_process, run_process_with_output, EnvMode, ProcessContext};

#[derive(Debug, Clone)]
struct ListedTestCase {
    name: String,
    kind: String,
}

#[derive(Debug, Clone)]
pub(super) enum TestRunUnit {
    TestCase {
        index: usize,
        executable: TestExecutable,
        name: String,
        kind: String,
    },
    ExecutableFallback {
        index: usize,
        executable: TestExecutable,
        fallback_reason: String,
    },
}

impl TestRunUnit {
    pub(super) fn set_index(&mut self, index: usize) {
        match self {
            Self::TestCase {
                index: unit_index, ..
            }
            | Self::ExecutableFallback {
                index: unit_index, ..
            } => *unit_index = index,
        }
    }

    pub(super) fn executable(&self) -> &TestExecutable {
        match self {
            Self::TestCase { executable, .. } | Self::ExecutableFallback { executable, .. } => {
                executable
            }
        }
    }

    pub(super) fn is_test_case(&self) -> bool {
        matches!(self, Self::TestCase { .. })
    }

    pub(super) fn is_executable_fallback(&self) -> bool {
        matches!(self, Self::ExecutableFallback { .. })
    }

    pub(super) fn to_json(&self) -> JsonValue {
        match self {
            Self::TestCase {
                index,
                executable,
                name,
                kind,
            } => json!({
                "index": index,
                "unit_kind": "test_case",
                "name": name,
                "kind": kind,
                "executable": executable.to_json(),
            }),
            Self::ExecutableFallback {
                index,
                executable,
                fallback_reason,
            } => json!({
                "index": index,
                "unit_kind": "test_executable_fallback",
                "fallback_reason": fallback_reason,
                "executable": executable.to_json(),
            }),
        }
    }
}
#[derive(Debug)]
struct ShardCompletion {
    shard_id: String,
    index: usize,
    result: JsonValue,
}
#[derive(Debug)]
pub(super) struct TestRunUnitDiscoveryReport {
    pub(super) duration_seconds: f64,
    pub(super) reports: Vec<JsonValue>,
    pub(super) units: Vec<TestRunUnit>,
    pub(super) excluded_test_cases: Vec<String>,
}

impl TestRunUnitDiscoveryReport {
    pub(super) fn to_json(&self) -> JsonValue {
        json!({
            "contract": "ait.server.ci_test_case_discovery.v1",
            "status": if self.units.is_empty() { "fail" } else { "pass" },
            "duration_seconds": self.duration_seconds,
            "test_case_count": self.units.iter().filter(|unit| unit.is_test_case()).count(),
            "fallback_executable_count": self.units.iter().filter(|unit| unit.is_executable_fallback()).count(),
            "excluded_test_case_count": self.excluded_test_cases.len(),
            "excluded_test_cases": self.excluded_test_cases,
            "unit_count": self.units.len(),
            "reports": self.reports,
            "units": self.units.iter().map(TestRunUnit::to_json).collect::<Vec<_>>(),
        })
    }
}

pub(super) fn discover_test_run_units(
    config: &DiscoveryShardedConfig,
    executables: &[TestExecutable],
) -> Result<TestRunUnitDiscoveryReport, String> {
    let started = Instant::now();
    let output_dir = config.output_dir.join("test_case_discovery");
    fs::create_dir_all(&output_dir).map_err(|exc| {
        format!(
            "Failed to create test case discovery output dir `{}`: {exc}",
            path_string(&output_dir)
        )
    })?;
    let mut reports = Vec::new();
    let mut units = Vec::new();
    let mut excluded_test_cases = Vec::new();

    for executable in executables {
        let mut env = command_env(config, EnvMode::TestList);
        env.extend(executable.cargo_bin_exe_env.clone());
        let args = vec![
            "--list".to_string(),
            "--format".to_string(),
            "terse".to_string(),
        ];
        let run = run_process_with_output(
            "test_list",
            executable.index,
            &path_string(&executable.path),
            &args,
            &config.adapter_working_dir(),
            ProcessContext {
                output_dir: &output_dir,
                env: &env,
                timeout_seconds: config.timeout_seconds,
            },
        )?;
        let mut report = run.report.to_json();
        report["executable"] = executable.to_json();
        if run.report.status != "pass" {
            report["fallback_reason"] = json!("test_list_failed");
            units.push(TestRunUnit::ExecutableFallback {
                index: 0,
                executable: executable.clone(),
                fallback_reason: "test_list_failed".to_string(),
            });
            reports.push(report);
            continue;
        }

        let test_cases = parse_libtest_listed_test_cases(&run.stdout);
        if test_cases.is_empty() {
            report["fallback_reason"] = json!("no_listed_test_cases");
            units.push(TestRunUnit::ExecutableFallback {
                index: 0,
                executable: executable.clone(),
                fallback_reason: "no_listed_test_cases".to_string(),
            });
        } else {
            let (included, excluded): (Vec<_>, Vec<_>) = test_cases
                .into_iter()
                .partition(|test_case| !config.exclude_test_cases.contains(&test_case.name));
            report["test_case_count"] = json!(included.len());
            report["excluded_test_case_count"] = json!(excluded.len());
            report["test_cases"] = json!(included
                .iter()
                .map(|test_case| json!({
                    "name": test_case.name,
                    "kind": test_case.kind,
                }))
                .collect::<Vec<_>>());
            report["excluded_test_cases"] = json!(excluded
                .iter()
                .map(|test_case| json!({
                    "name": test_case.name,
                    "kind": test_case.kind,
                }))
                .collect::<Vec<_>>());
            excluded_test_cases.extend(excluded.into_iter().map(|test_case| test_case.name));
            for test_case in included {
                units.push(TestRunUnit::TestCase {
                    index: 0,
                    executable: executable.clone(),
                    name: test_case.name,
                    kind: test_case.kind,
                });
            }
        }
        reports.push(report);
    }

    let excluded = excluded_test_cases
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let unmatched_exclusions = config
        .exclude_test_cases
        .iter()
        .filter(|name| !excluded.contains(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unmatched_exclusions.is_empty() {
        return Err(format!(
            "Configured exclude_test_cases did not match discovered tests: {}",
            unmatched_exclusions.join(", ")
        ));
    }

    for (index, unit) in units.iter_mut().enumerate() {
        unit.set_index(index + 1);
    }

    Ok(TestRunUnitDiscoveryReport {
        duration_seconds: duration_seconds(started),
        reports,
        units,
        excluded_test_cases,
    })
}

fn parse_libtest_listed_test_cases(stdout: &str) -> Vec<ListedTestCase> {
    let mut seen = BTreeSet::new();
    let mut cases = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        let Some(name) = line.strip_suffix(": test") else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || !seen.insert(name.to_string()) {
            continue;
        }
        cases.push(ListedTestCase {
            name: name.to_string(),
            kind: "test".to_string(),
        });
    }
    cases
}

pub(super) fn run_test_case_shards(
    config: &DiscoveryShardedConfig,
    units: &[TestRunUnit],
) -> Result<JsonValue, String> {
    if units.is_empty() {
        return Ok(json!({
            "contract": "ait.server.ci_test_case_shards.v1",
            "status": "fail",
            "failure": {
                "stage": "test_shards",
                "message": "No discovered test cases or executable fallbacks were available to shard."
            },
            "shard_count": 0,
            "shards": []
        }));
    }
    let started = Instant::now();
    let shard_count = config.shard_count(units.len());
    let mut shards = (0..shard_count)
        .map(|_| Vec::<TestRunUnit>::with_capacity(units.len() / shard_count + 1))
        .collect::<Vec<_>>();
    for (index, unit) in units.iter().cloned().enumerate() {
        shards[index % shard_count].push(unit);
    }

    let config = Arc::new(config.clone());
    let (tx, rx) = mpsc::channel::<ShardCompletion>();
    for (index, shard_units) in shards.into_iter().enumerate() {
        let tx = tx.clone();
        let config = Arc::clone(&config);
        thread::spawn(move || {
            let shard_id = format!("shard-{index}");
            let result = run_one_test_case_shard(&config, index, &shard_id, shard_units)
                .unwrap_or_else(|message| {
                    json!({
                        "shard_id": shard_id,
                        "index": index,
                        "status": "fail",
                        "failure": {
                            "stage": "test_shard",
                            "message": message,
                        }
                    })
                });
            let _ = tx.send(ShardCompletion {
                shard_id: format!("shard-{index}"),
                index,
                result,
            });
        });
    }
    drop(tx);

    let mut completed = Vec::new();
    for completion in rx {
        completed.push(completion);
    }
    completed.sort_by(|left, right| left.index.cmp(&right.index));
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
            .unwrap_or_else(|| json!({"stage": "test_shards"}))
    };
    Ok(json!({
        "contract": "ait.server.ci_test_case_shards.v1",
        "status": status,
        "duration_seconds": duration_seconds(started),
        "shard_count": shard_count,
        "distribution": "stable_round_robin_by_test_case",
        "shard_by": "test_case",
        "test_case_count": units.iter().filter(|unit| unit.is_test_case()).count(),
        "fallback_executable_count": units.iter().filter(|unit| unit.is_executable_fallback()).count(),
        "unit_count": units.len(),
        "shards": shard_values,
        "failure": failure,
    }))
}

#[derive(Debug)]
struct ExecutableRunGroup {
    executable: TestExecutable,
    test_case_names: Vec<String>,
    fallback_reason: Option<String>,
}

fn group_run_units_by_executable(units: &[TestRunUnit]) -> Vec<ExecutableRunGroup> {
    let mut groups = BTreeMap::<String, ExecutableRunGroup>::new();
    for unit in units {
        let key = path_string(&unit.executable().path);
        let group = groups.entry(key).or_insert_with(|| ExecutableRunGroup {
            executable: unit.executable().clone(),
            test_case_names: Vec::new(),
            fallback_reason: None,
        });
        match unit {
            TestRunUnit::TestCase { name, .. } => group.test_case_names.push(name.clone()),
            TestRunUnit::ExecutableFallback {
                fallback_reason, ..
            } => group.fallback_reason = Some(fallback_reason.clone()),
        }
    }
    groups.into_values().collect()
}

fn unique_executables_json(units: &[TestRunUnit]) -> Vec<JsonValue> {
    let mut values = BTreeMap::<String, JsonValue>::new();
    for unit in units {
        values
            .entry(path_string(&unit.executable().path))
            .or_insert_with(|| unit.executable().to_json());
    }
    values.into_values().collect()
}

fn run_one_test_case_shard(
    config: &DiscoveryShardedConfig,
    index: usize,
    shard_id: &str,
    units: Vec<TestRunUnit>,
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let output_dir = config.output_dir.join(shard_id);
    fs::create_dir_all(&output_dir).map_err(|exc| {
        format!(
            "Failed to create test shard output dir `{}`: {exc}",
            path_string(&output_dir)
        )
    })?;
    let mut reports = Vec::new();
    let mut status = "pass";
    let mut failure = JsonValue::Null;
    let groups = group_run_units_by_executable(&units);
    for (offset, group) in groups.iter().enumerate() {
        let mut env = command_env(
            config,
            EnvMode::TestShard {
                shard_id,
                shard_index: index,
            },
        );
        env.extend(group.executable.cargo_bin_exe_env.clone());
        let (phase, args, failure_stage) = if let Some(reason) = &group.fallback_reason {
            let _ = reason;
            ("test_executable", Vec::new(), "test_executable")
        } else {
            let mut args = vec!["--exact".to_string()];
            args.extend(group.test_case_names.iter().cloned());
            ("test_case", args, "test_case")
        };
        let report = run_process(
            phase,
            offset + 1,
            &path_string(&group.executable.path),
            &args,
            &config.adapter_working_dir(),
            ProcessContext {
                output_dir: &output_dir,
                env: &env,
                timeout_seconds: config.timeout_seconds,
            },
        )?;
        let mut report_json = report.to_json();
        report_json["executable"] = group.executable.to_json();
        report_json["test_case_count"] = json!(group.test_case_names.len());
        report_json["test_cases"] = json!(group.test_case_names);
        if let Some(reason) = &group.fallback_reason {
            report_json["fallback_reason"] = json!(reason);
        }
        if report.status != "pass" {
            status = "fail";
            failure = report.failure_json(failure_stage);
            reports.push(report_json);
            break;
        }
        reports.push(report_json);
    }
    Ok(json!({
        "shard_id": shard_id,
        "index": index,
        "status": status,
        "duration_seconds": duration_seconds(started),
        "unit_count": units.len(),
        "test_case_count": units.iter().filter(|unit| unit.is_test_case()).count(),
        "fallback_executable_count": units.iter().filter(|unit| unit.is_executable_fallback()).count(),
        "executable_count": groups.len(),
        "executables": unique_executables_json(&units),
        "units": units.iter().map(TestRunUnit::to_json).collect::<Vec<_>>(),
        "reports": reports,
        "failure": failure,
    }))
}
