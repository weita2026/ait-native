use super::*;

#[test]
fn default_state_contains_bindings_without_a_session_domain() {
    let state = default_runtime_binding_state_payload_json();
    assert_eq!(state["version"], DEFAULT_RUNTIME_BINDING_STATE_VERSION);
    assert!(state["surface_bindings"].is_object());
    assert!(state.get("chats").is_none());
    assert!(!state.to_string().contains("session_id"));
}

#[test]
fn normalization_drops_retired_session_compatibility_fields() {
    let normalized = normalize_runtime_binding_state_document_json(&json!({
        "version": 2,
        "last_update_id": 9,
        "chats": {"42": {"session_id": "S-legacy"}},
        "surface_bindings": {
            "telegram:42": {
                "binding_id": "telegram:42",
                "transport": "telegram",
                "surface_id": "42",
                "conversation_key": "telegram:42",
                "session_id": "S-legacy",
                "canonical_session_id": "S-legacy",
                "codex_thread_binding": {"thread_id": "T-1"}
            }
        }
    }));
    assert_eq!(normalized["ir_version"], RUNTIME_BINDING_STATE_IR_VERSION);
    assert_eq!(
        normalized["state"]["version"],
        DEFAULT_RUNTIME_BINDING_STATE_VERSION
    );
    assert_eq!(
        normalized["state"]["surface_bindings"]["telegram:42"]["conversation_key"],
        "telegram:42"
    );
    assert!(normalized["state"].get("chats").is_none());
    assert!(!normalized.to_string().contains("S-legacy"));
}

#[test]
fn normalization_prunes_only_legacy_or_foreign_telegram_reply_spool_entries() {
    let normalized = normalize_runtime_binding_state_document_json(&json!({
        "version": 3,
        "last_update_id": 19,
        "surface_bindings": {
            "telegram:42": {
                "binding_id": "telegram:42",
                "transport": "telegram",
                "surface_id": "42",
                "conversation_key": " telegram:42 ",
                "last_queue_summary_digest": "keep-digest",
                "last_queue_notification_at": "2026-07-20T01:38:09Z",
                "last_synced_sequence": 65,
                "extension_state": {"key": {"value": "preserved"}},
                "telegram_reply_spool": [
                    {
                        "spool_key": "keep",
                        "conversation_key": "telegram:42",
                        "status": "queued",
                        "provider_thread": {"session_id": "provider-owned"}
                    },
                    {
                        "spool_key": "legacy-session",
                        "conversation_key": "telegram:42",
                        "session_id": "S-legacy"
                    },
                    {"spool_key": "missing-conversation", "status": "queued"},
                    {"spool_key": "foreign", "conversation_key": "telegram:other"},
                    "malformed"
                ]
            },
            "telegram:missing-conversation": {
                "binding_id": "telegram:missing-conversation",
                "telegram_reply_spool": [
                    {"spool_key": "orphan", "conversation_key": "telegram:missing"}
                ]
            }
        },
        "telegram_bootstrap_auth": {"owner": "configured"}
    }));
    let state = &normalized["state"];
    let binding = &state["surface_bindings"]["telegram:42"];

    assert_eq!(state["version"], DEFAULT_RUNTIME_BINDING_STATE_VERSION);
    assert_eq!(state["last_update_id"], 19);
    assert_eq!(state["telegram_bootstrap_auth"]["owner"], "configured");
    assert_eq!(binding["last_queue_summary_digest"], "keep-digest");
    assert_eq!(
        binding["last_queue_notification_at"],
        "2026-07-20T01:38:09Z"
    );
    assert_eq!(binding["last_synced_sequence"], 65);
    assert_eq!(binding["extension_state"]["key"]["value"], "preserved");
    assert_eq!(binding["telegram_reply_spool"].as_array().unwrap().len(), 1);
    assert_eq!(binding["telegram_reply_spool"][0]["spool_key"], "keep");
    assert_eq!(
        binding["telegram_reply_spool"][0]["provider_thread"]["session_id"],
        "provider-owned"
    );
    assert!(state["surface_bindings"]["telegram:missing-conversation"]
        .get("telegram_reply_spool")
        .is_none());
    assert!(!normalized.to_string().contains("S-legacy"));
}

#[test]
fn malformed_or_empty_reply_spool_fields_are_removed() {
    for spool in [json!(null), json!({}), json!([])] {
        let normalized = normalize_runtime_binding_state_document_json(&json!({
            "surface_bindings": {
                "telegram:42": {
                    "conversation_key": "telegram:42",
                    "telegram_reply_spool": spool
                }
            }
        }));
        assert!(normalized["state"]["surface_bindings"]["telegram:42"]
            .get("telegram_reply_spool")
            .is_none());
    }
}

#[test]
fn schema_declares_only_sessionless_state_authorities() {
    let schema = runtime_binding_state_schema_json();
    let required = schema["properties"]["state"]["required"]
        .as_array()
        .expect("required fields");
    assert!(!required.iter().any(|value| value == "chats"));
    assert!(!schema.to_string().contains("session_id"));
}
