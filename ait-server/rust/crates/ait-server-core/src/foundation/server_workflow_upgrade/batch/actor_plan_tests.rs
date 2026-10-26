use super::*;
use crate::foundation::server_plan_binary_db::BinaryDbServerPlanService;
use crate::foundation::workflow_binary_v0_adapter::UserActorIdentity;

pub(super) fn plan_only_source(
    label: &str,
) -> (PathBuf, ServerBinaryLifecycleConfig, ServerBinaryActivation) {
    let (root, config, active) = fixture(label);
    let registry = ServerOperationalRepositoryRegistry::new(
        active.generation_root.join("global"),
        active.generation_root.join("repositories"),
    )
    .unwrap();
    for entry in registry.validate().unwrap() {
        let directory = active
            .generation_root
            .join("repositories")
            .join(entry.repository_index.to_string());
        for schema in SERVER_BINARY_DB_BIN_SCHEMAS
            .iter()
            .filter(|schema| schema.layout_id == 2)
        {
            fs::write(directory.join(schema.path), 2u32.to_le_bytes()).unwrap();
        }
        for name in ["task.bin", "snapshot.bin"] {
            fs::write(directory.join(name), 1u32.to_le_bytes()).unwrap();
        }
        fs::remove_file(directory.join("snapshot_link_payload.bin")).unwrap();
        if entry.repository_index == 0 {
            let db = FilesystemServerRemoteBinaryDb::serving_authority(
                RepoId::new("0"),
                RepoName::new(entry.repo_name.clone()),
                StorePath::new(directory.clone()),
                StoreGeneration::new(1),
            );
            let actor = BinaryDbServerWorkflowV0Store::new_frozen(db.clone())
                .resolve_user_actor(
                    &UserActorIdentity::new("fixture", "fixture@example.test").unwrap(),
                )
                .unwrap();
            BinaryDbServerPlanService::new(db).create_plan(&entry.repo_name,&json!({"actor_key_index":actor,"title":"Historical Plan","artifact_path":"docs/a.md","artifact_heading":"A",
                "items":[{"plan_item_ref":"a/item","text":"A","checkbox_state":"open","heading_path":["A"],"line_number":1}]})).unwrap();
        }
        for (name, width, old_width) in [("plan.bin", 52, 48), ("plan_revision.bin", 60, 56)] {
            let bytes = fs::read(directory.join(name)).unwrap();
            let old = bytes[4..]
                .chunks_exact(width)
                .map(|row| row[..old_width].to_vec())
                .collect::<Vec<_>>();
            fs::write(directory.join(name), record_file(1, &old)).unwrap();
        }
    }
    (root, config, active)
}

#[test]
fn actor_plan_conversion_reads_only_routing_headers_and_plan_pair() {
    let (root, config, active) = plan_only_source("actor-plan-only");
    let directory = active.generation_root.join("repositories/0");
    let plans = fs::read(directory.join("plan.bin")).unwrap();
    let revisions = fs::read(directory.join("plan_revision.bin")).unwrap();
    // These are deliberately unusable as a staged DB or full inventory source.
    let opaque_task = [1u32.to_le_bytes().as_slice(), &[255; 60]].concat();
    fs::write(directory.join("task.bin"), &opaque_task).unwrap();
    fs::remove_file(directory.join("plan_payload.bin")).unwrap();
    fs::create_dir(directory.join("plan_payload.bin")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            directory.join("plan.bin"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    admit(&config, &active).unwrap();
    let expected = convert_remote_plan_metadata(&plans, &revisions).unwrap();
    assert_eq!(fs::read(directory.join("plan.bin")).unwrap(), expected.0);
    assert_eq!(
        fs::read(directory.join("plan_revision.bin")).unwrap(),
        expected.1
    );
    assert_eq!(
        fs::read(directory.join("task.bin")).unwrap(),
        [2u32.to_le_bytes().as_slice(), &opaque_task[4..], &[0; 4]].concat()
    );
    assert!(directory.join("plan_payload.bin").is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(directory.join("plan.bin"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    // A target header skips body decoding; normal runtime owns domain validation.
    fs::write(directory.join("plan.bin"), [2, 0, 0, 0, 255]).unwrap();
    assert_eq!(admit(&config, &active).unwrap(), active);
    assert_eq!(
        fs::read(directory.join("plan.bin")).unwrap(),
        [2, 0, 0, 0, 255]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_plan_partial_files_and_cross_repository_profiles_fail_before_publication() {
    for complete_pair in [false, true] {
        let (root, config, active) = plan_only_source("actor-plan-partial");
        let first = active.generation_root.join("repositories/0/plan.bin");
        let before = fs::read(&first).unwrap();
        let directory = active.generation_root.join("repositories/1");
        fs::write(directory.join("plan.bin"), 2u32.to_le_bytes()).unwrap();
        if complete_pair {
            fs::write(directory.join("plan_revision.bin"), 2u32.to_le_bytes()).unwrap();
        }
        assert!(admit(&config, &active).is_err());
        assert_eq!(fs::read(first).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }
    let (root, config, active) = fixture("actor-plan-unsupported-combination");
    for index in 0..4 {
        for name in ["plan.bin", "plan_revision.bin"] {
            fs::write(
                active
                    .generation_root
                    .join("repositories")
                    .join(index.to_string())
                    .join(name),
                2u32.to_le_bytes(),
            )
            .unwrap();
        }
    }
    assert!(admit(&config, &active)
        .unwrap_err()
        .contains("Unsupported combined profile"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_plan_conversion_parses_complete_batch_before_writing_outputs() {
    let (root, config, active) = plan_only_source("actor-plan-truncated");
    let first = active.generation_root.join("repositories/0/plan.bin");
    let before = fs::read(&first).unwrap();
    fs::write(
        active
            .generation_root
            .join("repositories/3/plan_revision.bin"),
        [1, 0, 0, 0, 255],
    )
    .unwrap();
    assert!(admit(&config, &active).is_err());
    assert_eq!(fs::read(first).unwrap(), before);
    assert_eq!(fs::read_dir(&config.generations_root).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_plan_conversion_preserves_references_without_independent_domain_validation() {
    let (root, config, active) = plan_only_source("actor-plan-reference");
    let path = active.generation_root.join("repositories/0/plan.bin");
    let mut before = fs::read(&path).unwrap();
    before[4 + 12..4 + 16].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(&path, &before).unwrap();
    admit(&config, &active).unwrap();
    let after = fs::read(path).unwrap();
    assert_eq!(&after[4..52], &before[4..]);
    assert_eq!(&after[52..], &[0; 4]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_plan_conversion_skips_excluded_authority_files() {
    let (root, mut config, active) = plan_only_source("actor-plan-excluded");
    config.repository_scope =
        crate::foundation::server_repository_scope::ServerRepositoryScope::parse(Some("3"))
            .unwrap();
    let excluded = active.generation_root.join("repositories/3");
    for name in ["change.bin", "plan.bin", "plan_revision.bin"] {
        fs::remove_file(excluded.join(name)).unwrap();
    }
    admit(&config, &active).unwrap();
    assert!(!excluded.join("plan.bin").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_plan_conversion_rejects_unsupported_task_and_snapshot_headers() {
    for sentinel in ["task.bin", "snapshot.bin"] {
        let (root, config, active) = plan_only_source("actor-plan-source-sentinel");
        let directory = active.generation_root.join("repositories/1");
        let plan = active.generation_root.join("repositories/0/plan.bin");
        let before = fs::read(&plan).unwrap();
        fs::write(directory.join(sentinel), 2u32.to_le_bytes()).unwrap();
        assert!(admit(&config, &active)
            .unwrap_err()
            .contains("unsupported partial or mixed"));
        assert_eq!(fs::read(plan).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn actor_plan_conversion_propagates_replacement_io_failure() {
    let (root, config, active) = plan_only_source("actor-plan-publish-io");
    let directory = active.generation_root.join("repositories/0");
    let revision = fs::read(directory.join("plan_revision.bin")).unwrap();
    let error = config
        .with_exclusive_admission(|| {
            admit_with_hook(&config, &active, |phase| {
                if phase == "before_publish" {
                    fs::remove_file(directory.join("plan.bin")).unwrap();
                    fs::create_dir(directory.join("plan.bin")).unwrap();
                }
                Ok(())
            })
        })
        .unwrap_err();
    assert!(!error.is_empty());
    assert_eq!(
        fs::read(directory.join("plan_revision.bin")).unwrap(),
        revision
    );
    assert_eq!(fs::read_dir(&config.generations_root).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_workflow_conversion_handles_old_and_plan_only_complete_batches() {
    use crate::foundation::remote_actor_metadata::{
        convert_remote_workflow_file, RemoteActorProfile, RemoteWorkflowFile,
    };
    for plan_only in [false, true] {
        let (root, config, active) = plan_only_source("actor-workflow-batch");
        for index in 0..4 {
            let directory = active
                .generation_root
                .join("repositories")
                .join(index.to_string());
            if plan_only {
                let (plans, revisions) = convert_remote_plan_metadata(
                    &fs::read(directory.join("plan.bin")).unwrap(),
                    &fs::read(directory.join("plan_revision.bin")).unwrap(),
                )
                .unwrap();
                fs::write(directory.join("plan.bin"), plans).unwrap();
                fs::write(directory.join("plan_revision.bin"), revisions).unwrap();
            }
            for family in RemoteWorkflowFile::ALL {
                let mut bytes = family.source_layout().to_le_bytes().to_vec();
                bytes.extend(std::iter::repeat_n(
                    0xF0 | index as u8,
                    family.source_width() * 2,
                ));
                fs::write(directory.join(family.name()), bytes).unwrap();
            }
        }
        let directory = active.generation_root.join("repositories/0");
        let worker = fs::read(directory.join("worker_job.bin")).unwrap();
        let profile = if plan_only {
            RemoteActorProfile::PlanOnly
        } else {
            RemoteActorProfile::Old
        };
        let expected = RemoteWorkflowFile::ALL.map(|family| {
            let source = fs::read(directory.join(family.name())).unwrap();
            (
                family.name(),
                convert_remote_workflow_file(profile, family, &source)
                    .unwrap()
                    .unwrap(),
            )
        });
        admit(&config, &active).unwrap();
        for (name, bytes) in expected {
            assert_eq!(fs::read(directory.join(name)).unwrap(), bytes);
        }
        assert_eq!(fs::read(directory.join("worker_job.bin")).unwrap(), worker);
        // No body read or target proof on the subsequent target-profile admission.
        fs::write(directory.join("task.bin"), [2, 0, 0, 0, 255]).unwrap();
        admit(&config, &active).unwrap();
        assert_eq!(
            fs::read(directory.join("task.bin")).unwrap(),
            [2, 0, 0, 0, 255]
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn actor_workflow_truncated_last_authority_never_publishes_an_earlier_output() {
    use crate::foundation::remote_actor_metadata::RemoteWorkflowFile;
    for family in RemoteWorkflowFile::ALL {
        let (root, config, active) = plan_only_source("actor-workflow-truncated");
        let first = active.generation_root.join("repositories/0/task.bin");
        let before = fs::read(&first).unwrap();
        let path = active
            .generation_root
            .join("repositories/3")
            .join(family.name());
        let bytes = [family.source_layout().to_le_bytes().as_slice(), &[255]].concat();
        fs::write(path, bytes).unwrap();
        assert!(admit(&config, &active).unwrap_err().contains("truncated"));
        assert_eq!(fs::read(first).unwrap(), before);
        assert_eq!(fs::read_dir(&config.generations_root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
