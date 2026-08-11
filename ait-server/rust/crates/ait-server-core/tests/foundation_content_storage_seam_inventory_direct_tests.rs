use ait_server_core::foundation::content_storage_seam_inventory::{
    content_storage_seam_inventory_contract, seam_dispositions, seam_reference_modules,
    CONTENT_STORAGE_SEAM_INVENTORY_CONTRACT_VERSION, SERVER_API_CALLER_GLUE_MODULE,
};
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn content_storage_seam_inventory_covers_every_reference_once() {
    let modules = seam_reference_modules();
    assert_eq!(modules.len(), 1);

    let unique = modules.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), modules.len());

    let dispositions = seam_dispositions();
    assert_eq!(dispositions.len(), modules.len());

    for module in modules {
        let matches = dispositions
            .iter()
            .filter(|item| item["module"] == module)
            .count();
        assert_eq!(matches, 1, "expected exactly one disposition for {module}");
    }
}

#[test]
fn content_storage_seam_inventory_identifies_rust_owners() {
    let contract = content_storage_seam_inventory_contract();
    assert_eq!(
        contract["contract"],
        json!(CONTENT_STORAGE_SEAM_INVENTORY_CONTRACT_VERSION)
    );
    assert!(contract["native_contract_endpoints"]
        .as_array()
        .expect("native endpoints")
        .iter()
        .any(|endpoint| endpoint == "/v1/storage/:operation"));
    assert!(contract["native_contract_endpoints"]
        .as_array()
        .expect("native endpoints")
        .iter()
        .any(|endpoint| endpoint == "/v1/postgres/runtime-probe"));

    let dispositions = contract["dispositions"].as_array().expect("dispositions");
    for retired_module in [
        "../ait/src/ait_server/authority_store.py",
        "../ait/src/ait_server/local_repo_seams.py",
        "../ait/src/ait_server/repo_runtime_seams.py",
        "../ait/src/ait_server/rust_postgres_driver.py",
        "../ait/src/ait_server/server_content.py",
        "../ait/src/ait_server/server_content_groups.py",
        "../ait/src/ait_server/server_core_seam.py",
        "../ait/src/ait_server/server_db.py",
        "../ait/src/ait_server/server_protocol_seam.py",
        "../ait/src/ait_server/server_store.py",
        "../ait/src/ait_server/storage_seam.py",
        "../ait/src/ait_server/store/repo_ops.py",
    ] {
        assert!(
            dispositions
                .iter()
                .all(|item| item["module"] != retired_module),
            "retired module should not remain active: {retired_module}"
        );
    }

    let store = dispositions
        .iter()
        .find(|item| item["module"] == SERVER_API_CALLER_GLUE_MODULE)
        .expect("server API caller glue disposition");
    assert_eq!(store["disposition"], json!("rust_http_client_glue"));
}

#[test]
fn content_storage_seam_inventory_keeps_task_dag_out_of_storage_migration() {
    let dispositions = seam_dispositions();
    assert!(dispositions
        .iter()
        .all(|item| !item["module"].as_str().unwrap_or("").contains("task_dag")));

    let contract = content_storage_seam_inventory_contract();
    assert_eq!(
        contract["boundaries"]["task_dag"],
        json!("Task DAG is retired; no seam or read-model module remains.")
    );
}

#[test]
fn content_storage_seam_inventory_names_repository_and_storage_contracts() {
    let dispositions = seam_dispositions();
    let store = dispositions
        .iter()
        .find(|item| item["module"] == SERVER_API_CALLER_GLUE_MODULE)
        .expect("server API caller glue disposition");
    assert_eq!(store["disposition"], json!("rust_http_client_glue"));
    assert!(store["representative_paths"]
        .as_array()
        .expect("server store paths")
        .iter()
        .any(|path| path == "/v1/native/repositories/:repo_name/remote-sync/zstd-bulk/commit"));
    assert!(store["compatibility_scope"]
        .as_array()
        .expect("server store compatibility")
        .iter()
        .any(|note| note.as_str().unwrap_or("").contains(
            "Repository/content operations resolve through Rust NativeRepositoryService"
        )));

    let contract = content_storage_seam_inventory_contract();
    assert!(contract["native_contract_endpoints"]
        .as_array()
        .expect("native endpoints")
        .iter()
        .any(|endpoint| endpoint == "/v1/storage/:operation"));
}
