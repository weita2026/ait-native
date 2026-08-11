use super::*;

pub static AUTHORITY_MAP_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "documents",
        required: false,
        description:
            "Pre-read markdown authority documents with path, metadata, and related paths.",
    },
    ReadModelRowSetSpec {
        field: "authority_nodes",
        required: false,
        description: "Optional persisted authority graph nodes.",
    },
    ReadModelRowSetSpec {
        field: "actors",
        required: false,
        description: "Optional actor rows counted for authority summary only.",
    },
    ReadModelRowSetSpec {
        field: "roles",
        required: false,
        description: "Optional role rows counted for authority summary only.",
    },
    ReadModelRowSetSpec {
        field: "permissions",
        required: false,
        description: "Optional permission rows counted for authority summary only.",
    },
];

pub static AUTHORITY_MAP_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "authority_map",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "authority map read-model",
    public_surface: "middle.secondary_read_model.authority_map",
    output_shape: "repo_name, interactive, layer1, center_nodes, layer2, linked_documents, summary",
    mutates_state: false,
    row_sets: AUTHORITY_MAP_ROW_SETS,
};

pub static REVIEWER_INBOX_ROW_SETS: &[ReadModelRowSetSpec] = &[
    ReadModelRowSetSpec {
        field: "changes",
        required: false,
        description: "Reviewable change rows.",
    },
    ReadModelRowSetSpec {
        field: "tasks",
        required: false,
        description: "Task rows linked from reviewable changes.",
    },
    ReadModelRowSetSpec {
        field: "patchsets",
        required: false,
        description: "Patchset rows used for current, selected, and patchset list projection.",
    },
    ReadModelRowSetSpec {
        field: "reviews",
        required: false,
        description: "Review rows used for approval, blocking, and comment states.",
    },
    ReadModelRowSetSpec {
        field: "review_requests",
        required: false,
        description: "Review request rows used for requested reviewer group filtering.",
    },
    ReadModelRowSetSpec {
        field: "attestations",
        required: false,
        description: "Patchset attestation rows used for author/tests/evidence readiness fields.",
    },
    ReadModelRowSetSpec {
        field: "policy_decisions",
        required: false,
        description: "Policy rows used for policy filters and missing requirement summaries.",
    },
    ReadModelRowSetSpec {
        field: "refs",
        required: false,
        description: "Line refs used for base freshness.",
    },
    ReadModelRowSetSpec {
        field: "land_requests",
        required: false,
        description: "Land request rows used for latest landing summary.",
    },
];

pub static REVIEWER_INBOX_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "reviewer_inbox",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "reviewer inbox read-model",
    public_surface: "middle.secondary_read_model.reviewer_inbox",
    output_shape: "items, count, filters",
    mutates_state: false,
    row_sets: REVIEWER_INBOX_ROW_SETS,
};

pub fn authority_map_read_model_contract() -> &'static ReadModelContract {
    &AUTHORITY_MAP_READ_MODEL_CONTRACT
}

pub fn reviewer_inbox_read_model_contract() -> &'static ReadModelContract {
    &REVIEWER_INBOX_READ_MODEL_CONTRACT
}
