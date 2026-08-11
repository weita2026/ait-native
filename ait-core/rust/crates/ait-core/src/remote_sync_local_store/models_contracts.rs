use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSyncLocalStoreContext {
    repo_root: PathBuf,
}

impl RemoteSyncLocalStoreContext {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
        }
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

pub(super) fn repo_stored_path(ctx: &RemoteSyncLocalStoreContext, stored_path: &str) -> PathBuf {
    let path = PathBuf::from(stored_path);
    if path.is_absolute() {
        path
    } else {
        ctx.repo_root().join(path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSyncLocalInventoryMetadata {
    pub object_pack_formats: BTreeSet<String>,
    pub tree_pack_formats: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSyncLocalSnapshotParent {
    pub snapshot_id: String,
    pub parent_snapshot_ids: Vec<String>,
    pub primary_parent_snapshot_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ZstdBulkLocalPack {
    pub pack_id: String,
    pub pack_abs_path: PathBuf,
    pub metadata: JsonValue,
}

#[derive(Clone, Debug)]
pub struct ZstdBulkLocalPlan {
    pub snapshot_order: Vec<String>,
    pub snapshots: BTreeMap<String, JsonValue>,
    pub object_packs: BTreeMap<String, ZstdBulkLocalPack>,
    pub tree_packs: BTreeMap<String, ZstdBulkLocalPack>,
    pub tree_pack_order: Vec<String>,
    pub blob_locators: BTreeMap<String, JsonValue>,
    pub tree_locators: BTreeMap<String, JsonValue>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ZstdImportDownloadPlan {
    pub missing_object_pack_ids: Vec<String>,
    pub reusable_object_pack_ids: Vec<String>,
    pub missing_tree_pack_ids: Vec<String>,
    pub reusable_tree_pack_ids: Vec<String>,
    pub reusable_object_pack_stamps: BTreeMap<String, LocalPackValidationStamp>,
    pub reusable_tree_pack_stamps: BTreeMap<String, LocalPackValidationStamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPackValidationStamp {
    pub pack_id: String,
    pub pack_path: PathBuf,
    pub expected_index_checksum: String,
    pub file_identity: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ZstdImportApplyResult {
    pub snapshot_id: String,
    pub imported_snapshot: bool,
    pub downloaded_object_packs: i64,
    pub reused_object_packs: i64,
    pub downloaded_tree_packs: i64,
    pub reused_tree_packs: i64,
    pub upserted_blob_locators: i64,
    pub upserted_tree_locators: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ZstdImportPackStageResult {
    pub downloaded_object_packs: i64,
    pub reused_object_packs: i64,
    pub downloaded_tree_packs: i64,
    pub reused_tree_packs: i64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ZstdImportHistoryMode {
    #[default]
    CompleteAncestry,
    RemoteHeadBoundary,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ZstdImportMetadataCommitResult {
    pub imported_snapshot: bool,
    pub upserted_blob_locators: i64,
    pub upserted_tree_locators: i64,
}

pub trait RemoteSyncLocalInventorySource {
    fn snapshot_inventory_metadata(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        snapshot_ids: &[String],
    ) -> Result<RemoteSyncLocalInventoryMetadata, String>;
}

pub trait RemoteSyncLocalSnapshotSource {
    fn snapshot_parent_rows(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
    ) -> Result<Vec<RemoteSyncLocalSnapshotParent>, String>;

    fn snapshot_content_complete(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        snapshot_id: &str,
    ) -> Result<bool, String>;
}

pub trait RemoteSyncZstdLocalPlanSource {
    fn zstd_bulk_local_plan(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        snapshot_ids: &[String],
        present_set: &BTreeSet<String>,
    ) -> Result<ZstdBulkLocalPlan, String>;
}

pub trait RemoteSyncZstdImportSource {
    fn zstd_import_download_plan(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        manifest: &ZstdImportManifestPayload,
    ) -> Result<ZstdImportDownloadPlan, String>;

    fn import_zstd_manifest(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        manifest: &ZstdImportManifestPayload,
        history_mode: ZstdImportHistoryMode,
        plan: &ZstdImportDownloadPlan,
        object_pack_bytes: &BTreeMap<String, Vec<u8>>,
        tree_pack_bytes: &BTreeMap<String, Vec<u8>>,
    ) -> Result<ZstdImportApplyResult, String>;

    fn stage_zstd_import_pack_batch(
        &self,
        _ctx: &RemoteSyncLocalStoreContext,
        _object_packs: &[ZstdBulkObjectPackRow],
        _tree_packs: &[ZstdBulkTreePackRow],
        _object_pack_bytes: &BTreeMap<String, Vec<u8>>,
        _tree_pack_bytes: &BTreeMap<String, Vec<u8>>,
    ) -> Result<ZstdImportPackStageResult, String> {
        Err("Bounded Zstd import pack staging is not supported by this local store.".to_string())
    }

    fn import_zstd_snapshot_rows(
        &self,
        _ctx: &RemoteSyncLocalStoreContext,
        _snapshots: &[crate::repository_pack_json::ZstdBulkSnapshotRow],
    ) -> Result<Vec<String>, String> {
        Err("Direct Zstd snapshot-row import is not supported by this local store.".to_string())
    }
}

pub trait RemoteSyncZstdImportTransactionStore {
    fn zstd_import_snapshot_exists(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        snapshot_id: &str,
    ) -> Result<bool, String>;

    fn commit_zstd_import_metadata(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        manifest: &ZstdImportManifestPayload,
        history_mode: ZstdImportHistoryMode,
    ) -> Result<ZstdImportMetadataCommitResult, String>;
}

pub fn zstd_import_snapshot_exists_with_remote_sync_zstd_import_transaction_store<S>(
    store: &S,
    ctx: &RemoteSyncLocalStoreContext,
    snapshot_id: &str,
) -> Result<bool, String>
where
    S: RemoteSyncZstdImportTransactionStore + ?Sized,
{
    store.zstd_import_snapshot_exists(ctx, snapshot_id)
}

pub fn commit_zstd_import_metadata_with_remote_sync_zstd_import_transaction_store<S>(
    store: &S,
    ctx: &RemoteSyncLocalStoreContext,
    manifest: &ZstdImportManifestPayload,
    history_mode: ZstdImportHistoryMode,
) -> Result<ZstdImportMetadataCommitResult, String>
where
    S: RemoteSyncZstdImportTransactionStore + ?Sized,
{
    store.commit_zstd_import_metadata(ctx, manifest, history_mode)
}
