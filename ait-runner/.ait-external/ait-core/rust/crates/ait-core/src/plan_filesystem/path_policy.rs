use super::*;

pub fn normalize_markdown_artifact_path(path_value: &str) -> String {
    let replaced = path_value.replace('\\', "/");
    if replaced.is_empty() {
        ".".to_string()
    } else {
        Path::new(&replaced)
            .to_string_lossy()
            .trim_matches('/')
            .to_string()
    }
}

pub fn is_markdown_artifact_path(path_value: &str) -> bool {
    let path = normalize_markdown_artifact_path(path_value);
    if path.is_empty() {
        return false;
    }
    Path::new(&path)
        .extension()
        .and_then(OsStr::to_str)
        .map(|value| value.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

pub fn is_lineage_only_markdown_artifact_path(path_value: &str) -> bool {
    let normalized = normalize_markdown_artifact_path(path_value);
    is_markdown_artifact_path(&normalized)
}

pub fn path_is_projected_out_for_workspace(
    _repo_root: &str,
    rel_path: &str,
    is_worktree: bool,
) -> bool {
    let normalized = normalize_markdown_artifact_path(rel_path);
    if is_worktree && (normalized == "docs" || normalized.starts_with("docs/")) {
        return true;
    }
    if is_lineage_only_markdown_artifact_path(&normalized) {
        return true;
    }
    false
}

pub fn workspace_path_is_ignored(
    repo_root: &str,
    path_value: &str,
    ignore_rules_text: Option<&str>,
) -> Result<bool, PlanFilesystemError> {
    let root = canonical_root(repo_root)?;
    let rel_path = normalized_relative_path(&root, path_value)?;
    let matcher = load_workspace_ignore_matcher(&root, ignore_rules_text)?;
    Ok(workspace_relative_path_is_ignored_with_matcher(
        rel_path.to_string_lossy().as_ref(),
        &matcher,
    ))
}

pub fn parse_workspace_ignore_matcher(ignore_rules_text: &str) -> WorkspaceIgnoreMatcher {
    WorkspaceIgnoreMatcher {
        rules: parse_workspace_ignore_rules(ignore_rules_text),
    }
}

pub fn workspace_relative_path_is_ignored_with_matcher(
    path_value: &str,
    matcher: &WorkspaceIgnoreMatcher,
) -> bool {
    let rel_path = PathBuf::from(path_value.replace('\\', "/"));
    workspace_path_is_ignored_for_rules(&rel_path, &matcher.rules)
}

pub fn resolve_repo_artifact_path(
    repo_root: &str,
    path_value: &str,
    allow_missing: bool,
) -> Result<JsonValue, PlanFilesystemError> {
    let root = canonical_root(repo_root)?;
    let raw = expanduser_path(path_value);
    let resolved = if raw.is_absolute() {
        resolve_path_strict_false(&raw)
    } else {
        resolve_path_strict_false(&root.join(raw))
    };
    if !allow_missing && !resolved.exists() {
        return Err(PlanFilesystemError::Invalid(format!(
            "Path does not exist: {}",
            path_value
        )));
    }
    let artifact_path = resolved
        .strip_prefix(&root)
        .map_err(|_| {
            PlanFilesystemError::Invalid(format!(
                "Path must live inside the repository root: {}",
                path_value
            ))
        })?
        .to_string_lossy()
        .replace('\\', "/");
    let artifact_path = if artifact_path.is_empty() {
        ".".to_string()
    } else {
        artifact_path
    };
    if artifact_path == ".ait" || artifact_path.starts_with(".ait/") {
        return Err(PlanFilesystemError::Invalid(
            "Plan artifacts must be authored repository Markdown files, not runtime metadata under `.ait/`."
                .to_string(),
        ));
    }
    Ok(JsonValue::Object(Map::from_iter([
        (
            "resolved_path".to_string(),
            JsonValue::String(resolved.to_string_lossy().to_string()),
        ),
        (
            "artifact_path".to_string(),
            JsonValue::String(artifact_path),
        ),
    ])))
}
