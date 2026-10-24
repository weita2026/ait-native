//! Apply a fully verified package mapping using the existing publication
//! fields only. Local successful receipts and terminal lifecycle stay intact.
//! Atomic family replacement keeps active records complete through interrupted
//! staging and publication. Exact confirmed retry completes an old/new family
//! prefix; it does not reconstruct previously lost historical record bytes.
//! General excludes readers across Task-first/Change publication and ordinary
//! reverse-order rollback. Before-images exist only in memory. Existing atomic
//! staging files are temporary, ignored on retry, and carry no recovery authority.
//! Copy/staging cost covers changed Task/Change metadata families only. Local
//! receipts, later rows, main and content remain authoritative and untouched.
//! An uncertain flush triggers exact locked readback and a Local durability
//! retry; complete mappings are retained if durability remains unavailable.
//! This covers process interruption under the existing atomic rename/fsync
//! contract, not cross-file power-loss atomicity or storage/media-loss repair.
use super::*;
use crate::workflow_package::{
    PackageCompletion, PackageCompletionOutcome, PackageMappingExpectation,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationWritePoint {
    BeforeTask,
    AfterTask,
    BeforeChange,
    AfterChange,
    BeforeFlush,
    AfterFlush,
}

impl<B: BinaryDb, const W: u32> BinaryDbWorkflowStore<B, W> {
    pub fn record_package_publication(
        &self,
        result: &PackageCompletion,
        source: &[PackageMappingExpectation],
    ) -> PlanStoreResult<()> {
        self.record_package_publication_observed(result, source, |_| Ok(()))
    }

    fn record_package_publication_observed(
        &self,
        result: &PackageCompletion,
        source: &[PackageMappingExpectation],
        mut checkpoint: impl FnMut(PublicationWritePoint) -> PlanStoreResult<()>,
    ) -> PlanStoreResult<()> {
        PackageCompletion::from_value(&json!(result), &result.expected_identity, source)
            .map_err(PlanStoreError::Invalid)?;
        if result.outcome != PackageCompletionOutcome::Confirmed {
            return Err(PlanStoreError::Invalid(
                "Only confirmed full package mappings can be published locally".into(),
            ));
        }
        if self.legacy_land_reader {
            return Err(PlanStoreError::Invalid(
                "An upgrade source is read-only".into(),
            ));
        }
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)
            .map_err(storage_error)?;
        let rows = self.read_rows_with_access(&tx).map_err(storage_error)?;
        let owners = self
            .snapshot_owners_with_access(&tx, &rows)
            .map_err(storage_error)?;
        let mut seen = BTreeSet::new();
        for entry in source {
            let requested = entry
                .snapshots
                .iter()
                .map(|row| (&row.snapshot_id, row.local_change_ref.as_deref()))
                .collect::<BTreeSet<_>>();
            for (id, binding) in
                self.completed_task_source_snapshots_with_access(&tx, &rows, &entry.local_task_id)?
            {
                let change = (!binding.change_id.is_empty())
                    .then(|| format!("{}/{}", binding.task_id, binding.change_id));
                if !requested.contains(&(&id, change.as_deref())) {
                    return Err(PlanStoreError::Invalid(
                        "Package writeback omits or substitutes an original source Snapshot owner"
                            .into(),
                    ));
                }
            }
            for snapshot in &entry.snapshots {
                let index = snapshot_index_by_id(&tx, &snapshot.snapshot_id)
                    .map_err(storage_error)?
                    .ok_or_else(|| {
                        PlanStoreError::Invalid("Local source Snapshot is unavailable".into())
                    })?;
                let matching = owners
                    .iter()
                    .filter(|(link, _)| link.snapshot == index && link.meta & 0x80 == 0)
                    .collect::<Vec<_>>();
                let [(_, owner)] = matching.as_slice() else {
                    return Err(PlanStoreError::Invalid(
                        "Exact Local source Snapshot owner is unavailable".into(),
                    ));
                };
                let change = (!owner.change_id.is_empty())
                    .then(|| format!("{}/{}", entry.local_task_id, owner.change_id));
                if !seen.insert(&snapshot.snapshot_id)
                    || owner.task_id != entry.local_task_id
                    || change.as_deref() != snapshot.local_change_ref.as_deref()
                {
                    return Err(PlanStoreError::Invalid(
                        "Package writeback has a conflicting Local source Snapshot owner".into(),
                    ));
                }
            }
        }
        let now = now_s()?;
        let mut tasks = Vec::new();
        let mut changes = Vec::new();
        for entry in &result.entries {
            let index = self.task_index_for_id(&rows, &entry.local_task_id)?;
            let actual_changes = rows
                .changes
                .values()
                .filter(|row| row["task_id"].as_str() == Some(entry.local_task_id.as_str()))
                .map(|row| {
                    row["change_ref"].as_str().ok_or_else(|| {
                        PlanStoreError::Invalid("Native Change reference is missing".into())
                    })
                })
                .collect::<PlanStoreResult<BTreeSet<_>>>()?;
            let requested_changes = entry
                .changes
                .iter()
                .map(|row| row.local_change_ref.as_str())
                .collect::<BTreeSet<_>>();
            if actual_changes != requested_changes {
                return Err(PlanStoreError::Invalid(
                    "Package mapping omits or invents a source Change".into(),
                ));
            }
            let mut task = self.read_task_record(&tx, index).map_err(storage_error)?;
            if rows.tasks[&entry.local_task_id]["status"].as_str() != Some("completed")
                || !self
                    .receipt_at(&tx, index)
                    .map_err(storage_error)?
                    .is_some_and(|receipt| receipt.is_succeeded())
            {
                return Err(PlanStoreError::Invalid(
                    "Package publication requires the original completed Task receipt".into(),
                ));
            }
            let remote = self.parse_remote_task_id(&entry.remote_task_id)?;
            if task.is_published() && task.published_remote_task_index != remote {
                return Err(PlanStoreError::Concurrency(
                    "Package Task already has another Remote mapping".into(),
                ));
            }
            if !task.is_published() {
                task.local_meta |= 1;
                task.published_remote_task_index = remote;
                task.published_at_s = now;
                task.updated_at_s = now;
                tasks.push((index, task));
            }
            for mapping in &entry.changes {
                let change_index = self.change_index_for_id(&rows, &mapping.local_change_ref)?;
                let mut change = LocalChangeRecord::decode(
                    &tx.read_record(Self::change_record_file(), change_index)
                        .map_err(storage_error)?,
                )
                .map_err(storage_error)?;
                let (remote_task, ordinal) =
                    self.parse_published_change_id(&mapping.remote_change_ref)?;
                if change.task_index != index || remote_task != remote {
                    return Err(PlanStoreError::Invalid(
                        "Package Change mapping belongs to another Task".into(),
                    ));
                }
                let plus1 = ordinal.checked_add(1).ok_or_else(|| {
                    PlanStoreError::Invalid("Remote Change ordinal overflow".into())
                })?;
                if change.is_published() && change.published_remote_change_ordinal_plus1 != plus1 {
                    return Err(PlanStoreError::Concurrency(
                        "Package Change already has another Remote mapping".into(),
                    ));
                }
                if !change.is_published() {
                    change.local_meta |= 1;
                    change.published_remote_change_ordinal_plus1 = plus1;
                    change.published_at_s = now;
                    change.updated_at_s = now;
                    changes.push((change_index, change));
                }
            }
        }
        if tasks.is_empty() && changes.is_empty() {
            // A prior invocation may have published every record but failed a
            // directory flush. Retry only Local durability, without new writes.
            tx.track_record_file(Self::task_record_file())
                .map_err(storage_error)?;
            if source.iter().any(|entry| !entry.changes.is_empty()) {
                tx.track_record_file(Self::change_record_file())
                    .map_err(storage_error)?;
            }
            return tx.commit().map(|_| ()).map_err(storage_error);
        }
        let mut files = Vec::new();
        if !tasks.is_empty() {
            let mut file = PublicationFileReplacement::read(&self.db, Self::task_record_file())
                .map_err(storage_error)?;
            for (index, task) in tasks {
                checkpoint(PublicationWritePoint::BeforeTask)?;
                file.replace_record(index, &task.encode().map_err(storage_error)?)
                    .map_err(storage_error)?;
                checkpoint(PublicationWritePoint::AfterTask)?;
            }
            files.push(file);
        }
        if !changes.is_empty() {
            let mut file = PublicationFileReplacement::read(&self.db, Self::change_record_file())
                .map_err(storage_error)?;
            for (index, change) in changes {
                checkpoint(PublicationWritePoint::BeforeChange)?;
                file.replace_record(index, &change.encode().map_err(storage_error)?)
                    .map_err(storage_error)?;
                checkpoint(PublicationWritePoint::AfterChange)?;
            }
            files.push(file);
        }
        checkpoint(PublicationWritePoint::BeforeFlush)?;
        commit_publication_files(&mut tx, &files).map_err(storage_error)?;
        checkpoint(PublicationWritePoint::AfterFlush)?;
        Ok(())
    }
}

struct PublicationFileReplacement {
    file: BinaryFileId,
    before: Vec<u8>,
    after: Vec<u8>,
}

fn publication_file_bytes<B: BinaryDb + ?Sized>(
    db: &B,
    file: &BinaryFileId,
) -> StoreResult<Vec<u8>> {
    let mut cache = crate::binary_db::BinaryDbReadCache::default();
    let mut bytes = db.layout_id(file.clone())?.to_le_bytes().to_vec();
    for index in 0..db.record_count(file.clone())? {
        bytes.extend(db.read_record_in_read_txn(file.clone(), index, &mut cache)?);
    }
    Ok(bytes)
}

impl PublicationFileReplacement {
    fn read<B: BinaryDb + ?Sized>(db: &B, file: BinaryFileId) -> StoreResult<Self> {
        let before = publication_file_bytes(db, &file)?;
        Ok(Self {
            file,
            after: before.clone(),
            before,
        })
    }

    fn replace_record(&mut self, index: u32, record: &[u8]) -> StoreResult<()> {
        let size = self.file.record_size() as usize;
        let start = (index as usize)
            .checked_mul(size)
            .and_then(|offset| offset.checked_add(4))
            .ok_or_else(|| {
                crate::binary_db::BinaryDbError::invalid_domain_data(
                    "Publication record offset overflow",
                )
            })?;
        let end = start.checked_add(size).ok_or_else(|| {
            crate::binary_db::BinaryDbError::invalid_domain_data("Publication record end overflow")
        })?;
        let target = self
            .after
            .get_mut(start..end)
            .filter(|_| record.len() == size)
            .ok_or_else(|| {
                crate::binary_db::BinaryDbError::invalid_domain_data(
                    "Publication record lies outside its original fixed file",
                )
            })?;
        target.copy_from_slice(record);
        Ok(())
    }
}

fn commit_publication_files<B: BinaryDb + ?Sized, F: BinaryDbFsyncPolicy>(
    tx: &mut BinaryDbWriteTxn<'_, B, F>,
    files: &[PublicationFileReplacement],
) -> StoreResult<()> {
    use crate::binary_db::BinaryDbError;
    if tx.command_scope() != BinaryDbCommandScope::General {
        return Err(BinaryDbError::invalid_domain_data(
            "Publication replacement requires General exclusion",
        ));
    }
    let db = tx.db();
    let paths = files
        .iter()
        .map(|file| {
            db.authority_root()
                .as_path()
                .join(file.file.relative_path().as_path())
        })
        .collect::<Vec<_>>();
    // Tracking adds only fsync dependencies, never an in-place overwrite or a
    // persistent before-image. General remains held through commit/rollback.
    for file in files {
        tx.track_record_file(file.file.clone())?;
    }
    let mut staging_directories = Vec::new();
    for (index, file) in files.iter().enumerate() {
        match db.replace_file_atomically(&paths[index], &file.after, "package-publication") {
            Ok(staging) => staging_directories.push(staging),
            Err(error) => {
                let restored = (|| -> StoreResult<()> {
                    for previous in (0..index).rev() {
                        let staging = db.replace_file_atomically(
                            &paths[previous],
                            &files[previous].before,
                            "package-publication-rollback",
                        )?;
                        db.sync_file(&paths[previous])?;
                        db.sync_directory(paths[previous].parent().ok_or_else(|| {
                            BinaryDbError::invalid_domain_data("Publication file lacks a parent")
                        })?)?;
                        let _ = db.sync_directory(&staging);
                    }
                    Ok(())
                })();
                if let Err(rollback) = restored {
                    return Err(BinaryDbError::new(error.kind(), format!("{error}. Atomic publication rollback is incomplete: {rollback}; retain complete old/new files and retry only the original mapping operation.")));
                }
                tx.abort()?;
                return Err(error);
            }
        }
    }
    if let Err(error) = tx.commit() {
        if tx.is_finished() {
            // Durability crossed before outer checkpoint/cleanup failure.
            return Err(error);
        }
        // Read back only genuine commit uncertainty, through a fresh cache
        // while General still excludes every reader/writer. Never roll back
        // a proven complete mapping because a later directory flush failed.
        let exact = files.iter().try_fold(true, |matches, file| {
            publication_file_bytes(db, &file.file).map(|bytes| matches && bytes == file.after)
        });
        if exact != Ok(true) {
            return Err(BinaryDbError::new(error.kind(), format!("{error}. Publication durability is uncertain and exact readback differs or is unavailable; preserve authority and retry only the original mapping operation.")));
        }
        if let Err(retry) = tx.commit() {
            return Err(BinaryDbError::new(retry.kind(), format!("{retry}. Exact publication mappings are present; preserve them and retry Local durability only.")));
        }
    }
    for staging in staging_directories {
        let _ = db.sync_directory(&staging);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_db::{AuthorityId, LocalBinaryDbFs, LocalStateScope};
    use crate::content_binary_db::{snapshot_id_index_key, BinarySnapshotCodec};
    use crate::file_io::{
        BoxedFileIoProcessLockGuard, FileIoByteStore, FileIoDurabilityStore, FileIoError,
        FileIoLockMode, FileIoLockStore, FileIoLockWait, FileIoResult, FileIoStore,
        FilesystemFileIoStore,
    };
    use crate::workflow_binary_db::tests::{fixture, seed_line_and_snapshots};
    use crate::workflow_expected_identity::PackageWorkflowExpectedIdentity;
    use crate::workflow_package::{
        PackageChangeMapping, PackageChangeMappingExpectation, PackageMapping,
        PackageSnapshotOwnerExpectation,
    };
    use tempfile::TempDir;

    // Inject a real partial overwrite or exit at a real file fsync boundary.
    // This does not unwind the writer or preserve its in-memory before-images.
    struct InterruptedPublicationIo {
        phase: String,
        fired: std::cell::Cell<bool>,
    }

    impl FileIoStore for InterruptedPublicationIo {
        fn home_dir(&self) -> Option<std::path::PathBuf> {
            FilesystemFileIoStore.home_dir()
        }
        fn path_exists(&self, path: &std::path::Path) -> bool {
            FilesystemFileIoStore.path_exists(path)
        }
        fn read_bytes(&self, path: &std::path::Path) -> FileIoResult<Vec<u8>> {
            FilesystemFileIoStore.read_bytes(path)
        }
        fn read_to_string(&self, path: &std::path::Path) -> FileIoResult<String> {
            FilesystemFileIoStore.read_to_string(path)
        }
        fn write_string(&self, path: &std::path::Path, text: &str) -> FileIoResult<()> {
            FilesystemFileIoStore.write_string(path, text)
        }
        fn write_string_atomically(
            &self,
            path: &std::path::Path,
            text: &str,
            label: &str,
        ) -> FileIoResult<()> {
            FilesystemFileIoStore.write_string_atomically(path, text, label)
        }
    }

    impl FileIoByteStore for InterruptedPublicationIo {
        fn write_bytes_atomically_from_directory(
            &self,
            path: &std::path::Path,
            staging: &std::path::Path,
            bytes: &[u8],
            label: &str,
        ) -> FileIoResult<()> {
            let name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if (self.phase == format!("FailReplace:{name}") && !self.fired.replace(true))
                || (self.phase == "FailChangeAndRollback"
                    && ((name == "change" && label == "package-publication")
                        || (name == "task" && label == "package-publication-rollback")))
            {
                return Err(FileIoError::other("injected atomic replacement failure"));
            }
            if (name == "task" && self.phase.starts_with("TornTask"))
                || (name == "change" && self.phase == "TornChange")
            {
                // Death during staging never exposes this partial output as
                // an active fixed file or a recovery authority.
                let partial = staging.join(format!(".{name}.tmp-interrupted-test"));
                FilesystemFileIoStore.create_parent_dirs(&partial)?;
                let end = if self.phase == "TornTaskLast" {
                    4 + 64 + 8
                } else {
                    4 + 8
                };
                FilesystemFileIoStore.write_bytes(&partial, &bytes[..end])?;
                FilesystemFileIoStore.sync_file(&partial)?;
                std::process::exit(92);
            }
            if self.phase == format!("BeforeReplace:{name}") {
                std::process::exit(92);
            }
            FilesystemFileIoStore
                .write_bytes_atomically_from_directory(path, staging, bytes, label)?;
            if self.phase == "CheckReader" && name == "task" {
                let authority = path.parent().unwrap();
                let reader = LocalBinaryDbFs::new(
                    authority,
                    authority.parent().unwrap(),
                    AuthorityId::new("local:fixture"),
                    LocalStateScope::Repository,
                );
                assert!(
                    reader.acquire_read_lock().is_err(),
                    "Reader observed a replacement prefix while General should exclude it"
                );
                self.fired.set(true);
            }
            if self.phase == "CheckReader" && name == "change" {
                assert!(
                    self.fired.get(),
                    "Task-first reader exclusion was not exercised"
                );
            }
            if self.phase == format!("AfterReplace:{name}") {
                std::process::exit(92);
            }
            Ok(())
        }
        fn create_parent_dirs(&self, path: &std::path::Path) -> FileIoResult<()> {
            FilesystemFileIoStore.create_parent_dirs(path)
        }
        fn write_bytes(&self, path: &std::path::Path, bytes: &[u8]) -> FileIoResult<()> {
            FilesystemFileIoStore.write_bytes(path, bytes)
        }
        fn truncate_file(&self, path: &std::path::Path, len: u64) -> FileIoResult<()> {
            FilesystemFileIoStore.truncate_file(path, len)
        }
        fn remove_file_if_exists(&self, path: &std::path::Path) -> FileIoResult<()> {
            FilesystemFileIoStore.remove_file_if_exists(path)
        }
        fn read_range(
            &self,
            path: &std::path::Path,
            offset: u64,
            len: u32,
        ) -> FileIoResult<Vec<u8>> {
            if self.phase == "FailSyncAndReadback" && self.fired.get() {
                return Err(FileIoError::other("injected uncertain readback failure"));
            }
            FilesystemFileIoStore.read_range(path, offset, len)
        }
        fn metadata_len(&self, path: &std::path::Path) -> FileIoResult<Option<u64>> {
            FilesystemFileIoStore.metadata_len(path)
        }
        fn overwrite_range(
            &self,
            path: &std::path::Path,
            offset: u64,
            bytes: &[u8],
        ) -> FileIoResult<()> {
            let target = match (path.file_name().and_then(|name| name.to_str()), offset) {
                (Some("task.bin"), 4) => "TornTaskFirst",
                (Some("task.bin"), 68) => "TornTaskLast",
                (Some("change.bin"), 4) => "TornChange",
                _ => "",
            };
            if self.phase == target {
                // Keep the original file width. A torn record can be aligned
                // yet carry an incorrect mapping or fail semantic decoding.
                FilesystemFileIoStore.overwrite_range(path, offset, &bytes[..8])?;
                FilesystemFileIoStore.sync_file(path)?;
                std::process::exit(92);
            }
            FilesystemFileIoStore.overwrite_range(path, offset, bytes)
        }
    }

    impl FileIoDurabilityStore for InterruptedPublicationIo {
        fn sync_file(&self, path: &std::path::Path) -> FileIoResult<()> {
            let name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if self.phase == format!("FailSyncOnce:{name}") && !self.fired.replace(true) {
                return Err(FileIoError::other("injected transient file fsync failure"));
            }
            if self.phase == format!("BeforeSync:{name}") {
                std::process::exit(92);
            }
            FilesystemFileIoStore.sync_file(path)?;
            if self.phase == format!("AfterSync:{name}") {
                std::process::exit(92);
            }
            Ok(())
        }
        fn sync_dir(&self, path: &std::path::Path) -> FileIoResult<()> {
            let active = path.file_name().is_some_and(|name| name == "binary-db")
                && path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .is_none_or(|name| name != ".locks");
            if active
                && (self.phase == "FailSyncAlways:directory"
                    || (self.phase == "FailSyncOnce:directory" && !self.fired.replace(true)))
            {
                return Err(FileIoError::other("injected directory fsync failure"));
            }
            if active && self.phase == "FailSyncAndReadback" {
                self.fired.set(true);
                return Err(FileIoError::other(
                    "injected directory fsync failure before unavailable readback",
                ));
            }
            if active && self.phase == "BeforeSyncDirectory" {
                std::process::exit(92);
            }
            FilesystemFileIoStore.sync_dir(path)?;
            if active && self.phase == "AfterSyncDirectory" {
                std::process::exit(92);
            }
            Ok(())
        }
    }

    impl FileIoLockStore for InterruptedPublicationIo {
        fn acquire_process_lock(
            &self,
            path: &std::path::Path,
            mode: FileIoLockMode,
            wait: FileIoLockWait,
        ) -> FileIoResult<Option<BoxedFileIoProcessLockGuard>> {
            FilesystemFileIoStore.acquire_process_lock(path, mode, wait)
        }
    }

    #[test]
    fn package_mapping_rolls_back_ordinary_errors_and_preserves_completed_history() {
        let temp = match std::env::var_os("AIT_PACKAGE_WRITEBACK_CHILD_ROOT") {
            Some(root) => TempDir::new_in(root).unwrap(),
            None => TempDir::new().unwrap(),
        };
        let store = fixture(&temp);
        let (base, final_id) = seed_line_and_snapshots(&store);
        let mut write =
            BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
        for (index, id) in [&base, &final_id].into_iter().enumerate() {
            write
                .append_index_candidate(
                    BinarySnapshotCodec::<1>::id_index(),
                    &snapshot_id_index_key(id).unwrap(),
                    index as u32,
                )
                .unwrap();
        }
        write.commit().unwrap();
        let mut source = Vec::new();
        for (ordinal, snapshot) in [&base, &final_id].into_iter().enumerate() {
            let task = TaskStore::create_task(
                &store,
                "fixture",
                "Source",
                "Publish exact history",
                Some("C"),
                None,
                None,
                None,
            )
            .unwrap();
            let id = task["task_id"].as_str().unwrap().to_string();
            let change = if ordinal == 1 {
                ChangeStore::create_change(
                    &store,
                    &id,
                    "fixture",
                    "Revision",
                    "main",
                    Some("C"),
                    Some(&base),
                )
                .unwrap();
                Some(format!("{id}/C-01"))
            } else {
                None
            };
            let mut write =
                BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
            store
                .record_snapshot_link_in_write(
                    &mut write,
                    &SnapshotWorkflowBinding {
                        task_id: id.clone(),
                        change_id: if change.is_some() {
                            "C-01".into()
                        } else {
                            String::new()
                        },
                        line_name: "main".into(),
                    },
                    snapshot,
                    1,
                )
                .unwrap();
            write.commit().unwrap();
            store
                .finish_local_task(&id, snapshot, "main", Some(&base), change.as_deref(), None)
                .unwrap();
            source.push(PackageMappingExpectation {
                local_task_id: id,
                remote_task_id: None,
                changes: change
                    .iter()
                    .map(|reference| PackageChangeMappingExpectation {
                        local_change_ref: reference.clone(),
                        remote_change_ref: None,
                    })
                    .collect(),
                snapshots: vec![PackageSnapshotOwnerExpectation {
                    snapshot_id: snapshot.clone(),
                    local_change_ref: change,
                }],
            });
        }
        let before = source
            .iter()
            .map(|row| TaskStore::get_task(&store, &row.local_task_id).unwrap())
            .collect::<Vec<_>>();
        let revision_before =
            ChangeStore::get_change(&store, &source[1].changes[0].local_change_ref).unwrap();
        let identity = PackageWorkflowExpectedIdentity {
            repository_index: 0,
            worker_job_index: 0,
            package_hash: "ab".repeat(32),
            base_snapshot_id: base,
            revision_snapshot_id: final_id,
        };
        let result = PackageCompletion {
            outcome: PackageCompletionOutcome::Confirmed,
            expected_identity: identity,
            patchset_id: Some("RCT-0102/P-01".into()),
            submission_id: Some("RCT-0102/L-01".into()),
            entries: source
                .iter()
                .enumerate()
                .map(|(index, row)| PackageMapping {
                    local_task_id: row.local_task_id.clone(),
                    remote_task_id: format!("RCT-{:04}", 101 + index),
                    changes: row
                        .changes
                        .iter()
                        .map(|change| PackageChangeMapping {
                            local_change_ref: change.local_change_ref.clone(),
                            remote_change_ref: "RCT-0102/C-07".into(),
                        })
                        .collect(),
                })
                .collect(),
        };
        if let Ok(phase) = std::env::var("AIT_PACKAGE_WRITEBACK_EXIT_PHASE") {
            // Work completed after the original package pin must survive a
            // whole-family mapping replacement and every subsequent retry.
            let later = TaskStore::create_task(
                &store,
                "fixture",
                "Later Task",
                "Keep outside the original package",
                Some("C"),
                None,
                None,
                None,
            )
            .unwrap();
            let later_id = later["task_id"].as_str().unwrap();
            let revision = ChangeStore::create_change(
                &store,
                later_id,
                "fixture",
                "Later revision",
                "main",
                Some("C"),
                Some(&result.expected_identity.revision_snapshot_id),
            )
            .unwrap();
            let later_change = revision["change_ref"].as_str().unwrap();
            let snapshot_id = "SNP-000000000003";
            let mut write =
                BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::SnapshotWrite).unwrap();
            let file = BinarySnapshotCodec::<1>::record_file();
            let mut snapshot = BinarySnapshotCodec::<1>::decode_record(
                &write.read_record(file.clone(), 1).unwrap(),
            )
            .unwrap();
            snapshot.snapshot_hash48 = snapshot_hash48_from_id(snapshot_id).unwrap();
            snapshot.parent_snapshot_index_plus1 = 2;
            snapshot.manifest_hash = [3; 32];
            write
                .append_record(
                    file,
                    &BinarySnapshotCodec::<1>::encode_record(&snapshot).unwrap(),
                )
                .unwrap();
            write
                .append_index_candidate(
                    BinarySnapshotCodec::<1>::id_index(),
                    &snapshot_id_index_key(snapshot_id).unwrap(),
                    2,
                )
                .unwrap();
            store
                .record_snapshot_link_in_write(
                    &mut write,
                    &SnapshotWorkflowBinding {
                        task_id: later_id.into(),
                        change_id: "C-01".into(),
                        line_name: "main".into(),
                    },
                    snapshot_id,
                    1,
                )
                .unwrap();
            write.commit().unwrap();
            store
                .finish_local_task(
                    later_id,
                    snapshot_id,
                    "main",
                    Some(&result.expected_identity.revision_snapshot_id),
                    Some(later_change),
                    None,
                )
                .unwrap();
            if let Some(io_phase) = phase
                .strip_prefix("Io:")
                .or_else(|| phase.strip_prefix("Error:"))
            {
                let interrupted = BinaryDbWorkflowStore::<_, 1>::new_with_namespace(
                    LocalBinaryDbFs::with_file_io_store(
                        InterruptedPublicationIo {
                            phase: io_phase.into(),
                            fired: std::cell::Cell::new(false),
                        },
                        temp.path().join("binary-db"),
                        temp.path(),
                        AuthorityId::new("local:fixture"),
                        LocalStateScope::Repository,
                    ),
                    "fixture",
                    "C",
                );
                let applied = interrupted.record_package_publication(&result, &source);
                if phase.starts_with("Error:") {
                    if io_phase.starts_with("FailSyncOnce:") || io_phase == "CheckReader" {
                        applied.unwrap();
                    } else {
                        assert!(applied.is_err());
                    }
                    std::process::exit(93);
                }
                applied.unwrap();
                panic!("Child did not reach I/O interruption point {phase}");
            }
            store
                .record_package_publication_observed(&result, &source, |point| {
                    if format!("{point:?}") == phase {
                        // Exit the actual writer process without unwinding or Drop.
                        std::process::exit(91);
                    }
                    Ok(())
                })
                .unwrap();
            panic!("Child did not reach writeback interruption point {phase}");
        }
        for phase in [
            PublicationWritePoint::BeforeTask,
            PublicationWritePoint::AfterTask,
            PublicationWritePoint::BeforeChange,
            PublicationWritePoint::AfterChange,
            PublicationWritePoint::BeforeFlush,
        ] {
            assert!(store
                .record_package_publication_observed(&result, &source, |point| {
                    if point == phase {
                        Err(PlanStoreError::Invalid("injected write failure".into()))
                    } else {
                        Ok(())
                    }
                })
                .is_err());
            for (index, row) in source.iter().enumerate() {
                assert_eq!(
                    TaskStore::get_task(&store, &row.local_task_id).unwrap(),
                    before[index]
                );
            }
            assert_eq!(
                ChangeStore::get_change(&store, &source[1].changes[0].local_change_ref).unwrap(),
                revision_before
            );
        }
        for fault in ["missing", "substitute"] {
            let mut incomplete_source = source.clone();
            if fault == "missing" {
                incomplete_source[1].snapshots.clear();
            } else {
                incomplete_source[1].snapshots[0].snapshot_id =
                    source[0].snapshots[0].snapshot_id.clone();
            }
            assert!(store
                .record_package_publication(&result, &incomplete_source)
                .is_err());
            for (index, row) in source.iter().enumerate() {
                assert_eq!(
                    TaskStore::get_task(&store, &row.local_task_id).unwrap(),
                    before[index]
                );
            }
        }
        let mut invalid = result.clone();
        invalid.entries[1].remote_task_id = "not-a-task".into();
        invalid.entries[1].changes[0].remote_change_ref = "not-a-task/C-07".into();
        invalid.patchset_id = Some("not-a-task/P-01".into());
        invalid.submission_id = Some("not-a-task/L-01".into());
        assert!(store.record_package_publication(&invalid, &source).is_err());
        assert_eq!(
            TaskStore::get_task(&store, &source[0].local_task_id).unwrap(),
            before[0]
        );
        store.record_package_publication(&result, &source).unwrap();
        let once = source
            .iter()
            .map(|row| TaskStore::get_task(&store, &row.local_task_id).unwrap())
            .collect::<Vec<_>>();
        store.record_package_publication(&result, &source).unwrap();
        for (index, row) in source.iter().enumerate() {
            let after = TaskStore::get_task(&store, &row.local_task_id).unwrap();
            assert_eq!(after, once[index]);
            assert_eq!(after["status"], before[index]["status"]);
            assert_eq!(after["local_finish"], before[index]["local_finish"]);
            assert_eq!(
                after["published_task_id"],
                result.entries[index].remote_task_id
            );
        }
        let revision =
            ChangeStore::get_change(&store, &source[1].changes[0].local_change_ref).unwrap();
        assert_eq!(revision["status"], revision_before["status"]);
        assert_eq!(revision["published_change_id"], "RCT-0102/C-07");
        let mut replacement = result;
        replacement.entries[0].remote_task_id = "RCT-0109".into();
        assert!(store
            .record_package_publication(&replacement, &source)
            .is_err());
        assert_eq!(
            TaskStore::get_task(&store, &source[0].local_task_id).unwrap(),
            once[0]
        );
    }

    #[test]
    fn hard_writeback_interruption_recovers_readable_partial_mappings_and_holds_torn_state() {
        for phase in [
            "BeforeTask",
            "AfterTask",
            "BeforeChange",
            "AfterChange",
            "BeforeFlush",
            "AfterFlush",
            "Io:TornTaskFirst",
            "Io:TornTaskLast",
            "Io:TornChange",
            "Io:BeforeReplace:task",
            "Io:AfterReplace:task",
            "Io:BeforeReplace:change",
            "Io:AfterReplace:change",
            "Io:BeforeSync:task",
            "Io:AfterSync:task",
            "Io:BeforeSync:change",
            "Io:AfterSync:change",
            "Io:BeforeSyncDirectory",
            "Io:AfterSyncDirectory",
            "Error:FailReplace:task",
            "Error:FailReplace:change",
            "Error:FailChangeAndRollback",
            "Error:FailSyncOnce:task",
            "Error:FailSyncOnce:change",
            "Error:FailSyncOnce:directory",
            "Error:FailSyncAlways:directory",
            "Error:FailSyncAndReadback",
            "Error:CheckReader",
        ] {
            let parent = TempDir::new().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "workflow_binary_db::package_publication::tests::package_mapping_rolls_back_ordinary_errors_and_preserves_completed_history", "--nocapture"])
                .env("AIT_PACKAGE_WRITEBACK_CHILD_ROOT", parent.path())
                .env("AIT_PACKAGE_WRITEBACK_EXIT_PHASE", phase)
                .status().unwrap();
            assert_eq!(
                status.code(),
                Some(if phase.starts_with("Io:") {
                    92
                } else if phase.starts_with("Error:") {
                    93
                } else {
                    91
                }),
                "{phase}"
            );
            let root = std::fs::read_dir(parent.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let store = BinaryDbWorkflowStore::<_, 1>::new_with_namespace(
                LocalBinaryDbFs::new(
                    root.join("binary-db"),
                    root.as_path(),
                    AuthorityId::new("local:fixture"),
                    LocalStateScope::Repository,
                ),
                "fixture",
                "C",
            );
            let source = (1..=2)
                .map(|index| PackageMappingExpectation {
                    local_task_id: format!("LCT-{index:04}"),
                    remote_task_id: None,
                    changes: if index == 2 {
                        vec![PackageChangeMappingExpectation {
                            local_change_ref: "LCT-0002/C-01".into(),
                            remote_change_ref: None,
                        }]
                    } else {
                        vec![]
                    },
                    snapshots: vec![PackageSnapshotOwnerExpectation {
                        snapshot_id: format!("SNP-{index:012X}"),
                        local_change_ref: if index == 2 {
                            Some("LCT-0002/C-01".into())
                        } else {
                            None
                        },
                    }],
                })
                .collect::<Vec<_>>();
            let result = PackageCompletion {
                outcome: PackageCompletionOutcome::Confirmed,
                expected_identity: PackageWorkflowExpectedIdentity {
                    repository_index: 0,
                    worker_job_index: 0,
                    package_hash: "ab".repeat(32),
                    base_snapshot_id: "SNP-000000000001".into(),
                    revision_snapshot_id: "SNP-000000000002".into(),
                },
                patchset_id: Some("RCT-0102/P-01".into()),
                submission_id: Some("RCT-0102/L-01".into()),
                entries: source
                    .iter()
                    .enumerate()
                    .map(|(index, row)| PackageMapping {
                        local_task_id: row.local_task_id.clone(),
                        remote_task_id: format!("RCT-{:04}", 101 + index),
                        changes: row
                            .changes
                            .iter()
                            .map(|change| PackageChangeMapping {
                                local_change_ref: change.local_change_ref.clone(),
                                remote_change_ref: "RCT-0102/C-07".into(),
                            })
                            .collect(),
                    })
                    .collect(),
            };
            let untouched = [
                "land.bin",
                "line.bin",
                "snapshot.bin",
                "task_payload.bin",
                "change_payload.bin",
            ];
            let before =
                untouched.map(|path| std::fs::read(root.join("binary-db").join(path)).unwrap());
            let receipts = source
                .iter()
                .map(|row| {
                    TaskStore::get_task(&store, &row.local_task_id).unwrap()["local_finish"].clone()
                })
                .collect::<Vec<_>>();
            let later_task = TaskStore::get_task(&store, "LCT-0003").unwrap();
            let later_change = ChangeStore::get_change(&store, "LCT-0003/C-01").unwrap();
            assert_eq!(
                later_task["local_finish"]["landed_snapshot_id"],
                "SNP-000000000003"
            );
            let main = BinaryLineCodec::<1>::decode_record(
                &store
                    .db()
                    .read_record(BinaryLineCodec::<1>::record_file(), 0)
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(main.head_snapshot_index_plus1, 3);
            let before_tasks = source
                .iter()
                .map(|row| TaskStore::get_task(&store, &row.local_task_id).unwrap())
                .collect::<Vec<_>>();
            let before_change = ChangeStore::get_change(&store, "LCT-0002/C-01").unwrap();
            let expect_complete = phase.starts_with("Io:BeforeSync")
                || phase.starts_with("Io:AfterSync")
                || phase.starts_with("Error:FailSync")
                || phase == "Io:AfterReplace:change"
                || phase == "Error:CheckReader";
            let expect_partial = matches!(
                phase,
                "Io:AfterReplace:task"
                    | "Io:BeforeReplace:change"
                    | "Io:TornChange"
                    | "Error:FailChangeAndRollback"
            );
            if expect_complete || expect_partial {
                for (task, expected) in before_tasks.iter().zip(&result.entries) {
                    assert_eq!(
                        task["published_task_id"], expected.remote_task_id,
                        "{phase}"
                    );
                }
                if expect_complete {
                    assert_eq!(
                        before_change["published_change_id"], "RCT-0102/C-07",
                        "{phase}"
                    );
                } else {
                    assert!(before_change["published_change_id"].is_null(), "{phase}");
                }
            } else if phase.starts_with("Error:FailReplace:")
                || phase.starts_with("Io:TornTask")
                || phase == "Io:BeforeReplace:task"
            {
                for task in &before_tasks {
                    assert!(task["published_task_id"].is_null(), "{phase}");
                }
                assert!(before_change["published_change_id"].is_null(), "{phase}");
            }
            // Some phases deliberately leave only a prefix published. Formal
            // confirmation is retained by the caller, never inferred here.
            store.record_package_publication(&result, &source).unwrap();
            store.record_package_publication(&result, &source).unwrap();
            for (index, row) in source.iter().enumerate() {
                let task = TaskStore::get_task(&store, &row.local_task_id).unwrap();
                assert_eq!(
                    task["published_task_id"],
                    result.entries[index].remote_task_id
                );
                assert_eq!(task["local_finish"], receipts[index]);
                assert_eq!(task["status"], "completed");
            }
            assert_eq!(
                ChangeStore::get_change(&store, "LCT-0002/C-01").unwrap()["published_change_id"],
                "RCT-0102/C-07"
            );
            for (path, original) in untouched.into_iter().zip(before) {
                assert_eq!(
                    std::fs::read(root.join("binary-db").join(path)).unwrap(),
                    original,
                    "{phase}/{path}"
                );
            }
            assert_eq!(TaskStore::get_task(&store, "LCT-0003").unwrap(), later_task);
            if expect_complete {
                for (task, expected) in source.iter().zip(before_tasks) {
                    assert_eq!(
                        TaskStore::get_task(&store, &task.local_task_id).unwrap(),
                        expected
                    );
                }
                assert_eq!(
                    ChangeStore::get_change(&store, "LCT-0002/C-01").unwrap(),
                    before_change
                );
            }
            assert_eq!(
                ChangeStore::get_change(&store, "LCT-0003/C-01").unwrap(),
                later_change
            );
            if phase == "AfterTask" {
                let path = root.join("binary-db/task.bin");
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(4 + 64 + 31)
                    .unwrap();
                let torn = std::fs::read(&path).unwrap();
                assert!(store.record_package_publication(&result, &source).is_err());
                assert_eq!(std::fs::read(&path).unwrap(), torn);
            }
        }
    }
}
