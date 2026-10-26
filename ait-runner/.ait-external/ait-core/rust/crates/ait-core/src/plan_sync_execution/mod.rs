//! End-to-end `plan sync` orchestration remains concrete to the `plan` domain.
//! It still binds together plan-specific artifact resolution, store/remote
//! coordination, and sync decision behavior.

#[cfg(test)]
mod adoption_boundary_tests;
mod adoption_repair;
mod artifact_body_ports;
mod artifact_body_store;
mod artifact_state_ports;
mod artifact_state_store;
mod content_ports;
mod content_store;
mod identity_ports;
mod identity_source;
mod inventory_target;
mod local_ports;
mod local_store;
mod packed_artifact;
mod prune_sync;
mod publish;
mod remote_client;
mod remote_ports;
mod request_runtime;
mod result_projection;
mod shared;

use self::adoption_repair::{
    adopt_materialized_remote_plan_at_distinct_local_identity,
    adopt_materialized_remote_plan_for_local_sync, adopt_remote_plan_for_local_sync,
    bind_equivalent_local_plan_to_remote_identity, index_plans_by_identity, index_plans_by_path,
    load_remote_revisions_cached, map_equivalent_remote_plan_revision_suffix,
    map_exact_dense_remote_plan_revision_prefix, materialize_remote_plan_head_for_local_adoption,
    materialize_remote_revision, open_candidates, plan_publish_revision_metadata_matches,
    plan_sync_missing_artifact_paths, replace_plan_in_inventory,
    select_divergent_retry_publish_target, select_existing_plan_with_continuity,
    sort_revisions_ascending, tracked_missing_markdown_artifact_paths,
    validate_exact_mixed_local_plan_lineage_split, validate_materialized_remote_plan_lineage,
};
use self::inventory_target::*;
use self::packed_artifact::*;
use self::prune_sync::*;
use self::publish::*;
use self::request_runtime::*;
use self::result_projection::*;
use self::shared::*;

use self::artifact_body_ports::{
    read_plan_revision_artifact_body_with_plan_sync_local_artifact_body_source,
    PlanSyncLocalArtifactBodySource,
};
use self::artifact_body_store::FilesystemPlanSyncLocalArtifactBodySource;
use self::artifact_state_ports::{
    existing_artifact_paths_with_plan_sync_local_artifact_state_source,
    ignored_artifact_paths_with_plan_sync_local_artifact_state_source,
    PlanSyncLocalArtifactStateSource,
};
use self::artifact_state_store::FilesystemPlanSyncLocalArtifactStateSource;
use self::content_ports::{
    blob_chain_depth_with_plan_sync_local_blob_store,
    ensure_blob_bytes_with_plan_sync_local_blob_store,
    existing_zstd_object_pack_bundle_with_plan_sync_zstd_pack_store,
    existing_zstd_tree_pack_bundle_with_plan_sync_zstd_pack_store,
    prepare_artifact_tree_root_locator_with_plan_sync_zstd_pack_store,
    read_blob_bytes_with_plan_sync_local_blob_store,
    upsert_zstd_object_pack_metadata_with_plan_sync_zstd_pack_store,
    upsert_zstd_tree_pack_metadata_with_plan_sync_zstd_pack_store, PlanSyncLocalBlobStore,
    PlanSyncLocalContentStore, PlanSyncZstdObjectPackMetadata, PlanSyncZstdPackStore,
    PlanSyncZstdTreePackMetadata,
};
use self::content_store::BinaryDbPlanSyncLocalContentStore;
use self::identity_ports::{
    timestamp_with_plan_sync_workflow_identity_source,
    workflow_id_with_plan_sync_workflow_identity_source, PlanSyncWorkflowIdentitySource,
};
use self::identity_source::TimeIdentityPlanSyncWorkflowIdentitySource;
#[cfg(test)]
use self::local_ports::PlanSyncLocalPublicationStore;
use self::local_ports::{
    close_plan_with_plan_sync_local_lifecycle_store,
    create_plan_with_plan_sync_local_artifact_writer,
    get_plan_revision_artifact_with_plan_sync_local_store, get_plan_with_plan_sync_local_store,
    list_plan_revisions_with_plan_sync_local_store, list_plan_summaries_with_plan_sync_local_store,
    mark_plan_published_with_plan_sync_local_store,
    rekey_plan_with_plan_sync_local_lifecycle_store,
    revise_plan_with_plan_sync_local_artifact_writer, PlanSyncLocalAdoptionStore,
    PlanSyncLocalArtifactWriter, PlanSyncLocalFullStore, PlanSyncLocalIdentityRebindStore,
    PlanSyncLocalInventoryStore, PlanSyncLocalLifecycleStore, PlanSyncLocalPlanCreate,
    PlanSyncLocalPlanRevision, PlanSyncLocalPlanStore, PlanSyncLocalPublishSource,
    PlanSyncLocalRevisionStore, PlanSyncLocalStore,
};
use self::local_store::BinaryDbPlanSyncLocalStore;
use self::remote_ports::{
    commit_remote_zstd_bulk_with_plan_sync_remote_client, create_plan_with_plan_sync_remote_client,
    get_plan_revision_with_plan_sync_remote_client, get_plan_with_plan_sync_remote_client,
    get_remote_zstd_object_pack_if_present_with_plan_sync_remote_client,
    get_remote_zstd_tree_pack_if_present_with_plan_sync_remote_client,
    list_plan_revisions_with_plan_sync_remote_client,
    list_plan_summaries_with_plan_sync_remote_inventory_source,
    put_plan_revision_artifacts_with_plan_sync_remote_client,
    put_remote_zstd_object_pack_with_plan_sync_remote_client,
    put_remote_zstd_tree_pack_with_plan_sync_remote_client,
    revise_plan_with_plan_sync_remote_client, start_plan_bound_task_with_plan_sync_remote_client,
    update_plan_status_with_plan_sync_remote_client, PlanSyncRemoteContinuitySource,
    PlanSyncRemoteInventorySource, PlanSyncRemotePackedArtifactUploader, PlanSyncRemotePublisher,
    PlanSyncRemoteRevisionArtifactWriter, PlanSyncRemoteRevisionLister,
    PlanSyncRemoteRevisionReader,
};
use crate::binary_db::{AuthorityId, LocalBinaryDbFs, LocalStateScope, StorePath};
use crate::content_binary_db::LocalContentBinaryDb;
use crate::file_io::{FileIoStore, FilesystemFileIoStore};
use crate::json_support::{json, JsonMap, JsonNumber as Number, JsonValue};
use crate::json_support::{JsonCodec, JsonEncodeOptions};
use crate::object_diff::artifact_blob_id;
use crate::pack_substrate::{
    build_pack_members, build_tree_pack_members, default_object_pack_relative_path,
    default_tree_pack_relative_path, read_tree_pack_index_with_format,
    tree_pack_checksums_by_tree_id, validate_content_addressed_zstd_pack_reuse,
    write_pack_archive_with_format, write_tree_pack_archive_with_format, PackFormatKind,
    TreePackFormatKind,
};
use crate::plan_blob_diff::{
    local_plan_fully_published, plan_heads_equivalent, plan_matches_sync_artifact,
};
use crate::plan_filesystem::{
    list_visible_markdown_artifact_paths, read_json_file, read_utf8_text_file,
    resolve_repo_artifact_path, PlanFilesystemError,
};
use crate::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use crate::plan_items::{extract_plan_section, list_plan_section_refs, PlanItem};
use crate::repository_pack_json::{
    ZstdBulkBlobLocatorRow, ZstdBulkCommitRequest, ZstdBulkLineUpdate, ZstdBulkObjectPackRow,
    ZstdBulkSnapshotRow, ZstdBulkTreeLocatorRow, ZstdBulkTreePackRow,
};
use crate::repository_pack_policy::{
    zstd_only_object_pack_write_format, zstd_only_tree_pack_write_format,
};
use crate::server_operational::RepositoryIndex;
use chrono::{SecondsFormat, Utc};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_ACTOR_TYPE: &str = "human";
const DEFAULT_SOURCE_KIND: &str = "manual_edit";
const DEFAULT_PLAN_STATUS: &str = "draft";
const LOCAL_DRAFT_PUBLICATION_STATE: &str = "local_draft";
const PLAN_SYNC_BINARY_DB_WRITE_LAYOUT: u32 = 1;
const MAX_PACK_CHAIN_DEPTH: usize = 8;
const PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH: &str =
    "release/contracts/public_package_targets_contract.json";
const PUBLIC_PACKAGE_TARGETS_GUIDE_PATH: &str = "release/guides/PACKAGE_TARGETS.md";
const PUBLIC_FUTURE_REPO_PREP_CONTRACT_PATH: &str =
    "release/contracts/public_future_repo_extraction_prep_contract.json";
const PUBLIC_FUTURE_REPO_PREP_GUIDE_PATH: &str = "release/guides/FUTURE_REPOSITORY_PREP.md";
const PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_CONTRACT_PATH: &str =
    "release/contracts/public_future_repo_split_dry_run_contract.json";
const PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_GUIDE_PATH: &str =
    "release/guides/FUTURE_REPOSITORY_SPLIT_DRY_RUN.md";

pub struct PlanSyncExecutionJson<S> {
    store: S,
}

impl<S> PlanSyncExecutionJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl PlanSyncExecutionJson<FilesystemFileIoStore> {
    pub fn filesystem() -> Self {
        Self::new(FilesystemFileIoStore)
    }

    pub fn stateless() -> Self {
        Self::filesystem()
    }
}

impl<S> PlanSyncExecutionJson<S>
where
    S: FileIoStore,
{
    pub fn execute_plan_sync_command_request_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let identity_source = TimeIdentityPlanSyncWorkflowIdentitySource::default_source();
        self.execute_plan_sync_command_request_with_workflow_identity_source(
            &identity_source,
            payload_json,
        )
    }

    fn execute_plan_sync_command_request_with_workflow_identity_source<I>(
        &self,
        identity_source: &I,
        payload_json: &str,
    ) -> Result<JsonValue, String>
    where
        I: PlanSyncWorkflowIdentitySource + ?Sized,
    {
        let request = self.parse_request(payload_json)?;
        execute_plan_sync_command_request(&self.store, identity_source, request)
    }

    fn parse_request(&self, payload_json: &str) -> Result<SyncRequest, String> {
        let payload =
            self.parse_object_payload(payload_json, "plan sync command execution request")?;
        parse_request_map(payload)
    }

    fn parse_object_payload(
        &self,
        payload_json: &str,
        label: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        JsonCodec::parse_object_with_error_prefix(
            payload_json,
            &format!("{label} must be valid JSON"),
            &format!("{label} must be a JSON object."),
        )
        .map_err(String::from)
    }
}

#[derive(Clone, Debug)]
struct PlanSyncZstdObjectPackBundle {
    blob_id: String,
    sha256: String,
    byte_count: i64,
    pack_id: String,
    pack_path: PathBuf,
    pack_format: String,
    member_count: i64,
    total_bytes: i64,
    pack_index_entry_name: String,
    pack_index_checksum: String,
    pack_entry_name: String,
    pack_entry_type: String,
    pack_base_blob_id: Option<String>,
    pack_chain_depth: i64,
    created_at: String,
}

#[derive(Clone, Debug)]
struct PlanSyncZstdTreePackBundle {
    root_tree_id: String,
    root_entry_count: i64,
    root_entry_ordinal: i64,
    root_tree_checksum: String,
    pack_id: String,
    pack_path: PathBuf,
    pack_format: String,
    tree_count: i64,
    total_bytes: i64,
    pack_index_entry_name: String,
    pack_index_checksum: String,
    tree_locators: Vec<ZstdBulkTreeLocatorRow>,
    created_at: String,
}

#[derive(Clone, Debug)]
struct PlanSyncPackedArtifactBundle {
    object_pack: PlanSyncZstdObjectPackBundle,
    tree_pack: PlanSyncZstdTreePackBundle,
    artifact_payload: JsonValue,
    commit_payload: ZstdBulkCommitRequest,
}

pub struct RemoteSyncCommitJson<S> {
    store: S,
}

impl<S> RemoteSyncCommitJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RemoteSyncCommitJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> RemoteSyncCommitJson<S> {
    #[expect(
        clippy::too_many_arguments,
        reason = "arguments mirror the versioned bulk commit wire contract"
    )]
    pub fn zstd_bulk_commit_request(
        &self,
        generation_key: &str,
        object_packs: Vec<ZstdBulkObjectPackRow>,
        tree_packs: Vec<ZstdBulkTreePackRow>,
        blob_locators: Vec<ZstdBulkBlobLocatorRow>,
        tree_locators: Vec<ZstdBulkTreeLocatorRow>,
        snapshots: Vec<ZstdBulkSnapshotRow>,
        line_update: Option<ZstdBulkLineUpdate>,
    ) -> ZstdBulkCommitRequest {
        let _ = &self.store;
        ZstdBulkCommitRequest {
            contract: None,
            generation_key: Some(generation_key.to_string()),
            object_packs,
            tree_packs,
            blob_locators,
            tree_locators,
            snapshots,
            line_update,
        }
    }

    fn plan_revision_zstd_bulk_commit_request(
        &self,
        generation_key: &str,
        object_pack: &PlanSyncZstdObjectPackBundle,
        tree_pack: &PlanSyncZstdTreePackBundle,
    ) -> ZstdBulkCommitRequest {
        self.zstd_bulk_commit_request(
            generation_key,
            vec![ZstdBulkObjectPackRow {
                generation_key: Some(generation_key.to_string()),
                pack_id: object_pack.pack_id.clone(),
                repo_name: None,
                repo_id: None,
                status: None,
                pack_format: Some(
                    PackFormatKind::from_persisted(&object_pack.pack_format)
                        .expect("plan sync object pack format should be valid"),
                ),
                member_count: Some(object_pack.member_count),
                total_bytes: Some(object_pack.total_bytes),
                pack_path: None,
                pack_index_entry_name: Some(object_pack.pack_index_entry_name.clone()),
                pack_index_checksum: Some(object_pack.pack_index_checksum.clone()),
                created_at: Some(object_pack.created_at.clone()),
                pack_index: None,
            }],
            vec![ZstdBulkTreePackRow {
                generation_key: Some(generation_key.to_string()),
                pack_id: tree_pack.pack_id.clone(),
                repo_name: None,
                repo_id: None,
                status: None,
                pack_format: Some(
                    TreePackFormatKind::from_persisted(&tree_pack.pack_format)
                        .expect("plan sync tree pack format should be valid"),
                ),
                tree_count: Some(tree_pack.tree_count),
                total_bytes: Some(tree_pack.total_bytes),
                pack_path: None,
                pack_index_entry_name: Some(tree_pack.pack_index_entry_name.clone()),
                pack_index_checksum: Some(tree_pack.pack_index_checksum.clone()),
                created_at: Some(tree_pack.created_at.clone()),
                pack_index: None,
            }],
            vec![ZstdBulkBlobLocatorRow {
                generation_key: Some(generation_key.to_string()),
                blob_id: object_pack.blob_id.clone(),
                sha256: Some(object_pack.sha256.clone()),
                storage_path: None,
                storage_kind: None,
                size_bytes: Some(object_pack.byte_count),
                pack_id: Some(object_pack.pack_id.clone()),
                pack_entry_name: Some(object_pack.pack_entry_name.clone()),
                pack_entry_type: Some(object_pack.pack_entry_type.clone()),
                pack_base_blob_id: object_pack.pack_base_blob_id.clone(),
                pack_chain_depth: Some(object_pack.pack_chain_depth),
                created_at: Some(object_pack.created_at.clone()),
            }],
            tree_pack.tree_locators.clone(),
            Vec::new(),
            None,
        )
    }
}

#[derive(Clone, Debug)]
enum PlanSyncTreeNode {
    Blob {
        blob_id: String,
        size_bytes: i64,
        mode: String,
    },
    Tree {
        children: BTreeMap<String, PlanSyncTreeNode>,
    },
}
#[derive(Clone, Debug)]
struct SyncRequest {
    root_path: String,
    repo_name: String,
    repository_index: Option<RepositoryIndex>,
    id_namespace_prefix: Option<String>,
    created_by: Option<String>,
    target: String,
    plan_ref: Option<String>,
    prune: bool,
    local: bool,
    remote_name: Option<String>,
    remote_repo_name: Option<String>,
    base_url: Option<String>,
    rebase: bool,
    reconcile: bool,
    history_publish_plan_id: Option<String>,
    plan_storage: PlanSyncStorageRequest,
    task_start: Option<PlanSyncTaskStartRequest>,
}

#[derive(Clone, Debug)]
struct PlanSyncTaskStartRequest {
    contract: String,
    idempotency_key: String,
    plan_item_ref: String,
    task: JsonValue,
    change: JsonValue,
}

#[derive(Clone, Debug, Default)]
struct PlanSyncStorageRequest {
    write_layout: Option<u32>,
    authority_root: Option<String>,
    activation_pointer: Option<String>,
    pack_root: Option<String>,
    repo_root: Option<String>,
    local_authority_id: Option<String>,
    current_line_state_scope: Option<LocalStateScope>,
}

impl PlanSyncStorageRequest {
    fn require_binary_layout(&self) -> Result<(), String> {
        match self.write_layout {
            Some(PLAN_SYNC_BINARY_DB_WRITE_LAYOUT) => Ok(()),
            Some(layout) => Err(format!(
                "Plan sync Binary DB selected layout {layout}, but this runtime supports layout {PLAN_SYNC_BINARY_DB_WRITE_LAYOUT}."
            )),
            None => Err("plan_storage.write_layout is required for Binary DB plan sync.".to_string()),
        }
    }

    fn binary_db(&self, expected_repo_name: &str) -> Result<(LocalBinaryDbFs, PathBuf), String> {
        self.require_binary_layout()?;
        let authority_root = self.authority_root.as_deref().ok_or_else(|| {
            "plan_storage.authority_root is required for Binary DB plan sync.".to_string()
        })?;
        let repo_root = self.repo_root.as_deref().ok_or_else(|| {
            "plan_storage.repo_root is required for Binary DB plan sync.".to_string()
        })?;
        let local_authority_id = self.local_authority_id.as_deref().ok_or_else(|| {
            "plan_storage.local_authority_id is required for Binary DB plan sync.".to_string()
        })?;
        let current_line_state_scope = self.current_line_state_scope.ok_or_else(|| {
            "plan_storage.current_line_state_scope is required for Binary DB plan sync.".to_string()
        })?;
        if let Some(pointer) = self.activation_pointer.as_deref() {
            let (generation, guard) =
                crate::binary_db_generation::admit_activated_binary_db_generation_for_runtime(
                    Path::new(repo_root),
                    Path::new(pointer),
                    expected_repo_name,
                )?;
            if generation.authority_root != *authority_root
                || self
                    .pack_root
                    .as_deref()
                    .is_none_or(|root| generation.generation_root != *root)
            {
                return Err(
                    "plan_storage Binary DB authority/pack roots do not match the admitted activation pointer."
                        .to_string(),
                );
            }
            return Ok((
                LocalBinaryDbFs::new(
                    StorePath::from(generation.authority_root),
                    StorePath::from(repo_root),
                    AuthorityId::new(local_authority_id),
                    current_line_state_scope,
                )
                .with_declared_bin_paths(crate::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS)
                .with_declared_index_paths(crate::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS)
                .with_generation_guard(Some(guard)),
                generation.generation_root,
            ));
        }
        if !cfg!(test) {
            return Err(
                "plan_storage.activation_pointer is required for selected Binary DB plan sync."
                    .to_string(),
            );
        }
        Ok((
            LocalBinaryDbFs::new(
                StorePath::from(authority_root),
                StorePath::from(repo_root),
                AuthorityId::new(local_authority_id),
                current_line_state_scope,
            )
            .with_declared_bin_paths(crate::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS)
            .with_declared_index_paths(crate::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS),
            PathBuf::from(self.pack_root.as_deref().unwrap_or(repo_root)),
        ))
    }
}

#[derive(Clone, Debug)]
struct SyncTarget {
    scope: String,
    target_path: String,
    resolved_target: PathBuf,
    files: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct SyncArtifact {
    artifact_path: String,
    artifact_selector: Option<String>,
    artifact_heading: String,
    items: Vec<JsonValue>,
    artifact_body: String,
    artifact_blob_id: String,
}

#[derive(Clone, Debug)]
struct LocalInventory {
    plans: Vec<JsonValue>,
    indexed_plans: BTreeMap<String, Vec<JsonValue>>,
    indexed_by_identity: BTreeMap<(String, Option<String>), Vec<JsonValue>>,
}

#[derive(Clone, Debug)]
struct RemoteInventory {
    plans: Vec<JsonValue>,
    indexed_plans: BTreeMap<String, Vec<JsonValue>>,
    indexed_by_identity: BTreeMap<(String, Option<String>), Vec<JsonValue>>,
    scoped_artifact_path: Option<String>,
    full_loaded: bool,
}

pub fn execute_plan_sync_command_request_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanSyncExecutionJson::filesystem().execute_plan_sync_command_request_json(payload_json)
}

fn execute_plan_sync_command_request<F, I>(
    file_io_store: &F,
    identity_source: &I,
    request: SyncRequest,
) -> Result<JsonValue, String>
where
    F: FileIoStore + ?Sized,
    I: PlanSyncWorkflowIdentitySource + ?Sized,
{
    let publish_remote = request.base_url.is_some();
    let divergent_retry_mode = if request.rebase {
        Some("rebase")
    } else if request.reconcile {
        Some("reconcile")
    } else {
        None
    };
    validate_runtime_flags(&request)?;
    let history_publish = request.history_publish_plan_id.is_some();

    let sync_target = match resolve_sync_target(&request, request.prune || publish_remote) {
        Ok(target) => target,
        Err(err) => {
            return Ok(plan_sync_payload(
                "failed",
                &request,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Some(err),
            ));
        }
    };
    let artifacts = if history_publish {
        Vec::new()
    } else {
        match sync_target
            .files
            .iter()
            .map(|path| resolve_plan_artifact(&request, path, request.plan_ref.as_deref(), true))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(artifacts) => artifacts,
            Err(err) => {
                return Ok(plan_sync_payload(
                    "failed",
                    &request,
                    Some(&sync_target),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Some(err),
                ));
            }
        }
    };
    if request.task_start.is_some()
        && (!sync_target.resolved_target.is_file() || artifacts.len() != 1)
    {
        return Ok(plan_sync_payload(
            "failed",
            &request,
            Some(&sync_target),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(
                "Plan sync task_start requires one exact Markdown file and one selected Plan."
                    .to_string(),
            ),
        ));
    }
    if artifacts.is_empty() && !(request.prune || publish_remote) {
        return Ok(plan_sync_payload(
            "failed",
            &request,
            Some(&sync_target),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(format!(
                "No Markdown plan artifacts found under {}.",
                request.target
            )),
        ));
    }

    let (binary_db, pack_root) = request.plan_storage.binary_db(&request.repo_name)?;
    let binary_plan_store = BinaryDbPlanSyncLocalStore::<PLAN_SYNC_BINARY_DB_WRITE_LAYOUT>::from_db(
        request.repo_name.clone(),
        binary_db.clone(),
    );
    let binary_content_store =
        LocalContentBinaryDb::<PLAN_SYNC_BINARY_DB_WRITE_LAYOUT>::from_db_with_roots(
            binary_db,
            StorePath::from(PathBuf::from(request.root_path.clone())),
            StorePath::from(pack_root),
        );
    let binary_content_store =
        BinaryDbPlanSyncLocalContentStore::<_, PLAN_SYNC_BINARY_DB_WRITE_LAYOUT>::new(
            binary_content_store.blobs().clone(),
            binary_content_store.object_packs().clone(),
            binary_content_store.tree_packs().clone(),
            binary_content_store.trees().clone(),
            binary_content_store.snapshots().clone(),
        );
    let local_plan_store: &dyn PlanSyncLocalFullStore = &binary_plan_store;
    let local_content_store: &dyn PlanSyncLocalContentStore = &binary_content_store;

    let mut local_inventory = load_local_inventory_from_store(local_plan_store)?;
    let mut client = request
        .base_url
        .as_deref()
        .map(|base_url| build_http_client_manager(&request, base_url))
        .transpose()?;
    let mut remote_inventory = if history_publish {
        RemoteInventory {
            plans: Vec::new(),
            indexed_plans: BTreeMap::new(),
            indexed_by_identity: BTreeMap::new(),
            scoped_artifact_path: None,
            full_loaded: false,
        }
    } else if let Some(client_ref) = client.as_mut() {
        load_remote_inventory(client_ref, &request, &sync_target)?
    } else {
        RemoteInventory {
            plans: Vec::new(),
            indexed_plans: BTreeMap::new(),
            indexed_by_identity: BTreeMap::new(),
            scoped_artifact_path: None,
            full_loaded: false,
        }
    };
    let paired_artifacts = if publish_remote && !history_publish {
        match resolve_paired_artifacts(&request, &sync_target, &artifacts) {
            Ok(paired) => paired,
            Err(err) => {
                return Ok(plan_sync_payload(
                    "failed",
                    &request,
                    Some(&sync_target),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Some(err),
                ));
            }
        }
    } else {
        BTreeMap::new()
    };
    if request.task_start.is_some() && !paired_artifacts.is_empty() {
        return Ok(plan_sync_payload(
            "failed",
            &request,
            Some(&sync_target),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(
                "Plan sync task_start does not support paired artifact publication; publish the paired artifacts before starting the Task."
                    .to_string(),
            ),
        ));
    }

    let mut results = if let Some(plan_id) = request.history_publish_plan_id.as_deref() {
        match history_publish_result_row(&request, local_plan_store, plan_id) {
            Ok(row) => vec![row],
            Err(err) => {
                return Ok(plan_sync_payload(
                    "failed",
                    &request,
                    Some(&sync_target),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Some(err),
                ));
            }
        }
    } else {
        Vec::new()
    };
    let mut adoptions = Vec::new();
    let mut synced_artifact_paths = BTreeSet::new();
    let mut remote_revisions_cache: BTreeMap<String, Vec<JsonValue>> = BTreeMap::new();
    let mut remote_revision_detail_cache: BTreeMap<(String, String), JsonValue> = BTreeMap::new();
    let local_artifact_body_source = FilesystemPlanSyncLocalArtifactBodySource::new(file_io_store);
    let local_artifact_state_source = FilesystemPlanSyncLocalArtifactStateSource;

    for artifact in &artifacts {
        let (existing_plan, adoption, continuity_match) = if publish_remote {
            match resolve_local_sync_plan_candidate(
                local_plan_store,
                local_content_store,
                identity_source,
                &request,
                artifact,
                &mut local_inventory,
                &mut remote_inventory,
                client.as_mut(),
                &mut remote_revisions_cache,
                &mut remote_revision_detail_cache,
            ) {
                Ok(candidate) => candidate,
                Err(err) => {
                    return Ok(plan_sync_payload(
                        "failed",
                        &request,
                        Some(&sync_target),
                        results,
                        adoptions,
                        Vec::new(),
                        Vec::new(),
                        Some(err),
                    ));
                }
            }
        } else {
            let (existing_plan, continuity_match) =
                match select_or_reconcile_local_sync_plan_candidate(
                    local_plan_store,
                    identity_source,
                    &request,
                    artifact,
                    &mut local_inventory,
                ) {
                    Ok(candidate) => candidate,
                    Err(err) => {
                        return Ok(plan_sync_payload(
                            "failed",
                            &request,
                            Some(&sync_target),
                            results,
                            adoptions,
                            Vec::new(),
                            Vec::new(),
                            Some(err),
                        ));
                    }
                };
            (existing_plan, None, continuity_match)
        };
        if let Some(adoption_row) = adoption {
            adoptions.push(adoption_row);
        }
        let row = match sync_single_plan_artifact(
            local_plan_store,
            local_content_store,
            identity_source,
            &request,
            artifact,
            existing_plan.as_ref(),
            continuity_match,
        ) {
            Ok(row) => row,
            Err(err) => {
                return Ok(plan_sync_payload(
                    "failed",
                    &request,
                    Some(&sync_target),
                    results,
                    adoptions,
                    Vec::new(),
                    Vec::new(),
                    Some(err),
                ));
            }
        };
        synced_artifact_paths.insert(artifact.artifact_path.clone());
        results.push(row);
        if let Err(err) = load_local_inventory_from_store(local_plan_store) {
            return Ok(plan_sync_payload(
                "failed",
                &request,
                Some(&sync_target),
                results,
                adoptions,
                Vec::new(),
                Vec::new(),
                Some(err),
            ));
        }
    }

    if !history_publish && (request.prune || publish_remote) {
        let (pruned_results, prune_adoptions) = match run_prune_phase(
            local_plan_store,
            local_content_store,
            &local_artifact_state_source,
            identity_source,
            &request,
            &sync_target,
            &mut local_inventory,
            &remote_inventory,
            client.as_mut(),
            &mut remote_revisions_cache,
            &mut remote_revision_detail_cache,
            &synced_artifact_paths,
        ) {
            Ok(pruned) => pruned,
            Err(err) => {
                return Ok(plan_sync_payload(
                    "failed",
                    &request,
                    Some(&sync_target),
                    results,
                    adoptions,
                    Vec::new(),
                    Vec::new(),
                    Some(err),
                ));
            }
        };
        adoptions.extend(prune_adoptions);
        results.extend(pruned_results);
        if let Err(err) = load_local_inventory_from_store(local_plan_store) {
            return Ok(plan_sync_payload(
                "failed",
                &request,
                Some(&sync_target),
                results,
                adoptions,
                Vec::new(),
                Vec::new(),
                Some(err),
            ));
        }
    }

    let publish_results = if publish_remote {
        match publish_synced_local_results(
            &request,
            &results,
            local_plan_store,
            local_content_store,
            &local_artifact_body_source,
            file_io_store,
            identity_source,
            client
                .as_mut()
                .ok_or_else(|| "Remote client is required for publish mode.".to_string())?,
            divergent_retry_mode,
        ) {
            Ok(published) => published,
            Err(err) => {
                return Ok(plan_sync_payload(
                    "failed",
                    &request,
                    Some(&sync_target),
                    results,
                    adoptions,
                    Vec::new(),
                    Vec::new(),
                    Some(err),
                ));
            }
        }
    } else {
        Vec::new()
    };

    let artifact_results = if publish_remote {
        match publish_paired_artifacts(
            &results,
            &paired_artifacts,
            local_plan_store,
            client.as_mut().ok_or_else(|| {
                "Remote client is required for paired artifact upload.".to_string()
            })?,
        ) {
            Ok(artifacts) => artifacts,
            Err(err) => {
                let status = if publish_results.is_empty() {
                    "failed"
                } else {
                    "partial_success"
                };
                return Ok(plan_sync_payload(
                    status,
                    &request,
                    Some(&sync_target),
                    results,
                    adoptions,
                    publish_results,
                    Vec::new(),
                    Some(err),
                ));
            }
        }
    } else {
        Vec::new()
    };

    Ok(plan_sync_payload(
        "ok",
        &request,
        Some(&sync_target),
        results,
        adoptions,
        publish_results,
        artifact_results,
        None,
    ))
}
