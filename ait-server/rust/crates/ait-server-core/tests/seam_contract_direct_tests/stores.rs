#[test]
fn policy_store_contract_command_declares_postgres_surface() {
    let value = stdout_json(&run_seam(&["policy-store", "contract", "{}"]));

    assert_eq!(value["contract"], json!("ait.server.policy_store.v1"));
    assert_eq!(value["backend"], json!("postgres"));
    assert_eq!(
        value["migration_status"],
        json!("rust_owned_no_python_reference")
    );
    assert!(value.get("previous_reference_module").is_none());
    assert_eq!(value["mutates_state"], json!(true));
    assert!(value["operations"]
        .as_array()
        .expect("operations should be an array")
        .contains(&json!("evaluate-policy")));
    assert!(value["operations"]
        .as_array()
        .expect("operations should be an array")
        .contains(&json!("create-waiver")));
}

#[test]
fn policy_store_command_requires_real_postgres_dsn() {
    assert_failed_with(
        &run_seam_without_postgres_dsn(&[
            "policy-store",
            "get-policy",
            r#"{"patchset_id":"PS-1"}"#,
        ]),
        "PostgreSQL backend requested but AIT_NATIVE_SERVER_POSTGRES_DSN is not configured",
    );
    assert_failed_with(
        &run_seam(&[
            "policy-store",
            "get-policy",
            r#"{"patchset_id":"PS-1","dsn":"fake-postgres:///tmp/ait"}"#,
        ]),
        "fake-postgres is not supported",
    );
}

#[test]
fn patchset_store_contract_command_declares_postgres_surface() {
    let value = stdout_json(&run_seam(&["patchset-store", "contract", "{}"]));

    assert_eq!(value["contract"], json!("ait.server.patchset_store.v1"));
    assert_eq!(value["backend"], json!("postgres"));
    assert_eq!(
        value["migration_status"],
        json!("rust_owned_no_python_reference")
    );
    assert!(value.get("previous_reference_module").is_none());
    assert_eq!(value["mutates_state"], json!(true));
    assert!(value["operations"]
        .as_array()
        .expect("operations should be an array")
        .contains(&json!("publish-patchset")));
    assert!(value["operations"]
        .as_array()
        .expect("operations should be an array")
        .contains(&json!("upsert-attestation")));
}

#[test]
fn patchset_store_command_requires_real_postgres_dsn() {
    assert_failed_with(
        &run_seam_without_postgres_dsn(&[
            "patchset-store",
            "get-patchset",
            r#"{"patchset_id":"PS-1"}"#,
        ]),
        "PostgreSQL backend requested but AIT_NATIVE_SERVER_POSTGRES_DSN is not configured",
    );
    assert_failed_with(
        &run_seam(&[
            "patchset-store",
            "get-patchset",
            r#"{"patchset_id":"PS-1","dsn":"fake-postgres:///tmp/ait"}"#,
        ]),
        "fake-postgres is not supported",
    );
}

#[test]
fn review_store_contract_command_declares_postgres_surface() {
    let value = stdout_json(&run_seam(&["review-store", "contract", "{}"]));

    assert_eq!(value["contract"], json!("ait.server.review_store.v1"));
    assert_eq!(
        value["migration_status"],
        json!("rust_owned_no_python_reference")
    );
    assert!(value.get("previous_reference_module").is_none());
    assert_eq!(value["backend"], json!("postgres"));
    assert_eq!(value["mutates_state"], json!(true));
    assert!(value["operations"]
        .as_array()
        .expect("operations should be an array")
        .contains(&json!("request-review")));
    assert!(value["operations"]
        .as_array()
        .expect("operations should be an array")
        .contains(&json!("record-review")));
    assert!(value["operations"]
        .as_array()
        .expect("operations should be an array")
        .contains(&json!("list-reviews")));
}

#[test]
fn review_store_command_requires_real_postgres_dsn() {
    assert_failed_with(
        &run_seam_without_postgres_dsn(&["review-store", "list-reviews", r#"{"change_id":"C-1"}"#]),
        "PostgreSQL backend requested but AIT_NATIVE_SERVER_POSTGRES_DSN is not configured",
    );
    assert_failed_with(
        &run_seam(&[
            "review-store",
            "list-reviews",
            r#"{"change_id":"C-1","dsn":"fake-postgres:///tmp/ait"}"#,
        ]),
        "fake-postgres is not supported",
    );
}
