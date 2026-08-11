use super::*;

const CORE_SOURCE_FIXED_PATHS: &[&str] = &[
    "rust/Cargo.toml",
    "rust/Cargo.lock",
    "rust/crates/ait-agent-core/Cargo.toml",
    "rust/crates/ait-agent-worker/Cargo.toml",
    "rust/crates/ait-cli/Cargo.toml",
    "rust/crates/ait-core/Cargo.toml",
    "rust/crates/ait-py/Cargo.toml",
];
const CORE_SOURCE_DIRECTORIES: &[&str] = &[
    "rust/crates/ait-agent-core/src",
    "rust/crates/ait-agent-worker/src",
    "rust/crates/ait-cli/src",
    "rust/crates/ait-core/src",
    "rust/crates/ait-py/src",
];

pub fn current_core_source_identity(
    core_repo_root: &Path,
) -> Result<CurrentSourceIdentity, String> {
    let store = FilesystemCurrentSourceNativeCacheSourceStore;
    current_core_source_identity_with_source_store(&store, core_repo_root)
}

pub(super) fn current_core_source_identity_with_source_store<S>(
    store: &S,
    core_repo_root: &Path,
) -> Result<CurrentSourceIdentity, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    source_identity_with_source_store(
        store,
        core_repo_root,
        CORE_SOURCE_FIXED_PATHS,
        CORE_SOURCE_DIRECTORIES,
        false,
    )
}

pub fn current_core_source_fingerprint(core_repo_root: &Path) -> Result<String, String> {
    let store = FilesystemCurrentSourceNativeCacheSourceStore;
    current_core_source_fingerprint_with_source_store(&store, core_repo_root)
}

pub(super) fn current_core_source_fingerprint_with_source_store<S>(
    store: &S,
    core_repo_root: &Path,
) -> Result<String, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    source_fingerprint_with_source_store(
        store,
        core_repo_root,
        CORE_SOURCE_FIXED_PATHS,
        CORE_SOURCE_DIRECTORIES,
        false,
    )
}

pub fn current_server_source_fingerprint(server_core_repo_root: &Path) -> Result<String, String> {
    let store = FilesystemCurrentSourceNativeCacheSourceStore;
    current_server_source_fingerprint_with_source_store(&store, server_core_repo_root)
}

pub(super) fn current_server_source_fingerprint_with_source_store<S>(
    store: &S,
    server_core_repo_root: &Path,
) -> Result<String, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    source_fingerprint_with_source_store(
        store,
        server_core_repo_root,
        &["rust/Cargo.toml", "rust/crates/ait-server-core/Cargo.toml"],
        &["rust/crates/ait-server-core/src"],
        true,
    )
}

pub fn current_core_source_mtime_ns(core_repo_root: &Path) -> Result<u64, String> {
    let store = FilesystemCurrentSourceNativeCacheSourceStore;
    current_core_source_mtime_ns_with_source_store(&store, core_repo_root)
}

pub(super) fn current_core_source_mtime_ns_with_source_store<S>(
    store: &S,
    core_repo_root: &Path,
) -> Result<u64, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    source_mtime_ns_with_source_store(
        store,
        core_repo_root,
        CORE_SOURCE_FIXED_PATHS,
        CORE_SOURCE_DIRECTORIES,
        false,
    )
}

pub fn current_server_source_mtime_ns(server_core_repo_root: &Path) -> Result<u64, String> {
    let store = FilesystemCurrentSourceNativeCacheSourceStore;
    current_server_source_mtime_ns_with_source_store(&store, server_core_repo_root)
}

pub(super) fn current_server_source_mtime_ns_with_source_store<S>(
    store: &S,
    server_core_repo_root: &Path,
) -> Result<u64, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    source_mtime_ns_with_source_store(
        store,
        server_core_repo_root,
        &["rust/Cargo.toml", "rust/crates/ait-server-core/Cargo.toml"],
        &["rust/crates/ait-server-core/src"],
        true,
    )
}

pub(super) fn source_fingerprint_with_source_store<S>(
    store: &S,
    repo_root: &Path,
    fixed_rel_paths: &[&str],
    source_dirs: &[&str],
    require_candidates: bool,
) -> Result<String, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    let root = resolve_path_with_current_source_native_cache_source_store(store, repo_root);
    let candidates = source_inputs_with_source_store(
        store,
        &root,
        fixed_rel_paths,
        source_dirs,
        require_candidates,
    )?;
    let mut digest = Sha256::new();
    for path in candidates {
        let rel = path
            .strip_prefix(&root)
            .map_err(|_| format!("Source input escaped repo root: {}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        digest.update(rel.as_bytes());
        digest.update(b"\0");
        let bytes = read_source_file_with_current_source_native_cache_source_store(store, &path)?;
        digest.update(bytes);
        digest.update(b"\0");
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn source_identity_with_source_store<S>(
    store: &S,
    repo_root: &Path,
    fixed_rel_paths: &[&str],
    source_dirs: &[&str],
    require_candidates: bool,
) -> Result<CurrentSourceIdentity, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    let root = resolve_path_with_current_source_native_cache_source_store(store, repo_root);
    let candidates = source_inputs_with_source_store(
        store,
        &root,
        fixed_rel_paths,
        source_dirs,
        require_candidates,
    )?;
    if candidates.is_empty() {
        return Err(format!(
            "External repo {} does not provide current-source Rust inputs.",
            root.display()
        ));
    }

    let mut digest = Sha256::new();
    let mut source_mtime_ns = 0_u64;
    for path in candidates {
        source_mtime_ns = source_mtime_ns.max(
            path_mtime_ns_with_current_source_native_cache_source_store(store, &path)?,
        );
        let rel = path
            .strip_prefix(&root)
            .map_err(|_| format!("Source input escaped repo root: {}", path.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        digest.update(rel.as_bytes());
        digest.update(b"\0");
        let bytes = read_source_file_with_current_source_native_cache_source_store(store, &path)?;
        digest.update(bytes);
        digest.update(b"\0");
    }
    Ok(CurrentSourceIdentity {
        source_mtime_ns,
        source_fingerprint: format!("{:x}", digest.finalize()),
    })
}

pub(super) fn source_mtime_ns_with_source_store<S>(
    store: &S,
    repo_root: &Path,
    fixed_rel_paths: &[&str],
    source_dirs: &[&str],
    require_candidates: bool,
) -> Result<u64, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    let root = resolve_path_with_current_source_native_cache_source_store(store, repo_root);
    let candidates = source_inputs_with_source_store(
        store,
        &root,
        fixed_rel_paths,
        source_dirs,
        require_candidates,
    )?;
    if require_candidates && candidates.is_empty() {
        return Err(format!(
            "External repo {} does not provide current-source Rust inputs.",
            root.display()
        ));
    }
    candidates
        .iter()
        .map(|path| path_mtime_ns_with_current_source_native_cache_source_store(store, path))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .ok_or_else(|| {
            format!(
                "External repo {} does not provide current-source Rust inputs.",
                root.display()
            )
        })
}

pub(super) fn source_inputs_with_source_store<S>(
    store: &S,
    root: &Path,
    fixed_rel_paths: &[&str],
    source_dirs: &[&str],
    require_candidates: bool,
) -> Result<Vec<PathBuf>, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    let mut candidates = Vec::new();
    for rel in fixed_rel_paths {
        let path = root.join(rel);
        if path_exists_with_current_source_native_cache_source_store(store, &path) {
            candidates.push(path);
        }
    }
    for rel_dir in source_dirs {
        let dir = root.join(rel_dir);
        if path_is_dir_with_current_source_native_cache_source_store(store, &dir) {
            let mut entries = Vec::new();
            collect_rust_source_files_with_source_store(store, &dir, &mut entries)?;
            entries.sort();
            candidates.extend(entries);
        }
    }
    if require_candidates && candidates.is_empty() {
        return Err(format!(
            "External repo {} does not provide current-source Rust inputs.",
            root.display()
        ));
    }
    Ok(candidates)
}

pub(super) fn collect_rust_source_files_with_source_store<S>(
    store: &S,
    dir: &Path,
    entries: &mut Vec<PathBuf>,
) -> Result<(), String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    for entry in read_source_dir_with_current_source_native_cache_source_store(store, dir)? {
        let path = entry.path;
        match entry.kind {
            CurrentSourceNativeCacheSourceEntryKind::Directory => {
                collect_rust_source_files_with_source_store(store, &path, entries)?;
            }
            CurrentSourceNativeCacheSourceEntryKind::File
                if path.extension().and_then(|value| value.to_str()) == Some("rs") =>
            {
                entries.push(path);
            }
            _ => {}
        }
    }
    Ok(())
}
