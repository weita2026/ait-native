use crate::json_support::{encode_value_pretty_with_newline_error_string, parse_object_or_empty};
use ait_core::binary_db::{
    AuthorityId, BinaryDbReadTxn, LocalBinaryDbFs, LocalStateScope, StorePath,
};
use ait_core::binary_db_generation::admit_activated_binary_db_generation_for_runtime;
#[cfg(test)]
use ait_core::change_store::ChangeStore;
use ait_core::content_binary_db::{
    tree_id_from_hash80, BinaryDbBlobStore, BinaryDbObjectPackStore, BinaryDbSnapshotStore,
    BinaryDbTreePackStore, BinaryDbTreeReadCache, BinaryDbTreeStore, BinaryObjectPackMemberKind,
    BinarySnapshotView, BinaryTreePackView, BinaryTreeRecord, BinaryTreeView, LocalContentBinaryDb,
};
use ait_core::json_support::json;
use ait_core::json_support::JsonNumber;
use ait_core::json_support::{JsonMap, JsonValue};
use ait_core::line_binary_db::BinaryDbLineStore;
use ait_core::line_store::{LineRecord, LineStore};
#[cfg(test)]
use ait_core::local_content_gc::{
    LocalContentOrphanPackPruneStore, LocalContentStatsStore, LocalContentValidationStore,
};
use ait_core::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotReadStore, LocalSnapshotTreeReadStore,
    LocalSnapshotWriteStore, SnapshotFileRow, SnapshotPathBlobRow, SnapshotPathDelta,
    SnapshotTreeRootLocator,
};
use ait_core::pack_substrate::{
    pack_index_checksum_with_format, read_pack_index_with_format, read_tree_pack_index_with_format,
    tree_pack_index_checksum_with_format, PACK_FORMAT_ZSTD_CHUNKED_V1,
    TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use ait_core::plan_binary_db::LocalRepositoryPlanStore;
use ait_core::remote_store::{ConfigRemoteStore, RemoteStore};
use ait_core::remote_sync_local_store::{
    BinaryDbRemoteSyncZstdImportStore, RemoteSyncLocalInventoryMetadata,
    RemoteSyncLocalInventorySource, RemoteSyncLocalSnapshotParent, RemoteSyncLocalSnapshotSource,
    RemoteSyncLocalStoreContext, RemoteSyncZstdImportSource, RemoteSyncZstdLocalPlanSource,
    ZstdBulkLocalPlan, ZstdImportApplyResult, ZstdImportDownloadPlan, ZstdImportHistoryMode,
    ZstdImportPackStageResult,
};
use ait_core::repo_status_store::{BinaryDbRepoStatusStore, RepoStatusStore};
use ait_core::repository_pack_json::{
    ZstdBulkObjectPackRow, ZstdBulkSnapshotRow, ZstdBulkTreePackRow, ZstdImportManifestPayload,
};
use ait_core::server_operational::{
    RepositoryIndex, ServerRepositoryAuthorityConfig, REPOSITORY_INDEX_CONFIG_KEY,
};
use ait_core::snapshot_store::{
    SnapshotParentLink, SnapshotParentLinkPage, SnapshotRecord, SnapshotStore, SnapshotStoreResult,
};
use ait_core::stash_binary_db::BinaryDbStashStore;
#[cfg(test)]
use ait_core::stash_store::StashStore;
#[cfg(test)]
use ait_core::task_store::TaskStore;
use ait_core::workflow_binary_db::BinaryDbWorkflowStore;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const APP_DIR: &str = ".ait";
const BINARY_DB_DIR: &str = "binary-db";
const WORKTREE_CONFIG_NAME: &str = ".ait-worktree.json";
const CONFIG_NAME: &str = "config.json";
const REPO_DISCOVERY_ENV_VARS: &[&str] = &[ait_core::environment_contract::names::AIT_REPO_ROOT];
const DEFAULT_AUTHOR_MODE: &str = "ai_with_human_review";
const DEFAULT_WORKFLOW_SCOPE: &str = "local";
const DEFAULT_PLAN_TASK_BINDING_MODE: &str = "required";
pub const SNAPSHOT_BINARY_DB_WRITE_LAYOUT: u32 = 1;
pub const REMOTE_SYNC_BINARY_DB_WRITE_LAYOUT: u32 = 1;

pub(crate) fn canonical_repository_directory_name(root: &Path) -> Result<String, String> {
    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            format!(
                "Canonical Repository root {} must have a UTF-8 directory name.",
                root.display()
            )
        })?;
    if name.is_empty() || name.trim() != name {
        return Err(format!(
            "Canonical Repository root directory name {name:?} must be non-empty and have no surrounding whitespace."
        ));
    }
    Ok(name.to_string())
}

#[cfg(test)]
pub(crate) fn create_binary_test_snapshot(
    repo_root: &str,
    repo_name: &str,
    line_name: &str,
    message: Option<&str>,
    is_worktree: bool,
) -> Result<JsonValue, String> {
    let repo = RepoRuntime::discover_from_path(Path::new(repo_root))?;
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    ait_core::local_snapshot::create_snapshot_with_local_snapshot_operation_store(
        &store,
        repo_name,
        line_name,
        message,
        is_worktree,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalPlanHeadArtifact {
    pub status: String,
    pub artifact_path: String,
    pub artifact_blob_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ControlPlaneStoreFamily {
    Line,
    CurrentLine,
    Stash,
    Remote,
    RepoStatus,
}

impl ControlPlaneStoreFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::CurrentLine => "current_line",
            Self::Stash => "stash",
            Self::Remote => "remote",
            Self::RepoStatus => "repo_status",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Line => "LineStore",
            Self::CurrentLine => "CurrentLineStore",
            Self::Stash => "StashStore",
            Self::Remote => "RemoteStore",
            Self::RepoStatus => "RepoStatusStore",
        }
    }
}

pub const CONTROL_PLANE_STORE_FAMILIES: &[ControlPlaneStoreFamily] = &[
    ControlPlaneStoreFamily::Line,
    ControlPlaneStoreFamily::CurrentLine,
    ControlPlaneStoreFamily::Stash,
    ControlPlaneStoreFamily::Remote,
    ControlPlaneStoreFamily::RepoStatus,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlPlaneStoreDecisionMode {
    SelectedBinaryDb,
    RepositoryConfig,
}

impl ControlPlaneStoreDecisionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelectedBinaryDb => "selected_binary_db",
            Self::RepositoryConfig => "repository_config",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlPlaneStoreDecision {
    pub family: ControlPlaneStoreFamily,
    pub mode: ControlPlaneStoreDecisionMode,
    pub owner_phase: &'static str,
    pub runtime_accessor: &'static str,
    pub reason: &'static str,
}

impl ControlPlaneStoreDecision {
    pub fn to_json(&self) -> JsonValue {
        let mut object = JsonMap::new();
        object.insert(
            "family".to_string(),
            JsonValue::String(self.family.as_str().to_string()),
        );
        object.insert(
            "label".to_string(),
            JsonValue::String(self.family.label().to_string()),
        );
        object.insert(
            "mode".to_string(),
            JsonValue::String(self.mode.as_str().to_string()),
        );
        object.insert(
            "owner_phase".to_string(),
            JsonValue::String(self.owner_phase.to_string()),
        );
        object.insert(
            "runtime_accessor".to_string(),
            JsonValue::String(self.runtime_accessor.to_string()),
        );
        object.insert(
            "reason".to_string(),
            JsonValue::String(self.reason.to_string()),
        );
        JsonValue::Object(object)
    }
}

#[derive(Clone, Debug)]
pub struct RepoRuntime {
    pub root: PathBuf,
    pub ait_dir: PathBuf,
    pub config: JsonMap<String, JsonValue>,
    pub worktree_config_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct RepoBinaryDbStoreFactory<const WRITE_LAYOUT: u32> {
    repo_root: PathBuf,
    authority_root: PathBuf,
    pack_root: PathBuf,
    local_authority_id: AuthorityId,
    id_namespace_prefix: String,
    current_line_state_scope: LocalStateScope,
    admission_error: Option<String>,
    generation_guard:
        Option<std::sync::Arc<std::sync::Mutex<ait_core::binary_db::BinaryDbReadLockSet>>>,
}

pub type RepoLocalSnapshotOperationStore<const WRITE_LAYOUT: u32> =
    RepoBinaryDbLocalSnapshotOperationStore<WRITE_LAYOUT>;
pub type RepoLocalContentMaintenanceStore<const WRITE_LAYOUT: u32> =
    LocalContentBinaryDb<WRITE_LAYOUT>;
pub type RepoStashStore<const WRITE_LAYOUT: u32> =
    BinaryDbStashStore<LocalBinaryDbFs, WRITE_LAYOUT>;
pub type RepoWorkflowStore<const WRITE_LAYOUT: u32> =
    BinaryDbWorkflowStore<LocalBinaryDbFs, WRITE_LAYOUT>;

pub type RepoRemoteSyncLocalStore<const WRITE_LAYOUT: u32> =
    RepoRemoteSyncBinaryDbLocalStore<WRITE_LAYOUT>;

#[derive(Clone, Debug)]
pub struct RepoBinaryDbLocalSnapshotOperationStore<const WRITE_LAYOUT: u32> {
    content: LocalContentBinaryDb<WRITE_LAYOUT>,
    lines: BinaryDbLineStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    worktree_config_path: Option<PathBuf>,
}

pub struct RepoRemoteSyncBinaryDbLocalStore<const WRITE_LAYOUT: u32> {
    import_store: BinaryDbRemoteSyncZstdImportStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    lines: BinaryDbLineStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    blobs: BinaryDbBlobStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    snapshots: BinaryDbSnapshotStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    object_packs: BinaryDbObjectPackStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    tree_packs: BinaryDbTreePackStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    trees: BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
}

#[derive(Clone, Debug)]
pub struct RemoteRow {
    pub name: String,
    pub url: String,
    pub repo_name: Option<String>,
}

mod content_snapshot_operations;
mod remote_clients;
mod repository_context;
mod runtime_construction;
mod selected_storage_adapters;
mod workflow_stores;

use self::content_snapshot_operations::*;
use self::runtime_construction::*;

#[cfg(test)]
mod tests;
