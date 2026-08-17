use super::*;
use crate::json_support::{encode_string_or, parse_value_or};

pub(super) const WORKSPACE_IGNORE_FILE: &str = ".aitignore";
pub(super) const WORKTREE_CARGO_CONFIG_RELATIVE_PATH: &str = ".cargo/config.toml";
pub(super) const SHARED_CARGO_TARGET_DIRNAME: &str = "cargo-target";
pub(super) const SHARED_CARGO_BUILD_DIRNAME: &str = "cargo-build";
const CARGO_WORKSPACE_PATH_HASH_TEMPLATE: &str = "{workspace-path-hash}";
pub(super) const CANONICAL_CARGO_BUILD_DIRNAME: &str = "canonical";
pub(super) const MANAGED_WORKTREE_CARGO_TARGET_DIRNAME: &str = "task-workspaces";
pub(super) const MANAGED_WORKTREE_CARGO_BUILD_DIRNAME: &str = "task-workspaces";
const GENERATED_CARGO_CONFIG_HEADER: &str =
    "# Managed by ait: workspace-isolated final artifacts and intermediates.";
const SHARED_FINAL_ARTIFACT_GENERATED_CARGO_CONFIG_HEADER: &str =
    "# Managed by ait: stable final artifacts, workspace-isolated intermediates.";
const SOURCE_CARGO_CONFIG_HEADER: &str =
    "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.";
const REPOSITORY_SHARED_GENERATED_CARGO_CONFIG_HEADER: &str =
    "# Managed by ait: stable final artifacts, repository-shared intermediates.";
const WORKTREE_LOCAL_GENERATED_CARGO_CONFIG_HEADER: &str =
    "# Managed by ait: stable final artifacts, worktree-local intermediates.";
const LEGACY_GENERATED_CARGO_CONFIG_HEADER: &str =
    "# Managed by ait to share Rust build artifacts across task worktrees.";

pub(super) fn matches_generated_worktree_cargo_config_text(
    workspace_root: &Path,
    contents: &str,
) -> bool {
    if matches_source_worktree_cargo_config_text(contents) {
        return false;
    }
    upgrade_generated_worktree_cargo_config_text(workspace_root, contents).is_some()
}

pub(super) fn matches_source_worktree_cargo_config_text(contents: &str) -> bool {
    contents.lines().next() == Some(SOURCE_CARGO_CONFIG_HEADER)
}

fn matches_ait_managed_cargo_config_header(contents: &str) -> bool {
    matches!(
        contents.lines().next(),
        Some(
            GENERATED_CARGO_CONFIG_HEADER
                | SHARED_FINAL_ARTIFACT_GENERATED_CARGO_CONFIG_HEADER
                | SOURCE_CARGO_CONFIG_HEADER
                | REPOSITORY_SHARED_GENERATED_CARGO_CONFIG_HEADER
                | WORKTREE_LOCAL_GENERATED_CARGO_CONFIG_HEADER
                | LEGACY_GENERATED_CARGO_CONFIG_HEADER
        )
    )
}

pub(super) fn cargo_worktree_integration_enabled(
    repository_root: &Path,
    workspace_root: &Path,
) -> bool {
    let source_policy = repository_root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
    if fs::read_to_string(source_policy)
        .is_ok_and(|contents| matches_source_worktree_cargo_config_text(&contents))
    {
        return true;
    }

    let workspace_config = workspace_root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
    fs::read_to_string(workspace_config)
        .is_ok_and(|contents| matches_ait_managed_cargo_config_header(&contents))
}

pub(super) fn generated_worktree_cargo_config_text(workspace_root: &Path) -> String {
    let target_dir = worktree_cargo_target_dir(workspace_root);
    let build_dir = worktree_cargo_build_dir(workspace_root);
    let encoded_target_dir = encode_string_or(
        &target_dir.to_string_lossy(),
        &format!("\"{}\"", target_dir.display()),
    );
    let encoded_build_dir = encode_string_or(
        &build_dir.to_string_lossy(),
        &format!("\"{}\"", build_dir.display()),
    );
    format!(
        "{GENERATED_CARGO_CONFIG_HEADER}\n[build]\ntarget-dir = {encoded_target_dir}\nbuild-dir = {encoded_build_dir}\n"
    )
}

pub(super) fn upgrade_generated_worktree_cargo_config_text(
    workspace_root: &Path,
    contents: &str,
) -> Option<String> {
    upgrade_generated_worktree_cargo_config_text_with_additional_build_lines(
        workspace_root,
        contents,
        &[],
        &[],
    )
}

pub(super) fn upgrade_copied_main_seed_cargo_config_text(
    workspace_root: &Path,
    contents: &str,
) -> Option<String> {
    if !matches!(
        contents.lines().next(),
        Some(GENERATED_CARGO_CONFIG_HEADER | SHARED_FINAL_ARTIFACT_GENERATED_CARGO_CONFIG_HEADER)
    ) {
        return None;
    }
    let default_line = read_json_value(&workspace_root.join(".ait/config.json"))
        .get("default_line")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .unwrap_or_else(|| "main".to_string());
    let copied_seed_build_dir = shared_cargo_build_root(workspace_root)
        .join(MANAGED_WORKTREE_CARGO_BUILD_DIRNAME)
        .join(format!("{default_line}-seed"));
    let copied_seed_target_dir = shared_cargo_target_root(workspace_root)
        .join(MANAGED_WORKTREE_CARGO_TARGET_DIRNAME)
        .join(format!("{default_line}-seed"));
    let copied_seed_target_line = format!(
        "target-dir = {}",
        encode_string_or(
            &copied_seed_target_dir.to_string_lossy(),
            &format!("\"{}\"", copied_seed_target_dir.display()),
        )
    );
    let copied_seed_build_line = format!(
        "build-dir = {}",
        encode_string_or(
            &copied_seed_build_dir.to_string_lossy(),
            &format!("\"{}\"", copied_seed_build_dir.display()),
        )
    );
    upgrade_generated_worktree_cargo_config_text_with_additional_build_lines(
        workspace_root,
        contents,
        &[copied_seed_target_line],
        &[copied_seed_build_line],
    )
}

fn upgrade_generated_worktree_cargo_config_text_with_additional_build_lines(
    workspace_root: &Path,
    contents: &str,
    additional_target_lines: &[String],
    additional_build_lines: &[String],
) -> Option<String> {
    let current = generated_worktree_cargo_config_text(workspace_root);
    if contents == current {
        return Some(current);
    }

    let lines = contents.lines().collect::<Vec<_>>();
    if lines.len() < 3
        || ![
            GENERATED_CARGO_CONFIG_HEADER,
            SHARED_FINAL_ARTIFACT_GENERATED_CARGO_CONFIG_HEADER,
            SOURCE_CARGO_CONFIG_HEADER,
            REPOSITORY_SHARED_GENERATED_CARGO_CONFIG_HEADER,
            WORKTREE_LOCAL_GENERATED_CARGO_CONFIG_HEADER,
            LEGACY_GENERATED_CARGO_CONFIG_HEADER,
        ]
        .contains(&lines[0])
        || lines[1] != "[build]"
    {
        return None;
    }
    let trailing_section_start = lines
        .iter()
        .enumerate()
        .skip(2)
        .find_map(|(index, line)| {
            let trimmed = line.trim();
            (trimmed.starts_with('[') && trimmed.ends_with(']')).then_some(index)
        })
        .unwrap_or(lines.len());
    let mut build_section_end = trailing_section_start;
    while build_section_end > 2 && lines[build_section_end - 1].trim().is_empty() {
        build_section_end -= 1;
    }

    let target_dir = worktree_cargo_target_dir(workspace_root);
    let repository_shared_target_dir = shared_cargo_target_root(workspace_root);
    let build_dir = worktree_cargo_build_dir(workspace_root);
    let ait_dir = workspace_root.join(".ait");
    let shared_ait_dir = fs::canonicalize(&ait_dir).unwrap_or(ait_dir);
    let repository_shared_build_dir = {
        let candidate = shared_ait_dir.join(SHARED_CARGO_BUILD_DIRNAME);
        fs::canonicalize(&candidate).unwrap_or(candidate)
    };
    let repository_canonical_build_dir =
        repository_shared_build_dir.join(CANONICAL_CARGO_BUILD_DIRNAME);
    let target_line = format!(
        "target-dir = {}",
        encode_string_or(
            &target_dir.to_string_lossy(),
            &format!("\"{}\"", target_dir.display()),
        )
    );
    let relative_target_line = format!("target-dir = \".ait/{SHARED_CARGO_TARGET_DIRNAME}\"");
    let mut target_lines = vec![
        target_line.clone(),
        format!(
            "target-dir = {}",
            encode_string_or(
                &repository_shared_target_dir.to_string_lossy(),
                &format!("\"{}\"", repository_shared_target_dir.display()),
            )
        ),
        relative_target_line,
    ];
    target_lines.extend(additional_target_lines.iter().cloned());
    target_lines.sort();
    target_lines.dedup();
    let build_line = format!(
        "build-dir = {}",
        encode_string_or(
            &build_dir.to_string_lossy(),
            &format!("\"{}\"", build_dir.display()),
        )
    );
    let legacy_rust_build_dir = lexical_normalize(&workspace_root.join("rust/target"));
    let legacy_root_build_dir = lexical_normalize(&workspace_root.join("target"));
    let mut build_lines = vec![
        build_line.clone(),
        format!(
            "build-dir = {}",
            encode_string_or(
                &repository_shared_build_dir.to_string_lossy(),
                &format!("\"{}\"", repository_shared_build_dir.display()),
            )
        ),
        format!(
            "build-dir = {}",
            encode_string_or(
                &legacy_rust_build_dir.to_string_lossy(),
                &format!("\"{}\"", legacy_rust_build_dir.display()),
            )
        ),
        format!(
            "build-dir = {}",
            encode_string_or(
                &legacy_root_build_dir.to_string_lossy(),
                &format!("\"{}\"", legacy_root_build_dir.display()),
            )
        ),
        format!("build-dir = \".ait/{SHARED_CARGO_BUILD_DIRNAME}\""),
        format!(
            "build-dir = \".ait/{SHARED_CARGO_BUILD_DIRNAME}/{CANONICAL_CARGO_BUILD_DIRNAME}\""
        ),
        format!(
            "build-dir = {}",
            encode_string_or(
                &repository_canonical_build_dir.to_string_lossy(),
                &format!("\"{}\"", repository_canonical_build_dir.display()),
            )
        ),
        format!(
            "build-dir = \".ait/{SHARED_CARGO_BUILD_DIRNAME}/workspaces/{CARGO_WORKSPACE_PATH_HASH_TEMPLATE}\""
        ),
        "build-dir = \"rust/target\"".to_string(),
        "build-dir = \"target\"".to_string(),
    ];
    build_lines.extend(additional_build_lines.iter().cloned());
    let body = &lines[2..build_section_end];
    if body
        .iter()
        .filter(|line| target_lines.iter().any(|candidate| candidate == **line))
        .count()
        != 1
        || body
            .iter()
            .filter(|line| build_lines.iter().any(|candidate| candidate == **line))
            .count()
            > 1
    {
        return None;
    }
    let non_paths = body
        .iter()
        .filter(|line| {
            !target_lines.iter().any(|candidate| candidate == **line)
                && !build_lines.iter().any(|candidate| candidate == **line)
        })
        .copied()
        .collect::<Vec<_>>();
    if non_paths.len() > 1 {
        return None;
    }
    if let Some(line) = non_paths.first() {
        let (key, value) = line.split_once('=')?;
        if key.trim() != "jobs"
            || value.trim().is_empty()
            || !value.trim().chars().all(|ch| ch.is_ascii_digit())
        {
            return None;
        }
    }

    let mut upgraded = vec![
        GENERATED_CARGO_CONFIG_HEADER.to_string(),
        "[build]".to_string(),
    ];
    for line in body {
        if target_lines.iter().any(|candidate| candidate == *line) {
            upgraded.push(target_line.clone());
            upgraded.push(build_line.clone());
            continue;
        }
        if !build_lines.iter().any(|candidate| candidate == *line) {
            upgraded.push((*line).to_string());
        }
    }
    if trailing_section_start < lines.len() {
        upgraded.push(String::new());
        upgraded.extend(
            lines[trailing_section_start..]
                .iter()
                .map(|line| (*line).to_string()),
        );
    }
    Some(format!("{}\n", upgraded.join("\n")))
}

fn shared_cargo_target_root(workspace_root: &Path) -> PathBuf {
    let ait_dir = workspace_root.join(".ait");
    let shared_ait_dir = fs::canonicalize(&ait_dir).unwrap_or(ait_dir);
    lexical_normalize(&shared_ait_dir.join(SHARED_CARGO_TARGET_DIRNAME))
}

fn shared_cargo_build_root(workspace_root: &Path) -> PathBuf {
    let ait_dir = workspace_root.join(".ait");
    let shared_ait_dir = fs::canonicalize(&ait_dir).unwrap_or(ait_dir);
    let build_root = lexical_normalize(&shared_ait_dir.join(SHARED_CARGO_BUILD_DIRNAME));
    fs::canonicalize(&build_root).unwrap_or(build_root)
}

fn managed_worktree_name(workspace_root: &Path) -> Option<String> {
    let marker = read_json_value(&workspace_root.join(WORKTREE_CONFIG_NAME));
    let name = marker.get("worktree_name")?.as_str()?;
    normalize_worktree_name(name).ok()
}

pub(super) fn registered_worktree_cargo_target_dir(
    workspace_root: &Path,
    expected_name: &str,
) -> Option<PathBuf> {
    let name = managed_worktree_name(workspace_root)?;
    (name == expected_name).then(|| {
        shared_cargo_target_root(workspace_root)
            .join(MANAGED_WORKTREE_CARGO_TARGET_DIRNAME)
            .join(name)
    })
}

pub(super) fn worktree_cargo_target_dir(workspace_root: &Path) -> PathBuf {
    if let Some(name) = managed_worktree_name(workspace_root) {
        return shared_cargo_target_root(workspace_root)
            .join(MANAGED_WORKTREE_CARGO_TARGET_DIRNAME)
            .join(name);
    }
    shared_cargo_target_root(workspace_root)
}

pub(super) fn registered_worktree_cargo_build_dir(
    workspace_root: &Path,
    expected_name: &str,
) -> Option<PathBuf> {
    let name = managed_worktree_name(workspace_root)?;
    (name == expected_name).then(|| {
        shared_cargo_build_root(workspace_root)
            .join(MANAGED_WORKTREE_CARGO_BUILD_DIRNAME)
            .join(name)
    })
}

pub(super) fn worktree_cargo_build_dir(workspace_root: &Path) -> PathBuf {
    if let Some(name) = managed_worktree_name(workspace_root) {
        return shared_cargo_build_root(workspace_root)
            .join(MANAGED_WORKTREE_CARGO_BUILD_DIRNAME)
            .join(name);
    }
    shared_cargo_build_root(workspace_root).join(CANONICAL_CARGO_BUILD_DIRNAME)
}

pub(super) fn cargo_build_repo_segment(repo_name: &str) -> String {
    let mut normalized = String::new();
    let mut pending_separator = false;
    for ch in repo_name.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
            if pending_separator && !normalized.is_empty() {
                normalized.push('-');
            }
            pending_separator = false;
            normalized.push(ch.to_ascii_lowercase());
        } else {
            pending_separator = true;
        }
    }
    while normalized.ends_with('-') {
        normalized.pop();
    }
    if normalized.is_empty() {
        "repo".to_string()
    } else {
        normalized
    }
}

pub(super) fn repository_ram_cargo_build_dir(memory_root: &Path, repo_name: &str) -> PathBuf {
    lexical_normalize(
        &memory_root
            .join("ait-runtime")
            .join(SHARED_CARGO_BUILD_DIRNAME)
            .join(cargo_build_repo_segment(repo_name)),
    )
}

pub(super) fn system_event_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(super) fn read_json_value(path: &std::path::Path) -> JsonValue {
    let Ok(content) = fs::read_to_string(path) else {
        return json!({});
    };
    parse_value_or(&content, json!({}))
}

pub(super) fn change_uses_local_scope(
    repo: &RepoRuntime,
    local_requested: bool,
    remote_requested: Option<&str>,
) -> Result<bool, String> {
    if local_requested && normalized_text(remote_requested).is_some() {
        return Err("--local cannot be combined with --remote".to_string());
    }
    if local_requested {
        return Ok(true);
    }
    if normalized_text(remote_requested).is_some() {
        return Ok(false);
    }
    Ok(repo.change_uses_local_scope(false, None))
}

pub(super) fn local_snapshot_exists(repo: &RepoRuntime, snapshot_id: &str) -> Result<bool, String> {
    let store = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    store.snapshot_exists(snapshot_id)
}
