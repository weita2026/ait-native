use ait_server_core::foundation::community_auth::{
    community_actor_identity, community_auth_contract, community_auth_json, expires_at_text,
    hash_community_password_with_salt, normalize_community_email, session_payload,
    validate_password, verify_community_password, COMMUNITY_AUTH_CONTRACT_VERSION,
    COMMUNITY_AUTH_REFERENCE_MODULE,
};
use serde_json::json;

const FIXED_SALT: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];
const FIXED_PARAMS_JSON: &str =
    "{\"dklen\":64,\"n\":16384,\"p\":1,\"r\":8,\"salt_b64\":\"AAECAwQFBgcICQoLDA0ODw==\"}";
const FIXED_PASSWORD_HASH: &str = "d7590aca2c9801cf06eeba772a69dc31ce3862591d96522ac4e6bba6ad1f31a52d6f736f2b85adaa6262335eb112e56f014f417a37d74be0def7669b2c51c29e";

#[test]
fn community_auth_contract_names_security_scope_and_boundaries() {
    let contract = community_auth_contract();
    assert_eq!(contract["contract"], json!(COMMUNITY_AUTH_CONTRACT_VERSION));
    assert_eq!(
        contract["reference_modules"],
        json!([COMMUNITY_AUTH_REFERENCE_MODULE])
    );
    assert_eq!(contract["password"]["algorithm"], json!("scrypt"));
    assert_eq!(contract["password"]["n"], json!(16384));
    assert_eq!(contract["session"]["ttl_days"], json!(14));
    assert_eq!(
        contract["compatibility_notes"]["task_dag"],
        json!("Task DAG is retired and is not a community auth surface.")
    );
}

#[test]
fn community_auth_normalizes_email_and_actor_identity() {
    assert_eq!(
        normalize_community_email(Some("  PERSON@Example.COM  ")),
        Some("person@example.com".to_string())
    );
    assert_eq!(normalize_community_email(Some("   ")), None);
    assert_eq!(community_actor_identity("CA-123"), "community:CA-123");

    let payload = community_auth_json("normalize-email", &json!({"email": "  A@B.COM  "}))
        .expect("normalize email");
    assert_eq!(payload["email_normalized"], json!("a@b.com"));
}

#[test]
fn community_auth_validates_password_compatibility_errors() {
    assert_eq!(
        validate_password(None).expect_err("missing password"),
        "Password is required."
    );
    assert_eq!(
        validate_password(Some("short")).expect_err("short password"),
        "Password must be at least 10 characters."
    );
    assert_eq!(
        validate_password(Some("  long enough  ")).expect("valid password"),
        "long enough"
    );
}

#[test]
fn community_auth_hashes_and_verifies_python_scrypt_fixture() {
    let (hash, algo, params_json) =
        hash_community_password_with_salt("correct horse battery staple", FIXED_SALT)
            .expect("hash password");
    assert_eq!(algo, "scrypt");
    assert_eq!(params_json, FIXED_PARAMS_JSON);
    assert_eq!(hash, FIXED_PASSWORD_HASH);

    assert!(verify_community_password(
        "correct horse battery staple",
        FIXED_PASSWORD_HASH,
        "scrypt",
        FIXED_PARAMS_JSON,
    )
    .expect("verify password"));
    assert!(!verify_community_password(
        "wrong password",
        FIXED_PASSWORD_HASH,
        "scrypt",
        FIXED_PARAMS_JSON
    )
    .expect("reject wrong password"));
}

#[test]
fn community_auth_reports_unsupported_password_algorithm() {
    let error =
        verify_community_password("password", FIXED_PASSWORD_HASH, "argon2", FIXED_PARAMS_JSON)
            .expect_err("unsupported algorithm");
    assert_eq!(
        error,
        "Unsupported Community password algorithm: \"argon2\""
    );

    let payload = community_auth_json(
        "verify-password",
        &json!({
            "password": "password",
            "password_hash": FIXED_PASSWORD_HASH,
            "password_algo": "argon2",
            "password_params_json": FIXED_PARAMS_JSON
        }),
    )
    .expect_err("unsupported operation payload");
    assert_eq!(
        payload,
        "Unsupported Community password algorithm: \"argon2\""
    );
}

#[test]
fn community_auth_expires_at_adds_fourteen_days() {
    assert_eq!(
        expires_at_text("2026-07-08T00:00:00+00:00").expect("expires"),
        "2026-07-22T00:00:00+00:00"
    );
    let payload = community_auth_json("expires-at", &json!({"now": "2026-07-08T01:02:03Z"}))
        .expect("expires operation");
    assert_eq!(payload["expires_at"], json!("2026-07-22T01:02:03+00:00"));
}

#[test]
fn community_auth_session_payload_matches_fallback_defaults() {
    let account = json!({
        "account_id": "CA-123",
        "email_normalized": "person@example.com",
        "full_name": "  Full Name  ",
        "display_name": " ",
        "organization": "  Org  ",
        "role_title": null,
        "status": "",
        "primary_auth_method": null
    });
    let session = json!({
        "web_session_id": "CWS-1",
        "session_source": "",
        "created_at": "2026-07-08T00:00:00Z",
        "expires_at": "2026-07-22T00:00:00Z",
        "revoked_at": null,
        "last_seen_at": "2026-07-08T00:01:00Z"
    });
    let payload = session_payload(account.as_object().unwrap(), session.as_object().unwrap())
        .expect("session payload");
    assert_eq!(payload["account_id"], json!("CA-123"));
    assert_eq!(payload["actor_identity"], json!("community:CA-123"));
    assert_eq!(payload["actor_type"], json!("community_user"));
    assert_eq!(payload["display_name"], json!("Full Name"));
    assert_eq!(payload["full_name"], json!("Full Name"));
    assert_eq!(payload["organization"], json!("Org"));
    assert_eq!(payload["role_title"], json!(null));
    assert_eq!(payload["status"], json!("active"));
    assert_eq!(payload["primary_auth_method"], json!("password"));
    assert_eq!(payload["session_source"], json!("password"));

    let operation = community_auth_json(
        "session-payload",
        &json!({"account_row": account, "session_row": session}),
    )
    .expect("session operation");
    assert_eq!(operation["session"]["web_session_id"], json!("CWS-1"));
}
