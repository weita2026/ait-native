use super::*;

use super::*;

#[test]
fn contract_declares_postgres_patchset_store_operations() {
    let value = server_patchset_store_json("contract", &json!({})).expect("contract");
    assert_eq!(value["contract"], json!(SERVER_PATCHSET_STORE_CONTRACT));
    assert_eq!(value["backend"], json!("postgres"));
    assert_eq!(
        value["migration_status"],
        json!("rust_owned_no_python_reference")
    );
    assert!(value.get("previous_reference_module").is_none());
    assert_eq!(value["mutates_state"], json!(true));
    assert!(value["operations"]
        .as_array()
        .unwrap()
        .contains(&json!("publish-patchset")));
    assert!(value["operations"]
        .as_array()
        .unwrap()
        .contains(&json!("upsert-attestation")));
}

#[test]
fn runtime_rejects_non_postgres_and_fake_postgres() {
    let err = server_patchset_store_json(
        "get-patchset",
        &json!({"backend": "local-file", "patchset_id": "PS-1", "dsn": "postgresql://demo"}),
    )
    .expect_err("non-postgres backend should be rejected");
    assert!(err.contains("Only PostgreSQL is supported"));

    let err = server_patchset_store_json(
        "get-patchset",
        &json!({"backend": "postgres", "patchset_id": "PS-1", "dsn": "fake-postgres:///tmp/x"}),
    )
    .expect_err("fake postgres should be rejected");
    assert!(err.contains("fake-postgres is not supported"));
}

#[test]
fn diff_stats_compares_manifest_maps() {
    let base: BTreeMap<String, JsonValue> = BTreeMap::from_iter([
        (
            "a.txt".to_string(),
            json!({"blob_id":"A","size_bytes":1,"mode":"0o644","sha256":"a"}),
        ),
        (
            "b.txt".to_string(),
            json!({"blob_id":"B","size_bytes":1,"mode":"0o644","sha256":"b"}),
        ),
    ]);
    let revision: BTreeMap<String, JsonValue> = BTreeMap::from_iter([
        (
            "b.txt".to_string(),
            json!({"blob_id":"B2","size_bytes":2,"mode":"0o644","sha256":"b2"}),
        ),
        (
            "c.txt".to_string(),
            json!({"blob_id":"C","size_bytes":1,"mode":"0o644","sha256":"c"}),
        ),
    ]);
    let stats = diff_stats_for_maps(&base, &revision);
    assert_eq!(stats["files_added"], json!(1));
    assert_eq!(stats["files_deleted"], json!(1));
    assert_eq!(stats["files_modified"], json!(1));
    assert_eq!(stats["paths"]["added"], json!(["c.txt"]));
    assert_eq!(stats["paths"]["deleted"], json!(["a.txt"]));
    assert_eq!(stats["paths"]["modified"], json!(["b.txt"]));
}

fn diff_stats_for_maps(
    base_map: &BTreeMap<String, JsonValue>,
    revision_map: &BTreeMap<String, JsonValue>,
) -> JsonValue {
    let base_paths = base_map.keys().cloned().collect::<HashSet<_>>();
    let revision_paths = revision_map.keys().cloned().collect::<HashSet<_>>();
    let mut added = revision_paths
        .difference(&base_paths)
        .cloned()
        .collect::<Vec<_>>();
    let mut deleted = base_paths
        .difference(&revision_paths)
        .cloned()
        .collect::<Vec<_>>();
    let mut modified = base_paths
        .intersection(&revision_paths)
        .filter(|path| base_map.get(*path) != revision_map.get(*path))
        .cloned()
        .collect::<Vec<_>>();
    added.sort();
    deleted.sort();
    modified.sort();
    json!({
        "files_added": added.len(),
        "files_deleted": deleted.len(),
        "files_modified": modified.len(),
        "files_changed": added.len() + deleted.len() + modified.len(),
        "paths": {
            "added": added,
            "deleted": deleted,
            "modified": modified,
        }
    })
}
