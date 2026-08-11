use std::ops::Deref;

use crate::binary_db::{
    BinaryDbFsyncPolicy, BinaryDbStoreFsyncPolicy, RemoteBinaryDb, RemoteBinaryDbFs, StoreResult,
};

use crate::plan_binary_db::{
    BinaryDbPlanStore, PlanBinaryDbWritePurpose, PlanBinaryDbWriteTxn, PlanRevisionRootUpdate,
};

/// Remote Plan Binary DB adapter over the shared plan store substrate.
pub struct RemotePlanBinaryDb<B, const WRITE_LAYOUT: u32>
where
    B: RemoteBinaryDb,
{
    inner: BinaryDbPlanStore<B, WRITE_LAYOUT>,
}

pub type RemoteFsPlanBinaryDb<const WRITE_LAYOUT: u32> =
    RemotePlanBinaryDb<RemoteBinaryDbFs, WRITE_LAYOUT>;
pub use crate::plan_binary_db::PlanBinaryDbCommitPoint as RemotePlanSyncCommitPoint;
pub type RemotePlanSyncPublishTxn<'a, B, F, const WRITE_LAYOUT: u32> =
    PlanBinaryDbWriteTxn<'a, B, F, WRITE_LAYOUT>;
pub type RemotePlanSyncArtifactAttachTxn<'a, B, F, const WRITE_LAYOUT: u32> =
    PlanBinaryDbWriteTxn<'a, B, F, WRITE_LAYOUT>;

impl<B, const WRITE_LAYOUT: u32> RemotePlanBinaryDb<B, WRITE_LAYOUT>
where
    B: RemoteBinaryDb,
{
    pub fn from_db(db: B) -> Self {
        Self {
            inner: BinaryDbPlanStore::new(db),
        }
    }

    pub fn inner(&self) -> &BinaryDbPlanStore<B, WRITE_LAYOUT> {
        &self.inner
    }

    pub fn into_inner(self) -> BinaryDbPlanStore<B, WRITE_LAYOUT> {
        self.inner
    }

    pub fn db(&self) -> &B {
        self.inner.db()
    }

    pub fn begin_publish_txn(
        &self,
    ) -> StoreResult<RemotePlanSyncPublishTxn<'_, B, BinaryDbStoreFsyncPolicy<'_, B>, WRITE_LAYOUT>>
    {
        self.begin_publish_txn_with_fsync_policy(BinaryDbStoreFsyncPolicy::new(self.db()))
    }

    pub fn begin_publish_txn_with_fsync_policy<F>(
        &self,
        fsync_policy: F,
    ) -> StoreResult<RemotePlanSyncPublishTxn<'_, B, F, WRITE_LAYOUT>>
    where
        F: BinaryDbFsyncPolicy,
    {
        let write = self.inner.begin_write_txn_with_fsync_policy(
            PlanBinaryDbWritePurpose::RemotePlanSyncPublish.command_scope(),
            fsync_policy,
        )?;
        Ok(PlanBinaryDbWriteTxn::new(
            &self.inner,
            write,
            PlanBinaryDbWritePurpose::RemotePlanSyncPublish,
        ))
    }

    pub fn begin_artifact_attach_txn_for_roots<F>(
        &self,
        pending_roots: &[PlanRevisionRootUpdate],
        fsync_policy: F,
    ) -> StoreResult<Option<RemotePlanSyncArtifactAttachTxn<'_, B, F, WRITE_LAYOUT>>>
    where
        F: BinaryDbFsyncPolicy,
    {
        if pending_roots.is_empty() {
            return Ok(None);
        }
        let write = self.inner.begin_write_txn_with_fsync_policy(
            PlanBinaryDbWritePurpose::RemotePlanSyncArtifactAttach.command_scope(),
            fsync_policy,
        )?;
        Ok(Some(PlanBinaryDbWriteTxn::new(
            &self.inner,
            write,
            PlanBinaryDbWritePurpose::RemotePlanSyncArtifactAttach,
        )))
    }
}

impl<const WRITE_LAYOUT: u32> RemoteFsPlanBinaryDb<WRITE_LAYOUT> {
    pub fn from_fs(db: RemoteBinaryDbFs) -> Self {
        Self::from_db(db)
    }
}

impl<B, const WRITE_LAYOUT: u32> Deref for RemotePlanBinaryDb<B, WRITE_LAYOUT>
where
    B: RemoteBinaryDb,
{
    type Target = BinaryDbPlanStore<B, WRITE_LAYOUT>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
