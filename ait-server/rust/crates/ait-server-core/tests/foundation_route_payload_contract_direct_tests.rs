use ait_server_core::foundation::route_payload_contract::{
    native_route_payload_contract, native_route_payload_contract_version,
    native_route_payload_model_names, NativeRoutePayloadJson,
    NATIVE_ROUTE_PAYLOAD_CONTRACT_VERSION, RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
};
use serde_json::{json, Value as JsonValue};

fn model<'a>(payload: &'a JsonValue, name: &str) -> &'a JsonValue {
    payload["models"]
        .as_array()
        .expect("models should be an array")
        .iter()
        .find(|model| model["model"] == name)
        .unwrap_or_else(|| panic!("missing model {name}"))
}

fn field<'a>(model: &'a JsonValue, name: &str) -> &'a JsonValue {
    model["fields"]
        .as_array()
        .expect("fields should be an array")
        .iter()
        .find(|field| field["name"] == name)
        .unwrap_or_else(|| panic!("missing field {name}"))
}

#[test]
fn native_route_payload_contract_names_only_rust_authority_modules() {
    let payload = native_route_payload_contract();

    assert_eq!(payload["contract"], NATIVE_ROUTE_PAYLOAD_CONTRACT_VERSION);
    assert_eq!(
        native_route_payload_contract_version(),
        NATIVE_ROUTE_PAYLOAD_CONTRACT_VERSION
    );
    assert_eq!(
        payload["rust_authority_modules"],
        json!([RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE])
    );
    assert_eq!(payload["reference_modules"], json!([]));

    let model_names = native_route_payload_model_names();
    assert_eq!(model_names.len(), 35);
    assert!(model_names.contains(&"RepositoryCreate".to_string()));
    assert!(model_names.contains(&"ReleasePublishRequest".to_string()));
    assert!(model_names.contains(&"PlanRevisionArtifactsPut".to_string()));
    assert!(!model_names
        .iter()
        .any(|name| name.starts_with("Session") || name.starts_with("Telegram")));
    assert!(!model_names
        .iter()
        .any(|name| name.to_ascii_lowercase().contains("taskdag")));
    assert!(!model_names
        .iter()
        .any(|name| name.to_ascii_lowercase().contains("task_dag")));

    assert_eq!(
        payload["compatibility_notes"]["task_dag"],
        "Task DAG is retired; no route payload helper remains."
    );
    assert_eq!(
        payload["compatibility_notes"]["planning_routes"],
        "Planning DTO authority is Rust-owned with no Python route shell."
    );
}

#[test]
fn native_route_payload_contract_preserves_representative_defaults() {
    let payload = NativeRoutePayloadJson::stateless().contract_json();

    let repository_create = model(&payload, "RepositoryCreate");
    assert_eq!(
        repository_create["source_module"],
        RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE
    );
    assert_eq!(field(repository_create, "repo_name")["required"], true);
    assert_eq!(
        field(repository_create, "default_line")["default"],
        json!({"kind": "literal", "value": "main"})
    );
    assert_eq!(
        field(repository_create, "policy")["default"],
        json!({"kind": "factory", "factory": "dict", "value": {}})
    );

    let patchset_publish = model(&payload, "PatchsetPublish");
    let author_mode = field(patchset_publish, "author_mode");
    assert_eq!(author_mode["type"], json!("author_mode"));
    assert_eq!(
        author_mode["default"],
        json!({"kind": "literal", "value": "ai_with_human_review"})
    );
    assert_eq!(
        author_mode["enum_values"],
        json!([
            "human_only",
            "human_with_ai_assist",
            "ai_with_human_review",
            "ai_only_experimental"
        ])
    );

    let release = model(&payload, "ReleasePublishRequest");
    assert_eq!(
        field(release, "artifacts")["default"],
        json!({"kind": "factory", "factory": "list", "value": []})
    );
    assert_eq!(
        field(release, "artifacts")["type"],
        json!("list<ReleaseArtifactUpload>")
    );

    let repo_ci = model(&payload, "RunRepoCiRequest");
    assert_eq!(
        field(repo_ci, "target_line")["default"],
        json!({"kind": "literal", "value": "main"})
    );
    assert_eq!(
        field(repo_ci, "dependency_evidence")["default"],
        json!({"kind": "factory", "factory": "list", "value": []})
    );

    let artifact = model(&payload, "PlanRevisionArtifactPutItem");
    assert_eq!(
        artifact["source_module"],
        RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE
    );
    assert_eq!(
        field(artifact, "media_type")["default"],
        json!({"kind": "literal", "value": "application/octet-stream"})
    );
    assert_eq!(
        field(artifact, "encoding")["default"],
        json!({"kind": "literal", "value": "utf-8"})
    );

    let planning_session = model(&payload, "PlanningSessionCreate");
    assert_eq!(
        field(planning_session, "mode")["default"],
        json!({"kind": "literal", "value": "connected_local"})
    );
    assert_eq!(
        field(planning_session, "resume_if_active")["default"],
        json!({"kind": "literal", "value": true})
    );
}

#[test]
fn retired_agent_session_models_are_absent() {
    let payload = native_route_payload_contract();
    let names = payload["models"]
        .as_array()
        .expect("models should be an array")
        .iter()
        .filter_map(|entry| entry["model"].as_str())
        .collect::<Vec<_>>();

    assert!(!names.iter().any(|name| name.starts_with("Session")));
    assert!(!names.iter().any(|name| name.starts_with("Telegram")));
    assert!(payload["compatibility_notes"]
        .get("session_transport")
        .is_none());
}
