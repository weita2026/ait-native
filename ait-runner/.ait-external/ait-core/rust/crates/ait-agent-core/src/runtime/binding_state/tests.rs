use ait_core::json_support::json;

use super::*;

#[test]
fn agent_binding_state_forwards_the_sessionless_core_contract() {
    assert_eq!(
        agent_runtime_binding_state_ir_version(),
        "ait.runtime_binding_state.v2"
    );
    let normalized = agent_normalize_runtime_binding_state_document_json(&json!({
        "surface_bindings": {
            "telegram:1": {
                "conversation_key": "telegram:1",
                "session_id": "retired"
            }
        }
    }));
    assert_eq!(
        normalized["state"]["surface_bindings"]["telegram:1"]["conversation_key"],
        "telegram:1"
    );
    assert!(!normalized.to_string().contains("retired"));
}
