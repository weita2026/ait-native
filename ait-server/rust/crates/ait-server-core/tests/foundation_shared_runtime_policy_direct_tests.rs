use ait_server_core::foundation::shared_runtime_policy::{
    detect_shared_deployment, evaluate_shared_runtime_policy, is_loopback_host, normalize_host,
    parse_env_flag, shared_runtime_policy_contract, shared_runtime_policy_json,
    SHARED_RUNTIME_POLICY_CONTRACT_VERSION, SHARED_RUNTIME_POLICY_REFERENCE_MODULE,
};
use serde_json::json;

#[test]
fn shared_runtime_policy_contract_names_historical_reference_and_boundaries() {
    let contract = shared_runtime_policy_contract();
    assert_eq!(
        contract["contract"],
        json!(SHARED_RUNTIME_POLICY_CONTRACT_VERSION)
    );
    assert_eq!(
        contract["reference_modules"],
        json!([SHARED_RUNTIME_POLICY_REFERENCE_MODULE])
    );
    assert_eq!(contract["policy"]["required_backend"], json!("postgres"));
    assert_eq!(contract["policy"]["allowed_backends"], json!(["postgres"]));
    assert_eq!(
        contract["policy"]["legacy_override_supported"],
        json!(false)
    );
    assert_eq!(
        contract["compatibility_notes"]["python_reference"],
        json!("Web caller glue lives in ait_web.shared_runtime_policy; Rust owns the shared runtime policy contract.")
    );
    assert_eq!(
        contract["compatibility_notes"]["task_dag"],
        json!("Task DAG is retired and is not a shared runtime policy surface.")
    );
}

#[test]
fn shared_runtime_policy_parses_python_env_flag_semantics() {
    assert_eq!(parse_env_flag(None), None);
    assert_eq!(parse_env_flag(Some("  ")), None);
    assert_eq!(parse_env_flag(Some("0")), Some(false));
    assert_eq!(parse_env_flag(Some("false")), Some(false));
    assert_eq!(parse_env_flag(Some("off")), Some(false));
    assert_eq!(parse_env_flag(Some("1")), Some(true));
    assert_eq!(parse_env_flag(Some("yes")), Some(true));
    assert_eq!(parse_env_flag(Some("unexpected")), Some(true));
}

#[test]
fn shared_runtime_policy_normalizes_hosts_and_detects_loopback() {
    assert_eq!(normalize_host(Some(" [::1] "), "127.0.0.1"), "::1");
    assert_eq!(normalize_host(Some(" "), "127.0.0.1"), "127.0.0.1");
    assert!(is_loopback_host(Some("[::1]")));
    assert!(is_loopback_host(Some("127.8.9.10")));
    assert!(is_loopback_host(Some("localhost")));
    assert!(!is_loopback_host(Some("10.0.0.5")));
}

#[test]
fn shared_runtime_policy_detects_explicit_and_host_shared_deployments() {
    assert_eq!(
        detect_shared_deployment(Some("1"), "127.0.0.1", "127.0.0.1"),
        (true, "AIT_NATIVE_SHARED_DEPLOYMENT=1".to_string())
    );
    assert_eq!(
        detect_shared_deployment(Some("0"), "10.0.0.2", "127.0.0.1"),
        (false, "AIT_NATIVE_SHARED_DEPLOYMENT=0".to_string())
    );
    assert_eq!(
        detect_shared_deployment(None, "10.0.0.2", "127.0.0.1"),
        (true, "server_host=10.0.0.2".to_string())
    );
    assert_eq!(
        detect_shared_deployment(None, "localhost", "10.0.0.2"),
        (false, "loopback_hosts".to_string())
    );
}

#[test]
fn shared_runtime_policy_evaluates_postgres_compliance() {
    let policy = evaluate_shared_runtime_policy(
        "ait-server",
        Some("postgres"),
        None,
        Some("10.0.0.2"),
        Some("127.0.0.1"),
    )
    .expect("postgres policy");
    assert!(policy.ok);
    assert_eq!(policy.component, "ait-server");
    assert_eq!(policy.db_backend, "postgres");
    assert_eq!(policy.deployment_scope, "shared");
    assert_eq!(policy.state, "postgres_compliant");
    assert_eq!(
        policy.reason,
        "PostgreSQL-backed runtime satisfies the shared deployment policy."
    );
    assert!(!policy.override_active);
    assert!(!policy.override_supported);
}

#[test]
fn shared_runtime_policy_rejects_unsupported_backend() {
    let error = shared_runtime_policy_json(
        "evaluate",
        &json!({
            "component": "ait-server",
            "db_backend": "local-file"
        }),
    )
    .expect_err("unsupported backend");
    assert_eq!(
        error,
        "Unsupported AIT native server database backend: 'local-file'"
    );
}
