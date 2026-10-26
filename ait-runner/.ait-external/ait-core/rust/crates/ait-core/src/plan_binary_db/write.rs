use std::path::PathBuf;

use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbFsyncPolicy, BinaryDbWriteTxn,
    BinaryFileId, StorePath, StoreResult, BIN_FILE_HEADER_BYTES,
};
use crate::content_binary_db::{BinaryTreeCodec, BinaryTreePackCodec, BinaryTreePackFormatKind};

use super::{
    BinaryDbPlanStore, PlanCodec, PlanItemPayload, PlanItemRecord, PlanPayload, PlanRecord,
    PlanRevisionPayload, PlanRevisionRecord, PlanRevisionRootUpdate, PLAN_RECORD_SIZE,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanBinaryDbWritePurpose {
    LocalPlanSyncUpsert,
    LocalPlanSyncPrune,
    LocalPlanSyncAdoption,
    LocalPlanSyncPublishReceipt,
    RemotePlanSyncPublish,
    RemotePlanSyncArtifactAttach,
}

impl PlanBinaryDbWritePurpose {
    pub fn command_scope(self) -> BinaryDbCommandScope {
        match self {
            Self::LocalPlanSyncUpsert => BinaryDbCommandScope::PlanSyncLocal,
            Self::LocalPlanSyncPrune
            | Self::LocalPlanSyncAdoption
            | Self::LocalPlanSyncPublishReceipt => BinaryDbCommandScope::PlanSyncLocalPlan,
            Self::RemotePlanSyncPublish | Self::RemotePlanSyncArtifactAttach => {
                BinaryDbCommandScope::PlanSyncRemote
            }
        }
    }

    fn requires_domain_commit_point(self) -> bool {
        matches!(
            self,
            Self::LocalPlanSyncUpsert
                | Self::RemotePlanSyncPublish
                | Self::RemotePlanSyncArtifactAttach
        )
    }

    fn can_append_plan_revision(self) -> bool {
        matches!(
            self,
            Self::LocalPlanSyncUpsert | Self::RemotePlanSyncPublish
        )
    }

    fn can_attach_revision_root(self) -> bool {
        matches!(self, Self::RemotePlanSyncArtifactAttach)
    }

    fn can_overwrite_plan(self) -> bool {
        matches!(
            self,
            Self::LocalPlanSyncUpsert
                | Self::LocalPlanSyncPrune
                | Self::LocalPlanSyncAdoption
                | Self::LocalPlanSyncPublishReceipt
                | Self::RemotePlanSyncPublish
        )
    }

    fn can_overwrite_revision(self) -> bool {
        matches!(
            self,
            Self::LocalPlanSyncPublishReceipt | Self::RemotePlanSyncPublish
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanBinaryDbCommitPoint {
    PlanRevision { plan_revision_index: u32 },
    Plan { plan_index: u32 },
    RevisionRoot { plan_revision_index: u32 },
    NoRecordCommit { purpose: PlanBinaryDbWritePurpose },
}

pub struct PlanBinaryDbWriteTxn<'a, B, F, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    store: &'a BinaryDbPlanStore<B, WRITE_LAYOUT>,
    write: BinaryDbWriteTxn<'a, B, F>,
    purpose: PlanBinaryDbWritePurpose,
    commit_point: Option<PlanBinaryDbCommitPoint>,
    staged_plan_file: Option<Vec<u8>>,
}

impl<'a, B, F, const WRITE_LAYOUT: u32> PlanBinaryDbWriteTxn<'a, B, F, WRITE_LAYOUT>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    pub fn new(
        store: &'a BinaryDbPlanStore<B, WRITE_LAYOUT>,
        write: BinaryDbWriteTxn<'a, B, F>,
        purpose: PlanBinaryDbWritePurpose,
    ) -> Self {
        Self {
            store,
            write,
            purpose,
            commit_point: None,
            staged_plan_file: None,
        }
    }

    pub fn purpose(&self) -> PlanBinaryDbWritePurpose {
        self.purpose
    }

    pub fn command_scope(&self) -> BinaryDbCommandScope {
        self.write.command_scope()
    }

    pub fn lock_paths(&self) -> Vec<PathBuf> {
        self.write.lock_paths()
    }

    pub fn track_content_dependency_path(&mut self, path: &StorePath) -> StoreResult<()> {
        self.ensure_commit_point_open("track content dependency")?;
        self.write.track_relative_path(path)
    }

    pub fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.ensure_commit_point_open("read a record count")?;
        if file == BinaryDbPlanStore::<B, WRITE_LAYOUT>::plan_file() {
            if let Some(bytes) = self.staged_plan_file.as_deref() {
                return staged_plan_record_count(bytes);
            }
        }
        self.write.record_count(file)
    }

    pub fn require_unchanged_plan(
        &self,
        plan_index: u32,
        expected: &PlanRecord,
    ) -> StoreResult<()> {
        self.ensure_commit_point_open("validate a Plan record")?;
        let current = self.current_plan_record_locked(plan_index)?;
        if &current == expected {
            return Ok(());
        }
        Err(BinaryDbError::invalid_domain_data(format!(
            "Plan PR-{plan_index} state advanced under the Binary DB write lock: expected revision {}, got {}",
            revision_ref(expected.latest_revision_index_plus1),
            revision_ref(current.latest_revision_index_plus1),
        )))
    }

    pub fn bind_revision_content_root(
        &mut self,
        record: &PlanRevisionRecord,
        payload: &PlanRevisionPayload,
    ) -> StoreResult<()> {
        self.ensure_commit_point_open("bind revision content root")?;
        let Some(tree_pack_index) = record.root_tree_pack_index_plus1.checked_sub(1) else {
            if record.root_entry_ordinal != 0 {
                return Err(BinaryDbError::invalid_domain_data(format!(
                    "Plan revision root has ordinal {} without a tree-pack locator",
                    record.root_entry_ordinal
                )));
            }
            return Ok(());
        };
        let artifact_path = payload.artifact_path_text()?;
        let artifact_blob_id = payload.artifact_blob_id_text()?;
        let path_parts = artifact_path.split('/').collect::<Vec<_>>();
        if path_parts.is_empty()
            || path_parts
                .iter()
                .any(|part| part.is_empty() || matches!(*part, "." | ".."))
        {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "Plan revision packed artifact path is not canonical: {artifact_path}"
            )));
        }
        if artifact_blob_id.trim().is_empty() {
            return Err(BinaryDbError::invalid_domain_data(
                "Plan revision packed root requires artifact_blob_id",
            ));
        }

        let tree_pack_file = BinaryTreePackCodec::<WRITE_LAYOUT>::record_file();
        let tree_file = BinaryTreeCodec::<WRITE_LAYOUT>::record_file();
        self.write.track_record_file(tree_pack_file.clone())?;
        self.write.track_record_file(tree_file.clone())?;

        let pack = BinaryTreePackCodec::<WRITE_LAYOUT>::decode_record(
            &self.write.read_record(tree_pack_file, tree_pack_index)?,
        )?;
        if !pack.is_ready() || pack.is_tombstone() {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "Plan revision root tree pack {tree_pack_index} is not ready"
            )));
        }
        if matches!(pack.format_kind(), BinaryTreePackFormatKind::Reserved(_)) {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "Plan revision root tree pack {tree_pack_index} uses an unsupported format"
            )));
        }
        if record.root_entry_ordinal >= pack.tree_count {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "Plan revision root ordinal {} is outside tree pack {tree_pack_index} count {}",
                record.root_entry_ordinal, pack.tree_count
            )));
        }
        let tree_index = pack
            .first_tree_index
            .checked_add(record.root_entry_ordinal)
            .ok_or_else(|| BinaryDbError::corruption("Plan revision root tree index overflow"))?;
        let tree = BinaryTreeCodec::<WRITE_LAYOUT>::decode_record(
            &self.write.read_record(tree_file, tree_index)?,
        )?;
        if tree.is_tombstone() {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "Plan revision root tree {tree_index} is tombstoned"
            )));
        }
        Ok(())
    }

    pub fn append_plan(
        &mut self,
        mut record: PlanRecord,
        payload: &PlanPayload,
    ) -> StoreResult<(u32, PlanRecord)> {
        self.ensure_commit_point_open("append plan")?;
        let range = self.store.append_plan_payload(&mut self.write, payload)?;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16::try_from(range.payload_len).map_err(|_| {
            format!(
                "plan payload length exceeds u16::MAX: {}",
                range.payload_len
            )
        })?;
        let encoded = PlanCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        self.ensure_staged_plan_file()?;
        let bytes = self
            .staged_plan_file
            .as_mut()
            .unwrap_or_else(|| unreachable!("staged Plan root must be initialized"));
        let index = staged_plan_record_count(bytes)?;
        bytes.extend_from_slice(&encoded);
        Ok((index, record))
    }

    pub fn append_plan_item(
        &mut self,
        record: PlanItemRecord,
        payload: &PlanItemPayload,
    ) -> StoreResult<(u32, PlanItemRecord)> {
        self.ensure_commit_point_open("append plan item")?;
        self.store
            .append_plan_item(&mut self.write, record, payload)
    }

    pub fn append_plan_revision_commit(
        &mut self,
        record: PlanRevisionRecord,
        payload: &PlanRevisionPayload,
    ) -> StoreResult<(u32, PlanRevisionRecord)> {
        self.ensure_purpose("append plan revision commit", |purpose| {
            purpose.can_append_plan_revision()
        })?;
        self.ensure_commit_point_open("append plan revision commit")?;
        let (index, record) = self
            .store
            .append_plan_revision(&mut self.write, record, payload)?;
        self.commit_point = Some(PlanBinaryDbCommitPoint::PlanRevision {
            plan_revision_index: index,
        });
        Ok((index, record))
    }

    pub fn overwrite_plan_revision(
        &mut self,
        revision_index: u32,
        record: &PlanRevisionRecord,
    ) -> StoreResult<u32> {
        self.ensure_purpose("overwrite plan revision", |purpose| {
            purpose.can_overwrite_revision()
        })?;
        self.ensure_commit_point_open("overwrite plan revision")?;
        self.store
            .overwrite_plan_revision_record(&mut self.write, revision_index, record)?;
        Ok(revision_index)
    }

    pub fn overwrite_plan_commit(
        &mut self,
        plan_index: u32,
        record: PlanRecord,
        payload: &PlanPayload,
    ) -> StoreResult<(u32, PlanRecord)> {
        self.ensure_purpose("overwrite plan commit", |purpose| {
            purpose.can_overwrite_plan()
        })?;
        self.ensure_plan_commit_allowed()?;
        let range = self.store.append_plan_payload(&mut self.write, payload)?;
        let mut record = record;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16::try_from(range.payload_len).map_err(|_| {
            format!(
                "plan payload length exceeds u16::MAX: {}",
                range.payload_len
            )
        })?;
        let encoded = PlanCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        self.ensure_staged_plan_file()?;
        let bytes = self
            .staged_plan_file
            .as_mut()
            .unwrap_or_else(|| unreachable!("staged Plan root must be initialized"));
        let count = staged_plan_record_count(bytes)?;
        if plan_index >= count {
            return Err(BinaryDbError::missing_data(format!(
                "plan record index {plan_index} out of range for staged plan.bin count {count}"
            )));
        }
        let offset = usize::try_from(BIN_FILE_HEADER_BYTES)
            .unwrap_or(4)
            .checked_add(
                usize::try_from(plan_index)
                    .map_err(|_| format!("plan index overflows usize: {plan_index}"))?
                    .checked_mul(encoded.len())
                    .ok_or_else(|| BinaryDbError::invalid_domain_data("plan offset overflow"))?,
            )
            .ok_or_else(|| BinaryDbError::invalid_domain_data("plan offset overflow"))?;
        let end = offset
            .checked_add(encoded.len())
            .ok_or_else(|| BinaryDbError::invalid_domain_data("plan range overflow"))?;
        bytes[offset..end].copy_from_slice(&encoded);
        self.commit_point = Some(PlanBinaryDbCommitPoint::Plan { plan_index });
        Ok((plan_index, record))
    }

    pub fn attach_revision_root_commit(
        &mut self,
        update: &PlanRevisionRootUpdate,
    ) -> StoreResult<u32> {
        self.ensure_purpose("attach revision root commit", |purpose| {
            purpose.can_attach_revision_root()
        })?;
        self.ensure_commit_point_open("attach revision root commit")?;
        self.store
            .overwrite_plan_revision_root(&mut self.write, update)?;
        let index = update.plan_revision_index;
        self.commit_point = Some(PlanBinaryDbCommitPoint::RevisionRoot {
            plan_revision_index: update.plan_revision_index,
        });
        Ok(index)
    }

    pub fn commit(mut self) -> StoreResult<PlanBinaryDbCommitPoint> {
        let commit_point = match self.commit_point {
            Some(commit_point) => commit_point,
            None if self.purpose.requires_domain_commit_point() => {
                return Err(
                    format!("{:?} has no Binary DB domain commit point", self.purpose).into(),
                );
            }
            None => PlanBinaryDbCommitPoint::NoRecordCommit {
                purpose: self.purpose,
            },
        };
        match self.staged_plan_file.take() {
            Some(bytes) => {
                self.write.commit_with_atomic_file_replacement(
                    BinaryDbPlanStore::<B, WRITE_LAYOUT>::plan_file(),
                    &bytes,
                    "Plan root commit point",
                )?;
            }
            None => {
                self.write.commit()?;
            }
        }
        Ok(commit_point)
    }

    pub fn abort(mut self) -> StoreResult<()> {
        self.write.abort()
    }

    fn ensure_commit_point_open(&self, action: &str) -> StoreResult<()> {
        if self.commit_point.is_some() {
            return Err(format!(
                "{:?} cannot {action} after Binary DB domain commit point",
                self.purpose
            )
            .into());
        }
        Ok(())
    }

    fn ensure_plan_commit_allowed(&self) -> StoreResult<()> {
        match (&self.commit_point, self.purpose) {
            (None, _) => Ok(()),
            (
                Some(PlanBinaryDbCommitPoint::PlanRevision { .. }),
                PlanBinaryDbWritePurpose::LocalPlanSyncUpsert
                | PlanBinaryDbWritePurpose::RemotePlanSyncPublish,
            ) => Ok(()),
            (Some(_), _) => Err(format!(
                "{:?} cannot overwrite plan after Binary DB domain commit point",
                self.purpose
            )
            .into()),
        }
    }

    fn ensure_purpose(
        &self,
        action: &str,
        is_allowed: impl FnOnce(PlanBinaryDbWritePurpose) -> bool,
    ) -> StoreResult<()> {
        if is_allowed(self.purpose) {
            return Ok(());
        }
        Err(format!("{:?} cannot {action}", self.purpose).into())
    }

    fn current_plan_record_locked(&self, plan_index: u32) -> StoreResult<PlanRecord> {
        if let Some(bytes) = self.staged_plan_file.as_deref() {
            let count = staged_plan_record_count(bytes)?;
            if plan_index >= count {
                return Err(BinaryDbError::missing_data(format!(
                    "plan record index {plan_index} out of range for staged plan.bin count {count}"
                )));
            }
            let record_size = usize::try_from(PlanCodec::<WRITE_LAYOUT>::RECORD_SIZE)
                .map_err(|_| "Plan record size overflows usize".to_string())?;
            let offset = usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4)
                + usize::try_from(plan_index)
                    .map_err(|_| format!("plan index overflows usize: {plan_index}"))?
                    * record_size;
            return PlanCodec::<WRITE_LAYOUT>::decode_record(&bytes[offset..offset + record_size]);
        }
        let layout = self
            .write
            .db()
            .layout_id(BinaryDbPlanStore::<B, WRITE_LAYOUT>::plan_file())?;
        let file = BinaryDbPlanStore::<B, WRITE_LAYOUT>::plan_file_for(layout);
        let raw = self.write.read_record(file, plan_index)?;
        BinaryDbPlanStore::<B, WRITE_LAYOUT>::decode_plan_record(layout, &raw)
    }

    fn ensure_staged_plan_file(&mut self) -> StoreResult<()> {
        if self.staged_plan_file.is_some() {
            return Ok(());
        }
        let file = BinaryDbPlanStore::<B, WRITE_LAYOUT>::plan_file();
        let count = self.write.record_count(file.clone())?;
        let record_size = usize::try_from(file.record_size())
            .map_err(|_| format!("Plan record size overflows usize: {}", file.record_size()))?;
        let count_usize = usize::try_from(count)
            .map_err(|_| format!("Plan record count overflows usize: {count}"))?;
        let mut bytes = Vec::with_capacity(
            usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4)
                + count_usize.saturating_mul(record_size),
        );
        bytes.extend_from_slice(&WRITE_LAYOUT.to_le_bytes());
        for index in 0..count {
            bytes.extend_from_slice(&self.write.read_record(file.clone(), index)?);
        }
        self.staged_plan_file = Some(bytes);
        Ok(())
    }
}

fn staged_plan_record_count(bytes: &[u8]) -> StoreResult<u32> {
    let header_len = usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4);
    let record_size = usize::try_from(PLAN_RECORD_SIZE)
        .map_err(|_| "Plan record size overflows usize".to_string())?;
    let body_len = bytes
        .len()
        .checked_sub(header_len)
        .ok_or_else(|| BinaryDbError::corruption("staged plan.bin has no complete header"))?;
    if !body_len.is_multiple_of(record_size) {
        return Err(BinaryDbError::corruption(format!(
            "staged plan.bin body length {body_len} is not aligned to {record_size}"
        )));
    }
    u32::try_from(body_len / record_size)
        .map_err(|_| BinaryDbError::corruption("staged plan.bin record count exceeds u32::MAX"))
}

fn revision_ref(index_plus1: u32) -> String {
    index_plus1
        .checked_sub(1)
        .map(|index| format!("plan-revision:{index}"))
        .unwrap_or_else(|| "<none>".to_string())
}
