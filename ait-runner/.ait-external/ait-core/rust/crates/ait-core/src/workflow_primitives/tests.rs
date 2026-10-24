use super::{
    generate_namespaced_sequence_id, normalize_id_namespace_prefix,
    publication_state_has_unpublished_head, publication_state_value, task_status_details,
    task_status_value, workflow_error_envelope, workflow_id_matches, workflow_id_token,
    workflow_mode_value, workflow_origin_namespace_prefix, workflow_success_envelope,
    CheckboxState, DEFAULT_ID_NAMESPACE_PREFIX,
};

#[test]
fn markdown_checkbox_states_normalize_to_shared_values() {
    assert_eq!(
        CheckboxState::from_markdown_checked(Some(" ")).as_str(),
        "open"
    );
    assert_eq!(
        CheckboxState::from_markdown_checked(Some("x")).as_str(),
        "done"
    );
    assert_eq!(
        CheckboxState::from_markdown_checked(Some("X")).as_str(),
        "done"
    );
    assert_eq!(CheckboxState::from_markdown_checked(None).as_str(), "none");
}

#[test]
fn normalized_checkbox_states_parse_from_contract_values() {
    assert_eq!(
        CheckboxState::from_normalized_state("open")
            .unwrap()
            .as_str(),
        "open"
    );
    assert_eq!(
        CheckboxState::from_normalized_state("done")
            .unwrap()
            .as_str(),
        "done"
    );
    assert_eq!(
        CheckboxState::from_normalized_state("none")
            .unwrap()
            .as_str(),
        "none"
    );
    assert!(CheckboxState::from_normalized_state("invalid").is_none());
}

#[test]
fn workflow_modes_normalize_from_contract_values() {
    assert_eq!(
        workflow_mode_value(Some("solo_remote")).unwrap(),
        Some("solo_remote".to_string())
    );
    assert_eq!(
        workflow_mode_value(Some(" solo_local ")).unwrap(),
        Some("solo_local".to_string())
    );
    assert_eq!(workflow_mode_value(None).unwrap(), None);
}

#[test]
fn publication_states_normalize_and_report_unpublished_heads() {
    assert_eq!(
        publication_state_value(Some("published")).unwrap(),
        Some("published".to_string())
    );
    assert_eq!(
        publication_state_value(Some("local_draft")).unwrap(),
        Some("local_draft".to_string())
    );
    assert!(!publication_state_has_unpublished_head(Some("published")).unwrap());
    assert!(publication_state_has_unpublished_head(Some("local_draft")).unwrap());
}

#[test]
fn task_status_details_follow_python_authority_rules() {
    assert_eq!(
        task_status_value(Some(" completed ")).unwrap(),
        Some("completed".to_string())
    );
    let details = task_status_details(Some("later_promotion_excluded")).unwrap();
    assert_eq!(
        details.display_label,
        Some("later-promotion-excluded".to_string())
    );
    assert!(details.closed);
}

#[test]
fn workflow_id_helpers_match_expected_tokens() {
    assert_eq!(
        normalize_id_namespace_prefix(None, Some(DEFAULT_ID_NAMESPACE_PREFIX)).unwrap(),
        "".to_string()
    );
    assert_eq!(
        normalize_id_namespace_prefix(Some(""), Some("AIT")).unwrap(),
        "".to_string()
    );
    assert_eq!(
        normalize_id_namespace_prefix(Some("ait"), Some("AIT")).unwrap(),
        "AIT".to_string()
    );
    assert_eq!(workflow_id_token("PL", None).unwrap(), "PL".to_string());
    assert!(workflow_id_matches(Some("PL-0001"), "PL", None, true).unwrap());
    assert!(workflow_id_matches(Some("AITPL-0001"), "PL", None, true).unwrap());
    assert_eq!(workflow_id_token("STH", None).unwrap(), "STH".to_string());
    assert!(workflow_id_matches(Some("STH-0001"), "STH", None, true).unwrap());
    assert!(workflow_id_matches(Some("AITSTH-0001"), "STH", None, true).unwrap());
    assert_eq!(
        workflow_origin_namespace_prefix("L", Some("AIT")).unwrap(),
        "LAIT".to_string()
    );
    assert_eq!(
        generate_namespaced_sequence_id("T", 17, Some("R"), 4).unwrap(),
        "RT-0017".to_string()
    );
}

#[test]
fn workflow_result_envelopes_share_a_stable_shape() {
    let success = workflow_success_envelope("task_status", Some("completed")).unwrap();
    assert!(success.ok);
    assert_eq!(success.kind, "task_status".to_string());
    assert_eq!(success.value, Some("completed".to_string()));
    assert!(success.error.is_none());

    let failure =
        workflow_error_envelope("task_status", "bad_status", "unsupported", Some("demo")).unwrap();
    assert!(!failure.ok);
    assert_eq!(failure.kind, "task_status".to_string());
    assert!(failure.value.is_none());
    let error = failure.error.unwrap();
    assert_eq!(error.code, "bad_status".to_string());
    assert_eq!(error.message, "unsupported".to_string());
    assert_eq!(error.detail, Some("demo".to_string()));
}
