#[cfg(feature = "legacy-postgres-runtime")]
#[test]
fn postgres_runtime_probe_requires_configured_dsn() {
    assert_failed_with(
        &run_seam_without_postgres_dsn(&["postgres-runtime-probe", "{}"]),
        "PostgreSQL backend requested but AIT_NATIVE_SERVER_POSTGRES_DSN is not configured",
    );
}

#[cfg(feature = "legacy-postgres-runtime")]
#[test]
fn server_context_command_shapes_paths_and_can_bootstrap_directories() {
    let root = env::temp_dir().join(format!("ait-server-context-seam-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let payload = json!({
        "root": root,
        "backend": "postgres",
        "postgres_dsn": "postgres://example/ait",
        "content_schema": "custom_content",
        "control_schema": "custom_control",
        "ensure_directories": true,
    })
    .to_string();

    let value = stdout_json(&run_seam(&["server-context", "create", &payload]));

    assert_eq!(value["contract"], json!("ait.server.server_context.v1"));
    assert_eq!(value["context"]["db_backend"], json!("postgres"));
    assert_eq!(
        value["context"]["postgres_dsn"],
        json!("postgres://example/ait")
    );
    assert_eq!(value["context"]["content_schema"], json!("custom_content"));
    assert_eq!(value["context"]["control_schema"], json!("custom_control"));
    for field in [
        "root",
        "manifest_dir",
        "pack_dir",
        "tree_pack_dir",
        "ref_root",
    ] {
        let path = PathBuf::from(value["context"][field].as_str().expect("path field"));
        assert!(path.is_dir(), "{field} should be created at {path:?}");
    }
    let _ = fs::remove_dir_all(root);
}

#[cfg(feature = "legacy-postgres-runtime")]
#[test]
fn server_context_command_from_env_uses_payload_environment() {
    let root = env::temp_dir().join(format!(
        "ait-server-context-env-seam-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let payload = json!({
        "env": {
            "AIT_RUNTIME_DATA": root,
            "AIT_NATIVE_SERVER_DB_BACKEND": "postgres",
            "AIT_NATIVE_SERVER_POSTGRES_DSN": "postgres://example/ait"
        },
        "ensure_directories": true,
    })
    .to_string();

    let value = stdout_json(&run_seam(&["server-context", "from-env", &payload]));

    assert_eq!(value["context"]["root_source"], json!("env"));
    assert_eq!(value["context"]["using_postgres"], json!(true));
    assert!(PathBuf::from(value["context"]["ref_root"].as_str().expect("ref root")).is_dir());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn async_job_contract_command_returns_supported_job_metadata() {
    let value = stdout_json(&run_seam(&["async-job-contract"]));
    let rows = value.as_array().expect("contract should be an array");
    let content_gc = rows
        .iter()
        .find(|row| row.get("job_type") == Some(&json!("content.gc")))
        .expect("content.gc contract should be advertised");

    assert_eq!(content_gc.get("max_attempts"), Some(&json!(3)));
    assert_eq!(content_gc.get("retry_delay_seconds"), Some(&json!(3)));
}

#[test]
fn identity_command_normalizes_rows_and_derives_patchset_ids() {
    let normalized = stdout_json(&run_seam(&[
        "identity",
        "normalize-change-row",
        r#"{"row":{"change_id":"RSEC-0133","risk_tier":"high","lane":"legacy","status":"review"}}"#,
    ]));
    assert_eq!(normalized["contract"], json!("ait.server.identity.v1"));
    assert_eq!(normalized["row"]["change_id"], json!("RSEC-0133"));
    assert_eq!(normalized["row"]["status"], json!("review"));
    assert!(normalized["row"].get("risk_tier").is_none());
    assert!(normalized["row"].get("lane").is_none());

    let patchset = stdout_json(&run_seam(&[
        "identity",
        "derive-patchset-id",
        r#"{"change_id":"RSEC-0133","patchset_number":2}"#,
    ]));
    assert_eq!(patchset["patchset_id"], json!("RSEP-0133-2"));
}

#[test]
fn land_request_command_shapes_payload_freshness_and_alignment() {
    let payload = stdout_json(&run_seam(&[
        "land-request",
        "payload",
        r#"{"row":{"submission_id":"RSEL-1","priority":7,"status":"blocked","result_json":"{\"blocker_class\":\"BASE_STALE\"}"}}"#,
    ]));
    assert_eq!(payload["contract"], json!("ait.server.land_request.v1"));
    assert_eq!(payload["land_request"]["submission_id"], json!("RSEL-1"));
    assert_eq!(
        payload["land_request"]["result"]["blocker_class"],
        json!("BASE_STALE")
    );
    assert!(payload["land_request"].get("priority").is_none());

    let alignment = stdout_json(&run_seam(&[
        "land-request",
        "snapshot-alignment",
        r#"{"target_line_head":"SNP-LATER","revision_snapshot_id":"SNP-REV","target_manifest_hash":"mh-later","revision_manifest_hash":"mh-rev","target_root_tree_id":"TREE-SAME","revision_root_tree_id":"TREE-SAME"}"#,
    ]));
    assert_eq!(
        alignment["alignment"]["target_matches_revision_snapshot"],
        json!(false)
    );
    assert_eq!(
        alignment["alignment"]["target_matches_revision_tree"],
        json!(true)
    );
    assert_eq!(
        alignment["alignment"]["already_aligned_equivalent"],
        json!(true)
    );

    let freshness = stdout_json(&run_seam(&[
        "land-request",
        "freshness-result",
        r#"{"target_line":"main","target_line_head":"SNP-LATER","checked_at":"2026-07-08T10:00:00Z","patchset":{"base_snapshot_id":"SNP-BASE","revision_snapshot_id":"SNP-REV"},"alignment":{"target_matches_revision_snapshot":false,"target_matches_revision_tree":true}}"#,
    ]));
    assert_eq!(freshness["freshness"]["base_is_fresh"], json!(false));
    assert_eq!(
        freshness["freshness"]["already_aligned_equivalent"],
        json!(true)
    );
    assert_eq!(
        freshness["freshness"]["checked_at"],
        json!("2026-07-08T10:00:00Z")
    );

    let timings = stdout_json(&run_seam(&[
        "land-request",
        "phase-timings",
        r#"{"result":{"phase_timings_ms":{"target_alignment":1.25}}}"#,
    ]));
    assert_eq!(timings["phase_timings_ms"]["target_alignment"], json!(1.25));
}

#[test]
fn plan_revision_command_shapes_metadata_and_revision_view() {
    let metadata = stdout_json(&run_seam(&[
        "plan-revision",
        "metadata",
        r##"{"items":[{"plan_item_ref":"demo/first","text":" First   task ","checkbox_state":"todo","heading_path":["Sprint"],"line_number":3}],"artifact_body":"# Sprint\n\n- [ ] First task\n  Detail"}"##,
    ]));
    assert_eq!(metadata["contract"], json!("ait.server.plan_revision.v1"));
    assert_eq!(metadata["plan_links_changed_count_to_prev"], json!(0));
    assert_eq!(
        metadata["entries"]["demo/first"]["details"],
        json!("Detail")
    );

    let view = stdout_json(&run_seam(&[
        "plan-revision",
        "revision-view",
        r##"{"row":{"plan_revision_id":"PR-1","items_json":"[{\"plan_item_ref\":\"demo/first\",\"text\":\"First\"}]","plan_links_changed_count_to_prev":"bad"},"blob":{"blob_id":"BLB-1","media_type":"text/markdown","encoding":"utf-8","byte_count":12,"created_at":"2026-07-08T10:00:00Z"},"include_artifact_body":true,"artifact_body":"# Sprint"}"##,
    ]));
    assert_eq!(
        view["revision"]["items"][0]["plan_item_ref"],
        json!("demo/first")
    );
    assert_eq!(
        view["revision"]["plan_links_changed_count_to_prev"],
        json!(0)
    );
    assert_eq!(view["revision"]["artifact_blob_id"], json!("BLB-1"));
    assert_eq!(view["revision"]["artifact_body"], json!("# Sprint"));
}

#[test]
fn normalize_agent_server_job_command_shapes_scheduler_contract() {
    let value = stdout_json(&run_seam(&[
        "normalize-agent-server-job",
        r#"{"job_kind":"agent.turn.submit","repo_name":"ait","idempotency_key":"idem-1","payload":{"message":"hi"}}"#,
    ]));

    assert_eq!(
        value["contract_version"],
        json!("ait.agent_server_protocol.v2")
    );
    assert_eq!(
        value["singleflight_key"],
        json!("agent:ait:agent.turn.submit:idem-1")
    );
    assert_eq!(value["read_keys"], json!([]));
    assert_eq!(
        value["write_keys"],
        json!(["repo:ait:agent-turn:idem-1"])
    );
}

#[test]
fn agent_server_protocol_schema_command_preserves_schema_shape() {
    let value = stdout_json(&run_seam(&["agent-server-protocol-schema"]));

    assert_eq!(
        value["properties"]["contract_version"]["const"],
        json!("ait.agent_server_protocol.v2")
    );
    assert_eq!(
        value["properties"]["job_kind"]["enum"],
        json!(["agent.turn.submit"])
    );
    assert_eq!(
        value["required"],
        json!([
            "contract_version",
            "job_kind",
            "repo_name",
            "idempotency_key",
            "payload",
            "singleflight_key",
            "read_keys",
            "write_keys",
            "cpu_tokens",
            "io_tokens",
            "remote_tokens",
            "db_tokens",
            "priority",
            "lease_timeout_seconds",
            "retry_policy"
        ])
    );
}

#[test]
fn normalize_agent_server_job_command_accepts_stdin_and_file_payload_markers() {
    let stdin_payload = json!({
        "operation_kind": "agent.turn.submit",
        "repo_name": "ait",
        "idempotency_key": "idem-stdin",
        "payload": {"message": "from stdin"}
    })
    .to_string();
    let stdin_value = stdout_json(&run_seam_with_stdin(
        &["normalize-agent-server-job", "-"],
        &stdin_payload,
    ));
    assert_eq!(stdin_value["job_kind"], json!("agent.turn.submit"));
    assert_eq!(
        stdin_value["write_keys"],
        json!(["repo:ait:agent-turn:idem-stdin"])
    );

    let file_payload = json!({
        "job_kind": "agent.turn.submit",
        "repo_name": "ait",
        "idempotency_key": "idem-file",
        "transport": "telegram",
        "payload": {"message": "from file"}
    })
    .to_string();
    let path = temp_payload_file("normalize-agent-server-job", &file_payload);
    let marker = format!("@{}", path.display());
    let file_value = stdout_json(&run_seam(&["normalize-agent-server-job", &marker]));
    let _ = fs::remove_file(path);

    assert_eq!(
        file_value["singleflight_key"],
        json!("agent:ait:agent.turn.submit:idem-file")
    );
    assert_eq!(file_value["transport"], json!("telegram"));
}

#[test]
fn normalize_agent_server_job_command_preserves_failure_text() {
    assert_failed_with(
        &run_seam(&["normalize-agent-server-job", "{bad-json"]),
        "agent server job request must be valid JSON:",
    );
    assert_failed_with(
        &run_seam(&["normalize-agent-server-job", "[]"]),
        "agent server job request must be a JSON object.",
    );
    assert_failed_with(
        &run_seam(&[
            "normalize-agent-server-job",
            r#"{"job_kind":"agent.turn.submit","repo_name":"ait","idempotency_key":"idem-1","payload":[]}"#,
        ]),
        "agent server job payload must be a JSON object.",
    );
}

#[test]
fn normalize_async_job_payload_command_applies_defaults() {
    let value = stdout_json(&run_seam(&[
        "normalize-async-job-payload",
        "content.gc",
        r#"{"repo_name":"repo-alpha"}"#,
    ]));

    assert_eq!(
        value,
        json!({
            "repo_name": "repo-alpha",
            "prune_unreferenced": true,
            "prune_orphan_packs": true,
        })
    );
}

#[test]
fn server_storage_command_accepts_stdin_payload_marker() {
    let payload = json!({
        "packed_blob_count": 4,
        "packed_full_blob_count": 4,
        "packed_delta_blob_count": 0,
        "pack_count": 1,
        "pack_index_error_count": 0,
        "tree_pack_index_error_count": 0,
        "storage_savings_ratio": 0.5,
        "unreferenced_blob_count": 0,
        "unreferenced_tree_count": 1,
    })
    .to_string();
    let value = stdout_json(&run_seam_with_stdin(
        &["server-storage", "build-storage-validation-summary", "-"],
        &payload,
    ));

    assert_eq!(value.get("state"), Some(&json!("packed_full_only")));
    assert_eq!(value.get("storage_savings_ratio"), Some(&json!(0.5)));
}

#[test]
fn normalize_async_job_payload_command_accepts_file_payload_marker() {
    let path = temp_payload_file("normalize-async-job", r#"{"repo_name":"repo-file"}"#);
    let marker = format!("@{}", path.display());
    let value = stdout_json(&run_seam(&[
        "normalize-async-job-payload",
        "content.gc",
        &marker,
    ]));
    let _ = fs::remove_file(path);

    assert_eq!(
        value,
        json!({
            "repo_name": "repo-file",
            "prune_unreferenced": true,
            "prune_orphan_packs": true,
        })
    );
}

#[test]
fn normalize_async_job_payload_command_accepts_patchset_ci_profile_fields() {
    let value = stdout_json(&run_seam(&[
        "normalize-async-job-payload",
        "patchset.ci",
        r#"{"patchset_id":"RP-1","repo_name":"ait","trigger":"workflow_ready_apply","execution_profile":"workflow_ready_foreground"}"#,
    ]));

    assert_eq!(value["patchset_id"], json!("RP-1"));
    assert_eq!(value["repo_name"], json!("ait"));
    assert_eq!(value["trigger"], json!("workflow_ready_apply"));
    assert_eq!(
        value["execution_profile"],
        json!("workflow_ready_foreground")
    );
}

#[test]
fn retry_delay_command_shapes_json_response() {
    let value = stdout_json(&run_seam(&[
        "retry-delay-seconds-for-job",
        "policy.evaluate",
    ]));

    assert_eq!(
        value,
        json!({
            "job_type": "policy.evaluate",
            "retry_delay_seconds": 3,
        })
    );
}

#[test]
fn workflow_async_runtime_command_shapes_patchset_ci_start_plan() {
    let payload = json!({
        "patchset_id": "RP-SEAM",
        "trigger": "patchset_select",
        "contract_available": true,
        "queue_mode": "async"
    })
    .to_string();

    let value = stdout_json(&run_seam(&[
        "workflow-async-runtime",
        "patchset-ci-start-plan",
        &payload,
    ]));

    assert_eq!(
        value["contract"],
        json!("ait.server.workflow_async.patchset_ci_start_plan.v1")
    );
    assert_eq!(value["state"], json!("enqueue"));
    assert_eq!(value["delivery"], json!("async_queue"));
    assert_eq!(value["enqueue"]["job_type"], json!("patchset.ci"));
}

#[test]
fn workflow_artifacts_command_shapes_release_rows() {
    let payload = json!({
        "row": {
            "release_id": "REL-SEAM",
            "line_name": "release/1",
            "package_name": "ait",
            "package_version": "1.0.0",
            "package_requires_python": ">=3.11",
            "artifacts_json": serde_json::to_string(&json!([
                {"kind": "sdist", "path": "/tmp/dist/ait-1.0.0.tar.gz", "sha256": "abc"}
            ])).expect("artifact JSON should serialize"),
            "formula_json": "{}",
            "metadata_json": "{}"
        }
    })
    .to_string();

    let value = stdout_json(&run_seam(&["workflow-artifacts", "release-row", &payload]));

    assert_eq!(value["contract"], json!("ait.server.workflow_artifacts.v1"));
    assert_eq!(value["release"]["line"], json!("release/1"));
    assert_eq!(
        value["release"]["artifacts"][0]["download_path"],
        json!("/v1/native/releases/REL-SEAM/artifacts/sdist")
    );
    assert_eq!(
        value["release"]["artifacts"][0]["download_name"],
        json!("ait-1.0.0.tar.gz")
    );
}

#[test]
fn workflow_artifacts_command_shapes_release_formula_payload() {
    let payload = json!({
        "formula": {"name": "ait", "class_name": "Ait", "artifact_kind": "sdist"},
        "artifacts": [
            {"kind": "sdist", "path": "dist/ait.tar.gz", "sha256": "abc"},
            {"kind": "formula", "path": "Formula/ait.rb"}
        ]
    })
    .to_string();

    let value = stdout_json(&run_seam(&[
        "workflow-artifacts",
        "release-formula-payload",
        &payload,
    ]));

    assert_eq!(value["contract"], json!("ait.server.workflow_artifacts.v1"));
    assert_eq!(
        value["reference_module"],
        json!("../ait/src/ait_native/server_api.py")
    );
    assert_eq!(value["formula"]["name"], json!("ait"));
    assert_eq!(value["formula"]["class_name"], json!("Ait"));
    assert_eq!(value["formula"]["path"], json!("Formula/ait.rb"));
    assert_eq!(value["formula"]["sha256"], json!("abc"));
}

#[test]
fn workflow_artifacts_command_shapes_review_summary() {
    let payload = json!({
        "patchset_id": "RSEP-SEAM",
        "reviews": [
            {
                "review_id": 1,
                "patchset_id": "RSEP-SEAM",
                "reviewer": "Alice",
                "action": "task_approve",
                "comment": "looks good",
                "blocking": false
            },
            {
                "review_id": 2,
                "patchset_id": "RSEP-SEAM",
                "reviewer": "Alice",
                "action": "task_request_changes",
                "comment": "later finding",
                "blocking": false
            },
            {
                "review_id": 3,
                "patchset_id": "RSEP-SEAM",
                "reviewer": "Reviewer",
                "action": "approve",
                "comment": "ship",
                "blocking": false
            },
            {
                "review_id": 4,
                "patchset_id": "RSEP-SEAM",
                "reviewer": "Reviewer",
                "action": "code_review_summary",
                "comment": "Reviewed files: rust/lib.rs; Findings: none; Risks: low; Tests: cargo test; Recommendation: land",
                "blocking": false
            },
            {
                "review_id": 5,
                "patchset_id": "RSEP-OLD",
                "reviewer": "Old",
                "action": "approve",
                "comment": "old patchset",
                "blocking": false
            }
        ]
    })
    .to_string();

    let value = stdout_json(&run_seam(&[
        "workflow-artifacts",
        "review-summary",
        &payload,
    ]));

    assert_eq!(value["contract"], json!("ait.server.workflow_artifacts.v1"));
    assert_eq!(
        value["migration_status"],
        json!("rust_owned_no_python_reference")
    );
    assert!(value.get("reference_module").is_none());
    assert_eq!(value["summary"]["approval_count"], json!(1));
    assert_eq!(value["summary"]["task_approval_count"], json!(0));
    assert_eq!(value["summary"]["team_approval_count"], json!(1));
    assert_eq!(value["summary"]["human_approval_count"], json!(1));
    assert_eq!(
        value["summary"]["independent_human_approval_count"],
        json!(0)
    );
    assert_eq!(value["summary"]["blocking_count"], json!(1));
    assert_eq!(value["summary"]["comment_count"], json!(1));
    assert_eq!(value["summary"]["code_review_summary_count"], json!(1));
    assert_eq!(value["summary"]["review_count"], json!(4));
}

#[test]
fn policy_gate_command_shapes_policy_evaluation() {
    let payload = json!({
        "patchset": {"patchset_id": "RSEP-POLICY"},
        "effective_requirements": {
            "require_attestation": true,
            "require_tests": true,
            "require_lint": false
        },
        "attestation": {
            "evaluation_summary": {"tests": "pass", "lint": "failed"},
            "provenance_summary": {"policy_readable": true}
        },
        "review_summary": {
            "approval_count": 1,
            "code_review_summary_count": 0
        }
    })
    .to_string();

    let value = stdout_json(&run_seam(&["policy-gate", "evaluate", &payload]));

    assert_eq!(value["contract"], json!("ait.server.policy_gate.v1"));
    assert_eq!(
        value["migration_status"],
        json!("rust_owned_no_python_reference")
    );
    assert_eq!(value["reference_modules"], json!([]));
    assert!(value.get("reference_module").is_none());
    assert_eq!(value["evaluation"]["decision"], json!("pass"));
    assert!(value["evaluation"]["checks"]
        .as_array()
        .expect("checks should be an array")
        .iter()
        .any(|check| check["name"] == json!("lint") && check["status"] == json!("optional_fail")));
}

#[test]
fn policy_gate_command_shapes_waiver_request() {
    let payload = json!({
        "patchset_id": "RP-2477-1",
        "rule_name": "security_scan",
        "reason": "accepted risk",
        "expires_at": null,
        "existing_waiver_count": 1,
        "created_at": "2026-07-08T00:00:00+00:00",
        "change_id": "RC-2477"
    })
    .to_string();

    let value = stdout_json(&run_seam(&["policy-gate", "waiver-request", &payload]));

    assert_eq!(value["contract"], json!("ait.server.policy_gate.v1"));
    assert_eq!(value["waiver"]["waiver_id"], json!("W-2477-1-2"));
    assert_eq!(value["waiver"]["rule_name"], json!("security_scan"));
}
