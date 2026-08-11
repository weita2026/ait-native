use super::*;

fn managed_cargo_projection_matches_baseline(
    workspace_root: &Path,
    baseline_source: &str,
    projection: &[u8],
) -> bool {
    upgrade_generated_worktree_cargo_config_text(workspace_root, baseline_source).is_some_and(
        |expected| {
            expected.as_bytes() == projection
                || generated_worktree_cargo_config_text(workspace_root).as_bytes() == projection
        },
    )
}

pub(super) fn workspace_state(
    repo: &RepoRuntime,
    ignore_rules_text: Option<&str>,
) -> Result<BTreeMap<String, WorkspaceFileState>, String> {
    let workspace_root = repo.workspace_root();
    let runtime_root = active_workspace_runtime_root(&workspace_root);
    let visible_paths = list_visible_workspace_paths(
        workspace_root.to_string_lossy().as_ref(),
        ignore_rules_text,
        runtime_root.as_deref(),
    )
    .map_err(|err| format!("{err:?}"))?;
    let mut state = BTreeMap::new();
    let workspace_root_text = workspace_root.to_string_lossy().to_string();
    for rel in visible_paths {
        if path_is_projected_out_for_workspace(&workspace_root_text, &rel, repo.is_worktree()) {
            continue;
        }
        let abs_path = workspace_root.join(&rel);
        let metadata = abs_path.metadata().map_err(|err| err.to_string())?;
        if !metadata.is_file() {
            continue;
        }
        let data = fs::read(&abs_path).map_err(|err| err.to_string())?;
        state.insert(
            rel,
            WorkspaceFileState {
                sha256: sha256_hex_bytes(&data),
                mode: format!("{:#o}", metadata.permissions().mode() & 0o777),
            },
        );
    }
    Ok(state)
}

pub(super) fn workspace_state_for_exact_paths(
    repo: &RepoRuntime,
    paths: &[String],
) -> Result<BTreeMap<String, WorkspaceFileState>, String> {
    let workspace_root = repo.workspace_root();
    let workspace_root_text = workspace_root.to_string_lossy().to_string();
    let mut state = BTreeMap::new();
    for rel in paths {
        if path_is_projected_out_for_workspace(&workspace_root_text, rel, repo.is_worktree()) {
            continue;
        }
        let abs_path = workspace_root.join(rel);
        let metadata = match abs_path.metadata() {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(format!("Failed to read metadata for {rel}: {err}")),
        };
        if !metadata.is_file() {
            continue;
        }
        let data = fs::read(&abs_path)
            .map_err(|err| format!("Failed to read exact workspace path {rel}: {err}"))?;
        state.insert(
            rel.clone(),
            WorkspaceFileState {
                sha256: sha256_hex_bytes(&data),
                mode: format!("{:#o}", metadata.permissions().mode() & 0o777),
            },
        );
    }
    Ok(state)
}

pub(super) fn workspace_state_for_status(
    repo: &RepoRuntime,
    snapshot_id: &str,
    _ignore_rules_hash: &str,
    snapshot_index: &SnapshotTreeManifestIndex,
    ignore_rules_text: Option<&str>,
    supplied_cache: Option<&WorkspaceHashCacheLoad>,
) -> Result<WorkspaceStatusScan, String> {
    const WORKSPACE_STATUS_HASH_WORKERS: usize = 9;

    let workspace_root = repo.workspace_root();
    let locally_loaded_cache = {
        let _range = perfetto_range!("ait.cli.status.hash_cache_read");
        supplied_cache
            .is_none()
            .then(|| load_workspace_hash_cache(&workspace_root, snapshot_id))
    };
    let cache_load = supplied_cache
        .or(locally_loaded_cache.as_ref())
        .expect("status cache is supplied or loaded locally");
    let runtime_root = active_workspace_runtime_root(&workspace_root);
    let entries = {
        let _range = perfetto_range!("ait.cli.status.workspace_walk");
        list_visible_workspace_entries(
            workspace_root.to_string_lossy().as_ref(),
            ignore_rules_text,
            runtime_root.as_deref(),
        )
        .map_err(|err| format!("{err:?}"))?
    };
    let mut visible_paths = entries.files;
    let visible_file_metadata = entries.file_metadata;
    let operational_external_roots = entries.operational_external_roots;
    let mut state = BTreeMap::new();
    let mut tracked_fingerprints = BTreeMap::new();
    let mut tracked_files = Vec::new();
    let mut authoritative_row_by_path_id = vec![None; snapshot_index.paths.len()];
    for (row_index, row) in snapshot_index.rows.iter().enumerate() {
        let slot = authoritative_row_by_path_id
            .get_mut(row.path_id as usize)
            .ok_or_else(|| format!("Snapshot manifest path id {} is out of range.", row.path_id))?;
        if slot.replace(row_index).is_some() {
            return Err(format!(
                "Snapshot manifest contains duplicate path id {}.",
                row.path_id
            ));
        }
    }
    let mut reused_paths = 0_usize;
    if repo.is_worktree() {
        let cargo_config_path = workspace_root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
        let cargo_config_metadata = match fs::symlink_metadata(&cargo_config_path) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "Failed to read metadata for {}: {error}",
                    WORKTREE_CARGO_CONFIG_RELATIVE_PATH
                ));
            }
        };
        if cargo_config_metadata.is_some() {
            let fingerprint_before = workspace_file_fingerprint(&cargo_config_path)?;
            let contents = fs::read(&cargo_config_path).map_err(|error| {
                format!(
                    "Failed to read {}: {error}",
                    WORKTREE_CARGO_CONFIG_RELATIVE_PATH
                )
            })?;
            let fingerprint_after = workspace_file_fingerprint(&cargo_config_path)?;
            if fingerprint_after != fingerprint_before {
                return Err(format!(
                    "Workspace path {} changed while status was reading it; retry status.",
                    WORKTREE_CARGO_CONFIG_RELATIVE_PATH
                ));
            }
            let contents_text = std::str::from_utf8(&contents).ok();
            let is_generated_regular_file = fingerprint_after.file_kind == "file"
                && contents_text.is_some_and(|contents| {
                    matches_generated_worktree_cargo_config_text(&workspace_root, contents)
                });
            let baseline_row = snapshot_index
                .path_id_by_path
                .get(WORKTREE_CARGO_CONFIG_RELATIVE_PATH)
                .and_then(|path_id| {
                    authoritative_row_by_path_id
                        .get(*path_id as usize)
                        .and_then(|row_index| *row_index)
                })
                .and_then(|row_index| snapshot_index.rows.get(row_index));
            let projection_matches_baseline = if is_generated_regular_file {
                if let Some(row) = baseline_row {
                    let blob_id = snapshot_index.row_blob_id(row)?;
                    let source = read_selected_snapshot_blob_bytes(repo, blob_id)?;
                    std::str::from_utf8(&source).ok().is_some_and(|source| {
                        managed_cargo_projection_matches_baseline(
                            &workspace_root,
                            source,
                            &contents,
                        )
                    })
                } else {
                    false
                }
            } else {
                false
            };
            if projection_matches_baseline {
                let row = baseline_row.expect("matching projection requires a baseline row");
                let mode = format!("{:#o}", fingerprint_after.mode_bits & 0o777);
                state.insert(
                    WORKTREE_CARGO_CONFIG_RELATIVE_PATH.to_string(),
                    WorkspaceFileState {
                        sha256: row.sha256.clone(),
                        mode,
                    },
                );
                tracked_fingerprints.insert(
                    WORKTREE_CARGO_CONFIG_RELATIVE_PATH.to_string(),
                    fingerprint_after,
                );
                visible_paths.retain(|path| path != WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
                reused_paths = reused_paths.saturating_add(1);
            } else if (!is_generated_regular_file || baseline_row.is_some())
                && !visible_paths
                    .iter()
                    .any(|path| path == WORKTREE_CARGO_CONFIG_RELATIVE_PATH)
            {
                visible_paths.push(WORKTREE_CARGO_CONFIG_RELATIVE_PATH.to_string());
            }
        }
    }
    let workspace_root_text = workspace_root.to_string_lossy().to_string();
    let _metadata_range = perfetto_range!("ait.cli.status.metadata_cache_match");
    for rel in visible_paths {
        if path_is_projected_out_for_workspace(&workspace_root_text, &rel, repo.is_worktree())
            && !(repo.is_worktree() && rel == WORKTREE_CARGO_CONFIG_RELATIVE_PATH)
        {
            continue;
        }
        let abs_path = workspace_root.join(&rel);
        let fingerprint = if let Some(metadata) = visible_file_metadata.get(&rel) {
            workspace_file_fingerprint_from_visible_metadata(metadata)
        } else {
            match workspace_file_fingerprint(&abs_path) {
                Ok(fingerprint) => fingerprint,
                Err(_error) if !abs_path.exists() => continue,
                Err(error) => return Err(error),
            }
        };
        let is_symlink = fingerprint.file_kind == "symlink";
        let is_tracked = snapshot_index.path_id_by_path.contains_key(rel.as_str());
        let permission_bits = fingerprint.mode_bits & 0o777;
        let mode_bits = if is_symlink {
            0o120000 | permission_bits
        } else {
            permission_bits
        };
        let mode = format!("{:#o}", mode_bits);
        if is_tracked {
            tracked_fingerprints.insert(rel.clone(), fingerprint.clone());
            if let Some(cached_entry) = cache_load
                .cache()
                .and_then(|cache| cache.entries.get(&rel))
                .filter(|entry| entry.fingerprint == fingerprint)
                .filter(|entry| {
                    snapshot_index
                        .path_id_by_path
                        .get(&rel)
                        .and_then(|path_id| {
                            authoritative_row_by_path_id
                                .get(*path_id as usize)
                                .and_then(|index| *index)
                        })
                        .and_then(|index| snapshot_index.rows.get(index))
                        .is_some_and(|row| {
                            snapshot_index.row_blob_id(row).is_ok_and(|blob_id| {
                                blob_id == entry.blob_id
                                    && row.sha256 == entry.sha256
                                    && row.size_bytes >= 0
                                    && row.size_bytes as u64 == entry.size_bytes
                                    && row.mode == entry.mode
                            })
                        })
                })
            {
                state.insert(
                    rel,
                    WorkspaceFileState {
                        sha256: cached_entry.sha256.clone(),
                        mode: cached_entry.mode.clone(),
                    },
                );
                reused_paths = reused_paths.saturating_add(1);
            } else {
                tracked_files.push((rel, abs_path, mode, is_symlink, fingerprint));
            }
        } else {
            state.insert(
                rel,
                WorkspaceFileState {
                    sha256: String::new(),
                    mode,
                },
            );
        }
    }
    #[cfg(feature = "perfetto-tracing")]
    drop(_metadata_range);
    let rehashed_paths = tracked_files.len();
    if !tracked_files.is_empty() {
        let _hash_range = perfetto_range!("ait.cli.status.hash_changed_paths");
        let worker_count = WORKSPACE_STATUS_HASH_WORKERS.min(tracked_files.len());
        let chunk_size = tracked_files.len().div_ceil(worker_count);
        let hashed_files = std::thread::scope(|scope| -> Result<Vec<_>, String> {
            let mut workers = Vec::with_capacity(worker_count);
            for chunk in tracked_files.chunks(chunk_size) {
                workers.push(scope.spawn(move || -> Result<Vec<_>, String> {
                    chunk
                        .iter()
                        .map(|(rel, abs_path, mode, is_symlink, fingerprint_before)| {
                            let data = if *is_symlink {
                                let target = fs::read_link(abs_path).map_err(|err| {
                                    format!(
                                        "Failed to read symlink {}: {err}",
                                        abs_path.display()
                                    )
                                })?;
                                #[cfg(unix)]
                                {
                                    target.as_os_str().as_bytes().to_vec()
                                }
                                #[cfg(windows)]
                                {
                                    target
                                        .to_str()
                                        .ok_or_else(|| {
                                            format!(
                                                "Windows symlink target is not valid Unicode: {}",
                                                target.display()
                                            )
                                        })?
                                        .as_bytes()
                                        .to_vec()
                                }
                            } else {
                                fs::read(abs_path).map_err(|err| {
                                    format!("Failed to read {}: {err}", abs_path.display())
                                })?
                            };
                            let fingerprint_after = workspace_file_fingerprint(abs_path)?;
                            if &fingerprint_after != fingerprint_before {
                                return Err(format!(
                                    "Workspace path {rel} changed while status was reading it; retry status."
                                ));
                            }
                            Ok((
                                rel.clone(),
                                WorkspaceFileState {
                                    sha256: sha256_hex_bytes(&data),
                                    mode: mode.clone(),
                                },
                            ))
                        })
                        .collect::<Result<Vec<_>, String>>()
                }));
            }
            let mut rows = Vec::with_capacity(tracked_files.len());
            for worker in workers {
                let mut chunk = worker
                    .join()
                    .map_err(|_| "Workspace status hash worker panicked.".to_string())??;
                rows.append(&mut chunk);
            }
            Ok(rows)
        })?;
        state.extend(hashed_files);
    }
    Ok(WorkspaceStatusScan {
        files: state,
        tracked_fingerprints,
        operational_external_roots,
        reused_paths,
        rehashed_paths,
        cache_read: cache_load.state().to_string(),
    })
}

pub(super) fn repair_workspace_hash_cache_after_clean_status(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    baseline_manifest: &StatusBaselineManifest,
    tracked_fingerprints: &BTreeMap<String, WorkspaceFileFingerprint>,
    clean: bool,
) -> Result<String, String> {
    if baseline_manifest
        .hash_cache
        .as_ref()
        .is_some_and(|cache| matches!(cache, WorkspaceHashCacheLoad::Hit(_)))
    {
        return Ok("read_only".to_string());
    }
    let Some(snapshot_id) = snapshot_id.and_then(|value| normalized_text(Some(value))) else {
        return Ok("read_only".to_string());
    };
    let Some(root_tree_id) = baseline_manifest.root_tree_id.as_deref() else {
        return Ok("read_only".to_string());
    };
    if !clean {
        return Ok("skipped_dirty".to_string());
    }
    let mut cache_entries = Vec::with_capacity(baseline_manifest.index.rows.len());
    for row in &baseline_manifest.index.rows {
        let path = baseline_manifest.index.row_path(row)?;
        let blob_id = baseline_manifest.index.row_blob_id(row)?;
        let size_bytes = u64::try_from(row.size_bytes).map_err(|_| {
            format!(
                "Snapshot manifest entry {path} has negative size {}.",
                row.size_bytes
            )
        })?;
        let Some(fingerprint) = tracked_fingerprints.get(path) else {
            return Ok("skipped_incomplete".to_string());
        };
        cache_entries.push(workspace_hash_cache_entry(
            path,
            blob_id,
            &row.sha256,
            size_bytes,
            &row.mode,
            fingerprint.clone(),
        ));
    }
    let _range = perfetto_range!("ait.cli.status.hash_cache_write");
    Ok(match write_workspace_hash_cache(
        &repo.workspace_root(),
        &snapshot_id,
        root_tree_id,
        cache_entries,
    ) {
        Ok(_) => "written",
        Err(_) => "skipped_validation_failed",
    }
    .to_string())
}

fn status_snapshot_file_row_visible(
    repo: &RepoRuntime,
    row: &SnapshotFileRow,
    ignore_matcher: Option<&WorkspaceIgnoreMatcher>,
) -> Result<bool, String> {
    if !repo.is_worktree() || row.path != WORKTREE_CARGO_CONFIG_RELATIVE_PATH {
        return snapshot_file_row_visible(repo, row, ignore_matcher);
    }
    let workspace_root = repo.workspace_root();
    let workspace_root_text = workspace_root.to_string_lossy().to_string();
    if path_is_projected_out_for_workspace(&workspace_root_text, &row.path, true) {
        return Ok(false);
    }
    Ok(!ignore_matcher
        .map(|matcher| workspace_relative_path_is_ignored_with_matcher(&row.path, matcher))
        .unwrap_or(false))
}

pub(super) fn status_baseline_manifest(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    ignore_rules_text: Option<&str>,
    _ignore_rules_hash: &str,
) -> Result<StatusBaselineManifest, String> {
    let Some(snapshot_id) = snapshot_id.and_then(|value| normalized_text(Some(value))) else {
        return Ok(StatusBaselineManifest {
            index: SnapshotTreeManifestIndex::from_file_rows(Vec::new())?,
            source: "empty".to_string(),
            manifest_path: None,
            root_tree_id: None,
            hash_cache: None,
        });
    };
    let workspace_root = repo.workspace_root();
    let mut hash_cache = {
        let _range = perfetto_range!("ait.cli.status.hash_cache_read");
        load_workspace_hash_cache(&workspace_root, &snapshot_id)
    };
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let authoritative_root = store.snapshot_tree_root_locator(&snapshot_id)?;
    let cache_root_matches = hash_cache
        .cache()
        .is_some_and(|cache| cache.root_tree_id == authoritative_root.root_tree_id);
    if cache_root_matches {
        let ignore_matcher = ignore_rules_text.map(parse_workspace_ignore_matcher);
        let mut rows = Vec::with_capacity(
            hash_cache
                .cache()
                .map(|cache| cache.entries.len())
                .unwrap_or_default(),
        );
        for entry in hash_cache
            .cache()
            .expect("matching cache root requires a loaded cache")
            .entries
            .values()
        {
            let size_bytes = i64::try_from(entry.size_bytes).map_err(|_| {
                format!(
                    "Workspace hash cache entry {} exceeds Snapshot size limits.",
                    entry.path
                )
            })?;
            let row = SnapshotFileRow {
                path: entry.path.clone(),
                blob_id: entry.blob_id.clone(),
                size_bytes,
                mode: entry.mode.clone(),
                sha256: entry.sha256.clone(),
            };
            if status_snapshot_file_row_visible(repo, &row, ignore_matcher.as_ref())? {
                rows.push(row);
            }
        }
        return Ok(StatusBaselineManifest {
            index: SnapshotTreeManifestIndex::from_file_rows(rows)?,
            source: "validated_workspace_hash_cache".to_string(),
            manifest_path: None,
            root_tree_id: Some(authoritative_root.root_tree_id),
            hash_cache: Some(hash_cache),
        });
    }
    if matches!(hash_cache, WorkspaceHashCacheLoad::Hit(_)) {
        hash_cache = WorkspaceHashCacheLoad::Invalid(
            "Workspace hash cache root does not match the authoritative Snapshot root".to_string(),
        );
    }
    let ignore_matcher = ignore_rules_text.map(parse_workspace_ignore_matcher);
    let rows = store
        .snapshot_tree_file_rows(Some(snapshot_id.as_str()))?
        .into_iter()
        .filter_map(|row| {
            match status_snapshot_file_row_visible(repo, &row, ignore_matcher.as_ref()) {
                Ok(true) => Some(Ok(row)),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(StatusBaselineManifest {
        index: SnapshotTreeManifestIndex::from_file_rows(rows)?,
        source: "snapshot_binary_metadata".to_string(),
        manifest_path: None,
        root_tree_id: Some(authoritative_root.root_tree_id),
        hash_cache: Some(hash_cache),
    })
}

pub(super) fn effective_ignore_rules_text(
    repo: &RepoRuntime,
    snapshot_rules_text: Option<&str>,
) -> Result<Option<String>, String> {
    let mut parts = Vec::new();
    let ignore_path = repo.workspace_root().join(".aitignore");
    if ignore_path.is_file() {
        let text = fs::read_to_string(&ignore_path)
            .map_err(|err| format!("Failed to read {}: {err}", ignore_path.display()))?;
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(text) = snapshot_rules_text {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if repo.is_worktree() {
        parts.push("/docs/".to_string());
    }
    if parts.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parts.join("\n") + "\n"))
    }
}

pub(super) fn ignore_policy_payload(
    ignore_rules_text: Option<&str>,
    runtime_root: Option<&str>,
    external_materialization_roots: &[String],
) -> JsonValue {
    let mut operational_roots = vec![".ait".to_string(), ".ait-runtime".to_string()];
    let mut external_roots = external_materialization_roots.to_vec();
    let mut runtime_roots = Vec::new();
    if let Some(runtime_root) = runtime_root.and_then(|value| normalized_text(Some(value))) {
        runtime_roots.push(runtime_root.clone());
        operational_roots.push(runtime_root);
    }
    operational_roots.extend(external_roots.iter().cloned());
    operational_roots.sort();
    operational_roots.dedup();
    external_roots.sort();
    external_roots.dedup();
    runtime_roots.sort();
    runtime_roots.dedup();
    let custom_patterns = ignore_rules_text
        .map(parse_ignore_rule_sources)
        .unwrap_or_default();
    let mut payload = json!({
        "dir_names": [".ait", ".ait-runtime", ".ait-worktree", ".ait-worktree-links", ".git", "__pycache__", ".pytest_cache", ".venv", "venv", ".mypy_cache"],
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

pub(super) fn parse_ignore_rule_sources(text: &str) -> Vec<String> {
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

pub(super) fn active_workspace_runtime_root(workspace_root: &Path) -> Option<String> {
    let raw = std::env::var("AIT_RUNTIME_DATA").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = expanduser_path(trimmed);
    let resolved = if expanded.is_absolute() {
        resolve_path_strict_false(&expanded)
    } else {
        resolve_path_strict_false(&workspace_root.join(expanded))
    };
    if resolved == *workspace_root || !resolved.starts_with(workspace_root) {
        return None;
    }
    resolved
        .strip_prefix(workspace_root)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .filter(|value| !value.is_empty())
}

pub(super) fn expanduser_path(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

pub(super) fn resolve_path_strict_false(path: &Path) -> PathBuf {
    let normalized = lexical_normalize(path);
    if let Ok(canonical) = normalized.canonicalize() {
        return canonical;
    }
    let mut cursor = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        if cursor.exists() {
            if let Ok(canonical_parent) = cursor.canonicalize() {
                let mut resolved = canonical_parent;
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return lexical_normalize(&resolved);
            }
        }
        let Some(file_name) = cursor.file_name() else {
            return normalized;
        };
        missing.push(file_name.to_os_string());
        let Some(parent) = cursor.parent() else {
            return normalized;
        };
        cursor = parent;
    }
}

pub(super) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => output.push(prefix.as_os_str()),
            Component::RootDir => output.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            Component::Normal(part) => output.push(part),
        }
    }
    output
}

pub(super) fn normalize_workspace_restore_path(path: &str) -> Result<String, String> {
    let text = path.replace('\\', "/").trim().to_string();
    if text.is_empty() || text == "." || text.starts_with('/') {
        return Err(format!("Restore path must be workspace-relative: {path}"));
    }
    let mut normalized = PathBuf::new();
    for component in Path::new(&text).components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "Restore path must stay within the workspace: {path}"
                ));
            }
        }
    }
    let rel = normalized.to_string_lossy().replace('\\', "/");
    if rel.is_empty() || rel == "." || rel.starts_with("../") || rel.contains("/../") {
        return Err(format!(
            "Restore path must stay within the workspace: {path}"
        ));
    }
    Ok(rel)
}

pub(super) fn normalize_workspace_diff_paths(paths: &[String]) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for path in paths {
        let text = path.replace('\\', "/").trim().to_string();
        if text.is_empty() || text == "." || text.starts_with('/') {
            return Err(format!("Diff path must be workspace-relative: {path}"));
        }
        let mut rel_path = PathBuf::new();
        for component in Path::new(&text).components() {
            match component {
                Component::Normal(part) => rel_path.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(format!("Diff path must stay within the workspace: {path}"));
                }
            }
        }
        let rel = rel_path.to_string_lossy().replace('\\', "/");
        if rel.is_empty() || rel == "." || rel.starts_with("../") || rel.contains("/../") {
            return Err(format!("Diff path must stay within the workspace: {path}"));
        }
        normalized.push(rel);
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

pub(super) fn workspace_diff_path_selected(path: &str, filters: &BTreeSet<String>) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|filter| path == filter || path.starts_with(&format!("{filter}/")))
}

pub(super) fn read_workspace_file_bytes(repo: &RepoRuntime, path: &str) -> Result<Vec<u8>, String> {
    fs::read(repo.workspace_root().join(path))
        .map_err(|err| format!("Failed to read workspace file `{path}`: {err}"))
}

pub(super) fn sort_paths(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

pub(super) fn delta_summary_paths(
    status_by_path: &BTreeMap<String, String>,
    expected_status: &str,
) -> Vec<String> {
    status_by_path
        .iter()
        .filter(|(_, status)| status.as_str() == expected_status)
        .map(|(path, _)| path.clone())
        .collect()
}

pub(super) fn reverse_depth_sort_paths(mut values: Vec<String>) -> Vec<String> {
    values.sort_by_key(|item| (item.matches('/').count(), item.clone()));
    values.reverse();
    values
}

pub(super) fn summarize_path_sample(paths: &[String]) -> String {
    let mut sample = paths.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    if paths.len() > 5 {
        sample.push_str(", ...");
    }
    sample
}

pub(super) fn parse_mode_bits(mode: Option<&str>) -> Result<u32, String> {
    let text = mode.unwrap_or("0o644").trim();
    if let Some(octal) = text.strip_prefix("0o") {
        return u32::from_str_radix(octal, 8).map_err(|_| format!("Invalid mode value: {text}"));
    }
    text.parse::<u32>()
        .or_else(|_| u32::from_str_radix(text, 8))
        .map_err(|_| format!("Invalid mode value: {text}"))
}

pub(super) fn prune_empty_parent_dirs(root: &Path, path: &Path) -> Result<(), String> {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir == root || !dir.exists() {
            break;
        }
        match fs::remove_dir(dir) {
            Ok(()) => current = dir.parent(),
            Err(err) if err.kind() == std::io::ErrorKind::DirectoryNotEmpty => break,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => return Err(err.to_string()),
        }
    }
    Ok(())
}

pub(super) fn file_map_row_blob_id(row: &JsonValue) -> Option<String> {
    row.get("blob_id")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
}

pub(super) fn file_map_row_sha256(row: &JsonValue) -> Option<String> {
    row.get("sha256")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
}

pub(super) fn file_map_row_mode(row: &JsonValue) -> Option<String> {
    row.get("mode")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
}

pub(super) fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(super) fn elapsed_ms(started: Instant) -> f64 {
    ((started.elapsed().as_secs_f64() * 1000.0) * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_managed_cargo_projection_matches_alias_bearing_source_baseline() {
        let temp = tempfile::TempDir::new().unwrap();
        let baseline = "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/workspaces/{workspace-path-hash}\"\n\n[alias]\npatch-ci-build = [\"test\", \"--profile\", \"ait-ci\"]\n";
        let minimal = generated_worktree_cargo_config_text(temp.path());
        let alias_bearing =
            upgrade_generated_worktree_cargo_config_text(temp.path(), baseline).unwrap();

        assert!(managed_cargo_projection_matches_baseline(
            temp.path(),
            baseline,
            minimal.as_bytes(),
        ));
        assert!(managed_cargo_projection_matches_baseline(
            temp.path(),
            baseline,
            alias_bearing.as_bytes(),
        ));
        assert!(!managed_cargo_projection_matches_baseline(
            temp.path(),
            baseline,
            format!("{minimal}# manual edit\n").as_bytes(),
        ));
        assert!(!managed_cargo_projection_matches_baseline(
            temp.path(),
            "[build]\ntarget-dir = \"custom\"\n",
            minimal.as_bytes(),
        ));
    }
}
