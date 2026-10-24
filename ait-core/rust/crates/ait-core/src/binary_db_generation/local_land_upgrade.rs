//! Bounded metadata-only workflow admission under native writer exclusion.
use super::generation_activation::{
    configured_repo_name, fingerprint_direct_authority_contents, validate_direct_authority,
    validate_layout1_upgrade_file,
};
use super::local_change_cleanup::Conversion;
use super::*;
use crate::binary_db::{
    AuthorityId, BinaryDbReadLockSet, LocalBinaryDbFs, LocalStateScope,
    REPOSITORY_BINARY_DB_BIN_PATHS, REPOSITORY_BINARY_DB_INDEX_PATHS,
};
use crate::workflow_binary_db::BinaryDbWorkflowStore;
use serde::Serialize;
use std::fs;
use std::io::Read;

pub const LOCAL_LAND_V1_SOURCE_SELECTOR: &str = "local-u64-workflow-v1";
#[derive(Clone, Debug)]
pub struct StageLocalLandUpgradeOptions {
    pub repo_root: PathBuf,
    pub output_root: PathBuf,
    pub source_layout: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct StageLocalLandUpgradeReport {
    pub status: String,
    pub repo_name: String,
    pub source_authority_fingerprint: String,
    pub content_fingerprint: String,
    pub task_count: u64,
    pub receipt_count: u64,
    pub generation_root: PathBuf,
}
// The retired payload is readable only while admitting the exact legacy source.
const fn legacy_bin_paths() -> [&'static str; REPOSITORY_BINARY_DB_BIN_PATHS.len() + 1] {
    let mut paths = [""; REPOSITORY_BINARY_DB_BIN_PATHS.len() + 1];
    let mut index = 0;
    while index < REPOSITORY_BINARY_DB_BIN_PATHS.len() {
        paths[index] = REPOSITORY_BINARY_DB_BIN_PATHS[index];
        index += 1;
    }
    paths[index] = "snapshot_link_payload.bin";
    paths
}
pub(super) const LEGACY_LOCAL_U64_BIN_PATHS: &[&str] = &legacy_bin_paths();
fn database(authority: &Path, repo: &Path) -> LocalBinaryDbFs {
    LocalBinaryDbFs::new(
        authority.to_path_buf(),
        repo.to_path_buf(),
        AuthorityId::new("local-land-upgrade"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(LEGACY_LOCAL_U64_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS)
    .for_detached_generation_without_locks()
}
fn header(path: &Path) -> GenerationResult<Option<u32>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.file_type().is_file() {
        return Err("Local receipt authority must be a direct file".into());
    }
    let mut raw = [0; 4];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut raw))
        .map_err(|e| e.to_string())?;
    Ok(Some(u32::from_le_bytes(raw)))
}
fn count(path: &Path, width: u64) -> GenerationResult<u64> {
    match fs::metadata(path) {
        Ok(m) if m.len() >= 4 && (m.len() - 4) % width == 0 => Ok((m.len() - 4) / width),
        Ok(_) => Err(format!("Unsupported source width: {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e.to_string()),
    }
}
pub fn stage_local_land_upgrade(
    options: StageLocalLandUpgradeOptions,
) -> GenerationResult<StageLocalLandUpgradeReport> {
    let _coordination = crate::binary_db::BinaryDbCommandLockSet::try_acquire(
        &super::generation_activation::binary_db_activation_lock_root(&options.repo_root),
        crate::binary_db::BinaryDbCommandScope::General,
    )
    .map_err(|e| format!("Local receipt upgrade coordination is busy: {e}"))?
    .ok_or("Local receipt upgrade coordination required: native writers are active")?;
    stage_coordinated(options)
}

// This offline helper and runtime admission share the metadata preparation. It
// is deliberately not a self-contained content generation or recovery input.
fn stage_coordinated(
    options: StageLocalLandUpgradeOptions,
) -> GenerationResult<StageLocalLandUpgradeReport> {
    if options.source_layout != LOCAL_LAND_V1_SOURCE_SELECTOR {
        return Err(format!(
            "Only exact source selector {LOCAL_LAND_V1_SOURCE_SELECTOR} is supported"
        ));
    }
    let repo = options
        .repo_root
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let name = configured_repo_name(&repo)?;
    if !matches!(name.as_str(), "ait-core" | "ait-runner") {
        return Err("Known Local format conversion uses existing runtime admission, not an offline generation".into());
    }
    let authority = repo.join(".ait/binary-db");
    let output = resolve_output_root(&options.output_root)?;
    for protected in [
        &authority,
        &repo.join(".ait/objects"),
        &repo.join(super::generation_git::GIT_DIRECTORY),
    ] {
        if output.starts_with(protected) {
            return Err("Upgrade generation root must be separate from source authority, packs and Git evidence".into());
        }
    }
    require_direct_authority(&authority)?;
    let _read =
        BinaryDbReadLockSet::try_acquire(&crate::binary_db::StorePath::from(authority.clone()))
            .map_err(|e| e.to_string())?;
    let existed = output.exists();
    let current = header(&authority.join("land.bin"))? == Some(2)
        && header(&authority.join("snapshot_link.bin"))? == Some(2);
    let fingerprint = fingerprint_direct_authority_contents(&authority)?;
    if current {
        validate_current(&authority, &repo, &name)?;
    } else {
        validate_source(&authority, &repo, &name)?;
        let conversion = Conversion::build(&authority, &name)?;
        if output.exists() {
            validate_exact_metadata(&authority, &output.join("local"), &name, &conversion)?;
            validate_direct_authority(&output.join("local"))?;
            BinaryDbWorkflowStore::<_, 1>::new(database(&output.join("local"), &repo), &name)
                .validate_detached_authority()
                .map_err(|e| e.to_string())?;
        } else {
            prepare_metadata(&authority, &repo, &output.join("local"), &name, &conversion)?;
        }
    }
    if fingerprint_direct_authority_contents(&authority)? != fingerprint {
        return Err("Local receipt upgrade source drifted before target publication".into());
    }
    Ok(StageLocalLandUpgradeReport {
        status: if current {
            "already_upgraded"
        } else if existed {
            "staged_retry"
        } else {
            "staged"
        }
        .into(),
        repo_name: name,
        source_authority_fingerprint: fingerprint.clone(),
        content_fingerprint: if current {
            fingerprint
        } else {
            fingerprint_direct_authority_contents(&output.join("local"))?
        },
        task_count: count(&authority.join("task.bin"), 64)?,
        receipt_count: count(&authority.join("land.bin"), 44)?,
        generation_root: output,
    })
}

fn require_direct_authority(authority: &Path) -> GenerationResult<()> {
    if !fs::symlink_metadata(authority)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_dir()
    {
        return Err("Local receipt upgrade requires the current direct authority".into());
    }
    Ok(())
}

pub(super) fn validate_current(authority: &Path, repo: &Path, name: &str) -> GenerationResult<()> {
    validate_direct_authority(authority)?;
    BinaryDbWorkflowStore::<_, 1>::new(database(authority, repo), name)
        .validate_detached_authority()
        .map_err(|e| e.to_string())?;
    super::local_upgrade_cursors::validate_current(repo, authority)
}

// Whole-directory publication requires unchanged metadata in the replacement.
// Objects, configuration, Git and unrelated worktree files stay at their roots.
fn prepare_metadata(
    source: &Path,
    repo: &Path,
    target: &Path,
    name: &str,
    conversion: &Conversion,
) -> GenerationResult<()> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name() == ".locks"
            || super::local_change_cleanup::CHANGED_FILES
                .iter()
                .any(|name| entry.file_name() == *name)
        {
            continue;
        }
        fs::copy(entry.path(), target.join(entry.file_name())).map_err(|e| e.to_string())?;
    }
    conversion.write(target)?;
    let changes = disposed_change_bytes(source, name, conversion)?;
    if changes != conversion.files["change.bin"] {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(target.join("change.bin"))
            .map_err(|e| e.to_string())?;
        file.write_all(&changes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    validate_exact_metadata(source, target, name, conversion)?;
    validate_direct_authority(target)?;
    BinaryDbWorkflowStore::<_, 1>::new(database(target, repo), name)
        .validate_detached_authority()
        .map_err(|e| e.to_string())?;
    for entry in fs::read_dir(target).map_err(|e| e.to_string())? {
        fs::OpenOptions::new()
            .write(true)
            .open(entry.map_err(|e| e.to_string())?.path())
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    super::generation_capture::sync_directory(target)
}

pub(super) fn validate_source(authority: &Path, repo: &Path, name: &str) -> GenerationResult<()> {
    if !matches!(header(&authority.join("land.bin"))?, None | Some(1))
        || !matches!(
            header(&authority.join("snapshot_link.bin"))?,
            None | Some(1)
        )
    {
        return Err("Exact upgrade source must have local Land44 at layout 1".into());
    }
    for entry in fs::read_dir(authority).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let leaf = entry
            .file_name()
            .into_string()
            .map_err(|_| "Non-UTF8 source path")?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if leaf == ".locks" && kind.is_dir() {
            continue;
        }
        if !kind.is_file()
            || (!LEGACY_LOCAL_U64_BIN_PATHS.contains(&leaf.as_str())
                && !REPOSITORY_BINARY_DB_INDEX_PATHS.contains(&leaf.as_str()))
        {
            return Err(format!("Unsupported current-layout source member: {leaf}"));
        }
        validate_layout1_upgrade_file(
            &entry.path(),
            &format!("local/{leaf}"),
            entry.metadata().map_err(|e| e.to_string())?.len(),
        )?;
    }
    BinaryDbWorkflowStore::<_, 1>::new(database(authority, repo), name)
        .for_local_land_upgrade()
        .validate_detached_authority()
        .map_err(|e| e.to_string())?;
    if count(&authority.join("change.bin"), 68)? != 0 {
        let config: serde_json::Value = serde_json::from_slice(
            &fs::read(repo.join(".ait/config.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let namespace = if name == "ait-runner" { "R" } else { "C" };
        if config["id_namespace_prefix"].as_str() != Some(namespace) {
            return Err(
                "Legacy cohort has an unsupported workflow namespace; coordination required".into(),
            );
        }
    }
    Conversion::build(authority, name)?;
    Ok(())
}

fn equal_files(left: &Path, right: &Path) -> GenerationResult<bool> {
    let mut left = fs::File::open(left).map_err(|error| error.to_string())?;
    let mut right = fs::File::open(right).map_err(|error| error.to_string())?;
    if left.metadata().map_err(|error| error.to_string())?.len()
        != right.metadata().map_err(|error| error.to_string())?.len()
    {
        return Ok(false);
    }
    let mut a = [0u8; 8192];
    let mut b = [0u8; 8192];
    loop {
        let n = left.read(&mut a).map_err(|error| error.to_string())?;
        if n == 0 {
            return Ok(true);
        }
        right
            .read_exact(&mut b[..n])
            .map_err(|error| error.to_string())?;
        if a[..n] != b[..n] {
            return Ok(false);
        }
    }
}

// This bounded disposition follows the pure conversion. Source publication
// evidence and historical times remain intact; only current Change edges clear.
fn disposed_change_bytes(
    source: &Path,
    name: &str,
    conversion: &Conversion,
) -> GenerationResult<Vec<u8>> {
    let mut bytes = conversion
        .files
        .get("change.bin")
        .ok_or("Publication disposition requires the converted Change family")?
        .clone();
    if name != "ait-core" || !conversion.core_history {
        return Ok(bytes);
    }
    let edges = conversion
        .changes
        .iter()
        .enumerate()
        .filter(|(_, row)| [2, 53].contains(&row.task_index) && row.change_ordinal == 1)
        .collect::<Vec<_>>();
    if edges.is_empty() {
        return Ok(bytes);
    }
    if edges.len() != 2 || edges[0].1.task_index == edges[1].1.task_index {
        return Err("Publication disposition requires both exact retained legacy edges".into());
    }
    let tasks = match conversion.sources.get("task.bin") {
        Some(bytes) => bytes.clone(),
        None => fs::read(source.join("task.bin")).map_err(|e| e.to_string())?,
    };
    for (old, row) in edges {
        let task_at = 4 + row.task_index as usize * 64;
        let task = tasks
            .get(task_at..task_at + 64)
            .ok_or("Publication disposition Task is missing")?;
        let remote = if row.task_index == 2 { 981u32 } else { 787 };
        if tasks.get(..4) != Some(1u32.to_le_bytes().as_slice())
            || task[1] & 1 == 0
            || task[20..24] != remote.to_le_bytes()
            || row.change_meta & 3 != 3
            || row.change_state != 1
            || row.local_meta != 1
            || row.published_remote_change_ordinal_plus1 != 1
            || row.published_at_s == 0
        {
            return Err("Publication disposition legacy ownership/publication changed".into());
        }
        let new = conversion
            .mapping
            .get(old)
            .copied()
            .flatten()
            .ok_or("Publication disposition retained Change was removed")?;
        let at = 4 + new as usize * 68;
        let target = bytes
            .get_mut(at..at + 68)
            .ok_or("Publication disposition converted Change is missing")?;
        if target[16..20] != row.task_index.to_le_bytes()
            || target[4] != 0
            || target[1] != 1
            || target[28] != 1
            || target[48..56] != row.published_at_s.to_le_bytes()
        {
            return Err("Publication disposition converted ownership/publication changed".into());
        }
        target[1] = 0;
        target[28] = 0;
    }
    Ok(bytes)
}

fn validate_exact_metadata(
    source: &Path,
    target: &Path,
    name: &str,
    conversion: &Conversion,
) -> GenerationResult<()> {
    let mut expected = conversion.files.clone();
    expected.insert(
        "change.bin",
        disposed_change_bytes(source, name, conversion)?,
    );
    for (leaf, bytes) in &expected {
        let path = target.join(leaf);
        if bytes.len() == 4 && !["land.bin", "snapshot_link.bin"].contains(leaf) {
            if path.exists() {
                return Err(format!(
                    "Existing target retained empty legacy family: {leaf}"
                ));
            }
        } else if fs::read(path).map_err(|e| e.to_string())? != *bytes {
            return Err(format!(
                "Existing upgrade target is not the complete approved conversion: {leaf}"
            ));
        }
    }
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let leaf = entry.file_name();
        if leaf == ".locks" || expected.keys().any(|name| leaf == *name) {
            continue;
        }
        if !equal_files(&entry.path(), &target.join(&leaf))? {
            return Err(
                "Existing upgrade target changed an unrelated local authority member".into(),
            );
        }
    }
    Ok(())
}

fn resolve_output_root(path: &Path) -> GenerationResult<PathBuf> {
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(
            "Upgrade generation root must use an unambiguous path without parent traversal".into(),
        );
    }
    let mut ancestor = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    let mut suffix = Vec::new();
    loop {
        match fs::symlink_metadata(&ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    ancestor
                        .file_name()
                        .ok_or("Upgrade generation root has no name")?
                        .to_os_string(),
                );
                if !ancestor.pop() {
                    return Err("Upgrade generation root has no existing ancestor".into());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let mut resolved = ancestor.canonicalize().map_err(|error| error.to_string())?;
    for leaf in suffix.into_iter().rev() {
        resolved.push(leaf);
    }
    Ok(resolved)
}

/// Existing runtime admission owns the upgrade; codec reads never rewrite it.
pub(super) fn admit_local_receipt_upgrade(
    repo: &Path,
    authority: &Path,
    name: &str,
) -> GenerationResult<()> {
    let _range = perfetto_range!("ait.database.admission.upgrade");
    let version = header(&authority.join("land.bin"))?;
    let links = header(&authority.join("snapshot_link.bin"))?;
    if version == Some(2) && links == Some(2) {
        return Ok(());
    }
    if !matches!(version, None | Some(1) | Some(2)) || !matches!(links, None | Some(1) | Some(2)) {
        return Err("Unsupported local workflow file layout; source is unchanged".into());
    }
    let _coordination = {
        let _range = perfetto_range!("ait.database.admission.coordination");
        let root = super::generation_activation::binary_db_activation_lock_root(repo);
        let lock = crate::binary_db::BinaryDbCommandLockSet::try_acquire(
        &root,
        crate::binary_db::BinaryDbCommandScope::General,
    ).map_err(|e|format!("Local receipt upgrade coordination required: stop native writers before admission: {e}"))?;
        let Some(lock) = lock else {
            // A concurrent admission holds the exclusive generation lock. Wait
            // through its switch, then reuse the completed target. A preexisting
            // runtime reader cannot authorize conversion of its legacy source.
            let _read = BinaryDbReadLockSet::try_acquire(&root)
                .map_err(|e| format!("Local receipt upgrade coordination required: {e}"))?;
            if header(&authority.join("land.bin"))? == Some(2)
                && header(&authority.join("snapshot_link.bin"))? == Some(2)
            {
                return Ok(());
            }
            return Err(
                "Local receipt upgrade coordination required: native writers are active".into(),
            );
        };
        lock
    };
    if !authority.exists() {
        return Err(
            "Repository Binary DB authority is missing; admission requires coordination".into(),
        );
    }
    let version = header(&authority.join("land.bin"))?;
    let links = header(&authority.join("snapshot_link.bin"))?;
    if version == Some(2) && links == Some(2) {
        return Ok(());
    }
    if matches!(version, None | Some(2))
        && matches!(links, None | Some(2))
        && count(&authority.join("change.bin"), 68)? == 0
        && count(&authority.join("land.bin"), 44)? == 0
        && count(&authority.join("snapshot_link.bin"), 22)? == 0
    {
        let _read = BinaryDbReadLockSet::try_acquire(&crate::binary_db::StorePath::from(
            authority.to_path_buf(),
        ))
        .map_err(|e| e.to_string())?;
        super::generation_activation::admit_activated_binary_db_generation(authority, name)?;
        BinaryDbWorkflowStore::<_, 1>::new(database(authority, repo), name)
            .validate_detached_authority()
            .map_err(|e| e.to_string())?;
        // Existing per-file headers describe empty layout-2 families before
        // authoring. Completing an interrupted empty initialization changes no
        // domain record and never repeats legacy cleanup.
        use std::io::Write;
        for leaf in ["snapshot_link.bin", "land.bin"] {
            if header(&authority.join(leaf))? == Some(2) {
                continue;
            }
            let staged = repo
                .join(".ait")
                .join(format!(".workflow-header-{}", temporary_suffix()));
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&staged)
                .map_err(|e| e.to_string())?;
            file.write_all(&2u32.to_le_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            fs::rename(&staged, authority.join(leaf)).map_err(|e| e.to_string())?;
            super::generation_capture::sync_directory(&repo.join(".ait"))?;
            super::generation_capture::sync_directory(authority)?;
        }
        return Ok(());
    }
    if version == Some(2) || links == Some(2) {
        return Err("Incomplete earlier workflow conversion: preserve the receipt-only/mixed target; combined LocalLand2 and SnapshotLink2 admission is required".into());
    }
    admit_local_metadata(repo, authority, name)
}

fn admit_local_metadata(repo: &Path, authority: &Path, name: &str) -> GenerationResult<()> {
    require_direct_authority(authority)?;
    let packs = fs::symlink_metadata(repo.join(".ait/objects")).map_err(|e| e.to_string())?;
    if !packs.file_type().is_dir() {
        return Err("Repository content authority must be a direct directory".into());
    }
    if configured_repo_name(repo)? != name {
        return Err("Local source Repository owner mismatch".into());
    }
    let config: serde_json::Value = serde_json::from_slice(
        &fs::read(repo.join(".ait/config.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let namespace = match config.get("id_namespace_prefix") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(namespace)) => Some(namespace.as_str()),
        Some(_) => return Err("Invalid Local source namespace".into()),
    };
    let namespace = crate::workflow_primitives::normalize_id_namespace_prefix(
        namespace,
        Some(crate::workflow_primitives::DEFAULT_ID_NAMESPACE_PREFIX),
    )?;
    // Classify the complete affected source before staging or cursor writes.
    for leaf in super::local_change_cleanup::CHANGED_FILES
        .iter()
        .copied()
        .chain(["task.bin", "snapshot.bin"])
    {
        let layout = header(&authority.join(leaf))?;
        if !matches!(layout, None | Some(1)) {
            return Err(format!("Unsupported partial Local source profile: {leaf}"));
        }
    }
    let _read = BinaryDbReadLockSet::try_acquire(&crate::binary_db::StorePath::from(
        authority.to_path_buf(),
    ))
    .map_err(|e| e.to_string())?;
    let mut conversion = Conversion::build_with_namespace(authority, name, &namespace)?;
    // Historical edge disposition is composed only for its original owner.
    let changes = disposed_change_bytes(authority, name, &conversion)?;
    conversion.files.insert("change.bin", changes);
    conversion.retain_changed_files(authority)?;
    let cursors = super::local_upgrade_cursors::prepare(repo, &conversion, &namespace)?;
    let staged = repo
        .join(".ait")
        .join(format!(".workflow-metadata-{}", temporary_suffix()));
    fs::create_dir(&staged).map_err(|e| e.to_string())?;
    let result = (|| {
        conversion.write(&staged)?;
        super::generation_capture::sync_directory(&staged)?;
        let replace = |leaf: &str| -> GenerationResult<()> {
            let source = staged.join(leaf);
            if source.try_exists().map_err(|e| e.to_string())? {
                fs::rename(source, authority.join(leaf)).map_err(|e| e.to_string())?;
            } else {
                match fs::remove_file(authority.join(leaf)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok(())
        };
        for leaf in conversion
            .files
            .keys()
            .copied()
            .filter(|leaf| !["land.bin", "snapshot_link.bin"].contains(leaf))
        {
            replace(leaf)?;
        }
        cursors.publish()?;
        super::generation_capture::sync_directory(authority)?;
        // Only after dependent records and disposable selections are durable
        // can either target sentinel become visible. A partial batch rejects.
        for leaf in ["snapshot_link.bin", "land.bin"] {
            replace(leaf)?;
            super::generation_capture::sync_directory(authority)?;
        }
        Ok(())
    })();
    let cleanup = fs::remove_dir_all(&staged).map_err(|e| e.to_string());
    result.and(cleanup)
}

pub(super) fn temporary_suffix() -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|x| x.as_nanos())
        .unwrap_or_default();
    format!("{}-{nonce}", std::process::id())
}

#[cfg(test)]
mod runner_tests;

#[cfg(test)]
mod server_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_binary_db::LocalChangeRecord;
    use std::collections::BTreeMap;

    fn disposition_fixture() -> (tempfile::TempDir, Conversion) {
        let root = tempfile::TempDir::new().unwrap();
        let mut tasks = vec![0; 4 + 54 * 64];
        tasks[..4].copy_from_slice(&1u32.to_le_bytes());
        let mut changes = Vec::new();
        for (task_index, remote, time) in [(2u32, 981u32, 1782170210u64), (53, 787, 1782944552)] {
            let task = &mut tasks[4 + task_index as usize * 64..][..64];
            task[1] = 3;
            task[20..24].copy_from_slice(&remote.to_le_bytes());
            let mut raw = [0; 68];
            raw[0] = 3;
            raw[1] = 1;
            raw[2] = 1;
            raw[4] = 1;
            raw[5] = 1;
            raw[8] = 4;
            raw[16..20].copy_from_slice(&task_index.to_le_bytes());
            raw[28] = 1;
            raw[32..40].copy_from_slice(&(time - 1).to_le_bytes());
            raw[40..48].copy_from_slice(&time.to_le_bytes());
            raw[48..56].copy_from_slice(&time.to_le_bytes());
            raw[56] = 1;
            changes.push(LocalChangeRecord::decode(&raw).unwrap());
        }
        fs::write(root.path().join("task.bin"), tasks).unwrap();
        // Reverse the physical target order: source ownership/mapping, not a
        // guessed row number, must choose each disposition.
        let mut bytes = 1u32.to_le_bytes().to_vec();
        for change in changes.iter().rev() {
            let mut raw = change.encode().unwrap();
            raw[4] = 0;
            bytes.extend_from_slice(&raw);
        }
        let mut files = BTreeMap::new();
        files.insert("change.bin", bytes);
        (
            root,
            Conversion {
                files,
                changes,
                mapping: vec![Some(1), Some(0)],
                task_count: 54,
                sources: Default::default(),
                core_history: true,
            },
        )
    }

    #[test]
    fn publication_disposition_changes_only_edges_and_retains_historical_times() {
        let (source, conversion) = disposition_fixture();
        let before = &conversion.files["change.bin"];
        let after = disposed_change_bytes(source.path(), "ait-core", &conversion).unwrap();
        let changed = before
            .iter()
            .zip(&after)
            .enumerate()
            .filter_map(|(at, (a, b))| (a != b).then_some(at))
            .collect::<Vec<_>>();
        assert_eq!(changed, [5, 32, 73, 100]);
        assert_eq!(&after[52..60], &1782944552u64.to_le_bytes());
        assert_eq!(&after[120..128], &1782170210u64.to_le_bytes());
        assert_eq!(conversion.files["change.bin"], *before);
    }

    #[test]
    fn publication_disposition_is_noop_for_unrelated_or_initial_only_sources() {
        let (source, mut conversion) = disposition_fixture();
        let before = conversion.files["change.bin"].clone();
        assert_eq!(
            disposed_change_bytes(source.path(), "other", &conversion).unwrap(),
            before
        );
        for change in &mut conversion.changes {
            change.change_ordinal = 0;
        }
        conversion.mapping.fill(None);
        conversion
            .files
            .insert("change.bin", 1u32.to_le_bytes().to_vec());
        assert_eq!(
            disposed_change_bytes(source.path(), "ait-core", &conversion).unwrap(),
            1u32.to_le_bytes()
        );
    }

    #[test]
    fn publication_disposition_rejects_partial_or_wrong_source_and_target_ownership() {
        for case in 0..8 {
            let (source, mut conversion) = disposition_fixture();
            match case {
                0 => conversion.changes[0].local_meta = 0,
                1 => conversion.changes[0].published_remote_change_ordinal_plus1 = 0,
                2 => conversion.mapping[0] = None,
                3 => conversion.changes[1].change_ordinal = 0,
                4 => conversion.mapping.swap(0, 1),
                5 => conversion.files.get_mut("change.bin").unwrap()[5] = 0,
                6 => conversion.files.get_mut("change.bin").unwrap()[32] = 0,
                _ => {
                    let path = source.path().join("task.bin");
                    let mut tasks = fs::read(&path).unwrap();
                    tasks[4 + 2 * 64 + 20] ^= 1;
                    fs::write(path, tasks).unwrap();
                }
            }
            let before = conversion.files["change.bin"].clone();
            assert!(
                disposed_change_bytes(source.path(), "ait-core", &conversion).is_err(),
                "case {case}"
            );
            assert_eq!(conversion.files["change.bin"], before);
        }
    }
    #[test]
    #[ignore = "requires the immutable actual-source fixture; copies metadata only"]
    fn actual_source_metadata_runtime_admission_and_reopen() {
        use crate::change_store::ChangeStore;
        use crate::content_binary_db::{BinaryDbSnapshotReader, LocalContentBinaryDb};
        use crate::object_diff_ports::SnapshotReader;
        use crate::task_store::TaskStore;
        let source = PathBuf::from(
            std::env::var_os("AIT_TEST_LEGACY_METADATA_SOURCE")
                .expect("actual-source fixture path"),
        )
        .canonicalize()
        .unwrap();
        let source_db = source.join(".ait/binary-db");
        let source_fingerprint = fingerprint_direct_authority_contents(&source_db).unwrap();
        assert_eq!(
            source_fingerprint,
            "225ddf0ed3f5d705304a6a836aab984a684179bc4ffa9651163823f92723e5b8"
        );
        let temp = tempfile::TempDir::new_in("/private/tmp").unwrap();
        let repo = temp.path().join("repo");
        let authority = repo.join(".ait/binary-db");
        fs::create_dir_all(&authority).unwrap();
        fs::create_dir_all(repo.join(".ait/objects")).unwrap();
        // Fixture configuration only. Production migration never stages config.
        fs::write(
            repo.join(".ait/config.json"),
            br#"{"repo_name":"ait-core","id_namespace_prefix":"C"}"#,
        )
        .unwrap();
        for entry in fs::read_dir(&source_db).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                fs::copy(entry.path(), authority.join(entry.file_name())).unwrap();
            }
        }
        // Rebase only local dependent metadata. Never access original worktrees.
        fs::create_dir_all(repo.join(".ait/worktrees")).unwrap();
        for entry in fs::read_dir(source.join(".ait/worktrees")).unwrap() {
            let entry = entry.unwrap();
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
            if !value["bound_task_id"]
                .as_str()
                .is_some_and(|id| id.starts_with('L'))
            {
                continue;
            }
            let name = value["name"].as_str().unwrap().to_string();
            let worktree = temp.path().join(&name);
            fs::create_dir_all(&worktree).unwrap();
            value["repo_root"] = serde_json::json!(repo);
            value["path"] = serde_json::json!(worktree);
            fs::write(
                repo.join(".ait/worktrees").join(entry.file_name()),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
            let mut overlay: serde_json::Value = serde_json::from_slice(
                &fs::read(
                    source
                        .join("dependent-worktrees")
                        .join(&name)
                        .join(".ait-worktree.json"),
                )
                .unwrap(),
            )
            .unwrap();
            overlay["repo_root"] = serde_json::json!(repo);
            overlay["workspace_root"] = serde_json::json!(worktree);
            fs::write(
                worktree.join(".ait-worktree.json"),
                serde_json::to_vec(&overlay).unwrap(),
            )
            .unwrap();
        }
        let legacy = BinaryDbWorkflowStore::<_, 1>::new_with_namespace(
            database(&authority, &repo),
            "ait-core",
            "C",
        )
        .for_local_land_upgrade();
        let before_changes = ["LCT-0003", "LCT-0054", "LCT-0621"]
            .map(|id| legacy.get_change(&format!("{id}/C-02")).unwrap());
        let expected = Conversion::build(&authority, "ait-core").unwrap();
        let before_tasks = legacy.list_tasks().unwrap();
        let receipts = before_tasks
            .iter()
            .filter(|row| !row["local_finish"].is_null())
            .map(|row| {
                (
                    row["task_id"].as_str().unwrap().to_string(),
                    row["local_finish"].clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(receipts.len(), 663);
        let started = std::time::Instant::now();
        let (_, guard) =
            super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
                &repo, &authority, "ait-core",
            )
            .unwrap();
        let admission_s = started.elapsed().as_secs_f64();
        drop(guard);
        let started = std::time::Instant::now();
        let (_, guard) =
            super::super::generation_activation::admit_activated_binary_db_generation_for_runtime(
                &repo, &authority, "ait-core",
            )
            .unwrap();
        let reopen_s = started.elapsed().as_secs_f64();
        let current = BinaryDbWorkflowStore::<_, 1>::new_with_namespace(
            database(&authority, &repo),
            "ait-core",
            "C",
        );
        let content = LocalContentBinaryDb::<1>::from_db_with_roots(
            database(&authority, &repo),
            repo.clone(),
            source.clone(),
        );
        let reader = BinaryDbSnapshotReader::with_root_resolver(
            content.trees().clone(),
            content.snapshots().clone(),
        );
        for before in before_changes {
            let task = before["task_id"].as_str().unwrap();
            let after = current.get_change(&format!("{task}/C-01")).unwrap();
            for key in [
                "title",
                "status",
                "fork_snapshot_id",
                "base_line",
                "published_at",
            ] {
                assert_eq!(after[key], before[key], "{task}/{key}");
            }
            if let Some(snapshot) = after["fork_snapshot_id"].as_str() {
                assert!(reader
                    .read_snapshot_root_tree_payload(snapshot)
                    .unwrap()
                    .is_some());
            }
        }
        // Receipt result/identity remains exact; only the optional Change clears.
        for (task, before) in receipts {
            let after = current.get_task(&task).unwrap()["local_finish"].clone();
            for key in [
                "submission_id",
                "landed_snapshot_id",
                "pre_land_target_snapshot_id",
                "target_line",
                "submitted_at",
                "updated_at",
            ] {
                assert_eq!(after[key], before[key], "{task}/{key}");
            }
        }
        let ids = current
            .list_tasks()
            .unwrap()
            .iter()
            .filter_map(|row| {
                row["local_finish"]["landed_snapshot_id"]
                    .as_str()
                    .map(String::from)
            })
            .collect::<Vec<_>>();
        let owners = current.snapshot_ownership_rows(&ids).unwrap();
        assert!(!owners.is_empty());
        for owner in owners {
            if owner["ownership_source"] == "ambiguous_recorded_task_receipts" {
                assert!(owner["task_id"].is_null());
                assert!(owner["accepted_task_ids"].as_array().unwrap().len() > 1);
            } else {
                let row = current
                    .get_task(owner["task_id"].as_str().unwrap())
                    .unwrap();
                if owner["ownership_source"] == "derived_from_recorded_task_receipt" {
                    assert_eq!(
                        row["local_finish"]["landed_snapshot_id"],
                        owner["snapshot_id"]
                    );
                }
            }
        }
        for leaf in ["land.bin", "snapshot_link.bin"] {
            assert_eq!(
                fs::read(authority.join(leaf)).unwrap(),
                expected.files[leaf]
            );
        }
        assert_eq!(fs::read_dir(repo.join(".ait/objects")).unwrap().count(), 0);
        assert!(!repo.join(".ait/local-task-receipts-layout2").exists());
        assert_eq!(
            fingerprint_direct_authority_contents(&source_db).unwrap(),
            source_fingerprint
        );
        drop(guard);
        eprintln!("actual-source metadata admission={admission_s:.6}s reopen={reopen_s:.6}s; warm APFS /private/tmp; existing object root read-only; isolated={}", temp.keep().display());
    }
}
