use super::*;

#[test]
fn authority_constructors_require_an_explicit_operating_role() {
    let serving = FilesystemServerRemoteBinaryDb::serving_authority(
        RepoId::new("serving-repo"),
        RepoName::new("serving"),
        make_temporary_root(),
        StoreGeneration::new(1),
    );
    let fixture = FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("fixture-repo"),
        RepoName::new("fixture"),
        make_temporary_root(),
        StoreGeneration::new(3),
    );

    assert!(serving.authority_mode().is_serving_authority());
    assert_eq!(
        fixture.authority_mode(),
        ServerBinaryDbAuthorityMode::TestFixture
    );
}

#[test]
fn authority_contract_forbids_direct_client_filesystem_writes() {
    let guarantees = SERVER_BINARY_DB_AUTHORITY_CONTRACT
        .iter()
        .map(|row| row.guarantee)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(guarantees.contains("ait-server serving authority is the sole filesystem writer"));
    assert!(guarantees.contains("HTTP, compare-and-swap, pack, and import/export"));
    assert!(guarantees.contains("never mutate the deployed authority root directly"));
    assert!(guarantees.contains("persistent rollback journals"));
}

#[test]
fn public_binary_db_contract_exposes_no_raw_file_mutators() {
    let source = include_str!("../contracts.rs");
    let public_contract = source
        .split_once("pub trait BinaryDb:")
        .expect("public BinaryDb trait")
        .1
        .split_once("pub trait RemoteBinaryDb")
        .expect("end of public BinaryDb trait")
        .0;

    for raw_mutator in [
        "fn append_bytes",
        "fn overwrite_range",
        "fn truncate_file",
        "fn remove_file_if_exists",
    ] {
        assert!(
            !public_contract.contains(raw_mutator),
            "public BinaryDb must not expose {raw_mutator}"
        );
    }
}

#[test]
fn server_lock_namespaces_remain_domain_specific() {
    assert_eq!(
        BinaryDbCommandScope::ServerWorkflow.lock_file_names(),
        &["server-workflow.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::ServerPlan.lock_file_names(),
        &["server-plan.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::ServerQueue.lock_file_names(),
        &["server-queue.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::ServerRepositoryPack.lock_file_names(),
        &["server-repository-pack.write.lock"]
    );
    assert!(SERVER_BINARY_DB_AUTHORITY_CONTRACT.iter().any(|row| {
        row.invariant == "independent local locks" && row.guarantee.contains("different writers")
    }));
}
