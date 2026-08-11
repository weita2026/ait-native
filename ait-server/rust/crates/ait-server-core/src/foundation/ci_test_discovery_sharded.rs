use serde_json::{json, Value as JsonValue};
use std::fs;
use std::time::Instant;

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::env;
#[cfg(test)]
use std::path::{Path, PathBuf};

mod artifacts;
mod cargo;
mod command;
mod config;
mod paths;
mod process;
mod shards;

use artifacts::{report_passed, write_artifacts};
#[cfg(test)]
use cargo::cargo_build_relevant_path;
use cargo::{run_cargo_discovery_build, run_cargo_doc_tests, TestExecutable};
use command::run_command_adapter;
use config::{run_checks, DiscoveryShardedConfig};
use paths::{duration_seconds, path_string};
#[cfg(test)]
use process::resolve_ci_process_program;
use shards::{discover_test_run_units, run_test_case_shards, TestRunUnit};

use crate::foundation::ci_process_env::ci_process_environment_report;

const CONTRACT: &str = "ait.server.ci_test_discovery_sharded_run.v1";

pub fn ci_test_discovery_sharded_run_json(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request.as_object().ok_or_else(|| {
        "ci-test-discovery-sharded-run payload must be a JSON object.".to_string()
    })?;
    let config = DiscoveryShardedConfig::from_request(request)?;
    fs::create_dir_all(&config.output_dir).map_err(|exc| {
        format!(
            "Failed to create CI test discovery output dir `{}`: {exc}",
            path_string(&config.output_dir)
        )
    })?;

    let started = Instant::now();
    let checks_started = Instant::now();
    let check_reports = run_checks(&config)?;
    let checks_duration_seconds = duration_seconds(checks_started);
    if config.adapter == "command" {
        return run_command_adapter(&config, check_reports, checks_duration_seconds, started);
    }
    let checks_pass = check_reports.iter().all(report_passed);
    let mut build_report = JsonValue::Null;
    let mut discovered = Vec::<TestExecutable>::new();
    let mut test_run_units = Vec::<TestRunUnit>::new();
    let mut test_case_discovery_report = JsonValue::Null;
    let mut shard_summary = JsonValue::Null;
    let mut doc_test_report = JsonValue::Null;
    let mut failure = JsonValue::Null;
    let mut status = if checks_pass { "pass" } else { "fail" };

    let mut discovery_build_duration_seconds = JsonValue::Null;
    if status == "pass" {
        let build = run_cargo_discovery_build(&config)?;
        if build.status != "pass" {
            status = "fail";
            failure = build.failure_json("discover_build");
        } else {
            discovered = build.executables.clone();
        }
        discovery_build_duration_seconds = json!(build.process.duration_seconds);
        build_report = build.to_json();
    }

    if status == "pass" {
        let test_case_discovery = discover_test_run_units(&config, &discovered)?;
        test_run_units = test_case_discovery.units.clone();
        test_case_discovery_report = test_case_discovery.to_json();
        if test_run_units.is_empty() {
            status = "fail";
            failure = json!({
                "stage": "test_case_discovery",
                "message": "No discovered test cases or executable fallbacks were available to shard."
            });
        }
    }

    let mut test_shards_duration_seconds = JsonValue::Null;
    if status == "pass" {
        let shards = run_test_case_shards(&config, &test_run_units)?;
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
                .unwrap_or_else(|| json!({"stage": "test_shards"}));
        }
        test_shards_duration_seconds = shards
            .get("duration_seconds")
            .cloned()
            .unwrap_or(JsonValue::Null);
        shard_summary = shards;
    }

    let mut doc_tests_duration_seconds = JsonValue::Null;
    if status == "pass" && config.doc_tests {
        let report = run_cargo_doc_tests(&config)?;
        if report.status != "pass" {
            status = "fail";
            failure = report.failure_json("doc_tests");
        }
        doc_tests_duration_seconds = json!(report.duration_seconds);
        doc_test_report = report.to_json();
    } else if config.doc_tests {
        doc_test_report = json!({
            "status": "skipped",
            "reason": "previous_phase_failed"
        });
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
            "adapter": config.adapter,
            "shard_by": "test_case",
            "discovery_phase": "build_once",
            "run_phase": "direct_test_case_shards",
            "doc_tests": config.doc_tests,
            "timeout_seconds": config.timeout_seconds,
            "language_adapter_contract": "ait.server.ci_language_adapter.v1"
        },
        "checks": {
            "status": if checks_pass { "pass" } else { "fail" },
            "reports": check_reports
        },
        "discovery": {
            "status": build_report.get("status").cloned().unwrap_or(JsonValue::Null),
            "executable_count": discovered.len(),
            "executables": discovered.iter().map(TestExecutable::to_json).collect::<Vec<_>>(),
            "test_case_count": test_run_units.iter().filter(|unit| unit.is_test_case()).count(),
            "fallback_executable_count": test_run_units.iter().filter(|unit| unit.is_executable_fallback()).count(),
            "excluded_test_case_count": test_case_discovery_report.get("excluded_test_case_count").cloned().unwrap_or(JsonValue::Null),
            "excluded_test_cases": test_case_discovery_report.get("excluded_test_cases").cloned().unwrap_or_else(|| json!([])),
            "test_case_discovery": test_case_discovery_report,
            "build_report": build_report
        },
        "test_shards": shard_summary,
        "doc_tests": doc_test_report,
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
            "cargo_build_jobs_env": config.runner_parallelism.map(|value| value.to_string()),
            "rust_test_threads_env": "1"
        },
        "phase_durations_seconds": {
            "checks": checks_duration_seconds,
            "snapshot_materialization": config.snapshot_materialization_duration_seconds.clone(),
            "snapshot_materialization_phases": config.snapshot_materialization_phase_durations.clone(),
            "discovery_build": discovery_build_duration_seconds,
            "test_shards": test_shards_duration_seconds,
            "doc_tests": doc_tests_duration_seconds,
        },
        "diagnostics": {
            "generic_runner_contract": true,
            "language_adapter": config.adapter,
            "cargo_compiles_once": true,
            "cargo_build_cache_policy": config.build_cache.policy.clone(),
            "test_execution_sharded": true,
            "test_case_shards": true,
            "test_executable_shards": false,
            "test_executable_fallback": true,
            "python_command_runner": false,
            "python_glue": false,
            "shell_command_bundle": false,
            "full_logs_retained": true,
            "json_output_uses_tail": true
        }
    });
    let artifacts = write_artifacts(&config, &summary)?;
    summary["artifacts"] = artifacts;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_temp_root(name: &str) -> PathBuf {
        env::temp_dir().join(format!("ait-ci-discovery-{name}-{}", std::process::id()))
    }

    fn write_executable(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("executable parent should be created");
        }
        fs::write(path, text).expect("executable should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .expect("executable should be marked executable");
        }
    }

    fn fake_cargo_script(executable_path: &Path) -> String {
        let line = serde_json::to_string(&json!({
            "reason": "compiler-artifact",
            "package_id": "path+file:///tmp/example#0.1.0",
            "target": {
                "kind": ["test"],
                "name": "example_tests",
            },
            "profile": {
                "test": true,
            },
            "executable": path_string(executable_path),
        }))
        .expect("fake cargo json should serialize");
        format!("#!/bin/sh\ncat <<'JSON'\n{line}\nJSON\n")
    }

    fn fake_cargo_script_with_binary(
        test_executable_path: &Path,
        binary_executable_path: &Path,
    ) -> String {
        let package_id = "path+file:///tmp/example#0.1.0";
        let binary_line = serde_json::to_string(&json!({
            "reason": "compiler-artifact",
            "package_id": package_id,
            "target": {
                "kind": ["bin"],
                "name": "example-cli",
            },
            "profile": {
                "test": false,
            },
            "executable": path_string(binary_executable_path),
        }))
        .expect("fake cargo binary json should serialize");
        let test_line = serde_json::to_string(&json!({
            "reason": "compiler-artifact",
            "package_id": package_id,
            "target": {
                "kind": ["test"],
                "name": "example_tests",
            },
            "profile": {
                "test": true,
            },
            "executable": path_string(test_executable_path),
        }))
        .expect("fake cargo test json should serialize");
        format!("#!/bin/sh\ncat <<'JSON'\n{binary_line}\n{test_line}\nJSON\n")
    }

    #[test]
    fn direct_test_cases_receive_cargo_binary_executable_environment() {
        let root = test_temp_root("cargo-bin-exe-env");
        let workspace = root.join("workspace");
        let output = root.join("output");
        let bin_dir = root.join("bin");
        let fake_cargo = bin_dir.join("cargo");
        let test_executable = bin_dir.join("example_tests");
        let binary_executable = bin_dir.join("example-cli");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        write_executable(&binary_executable, "#!/bin/sh\nexit 0\n");
        write_executable(
            &test_executable,
            &format!(
                "#!/bin/sh\nif ! env | grep -Fqx 'CARGO_BIN_EXE_example-cli={}'; then\n  echo missing-cargo-bin-exe >&2\n  exit 41\nfi\nif [ \"${{1:-}}\" = \"--list\" ]; then\n  echo 'cargo_binary_environment_is_available: test'\nfi\n",
                path_string(&binary_executable)
            ),
        );
        write_executable(
            &fake_cargo,
            &fake_cargo_script_with_binary(&test_executable, &binary_executable),
        );

        let result = ci_test_discovery_sharded_run_json(&json!({
            "workspace_path": path_string(&workspace),
            "output_dir": path_string(&output),
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "cargo",
                "cargo_binary": path_string(&fake_cargo),
                "manifest_path": "Cargo.toml"
            }
        }))
        .expect("direct test case runner should preserve Cargo binary environment");

        assert_eq!(result["status"], json!("pass"));
        assert_eq!(
            result["discovery"]["executables"][0]["cargo_bin_exe_env"]["CARGO_BIN_EXE_example-cli"],
            json!(path_string(&binary_executable))
        );
        fs::remove_dir_all(root).expect("test root should be removed");
    }

    #[test]
    fn resolves_ci_process_program_from_runner_path() {
        let root = env::temp_dir().join(format!("ait-ci-process-path-{}", std::process::id()));
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).expect("test bin dir should be created");
        let cargo_path = bin_dir.join("cargo");
        fs::write(&cargo_path, "#!/bin/sh\nexit 0\n").expect("fake cargo should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&cargo_path, fs::Permissions::from_mode(0o755))
                .expect("fake cargo should be executable");
        }
        let env_map = BTreeMap::from([("PATH".to_string(), path_string(&bin_dir))]);

        let resolved = resolve_ci_process_program("cargo", &env_map)
            .expect("cargo should resolve through runner PATH");

        assert_eq!(resolved, path_string(&cargo_path));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_ci_process_program_error_names_runner_path() {
        let env_map = BTreeMap::from([("PATH".to_string(), String::new())]);

        let error = resolve_ci_process_program("cargo", &env_map)
            .expect_err("missing cargo should produce a diagnostic error");

        assert!(error.contains("executable `cargo` was not found in PATH"));
        assert!(error.contains("PATH=<empty>"));
    }

    #[test]
    fn reuses_cached_test_executables_when_rust_inputs_are_unchanged() {
        let root = test_temp_root("cache-hit");
        let workspace = root.join("workspace");
        let output_a = root.join("output-a");
        let output_b = root.join("output-b");
        let bin_dir = root.join("bin");
        let fake_cargo = bin_dir.join("cargo");
        let test_executable = bin_dir.join("example_tests");
        let executable_manifest = root.join("cache").join("executables.json");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        write_executable(&test_executable, "#!/bin/sh\nexit 0\n");
        write_executable(&fake_cargo, &fake_cargo_script(&test_executable));

        let first = ci_test_discovery_sharded_run_json(&json!({
            "workspace_path": path_string(&workspace),
            "output_dir": path_string(&output_a),
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "cargo",
                "cargo_binary": path_string(&fake_cargo),
                "manifest_path": "Cargo.toml"
            },
            "build_cache": {
                "policy": "reuse_when_rust_inputs_unchanged",
                "executable_manifest_path": path_string(&executable_manifest),
                "changed_paths": ["rust/src/lib.rs"]
            }
        }))
        .expect("first run should build and write executable manifest");
        assert_eq!(first["status"], json!("pass"));
        assert!(executable_manifest.is_file());

        write_executable(&fake_cargo, "#!/bin/sh\nexit 99\n");
        let second = ci_test_discovery_sharded_run_json(&json!({
            "workspace_path": path_string(&workspace),
            "output_dir": path_string(&output_b),
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "cargo",
                "cargo_binary": path_string(&fake_cargo),
                "manifest_path": "Cargo.toml"
            },
            "build_cache": {
                "policy": "reuse_when_rust_inputs_unchanged",
                "executable_manifest_path": path_string(&executable_manifest),
                "changed_paths": ["README.md"]
            }
        }))
        .expect("second run should reuse executable manifest");

        fs::remove_dir_all(root).expect("test root should be removed");

        assert_eq!(second["status"], json!("pass"));
        assert_eq!(
            second["discovery"]["build_report"]["command"],
            json!(format!(
                "reuse_cargo_test_executable_manifest {}",
                path_string(&executable_manifest)
            ))
        );
        assert_eq!(second["discovery"]["executable_count"], json!(1));
    }

    #[test]
    fn rust_input_changes_reject_cached_test_executables() {
        let root = test_temp_root("cache-miss");
        let workspace = root.join("workspace");
        let rust_input = workspace.join("rust/src/lib.rs");
        let output_a = root.join("output-a");
        let output_b = root.join("output-b");
        let bin_dir = root.join("bin");
        let fake_cargo = bin_dir.join("cargo");
        let test_executable = bin_dir.join("example_tests");
        let executable_manifest = root.join("cache").join("executables.json");
        fs::create_dir_all(
            rust_input
                .parent()
                .expect("Rust input should have a parent directory"),
        )
        .expect("workspace should be created");
        fs::write(&rust_input, "pub fn changed() {}\n").expect("Rust input should be written");
        write_executable(&test_executable, "#!/bin/sh\nexit 0\n");
        write_executable(&fake_cargo, &fake_cargo_script(&test_executable));

        let first = ci_test_discovery_sharded_run_json(&json!({
            "workspace_path": path_string(&workspace),
            "output_dir": path_string(&output_a),
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "cargo",
                "cargo_binary": path_string(&fake_cargo),
                "manifest_path": "Cargo.toml"
            },
            "build_cache": {
                "policy": "reuse_when_rust_inputs_unchanged",
                "executable_manifest_path": path_string(&executable_manifest),
                "changed_paths": ["rust/src/lib.rs"]
            }
        }))
        .expect("first run should build and write executable manifest");
        assert_eq!(first["status"], json!("pass"));

        let stale_modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1);
        fs::File::open(&rust_input)
            .and_then(|file| file.set_times(std::fs::FileTimes::new().set_modified(stale_modified)))
            .expect("Rust input should receive a stale materialized mtime");
        write_executable(&fake_cargo, "#!/bin/sh\nexit 99\n");
        let second = ci_test_discovery_sharded_run_json(&json!({
            "workspace_path": path_string(&workspace),
            "output_dir": path_string(&output_b),
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "cargo",
                "cargo_binary": path_string(&fake_cargo),
                "manifest_path": "Cargo.toml"
            },
            "build_cache": {
                "policy": "reuse_when_rust_inputs_unchanged",
                "executable_manifest_path": path_string(&executable_manifest),
                "changed_paths": ["rust/src/lib.rs"]
            }
        }))
        .expect("second run should attempt a real build after Rust changes");

        let refreshed_modified = fs::metadata(&rust_input)
            .and_then(|metadata| metadata.modified())
            .expect("refreshed Rust input mtime should be readable");
        fs::remove_dir_all(root).expect("test root should be removed");

        assert_eq!(second["status"], json!("fail"));
        assert!(
            refreshed_modified > stale_modified,
            "Rust inputs copied with stale mtimes must be refreshed before Cargo runs"
        );
        assert!(second["discovery"]["build_report"]["command"]
            .as_str()
            .is_some_and(|command| command.starts_with(&path_string(&fake_cargo))));
    }

    #[test]
    fn missing_changed_paths_reject_cached_test_executables() {
        let root = test_temp_root("cache-miss-unknown-changes");
        let workspace = root.join("workspace");
        let output_a = root.join("output-a");
        let output_b = root.join("output-b");
        let bin_dir = root.join("bin");
        let fake_cargo = bin_dir.join("cargo");
        let test_executable = bin_dir.join("example_tests");
        let executable_manifest = root.join("cache").join("executables.json");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        write_executable(&test_executable, "#!/bin/sh\nexit 0\n");
        write_executable(&fake_cargo, &fake_cargo_script(&test_executable));

        let first = ci_test_discovery_sharded_run_json(&json!({
            "workspace_path": path_string(&workspace),
            "output_dir": path_string(&output_a),
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "cargo",
                "cargo_binary": path_string(&fake_cargo),
                "manifest_path": "Cargo.toml"
            },
            "build_cache": {
                "policy": "reuse_when_rust_inputs_unchanged",
                "executable_manifest_path": path_string(&executable_manifest),
                "changed_paths": ["rust/src/lib.rs"]
            }
        }))
        .expect("first run should build and write executable manifest");
        assert_eq!(first["status"], json!("pass"));

        write_executable(&fake_cargo, "#!/bin/sh\nexit 99\n");
        let second = ci_test_discovery_sharded_run_json(&json!({
            "workspace_path": path_string(&workspace),
            "output_dir": path_string(&output_b),
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "cargo",
                "cargo_binary": path_string(&fake_cargo),
                "manifest_path": "Cargo.toml"
            },
            "build_cache": {
                "policy": "reuse_when_rust_inputs_unchanged",
                "executable_manifest_path": path_string(&executable_manifest)
            }
        }))
        .expect("second run should attempt a real build when changed paths are unknown");

        fs::remove_dir_all(root).expect("test root should be removed");

        assert_eq!(second["status"], json!("fail"));
        assert!(second["discovery"]["build_report"]["command"]
            .as_str()
            .is_some_and(|command| command.starts_with(&path_string(&fake_cargo))));
    }

    #[test]
    fn cargo_build_relevant_path_detects_rust_and_manifest_inputs() {
        assert!(cargo_build_relevant_path(
            "rust/crates/ait-server/src/lib.rs"
        ));
        assert!(cargo_build_relevant_path("rust/Cargo.lock"));
        assert!(cargo_build_relevant_path("crates/example/build.rs"));
        assert!(cargo_build_relevant_path(".cargo/config.toml"));
        assert!(!cargo_build_relevant_path("README.md"));
        assert!(!cargo_build_relevant_path("docs/plan.md"));
    }
}
