use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::local_snapshot::{build_tree_root_id, SnapshotFileEntry};
use crate::plan_filesystem::VisibleWorkspaceFileMetadata;

pub const WORKSPACE_HASH_CACHE_CONTRACT: &str = "ait-workspace-hash-cache/v1";
pub const WORKSPACE_HASH_CACHE_MAX_AGE_SECS: u64 = 60 * 60;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceFileFingerprint {
    pub file_kind: String,
    pub size_bytes: u64,
    pub mode_bits: u32,
    pub modified_ns: u64,
    pub changed_ns: u64,
    pub device_id: u64,
    pub file_id: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkspaceHashCacheEntry {
    pub path: String,
    pub blob_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub mode: String,
    pub fingerprint: WorkspaceFileFingerprint,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkspaceHashCache {
    pub contract: String,
    pub workspace_root: String,
    pub snapshot_id: String,
    pub root_tree_id: String,
    pub file_count: u64,
    pub generated_at_unix_ns: u64,
    pub entries: BTreeMap<String, WorkspaceHashCacheEntry>,
    pub checksum: String,
}

#[derive(Clone, Debug)]
pub enum WorkspaceHashCacheLoad {
    Hit(WorkspaceHashCache),
    Miss,
    Invalid(String),
    Stale(String),
}

impl WorkspaceHashCacheLoad {
    pub fn state(&self) -> &'static str {
        match self {
            Self::Hit(_) => "hit",
            Self::Miss => "miss",
            Self::Invalid(_) => "invalid_fallback",
            Self::Stale(_) => "stale_fallback",
        }
    }

    pub fn cache(&self) -> Option<&WorkspaceHashCache> {
        match self {
            Self::Hit(cache) => Some(cache),
            Self::Miss | Self::Invalid(_) | Self::Stale(_) => None,
        }
    }
}

pub fn load_workspace_hash_cache(
    workspace_root: &Path,
    snapshot_id: &str,
) -> WorkspaceHashCacheLoad {
    let canonical_root = match canonical_workspace_root(workspace_root) {
        Ok(root) => root,
        Err(error) => return WorkspaceHashCacheLoad::Invalid(error),
    };
    let path = workspace_hash_cache_path_for_canonical_root(&canonical_root);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return WorkspaceHashCacheLoad::Miss;
        }
        Err(error) => {
            return WorkspaceHashCacheLoad::Invalid(format!(
                "Failed to read workspace hash cache {}: {error}",
                path.display()
            ));
        }
    };
    let cache = match serde_json::from_slice::<WorkspaceHashCache>(&bytes) {
        Ok(cache) => cache,
        Err(error) => {
            return WorkspaceHashCacheLoad::Invalid(format!(
                "Workspace hash cache {} is invalid JSON: {error}",
                path.display()
            ));
        }
    };
    if cache.contract != WORKSPACE_HASH_CACHE_CONTRACT {
        return WorkspaceHashCacheLoad::Invalid(format!(
            "Workspace hash cache uses unsupported contract {}",
            cache.contract
        ));
    }
    if cache.workspace_root != canonical_root.to_string_lossy() {
        return WorkspaceHashCacheLoad::Invalid(
            "Workspace hash cache root identity does not match the current workspace".to_string(),
        );
    }
    if cache.snapshot_id != snapshot_id {
        return WorkspaceHashCacheLoad::Miss;
    }
    if cache.checksum != cache_checksum(&cache) {
        return WorkspaceHashCacheLoad::Invalid(
            "Workspace hash cache checksum does not match its content".to_string(),
        );
    }
    if let Err(error) = validate_cache_entries(&cache) {
        return WorkspaceHashCacheLoad::Invalid(error);
    }
    let now_ns = system_time_ns(SystemTime::now());
    let max_age_ns = WORKSPACE_HASH_CACHE_MAX_AGE_SECS.saturating_mul(1_000_000_000);
    if cache.generated_at_unix_ns == 0 || cache.generated_at_unix_ns > now_ns {
        return WorkspaceHashCacheLoad::Invalid(
            "Workspace hash cache generation time is invalid".to_string(),
        );
    }
    if now_ns.saturating_sub(cache.generated_at_unix_ns) > max_age_ns {
        return WorkspaceHashCacheLoad::Stale(format!(
            "Workspace hash cache exceeded its {} second correctness horizon",
            WORKSPACE_HASH_CACHE_MAX_AGE_SECS
        ));
    }
    WorkspaceHashCacheLoad::Hit(cache)
}

pub fn write_workspace_hash_cache(
    workspace_root: &Path,
    snapshot_id: &str,
    root_tree_id: &str,
    entries: impl IntoIterator<Item = WorkspaceHashCacheEntry>,
) -> Result<PathBuf, String> {
    let canonical_root = canonical_workspace_root(workspace_root)?;
    let path = workspace_hash_cache_path_for_canonical_root(&canonical_root);
    let entries = entries
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let file_count = u64::try_from(entries.len())
        .map_err(|_| "Workspace hash cache exceeds u64 file count limits.".to_string())?;
    let calculated_root_tree_id = workspace_hash_cache_root_tree_id(&entries)?;
    if calculated_root_tree_id != root_tree_id {
        return Err(format!(
            "Workspace hash cache entries reconstruct tree {calculated_root_tree_id}, not requested tree {root_tree_id}"
        ));
    }
    let mut cache = WorkspaceHashCache {
        contract: WORKSPACE_HASH_CACHE_CONTRACT.to_string(),
        workspace_root: canonical_root.to_string_lossy().to_string(),
        snapshot_id: snapshot_id.to_string(),
        root_tree_id: root_tree_id.to_string(),
        file_count,
        generated_at_unix_ns: system_time_ns(SystemTime::now()),
        entries,
        checksum: String::new(),
    };
    cache.checksum = cache_checksum(&cache);
    let mut bytes = serde_json::to_vec(&cache)
        .map_err(|error| format!("Failed to encode workspace hash cache: {error}"))?;
    bytes.push(b'\n');
    let parent = path.parent().ok_or_else(|| {
        format!(
            "Workspace hash cache path has no parent: {}",
            path.display()
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Failed to create workspace hash cache directory {}: {error}",
            parent.display()
        )
    })?;
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace-cache"),
        std::process::id(),
        system_time_ns(SystemTime::now())
    ));
    fs::write(&temporary, bytes).map_err(|error| {
        format!(
            "Failed to write workspace hash cache temporary file {}: {error}",
            temporary.display()
        )
    })?;
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Failed to publish workspace hash cache {}: {error}",
            path.display()
        ));
    }
    Ok(path)
}

pub fn workspace_hash_cache_path(workspace_root: &Path) -> Result<PathBuf, String> {
    canonical_workspace_root(workspace_root)
        .map(|root| workspace_hash_cache_path_for_canonical_root(&root))
}

pub fn workspace_hash_cache_entry(
    path: &str,
    blob_id: &str,
    sha256: &str,
    size_bytes: u64,
    mode: &str,
    fingerprint: WorkspaceFileFingerprint,
) -> WorkspaceHashCacheEntry {
    WorkspaceHashCacheEntry {
        path: path.to_string(),
        blob_id: blob_id.to_string(),
        sha256: sha256.to_string(),
        size_bytes,
        mode: mode.to_string(),
        fingerprint,
    }
}

pub fn workspace_hash_cache_entry_matches(entry: &WorkspaceHashCacheEntry, path: &Path) -> bool {
    workspace_file_fingerprint(path)
        .map(|fingerprint| fingerprint == entry.fingerprint)
        .unwrap_or(false)
}

#[cfg(unix)]
pub fn workspace_file_fingerprint(path: &Path) -> Result<WorkspaceFileFingerprint, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to fingerprint {}: {error}", path.display()))?;
    Ok(workspace_file_fingerprint_from_metadata(&metadata))
}

#[cfg(unix)]
pub fn workspace_file_fingerprint_from_metadata(
    metadata: &fs::Metadata,
) -> WorkspaceFileFingerprint {
    use std::os::unix::fs::MetadataExt;

    let file_kind = if metadata.file_type().is_symlink() {
        "symlink"
    } else if metadata.is_file() {
        "file"
    } else {
        "other"
    };
    WorkspaceFileFingerprint {
        file_kind: file_kind.to_string(),
        size_bytes: metadata.len(),
        mode_bits: metadata.mode(),
        modified_ns: unix_time_parts_ns(metadata.mtime(), metadata.mtime_nsec()),
        changed_ns: unix_time_parts_ns(metadata.ctime(), metadata.ctime_nsec()),
        device_id: metadata.dev(),
        file_id: metadata.ino(),
    }
}

pub fn workspace_file_fingerprint_from_visible_metadata(
    metadata: &VisibleWorkspaceFileMetadata,
) -> WorkspaceFileFingerprint {
    WorkspaceFileFingerprint {
        file_kind: metadata.file_kind.clone(),
        size_bytes: metadata.size_bytes,
        mode_bits: metadata.mode_bits,
        modified_ns: metadata.modified_ns,
        changed_ns: metadata.changed_ns,
        device_id: metadata.device_id,
        file_id: metadata.file_id,
    }
}

#[cfg(not(unix))]
pub fn workspace_file_fingerprint(path: &Path) -> Result<WorkspaceFileFingerprint, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to fingerprint {}: {error}", path.display()))?;
    Ok(workspace_file_fingerprint_from_metadata(&metadata))
}

#[cfg(not(unix))]
pub fn workspace_file_fingerprint_from_metadata(
    metadata: &fs::Metadata,
) -> WorkspaceFileFingerprint {
    WorkspaceFileFingerprint {
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
        modified_ns: metadata.modified().map(system_time_ns).unwrap_or_default(),
        changed_ns: 0,
        device_id: 0,
        file_id: 0,
    }
}

fn canonical_workspace_root(workspace_root: &Path) -> Result<PathBuf, String> {
    workspace_root.canonicalize().map_err(|error| {
        format!(
            "Failed to resolve workspace root {} for hash cache: {error}",
            workspace_root.display()
        )
    })
}

fn workspace_hash_cache_path_for_canonical_root(canonical_root: &Path) -> PathBuf {
    canonical_root
        .join(".ait-runtime")
        .join("workspace-hash-cache-v1.json")
}

fn cache_checksum(cache: &WorkspaceHashCache) -> String {
    let mut digest = Sha256::new();
    hash_cache_checksum_text(&mut digest, "ait-workspace-hash-cache-checksum/v1");
    hash_cache_checksum_text(&mut digest, &cache.contract);
    hash_cache_checksum_text(&mut digest, &cache.workspace_root);
    hash_cache_checksum_text(&mut digest, &cache.snapshot_id);
    hash_cache_checksum_text(&mut digest, &cache.root_tree_id);
    hash_cache_checksum_u64(&mut digest, cache.file_count);
    hash_cache_checksum_u64(&mut digest, cache.generated_at_unix_ns);
    hash_cache_checksum_u64(&mut digest, cache.entries.len() as u64);
    for (path, entry) in &cache.entries {
        hash_cache_checksum_text(&mut digest, path);
        hash_cache_checksum_text(&mut digest, &entry.path);
        hash_cache_checksum_text(&mut digest, &entry.blob_id);
        hash_cache_checksum_text(&mut digest, &entry.sha256);
        hash_cache_checksum_u64(&mut digest, entry.size_bytes);
        hash_cache_checksum_text(&mut digest, &entry.mode);
        hash_cache_checksum_text(&mut digest, &entry.fingerprint.file_kind);
        hash_cache_checksum_u64(&mut digest, entry.fingerprint.size_bytes);
        hash_cache_checksum_u64(&mut digest, u64::from(entry.fingerprint.mode_bits));
        hash_cache_checksum_u64(&mut digest, entry.fingerprint.modified_ns);
        hash_cache_checksum_u64(&mut digest, entry.fingerprint.changed_ns);
        hash_cache_checksum_u64(&mut digest, entry.fingerprint.device_id);
        hash_cache_checksum_u64(&mut digest, entry.fingerprint.file_id);
    }
    format!("sha256:{:x}", digest.finalize())
}

fn hash_cache_checksum_text(digest: &mut Sha256, value: &str) {
    hash_cache_checksum_u64(digest, value.len() as u64);
    digest.update(value.as_bytes());
}

fn hash_cache_checksum_u64(digest: &mut Sha256, value: u64) {
    digest.update(value.to_le_bytes());
}

fn validate_cache_entries(cache: &WorkspaceHashCache) -> Result<(), String> {
    if cache.snapshot_id.trim().is_empty() || cache.root_tree_id.trim().is_empty() {
        return Err("Workspace hash cache is missing Snapshot or tree identity".to_string());
    }
    if cache.file_count != cache.entries.len() as u64 {
        return Err("Workspace hash cache file count does not match its entries".to_string());
    }
    for (path, entry) in &cache.entries {
        if path.is_empty() || path != &entry.path {
            return Err("Workspace hash cache entry path identity is invalid".to_string());
        }
        if entry.sha256.len() != 64
            || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || entry.blob_id != format!("BLB-{}", entry.sha256[..20].to_ascii_lowercase())
        {
            return Err(format!(
                "Workspace hash cache entry {path} has invalid content identity"
            ));
        }
        if entry.size_bytes > i64::MAX as u64 {
            return Err(format!(
                "Workspace hash cache entry {path} exceeds Snapshot size limits"
            ));
        }
        if entry.size_bytes != entry.fingerprint.size_bytes {
            return Err(format!(
                "Workspace hash cache entry {path} has inconsistent size metadata"
            ));
        }
        if entry.fingerprint.file_kind != "file" && entry.fingerprint.file_kind != "symlink" {
            return Err(format!(
                "Workspace hash cache entry {path} has unsupported file kind"
            ));
        }
        let permission_bits = entry.fingerprint.mode_bits & 0o777;
        let expected_mode = if entry.fingerprint.file_kind == "symlink" {
            format!("{:#o}", 0o120000 | permission_bits)
        } else {
            format!("{:#o}", permission_bits)
        };
        if entry.mode != expected_mode {
            return Err(format!(
                "Workspace hash cache entry {path} has inconsistent mode metadata"
            ));
        }
    }
    let calculated_root_tree_id = workspace_hash_cache_root_tree_id(&cache.entries)?;
    if calculated_root_tree_id != cache.root_tree_id {
        return Err(format!(
            "Workspace hash cache entries reconstruct tree {calculated_root_tree_id}, not recorded tree {}",
            cache.root_tree_id
        ));
    }
    Ok(())
}

fn workspace_hash_cache_root_tree_id(
    entries: &BTreeMap<String, WorkspaceHashCacheEntry>,
) -> Result<String, String> {
    let file_entries = entries
        .values()
        .map(|entry| {
            Ok(SnapshotFileEntry {
                path: entry.path.clone(),
                blob_id: entry.blob_id.clone(),
                size_bytes: i64::try_from(entry.size_bytes).map_err(|_| {
                    format!(
                        "Workspace hash cache entry {} exceeds Snapshot size limits",
                        entry.path
                    )
                })?,
                mode: entry.mode.clone(),
                sha256: entry.sha256.clone(),
                data: Vec::new(),
                data_reused: true,
                cache_fingerprint: Some(entry.fingerprint.clone()),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    build_tree_root_id(&file_entries)
}

fn system_time_ns(value: SystemTime) -> u64 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_round_trip_is_workspace_and_snapshot_scoped() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".ait")).unwrap();
        let file = root.path().join("tracked.txt");
        fs::write(&file, "alpha").unwrap();
        let fingerprint = workspace_file_fingerprint(&file).unwrap();
        let mode = format!("{:#o}", fingerprint.mode_bits & 0o777);
        let entry = workspace_hash_cache_entry(
            "tracked.txt",
            "BLB-8ed3f6ad685b959ead70",
            "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8",
            5,
            &mode,
            fingerprint,
        );
        let entries = BTreeMap::from([(entry.path.clone(), entry.clone())]);
        let root_tree_id = workspace_hash_cache_root_tree_id(&entries).unwrap();
        write_workspace_hash_cache(root.path(), "SNP-1", &root_tree_id, [entry]).unwrap();
        let loaded = load_workspace_hash_cache(root.path(), "SNP-1");
        let cache = loaded.cache().expect("cache hit");
        assert_eq!(cache.entries.len(), 1);
        assert!(workspace_hash_cache_entry_matches(
            &cache.entries["tracked.txt"],
            &file
        ));
        assert!(matches!(
            load_workspace_hash_cache(root.path(), "SNP-2"),
            WorkspaceHashCacheLoad::Miss
        ));
    }

    #[test]
    fn metadata_change_invalidates_cached_hash_reuse() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".ait")).unwrap();
        let file = root.path().join("tracked.txt");
        fs::write(&file, "alpha").unwrap();
        let fingerprint = workspace_file_fingerprint(&file).unwrap();
        let mode = format!("{:#o}", fingerprint.mode_bits & 0o777);
        let entry = workspace_hash_cache_entry(
            "tracked.txt",
            "BLB-8ed3f6ad685b959ead70",
            "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8",
            5,
            &mode,
            fingerprint,
        );
        fs::write(&file, "bravo!").unwrap();
        assert!(!workspace_hash_cache_entry_matches(&entry, &file));
    }

    #[test]
    fn expired_cache_falls_back_without_exposing_entries() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".ait")).unwrap();
        let file = root.path().join("tracked.txt");
        fs::write(&file, "alpha").unwrap();
        let fingerprint = workspace_file_fingerprint(&file).unwrap();
        let mode = format!("{:#o}", fingerprint.mode_bits & 0o777);
        let entry = workspace_hash_cache_entry(
            "tracked.txt",
            "BLB-8ed3f6ad685b959ead70",
            "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8",
            5,
            &mode,
            fingerprint,
        );
        let entries = BTreeMap::from([(entry.path.clone(), entry.clone())]);
        let root_tree_id = workspace_hash_cache_root_tree_id(&entries).unwrap();
        write_workspace_hash_cache(root.path(), "SNP-1", &root_tree_id, [entry]).unwrap();
        let path = workspace_hash_cache_path(root.path()).unwrap();
        let mut cache: WorkspaceHashCache =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        cache.generated_at_unix_ns = 1;
        cache.checksum = cache_checksum(&cache);
        fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        assert!(matches!(
            load_workspace_hash_cache(root.path(), "SNP-1"),
            WorkspaceHashCacheLoad::Stale(_)
        ));
    }
}
