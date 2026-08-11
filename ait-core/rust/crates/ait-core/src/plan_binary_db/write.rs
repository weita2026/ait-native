use std::path::PathBuf;

use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbFsyncPolicy, BinaryDbWriteTxn,
    BinaryFileId, StorePath, StoreResult,
};
use crate::content_binary_db::{BinaryTreeCodec, BinaryTreePackCodec, BinaryTreePackFormatKind};

use super::{
    BinaryDbPlanStore, PlanItemPayload, PlanItemRecord, PlanPayload, PlanRecord,
    PlanRevisionPayload, PlanRevisionRecord, PlanRevisionRootUpdate,
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
        record: PlanRecord,
        payload: &PlanPayload,
    ) -> StoreResult<(u32, PlanRecord)> {
        self.ensure_commit_point_open("append plan")?;
        self.store.append_plan(&mut self.write, record, payload)
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
        let record = self
            .store
            .overwrite_plan(&mut self.write, plan_index, record, payload)?;
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
        self.write.commit()?;
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
        let layout = self
            .write
            .db()
            .layout_id(BinaryDbPlanStore::<B, WRITE_LAYOUT>::plan_file())?;
        let file = BinaryDbPlanStore::<B, WRITE_LAYOUT>::plan_file_for(layout);
        let raw = self.write.read_record(file, plan_index)?;
        BinaryDbPlanStore::<B, WRITE_LAYOUT>::decode_plan_record(layout, &raw)
    }
}

fn revision_ref(index_plus1: u32) -> String {
    index_plus1
        .checked_sub(1)
        .map(|index| format!("plan-revision:{index}"))
        .unwrap_or_else(|| "<none>".to_string())
}
