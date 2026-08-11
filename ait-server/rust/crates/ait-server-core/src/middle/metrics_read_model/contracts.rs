use super::*;

pub static RUNTIME_METRICS_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "repo_activity",
        required: false,
        description: "Optional live-turn repository activity rows.",
    },
    ReadModelRowSetSpec {
        field: "recent_completed_turns",
        required: false,
        description: "Optional recent completed live-turn rows.",
    },
    ReadModelRowSetSpec {
        field: "recent_failed_turns",
        required: false,
        description: "Optional recent failed live-turn rows.",
    },
];

pub static RUNTIME_METRICS_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "runtime_metrics",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "runtime metrics read-model",
    public_surface: "middle.metrics_read_model.runtime_metrics",
    output_shape: "live_turn_metrics, live_turn_pressure",
    mutates_state: false,
    row_sets: RUNTIME_METRICS_ROW_SETS,
};

pub static OPERATOR_METRICS_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "repositories",
        required: false,
        description: "Repository index rows.",
    },
    ReadModelRowSetSpec {
        field: "repository_storage",
        required: false,
        description: "Per-repository storage summary rows.",
    },
    ReadModelRowSetSpec {
        field: "repository_workers",
        required: false,
        description: "Per-repository worker summary rows.",
    },
    ReadModelRowSetSpec {
        field: "jobs",
        required: false,
        description: "Server worker job rows ordered newest first.",
    },
    ReadModelRowSetSpec {
        field: "job_diagnostics",
        required: false,
        description: "Server-wide job diagnostics rows.",
    },
    ReadModelRowSetSpec {
        field: "shared_runtime_policy",
        required: false,
        description: "Shared runtime policy facts used by readiness checks.",
    },
    ReadModelRowSetSpec {
        field: "rust_server_core_seam",
        required: false,
        description: "Rust server-core seam status facts used by readiness checks.",
    },
    ReadModelRowSetSpec {
        field: "postgres_schema",
        required: false,
        description: "PostgreSQL schema status facts used by readiness checks.",
    },
];

pub static OPERATOR_METRICS_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "operator_metrics",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "operator metrics read-model",
    public_surface: "middle.metrics_read_model.operator_metrics",
    output_shape: "summary, storage_metrics, worker_metrics, job_outcome_metrics, live_turn_metrics, live_turn_pressure, repositories",
    mutates_state: false,
    row_sets: OPERATOR_METRICS_ROW_SETS,
};

pub fn runtime_metrics_read_model_contract() -> &'static ReadModelContract {
    &RUNTIME_METRICS_READ_MODEL_CONTRACT
}

pub fn operator_metrics_read_model_contract() -> &'static ReadModelContract {
    &OPERATOR_METRICS_READ_MODEL_CONTRACT
}
