use super::*;

#[test]
fn sbdh_fault_injector_targets_required_storage_stages() -> StoreResult<()> {
    #[derive(Clone, Copy)]
    enum Exercise {
        ReadRange,
        Append,
        Overwrite,
        SyncFile,
        Remove,
    }

    struct Case {
        label: &'static str,
        operation: BinaryDbTestStorageOperation,
        timing: BinaryDbTestFaultTiming,
        file_name: &'static str,
        exercise: Exercise,
    }

    let cases = [
        Case {
            label: "before header validation",
            operation: BinaryDbTestStorageOperation::ReadRange,
            timing: BinaryDbTestFaultTiming::Before,
            file_name: "header-before.bin",
            exercise: Exercise::ReadRange,
        },
        Case {
            label: "after header validation",
            operation: BinaryDbTestStorageOperation::ReadRange,
            timing: BinaryDbTestFaultTiming::After,
            file_name: "header-after.bin",
            exercise: Exercise::ReadRange,
        },
        Case {
            label: "payload append",
            operation: BinaryDbTestStorageOperation::AppendBytes,
            timing: BinaryDbTestFaultTiming::After,
            file_name: "payload.bin",
            exercise: Exercise::Append,
        },
        Case {
            label: "fixed-record append",
            operation: BinaryDbTestStorageOperation::AppendBytes,
            timing: BinaryDbTestFaultTiming::Before,
            file_name: "record.bin",
            exercise: Exercise::Append,
        },
        Case {
            label: "index append",
            operation: BinaryDbTestStorageOperation::AppendBytes,
            timing: BinaryDbTestFaultTiming::Before,
            file_name: "record.idx",
            exercise: Exercise::Append,
        },
        Case {
            label: "overwrite",
            operation: BinaryDbTestStorageOperation::OverwriteRange,
            timing: BinaryDbTestFaultTiming::After,
            file_name: "overwrite.bin",
            exercise: Exercise::Overwrite,
        },
        Case {
            label: "journal flush",
            operation: BinaryDbTestStorageOperation::SyncFile,
            timing: BinaryDbTestFaultTiming::After,
            file_name: "server-workflow.write.journal",
            exercise: Exercise::SyncFile,
        },
        Case {
            label: "data sync",
            operation: BinaryDbTestStorageOperation::SyncFile,
            timing: BinaryDbTestFaultTiming::After,
            file_name: "data.bin",
            exercise: Exercise::SyncFile,
        },
        Case {
            label: "journal cleanup",
            operation: BinaryDbTestStorageOperation::RemoveFile,
            timing: BinaryDbTestFaultTiming::After,
            file_name: "cleanup.write.journal",
            exercise: Exercise::Remove,
        },
    ];

    for case in cases {
        let root = make_temporary_root();
        fs::create_dir_all(root.as_path()).expect("create fault injector test root");
        let path = root.as_path().join(case.file_name);
        if matches!(
            case.exercise,
            Exercise::ReadRange | Exercise::Overwrite | Exercise::SyncFile | Exercise::Remove
        ) {
            fs::write(&path, b"seed").expect("seed fault injector test file");
        }
        let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
        store.arm(BinaryDbTestFault::once(
            case.operation,
            case.timing,
            case.file_name,
        ));

        let result = match case.exercise {
            Exercise::ReadRange => store.read_range(&path, 0, 1).map(|_| ()),
            Exercise::Append => store.append_bytes(&path, b"x").map(|_| ()),
            Exercise::Overwrite => store.overwrite_range(&path, 0, b"x"),
            Exercise::SyncFile => store.sync_file(&path),
            Exercise::Remove => store.remove_file_if_exists(&path),
        };
        let error = match result {
            Ok(()) => panic!("{} fault should fail its storage operation", case.label),
            Err(error) => error,
        };
        assert_eq!(error.kind(), BinaryDbErrorKind::Io, "{}", case.label);
        assert_eq!(store.fired_fault_count(), 1, "{}", case.label);
        assert!(store.events().iter().any(|event| {
            event.operation == case.operation
                && event.timing == case.timing
                && event.path.ends_with(case.file_name)
        }));
    }
    Ok(())
}

#[test]
fn sbdh_fault_injected_transaction_failures_restore_pre_transaction_bytes() -> StoreResult<()> {
    struct Case {
        label: &'static str,
        fault: BinaryDbTestFault,
    }

    let cases = [
        Case {
            label: "record append",
            fault: BinaryDbTestFault::once(
                BinaryDbTestStorageOperation::AppendBytes,
                BinaryDbTestFaultTiming::After,
                "task.bin",
            ),
        },
        Case {
            label: "data sync",
            fault: BinaryDbTestFault::once(
                BinaryDbTestStorageOperation::SyncFile,
                BinaryDbTestFaultTiming::After,
                "task.bin",
            ),
        },
        Case {
            label: "journal cleanup",
            fault: BinaryDbTestFault::once(
                BinaryDbTestStorageOperation::RemoveFile,
                BinaryDbTestFaultTiming::After,
                BinaryDbCommandScope::ServerWorkflow.journal_file_name(),
            ),
        },
    ];

    for case in cases {
        let authority_root = make_temporary_root();
        let root = authority_root.as_path().to_path_buf();
        fs::create_dir_all(&root).expect("create transaction fault test root");
        let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
        let db = FilesystemServerRemoteBinaryDb::with_file_store(
            store.clone(),
            RepoId::new("repo-uuid-001"),
            RepoName::new("repo-name"),
            authority_root,
            StoreGeneration::new(7),
            ServerBinaryDbAuthorityMode::TestFixture,
        );
        let before = capture_binary_db_files(&root, &["task.bin"])?;
        store.arm(case.fault);

        {
            let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
            let record = vec![7_u8; task_file_id().record_size() as usize];
            let append_result = tx.append_record(task_file_id(), &record);
            if append_result.is_ok() {
                tx.commit()
                    .expect_err("commit-stage injected fault should fail commit");
            } else {
                append_result.expect_err("append-stage injected fault should fail append");
            }
        }

        assert_eq!(store.fired_fault_count(), 1, "{}", case.label);
        assert_binary_db_files_unchanged(&root, &before);
        assert_binary_db_path_missing(
            &root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name()),
        );
    }
    Ok(())
}

#[test]
fn sbdh_aggregate_write_sets_and_visibility_are_explicit() {
    for (aggregate, families) in [
        ("land", SERVER_LAND_AGGREGATE_WRITE_FAMILIES),
        ("zstd_bulk", SERVER_ZSTD_BULK_AGGREGATE_WRITE_FAMILIES),
    ] {
        assert!(families.iter().all(|row| row.aggregate == aggregate));
        assert!(families
            .iter()
            .any(|row| row.authority == ServerBinaryDbAuthorityClass::CanonicalRepository));
        assert!(families
            .iter()
            .all(|row| !row.family.trim().is_empty() && !row.files.is_empty()));

        for phase in SERVER_BINARY_DB_AGGREGATE_VISIBILITY_CONTRACT {
            let expected_visible_files = if phase.externally_visible_new_state {
                families
                    .iter()
                    .filter(|row| row.mutable_during_commit)
                    .flat_map(|row| row.files.iter().copied())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            if phase.phase == ServerBinaryDbAggregateVisibility::AfterCommit {
                assert!(!expected_visible_files.is_empty(), "{aggregate}");
            } else {
                assert!(expected_visible_files.is_empty(), "{aggregate}");
            }
            assert!(!phase.expected_state.trim().is_empty());
        }
    }

    assert!(SERVER_LAND_AGGREGATE_WRITE_FAMILIES
        .iter()
        .filter(|row| row.authority == ServerBinaryDbAuthorityClass::CanonicalRepository)
        .any(|row| row.family == "canonical_line"));
    let immutable_packs = SERVER_ZSTD_BULK_AGGREGATE_WRITE_FAMILIES
        .iter()
        .filter(|row| row.authority == ServerBinaryDbAuthorityClass::ImmutableRepositoryPack)
        .collect::<Vec<_>>();
    assert_eq!(immutable_packs.len(), 2);
    assert!(immutable_packs.iter().all(|row| !row.mutable_during_commit));
}

#[test]
fn sbdh_zstd_and_land_retry_lifecycle_contracts_are_closed() {
    assert_eq!(SERVER_ZSTD_PACK_LIFECYCLE_TRANSITIONS.len(), 4);
    assert_eq!(
        SERVER_ZSTD_PACK_LIFECYCLE_TRANSITIONS
            .iter()
            .filter(|transition| transition.appends_payload)
            .collect::<Vec<_>>(),
        vec![&ServerZstdPackLifecycleTransition {
            from: ServerZstdPackLifecycleState::Missing,
            to: ServerZstdPackLifecycleState::Uploaded,
            idempotent: false,
            appends_payload: true,
        }]
    );
    assert!(SERVER_ZSTD_PACK_LIFECYCLE_TRANSITIONS
        .iter()
        .any(|transition| {
            transition.from == ServerZstdPackLifecycleState::Uploaded
                && transition.to == ServerZstdPackLifecycleState::Ready
                && !transition.appends_payload
        }));
    assert!(!SERVER_ZSTD_PACK_LIFECYCLE_TRANSITIONS
        .iter()
        .any(|transition| {
            transition.from == ServerZstdPackLifecycleState::Missing
                && transition.to == ServerZstdPackLifecycleState::Ready
        }));

    assert_eq!(SERVER_LAND_RETRY_CONTRACT.len(), 4);
    for state in [
        ServerLandRetryState::Missing,
        ServerLandRetryState::Complete,
        ServerLandRetryState::IncompleteRecoverable,
        ServerLandRetryState::Conflicting,
    ] {
        let row = SERVER_LAND_RETRY_CONTRACT
            .iter()
            .find(|row| row.state == state)
            .unwrap_or_else(|| panic!("missing land retry state {state:?}"));
        assert!(!row.action.trim().is_empty());
    }
}

#[test]
fn sbdh_followup_test_inventory_covers_every_hardening_card() {
    let expected_refs = [
        "SBDH-01", "SBDH-02", "SBDH-03", "SBDH-04", "SBDH-05", "SBDH-06", "SBDH-07", "SBDH-08",
        "SBDH-09",
    ];
    assert_eq!(SERVER_BINARY_DB_FOLLOWUP_TEST_INVENTORY.len(), 9);
    for sprint_ref in expected_refs {
        let row = SERVER_BINARY_DB_FOLLOWUP_TEST_INVENTORY
            .iter()
            .find(|row| row.sprint_ref == sprint_ref)
            .unwrap_or_else(|| panic!("missing follow-up test inventory for {sprint_ref}"));
        assert!(!row.required_test.trim().is_empty());
    }
}

#[test]
fn sbdh_recovery_is_idempotent_and_preserves_committed_bytes() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("create idempotent recovery test root");
    let record_file = task_file_id();
    let payload_file = task_payload_file_id();
    let index_file = task_change_index_id();
    let first_record = vec![1_u8; record_file.record_size() as usize];
    let second_record = vec![2_u8; record_file.record_size() as usize];

    let mut committed = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    committed.append_payload(payload_file.clone(), b"first")?;
    committed.append_record(record_file.clone(), &first_record)?;
    committed.append_index_candidate(index_file.clone(), b"task", 0)?;
    committed.commit()?;

    let tracked_files = [
        record_file.as_str(),
        payload_file.as_str(),
        index_file.as_str(),
    ];
    let before = capture_binary_db_files(&root, &tracked_files)?;

    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    interrupted.append_payload(payload_file.clone(), b"second")?;
    interrupted.append_record(record_file.clone(), &second_record)?;
    interrupted.append_index_candidate(index_file.clone(), b"task", 1)?;
    std::mem::forget(interrupted);
    fs::remove_file(server_workflow_write_lock_path(&root))
        .expect("operator removes stale lock after writer death");

    let mut first_recovery = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    first_recovery.abort()?;
    assert_binary_db_files_unchanged(&root, &before);

    let mut second_recovery = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    second_recovery.abort()?;
    assert_binary_db_files_unchanged(&root, &before);
    assert_binary_db_path_missing(
        &root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name()),
    );
    Ok(())
}
