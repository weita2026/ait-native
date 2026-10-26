use crate::json_support::json;

use super::task_remote_change_lineage_payload;

#[test]
fn lineage_payload_normalizes_inputs() {
    let payload =
        task_remote_change_lineage_payload(" main ", Some(&json!({"head_snapshot_id": " SNP-1 "})))
            .unwrap();
    assert_eq!(
        payload,
        json!({"forked_from_line": "main", "fork_snapshot_id": "SNP-1"})
    );
}

#[test]
fn lineage_payload_rejects_invalid_inputs() {
    assert_eq!(
        task_remote_change_lineage_payload("", None).unwrap_err(),
        "Task remote lineage payload requires `base_line`.".to_string()
    );
    assert_eq!(
        task_remote_change_lineage_payload("main", Some(&json!([]))).unwrap_err(),
        "Task remote lineage line row must be an object when provided.".to_string()
    );
}
