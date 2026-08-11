use ait_server_core::foundation::binary_body_route_contract::{
    binary_body_route_contract, release_artifact_response_contract,
    BINARY_BODY_ROUTE_CONTRACT_VERSION, SERVER_API_CLIENT_GLUE_REFERENCE_MODULE,
};
use serde_json::json;

#[test]
fn binary_body_contract_has_only_current_binary_routes() {
    let contract = binary_body_route_contract();

    assert_eq!(contract["contract"], BINARY_BODY_ROUTE_CONTRACT_VERSION);
    assert_eq!(
        contract["reference_modules"],
        json!([SERVER_API_CLIENT_GLUE_REFERENCE_MODULE])
    );
    assert!(!contract["reference_modules"]
        .as_array()
        .expect("reference modules")
        .iter()
        .any(|module| module == "../ait/src/ait_server/app.py"));
    assert!(!contract["reference_modules"]
        .as_array()
        .expect("reference modules")
        .iter()
        .any(|module| module == "../ait/src/ait_server/server_store.py"));
    assert!(!contract["reference_modules"]
        .as_array()
        .expect("reference modules")
        .iter()
        .any(|module| module == "../ait/src/ait_server/release_route_helpers.py"));
    assert!(!contract["reference_modules"]
        .as_array()
        .expect("reference modules")
        .iter()
        .any(|module| module == "../ait/src/ait_server/repository_routes.py"));
    let retired_snapshot_key = ["snapshot", "pack"].join("_");
    assert!(contract.get(&retired_snapshot_key).is_none());
}

#[test]
fn binary_body_contract_covers_release_artifact_headers_and_media_types() {
    let contract = binary_body_route_contract();

    assert_eq!(
        contract["release_artifact"]["path"],
        json!("/v1/native/releases/:release_id/artifacts/:artifact_kind")
    );
    assert_eq!(
        contract["release_artifact"]["response_headers"]["content-disposition"],
        json!("attachment; filename=\"{filename}\"")
    );
    assert_eq!(
        contract["release_artifact"]["response_headers"]["etag"],
        json!("{artifact.sha256 || \"\"}")
    );
    assert!(contract["release_artifact"]["media_type_examples"]
        .as_array()
        .expect("media type examples")
        .iter()
        .any(|example| {
            example["kind"] == "sdist" && example["media_type"] == "application/gzip"
        }));
    assert_eq!(
        contract["compatibility_notes"]["task_dag"],
        json!("Task DAG is retired; no binary-body helper remains.")
    );
    assert_eq!(
        contract["compatibility_notes"]["python_release_helper"],
        json!("release_route_helpers.py and server_store.py are deleted; the Rust binary-body route contract remains the authority.")
    );
}

#[test]
fn release_artifact_response_contract_matches_python_helper_header_shape() {
    let response = release_artifact_response_contract(
        "REL-1",
        "sdist",
        &json!({
            "download_name": "ait-1.0.0.tar.gz",
            "path": "dist/ignored.tar.gz",
            "media_type": "application/gzip",
            "sha256": "abc123"
        }),
    )
    .expect("release artifact response contract");

    assert_eq!(response["filename"], json!("ait-1.0.0.tar.gz"));
    assert_eq!(
        response["headers"]["content-disposition"],
        json!("attachment; filename=\"ait-1.0.0.tar.gz\"")
    );
    assert_eq!(response["headers"]["etag"], json!("abc123"));
    assert_eq!(
        response["headers"]["content-type"],
        json!("application/gzip")
    );
}

#[test]
fn release_artifact_response_contract_uses_python_helper_fallbacks() {
    let path_fallback = release_artifact_response_contract(
        "REL-1",
        "formula",
        &json!({
            "path": "Formula/ait.rb",
            "sha256": ""
        }),
    )
    .expect("release artifact path fallback");
    assert_eq!(path_fallback["filename"], json!("Formula/ait.rb"));
    assert_eq!(
        path_fallback["headers"]["content-type"],
        json!("application/octet-stream")
    );
    assert_eq!(path_fallback["headers"]["etag"], json!(""));

    let release_kind_fallback = release_artifact_response_contract("REL-1", "wheel", &json!({}))
        .expect("release artifact release-kind fallback");
    assert_eq!(release_kind_fallback["filename"], json!("REL-1-wheel"));
}

#[test]
fn release_artifact_response_contract_rejects_non_object_payloads() {
    let error = release_artifact_response_contract("REL-1", "sdist", &json!([]))
        .expect_err("non-object artifact should fail");
    assert_eq!(
        error,
        "release artifact contract payload must be a JSON object."
    );
}
