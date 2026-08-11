use crate::json_support::JsonValue;
use crate::json_support::{optional_array_field, optional_bool_field, required_object_value};
use crate::pack_substrate::{
    PackFormatKind, TreePackFormatKind, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use crate::repository_pack_json::{RemoteSyncBackendPayload, RemoteSyncBackendPayloadJson};
use std::collections::BTreeSet;

pub const REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK: &str = "remote_sync.pack_bulk.zstd.v1";
pub const REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK_DOWNLOAD: &str =
    "remote_sync.pack_bulk.zstd.download.v1";
pub const REMOTE_SYNC_CAPABILITY_ZSTD_PULL_MANIFEST: &str =
    "remote_sync.pack_bulk.zstd.pull_manifest.v1";
pub const REMOTE_SYNC_CAPABILITY_SNAPSHOT_DAG_V2: &str = "remote_sync.snapshot_dag.v2";
pub const ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE: &str =
    "application/vnd.ait.remote-sync.object-pack+zstd";
pub const ZSTD_BULK_TREE_PACK_MEDIA_TYPE: &str = "application/vnd.ait.remote-sync.tree-pack+zstd";

pub struct RemoteSyncPlanJson<S> {
    store: S,
}

impl<S> RemoteSyncPlanJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RemoteSyncPlanJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> RemoteSyncPlanJson<S> {
    pub fn capabilities_from_server_payload(
        &self,
        payload: Option<&JsonValue>,
    ) -> RemoteSyncCapabilities {
        let _ = &self.store;
        let Some(payload) = payload else {
            return RemoteSyncCapabilities::default();
        };
        let mut capabilities = RemoteSyncCapabilities::default();
        for remote_sync in remote_sync_capability_payloads(payload) {
            if capability_list_contains(remote_sync, REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK) {
                capabilities.zstd_pack_bulk = true;
            }
            if capability_list_contains(remote_sync, REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK_DOWNLOAD)
            {
                capabilities.zstd_pack_bulk_download = true;
            }
            if capability_list_contains(remote_sync, REMOTE_SYNC_CAPABILITY_ZSTD_PULL_MANIFEST) {
                capabilities.zstd_pull_manifest = true;
            }
            if capability_list_contains(remote_sync, REMOTE_SYNC_CAPABILITY_SNAPSHOT_DAG_V2) {
                capabilities.snapshot_dag_v2 = true;
            }
            if let Some(value) = bool_capability(remote_sync, &["zstd_pack_bulk"]) {
                capabilities.zstd_pack_bulk = value;
            }
            if let Some(value) = bool_capability(remote_sync, &["zstd_pack_bulk_download"]) {
                capabilities.zstd_pack_bulk_download = value;
            }
            if let Some(value) = bool_capability(remote_sync, &["zstd_pull_manifest"]) {
                capabilities.zstd_pull_manifest = value;
            }
            if let Some(value) = bool_capability(remote_sync, &["snapshot_dag_v2"]) {
                capabilities.snapshot_dag_v2 = value;
            }
        }
        capabilities
    }

    pub fn inventory_diff_from_present_snapshot_ids(
        &self,
        checked_snapshot_ids: &[String],
        present_snapshot_ids: &BTreeSet<String>,
    ) -> RemoteSyncInventoryDiff {
        let _ = &self.store;
        let mut present = Vec::new();
        let mut missing = Vec::new();
        for snapshot_id in checked_snapshot_ids {
            if present_snapshot_ids.contains(snapshot_id) {
                present.push(snapshot_id.clone());
            } else {
                missing.push(snapshot_id.clone());
            }
        }
        RemoteSyncInventoryDiff {
            checked_snapshot_ids: checked_snapshot_ids.to_vec(),
            present_snapshot_ids: present,
            missing_snapshot_ids: missing,
        }
    }

    pub fn backend_payload(
        &self,
        negotiation: &RemoteSyncBackendNegotiation,
        diff: &RemoteSyncInventoryDiff,
    ) -> JsonValue {
        let _ = &self.store;
        RemoteSyncBackendPayloadJson::stateless()
            .encode_value(&self.backend_domain(negotiation, diff))
            .expect("remote sync backend payload DTO should be valid")
    }

    pub fn backend_domain(
        &self,
        negotiation: &RemoteSyncBackendNegotiation,
        diff: &RemoteSyncInventoryDiff,
    ) -> RemoteSyncBackendPayload {
        let _ = &self.store;
        RemoteSyncBackendPayload {
            backend: negotiation.backend,
            reason: negotiation.reason.to_string(),
            capabilities: negotiation.capabilities.clone(),
            diff: diff.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteSyncBackendKind {
    ZstdPackBulk,
}

impl RemoteSyncBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ZstdPackBulk => "zstd_pack_bulk",
        }
    }
}

pub trait RemoteSyncBackend {
    fn kind(&self) -> RemoteSyncBackendKind;
    fn required_capability(&self) -> &'static str;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ZstdPackBulkRemoteBackend;

impl RemoteSyncBackend for ZstdPackBulkRemoteBackend {
    fn kind(&self) -> RemoteSyncBackendKind {
        RemoteSyncBackendKind::ZstdPackBulk
    }

    fn required_capability(&self) -> &'static str {
        REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RemoteSyncCapabilities {
    pub zstd_pack_bulk: bool,
    pub zstd_pack_bulk_download: bool,
    pub zstd_pull_manifest: bool,
    pub snapshot_dag_v2: bool,
}

impl RemoteSyncCapabilities {
    pub fn with_zstd_pack_bulk() -> Self {
        Self {
            zstd_pack_bulk: true,
            zstd_pack_bulk_download: false,
            zstd_pull_manifest: false,
            snapshot_dag_v2: false,
        }
    }

    pub fn with_zstd_pack_bulk_download() -> Self {
        Self {
            zstd_pack_bulk: true,
            zstd_pack_bulk_download: true,
            zstd_pull_manifest: false,
            snapshot_dag_v2: false,
        }
    }

    pub fn with_snapshot_dag_v2(mut self) -> Self {
        self.snapshot_dag_v2 = true;
        self
    }

    pub fn with_zstd_pull_manifest(mut self) -> Self {
        self.zstd_pull_manifest = true;
        self
    }

    pub fn from_server_payload(payload: Option<&JsonValue>) -> Self {
        RemoteSyncPlanJson::stateless().capabilities_from_server_payload(payload)
    }

    pub fn supports(&self, backend: RemoteSyncBackendKind) -> bool {
        match backend {
            RemoteSyncBackendKind::ZstdPackBulk => self.zstd_pack_bulk,
        }
    }
}

pub fn require_snapshot_dag_remote_capability(
    capabilities: &RemoteSyncCapabilities,
    multi_parent_snapshot_ids: &[String],
) -> Result<(), String> {
    if multi_parent_snapshot_ids.is_empty() || capabilities.snapshot_dag_v2 {
        return Ok(());
    }
    Err(format!(
        "Remote must advertise capability {REMOTE_SYNC_CAPABILITY_SNAPSHOT_DAG_V2} before uploading multi-parent snapshots; refusing before mutation (first affected snapshot: {}).",
        multi_parent_snapshot_ids[0]
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSyncSnapshotInventory {
    pub snapshot_ids: Vec<String>,
    pub object_pack_formats: BTreeSet<String>,
    pub tree_pack_formats: BTreeSet<String>,
}

impl RemoteSyncSnapshotInventory {
    pub fn empty() -> Self {
        Self {
            snapshot_ids: Vec::new(),
            object_pack_formats: BTreeSet::new(),
            tree_pack_formats: BTreeSet::new(),
        }
    }

    pub fn from_pack_formats<I, O, T>(snapshot_ids: I, object_formats: O, tree_formats: T) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
        O: IntoIterator,
        O::Item: Into<String>,
        T: IntoIterator,
        T::Item: Into<String>,
    {
        Self {
            snapshot_ids: snapshot_ids.into_iter().map(Into::into).collect(),
            object_pack_formats: object_formats
                .into_iter()
                .filter_map(|value| normalize_format_text(value.into()))
                .collect(),
            tree_pack_formats: tree_formats
                .into_iter()
                .filter_map(|value| normalize_format_text(value.into()))
                .collect(),
        }
    }

    pub fn validate_formats(&self) -> Result<(), String> {
        for format in &self.object_pack_formats {
            PackFormatKind::from_persisted(format)?;
        }
        for format in &self.tree_pack_formats {
            TreePackFormatKind::from_persisted(format)?;
        }
        Ok(())
    }

    pub fn uses_zstd_pack_formats(&self) -> Result<bool, String> {
        self.validate_formats()?;
        Ok(self
            .object_pack_formats
            .iter()
            .any(|format| is_zstd_object_pack_format(format))
            || self
                .tree_pack_formats
                .iter()
                .any(|format| is_zstd_tree_pack_format(format)))
    }

    pub fn uses_only_zstd_pack_formats(&self) -> Result<bool, String> {
        self.validate_formats()?;
        let has_pack_formats =
            !self.object_pack_formats.is_empty() || !self.tree_pack_formats.is_empty();
        Ok(has_pack_formats
            && self
                .object_pack_formats
                .iter()
                .all(|format| is_zstd_object_pack_format(format))
            && self
                .tree_pack_formats
                .iter()
                .all(|format| is_zstd_tree_pack_format(format)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSyncInventoryDiff {
    pub checked_snapshot_ids: Vec<String>,
    pub present_snapshot_ids: Vec<String>,
    pub missing_snapshot_ids: Vec<String>,
}

impl RemoteSyncInventoryDiff {
    pub fn from_present_snapshot_ids(
        checked_snapshot_ids: &[String],
        present_snapshot_ids: &BTreeSet<String>,
    ) -> Self {
        RemoteSyncPlanJson::stateless()
            .inventory_diff_from_present_snapshot_ids(checked_snapshot_ids, present_snapshot_ids)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSyncBackendNegotiation {
    pub backend: RemoteSyncBackendKind,
    pub reason: &'static str,
    pub capabilities: RemoteSyncCapabilities,
}

pub fn negotiate_remote_sync_backend(
    local_inventory: &RemoteSyncSnapshotInventory,
    capabilities: &RemoteSyncCapabilities,
) -> Result<RemoteSyncBackendNegotiation, String> {
    let local_uses_only_zstd = local_inventory.uses_only_zstd_pack_formats()?;
    if !local_uses_only_zstd {
        return Err(
            "Remote sync requires all local pack metadata to use the current zstd formats."
                .to_string(),
        );
    }
    if capabilities.zstd_pack_bulk {
        return Ok(RemoteSyncBackendNegotiation {
            backend: RemoteSyncBackendKind::ZstdPackBulk,
            reason: "local_inventory_and_remote_support_zstd_pack_bulk",
            capabilities: capabilities.clone(),
        });
    }
    Err(format!(
        "Remote sync requires capability {REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK}."
    ))
}

pub fn validate_remote_sync_backend_request(
    requested_backend: RemoteSyncBackendKind,
    local_inventory: &RemoteSyncSnapshotInventory,
    capabilities: &RemoteSyncCapabilities,
) -> Result<(), String> {
    validate_remote_sync_backend_capability(requested_backend, capabilities)?;
    let local_uses_only_zstd = local_inventory.uses_only_zstd_pack_formats()?;
    if requested_backend == RemoteSyncBackendKind::ZstdPackBulk && !local_uses_only_zstd {
        return Err(
            "ZstdPackBulkRemoteBackend requires all local pack metadata to use the current zstd formats."
                .to_string(),
        );
    }
    Ok(())
}

pub fn validate_remote_sync_backend_capability(
    requested_backend: RemoteSyncBackendKind,
    capabilities: &RemoteSyncCapabilities,
) -> Result<(), String> {
    if capabilities.supports(requested_backend) {
        return Ok(());
    }
    Err(format!(
        "Remote does not advertise capability required by {}.",
        requested_backend.as_str()
    ))
}

pub fn remote_sync_backend_payload(
    negotiation: &RemoteSyncBackendNegotiation,
    diff: &RemoteSyncInventoryDiff,
) -> JsonValue {
    RemoteSyncPlanJson::stateless().backend_payload(negotiation, diff)
}

fn capability_list_contains(payload: &JsonValue, capability: &str) -> bool {
    let Some(object) = required_object_value(payload, "remote sync capability payload").ok() else {
        return false;
    };
    let Some(capabilities) = optional_array_field(object, "capabilities").ok().flatten() else {
        return false;
    };
    capabilities
        .iter()
        .any(|value| value.as_str() == Some(capability))
}

fn remote_sync_capability_payloads(payload: &JsonValue) -> Vec<&JsonValue> {
    let mut values = vec![payload];
    for container in [
        payload.get("capabilities"),
        payload.get("ci_capabilities"),
        Some(payload),
    ]
    .into_iter()
    .flatten()
    {
        for key in [
            "remote_sync_capabilities",
            "remote_sync",
            "remote_sync_backend_capabilities",
        ] {
            if let Some(value) = container.get(key) {
                values.push(value);
            }
        }
    }
    values
}

fn bool_capability(payload: &JsonValue, names: &[&str]) -> Option<bool> {
    let object = required_object_value(payload, "remote sync capability payload").ok()?;
    for name in names {
        if let Some(value) = optional_bool_field(object, name).ok().flatten() {
            return Some(value);
        }
    }
    None
}

fn normalize_format_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_zstd_object_pack_format(format: &str) -> bool {
    matches!(
        format.trim(),
        PACK_FORMAT_ZSTD_CHUNKED_V1 | PACK_FORMAT_KIND_ZSTD_CHUNKED_V1
    )
}

fn is_zstd_tree_pack_format(format: &str) -> bool {
    matches!(
        format.trim(),
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 | TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1
    )
}

#[cfg(test)]
mod tests;
