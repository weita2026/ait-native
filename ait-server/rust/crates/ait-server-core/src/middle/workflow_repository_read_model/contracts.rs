use super::*;

pub static TASK_WORKFLOW_DETAIL_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "changes",
        required: false,
        description: "Change rows linked to the focused task.",
    },
    ReadModelRowSetSpec {
        field: "patchsets",
        required: false,
        description: "Patchset rows used to select current, selected, and display patchsets.",
    },
    ReadModelRowSetSpec {
        field: "reviews",
        required: false,
        description: "Review rows used for approval, blocking, and comment summaries.",
    },
    ReadModelRowSetSpec {
        field: "attestations",
        required: false,
        description: "Patchset attestation rows used for validation readiness.",
    },
    ReadModelRowSetSpec {
        field: "policy_decisions",
        required: false,
        description: "Policy decision rows used for landability and review packets.",
    },
    ReadModelRowSetSpec {
        field: "land_requests",
        required: false,
        description: "Land request rows used for latest land summary.",
    },
    ReadModelRowSetSpec {
        field: "refs",
        required: false,
        description: "Repository line refs used for base freshness checks.",
    },
    ReadModelRowSetSpec {
        field: "patchset_deltas",
        required: false,
        description: "Pre-shaped patchset delta rows, avoiding raw snapshot/blob read authority.",
    },
    ReadModelRowSetSpec {
        field: "events",
        required: false,
        description: "Workflow event rows used for task timeline projection.",
    },
];

pub static TASK_WORKFLOW_DETAIL_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "task_workflow_detail",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "task workflow detail read-model",
    public_surface: "native.read.task_detail",
    output_shape:
        "task, repository, changes, workflow_context, summary, aggregate_diff, review packets, timeline",
    mutates_state: false,
    row_sets: TASK_WORKFLOW_DETAIL_ROW_SETS,
};

pub static REPOSITORY_INDEX_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "repositories",
        required: false,
        description: "Repository rows listed in the overview index.",
    },
    ReadModelRowSetSpec {
        field: "lines",
        required: false,
        description: "Repository line rows used for per-repository line counts.",
    },
    ReadModelRowSetSpec {
        field: "groups",
        required: false,
        description: "Repository group rows used to project grouped overview sections.",
    },
];

pub static REPOSITORY_INDEX_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "repository_index",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "repository index read-model",
    public_surface: "native.read.repository_index",
    output_shape: "count, total_lines, repositories, groups, group_count, latest_activity",
    mutates_state: false,
    row_sets: REPOSITORY_INDEX_ROW_SETS,
};

pub static REPOSITORY_DETAIL_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "lines",
        required: false,
        description: "Repository line rows split into active and archived groups.",
    },
    ReadModelRowSetSpec {
        field: "line_work_contexts",
        required: false,
        description: "Pre-shaped non-default line work context rows.",
    },
    ReadModelRowSetSpec {
        field: "jobs",
        required: false,
        description: "Recent worker job rows used for repository job summaries.",
    },
    ReadModelRowSetSpec {
        field: "ci_runs",
        required: false,
        description: "Repository CI run summary rows.",
    },
];

pub static REPOSITORY_WORKER_STATUS_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "jobs",
        required: false,
        description:
            "All scoped worker job rows used for state counts and active worker projection.",
    },
    ReadModelRowSetSpec {
        field: "recent_jobs",
        required: false,
        description: "Already limited recent job rows returned unchanged for operator views.",
    },
];

pub static REPOSITORY_DETAIL_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "repository_detail",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "repository detail read-model",
    public_surface: "native.read.repository_detail",
    output_shape:
        "repository, lines, active_lines, archived_lines, line_summary, jobs, ci_runs, storage_summary, job_summary",
    mutates_state: false,
    row_sets: REPOSITORY_DETAIL_ROW_SETS,
};

pub static REPOSITORY_WORKER_STATUS_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "repository_worker_status",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "repository worker status read-model",
    public_surface: "native.read.repository_worker_status",
    output_shape:
        "repo_name, snapshot_at, state_summary, workers, worker_count, job counts, diagnostics, recent_jobs",
    mutates_state: false,
    row_sets: REPOSITORY_WORKER_STATUS_ROW_SETS,
};

pub fn task_workflow_detail_read_model_contract() -> &'static ReadModelContract {
    &TASK_WORKFLOW_DETAIL_READ_MODEL_CONTRACT
}

pub fn repository_index_read_model_contract() -> &'static ReadModelContract {
    &REPOSITORY_INDEX_READ_MODEL_CONTRACT
}

pub fn repository_detail_read_model_contract() -> &'static ReadModelContract {
    &REPOSITORY_DETAIL_READ_MODEL_CONTRACT
}

pub fn repository_worker_status_read_model_contract() -> &'static ReadModelContract {
    &REPOSITORY_WORKER_STATUS_READ_MODEL_CONTRACT
}
