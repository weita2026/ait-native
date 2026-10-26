use super::*;
use crate::foundation::remote_actor_metadata::{
    classify_remote_actor_profile, convert_remote_workflow_file, RemoteActorHeaders,
    RemoteActorProfile, RemoteWorkflowFile,
};
use std::io::Read;

fn header(path: &Path) -> Result<[u8; 4], String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("Not a regular profile file: {}", path.display()));
    }
    let mut bytes = [0; 4];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| format!("Read profile {}: {error}", path.display()))?;
    Ok(bytes)
}

/// Route the fixed Actor batch before legacy inventory/staging/proof machinery.
/// Only converted bytes are staged; target profiles need no record-body reads.
pub(super) fn convert_if_current(
    config: &ServerBinaryLifecycleConfig,
    active: &ServerBinaryActivation,
    source: &Path,
    entries: &[OperationalRepositoryEntry],
    global_read: BoxedServerBinaryDbProcessLockGuard,
    hook: &mut impl FnMut(&str) -> Result<(), String>,
    observer: &mut dyn FnMut(WorkflowUpgradeProgress),
) -> Result<bool, String> {
    let mut reads = Vec::new();
    let mut queues = Vec::new();
    let mut workflow_profile = None;
    let mut actor_profile = None;
    report_progress(
        observer,
        WorkflowUpgradePhase::Inspect,
        0,
        Some(entries.len()),
    );
    for (inspected_count, entry) in entries.iter().enumerate() {
        let root = source
            .join("repositories")
            .join(entry.repository_index.to_string());
        canonical_real_directory(&root)?;
        queues.push(shared_lock(&root.join(WORKER_QUEUE_LOCK_FILE_NAME))?);
        reads.push(
            BinaryDbReadLockSet::try_acquire(&StorePath::new(root.clone()))
                .map_err(|error| error.to_string())?,
        );
        for schema in SERVER_BINARY_DB_BIN_SCHEMAS.iter().filter(|schema| {
            schema.layout_id == 2
                && !matches!(
                    schema.path,
                    "plan.bin"
                        | "plan_revision.bin"
                        | "task.bin"
                        | "change.bin"
                        | "snapshot.bin"
                        | "land.bin"
                )
        }) {
            let this = match header(&root.join(schema.path))? {
                [1, 0, 0, 0] => WorkflowProfile::Legacy,
                [2, 0, 0, 0] => WorkflowProfile::Current,
                _ => return Err(format!("Unsupported workflow profile at {}", schema.path)),
            };
            if workflow_profile
                .replace(this)
                .is_some_and(|previous| previous != this)
            {
                return Err(
                    "Partial workflow layout profile; whole-generation admission required".into(),
                );
            }
        }
        let read_header = |name| header(&root.join(name)).map(u32::from_le_bytes);
        let headers = RemoteActorHeaders {
            plan: read_header("plan.bin")?,
            plan_revision: read_header("plan_revision.bin")?,
            task: read_header("task.bin")?,
            change: read_header("change.bin")?,
            snapshot: read_header("snapshot.bin")?,
            land: read_header("land.bin")?,
        };
        let this = if workflow_profile == Some(WorkflowProfile::Legacy) {
            if headers
                != (RemoteActorHeaders {
                    plan: 1,
                    plan_revision: 1,
                    task: 1,
                    change: 1,
                    snapshot: 1,
                    land: 1,
                })
            {
                return Err(
                    "Unsupported combined profile: legacy workflow with partial metadata".into(),
                );
            }
            None
        } else {
            Some(classify_remote_actor_profile(headers).map_err(|error| error.to_string())?)
        };
        if actor_profile
            .replace(this)
            .is_some_and(|previous| previous != this)
        {
            return Err(
                "Partial generation Actor profile; all included authorities form one batch".into(),
            );
        }
        report_progress(
            observer,
            WorkflowUpgradePhase::Inspect,
            inspected_count + 1,
            Some(entries.len()),
        );
    }
    if workflow_profile == Some(WorkflowProfile::Legacy) {
        return Ok(false);
    }
    if workflow_profile != Some(WorkflowProfile::Current) {
        return Err("No declared workflow profile".into());
    }
    let profile = actor_profile.flatten().ok_or("No declared Actor profile")?;
    if profile == RemoteActorProfile::Actor {
        report_progress(observer, WorkflowUpgradePhase::Complete, 0, Some(0));
        return Ok(true);
    }

    // Parse the complete affected batch before creating replacement outputs.
    let mut converted_count = 0;
    report_progress(observer, WorkflowUpgradePhase::Convert, 0, None);
    let converted = entries
        .iter()
        .map(|entry| {
            let root = source
                .join("repositories")
                .join(entry.repository_index.to_string());
            let mut files = Vec::new();
            if profile == RemoteActorProfile::Old {
                let plans = read_regular_file(&root.join("plan.bin"))?;
                let revisions = read_regular_file(&root.join("plan_revision.bin"))?;
                let (plans, revisions) = convert_remote_plan_metadata(&plans, &revisions)
                    .map_err(|error| format!("Repository {}: {error}", entry.repository_index))?;
                files.push(("plan.bin", plans));
                files.push(("plan_revision.bin", revisions));
                converted_count += 2;
                report_progress(
                    observer,
                    WorkflowUpgradePhase::Convert,
                    converted_count,
                    None,
                );
            }
            for family in RemoteWorkflowFile::ALL {
                let bytes = read_regular_file(&root.join(family.name()))?;
                if let Some(bytes) = convert_remote_workflow_file(profile, family, &bytes)
                    .map_err(|error| format!("Repository {}: {error}", entry.repository_index))?
                {
                    files.push((family.name(), bytes));
                    converted_count += 1;
                    report_progress(
                        observer,
                        WorkflowUpgradePhase::Convert,
                        converted_count,
                        None,
                    );
                }
            }
            Ok(files)
        })
        .collect::<Result<Vec<_>, String>>()?;
    // Empty, already-current files need no replacement. The exact conversion
    // total is known only after the existing parser has selected its outputs.
    let total_files = converted.iter().map(Vec::len).sum();
    report_progress(
        observer,
        WorkflowUpgradePhase::Convert,
        converted_count,
        Some(total_files),
    );
    let outputs = MetadataOutputs::new(&config.generations_root)?;
    let mut staged_count = 0;
    report_progress(observer, WorkflowUpgradePhase::Stage, 0, Some(total_files));
    for (entry, files) in entries.iter().zip(&converted) {
        let relative = Path::new("repositories").join(entry.repository_index.to_string());
        let directory = outputs.0.join(&relative);
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        for (name, bytes) in files {
            let path = directory.join(name);
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .map_err(|error| error.to_string())?;
            file.write_all(bytes).map_err(|error| error.to_string())?;
            fs::set_permissions(
                &path,
                fs::metadata(source.join(&relative).join(name))
                    .map_err(|error| error.to_string())?
                    .permissions(),
            )
            .map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            staged_count += 1;
            report_progress(
                observer,
                WorkflowUpgradePhase::Stage,
                staged_count,
                Some(total_files),
            );
        }
    }
    hook("after_stage")?;
    drop(reads);
    drop(queues);
    drop(global_read);
    report_progress(observer, WorkflowUpgradePhase::AwaitPublication, 0, None);
    let _registry_write = process_lock(
        &source.join("global").join(REGISTRY_LOCK_FILE_NAME),
        ServerBinaryDbLockMode::Exclusive,
    )?;
    let mut writers = Vec::new();
    let mut queues = Vec::new();
    for entry in entries {
        let root = source
            .join("repositories")
            .join(entry.repository_index.to_string());
        queues.push(process_lock(
            &root.join(WORKER_QUEUE_LOCK_FILE_NAME),
            ServerBinaryDbLockMode::Exclusive,
        )?);
        writers.push(
            BinaryDbCommandLockSet::try_acquire(
                &StorePath::new(root),
                BinaryDbCommandScope::General,
            )
            .map_err(|error| error.to_string())?,
        );
    }
    hook("before_publish")?;
    if config.activation()?.as_ref() != Some(active) {
        return Err("Startup activation changed before Actor publication".into());
    }
    let mut published_count = 0;
    report_progress(
        observer,
        WorkflowUpgradePhase::Publish,
        0,
        Some(total_files),
    );
    for (entry, files) in entries.iter().zip(&converted) {
        let relative = Path::new("repositories").join(entry.repository_index.to_string());
        for (name, _) in files {
            fs::rename(
                outputs.0.join(&relative).join(name),
                source.join(&relative).join(name),
            )
            .map_err(|error| error.to_string())?;
        }
        sync_directory(&source.join(relative))?;
        published_count += files.len();
        report_progress(
            observer,
            WorkflowUpgradePhase::Publish,
            published_count,
            Some(total_files),
        );
    }
    hook("after_publish")?;
    report_progress(
        observer,
        WorkflowUpgradePhase::Complete,
        total_files,
        Some(total_files),
    );
    Ok(true)
}
