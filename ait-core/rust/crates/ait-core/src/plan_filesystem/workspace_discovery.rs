use super::*;

pub fn list_visible_workspace_paths(
    repo_root: &str,
    ignore_rules_text: Option<&str>,
    runtime_root: Option<&str>,
) -> Result<Vec<String>, PlanFilesystemError> {
    Ok(list_visible_workspace_entries(repo_root, ignore_rules_text, runtime_root)?.files)
}

pub fn list_visible_workspace_entries(
    repo_root: &str,
    ignore_rules_text: Option<&str>,
    runtime_root: Option<&str>,
) -> Result<VisibleWorkspaceEntries, PlanFilesystemError> {
    let root = canonical_root(repo_root)?;
    let rules = load_workspace_ignore_rules(&root, ignore_rules_text)?;
    let runtime_root = runtime_root
        .map(|value| normalized_runtime_root(&root, value))
        .transpose()?;
    let operational_external_roots = {
        let _range =
            crate::perfetto_range!("ait.core.workspace_discovery.external_projection_roots");
        operational_external_materialization_roots_for_root(&root)
    };
    let projected_external_roots = operational_external_roots
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let mut output = VisibleWorkspaceEntries {
        files: Vec::new(),
        file_metadata: BTreeMap::new(),
        directories: vec![String::new()],
        operational_external_roots,
    };
    {
        let _range = crate::perfetto_range!("ait.core.workspace_discovery.physical_walk");
        walk_visible_entries(
            &root,
            &root,
            &rules,
            runtime_root.as_deref(),
            &projected_external_roots,
            &mut output,
        )?;
    }
    output.files.sort();
    output.directories.sort();
    output.directories.dedup();
    output.operational_external_roots.sort();
    output.operational_external_roots.dedup();
    Ok(output)
}

pub fn operational_external_materialization_roots(
    repo_root: &str,
) -> Result<Vec<String>, PlanFilesystemError> {
    let root = canonical_root(repo_root)?;
    Ok(operational_external_materialization_roots_for_root(&root))
}

pub fn list_visible_markdown_artifact_paths(
    repo_root: &str,
    ignore_rules_text: Option<&str>,
    runtime_root: Option<&str>,
) -> Result<Vec<String>, PlanFilesystemError> {
    let visible = list_visible_workspace_paths(repo_root, ignore_rules_text, runtime_root)?;
    Ok(visible
        .into_iter()
        .filter(|path| is_markdown_artifact_path(path))
        .collect())
}

pub(super) fn workspace_relative_string(
    root: &Path,
    path: &Path,
) -> Result<String, PlanFilesystemError> {
    let normalized = lexical_normalize(path);
    let rel = normalized
        .strip_prefix(root)
        .map_err(|_| {
            PlanFilesystemError::Invalid(format!(
                "Workspace entry escaped repository root: {}",
                path.display()
            ))
        })?
        .to_path_buf();
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

pub(super) fn operational_external_materialization_roots_for_root(root: &Path) -> Vec<String> {
    let repo_name = root
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("repo")
        .to_string();
    let Ok(mut roots) = inspect_operational_external_projection_roots(root, repo_name) else {
        return Vec::new();
    };
    roots = roots
        .into_iter()
        .filter_map(|materialize_to| normalized_external_projection_root(root, &materialize_to))
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

pub(super) fn normalized_external_projection_root(
    root: &Path,
    materialize_to: &str,
) -> Option<String> {
    let normalized = lexical_normalize(&root.join(materialize_to));
    if normalized == *root || !normalized.starts_with(root) {
        return None;
    }
    let rel = normalized.strip_prefix(root).ok()?;
    let text = rel.to_string_lossy().replace('\\', "/");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

pub(super) fn path_is_under_projected_root(path: &Path, projected_roots: &[PathBuf]) -> bool {
    projected_roots
        .iter()
        .any(|root| path == root || path.starts_with(root))
}

pub(super) fn walk_visible_entries(
    root: &Path,
    dir: &Path,
    rules: &[WorkspaceIgnoreRule],
    runtime_root: Option<&Path>,
    projected_external_roots: &[PathBuf],
    output: &mut VisibleWorkspaceEntries,
) -> Result<(), PlanFilesystemError> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|err| io_error_for_path("scan workspace", dir, err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| io_error_for_path("scan workspace", dir, err))?;
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_string());
    for entry in entries {
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| io_error_for_path("inspect workspace entry", &path, err))?;
        let is_dir = metadata.file_type().is_dir();
        let is_symlink = metadata.file_type().is_symlink();
        if IGNORED_DIRS
            .iter()
            .any(|value| value == &file_name.as_str())
        {
            continue;
        }
        if is_dir {
            if is_symlink {
                continue;
            }
            if let Some(runtime_root) = runtime_root {
                let normalized = lexical_normalize(&path);
                if normalized.starts_with(runtime_root) {
                    continue;
                }
            }
            let rel = lexical_normalize(&path)
                .strip_prefix(root)
                .map_err(|_| {
                    PlanFilesystemError::Invalid(format!(
                        "Workspace entry escaped repository root: {}",
                        path.display()
                    ))
                })?
                .to_path_buf();
            if path_is_under_projected_root(&rel, projected_external_roots) {
                continue;
            }
            let ignored_dir = workspace_path_is_ignored_for_rules_with_kind(&rel, rules, true);
            if ignored_dir && !ignored_directory_may_contain_negated_match(&rel, rules) {
                continue;
            }
            if !ignored_dir {
                output
                    .directories
                    .push(workspace_relative_string(root, &path)?);
            }
            walk_visible_entries(
                root,
                &path,
                rules,
                runtime_root,
                projected_external_roots,
                output,
            )?;
            continue;
        }
        if !metadata.file_type().is_file() && !is_symlink {
            continue;
        }
        if IGNORED_FILES
            .iter()
            .any(|value| value == &file_name.as_str())
        {
            continue;
        }
        let normalized = lexical_normalize(&path);
        if let Some(runtime_root) = runtime_root {
            if normalized.starts_with(runtime_root) {
                continue;
            }
        }
        let rel = normalized
            .strip_prefix(root)
            .map_err(|_| {
                PlanFilesystemError::Invalid(format!(
                    "Workspace entry escaped repository root: {}",
                    path.display()
                ))
            })?
            .to_path_buf();
        if path_is_under_projected_root(&rel, projected_external_roots) {
            continue;
        }
        if is_generated_worktree_cargo_config(root, &rel, &path) {
            continue;
        }
        if workspace_path_is_ignored_for_rules(&rel, rules) {
            continue;
        }
        let rel = rel.to_string_lossy().replace('\\', "/");
        output.file_metadata.insert(
            rel.clone(),
            visible_workspace_file_metadata_from_metadata(&metadata),
        );
        output.files.push(rel);
    }
    Ok(())
}

#[cfg(unix)]
fn visible_workspace_file_metadata_from_metadata(
    metadata: &fs::Metadata,
) -> VisibleWorkspaceFileMetadata {
    use std::os::unix::fs::MetadataExt;

    VisibleWorkspaceFileMetadata {
        file_kind: if metadata.file_type().is_symlink() {
            "symlink"
        } else if metadata.is_file() {
            "file"
        } else {
            "other"
        }
        .to_string(),
        size_bytes: metadata.len(),
        mode_bits: metadata.mode(),
        modified_ns: unix_time_parts_ns(metadata.mtime(), metadata.mtime_nsec()),
        changed_ns: unix_time_parts_ns(metadata.ctime(), metadata.ctime_nsec()),
        device_id: metadata.dev(),
        file_id: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn visible_workspace_file_metadata_from_metadata(
    metadata: &fs::Metadata,
) -> VisibleWorkspaceFileMetadata {
    VisibleWorkspaceFileMetadata {
        file_kind: if metadata.file_type().is_symlink() {
            "symlink"
        } else if metadata.is_file() {
            "file"
        } else {
            "other"
        }
        .to_string(),
        size_bytes: metadata.len(),
        mode_bits: if metadata.permissions().readonly() {
            0o444
        } else {
            0o644
        },
        modified_ns: metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
            .unwrap_or_default(),
        changed_ns: 0,
        device_id: 0,
        file_id: 0,
    }
}

#[cfg(unix)]
fn unix_time_parts_ns(seconds: i64, nanos: i64) -> u64 {
    if seconds < 0 || nanos < 0 {
        return 0;
    }
    (seconds as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(nanos as u64)
}

pub(super) fn worktree_cargo_target_dir(root: &Path) -> PathBuf {
    let ait_dir = root.join(".ait");
    let shared_ait_dir = fs::canonicalize(&ait_dir).unwrap_or(ait_dir);
    lexical_normalize(&shared_ait_dir.join(SHARED_CARGO_TARGET_DIRNAME))
}

pub(super) fn worktree_cargo_build_dir(root: &Path) -> PathBuf {
    let ait_dir = root.join(".ait");
    let shared_ait_dir = fs::canonicalize(&ait_dir).unwrap_or(ait_dir);
    let build_root = lexical_normalize(&shared_ait_dir.join(SHARED_CARGO_BUILD_DIRNAME));
    let build_root = fs::canonicalize(&build_root).unwrap_or(build_root);
    if let Some(name) = managed_worktree_name(root) {
        return build_root
            .join(MANAGED_WORKTREE_CARGO_BUILD_DIRNAME)
            .join(name);
    }
    build_root
        .join("workspaces")
        .join(CARGO_WORKSPACE_PATH_HASH_TEMPLATE)
}

fn managed_worktree_name(root: &Path) -> Option<String> {
    let contents = fs::read_to_string(root.join(WORKTREE_CONFIG_NAME)).ok()?;
    let payload = JsonCodec::parse_value(&contents, "worktree marker").ok()?;
    let name = payload.get("worktree_name")?.as_str()?.trim();
    if name.is_empty() || matches!(name, "." | "..") || name.contains('/') || name.contains('\\') {
        return None;
    }
    Some(name.to_string())
}

pub(super) fn encoded_cargo_path(path: &Path) -> String {
    JsonCodec::encode_value(
        &JsonValue::String(path.to_string_lossy().to_string()),
        JsonEncodeOptions::compact(),
    )
    .unwrap_or_else(|_| format!("\"{}\"", path.display()))
}

pub(super) fn generated_worktree_cargo_config_text(root: &Path) -> String {
    let target_dir = worktree_cargo_target_dir(root);
    let build_dir = worktree_cargo_build_dir(root);
    format!(
        "{GENERATED_CARGO_CONFIG_HEADER}\n[build]\ntarget-dir = {}\nbuild-dir = {}\n",
        encoded_cargo_path(&target_dir),
        encoded_cargo_path(&build_dir),
    )
}

fn matches_generated_worktree_cargo_config_text(root: &Path, contents: &str) -> bool {
    if contents == generated_worktree_cargo_config_text(root) {
        return true;
    }
    let lines = contents.lines().collect::<Vec<_>>();
    if lines.len() < 3
        || ![
            GENERATED_CARGO_CONFIG_HEADER,
            REPOSITORY_SHARED_GENERATED_CARGO_CONFIG_HEADER,
            WORKTREE_LOCAL_GENERATED_CARGO_CONFIG_HEADER,
            LEGACY_GENERATED_CARGO_CONFIG_HEADER,
        ]
        .contains(&lines[0])
        || lines[1] != "[build]"
    {
        return false;
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

    let target_dir = worktree_cargo_target_dir(root);
    let target_lines = [
        format!("target-dir = {}", encoded_cargo_path(&target_dir)),
        format!("target-dir = \".ait/{SHARED_CARGO_TARGET_DIRNAME}\""),
    ];
    let ait_dir = root.join(".ait");
    let shared_ait_dir = fs::canonicalize(&ait_dir).unwrap_or(ait_dir);
    let repository_shared_build_dir = {
        let candidate = shared_ait_dir.join(SHARED_CARGO_BUILD_DIRNAME);
        fs::canonicalize(&candidate).unwrap_or(candidate)
    };
    let build_lines = [
        format!(
            "build-dir = {}",
            encoded_cargo_path(&worktree_cargo_build_dir(root))
        ),
        format!(
            "build-dir = {}",
            encoded_cargo_path(&repository_shared_build_dir)
        ),
        format!(
            "build-dir = {}",
            encoded_cargo_path(&lexical_normalize(&root.join("rust/target")))
        ),
        format!(
            "build-dir = {}",
            encoded_cargo_path(&lexical_normalize(&root.join("target")))
        ),
        format!("build-dir = \".ait/{SHARED_CARGO_BUILD_DIRNAME}\""),
        format!(
            "build-dir = \".ait/{SHARED_CARGO_BUILD_DIRNAME}/workspaces/{CARGO_WORKSPACE_PATH_HASH_TEMPLATE}\""
        ),
        "build-dir = \"rust/target\"".to_string(),
        "build-dir = \"target\"".to_string(),
    ];
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
        return false;
    }
    let non_paths = body
        .iter()
        .filter(|line| {
            !target_lines.iter().any(|candidate| candidate == **line)
                && !build_lines.iter().any(|candidate| candidate == **line)
        })
        .collect::<Vec<_>>();
    if non_paths.len() > 1 {
        return false;
    }
    non_paths.first().is_none_or(|line| {
        line.split_once('=').is_some_and(|(key, value)| {
            key.trim() == "jobs"
                && !value.trim().is_empty()
                && value.trim().chars().all(|ch| ch.is_ascii_digit())
        })
    })
}

pub(crate) fn is_generated_worktree_cargo_config(root: &Path, rel: &Path, path: &Path) -> bool {
    is_generated_worktree_cargo_config_with_file_io_store(&FilesystemFileIoStore, root, rel, path)
}

pub(super) fn is_generated_worktree_cargo_config_with_file_io_store<S>(
    store: &S,
    root: &Path,
    rel: &Path,
    path: &Path,
) -> bool
where
    S: FileIoStore + ?Sized,
{
    if !store.path_exists(&root.join(WORKTREE_CONFIG_NAME)) {
        return false;
    }
    if rel.to_string_lossy().replace('\\', "/") != WORKTREE_CARGO_CONFIG_RELATIVE_PATH {
        return false;
    }
    let Ok(contents) = store.read_to_string(path) else {
        return false;
    };
    matches_generated_worktree_cargo_config_text(root, &contents)
}
