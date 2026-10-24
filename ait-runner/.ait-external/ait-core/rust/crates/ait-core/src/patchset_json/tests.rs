use super::*;
use crate::server_operational::RepositoryIndex;
use std::collections::BTreeMap;

fn config() -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        repository_index: Some(RepositoryIndex::new(7)),
        headers: BTreeMap::from([(String::from("X-Test"), String::from("yes"))]),
        default_timeout_ms: 12_345,
        retry_attempts: 2,
        retry_backoff_ms: 7,
        pool_max_idle_per_host: 3,
    }
}

fn repository_authority_config() -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        repository_index: Some(RepositoryIndex::new(9)),
        ..config()
    }
}

#[test]
fn patchset_json_builds_request_specs() {
    let patchset = PatchsetJson::stateless();

    let publish = patchset
        .build_publish_patchset_request_spec(
            &config(),
            " RCC-1 ",
            " SNP-BASE ",
            " SNP-REV ",
            " summary ",
            " codex ",
        )
        .unwrap();
    assert_eq!(publish.method, "POST");
    assert_eq!(
        publish.path,
        "/v1/native/repository-authorities/7/changes/RCC-1/patchsets"
    );
    assert_eq!(
        publish.body,
        Some(json!({
            "base_snapshot_id": "SNP-BASE",
            "revision_snapshot_id": "SNP-REV",
            "summary": "summary",
            "author_mode": "codex",
        }))
    );

    let list = patchset
        .build_list_patchsets_request_spec(&config(), "RCC-1", Some("repo"))
        .unwrap();
    assert_eq!(
        list.path,
        "/v1/native/repository-authorities/7/changes/RCC-1/patchsets"
    );

    let get = patchset
        .build_get_patchset_request_spec(&config(), "RCP-1", None, Some(" RCC-1 "))
        .unwrap();
    assert_eq!(
        get.path,
        "/v1/native/repository-authorities/7/patchsets/RCP-1"
    );
    assert_eq!(
        get.query_pairs,
        vec![("change_ref".to_string(), "RCC-1".to_string())]
    );

    let select = patchset
        .build_select_patchset_request_spec(&config(), "RCC-1", " RCP-1 ")
        .unwrap();
    assert_eq!(
        select.path,
        "/v1/native/repository-authorities/7/changes/RCC-1:selectPatchset"
    );
    assert_eq!(select.body, Some(json!({"patchset_id": "RCP-1"})));

    let run_ci = patchset
        .build_run_patchset_ci_request_spec(
            &config(),
            "RCP-1",
            " workflow_ready_apply ",
            Some(" foreground "),
        )
        .unwrap();
    assert_eq!(
        run_ci.path,
        "/v1/native/repository-authorities/7/patchsets/RCP-1:runCi"
    );
    assert_eq!(
        run_ci.body,
        Some(json!({
            "trigger": "workflow_ready_apply",
            "execution_profile": "foreground",
        }))
    );

    let ci_status = patchset
        .build_read_patchset_ci_status_request_spec(&config(), "RCP-1", 0)
        .unwrap();
    assert_eq!(ci_status.method, "GET");
    assert_eq!(
        ci_status.path,
        "/v1/native/repository-authorities/7/read/patchsets/RCP-1/ci-status"
    );
    assert_eq!(
        ci_status.query_pairs,
        vec![("recent_limit".to_string(), "1".to_string())]
    );

    let readiness = patchset
        .build_read_patchset_ci_readiness_request_spec(&config(), "RCP-1", 200)
        .unwrap();
    assert_eq!(readiness.method, "GET");
    assert_eq!(
        readiness.path,
        "/v1/native/repository-authorities/7/read/patchsets/RCP-1/ci-status"
    );
    assert_eq!(
        readiness.query_pairs,
        vec![
            ("recent_limit".to_string(), "20".to_string()),
            ("projection".to_string(), "readiness".to_string()),
        ]
    );
}

#[test]
fn patchset_ci_specs_prefer_the_configured_repository_authority_id() {
    let patchset = PatchsetJson::stateless();
    let config = repository_authority_config();

    let publish = patchset
        .build_publish_patchset_request_spec(
            &config,
            "RCT-1/C-01",
            "SNP-BASE",
            "SNP-REV",
            "summary",
            "ai_with_human_review",
        )
        .unwrap();
    assert_eq!(
        publish.path,
        "/v1/native/repository-authorities/9/changes/RCT-1%2FC-01/patchsets"
    );

    let list = patchset
        .build_list_patchsets_request_spec(&config, "RCT-1/C-01", Some("legacy-name"))
        .unwrap();
    assert_eq!(list.path, publish.path);

    let get = patchset
        .build_get_patchset_request_spec(
            &config,
            "P-RCT-1/C-01-1",
            Some("legacy-name"),
            Some("RCT-1/C-01"),
        )
        .unwrap();
    assert_eq!(
        get.path,
        "/v1/native/repository-authorities/9/patchsets/P-RCT-1%2FC-01-1"
    );

    let select = patchset
        .build_select_patchset_request_spec(&config, "RCT-1/C-01", "P-RCT-1/C-01-1")
        .unwrap();
    assert_eq!(
        select.path,
        "/v1/native/repository-authorities/9/changes/RCT-1%2FC-01:selectPatchset"
    );

    let run = patchset
        .build_run_patchset_ci_request_spec(
            &config,
            "P-RCT-1/C-01-1",
            "workflow_ready_apply",
            Some("workflow_ready_foreground"),
        )
        .unwrap();
    assert_eq!(
        run.path,
        "/v1/native/repository-authorities/9/patchsets/P-RCT-1%2FC-01-1:runCi"
    );

    let status = patchset
        .build_read_patchset_ci_status_request_spec(&config, "P-RCT-1/C-01-1", 10)
        .unwrap();
    assert_eq!(
        status.path,
        "/v1/native/repository-authorities/9/read/patchsets/P-RCT-1%2FC-01-1/ci-status"
    );

    let readiness = patchset
        .build_read_patchset_ci_readiness_request_spec(&config, "P-RCT-1/C-01-1", 200)
        .unwrap();
    assert_eq!(readiness.path, status.path);
    assert_eq!(
        readiness.query_pairs,
        vec![
            ("recent_limit".to_string(), "20".to_string()),
            ("projection".to_string(), "readiness".to_string()),
        ]
    );
}

#[test]
fn patchset_json_resolves_ids_and_derives_patchset_ids() {
    let patchset = PatchsetJson::stateless();
    assert_eq!(
        patchset.resolved_patchset_id_from_payload(&json!({"patchset_id": " RCP-1 "}), "fallback"),
        "RCP-1"
    );
    assert_eq!(
        patchset.resolved_patchset_id_from_payload(&json!({}), "fallback"),
        "fallback"
    );
    assert_eq!(patchset.patchset_number(&json!({"patchset_number": 3})), 3);
    assert_eq!(
        patchset
            .derive_patchset_id("RC-0779", 2, Some("R"))
            .unwrap(),
        "RP-0779-2"
    );
}

#[test]
fn patchset_json_recovers_published_patchset_from_rows() {
    let patchset = PatchsetJson::stateless();
    let recovered = patchset
        .recover_published_patchset_from_rows(
            vec![
                json!({
                    "patchset_id": "RCP-1-1",
                    "base_snapshot_id": "SNP-BASE",
                    "revision_snapshot_id": "SNP-OLD",
                    "patchset_number": 2,
                }),
                json!({
                    "patchset_id": "RCP-1-2",
                    "base_snapshot_id": "SNP-BASE",
                    "revision_snapshot_id": "SNP-REV",
                    "patchset_number": 2,
                }),
                json!({
                    "patchset_id": "RCP-1-3",
                    "base_snapshot_id": "SNP-BASE",
                    "revision_snapshot_id": "SNP-REV",
                    "patchset_number": 3,
                }),
            ],
            "RCC-1",
            "SNP-BASE",
            "SNP-REV",
            1,
        )
        .unwrap();

    assert_eq!(recovered["patchset_id"], json!("RCP-1-3"));
    assert_eq!(
        recovered["response_recovery"],
        json!({
            "action": "publish_patchset",
            "state": "recovered_from_remote_publish",
            "change_id": "RCC-1",
        })
    );
    assert!(patchset
        .recover_published_patchset_from_rows(
            vec![json!({
                "patchset_id": "RCP-1-1",
                "base_snapshot_id": "SNP-BASE",
                "revision_snapshot_id": "SNP-REV",
                "patchset_number": 1,
            })],
            "RCC-1",
            "SNP-BASE",
            "SNP-REV",
            1,
        )
        .is_none());
}
