use ait_server_core::foundation::remote_binary_db::{
    BinaryDbReadTxn, FilesystemServerRemoteBinaryDb, RepoId, RepoName, StoreGeneration, StorePath,
};
use ait_server_core::foundation::server_content_binary_db::{
    validate_server_snapshot_dag_v0, validate_server_tree_authority_v0, ServerBinaryDbLineStore,
    ServerBinaryDbSnapshotStore, SERVER_CONTENT_BINARY_LAYOUT_ID,
};
use ait_server_core::foundation::workflow_binary_v0::{
    V0ChangeRecord, V0FrozenPatchsetRecord, V0LandRecord, V0PatchsetRecord, WorkflowBinaryV0Codec,
    LAND_HAS_LANDED_SNAPSHOT, LAND_MODE_DIRECT, LAND_MODE_MASK, LAND_STATUS_MASK,
    LAND_STATUS_SUCCEEDED,
};
use ait_server_core::foundation::workflow_binary_v0_adapter::{
    validate_frozen_server_workflow_v0, validate_server_workflow_v0,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type SourceDb = FilesystemServerRemoteBinaryDb;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PatchsetIdentity {
    pub index: u32,
    pub record: V0PatchsetRecord,
    pub change: V0ChangeRecord,
}

impl PatchsetIdentity {
    pub(crate) fn patchset_number(self) -> u32 {
        u32::from(self.record.patch_ordinal) + 1
    }

    pub(crate) fn change_sequence(self) -> u32 {
        u32::from(self.change.change_ordinal) + 1
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LandIdentity {
    pub record: V0LandRecord,
}

#[derive(Clone, Debug)]
pub(crate) struct RepositoryDomain {
    pub source_root: PathBuf,
    pub patchsets_by_id: BTreeMap<String, PatchsetIdentity>,
    pub patchsets_by_index: Vec<V0PatchsetRecord>,
    pub snapshots_by_id: BTreeMap<String, u32>,
    pub snapshot_ids_by_index: BTreeMap<u32, String>,
    pub lands_by_id: BTreeMap<String, LandIdentity>,
    pub line_names_by_index: BTreeMap<u32, String>,
    pub main_head_snapshot_index: Option<u32>,
}

impl RepositoryDomain {
    pub(crate) fn load(
        source_root: &Path,
        repo_id: &str,
        repo_name: &str,
        storage_generation: u64,
        namespace_ascii: [u8; 2],
    ) -> Result<Self, String> {
        Self::load_with_patchset_codec(
            source_root,
            repo_id,
            repo_name,
            storage_generation,
            namespace_ascii,
            PatchsetSourceCodec::Transitional,
        )
    }

    pub(crate) fn load_frozen(
        source_root: &Path,
        repo_id: &str,
        repo_name: &str,
        storage_generation: u64,
        namespace_ascii: [u8; 2],
    ) -> Result<Self, String> {
        Self::load_with_patchset_codec(
            source_root,
            repo_id,
            repo_name,
            storage_generation,
            namespace_ascii,
            PatchsetSourceCodec::Frozen,
        )
    }

    fn load_with_patchset_codec(
        source_root: &Path,
        repo_id: &str,
        repo_name: &str,
        storage_generation: u64,
        namespace_ascii: [u8; 2],
        patchset_codec: PatchsetSourceCodec,
    ) -> Result<Self, String> {
        let source_root = canonical_real_directory(source_root)?;
        let db = FilesystemServerRemoteBinaryDb::serving_authority(
            RepoId::new(repo_id),
            RepoName::new(repo_name),
            StorePath::new(source_root.clone()),
            StoreGeneration::new(storage_generation),
        );
        match patchset_codec {
            PatchsetSourceCodec::Transitional => validate_server_workflow_v0(&db),
            PatchsetSourceCodec::Frozen => validate_frozen_server_workflow_v0(&db),
        }
        .map_err(|error| format!("source workflow authority is invalid: {error}"))?;
        validate_server_snapshot_dag_v0(&db)
            .map_err(|error| format!("source Snapshot authority is invalid: {error}"))?;
        let read = BinaryDbReadTxn::new(&db);
        let changes = read_records(
            &read,
            WorkflowBinaryV0Codec::change_file(),
            WorkflowBinaryV0Codec::decode_change,
        )?;
        let patchsets_by_index = match patchset_codec {
            PatchsetSourceCodec::Transitional => read_records(
                &read,
                WorkflowBinaryV0Codec::patchset_file(),
                WorkflowBinaryV0Codec::decode_patchset,
            )?,
            PatchsetSourceCodec::Frozen => read_records(
                &read,
                WorkflowBinaryV0Codec::patchset_file(),
                WorkflowBinaryV0Codec::decode_frozen_patchset,
            )?
            .into_iter()
            .map(logical_patchset_from_frozen)
            .collect(),
        };
        let task_prefix = task_prefix(namespace_ascii)?;
        let mut patchsets_by_id = BTreeMap::new();
        for (index, patchset) in patchsets_by_index.iter().copied().enumerate() {
            let change = *changes.get(patchset.change_index as usize).ok_or_else(|| {
                format!(
                    "Patchset {index} references missing Change {}",
                    patchset.change_index
                )
            })?;
            if change.change_ordinal != patchset.change_ordinal {
                return Err(format!(
                    "Patchset {index} Change ordinal disagrees with its owning Change"
                ));
            }
            let task_id = format!("{task_prefix}T-{:04}", change.task_index + 1);
            let change_ref = format!("{task_id}/C-{:02}", change.change_ordinal + 1);
            let patchset_id = format!("{change_ref}/P-{:02}", patchset.patch_ordinal + 1);
            let identity = PatchsetIdentity {
                index: u32::try_from(index)
                    .map_err(|_| "Patchset count exceeds u32".to_string())?,
                record: patchset,
                change,
            };
            if patchsets_by_id
                .insert(patchset_id.clone(), identity)
                .is_some()
            {
                return Err(format!("duplicate Patchset identity {patchset_id}"));
            }
        }

        let snapshot_store =
            ServerBinaryDbSnapshotStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
        let mut snapshots_by_id = BTreeMap::new();
        let mut snapshot_ids_by_index = BTreeMap::new();
        for (index, _) in snapshot_store
            .all_snapshots(&read)
            .map_err(|error| format!("failed to inventory source Snapshots: {error}"))?
        {
            let snapshot_id = snapshot_store
                .snapshot_id_at(&read, index)
                .map_err(|error| format!("failed to resolve source Snapshot {index}: {error}"))?;
            if snapshots_by_id.insert(snapshot_id.clone(), index).is_some() {
                return Err(format!("duplicate Snapshot identity {snapshot_id}"));
            }
            snapshot_ids_by_index.insert(index, snapshot_id);
        }

        let line_store =
            ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
        let mut line_names_by_index = BTreeMap::new();
        let mut main_line_seen = false;
        let mut main_head_snapshot_index = None;
        for (index, name, record) in line_store
            .all_lines(&read)
            .map_err(|error| format!("failed to inventory source Lines: {error}"))?
        {
            observe_logical_main(
                &name,
                record.head_snapshot_index(),
                &mut main_line_seen,
                &mut main_head_snapshot_index,
            )?;
            line_names_by_index.insert(index, name);
        }
        let lands = read_records(
            &read,
            WorkflowBinaryV0Codec::land_file(),
            WorkflowBinaryV0Codec::decode_land,
        )?;
        let mut lands_by_id = BTreeMap::new();
        for land in lands {
            let change = *changes.get(land.change_index as usize).ok_or_else(|| {
                format!("Land references missing Change index {}", land.change_index)
            })?;
            if change.change_ordinal != land.change_ordinal {
                return Err("Land Change ordinal disagrees with its owning Change".to_string());
            }
            if land.patchset_index as usize >= patchsets_by_index.len() {
                return Err(format!(
                    "Land references missing Patchset index {}",
                    land.patchset_index
                ));
            }
            let task_id = format!("{task_prefix}T-{:04}", change.task_index + 1);
            let change_ref = format!("{task_id}/C-{:02}", change.change_ordinal + 1);
            let land_id = format!("{change_ref}/L-{:02}", land.land_ordinal + 1);
            if lands_by_id
                .insert(land_id.clone(), LandIdentity { record: land })
                .is_some()
            {
                return Err(format!("duplicate Land identity {land_id}"));
            }
        }

        Ok(Self {
            source_root,
            patchsets_by_id,
            patchsets_by_index,
            snapshots_by_id,
            snapshot_ids_by_index,
            lands_by_id,
            line_names_by_index,
            main_head_snapshot_index,
        })
    }

    pub(crate) fn validate_tree_authority(
        &self,
        repo_id: &str,
        repo_name: &str,
        storage_generation: u64,
    ) -> Result<(), String> {
        let db = FilesystemServerRemoteBinaryDb::serving_authority(
            RepoId::new(repo_id),
            RepoName::new(repo_name),
            StorePath::new(self.source_root.clone()),
            StoreGeneration::new(storage_generation),
        );
        validate_server_tree_authority_v0(&db)
            .map_err(|error| format!("source Tree authority is invalid: {error}"))
    }

    pub(crate) fn patchset(&self, patchset_id: &str) -> Result<PatchsetIdentity, String> {
        self.patchsets_by_id
            .get(patchset_id)
            .copied()
            .ok_or_else(|| format!("unknown same-Repository Patchset {patchset_id:?}"))
    }

    pub(crate) fn patchset_by_immutable_identity(
        &self,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        patchset_number: u32,
    ) -> Result<Option<PatchsetIdentity>, String> {
        if patchset_number == 0 {
            return Err("Patchset immutable identity has a zero ordinal".to_string());
        }
        let Some(base_snapshot_index) = self.snapshots_by_id.get(base_snapshot_id).copied() else {
            return Ok(None);
        };
        let Some(revision_snapshot_index) = self.snapshots_by_id.get(revision_snapshot_id).copied()
        else {
            return Ok(None);
        };
        let mut candidates = self.patchsets_by_id.values().copied().filter(|identity| {
            identity.record.base_snapshot_index == base_snapshot_index
                && identity.record.revision_snapshot_index == revision_snapshot_index
                && identity.patchset_number() == patchset_number
        });
        let first = candidates.next();
        if candidates.next().is_some() {
            return Err(format!(
                "immutable Snapshot pair {base_snapshot_id:?} -> {revision_snapshot_id:?} and Patchset ordinal {patchset_number} select multiple same-Repository Patchsets"
            ));
        }
        Ok(first)
    }

    pub(crate) fn snapshot(&self, snapshot_id: &str) -> Result<u32, String> {
        self.snapshots_by_id
            .get(snapshot_id)
            .copied()
            .ok_or_else(|| format!("unknown same-Repository Snapshot {snapshot_id:?}"))
    }

    pub(crate) fn snapshot_id(&self, snapshot_index: u32) -> Result<&str, String> {
        self.snapshot_ids_by_index
            .get(&snapshot_index)
            .map(String::as_str)
            .ok_or_else(|| format!("unknown same-Repository Snapshot index {snapshot_index}"))
    }

    pub(crate) fn direct_main_land_patchset(&self, submission_id: &str) -> Result<u32, String> {
        let land = self
            .lands_by_id
            .get(submission_id)
            .ok_or_else(|| format!("unknown same-Repository Land {submission_id:?}"))?
            .record;
        let mode = (land.land_meta & LAND_MODE_MASK) >> 5;
        if mode != LAND_MODE_DIRECT {
            return Err(format!(
                "Land {submission_id:?} does not use exact direct mode"
            ));
        }
        let line_index = land
            .target_line_index_plus1
            .checked_sub(1)
            .ok_or_else(|| format!("Land {submission_id:?} has no target Line"))?;
        if self
            .line_names_by_index
            .get(&line_index)
            .map(String::as_str)
            != Some("main")
        {
            return Err(format!(
                "Land {submission_id:?} does not target exact logical main"
            ));
        }
        Ok(land.patchset_index)
    }

    pub(crate) fn validate_successful_main_land_snapshot(
        &self,
        patchset_index: u32,
        snapshot_id: &str,
    ) -> Result<(), String> {
        let snapshot_index = self.snapshot(snapshot_id)?;
        let snapshot_index_plus1 = snapshot_index
            .checked_add(1)
            .ok_or_else(|| "successful Land Snapshot plus-one index overflow".to_string())?;
        let matched = self.lands_by_id.values().any(|identity| {
            let land = identity.record;
            let target_line = land
                .target_line_index_plus1
                .checked_sub(1)
                .and_then(|index| self.line_names_by_index.get(&index))
                .map(String::as_str);
            land.patchset_index == patchset_index
                && land.land_meta & LAND_STATUS_MASK == LAND_STATUS_SUCCEEDED
                && land.land_meta & LAND_HAS_LANDED_SNAPSHOT != 0
                && land.landed_snapshot_index_plus1 == snapshot_index_plus1
                && target_line == Some("main")
        });
        if matched {
            Ok(())
        } else {
            Err(format!(
                "Snapshot {snapshot_id:?} is not an exact successful main Land result for Patchset index {patchset_index}"
            ))
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum PatchsetSourceCodec {
    Transitional,
    Frozen,
}

fn logical_patchset_from_frozen(record: V0FrozenPatchsetRecord) -> V0PatchsetRecord {
    V0PatchsetRecord {
        patchset_meta: record.patchset_meta,
        patch_ordinal: record.patch_ordinal,
        change_ordinal: record.change_ordinal,
        reserved0: record.reserved0,
        change_index: record.change_index,
        previous_task_patchset_index_plus1: record.previous_task_patchset_index_plus1,
        previous_change_patchset_index_plus1: record.previous_change_patchset_index_plus1,
        base_snapshot_index: record.base_snapshot_index,
        revision_snapshot_index: record.revision_snapshot_index,
        created_at_s: record.created_at_s,
        ci_completed_at_s: record.ci_completed_at_s,
        ci_run_seq: record.ci_run_seq,
        ci_selected_suite_count: record.ci_selected_suite_count,
        ci_suite_result_count: record.ci_suite_result_count,
        ci_blocking_failure_count: record.ci_blocking_failure_count,
        ci_status_bits: record.ci_status_bits,
        summary_offset: record.summary_offset,
        summary_len: record.summary_len,
        ci_worker_job_index_plus1: record.ci_worker_job_index_plus1,
    }
}

fn observe_logical_main(
    name: &str,
    head_snapshot_index: Option<u32>,
    main_line_seen: &mut bool,
    main_head_snapshot_index: &mut Option<u32>,
) -> Result<(), String> {
    if name != "main" {
        return Ok(());
    }
    if *main_line_seen {
        return Err("source authority has duplicate logical main Lines".to_string());
    }
    *main_line_seen = true;
    *main_head_snapshot_index = head_snapshot_index;
    Ok(())
}

fn read_records<T>(
    read: &BinaryDbReadTxn<'_, SourceDb>,
    file: ait_server_core::foundation::remote_binary_db::BinaryFileId,
    decode: fn(&[u8]) -> ait_server_core::foundation::remote_binary_db::StoreResult<T>,
) -> Result<Vec<T>, String> {
    let count = read
        .record_count(file.clone())
        .map_err(|error| format!("failed to count {}: {error}", file.as_str()))?;
    (0..count)
        .map(|index| {
            let raw = read.read_record(file.clone(), index).map_err(|error| {
                format!("failed to read {} record {index}: {error}", file.as_str())
            })?;
            decode(&raw).map_err(|error| {
                format!("failed to decode {} record {index}: {error}", file.as_str())
            })
        })
        .collect()
}

fn task_prefix(namespace_ascii: [u8; 2]) -> Result<String, String> {
    let bytes = namespace_ascii
        .into_iter()
        .take_while(|byte| *byte != 0)
        .map(|byte| byte.to_ascii_uppercase())
        .collect::<Vec<_>>();
    let namespace =
        String::from_utf8(bytes).map_err(|_| "Repository namespace is not ASCII".to_string())?;
    Ok(format!("R{namespace}"))
}

fn canonical_real_directory(path: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect source authority {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "source authority is not a real non-symlink directory: {}",
            path.display()
        ));
    }
    std::fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{
        observe_logical_main, PatchsetIdentity, RepositoryDomain, V0ChangeRecord, V0PatchsetRecord,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn unique_logical_main_may_have_no_snapshot_head() {
        let mut seen = false;
        let mut head = Some(99);
        observe_logical_main("main", None, &mut seen, &mut head).unwrap();
        assert!(seen);
        assert_eq!(head, None);
        assert!(observe_logical_main("main", Some(1), &mut seen, &mut head)
            .unwrap_err()
            .contains("duplicate"));
    }

    #[test]
    fn immutable_patchset_identity_finds_a_renamed_tail_and_rejects_ambiguity() {
        let identity = PatchsetIdentity {
            index: 1204,
            record: V0PatchsetRecord {
                patch_ordinal: 0,
                base_snapshot_index: 7,
                revision_snapshot_index: 8,
                ..V0PatchsetRecord::default()
            },
            change: V0ChangeRecord::default(),
        };
        let mut domain = RepositoryDomain {
            source_root: PathBuf::new(),
            patchsets_by_id: BTreeMap::from([("RCT-1208/C-01/P-01".to_string(), identity)]),
            patchsets_by_index: Vec::new(),
            snapshots_by_id: BTreeMap::from([
                ("SNP-ED5FF2BFBF37".to_string(), 7),
                ("SNP-9AAB44D49894".to_string(), 8),
            ]),
            snapshot_ids_by_index: BTreeMap::new(),
            lands_by_id: BTreeMap::new(),
            line_names_by_index: BTreeMap::new(),
            main_head_snapshot_index: None,
        };

        assert_eq!(
            domain
                .patchset_by_immutable_identity("SNP-ED5FF2BFBF37", "SNP-9AAB44D49894", 1,)
                .unwrap()
                .map(|found| found.index),
            Some(identity.index)
        );
        assert!(domain
            .patchset_by_immutable_identity("SNP-ED5FF2BFBF37", "SNP-000000000000", 1,)
            .unwrap()
            .is_none());

        domain.patchsets_by_id.insert(
            "RCT-9999/C-01/P-01".to_string(),
            PatchsetIdentity {
                index: 9999,
                ..identity
            },
        );
        assert!(domain
            .patchset_by_immutable_identity("SNP-ED5FF2BFBF37", "SNP-9AAB44D49894", 1,)
            .unwrap_err()
            .contains("multiple"));
    }
}
