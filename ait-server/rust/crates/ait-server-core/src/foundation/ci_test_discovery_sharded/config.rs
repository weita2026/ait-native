use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use super::artifacts::artifact_path;
use super::cargo::{default_cargo_build_args, default_cargo_fmt_args};
use super::paths::{
    duration_seconds, optional_bool, optional_path, optional_positive_i64, optional_text,
    path_field, path_string, relative_path_string, string_array, string_map, string_set,
    validate_relative_path,
};
use super::process::{command_env, run_process, EnvMode, ProcessContext};
use crate::foundation::ci_process_stream::validated_ci_process_timeout_seconds;

#[derive(Debug, Clone)]
pub(super) struct DiscoveryShardedConfig {
    pub(super) workspace_path: PathBuf,
    pub(super) output_dir: PathBuf,
    pub(super) suite_id: Option<String>,
    pub(super) job_type: Option<String>,
    pub(super) job_id: Option<String>,
    pub(super) adapter: String,
    pub(super) command_adapter: Option<CommandAdapterConfig>,
    pub(super) cargo_binary: String,
    pub(super) manifest_path: PathBuf,
    pub(super) workspace: bool,
    pub(super) build_args: Vec<String>,
    pub(super) doc_test_args: Vec<String>,
    pub(super) doc_tests: bool,
    pub(super) checks: Vec<CheckConfig>,
    pub(super) exclude_test_cases: BTreeSet<String>,
    pub(super) env: BTreeMap<String, String>,
    pub(super) shared_cargo_target_dir: Option<PathBuf>,
    pub(super) shared_cargo_build_dir: Option<PathBuf>,
    pub(super) temp_dir: Option<PathBuf>,
    pub(super) runner_parallelism: Option<i64>,
    pub(super) timeout_seconds: u64,
    pub(super) build_cache: CargoBuildCacheConfig,
    pub(super) snapshot_materialization_duration_seconds: JsonValue,
    pub(super) snapshot_materialization_phase_durations: JsonValue,
    pub(super) log_path: PathBuf,
    pub(super) summary_path: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct CommandAdapterConfig {
    pub(super) discovery_program: String,
    pub(super) discovery_args: Vec<String>,
    pub(super) discovery_output_format: String,
    pub(super) run_program: String,
    pub(super) run_args: Vec<String>,
    pub(super) append_test_items: bool,
    pub(super) working_directory: PathBuf,
}

impl DiscoveryShardedConfig {
    pub(super) fn from_request(request: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let workspace_path = path_field(request, "workspace_path")?;
        if !workspace_path.is_dir() {
            return Err(format!(
                "workspace_path `{}` must be an existing directory.",
                path_string(&workspace_path)
            ));
        }
        let output_dir = optional_path(request, "output_dir")
            .unwrap_or_else(|| workspace_path.join(".ait/generated/ci"));
        let runner = request
            .get("runner")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| "Field `runner` must be a JSON object.".to_string())?;
        let kind =
            optional_text(runner, "kind").unwrap_or_else(|| "test_discovery_sharded".to_string());
        if kind != "test_discovery_sharded" {
            return Err(format!(
                "ci-test-discovery-sharded-run requires runner.kind `test_discovery_sharded`; got `{kind}`."
            ));
        }
        let adapter = optional_text(runner, "adapter").unwrap_or_else(|| "cargo".to_string());
        if !matches!(adapter.as_str(), "cargo" | "command") {
            return Err(format!(
                "Unsupported test discovery adapter `{adapter}`. Supported adapters: cargo, command."
            ));
        }
        let command_adapter = if adapter == "command" {
            Some(CommandAdapterConfig::from_runner(runner, &workspace_path)?)
        } else {
            None
        };
        let manifest_path =
            optional_path(runner, "manifest_path").unwrap_or_else(|| PathBuf::from("Cargo.toml"));
        validate_relative_path(&manifest_path, "runner.manifest_path")?;
        let workspace = optional_bool(runner, "workspace")?.unwrap_or(true);
        let build_args = string_array(runner, "build_args")?;
        let doc_test_args = string_array(runner, "doc_test_args")?;
        let doc_tests = optional_bool(runner, "doc_tests")?.unwrap_or(false);
        if adapter == "command" && doc_tests {
            return Err("runner.doc_tests is supported only by the cargo adapter.".to_string());
        }
        let exclude_test_cases = string_set(runner, "exclude_test_cases")?;
        let mut env = string_map(request, "env")?;
        env.extend(string_map(runner, "env")?);
        let shared_cargo_target_dir = optional_path(request, "shared_cargo_target_dir")
            .or_else(|| optional_path(request, "cargo_target_dir"));
        let shared_cargo_build_dir = optional_path(request, "shared_cargo_build_dir")
            .or_else(|| optional_path(request, "cargo_build_dir"));
        let temp_dir = optional_path(request, "temp_dir");
        let runner_parallelism = optional_positive_i64(request, "runner_parallelism")?
            .or(optional_positive_i64(request, "admitted_cpu_tokens")?)
            .or(optional_positive_i64(runner, "runner_parallelism")?)
            .or(optional_positive_i64(runner, "cpu_tokens")?)
            .or(optional_positive_i64(runner, "workers")?);
        let timeout_seconds = validated_ci_process_timeout_seconds(
            optional_positive_i64(runner, "timeout_seconds")?
                .or(optional_positive_i64(request, "timeout_seconds")?),
            "timeout_seconds",
        )?;
        let build_cache = CargoBuildCacheConfig::from_request(request, runner)?;
        let snapshot_materialization = request
            .get("snapshot_materialization_result")
            .and_then(JsonValue::as_object);
        let snapshot_materialization_duration_seconds = snapshot_materialization
            .and_then(|value| value.get("duration_seconds"))
            .cloned()
            .unwrap_or(JsonValue::Null);
        let snapshot_materialization_phase_durations = snapshot_materialization
            .and_then(|value| value.get("phase_durations_seconds"))
            .cloned()
            .unwrap_or(JsonValue::Null);
        let artifacts = request
            .get("artifacts")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let log_path = artifact_path(&output_dir, &artifacts, "log_path", "run.log")?;
        let summary_path = artifact_path(&output_dir, &artifacts, "summary_json", "summary.json")?;
        Ok(Self {
            workspace_path,
            output_dir,
            suite_id: optional_text(request, "suite_id"),
            job_type: optional_text(request, "job_type"),
            job_id: optional_text(request, "job_id"),
            adapter,
            command_adapter,
            cargo_binary: optional_text(runner, "cargo_binary")
                .or_else(|| env::var("AIT_NATIVE_SERVER_CARGO_BIN").ok())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "cargo".to_string()),
            manifest_path,
            workspace,
            build_args,
            doc_test_args,
            doc_tests,
            checks: checks_from_runner(runner)?,
            exclude_test_cases,
            env,
            shared_cargo_target_dir,
            shared_cargo_build_dir,
            temp_dir,
            runner_parallelism,
            timeout_seconds,
            build_cache,
            snapshot_materialization_duration_seconds,
            snapshot_materialization_phase_durations,
            log_path,
            summary_path,
        })
    }

    pub(super) fn manifest_full_path(&self) -> PathBuf {
        self.workspace_path.join(&self.manifest_path)
    }

    pub(super) fn adapter_working_dir(&self) -> PathBuf {
        if let Some(command) = &self.command_adapter {
            if command.working_directory.as_os_str().is_empty() {
                return self.workspace_path.clone();
            }
            return self.workspace_path.join(&command.working_directory);
        }
        self.manifest_full_path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.workspace_path.clone())
    }

    pub(super) fn shard_count(&self, executable_count: usize) -> usize {
        let requested = self.runner_parallelism.unwrap_or(1).max(1) as usize;
        requested.min(executable_count.max(1)).max(1)
    }

    pub(super) fn effective_build_args(&self) -> Vec<String> {
        if self.build_args.is_empty() {
            default_cargo_build_args(self)
        } else {
            self.build_args.clone()
        }
    }
}

impl CommandAdapterConfig {
    fn from_runner(
        runner: &JsonMap<String, JsonValue>,
        workspace_path: &Path,
    ) -> Result<Self, String> {
        let discovery_program = optional_text(runner, "discovery_program").ok_or_else(|| {
            "runner.discovery_program is required for adapter `command`.".to_string()
        })?;
        let run_program = optional_text(runner, "run_program")
            .ok_or_else(|| "runner.run_program is required for adapter `command`.".to_string())?;
        let discovery_output_format = optional_text(runner, "discovery_output_format")
            .unwrap_or_else(|| "json_array".to_string());
        if !matches!(discovery_output_format.as_str(), "json_array" | "lines") {
            return Err(format!(
                "Unsupported runner.discovery_output_format `{discovery_output_format}`. Expected json_array or lines."
            ));
        }
        let working_directory = optional_path(runner, "working_directory").unwrap_or_default();
        validate_relative_path(&working_directory, "runner.working_directory")?;
        if !workspace_path.join(&working_directory).is_dir() {
            return Err(format!(
                "runner.working_directory `{}` must be an existing directory inside workspace_path.",
                path_string(&working_directory)
            ));
        }
        Ok(Self {
            discovery_program,
            discovery_args: string_array(runner, "discovery_args")?,
            discovery_output_format,
            run_program,
            run_args: string_array(runner, "run_args")?,
            append_test_items: optional_bool(runner, "append_test_items")?.unwrap_or(false),
            working_directory,
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct CargoBuildCacheConfig {
    pub(super) policy: String,
    pub(super) executable_manifest_path: Option<PathBuf>,
    pub(super) changed_paths_known: bool,
    pub(super) changed_paths: Vec<String>,
}

impl CargoBuildCacheConfig {
    pub(super) fn from_request(
        request: &JsonMap<String, JsonValue>,
        runner: &JsonMap<String, JsonValue>,
    ) -> Result<Self, String> {
        let raw = request
            .get("build_cache")
            .and_then(JsonValue::as_object)
            .or_else(|| runner.get("build_cache").and_then(JsonValue::as_object));
        let Some(raw) = raw else {
            return Ok(Self {
                policy: "disabled".to_string(),
                executable_manifest_path: None,
                changed_paths_known: false,
                changed_paths: Vec::new(),
            });
        };
        let policy = optional_text(raw, "policy").unwrap_or_else(|| "disabled".to_string());
        let executable_manifest_path = optional_path(raw, "executable_manifest_path");
        let changed_paths_known = raw.contains_key("changed_paths");
        let changed_paths = string_array(raw, "changed_paths")?;
        Ok(Self {
            policy,
            executable_manifest_path,
            changed_paths_known,
            changed_paths,
        })
    }

    pub(super) fn enabled(&self) -> bool {
        self.policy == "reuse_when_rust_inputs_unchanged"
    }
}

#[derive(Debug, Clone)]
pub(super) struct CheckConfig {
    check_id: String,
    kind: String,
    file_name_suffix: String,
    exclude_dirs: Vec<String>,
    args: Vec<String>,
}
pub(super) fn run_checks(config: &DiscoveryShardedConfig) -> Result<Vec<JsonValue>, String> {
    let mut reports = Vec::new();
    for (index, check) in config.checks.iter().enumerate() {
        reports.push(match check.kind.as_str() {
            "forbid_files" => run_forbid_files_check(index, check, &config.workspace_path)?,
            "cargo_fmt" => run_cargo_fmt_check(index, check, config)?,
            _ => {
                return Err(format!(
                    "Unsupported test discovery check kind `{}`.",
                    check.kind
                ))
            }
        });
    }
    Ok(reports)
}

fn run_forbid_files_check(
    index: usize,
    check: &CheckConfig,
    workspace_path: &Path,
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let matches = forbidden_file_matches(workspace_path, check)?;
    let status = if matches.is_empty() { "pass" } else { "fail" };
    Ok(json!({
        "index": index + 1,
        "check_id": check.check_id,
        "kind": check.kind,
        "status": status,
        "duration_seconds": duration_seconds(started),
        "file_name_suffix": check.file_name_suffix,
        "match_count": matches.len(),
        "matches": matches,
    }))
}

fn run_cargo_fmt_check(
    index: usize,
    check: &CheckConfig,
    config: &DiscoveryShardedConfig,
) -> Result<JsonValue, String> {
    let args = if check.args.is_empty() {
        default_cargo_fmt_args(config)
    } else {
        check.args.clone()
    };
    let report = run_process(
        "check_cargo_fmt",
        index + 1,
        &config.cargo_binary,
        &args,
        &config.adapter_working_dir(),
        ProcessContext {
            output_dir: &config.output_dir.join("checks"),
            env: &command_env(config, EnvMode::Build),
            timeout_seconds: config.timeout_seconds,
        },
    )?;
    let mut value = report.to_json();
    value["check_id"] = json!(check.check_id);
    value["kind"] = json!(check.kind);
    Ok(value)
}

fn forbidden_file_matches(root: &Path, check: &CheckConfig) -> Result<Vec<String>, String> {
    let mut matches = Vec::new();
    visit_forbidden_files(root, root, check, &mut matches)?;
    matches.sort();
    Ok(matches)
}

fn visit_forbidden_files(
    root: &Path,
    path: &Path,
    check: &CheckConfig,
    matches: &mut Vec<String>,
) -> Result<(), String> {
    if should_skip_dir(root, path, &check.exclude_dirs) {
        return Ok(());
    }
    for entry in fs::read_dir(path)
        .map_err(|exc| format!("Failed to read directory `{}`: {exc}", path_string(path)))?
    {
        let entry = entry.map_err(|exc| {
            format!(
                "Failed to read directory entry under `{}`: {exc}",
                path_string(path)
            )
        })?;
        let entry_path = entry.path();
        let file_type = entry.file_type().map_err(|exc| {
            format!(
                "Failed to inspect file type `{}`: {exc}",
                path_string(&entry_path)
            )
        })?;
        if file_type.is_dir() {
            visit_forbidden_files(root, &entry_path, check, matches)?;
        } else if file_type.is_file()
            && entry_path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(&check.file_name_suffix))
        {
            matches.push(relative_path_string(root, &entry_path));
        }
    }
    Ok(())
}

fn should_skip_dir(root: &Path, path: &Path, exclude_dirs: &[String]) -> bool {
    if path == root {
        return false;
    }
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().any(|component| match component {
        Component::Normal(value) => value
            .to_str()
            .is_some_and(|text| exclude_dirs.iter().any(|exclude| exclude == text)),
        _ => false,
    })
}
fn checks_from_runner(runner: &JsonMap<String, JsonValue>) -> Result<Vec<CheckConfig>, String> {
    let Some(raw) = runner.get("checks") else {
        return Ok(Vec::new());
    };
    let values = raw
        .as_array()
        .ok_or_else(|| "Field `runner.checks` must be an array.".to_string())?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let check = value
                .as_object()
                .ok_or_else(|| "runner.checks entries must be objects.".to_string())?;
            let kind = optional_text(check, "kind").unwrap_or_else(|| "forbid_files".to_string());
            let check_id =
                optional_text(check, "check_id").unwrap_or_else(|| format!("check-{}", index + 1));
            let file_name_suffix =
                optional_text(check, "file_name_suffix").unwrap_or_else(|| ".py".to_string());
            let exclude_dirs = string_array(check, "exclude_dirs")?;
            Ok(CheckConfig {
                check_id,
                kind,
                file_name_suffix,
                exclude_dirs,
                args: string_array(check, "args")?,
            })
        })
        .collect()
}
