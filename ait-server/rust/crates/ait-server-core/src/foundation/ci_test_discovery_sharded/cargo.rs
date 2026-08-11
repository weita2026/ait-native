use serde_json::{json, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::fs::{File, FileTimes};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use super::config::DiscoveryShardedConfig;
use super::paths::{
    duration_seconds, optional_text, path_string, string_array, validate_relative_path,
};
use super::process::{command_env, run_process, EnvMode, ProcessContext, ProcessReport};

#[derive(Debug, Clone)]
pub(super) struct TestExecutable {
    pub(super) index: usize,
    pub(super) path: PathBuf,
    pub(super) package_name: Option<String>,
    pub(super) target_name: Option<String>,
    pub(super) target_kind: Vec<String>,
    pub(super) cargo_bin_exe_env: BTreeMap<String, String>,
}

impl TestExecutable {
    pub(super) fn to_json(&self) -> JsonValue {
        json!({
            "index": self.index,
            "path": path_string(&self.path),
            "package_name": self.package_name,
            "target_name": self.target_name,
            "target_kind": self.target_kind,
            "cargo_bin_exe_env": self.cargo_bin_exe_env,
        })
    }
}
#[derive(Debug)]
pub(super) struct DiscoveryBuildReport {
    pub(super) process: ProcessReport,
    pub(super) executables: Vec<TestExecutable>,
    pub(super) status: &'static str,
}

impl DiscoveryBuildReport {
    pub(super) fn to_json(&self) -> JsonValue {
        let mut value = self.process.to_json();
        value["executable_count"] = json!(self.executables.len());
        value["executables"] = json!(self
            .executables
            .iter()
            .map(TestExecutable::to_json)
            .collect::<Vec<_>>());
        value
    }

    pub(super) fn failure_json(&self, stage: &str) -> JsonValue {
        self.process.failure_json(stage)
    }
}
pub(super) fn run_cargo_discovery_build(
    config: &DiscoveryShardedConfig,
) -> Result<DiscoveryBuildReport, String> {
    if let Some(reused) = try_reuse_cargo_discovery_build(config)? {
        return Ok(reused);
    }
    refresh_changed_cargo_input_mtimes(config)?;
    let args = config.effective_build_args();
    let output = run_process(
        "discover_build",
        1,
        &config.cargo_binary,
        &args,
        &config.adapter_working_dir(),
        ProcessContext {
            output_dir: &config.output_dir,
            env: &command_env(config, EnvMode::Build),
            timeout_seconds: config.timeout_seconds,
        },
    )?;
    let executables = if output.status == "pass" {
        parse_cargo_test_executables_from_log(&output.log_path)?
    } else {
        Vec::new()
    };
    let status = if output.status == "pass" && executables.is_empty() {
        "fail"
    } else {
        output.status
    };
    let process = if status == "fail" && output.status == "pass" {
        ProcessReport {
            status: "fail",
            stderr_tail: "Cargo discovery did not report any test executables.".to_string(),
            combined_tail: output.combined_tail.clone(),
            ..output
        }
    } else {
        output
    };
    if status == "pass" {
        write_cargo_executable_manifest(config, &executables)?;
    }
    Ok(DiscoveryBuildReport {
        process,
        executables,
        status,
    })
}

fn refresh_changed_cargo_input_mtimes(config: &DiscoveryShardedConfig) -> Result<(), String> {
    if !config.build_cache.enabled() || !config.build_cache.changed_paths_known {
        return Ok(());
    }
    let changed_paths = config
        .build_cache
        .changed_paths
        .iter()
        .filter(|path| cargo_build_relevant_path(path))
        .map(|path| path.trim().trim_start_matches("./").replace('\\', "/"))
        .collect::<BTreeSet<_>>();
    if changed_paths.is_empty() {
        return Ok(());
    }
    let workspace = fs::canonicalize(&config.workspace_path).map_err(|exc| {
        format!(
            "Failed to resolve CI workspace before refreshing changed Cargo input mtimes `{}`: {exc}",
            path_string(&config.workspace_path)
        )
    })?;
    let modified = SystemTime::now();
    for relative_text in changed_paths {
        let relative = PathBuf::from(&relative_text);
        validate_relative_path(&relative, "build_cache.changed_paths")?;
        let path = config.workspace_path.join(&relative);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(exc) if exc.kind() == io::ErrorKind::NotFound => continue,
            Err(exc) => {
                return Err(format!(
                    "Failed to inspect changed Cargo input `{}` before discovery build: {exc}",
                    path_string(&path)
                ))
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Changed Cargo input must not be a symlink before discovery build: {}",
                path_string(&path)
            ));
        }
        if !metadata.is_file() {
            continue;
        }
        let resolved = fs::canonicalize(&path).map_err(|exc| {
            format!(
                "Failed to resolve changed Cargo input `{}` before discovery build: {exc}",
                path_string(&path)
            )
        })?;
        if !resolved.starts_with(&workspace) {
            return Err(format!(
                "Changed Cargo input escaped the CI workspace before discovery build: {}",
                path_string(&path)
            ));
        }
        File::open(&resolved)
            .and_then(|file| file.set_times(FileTimes::new().set_modified(modified)))
            .map_err(|exc| {
                format!(
                    "Failed to refresh changed Cargo input mtime `{}` before discovery build: {exc}",
                    path_string(&resolved)
                )
            })?;
    }
    Ok(())
}

fn try_reuse_cargo_discovery_build(
    config: &DiscoveryShardedConfig,
) -> Result<Option<DiscoveryBuildReport>, String> {
    if !config.build_cache.enabled() {
        return Ok(None);
    }
    let Some(manifest_path) = config.build_cache.executable_manifest_path.as_ref() else {
        return Ok(None);
    };
    let started = Instant::now();
    let cache_key = cargo_executable_manifest_cache_key(config);
    let reason = cargo_build_cache_miss_reason(config, manifest_path, &cache_key)?;
    let log_path = config.output_dir.join("discover_build-001.log");
    if let Some(reason) = reason {
        write_cache_reuse_log(
            &log_path,
            "miss",
            &reason,
            manifest_path,
            &config.build_cache.changed_paths,
        )?;
        return Ok(None);
    }
    let manifest = read_cargo_executable_manifest(manifest_path)?;
    let mut executables = test_executables_from_manifest(&manifest)?;
    executables.sort_by(|left, right| path_string(&left.path).cmp(&path_string(&right.path)));
    for (index, executable) in executables.iter_mut().enumerate() {
        executable.index = index + 1;
    }
    write_cache_reuse_log(
        &log_path,
        "hit",
        "rust_inputs_unchanged_and_all_executables_exist",
        manifest_path,
        &config.build_cache.changed_paths,
    )?;
    let process = ProcessReport {
        index: 1,
        phase: "discover_build",
        command: format!(
            "reuse_cargo_test_executable_manifest {}",
            path_string(manifest_path)
        ),
        status: "pass",
        exit_code: 0,
        timed_out: false,
        timeout_seconds: config.timeout_seconds,
        duration_seconds: duration_seconds(started),
        stdout_tail: format!(
            "Reused {} cached cargo test executables from `{}`.",
            executables.len(),
            path_string(manifest_path)
        ),
        stderr_tail: String::new(),
        combined_tail: format!(
            "stdout:\nReused {} cached cargo test executables from `{}`.\n\nstderr:\n",
            executables.len(),
            path_string(manifest_path)
        ),
        log_path,
        stdout_bytes: 0,
        stderr_bytes: 0,
    };
    Ok(Some(DiscoveryBuildReport {
        process,
        executables,
        status: "pass",
    }))
}

fn cargo_build_cache_miss_reason(
    config: &DiscoveryShardedConfig,
    manifest_path: &Path,
    cache_key: &str,
) -> Result<Option<String>, String> {
    if !config.build_cache.changed_paths_known {
        return Ok(Some("changed_paths_unknown".to_string()));
    }
    if config
        .build_cache
        .changed_paths
        .iter()
        .any(|path| cargo_build_relevant_path(path))
    {
        return Ok(Some("rust_or_cargo_input_changed".to_string()));
    }
    if !manifest_path.is_file() {
        return Ok(Some("executable_manifest_missing".to_string()));
    }
    let manifest = read_cargo_executable_manifest(manifest_path)?;
    if manifest.get("cache_key").and_then(JsonValue::as_str) != Some(cache_key) {
        return Ok(Some("executable_manifest_cache_key_mismatch".to_string()));
    }
    let executables = test_executables_from_manifest(&manifest)?;
    if executables.is_empty() {
        return Ok(Some("executable_manifest_empty".to_string()));
    }
    if executables
        .iter()
        .any(|executable| !executable.path.is_file())
    {
        return Ok(Some("cached_executable_missing".to_string()));
    }
    if executables.iter().any(|executable| {
        executable
            .cargo_bin_exe_env
            .values()
            .any(|path| !Path::new(path).is_file())
    }) {
        return Ok(Some("cached_cargo_bin_executable_missing".to_string()));
    }
    Ok(None)
}

fn write_cargo_executable_manifest(
    config: &DiscoveryShardedConfig,
    executables: &[TestExecutable],
) -> Result<(), String> {
    let Some(manifest_path) = config.build_cache.executable_manifest_path.as_ref() else {
        return Ok(());
    };
    if !config.build_cache.enabled() {
        return Ok(());
    }
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create cargo executable manifest parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    let manifest = json!({
        "contract": "ait.server.ci.cargo_test_executable_manifest.v1",
        "cache_key": cargo_executable_manifest_cache_key(config),
        "adapter": config.adapter,
        "manifest_path": path_string(&config.manifest_path),
        "workspace_path": path_string(&config.workspace_path),
        "build_args": config.effective_build_args(),
        "executable_count": executables.len(),
        "executables": executables.iter().map(TestExecutable::to_json).collect::<Vec<_>>(),
    });
    fs::write(
        manifest_path,
        serde_json::to_string_pretty(&manifest).map_err(|exc| exc.to_string())? + "\n",
    )
    .map_err(|exc| {
        format!(
            "Failed to write cargo executable manifest `{}`: {exc}",
            path_string(manifest_path)
        )
    })
}

fn read_cargo_executable_manifest(path: &Path) -> Result<JsonValue, String> {
    let text = fs::read_to_string(path).map_err(|exc| {
        format!(
            "Failed to read cargo executable manifest `{}`: {exc}",
            path_string(path)
        )
    })?;
    serde_json::from_str(&text).map_err(|exc| {
        format!(
            "Failed to parse cargo executable manifest `{}`: {exc}",
            path_string(path)
        )
    })
}

fn test_executables_from_manifest(manifest: &JsonValue) -> Result<Vec<TestExecutable>, String> {
    let values = manifest
        .get("executables")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Cargo executable manifest must contain `executables` array.".to_string())?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let object = value.as_object().ok_or_else(|| {
                "Cargo executable manifest executables must be objects.".to_string()
            })?;
            let path = optional_text(object, "path").ok_or_else(|| {
                "Cargo executable manifest executable.path is required.".to_string()
            })?;
            Ok(TestExecutable {
                index: index + 1,
                path: PathBuf::from(path),
                package_name: optional_text(object, "package_name"),
                target_name: optional_text(object, "target_name"),
                target_kind: string_array(object, "target_kind")?,
                cargo_bin_exe_env: object
                    .get("cargo_bin_exe_env")
                    .and_then(JsonValue::as_object)
                    .map(|values| {
                        values
                            .iter()
                            .map(|(key, value)| {
                                value
                                    .as_str()
                                    .map(|value| (key.clone(), value.to_string()))
                                    .ok_or_else(|| {
                                        "Cargo executable manifest cargo_bin_exe_env values must be strings."
                                            .to_string()
                                    })
                            })
                            .collect::<Result<BTreeMap<_, _>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn cargo_executable_manifest_cache_key(config: &DiscoveryShardedConfig) -> String {
    serde_json::to_string(&json!({
        "adapter": config.adapter,
        "manifest_path": path_string(&config.manifest_path),
        "workspace_path": path_string(&config.workspace_path),
        "workspace": config.workspace,
        "build_args": config.effective_build_args(),
        "cargo_bin_exe_env_contract": "cargo_json_compiler_artifact/v1",
    }))
    .unwrap_or_else(|_| "unserializable-cache-key".to_string())
}

pub(super) fn cargo_build_relevant_path(path: &str) -> bool {
    let normalized = path.trim().trim_start_matches("./").replace('\\', "/");
    if normalized.is_empty() {
        return false;
    }
    normalized == "Cargo.toml"
        || normalized == "Cargo.lock"
        || normalized == "rust-toolchain"
        || normalized == "rust-toolchain.toml"
        || normalized == ".cargo"
        || normalized.starts_with(".cargo/")
        || normalized.starts_with("rust/")
        || normalized.ends_with("/Cargo.toml")
        || normalized.ends_with("/Cargo.lock")
        || normalized.ends_with("/build.rs")
}

fn write_cache_reuse_log(
    log_path: &Path,
    status: &str,
    reason: &str,
    manifest_path: &Path,
    changed_paths: &[String],
) -> Result<(), String> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create cache reuse log parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    fs::write(
        log_path,
        format!(
            "cargo_build_cache_status={status}\nreason={reason}\nexecutable_manifest_path={}\nchanged_path_count={}\nchanged_paths={}\n",
            path_string(manifest_path),
            changed_paths.len(),
            changed_paths.join(",")
        ),
    )
    .map_err(|exc| {
        format!(
            "Failed to write cache reuse log `{}`: {exc}",
            path_string(log_path)
        )
    })
}
pub(super) fn run_cargo_doc_tests(
    config: &DiscoveryShardedConfig,
) -> Result<ProcessReport, String> {
    let args = if config.doc_test_args.is_empty() {
        default_cargo_doc_test_args(config)
    } else {
        config.doc_test_args.clone()
    };
    run_process(
        "doc_tests",
        1,
        &config.cargo_binary,
        &args,
        &config.adapter_working_dir(),
        ProcessContext {
            output_dir: &config.output_dir,
            env: &command_env(config, EnvMode::Build),
            timeout_seconds: config.timeout_seconds,
        },
    )
}
fn parse_cargo_test_executables_from_log(log_path: &Path) -> Result<Vec<TestExecutable>, String> {
    let file = File::open(log_path).map_err(|exc| {
        format!(
            "Failed to read cargo discovery log `{}`: {exc}",
            path_string(log_path)
        )
    })?;
    parse_cargo_test_executable_lines(BufReader::new(file).lines())
}

fn parse_cargo_test_executable_lines<I>(lines: I) -> Result<Vec<TestExecutable>, String>
where
    I: IntoIterator<Item = Result<String, io::Error>>,
{
    let mut executables = Vec::new();
    let mut cargo_bin_exe_by_package = BTreeMap::<String, BTreeMap<String, String>>::new();
    for line in lines {
        let line = line.map_err(|exc| format!("Failed to stream cargo discovery output: {exc}"))?;
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<JsonValue>(line) else {
            continue;
        };
        if value.get("reason").and_then(JsonValue::as_str) != Some("compiler-artifact") {
            continue;
        }
        let Some(executable) = value
            .get("executable")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let target = value.get("target").and_then(JsonValue::as_object);
        let target_kind = target
            .and_then(|target| target.get("kind"))
            .and_then(JsonValue::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if target_kind.iter().any(|kind| kind == "custom-build") {
            continue;
        }
        let package_name = value
            .get("package_id")
            .and_then(JsonValue::as_str)
            .map(str::to_string);
        let target_name = target
            .and_then(|target| target.get("name"))
            .and_then(JsonValue::as_str)
            .map(str::to_string);
        let profile_is_test = value
            .get("profile")
            .and_then(|profile| profile.get("test"))
            .and_then(JsonValue::as_bool)
            == Some(true);
        if !profile_is_test && target_kind.iter().any(|kind| kind == "bin") {
            if let (Some(package_name), Some(target_name)) =
                (package_name.as_ref(), target_name.as_ref())
            {
                cargo_bin_exe_by_package
                    .entry(package_name.clone())
                    .or_default()
                    .insert(
                        format!("CARGO_BIN_EXE_{target_name}"),
                        executable.to_string(),
                    );
            }
        }
        if !profile_is_test {
            continue;
        }
        executables.push(TestExecutable {
            index: 0,
            path: PathBuf::from(executable),
            package_name,
            target_name,
            target_kind,
            cargo_bin_exe_env: BTreeMap::new(),
        });
    }
    for executable in &mut executables {
        executable.cargo_bin_exe_env = executable
            .package_name
            .as_ref()
            .and_then(|package_name| cargo_bin_exe_by_package.get(package_name))
            .cloned()
            .unwrap_or_default();
    }
    executables.sort_by(|left, right| path_string(&left.path).cmp(&path_string(&right.path)));
    executables.dedup_by(|left, right| left.path == right.path);
    for (index, executable) in executables.iter_mut().enumerate() {
        executable.index = index + 1;
    }
    Ok(executables)
}

pub(super) fn default_cargo_build_args(config: &DiscoveryShardedConfig) -> Vec<String> {
    let mut args = vec![
        "test".to_string(),
        "--manifest-path".to_string(),
        path_string(&config.manifest_full_path()),
    ];
    if config.workspace {
        args.push("--workspace".to_string());
    }
    args.extend([
        "--profile".to_string(),
        "ait-ci".to_string(),
        "--no-run".to_string(),
        "--message-format=json".to_string(),
    ]);
    args
}

pub(super) fn default_cargo_doc_test_args(config: &DiscoveryShardedConfig) -> Vec<String> {
    let mut args = vec![
        "test".to_string(),
        "--manifest-path".to_string(),
        path_string(&config.manifest_full_path()),
    ];
    if config.workspace {
        args.push("--workspace".to_string());
    }
    args.push("--profile".to_string());
    args.push("ait-ci".to_string());
    args.push("--doc".to_string());
    args
}

pub(super) fn default_cargo_fmt_args(config: &DiscoveryShardedConfig) -> Vec<String> {
    vec![
        "fmt".to_string(),
        "--manifest-path".to_string(),
        path_string(&config.manifest_full_path()),
        "--all".to_string(),
        "--check".to_string(),
    ]
}
