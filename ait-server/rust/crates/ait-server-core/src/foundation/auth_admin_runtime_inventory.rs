use serde_json::{json, Value as JsonValue};

pub const AUTH_ADMIN_RUNTIME_INVENTORY_CONTRACT_VERSION: &str =
    "ait.server.auth_admin_runtime_inventory.v1";
pub const RUNTIME_ENTRYPOINT_REFERENCE_MODULE: &str = "../ait/src/ait/server_entrypoint.py";

pub fn auth_admin_runtime_inventory_contract() -> JsonValue {
    json!({
        "contract": AUTH_ADMIN_RUNTIME_INVENTORY_CONTRACT_VERSION,
        "reference_modules": auth_admin_runtime_reference_modules(),
        "dispositions": auth_admin_runtime_dispositions(),
        "native_contract_endpoints": [
            "/v1/contracts/auth-admin-runtime",
            "/v1/contracts/admin-cache",
            "/v1/admin/cache/:operation",
            "/v1/contracts/server-auth-policy",
            "/v1/auth/policy/:operation/evaluate",
            "/v1/contracts/community-ids",
            "/v1/community/ids/:operation",
            "/v1/contracts/community-auth",
            "/v1/community/auth/:operation",
            "/v1/contracts/shared-runtime-policy",
            "/v1/runtime/shared-policy/:operation",
            "/v1/contracts/live-turns",
            "/v1/runtime/live-turns/:operation",
            "/v1/contracts/server-context",
            "/v1/runtime/server-context/:operation",
            "/v1/scheduler/async-jobs/:job_type/shape",
            "/v1/scheduler/admit-async-jobs",
            "/v1/scheduler/status",
            "/v1/worker/status",
            "/v1/worker/process-one",
            "/v1/operator/read-model/readiness",
            "/v1/operator/read-model/metrics",
            "/v1/runtime/read-model/metrics",
            "/v1/native/admin/jobs/:job_id",
            "/v1/native/admin/repositories/:repo_name/jobs",
            "/v1/native/workflow-backend/status",
        ],
        "boundaries": {
            "python_boundary": "Only ../ait/src/ait/server_entrypoint.py remains as one-shot launcher glue; it execs the Rust ait-server binary and owns no server behavior.",
            "auth_security": "Authentication, authorization, and community security contracts are Rust-owned server surfaces.",
            "process_lifecycle": "The Python console entrypoint resolves and execs the Rust binary once; Rust owns the process after exec.",
            "task_dag": "Task DAG is retired and is not an auth/admin/runtime surface.",
        },
    })
}

pub fn auth_admin_runtime_reference_modules() -> Vec<&'static str> {
    vec![RUNTIME_ENTRYPOINT_REFERENCE_MODULE]
}

pub fn auth_admin_runtime_dispositions() -> Vec<JsonValue> {
    vec![json!({
        "module": RUNTIME_ENTRYPOINT_REFERENCE_MODULE,
        "disposition": "rust_binary_launcher_glue",
        "rust_owner": "rust/crates/ait-server/src/main.rs",
        "representative_paths": ["/healthz", "/v1/handshake"],
        "compatibility_scope": [
            "resolve the configured Rust ait-server executable",
            "replace the Python process with the Rust binary through one exec call",
        ],
        "follow_up_required": false,
    })]
}
