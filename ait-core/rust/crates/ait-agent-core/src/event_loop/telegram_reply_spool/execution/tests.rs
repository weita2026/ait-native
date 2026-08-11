use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

use ait_core::json_support::{json, JsonValue};
use tempfile::TempDir;

use super::{
    execute_with_telegram_reply_spool_ports, load_telegram_reply_spool_entries,
    RuntimeBindingTelegramReplySpoolStatePort, TelegramReplySpoolClockPort,
    TelegramReplySpoolExecutionErrorKind, TelegramReplySpoolMutation, TelegramReplySpoolStatePort,
    CONTRACT, MIGRATION_STAGE,
};
use crate::event_loop::telegram_reply_spool::{
    DefaultTelegramReplySpoolPlanner, TelegramReplySpoolPlanner,
};
use crate::runtime::AgentRuntimeBindingStore;

#[derive(Debug, Clone, Copy)]
struct FixedClock;

impl TelegramReplySpoolClockPort for FixedClock {
    fn now_iso(&self) -> Result<String, String> {
        Ok("2026-07-18T00:00:00Z".to_string())
    }
}

fn fixture() -> (
    TempDir,
    AgentRuntimeBindingStore,
    RuntimeBindingTelegramReplySpoolStatePort,
) {
    let temp = TempDir::new().unwrap();
    let store = AgentRuntimeBindingStore::new(temp.path().join("bindings.json"));
    store
        .execute(
            "upsert_binding",
            &json!({
                "transport": "telegram",
                "surface_id": "42",
                "repo_name": "demo",
                "updates": {"conversation_key": "telegram:42"},
            }),
        )
        .unwrap();
    let state = RuntimeBindingTelegramReplySpoolStatePort::from_store(store.clone());
    (temp, store, state)
}

fn pending(message_id: i64, text: &str) -> JsonValue {
    json!({
        "conversation_key": "telegram:42",
        "chat_id": 42,
        "chat_type": "private",
        "chat_title": "Demo chat",
        "actor_identity": "telegram:user:7",
        "text": text,
        "telegram_message_id": message_id,
        "telegram_message_ids": [message_id],
        "transport_envelope": {
            "event_id": format!("telegram:42:{message_id}"),
            "secret": "envelope-secret",
        },
        "watch_spec": {
            "secret": "watch-secret",
        },
    })
}

fn execute(
    state: &RuntimeBindingTelegramReplySpoolStatePort,
    request: JsonValue,
) -> Result<JsonValue, super::TelegramReplySpoolExecutionError> {
    execute_with_telegram_reply_spool_ports(
        &DefaultTelegramReplySpoolPlanner,
        state,
        &FixedClock,
        &request,
    )
}

#[test]
fn remember_attempt_failure_reload_and_clear_are_persistent() {
    let (_temp, store, state) = fixture();
    let pending = pending(11, "private turn text");

    let queued = execute(
        &state,
        json!({
            "stage": "remember",
            "pending_turn": pending,
            "status": "queued",
            "spool_limit": 100,
        }),
    )
    .unwrap();
    assert_eq!(queued["contract"], CONTRACT);
    assert_eq!(queued["migration_stage"], MIGRATION_STAGE);
    assert_eq!(queued["applied"], true);
    assert_eq!(queued["entry_count"], 1);
    assert_eq!(queued["python_reply_spool_allowed"], false);

    execute(
        &state,
        json!({
            "stage": "remember",
            "pending_turn": pending,
            "status": "attempting",
            "attempt_increment": true,
        }),
    )
    .unwrap();
    execute(
        &state,
        json!({
            "stage": "remember",
            "pending_turn": pending,
            "status": "failed",
            "last_error": "private-backend-error",
            "user_event": {"sequence": 21},
        }),
    )
    .unwrap();

    let restarted = RuntimeBindingTelegramReplySpoolStatePort::from_store(store.clone());
    let entries = load_telegram_reply_spool_entries(
        &DefaultTelegramReplySpoolPlanner,
        &restarted,
        &json!(42),
    )
    .unwrap();
    assert_eq!(entries.len(), 1);
    let entry = entries.iter().next().unwrap();
    assert_eq!(entry["status"], "failed");
    assert_eq!(entry["attempt_count"], 1);
    assert_eq!(entry["last_attempt_at"], "2026-07-18T00:00:00Z");
    assert_eq!(entry["last_error"], "private-backend-error");
    assert_eq!(entry["last_user_sequence"], 21);

    let cleared = execute(
        &restarted,
        json!({
            "stage": "clear",
            "pending_turn": pending,
        }),
    )
    .unwrap();
    assert_eq!(cleared["applied"], true);
    assert_eq!(cleared["entry_count"], 0);
    assert!(load_telegram_reply_spool_entries(
        &DefaultTelegramReplySpoolPlanner,
        &restarted,
        &json!(42),
    )
    .unwrap()
    .is_empty());
}

#[test]
fn missing_link_and_conversation_mismatch_are_safe_noops() {
    let (_temp, store, state) = fixture();
    let missing = execute_with_telegram_reply_spool_ports(
        &DefaultTelegramReplySpoolPlanner,
        &RuntimeBindingTelegramReplySpoolStatePort::new(
            store.path().with_file_name("missing.json"),
        ),
        &FixedClock,
        &json!({
            "stage": "remember",
            "pending_turn": pending(12, "missing"),
            "status": "queued",
        }),
    )
    .unwrap();
    assert_eq!(missing["applied"], false);
    assert_eq!(missing["reason"], "missing_link");

    let mut mismatch = pending(13, "mismatch");
    mismatch["conversation_key"] = json!("telegram:other");
    let mismatch = execute(
        &state,
        json!({
            "stage": "remember",
            "pending_turn": mismatch,
            "status": "queued",
        }),
    )
    .unwrap();
    assert_eq!(mismatch["applied"], false);
    assert_eq!(mismatch["reason"], "conversation_mismatch");
}

#[test]
fn retention_keeps_only_newest_entries() {
    let (_temp, _store, state) = fixture();
    for message_id in [21, 22, 23] {
        execute(
            &state,
            json!({
                "stage": "remember",
                "pending_turn": pending(message_id, &format!("turn-{message_id}")),
                "status": "queued",
                "spool_limit": 2,
            }),
        )
        .unwrap();
    }
    let entries =
        load_telegram_reply_spool_entries(&DefaultTelegramReplySpoolPlanner, &state, &json!(42))
            .unwrap()
            .into_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["telegram_message_id"], 22);
    assert_eq!(entries[1]["telegram_message_id"], 23);
}

#[test]
fn concurrent_remember_mutations_do_not_lose_an_entry() {
    let (_temp, _store, state) = fixture();
    let state = Arc::new(state);
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for message_id in [31, 32] {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            execute(
                &state,
                json!({
                    "stage": "remember",
                    "pending_turn": pending(message_id, &format!("turn-{message_id}")),
                    "status": "queued",
                }),
            )
            .unwrap();
        }));
    }
    barrier.wait();
    for handle in handles {
        handle.join().unwrap();
    }
    let entries = load_telegram_reply_spool_entries(
        &DefaultTelegramReplySpoolPlanner,
        state.as_ref(),
        &json!(42),
    )
    .unwrap();
    assert_eq!(entries.len(), 2);
}

#[test]
fn mutation_recovers_interrupted_binding_write_artifacts() {
    let (_temp, store, state) = fixture();
    let file_name = store.path().file_name().unwrap().to_string_lossy();
    let interrupted = store
        .path()
        .with_file_name(format!("{file_name}.tmp-interrupted"));
    fs::write(&interrupted, b"private interrupted content").unwrap();
    execute(
        &state,
        json!({
            "stage": "remember",
            "pending_turn": pending(41, "recover"),
            "status": "queued",
        }),
    )
    .unwrap();
    assert!(!interrupted.exists());
    assert_eq!(store.load().unwrap()["version"], 4);
}

#[derive(Debug, Clone, Copy)]
struct BadPlanner {
    mode: &'static str,
}

impl TelegramReplySpoolPlanner for BadPlanner {
    fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        match self.mode {
            "error" => Err("private-planner-secret".to_string()),
            "identity" => Ok(json!({
                "stage": "remember",
                "execution_kind": "wrong",
                "patch_required": false,
                "patch_payload": null,
                "result": {
                    "execution_kind": "wrong",
                    "patch_required": false,
                    "reason": "missing_current_link",
                    "patch_payload": null,
                },
            })),
            "flag" => Ok(json!({
                "stage": "remember",
                "execution_kind": "telegram_reply_spool",
                "patch_required": "yes",
                "result": {},
            })),
            "entries" => Ok(json!({
                "stage": "remember",
                "execution_kind": "telegram_reply_spool",
                "patch_required": true,
                "entries": "not-an-array",
                "patch_payload": {"telegram_reply_spool": "not-an-array"},
                "result": {
                    "execution_kind": "telegram_reply_spool",
                    "patch_required": true,
                },
            })),
            _ => unreachable!(),
        }
    }
}

#[test]
fn corrupt_planner_results_fail_closed_before_persistence() {
    let (_temp, _store, state) = fixture();
    for mode in ["error", "identity", "flag", "entries"] {
        let error = execute_with_telegram_reply_spool_ports(
            &BadPlanner { mode },
            &state,
            &FixedClock,
            &json!({
                "stage": "remember",
                "pending_turn": pending(51, "planner secret"),
                "status": "queued",
            }),
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            if mode == "error" {
                TelegramReplySpoolExecutionErrorKind::Planner
            } else {
                TelegramReplySpoolExecutionErrorKind::PlannerContract
            }
        );
    }
    assert!(load_telegram_reply_spool_entries(
        &DefaultTelegramReplySpoolPlanner,
        &state,
        &json!(42),
    )
    .unwrap()
    .is_empty());
}

#[derive(Debug)]
struct FailingState;

impl TelegramReplySpoolStatePort for FailingState {
    fn load_link(&self, _chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        Err("private-state-secret".to_string())
    }

    fn mutate_link(
        &self,
        _chat_id: &JsonValue,
        _mutation: &mut TelegramReplySpoolMutation<'_>,
    ) -> Result<Option<JsonValue>, String> {
        Err("private-state-secret".to_string())
    }
}

#[test]
fn public_outcomes_errors_and_entry_debug_are_secret_safe() {
    let (_temp, _store, state) = fixture();
    let secret_values = [
        "pending-text-secret",
        "telegram:user:7",
        "telegram:42",
        "telegram:42:61",
        "envelope-secret",
        "watch-secret",
        "private-planner-secret",
        "private-state-secret",
    ];
    let outcome = execute(
        &state,
        json!({
            "stage": "remember",
            "pending_turn": pending(61, "pending-text-secret"),
            "status": "queued",
        }),
    )
    .unwrap();
    let outcome_debug = format!("{outcome:?}");
    let entries =
        load_telegram_reply_spool_entries(&DefaultTelegramReplySpoolPlanner, &state, &json!(42))
            .unwrap();
    let entries_debug = format!("{entries:?}");
    let planner_error = execute_with_telegram_reply_spool_ports(
        &BadPlanner { mode: "error" },
        &state,
        &FixedClock,
        &json!({
            "stage": "remember",
            "pending_turn": pending(62, "pending-text-secret"),
            "status": "queued",
        }),
    )
    .unwrap_err();
    let state_error = execute_with_telegram_reply_spool_ports(
        &DefaultTelegramReplySpoolPlanner,
        &FailingState,
        &FixedClock,
        &json!({
            "stage": "remember",
            "pending_turn": pending(63, "pending-text-secret"),
            "status": "queued",
        }),
    )
    .unwrap_err();
    let rendered = format!(
        "{outcome_debug} {entries_debug} {planner_error:?} {planner_error} {state_error:?} {state_error}"
    );
    for secret in secret_values {
        assert!(!rendered.contains(secret), "leaked {secret}: {rendered}");
    }
}

#[test]
fn malformed_requests_and_clock_failures_are_classified() {
    let (_temp, _store, state) = fixture();
    for request in [
        json!([]),
        json!({"stage": "unknown"}),
        json!({"stage": "entries", "chat_id": {}}),
        json!({"stage": "remember", "pending_turn": {}, "status": "queued"}),
        json!({
            "stage": "remember",
            "pending_turn": pending(71, "bad status"),
            "status": "unknown",
        }),
    ] {
        assert_eq!(
            execute_with_telegram_reply_spool_ports(
                &DefaultTelegramReplySpoolPlanner,
                &state,
                &FixedClock,
                &request,
            )
            .unwrap_err()
            .kind(),
            TelegramReplySpoolExecutionErrorKind::InvalidRequest,
        );
    }

    struct FailingClock;
    impl TelegramReplySpoolClockPort for FailingClock {
        fn now_iso(&self) -> Result<String, String> {
            Err("private-clock-secret".to_string())
        }
    }
    let clock_error = execute_with_telegram_reply_spool_ports(
        &DefaultTelegramReplySpoolPlanner,
        &state,
        &FailingClock,
        &json!({
            "stage": "remember",
            "pending_turn": pending(72, "clock"),
            "status": "queued",
        }),
    )
    .unwrap_err();
    assert_eq!(
        clock_error.kind(),
        TelegramReplySpoolExecutionErrorKind::Clock
    );
    assert!(!format!("{clock_error:?} {clock_error}").contains("private-clock-secret"));
}
