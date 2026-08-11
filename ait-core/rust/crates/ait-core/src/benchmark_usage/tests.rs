use super::*;
use tempfile::tempdir;

#[test]
fn extracts_token_count_usage_projects_manifest_contract() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("usage.jsonl");
    fs::write(
        &path,
        r#"{"payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":3,"output_tokens":2},"total_token_usage":{"input_tokens":8,"output_tokens":4,"cached_input_tokens":1,"reasoning_output_tokens":2}}}}"#,
    )
    .expect("write usage");

    let payload = extract_codex_usage_jsonl(&path, "total").expect("extract usage");
    let mut payload_keys = payload
        .as_object()
        .expect("usage payload")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    payload_keys.sort();
    assert_eq!(
        payload_keys,
        vec![
            "last_token_usage",
            "manifest_usage",
            "token_event_count",
            "total_token_usage",
            "usage_jsonl_path",
            "usage_scope",
            "usage_source",
        ]
    );

    assert_eq!(payload["usage_jsonl_path"], json!(path.to_string_lossy()));
    assert_eq!(payload["token_event_count"], json!(1));
    assert_eq!(payload["manifest_usage"]["prompt_tokens"], json!(8));
    assert_eq!(payload["manifest_usage"]["completion_tokens"], json!(4));
    assert_eq!(payload["manifest_usage"]["total_tokens"], json!(12));
}

#[test]
fn aggregates_usage_files_by_role() {
    let dir = tempdir().expect("tempdir");
    let coordinator = dir.path().join("coordinator.jsonl");
    let worker = dir.path().join("worker.jsonl");
    fs::write(
        &coordinator,
        r#"{"type":"turn.completed","usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}"#,
    )
    .expect("write coordinator");
    fs::write(
        &worker,
        r#"{"type":"turn.completed","usage":{"input_tokens":11,"output_tokens":4,"total_tokens":15}}"#,
    )
    .expect("write worker");

    let payload = extract_codex_usage_bundle_jsonl(
        &[
            coordinator.to_string_lossy().to_string(),
            worker.to_string_lossy().to_string(),
        ],
        Some(&["coordinator".to_string(), "worker".to_string()]),
        "total",
    )
    .expect("aggregate usage");

    assert_eq!(payload["usage_file_count"], json!(2));
    assert_eq!(payload["manifest_usage"]["total_tokens"], json!(22));
    assert_eq!(
        payload["role_breakdown"]["coordinator"]["usage"]["total_tokens"],
        json!(7)
    );
    assert_eq!(
        payload["role_breakdown"]["worker"]["usage"]["total_tokens"],
        json!(15)
    );
}
