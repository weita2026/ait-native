use serde_json::{json, Map as JsonMap, Value as JsonValue};

use crate::foundation::workflow_artifacts::{
    release_artifact_media_type, RELEASE_ARTIFACT_PACK_FORMAT_V1,
};

pub const BINARY_BODY_ROUTE_CONTRACT_VERSION: &str = "ait.server.binary_body_routes.v1";
pub const SERVER_API_CLIENT_GLUE_REFERENCE_MODULE: &str = "../ait/src/ait_native/server_api.py";

pub fn binary_body_route_contract() -> JsonValue {
    json!({
        "contract": BINARY_BODY_ROUTE_CONTRACT_VERSION,
        "reference_modules": [
            SERVER_API_CLIENT_GLUE_REFERENCE_MODULE,
        ],
        "release_artifact": {
            "method": "GET",
            "path": "/v1/native/releases/:release_id/artifacts/:artifact_kind",
            "content_pack_format": RELEASE_ARTIFACT_PACK_FORMAT_V1,
            "response_headers": {
                "content-disposition": "attachment; filename=\"{filename}\"",
                "etag": "{artifact.sha256 || \"\"}",
                "content-type": "{artifact.media_type || application/octet-stream}",
            },
            "filename_fallback_order": [
                "artifact.download_name",
                "artifact.path",
                "{release_id}-{artifact_kind}",
            ],
            "media_type_fallback": "application/octet-stream",
            "media_type_examples": release_artifact_media_type_examples(),
            "storage_scope": "Release artifact bytes remain service/storage-owned; ../ait Python may only call the Rust binary-body API.",
        },
        "compatibility_notes": {
            "python_release_helper": "release_route_helpers.py and server_store.py are deleted; the Rust binary-body route contract remains the authority.",
            "task_dag": "Task DAG is retired; no binary-body helper remains.",
        },
    })
}

pub fn release_artifact_response_contract(
    release_id: &str,
    artifact_kind: &str,
    artifact: &JsonValue,
) -> Result<JsonValue, String> {
    let artifact = artifact
        .as_object()
        .ok_or_else(|| "release artifact contract payload must be a JSON object.".to_string())?;
    let filename = artifact_text(artifact, "download_name")
        .or_else(|| artifact_text(artifact, "path"))
        .unwrap_or_else(|| format!("{release_id}-{artifact_kind}"));
    let media_type = artifact_text(artifact, "media_type")
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let etag = artifact_text(artifact, "sha256").unwrap_or_default();
    Ok(json!({
        "release_id": release_id,
        "artifact_kind": artifact_kind,
        "filename": filename,
        "media_type": media_type,
        "headers": {
            "content-disposition": format!("attachment; filename=\"{filename}\""),
            "etag": etag,
            "content-type": media_type,
        },
    }))
}

fn release_artifact_media_type_examples() -> JsonValue {
    json!([
        {
            "kind": "manifest",
            "path": "dist/release.manifest.json",
            "media_type": release_artifact_media_type("manifest", "dist/release.manifest.json"),
        },
        {
            "kind": "checksum",
            "path": "dist/ait.sha256",
            "media_type": release_artifact_media_type("checksum", "dist/ait.sha256"),
        },
        {
            "kind": "formula",
            "path": "Formula/ait.rb",
            "media_type": release_artifact_media_type("formula", "Formula/ait.rb"),
        },
        {
            "kind": "wheel",
            "path": "dist/ait-1.0.0-py3-none-any.whl",
            "media_type": release_artifact_media_type("wheel", "dist/ait-1.0.0-py3-none-any.whl"),
        },
        {
            "kind": "sdist",
            "path": "dist/ait-1.0.0.tar.gz",
            "media_type": release_artifact_media_type("sdist", "dist/ait-1.0.0.tar.gz"),
        },
    ])
}

fn artifact_text(artifact: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    artifact
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}
