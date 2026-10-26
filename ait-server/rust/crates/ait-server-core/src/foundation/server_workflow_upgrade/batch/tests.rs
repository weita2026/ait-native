use super::*;
use crate::foundation::server_binary_db_schema_registry::{
    SERVER_REPOSITORY_OPERATIONAL_BIN_SCHEMAS, SERVER_REPOSITORY_OPERATIONAL_INDEX_SCHEMAS,
};
use crate::foundation::server_content_binary_db::ServerBinaryDbLineStore;
use crate::foundation::server_workflow_store::ServerWorkflowTaskStore;
use crate::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store;
use serde_json::json;
#[path = "actor_plan_tests.rs"]
mod actor_plan;
#[path = "progress_tests.rs"]
mod progress;

fn fixture(label: &str) -> (PathBuf, ServerBinaryLifecycleConfig, ServerBinaryActivation) {
    let root = std::env::temp_dir().join(format!(
        "ait-workflow-batch-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config =
        ServerBinaryLifecycleConfig::from_server_data_root(root.join("server-data")).unwrap();
    let active=config.ensure_fresh_activation(|generation|{
        fs::create_dir(generation).map_err(|error|error.to_string())?;fs::create_dir(generation.join("global")).map_err(|error|error.to_string())?;fs::create_dir(generation.join("repositories")).map_err(|error|error.to_string())?;
        let registry=ServerOperationalRepositoryRegistry::new(generation.join("global"),generation.join("repositories")).map_err(|error|error.to_string())?;
        let entries=registry.initialize_fresh(100).map_err(|error|error.to_string())?;
        for entry in &entries {
            let directory=generation.join("repositories").join(entry.repository_index.to_string());
            for schema in SERVER_BINARY_DB_BIN_SCHEMAS.iter().chain(SERVER_REPOSITORY_OPERATIONAL_BIN_SCHEMAS) {write_new_sync(&directory.join(schema.path),&1u32.to_le_bytes())?;}
            for schema in SERVER_BINARY_DB_INDEX_SCHEMAS.iter().chain(SERVER_REPOSITORY_OPERATIONAL_INDEX_SCHEMAS) {write_new_sync(&directory.join(schema.path),&1u32.to_le_bytes())?;}
            write_new_sync(&directory.join("snapshot_link_payload.bin"),&1u32.to_le_bytes())?;
        }
        let manifest=serde_json::to_vec(&json!({"schema":"ait.server.binary_v0.operational_generation.v1","layout_id":1,"status":"validated_inactive","global_registry":"global","repository_authorities":"repositories","repository_count":entries.len()})).unwrap();
        write_new_sync(&generation.join("generation.json"),&manifest)?;
        write_new_sync(&generation.join("generation-complete.json"),&serde_json::to_vec(&json!({"schema":"ait.server.binary_v0.fresh.complete.v1","layout_id":1,"status":"validated_inactive","generation_manifest_sha256":sha256(&manifest),"repository_count":entries.len()})).unwrap())?;Ok(())
    }).unwrap();
    (root, config, active)
}
fn admit(
    config: &ServerBinaryLifecycleConfig,
    active: &ServerBinaryActivation,
) -> Result<ServerBinaryActivation, String> {
    config.with_exclusive_admission(|| admit_workflow_generation(config, active))
}

#[test]
fn metadata_batch_keeps_content_roots_and_unchanged_files_in_place() {
    let (root, config, source) = fixture("metadata-only");
    let pointer = fs::read(&config.activation_pointer).unwrap();
    let packs = source
        .generation_root
        .join("repositories/0/.ait/objects/packs");
    fs::create_dir_all(&packs).unwrap();
    let pack = packs.join("existing-content.zstpack");
    fs::write(
        &pack,
        b"unchanged content is outside the metadata migration",
    )
    .unwrap();
    let unchanged = source.generation_root.join("repositories/0/line.bin");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            source.generation_root.join("repositories/0/change.bin"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let unchanged_bytes = fs::read(&unchanged).unwrap();
    #[cfg(unix)]
    let unchanged_inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(&unchanged).unwrap().ino()
    };
    let before = inventory(&source.generation_root).unwrap();
    assert!(!before
        .iter()
        .any(|file| file.relative_path.contains(".ait/")));
    let target = admit(&config, &source).unwrap();
    assert_eq!(target, source);
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    assert_eq!(
        fs::read(&pack).unwrap(),
        b"unchanged content is outside the metadata migration"
    );
    assert_eq!(fs::read(&unchanged).unwrap(), unchanged_bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(&unchanged).unwrap().ino(), unchanged_inode);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(source.generation_root.join("repositories/0/change.bin"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    for index in 0..4 {
        let directory = target
            .generation_root
            .join("repositories")
            .join(index.to_string());
        assert_eq!(
            profile(&read_authority_files(&directory, false).unwrap()).unwrap(),
            WorkflowProfile::Current
        );
        assert!(!directory.join("snapshot_link_payload.bin").exists());
    }
    let current = inventory(&source.generation_root).unwrap();
    assert_ne!(current, before);
    assert_eq!(admit(&config, &target).unwrap(), target);
    assert_eq!(inventory(&source.generation_root).unwrap(), current);
    assert_eq!(fs::read_dir(&config.generations_root).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn partial_profiles_and_unadmitted_authorities_never_switch_any_repository() {
    let (root, config, source) = fixture("partial");
    let pointer = fs::read(&config.activation_pointer).unwrap();
    fs::write(
        source.generation_root.join("repositories/3/change.bin"),
        2u32.to_le_bytes(),
    )
    .unwrap();
    assert!(admit(&config, &source)
        .unwrap_err()
        .to_ascii_lowercase()
        .contains("partial"));
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    fs::write(
        source.generation_root.join("repositories/3/change.bin"),
        1u32.to_le_bytes(),
    )
    .unwrap();
    fs::create_dir(source.generation_root.join("repositories/999")).unwrap();
    assert!(admit(&config, &source).is_err());
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn normal_admission_derives_first_old_c01_without_title_or_producer_filters() {
    let (root, config, source) = fixture("held-cohort");
    let directory = source.generation_root.join("repositories/0");
    let db = FilesystemServerRemoteBinaryDb::serving_authority(
        RepoId::new("0"),
        RepoName::new("ait-core"),
        StorePath::new(directory.clone()),
        StoreGeneration::new(1),
    );
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    store
        .resolve_user_actor(
            &crate::foundation::workflow_binary_v0_adapter::UserActorIdentity::new(
                "fixture",
                "fixture@example.test",
            )
            .unwrap(),
        )
        .unwrap();
    fs::write(directory.join("task.bin"), 2u32.to_le_bytes()).unwrap();
    store
        .create_task(
            "ait-core",
            &json!({"actor_key_index":0,"title":"Initial work","intent":"Retain pending disposition"}),
        )
        .unwrap();
    let bytes = fs::read(directory.join("task.bin")).unwrap();
    fs::write(
        directory.join("task.bin"),
        [1u32.to_le_bytes().as_slice(), &bytes[4..64]].concat(),
    )
    .unwrap();
    let line = ServerBinaryDbLineStore::<_, 1>::new(db)
        .create_line("main", 0, 100)
        .unwrap();
    let title = WorkflowBinaryV0Codec::encode_single_text_payload(
        "admitted shape is insufficient without the cohort",
        "title",
    )
    .unwrap();
    let mut payload = 1u32.to_le_bytes().to_vec();
    payload.extend_from_slice(&title);
    fs::write(directory.join("change_payload.bin"), payload).unwrap();
    let task = WorkflowBinaryV0Codec::decode_task_prefix(
        &records(
            &read_authority_files(&directory, false).unwrap(),
            "task.bin",
            60,
        )
        .unwrap()[0],
    )
    .unwrap();
    let change = WorkflowBinaryV0Codec::encode_change_prefix(V0ChangeRecord {
        payload_len: title.len() as u16,
        payload_offset: 4,
        task_index: 0,
        base_line_index_plus1: line + 1,
        created_at_s: task.created_at_s,
        updated_at_s: task.created_at_s,
        ..V0ChangeRecord::default()
    })
    .unwrap();
    fs::write(directory.join("change.bin"), record_file(1, &[change])).unwrap();
    fs::write(
        directory.join("task_change_index.bin"),
        index_file(1, &[(0, 0, 0, 0)], true).unwrap(),
    )
    .unwrap();
    for name in [
        "change_snapshot_index.bin",
        "change_land_index.bin",
        "change_patchset_index.bin",
    ] {
        fs::write(
            directory.join(name),
            index_file(1, &[], name != "change_snapshot_index.bin").unwrap(),
        )
        .unwrap();
    }
    let before = inventory(&source.generation_root).unwrap();
    let admitted = WorkflowGenerationAdmission {
        source_fingerprint: inventory_fingerprint(&before).unwrap(),
        initial_changes_by_repository: (0..4)
            .map(|i| {
                (
                    i,
                    if i == 0 {
                        BTreeSet::from([0])
                    } else {
                        BTreeSet::new()
                    },
                )
            })
            .collect(),
        incoming_changes_by_repository: (0..4).map(|i| (i, BTreeSet::new())).collect(),
        historical_dispositions_by_repository: (0..4).map(|i| (i, Default::default())).collect(),
    };
    let stale = WorkflowGenerationAdmission {
        source_fingerprint: "0".repeat(64),
        initial_changes_by_repository: admitted.initial_changes_by_repository.clone(),
        incoming_changes_by_repository: admitted.incoming_changes_by_repository.clone(),
        historical_dispositions_by_repository: admitted
            .historical_dispositions_by_repository
            .clone(),
    };
    assert!(config
        .with_exclusive_admission(|| admit_workflow_generation_with_admission(
            &config, &source, &stale
        ))
        .is_err());
    assert_eq!(inventory(&source.generation_root).unwrap(), before);
    let target = admit(&config, &source).unwrap();
    assert_eq!(target, source);
    assert!(records(
        &read_authority_files(&directory, false).unwrap(),
        "change.bin",
        68
    )
    .unwrap()
    .is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn metadata_batch_preserves_nonserving_authorities_without_decoding_them() {
    use crate::foundation::operational_binary_v0::{
        ServerOperationalBinaryV0Codec, OPERATIONAL_REPOSITORY_RECORD_SIZE,
        REPOSITORY_LIFECYCLE_PURGED, REPOSITORY_META_TOMBSTONED,
    };
    for mode in 0..3 {
        let (root, mut config, source) = fixture("nonserving-scope");
        ServerOperationalRepositoryRegistry::new(
            source.generation_root.join("global"),
            source.generation_root.join("repositories"),
        )
        .unwrap()
        .append_repository(
            &crate::foundation::server_operational_repository_registry::RepositoryCreateSpec {
                repo_name: "nonserving-fixture".into(),
                namespace_ascii: [0, 0],
                policy_flags: 0x83,
                created_at_s: 100,
            },
        )
        .unwrap();
        let registry_path = source.generation_root.join("global/repository.bin");
        let mut bytes = fs::read(&registry_path).unwrap();
        let start = 4 + 4 * OPERATIONAL_REPOSITORY_RECORD_SIZE as usize;
        let end = start + OPERATIONAL_REPOSITORY_RECORD_SIZE as usize;
        let mut record =
            ServerOperationalBinaryV0Codec::decode_repository(&bytes[start..end]).unwrap();
        if mode == 1 {
            record.repository_meta |= REPOSITORY_META_TOMBSTONED;
        } else if mode == 0 {
            record.lifecycle_kind = REPOSITORY_LIFECYCLE_PURGED;
        } else {
            config.repository_scope =
                crate::foundation::server_repository_scope::ServerRepositoryScope::parse(Some("4"))
                    .unwrap();
        }
        bytes[start..end]
            .copy_from_slice(&ServerOperationalBinaryV0Codec::encode_repository(record).unwrap());
        fs::write(&registry_path, &bytes).unwrap();
        let opaque = source
            .generation_root
            .join("repositories/4/snapshot_link.bin");
        fs::write(&opaque, b"Opaque nonserving authority").unwrap();
        let before = inventory(&source.generation_root).unwrap();
        let pointer = fs::read(&config.activation_pointer).unwrap();
        let target = admit(&config, &source).unwrap();
        assert_eq!(target, source);
        assert_eq!(fs::read(&registry_path).unwrap(), bytes);
        assert_eq!(fs::read(&opaque).unwrap(), b"Opaque nonserving authority");
        assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
        for file in before
            .iter()
            .filter(|file| file.relative_path.starts_with("repositories/4/"))
        {
            assert!(inventory(&source.generation_root).unwrap().contains(file));
        }
        assert_eq!(admit(&config, &target).unwrap(), target);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn unknown_runtime_exclusions_reject_before_metadata_writes() {
    let (root, mut config, source) = fixture("unknown-exclusion");
    config.repository_scope =
        crate::foundation::server_repository_scope::ServerRepositoryScope::parse(Some("99"))
            .unwrap();
    let before = inventory(&source.generation_root).unwrap();
    let pointer = fs::read(&config.activation_pointer).unwrap();
    let error = admit(&config, &source).unwrap_err();
    assert!(error.contains("unknown Repository index 99"), "{error}");
    assert_eq!(inventory(&source.generation_root).unwrap(), before);
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepublication_errors_discard_only_temporary_metadata() {
    for stop in ["after_stage", "before_publish"] {
        let (root, config, source) = fixture(stop);
        let before = inventory(&source.generation_root).unwrap();
        let pointer = fs::read(&config.activation_pointer).unwrap();
        let error = config
            .with_exclusive_admission(|| {
                admit_with_hook(&config, &source, |phase| {
                    if phase == stop {
                        Err(format!("injected {phase}"))
                    } else {
                        Ok(())
                    }
                })
            })
            .unwrap_err();
        assert!(error.contains("injected"), "{error}");
        assert_eq!(inventory(&source.generation_root).unwrap(), before);
        assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
        assert_eq!(fs::read_dir(&config.generations_root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn historical_and_plan_inputs_validate_before_metadata_replacement() {
    let (root, config, source) = fixture("historical-sealed-recovery");
    let (fixture_root, request, mut files) =
        super::super::tests::historical_disposition::pseudo_fixture("batch-pseudo-source");
    let registry = ServerOperationalRepositoryRegistry::new(
        source.generation_root.join("global"),
        source.generation_root.join("repositories"),
    )
    .unwrap();
    let namespace = registry.validate().unwrap()[0].record.namespace_ascii;
    let namespace = std::str::from_utf8(&namespace)
        .unwrap()
        .trim_end_matches('\0');
    let task_id = format!("R{namespace}T-0002");
    let mut patchsets = records(&files, "patchset.bin", 65).unwrap();
    let raw = payload(
        &files,
        "patchset_summary_payload.bin",
        u64_at(&patchsets[0], 51),
        u16_at(&patchsets[0], 59) as usize,
    )
    .unwrap();
    let summary = std::str::from_utf8(raw)
        .unwrap()
        .replace("\"T-0002", &format!("\"{task_id}"));
    let bytes = files.get_mut("patchset_summary_payload.bin").unwrap();
    set_u64(&mut patchsets[0], 51, bytes.len() as u64);
    patchsets[0][59..61].copy_from_slice(&(summary.len() as u16).to_le_bytes());
    bytes.extend_from_slice(summary.as_bytes());
    files.insert("patchset.bin".into(), record_file(1, &patchsets));
    let directory = source.generation_root.join("repositories/0");
    for (name, bytes) in &files {
        fs::write(directory.join(name), bytes).unwrap();
    }
    let before = inventory(&source.generation_root).unwrap();
    let pointer = fs::read(&config.activation_pointer).unwrap();
    let approved = history::derive_approved_disposition(&files).unwrap();
    assert_eq!(approved.retired_pseudo_land_indices, BTreeSet::from([0]));
    assert_eq!(approved.retained_terminal_task_indices, BTreeSet::from([1]));
    assert_eq!(inventory(&source.generation_root).unwrap(), before);
    // A later real receipt resolves the terminal gap without changing the
    // approved retirement of the imported receipt.
    let mut paired = files.clone();
    let mut patchsets = records(&paired, "patchset.bin", 65).unwrap();
    let mut actual = patchsets[0].clone();
    actual[1] = 1;
    set_u32(&mut actual, 8, 1);
    set_u32(&mut actual, 12, 1);
    let summary = b"A later actual accepted candidate";
    let bytes = paired.get_mut("patchset_summary_payload.bin").unwrap();
    set_u64(&mut actual, 51, bytes.len() as u64);
    actual[59..61].copy_from_slice(&(summary.len() as u16).to_le_bytes());
    bytes.extend_from_slice(summary);
    patchsets.push(actual);
    paired.insert("patchset.bin".into(), record_file(1, &patchsets));
    let mut lands = records(&paired, "land.bin", 48).unwrap();
    let mut actual = lands[0].clone();
    actual[1] = 1;
    set_u32(&mut actual, 8, 1);
    set_u32(&mut actual, 12, 1);
    set_u32(&mut actual, 16, 1);
    let time = u64_at(&actual, 28) + 12;
    set_u64(&mut actual, 28, time);
    set_u64(&mut actual, 36, time);
    lands.push(actual);
    paired.insert("land.bin".into(), record_file(1, &lands));
    for (name, ordinal) in [
        ("task_patchset_index.bin", false),
        ("change_patchset_index.bin", true),
        ("task_land_index.bin", false),
        ("change_land_index.bin", true),
    ] {
        paired.insert(
            name.into(),
            index_file(2, &[(0, 1, 0, 0), (1, 1, 1, 1)], ordinal).unwrap(),
        );
    }
    let facts = history::derive_approved_disposition(&paired).unwrap();
    assert!(facts.retained_terminal_task_indices.is_empty());
    assert_eq!(facts.retired_pseudo_land_indices, BTreeSet::from([0]));
    assert_eq!(inventory(&source.generation_root).unwrap(), before);
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    let admission = WorkflowGenerationAdmission {
        source_fingerprint: inventory_fingerprint(&before).unwrap(),
        initial_changes_by_repository: (0..4)
            .map(|i| {
                (
                    i,
                    if i == 0 {
                        request.initial_change_indices.clone()
                    } else {
                        BTreeSet::new()
                    },
                )
            })
            .collect(),
        incoming_changes_by_repository: (0..4).map(|i| (i, BTreeSet::new())).collect(),
        historical_dispositions_by_repository: (0..4)
            .map(|i| {
                (
                    i,
                    if i == 0 {
                        request.historical_disposition.clone()
                    } else {
                        Default::default()
                    },
                )
            })
            .collect(),
    };
    let mut wrong_tasks = records(&files, "task.bin", 60).unwrap();
    set_u32(&mut wrong_tasks[1], 16, 2);
    fs::write(directory.join("task.bin"), record_file(1, &wrong_tasks)).unwrap();
    let invalid_source = inventory(&source.generation_root).unwrap();
    let invalid = WorkflowGenerationAdmission {
        source_fingerprint: inventory_fingerprint(&invalid_source).unwrap(),
        initial_changes_by_repository: admission.initial_changes_by_repository.clone(),
        incoming_changes_by_repository: admission.incoming_changes_by_repository.clone(),
        historical_dispositions_by_repository: admission
            .historical_dispositions_by_repository
            .clone(),
    };
    let error = config
        .with_exclusive_admission(|| {
            admit_workflow_generation_with_admission(&config, &source, &invalid)
        })
        .unwrap_err();
    assert!(
        error.contains("outside its bound revision range"),
        "{error}"
    );
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    assert_eq!(inventory(&source.generation_root).unwrap(), invalid_source);
    assert!(!config
        .generations_root
        .join(format!("workflow-{}", invalid.source_fingerprint))
        .exists());
    assert!(!config
        .generations_root
        .join(format!(".workflow-{}.staging", invalid.source_fingerprint))
        .exists());
    fs::write(directory.join("task.bin"), &files["task.bin"]).unwrap();
    let mut wrong = WorkflowGenerationAdmission {
        source_fingerprint: admission.source_fingerprint.clone(),
        initial_changes_by_repository: admission.initial_changes_by_repository.clone(),
        incoming_changes_by_repository: admission.incoming_changes_by_repository.clone(),
        historical_dispositions_by_repository: admission
            .historical_dispositions_by_repository
            .clone(),
    };
    wrong
        .historical_dispositions_by_repository
        .get_mut(&0)
        .unwrap()
        .retained_terminal_task_indices
        .clear();
    assert!(config
        .with_exclusive_admission(|| admit_workflow_generation_with_admission(
            &config, &source, &wrong
        ))
        .is_err());
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    assert_eq!(inventory(&source.generation_root).unwrap(), before);
    let target = admit(&config, &source).unwrap();
    assert_eq!(target, source);
    assert_ne!(inventory(&source.generation_root).unwrap(), before);
    let db = FilesystemServerRemoteBinaryDb::serving_authority(
        RepoId::new("0"),
        RepoName::new("ait-core"),
        StorePath::new(target.generation_root.join("repositories/0")),
        StoreGeneration::new(1),
    );
    validate_frozen_server_workflow_v0(&db).unwrap();
    let store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db, namespace).unwrap();
    let task = store.get_task(Some("ait-core"), &task_id).unwrap();
    assert_eq!(task["status"], "completed");
    assert!(task["successful_land"].is_null());
    assert_eq!(admit(&config, &target).unwrap(), target);
    fs::remove_dir_all(fixture_root).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_drift_and_concurrent_startup_stop_before_pointer_effects() {
    let (root, config, source) = fixture("drift");
    let pointer = fs::read(&config.activation_pointer).unwrap();
    let file = source
        .generation_root
        .join("repositories/2/task_payload.bin");
    let before = fs::read(&file).unwrap();
    let error = config
        .with_exclusive_admission(|| {
            admit_with_hook(&config, &source, |phase| {
                if phase == "after_stage" {
                    let mut changed = before.clone();
                    changed.push(0);
                    fs::write(&file, changed).unwrap();
                }
                Ok(())
            })
        })
        .unwrap_err();
    assert!(error.contains("drifted"), "{error}");
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    fs::write(&file, before).unwrap();
    config
        .with_exclusive_admission(|| {
            let other = config.clone();
            let busy = std::thread::spawn(move || other.with_exclusive_admission(|| Ok(())))
                .join()
                .unwrap()
                .unwrap_err();
            assert!(busy.contains("busy"));
            admit_workflow_generation(&config, &source)
        })
        .unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "read-only validation of the existing sealed actual-source metadata copy"]
fn actual_source_approved_dispositions_validate_without_source_mutation() {
    let root = canonical_real_directory(&PathBuf::from(
        std::env::var_os("AIT_WORKFLOW_LEGACY_COPY").expect("set isolated metadata copy"),
    ))
    .unwrap();
    assert!(
        root.starts_with("/private/tmp") || root.starts_with("/tmp"),
        "use an isolated temporary copy"
    );
    let before = inventory(&root).unwrap();
    let registry =
        ServerOperationalRepositoryRegistry::new(root.join("global"), root.join("repositories"))
            .unwrap();
    let mut totals = [0usize; 5];
    for entry in registry.validate().unwrap() {
        // The owner excludes this retired project from the assessed source.
        // This test scope is not a runtime registry exception or mutation.
        if entry.repo_name == "ait" {
            continue;
        }
        let directory = root
            .join("repositories")
            .join(entry.repository_index.to_string());
        let files = read_authority_files(&directory, false).unwrap();
        history::validate_source_namespace(&files, &entry.record.namespace_ascii)
            .unwrap_or_else(|e| panic!("Repository {} namespace: {e}", entry.repository_index));
        history::validate_source_plan_bindings(&directory, &files)
            .unwrap_or_else(|e| panic!("Repository {} Plan: {e}", entry.repository_index));
        let facts = history::derive_approved_disposition(&files).unwrap_or_else(|e| {
            panic!("Repository {} source history: {e}", entry.repository_index)
        });
        totals[0] += 1;
        totals[1] += facts.legacy_history_patchset_indices.len();
        totals[2] += facts.retired_pseudo_land_indices.len();
        totals[3] += facts.retained_terminal_task_indices.len();
        totals[4] += facts.retired_duplicate_land_indices.len();
    }
    assert_eq!(inventory(&root).unwrap(), before);
    assert_eq!(totals, [52, 1347, 916, 564, 1]);
    println!("actual sealed metadata admission derivation: {} in-scope repositories, {} typed Patchsets, {} approved pseudo Lands, {} terminal/no-live-receipt Tasks, {} later exact duplicate; source unchanged", totals[0], totals[1], totals[2], totals[3], totals[4]);
}
