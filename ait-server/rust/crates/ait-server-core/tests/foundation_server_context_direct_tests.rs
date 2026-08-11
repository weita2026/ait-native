use ait_server_core::foundation::server_context::{
    create_server_context, server_context_contract, server_context_from_env_map,
    server_context_json, SERVER_CONTEXT_CONTRACT_VERSION,
    SERVER_RUNTIME_PREFLIGHT_REFERENCE_MODULE,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

fn object(payload: JsonValue) -> JsonMap<String, JsonValue> {
    payload.as_object().expect("object").clone()
}

#[test]
fn server_context_contract_names_references_and_boundaries() {
    let contract = server_context_contract();
    assert_eq!(contract["contract"], json!(SERVER_CONTEXT_CONTRACT_VERSION));
    assert_eq!(
        contract["reference_modules"],
        json!([SERVER_RUNTIME_PREFLIGHT_REFERENCE_MODULE])
    );
    assert_eq!(contract["defaults"]["create_backend"], json!("postgres"));
    assert_eq!(
        contract["compatibility_notes"]["task_dag"],
        json!("Task DAG is retired and is not a server context/path surface.")
    );
    assert_eq!(
        contract["compatibility_notes"]["python_reference"],
        json!("ServerContext caller glue lives outside ait_server in ait.server_runtime_preflight; Rust owns validation and server runtime behavior.")
    );
    assert_eq!(
        contract["compatibility_notes"]["protocol_reference"],
        json!("server_protocol_seam.py was removed in ../ait LT-1940/LC-1775 after callers imported protocol helpers directly.")
    );
}

#[test]
fn server_context_create_shapes_paths_and_defaults() {
    let context = create_server_context(
        std::path::Path::new("/tmp/ait-runtime"),
        None,
        None,
        "ait_native_content",
        "ait_native_control",
        "explicit",
    )
    .expect("context");
    let payload = context.to_json();
    assert_eq!(payload["root"], json!("/tmp/ait-runtime"));
    assert!(payload.get("content_db_path").is_none());
    assert!(payload.get("control_db_path").is_none());
    assert_eq!(
        payload["manifest_dir"],
        json!("/tmp/ait-runtime/objects/manifests")
    );
    assert_eq!(payload["pack_dir"], json!("/tmp/ait-runtime/objects/packs"));
    assert_eq!(
        payload["tree_pack_dir"],
        json!("/tmp/ait-runtime/objects/tree-packs")
    );
    assert_eq!(payload["ref_root"], json!("/tmp/ait-runtime/refs"));
    assert_eq!(payload["db_backend"], json!("postgres"));
    assert_eq!(payload["using_postgres"], json!(true));
    assert_eq!(payload["root_source"], json!("explicit"));
}

#[test]
fn server_context_create_preserves_explicit_schema_values() {
    let payload = server_context_json(
        "create",
        &json!({
            "root": "/tmp/ait-runtime",
            "backend": " POSTGRES ",
            "postgres_dsn": " postgres://example ",
            "content_schema": "",
            "control_schema": "custom_control",
            "root_source": ""
        }),
    )
    .expect("create");
    assert_eq!(payload["context"]["db_backend"], json!("postgres"));
    assert_eq!(
        payload["context"]["postgres_dsn"],
        json!("postgres://example")
    );
    assert_eq!(payload["context"]["content_schema"], json!(""));
    assert_eq!(
        payload["context"]["control_schema"],
        json!("custom_control")
    );
    assert_eq!(payload["context"]["root_source"], json!(""));
}

#[test]
fn server_context_create_can_bootstrap_runtime_directories_when_explicit() {
    let root =
        std::env::temp_dir().join(format!("ait-server-context-direct-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let payload = server_context_json(
        "create",
        &json!({
            "root": root,
            "backend": "postgres",
            "ensure_directories": true,
        }),
    )
    .expect("create");

    for field in [
        "root",
        "manifest_dir",
        "pack_dir",
        "tree_pack_dir",
        "ref_root",
    ] {
        let path = std::path::PathBuf::from(
            payload["context"][field]
                .as_str()
                .expect("path field should be text"),
        );
        assert!(path.is_dir(), "{field} should be created at {path:?}");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn server_context_create_rejects_non_postgres_backends() {
    let local_file = server_context_json(
        "create",
        &json!({
            "root": "/tmp/ait-runtime",
            "backend": "local-file"
        }),
    )
    .expect_err("local file backend");
    assert_eq!(
        local_file,
        "Unsupported AIT native server database backend: 'local-file'"
    );

    let mysql = server_context_json(
        "create",
        &json!({
            "root": "/tmp/ait-runtime",
            "backend": "mysql"
        }),
    )
    .expect_err("mysql");
    assert_eq!(
        mysql,
        "Unsupported AIT native server database backend: 'mysql'"
    );
}

#[test]
fn server_context_from_env_requires_backend_and_dsn() {
    let missing_backend = server_context_from_env_map(&object(json!({
        "AIT_NATIVE_SERVER_DATA": "/tmp/ait-runtime"
    })))
    .expect_err("missing backend");
    assert_eq!(
        missing_backend,
        "AIT_NATIVE_SERVER_DB_BACKEND is required for server runtime startup; set it explicitly to 'postgres'."
    );

    let missing_dsn = server_context_from_env_map(&object(json!({
        "AIT_NATIVE_SERVER_DATA": "/tmp/ait-runtime",
        "AIT_NATIVE_SERVER_DB_BACKEND": "postgres"
    })))
    .expect_err("missing dsn");
    assert_eq!(
        missing_dsn,
        "AIT_NATIVE_SERVER_POSTGRES_DSN is required when AIT_NATIVE_SERVER_DB_BACKEND=postgres."
    );
}

#[test]
fn server_context_from_env_rejects_non_postgres_backends() {
    let local_file = server_context_from_env_map(&object(json!({
        "AIT_NATIVE_SERVER_DATA": "/tmp/ait-runtime",
        "AIT_NATIVE_SERVER_DB_BACKEND": "local-file"
    })))
    .expect_err("local file backend");
    assert_eq!(
        local_file,
        "Unsupported AIT_NATIVE_SERVER_DB_BACKEND value: 'local-file'"
    );

    let unknown = server_context_from_env_map(&object(json!({
        "AIT_NATIVE_SERVER_DATA": "/tmp/ait-runtime",
        "AIT_NATIVE_SERVER_DB_BACKEND": "mysql"
    })))
    .expect_err("unknown");
    assert_eq!(
        unknown,
        "Unsupported AIT_NATIVE_SERVER_DB_BACKEND value: 'mysql'"
    );
}

#[test]
fn server_context_from_env_uses_runtime_root_and_schema_defaults() {
    let payload = server_context_json(
        "from-env",
        &json!({
            "env": {
                "AIT_RUNTIME_DATA": "/tmp/runtime-data",
                "AIT_NATIVE_SERVER_DATA": "/tmp/legacy-data",
                "AIT_NATIVE_SERVER_DB_BACKEND": "postgres",
                "AIT_NATIVE_SERVER_POSTGRES_DSN": "postgres://example"
            }
        }),
    )
    .expect("from env");

    assert_eq!(payload["context"]["root"], json!("/tmp/runtime-data"));
    assert_eq!(payload["context"]["root_source"], json!("env"));
    assert_eq!(
        payload["context"]["content_schema"],
        json!("ait_native_content")
    );
    assert_eq!(
        payload["context"]["control_schema"],
        json!("ait_native_control")
    );
    assert_eq!(
        payload["context"]["postgres_dsn"],
        json!("postgres://example")
    );
}

#[test]
fn server_context_resolve_root_reports_missing_runtime_data() {
    let error =
        server_context_json("resolve-root", &json!({"env": {}})).expect_err("missing runtime data");
    assert_eq!(
        error,
        "AIT_NATIVE_SERVER_DATA is required for server runtime access; platform default runtime roots are no longer supported."
    );
}
