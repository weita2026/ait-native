#![cfg(test)]

use super::*;
use ait_server_core::foundation::remote_binary_db::{
    BinaryDbReadLockSet, ServerBinaryDbFilesystemStore, ServerBinaryDbLockMode,
    ServerBinaryDbLockStore, ServerBinaryDbLockWait,
};
use ait_server_core::foundation::server_operational_repository_registry::REGISTRY_LOCK_FILE_NAME;
use ait_server_core::foundation::server_operational_worker_jobs::WORKER_QUEUE_LOCK_FILE_NAME;
use ait_server_core::foundation::server_workflow_store::{
    ServerWorkflowChangeStore, ServerWorkflowTaskStore,
};
use ait_server_core::foundation::server_workflow_upgrade::workflow_generation_fingerprint;
use ait_server_core::foundation::workflow_binary_v0::{
    WorkflowBinaryV0Codec, LAND_STATUS_MASK, LAND_STATUS_SUCCEEDED, LAND_TOMBSTONE,
    TASK_META_COMPLETED,
};
use std::env;
use std::time::Instant;

fn copy_metadata(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for item in fs::read_dir(source).unwrap() {
        let item = item.unwrap();
        let name = item.file_name();
        if name == ".ait" || name == ".locks" || name.to_string_lossy().ends_with(".lock") {
            continue;
        }
        let metadata = fs::symlink_metadata(item.path()).unwrap();
        assert!(!metadata.file_type().is_symlink());
        let destination = target.join(name);
        if metadata.is_dir() {
            copy_metadata(&item.path(), &destination);
        } else {
            assert!(metadata.is_file());
            fs::copy(item.path(), destination).unwrap();
        }
    }
}

fn rows(root: &Path, name: &str, width: usize) -> Vec<Vec<u8>> {
    let bytes = fs::read(root.join(name)).unwrap();
    assert_eq!((bytes.len() - 4) % width, 0);
    bytes[4..].chunks_exact(width).map(<[u8]>::to_vec).collect()
}

#[test]
#[ignore = "requires the existing coherent scoped metadata source"]
fn actor_plan_actual_source_file_conversion() {
    use ait_server_core::foundation::server_binary_db_schema_registry::SERVER_BINARY_DB_BIN_SCHEMAS;
    use std::io::Read;
    let requested = PathBuf::from(
        std::env::var_os("AIT_WORKFLOW_LEGACY_COPY").expect("existing coherent metadata source"),
    );
    assert!(requested.starts_with("/tmp") || requested.starts_with("/private/tmp"));
    let source = canonical_real_directory(&requested).unwrap();
    let source_registry = ServerOperationalRepositoryRegistry::new(
        source.join("global"),
        source.join("repositories"),
    )
    .unwrap();
    let physical = source_registry.list().unwrap();
    let scope = ServerRepositoryScope::from_environment().unwrap();
    scope.validate_known(&physical).unwrap();
    let root = std::env::temp_dir().join(format!(
        "ait-server-actor-plan-file-conversion-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let config =
        ServerBinaryLifecycleConfig::from_server_data_root(root.join("server-data")).unwrap();
    initialize_installed_runtime(&config).unwrap();
    let fresh = config.activation().unwrap().unwrap();
    let target = config.generations_root.join("actor-plan-file-conversion");
    fs::create_dir(&target).unwrap();
    fs::create_dir(target.join("global")).unwrap();
    fs::create_dir(target.join("repositories")).unwrap();
    for name in ["repository.bin", "repository_payload.bin"] {
        fs::copy(
            source.join("global").join(name),
            target.join("global").join(name),
        )
        .unwrap();
    }
    for name in ["generation.json", SERVER_LEGACY_CONVERSION_COMPLETION_FILE] {
        fs::copy(source.join(name), target.join(name)).unwrap();
    }
    let mut authorities = 0;
    let mut rows = 0;
    let mut source_bytes = 0;
    for entry in &physical {
        let relative = Path::new("repositories").join(entry.repository_index.to_string());
        fs::create_dir(target.join(&relative)).unwrap();
        if !scope.includes(entry) {
            continue;
        }
        authorities += 1;
        // Only routing headers and the two changed fixed files form the fixture.
        for name in SERVER_BINARY_DB_BIN_SCHEMAS
            .iter()
            .filter(|schema| {
                schema.layout_id == 2 && !matches!(schema.path, "plan.bin" | "plan_revision.bin")
            })
            .map(|schema| schema.path)
            .chain(["task.bin", "snapshot.bin"])
        {
            let mut header = [0; 4];
            fs::File::open(source.join(&relative).join(name))
                .unwrap()
                .read_exact(&mut header)
                .unwrap();
            fs::write(target.join(&relative).join(name), header).unwrap();
        }
        for (name, width) in [("plan.bin", 48), ("plan_revision.bin", 56)] {
            let path = source.join(&relative).join(name);
            let length = fs::metadata(&path).unwrap().len();
            rows += (length - 4) / width;
            source_bytes += length;
            fs::copy(path, target.join(&relative).join(name)).unwrap();
        }
    }
    let active = config
        .with_exclusive_admission(|| config.activate_staged_generation(&fresh, &target))
        .unwrap();
    let started = Instant::now();
    config
        .with_exclusive_admission(|| admit_workflow_generation(&config, &active))
        .unwrap();
    let conversion_seconds = started.elapsed().as_secs_f64();
    let target_bytes = physical
        .iter()
        .filter(|entry| scope.includes(entry))
        .map(|entry| {
            let directory = target
                .join("repositories")
                .join(entry.repository_index.to_string());
            fs::metadata(directory.join("plan.bin")).unwrap().len()
                + fs::metadata(directory.join("plan_revision.bin"))
                    .unwrap()
                    .len()
        })
        .sum::<u64>();
    println!("Actor Plan actual file conversion: {} physical, {authorities} in-scope authorities, {} files, {rows} rows, {source_bytes}->{target_bytes} bytes; routing/read/decode/write/fsync/replace/directory-sync={conversion_seconds:.6}s; ait-ci unoptimized, warm local APFS metadata, no cache flush, existing coherent source, no packs or unrelated record bodies copied, normal runtime open excluded",physical.len(),authorities*2);
    fs::remove_dir_all(root).unwrap();
}

fn capture_current_metadata(content: &Path, target: &Path, scope: &ServerRepositoryScope) {
    let config = ServerBinaryLifecycleConfig::from_server_data_root(
        content.ancestors().nth(3).unwrap().to_path_buf(),
    )
    .unwrap();
    config.with_exclusive_admission(|| {
        let active = config.activation()?.ok_or("missing canonical activation")?;
        if active.generation_root != content { return Err("content root is not the current activation".into()) }
        let global = content.join("global");
        let store = ServerBinaryDbFilesystemStore;
        let _registry_lock = store.acquire_process_lock(&global.join(REGISTRY_LOCK_FILE_NAME), ServerBinaryDbLockMode::Shared, ServerBinaryDbLockWait::Nonblocking)
            .map_err(|error| error.to_string())?.ok_or("registry writer is active")?;
        let registry = ServerOperationalRepositoryRegistry::new(&global, content.join("repositories")).map_err(|error| error.to_string())?;
        let entries = registry.validate().map_err(|error| error.to_string())?;
        scope.validate_known(&entries)?;
        let mut queues = Vec::new();
        let mut reads = Vec::new();
        for entry in entries.iter().filter(|entry| scope.includes(entry)) {
            let directory = content.join("repositories").join(entry.repository_index.to_string());
            queues.push(store.acquire_process_lock(&directory.join(WORKER_QUEUE_LOCK_FILE_NAME), ServerBinaryDbLockMode::Shared, ServerBinaryDbLockWait::Nonblocking)
                .map_err(|error| error.to_string())?.ok_or("queue writer is active")?);
            reads.push(BinaryDbReadLockSet::try_acquire(&StorePath::new(directory)).map_err(|error| error.to_string())?);
        }
        fs::create_dir(target).map_err(|error| error.to_string())?;
        copy_metadata(&global, &target.join("global"));
        fs::create_dir(target.join("repositories")).map_err(|error| error.to_string())?;
        for entry in &entries {
            let relative = Path::new("repositories").join(entry.repository_index.to_string());
            fs::create_dir(target.join(&relative)).map_err(|error| error.to_string())?;
            if scope.includes(entry) {
                copy_metadata(&content.join(&relative), &target.join(&relative));
            }
        }
        for item in fs::read_dir(content).map_err(|error| error.to_string())? {
            let item = item.map_err(|error| error.to_string())?;
            if item.file_type().map_err(|error| error.to_string())?.is_file() {
                fs::copy(item.path(), target.join(item.file_name())).map_err(|error| error.to_string())?;
            }
        }
        if config.activation()?.as_ref() != Some(&active) { return Err("activation changed during capture".into()) }
        println!("current coherent metadata captured: {} physical, {} native serving, {} configured serving; excluded authority files were not opened", entries.len(), entries.iter().filter(|entry| entry.record.is_serving()).count(), entries.iter().filter(|entry| scope.includes(entry)).count());
        Ok(())
    }).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "run only with an owner-admitted isolated metadata view and read-only existing content"]
fn actual_source_metadata_conversion_normal_startup_and_reopen() {
    let requested_source = PathBuf::from(
        std::env::var_os("AIT_WORKFLOW_LEGACY_COPY").expect("set admitted isolated metadata view"),
    );
    assert!(requested_source.starts_with("/private/tmp") || requested_source.starts_with("/tmp"));
    let content = canonical_real_directory(&PathBuf::from(
        std::env::var_os("AIT_WORKFLOW_CONTENT_ROOT").expect("set existing read-only content root"),
    ))
    .unwrap();
    let scope = ServerRepositoryScope::from_environment().unwrap();
    if !requested_source.exists() {
        capture_current_metadata(&content, &requested_source, &scope);
    }
    let source = canonical_real_directory(&requested_source).unwrap();
    let source_before = workflow_generation_fingerprint(&source).unwrap();
    let root = std::env::temp_dir().join(format!(
        "ait-server-actual-metadata-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let config =
        ServerBinaryLifecycleConfig::from_server_data_root(root.join("server-data")).unwrap();
    initialize_installed_runtime(&config).unwrap();
    let fresh = config.activation().unwrap().unwrap();
    let target = config.generations_root.join("actual-source");
    copy_metadata(&source, &target);
    let registry = ServerOperationalRepositoryRegistry::new(
        target.join("global"),
        target.join("repositories"),
    )
    .unwrap();
    let serving = registry
        .validate()
        .unwrap()
        .into_iter()
        .filter(|entry| config.repository_scope.includes(entry))
        .collect::<Vec<_>>();
    let mut content_links = Vec::new();
    let mut expected_tasks = BTreeMap::new();
    let mut expected_changes = 0;
    let mut expected_land_rows = 0;
    let mut conversion_count = 0;
    for entry in &serving {
        let relative = Path::new("repositories").join(entry.repository_index.to_string());
        let directory = target.join(&relative);
        conversion_count +=
            usize::from(fs::read(directory.join("change.bin")).unwrap()[..4] == 1u32.to_le_bytes());
        expected_tasks.insert(
            entry.repository_index,
            fs::read(directory.join("task.bin")).unwrap(),
        );
        expected_changes += rows(&directory, "change.bin", 68)
            .iter()
            .filter(|row| row[4] != 0)
            .count();
        expected_land_rows += rows(&directory, "land.bin", 48).len();
        let objects = content.join(&relative).join(".ait/objects");
        if objects.exists() {
            fs::create_dir_all(directory.join(".ait")).unwrap();
            let link = directory.join(".ait/objects");
            std::os::unix::fs::symlink(&objects, &link).unwrap();
            content_links.push((link, objects));
        }
    }
    let active = config
        .with_exclusive_admission(|| config.activate_staged_generation(&fresh, &target))
        .unwrap();
    let pointer = fs::read(&config.activation_pointer).unwrap();
    let started = Instant::now();
    let runtime = OperationalBinaryRuntime::ensure_from_config(&config).unwrap();
    let startup_seconds = started.elapsed().as_secs_f64();
    assert_eq!(
        runtime.serving_repository_indexes(),
        serving
            .iter()
            .map(|entry| entry.repository_index)
            .collect::<Vec<_>>()
    );
    let mut totals = [0usize; 4];
    for entry in &serving {
        let directory = target
            .join("repositories")
            .join(entry.repository_index.to_string());
        assert_eq!(
            fs::read(directory.join("task.bin")).unwrap(),
            expected_tasks[&entry.repository_index]
        );
        let lands = rows(&directory, "land.bin", 48);
        let actual = lands
            .iter()
            .filter(|row| row[0] & (LAND_TOMBSTONE | LAND_STATUS_MASK) == LAND_STATUS_SUCCEEDED)
            .map(|row| u32::from_le_bytes(row[4..8].try_into().unwrap()))
            .collect::<BTreeSet<_>>();
        totals[0] += rows(&directory, "change.bin", 68).len();
        totals[1] += lands.len();
        totals[2] += lands
            .iter()
            .filter(|row| row[0] & LAND_TOMBSTONE != 0)
            .count();
        totals[3] += rows(&directory, "task.bin", 60)
            .iter()
            .enumerate()
            .filter(|(index, row)| {
                row[0] & TASK_META_COMPLETED != 0 && !actual.contains(&(*index as u32))
            })
            .count();
        let original = source
            .join("repositories")
            .join(entry.repository_index.to_string());
        let old_changes = rows(&original, "change.bin", 68);
        let mut next_change = 0;
        let change_map = old_changes
            .iter()
            .map(|row| {
                if row[4] == 0 {
                    None
                } else {
                    let index = next_change;
                    next_change += 1;
                    Some(index)
                }
            })
            .collect::<Vec<Option<u32>>>();
        let old_links = rows(&original, "snapshot_link.bin", 40);
        let new_links = rows(&directory, "snapshot_link.bin", 22);
        assert_eq!(old_links.len(), new_links.len());
        for (old, new) in old_links.iter().zip(new_links) {
            let link = WorkflowBinaryV0Codec::decode_snapshot_link(&new).unwrap();
            assert_eq!(link.link_meta & 6, old[0] & 6);
            assert_eq!(link.snapshot_ordinal, old[1]);
            assert_eq!(
                link.task_index,
                u32::from_le_bytes(old[12..16].try_into().unwrap())
            );
            assert_eq!(
                link.content_snapshot_index,
                u32::from_le_bytes(old[20..24].try_into().unwrap())
            );
            assert_eq!(
                link.created_at_s,
                u64::from_le_bytes(old[32..40].try_into().unwrap())
            );
            let old_change = u32::from_le_bytes(old[16..20].try_into().unwrap());
            let expected = old_change
                .checked_sub(1)
                .and_then(|index| change_map[index as usize])
                .map_or(0, |index| index + 1);
            assert_eq!(link.change_index_plus1, expected);
        }
        for name in [
            "snapshot.bin",
            "snapshot_payload.bin",
            "patchset_summary_payload.bin",
            "policy_check.bin",
            "actor.bin",
            "actor_payload.bin",
            "task_land_index.bin",
            "task_snapshot_index.bin",
        ] {
            assert_eq!(
                fs::read(directory.join(name)).unwrap(),
                fs::read(original.join(name)).unwrap(),
                "Repository {} {name}",
                entry.repository_index
            );
        }
        for name in ["attest.bin", "review.bin", "policy.bin", "waiver.bin"] {
            assert_eq!(
                &fs::read(directory.join(name)).unwrap()[4..],
                &fs::read(original.join(name)).unwrap()[4..],
                "Repository {} {name}",
                entry.repository_index
            );
        }
    }
    assert_eq!(totals[0], expected_changes);
    assert_eq!(totals[1], expected_land_rows);
    assert_eq!(totals[2], 917);
    assert_eq!(totals[3], 564);
    let (_, db) = runtime.repository_db(0).unwrap();
    let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(db, "C")
        .unwrap()
        .with_repository_index(0);
    for (task, original_index) in [("RCT-0788", 775usize), ("RCT-0858", 846)] {
        let original = source.join("repositories/0");
        let old = &rows(&original, "change.bin", 68)[original_index];
        let payload = fs::read(original.join("change_payload.bin")).unwrap();
        let offset = u64::from_le_bytes(old[8..16].try_into().unwrap()) as usize;
        let len = u16::from_le_bytes(old[2..4].try_into().unwrap()) as usize;
        let change = workflow
            .get_change(Some("ait-core"), &format!("{task}/C-01"))
            .unwrap();
        assert_eq!(
            change["title"],
            std::str::from_utf8(&payload[offset..offset + len]).unwrap()
        );
    }
    assert_eq!(
        workflow.get_task(Some("ait-core"), "RCT-1036").unwrap()["successful_land"]
            ["submission_id"],
        "RCT-1036/L-01"
    );
    for task in ["RCT-0006", "RCT-0783", "RCT-0810", "RCT-0943"] {
        let task = workflow.get_task(Some("ait-core"), task).unwrap();
        assert_eq!(task["status"], "completed");
        assert!(task["successful_land"].is_null());
    }
    for (link, objects) in &content_links {
        assert_eq!(fs::read_link(link).unwrap(), *objects);
    }
    drop(workflow);
    drop(runtime);
    let converted = workflow_generation_fingerprint(&target).unwrap();
    let started = Instant::now();
    let reopened = OperationalBinaryRuntime::ensure_from_config(&config).unwrap();
    let reopen_seconds = started.elapsed().as_secs_f64();
    assert_eq!(workflow_generation_fingerprint(&target).unwrap(), converted);
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    assert_eq!(config.activation().unwrap().unwrap(), active);
    assert_eq!(
        workflow_generation_fingerprint(&source).unwrap(),
        source_before
    );
    println!("actual-source metadata normal startup={startup_seconds:.6}s, reopen={reopen_seconds:.6}s; {conversion_count} converted authorities, {} serving authorities, {} retained Changes, {} physical Lands, {} tombstones, {} terminal/null Tasks; temporary local metadata, existing content roots reused read-only, no pack copy", serving.len(), totals[0], totals[1], totals[2], totals[3]);
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires the existing coherent scoped metadata source"]
fn actor_workflow_actual_source_file_conversion() {
    use ait_server_core::foundation::remote_actor_metadata::{
        convert_remote_workflow_file, RemoteActorProfile, RemoteWorkflowFile,
    };
    use ait_server_core::foundation::server_plan_binary_db::remote_actor_profile::convert_remote_plan_metadata;
    let source = canonical_real_directory(&PathBuf::from(
        env::var_os("AIT_WORKFLOW_LEGACY_COPY").expect("coherent isolated metadata source"),
    ))
    .unwrap();
    assert!(source.starts_with("/tmp") || source.starts_with("/private/tmp"));
    let registry = ServerOperationalRepositoryRegistry::new(
        source.join("global"),
        source.join("repositories"),
    )
    .unwrap();
    let entries = registry.list().unwrap();
    let scope = ServerRepositoryScope::from_environment().unwrap();
    scope.validate_known(&entries).unwrap();
    for combined in [false, true] {
        let root = env::temp_dir().join(format!(
            "ait-actor-workflow-file-conversion-{}-{}-{}",
            std::process::id(),
            combined,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let mut config =
            ServerBinaryLifecycleConfig::from_server_data_root(root.join("server-data")).unwrap();
        config.repository_scope = scope.clone();
        initialize_installed_runtime(&config).unwrap();
        let fresh = config.activation().unwrap().unwrap();
        let target = config
            .generations_root
            .join("actor-workflow-file-conversion");
        fs::create_dir(&target).unwrap();
        fs::create_dir(target.join("global")).unwrap();
        fs::create_dir(target.join("repositories")).unwrap();
        for name in ["repository.bin", "repository_payload.bin"] {
            fs::copy(
                source.join("global").join(name),
                target.join("global").join(name),
            )
            .unwrap();
        }
        for name in ["generation.json", SERVER_LEGACY_CONVERSION_COMPLETION_FILE] {
            fs::copy(source.join(name), target.join(name)).unwrap();
        }
        let profile = if combined {
            RemoteActorProfile::Old
        } else {
            RemoteActorProfile::PlanOnly
        };
        let mut inputs = Vec::new();
        let mut plans = Vec::new();
        let mut authorities = 0;
        for entry in &entries {
            let relative = Path::new("repositories").join(entry.repository_index.to_string());
            fs::create_dir(target.join(&relative)).unwrap();
            if !scope.includes(entry) {
                continue;
            }
            authorities += 1;
            for name in [
                "attest.bin",
                "patchset.bin",
                "review.bin",
                "policy.bin",
                "waiver.bin",
                "snapshot_link.bin",
            ] {
                use std::io::Read;
                let mut header = [0; 4];
                fs::File::open(source.join(&relative).join(name))
                    .unwrap()
                    .read_exact(&mut header)
                    .unwrap();
                fs::write(target.join(&relative).join(name), header).unwrap();
            }
            let plan = fs::read(source.join(&relative).join("plan.bin")).unwrap();
            let revision = fs::read(source.join(&relative).join("plan_revision.bin")).unwrap();
            if combined {
                fs::write(target.join(&relative).join("plan.bin"), &plan).unwrap();
                fs::write(target.join(&relative).join("plan_revision.bin"), &revision).unwrap();
                plans.push((plan, revision));
            } else {
                let (plan, revision) = convert_remote_plan_metadata(&plan, &revision).unwrap();
                fs::write(target.join(&relative).join("plan.bin"), plan).unwrap();
                fs::write(target.join(&relative).join("plan_revision.bin"), revision).unwrap();
            }
            for family in RemoteWorkflowFile::ALL {
                let bytes = fs::read(source.join(&relative).join(family.name())).unwrap();
                fs::write(target.join(&relative).join(family.name()), &bytes).unwrap();
                inputs.push((family, bytes));
            }
        }
        let workflow_rows = inputs
            .iter()
            .map(|(family, bytes)| (bytes.len() - 4) / family.source_width())
            .sum::<usize>();
        let workflow_bytes = inputs.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
        assert_eq!(
            (inputs.len(), workflow_rows, workflow_bytes),
            (192, 10_210, 671_196)
        );
        let started = Instant::now();
        let output_bytes = inputs
            .iter()
            .map(|(family, bytes)| {
                convert_remote_workflow_file(profile, *family, bytes)
                    .unwrap()
                    .map_or(bytes.len(), |output| output.len())
            })
            .sum::<usize>();
        let plan_output_bytes = plans
            .iter()
            .map(|(plan, revision)| {
                let (plan, revision) = convert_remote_plan_metadata(plan, revision).unwrap();
                plan.len() + revision.len()
            })
            .sum::<usize>();
        let pure_seconds = started.elapsed().as_secs_f64();
        let source_bytes = workflow_bytes
            + plans
                .iter()
                .map(|(plan, revision)| plan.len() + revision.len())
                .sum::<usize>();
        let affected_rows = workflow_rows
            + plans
                .iter()
                .map(|(plan, revision)| (plan.len() - 4) / 48 + (revision.len() - 4) / 56)
                .sum::<usize>();
        let active = config
            .with_exclusive_admission(|| config.activate_staged_generation(&fresh, &target))
            .unwrap();
        let started = Instant::now();
        config
            .with_exclusive_admission(|| admit_workflow_generation(&config, &active))
            .unwrap();
        let durable_seconds = started.elapsed().as_secs_f64();
        println!("Actor {} actual conversion: {authorities} authorities, {} files, {affected_rows} rows, {source_bytes}->{} bytes; pure={pure_seconds:.6}s, routing/read/transform/write/fsync/replace/directory-sync={durable_seconds:.6}s; ait-ci unoptimized, warm local APFS, no cache flush, Job excluded, packs/unrelated bodies absent, runtime open excluded",
            if combined {"combined"} else {"workflow"}, inputs.len()+plans.len()*2,output_bytes+plan_output_bytes);
        fs::remove_dir_all(root).unwrap();
    }
}
