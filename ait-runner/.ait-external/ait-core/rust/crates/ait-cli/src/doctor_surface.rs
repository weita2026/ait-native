use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::plan_binary_db::{
    inspect_plan_binary_db_authority, PlanBinaryDbRecoveryReport, PlanBinaryDbRecoveryState,
};
use ait_core::plan_filesystem::operational_external_materialization_roots;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const RUNTIME_DATA_ENV: &str = ait_core::environment_contract::names::AIT_RUNTIME_DATA;
const LEGACY_SERVER_DATA_ENV: &str = ait_core::environment_contract::names::AIT_NATIVE_SERVER_DATA;
const PLAN_AUTHORITY_CONTRACT_VERSION: &str = "plan-foundation-v7";
const TASK_CONTRACT_VERSION: &str = "task-close-v1";
const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

const PLAN_AUTHORITY_COMMAND_SURFACE: &[&str] =
    &["ait plan items", "ait plan candidates", "ait plan inspect"];

const PLAN_AUTHORITY_REQUIRED_EXPORTS: &[&str] = &[
    "plan_items_payload",
    "plan_dispatch_summary",
    "plan_task_link_indexes",
    "compute_taskable_items",
    "validate_dispatch_legality",
    "plan_candidates_payload",
    "extract_plan_items",
    "extract_plan_section",
    "find_plan_item",
    "normalize_plan_items",
];

pub fn doctor_memory_root(repo: &crate::runtime::RepoRuntime) -> Result<JsonValue, String> {
    crate::task_worktree_layout::doctor_memory_root_payload(repo)
}

pub fn doctor_runtime_root(repo_root: &Path) -> Result<JsonValue, String> {
    let resolved_root = resolve_path_strict_false(repo_root);
    let configured_runtime_root = resolved_runtime_data_root();
    let policy = workspace_ignore_policy(&resolved_root)?;
    let Some(configured_runtime_root) = configured_runtime_root else {
        return Ok(json!({
            "repo_root": path_text(&resolved_root),
            "runtime_root": JsonValue::Null,
            "runtime_root_source": "unconfigured",
            "runtime_root_relative_to_repo": JsonValue::Null,
            "inside_repo": false,
            "outside_repo": false,
            "equals_repo_root": false,
            "snapshot_ignored": false,
            "protected_from_snapshots": false,
            "state": "pass",
            "recommended_action": "configure_when_server_is_enabled",
            "reasons": ["No server runtime root is configured."],
            "next_actions": [
                format!("Configure {RUNTIME_DATA_ENV} only when this repository will run ait-server locally."),
            ],
            "ignore_policy": policy,
        }));
    };

    let resolved_runtime = resolve_path_strict_false(&configured_runtime_root);
    let runtime_rel = relative_to(&resolved_runtime, &resolved_root);
    let inside_repo = runtime_rel.is_some();
    let equals_repo_root = resolved_runtime == resolved_root;
    let ignored_runtime_roots = workspace_runtime_roots(&resolved_root);
    let snapshot_ignored = ignored_runtime_roots.contains(&resolved_runtime);
    let protected_from_snapshots = !inside_repo || snapshot_ignored;
    let (state, recommended_action, reasons, next_actions) = if equals_repo_root {
        (
            "fail",
            "move_runtime_root_outside_repo",
            vec!["The configured runtime root is the repository checkout itself.".to_string()],
            vec![
                format!(
                    "Set {RUNTIME_DATA_ENV} to a dedicated directory outside the repo checkout."
                ),
                "Move existing server data before creating new snapshots.".to_string(),
            ],
        )
    } else if inside_repo {
        (
            "warn",
            "prefer_external_runtime_root",
            vec![
                "The configured runtime root is inside the repository checkout but is ignored by snapshot/status scans."
                    .to_string(),
            ],
            vec![format!(
                "Prefer the default external runtime root or an absolute {RUNTIME_DATA_ENV} path outside the repo."
            )],
        )
    } else {
        (
            "pass",
            "none",
            vec!["The configured runtime root is outside the repository checkout.".to_string()],
            Vec::new(),
        )
    };

    Ok(json!({
        "repo_root": path_text(&resolved_root),
        "runtime_root": path_text(&resolved_runtime),
        "runtime_root_source": runtime_root_source(),
        "runtime_root_relative_to_repo": runtime_rel.map(JsonValue::String).unwrap_or(JsonValue::Null),
        "inside_repo": inside_repo,
        "outside_repo": !inside_repo,
        "equals_repo_root": equals_repo_root,
        "snapshot_ignored": snapshot_ignored,
        "protected_from_snapshots": protected_from_snapshots,
        "state": state,
        "recommended_action": recommended_action,
        "reasons": reasons,
        "next_actions": next_actions,
        "ignore_policy": policy,
    }))
}

pub fn doctor_plan_authority() -> Result<JsonValue, String> {
    doctor_plan_authority_impl(None)
}

pub fn doctor_plan_authority_for_repository(repo_root: &Path) -> Result<JsonValue, String> {
    doctor_plan_authority_impl(Some(repo_root))
}

fn doctor_plan_authority_impl(repo_root: Option<&Path>) -> Result<JsonValue, String> {
    let selected_backend = selected_plan_backend()?;
    let mut payload = json!({
        "selected_backend": selected_backend,
        "selected_backend_ready": selected_backend != "rust",
        "rust_authority_ready": false,
        "compatibility": if selected_backend == "rust" { "unavailable" } else { "inactive" },
        "authority_source": "ait-core-rust",
        "extension_module": "ait_py",
        "extension_loaded": false,
        "extension_path": JsonValue::Null,
        "package_version": PACKAGE_VERSION,
        "extension_package_version": JsonValue::Null,
        "extension_task_contract_version": JsonValue::Null,
        "extension_plan_contract_version": JsonValue::Null,
        "expected_plan_contract_version": PLAN_AUTHORITY_CONTRACT_VERSION,
        "surface_commands": PLAN_AUTHORITY_COMMAND_SURFACE,
        "required_exports": PLAN_AUTHORITY_REQUIRED_EXPORTS,
        "exports": {},
        "missing_exports": [],
        "issues": [],
        "env": {},
        "repository_inspected": false,
        "repository_ready": JsonValue::Null,
        "repository_authority": JsonValue::Null,
    });
    if selected_backend != "rust" {
        return Ok(payload);
    }

    let exports = PLAN_AUTHORITY_REQUIRED_EXPORTS
        .iter()
        .map(|name| ((*name).to_string(), JsonValue::Bool(true)))
        .collect::<JsonMap<String, JsonValue>>();
    let obj = payload
        .as_object_mut()
        .ok_or_else(|| "plan-authority payload must be an object".to_string())?;
    obj.insert("selected_backend_ready".to_string(), JsonValue::Bool(true));
    obj.insert("rust_authority_ready".to_string(), JsonValue::Bool(true));
    obj.insert(
        "compatibility".to_string(),
        JsonValue::String("compatible".to_string()),
    );
    obj.insert("extension_loaded".to_string(), JsonValue::Bool(true));
    obj.insert(
        "extension_package_version".to_string(),
        JsonValue::String(PACKAGE_VERSION.to_string()),
    );
    obj.insert(
        "extension_task_contract_version".to_string(),
        JsonValue::String(TASK_CONTRACT_VERSION.to_string()),
    );
    obj.insert(
        "extension_plan_contract_version".to_string(),
        JsonValue::String(PLAN_AUTHORITY_CONTRACT_VERSION.to_string()),
    );
    obj.insert("exports".to_string(), JsonValue::Object(exports));
    if let Some(repo_root) = repo_root {
        let report = inspect_plan_binary_db_authority(&repo_root.join(".ait/binary-db"));
        let repository_ready = report.is_ready();
        obj.insert("repository_inspected".to_string(), JsonValue::Bool(true));
        obj.insert(
            "repository_ready".to_string(),
            JsonValue::Bool(repository_ready),
        );
        obj.insert(
            "repository_authority".to_string(),
            plan_recovery_report_json(&report),
        );
        if !repository_ready {
            let issues = obj
                .get_mut("issues")
                .and_then(JsonValue::as_array_mut)
                .ok_or_else(|| "plan-authority issues must be an array".to_string())?;
            issues.extend(report.issues.iter().cloned().map(JsonValue::String));
            if report.state == PlanBinaryDbRecoveryState::Repairable {
                issues.push(JsonValue::String(
                    "Active Plan authority contains only uncommitted damage that the next repository admission can repair safely."
                        .to_string(),
                ));
            }
        }
    }
    Ok(payload)
}

fn plan_recovery_report_json(report: &PlanBinaryDbRecoveryReport) -> JsonValue {
    let recommended_action = match report.state {
        PlanBinaryDbRecoveryState::Clean | PlanBinaryDbRecoveryState::Repaired => "none",
        PlanBinaryDbRecoveryState::Repairable => "retry_for_safe_automatic_recovery",
        PlanBinaryDbRecoveryState::Blocked => "restore_known_good_binary_db_authority",
    };
    json!({
        "authority_root": report.authority_root.to_string_lossy().to_string(),
        "state": report.state.as_str(),
        "ready": report.is_ready(),
        "committed_plan_count": report.committed_plan_count,
        "repair_candidates": report.repair_candidates,
        "repairs": report.repairs,
        "issues": report.issues,
        "recommended_action": recommended_action,
    })
}

pub fn render_doctor_text(title: &str, payload: &JsonValue) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "doctor payload must be an object".to_string())?;
    let mut lines = vec![title.to_string()];
    for key in [
        "state",
        "ready",
        "recommended_action",
        "compatibility",
        "selected_backend",
        "mount_point",
        "runtime_root",
        "filesystem_total_bytes",
        "available_bytes",
        "platform_proof",
    ] {
        if let Some(value) = obj.get(key) {
            if !value.is_null() {
                lines.push(format!("{key}: {}", scalar_text(value)));
            }
        }
    }
    if let Some(issues) = obj.get("issues").and_then(JsonValue::as_array) {
        if !issues.is_empty() {
            lines.push("issues:".to_string());
            for issue in issues {
                if let Some(text) = issue.as_str() {
                    lines.push(format!("- {text}"));
                }
            }
        }
    }
    if let Some(warnings) = obj.get("warnings").and_then(JsonValue::as_array) {
        if !warnings.is_empty() {
            lines.push("warnings:".to_string());
            for warning in warnings {
                if let Some(text) = warning.as_str() {
                    lines.push(format!("- {text}"));
                }
            }
        }
    }
    Ok(lines.join("\n"))
}

fn selected_plan_backend() -> Result<String, String> {
    Ok("rust".to_string())
}

fn workspace_ignore_policy(repo_root: &Path) -> Result<JsonValue, String> {
    let mut operational_roots = vec![".ait".to_string(), ".ait-runtime".to_string()];
    let mut external_roots =
        operational_external_materialization_roots(repo_root.to_string_lossy().as_ref())
            .unwrap_or_default();
    let mut runtime_roots = Vec::new();
    for runtime_root in workspace_runtime_roots(repo_root) {
        let runtime_rel =
            relative_to(&runtime_root, repo_root).unwrap_or_else(|| path_text(&runtime_root));
        runtime_roots.push(runtime_rel.clone());
        operational_roots.push(runtime_rel);
    }
    operational_roots.extend(external_roots.iter().cloned());
    operational_roots.sort();
    operational_roots.dedup();
    external_roots.sort();
    external_roots.dedup();
    runtime_roots.sort();
    runtime_roots.dedup();
    let custom_patterns = load_workspace_ignore_rule_sources(repo_root);
    let mut payload = json!({
        "dir_names": [".ait", ".ait-runtime", ".ait-server", ".ait-worktree", ".ait-worktree-links", ".git", "__pycache__", ".pytest_cache", ".venv", "venv", ".mypy_cache"],
        "file_names": [".DS_Store", ".ait-worktree.json"],
        "operational_roots": operational_roots,
        "external_materialization_roots": external_roots,
        "runtime_roots": runtime_roots,
    });
    if !custom_patterns.is_empty() {
        let obj = payload
            .as_object_mut()
            .ok_or_else(|| "ignore policy payload must be an object".to_string())?;
        obj.insert(
            "rule_files".to_string(),
            JsonValue::Array(vec![JsonValue::String(".aitignore".to_string())]),
        );
        obj.insert(
            "custom_patterns".to_string(),
            JsonValue::Array(custom_patterns.into_iter().map(JsonValue::String).collect()),
        );
    }
    Ok(payload)
}

fn workspace_runtime_roots(repo_root: &Path) -> Vec<PathBuf> {
    let configured_runtime_root = resolved_runtime_data_root();
    let Some(configured_runtime_root) = configured_runtime_root else {
        return Vec::new();
    };
    let resolved_root = resolve_path_strict_false(repo_root);
    let resolved_runtime = resolve_path_strict_false(&configured_runtime_root);
    if resolved_runtime == resolved_root || !resolved_runtime.starts_with(&resolved_root) {
        return Vec::new();
    }
    vec![resolved_runtime]
}

fn resolved_runtime_data_root() -> Option<PathBuf> {
    let (_, value) = configured_runtime_data_env()?;
    Some(resolve_path_strict_false(&expanduser_str(&value)))
}

fn runtime_root_source() -> String {
    configured_runtime_data_env()
        .map(|(name, _)| name)
        .unwrap_or_else(|| "unconfigured".to_string())
}

fn configured_runtime_data_env() -> Option<(String, String)> {
    for name in [RUNTIME_DATA_ENV, LEGACY_SERVER_DATA_ENV] {
        if let Ok(value) = env::var(name) {
            if !value.trim().is_empty() {
                return Some((name.to_string(), value.trim().to_string()));
            }
        }
    }
    None
}

fn load_workspace_ignore_rule_sources(repo_root: &Path) -> Vec<String> {
    let path = repo_root.join(".aitignore");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

fn relative_to(path: &Path, root: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    if rel.as_os_str().is_empty() {
        Some(".".to_string())
    } else {
        Some(rel.to_string_lossy().replace('\\', "/"))
    }
}

fn resolve_path_strict_false(path: &Path) -> PathBuf {
    if let Ok(resolved) = path.canonicalize() {
        return resolved;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    normalize_existing_ancestor(&absolute)
}

fn normalize_existing_ancestor(path: &Path) -> PathBuf {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::new();
    while !cursor.exists() {
        let Some(name) = cursor.file_name().map(|value| value.to_os_string()) else {
            return normalize_path(path);
        };
        suffix.push(name);
        let Some(parent) = cursor.parent() else {
            return normalize_path(path);
        };
        cursor = parent.to_path_buf();
    }
    let mut resolved = cursor
        .canonicalize()
        .unwrap_or_else(|_| normalize_path(&cursor));
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    normalize_path(&resolved)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}

fn expanduser_str(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn scalar_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(value) => value.clone(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::Null => "null".to_string(),
        other => other.to_string(),
    }
}
