use ait_server_core::foundation::native_route_host_wiring::{
    native_route_host_wiring_contract, route_shell_dispositions, route_shell_reference_modules,
    NATIVE_ROUTE_HOST_WIRING_CONTRACT_VERSION, RUST_NATIVE_ROUTE_HOST_MODULE,
};
use serde_json::json;

#[test]
fn native_route_host_wiring_has_no_python_route_shells() {
    let contract = native_route_host_wiring_contract();

    assert_eq!(
        contract["contract"],
        json!(NATIVE_ROUTE_HOST_WIRING_CONTRACT_VERSION)
    );
    assert!(route_shell_reference_modules().is_empty());
    assert!(route_shell_dispositions().is_empty());
    assert_eq!(contract["reference_modules"], json!([]));
    assert_eq!(contract["dispositions"], json!([]));
    assert_eq!(
        contract["rust_authority_modules"],
        json!([RUST_NATIVE_ROUTE_HOST_MODULE])
    );
}

#[test]
fn native_route_host_wiring_keeps_native_contract_endpoints() {
    let contract = native_route_host_wiring_contract();
    let endpoints = contract["native_contract_endpoints"]
        .as_array()
        .expect("native endpoints");

    assert!(endpoints
        .iter()
        .any(|path| path == "/v1/contracts/native-route-payloads"));
    assert!(!endpoints
        .iter()
        .any(|path| path == "/v1/contracts/session-transport-payloads"));
    assert_eq!(
        contract["compatibility_notes"]["task_dag"],
        json!("Task DAG is retired; no route-host compatibility surface remains.")
    );
}
