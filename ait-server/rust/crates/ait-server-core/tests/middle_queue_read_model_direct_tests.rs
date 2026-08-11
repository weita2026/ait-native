use ait_server_core::middle::queue_read_model::{
    queue_summary_read_model, queue_summary_read_model_contract, QueueReadModelInput,
};
use ait_server_core::middle::read_model_contract::{
    ReadModelContract, ReadModelRowSetSpec, ReadModelRows,
};
use serde_json::{json, Value as JsonValue};
use std::sync::Arc;

static REQUIRED_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "required_rows",
        required: true,
        description: "Rows that must be present in the payload.",
    },
    ReadModelRowSetSpec {
        field: "optional_rows",
        required: false,
        description: "Rows that may be omitted.",
    },
];

static REQUIRED_ROW_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "test_required_rows",
    reference_module: "../ait/src/ait_server/read_models_domains/test_required_rows.py",
    payload_label: "required-row read-model",
    public_surface: "middle.test_required_rows",
    output_shape: "test payload",
    mutates_state: false,
    row_sets: REQUIRED_ROW_SETS,
};

fn summary(payload: JsonValue) -> JsonValue {
    let input = QueueReadModelInput::from_value(&payload).expect("input should parse");
    queue_summary_read_model(&input).expect("summary should build")
}

#[test]
fn queue_input_clone_shares_large_immutable_row_collections() {
    let tasks = (0..10_000)
        .map(|index| {
            json!({
                "task_id": format!("TASK-{index}"),
                "repo_name": "ait",
                "status": "active",
            })
        })
        .collect::<Vec<_>>();
    let input = QueueReadModelInput::from_value(&json!({"tasks": tasks}))
        .expect("large queue input should parse");
    let cloned = input.clone();

    assert_eq!(cloned.tasks.len(), 10_000);
    assert!(Arc::ptr_eq(&input.tasks, &cloned.tasks));
    assert!(Arc::ptr_eq(&input.changes, &cloned.changes));
    assert!(Arc::ptr_eq(&input.patchsets, &cloned.patchsets));
}

#[test]
fn queue_summary_contract_names_rust_ownership_and_row_sets() {
    let contract = queue_summary_read_model_contract();

    assert_eq!(contract.domain_id, "task_queue");
    assert_eq!(contract.reference_module, "rust_owned_no_python_reference");
    assert_eq!(contract.public_surface, "middle.queue_read_model.summary");
    assert!(!contract.mutates_state);
    assert_eq!(contract.row_sets.len(), 9);
    assert!(contract.row_set("tasks").is_some());
    assert!(contract.row_set("ci_statuses").is_some());
    assert!(contract.row_set("unknown_rows").is_none());
}

#[test]
fn read_model_rows_preserve_counts_and_row_order() {
    let rows = ReadModelRows::from_payload(
        &json!({
            "required_rows": [
                {"id": "first"},
                {"id": "second"}
            ],
            "optional_rows": [
                {"id": "optional"}
            ]
        }),
        &REQUIRED_ROW_CONTRACT,
    )
    .expect("rows should parse");

    assert_eq!(rows.counts().get("required_rows"), Some(&2));
    assert_eq!(rows.counts().get("optional_rows"), Some(&1));
    assert_eq!(
        rows.get("required_rows")
            .expect("required rows should exist")[0]["id"],
        json!("first")
    );
    assert_eq!(
        rows.get("required_rows")
            .expect("required rows should exist")[1]["id"],
        json!("second")
    );
}

#[test]
fn read_model_rows_reject_bad_payload_and_row_shapes() {
    let not_object = ReadModelRows::from_payload(&json!([]), queue_summary_read_model_contract())
        .expect_err("payload must be an object");
    assert_eq!(
        not_object,
        "queue read-model payload must be a JSON object."
    );

    let not_array = QueueReadModelInput::from_value(&json!({"tasks": {}}))
        .expect_err("row set must be an array");
    assert_eq!(not_array, "`tasks` must be an array.");

    let not_object_row = QueueReadModelInput::from_value(&json!({"tasks": [1]}))
        .expect_err("row set entries must be objects");
    assert_eq!(not_object_row, "`tasks` rows must be JSON objects.");
}

#[test]
fn read_model_rows_enforce_required_row_sets() {
    let error = ReadModelRows::from_payload(&json!({}), &REQUIRED_ROW_CONTRACT)
        .expect_err("required row sets must be present");

    assert_eq!(error, "`required_rows` is required.");
}

#[test]
fn active_task_queue_filters_changes_to_selected_tasks_before_hydration() {
    let value = summary(json!({
        "repo_name": "ait",
        "status": "active",
        "tasks": [
            {"task_id": "RT-1", "repo_name": "ait", "status": "active", "title": "Active", "created_at": "2026-06-25T00:00:00Z"},
            {"task_id": "RT-2", "repo_name": "ait", "status": "completed", "title": "Done", "created_at": "2026-06-24T00:00:00Z"}
        ],
        "changes": [
            {"change_id": "RC-1", "task_id": "RT-1", "repo_name": "ait", "status": "draft", "title": "Active draft", "base_line": "main", "updated_at": "2026-06-25T01:00:00Z"},
            {"change_id": "RC-2", "task_id": "RT-2", "repo_name": "ait", "status": "landed", "title": "Completed landed", "base_line": "main", "updated_at": "2026-06-24T01:00:00Z"}
        ]
    }));

    assert_eq!(value["task_queue"]["count"], json!(1));
    assert_eq!(
        value["task_queue"]["items"][0]["task"]["task_id"],
        json!("RT-1")
    );
    assert_eq!(
        value["task_queue"]["items"][0]["focus_change"]["change_id"],
        json!("RC-1")
    );
    assert_eq!(value["query_plan"]["task_statuses"], json!(["active"]));
    assert_eq!(
        value["query_plan"]["queue_change_task_ids"],
        json!(["RT-1"])
    );
    assert_eq!(
        value["query_plan"]["selected_counts"],
        json!({"queue_changes": 1, "tasks": 1})
    );
}

#[test]
fn all_changes_inventory_excludes_landed_and_archived_server_side() {
    let value = summary(json!({
        "repo_name": "ait",
        "status": "active",
        "include_all_changes": true,
        "tasks": [
            {"task_id": "RT-1", "repo_name": "ait", "status": "active", "title": "Active", "created_at": "2026-06-25T00:00:00Z"}
        ],
        "changes": [
            {"change_id": "RC-1", "task_id": "RT-1", "repo_name": "ait", "status": "draft", "title": "Draft", "base_line": "main", "updated_at": "2026-06-25T01:00:00Z"},
            {"change_id": "RC-2", "task_id": "RT-1", "repo_name": "ait", "status": "landed", "title": "Landed", "base_line": "main", "updated_at": "2026-06-25T02:00:00Z"},
            {"change_id": "RC-3", "task_id": "RT-1", "repo_name": "ait", "status": "archived", "title": "Archived", "base_line": "main", "updated_at": "2026-06-25T03:00:00Z"}
        ]
    }));

    assert_eq!(value["change_inventory"]["count"], json!(1));
    assert_eq!(
        value["change_inventory"]["items"][0]["change_id"],
        json!("RC-1")
    );
    assert_eq!(
        value["change_inventory"]["filters"]["exclude_statuses"],
        json!(["archived", "landed"])
    );
}

#[test]
fn gate_summaries_rank_ready_changes_after_attestation_policy_review_and_fresh_base() {
    let value = summary(json!({
        "repo_name": "ait",
        "status": "active",
        "tasks": [
            {"task_id": "RT-1", "repo_name": "ait", "status": "active", "title": "Active", "created_at": "2026-06-25T00:00:00Z"}
        ],
        "changes": [
            {"change_id": "RC-1", "task_id": "RT-1", "repo_name": "ait", "status": "review", "title": "Review", "base_line": "main", "current_patchset_id": "RP-1", "updated_at": "2026-06-25T01:00:00Z"}
        ],
        "patchsets": [
            {"patchset_id": "RP-1", "change_id": "RC-1", "patchset_number": 1, "base_snapshot_id": "SNP-BASE"}
        ],
        "reviews": [
            {"change_id": "RC-1", "patchset_id": "RP-1", "reviewer": "alice", "action": "approve", "blocking": false}
        ],
        "attestations": [
            {"patchset_id": "RP-1", "author_mode": "ai_with_human_review", "evaluation_summary": {"tests": "pass"}}
        ],
        "policy_decisions": [
            {"patchset_id": "RP-1", "decision": "pass", "effective_requirements": {"require_tests": true}, "created_at": "2026-06-25T01:01:00Z"}
        ],
        "refs": [
            {"repo_name": "ait", "line_name": "main", "head_snapshot_id": "SNP-BASE"}
        ],
        "ci_statuses": [
            {"patchset_id": "RP-1", "tests_status": "pass"}
        ]
    }));

    assert_eq!(
        value["task_queue"]["items"][0]["workflow"]["state"],
        json!("ready_to_land")
    );
    assert_eq!(
        value["task_queue"]["items"][0]["focus_change"]["action"],
        json!("land_change")
    );
    assert_eq!(
        value["task_queue"]["items"][0]["ci_summary"]["remote_land_gate"],
        json!("pass")
    );
}

#[test]
fn task_scoped_short_change_ids_do_not_cross_contaminate_queue_state() {
    let value = summary(json!({
        "repo_name": "ait",
        "status": "active",
        "tasks": [
            {"task_id": "RT-0010", "repo_name": "ait", "status": "active", "title": "First active Task", "created_at": "2026-07-27T01:00:00Z"},
            {"task_id": "RT-0112", "repo_name": "ait", "status": "active", "title": "Second active Task", "created_at": "2026-07-27T02:00:00Z"},
            {"task_id": "RT-2554", "repo_name": "ait", "status": "completed", "title": "Completed Task", "created_at": "2026-07-27T03:00:00Z"}
        ],
        "changes": [
            {"change_id": "C-01", "change_ref": "RT-0010/C-01", "task_id": "RT-0010", "repo_name": "ait", "status": "review", "title": "First review", "base_line": "main", "updated_at": "2026-07-27T01:01:00Z"},
            {"change_id": "C-01", "change_ref": "RT-0112/C-01", "task_id": "RT-0112", "repo_name": "ait", "status": "review", "title": "Second review", "base_line": "main", "updated_at": "2026-07-27T02:01:00Z"},
            {"change_id": "C-01", "change_ref": "RT-2554/C-01", "task_id": "RT-2554", "repo_name": "ait", "status": "landed", "title": "Completed change", "base_line": "main", "updated_at": "2026-07-27T03:01:00Z"}
        ],
        "patchsets": [
            {"patchset_id": "RT-0010/C-01/P-01", "change_id": "C-01", "change_ref": "RT-0010/C-01", "patchset_number": 1, "base_snapshot_id": "SNP-FIRST"},
            {"patchset_id": "RT-0112/C-01/P-01", "change_id": "C-01", "change_ref": "RT-0112/C-01", "patchset_number": 1, "base_snapshot_id": "SNP-SECOND"},
            {"patchset_id": "RT-2554/C-01/P-06", "change_id": "C-01", "change_ref": "RT-2554/C-01", "patchset_number": 6, "base_snapshot_id": "SNP-LANDED"},
            {"patchset_id": "AMBIGUOUS/C-01/P-99", "change_id": "C-01", "patchset_number": 99, "base_snapshot_id": "SNP-AMBIGUOUS"}
        ],
        "reviews": [
            {"change_id": "C-01", "change_ref": "RT-0010/C-01", "patchset_id": "RT-0010/C-01/P-01", "reviewer": "alice", "action": "request_changes", "blocking": true},
            {"change_id": "C-01", "change_ref": "RT-0010/C-01", "patchset_id": "RT-0010/C-01/P-01", "review_id": "RT-0010/C-01/RR-02", "action": "request", "status": "requested", "reviewer_groups": ["maintainers"]},
            {"change_id": "C-01", "change_ref": "RT-0112/C-01", "patchset_id": "RT-0112/C-01/P-01", "reviewer": "bob", "action": "approve", "blocking": false},
            {"change_id": "C-01", "change_ref": "RT-2554/C-01", "patchset_id": "RT-2554/C-01/P-06", "reviewer": "carol", "action": "approve", "blocking": false}
        ],
        "review_requests": [
            {"change_id": "C-01", "change_ref": "RT-0010/C-01", "patchset_id": "RT-0010/C-01/P-01", "reviewer_group": "maintainers"}
        ],
        "policy_decisions": [
            {"patchset_id": "RT-2554/C-01/P-06", "decision": "pass", "effective_requirements": {}, "created_at": "2026-07-27T03:02:00Z"},
            {"patchset_id": "AMBIGUOUS/C-01/P-99", "decision": "pass", "effective_requirements": {}, "created_at": "2026-07-27T04:02:00Z"}
        ]
    }));

    assert_eq!(value["task_queue"]["summary"]["ready_to_land"], json!(0));
    let task_items = value["task_queue"]["items"].as_array().expect("task items");
    let first = task_items
        .iter()
        .find(|item| item["task"]["task_id"] == json!("RT-0010"))
        .expect("first Task");
    let second = task_items
        .iter()
        .find(|item| item["task"]["task_id"] == json!("RT-0112"))
        .expect("second Task");
    assert_eq!(
        first["focus_change"]["patchset_id"],
        json!("RT-0010/C-01/P-01")
    );
    assert_eq!(
        second["focus_change"]["patchset_id"],
        json!("RT-0112/C-01/P-01")
    );
    assert_eq!(first["focus_change"]["policy_decision"], json!("pending"));
    assert_eq!(second["focus_change"]["policy_decision"], json!("pending"));
    assert_eq!(first["attention"]["blocking_reviews"], json!(1));
    assert_eq!(second["attention"]["blocking_reviews"], json!(0));

    let inbox_items = value["reviewer_inbox"]["items"]
        .as_array()
        .expect("reviewer inbox items");
    let first_inbox = inbox_items
        .iter()
        .find(|item| item["task"]["task_id"] == json!("RT-0010"))
        .expect("first inbox item");
    let second_inbox = inbox_items
        .iter()
        .find(|item| item["task"]["task_id"] == json!("RT-0112"))
        .expect("second inbox item");
    assert_eq!(
        first_inbox["current_patchset"]["patchset_id"],
        json!("RT-0010/C-01/P-01")
    );
    assert_eq!(
        second_inbox["current_patchset"]["patchset_id"],
        json!("RT-0112/C-01/P-01")
    );
    assert_eq!(first_inbox["review_state"]["blocking"], json!(1));
    assert_eq!(second_inbox["review_state"]["approvals"], json!(1));
    assert_eq!(first_inbox["requested_groups"].as_array().unwrap().len(), 1);
    assert_eq!(second_inbox["requested_groups"], json!([]));
}
