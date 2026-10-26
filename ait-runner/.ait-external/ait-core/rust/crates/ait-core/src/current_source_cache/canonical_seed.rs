use super::*;

pub fn seed_current_source_native_cache_from_canonical_json(
    request: &CurrentSourceNativeCacheCanonicalSeedRequest,
) -> Result<JsonValue, String> {
    let store = FilesystemCurrentSourceNativeCacheArtifactStore;
    seed_current_source_native_cache_from_canonical_with_artifact_store(&store, request)
}

pub(super) fn seed_current_source_native_cache_from_canonical_with_artifact_store<S>(
    store: &S,
    request: &CurrentSourceNativeCacheCanonicalSeedRequest,
) -> Result<JsonValue, String>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    let repo_root = resolve_path_strict_false(&request.repo_root);
    let canonical_repo_root = resolve_path_strict_false(&request.canonical_repo_root);
    if repo_root == canonical_repo_root {
        return Ok(json!({
            "seeded": false,
            "reason": "repo_root_is_canonical",
        }));
    }
    let native_request = CurrentSourceNativeCacheRequest {
        namespace_root: request.namespace_root.clone(),
        core_repo_root: request.core_repo_root.clone(),
        core_source_fingerprint: Some(request.core_source_fingerprint.clone()),
        server_source_fingerprint: request.server_source_fingerprint.clone(),
        ext_suffix: request.ext_suffix.clone(),
        rustflags: request.rustflags.clone(),
        worker_id: request.worker_id.clone(),
    };
    let (paths, core_repo_root, _, _, _) = current_source_native_cache_paths(&native_request)?;
    let canonical_state_root = canonical_repo_root.join(".ait");
    let canonical_extension_dir = canonical_state_root
        .join("runtime-extensions")
        .join(DEFAULT_EXTENSION_MODULE);
    let canonical_metadata_path = canonical_extension_dir.join(".current-source-build.json");
    let canonical_extension_path =
        canonical_extension_dir.join(format!("{DEFAULT_EXTENSION_MODULE}{}", request.ext_suffix));
    if !current_source_extension_is_fresh_with_artifact_store(
        store,
        &canonical_metadata_path,
        &canonical_extension_path,
        request.core_source_mtime_ns,
        &request.core_source_fingerprint,
    ) {
        return Ok(json!({
            "seeded": false,
            "reason": "canonical_extension_not_fresh",
            "canonical_extension_path": path_text(&canonical_extension_path),
        }));
    }

    let extension_path = paths
        .package_dir
        .join(format!("{DEFAULT_EXTENSION_MODULE}{}", request.ext_suffix));
    publish_artifact_with_current_source_native_cache_artifact_store(
        store,
        &canonical_extension_path,
        &extension_path,
        true,
    )?;
    ensure_local_extension_init_with_current_source_native_cache_artifact_store(
        store,
        &paths.package_dir.join("__init__.py"),
    )?;

    let metadata = load_metadata_with_current_source_native_cache_artifact_store(
        store,
        &canonical_metadata_path,
    );
    let canonical_target_dir = canonical_state_root.join("cargo-target");
    let mut output = JsonMap::new();
    output.insert("seeded".to_string(), json!(true));
    output.insert(
        "cache_root".to_string(),
        json!(path_text(&paths.cache_root)),
    );
    output.insert(
        "target_dir".to_string(),
        json!(path_text(&paths.target_dir)),
    );
    output.insert(
        "extension_path".to_string(),
        json!(path_text(&extension_path)),
    );
    let mut target_metadata = JsonMap::new();
    target_metadata.insert(
        "core_source_fingerprint".to_string(),
        json!(request.core_source_fingerprint),
    );
    target_metadata.insert(
        "core_source_mtime_ns".to_string(),
        json!(request.core_source_mtime_ns),
    );
    target_metadata.insert("core_repo_root".to_string(), json!(core_repo_root));

    if let Some(canonical_cli) =
        built_ait_cli_binary_path_with_artifact_store(store, &canonical_target_dir)
    {
        if canonical_current_source_binary_can_seed_with_artifact_store(
            store,
            &canonical_cli,
            &metadata,
            "core_source_fingerprint",
            "core_source_mtime_ns",
            "ait_cli_sha256",
            request.core_source_mtime_ns,
            &request.core_source_fingerprint,
        ) {
            let target_cli = paths
                .target_dir
                .join(CURRENT_SOURCE_CACHE_BINARY_PROFILE)
                .join(
                    canonical_cli
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("ait-cli")),
                );
            publish_artifact_with_current_source_native_cache_artifact_store(
                store,
                &canonical_cli,
                &target_cli,
                false,
            )?;
            let target_cli_mtime_ns =
                artifact_mtime_ns_with_current_source_native_cache_artifact_store(
                    store,
                    &target_cli,
                )?;
            target_metadata.insert("ait_cli_mtime_ns".to_string(), json!(target_cli_mtime_ns));
            target_metadata.insert(
                "ait_cli_sha256".to_string(),
                json!(
                    artifact_sha256_hex_with_current_source_native_cache_artifact_store(
                        store,
                        &target_cli
                    )?
                ),
            );
            target_metadata.insert(
                "ait_cli_profile".to_string(),
                json!(CURRENT_SOURCE_CACHE_BINARY_PROFILE),
            );
            output.insert("ait_cli_path".to_string(), json!(path_text(&target_cli)));
        }
    }

    let server_source_fingerprint = metadata_text(&metadata, "server_source_fingerprint");
    let server_source_mtime_ns = metadata_u64(&metadata, "server_source_mtime_ns");
    let server_core_repo_root = metadata_text(&metadata, "server_core_repo_root");
    if let (Some(server_fingerprint), Some(server_mtime), Some(server_root)) = (
        server_source_fingerprint,
        server_source_mtime_ns,
        server_core_repo_root,
    ) {
        if let Some(canonical_seam) =
            built_ait_server_core_seam_binary_path_with_artifact_store(store, &canonical_target_dir)
        {
            if canonical_current_source_binary_can_seed_with_artifact_store(
                store,
                &canonical_seam,
                &metadata,
                "server_source_fingerprint",
                "server_source_mtime_ns",
                "ait_server_core_seam_sha256",
                server_mtime,
                &server_fingerprint,
            ) {
                let target_seam = paths
                    .target_dir
                    .join(CURRENT_SOURCE_CACHE_BINARY_PROFILE)
                    .join(
                        canonical_seam
                            .file_name()
                            .unwrap_or_else(|| std::ffi::OsStr::new("ait-server-core-seam")),
                    );
                publish_artifact_with_current_source_native_cache_artifact_store(
                    store,
                    &canonical_seam,
                    &target_seam,
                    false,
                )?;
                let target_seam_mtime_ns =
                    artifact_mtime_ns_with_current_source_native_cache_artifact_store(
                        store,
                        &target_seam,
                    )?;
                target_metadata.insert(
                    "server_source_fingerprint".to_string(),
                    json!(server_fingerprint),
                );
                target_metadata.insert("server_source_mtime_ns".to_string(), json!(server_mtime));
                target_metadata.insert("server_core_repo_root".to_string(), json!(server_root));
                target_metadata.insert(
                    "ait_server_core_seam_mtime_ns".to_string(),
                    json!(target_seam_mtime_ns),
                );
                target_metadata.insert(
                    "ait_server_core_seam_sha256".to_string(),
                    json!(
                        artifact_sha256_hex_with_current_source_native_cache_artifact_store(
                            store,
                            &target_seam
                        )?
                    ),
                );
                target_metadata.insert(
                    "ait_server_core_seam_profile".to_string(),
                    json!(CURRENT_SOURCE_CACHE_BINARY_PROFILE),
                );
                output.insert(
                    "ait_server_core_seam_path".to_string(),
                    json!(path_text(&target_seam)),
                );
            }
        }
    }

    write_metadata_with_current_source_native_cache_artifact_store(
        store,
        &paths.package_dir.join(".current-source-build.json"),
        &JsonValue::Object(target_metadata),
    )?;
    let mut manifest_extra = JsonMap::new();
    manifest_extra.insert("core_repo_root".to_string(), json!(core_repo_root));
    manifest_extra.insert(
        "core_source_fingerprint".to_string(),
        json!(request.core_source_fingerprint),
    );
    manifest_extra.insert(
        "seeded_from_canonical_repo_root".to_string(),
        json!(path_text(&canonical_repo_root)),
    );
    write_current_source_native_cache_manifest_json(&CurrentSourceNativeCacheManifestRequest {
        paths: paths.clone(),
        state: "ready".to_string(),
        source_mtime_ns: request.core_source_mtime_ns,
        last_used_at: None,
        size_bytes: None,
        extra: manifest_extra,
    })?;
    Ok(JsonValue::Object(output))
}

#[expect(
    clippy::too_many_arguments,
    reason = "cache admission keeps each independently validated artifact fact explicit"
)]
pub(super) fn canonical_current_source_binary_can_seed_with_artifact_store<S>(
    store: &S,
    binary_path: &Path,
    metadata: &JsonMap<String, JsonValue>,
    metadata_fingerprint_key: &str,
    metadata_source_mtime_key: &str,
    metadata_sha_key: &str,
    source_mtime_ns: u64,
    source_fingerprint: &str,
) -> bool
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    if !artifact_exists_with_current_source_native_cache_artifact_store(store, binary_path)
        || !artifact_is_executable_with_current_source_native_cache_artifact_store(
            store,
            binary_path,
        )
    {
        return false;
    }
    if metadata_text(metadata, metadata_fingerprint_key).as_deref() != Some(source_fingerprint) {
        return false;
    }
    if metadata_u64(metadata, metadata_source_mtime_key) != Some(source_mtime_ns) {
        return false;
    }
    let Ok(binary_mtime_ns) =
        artifact_mtime_ns_with_current_source_native_cache_artifact_store(store, binary_path)
    else {
        return false;
    };
    if binary_mtime_ns < source_mtime_ns {
        return false;
    }
    let Some(recorded_sha256) = metadata_text(metadata, metadata_sha_key) else {
        return false;
    };
    artifact_sha256_hex_with_current_source_native_cache_artifact_store(store, binary_path)
        .map(|sha| sha == recorded_sha256)
        .unwrap_or(false)
}
