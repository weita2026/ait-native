use std::ops::Deref;

use crate::binary_db::{
    AuthorityId, BinaryDbFsyncPolicy, BinaryDbStoreFsyncPolicy, LocalBinaryDbFs, LocalStateScope,
    StorePath, StoreResult,
};

use crate::plan_binary_db::{BinaryDbPlanStore, PlanBinaryDbWritePurpose, PlanBinaryDbWriteTxn};

pub type LocalPlanSyncUpsertTxn<'a, F, const WRITE_LAYOUT: u32> =
    PlanBinaryDbWriteTxn<'a, LocalBinaryDbFs, F, WRITE_LAYOUT>;
pub type LocalPlanSyncPruneTxn<'a, F, const WRITE_LAYOUT: u32> =
    PlanBinaryDbWriteTxn<'a, LocalBinaryDbFs, F, WRITE_LAYOUT>;
pub type LocalPlanSyncAdoptionTxn<'a, F, const WRITE_LAYOUT: u32> =
    PlanBinaryDbWriteTxn<'a, LocalBinaryDbFs, F, WRITE_LAYOUT>;
pub type LocalPlanSyncPublishReceiptTxn<'a, F, const WRITE_LAYOUT: u32> =
    PlanBinaryDbWriteTxn<'a, LocalBinaryDbFs, F, WRITE_LAYOUT>;

/// Local Plan Binary DB adapter over the shared plan store substrate.
pub struct LocalPlanBinaryDb<const WRITE_LAYOUT: u32> {
    inner: BinaryDbPlanStore<LocalBinaryDbFs, WRITE_LAYOUT>,
}

impl<const WRITE_LAYOUT: u32> LocalPlanBinaryDb<WRITE_LAYOUT> {
    pub fn new(
        authority_root: impl Into<StorePath>,
        local_repo_root: impl Into<StorePath>,
        local_authority_id: AuthorityId,
        current_line_state_scope: LocalStateScope,
    ) -> Self {
        Self::from_db(LocalBinaryDbFs::new(
            authority_root,
            local_repo_root,
            local_authority_id,
            current_line_state_scope,
        ))
    }

    pub fn from_db(db: LocalBinaryDbFs) -> Self {
        Self {
            inner: BinaryDbPlanStore::new(db),
        }
    }

    pub fn inner(&self) -> &BinaryDbPlanStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        &self.inner
    }

    pub fn into_inner(self) -> BinaryDbPlanStore<LocalBinaryDbFs, WRITE_LAYOUT> {
        self.inner
    }

    pub fn db(&self) -> &LocalBinaryDbFs {
        self.inner.db()
    }

    pub fn begin_local_upsert_txn(
        &self,
    ) -> StoreResult<
        LocalPlanSyncUpsertTxn<'_, BinaryDbStoreFsyncPolicy<'_, LocalBinaryDbFs>, WRITE_LAYOUT>,
    > {
        self.begin_local_upsert_txn_with_fsync_policy(BinaryDbStoreFsyncPolicy::new(self.db()))
    }

    pub fn begin_local_upsert_txn_with_fsync_policy<F>(
        &self,
        fsync_policy: F,
    ) -> StoreResult<LocalPlanSyncUpsertTxn<'_, F, WRITE_LAYOUT>>
    where
        F: BinaryDbFsyncPolicy,
    {
        self.begin_local_txn_with_fsync_policy(
            PlanBinaryDbWritePurpose::LocalPlanSyncUpsert,
            fsync_policy,
        )
    }

    pub fn begin_local_prune_txn(
        &self,
    ) -> StoreResult<
        LocalPlanSyncPruneTxn<'_, BinaryDbStoreFsyncPolicy<'_, LocalBinaryDbFs>, WRITE_LAYOUT>,
    > {
        self.begin_local_prune_txn_with_fsync_policy(BinaryDbStoreFsyncPolicy::new(self.db()))
    }

    pub fn begin_local_prune_txn_with_fsync_policy<F>(
        &self,
        fsync_policy: F,
    ) -> StoreResult<LocalPlanSyncPruneTxn<'_, F, WRITE_LAYOUT>>
    where
        F: BinaryDbFsyncPolicy,
    {
        self.begin_local_txn_with_fsync_policy(
            PlanBinaryDbWritePurpose::LocalPlanSyncPrune,
            fsync_policy,
        )
    }

    pub fn begin_local_adoption_txn(
        &self,
    ) -> StoreResult<
        LocalPlanSyncAdoptionTxn<'_, BinaryDbStoreFsyncPolicy<'_, LocalBinaryDbFs>, WRITE_LAYOUT>,
    > {
        self.begin_local_adoption_txn_with_fsync_policy(BinaryDbStoreFsyncPolicy::new(self.db()))
    }

    pub fn begin_local_adoption_txn_with_fsync_policy<F>(
        &self,
        fsync_policy: F,
    ) -> StoreResult<LocalPlanSyncAdoptionTxn<'_, F, WRITE_LAYOUT>>
    where
        F: BinaryDbFsyncPolicy,
    {
        self.begin_local_txn_with_fsync_policy(
            PlanBinaryDbWritePurpose::LocalPlanSyncAdoption,
            fsync_policy,
        )
    }

    pub fn begin_local_publish_receipt_txn(
        &self,
    ) -> StoreResult<
        LocalPlanSyncPublishReceiptTxn<
            '_,
            BinaryDbStoreFsyncPolicy<'_, LocalBinaryDbFs>,
            WRITE_LAYOUT,
        >,
    > {
        self.begin_local_publish_receipt_txn_with_fsync_policy(BinaryDbStoreFsyncPolicy::new(
            self.db(),
        ))
    }

    pub fn begin_local_publish_receipt_txn_with_fsync_policy<F>(
        &self,
        fsync_policy: F,
    ) -> StoreResult<LocalPlanSyncPublishReceiptTxn<'_, F, WRITE_LAYOUT>>
    where
        F: BinaryDbFsyncPolicy,
    {
        self.begin_local_txn_with_fsync_policy(
            PlanBinaryDbWritePurpose::LocalPlanSyncPublishReceipt,
            fsync_policy,
        )
    }

    fn begin_local_txn_with_fsync_policy<F>(
        &self,
        purpose: PlanBinaryDbWritePurpose,
        fsync_policy: F,
    ) -> StoreResult<PlanBinaryDbWriteTxn<'_, LocalBinaryDbFs, F, WRITE_LAYOUT>>
    where
        F: BinaryDbFsyncPolicy,
    {
        let write = self
            .inner
            .begin_write_txn_with_fsync_policy(purpose.command_scope(), fsync_policy)?;
        Ok(PlanBinaryDbWriteTxn::new(&self.inner, write, purpose))
    }
}

impl<const WRITE_LAYOUT: u32> Deref for LocalPlanBinaryDb<WRITE_LAYOUT> {
    type Target = BinaryDbPlanStore<LocalBinaryDbFs, WRITE_LAYOUT>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
