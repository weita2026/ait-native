use crate::json_support::{json, JsonMap, JsonValue};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::file_io::{FileIoStore, FilesystemFileIoStore};
use crate::json_support::{
    read_json_object_or_empty_with_file_io_store,
    write_pretty_json_atomically_with_newline_with_file_io_store, JsonCodec, JsonEncodeOptions,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

mod artifact_ports;
mod artifact_store;
mod lease_ports;
mod lease_store;
mod manifest_ports;
mod manifest_store;
mod source_ports;
mod source_store;

use self::artifact_ports::{
    artifact_exists_with_current_source_native_cache_artifact_store,
    artifact_is_executable_with_current_source_native_cache_artifact_store,
    artifact_mtime_ns_with_current_source_native_cache_artifact_store,
    artifact_sha256_hex_with_current_source_native_cache_artifact_store,
    ensure_local_extension_init_with_current_source_native_cache_artifact_store,
    first_existing_artifact_with_current_source_native_cache_artifact_store,
    load_metadata_with_current_source_native_cache_artifact_store,
    publish_artifact_with_current_source_native_cache_artifact_store,
    write_metadata_with_current_source_native_cache_artifact_store,
    CurrentSourceNativeCacheArtifactStore,
};
use self::artifact_store::FilesystemCurrentSourceNativeCacheArtifactStore;
use self::lease_ports::{
    ensure_leases_dir_with_current_source_native_cache_lease_store,
    live_lease_paths_with_current_source_native_cache_lease_store,
    release_lease_with_current_source_native_cache_lease_store,
    write_lease_with_current_source_native_cache_lease_store, CurrentSourceNativeCacheLeaseStore,
};
use self::lease_store::FilesystemCurrentSourceNativeCacheLeaseStore;
use self::manifest_ports::{
    cache_size_bytes_with_current_source_native_cache_manifest_store,
    ensure_cache_root_with_current_source_native_cache_manifest_store,
    load_manifest_with_current_source_native_cache_manifest_store,
    write_manifest_with_current_source_native_cache_manifest_store,
    CurrentSourceNativeCacheManifestStore,
};
use self::manifest_store::FilesystemCurrentSourceNativeCacheManifestStore;
use self::source_ports::{
    path_exists_with_current_source_native_cache_source_store,
    path_is_dir_with_current_source_native_cache_source_store,
    path_mtime_ns_with_current_source_native_cache_source_store,
    read_source_dir_with_current_source_native_cache_source_store,
    read_source_file_with_current_source_native_cache_source_store,
    resolve_path_with_current_source_native_cache_source_store,
    CurrentSourceNativeCacheSourceEntryKind, CurrentSourceNativeCacheSourceStore,
};
use self::source_store::FilesystemCurrentSourceNativeCacheSourceStore;

pub const CURRENT_SOURCE_CACHE_NAMESPACE: &str = "current-source-native";
pub const CURRENT_SOURCE_CACHE_SCHEMA_VERSION: &str = "v3-source-fingerprint";
pub const CURRENT_SOURCE_CACHE_BINARY_PROFILE: &str = "release";
pub const CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS: u64 = 6 * 60 * 60;
pub const CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS: u64 = 15 * 60;
pub const CURRENT_SOURCE_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const DEFAULT_EXTENSION_MODULE: &str = "ait_py";
const LOCAL_EXTENSION_INIT: &str = r#"from . import ait_py as _ait_py
from .ait_py import *

__doc__ = _ait_py.__doc__
if hasattr(_ait_py, "__all__"):
    __all__ = _ait_py.__all__
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentSourceNativeCacheRequest {
    pub namespace_root: PathBuf,
    pub core_repo_root: PathBuf,
    pub core_source_fingerprint: Option<String>,
    pub server_source_fingerprint: Option<String>,
    pub ext_suffix: String,
    pub rustflags: String,
    pub worker_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentSourceNativeCachePaths {
    pub namespace_root: PathBuf,
    pub build_key: String,
    pub cache_root: PathBuf,
    pub runtime_extensions_root: PathBuf,
    pub package_dir: PathBuf,
    pub target_dir: PathBuf,
    pub lock_path: PathBuf,
    pub manifest_path: PathBuf,
    pub leases_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CurrentSourceNativeCacheManifestRequest {
    pub paths: CurrentSourceNativeCachePaths,
    pub state: String,
    pub source_mtime_ns: u64,
    pub last_used_at: Option<f64>,
    pub size_bytes: Option<u64>,
    pub extra: JsonMap<String, JsonValue>,
}

pub struct CurrentSourceCacheJson<S> {
    store: S,
}

impl<S> CurrentSourceCacheJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl CurrentSourceCacheJson<FilesystemFileIoStore> {
    pub fn filesystem() -> Self {
        Self::new(FilesystemFileIoStore)
    }
}

impl<S> CurrentSourceCacheJson<S>
where
    S: FileIoStore,
{
    pub fn load_object_or_empty(&self, path: &Path) -> JsonMap<String, JsonValue> {
        read_json_object_or_empty_with_file_io_store(&self.store, path)
    }

    pub fn write_pretty_json_atomically(
        &self,
        path: &Path,
        payload: &JsonValue,
    ) -> Result<(), String> {
        write_pretty_json_atomically_with_newline_with_file_io_store(
            &self.store,
            path,
            payload,
            "current-source JSON",
        )
    }
}

#[derive(Debug, Clone)]
pub struct CurrentSourceNativeCachePruneRequest {
    pub namespace_root: PathBuf,
    pub now: Option<f64>,
    pub idle_ttl_seconds: u64,
    pub build_stale_seconds: u64,
    pub max_bytes: u64,
    pub remove_unleased_ready: bool,
}

#[derive(Debug, Clone)]
pub struct CurrentSourceNativeCacheCanonicalSeedRequest {
    pub namespace_root: PathBuf,
    pub core_repo_root: PathBuf,
    pub repo_root: PathBuf,
    pub canonical_repo_root: PathBuf,
    pub core_source_mtime_ns: u64,
    pub core_source_fingerprint: String,
    pub server_source_fingerprint: Option<String>,
    pub ext_suffix: String,
    pub rustflags: String,
    pub worker_id: String,
}

#[derive(Debug, Clone)]
pub struct CurrentSourceExtensionFreshnessRequest {
    pub metadata_path: PathBuf,
    pub extension_path: PathBuf,
    pub source_mtime_ns: u64,
    pub source_fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct CurrentSourceBinaryFreshnessRequest {
    pub metadata_path: PathBuf,
    pub binary_path: PathBuf,
    pub metadata_fingerprint_key: String,
    pub metadata_source_mtime_key: String,
    pub metadata_mtime_key: String,
    pub metadata_sha_key: String,
    pub source_mtime_ns: u64,
    pub source_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentSourceIdentity {
    pub source_mtime_ns: u64,
    pub source_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentSourceCliBootstrapRequest {
    pub core_repo_root: PathBuf,
    pub metadata_path: PathBuf,
    pub executable_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentSourceCliBootstrapValidation {
    pub core_repo_root: PathBuf,
    pub source_mtime_ns: u64,
    pub source_fingerprint: String,
    pub executable_sha256: String,
}

pub fn current_source_native_cache_paths(
    request: &CurrentSourceNativeCacheRequest,
) -> Result<
    (
        CurrentSourceNativeCachePaths,
        String,
        String,
        Option<String>,
        String,
    ),
    String,
> {
    let core_repo_root = resolve_path_strict_false(&request.core_repo_root);
    let namespace_root = resolve_path_strict_false(&request.namespace_root);
    let core_source_fingerprint = match normalized_text(request.core_source_fingerprint.as_deref())
    {
        Some(value) => value,
        None => current_core_source_fingerprint(&core_repo_root)?,
    };
    let worker_id =
        normalized_text(Some(&request.worker_id)).unwrap_or_else(|| "shared".to_string());
    let server_source_fingerprint = normalized_text(request.server_source_fingerprint.as_deref());
    let build_key = current_source_native_cache_build_key(
        &core_repo_root,
        &core_source_fingerprint,
        server_source_fingerprint.as_deref(),
        &request.ext_suffix,
        &request.rustflags,
        &worker_id,
    )?;
    let cache_root = namespace_root
        .join(CURRENT_SOURCE_CACHE_NAMESPACE)
        .join(&build_key);
    let runtime_extensions_root = cache_root.join("runtime-extensions");
    let package_dir = runtime_extensions_root.join(DEFAULT_EXTENSION_MODULE);
    let target_dir = cache_root.join("cargo-target");
    let paths = CurrentSourceNativeCachePaths {
        namespace_root,
        build_key,
        cache_root: cache_root.clone(),
        runtime_extensions_root,
        package_dir,
        target_dir,
        lock_path: cache_root.join(".build.lock"),
        manifest_path: cache_root.join("manifest.json"),
        leases_dir: cache_root.join("leases"),
    };
    Ok((
        paths,
        path_text(&core_repo_root),
        core_source_fingerprint,
        server_source_fingerprint,
        worker_id,
    ))
}

pub fn current_source_native_cache_contract_json(
    request: &CurrentSourceNativeCacheRequest,
) -> Result<JsonValue, String> {
    let (paths, core_repo_root, core_source_fingerprint, server_source_fingerprint, worker_id) =
        current_source_native_cache_paths(request)?;
    Ok(json!({
        "cache_schema_version": CURRENT_SOURCE_CACHE_SCHEMA_VERSION,
        "namespace": CURRENT_SOURCE_CACHE_NAMESPACE,
        "namespace_root": path_text(&paths.namespace_root),
        "build_key": paths.build_key,
        "cache_root": path_text(&paths.cache_root),
        "runtime_extensions_root": path_text(&paths.runtime_extensions_root),
        "package_dir": path_text(&paths.package_dir),
        "target_dir": path_text(&paths.target_dir),
        "binary_profile": CURRENT_SOURCE_CACHE_BINARY_PROFILE,
        "lock_path": path_text(&paths.lock_path),
        "manifest_path": path_text(&paths.manifest_path),
        "leases_dir": path_text(&paths.leases_dir),
        "core_repo_root": core_repo_root,
        "core_source_fingerprint": core_source_fingerprint,
        "server_source_fingerprint": server_source_fingerprint,
        "ext_suffix": request.ext_suffix,
        "rustflags": request.rustflags,
        "worker_id": worker_id,
    }))
}

pub fn current_source_native_cache_build_key(
    core_repo_root: &Path,
    core_source_fingerprint: &str,
    server_source_fingerprint: Option<&str>,
    ext_suffix: &str,
    rustflags: &str,
    worker_id: &str,
) -> Result<String, String> {
    let core_source_fingerprint =
        normalized_text(Some(core_source_fingerprint)).ok_or_else(|| {
            "current-source native cache requires a non-empty core_source_fingerprint.".to_string()
        })?;
    let worker_id = normalized_text(Some(worker_id)).unwrap_or_else(|| "shared".to_string());
    let mut payload = BTreeMap::<&str, String>::new();
    payload.insert(
        "cache_schema_version",
        CURRENT_SOURCE_CACHE_SCHEMA_VERSION.to_string(),
    );
    payload.insert(
        "core_repo_root",
        path_text(&resolve_path_strict_false(core_repo_root)),
    );
    payload.insert("core_source_fingerprint", core_source_fingerprint);
    payload.insert("ext_suffix", ext_suffix.to_string());
    payload.insert("rustflags", rustflags.to_string());
    payload.insert(
        "server_source_fingerprint",
        normalized_text(server_source_fingerprint).unwrap_or_default(),
    );
    payload.insert("worker_id", worker_id);
    let encoded = JsonCodec::encode_serializable(&payload, JsonEncodeOptions::compact())
        .map_err(String::from)?;
    Ok(sha256_hex(encoded.as_bytes())[..16].to_string())
}

mod artifact_publication;
mod canonical_seed;
mod cli_bootstrap;
mod filesystem_utils;
mod freshness;
mod lease_lifecycle;
mod manifest;
mod pruning;
mod source_fingerprint;

use self::artifact_publication::*;
pub use self::canonical_seed::*;
pub use self::cli_bootstrap::*;
pub use self::filesystem_utils::*;
pub use self::freshness::*;
pub use self::lease_lifecycle::*;
pub use self::manifest::*;
pub use self::pruning::*;
pub use self::source_fingerprint::*;

#[cfg(test)]
mod tests;
