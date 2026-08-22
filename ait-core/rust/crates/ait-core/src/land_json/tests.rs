use super::*;
use crate::server_operational::RepositoryIndex;
use std::collections::BTreeMap;

fn config() -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: "https://example.invalid/api/".to_string(),
        repository_index: Some(RepositoryIndex::new(7)),
        headers: BTreeMap::from([(String::from("X-Test"), String::from("yes"))]),
        default_timeout_ms: 5000,
        retry_attempts: 2,
        retry_backoff_ms: 7,
        pool_max_idle_per_host: 3,
    }
}

#[test]
fn land_json_builds_request_specs_and_bodies() {
    let land = LandJson::stateless();
    let atomic = land
        .build_submit_task_land_request_spec(
            &config(),
            "RCT-1/C-01",
            Some("main"),
            "merge",
            "task-land-atomic:123",
            Some("legacy-name"),
        )
        .unwrap();
    assert_eq!(atomic.path, "/v1/native/repository-authorities/7/task-land");
    assert_eq!(
        atomic.body,
        Some(json!({
            "contract": "task-land-atomic/v1",
            "idempotency_key": "task-land-atomic:123",
            "task_or_change_ref": "RCT-1/C-01",
            "target_line": "main",
            "mode": "merge",
        }))
    );

    let submit = land
        .build_submit_land_request_spec(
            &config(),
            "C-1",
            Some("P-C-1-1"),
            "main",
            "merge",
            Some("repo"),
        )
        .unwrap();
    assert_eq!(submit.method, "POST");
    assert_eq!(
        submit.path,
        "/v1/native/repository-authorities/7/changes/C-1:submit"
    );
    assert_eq!(
        submit.body,
        Some(json!({
            "patchset_id": "P-C-1-1",
            "target_line": "main",
            "mode": "merge",
        }))
    );

    let get = land
        .build_get_land_request_spec(&config(), "LAND-1", Some("repo"))
        .unwrap();
    assert_eq!(get.method, "GET");
    assert_eq!(get.path, "/v1/native/repository-authorities/7/lands/LAND-1");
    assert!(get.body.is_none());

    let retry = land
        .build_retry_land_request_spec(&config(), "LAND-1", Some("retry"), Some("repo"))
        .unwrap();
    assert_eq!(retry.method, "POST");
    assert_eq!(
        retry.path,
        "/v1/native/repository-authorities/7/lands/LAND-1:retry"
    );
    assert_eq!(retry.body, Some(json!({ "reason": "retry" })));
    assert_eq!(
        land.build_submit_land_body(None, "main", "merge").unwrap()["patchset_id"],
        JsonValue::Null
    );
}

#[test]
fn atomic_task_land_spec_requires_exact_repository_scope_and_bounds_identity() {
    let land = LandJson::stateless();
    let mut unscoped = config();
    unscoped.repository_index = None;
    let by_name = land
        .build_submit_task_land_request_spec(
            &unscoped,
            "RCT-1",
            None,
            "direct",
            "stable-key",
            Some("repo/name"),
        )
        .unwrap_err();
    assert!(by_name.to_string().contains("repository_index is required"));

    let missing_scope = land
        .build_submit_task_land_request_spec(&unscoped, "RCT-1", None, "direct", "stable-key", None)
        .unwrap_err();
    assert!(missing_scope
        .to_string()
        .contains("repository_index is required"));

    let oversized = "x".repeat(257);
    let oversized_error = land
        .build_submit_task_land_request_spec(
            &config(),
            "RCT-1",
            None,
            "direct",
            &oversized,
            Some("repo"),
        )
        .unwrap_err();
    assert!(oversized_error.to_string().contains("must not exceed 256"));
}

#[test]
fn land_json_extracts_landing_state_and_recovers_submission() {
    let land = LandJson::stateless();
    let change = json!({
        "change_id": "RCC-1",
        "landing_summary": {
            "status": "succeeded",
            "submission_id": "LAND-1",
            "result": {"landed_snapshot_id": "SNP-2"}
        }
    });
    let change_map = change.as_object().unwrap();
    let landing_summary = change_map
        .get("landing_summary")
        .and_then(JsonValue::as_object);

    assert!(land.change_effectively_landed(change_map, landing_summary));
    assert!(land.change_has_landing_evidence(&change));
    assert_eq!(
        land.landing_summary_status(landing_summary).as_str(),
        "succeeded"
    );
    assert_eq!(
        land.landing_summary_submission_id(landing_summary)
            .as_deref(),
        Some("LAND-1")
    );
    assert_eq!(
        land.landing_summary_result(landing_summary)["landed_snapshot_id"],
        "SNP-2"
    );

    let recovered = land
        .recover_land_submission_from_change_state(&change, "RCC-FALLBACK")
        .unwrap();
    assert_eq!(recovered["status"], "succeeded");
    assert_eq!(recovered["submission_id"], "LAND-1");
    assert_eq!(recovered["response_recovery"]["action"], "submit_land");
    assert_eq!(recovered["response_recovery"]["change_id"], "RCC-1");
}
