use ait_server_core::foundation::community_ids::{
    build_community_id, community_ids_contract, community_ids_json, encode_crockford_base32,
    generate_community_id, randomness_from_hex, COMMUNITY_IDS_CONTRACT_VERSION, CROCKFORD_BASE32,
};
use serde_json::json;

#[test]
fn community_ids_contract_reports_rust_owned_wrapper_removed_status() {
    let contract = community_ids_contract();
    assert_eq!(contract["contract"], json!(COMMUNITY_IDS_CONTRACT_VERSION));
    assert_eq!(contract["reference_modules"], json!([]));
    assert_eq!(
        contract["migration_status"],
        json!("python_wrapper_removed_rust_owned")
    );
    assert_eq!(contract["alphabet"], json!(CROCKFORD_BASE32));
    assert_eq!(contract["ulid_shape"]["ulid_length"], json!(26));
    assert_eq!(
        contract["compatibility_notes"]["task_dag"],
        json!("Task DAG is retired and is not a community ID surface.")
    );
}

#[test]
fn community_ids_encode_crockford_base32_matches_reference_shape() {
    assert_eq!(
        encode_crockford_base32(0, 10).expect("zero timestamp"),
        "0000000000"
    );
    assert_eq!(
        encode_crockford_base32(1, 10).expect("one timestamp"),
        "0000000001"
    );
    assert_eq!(encode_crockford_base32(31, 1).expect("max one char"), "Z");
    assert_eq!(
        encode_crockford_base32(32, 1).expect_err("overflow"),
        "Value does not fit requested Crockford base32 length"
    );
}

#[test]
fn community_ids_builds_deterministic_prefixed_ids() {
    let zeros = randomness_from_hex("00000000000000000000").expect("zero randomness");
    assert_eq!(
        build_community_id("CA", 0, &zeros).expect("zero id"),
        "CA-00000000000000000000000000"
    );

    let max_randomness = randomness_from_hex("ffffffffffffffffffff").expect("max randomness");
    assert_eq!(
        build_community_id("CWS", 1, &max_randomness).expect("max random id"),
        "CWS-0000000001ZZZZZZZZZZZZZZZZ"
    );
}

#[test]
fn community_ids_rejects_invalid_randomness_hex() {
    assert_eq!(
        randomness_from_hex("00").expect_err("too short"),
        "randomness_hex must contain exactly 20 hex characters."
    );
    assert_eq!(
        randomness_from_hex("zzzzzzzzzzzzzzzzzzzz").expect_err("not hex"),
        "randomness_hex must contain only hexadecimal characters."
    );

    let payload = community_ids_json(
        "build",
        &json!({
            "prefix": "CA",
            "timestamp_ms": 0,
            "randomness_hex": "zzzzzzzzzzzzzzzzzzzz"
        }),
    )
    .expect_err("bad randomness");
    assert_eq!(
        payload,
        "randomness_hex must contain only hexadecimal characters."
    );
}

#[test]
fn community_ids_json_build_and_encode_operations_are_stable() {
    let encoded = community_ids_json(
        "encode-crockford-base32",
        &json!({"value": "31", "length": 2}),
    )
    .expect("encode operation");
    assert_eq!(encoded["encoded"], json!("0Z"));

    let built = community_ids_json(
        "build",
        &json!({
            "prefix": "CA",
            "timestamp_ms": "1",
            "randomness_hex": "00000000000000000001"
        }),
    )
    .expect("build operation");
    assert_eq!(built["id"], json!("CA-00000000010000000000000001"));
}

#[test]
fn community_ids_generate_uses_prefix_and_ulid_shape() {
    let generated = generate_community_id("CA").expect("generated id");
    let (prefix, ulid) = generated.split_once('-').expect("prefixed id");
    assert_eq!(prefix, "CA");
    assert_eq!(ulid.len(), 26);
    assert!(ulid.chars().all(|ch| CROCKFORD_BASE32.contains(ch)));
}
