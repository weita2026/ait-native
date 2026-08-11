use sha2::{Digest, Sha256};

pub const SERVER_BINARY_DB_CONFORMANCE_VECTOR_VERSION: &str =
    "ait.binary-db.conformance-vectors.v2";
pub const SERVER_BINARY_DB_CONFORMANCE_VECTOR_CHECKSUM: &str =
    "98cb6f0eb09037bd88b42c9f984426d353c44ab81e19564510e0a8fa43c66866";
pub const SERVER_BINARY_DB_CONFORMANCE_VECTOR_SOURCE: &[u8] =
    include_bytes!("../../../tests/fixtures/binary_db_conformance_vectors_v2.json");
pub const SERVER_BINARY_DB_PLAN_GOLDEN_VERSION: &str = "ait.plan-binary-db.golden-bytes.v1";
pub const SERVER_BINARY_DB_PLAN_GOLDEN_CHECKSUM: &str =
    "feeb856eba66b4040b85b6a462b7342a94f978798052b8097b526e2cbcae0d96";
pub const SERVER_BINARY_DB_PLAN_GOLDEN_SOURCE: &[u8] =
    include_bytes!("../../../tests/fixtures/plan_binary_db_layout1_golden_v1.json");
pub const SERVER_BINARY_DB_CROSS_REPO_PARITY_MANIFEST_VERSION: &str =
    "ait.binary-db.cross-repo-parity-manifest.v1";
pub const SERVER_BINARY_DB_CROSS_REPO_PARITY_MANIFEST_CHECKSUM: &str =
    "eaf68ddc057e06fd6d01cecab0d6d89d7862a65432c651179f13fba9f7ef9535";
pub const SERVER_BINARY_DB_CROSS_REPO_PARITY_MANIFEST_SOURCE: &[u8] =
    include_bytes!("../../../tests/fixtures/binary_db_cross_repo_parity_manifest_v1.json");

pub fn server_binary_db_conformance_vector_checksum() -> String {
    format!(
        "{:x}",
        Sha256::digest(SERVER_BINARY_DB_CONFORMANCE_VECTOR_SOURCE)
    )
}

pub fn server_binary_db_plan_golden_checksum() -> String {
    format!("{:x}", Sha256::digest(SERVER_BINARY_DB_PLAN_GOLDEN_SOURCE))
}

pub fn server_binary_db_cross_repo_parity_manifest_checksum() -> String {
    format!(
        "{:x}",
        Sha256::digest(SERVER_BINARY_DB_CROSS_REPO_PARITY_MANIFEST_SOURCE)
    )
}

pub const SERVER_BINARY_DB_CANONICAL_ROLLOUT_TESTS: &[&str] = &[
    "server_binary_db_conformance_vectors_v2",
    "server_binary_db_transaction_conformance_v2",
    "server_binary_db_extended_conformance_v2",
    "server_plan_binary_db_complete_golden_fixture_matches_core_wire_contract",
    "land_failure_matrix_restores_canonical_aggregate",
    "zstd_bulk_failure_matrix_restores_all_publication_files",
    "sbdh_recovery_is_idempotent_and_preserves_committed_bytes",
    "server_content_reads_dispatch_from_persisted_layout",
];
