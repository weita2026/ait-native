use serde_json::{json, Value as JsonValue};

pub const NATIVE_ROUTE_HOST_WIRING_CONTRACT_VERSION: &str =
    "ait.server.native_route_host_wiring.v1";
pub const RUST_NATIVE_ROUTE_HOST_MODULE: &str = "rust/crates/ait-server/src/router.rs";

pub fn native_route_host_wiring_contract() -> JsonValue {
    json!({
        "contract": NATIVE_ROUTE_HOST_WIRING_CONTRACT_VERSION,
        "reference_modules": route_shell_reference_modules(),
        "dispositions": route_shell_dispositions(),
        "rust_authority_modules": [
            RUST_NATIVE_ROUTE_HOST_MODULE,
        ],
        "native_contract_endpoints": [
            "/v1/contracts/native-route-payloads",
            "/v1/contracts/binary-body-routes",
        ],
        "compatibility_notes": {
            "python_route_shells": "All ../ait/src/ait_server Python route shells are deleted; the Rust router is authoritative.",
            "auth_and_runtime": "Auth and operator runtime behavior are owned by Rust services or their application-layer callers.",
            "task_dag": "Task DAG is retired; no route-host compatibility surface remains.",
        },
    })
}

pub fn route_shell_reference_modules() -> Vec<&'static str> {
    Vec::new()
}

pub fn route_shell_dispositions() -> Vec<JsonValue> {
    Vec::new()
}
