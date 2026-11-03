use crate::JsonInput;
use ait_cli::binding_source_cache as service;
use ait_core::{current_source_cache as cache, json_support::JsonValue};
use napi::{Error, Result, Status};
use napi_derive::napi;
use std::path::{Path, PathBuf};

fn unsigned_u64(value: JsonInput, name: &str) -> Result<u64> {
    value.as_u64().ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            format!("{name} must be an unsigned 64-bit integer"),
        )
    })
}

#[napi(object)]
pub struct CurrentSourceCacheRequest {
    pub namespace_root: String,
    pub core_repo_root: String,
    pub ext_suffix: String,
    pub rustflags: Option<String>,
    pub worker_id: Option<String>,
    pub core_source_fingerprint: Option<String>,
    pub server_source_fingerprint: Option<String>,
}

impl CurrentSourceCacheRequest {
    fn into_request(self) -> cache::CurrentSourceNativeCacheRequest {
        cache::CurrentSourceNativeCacheRequest {
            namespace_root: PathBuf::from(self.namespace_root),
            core_repo_root: PathBuf::from(self.core_repo_root),
            ext_suffix: self.ext_suffix,
            rustflags: self.rustflags.unwrap_or_default(),
            worker_id: self.worker_id.unwrap_or_else(|| "shared".to_string()),
            core_source_fingerprint: self.core_source_fingerprint,
            server_source_fingerprint: self.server_source_fingerprint,
        }
    }
}

#[napi]
pub fn current_core_source_fingerprint(core_repo_root: String) -> Result<String> {
    cache::current_core_source_fingerprint(Path::new(&core_repo_root)).map_err(Error::from_reason)
}

#[napi]
pub fn current_server_source_fingerprint(server_core_repo_root: String) -> Result<String> {
    cache::current_server_source_fingerprint(Path::new(&server_core_repo_root))
        .map_err(Error::from_reason)
}

#[napi]
pub fn current_source_native_cache_contract(
    request: CurrentSourceCacheRequest,
) -> Result<JsonValue> {
    cache::current_source_native_cache_contract_json(&request.into_request())
        .map_err(Error::from_reason)
}

#[napi]
pub fn current_source_cache_mark_building(
    request: CurrentSourceCacheRequest,
    source_mtime_ns: JsonInput,
    extra_json: Option<String>,
) -> Result<JsonValue> {
    service::mark_building(
        &request.into_request(),
        unsigned_u64(source_mtime_ns, "sourceMtimeNs")?,
        extra_json.as_deref().unwrap_or("{}"),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct CurrentSourceCacheActivateOptions {
    pub extra_json: Option<String>,
    pub register_lease: Option<bool>,
    pub owner_pid: Option<JsonInput>,
}

#[napi]
pub fn current_source_cache_activate(
    request: CurrentSourceCacheRequest,
    source_mtime_ns: JsonInput,
    options: Option<CurrentSourceCacheActivateOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let owner_pid = options
        .owner_pid
        .map(|value| {
            unsigned_u64(value, "ownerPid").and_then(|value| {
                u32::try_from(value).map_err(|_| {
                    Error::new(
                        Status::InvalidArg,
                        "ownerPid must be an unsigned 32-bit integer",
                    )
                })
            })
        })
        .transpose()?;
    service::activate(
        &request.into_request(),
        unsigned_u64(source_mtime_ns, "sourceMtimeNs")?,
        options.extra_json.as_deref().unwrap_or("{}"),
        options.register_lease.unwrap_or(false),
        owner_pid,
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn current_source_cache_release_lease(
    lease_path: String,
    namespace_root: String,
    remove_unleased_ready: Option<bool>,
) -> Result<JsonValue> {
    cache::release_current_source_native_cache_lease_json(
        Path::new(&lease_path),
        Path::new(&namespace_root),
        remove_unleased_ready.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct CurrentSourceCachePruneOptions {
    pub now: Option<f64>,
    pub idle_ttl_seconds: Option<JsonInput>,
    pub build_stale_seconds: Option<JsonInput>,
    pub max_bytes: Option<JsonInput>,
    pub remove_unleased_ready: Option<bool>,
}

#[napi]
pub fn current_source_cache_prune(
    namespace_root: String,
    options: Option<CurrentSourceCachePruneOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let request = cache::CurrentSourceNativeCachePruneRequest {
        namespace_root: PathBuf::from(namespace_root),
        now: options.now,
        idle_ttl_seconds: options
            .idle_ttl_seconds
            .map(|v| unsigned_u64(v, "idleTtlSeconds"))
            .transpose()?
            .unwrap_or(cache::CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS),
        build_stale_seconds: options
            .build_stale_seconds
            .map(|v| unsigned_u64(v, "buildStaleSeconds"))
            .transpose()?
            .unwrap_or(cache::CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS),
        max_bytes: options
            .max_bytes
            .map(|v| unsigned_u64(v, "maxBytes"))
            .transpose()?
            .unwrap_or(cache::CURRENT_SOURCE_CACHE_MAX_BYTES),
        remove_unleased_ready: options.remove_unleased_ready.unwrap_or(false),
    };
    cache::prune_current_source_native_caches_json(&request).map_err(Error::from_reason)
}

#[napi]
pub fn current_source_cache_seed_canonical(
    request: CurrentSourceCacheRequest,
    repo_root: String,
    canonical_repo_root: String,
    core_source_mtime_ns: JsonInput,
) -> Result<JsonValue> {
    service::seed_canonical(
        &request.into_request(),
        Path::new(&repo_root),
        Path::new(&canonical_repo_root),
        unsigned_u64(core_source_mtime_ns, "coreSourceMtimeNs")?,
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
pub struct CurrentSourceExtensionFreshnessRequest {
    pub metadata_path: String,
    pub extension_path: String,
    pub source_mtime_ns: JsonInput,
    pub source_fingerprint: String,
}

#[napi]
pub fn current_source_cache_extension_fresh(
    request: CurrentSourceExtensionFreshnessRequest,
) -> Result<JsonValue> {
    Ok(cache::current_source_extension_is_fresh_json(
        &cache::CurrentSourceExtensionFreshnessRequest {
            metadata_path: PathBuf::from(request.metadata_path),
            extension_path: PathBuf::from(request.extension_path),
            source_mtime_ns: unsigned_u64(request.source_mtime_ns, "sourceMtimeNs")?,
            source_fingerprint: request.source_fingerprint,
        },
    ))
}

#[napi(object)]
pub struct CurrentSourceBinaryFreshnessRequest {
    pub metadata_path: String,
    pub binary_path: String,
    pub metadata_fingerprint_key: String,
    pub metadata_source_mtime_key: String,
    pub metadata_mtime_key: String,
    pub metadata_sha_key: String,
    pub source_mtime_ns: JsonInput,
    pub source_fingerprint: String,
}

#[napi]
pub fn current_source_cache_binary_fresh(
    request: CurrentSourceBinaryFreshnessRequest,
) -> Result<JsonValue> {
    Ok(cache::current_source_binary_is_fresh_json(
        &cache::CurrentSourceBinaryFreshnessRequest {
            metadata_path: PathBuf::from(request.metadata_path),
            binary_path: PathBuf::from(request.binary_path),
            metadata_fingerprint_key: request.metadata_fingerprint_key,
            metadata_source_mtime_key: request.metadata_source_mtime_key,
            metadata_mtime_key: request.metadata_mtime_key,
            metadata_sha_key: request.metadata_sha_key,
            source_mtime_ns: unsigned_u64(request.source_mtime_ns, "sourceMtimeNs")?,
            source_fingerprint: request.source_fingerprint,
        },
    ))
}

#[napi]
pub fn current_source_cache_core_fingerprint(repo_root: String) -> Result<JsonValue> {
    service::core_fingerprint(Path::new(&repo_root)).map_err(Error::from_reason)
}

#[napi]
pub fn current_source_cache_server_fingerprint(repo_root: String) -> Result<JsonValue> {
    service::server_fingerprint(Path::new(&repo_root)).map_err(Error::from_reason)
}

#[napi]
pub fn current_source_cache_core_mtime(repo_root: String) -> Result<JsonValue> {
    service::core_mtime(Path::new(&repo_root)).map_err(Error::from_reason)
}

#[napi]
pub fn current_source_cache_server_mtime(repo_root: String) -> Result<JsonValue> {
    service::server_mtime(Path::new(&repo_root)).map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_contract(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_contract(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_mark_building(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_mark_building(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_activate(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_activate(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_release_lease(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_release_lease(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_prune(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_prune(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_seed_canonical(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_seed_canonical(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_extension_fresh(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_extension_fresh(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_binary_fresh(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_binary_fresh(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_core_fingerprint(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_core_fingerprint(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_server_fingerprint(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_server_fingerprint(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_core_mtime(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_core_mtime(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_current_source_cache_server_mtime(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_current_source_cache_server_mtime(&payload)
        .map_err(Error::from_reason)
}
