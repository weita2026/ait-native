//! Specific current-source cache operations shared with language frontends.
use ait_core::current_source_cache::*;
use ait_core::json_support::{json, JsonValue};
use std::path::Path;

fn manifest_request(
    request: &CurrentSourceNativeCacheRequest,
    source_mtime_ns: u64,
    state: &str,
    extra_json: &str,
) -> Result<CurrentSourceNativeCacheManifestRequest, String> {
    let (paths, _, _, _, _) = current_source_native_cache_paths(request)?;
    let extra = crate::json_support::parse_value(extra_json, "--extra-json must be a JSON object")?
        .as_object()
        .cloned()
        .ok_or_else(|| "--extra-json must be a JSON object.".to_string())?;
    Ok(CurrentSourceNativeCacheManifestRequest {
        paths,
        state: state.to_string(),
        source_mtime_ns,
        last_used_at: None,
        size_bytes: None,
        extra,
    })
}

pub fn mark_building(
    request: &CurrentSourceNativeCacheRequest,
    source_mtime_ns: u64,
    extra_json: &str,
) -> Result<JsonValue, String> {
    write_current_source_native_cache_manifest_json(&manifest_request(
        request,
        source_mtime_ns,
        "building",
        extra_json,
    )?)
}

pub fn activate(
    request: &CurrentSourceNativeCacheRequest,
    source_mtime_ns: u64,
    extra_json: &str,
    register_lease: bool,
    owner_pid: Option<u32>,
) -> Result<JsonValue, String> {
    let _activate_range = ait_core::perfetto_range!("ait.core.current_source_cache.activate");
    let manifest_request = manifest_request(request, source_mtime_ns, "ready", extra_json)?;
    let paths = &manifest_request.paths;
    let mut payload = write_current_source_native_cache_manifest_json(&manifest_request)?;
    let manifest = payload.clone();
    let lease = if register_lease {
        Some(match owner_pid {
            Some(owner_pid) => register_current_source_native_cache_lease_for_owner_json(
                paths,
                &request.worker_id,
                owner_pid,
            )?,
            None => register_current_source_native_cache_lease_json(paths, &request.worker_id)?,
        })
    } else {
        None
    };
    let prune = prune_current_source_native_caches_json(&CurrentSourceNativeCachePruneRequest {
        namespace_root: paths.namespace_root.clone(),
        now: None,
        idle_ttl_seconds: CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS,
        build_stale_seconds: CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS,
        max_bytes: CURRENT_SOURCE_CACHE_MAX_BYTES,
        remove_unleased_ready: false,
    })?;
    if let Some(object) = payload.as_object_mut() {
        object.insert("manifest".to_string(), manifest);
        if let Some(lease) = lease {
            object.insert("lease".to_string(), lease);
        }
        object.insert("prune".to_string(), prune);
    }
    Ok(payload)
}

pub fn seed_canonical(
    request: &CurrentSourceNativeCacheRequest,
    repo_root: &Path,
    canonical_repo_root: &Path,
    core_source_mtime_ns: u64,
) -> Result<JsonValue, String> {
    let core_source_fingerprint = request.core_source_fingerprint.clone().ok_or_else(|| {
        "current-source-cache seed-canonical requires --core-source-fingerprint.".to_string()
    })?;
    seed_current_source_native_cache_from_canonical_json(
        &CurrentSourceNativeCacheCanonicalSeedRequest {
            namespace_root: request.namespace_root.clone(),
            core_repo_root: request.core_repo_root.clone(),
            repo_root: repo_root.to_path_buf(),
            canonical_repo_root: canonical_repo_root.to_path_buf(),
            core_source_mtime_ns,
            core_source_fingerprint,
            server_source_fingerprint: request.server_source_fingerprint.clone(),
            ext_suffix: request.ext_suffix.clone(),
            rustflags: request.rustflags.clone(),
            worker_id: request.worker_id.clone(),
        },
    )
}

macro_rules! source_identity_operation {
    ($name:ident, $target:ident, $kind:literal, $field:literal) => {
        pub fn $name(repo_root: &Path) -> Result<JsonValue, String> {
            Ok(json!({ "repo_root": repo_root.to_string_lossy().to_string(), "kind": $kind, $field: $target(repo_root)? }))
        }
    };
}

source_identity_operation!(
    core_fingerprint,
    current_core_source_fingerprint,
    "core",
    "fingerprint"
);
source_identity_operation!(
    server_fingerprint,
    current_server_source_fingerprint,
    "server",
    "fingerprint"
);
source_identity_operation!(
    core_mtime,
    current_core_source_mtime_ns,
    "core",
    "source_mtime_ns"
);
source_identity_operation!(
    server_mtime,
    current_server_source_mtime_ns,
    "server",
    "source_mtime_ns"
);
