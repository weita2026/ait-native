use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ait_core::json_support::{json, JsonValue};
use ait_core::runtime_binding_state::DEFAULT_RUNTIME_BINDING_STATE_VERSION;

use super::*;

fn store(name: &str) -> AgentRuntimeBindingStore {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    AgentRuntimeBindingStore::new(PathBuf::from(format!(
        "/tmp/ait-binding-{name}-{}-{nonce}.json",
        std::process::id()
    )))
}

#[test]
fn binding_store_persists_conversation_and_codex_thread_only() {
    let store = store("sessionless");
    let binding = store
        .execute(
            "upsert_binding",
            &json!({
                "transport": "telegram",
                "surface_id": "42",
                "repo_name": "demo",
                "session_id": "retired",
                "updates": {
                    "conversation_key": "telegram:42",
                    "codex_thread_binding": {"thread_id": "T-1"},
                    "canonical_session_id": "retired"
                }
            }),
        )
        .expect("upsert");
    assert_eq!(binding["conversation_key"], "telegram:42");
    assert_eq!(binding["codex_thread_binding"]["thread_id"], "T-1");
    assert!(!binding.to_string().contains("retired"));
    assert!(!store
        .load()
        .expect("load")
        .to_string()
        .contains("session_id"));
}

#[test]
fn binding_store_persists_legacy_spool_cleanup_without_resetting_notification_state() {
    let store = store("legacy-spool-cleanup");
    let saved = store
        .execute(
            "save",
            &json!({
                "state": {
                    "version": 3,
                    "last_update_id": 77,
                    "surface_bindings": {
                        "telegram:42": {
                            "binding_id": "telegram:42",
                            "transport": "telegram",
                            "surface_id": "42",
                            "conversation_key": "telegram:42",
                            "last_queue_summary_digest": "current-digest",
                            "last_queue_notification_at": "2026-07-20T01:38:09Z",
                            "workflow_notifications_enabled": true,
                            "telegram_reply_spool": [
                                {
                                    "spool_key": "current",
                                    "conversation_key": "telegram:42",
                                    "status": "queued"
                                },
                                {
                                    "spool_key": "legacy",
                                    "status": "attempting",
                                    "session_id": "S-legacy"
                                }
                            ]
                        }
                    },
                    "telegram_bootstrap_auth": {}
                }
            }),
        )
        .expect("save");
    let binding = &saved["surface_bindings"]["telegram:42"];

    assert_eq!(saved["version"], DEFAULT_RUNTIME_BINDING_STATE_VERSION);
    assert_eq!(saved["last_update_id"], 77);
    assert_eq!(binding["last_queue_summary_digest"], "current-digest");
    assert_eq!(
        binding["last_queue_notification_at"],
        "2026-07-20T01:38:09Z"
    );
    assert_eq!(binding["workflow_notifications_enabled"], true);
    assert_eq!(binding["telegram_reply_spool"].as_array().unwrap().len(), 1);
    assert_eq!(binding["telegram_reply_spool"][0]["spool_key"], "current");
    assert!(!store.load().unwrap().to_string().contains("S-legacy"));
}

#[test]
fn removed_session_index_operations_fail_closed() {
    let store = store("removed-ops");
    for operation in [
        "linkage_by_session",
        "bindings_for_session",
        "linked_session_ids",
        "resolve_repo_shared_binding",
        "merge_telegram_state",
        "recover_repo_local_state",
    ] {
        let error = store.execute(operation, &json!({})).expect_err(operation);
        assert!(error.contains("unsupported"));
    }
}

#[test]
fn recent_delivery_values_are_bounded_per_binding() {
    let store = store("recent");
    store
        .execute(
            "upsert_binding",
            &json!({"transport": "line", "surface_id": "room-1"}),
        )
        .expect("upsert");
    for value in ["e1", "e2", "e3"] {
        store
            .execute(
                "remember_recent_value",
                &json!({
                    "transport": "line",
                    "surface_id": "room-1",
                    "recent_key": "recent_event_ids",
                    "last_value_key": "last_event_id",
                    "value": value,
                    "limit": 2
                }),
            )
            .expect("remember");
    }
    let binding = store
        .execute(
            "get_binding",
            &json!({"transport": "line", "surface_id": "room-1"}),
        )
        .expect("get");
    assert_eq!(binding["recent_event_ids"], json!(["e2", "e3"]));
    assert_eq!(
        store
            .execute(
                "has_recent_value",
                &json!({
                    "transport": "line",
                    "surface_id": "room-1",
                    "recent_key": "recent_event_ids",
                    "value": "e3"
                }),
            )
            .expect("has"),
        JsonValue::Bool(true)
    );
}

#[test]
fn projection_exposes_binding_identity_without_session_aliases() {
    let projected = agent_runtime_binding_projection_json(&json!({
        "binding_id": "discord:7",
        "transport": "discord",
        "surface_id": "7",
        "surface_title": "general",
        "conversation_key": "discord:7",
        "codex_thread_binding": {"thread_id": "T-7"}
    }))
    .expect("projection");
    assert_eq!(projected["conversation_key"], "discord:7");
    assert_eq!(projected["provider_thread"]["thread_id"], "T-7");
    assert!(!projected.to_string().contains("session"));
}
