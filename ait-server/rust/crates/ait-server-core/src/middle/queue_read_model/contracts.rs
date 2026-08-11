use super::*;

pub static QUEUE_SUMMARY_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "tasks",
        required: false,
        description: "Server task rows selected by repository and status.",
    },
    ReadModelRowSetSpec {
        field: "changes",
        required: false,
        description: "Server change rows used for task focus and change inventory projection.",
    },
    ReadModelRowSetSpec {
        field: "patchsets",
        required: false,
        description: "Patchset rows used to select current review and validation context.",
    },
    ReadModelRowSetSpec {
        field: "reviews",
        required: false,
        description: "Review rows used for approval, blocking, and comment summaries.",
    },
    ReadModelRowSetSpec {
        field: "review_requests",
        required: false,
        description: "Review request rows surfaced with queue and reviewer inbox entries.",
    },
    ReadModelRowSetSpec {
        field: "attestations",
        required: false,
        description: "Patchset attestation rows used for validation requirement summaries.",
    },
    ReadModelRowSetSpec {
        field: "policy_decisions",
        required: false,
        description: "Policy decision rows used for workflow gate summaries.",
    },
    ReadModelRowSetSpec {
        field: "refs",
        required: false,
        description: "Line ref rows used for base freshness checks.",
    },
    ReadModelRowSetSpec {
        field: "ci_statuses",
        required: false,
        description: "Patchset CI status rows used for remote land gate summaries.",
    },
];

pub static QUEUE_SUMMARY_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "task_queue",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "queue read-model",
    public_surface: "middle.queue_read_model.summary",
    output_shape: "task_queue, reviewer_inbox, query_plan, optional change_inventory",
    mutates_state: false,
    row_sets: QUEUE_SUMMARY_ROW_SETS,
};

pub fn queue_summary_read_model_contract() -> &'static ReadModelContract {
    &QUEUE_SUMMARY_READ_MODEL_CONTRACT
}
