use super::*;

#[test]
fn identity_getters() {
    let (db, root, _ctx) = make_db();
    assert_eq!(db.repo_id(), &RepoId::new("repo-uuid-001"));
    assert_eq!(db.repo_name(), &RepoName::new("repo-name"));
    assert_eq!(db.storage_generation(), StoreGeneration::new(7));
    assert_eq!(
        ServerRemoteBinaryDb::authority_root(&db).as_path(),
        root.as_path()
    );
}

#[test]
fn file_ids_carry_core_compatible_layout_metadata() {
    let task = task_file_id();
    assert_eq!(
        task.relative_path().as_path(),
        std::path::Path::new("task.bin")
    );
    assert_eq!(task.layout_id(), 1);
    assert_eq!(
        task.record_size(),
        crate::foundation::workflow_binary_v0::TASK_RECORD_SIZE
    );
    assert_eq!(task.family(), BinaryDbFileFamily::Workflow);

    let payload = task_payload_file_id();
    assert_eq!(
        payload.relative_path().as_path(),
        std::path::Path::new("task_payload.bin")
    );
    assert_eq!(payload.layout_id(), 1);
    assert_eq!(payload.family(), BinaryDbFileFamily::Workflow);

    let index = task_change_index_id();
    assert_eq!(
        index.relative_path().as_path(),
        std::path::Path::new("task_change_index.idx")
    );
    assert_eq!(index.layout_id(), 1);
    assert_eq!(index.family(), BinaryDbFileFamily::Workflow);
}

#[test]
fn compatibility_contract_covers_ait_core_binary_db_primitives() {
    let expected = [
        BinaryDbCompatibilityPrimitive::ErrorKind,
        BinaryDbCompatibilityPrimitive::StorePath,
        BinaryDbCompatibilityPrimitive::FileFamily,
        BinaryDbCompatibilityPrimitive::FileId,
        BinaryDbCompatibilityPrimitive::PayloadFileId,
        BinaryDbCompatibilityPrimitive::IndexId,
        BinaryDbCompatibilityPrimitive::PayloadRange,
        BinaryDbCompatibilityPrimitive::ReadScope,
        BinaryDbCompatibilityPrimitive::ReadTxn,
        BinaryDbCompatibilityPrimitive::WriteTxn,
        BinaryDbCompatibilityPrimitive::CommandScope,
        BinaryDbCompatibilityPrimitive::FsyncPolicy,
    ];

    for primitive in expected {
        let row = AIT_CORE_BINARY_DB_COMPATIBILITY_CONTRACT
            .iter()
            .find(|row| row.primitive == primitive)
            .unwrap_or_else(|| panic!("missing compatibility primitive {primitive:?}"));
        assert!(
            row.ait_core_reference.starts_with("ait_core::binary_db::"),
            "{primitive:?} must cite the local ait-core Binary DB reference"
        );
        assert!(
            !row.server_type.trim().is_empty() && !row.guarantee.trim().is_empty(),
            "{primitive:?} must document server type and guarantee"
        );
    }
}

#[test]
fn read_scope_keeps_core_families_compatible_and_server_families_explicit() {
    assert_eq!(BinaryDbReadScope::default(), BinaryDbReadScope::ALL);
    assert!(BinaryDbReadScope::CONTENT.includes_family(BinaryDbFileFamily::Content));
    assert!(BinaryDbReadScope::PLAN.includes_family(BinaryDbFileFamily::Plan));
    assert!(!BinaryDbReadScope::PLAN.includes_family(BinaryDbFileFamily::Content));
    assert!(BinaryDbReadScope::WORKFLOW.includes_family(BinaryDbFileFamily::Workflow));
    assert!(BinaryDbReadScope::QUEUE.includes_family(BinaryDbFileFamily::Queue));
    assert!(BinaryDbReadScope::REPOSITORY_PACK.includes_family(BinaryDbFileFamily::RepositoryPack));
}

#[test]
fn error_kinds_match_ait_core_binary_db_categories() {
    let kinds = [
        BinaryDbErrorKind::RetryableBusy,
        BinaryDbErrorKind::Corruption,
        BinaryDbErrorKind::LayoutMismatch,
        BinaryDbErrorKind::MissingData,
        BinaryDbErrorKind::InvalidDomainData,
        BinaryDbErrorKind::Io,
        BinaryDbErrorKind::Unsupported,
        BinaryDbErrorKind::Other,
    ];
    assert_eq!(kinds.len(), 8);
    assert!(BinaryDbError::retryable_busy("busy").is_retryable_busy());
    assert_eq!(
        BinaryDbError::invalid_domain_data("bad domain").kind(),
        BinaryDbErrorKind::InvalidDomainData
    );
    assert_eq!(
        BinaryDbError::unsupported("layout").kind(),
        BinaryDbErrorKind::Unsupported
    );
}

#[test]
fn command_scopes_use_ait_core_lock_family_shape() {
    assert_eq!(
        BinaryDbCommandScope::General.lock_file_names(),
        &[
            "global.write.lock",
            "server-content.write.lock",
            "server-plan.write.lock",
            "server-queue.write.lock",
            "server-repository-pack.write.lock",
            "server-workflow.write.lock",
        ]
    );
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
    assert_eq!(
        BinaryDbCommandScope::ServerContent.lock_file_names(),
        &["server-content.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::ServerTaskStart.lock_file_names(),
        &["server-plan.write.lock", "server-workflow.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::ServerLand.lock_file_names(),
        &["server-content.write.lock", "server-workflow.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::ServerRemoteSyncCommit.lock_file_names(),
        &["server-content.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::all_write_lock_file_names(),
        &[
            "global.write.lock",
            "server-content.write.lock",
            "server-plan.write.lock",
            "server-queue.write.lock",
            "server-repository-pack.write.lock",
            "server-workflow.write.lock",
        ]
    );
}

#[test]
fn command_scopes_authorize_exact_server_file_families() {
    let all_families = [
        BinaryDbFileFamily::Workflow,
        BinaryDbFileFamily::Plan,
        BinaryDbFileFamily::Queue,
        BinaryDbFileFamily::RepositoryPack,
        BinaryDbFileFamily::Content,
    ];
    let cases: &[(BinaryDbCommandScope, &[BinaryDbFileFamily])] = &[
        (BinaryDbCommandScope::General, &all_families),
        (
            BinaryDbCommandScope::ServerWorkflow,
            &[BinaryDbFileFamily::Workflow],
        ),
        (
            BinaryDbCommandScope::ServerPlan,
            &[BinaryDbFileFamily::Plan],
        ),
        (
            BinaryDbCommandScope::ServerQueue,
            &[BinaryDbFileFamily::Queue],
        ),
        (
            BinaryDbCommandScope::ServerRepositoryPack,
            &[BinaryDbFileFamily::RepositoryPack],
        ),
        (
            BinaryDbCommandScope::ServerContent,
            &[BinaryDbFileFamily::Content],
        ),
        (
            BinaryDbCommandScope::ServerTaskStart,
            &[BinaryDbFileFamily::Workflow, BinaryDbFileFamily::Plan],
        ),
        (
            BinaryDbCommandScope::ServerLand,
            &[BinaryDbFileFamily::Workflow, BinaryDbFileFamily::Content],
        ),
        (
            BinaryDbCommandScope::ServerRemoteSyncCommit,
            &[BinaryDbFileFamily::Content],
        ),
    ];

    for (scope, expected) in cases {
        for family in all_families {
            assert_eq!(
                scope.authorizes_file_family(family),
                expected.contains(&family),
                "{scope:?} authorization for {family:?}"
            );
        }
        for family in all_families {
            assert_eq!(
                scope.write_scope().includes_family(family),
                expected.contains(&family),
                "{scope:?} declared write scope for {family:?}"
            );
        }
    }
}

#[test]
fn composite_authorization_does_not_cross_declared_file_family_boundaries() {
    assert!(BinaryDbCommandScope::ServerTaskStart
        .write_scope()
        .is_subset_of(BinaryDbCommandScope::General.write_scope()));
    assert!(BinaryDbCommandScope::ServerTaskStart.authorizes(BinaryDbCommandScope::ServerPlan));
    assert!(BinaryDbCommandScope::ServerTaskStart.authorizes(BinaryDbCommandScope::ServerWorkflow));
    assert!(!BinaryDbCommandScope::ServerTaskStart.authorizes(BinaryDbCommandScope::ServerContent));
    assert!(!BinaryDbCommandScope::ServerTaskStart.authorizes(BinaryDbCommandScope::ServerQueue));
    assert!(!BinaryDbCommandScope::ServerTaskStart
        .authorizes(BinaryDbCommandScope::ServerRepositoryPack));
    assert!(BinaryDbCommandScope::ServerTaskStart.conflicts_with(BinaryDbCommandScope::ServerPlan));
    assert!(
        BinaryDbCommandScope::ServerTaskStart.conflicts_with(BinaryDbCommandScope::ServerWorkflow)
    );
    assert!(BinaryDbCommandScope::ServerTaskStart.conflicts_with(BinaryDbCommandScope::General));
    assert!(
        !BinaryDbCommandScope::ServerTaskStart.conflicts_with(BinaryDbCommandScope::ServerContent)
    );
    assert!(
        !BinaryDbCommandScope::ServerTaskStart.conflicts_with(BinaryDbCommandScope::ServerQueue)
    );
    assert!(!BinaryDbCommandScope::ServerTaskStart
        .conflicts_with(BinaryDbCommandScope::ServerRepositoryPack));

    assert!(BinaryDbCommandScope::ServerLand
        .write_scope()
        .is_subset_of(BinaryDbCommandScope::General.write_scope()));
    assert!(BinaryDbCommandScope::ServerRemoteSyncCommit
        .authorizes(BinaryDbCommandScope::ServerContent));
    assert!(!BinaryDbCommandScope::ServerRemoteSyncCommit
        .authorizes(BinaryDbCommandScope::ServerRepositoryPack));
    assert!(!BinaryDbCommandScope::ServerRemoteSyncCommit
        .authorizes(BinaryDbCommandScope::ServerWorkflow));
    assert!(!BinaryDbCommandScope::ServerRemoteSyncCommit
        .conflicts_with(BinaryDbCommandScope::ServerRepositoryPack));
    assert!(!BinaryDbCommandScope::ServerRemoteSyncCommit
        .conflicts_with(BinaryDbCommandScope::ServerWorkflow));
    assert!(BinaryDbCommandScope::ServerRemoteSyncCommit
        .conflicts_with(BinaryDbCommandScope::ServerContent));
}
