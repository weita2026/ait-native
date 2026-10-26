use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workspace_hash_cache::WorkspaceFileFingerprint;

const EXTERNAL_MATERIALIZATION_HASH_CACHE_CONTRACT: &str =
    "ait-external-materialization-hash-cache/v1";
const EXTERNAL_MATERIALIZATION_HASH_CACHE_MAX_AGE_SECS: u64 = 60 * 60;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ExternalMaterializationHashCacheFile {
    pub sha256: String,
    pub fingerprint: WorkspaceFileFingerprint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ExternalMaterializationHashCacheRoot {
    pub marker_sha256: String,
    pub files: BTreeMap<String, ExternalMaterializationHashCacheFile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ExternalMaterializationHashCache {
    contract: String,
    repo_root: String,
    generated_at_unix_ns: u64,
    pub roots: BTreeMap<String, ExternalMaterializationHashCacheRoot>,
    checksum: String,
}

pub(super) fn load_external_materialization_hash_cache(
    repo_root: &Path,
) -> Option<ExternalMaterializationHashCache> {
    let canonical_root = repo_root.canonicalize().ok()?;
    let path = external_materialization_hash_cache_path(&canonical_root);
    let bytes = fs::read(path).ok()?;
    let cache = serde_json::from_slice::<ExternalMaterializationHashCache>(&bytes).ok()?;
    if cache.contract != EXTERNAL_MATERIALIZATION_HASH_CACHE_CONTRACT
        || cache.repo_root != canonical_root.to_string_lossy()
        || cache.checksum != cache_checksum(&cache)
        || validate_cache(&cache).is_err()
    {
        return None;
    }
    let now_ns = system_time_ns(SystemTime::now());
    let max_age_ns = EXTERNAL_MATERIALIZATION_HASH_CACHE_MAX_AGE_SECS.saturating_mul(1_000_000_000);
    if cache.generated_at_unix_ns == 0
        || cache.generated_at_unix_ns > now_ns
        || now_ns.saturating_sub(cache.generated_at_unix_ns) > max_age_ns
    {
        return None;
    }
    Some(cache)
}

pub(super) fn write_external_materialization_hash_cache(
    repo_root: &Path,
    roots: BTreeMap<String, ExternalMaterializationHashCacheRoot>,
) -> Result<(), String> {
    let canonical_root = repo_root.canonicalize().map_err(|error| {
        format!(
            "Failed to resolve external materialization cache root {}: {error}",
            repo_root.display()
        )
    })?;
    let path = external_materialization_hash_cache_path(&canonical_root);
    let mut cache = ExternalMaterializationHashCache {
        contract: EXTERNAL_MATERIALIZATION_HASH_CACHE_CONTRACT.to_string(),
        repo_root: canonical_root.to_string_lossy().to_string(),
        generated_at_unix_ns: system_time_ns(SystemTime::now()),
        roots,
        checksum: String::new(),
    };
    validate_cache(&cache)?;
    cache.checksum = cache_checksum(&cache);
    let mut bytes = serde_json::to_vec(&cache)
        .map_err(|error| format!("Failed to encode external materialization cache: {error}"))?;
    bytes.push(b'\n');
    let parent = path.parent().ok_or_else(|| {
        format!(
            "External materialization cache path has no parent: {}",
            path.display()
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Failed to create external materialization cache directory {}: {error}",
            parent.display()
        )
    })?;
    let temporary = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("external-materialization-cache"),
        std::process::id(),
        system_time_ns(SystemTime::now())
    ));
    fs::write(&temporary, bytes).map_err(|error| {
        format!(
            "Failed to write external materialization cache temporary file {}: {error}",
            temporary.display()
        )
    })?;
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Failed to publish external materialization cache {}: {error}",
            path.display()
        ));
    }
    Ok(())
}

fn external_materialization_hash_cache_path(canonical_root: &Path) -> PathBuf {
    canonical_root
        .join(".ait-runtime")
        .join("external-materialization-hash-cache-v1.json")
}

fn validate_cache(cache: &ExternalMaterializationHashCache) -> Result<(), String> {
    if cache.repo_root.trim().is_empty() {
        return Err("External materialization cache is missing repository identity".to_string());
    }
    for (materialize_to, root) in &cache.roots {
        if materialize_to.trim().is_empty() || !valid_sha256(&root.marker_sha256) {
            return Err(
                "External materialization cache contains an invalid root identity".to_string(),
            );
        }
        for (path, entry) in &root.files {
            if path.trim().is_empty()
                || !valid_sha256(&entry.sha256)
                || (entry.fingerprint.file_kind != "file"
                    && entry.fingerprint.file_kind != "symlink")
            {
                return Err(
                    "External materialization cache contains an invalid file entry".to_string(),
                );
            }
        }
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn cache_checksum(cache: &ExternalMaterializationHashCache) -> String {
    let mut digest = Sha256::new();
    checksum_text(
        &mut digest,
        "ait-external-materialization-hash-cache-checksum/v1",
    );
    checksum_text(&mut digest, &cache.contract);
    checksum_text(&mut digest, &cache.repo_root);
    checksum_u64(&mut digest, cache.generated_at_unix_ns);
    checksum_u64(&mut digest, cache.roots.len() as u64);
    for (materialize_to, root) in &cache.roots {
        checksum_text(&mut digest, materialize_to);
        checksum_text(&mut digest, &root.marker_sha256);
        checksum_u64(&mut digest, root.files.len() as u64);
        for (path, entry) in &root.files {
            checksum_text(&mut digest, path);
            checksum_text(&mut digest, &entry.sha256);
            checksum_text(&mut digest, &entry.fingerprint.file_kind);
            checksum_u64(&mut digest, entry.fingerprint.size_bytes);
            checksum_u64(&mut digest, u64::from(entry.fingerprint.mode_bits));
            checksum_u64(&mut digest, entry.fingerprint.modified_ns);
            checksum_u64(&mut digest, entry.fingerprint.changed_ns);
            checksum_u64(&mut digest, entry.fingerprint.device_id);
            checksum_u64(&mut digest, entry.fingerprint.file_id);
        }
    }
    format!("sha256:{:x}", digest.finalize())
}

fn checksum_text(digest: &mut Sha256, value: &str) {
    checksum_u64(digest, value.len() as u64);
    digest.update(value.as_bytes());
}

fn checksum_u64(digest: &mut Sha256, value: u64) {
    digest.update(value.to_le_bytes());
}

fn system_time_ns(value: SystemTime) -> u64 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_roots() -> BTreeMap<String, ExternalMaterializationHashCacheRoot> {
        BTreeMap::from([(
            ".ait-external/ait-core".to_string(),
            ExternalMaterializationHashCacheRoot {
                marker_sha256: "a".repeat(64),
                files: BTreeMap::from([(
                    "Cargo.toml".to_string(),
                    ExternalMaterializationHashCacheFile {
                        sha256: "b".repeat(64),
                        fingerprint: WorkspaceFileFingerprint {
                            file_kind: "file".to_string(),
                            size_bytes: 42,
                            mode_bits: 0o100644,
                            modified_ns: 10,
                            changed_ns: 11,
                            device_id: 12,
                            file_id: 13,
                        },
                    },
                )]),
            },
        )])
    }

    #[test]
    fn cache_round_trip_is_repo_scoped_and_checksum_validated() {
        let root = tempfile::tempdir().unwrap();
        let expected = fixture_roots();
        write_external_materialization_hash_cache(root.path(), expected.clone()).unwrap();

        let loaded = load_external_materialization_hash_cache(root.path()).unwrap();
        assert_eq!(loaded.roots, expected);

        let canonical_root = root.path().canonicalize().unwrap();
        let path = external_materialization_hash_cache_path(&canonical_root);
        let mut payload =
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap();
        payload["roots"][".ait-external/ait-core"]["files"]["Cargo.toml"]["sha256"] =
            serde_json::Value::String("c".repeat(64));
        fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap();

        assert!(load_external_materialization_hash_cache(root.path()).is_none());
    }

    #[test]
    fn expired_cache_is_never_reused() {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let path = external_materialization_hash_cache_path(&canonical_root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut cache = ExternalMaterializationHashCache {
            contract: EXTERNAL_MATERIALIZATION_HASH_CACHE_CONTRACT.to_string(),
            repo_root: canonical_root.to_string_lossy().to_string(),
            generated_at_unix_ns: 1,
            roots: fixture_roots(),
            checksum: String::new(),
        };
        cache.checksum = cache_checksum(&cache);
        fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();

        assert!(load_external_materialization_hash_cache(root.path()).is_none());
    }
}
