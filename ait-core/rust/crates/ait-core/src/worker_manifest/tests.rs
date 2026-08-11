use super::*;

#[test]
fn exposes_worker_manifest_schema() {
    let schema = worker_manifest_schema_json();
    assert_eq!(
        schema["properties"]["ir_version"]["const"].as_str(),
        Some(WORKER_MANIFEST_IR_VERSION)
    );
}

#[test]
fn worker_manifest_wrapper_preserves_default_normalize_upsert_and_selection_fixtures() {
    let contract = WorkerManifestJson::stateless();

    assert_eq!(contract.ir_version(), WORKER_MANIFEST_IR_VERSION);
    assert_eq!(contract.schema_json(), worker_manifest_schema_json());
    assert_eq!(
        contract.default_config_json(),
        json!({"version": 1, "workers": {}})
    );

    let upserted = contract
        .upsert_worker_json(&json!({
            "config": contract.default_config_json(),
            "worker": {
                "kind": "telegram",
                "name": "main",
                "token": "secret"
            },
            "updated_at": "2026-07-05T00:00:00Z"
        }))
        .expect("upsert worker");

    let normalized = contract.normalize_document_json(
        &json!({"config": upserted["config"]}),
        Some("/tmp/agent-workers.json"),
    );
    assert_eq!(
        normalized["config"]["workers"]["telegram/main"]["created_at"],
        "2026-07-05T00:00:00Z"
    );
    assert_eq!(
        contract.select_telegram_worker(&upserted["config"], Some("main"))["token"],
        "secret"
    );
}

#[test]
fn normalizes_worker_manifest_and_collects_issues() {
    let normalized = normalize_worker_manifest_document_json(
        &json!({
            "version": "bad",
            "workers": {
                "telegram/main": {
                    "kind": 123,
                    "token": 99
                }
            }
        }),
        Some("/tmp/agent-workers.json"),
    );

    assert_eq!(normalized["ir_version"], WORKER_MANIFEST_IR_VERSION);
    assert_eq!(normalized["config"]["version"], 1);
    assert_eq!(
        normalized["config"]["workers"]["telegram/main"]["kind"],
        "telegram"
    );
    assert_eq!(
        normalized["config"]["workers"]["telegram/main"]["token"],
        JsonValue::Null
    );
    assert!(!normalized["issues"]
        .as_array()
        .unwrap_or(&Vec::new())
        .is_empty());
}

#[test]
fn selects_requested_telegram_worker() {
    let selected = select_telegram_worker_json(
        &json!({
            "workers": {
                "telegram/main": {"name": "main"},
                "telegram/side": {"name": "side"}
            }
        }),
        Some("side"),
    );

    assert_eq!(selected["name"], "side");
}

#[test]
fn upserts_worker_with_rust_owned_timestamps() {
    let payload = upsert_worker_manifest_worker_json(&json!({
        "config": {"version": 1, "workers": {}},
        "worker": {
            "kind": "telegram",
            "name": "main",
            "token": "secret",
            "username": "bot"
        },
        "updated_at": "2026-07-04T06:30:00Z"
    }))
    .expect("upsert worker");

    assert_eq!(payload["worker_key"], "telegram/main");
    assert_eq!(payload["worker"]["kind"], "telegram");
    assert_eq!(payload["worker"]["name"], "main");
    assert_eq!(payload["worker"]["created_at"], "2026-07-04T06:30:00Z");
    assert_eq!(payload["worker"]["updated_at"], "2026-07-04T06:30:00Z");
    assert_eq!(
        payload["config"]["workers"]["telegram/main"]["token"],
        "secret"
    );
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert_eq!(
        payload["migration_stage"],
        "rust_agent_worker_manifest_upsert_contract"
    );
}

#[test]
fn upsert_preserves_existing_created_at_and_refreshes_updated_at() {
    let payload = upsert_worker_manifest_worker_json(&json!({
        "config": {
            "version": 1,
            "workers": {
                "line/main": {
                    "kind": "line",
                    "name": "main",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z",
                    "token": "old"
                }
            }
        },
        "worker": {
            "kind": "line",
            "name": "main",
            "token": "new",
            "secret": "secret"
        },
        "updated_at": "2026-07-04T06:31:00Z"
    }))
    .expect("upsert worker");

    assert_eq!(payload["worker_key"], "line/main");
    assert_eq!(payload["worker"]["created_at"], "2026-01-01T00:00:00Z");
    assert_eq!(payload["worker"]["updated_at"], "2026-07-04T06:31:00Z");
    assert_eq!(payload["worker"]["token"], "new");
    assert_eq!(payload["worker"]["secret"], "secret");
}

#[test]
fn upsert_rejects_invalid_worker_key_parts() {
    let err = upsert_worker_manifest_worker_json(&json!({
        "config": {"version": 1, "workers": {}},
        "worker": {"kind": "telegram", "name": "bad/name"}
    }))
    .expect_err("slash in worker name should fail");

    assert!(err.contains("worker.name must not contain"));
}
