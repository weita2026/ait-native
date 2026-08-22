use ait_server_core::foundation::server_auth_policy::{
    actor_from_headers, all_roles, evaluate_repo_action, line_update_required_action, role_set,
    server_auth_policy_contract, server_auth_policy_json, SERVER_AUTH_POLICY_CONTRACT_VERSION,
    SERVER_AUTH_REFERENCE_MODULE,
};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeSet;

#[test]
fn server_auth_policy_contract_names_roles_and_reference() {
    let contract = server_auth_policy_contract();
    assert_eq!(
        contract["contract"],
        json!(SERVER_AUTH_POLICY_CONTRACT_VERSION)
    );
    assert_eq!(
        contract["reference_modules"],
        json!([SERVER_AUTH_REFERENCE_MODULE])
    );
    assert_eq!(
        contract["role_sets"]["land"],
        json!(["operator", "release_manager", "repo_owner"])
    );
    assert_eq!(
        contract["compatibility_notes"]["python_reference"],
        json!("Web authorization caller glue lives in ait_web.server_auth_runtime; Rust owns the server auth policy contract.")
    );
    assert_eq!(
        contract["compatibility_notes"]["task_dag"],
        json!("Task DAG is retired and is not a server auth policy surface.")
    );

    let roles = all_roles();
    assert_eq!(roles.len(), 8);
    assert!(roles.contains(&"operator".to_string()));
    assert_eq!(role_set("approve_assisted"), role_set("review"));
}

#[test]
fn server_auth_policy_actor_normalization_matches_open_and_strict_modes() {
    let open_payload = json!({"x-ait-actor-type": "agent"});
    let open_actor =
        actor_from_headers("open".into(), open_payload.as_object().unwrap()).expect("open actor");
    assert_eq!(open_actor.identity, "anonymous");
    assert_eq!(open_actor.actor_type, "agent");
    assert_eq!(
        open_actor.claimed_roles,
        all_roles().into_iter().collect::<BTreeSet<_>>()
    );
    assert_eq!(open_actor.claimed_repos, BTreeSet::from(["*".to_string()]));

    let strict_missing = actor_from_headers("strict".into(), &Default::default())
        .expect_err("strict mode should require actor");
    assert_eq!(strict_missing.status, 401);
    assert_eq!(
        strict_missing.detail,
        "Missing X-AIT-Actor in strict auth mode"
    );

    let strict_payload = json!({
        "X-AIT-Actor": "alice",
        "X-AIT-Roles": "repo_contributor,invalid,operator",
        "X-AIT-Repos": "ait-server,other"
    });
    let strict_actor = actor_from_headers("strict".into(), strict_payload.as_object().unwrap())
        .expect("strict actor");
    assert_eq!(strict_actor.identity, "alice");
    assert_eq!(
        strict_actor.claimed_roles,
        BTreeSet::from(["operator".to_string(), "repo_contributor".to_string()])
    );
    assert_eq!(
        strict_actor.claimed_repos,
        BTreeSet::from(["ait-server".to_string(), "other".to_string()])
    );
}

#[test]
fn server_auth_policy_evaluates_repo_action_permissions() {
    let headers = json!({"X-AIT-Actor": "reader"});
    let actor =
        actor_from_headers("strict".into(), headers.as_object().unwrap()).expect("strict actor");
    let read = evaluate_repo_action(
        &actor,
        "ait-server",
        "read",
        BTreeSet::from(["repo_reader".to_string()]),
        None,
        None,
    )
    .expect("read decision");
    assert!(read.allowed);

    let denied = evaluate_repo_action(
        &actor,
        "ait-server",
        "contribute",
        BTreeSet::from(["repo_reader".to_string()]),
        None,
        None,
    )
    .expect("contribute decision");
    assert!(!denied.allowed);
    assert_eq!(denied.status, 403);
    assert_eq!(
        denied.detail,
        "Actor reader lacks permission for contribute on repository ait-server"
    );

    let operator_headers = json!({"X-AIT-Actor": "ops", "X-AIT-Roles": "operator"});
    let operator_actor = actor_from_headers("strict".into(), operator_headers.as_object().unwrap())
        .expect("operator actor");
    let operator_decision = evaluate_repo_action(
        &operator_actor,
        "ait-server",
        "admin",
        BTreeSet::new(),
        None,
        None,
    )
    .expect("operator decision");
    assert!(operator_decision.allowed);
    assert!(operator_decision.effective_roles.contains("operator"));
}

#[test]
fn server_auth_policy_rejects_inactive_repository_writes_after_role_check() {
    let headers = json!({"X-AIT-Actor": "owner"});
    let actor =
        actor_from_headers("strict".into(), headers.as_object().unwrap()).expect("strict actor");

    let read = evaluate_repo_action(
        &actor,
        "ait-server",
        "read",
        BTreeSet::from(["repo_reader".to_string()]),
        Some("archived"),
        None,
    )
    .expect("read decision");
    assert!(read.allowed);

    let write = evaluate_repo_action(
        &actor,
        "ait-server",
        "land",
        BTreeSet::from(["repo_owner".to_string()]),
        Some("archived"),
        None,
    )
    .expect("write decision");
    assert!(!write.allowed);
    assert_eq!(write.status, 409);
    assert_eq!(
        write.detail,
        "Repository ait-server is archived and does not accept land actions"
    );
}

#[test]
fn server_auth_policy_maps_line_review_and_admin_actions() {
    assert_eq!(
        line_update_required_action("main", "main"),
        (
            "land",
            "Updating default line main requires release or owner authority".to_string()
        )
    );
    assert_eq!(
        line_update_required_action("feature", "main"),
        ("contribute", String::new())
    );

    let default_line = server_auth_policy_json(
        "line-update",
        &json!({
            "auth_mode": "strict",
            "headers": {"X-AIT-Actor": "contrib"},
            "repo_name": "ait-server",
            "line_name": "main",
            "default_line": "main",
            "bound_roles": ["repo_contributor"]
        }),
    )
    .expect("default line decision");
    assert_eq!(default_line["decision"]["allowed"], json!(false));
    assert_eq!(default_line["decision"]["status"], json!(403));
    assert_eq!(
        default_line["decision"]["detail"],
        json!("Updating default line main requires release or owner authority")
    );

    let review = server_auth_policy_json(
        "review-action",
        &json!({
            "auth_mode": "strict",
            "headers": {"X-AIT-Actor": "reviewer"},
            "repo_name": "ait-server",
            "review_action": "approve",
            "bound_roles": ["repo_reviewer"]
        }),
    )
    .expect("review decision");
    assert_eq!(
        review["decision"]["required_action"],
        json!("approve_assisted")
    );
    assert_eq!(review["decision"]["allowed"], json!(true));

    let admin = server_auth_policy_json(
        "admin-action",
        &json!({
            "auth_mode": "strict",
            "headers": {"X-AIT-Actor": "reader"},
            "repo_name": "ait-server",
            "bound_roles": ["repo_reader"]
        }),
    )
    .expect("admin decision");
    assert_eq!(admin["decision"]["status"], json!(403));
    assert_eq!(
        admin["decision"]["detail"],
        json!("Managing role bindings for ait-server requires repo_owner or operator")
    );
}

#[test]
fn server_auth_policy_endpoint_payloads_return_decisions_not_python_errors() {
    let strict_missing = server_auth_policy_json(
        "repo-action",
        &json!({
            "auth_mode": "strict",
            "repo_name": "ait-server",
            "action": "read"
        }),
    )
    .expect("strict missing actor decision");
    assert_eq!(strict_missing["decision"]["status"], json!(401));

    let unsupported = server_auth_policy_json(
        "repo-action",
        &json!({"repo_name": "ait-server", "action": "x"}),
    )
    .expect_err("unsupported action should be a contract error");
    assert_eq!(unsupported, "Unsupported repo action `x`.");

    let malformed = server_auth_policy_json("repo-action", &JsonValue::Null)
        .expect_err("malformed payload should fail");
    assert_eq!(
        malformed,
        "server auth policy payload must be a JSON object."
    );
}
