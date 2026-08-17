use ait_server_core::foundation::admin_cache::{
    admin_cache_contract, admin_cache_json_with_cache, admin_metrics_cache_ttl_seconds,
    annotated_admin_payload, AdminResponseCache, ADMIN_CACHE_CONTRACT_VERSION,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

fn object(payload: JsonValue) -> JsonMap<String, JsonValue> {
    payload.as_object().expect("object").clone()
}

#[test]
fn admin_cache_contract_names_reference_and_boundaries() {
    let contract = admin_cache_contract();
    assert_eq!(contract["contract"], json!(ADMIN_CACHE_CONTRACT_VERSION));
    assert_eq!(contract["reference_modules"], json!([]));
    assert_eq!(
        contract["migration_status"],
        json!("python_wrapper_removed_rust_owned")
    );
    assert_eq!(contract["defaults"]["ttl_seconds"], json!(5.0));
    assert_eq!(
        contract["annotation_fields"],
        json!([
            "cache_state",
            "cache_age_seconds",
            "cache_ttl_seconds",
            "cached_at"
        ])
    );
}

#[test]
fn admin_cache_ttl_parsing_matches_retired_python_wrapper_contract() {
    assert_eq!(admin_metrics_cache_ttl_seconds(None), 5.0);
    assert_eq!(admin_metrics_cache_ttl_seconds(Some("  ")), 5.0);
    assert_eq!(admin_metrics_cache_ttl_seconds(Some("bad")), 5.0);
    assert_eq!(admin_metrics_cache_ttl_seconds(Some("-2.5")), 0.0);
    assert_eq!(admin_metrics_cache_ttl_seconds(Some("0")), 0.0);
    assert_eq!(admin_metrics_cache_ttl_seconds(Some("1.25")), 1.25);

    let mut cache = AdminResponseCache::new();
    let ttl = admin_cache_json_with_cache(&mut cache, "ttl", &object(json!({"raw": "2.5"})))
        .expect("ttl operation");
    assert_eq!(ttl["cache_ttl_seconds"], json!(2.5));
}

#[test]
fn admin_cache_annotation_clamps_and_rounds_age() {
    let annotated = annotated_admin_payload(
        object(json!({"pressure": "low"})),
        "cached",
        1.23456,
        5.0,
        "2026-07-08T00:00:00Z",
    );
    assert_eq!(annotated["pressure"], json!("low"));
    assert_eq!(annotated["cache_state"], json!("cached"));
    assert_eq!(annotated["cache_age_seconds"], json!(1.235));
    assert_eq!(annotated["cache_ttl_seconds"], json!(5.0));
    assert_eq!(annotated["cached_at"], json!("2026-07-08T00:00:00Z"));

    let negative = annotated_admin_payload(
        object(json!({})),
        "cached",
        -10.0,
        5.0,
        "2026-07-08T00:00:00Z",
    );
    assert_eq!(negative["cache_age_seconds"], json!(0.0));
}

#[test]
fn admin_cache_stores_computed_payload_and_serves_hits() {
    let mut cache = AdminResponseCache::new();
    let first = admin_cache_json_with_cache(
        &mut cache,
        "cached-payload",
        &object(json!({
            "name": "operator-metrics",
            "key": [2, 10],
            "payload": {"value": "first"},
            "cache_ttl_seconds": 5.0,
            "now_monotonic": 100.0,
            "cached_at": "2026-07-08T00:00:00Z"
        })),
    )
    .expect("first payload");
    assert_eq!(first["payload"]["value"], json!("first"));
    assert_eq!(first["payload"]["cache_state"], json!("computed"));
    assert_eq!(first["payload"]["cache_age_seconds"], json!(0.0));

    let second = admin_cache_json_with_cache(
        &mut cache,
        "cached-payload",
        &object(json!({
            "name": "operator-metrics",
            "key": [2, 10],
            "payload": {"value": "second"},
            "cache_ttl_seconds": 5.0,
            "now_monotonic": 101.23456,
            "cached_at": "2026-07-08T00:00:01Z"
        })),
    )
    .expect("cached payload");
    assert_eq!(second["payload"]["value"], json!("first"));
    assert_eq!(second["payload"]["cache_state"], json!("cached"));
    assert_eq!(second["payload"]["cache_age_seconds"], json!(1.235));
    assert_eq!(
        second["payload"]["cached_at"],
        json!("2026-07-08T00:00:00Z")
    );
}

#[test]
fn admin_cache_ttl_disabled_does_not_store_payload() {
    let mut cache = AdminResponseCache::new();
    let first = admin_cache_json_with_cache(
        &mut cache,
        "cached-payload",
        &object(json!({
            "name": "readiness",
            "key": [1, 1],
            "payload": {"value": "first"},
            "cache_ttl_seconds": 0.0,
            "now_monotonic": 100.0,
            "cached_at": "first-at"
        })),
    )
    .expect("first");
    let second = admin_cache_json_with_cache(
        &mut cache,
        "cached-payload",
        &object(json!({
            "name": "readiness",
            "key": [1, 1],
            "payload": {"value": "second"},
            "cache_ttl_seconds": 0.0,
            "now_monotonic": 101.0,
            "cached_at": "second-at"
        })),
    )
    .expect("second");
    assert_eq!(first["payload"]["value"], json!("first"));
    assert_eq!(second["payload"]["value"], json!("second"));
    assert_eq!(second["payload"]["cache_state"], json!("computed"));
}

#[test]
fn admin_cache_replaces_expired_payload_and_clear_resets_cache() {
    let mut cache = AdminResponseCache::new();
    admin_cache_json_with_cache(
        &mut cache,
        "cached-payload",
        &object(json!({
            "name": "readiness",
            "key": [1, 2],
            "payload": {"value": "first"},
            "cache_ttl_seconds": 5.0,
            "now_monotonic": 100.0,
            "cached_at": "first-at"
        })),
    )
    .expect("first");
    let expired = admin_cache_json_with_cache(
        &mut cache,
        "cached-payload",
        &object(json!({
            "name": "readiness",
            "key": [1, 2],
            "payload": {"value": "second"},
            "cache_ttl_seconds": 5.0,
            "now_monotonic": 106.0,
            "cached_at": "second-at"
        })),
    )
    .expect("expired");
    assert_eq!(expired["payload"]["value"], json!("second"));
    assert_eq!(expired["payload"]["cache_state"], json!("computed"));

    let cleared =
        admin_cache_json_with_cache(&mut cache, "clear", &object(json!({}))).expect("clear");
    assert_eq!(cleared["cleared"], json!(true));
    let after_clear = admin_cache_json_with_cache(
        &mut cache,
        "cached-payload",
        &object(json!({
            "name": "readiness",
            "key": [1, 2],
            "payload": {"value": "third"},
            "cache_ttl_seconds": 5.0,
            "now_monotonic": 107.0,
            "cached_at": "third-at"
        })),
    )
    .expect("after clear");
    assert_eq!(after_clear["payload"]["value"], json!("third"));
    assert_eq!(after_clear["payload"]["cache_state"], json!("computed"));
}
