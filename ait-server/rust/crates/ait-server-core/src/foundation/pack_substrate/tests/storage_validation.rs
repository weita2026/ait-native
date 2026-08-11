use super::super::*;

#[test]
fn storage_validation_summary_matches_packed_full_only_case() {
    let summary = build_storage_validation_summary(4, 4, 0, 1, 0, 0, 0.5, 0, 1, None);
    assert_eq!(summary["state"], "packed_full_only");
    assert_eq!(summary["recommended_action"], "repack");
}
