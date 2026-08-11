use super::*;

pub(crate) fn active_workspace_runtime_root(repo_root: &Path) -> Option<String> {
    let raw = std::env::var("AIT_RUNTIME_DATA").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = expanduser_path(trimmed);
    let resolved = if expanded.is_absolute() {
        resolve_path_strict_false(&expanded)
    } else {
        resolve_path_strict_false(&repo_root.join(expanded))
    };
    if resolved == *repo_root || !resolved.starts_with(repo_root) {
        return None;
    }
    resolved
        .strip_prefix(repo_root)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .filter(|value| !value.is_empty())
}

pub(crate) fn workspace_ignore_policy(
    repo_root: &Path,
    runtime_root: Option<&str>,
    external_materialization_roots: &[String],
) -> JsonValue {
    let mut operational_roots = vec![".ait".to_string(), ".ait-runtime".to_string()];
    let mut external_roots = external_materialization_roots.to_vec();
    let mut runtime_roots = Vec::new();
    if let Some(value) = runtime_root {
        runtime_roots.push(value.to_string());
        operational_roots.push(value.to_string());
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
        "dir_names": [".ait", ".ait-runtime", ".git", "__pycache__", ".pytest_cache", ".venv", "venv", ".mypy_cache"],
        "file_names": [".DS_Store", ".ait-worktree.json"],
        "operational_roots": operational_roots,
        "external_materialization_roots": external_roots,
        "runtime_roots": runtime_roots,
    });
    if !custom_patterns.is_empty() {
        let obj = payload.as_object_mut().expect("ignore policy payload");
        obj.insert(
            "rule_files".to_string(),
            JsonValue::Array(vec![JsonValue::String(".aitignore".to_string())]),
        );
        obj.insert(
            "custom_patterns".to_string(),
            JsonValue::Array(custom_patterns.into_iter().map(JsonValue::String).collect()),
        );
    }
    payload
}

pub(in crate::local_snapshot) fn load_workspace_ignore_rule_sources(
    repo_root: &Path,
) -> Vec<String> {
    let path = repo_root.join(".aitignore");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(parse_workspace_ignore_rule_source)
        .collect()
}

pub(in crate::local_snapshot) fn parse_workspace_ignore_rule_source(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    Some(trimmed.to_string())
}
