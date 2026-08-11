use super::*;

#[test]
fn summary_parser_accepts_numbered_sections() {
    let text = "1. Reviewed files\nsrc/lib.rs\n2. Findings\nNone\n3. Risks\nLow\n4. Tests\ncargo test\n5. Recommendation\nland";
    assert!(is_structured_code_review_summary(Some(text)));
}

#[test]
fn yaml_round_trip_preserves_defaults() {
    let policy = policy_profile("team").expect("team policy");
    let yaml = policy_to_yaml(Some(&policy), "prototype").expect("yaml");
    let parsed = parse_policy_yaml(&yaml, "prototype").expect("parsed");
    assert_eq!(parsed["policy_id"], "team");
    assert_eq!(parsed["defaults"]["require_lint"], true);
}
