use crate::json_support::{JsonCodec, JsonEncodeOptions, JsonValue};
use crate::pack_substrate::{PackFormatKind, TreePackFormatKind};
use crate::remote_sync_backend::{
    RemoteSyncBackendKind, RemoteSyncCapabilities, RemoteSyncInventoryDiff,
};
use crate::repository_pack_policy::{
    ObjectPackIndexInventory, RepositoryBlobLocatorInventoryRow, RepositoryLineHeadInventoryRow,
    RepositoryObjectPackInventoryRow, RepositoryPackInventory, RepositorySnapshotInventoryRow,
    RepositoryTreeLocatorInventoryRow, RepositoryTreePackInventoryRow, TreePackIndexInventory,
};

mod codec;

use self::codec::*;
pub(crate) use self::codec::{
    validate_zstd_import_manifest_object_pack_row, validate_zstd_import_manifest_snapshot_row,
    validate_zstd_import_manifest_tree_pack_row,
};

pub trait JsonPayloadContract {
    type Domain;
    type Error;

    const CONTRACT_NAME: &'static str;
    const CONTENT_TYPE: &'static str = "application/json";

    fn decode_str(&self, text: &str) -> Result<Self::Domain, Self::Error>;
    fn decode_bytes(&self, bytes: &[u8]) -> Result<Self::Domain, Self::Error>;
    fn encode_string(&self, domain: &Self::Domain) -> Result<String, Self::Error>;
    fn encode_bytes(&self, domain: &Self::Domain) -> Result<Vec<u8>, Self::Error>;
    fn normalize_domain(&self, domain: Self::Domain) -> Result<Self::Domain, Self::Error>;
    fn validate_domain(&self, domain: &Self::Domain) -> Result<(), Self::Error>;
}

pub const REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD: &str = "pack_storage";
pub const REPOSITORY_PACK_STORAGE_CAPABILITY_FIELD: &str = "repository_pack_storage";
pub const ZSTD_IMPORT_MANIFEST_CONTRACT_NAME: &str = "ait.remote_sync.zstd_bulk.import_manifest.v1";
pub const ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_NAME: &str =
    "ait.remote_sync.zstd_bulk.pull_manifest.request.v1";
pub const ZSTD_PULL_MANIFEST_CONTRACT_NAME: &str = "ait.remote_sync.zstd_bulk.pull_manifest.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryPackStorageContract {
    V1,
}

impl RepositoryPackStorageContract {
    pub const NAME: &'static str = "ait.repository.pack_storage.v1";

    fn as_str(&self) -> &'static str {
        match self {
            Self::V1 => Self::NAME,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryPackStorageValidationState {
    NotLoaded,
    Valid,
    Invalid,
}

impl RepositoryPackStorageValidationState {
    fn as_str(&self) -> &'static str {
        match self {
            Self::NotLoaded => "not_loaded",
            Self::Valid => "valid",
            Self::Invalid => "invalid",
        }
    }

    fn from_str(value: &str) -> Result<Self, String> {
        match value.trim() {
            "not_loaded" => Ok(Self::NotLoaded),
            "valid" => Ok(Self::Valid),
            "invalid" => Ok(Self::Invalid),
            other => Err(format!(
                "Unsupported repository pack storage validation state: {other}"
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryPackStorageValidationPayload {
    pub state: RepositoryPackStorageValidationState,
    pub error_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryPackStoragePayload {
    pub contract: RepositoryPackStorageContract,
    pub zstd_only_verified: bool,
    pub object_pack_format: PackFormatKind,
    pub tree_pack_format: TreePackFormatKind,
    pub object_pack_count: u64,
    pub tree_pack_count: u64,
    pub zstd_object_pack_count: u64,
    pub zstd_tree_pack_count: u64,
    pub requires_zstd_remote_sync: bool,
    pub validation: RepositoryPackStorageValidationPayload,
}

impl RepositoryPackStoragePayload {
    pub fn current_default() -> Self {
        Self {
            contract: RepositoryPackStorageContract::V1,
            zstd_only_verified: true,
            object_pack_format: PackFormatKind::ZstdChunkedV1,
            tree_pack_format: TreePackFormatKind::ZstdChunkedTreeV1,
            object_pack_count: 0,
            tree_pack_count: 0,
            zstd_object_pack_count: 0,
            zstd_tree_pack_count: 0,
            requires_zstd_remote_sync: true,
            validation: RepositoryPackStorageValidationPayload {
                state: RepositoryPackStorageValidationState::Valid,
                error_count: 0,
            },
        }
    }

    pub fn from_repository_payload(payload: Option<&JsonValue>) -> Result<Option<Self>, String> {
        let Some(payload) = payload else {
            return Ok(None);
        };
        let Some(object) = payload.as_object() else {
            return Err("repository payload must be a JSON object.".to_string());
        };
        let Some(value) = object.get(REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD) else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        RepositoryPackStorageJson::stateless()
            .decode_value(value.clone())
            .map(Some)
    }
}

pub fn repository_payload_with_validated_pack_storage(
    payload: JsonValue,
) -> Result<JsonValue, String> {
    let object = object_from_value(payload, "repository payload")?;
    let payload = JsonValue::Object(object);
    RepositoryPackStoragePayload::from_repository_payload(Some(&payload))?;
    Ok(payload)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryPackInventoryPayload {
    pub repo_name: String,
    pub object_packs: Vec<RepositoryObjectPackInventoryRow>,
    pub tree_packs: Vec<RepositoryTreePackInventoryRow>,
    pub blob_locators: Vec<RepositoryBlobLocatorInventoryRow>,
    pub tree_locators: Vec<RepositoryTreeLocatorInventoryRow>,
    pub snapshots: Vec<RepositorySnapshotInventoryRow>,
    pub line_heads: Vec<RepositoryLineHeadInventoryRow>,
}

impl From<RepositoryPackInventory> for RepositoryPackInventoryPayload {
    fn from(inventory: RepositoryPackInventory) -> Self {
        Self {
            repo_name: inventory.repo_name,
            object_packs: inventory.object_packs,
            tree_packs: inventory.tree_packs,
            blob_locators: inventory.blob_locators,
            tree_locators: inventory.tree_locators,
            snapshots: inventory.snapshots,
            line_heads: inventory.line_heads,
        }
    }
}

impl From<RepositoryPackInventoryPayload> for RepositoryPackInventory {
    fn from(payload: RepositoryPackInventoryPayload) -> Self {
        Self {
            repo_name: payload.repo_name,
            object_packs: payload.object_packs,
            tree_packs: payload.tree_packs,
            blob_locators: payload.blob_locators,
            tree_locators: payload.tree_locators,
            snapshots: payload.snapshots,
            line_heads: payload.line_heads,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdImportManifestPayload {
    pub contract: String,
    pub repo_name: String,
    pub snapshot_id: String,
    pub snapshots: Vec<ZstdBulkSnapshotRow>,
    pub object_packs: Vec<ZstdBulkObjectPackRow>,
    pub tree_packs: Vec<ZstdBulkTreePackRow>,
    pub blob_locators: Vec<ZstdBulkBlobLocatorRow>,
    pub tree_locators: Vec<ZstdBulkTreeLocatorRow>,
    pub line_update: Option<ZstdBulkLineUpdate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdPullManifestRequest {
    pub contract: String,
    pub head_snapshot_id: String,
    pub have_snapshot_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdPullManifestPayload {
    pub contract: String,
    pub repo_name: String,
    pub head_snapshot_id: String,
    pub boundary_snapshot_ids: Vec<String>,
    pub snapshots: Vec<ZstdBulkSnapshotRow>,
    pub object_packs: Vec<ZstdBulkObjectPackRow>,
    pub tree_packs: Vec<ZstdBulkTreePackRow>,
    pub blob_locators: Vec<ZstdBulkBlobLocatorRow>,
    pub tree_locators: Vec<ZstdBulkTreeLocatorRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkPlanRequest {
    pub snapshot_ids: Vec<String>,
    pub object_packs: Vec<ZstdBulkObjectPackRow>,
    pub tree_packs: Vec<ZstdBulkTreePackRow>,
}

impl ZstdBulkPlanRequest {
    pub fn from_json_rows(
        snapshot_ids: Vec<String>,
        object_packs: Vec<JsonValue>,
        tree_packs: Vec<JsonValue>,
    ) -> Result<Self, String> {
        Ok(Self {
            snapshot_ids,
            object_packs: object_packs
                .into_iter()
                .map(object_pack_row_from_value)
                .collect::<Result<Vec<_>, _>>()?,
            tree_packs: tree_packs
                .into_iter()
                .map(tree_pack_row_from_value)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkPlanResponse {
    pub repo_name: Option<String>,
    pub present_snapshot_ids: Vec<String>,
    pub missing_snapshot_ids: Vec<String>,
    pub present_object_pack_ids: Vec<String>,
    pub missing_object_pack_ids: Vec<String>,
    pub present_tree_pack_ids: Vec<String>,
    pub missing_tree_pack_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSyncBackendPayload {
    pub backend: RemoteSyncBackendKind,
    pub reason: String,
    pub capabilities: RemoteSyncCapabilities,
    pub diff: RemoteSyncInventoryDiff,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkCommitRequest {
    pub contract: Option<String>,
    pub generation_key: Option<String>,
    pub object_packs: Vec<ZstdBulkObjectPackRow>,
    pub tree_packs: Vec<ZstdBulkTreePackRow>,
    pub blob_locators: Vec<ZstdBulkBlobLocatorRow>,
    pub tree_locators: Vec<ZstdBulkTreeLocatorRow>,
    pub snapshots: Vec<ZstdBulkSnapshotRow>,
    pub line_update: Option<ZstdBulkLineUpdate>,
}

impl ZstdBulkCommitRequest {
    #[expect(
        clippy::too_many_arguments,
        reason = "arguments mirror the versioned bulk commit JSON contract"
    )]
    pub fn from_json_rows(
        contract: Option<String>,
        generation_key: Option<String>,
        object_packs: Vec<JsonValue>,
        tree_packs: Vec<JsonValue>,
        blob_locators: Vec<JsonValue>,
        tree_locators: Vec<JsonValue>,
        snapshots: Vec<JsonValue>,
        line_update: Option<ZstdBulkLineUpdate>,
    ) -> Result<Self, String> {
        Ok(Self {
            contract,
            generation_key,
            object_packs: object_packs
                .into_iter()
                .map(object_pack_row_from_value)
                .collect::<Result<Vec<_>, _>>()?,
            tree_packs: tree_packs
                .into_iter()
                .map(tree_pack_row_from_value)
                .collect::<Result<Vec<_>, _>>()?,
            blob_locators: blob_locators
                .into_iter()
                .map(blob_locator_row_from_value)
                .collect::<Result<Vec<_>, _>>()?,
            tree_locators: tree_locators
                .into_iter()
                .map(tree_locator_row_from_value)
                .collect::<Result<Vec<_>, _>>()?,
            snapshots: snapshots
                .into_iter()
                .map(snapshot_row_from_value)
                .collect::<Result<Vec<_>, _>>()?,
            line_update,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkCommitResponse {
    pub repo_name: Option<String>,
    pub committed_snapshot_ids: Vec<String>,
    pub committed_object_pack_ids: Vec<String>,
    pub committed_tree_pack_ids: Vec<String>,
    pub upserted_snapshots: Option<i64>,
    pub remote_line: Option<ZstdBulkRemoteLine>,
    pub line_update: Option<ZstdBulkLineUpdateResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkRemoteLine {
    pub repo_name: Option<String>,
    pub line_name: Option<String>,
    pub status: Option<String>,
    pub head_snapshot_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdPackUploadResponse {
    pub repo_name: Option<String>,
    pub pack_id: String,
    pub stored: Option<bool>,
    pub pack_format: Option<ZstdPackFormat>,
    pub checksum: Option<String>,
    pub pack_bytes: Option<i64>,
    pub raw_binary_upload: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ZstdPackFormat {
    Object(PackFormatKind),
    Tree(TreePackFormatKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkObjectPackRow {
    pub generation_key: Option<String>,
    pub pack_id: String,
    pub repo_name: Option<String>,
    pub repo_id: Option<String>,
    pub status: Option<String>,
    pub pack_format: Option<PackFormatKind>,
    pub member_count: Option<i64>,
    pub total_bytes: Option<i64>,
    pub pack_path: Option<String>,
    pub pack_index_entry_name: Option<String>,
    pub pack_index_checksum: Option<String>,
    pub created_at: Option<String>,
    pub pack_index: Option<ObjectPackIndexInventory>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkTreePackRow {
    pub generation_key: Option<String>,
    pub pack_id: String,
    pub repo_name: Option<String>,
    pub repo_id: Option<String>,
    pub status: Option<String>,
    pub pack_format: Option<TreePackFormatKind>,
    pub tree_count: Option<i64>,
    pub total_bytes: Option<i64>,
    pub pack_path: Option<String>,
    pub pack_index_entry_name: Option<String>,
    pub pack_index_checksum: Option<String>,
    pub created_at: Option<String>,
    pub pack_index: Option<TreePackIndexInventory>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkBlobLocatorRow {
    pub generation_key: Option<String>,
    pub blob_id: String,
    pub sha256: Option<String>,
    pub storage_path: Option<String>,
    pub storage_kind: Option<String>,
    pub size_bytes: Option<i64>,
    pub pack_id: Option<String>,
    pub pack_entry_name: Option<String>,
    pub pack_entry_type: Option<String>,
    pub pack_base_blob_id: Option<String>,
    pub pack_chain_depth: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkTreeLocatorRow {
    pub generation_key: Option<String>,
    pub tree_id: String,
    pub entry_count: Option<i64>,
    pub tree_pack_id: Option<String>,
    pub tree_pack_checksum: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkSnapshotRow {
    pub snapshot_id: String,
    pub parent_snapshot_ids: Vec<String>,
    pub primary_parent_snapshot_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub root_tree_pack_id: Option<String>,
    pub root_entry_ordinal: Option<i64>,
    pub manifest_hash: Option<String>,
    pub message: Option<String>,
    pub line_name: Option<String>,
    pub snapshot_kind: Option<String>,
    pub file_count: Option<i64>,
    pub total_bytes: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkLineUpdate {
    pub line_name: String,
    pub head_snapshot_id: Option<String>,
    pub expected_head_snapshot_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZstdBulkLineUpdateResult {
    pub line_name: Option<String>,
    pub head_snapshot_id: Option<String>,
    pub updated: Option<bool>,
}

pub struct RepositoryPackStorageJson<S> {
    store: S,
}

pub struct ZstdImportManifestJson<S> {
    store: S,
}

pub struct ZstdPullManifestRequestJson<S> {
    store: S,
}

pub struct ZstdPullManifestJson<S> {
    store: S,
}

pub struct RepositoryPackInventoryJson<S> {
    store: S,
}

pub struct ZstdBulkPlanRequestJson<S> {
    store: S,
}

pub struct ZstdBulkPlanResponseJson<S> {
    store: S,
}

pub struct RemoteSyncBackendPayloadJson<S> {
    store: S,
}

pub struct ZstdBulkCommitRequestJson<S> {
    store: S,
}

pub struct ZstdBulkCommitResponseJson<S> {
    store: S,
}

pub struct ZstdPackUploadResponseJson<S> {
    store: S,
}

macro_rules! impl_stateless_json_wrapper {
    ($type_name:ident) => {
        impl<S> $type_name<S> {
            pub fn new(store: S) -> Self {
                Self { store }
            }
        }

        impl $type_name<()> {
            pub fn stateless() -> Self {
                Self::new(())
            }
        }
    };
}

impl_stateless_json_wrapper!(RepositoryPackStorageJson);
impl_stateless_json_wrapper!(ZstdImportManifestJson);
impl_stateless_json_wrapper!(ZstdPullManifestRequestJson);
impl_stateless_json_wrapper!(ZstdPullManifestJson);
impl_stateless_json_wrapper!(RepositoryPackInventoryJson);
impl_stateless_json_wrapper!(ZstdBulkPlanRequestJson);
impl_stateless_json_wrapper!(ZstdBulkPlanResponseJson);
impl_stateless_json_wrapper!(RemoteSyncBackendPayloadJson);
impl_stateless_json_wrapper!(ZstdBulkCommitRequestJson);
impl_stateless_json_wrapper!(ZstdBulkCommitResponseJson);
impl_stateless_json_wrapper!(ZstdPackUploadResponseJson);

macro_rules! impl_contract {
    ($type_name:ident, $domain:ty, $contract_name:expr, $to_value:ident, $from_value:ident, $validate:ident) => {
        impl<S> JsonPayloadContract for $type_name<S> {
            type Domain = $domain;
            type Error = String;

            const CONTRACT_NAME: &'static str = $contract_name;

            fn decode_str(&self, text: &str) -> Result<Self::Domain, Self::Error> {
                let _ = &self.store;
                parse_json_value(text, Self::CONTRACT_NAME).and_then($from_value)
            }

            fn decode_bytes(&self, bytes: &[u8]) -> Result<Self::Domain, Self::Error> {
                let _ = &self.store;
                parse_json_bytes(bytes, Self::CONTRACT_NAME).and_then($from_value)
            }

            fn encode_string(&self, domain: &Self::Domain) -> Result<String, Self::Error> {
                let _ = &self.store;
                self.validate_domain(domain)?;
                JsonCodec::encode_value(&$to_value(domain)?, JsonEncodeOptions::compact())
                    .map_err(String::from)
            }

            fn encode_bytes(&self, domain: &Self::Domain) -> Result<Vec<u8>, Self::Error> {
                self.encode_string(domain).map(String::into_bytes)
            }

            fn normalize_domain(&self, domain: Self::Domain) -> Result<Self::Domain, Self::Error> {
                self.validate_domain(&domain)?;
                Ok(domain)
            }

            fn validate_domain(&self, domain: &Self::Domain) -> Result<(), Self::Error> {
                $validate(domain)
            }
        }

        impl<S> $type_name<S> {
            pub fn encode_value(&self, domain: &$domain) -> Result<JsonValue, String> {
                let _ = &self.store;
                self.validate_domain(domain)?;
                $to_value(domain)
            }

            pub fn decode_value(&self, value: JsonValue) -> Result<$domain, String> {
                let _ = &self.store;
                $from_value(value).and_then(|domain| self.normalize_domain(domain))
            }
        }
    };
}

impl_contract!(
    RepositoryPackStorageJson,
    RepositoryPackStoragePayload,
    RepositoryPackStorageContract::NAME,
    pack_storage_to_value,
    pack_storage_from_value,
    validate_pack_storage
);
impl_contract!(
    ZstdImportManifestJson,
    ZstdImportManifestPayload,
    ZSTD_IMPORT_MANIFEST_CONTRACT_NAME,
    import_manifest_to_value,
    import_manifest_from_value,
    validate_import_manifest
);
impl_contract!(
    ZstdPullManifestRequestJson,
    ZstdPullManifestRequest,
    ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_NAME,
    pull_manifest_request_to_value,
    pull_manifest_request_from_value,
    validate_pull_manifest_request
);
impl_contract!(
    ZstdPullManifestJson,
    ZstdPullManifestPayload,
    ZSTD_PULL_MANIFEST_CONTRACT_NAME,
    pull_manifest_to_value,
    pull_manifest_from_value,
    validate_pull_manifest
);
impl_contract!(
    RepositoryPackInventoryJson,
    RepositoryPackInventoryPayload,
    "ait.repository.pack_inventory.v1",
    pack_inventory_to_value,
    pack_inventory_from_value,
    validate_pack_inventory
);
impl_contract!(
    ZstdBulkPlanRequestJson,
    ZstdBulkPlanRequest,
    "ait.remote_sync.zstd_bulk.plan.request.v1",
    plan_request_to_value,
    plan_request_from_value,
    validate_plan_request
);
impl_contract!(
    ZstdBulkPlanResponseJson,
    ZstdBulkPlanResponse,
    "ait.remote_sync.zstd_bulk.plan.response.v1",
    plan_response_to_value,
    plan_response_from_value,
    validate_plan_response
);
impl_contract!(
    RemoteSyncBackendPayloadJson,
    RemoteSyncBackendPayload,
    "ait.remote_sync.backend_payload.v1",
    backend_payload_to_value,
    backend_payload_from_value,
    validate_backend_payload
);
impl_contract!(
    ZstdBulkCommitRequestJson,
    ZstdBulkCommitRequest,
    "ait.remote_sync.zstd_bulk.commit.v1",
    commit_request_to_value,
    commit_request_from_value,
    validate_commit_request
);
impl_contract!(
    ZstdBulkCommitResponseJson,
    ZstdBulkCommitResponse,
    "ait.remote_sync.zstd_bulk.commit.response.v1",
    commit_response_to_value,
    commit_response_from_value,
    validate_commit_response
);
impl_contract!(
    ZstdPackUploadResponseJson,
    ZstdPackUploadResponse,
    "ait.remote_sync.zstd_bulk.pack_upload.response.v1",
    pack_upload_response_to_value,
    pack_upload_response_from_value,
    validate_pack_upload_response
);

#[cfg(test)]
mod tests;
