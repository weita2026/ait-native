use super::*;
use crate::json_support::json;

#[test]
fn planning_session_join_payload_normalizes_required_fields() {
    let payload = validate_planning_session_join_payload_json(
        &json!({
            "planning_session": {
                "planning_session_id": " PS-1 ",
                "plan_id": " PL-1 ",
                "status": " active ",
                "mode": " connected_local "
            },
            "attachment": {
                "planning_session_id": " PS-1 ",
                "plan_id": " PL-1 ",
                "surface": " cli "
            }
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(payload["planning_session"]["planning_session_id"], "PS-1");
    assert_eq!(payload["planning_session"]["plan_id"], "PL-1");
    assert_eq!(payload["planning_session"]["status"], "active");
    assert_eq!(payload["planning_session"]["mode"], "connected_local");
    assert_eq!(payload["attachment"]["surface"], "cli");
}

#[test]
fn planning_session_join_payload_projects_known_attachment_fields() {
    let payload = validate_planning_session_join_payload_json(
        &json!({
            "ignored_top_level": "drop",
            "planning_session": {
                "planning_session_id": "PS-1",
                "plan_id": "PL-1",
                "status": "active"
            },
            "attachment": {
                "planning_session_id": "PS-1",
                "plan_id": "PL-1",
                "surface": "cli",
                "preferred_agent": " codex ",
                "title": " Join ",
                "model_name": " gpt-5-codex ",
                "ignored_attachment": "drop"
            }
        })
        .to_string(),
    )
    .unwrap();

    assert!(payload.get("ignored_top_level").is_none());
    assert!(payload["attachment"].get("ignored_attachment").is_none());
    assert_eq!(payload["attachment"]["preferred_agent"], "codex");
    assert_eq!(payload["attachment"]["title"], "Join");
    assert_eq!(payload["attachment"]["model_name"], "gpt-5-codex");
}
