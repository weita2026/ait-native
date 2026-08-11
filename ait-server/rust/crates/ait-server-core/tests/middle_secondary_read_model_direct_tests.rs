use ait_server_core::middle::secondary_read_model::{
    authority_map_read_model, authority_map_read_model_contract, reviewer_inbox_read_model,
    reviewer_inbox_read_model_contract, AuthorityMapInput, ReviewerInboxInput,
};
use serde_json::{json, Value as JsonValue};

fn authority_map(payload: JsonValue) -> JsonValue {
    let input = AuthorityMapInput::from_value(&payload).expect("authority input should parse");
    authority_map_read_model(&input).expect("authority map should project")
}

fn reviewer_inbox(payload: JsonValue) -> JsonValue {
    let input = ReviewerInboxInput::from_value(&payload).expect("reviewer input should parse");
    reviewer_inbox_read_model(&input).expect("reviewer inbox should project")
}

fn find_by_path<'a>(items: &'a [JsonValue], path: &str) -> &'a JsonValue {
    items
        .iter()
        .find(|item| item["path"] == json!(path))
        .unwrap_or_else(|| panic!("expected item at path {path}"))
}

#[test]
fn secondary_read_model_contracts_name_rust_ownership_and_row_sets() {
    let authority_contract = authority_map_read_model_contract();
    assert_eq!(authority_contract.domain_id, "authority_map");
    assert_eq!(
        authority_contract.reference_module,
        "rust_owned_no_python_reference"
    );
    assert_eq!(
        authority_contract.public_surface,
        "middle.secondary_read_model.authority_map"
    );
    assert!(!authority_contract.mutates_state);
    assert!(authority_contract.row_set("documents").is_some());
    assert!(authority_contract.row_set("authority_nodes").is_some());
    assert!(authority_contract.row_set("actors").is_some());
    assert!(authority_contract.row_set("permissions").is_some());

    let reviewer_contract = reviewer_inbox_read_model_contract();
    assert_eq!(reviewer_contract.domain_id, "reviewer_inbox");
    assert_eq!(
        reviewer_contract.reference_module,
        "rust_owned_no_python_reference"
    );
    assert_eq!(
        reviewer_contract.public_surface,
        "middle.secondary_read_model.reviewer_inbox"
    );
    assert!(!reviewer_contract.mutates_state);
    assert!(reviewer_contract.row_set("changes").is_some());
    assert!(reviewer_contract.row_set("patchsets").is_some());
    assert!(reviewer_contract.row_set("review_requests").is_some());
    assert!(reviewer_contract.row_set("land_requests").is_some());
}

#[test]
fn authority_map_projects_markdown_documents_into_authority_layers() {
    let value = authority_map(json!({
        "repo_name": "ait-server",
        "local_repo_name": "ait-server",
        "actors": [{"actor_id": "codex"}],
        "roles": [{"role_id": "reviewer"}],
        "permissions": [{"permission_id": "land"}],
        "documents": [
            {
                "path": "docs/plan.md",
                "markdown": "# Plan\nStatus: current\n\n[Engineering](engineering_plan.md)"
            },
            {
                "path": "docs/engineering_plan.md",
                "markdown": "# Engineering Plan\nScope: server runtime"
            },
            {
                "path": "docs/legal_plan.md",
                "markdown": "# Legal Plan"
            },
            {
                "path": "docs/rust/server_runtime.md",
                "markdown": "# Server Runtime\nStatus: draft\nAuthority: [Engineering](../engineering_plan.md)\n\nSee [Legal](../legal_plan.md)."
            }
        ]
    }));

    assert_eq!(value["repo_name"], json!("ait-server"));
    assert_eq!(value["interactive"], json!(true));
    assert_eq!(value["authority_summary"]["actor_count"], json!(1));
    assert_eq!(value["authority_summary"]["role_count"], json!(1));
    assert_eq!(value["authority_summary"]["permission_count"], json!(1));
    assert_eq!(value["summary"]["center_node_count"], json!(1));
    assert_eq!(value["summary"]["layer2_count"], json!(2));

    let layer1_related = value["layer1"]["related_documents"]
        .as_array()
        .expect("layer1 related documents should be visible");
    assert_eq!(layer1_related[0]["path"], json!("docs/engineering_plan.md"));

    let center = &value["center_nodes"][0];
    assert_eq!(center["path"], json!("docs/milestone.md"));
    assert_eq!(center["node_role"], json!("milestone"));
    assert_eq!(
        center["display_parent_path"],
        json!("docs/engineering_plan.md")
    );

    let layer2 = value["layer2"]
        .as_array()
        .expect("layer2 should be an array");
    let engineering = find_by_path(layer2, "docs/engineering_plan.md");
    let children = engineering["children"]
        .as_array()
        .expect("engineering children should be visible");
    assert_eq!(children[0]["path"], json!("docs/rust/server_runtime.md"));
    assert_eq!(
        children[0]["display_parent_path"],
        json!("docs/engineering_plan.md")
    );
    assert_eq!(children[0]["status"], json!("draft"));
    assert_eq!(
        children[0]["related_documents"][1]["path"],
        json!("docs/legal_plan.md")
    );
}

#[test]
fn authority_map_projects_persisted_authority_nodes_and_linked_documents() {
    let value = authority_map(json!({
        "repo_name": "ait-server",
        "documents": [
            {"path": "docs/plan.md", "markdown": "# Root Plan"},
            {"path": "docs/engineering_plan.md", "markdown": "# Engineering"},
            {"path": "docs/legal_plan.md", "markdown": "# Legal"},
            {"path": "docs/runtime.md", "markdown": "# Runtime"},
            {"path": "docs/security_review.md", "markdown": "# Security"},
            {"path": "docs/orphan.md", "markdown": "# Detached"}
        ],
        "authority_nodes": [
            {
                "authority_node_id": "N-1",
                "node_kind": "layer1",
                "document_path": "docs/plan.md",
                "sort_index": 1
            },
            {
                "authority_node_id": "N-2",
                "node_kind": "layer2",
                "document_path": "docs/engineering_plan.md",
                "sort_index": 1
            },
            {
                "authority_node_id": "N-3",
                "node_kind": "layer2",
                "document_path": "docs/legal_plan.md",
                "sort_index": 2
            },
            {
                "authority_node_id": "N-4",
                "node_kind": "milestone",
                "document_path": "docs/milestone.md",
                "title": "Milestone Index",
                "sort_index": 3
            },
            {
                "authority_node_id": "N-5",
                "node_kind": "layer3",
                "parent_node_id": "N-2",
                "document_path": "docs/runtime.md",
                "sort_index": 1
            },
            {
                "authority_node_id": "N-6",
                "node_kind": "layer3",
                "parent_node_id": "N-3",
                "document_path": "docs/security_review.md",
                "sort_index": 2
            },
            {
                "authority_node_id": "N-7",
                "node_kind": "layer3",
                "parent_node_id": "N-1",
                "document_path": "docs/orphan.md",
                "sort_index": 3
            }
        ]
    }));

    assert_eq!(value["layer1"]["path"], json!("docs/plan.md"));
    assert_eq!(value["summary"]["center_node_count"], json!(1));
    assert_eq!(value["summary"]["layer2_count"], json!(2));
    assert_eq!(value["summary"]["layer3_count"], json!(4));

    let layer2 = value["layer2"]
        .as_array()
        .expect("layer2 should be an array");
    assert_eq!(layer2[0]["path"], json!("docs/engineering_plan.md"));
    assert_eq!(layer2[0]["children"][0]["path"], json!("docs/runtime.md"));
    assert_eq!(layer2[1]["path"], json!("docs/legal_plan.md"));
    assert_eq!(
        layer2[1]["children"][0]["path"],
        json!("docs/security_review.md")
    );

    assert_eq!(
        value["linked_documents"][0]["path"],
        json!("docs/orphan.md")
    );
    assert_eq!(
        value["linked_documents"][0]["display_parent_path"],
        json!("docs/plan.md")
    );
}

#[test]
fn reviewer_inbox_filters_and_projects_current_review_payload() {
    let value = reviewer_inbox(json!({
        "repo_name": "ait",
        "author_class": "ai_related",
        "tests": "pass",
        "policy": "pass",
        "freshness": "fresh",
        "review": "requested",
        "tasks": [{
            "repo_name": "ait",
            "task_id": "T-1",
            "title": "Port reviewer inbox",
            "intent": "Move projection to Rust.",
            "status": "active"
        }],
        "changes": [
            {
                "repo_name": "ait",
                "change_id": "C-1",
                "task_id": "T-1",
                "title": "Reviewer inbox",
                "base_line": "main",
                "status": "review",
                "current_patchset_id": "P-2",
                "selected_patchset_number": 1,
                "updated_at": "2026-07-08T03:00:00Z"
            },
            {
                "repo_name": "ait",
                "change_id": "C-2",
                "task_id": "T-1",
                "title": "Human-only change",
                "base_line": "main",
                "status": "review",
                "current_patchset_id": "P-3",
                "updated_at": "2026-07-08T04:00:00Z"
            }
        ],
        "patchsets": [
            {
                "change_id": "C-1",
                "patchset_id": "P-1",
                "patchset_number": 1,
                "base_snapshot_id": "SBASE-1",
                "summary": "first"
            },
            {
                "change_id": "C-1",
                "patchset_id": "P-2",
                "patchset_number": 2,
                "base_snapshot_id": "SBASE-2",
                "summary": "second"
            },
            {
                "change_id": "C-2",
                "patchset_id": "P-3",
                "patchset_number": 1,
                "base_snapshot_id": "SBASE-2",
                "summary": "human"
            }
        ],
        "reviews": [{
            "change_id": "C-1",
            "patchset_id": "P-2",
            "action": "approve",
            "comment": "ready",
            "blocking": false
        }],
        "review_requests": [{
            "change_id": "C-1",
            "patchset_id": "P-2",
            "reviewer_group": "core"
        }],
        "attestations": [
            {
                "patchset_id": "P-2",
                "author_mode": "ai_with_human_review",
                "evaluation_summary_json": "{\"tests\":\"pass\",\"lint\":\"pass\"}",
                "provenance_summary_json": "{\"model_name\":\"GPT-5 Codex\",\"evidence_readiness\":\"complete\"}",
                "updated_at": "2026-07-08T03:10:00Z"
            },
            {
                "patchset_id": "P-3",
                "author_mode": "human_only",
                "evaluation_summary_json": "{\"tests\":\"pass\"}",
                "provenance_summary_json": "{}",
                "updated_at": "2026-07-08T04:10:00Z"
            }
        ],
        "policy_decisions": [
            {
                "policy_decision_id": 10,
                "patchset_id": "P-2",
                "decision": "pass",
                "checks_json": "[{\"name\":\"tests\",\"status\":\"pass\"},{\"name\":\"license\",\"status\":\"pass\"}]",
                "effective_requirements_json": "{\"require_tests\":true}"
            },
            {
                "policy_decision_id": 11,
                "patchset_id": "P-3",
                "decision": "pass",
                "checks_json": "[]",
                "effective_requirements_json": "{\"require_tests\":true}"
            }
        ],
        "refs": [{
            "repo_name": "ait",
            "line_name": "main",
            "head_snapshot_id": "SBASE-2"
        }],
        "land_requests": [{
            "submission_id": "L-1",
            "change_id": "C-1",
            "patchset_id": "P-2",
            "target_line": "main",
            "status": "blocked",
            "result_json": "{\"code\":\"needs_rebase\",\"message\":\"rerun land\"}",
            "created_at": "2026-07-08T03:20:00Z",
            "updated_at": "2026-07-08T03:21:00Z"
        }]
    }));

    assert_eq!(value["count"], json!(1));
    assert_eq!(value["filters"]["author_class"], json!("ai_related"));
    let item = &value["items"][0];
    assert_eq!(item["change_id"], json!("C-1"));
    assert_eq!(item["task"]["task_id"], json!("T-1"));
    assert_eq!(item["current_patchset"]["patchset_id"], json!("P-2"));
    assert_eq!(item["current_patchset"]["patchset_number"], json!(2));
    assert_eq!(item["selected_patchset"]["patchset_id"], json!("P-1"));
    assert_eq!(item["patchsets"].as_array().expect("patchsets").len(), 2);
    assert_eq!(item["review_state"]["approvals"], json!(1));
    assert_eq!(item["requested_groups"], json!(["core"]));
    assert_eq!(item["policy_state"]["decision"], json!("pass"));
    assert_eq!(item["policy_state"]["missing_requirements"], json!([]));
    assert_eq!(item["freshness"]["state"], json!("fresh"));
    assert_eq!(item["attestation"]["tests"], json!("pass"));
    assert_eq!(item["attestation"]["model_name"], json!("GPT-5 Codex"));
    assert_eq!(item["attestation"]["evidence_readiness"], json!("complete"));
    assert_eq!(
        item["landing_summary"]["blocker_class"],
        json!("needs_rebase")
    );
    assert_eq!(
        item["landing_summary"]["suggested_action"],
        json!("rerun land")
    );
}

#[test]
fn reviewer_inbox_handles_missing_optional_rows_and_rejects_bad_payloads() {
    let error = ReviewerInboxInput::from_value(&json!([])).expect_err("payload must be object");
    assert_eq!(
        error,
        "reviewer inbox read-model payload must be a JSON object."
    );

    let value = reviewer_inbox(json!({
        "changes": [{
            "repo_name": "ait",
            "change_id": "C-missing",
            "title": "Missing patchset",
            "base_line": "main",
            "status": "review",
            "updated_at": "2026-07-08T00:00:00Z"
        }]
    }));

    assert_eq!(value["count"], json!(1));
    let item = &value["items"][0];
    assert_eq!(item["current_patchset"]["patchset_id"], JsonValue::Null);
    assert_eq!(item["current_patchset"]["patchset_number"], json!(0));
    assert_eq!(item["policy_state"]["decision"], json!("pending"));
    assert_eq!(item["freshness"]["state"], json!("stale"));
    assert_eq!(item["attestation"]["completeness"], json!("missing"));
}
