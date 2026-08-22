use ait_server_core::foundation::io::{
    postgres_schema_ready_key, require_schema_ready, snapshot_manifest_map_from_rows,
    PostgresSchemaReadyCache,
};
use serde_json::Map as JsonMap;
use serde_json::{json, Value as JsonValue};

#[test]
fn snapshot_manifest_map_from_rows_shapes_rows_into_map() {
    let rows = json!([
        {
            "path": "nested/file-a.txt",
            "blob_id": "BLB-a",
            "size_bytes": 12,
            "mode": "0o644",
            "sha256": "hash-a",
            "extra": "ignored",
        },
        {
            "path": "file-b.txt",
            "blob_id": "BLB-b",
            "mode": "0o600",
            "sha256": "hash-b",
        },
    ]);

    let rows = rows
        .as_array()
        .cloned()
        .expect("rows fixture should be an array");
    let manifest = snapshot_manifest_map_from_rows(&rows).expect("rows should shape into manifest");
    let expected: JsonMap<String, JsonValue> = serde_json::from_value(json!({
        "nested/file-a.txt": {
            "blob_id": "BLB-a",
            "size_bytes": 12,
            "mode": "0o644",
            "sha256": "hash-a",
        },
        "file-b.txt": {
            "blob_id": "BLB-b",
            "size_bytes": null,
            "mode": "0o600",
            "sha256": "hash-b",
        },
    }))
    .expect("expected manifest should be valid json");

    assert_eq!(manifest, expected);
}

#[test]
fn snapshot_manifest_map_from_rows_rejects_bad_rows() {
    let invalid_rows: Vec<(JsonValue, &str)> = vec![
        (
            json!({
                "blob_id": "BLB-a",
                "size_bytes": 10,
                "mode": "0o644",
                "sha256": "hash-a",
            }),
            "path is required",
        ),
        (
            json!({
                "path": "file.txt",
                "size_bytes": 10,
                "mode": "0o644",
                "sha256": "hash-a",
            }),
            "blob_id is required",
        ),
        (
            json!({
                "path": "",
                "blob_id": "BLB-a",
                "size_bytes": 10,
                "mode": "0o644",
                "sha256": "hash-a",
            }),
            "path is required",
        ),
        (
            json!({
                "path": "file.txt",
                "blob_id": "BLB-a",
                "size_bytes": 10,
                "mode": "",
                "sha256": "hash-a",
            }),
            "mode is required",
        ),
    ];

    for (row, expected_error) in invalid_rows {
        let actual = snapshot_manifest_map_from_rows(&[row]);
        let error = actual.expect_err("invalid manifest row should fail");
        assert!(
            error.contains(expected_error),
            "unexpected error message: {error}"
        );
    }
}

#[test]
fn snapshot_manifest_map_from_rows_skips_non_object_rows_with_error() {
    let actual = snapshot_manifest_map_from_rows(&[json!("not-a-row")]);
    let error = actual.expect_err("non-object rows should fail");
    assert!(error.contains("snapshot row must be an object"), "{error}");
}

#[test]
fn postgres_schema_ready_key_and_cache_are_predictable() {
    assert_eq!(
        postgres_schema_ready_key("postgres", Some("postgresql://demo "), Some(" schema_one ")),
        Some(ait_server_core::foundation::io::PostgresSchemaReadyKey {
            dsn: "postgresql://demo".to_string(),
            schema: "schema_one".to_string(),
        }),
    );
    assert_eq!(
        postgres_schema_ready_key("local-file", Some("postgresql://demo"), Some("schema_one")),
        None
    );
    assert_eq!(
        postgres_schema_ready_key("postgres", Some(""), Some("schema_one")),
        None
    );

    let mut cache = PostgresSchemaReadyCache::default();
    let key = postgres_schema_ready_key("postgres", Some("dsn"), Some("schema"));
    assert!(!cache.is_ready(key.as_ref()));
    cache.mark_ready(key.clone());
    assert!(cache.is_ready(key.as_ref()));

    let missing_key = postgres_schema_ready_key("postgres", Some("other"), Some("other"));
    assert!(!cache.is_ready(missing_key.as_ref()));
    assert!(cache.is_ready(None));

    cache.reset();
    assert!(!cache.is_ready(key.as_ref()));
    assert!(cache.is_ready(None));
}

#[test]
fn require_schema_ready_contract_is_stable() {
    assert_eq!(require_schema_ready(true), Ok(()));
    assert_eq!(
        require_schema_ready(false).expect_err("unready schema should fail"),
        "Content schema bootstrap has not run in this process. Call server_content.initialize(ctx) during ait-server startup before serving request-time content helpers."
    );
}
