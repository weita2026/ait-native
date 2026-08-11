use serde_json::{json, Value as JsonValue};

pub const NATIVE_ROUTE_PAYLOAD_CONTRACT_VERSION: &str = "ait.server.native_route_payloads.v1";
pub const RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE: &str =
    "rust/crates/ait-server-core/src/foundation/route_payload_contract.rs";

const AUTHOR_MODE_VALUES: &[&str] = &[
    "human_only",
    "human_with_ai_assist",
    "ai_with_human_review",
    "ai_only_experimental",
];

#[derive(Clone, Copy)]
enum DefaultSpec {
    Required,
    Null,
    String(&'static str),
    Bool(bool),
    EmptyObject,
    EmptyArray,
}

#[derive(Clone, Copy)]
struct FieldSpec {
    name: &'static str,
    kind: &'static str,
    default: DefaultSpec,
}

impl FieldSpec {
    const fn required(name: &'static str, kind: &'static str) -> Self {
        Self {
            name,
            kind,
            default: DefaultSpec::Required,
        }
    }

    const fn null(name: &'static str, kind: &'static str) -> Self {
        Self {
            name,
            kind,
            default: DefaultSpec::Null,
        }
    }

    const fn string(name: &'static str, kind: &'static str, value: &'static str) -> Self {
        Self {
            name,
            kind,
            default: DefaultSpec::String(value),
        }
    }

    const fn bool(name: &'static str, value: bool) -> Self {
        Self {
            name,
            kind: "bool",
            default: DefaultSpec::Bool(value),
        }
    }

    const fn empty_object(name: &'static str) -> Self {
        Self {
            name,
            kind: "dict<string,any>",
            default: DefaultSpec::EmptyObject,
        }
    }

    const fn empty_array(name: &'static str, kind: &'static str) -> Self {
        Self {
            name,
            kind,
            default: DefaultSpec::EmptyArray,
        }
    }
}

#[derive(Clone, Copy)]
struct ModelSpec {
    model: &'static str,
    source_module: &'static str,
    role: &'static str,
    fields: &'static [FieldSpec],
}

pub struct NativeRoutePayloadJson;

impl NativeRoutePayloadJson {
    pub fn stateless() -> Self {
        Self
    }

    pub fn contract_json(&self) -> JsonValue {
        json!({
            "contract": NATIVE_ROUTE_PAYLOAD_CONTRACT_VERSION,
            "reference_modules": [],
            "rust_authority_modules": [
                RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
            ],
            "compatibility_notes": {
                "route_shells": "Python FastAPI route wrappers are deleted; native Rust handlers own the route host.",
                "route_request_models": "Request and payload DTO authority is Rust-owned with no Python compatibility source.",
                "planning_routes": "Planning DTO authority is Rust-owned with no Python route shell.",
                "task_dag": "Task DAG is retired; no route payload helper remains.",
            },
            "models": model_specs()
                .iter()
                .map(model_spec_json)
                .collect::<Vec<_>>(),
        })
    }

    pub fn model_names(&self) -> Vec<String> {
        model_specs()
            .iter()
            .map(|spec| spec.model.to_string())
            .collect()
    }
}

pub fn native_route_payload_contract() -> JsonValue {
    NativeRoutePayloadJson::stateless().contract_json()
}

pub fn native_route_payload_model_names() -> Vec<String> {
    NativeRoutePayloadJson::stateless().model_names()
}

pub fn native_route_payload_contract_version() -> &'static str {
    NATIVE_ROUTE_PAYLOAD_CONTRACT_VERSION
}

fn model_spec_json(spec: &ModelSpec) -> JsonValue {
    json!({
        "model": spec.model,
        "source_module": spec.source_module,
        "role": spec.role,
        "fields": spec.fields.iter().map(field_spec_json).collect::<Vec<_>>(),
    })
}

fn field_spec_json(field: &FieldSpec) -> JsonValue {
    let mut out = serde_json::Map::new();
    out.insert("name".to_string(), json!(field.name));
    out.insert("type".to_string(), json!(field.kind));
    out.insert(
        "required".to_string(),
        json!(matches!(field.default, DefaultSpec::Required)),
    );
    out.insert("default".to_string(), default_spec_json(field.default));
    if field.kind == "author_mode" {
        out.insert("enum_values".to_string(), json!(AUTHOR_MODE_VALUES));
    }
    JsonValue::Object(out)
}

fn default_spec_json(default: DefaultSpec) -> JsonValue {
    match default {
        DefaultSpec::Required => json!({"kind": "required"}),
        DefaultSpec::Null => json!({"kind": "literal", "value": null}),
        DefaultSpec::String(value) => json!({"kind": "literal", "value": value}),
        DefaultSpec::Bool(value) => json!({"kind": "literal", "value": value}),
        DefaultSpec::EmptyObject => json!({"kind": "factory", "factory": "dict", "value": {}}),
        DefaultSpec::EmptyArray => json!({"kind": "factory", "factory": "list", "value": []}),
    }
}

const REPOSITORY_CREATE_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("repo_name", "str"),
    FieldSpec::string("default_line", "str", "main"),
    FieldSpec::empty_object("policy"),
    FieldSpec::null("id_namespace_prefix", "str|null"),
];
const LINE_UPDATE_FIELDS: &[FieldSpec] = &[
    FieldSpec::null("head_snapshot_id", "str|null"),
    FieldSpec::null("expected_head_snapshot_id", "str|null"),
];
const LINE_CLOSE_REQUEST_FIELDS: &[FieldSpec] = &[FieldSpec::string("status", "str", "archived")];
const SNAPSHOT_EXISTS_REQUEST_FIELDS: &[FieldSpec] =
    &[FieldSpec::empty_array("snapshot_ids", "list<str>")];
const TASK_CREATE_FIELDS: &[FieldSpec] = &[
    FieldSpec::null("task_id", "str|null"),
    FieldSpec::required("title", "str"),
    FieldSpec::required("intent", "str"),
    FieldSpec::null("plan_id", "str|null"),
    FieldSpec::null("origin_plan_revision_id", "str|null"),
    FieldSpec::null("plan_item_ref", "str|null"),
];
const TASK_CLOSE_REQUEST_FIELDS: &[FieldSpec] = &[FieldSpec::string("status", "str", "completed")];
const CHANGE_CREATE_FIELDS: &[FieldSpec] = &[
    FieldSpec::null("change_id", "str|null"),
    FieldSpec::required("task_id", "str"),
    FieldSpec::required("title", "str"),
    FieldSpec::required("base_line", "str"),
    FieldSpec::null("fork_snapshot_id", "str|null"),
    FieldSpec::null("forked_from_line", "str|null"),
];
const CHANGE_CLOSE_REQUEST_FIELDS: &[FieldSpec] = &[FieldSpec::string("status", "str", "archived")];
const PATCHSET_PUBLISH_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("base_snapshot_id", "str"),
    FieldSpec::required("revision_snapshot_id", "str"),
    FieldSpec::required("summary", "str"),
    FieldSpec::string("author_mode", "author_mode", "ai_with_human_review"),
];
const RELEASE_ARTIFACT_UPLOAD_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("kind", "str"),
    FieldSpec::required("path", "str"),
    FieldSpec::required("sha256", "str"),
    FieldSpec::null("size_bytes", "int|null"),
    FieldSpec::required("content_entry_name", "str"),
    FieldSpec::required("content_pack", "dict<string,any>"),
];
const RELEASE_PUBLISH_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("release_id", "str"),
    FieldSpec::required("version", "str"),
    FieldSpec::required("line", "str"),
    FieldSpec::required("snapshot_id", "str"),
    FieldSpec::required("manifest_hash", "str"),
    FieldSpec::required("profile", "str"),
    FieldSpec::empty_object("package"),
    FieldSpec::empty_array("checks", "list<dict<string,any>>"),
    FieldSpec::empty_array("artifacts", "list<ReleaseArtifactUpload>"),
    FieldSpec::empty_object("formula"),
    FieldSpec::empty_object("metadata"),
];
const SELECT_PATCHSET_REQUEST_FIELDS: &[FieldSpec] = &[FieldSpec::required("patchset_id", "str")];
const RUN_PATCHSET_CI_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::string("trigger", "str", "manual_rerun"),
    FieldSpec::null("execution_profile", "str|null"),
];
const RUN_REPO_CI_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::empty_array("suite_ids", "list<str>"),
    FieldSpec::null("plane", "str|null"),
    FieldSpec::string("target_line", "str", "main"),
    FieldSpec::string("trigger", "str", "manual_rerun"),
    FieldSpec::null("selector", "str|null"),
    FieldSpec::empty_array("task_ids", "list<str>"),
    FieldSpec::null("curated_corpus", "str|null"),
    FieldSpec::null("count", "int|null"),
    FieldSpec::null("window_days", "int|null"),
    FieldSpec::empty_array("dependency_evidence", "list<str>"),
    FieldSpec::empty_array("compliance_evidence", "list<str>"),
];
const REQUEST_REVIEW_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("patchset_id", "str"),
    FieldSpec::empty_array("reviewer_groups", "list<str>"),
    FieldSpec::null("note", "str|null"),
];
const RECORD_REVIEW_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("patchset_id", "str"),
    FieldSpec::required("reviewer", "str"),
    FieldSpec::required("action", "str"),
    FieldSpec::null("comment", "str|null"),
    FieldSpec::bool("blocking", false),
];
const UPSERT_ATTESTATION_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::string("author_mode", "author_mode", "ai_with_human_review"),
    FieldSpec::required("evaluation_summary", "dict<string,any>"),
    FieldSpec::empty_object("provenance_summary"),
    FieldSpec::empty_object("detail"),
];
const CREATE_WAIVER_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("rule_name", "str"),
    FieldSpec::required("reason", "str"),
    FieldSpec::null("expires_at", "str|null"),
];
const SUBMIT_LAND_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::null("patchset_id", "str|null"),
    FieldSpec::string("target_line", "str", "main"),
    FieldSpec::string("mode", "str", "direct"),
];
const RETRY_LAND_REQUEST_FIELDS: &[FieldSpec] = &[FieldSpec::null("reason", "str|null")];
const RECONCILE_REQUEST_FIELDS: &[FieldSpec] = &[FieldSpec::bool("repair", false)];
const OPTIMIZE_REQUEST_FIELDS: &[FieldSpec] = &[FieldSpec::bool("repair", true)];
const PACK_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::bool("repack", false),
    FieldSpec::null("max_members", "int|null"),
];
const GC_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::bool("prune_unreferenced", true),
    FieldSpec::bool("prune_orphan_packs", true),
];
const ROLE_BINDING_GRANT_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("actor_identity", "str"),
    FieldSpec::empty_array("roles", "list<str>"),
];

const PLAN_CREATE_FIELDS: &[FieldSpec] = &[
    FieldSpec::null("plan_id", "str|null"),
    FieldSpec::required("title", "str"),
    FieldSpec::required("artifact_path", "str"),
    FieldSpec::null("artifact_selector", "str|null"),
    FieldSpec::required("artifact_heading", "str"),
    FieldSpec::empty_array("items", "list<dict<string,any>>"),
    FieldSpec::null("summary", "str|null"),
    FieldSpec::string("status", "str", "draft"),
    FieldSpec::string("source_kind", "str", "manual_edit"),
    FieldSpec::null("artifact_body", "str|null"),
    FieldSpec::null("packed_artifact", "dict<string,any>|null"),
];
const PLAN_REVISION_CREATE_FIELDS: &[FieldSpec] = &[
    FieldSpec::null("title", "str|null"),
    FieldSpec::required("artifact_path", "str"),
    FieldSpec::null("artifact_selector", "str|null"),
    FieldSpec::required("artifact_heading", "str"),
    FieldSpec::empty_array("items", "list<dict<string,any>>"),
    FieldSpec::null("summary", "str|null"),
    FieldSpec::string("source_kind", "str", "manual_edit"),
    FieldSpec::null("artifact_body", "str|null"),
    FieldSpec::null("packed_artifact", "dict<string,any>|null"),
    FieldSpec::null("expected_head_revision_id", "str|null"),
];
const PLAN_REVISION_ARTIFACT_PUT_ITEM_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("artifact_path", "str"),
    FieldSpec::string("role", "str", "supporting_artifact"),
    FieldSpec::string("media_type", "str", "application/octet-stream"),
    FieldSpec::string("encoding", "str|null", "utf-8"),
    FieldSpec::required("body", "str"),
    FieldSpec::empty_object("metadata"),
];
const PLAN_REVISION_ARTIFACTS_PUT_FIELDS: &[FieldSpec] = &[FieldSpec::empty_array(
    "artifacts",
    "list<PlanRevisionArtifactPutItem>",
)];
const PLAN_UPDATE_FIELDS: &[FieldSpec] = &[FieldSpec::required("status", "str")];
const PLANNING_SESSION_CREATE_FIELDS: &[FieldSpec] = &[
    FieldSpec::null("planning_session_id", "str|null"),
    FieldSpec::null("title", "str|null"),
    FieldSpec::string("mode", "str", "connected_local"),
    FieldSpec::null("preferred_agent", "str|null"),
    FieldSpec::bool("resume_if_active", true),
];
const PLANNING_SESSION_EVENT_APPEND_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("event_type", "str"),
    FieldSpec::empty_object("payload"),
];
const PLANNING_SESSION_PROMOTE_FIELDS: &[FieldSpec] = &[
    FieldSpec::required("artifact_path", "str"),
    FieldSpec::required("artifact_selector", "str"),
    FieldSpec::required("artifact_heading", "str"),
    FieldSpec::empty_array("items", "list<dict<string,any>>"),
    FieldSpec::null("title", "str|null"),
    FieldSpec::null("summary", "str|null"),
    FieldSpec::null("artifact_body", "str|null"),
];
const PLANNING_SESSION_CLOSE_REQUEST_FIELDS: &[FieldSpec] =
    &[FieldSpec::string("status", "str", "closed")];
const PLANNING_SESSION_JOIN_REQUEST_FIELDS: &[FieldSpec] = &[
    FieldSpec::string("surface", "str", "cli"),
    FieldSpec::null("title", "str|null"),
    FieldSpec::null("model_name", "str|null"),
    FieldSpec::bool("resume_if_active", true),
];

const MODEL_SPECS: &[ModelSpec] = &[
    ModelSpec {
        model: "RepositoryCreate",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: REPOSITORY_CREATE_FIELDS,
    },
    ModelSpec {
        model: "LineUpdate",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: LINE_UPDATE_FIELDS,
    },
    ModelSpec {
        model: "LineCloseRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: LINE_CLOSE_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "SnapshotExistsRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: SNAPSHOT_EXISTS_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "TaskCreate",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: TASK_CREATE_FIELDS,
    },
    ModelSpec {
        model: "TaskCloseRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: TASK_CLOSE_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "ChangeCreate",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: CHANGE_CREATE_FIELDS,
    },
    ModelSpec {
        model: "ChangeCloseRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: CHANGE_CLOSE_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "PatchsetPublish",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PATCHSET_PUBLISH_FIELDS,
    },
    ModelSpec {
        model: "ReleaseArtifactUpload",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "nested_request_model",
        fields: RELEASE_ARTIFACT_UPLOAD_FIELDS,
    },
    ModelSpec {
        model: "ReleasePublishRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: RELEASE_PUBLISH_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "SelectPatchsetRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: SELECT_PATCHSET_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "RunPatchsetCiRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: RUN_PATCHSET_CI_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "RunRepoCiRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: RUN_REPO_CI_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "RequestReviewRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: REQUEST_REVIEW_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "RecordReviewRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: RECORD_REVIEW_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "UpsertAttestationRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: UPSERT_ATTESTATION_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "CreateWaiverRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: CREATE_WAIVER_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "SubmitLandRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: SUBMIT_LAND_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "RetryLandRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: RETRY_LAND_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "ReconcileRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: RECONCILE_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "OptimizeRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: OPTIMIZE_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "PackRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PACK_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "GcRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: GC_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "RoleBindingGrant",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: ROLE_BINDING_GRANT_FIELDS,
    },
    ModelSpec {
        model: "PlanCreate",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PLAN_CREATE_FIELDS,
    },
    ModelSpec {
        model: "PlanRevisionCreate",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PLAN_REVISION_CREATE_FIELDS,
    },
    ModelSpec {
        model: "PlanRevisionArtifactPutItem",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "nested_request_model",
        fields: PLAN_REVISION_ARTIFACT_PUT_ITEM_FIELDS,
    },
    ModelSpec {
        model: "PlanRevisionArtifactsPut",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PLAN_REVISION_ARTIFACTS_PUT_FIELDS,
    },
    ModelSpec {
        model: "PlanUpdate",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PLAN_UPDATE_FIELDS,
    },
    ModelSpec {
        model: "PlanningSessionCreate",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PLANNING_SESSION_CREATE_FIELDS,
    },
    ModelSpec {
        model: "PlanningSessionEventAppend",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PLANNING_SESSION_EVENT_APPEND_FIELDS,
    },
    ModelSpec {
        model: "PlanningSessionPromote",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PLANNING_SESSION_PROMOTE_FIELDS,
    },
    ModelSpec {
        model: "PlanningSessionCloseRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PLANNING_SESSION_CLOSE_REQUEST_FIELDS,
    },
    ModelSpec {
        model: "PlanningSessionJoinRequest",
        source_module: RUST_ROUTE_PAYLOAD_AUTHORITY_MODULE,
        role: "request_model",
        fields: PLANNING_SESSION_JOIN_REQUEST_FIELDS,
    },
];

fn model_specs() -> &'static [ModelSpec] {
    MODEL_SPECS
}
