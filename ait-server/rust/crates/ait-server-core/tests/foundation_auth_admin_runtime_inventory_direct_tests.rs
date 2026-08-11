use ait_server_core::foundation::auth_admin_runtime_inventory::{
    auth_admin_runtime_dispositions, auth_admin_runtime_inventory_contract,
    auth_admin_runtime_reference_modules, AUTH_ADMIN_RUNTIME_INVENTORY_CONTRACT_VERSION,
    RUNTIME_ENTRYPOINT_REFERENCE_MODULE,
};
use serde_json::json;

#[test]
fn auth_admin_runtime_inventory_keeps_only_one_shot_rust_launcher_glue() {
    let modules = auth_admin_runtime_reference_modules();
    let dispositions = auth_admin_runtime_dispositions();

    assert_eq!(modules, vec![RUNTIME_ENTRYPOINT_REFERENCE_MODULE]);
    assert_eq!(dispositions.len(), 1);
    assert_eq!(
        dispositions[0]["module"],
        RUNTIME_ENTRYPOINT_REFERENCE_MODULE
    );
    assert_eq!(
        dispositions[0]["disposition"],
        json!("rust_binary_launcher_glue")
    );
    assert_eq!(dispositions[0]["follow_up_required"], json!(false));
}

#[test]
fn auth_admin_runtime_inventory_names_rust_owned_endpoints_and_boundaries() {
    let contract = auth_admin_runtime_inventory_contract();

    assert_eq!(
        contract["contract"],
        json!(AUTH_ADMIN_RUNTIME_INVENTORY_CONTRACT_VERSION)
    );
    assert_eq!(
        contract["reference_modules"],
        json!([RUNTIME_ENTRYPOINT_REFERENCE_MODULE])
    );
    assert!(contract["native_contract_endpoints"]
        .as_array()
        .expect("native endpoints")
        .iter()
        .any(|path| path == "/v1/runtime/live-turns/:operation"));
    assert_eq!(
        contract["boundaries"]["process_lifecycle"],
        json!("The Python console entrypoint resolves and execs the Rust binary once; Rust owns the process after exec.")
    );
    assert!(contract["boundaries"]["python_boundary"]
        .as_str()
        .expect("python boundary")
        .contains("server_entrypoint.py"));
}
