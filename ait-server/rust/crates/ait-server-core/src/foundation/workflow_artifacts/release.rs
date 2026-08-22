use super::*;

pub const RELEASES_REFERENCE_MODULE: &str = "../ait/src/ait_native/server_api.py";
pub const RELEASE_ARTIFACT_PACK_FORMAT_V1: &str = "ait-release-artifact-pack-v1";
pub const RELEASE_ARTIFACT_PACK_MANIFEST_ENTRY: &str = "release-artifact-manifest.json";

pub fn release_artifact_download_path(release_id: &str, kind: &str) -> String {
    format!("/v1/native/releases/{release_id}/artifacts/{kind}")
}

pub fn release_artifact_media_type(kind: &str, path: &str) -> &'static str {
    let normalized_kind = kind.trim().to_ascii_lowercase();
    let normalized_path = path.trim().to_ascii_lowercase();
    if normalized_kind == "manifest" || normalized_path.ends_with(".manifest.json") {
        "application/json"
    } else if normalized_kind == "checksum"
        || normalized_path.ends_with(".sha256")
        || normalized_kind == "formula"
        || normalized_path.ends_with(".rb")
    {
        "text/plain; charset=utf-8"
    } else if normalized_kind == "wheel" || normalized_path.ends_with(".whl") {
        "application/octet-stream"
    } else if normalized_kind == "sdist" || normalized_path.ends_with(".tar.gz") {
        "application/gzip"
    } else {
        "application/octet-stream"
    }
}

pub fn sanitize_release_artifact_path(value: Option<&str>) -> String {
    let text = value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("artifact");
    let path = Path::new(text);
    if !path.is_absolute() {
        return text.to_string();
    }
    let parts = path
        .components()
        .filter_map(component_name)
        .collect::<Vec<_>>();
    if parts.len() >= 2 {
        let left = &parts[parts.len() - 2];
        let right = &parts[parts.len() - 1];
        if left == "/" {
            format!("/{right}")
        } else {
            format!("{left}/{right}")
        }
    } else {
        path_file_name(path).unwrap_or_else(|| "artifact".to_string())
    }
}

pub fn release_artifact_view(
    release_id: &str,
    artifact: &JsonMap<String, JsonValue>,
) -> JsonMap<String, JsonValue> {
    let mut out = artifact.clone();
    let kind = optional_text(out.get("kind")).unwrap_or_default();
    out.insert(
        "download_path".to_string(),
        json!(release_artifact_download_path(release_id, &kind)),
    );
    if !truthy(out.get("download_name")) {
        let fallback = format!("{release_id}-{kind}");
        let download_name = optional_text(out.get("path"))
            .and_then(|path| path_file_name(Path::new(&path)))
            .filter(|value| !value.is_empty())
            .unwrap_or(fallback);
        out.insert("download_name".to_string(), json!(download_name));
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReleaseArtifactPackValidation {
    pub artifact: JsonMap<String, JsonValue>,
    pub content: Vec<u8>,
}

pub fn validate_release_artifact_pack(
    release_id: &str,
    artifact: &JsonMap<String, JsonValue>,
) -> Result<ReleaseArtifactPackValidation, String> {
    let kind = required_text(artifact.get("kind"), "Release artifact kind")?.to_ascii_lowercase();
    let artifact_path = sanitize_release_artifact_path(Some(&required_text(
        artifact.get("path"),
        "Release artifact path",
    )?));
    let data = release_artifact_pack_content(artifact, &kind, &artifact_path)?;
    let sha256 = sha256_hex(&data);
    if let Some(expected_sha) = optional_text(artifact.get("sha256")) {
        if expected_sha != sha256 {
            return Err(format!(
                "Release artifact {kind} sha256 mismatch: expected {expected_sha}, got {sha256}"
            ));
        }
    }
    if let Some(expected_size) = optional_i64(artifact.get("size_bytes"))? {
        if expected_size != data.len() as i64 {
            return Err(format!(
                "Release artifact {kind} size mismatch: expected {expected_size}, got {}",
                data.len()
            ));
        }
    }
    let mut stored = JsonMap::new();
    stored.insert("kind".to_string(), json!(kind));
    stored.insert("path".to_string(), json!(artifact_path));
    stored.insert("size_bytes".to_string(), json!(data.len()));
    stored.insert("sha256".to_string(), json!(sha256));
    stored.insert(
        "download_name".to_string(),
        json!(path_file_name(Path::new(
            stored.get("path").and_then(JsonValue::as_str).unwrap_or("")
        ))
        .unwrap_or_else(|| {
            format!(
                "{}-{}",
                release_id,
                stored.get("kind").and_then(JsonValue::as_str).unwrap_or("")
            )
        })),
    );
    stored.insert(
        "media_type".to_string(),
        json!(release_artifact_media_type(
            stored.get("kind").and_then(JsonValue::as_str).unwrap_or(""),
            stored.get("path").and_then(JsonValue::as_str).unwrap_or(""),
        )),
    );
    Ok(ReleaseArtifactPackValidation {
        artifact: stored,
        content: data,
    })
}

pub fn release_formula_payload(
    formula: Option<&JsonMap<String, JsonValue>>,
    artifacts: &[JsonMap<String, JsonValue>],
) -> JsonMap<String, JsonValue> {
    let Some(formula) = formula.filter(|value| !value.is_empty()) else {
        return JsonMap::new();
    };
    let artifact_kind =
        optional_text(formula.get("artifact_kind")).unwrap_or_else(|| "sdist".to_string());
    let source_artifact = artifacts.iter().find(|artifact| {
        optional_text(artifact.get("kind")).as_deref() == Some(artifact_kind.as_str())
    });
    let formula_artifact = artifacts
        .iter()
        .find(|artifact| optional_text(artifact.get("kind")).as_deref() == Some("formula"));
    let mut out = JsonMap::new();
    out.insert(
        "name".to_string(),
        optional_text(formula.get("name")).map_or(JsonValue::Null, JsonValue::String),
    );
    out.insert(
        "class_name".to_string(),
        optional_text(formula.get("class_name")).map_or(JsonValue::Null, JsonValue::String),
    );
    out.insert("artifact_kind".to_string(), json!(artifact_kind));
    out.insert(
        "path".to_string(),
        formula_artifact
            .and_then(|artifact| artifact.get("path"))
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    out.insert(
        "sha256".to_string(),
        source_artifact
            .and_then(|artifact| artifact.get("sha256"))
            .cloned()
            .or_else(|| optional_text(formula.get("sha256")).map(JsonValue::String))
            .unwrap_or(JsonValue::Null),
    );
    out
}

fn release_artifact_pack_content(
    artifact: &JsonMap<String, JsonValue>,
    kind: &str,
    artifact_path: &str,
) -> Result<Vec<u8>, String> {
    let pack = required_object(artifact.get("content_pack"), "content_pack")
        .map_err(|_| format!("Release artifact {kind} is missing content_pack"))?;
    if optional_text(pack.get("pack_format")).as_deref() != Some(RELEASE_ARTIFACT_PACK_FORMAT_V1) {
        return Err(format!(
            "Release artifact {kind} content_pack has unsupported pack_format"
        ));
    }
    let entry_name = optional_text(artifact.get("content_entry_name"))
        .or_else(|| optional_text(pack.get("entry_name")))
        .ok_or_else(|| format!("Release artifact {kind} content_entry_name must not be empty"))?;
    let pack_bytes = coerce_pack_bytes(
        pack.get("bytes"),
        &format!("Release artifact {kind} content_pack.bytes"),
    )?;
    let mut archive = ZipArchive::new(Cursor::new(pack_bytes)).map_err(|exc| exc.to_string())?;
    let manifest_bytes = read_zip_entry(&mut archive, RELEASE_ARTIFACT_PACK_MANIFEST_ENTRY)
        .map_err(|_| format!("Release artifact {kind} content_pack is missing manifest"))?;
    let manifest = serde_json::from_slice::<JsonValue>(&manifest_bytes).map_err(|exc| {
        format!("Release artifact {kind} content_pack manifest is invalid JSON: {exc}")
    })?;
    let manifest = manifest.as_object().ok_or_else(|| {
        format!("Release artifact {kind} content_pack manifest must be an object")
    })?;
    if optional_text(manifest.get("pack_format")).as_deref()
        != Some(RELEASE_ARTIFACT_PACK_FORMAT_V1)
    {
        return Err(format!(
            "Release artifact {kind} content_pack manifest has unsupported pack_format"
        ));
    }
    if optional_text(manifest.get("entry_name")).as_deref() != Some(entry_name.as_str()) {
        return Err(format!(
            "Release artifact {kind} content_pack entry mismatch"
        ));
    }
    if optional_text(manifest.get("path")).as_deref() != Some(artifact_path) {
        return Err(format!(
            "Release artifact {kind} content_pack path mismatch"
        ));
    }
    read_zip_entry(&mut archive, &entry_name).map_err(|_| {
        format!("Release artifact {kind} content_pack is missing entry {entry_name:?}")
    })
}

pub fn release_row(row: &JsonMap<String, JsonValue>) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = row.clone();
    let line = out
        .remove("line_name")
        .ok_or_else(|| "Release row is missing `line_name`.".to_string())?;
    out.insert("line".to_string(), line);

    let mut package = JsonMap::new();
    package.insert(
        "name".to_string(),
        out.remove("package_name").unwrap_or(JsonValue::Null),
    );
    package.insert(
        "version".to_string(),
        out.remove("package_version").unwrap_or(JsonValue::Null),
    );
    package.insert(
        "requires_python".to_string(),
        out.remove("package_requires_python")
            .unwrap_or(JsonValue::Null),
    );

    for (source_key, target_key, default) in [
        ("checks_json", "checks", json!([])),
        ("artifacts_json", "artifacts", json!([])),
        ("formula_json", "formula", json!({})),
        ("metadata_json", "metadata", json!({})),
    ] {
        let raw = out.remove(source_key);
        out.insert(
            target_key.to_string(),
            json_loads_or_default(raw.as_ref(), default),
        );
    }

    if let Some(metadata_package) = out
        .get("metadata")
        .and_then(JsonValue::as_object)
        .and_then(|metadata| metadata.get("package"))
        .and_then(JsonValue::as_object)
    {
        for (key, value) in metadata_package {
            if !value.is_null() {
                package.insert(key.clone(), value.clone());
            }
        }
    }
    out.insert("package".to_string(), JsonValue::Object(package));

    let release_id = required_text(out.get("release_id"), "release_id")?;
    let artifacts = out
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_object)
        .map(|artifact| JsonValue::Object(release_artifact_view(&release_id, artifact)))
        .collect::<Vec<_>>();
    out.insert("artifacts".to_string(), JsonValue::Array(artifacts.clone()));

    let mut formula = out
        .get("formula")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    if !formula.is_empty() {
        let source_kind =
            optional_text(formula.get("artifact_kind")).unwrap_or_else(|| "sdist".to_string());
        let source_artifact = artifacts.iter().find_map(|artifact| {
            let object = artifact.as_object()?;
            (optional_text(object.get("kind")).as_deref() == Some(source_kind.as_str()))
                .then_some(object)
        });
        let formula_artifact = artifacts.iter().find_map(|artifact| {
            let object = artifact.as_object()?;
            (optional_text(object.get("kind")).as_deref() == Some("formula")).then_some(object)
        });
        if let Some(source_artifact) = source_artifact {
            if let Some(download_path) = source_artifact.get("download_path") {
                formula.insert("url".to_string(), download_path.clone());
            }
            formula.insert(
                "sha256".to_string(),
                source_artifact
                    .get("sha256")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
        }
        if let Some(formula_artifact) = formula_artifact {
            formula.insert(
                "path".to_string(),
                formula_artifact
                    .get("path")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            );
            if let Some(download_path) = formula_artifact.get("download_path") {
                formula.insert("download_path".to_string(), download_path.clone());
            }
        }
    }
    out.insert("formula".to_string(), JsonValue::Object(formula));
    out.insert(
        "next_action".to_string(),
        json!({
            "code": "published_remote",
            "detail": "Release is published to ait-server with downloadable release artifacts.",
        }),
    );
    Ok(out)
}

fn coerce_pack_bytes(value: Option<&JsonValue>, field: &str) -> Result<Vec<u8>, String> {
    let Some(values) = value.and_then(JsonValue::as_array) else {
        return Err(format!("{field} must be a byte array"));
    };
    values
        .iter()
        .map(|item| {
            let byte = match item {
                JsonValue::Number(number) => number.as_i64(),
                JsonValue::String(text) => text.trim().parse::<i64>().ok(),
                _ => None,
            }
            .ok_or_else(|| format!("{field} contains non-byte values"))?;
            u8::try_from(byte).map_err(|_| format!("{field} contains non-byte values"))
        })
        .collect()
}

fn read_zip_entry(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    entry_name: &str,
) -> Result<Vec<u8>, String> {
    let mut entry = archive.by_name(entry_name).map_err(|exc| exc.to_string())?;
    let mut out = Vec::new();
    entry.read_to_end(&mut out).map_err(|exc| exc.to_string())?;
    Ok(out)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
