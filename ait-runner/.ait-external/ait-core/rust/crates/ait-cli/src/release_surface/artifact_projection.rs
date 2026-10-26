use super::*;

pub(super) const RELEASE_ARTIFACT_PACK_FORMAT_V1: &str = "ait-release-artifact-pack-v1";
pub(super) const RELEASE_ARTIFACT_PACK_MANIFEST_ENTRY: &str = "release-artifact-manifest.json";

pub(super) fn bytes_json(data: &[u8]) -> JsonValue {
    JsonValue::Array(data.iter().map(|byte| JsonValue::from(*byte)).collect())
}

pub(super) fn pack_safe_path(path: &str) -> String {
    path.split('/')
        .filter_map(|segment| {
            let trimmed = segment.trim();
            if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
                None
            } else {
                Some(trimmed.replace('\\', "_"))
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn release_artifact_entry_name(kind: &str, path: &str, data: &[u8]) -> String {
    let safe_path = pack_safe_path(path);
    let file_name = Path::new(&safe_path)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("artifact");
    format!("artifacts/{}/{}/{}", kind, sha256_hex(data), file_name)
}

pub(super) fn release_source_entry_name(path: &str, data: &[u8]) -> String {
    format!(
        "release-source/{}/{}",
        sha256_hex(data),
        pack_safe_path(path)
    )
}

pub(super) fn release_artifact_pack(
    kind: &str,
    path: &str,
    data: &[u8],
) -> Result<JsonValue, String> {
    let entry_name = release_artifact_entry_name(kind, path, data);
    let manifest = json!({
        "pack_format": RELEASE_ARTIFACT_PACK_FORMAT_V1,
        "kind": kind,
        "path": path,
        "entry_name": entry_name,
        "sha256": sha256_hex(data),
        "size_bytes": data.len(),
    });
    let manifest_bytes =
        encode_value_pretty_to_vec(&manifest, "failed to encode release artifact pack manifest")?;
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::<u8>::new()));
    let options = FileOptions::default().compression_method(CompressionMethod::Deflated);
    writer
        .start_file(entry_name.as_str(), options)
        .map_err(|err| format!("failed to add release artifact pack entry: {err}"))?;
    writer
        .write_all(data)
        .map_err(|err| format!("failed to write release artifact pack entry: {err}"))?;
    writer
        .start_file(RELEASE_ARTIFACT_PACK_MANIFEST_ENTRY, options)
        .map_err(|err| format!("failed to add release artifact pack manifest: {err}"))?;
    writer
        .write_all(&manifest_bytes)
        .map_err(|err| format!("failed to write release artifact pack manifest: {err}"))?;
    let cursor = writer
        .finish()
        .map_err(|err| format!("failed to finish release artifact pack: {err}"))?;
    Ok(json!({
        "pack_format": RELEASE_ARTIFACT_PACK_FORMAT_V1,
        "entry_name": entry_name,
        "bytes": bytes_json(&cursor.into_inner()),
    }))
}

pub(super) fn release_publish_artifacts(
    repo: &RepoRuntime,
    record: &JsonValue,
) -> Result<Vec<JsonValue>, String> {
    let mut uploads = Vec::new();
    for artifact in record
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let kind = required_string_field(&artifact, "kind")?;
        let path = required_string_field(&artifact, "path")?;
        let source_path = resolve_artifact_path(repo, &path);
        let data = fs::read(&source_path).map_err(|err| {
            format!(
                "Release artifact {kind:?} is missing on disk: {} ({err})",
                source_path.display()
            )
        })?;
        let actual_sha256 = sha256_hex(&data);
        let recorded_sha256 = required_string_field(&artifact, "sha256")?;
        let recorded_size = artifact
            .get("size_bytes")
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| format!("Release artifact {kind:?} is missing size_bytes."))?;
        if recorded_sha256 != actual_sha256 || recorded_size != data.len() as u64 {
            return Err(format!(
                "Release artifact {kind:?} digest or size changed after build: {}.",
                source_path.display()
            ));
        }
        uploads.push(json!({
            "kind": kind,
            "path": path,
            "sha256": artifact.get("sha256").cloned().unwrap_or(JsonValue::Null),
            "size_bytes": artifact.get("size_bytes").cloned().unwrap_or(JsonValue::Number(data.len().into())),
            "content_entry_name": release_artifact_entry_name(&kind, &path, &data),
            "content_pack": release_artifact_pack(&kind, &path, &data)?,
        }));
    }
    Ok(uploads)
}

pub(super) fn release_publish_metadata(record: &JsonValue) -> JsonValue {
    let mut safe = JsonMap::new();
    if let Some(value) = record
        .get("metadata")
        .and_then(|value| value.get("source_snapshot_created_at"))
        .cloned()
    {
        safe.insert("source_snapshot_created_at".to_string(), value);
    }
    if let Some(value) = record.get("package").cloned() {
        safe.insert("package".to_string(), value);
    }
    if let Some(value) = record.get("check_summary").cloned() {
        safe.insert("check_summary".to_string(), value);
    }
    if let Some(build) = record.get("metadata").and_then(|value| value.get("build")) {
        let mut build_safe = JsonMap::new();
        for key in [
            "built_at",
            "source_date_epoch",
            "builder",
            "rust_release_profile",
            "rust_ci_profile",
        ] {
            if let Some(value) = build.get(key).cloned() {
                build_safe.insert(key.to_string(), value);
            }
        }
        safe.insert("build".to_string(), JsonValue::Object(build_safe));
    }
    if let Some(value) = record
        .get("metadata")
        .and_then(|value| value.get("native_distribution"))
        .cloned()
    {
        safe.insert("native_distribution".to_string(), value);
    }
    JsonValue::Object(safe)
}

pub(super) fn rust_release_profile_contract() -> JsonValue {
    json!({
        "cargo_profile": "release",
        "opt_level": 3,
        "debug": 0,
        "debug_assertions": false,
        "overflow_checks": false,
        "incremental": false,
        "rustc_opt_level_flag": "-C opt-level=3",
        "diagnostic_role": "publish artifact profile; optimized and not intended for single-step debugging or invariant-check validation",
        "artifact_path_policy": "release artifacts must not reference Cargo target/debug outputs",
    })
}

pub(super) fn rust_ci_profile_contract() -> JsonValue {
    json!({
        "cargo_profile": "ait-ci",
        "opt_level": 0,
        "debug": 0,
        "debug_assertions": true,
        "overflow_checks": true,
        "incremental": false,
        "profile_source": "workspace profile.ait-ci",
        "diagnostic_role": "lean test profile; preserves debug_assert! and overflow-check validation without debug symbols or incremental state",
        "artifact_path_policy": "AIT-owned tests must not reference Cargo target/debug outputs",
    })
}

pub(super) fn assert_release_metadata_has_build_profile_contracts(
    record: &JsonValue,
) -> Result<(), String> {
    let release_id = required_string_field(record, "release_id")?;
    let build = record
        .get("metadata")
        .and_then(|value| value.get("build"))
        .ok_or_else(|| {
            format!(
                "Release {release_id} is missing Rust build-profile contracts. Rerun `ait release build {release_id}`."
            )
        })?;
    let profile = build.get("rust_release_profile").ok_or_else(|| {
        format!(
            "Release {release_id} is missing the Rust release build-profile contract. Rerun `ait release build {release_id}`."
        )
    })?;
    let expected = rust_release_profile_contract();
    if profile != &expected {
        return Err(format!(
            "Release {release_id} was not built with the expected Rust release build-profile contract. Rerun `ait release build {release_id}`."
        ));
    }
    let ci_profile = build
        .get("rust_ci_profile")
        .ok_or_else(|| {
            format!(
                "Release {release_id} is missing the Rust lean-CI build-profile contract. Rerun `ait release build {release_id}`."
            )
        })?;
    let expected_ci = rust_ci_profile_contract();
    if ci_profile != &expected_ci {
        return Err(format!(
            "Release {release_id} was not built with the expected Rust lean-CI build-profile contract. Rerun `ait release build {release_id}`."
        ));
    }
    Ok(())
}

pub(super) fn assert_release_artifact_paths_are_publishable(
    release_id: &str,
    artifacts: &[JsonValue],
) -> Result<(), String> {
    let debug_paths = artifacts
        .iter()
        .filter_map(|artifact| string_field(artifact, "path"))
        .filter(|path| path_references_debug_cargo_target(path))
        .collect::<Vec<_>>();
    if debug_paths.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Release {release_id} artifact paths must not reference debug Cargo target outputs: {}",
        debug_paths.join(", ")
    ))
}

pub(super) fn path_references_debug_cargo_target(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized == "target/debug"
        || normalized.starts_with("target/debug/")
        || normalized.contains("/target/debug/")
}

pub(super) fn artifact_info(repo: &RepoRuntime, path: &Path) -> Result<JsonValue, String> {
    let data = fs::read(path).map_err(io_error)?;
    Ok(json!({
        "kind": artifact_kind(path),
        "path": relative_or_absolute(repo, path),
        "absolute_path": path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().to_string(),
        "url": file_url(path),
        "size_bytes": data.len(),
        "sha256": sha256_hex(&data),
    }))
}

pub(super) fn artifact_kind(path: &Path) -> &'static str {
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if name.ends_with(".tar.gz") {
        "sdist"
    } else if name.ends_with(".whl") {
        "wheel"
    } else if name.ends_with(".manifest.json") {
        "manifest"
    } else if name.ends_with(".sha256") {
        "checksum"
    } else if name.ends_with(".rb") {
        "formula"
    } else {
        "artifact"
    }
}
