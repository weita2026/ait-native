use super::{
    agent_telegram_workflow_notification_format_json,
    format_with_telegram_workflow_notification_formatter,
    DefaultTelegramWorkflowNotificationFormatter, TelegramWorkflowNotificationFormatter,
};
use ait_core::json_support::{json, JsonValue};

fn format(request: ait_core::json_support::JsonValue) -> ait_core::json_support::JsonValue {
    agent_telegram_workflow_notification_format_json(&request).unwrap()
}

#[test]
fn formats_workflow_notification_with_gate_sections() {
    let payload = json!({
        "config": {"repo_name": "ait"},
        "kind": "workflow_notification",
        "payload": {
            "summary": {"active": 3, "attention_required": 1, "ready_to_land": 1, "ready_to_complete": 1},
            "items": [
                {
                    "task": {"task_id": "AITT-1000", "title": "Blocked task"},
                    "workflow": {"state": "attention_required", "reason": "Policy is still pending."},
                    "primary_gate": "policy",
                    "ci_summary": {
                        "patchset_id": "AITP-1000",
                        "tg1_required": {"status": "pass", "live_count": 24, "minimum_count": 24},
                        "remote_land_gate": "pending"
                    },
                    "next_action": {"code": "inspect_change", "label": "Inspect change", "detail": "Open the focus change and fix policy."}
                },
                {
                    "task": {"task_id": "AITT-1001", "title": "Landable task"},
                    "workflow": {"state": "ready_to_land", "reason": "1 linked change can land now."},
                    "next_action": {"code": "land_change", "label": "Land change", "detail": "Submit land for the selected patchset."}
                },
                {
                    "task": {"task_id": "AITT-1002", "title": "Completable task"},
                    "workflow": {"state": "ready_to_complete", "reason": "All linked changes are landed."},
                    "next_action": {"code": "complete_task", "label": "Complete task", "detail": "Close the task after verifying target line state."}
                }
            ]
        }
    });

    let formatted = agent_telegram_workflow_notification_format_json(&payload).unwrap();
    let text = formatted["text"].as_str().unwrap();
    assert!(text.contains("\n\nPolicy\n"));
    assert!(text.contains("\n\nReady to land\n"));
    assert!(text.contains("\n\nReady to complete\n"));
    assert!(text.contains("TG-1=pass 24/24"));
    assert!(text.contains("land=pending"));
    assert!(!text.contains("Need attention"));
}

#[test]
fn formats_proactive_task_age_and_action_without_not_required_ci_noise() {
    let formatted = format(json!({
        "kind": "workflow_notification",
        "config": {"repo_name": "ait"},
        "payload": {
            "items": [
                {
                    "task": {"task_id": "RT-2706", "title": "Remove legacy session route"},
                    "updated_at": "2026-07-10 08:29:24.864995+08",
                    "workflow": {
                        "state": "attention_required",
                        "reason": "Policy evaluation is still pending."
                    },
                    "primary_gate": "policy",
                    "ci_summary": {
                        "patchset_id": "RP-2546-1",
                        "tests_status": "not_required",
                        "remote_land_gate": "pending"
                    },
                    "next_action": {
                        "code": "satisfy_policy",
                        "label": "Open task"
                    }
                },
                {
                    "task": {"task_id": "RT-2707", "title": "Repair failing checks"},
                    "updated_at": "2026-07-19T11:20:00Z",
                    "workflow": {
                        "state": "attention_required",
                        "reason": "Required checks are failing."
                    },
                    "primary_gate": "ci",
                    "ci_summary": {
                        "patchset_id": "RP-2547-1",
                        "tests_status": "failed",
                        "remote_land_gate": "blocked"
                    },
                    "next_action": {"code": "repair_ci", "label": "Open task"}
                },
                {
                    "task": {"task_id": "RT-2439", "title": "Complete landed work"},
                    "updated_at": "not-a-date",
                    "workflow": {
                        "state": "ready_to_complete",
                        "reason": "All linked changes are landed."
                    },
                    "next_action": {"code": "complete_task", "label": "Open task and complete it"}
                }
            ]
        }
    }));

    let text = formatted["text"].as_str().unwrap();
    assert!(text.contains("updated=2026-07-10 · next=satisfy_policy"));
    assert!(text.contains("updated=2026-07-19 · next=repair_ci"));
    assert!(text.contains("patchset=RP-2546-1 · land=pending"));
    assert!(text.contains("patchset=RP-2547-1 · CI=failed · land=blocked"));
    assert!(text.contains("RT-2439 · Complete landed work\n  next=complete_task"));
    assert!(!text.contains("CI=not_required"));
    assert!(!text.contains("updated=not-a-date"));
    assert!(!text.contains("Policy evaluation is still pending."));
    assert!(!text.contains("All linked changes are landed."));
}

#[test]
fn formats_queue_digest_and_actionable_helpers() {
    let digest = format(json!({
        "kind": "queue_digest",
        "payload": {
            "items": [
                {
                    "task": {"task_id": "AITT-1", "title": "Policy wait"},
                    "workflow": {"state": "attention_required", "reason": "Policy pending"},
                    "primary_gate": "policy"
                }
            ]
        }
    }));
    assert_eq!(digest["kind"], "queue_digest");
    assert_eq!(digest["actionable"], true);
    let raw = digest["digest"].as_str().unwrap();
    assert!(raw.contains("Policy"));
    assert!(raw.contains("AITT-1"));

    let actionable = format(json!({
        "kind": "queue_digest_actionable",
        "raw": raw
    }));
    assert_eq!(actionable["actionable"], true);

    let empty = format(json!({
        "kind": "queue_digest_actionable",
        "raw": "{\"actionable\": false, \"lines\": []}"
    }));
    assert_eq!(empty["actionable"], false);
}

#[test]
fn queue_digest_tracks_visible_action_and_date_without_clock_churn() {
    let request = |updated_at: &str, action: &str| {
        json!({
            "kind": "queue_digest",
            "payload": {
                "items": [{
                    "task": {"task_id": "AITT-1", "title": "Policy wait"},
                    "updated_at": updated_at,
                    "workflow": {"state": "attention_required", "reason": "Policy pending"},
                    "primary_gate": "policy",
                    "next_action": {"code": action, "label": "Open task"}
                }]
            }
        })
    };

    let initial = format(request("2026-07-19T08:00:00Z", "satisfy_policy"));
    let same_day = format(request("2026-07-19T20:00:00Z", "satisfy_policy"));
    let next_day = format(request("2026-07-20T08:00:00Z", "satisfy_policy"));
    let new_action = format(request("2026-07-19T08:00:00Z", "record_attestation"));

    assert_eq!(initial["digest"], same_day["digest"]);
    assert_ne!(initial["digest"], next_day["digest"]);
    assert_ne!(initial["digest"], new_action["digest"]);
}

#[test]
fn formats_queue_attention_and_ready_summaries() {
    let payload = json!({
        "summary": {
            "active": 4,
            "attention_required": 1,
            "ready_to_land": 1,
            "ready_to_complete": 1
        },
        "items": [
            {
                "task": {"task_id": "AITT-1", "title": "Policy wait"},
                "workflow": {"state": "attention_required", "reason": "Policy pending"},
                "primary_gate": "policy",
                "next_action": {"label": "Inspect"}
            },
            {
                "task": {"task_id": "AITT-2", "title": "Land me"},
                "workflow": {"state": "ready_to_land", "reason": "Fresh"},
                "next_action": {"label": "Land"}
            },
            {
                "task": {"task_id": "AITT-3", "title": "Complete me"},
                "workflow": {"state": "ready_to_complete", "reason": "Done"},
                "next_action": {"label": "Complete"}
            },
            {
                "task": {"task_id": "AITT-4", "title": "Draft"},
                "workflow": {"state": "in_progress", "reason": "Working"},
                "next_action": {"label": "Publish"}
            }
        ]
    });

    let queue = format(json!({
        "kind": "queue_summary",
        "config": {"repo_name": "ait"},
        "payload": payload
    }));
    let queue_text = queue["text"].as_str().unwrap();
    assert!(queue_text.contains("ait queue · repo=ait"));
    assert!(queue_text.contains("active=4 attention=1 ready_to_land=1 ready_to_complete=1"));
    assert!(queue_text.contains("Other active tasks"));

    let attention = format(json!({
        "kind": "attention_summary",
        "config": {"repo_name": "ait"},
        "payload": payload
    }));
    let attention_text = attention["text"].as_str().unwrap();
    assert!(attention_text.contains("ait attention · repo=ait"));
    assert!(attention_text.contains("Policy"));
    assert!(!attention_text.contains("Ready to land"));

    let ready = format(json!({
        "kind": "ready_summary",
        "config": {"repo_name": "ait"},
        "payload": payload
    }));
    let ready_text = ready["text"].as_str().unwrap();
    assert!(ready_text.contains("ait ready · repo=ait"));
    assert!(ready_text.contains("ready_to_land=1 ready_to_complete=1"));
    assert!(ready_text.contains("Ready to complete"));
}

#[test]
fn formats_task_change_audit_and_land_summaries() {
    let detail = json!({
        "task": {
            "task_id": "AITT-1",
            "title": "Trait split",
            "status": "active",
            "intent": "Decouple formatter"
        },
        "change": {
            "change_id": "AITC-1",
            "title": "Formatter ports",
            "status": "review"
        },
        "workflow": {
            "state": "ready_to_land",
            "reason": "Ready"
        },
        "summary": {
            "verdict": "ready",
            "open_change_count": 1,
            "landed_change_count": 0,
            "effective_on_target_change_count": 1
        },
        "target": {"line_name": "main"},
        "recommended_action": {"code": "land_change", "label": "Land change", "detail": "Land it"},
        "changes": [
            {"change": {"change_id": "AITC-1", "status": "review"}, "target_state": "ready"}
        ],
        "current_patchset": {"patchset_id": "AITP-1"},
        "policy_summary": {"decision": "pass"},
        "review_summary": {"approvals": 1, "blocking": 0, "comments": 2},
        "freshness": {"base_is_fresh": true}
    });

    let task = format(json!({
        "kind": "task_summary",
        "config": {"ait_web_url": "https://ait.example"},
        "payload": detail
    }));
    let task_text = task["text"].as_str().unwrap();
    assert!(task_text.contains("AITT-1 · Trait split"));
    assert!(task_text.contains("https://ait.example/tasks/AITT-1"));

    let change = format(json!({
        "kind": "change_summary",
        "config": {"ait_web_url": "https://ait.example"},
        "payload": detail
    }));
    let change_text = change["text"].as_str().unwrap();
    assert!(change_text.contains("AITC-1 · Formatter ports"));
    assert!(change_text.contains("policy=pass"));
    assert!(change_text.contains("https://ait.example/changes/AITC-1"));

    let audit = format(json!({
        "kind": "task_audit_summary",
        "config": {"ait_web_url": "https://ait.example"},
        "payload": detail
    }));
    let audit_text = audit["text"].as_str().unwrap();
    assert!(audit_text.contains("workflow=ready_to_land verdict=ready target=main"));
    assert!(audit_text.contains("recommended=Land change"));
    assert!(audit_text.contains("Linked changes"));

    let land = format(json!({
        "kind": "change_land_summary",
        "config": {"ait_web_url": "https://ait.example"},
        "payload": detail
    }));
    let land_text = land["text"].as_str().unwrap();
    assert!(land_text.contains("land_state=ready_to_land status=review task=AITT-1"));
    assert!(land_text.contains("patchset=AITP-1 policy=pass base_fresh=true"));
    assert!(land_text.contains("Selected patchset looks landable on the current base."));
}

#[test]
fn workflow_notification_default_formatter_satisfies_trait_entrypoint() {
    let formatter: &dyn TelegramWorkflowNotificationFormatter =
        &DefaultTelegramWorkflowNotificationFormatter;
    let formatted = formatter
        .format_json(&json!({
            "kind": "workflow_notification",
            "config": {"repo_name": "ait"},
            "payload": {"items": []}
        }))
        .unwrap();
    assert_eq!(formatted["kind"], "workflow_notification");
    assert!(formatted["text"]
        .as_str()
        .unwrap()
        .contains("workflow (ait)"));
    assert!(formatted["text"].as_str().unwrap().contains("Complete"));
}

#[test]
fn workflow_notification_bound_entrypoint_accepts_substitute_formatter() {
    struct SubstituteFormatter;

    impl TelegramWorkflowNotificationFormatter for SubstituteFormatter {
        fn format_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "kind": "substitute",
                "seen_kind": request.get("kind").cloned().unwrap_or(JsonValue::Null)
            }))
        }
    }

    let formatted = format_with_telegram_workflow_notification_formatter(
        &SubstituteFormatter,
        &json!({"kind": "custom"}),
    )
    .unwrap();
    assert_eq!(formatted["kind"], "substitute");
    assert_eq!(formatted["seen_kind"], "custom");
}

#[test]
fn rejects_invalid_request_shapes_and_unknown_kinds() {
    let non_object = agent_telegram_workflow_notification_format_json(&json!(null)).unwrap_err();
    assert_eq!(non_object, "request must be a JSON object");

    let unsupported = agent_telegram_workflow_notification_format_json(&json!({
        "kind": "not_supported"
    }))
    .unwrap_err();
    assert_eq!(
        unsupported,
        "unsupported Telegram workflow notification format kind `not_supported`"
    );
}
