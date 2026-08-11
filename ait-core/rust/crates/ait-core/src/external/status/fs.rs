use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use super::materialization_hash_cache::{
    load_external_materialization_hash_cache, write_external_materialization_hash_cache,
    ExternalMaterializationHashCacheFile, ExternalMaterializationHashCacheRoot,
};
use crate::external::bindings::inspect_external_binding_paths;
use crate::external::link::{ExternalLinkStore, FsExternalLinkStore};
use crate::external::lockfile::{ExternalLockNode, ExternalLockfile};
use crate::external::manifest::ExternalManifest;
use crate::external::materializer::{
    ExternalLocalLinkOverride, ExternalMaterializerMarkerFileEntry, ExternalMaterializerMarkerJson,
    ExternalMaterializerMarkerRecord, ExternalMaterializerMarkerV3, EXTERNAL_MATERIALIZER_MARKER,
};
use crate::external::status::model::{
    build_external_status_report, ExternalCurrentSourceArtifactRole,
    ExternalCurrentSourceArtifactState, ExternalCurrentSourceArtifactStatus,
    ExternalCurrentSourceCoreStatus, ExternalMaterializationObservation,
    ExternalObservedMaterializationState, ExternalStatusInput, ExternalStatusReport,
};
use crate::external::update::{ExternalUpdateStore, FilesystemExternalUpdateStore};
use crate::external::{ExternalError, ExternalResult};
use crate::json_support::{JsonCodec, JsonMap, JsonValue};
use crate::workspace_hash_cache::{
    workspace_file_fingerprint, workspace_file_fingerprint_from_metadata,
};

const EXTERNAL_MANIFEST_FILE: &str = "ait-external.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFilesystemStatusReport {
    pub manifest_present: bool,
    pub report: ExternalStatusReport,
}

pub fn inspect_external_filesystem_status_report(
    repo_root: impl AsRef<Path>,
    repo_name: impl Into<String>,
) -> ExternalResult<ExternalFilesystemStatusReport> {
    let repo_root = repo_root.as_ref();
    let manifest_present = manifest_file_present(repo_root)?;
    let update_store = FilesystemExternalUpdateStore::for_repo_root(repo_root)?;
    let manifest = update_store.read_manifest()?;
    let lockfile = update_store.read_lockfile()?;
    let link_store = FsExternalLinkStore::for_repo_root(repo_root);
    let local_links = link_store.load_links()?;
    let report =
        inspect_external_status_report(repo_root, repo_name, manifest, lockfile, local_links)?;
    Ok(ExternalFilesystemStatusReport {
        manifest_present,
        report,
    })
}

pub fn inspect_operational_external_projection_roots(
    repo_root: impl AsRef<Path>,
    repo_name: impl Into<String>,
) -> ExternalResult<Vec<String>> {
    let repo_root = repo_root.as_ref();
    let update_store = FilesystemExternalUpdateStore::for_repo_root(repo_root)?;
    let manifest = update_store.read_manifest()?;
    let lockfile = update_store.read_lockfile()?;
    let link_store = FsExternalLinkStore::for_repo_root(repo_root);
    let local_links = link_store.load_links()?;
    let report = inspect_external_status_report_with_diagnostics(
        repo_root,
        repo_name.into(),
        manifest,
        lockfile,
        local_links,
        false,
    )?;
    Ok(report
        .externals
        .into_iter()
        .filter(|entry| entry.state == crate::external::status::ExternalStatusState::Materialized)
        .filter(|entry| !entry.lock_drift)
        .map(|entry| entry.materialize_to)
        .collect())
}

pub fn inspect_external_status_report(
    repo_root: impl AsRef<Path>,
    repo_name: impl Into<String>,
    manifest: ExternalManifest,
    lockfile: Option<ExternalLockfile>,
    local_links: Vec<ExternalLocalLinkOverride>,
) -> ExternalResult<ExternalStatusReport> {
    inspect_external_status_report_with_diagnostics(
        repo_root.as_ref(),
        repo_name.into(),
        manifest,
        lockfile,
        local_links,
        true,
    )
}

fn inspect_external_status_report_with_diagnostics(
    repo_root: &Path,
    repo_name: String,
    manifest: ExternalManifest,
    lockfile: Option<ExternalLockfile>,
    local_links: Vec<ExternalLocalLinkOverride>,
    include_diagnostics: bool,
) -> ExternalResult<ExternalStatusReport> {
    let nodes = status_nodes_for_inspection(&manifest, lockfile.as_ref())?;
    let materializations = if include_diagnostics {
        nodes
            .iter()
            .map(|node| inspect_external_materialization(repo_root, node))
            .collect::<ExternalResult<Vec<_>>>()?
    } else {
        inspect_external_materializations_with_hash_cache(repo_root, &nodes)?
    };
    let binding_checks = if include_diagnostics {
        inspect_external_binding_paths(repo_root, &nodes)?
    } else {
        Vec::new()
    };
    let current_source_core = if include_diagnostics {
        inspect_current_source_core_status(repo_root)?
    } else {
        None
    };
    build_external_status_report(ExternalStatusInput {
        repo_name,
        manifest_path: "ait-external.toml".to_string(),
        lockfile_path: "ait-external.lock".to_string(),
        manifest,
        lockfile,
        local_links,
        materializations,
        binding_checks,
        current_source_core,
    })
}

fn inspect_external_materializations_with_hash_cache(
    repo_root: &Path,
    nodes: &[ExternalLockNode],
) -> ExternalResult<Vec<ExternalMaterializationObservation>> {
    let loaded_cache = {
        let _range = crate::perfetto_range!("ait.core.external_projection.hash_cache_read");
        load_external_materialization_hash_cache(repo_root)
    };
    let mut refreshed_roots = BTreeMap::new();
    let mut observations = Vec::with_capacity(nodes.len());
    {
        let _range =
            crate::perfetto_range!("ait.core.external_projection.materialization_validation");
        for node in nodes {
            let cached_root = loaded_cache
                .as_ref()
                .and_then(|cache| cache.roots.get(&node.materialize_to));
            let inspected =
                inspect_external_materialization_with_hash_cache(repo_root, node, cached_root)?;
            if let Some(cache_root) = inspected.cache_root {
                refreshed_roots.insert(node.materialize_to.clone(), cache_root);
            }
            observations.push(inspected.observation);
        }
    }
    let cache_changed = loaded_cache
        .as_ref()
        .map(|cache| cache.roots != refreshed_roots)
        .unwrap_or(!refreshed_roots.is_empty());
    if cache_changed {
        let _range = crate::perfetto_range!("ait.core.external_projection.hash_cache_write");
        let _ = write_external_materialization_hash_cache(repo_root, refreshed_roots);
    }
    Ok(observations)
}

pub fn inspect_current_source_core_status(
    repo_root: &Path,
) -> ExternalResult<Option<ExternalCurrentSourceCoreStatus>> {
    if !should_inspect_current_source_core(repo_root) {
        return Ok(None);
    }
    let metadata_path = repo_root
        .join(".ait")
        .join("runtime-extensions")
        .join("ait_py")
        .join(".current-source-build.json");
    let metadata = load_json_object_or_empty(&metadata_path)?;
    let metadata_present = metadata_path.is_file();
    let core_repo_root = metadata_text(&metadata, "core_repo_root")
        .or_else(|| current_source_core_repo_root_hint(repo_root).map(|path| path_text(&path)));
    let core_source_fingerprint = metadata_text(&metadata, "core_source_fingerprint");
    let core_source_mtime_ns = metadata_u64(&metadata, "core_source_mtime_ns");
    let active_binary_path = std::env::current_exe().ok();
    let canonical_binary_path = repo_root
        .join(".ait")
        .join("cargo-target")
        .join("release")
        .join(binary_file_name("ait-cli"));
    let debug_binary_path = repo_root
        .join(".ait")
        .join("cargo-target")
        .join("debug")
        .join(binary_file_name("ait-cli"));
    let extension_path = find_current_source_extension(repo_root);

    let active_binary_role = classify_active_binary(
        active_binary_path.as_deref(),
        &canonical_binary_path,
        &debug_binary_path,
    );
    let artifacts = vec![
        metadata_artifact(&metadata_path, metadata_present),
        canonical_binary_artifact(&canonical_binary_path, &metadata, "ait-cli"),
        active_binary_artifact(
            active_binary_path.as_deref(),
            active_binary_role,
            &canonical_binary_path,
            &metadata,
        ),
        extension_artifact(
            extension_path.as_deref(),
            core_source_mtime_ns,
            metadata_present,
        ),
    ];

    Ok(Some(ExternalCurrentSourceCoreStatus {
        repo_root: path_text(repo_root),
        metadata_path: path_text(&metadata_path),
        metadata_present,
        core_repo_root,
        core_source_fingerprint,
        core_source_mtime_ns,
        active_binary_path: active_binary_path.as_deref().map(path_text),
        active_binary_role,
        artifacts,
    }))
}

fn should_inspect_current_source_core(repo_root: &Path) -> bool {
    if repo_root.join("rust/crates/ait-core/Cargo.toml").is_file() {
        return false;
    }
    let metadata_path = repo_root
        .join(".ait")
        .join("runtime-extensions")
        .join("ait_py")
        .join(".current-source-build.json");
    let release_cli = repo_root
        .join(".ait")
        .join("cargo-target")
        .join("release")
        .join(binary_file_name("ait-cli"));
    let debug_cli = repo_root
        .join(".ait")
        .join("cargo-target")
        .join("debug")
        .join(binary_file_name("ait-cli"));
    metadata_path.exists() || release_cli.exists() || debug_cli.exists()
}

fn current_source_core_repo_root_hint(repo_root: &Path) -> Option<PathBuf> {
    std::env::var_os("AIT_EXTERNAL_CORE_REPO_ROOT")
        .map(PathBuf::from)
        .filter(|path| path.join("rust/crates/ait-core/Cargo.toml").is_file())
        .or_else(|| {
            repo_root
                .parent()
                .map(|parent| parent.join("ait-core"))
                .filter(|path| path.join("rust/crates/ait-core/Cargo.toml").is_file())
        })
}

fn metadata_artifact(path: &Path, present: bool) -> ExternalCurrentSourceArtifactStatus {
    ExternalCurrentSourceArtifactStatus {
        name: "ait_py_metadata".to_string(),
        role: ExternalCurrentSourceArtifactRole::Metadata,
        path: Some(path_text(path)),
        state: if present {
            ExternalCurrentSourceArtifactState::Ready
        } else {
            ExternalCurrentSourceArtifactState::Missing
        },
        reason: if present {
            None
        } else {
            Some("current-source build metadata is missing".to_string())
        },
        expected_profile: None,
        metadata_sha256: None,
        actual_sha256: None,
        metadata_mtime_ns: None,
        actual_mtime_ns: file_mtime_ns(path),
    }
}

fn canonical_binary_artifact(
    path: &Path,
    metadata: &JsonMap<String, JsonValue>,
    name: &str,
) -> ExternalCurrentSourceArtifactStatus {
    let metadata_sha256 = metadata_text(metadata, "ait_cli_sha256");
    let actual_sha256 = file_sha256(path).ok();
    let metadata_mtime_ns = metadata_u64(metadata, "ait_cli_mtime_ns");
    let actual_mtime_ns = file_mtime_ns(path);
    let expected_profile = metadata_text(metadata, "ait_cli_profile");
    let (state, reason) = if !path.is_file() {
        (
            ExternalCurrentSourceArtifactState::Missing,
            Some("canonical current-source ait-cli binary is missing".to_string()),
        )
    } else if !is_executable(path) {
        (
            ExternalCurrentSourceArtifactState::Stale,
            Some("canonical current-source ait-cli binary is not executable".to_string()),
        )
    } else if expected_profile.as_deref() != Some("release") {
        (
            ExternalCurrentSourceArtifactState::Stale,
            Some("current-source metadata does not record the release ait-cli profile".to_string()),
        )
    } else if metadata_sha256.as_deref() != actual_sha256.as_deref() {
        (
            ExternalCurrentSourceArtifactState::Stale,
            Some("canonical current-source ait-cli sha256 does not match metadata".to_string()),
        )
    } else if metadata_mtime_ns != actual_mtime_ns {
        (
            ExternalCurrentSourceArtifactState::Stale,
            Some("canonical current-source ait-cli mtime does not match metadata".to_string()),
        )
    } else {
        (ExternalCurrentSourceArtifactState::Ready, None)
    };
    ExternalCurrentSourceArtifactStatus {
        name: name.to_string(),
        role: ExternalCurrentSourceArtifactRole::CanonicalBinary,
        path: Some(path_text(path)),
        state,
        reason,
        expected_profile,
        metadata_sha256,
        actual_sha256,
        metadata_mtime_ns,
        actual_mtime_ns,
    }
}

fn active_binary_artifact(
    path: Option<&Path>,
    role: ExternalCurrentSourceArtifactRole,
    canonical_binary_path: &Path,
    metadata: &JsonMap<String, JsonValue>,
) -> ExternalCurrentSourceArtifactStatus {
    let metadata_sha256 = metadata_text(metadata, "ait_cli_sha256");
    let actual_sha256 = path.and_then(|path| file_sha256(path).ok());
    let metadata_mtime_ns = metadata_u64(metadata, "ait_cli_mtime_ns");
    let actual_mtime_ns = path.and_then(file_mtime_ns);
    let path_text_value = path.map(path_text);
    let canonical = same_path(path, canonical_binary_path);
    let (state, reason) = if path.is_none() {
        (
            ExternalCurrentSourceArtifactState::Missing,
            Some("active ait-cli executable path could not be resolved".to_string()),
        )
    } else if !canonical {
        (
            ExternalCurrentSourceArtifactState::WrongBinary,
            Some("active ait-cli is not the canonical current-source release binary".to_string()),
        )
    } else if metadata_sha256.as_deref() != actual_sha256.as_deref() {
        (
            ExternalCurrentSourceArtifactState::Stale,
            Some("active ait-cli sha256 does not match current-source metadata".to_string()),
        )
    } else if metadata_mtime_ns != actual_mtime_ns {
        (
            ExternalCurrentSourceArtifactState::Stale,
            Some("active ait-cli mtime does not match current-source metadata".to_string()),
        )
    } else {
        (ExternalCurrentSourceArtifactState::Ready, None)
    };
    ExternalCurrentSourceArtifactStatus {
        name: "active_ait_cli".to_string(),
        role,
        path: path_text_value,
        state,
        reason,
        expected_profile: Some("release".to_string()),
        metadata_sha256,
        actual_sha256,
        metadata_mtime_ns,
        actual_mtime_ns,
    }
}

fn extension_artifact(
    path: Option<&Path>,
    core_source_mtime_ns: Option<u64>,
    metadata_present: bool,
) -> ExternalCurrentSourceArtifactStatus {
    let actual_mtime_ns = path.and_then(file_mtime_ns);
    let (state, reason) = if !metadata_present {
        (
            ExternalCurrentSourceArtifactState::Missing,
            Some("current-source extension metadata is missing".to_string()),
        )
    } else if path.is_none() {
        (
            ExternalCurrentSourceArtifactState::Missing,
            Some("current-source ait_py extension artifact is missing".to_string()),
        )
    } else if core_source_mtime_ns.is_none() {
        (
            ExternalCurrentSourceArtifactState::Stale,
            Some("current-source metadata is missing core_source_mtime_ns".to_string()),
        )
    } else if actual_mtime_ns < core_source_mtime_ns {
        (
            ExternalCurrentSourceArtifactState::Stale,
            Some("current-source ait_py extension is older than core source metadata".to_string()),
        )
    } else {
        (ExternalCurrentSourceArtifactState::Ready, None)
    };
    ExternalCurrentSourceArtifactStatus {
        name: "ait_py_extension".to_string(),
        role: ExternalCurrentSourceArtifactRole::PythonExtension,
        path: path.map(path_text),
        state,
        reason,
        expected_profile: Some("release".to_string()),
        metadata_sha256: None,
        actual_sha256: path.and_then(|path| file_sha256(path).ok()),
        metadata_mtime_ns: core_source_mtime_ns,
        actual_mtime_ns,
    }
}

fn classify_active_binary(
    active: Option<&Path>,
    canonical: &Path,
    debug: &Path,
) -> ExternalCurrentSourceArtifactRole {
    if same_path(active, canonical) || same_path(active, debug) {
        ExternalCurrentSourceArtifactRole::ActiveBinary
    } else if active.is_some() {
        ExternalCurrentSourceArtifactRole::Unknown
    } else {
        ExternalCurrentSourceArtifactRole::ActiveBinary
    }
}

fn find_current_source_extension(repo_root: &Path) -> Option<PathBuf> {
    let package_dir = repo_root
        .join(".ait")
        .join("runtime-extensions")
        .join("ait_py");
    fs::read_dir(package_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .map(|name| {
                        name.starts_with("ait_py.")
                            && (name.ends_with(".so")
                                || name.ends_with(".pyd")
                                || name.ends_with(".dll")
                                || name.ends_with(".dylib"))
                    })
                    .unwrap_or(false)
        })
}

fn load_json_object_or_empty(path: &Path) -> ExternalResult<JsonMap<String, JsonValue>> {
    if !path.is_file() {
        return Ok(JsonMap::new());
    }
    let text = fs::read_to_string(path).map_err(|err| {
        ExternalError::with_code(
            "external_status_current_source_metadata",
            format!(
                "failed to read current-source metadata {}: {err}",
                path.display()
            ),
        )
    })?;
    match JsonCodec::parse_value_with_error_prefix(&text, "Failed to parse current-source metadata")
        .map_err(|err| {
            ExternalError::with_code("external_status_current_source_metadata", err.to_string())
        })? {
        JsonValue::Object(object) => Ok(object),
        _ => Err(ExternalError::with_code(
            "external_status_current_source_metadata",
            "current-source metadata must be a JSON object",
        )),
    }
}

fn metadata_text(metadata: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn metadata_u64(metadata: &JsonMap<String, JsonValue>, key: &str) -> Option<u64> {
    metadata.get(key).and_then(JsonValue::as_u64)
}

fn file_mtime_ns(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
}

fn file_sha256(path: &Path) -> Result<String, std::io::Error> {
    let data = fs::read(path)?;
    let mut digest = Sha256::new();
    digest.update(&data);
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn same_path(left: Option<&Path>, right: &Path) -> bool {
    let Some(left) = left else {
        return false;
    };
    normalize_path_for_compare(left) == normalize_path_for_compare(right)
}

fn normalize_path_for_compare(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn binary_file_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn manifest_file_present(repo_root: &Path) -> ExternalResult<bool> {
    match fs::metadata(repo_root.join(EXTERNAL_MANIFEST_FILE)) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(ExternalError::with_code(
            "external_status_manifest_stat",
            format!("failed to inspect {EXTERNAL_MANIFEST_FILE}: {err}"),
        )),
    }
}

pub fn inspect_external_materialization(
    repo_root: &Path,
    node: &ExternalLockNode,
) -> ExternalResult<ExternalMaterializationObservation> {
    inspect_external_materialization_with_hash_cache(repo_root, node, None)
        .map(|inspected| inspected.observation)
}

struct ExternalMaterializationInspection {
    observation: ExternalMaterializationObservation,
    cache_root: Option<ExternalMaterializationHashCacheRoot>,
}

impl ExternalMaterializationInspection {
    fn without_cache(observation: ExternalMaterializationObservation) -> Self {
        Self {
            observation,
            cache_root: None,
        }
    }
}

fn inspect_external_materialization_with_hash_cache(
    repo_root: &Path,
    node: &ExternalLockNode,
    cached_root: Option<&ExternalMaterializationHashCacheRoot>,
) -> ExternalResult<ExternalMaterializationInspection> {
    let destination = safe_destination(repo_root, &node.materialize_to)?;
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Ok(ExternalMaterializationInspection::without_cache(
                dirty_node(node, "materialization path is a symlink"),
            ))
        }
        Ok(metadata) if metadata.is_file() => Ok(ExternalMaterializationInspection::without_cache(
            dirty_node(node, "materialization path is a file"),
        )),
        Ok(metadata) if !metadata.is_dir() => Ok(ExternalMaterializationInspection::without_cache(
            dirty_node(node, "materialization path is not a directory"),
        )),
        Ok(_) => inspect_existing_directory(&destination, node, cached_root),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(ExternalMaterializationInspection::without_cache(
                ExternalMaterializationObservation::missing(node),
            ))
        }
        Err(err) => Err(ExternalError::with_code(
            "external_status_stat",
            format!(
                "failed to inspect external materialization path {:?}: {err}",
                node.materialize_to
            ),
        )),
    }
}

fn inspect_existing_directory(
    destination: &Path,
    node: &ExternalLockNode,
    cached_root: Option<&ExternalMaterializationHashCacheRoot>,
) -> ExternalResult<ExternalMaterializationInspection> {
    let marker_path = destination.join(EXTERNAL_MATERIALIZER_MARKER);
    let marker_metadata = match fs::symlink_metadata(&marker_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ExternalMaterializationInspection::without_cache(
                dirty_node(node, "generated marker is missing"),
            ));
        }
        Err(err) => {
            return Err(ExternalError::with_code(
                "external_status_stat",
                format!("failed to inspect external materialization marker: {err}"),
            ));
        }
    };
    if marker_metadata.file_type().is_symlink() || !marker_metadata.is_file() {
        return Ok(ExternalMaterializationInspection::without_cache(
            dirty_node(node, "generated marker is not a regular file"),
        ));
    }
    if generated_tree_contains_symlink(destination)? {
        return Ok(ExternalMaterializationInspection::without_cache(
            dirty_node(node, "generated materialization contains a symlink"),
        ));
    }
    let marker_sha256 = file_sha256(&marker_path).map_err(|error| {
        ExternalError::with_code(
            "external_status_marker",
            format!("failed to hash external materialization marker: {error}"),
        )
    })?;
    let matching_cached_root = cached_root.filter(|cached| cached.marker_sha256 == marker_sha256);
    match ExternalMaterializerMarkerJson::filesystem().read_marker(&marker_path)? {
        ExternalMaterializerMarkerRecord::V3(marker) => validate_materialized_directory(
            destination,
            node,
            &marker,
            &marker_sha256,
            matching_cached_root,
        ),
        ExternalMaterializerMarkerRecord::Legacy { .. } => {
            Ok(ExternalMaterializationInspection::without_cache(
                dirty_node(node, "generated marker format requires refresh"),
            ))
        }
    }
}

fn generated_tree_contains_symlink(directory: &Path) -> ExternalResult<bool> {
    for entry in fs::read_dir(directory).map_err(|err| {
        ExternalError::with_code(
            "external_status_stat",
            format!(
                "failed to inspect generated external directory {:?}: {err}",
                directory
            ),
        )
    })? {
        let entry = entry.map_err(|err| {
            ExternalError::with_code(
                "external_status_stat",
                format!(
                    "failed to inspect generated external directory {:?}: {err}",
                    directory
                ),
            )
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|err| {
            ExternalError::with_code(
                "external_status_stat",
                format!(
                    "failed to inspect generated external entry {:?}: {err}",
                    path
                ),
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Ok(true);
        }
        if metadata.is_dir() && generated_tree_contains_symlink(&path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn dirty_node(node: &ExternalLockNode, reason: &str) -> ExternalMaterializationObservation {
    ExternalMaterializationObservation::dirty(
        node.name.clone(),
        node.parent_path.clone(),
        node.materialize_to.clone(),
        reason,
    )
}

fn validate_materialized_directory(
    destination: &Path,
    node: &ExternalLockNode,
    marker: &ExternalMaterializerMarkerV3,
    marker_sha256: &str,
    cached_root: Option<&ExternalMaterializationHashCacheRoot>,
) -> ExternalResult<ExternalMaterializationInspection> {
    if marker.name != node.name
        || marker.repo_name != node.repo_name
        || marker.repository_index != node.repository_index
        || marker.remote != node.remote
        || marker.line != node.line
        || marker.parent_path != node.parent_path
        || marker.materialize_to != node.materialize_to
    {
        return Ok(ExternalMaterializationInspection::without_cache(
            dirty_node(
                node,
                "generated marker metadata does not match the lockfile entry",
            ),
        ));
    }
    let (live_files, cache_files) = collect_live_materialized_file_entries(
        destination,
        cached_root.map(|cached| &cached.files),
    )?;
    let mut live_by_path = live_files
        .into_iter()
        .map(|entry| (entry.path, entry.sha256))
        .collect::<BTreeMap<_, _>>();
    for expected in &marker.files {
        let Some(actual_sha256) = live_by_path.remove(expected.path.as_str()) else {
            return Ok(ExternalMaterializationInspection::without_cache(
                dirty_node(
                    node,
                    &format!("expected materialized file is missing: {}", expected.path),
                ),
            ));
        };
        if actual_sha256 != expected.sha256 {
            return Ok(ExternalMaterializationInspection::without_cache(
                dirty_node(
                    node,
                    &format!("materialized file content changed: {}", expected.path),
                ),
            ));
        }
    }
    if let Some((unexpected_path, _)) = live_by_path.into_iter().next() {
        return Ok(ExternalMaterializationInspection::without_cache(
            dirty_node(
                node,
                &format!("unexpected materialized file is present: {unexpected_path}"),
            ),
        ));
    }
    Ok(ExternalMaterializationInspection {
        observation: ExternalMaterializationObservation {
            name: node.name.clone(),
            parent_path: node.parent_path.clone(),
            materialize_to: node.materialize_to.clone(),
            state: ExternalObservedMaterializationState::Generated,
            snapshot: Some(marker.snapshot.clone()),
            reason: None,
        },
        cache_root: Some(ExternalMaterializationHashCacheRoot {
            marker_sha256: marker_sha256.to_string(),
            files: cache_files,
        }),
    })
}

fn collect_live_materialized_file_entries(
    destination: &Path,
    cached_files: Option<&BTreeMap<String, ExternalMaterializationHashCacheFile>>,
) -> ExternalResult<(
    Vec<ExternalMaterializerMarkerFileEntry>,
    BTreeMap<String, ExternalMaterializationHashCacheFile>,
)> {
    let mut entries = Vec::new();
    let mut refreshed_files = BTreeMap::new();
    collect_live_materialized_file_entries_recursive(
        destination,
        destination,
        cached_files,
        &mut entries,
        &mut refreshed_files,
    )?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((entries, refreshed_files))
}

fn collect_live_materialized_file_entries_recursive(
    root: &Path,
    cursor: &Path,
    cached_files: Option<&BTreeMap<String, ExternalMaterializationHashCacheFile>>,
    entries: &mut Vec<ExternalMaterializerMarkerFileEntry>,
    refreshed_files: &mut BTreeMap<String, ExternalMaterializationHashCacheFile>,
) -> ExternalResult<()> {
    for entry in fs::read_dir(cursor).map_err(|err| {
        ExternalError::with_code(
            "external_status_stat",
            format!(
                "failed to inspect generated external directory {:?}: {err}",
                cursor
            ),
        )
    })? {
        let entry = entry.map_err(|err| {
            ExternalError::with_code(
                "external_status_stat",
                format!(
                    "failed to inspect generated external directory {:?}: {err}",
                    cursor
                ),
            )
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|err| {
            ExternalError::with_code(
                "external_status_stat",
                format!(
                    "failed to inspect generated external entry {:?}: {err}",
                    path
                ),
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ExternalError::with_code(
                "external_status_stat",
                format!("generated materialization contains a symlink at {:?}", path),
            ));
        }
        if metadata.is_dir() {
            collect_live_materialized_file_entries_recursive(
                root,
                &path,
                cached_files,
                entries,
                refreshed_files,
            )?;
            continue;
        }
        if !metadata.is_file() {
            return Err(ExternalError::with_code(
                "external_status_stat",
                format!(
                    "generated materialization contains unsupported entry {:?}",
                    path
                ),
            ));
        }
        if path.file_name() == Some(OsStr::new(EXTERNAL_MATERIALIZER_MARKER)) {
            continue;
        }
        let relative = path.strip_prefix(root).map_err(|_| {
            ExternalError::with_code(
                "external_status_stat",
                format!(
                    "generated external file {:?} is outside the materialized root",
                    path
                ),
            )
        })?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        let fingerprint_before = workspace_file_fingerprint_from_metadata(&metadata);
        let sha256 = if let Some(cached) = cached_files
            .and_then(|files| files.get(&relative))
            .filter(|cached| cached.fingerprint == fingerprint_before)
        {
            cached.sha256.clone()
        } else {
            let _range = crate::perfetto_range!("ait.core.external_projection.hash_changed_file");
            let data = fs::read(&path).map_err(|err| {
                ExternalError::with_code(
                    "external_status_stat",
                    format!("failed to read generated external file {:?}: {err}", path),
                )
            })?;
            let fingerprint_after = workspace_file_fingerprint(&path)
                .map_err(|error| ExternalError::with_code("external_status_stat", error))?;
            if fingerprint_after != fingerprint_before {
                return Err(ExternalError::with_code(
                    "external_status_changed_during_read",
                    format!(
                        "generated external file {:?} changed while status was reading it",
                        path
                    ),
                ));
            }
            sha256_hex(&data)
        };
        entries.push(ExternalMaterializerMarkerFileEntry::new(
            relative.clone(),
            sha256.clone(),
        ));
        refreshed_files.insert(
            relative,
            ExternalMaterializationHashCacheFile {
                sha256,
                fingerprint: fingerprint_before,
            },
        );
    }
    Ok(())
}

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn status_nodes_for_inspection(
    manifest: &ExternalManifest,
    lockfile: Option<&ExternalLockfile>,
) -> ExternalResult<Vec<ExternalLockNode>> {
    manifest.validate()?;
    if let Some(lockfile) = lockfile {
        lockfile.validate()?;
        return Ok(lockfile.sorted_nodes());
    }
    ExternalLockfile::direct_manifest_lock(manifest).map(|lockfile| lockfile.sorted_nodes())
}

fn safe_destination(repo_root: &Path, materialize_to: &str) -> ExternalResult<PathBuf> {
    let relative = validate_repo_relative_path(materialize_to, "materialize_to")?;
    let mut destination = repo_root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            destination.push(part);
        }
    }
    ensure_existing_ancestors_are_not_symlinks(repo_root, &destination, materialize_to)?;
    Ok(destination)
}

fn validate_repo_relative_path(path: &str, field: &str) -> ExternalResult<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return Err(ExternalError::with_code(
            "external_status_path",
            format!("{field} must not be empty"),
        ));
    }
    let parsed = Path::new(path);
    if parsed.is_absolute() {
        return Err(ExternalError::with_code(
            "external_status_path",
            format!("{field} must be repository-relative, got absolute path {path:?}"),
        ));
    }

    let mut normalized = PathBuf::new();
    let mut has_normal = false;
    for component in parsed.components() {
        match component {
            Component::Normal(part) => {
                has_normal = true;
                normalized.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ExternalError::with_code(
                    "external_status_path",
                    format!("{field} must not escape the repository, got {path:?}"),
                ));
            }
        }
    }
    if !has_normal {
        return Err(ExternalError::with_code(
            "external_status_path",
            format!("{field} must contain a repository-relative path component"),
        ));
    }
    Ok(normalized)
}

fn ensure_existing_ancestors_are_not_symlinks(
    repo_root: &Path,
    destination: &Path,
    display_path: &str,
) -> ExternalResult<()> {
    let relative = destination.strip_prefix(repo_root).map_err(|_| {
        ExternalError::with_code(
            "external_status_path",
            format!("external materialization path {display_path:?} is outside the repository"),
        )
    })?;

    let mut cursor = repo_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        cursor.push(part);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ExternalError::with_code(
                    "external_status_symlink",
                    format!(
                        "external materialization path {display_path:?} crosses symlink {:?}",
                        cursor
                    ),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(ExternalError::with_code(
                    "external_status_stat",
                    format!(
                        "failed to inspect external materialization path {display_path:?}: {err}"
                    ),
                ));
            }
        }
    }
    Ok(())
}
