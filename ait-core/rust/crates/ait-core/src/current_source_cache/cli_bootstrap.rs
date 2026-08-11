use super::*;

const CLI_PROFILE_METADATA_KEY: &str = "ait_cli_profile";
const CLI_MTIME_METADATA_KEY: &str = "ait_cli_mtime_ns";
const CLI_SHA256_METADATA_KEY: &str = "ait_cli_sha256";
const CORE_FINGERPRINT_METADATA_KEY: &str = "core_source_fingerprint";
const CORE_MTIME_METADATA_KEY: &str = "core_source_mtime_ns";

pub fn validate_current_source_cli_bootstrap(
    request: &CurrentSourceCliBootstrapRequest,
) -> Result<CurrentSourceCliBootstrapValidation, String> {
    let request = {
        let _range = crate::perfetto_range!("ait.core.current_source_cli.resolve_paths");
        CurrentSourceCliBootstrapRequest {
            core_repo_root: resolve_path_strict_false(&request.core_repo_root),
            metadata_path: resolve_path_strict_false(&request.metadata_path),
            executable_path: resolve_path_strict_false(&request.executable_path),
        }
    };
    validate_current_source_cli_bootstrap_with_stores(
        &FilesystemCurrentSourceNativeCacheSourceStore,
        &FilesystemCurrentSourceNativeCacheArtifactStore,
        &request,
    )
}

pub(super) fn validate_current_source_cli_bootstrap_with_stores<S, A>(
    source_store: &S,
    artifact_store: &A,
    request: &CurrentSourceCliBootstrapRequest,
) -> Result<CurrentSourceCliBootstrapValidation, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
    A: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    let metadata = {
        let _range = crate::perfetto_range!("ait.core.current_source_cli.metadata_load");
        load_metadata_with_current_source_native_cache_artifact_store(
            artifact_store,
            &request.metadata_path,
        )
    };

    let (recorded_source_mtime, recorded_fingerprint, profile, recorded_cli_mtime, recorded_sha256) = {
        let _range = crate::perfetto_range!("ait.core.current_source_cli.metadata_validate");
        (
            required_metadata_u64(&metadata, CORE_MTIME_METADATA_KEY, &request.metadata_path)?,
            required_metadata_text(
                &metadata,
                CORE_FINGERPRINT_METADATA_KEY,
                &request.metadata_path,
            )?,
            required_metadata_text(&metadata, CLI_PROFILE_METADATA_KEY, &request.metadata_path)?,
            required_metadata_u64(&metadata, CLI_MTIME_METADATA_KEY, &request.metadata_path)?,
            required_metadata_text(&metadata, CLI_SHA256_METADATA_KEY, &request.metadata_path)?,
        )
    };
    if profile != CURRENT_SOURCE_CACHE_BINARY_PROFILE {
        return Err(stale_cli_error(format!(
            "published ait-cli profile is {profile:?}, expected {CURRENT_SOURCE_CACHE_BINARY_PROFILE:?}"
        )));
    }

    let current_source_mtime = {
        let _range = crate::perfetto_range!("ait.core.current_source_cli.source_mtime");
        current_core_source_mtime_ns_with_source_store(source_store, &request.core_repo_root)?
    };
    if recorded_source_mtime != current_source_mtime {
        return Err(stale_cli_error(format!(
            "core source mtime changed (built={recorded_source_mtime}, current={current_source_mtime})"
        )));
    }

    let executable_is_valid = {
        let _range = crate::perfetto_range!("ait.core.current_source_cli.executable_mode");
        artifact_is_executable_with_current_source_native_cache_artifact_store(
            artifact_store,
            &request.executable_path,
        )
    };
    if !executable_is_valid {
        return Err(format!(
            "Current-source CLI bootstrap executable is missing or not executable at {}.",
            request.executable_path.display()
        ));
    }
    let current_cli_mtime = {
        let _range = crate::perfetto_range!("ait.core.current_source_cli.executable_mtime");
        artifact_mtime_ns_with_current_source_native_cache_artifact_store(
            artifact_store,
            &request.executable_path,
        )?
    };
    if recorded_cli_mtime != current_cli_mtime {
        return Err(stale_cli_error(format!(
            "bootstrap executable mtime changed (built={recorded_cli_mtime}, current={current_cli_mtime})"
        )));
    }

    Ok(CurrentSourceCliBootstrapValidation {
        core_repo_root: request.core_repo_root.clone(),
        source_mtime_ns: current_source_mtime,
        source_fingerprint: recorded_fingerprint,
        executable_sha256: recorded_sha256,
    })
}

fn required_metadata_text(
    metadata: &JsonMap<String, JsonValue>,
    key: &str,
    metadata_path: &Path,
) -> Result<String, String> {
    metadata_text(metadata, key).ok_or_else(|| {
        format!(
            "Current-source CLI build metadata at {} is missing `{key}`.",
            metadata_path.display()
        )
    })
}

fn required_metadata_u64(
    metadata: &JsonMap<String, JsonValue>,
    key: &str,
    metadata_path: &Path,
) -> Result<u64, String> {
    metadata_u64(metadata, key).ok_or_else(|| {
        format!(
            "Current-source CLI build metadata at {} is missing or invalid for `{key}`.",
            metadata_path.display()
        )
    })
}

fn stale_cli_error(reason: impl AsRef<str>) -> String {
    format!(
        "Current-source ait-cli is stale because {}. Run `./ait.sh core build` from the stable `ait` repository root and retry.",
        reason.as_ref()
    )
}
