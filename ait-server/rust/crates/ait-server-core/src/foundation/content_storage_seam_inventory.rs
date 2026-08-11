use serde_json::{json, Value as JsonValue};

pub const CONTENT_STORAGE_SEAM_INVENTORY_CONTRACT_VERSION: &str =
    "ait.server.content_storage_seam_inventory.v1";

pub const SERVER_API_CALLER_GLUE_MODULE: &str = "../ait/src/ait_native/server_api.py";

pub fn content_storage_seam_inventory_contract() -> JsonValue {
    json!({
        "contract": CONTENT_STORAGE_SEAM_INVENTORY_CONTRACT_VERSION,
        "reference_modules": seam_reference_modules(),
        "dispositions": seam_dispositions(),
        "native_contract_endpoints": [
            "/v1/contracts/native-route-host-wiring",
            "/v1/contracts/binary-body-routes",
            "/v1/storage/:operation",
            "/v1/postgres/runtime-probe",
            "/v1/postgres/server-plane-batch",
            "/v1/native/repositories/:repo_name/snapshots/:snapshot_tail",
            "/v1/native/repositories/:repo_name/remote-sync/zstd-bulk/plan",
            "/v1/native/repositories/:repo_name/remote-sync/zstd-bulk/commit",
        ],
        "boundaries": {
            "raw_storage_api": "Do not expose raw storage or Binary DB mutation APIs through this inventory.",
            "python_fallback": "Do not reintroduce Python seam fallback once a Rust protocol or service surface exists.",
            "task_dag": "Task DAG is retired; no seam or read-model module remains.",
        },
    })
}

pub fn seam_reference_modules() -> Vec<&'static str> {
    vec![SERVER_API_CALLER_GLUE_MODULE]
}

pub fn seam_dispositions() -> Vec<JsonValue> {
    vec![
        seam(
            SERVER_API_CALLER_GLUE_MODULE,
            "rust_http_client_glue",
            "ServerWorkflowStore and NativeRepositoryService HTTP APIs",
            &[
                "/v1/native/repositories",
                "/v1/native/repositories/:repo_name",
                "/v1/native/repositories/:repo_name/lines",
                "/v1/native/repositories/:repo_name/snapshots/:snapshot_tail",
                "/v1/native/tasks/:task_action",
                "/v1/native/changes/:change_action",
                "/v1/native/patchsets/:patchset_action",
                "/v1/native/lands/:submission_id",
                "/v1/native/repositories/:repo_name/remote-sync/zstd-bulk/commit",
            ],
            &["Repository/content operations resolve through Rust NativeRepositoryService; ../ait Python is caller glue only."],
        ),
    ]
}

fn seam(
    module: &'static str,
    disposition: &'static str,
    rust_owner: &'static str,
    representative_paths: &[&'static str],
    compatibility_scope: &[&'static str],
) -> JsonValue {
    json!({
        "module": module,
        "disposition": disposition,
        "rust_owner": rust_owner,
        "representative_paths": representative_paths,
        "compatibility_scope": compatibility_scope,
    })
}
