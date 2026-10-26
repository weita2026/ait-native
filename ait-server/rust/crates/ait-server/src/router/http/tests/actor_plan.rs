use super::actor_registration::{register, request_for};
use super::*;
use ait_server_core::foundation::workflow_binary_v0::WorkflowBinaryV0Codec;

fn plan_payload(title: &str, item: &str) -> JsonValue {
    json!({"title":title,"status":"draft","summary":"Actor Plan","artifact_path":"docs/actor.md","artifact_heading":"Actor",
        "items":[{"plan_item_ref":item,"text":"Actor item","checkbox_state":"open","heading_path":["Actor"],"line_number":1}]})
}

pub(super) async fn call(
    app: &Router,
    method: &str,
    path: &str,
    payload: Option<JsonValue>,
) -> (StatusCode, JsonValue) {
    let body = payload
        .map(|value| Body::from(value.to_string()))
        .unwrap_or_else(Body::empty);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(CONTENT_TYPE, "application/json")
                .body(body)
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

fn authority_bytes(state: &ServerState) -> BTreeMap<String, Vec<u8>> {
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    let actor = db
        .resolve_record_path(&WorkflowBinaryV0Codec::actor_file())
        .unwrap();
    fs::read_dir(actor.parent().unwrap())
        .unwrap()
        .map(Result::unwrap)
        .filter(|item| {
            item.file_type().unwrap().is_file()
                && matches!(
                    item.path().extension().and_then(|v| v.to_str()),
                    Some("bin" | "idx")
                )
        })
        .map(|item| {
            (
                item.file_name().into_string().unwrap(),
                fs::read(item.path()).unwrap(),
            )
        })
        .collect()
}

async fn actors(state: &ServerState, app: &Router) {
    let first = request_for(state, 1);
    assert_eq!(register(app, first.clone()).await.1["actor_key_index"], 0);
    let mut second = first;
    second["user_name"] = json!("Bob");
    second["user_email"] = json!("bob@example.test");
    assert_eq!(register(app, second).await.1["actor_key_index"], 1);
}

#[tokio::test]
async fn actor_plan_http_mutations_retry_and_author_readback() {
    let root = TestDirectory::new("actor-plan-http");
    let state = fresh_state(root.path());
    let app = build_router_with_state(state.clone());
    actors(&state, &app).await;
    let base = "/v1/native/repository-authorities/1/sprints";
    let mut create = plan_payload("Actor Plan", "actor/item");
    create["actor_key_index"] = json!(0);
    let (status, first) = call(&app, "POST", base, Some(create.clone())).await;
    assert!(status.is_success(), "{status} {first}");
    assert_eq!(first["actor_key_index"], 0);
    assert_eq!(first["head_revision"]["actor_key_index"], 0);
    let before = authority_bytes(&state);
    create["actor_key_index"] = json!(1);
    assert_eq!(
        call(&app, "POST", base, Some(create.clone())).await.1["plan_id"],
        first["plan_id"]
    );
    assert_eq!(authority_bytes(&state), before);
    let plan_id = first["plan_id"].as_str().unwrap();
    let plan_path = format!("{base}/{plan_id}");
    let revision_path = format!("{plan_path}/revisions");
    let mut revision = create.clone();
    revision["title"] = json!("Revised Actor Plan");
    revision["expected_head_revision_id"] = first["head_revision_id"].clone();
    let (status, revised) = call(&app, "POST", &revision_path, Some(revision.clone())).await;
    assert!(status.is_success(), "{status} {revised}");
    assert_eq!(revised["actor_key_index"], 1);
    assert_eq!(revised["head_revision"]["actor_key_index"], 1);
    let before = authority_bytes(&state);
    revision["actor_key_index"] = json!(0);
    assert_eq!(
        call(&app, "POST", &revision_path, Some(revision.clone()))
            .await
            .1["head_revision_id"],
        revised["head_revision_id"]
    );
    assert_eq!(authority_bytes(&state), before);
    let (status, _) = call(
        &app,
        "PATCH",
        &plan_path,
        Some(json!({"actor_key_index":0,"status":"archived"})),
    )
    .await;
    assert!(status.is_success());
    let (status, read) = call(&app, "GET", &plan_path, None).await;
    assert!(status.is_success());
    assert_eq!(read["actor_key_index"], 0);
    assert_eq!(read["head_revision"]["actor_key_index"], 1);
    let (_, history) = call(
        &app,
        "GET",
        &format!(
            "{plan_path}/revisions/{}",
            first["head_revision_id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(history["actor_key_index"], 0);
    let before = authority_bytes(&state);
    for actor in [
        JsonValue::Null,
        json!(-1),
        json!("0"),
        json!(2),
        json!(u32::MAX),
    ] {
        let mut invalid = plan_payload("Invalid", "actor/invalid");
        invalid["actor_key_index"] = actor;
        for (method, path) in [
            ("POST", base),
            ("POST", revision_path.as_str()),
            ("PATCH", plan_path.as_str()),
        ] {
            let (status, body) = call(&app, method, path, Some(invalid.clone())).await;
            assert!(status.is_client_error(), "{method} {status} {body}");
            assert_eq!(authority_bytes(&state), before);
        }
    }
    let artifacts = format!(
        "{plan_path}/revisions/{}/artifacts",
        revised["head_revision_id"].as_str().unwrap()
    );
    for actor in [None, Some(JsonValue::Null), Some(json!(2)), Some(json!(1))] {
        let mut body = json!({"artifacts":[{"artifact_path":"docs/extra.md","role":"supporting","body":"extra","metadata":{}}]});
        if let Some(actor) = actor {
            body["actor_key_index"] = actor;
        }
        let (status, response) = call(&app, "PUT", &artifacts, Some(body)).await;
        assert!(!status.is_success(), "{response}");
        assert_eq!(authority_bytes(&state), before);
    }
}

#[tokio::test]
async fn actor_plan_atomic_task_start_validates_all_actions_persists_task_actor() {
    let root = TestDirectory::new("actor-plan-atomic-http");
    let state = fresh_state(root.path());
    let app = build_router_with_state(state.clone());
    actors(&state, &app).await;
    let path = "/v1/native/repository-authorities/1/task-start";
    let mut request = json!({"contract":"task-start-atomic/v2","actor_key_index":0,"idempotency_key":"actor/start/1","plan_item_ref":"actor/open",
        "plan":{"action":"create","payload":plan_payload("Atomic Actor Plan","actor/open")},
        "task":{"title":"Atomic Actor Task","intent":"Actor context","author_mode":"ai_with_human_review"},"change":null});
    let (status, first) = call(&app, "POST", path, Some(request.clone())).await;
    assert!(status.is_success(), "{status} {first}");
    assert!(first["change"].is_null());
    let (_, plan) = call(
        &app,
        "GET",
        "/v1/native/repository-authorities/1/sprints/PR-0",
        None,
    )
    .await;
    assert_eq!(plan["actor_key_index"], 0);
    assert_eq!(plan["head_revision"]["actor_key_index"], 0);
    let before = authority_bytes(&state);
    assert_eq!(before["task.bin"].len(), 4 + 64);
    assert_eq!(&before["task.bin"][..4], &2u32.to_le_bytes());
    request["actor_key_index"] = json!(1);
    let (status, replayed) = call(&app, "POST", path, Some(request.clone())).await;
    assert!(status.is_success(), "{status} {replayed}");
    assert_eq!(replayed["task"]["task_id"], first["task"]["task_id"]);
    assert_eq!(authority_bytes(&state), before);
    let existing = json!({"action":"existing","plan_id":plan["plan_id"],"plan_revision_id":plan["head_revision_id"]});
    request["plan"] = existing.clone();
    assert!(call(&app, "POST", path, Some(request.clone()))
        .await
        .0
        .is_success());
    assert_eq!(authority_bytes(&state), before);
    let mut revise = plan_payload("Atomic real revision", "actor/new");
    revise["expected_head_revision_id"] = plan["head_revision_id"].clone();
    request["plan"] = json!({"action":"revise","plan_id":plan["plan_id"],"payload":revise});
    request["plan_item_ref"] = json!("actor/new");
    request["idempotency_key"] = json!("actor/start/2");
    request["task"]["title"] = json!("Revised Actor Task");
    let (status, revised) = call(&app, "POST", path, Some(request.clone())).await;
    assert!(status.is_success(), "{status} {revised}");
    let (_, current) = call(
        &app,
        "GET",
        "/v1/native/repository-authorities/1/sprints/PR-0",
        None,
    )
    .await;
    assert_eq!(current["actor_key_index"], 1);
    assert_eq!(current["head_revision"]["actor_key_index"], 1);
    let before = authority_bytes(&state);
    assert_eq!(before["task.bin"].len(), 4 + 2 * 64);
    assert_eq!(&before["task.bin"][..4], &2u32.to_le_bytes());
    for operation in [
        json!({"action":"create","payload":plan_payload("Invalid","actor/invalid")}),
        existing,
        request["plan"].clone(),
    ] {
        for actor in [JsonValue::Null, json!(2), json!(u32::MAX)] {
            let mut invalid = request.clone();
            invalid["plan"] = operation.clone();
            invalid["actor_key_index"] = actor;
            assert!(call(&app, "POST", path, Some(invalid))
                .await
                .0
                .is_client_error());
            assert_eq!(authority_bytes(&state), before);
        }
    }
    let mut nested = request;
    nested["task"]["actor_key_index"] = json!(0);
    assert!(call(&app, "POST", path, Some(nested))
        .await
        .0
        .is_client_error());
    assert_eq!(authority_bytes(&state), before);
}
