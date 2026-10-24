use super::*;
use ait_server_core::foundation::remote_binary_db::FilesystemServerRemoteBinaryDb;
use ait_server_core::foundation::workflow_binary_v0::WorkflowBinaryV0Codec;

pub(super) fn request_for(state: &ServerState, index: u32) -> JsonValue {
    let (entry, _) = state.operational_binary.repository_db(index).unwrap();
    let namespace = entry
        .record
        .namespace_ascii
        .into_iter()
        .filter(|byte| *byte != 0)
        .map(char::from)
        .collect::<String>();
    json!({"repository_index":index,"repository_name":entry.repo_name,"namespace":namespace,
        "policy_flags":entry.record.policy_flags,"user_name":" Alice ","user_email":" alice@example.test "})
}

pub(super) async fn register(app: &Router, payload: JsonValue) -> (StatusCode, JsonValue) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/native/repository-authorities")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body()).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes))),
    )
}

fn actor_bytes(db: &FilesystemServerRemoteBinaryDb) -> Vec<Vec<u8>> {
    [
        db.resolve_record_path(&WorkflowBinaryV0Codec::actor_file())
            .unwrap(),
        db.resolve_payload_path(&WorkflowBinaryV0Codec::actor_payload_file())
            .unwrap(),
        db.resolve_index_path(&WorkflowBinaryV0Codec::actor_lookup_index())
            .unwrap(),
    ]
    .iter()
    .map(|path| fs::read(path).unwrap())
    .collect()
}

#[tokio::test]
async fn actor_registration_http_exact_selector_and_request_identity() {
    let root = TestDirectory::new("actor-registration-http");
    let state = fresh_state(root.path());
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    let payload = request_for(&state, 1);
    let app = build_router_with_state(state.clone());
    let (a, b, c) = tokio::join!(
        register(&app, payload.clone()),
        register(&app, payload.clone()),
        register(&app, payload.clone())
    );
    for (status, body) in [a, b, c] {
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["actor_key_index"], 0);
        assert_eq!(body["repository"]["repository_index"], 1);
        assert_eq!(body["created"], false);
    }
    assert_eq!(
        fs::read(
            db.resolve_record_path(&WorkflowBinaryV0Codec::actor_file())
                .unwrap()
        )
        .unwrap()
        .len(),
        4 + 36
    );
    let mut changed = payload.clone();
    changed["user_email"] = json!("other@example.test");
    let (status, body) = register(&app, changed).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["actor_key_index"], 1);
    let (status, body) = register(&app, request_for(&state, 0)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["actor_key_index"], 0);
    assert_eq!(body["repository"]["repository_index"], 0);
    let mut legacy = payload.clone();
    legacy.as_object_mut().unwrap().remove("user_name");
    legacy.as_object_mut().unwrap().remove("user_email");
    let before = actor_bytes(&db);
    let (status, body) = register(&app, legacy).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("actor_key_index").is_none());
    assert_eq!(actor_bytes(&db), before);
    let (status, body) = register(
        &app,
        json!({"repository_name":"actor-new","namespace":"AX","policy_flags":131,
        "user_name":"Alice","user_email":"alice@example.test"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["repository"]["repository_index"], 4);
    assert_eq!(body["actor_key_index"], 0);
    assert_eq!(state.operational_binary.repository_indexes().len(), 5);
}

#[tokio::test]
async fn actor_registration_http_rejects_stale_selector_and_invalid_identity_without_effects() {
    let root = TestDirectory::new("actor-registration-http-invalid");
    let state = fresh_state(root.path());
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    let payload = request_for(&state, 1);
    let app = build_router_with_state(state.clone());
    let before = actor_bytes(&db);
    for (field, value) in [
        ("repository_name", json!("different")),
        ("namespace", json!("ZZ")),
        ("policy_flags", json!(0)),
    ] {
        let mut invalid = payload.clone();
        invalid[field] = value;
        let (status, body) = register(&app, invalid).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(actor_bytes(&db), before);
    }
    for (field, value) in [
        ("repository_index", json!(999)),
        ("repository_index", JsonValue::Null),
        ("user_name", JsonValue::Null),
        ("user_email", JsonValue::Null),
        ("user_name", json!(" ")),
        ("user_email", json!(" ")),
        ("user_name", json!("é".repeat(128))),
        ("user_email", json!("a".repeat(256))),
    ] {
        let mut invalid = payload.clone();
        invalid[field] = value;
        let (status, body) = register(&app, invalid).await;
        assert!(status.is_client_error(), "{status} {body}");
        assert_eq!(actor_bytes(&db), before);
        assert_eq!(state.operational_binary.repository_indexes().len(), 4);
    }
    let mut missing = payload;
    missing.as_object_mut().unwrap().remove("user_email");
    assert!(register(&app, missing).await.0.is_client_error());
    assert_eq!(actor_bytes(&db), before);
}
