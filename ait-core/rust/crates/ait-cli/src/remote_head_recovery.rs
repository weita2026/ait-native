use crate::init_surface::{init_repo_for_remote_head_recovery, InitRequest};
use crate::json_support::parse_value;
use crate::runtime::{RepoRuntime, REMOTE_SYNC_BINARY_DB_WRITE_LAYOUT};
use ait_core::binary_db_generation::{
    activate_binary_db_generation, capture_binary_db_generation,
    snapshot_binary_db_authority_fingerprint, BinaryDbGenerationActivationOptions,
    CaptureBinaryDbGenerationOptions,
};
use ait_core::json_support::{json, JsonValue};
use ait_core::line_store::LineStore;
use ait_core::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use ait_core::remote_store::{ConfigRemoteStore, RemoteRecord, RemoteStore};
use ait_core::remote_sync_local_store::{
    RemoteSyncLocalStoreContext, RemoteSyncZstdImportSource, ZstdImportHistoryMode,
};
use ait_core::repository_pack_json::ZstdImportManifestPayload;
use ait_core::server_operational::RepositoryIndex;
use ait_core::snapshot_dag::topological_snapshot_order;
use ait_core::snapshot_store::normalize_snapshot_parent_set;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

const RECOVERY_ROOT_NAME: &str = "remote-head-recovery";
const MAX_RECOVERY_JOBS: usize = 64;
const REPOSITORY_CONFIG_PATH: &str = ".ait/config.json";
const WORKTREE_CONFIG_NAME: &str = ".ait-worktree.json";
const RECOVERY_DISCOVERY_ENV_VARS: &[&str] = &[
    "AIT_REPO_ROOT",
    "AIT_NATIVE_WORKSPACE_ROOT",
    "AIT_WORKSPACE_ROOT",
];

/// The deliberately narrow bootstrap available when local Binary DB authority
/// cannot pass normal runtime admission. It reads repository identity, remote
/// coordinates, and authentication context only; it never opens local stores.
#[derive(Clone, Debug)]
pub struct RemoteHeadRecoveryContext {
    root: PathBuf,
    config_path: PathBuf,
    config: ait_core::json_support::JsonMap<String, JsonValue>,
}

impl RemoteHeadRecoveryContext {
    pub fn discover() -> Result<Self, String> {
        let current = env::current_dir().map_err(|error| error.to_string())?;
        match Self::discover_from_path(&current) {
            Ok(context) => Ok(context),
            Err(primary_error) => {
                for variable in RECOVERY_DISCOVERY_ENV_VARS {
                    let Some(candidate) = env::var_os(variable)
                        .map(PathBuf::from)
                        .filter(|value| !value.as_os_str().is_empty())
                    else {
                        continue;
                    };
                    if let Ok(context) = Self::discover_from_path(&candidate) {
                        return Ok(context);
                    }
                }
                Err(primary_error)
            }
        }
    }

    pub fn discover_from_path(start: &Path) -> Result<Self, String> {
        let mut current = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
        loop {
            let config_path = current.join(REPOSITORY_CONFIG_PATH);
            if config_path.is_file() {
                let mut config = read_required_config(&config_path)?;
                let worktree_config_path = current.join(WORKTREE_CONFIG_NAME);
                if worktree_config_path.is_file() {
                    for (key, value) in read_required_config(&worktree_config_path)? {
                        if !value.is_null() {
                            config.insert(key, value);
                        }
                    }
                }
                return Ok(Self {
                    root: current,
                    config_path,
                    config,
                });
            }
            let Some(parent) = current.parent() else {
                break;
            };
            if parent == current {
                break;
            }
            current = parent.to_path_buf();
        }
        Err(
            "No .ait/config.json found in current path or parents for remote-head recovery."
                .to_string(),
        )
    }

    fn remote_row(&self, requested: Option<&str>) -> Result<RemoteRecord, String> {
        let name = normalized_text(requested)
            .or_else(|| self.config_text("default_remote"))
            .ok_or_else(|| {
                "No remote configured. Pass --remote or configure default_remote first.".to_string()
            })?;
        ConfigRemoteStore::new(self.config_path.clone())?
            .remote_by_name(&name)?
            .ok_or_else(|| format!("Unknown remote: {name}"))
    }

    fn repo_name(&self) -> String {
        self.config_text("repo_name").unwrap_or_else(|| {
            self.authority_root()
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("repo")
                .to_string()
        })
    }

    fn repository_index(&self) -> Option<RepositoryIndex> {
        self.config
            .get("repository_index")
            .and_then(|value| RepositoryIndex::parse_config_value(value).ok())
    }

    fn default_line_name(&self) -> String {
        self.config_text("default_line")
            .unwrap_or_else(|| "main".to_string())
    }

    fn authority_root(&self) -> PathBuf {
        let Some(configured) = self.config_text("repo_root") else {
            return self.root.clone();
        };
        let configured = PathBuf::from(configured);
        if configured.is_absolute() {
            configured
        } else {
            self.root.join(configured)
        }
    }

    fn auth_headers(&self) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::new();
        if let Some(actor) = recovery_env("AIT_NATIVE_ACTOR")
            .or_else(|| recovery_env("AIT_ACTOR"))
            .or_else(|| self.config_text("user_email"))
            .or_else(|| self.config_text("user_name"))
        {
            headers.insert("X-AIT-Actor".to_string(), actor);
        }
        for (header, native_variable, variable) in [
            (
                "X-AIT-Actor-Type",
                "AIT_NATIVE_ACTOR_TYPE",
                "AIT_ACTOR_TYPE",
            ),
            ("X-AIT-Roles", "AIT_NATIVE_ROLES", "AIT_ROLES"),
            ("X-AIT-Repos", "AIT_NATIVE_REPOS", "AIT_REPOS"),
        ] {
            if let Some(value) = recovery_env(native_variable).or_else(|| recovery_env(variable)) {
                headers.insert(header.to_string(), value);
            }
        }
        headers
    }

    fn config_text(&self, key: &str) -> Option<String> {
        self.config
            .get(key)
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteHeadRecoveryRequest {
    pub remote_name: Option<String>,
    pub line_name: Option<String>,
    pub include_line_names: Vec<String>,
    pub jobs: usize,
    pub apply: bool,
}

impl Default for RemoteHeadRecoveryRequest {
    fn default() -> Self {
        Self {
            remote_name: None,
            line_name: None,
            include_line_names: Vec::new(),
            jobs: 8,
            apply: false,
        }
    }
}

#[derive(Clone, Debug)]
enum RecoveryPackKind {
    Object,
    Tree,
}

#[derive(Clone, Debug)]
struct RecoveryPackRequest {
    kind: RecoveryPackKind,
    pack_id: String,
}

#[derive(Clone, Debug)]
struct RecoveryLineHead {
    line_name: String,
    snapshot_id: String,
    manifest: ZstdImportManifestPayload,
}

#[derive(Clone, Debug)]
struct RecoveryAncestry {
    intermediate_manifests_topological: Vec<ZstdImportManifestPayload>,
    anchor_snapshot_ids: Vec<String>,
    primary_is_boundary: bool,
}

impl RecoveryAncestry {
    fn history_mode(&self) -> &'static str {
        if self.primary_is_boundary {
            "remote_head_boundary"
        } else {
            "remote_head_anchored_ancestry"
        }
    }

    fn complete_ancestry_snapshot_count(&self) -> usize {
        if self.primary_is_boundary {
            0
        } else {
            self.intermediate_manifests_topological
                .len()
                .saturating_add(1)
        }
    }

    fn compatibility_anchor_snapshot_id(&self) -> Option<String> {
        self.anchor_snapshot_ids.first().cloned()
    }
}

fn recovery_line_names(primary: &str, included: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut lines = Vec::with_capacity(included.len().saturating_add(1));
    for line_name in std::iter::once(primary.to_string()).chain(
        included
            .iter()
            .filter_map(|line_name| normalized_text(Some(line_name))),
    ) {
        if seen.insert(line_name.clone()) {
            lines.push(line_name);
        }
    }
    lines
}

fn recovery_lines_payload(line_heads: &[RecoveryLineHead]) -> Vec<JsonValue> {
    line_heads
        .iter()
        .map(|line_head| {
            json!({
                "line_name": line_head.line_name,
                "snapshot_id": line_head.snapshot_id,
                "source_parent_snapshot_ids": line_head.manifest.snapshots.first().map(|row| row.parent_snapshot_ids.clone()).unwrap_or_default(),
                "source_primary_parent_snapshot_id": line_head.manifest.snapshots.first().and_then(|row| row.primary_parent_snapshot_id.clone()),
                "source_parent_snapshot_id": line_head.manifest.snapshots.first().and_then(|row| row.parent_snapshot_id.clone()),
                "object_pack_count": line_head.manifest.object_packs.len(),
                "tree_pack_count": line_head.manifest.tree_packs.len(),
            })
        })
        .collect()
}

fn unique_recovery_pack_counts(
    line_heads: &[RecoveryLineHead],
    ancestry: &RecoveryAncestry,
) -> (usize, usize) {
    let object_packs = line_heads
        .iter()
        .map(|line_head| &line_head.manifest)
        .chain(ancestry.intermediate_manifests_topological.iter())
        .flat_map(|manifest| manifest.object_packs.iter())
        .map(|row| row.pack_id.as_str())
        .collect::<BTreeSet<_>>();
    let tree_packs = line_heads
        .iter()
        .map(|line_head| &line_head.manifest)
        .chain(ancestry.intermediate_manifests_topological.iter())
        .flat_map(|manifest| manifest.tree_packs.iter())
        .map(|row| row.pack_id.as_str())
        .collect::<BTreeSet<_>>();
    (object_packs.len(), tree_packs.len())
}

fn collect_primary_ancestry_with<F>(
    repo_name: &str,
    line_heads: &[RecoveryLineHead],
    mut fetch_manifest: F,
) -> Result<RecoveryAncestry, String>
where
    F: FnMut(&str, &str) -> Result<ZstdImportManifestPayload, String>,
{
    let primary = line_heads
        .first()
        .ok_or_else(|| "remote-head recovery resolved no lines".to_string())?;
    if line_heads.len() == 1 {
        return Ok(RecoveryAncestry {
            intermediate_manifests_topological: Vec::new(),
            anchor_snapshot_ids: Vec::new(),
            primary_is_boundary: true,
        });
    }

    let anchor_snapshot_ids = line_heads
        .iter()
        .skip(1)
        .map(|line_head| line_head.snapshot_id.clone())
        .collect::<BTreeSet<_>>();
    if anchor_snapshot_ids.contains(&primary.snapshot_id) {
        return Ok(RecoveryAncestry {
            intermediate_manifests_topological: Vec::new(),
            anchor_snapshot_ids: vec![primary.snapshot_id.clone()],
            primary_is_boundary: true,
        });
    }

    let limits = ait_core::snapshot_dag::SnapshotDagLimits::default();
    let primary_parents = recovery_manifest_parent_snapshot_ids(&primary.manifest)?;
    if primary_parents.is_empty() {
        return Err(disconnected_recovery_ancestry_error(
            &primary.snapshot_id,
            &anchor_snapshot_ids,
            &primary.snapshot_id,
        ));
    }
    let mut pending = VecDeque::new();
    let mut queued = BTreeSet::from([primary.snapshot_id.clone()]);
    let mut reached_anchors = BTreeSet::new();
    let mut parent_map = BTreeMap::from([(primary.snapshot_id.clone(), primary_parents.clone())]);
    let mut manifests = BTreeMap::new();
    for parent in primary_parents {
        if anchor_snapshot_ids.contains(&parent) {
            reached_anchors.insert(parent);
        } else if queued.insert(parent.clone()) {
            pending.push_back(parent);
        }
    }
    while let Some(snapshot_id) = pending.pop_front() {
        if queued.len() > limits.max_results {
            return Err(format!(
                "remote recovery Snapshot DAG exceeded max_results {} at {snapshot_id}",
                limits.max_results
            ));
        }
        let manifest = fetch_manifest(repo_name, &snapshot_id)?;
        verify_remote_manifest(&manifest, repo_name, &snapshot_id)?;
        let parents = recovery_manifest_parent_snapshot_ids(&manifest)?;
        if parents.is_empty() {
            return Err(disconnected_recovery_ancestry_error(
                &primary.snapshot_id,
                &anchor_snapshot_ids,
                &snapshot_id,
            ));
        }
        for parent in &parents {
            if anchor_snapshot_ids.contains(parent) {
                reached_anchors.insert(parent.clone());
            } else if queued.insert(parent.clone()) {
                pending.push_back(parent.clone());
            }
        }
        parent_map.insert(snapshot_id.clone(), parents);
        manifests.insert(snapshot_id, manifest);
    }
    let order = topological_snapshot_order(&parent_map, &anchor_snapshot_ids).map_err(|error| {
        if error.contains("Cycle detected") {
            format!("remote snapshot ancestry contains a cycle: {error}")
        } else {
            error
        }
    })?;
    if reached_anchors.is_empty() {
        return Err(disconnected_recovery_ancestry_error(
            &primary.snapshot_id,
            &anchor_snapshot_ids,
            &primary.snapshot_id,
        ));
    }
    let intermediate_manifests_topological = order
        .into_iter()
        .filter(|snapshot_id| snapshot_id != &primary.snapshot_id)
        .map(|snapshot_id| {
            manifests
                .remove(&snapshot_id)
                .ok_or_else(|| format!("remote recovery Snapshot DAG lost manifest {snapshot_id}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RecoveryAncestry {
        intermediate_manifests_topological,
        anchor_snapshot_ids: reached_anchors.into_iter().collect(),
        primary_is_boundary: false,
    })
}

fn recovery_manifest_parent_snapshot_ids(
    manifest: &ZstdImportManifestPayload,
) -> Result<Vec<String>, String> {
    let snapshot = manifest.snapshots.first().ok_or_else(|| {
        format!(
            "remote recovery manifest {} contains no snapshot row",
            manifest.snapshot_id
        )
    })?;
    normalize_snapshot_parent_set(
        Some(&snapshot.snapshot_id),
        Some(snapshot.parent_snapshot_ids.clone()),
        snapshot.primary_parent_snapshot_id.clone(),
        snapshot.parent_snapshot_id.clone(),
    )
    .map(|(parents, _, _)| parents)
}

fn disconnected_recovery_ancestry_error(
    primary_snapshot_id: &str,
    anchor_snapshot_ids: &BTreeSet<String>,
    unanchored_snapshot_id: &str,
) -> String {
    format!(
        "remote primary line head {primary_snapshot_id} has unanchored parent branch ending at {unanchored_snapshot_id}; it did not reach any included-line anchor ({})",
        anchor_snapshot_ids.iter().cloned().collect::<Vec<_>>().join(", ")
    )
}

pub fn recover_remote_head(
    context: &RemoteHeadRecoveryContext,
    request: &RemoteHeadRecoveryRequest,
) -> Result<JsonValue, String> {
    validate_jobs(request.jobs)?;
    let remote = context.remote_row(request.remote_name.as_deref())?;
    let repo_name = remote
        .repo_name
        .clone()
        .unwrap_or_else(|| context.repo_name());
    let line_name = normalized_text(request.line_name.as_deref())
        .unwrap_or_else(|| context.default_line_name());
    let line_names = recovery_line_names(&line_name, &request.include_line_names);
    let http_config = recovery_http_config(context, &remote.url, request.jobs);
    let mut client = PlanHttpClientManager::new(http_config.clone())
        .map_err(|error| format!("remote-head recovery transport setup failed: {error}"))?;
    let mut line_heads = Vec::with_capacity(line_names.len());
    for requested_line_name in line_names {
        let remote_line = client
            .get_line(&repo_name, &requested_line_name)
            .map_err(|error| {
                format!("failed to read remote line {requested_line_name:?}: {error}")
            })?;
        verify_remote_line(&remote_line, &repo_name, &requested_line_name)?;
        let snapshot_id = required_text(&remote_line, "head_snapshot_id")?;
        let manifest = client
            .get_remote_zstd_import_manifest(&repo_name, &snapshot_id)
            .map_err(|error| {
                format!("failed to read remote head manifest for {snapshot_id}: {error}")
            })?;
        verify_remote_manifest(&manifest, &repo_name, &snapshot_id)?;
        line_heads.push(RecoveryLineHead {
            line_name: requested_line_name,
            snapshot_id,
            manifest,
        });
    }
    let ancestry = collect_primary_ancestry_with(
        &repo_name,
        &line_heads,
        |requested_repo_name, snapshot_id| {
            client
                .get_remote_zstd_import_manifest(requested_repo_name, snapshot_id)
                .map_err(|error| {
                    format!("failed to read remote ancestry manifest for {snapshot_id}: {error}")
                })
        },
    )?;
    let primary = line_heads
        .first()
        .ok_or_else(|| "remote-head recovery resolved no lines".to_string())?;
    let snapshot = primary
        .manifest
        .snapshots
        .first()
        .ok_or_else(|| "remote head manifest contains no snapshot row".to_string())?;
    let source_parent_snapshot_id = snapshot.parent_snapshot_id.clone();
    let source_parent_snapshot_ids = snapshot.parent_snapshot_ids.clone();
    let recovered_lines = recovery_lines_payload(&line_heads);
    let (object_pack_count, tree_pack_count) = unique_recovery_pack_counts(&line_heads, &ancestry);
    let preview = json!({
        "action": "recover_remote_head",
        "apply": request.apply,
        "remote": remote.name,
        "repo_name": repo_name,
        "line_name": line_name,
        "snapshot_id": primary.snapshot_id,
        "source_parent_snapshot_ids": source_parent_snapshot_ids,
        "source_primary_parent_snapshot_id": snapshot.primary_parent_snapshot_id,
        "source_parent_snapshot_id": source_parent_snapshot_id,
        "history_mode": ancestry.history_mode(),
        "ancestry_anchor_snapshot_id": ancestry.compatibility_anchor_snapshot_id(),
        "ancestry_anchor_snapshot_ids": ancestry.anchor_snapshot_ids,
        "complete_ancestry_snapshot_count": ancestry.complete_ancestry_snapshot_count(),
        "recovered_line_count": line_heads.len(),
        "recovered_lines": recovered_lines,
        "object_pack_count": object_pack_count,
        "tree_pack_count": tree_pack_count,
        "lock_boundary": {
            "remote_download": "no original repository authority lock",
            "staging_import": "fresh staging authority locks only",
            "activation": "original repository generation activation lock",
        },
    });
    if !request.apply {
        return Ok(preview);
    }

    let original_root = context.authority_root();
    let expected_current_authority_fingerprint =
        snapshot_binary_db_authority_fingerprint(&original_root)?;
    let recovery_root = fresh_recovery_root(&original_root, &primary.snapshot_id)?;
    match apply_remote_head_recovery(
        &original_root,
        &http_config,
        &line_heads,
        &ancestry,
        &repo_name,
        &line_name,
        request.jobs,
        &recovery_root,
        &expected_current_authority_fingerprint,
    ) {
        Ok(mut applied) => {
            fs::remove_dir_all(&recovery_root).map_err(|error| {
                format!(
                    "remote head was activated but recovery staging {} could not be removed: {error}",
                    recovery_root.display()
                )
            })?;
            let object = applied
                .as_object_mut()
                .ok_or_else(|| "remote-head recovery report must be an object".to_string())?;
            object.insert("staging_removed".to_string(), JsonValue::Bool(true));
            Ok(applied)
        }
        Err(error) => Err(format!(
            "{error}\nRemote-head recovery left its isolated staging data at {} for inspection; the original authority was not partially activated.",
            recovery_root.display()
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_remote_head_recovery(
    original_repo_root: &Path,
    http_config: &PlanHttpClientConfig,
    line_heads: &[RecoveryLineHead],
    ancestry: &RecoveryAncestry,
    repo_name: &str,
    primary_line_name: &str,
    jobs: usize,
    recovery_root: &Path,
    expected_current_authority_fingerprint: &str,
) -> Result<JsonValue, String> {
    let primary = line_heads
        .first()
        .ok_or_else(|| "remote-head recovery resolved no lines".to_string())?;
    let staging_repo_root = recovery_root.join("repository");
    fs::create_dir(&staging_repo_root).map_err(|error| {
        format!(
            "failed to create remote-head staging repository {}: {error}",
            staging_repo_root.display()
        )
    })?;
    init_repo_for_remote_head_recovery(&InitRequest {
        root: staging_repo_root.clone(),
        name: Some(repo_name.to_string()),
        default_line: primary_line_name.to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })?;
    let staging_repo = RepoRuntime::discover_from_path(&staging_repo_root)?;
    let import_store =
        staging_repo.remote_sync_local_store::<REMOTE_SYNC_BINARY_DB_WRITE_LAYOUT>()?;
    let import_context = RemoteSyncLocalStoreContext::new(&staging_repo_root);
    let mut downloaded_object_packs = 0_i64;
    let mut downloaded_tree_packs = 0_i64;
    let mut reused_object_packs = 0_i64;
    let mut reused_tree_packs = 0_i64;
    {
        let mut import_manifest = |manifest: &ZstdImportManifestPayload,
                                   history_mode: ZstdImportHistoryMode|
         -> Result<(), String> {
            let plan = import_store.zstd_import_download_plan(&import_context, manifest)?;
            let (object_pack_bytes, tree_pack_bytes) =
                download_recovery_packs(http_config, repo_name, &plan, jobs)?;
            let imported = import_store.import_zstd_manifest(
                &import_context,
                manifest,
                history_mode,
                &plan,
                &object_pack_bytes,
                &tree_pack_bytes,
            )?;
            if imported.snapshot_id != manifest.snapshot_id {
                return Err(format!(
                    "remote-head staging imported unexpected snapshot {:?}, expected {:?}",
                    imported.snapshot_id, manifest.snapshot_id
                ));
            }
            verify_staged_snapshot_history(
                &staging_repo,
                &manifest.snapshot_id,
                manifest,
                history_mode,
            )?;
            downloaded_object_packs =
                downloaded_object_packs.saturating_add(imported.downloaded_object_packs);
            downloaded_tree_packs =
                downloaded_tree_packs.saturating_add(imported.downloaded_tree_packs);
            reused_object_packs = reused_object_packs.saturating_add(imported.reused_object_packs);
            reused_tree_packs = reused_tree_packs.saturating_add(imported.reused_tree_packs);
            Ok(())
        };

        let mut imported_snapshot_ids = BTreeSet::new();
        if ancestry.primary_is_boundary {
            import_manifest(&primary.manifest, ZstdImportHistoryMode::RemoteHeadBoundary)?;
            imported_snapshot_ids.insert(primary.snapshot_id.clone());
        }
        for line_head in line_heads.iter().skip(1) {
            if imported_snapshot_ids.insert(line_head.snapshot_id.clone()) {
                import_manifest(
                    &line_head.manifest,
                    ZstdImportHistoryMode::RemoteHeadBoundary,
                )?;
            }
        }
        if !ancestry.primary_is_boundary {
            for manifest in &ancestry.intermediate_manifests_topological {
                if !imported_snapshot_ids.insert(manifest.snapshot_id.clone()) {
                    return Err(format!(
                        "remote ancestry manifest {} overlaps an imported history boundary",
                        manifest.snapshot_id
                    ));
                }
                import_manifest(manifest, ZstdImportHistoryMode::CompleteAncestry)?;
            }
            if !imported_snapshot_ids.insert(primary.snapshot_id.clone()) {
                return Err(format!(
                    "remote primary snapshot {} overlaps an imported history boundary",
                    primary.snapshot_id
                ));
            }
            import_manifest(&primary.manifest, ZstdImportHistoryMode::CompleteAncestry)?;
        }
    }

    for line_head in line_heads {
        let updated_at = line_head
            .manifest
            .snapshots
            .first()
            .and_then(|row| row.created_at.as_deref())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("1970-01-01T00:00:00Z");
        let line_store = staging_repo.line_store()?;
        if line_store.line_by_name(&line_head.line_name)?.is_some() {
            line_store.set_line_head(
                &line_head.line_name,
                Some(&line_head.snapshot_id),
                updated_at,
            )?;
        } else {
            line_store.create_line(
                &line_head.line_name,
                Some(&line_head.snapshot_id),
                updated_at,
            )?;
        }
    }

    let generation_root = recovery_root.join("generation");
    let capture = capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
        repo_root: staging_repo_root,
        output_root: generation_root.clone(),
        jobs,
    })?;
    let activation = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
        repo_root: original_repo_root.to_path_buf(),
        generation_root,
        expected_current_authority_fingerprint: Some(
            expected_current_authority_fingerprint.to_string(),
        ),
    })?;
    Ok(json!({
        "action": "recover_remote_head",
        "apply": true,
        "repo_name": repo_name,
        "line_name": primary.line_name,
        "snapshot_id": primary.snapshot_id,
        "source_parent_snapshot_ids": primary.manifest.snapshots.first().map(|row| row.parent_snapshot_ids.clone()).unwrap_or_default(),
        "source_primary_parent_snapshot_id": primary.manifest.snapshots.first().and_then(|row| row.primary_parent_snapshot_id.clone()),
        "source_parent_snapshot_id": primary.manifest.snapshots.first().and_then(|row| row.parent_snapshot_id.clone()),
        "history_mode": ancestry.history_mode(),
        "ancestry_anchor_snapshot_id": ancestry.compatibility_anchor_snapshot_id(),
        "ancestry_anchor_snapshot_ids": ancestry.anchor_snapshot_ids,
        "complete_ancestry_snapshot_count": ancestry.complete_ancestry_snapshot_count(),
        "recovered_line_count": line_heads.len(),
        "recovered_lines": recovery_lines_payload(line_heads),
        "downloaded_object_packs": downloaded_object_packs,
        "downloaded_tree_packs": downloaded_tree_packs,
        "reused_object_packs": reused_object_packs,
        "reused_tree_packs": reused_tree_packs,
        "captured_file_count": capture.file_count,
        "content_fingerprint": activation.content_fingerprint,
        "authority_root": activation.authority_root,
        "pack_root": activation.pack_root,
        "activation_strategy": activation.activation_strategy,
        "single_syscall_atomic": activation.single_syscall_atomic,
        "activation_lock_protected": activation.activation_lock_protected,
    }))
}

fn verify_staged_snapshot_history(
    repo: &RepoRuntime,
    snapshot_id: &str,
    manifest: &ZstdImportManifestPayload,
    history_mode: ZstdImportHistoryMode,
) -> Result<(), String> {
    let content = repo
        .binary_db_stores::<REMOTE_SYNC_BINARY_DB_WRITE_LAYOUT>()
        .content();
    let read = content.snapshots().begin_read_txn();
    let snapshot = content
        .snapshots()
        .get_snapshot_view(&read, snapshot_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("staging snapshot {snapshot_id} is missing after import"))?;
    let source_parents = recovery_manifest_parent_snapshot_ids(manifest)?;
    match history_mode {
        ZstdImportHistoryMode::RemoteHeadBoundary => {
            if !snapshot.parent_snapshot_ids.is_empty() {
                return Err(format!(
                    "staging snapshot {snapshot_id} unexpectedly retained a local parent"
                ));
            }
            if !source_parents.is_empty() && !snapshot.record.is_remote_head_history_boundary() {
                return Err(format!(
                    "staging snapshot {snapshot_id} did not record the remote-head history boundary"
                ));
            }
        }
        ZstdImportHistoryMode::CompleteAncestry => {
            if snapshot.parent_snapshot_ids != source_parents {
                return Err(format!(
                    "staging snapshot {snapshot_id} retained parents {:?}, expected {:?}",
                    snapshot.parent_snapshot_ids, source_parents
                ));
            }
            if snapshot.record.is_remote_head_history_boundary() {
                return Err(format!(
                    "staging snapshot {snapshot_id} unexpectedly recorded a history boundary"
                ));
            }
        }
    }
    Ok(())
}

type RecoveryPackBytes = BTreeMap<String, Vec<u8>>;

fn download_recovery_packs(
    http_config: &PlanHttpClientConfig,
    repo_name: &str,
    plan: &ait_core::remote_sync_local_store::ZstdImportDownloadPlan,
    jobs: usize,
) -> Result<(RecoveryPackBytes, RecoveryPackBytes), String> {
    let mut requests = plan
        .missing_object_pack_ids
        .iter()
        .map(|pack_id| RecoveryPackRequest {
            kind: RecoveryPackKind::Object,
            pack_id: pack_id.clone(),
        })
        .chain(
            plan.missing_tree_pack_ids
                .iter()
                .map(|pack_id| RecoveryPackRequest {
                    kind: RecoveryPackKind::Tree,
                    pack_id: pack_id.clone(),
                }),
        )
        .collect::<Vec<_>>();
    requests.sort_by(|left, right| left.pack_id.cmp(&right.pack_id));
    let results = bounded_pack_downloads(http_config, repo_name, &requests, jobs)?;
    let mut object_packs = BTreeMap::new();
    let mut tree_packs = BTreeMap::new();
    for (request, bytes) in requests.into_iter().zip(results) {
        let target = match request.kind {
            RecoveryPackKind::Object => &mut object_packs,
            RecoveryPackKind::Tree => &mut tree_packs,
        };
        if target.insert(request.pack_id.clone(), bytes).is_some() {
            return Err(format!(
                "remote-head manifest requested duplicate pack {}",
                request.pack_id
            ));
        }
    }
    Ok((object_packs, tree_packs))
}

fn bounded_pack_downloads(
    http_config: &PlanHttpClientConfig,
    repo_name: &str,
    requests: &[RecoveryPackRequest],
    jobs: usize,
) -> Result<Vec<Vec<u8>>, String> {
    if requests.is_empty() {
        return Ok(Vec::new());
    }
    let worker_count = jobs.min(requests.len()).max(1);
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel::<(usize, Result<Vec<u8>, String>)>();
    std::thread::scope(|scope| -> Result<(), String> {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let sender = sender.clone();
            let next = &next;
            let config = http_config.clone();
            handles.push(scope.spawn(move || {
                let mut client = match PlanHttpClientManager::new(config) {
                    Ok(client) => client,
                    Err(error) => {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        if index < requests.len() {
                            let _ = sender.send((index, Err(error.to_string())));
                        }
                        return;
                    }
                };
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(request) = requests.get(index) else {
                        break;
                    };
                    let result = match request.kind {
                        RecoveryPackKind::Object => client
                            .get_remote_zstd_object_pack(repo_name, &request.pack_id)
                            .map_err(|error| error.to_string()),
                        RecoveryPackKind::Tree => client
                            .get_remote_zstd_tree_pack(repo_name, &request.pack_id)
                            .map_err(|error| error.to_string()),
                    };
                    if sender.send((index, result)).is_err() {
                        break;
                    }
                }
            }));
        }
        drop(sender);
        for handle in handles {
            if handle.join().is_err() {
                return Err("remote-head pack download worker panicked".to_string());
            }
        }
        Ok(())
    })?;
    let mut ordered = BTreeMap::new();
    for (index, result) in receiver {
        if ordered.insert(index, result).is_some() {
            return Err(format!(
                "remote-head pack downloader returned duplicate index {index}"
            ));
        }
    }
    if ordered.len() != requests.len() {
        return Err(format!(
            "remote-head pack downloader returned {} results for {} requests",
            ordered.len(),
            requests.len()
        ));
    }
    ordered.into_values().collect()
}

fn recovery_http_config(
    context: &RemoteHeadRecoveryContext,
    base_url: &str,
    jobs: usize,
) -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: base_url.to_string(),
        repository_index: context.repository_index(),
        headers: context.auth_headers(),
        default_timeout_ms: 300_000,
        retry_attempts: 2,
        retry_backoff_ms: 250,
        pool_max_idle_per_host: jobs.max(1),
    }
}

fn verify_remote_line(
    line: &JsonValue,
    expected_repo_name: &str,
    expected_line_name: &str,
) -> Result<(), String> {
    let repo_name = required_text(line, "repo_name")?;
    let line_name = required_text(line, "line_name")?;
    if repo_name != expected_repo_name || line_name != expected_line_name {
        return Err(format!(
            "remote line identity mismatch: expected {expected_repo_name:?}/{expected_line_name:?}, got {repo_name:?}/{line_name:?}"
        ));
    }
    Ok(())
}

fn verify_remote_manifest(
    manifest: &ait_core::repository_pack_json::ZstdImportManifestPayload,
    expected_repo_name: &str,
    expected_snapshot_id: &str,
) -> Result<(), String> {
    if manifest.repo_name != expected_repo_name || manifest.snapshot_id != expected_snapshot_id {
        return Err(format!(
            "remote head manifest identity mismatch: expected {expected_repo_name:?}/{expected_snapshot_id:?}, got {:?}/{:?}",
            manifest.repo_name, manifest.snapshot_id
        ));
    }
    if manifest.snapshots.len() != 1 || manifest.snapshots[0].snapshot_id != expected_snapshot_id {
        return Err(
            "remote head manifest must contain exactly its requested snapshot row".to_string(),
        );
    }
    Ok(())
}

fn fresh_recovery_root(repo_root: &Path, snapshot_id: &str) -> Result<PathBuf, String> {
    let parent = repo_root.join(".ait").join(RECOVERY_ROOT_NAME);
    fs::create_dir_all(&parent).map_err(|error| {
        format!(
            "failed to create remote-head recovery root {}: {error}",
            parent.display()
        )
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let safe_snapshot = snapshot_id
        .chars()
        .filter(|value| value.is_ascii_alphanumeric() || *value == '-')
        .collect::<String>();
    let path = parent.join(format!(
        "{}-{}-{nonce}",
        if safe_snapshot.is_empty() {
            "head"
        } else {
            &safe_snapshot
        },
        std::process::id()
    ));
    fs::create_dir(&path).map_err(|error| {
        format!(
            "failed to reserve remote-head recovery staging {}: {error}",
            path.display()
        )
    })?;
    Ok(path)
}

fn required_text(value: &JsonValue, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("remote head response is missing {field}"))
}

fn normalized_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn recovery_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .and_then(|value| normalized_text(Some(&value)))
}

fn read_required_config(
    path: &Path,
) -> Result<ait_core::json_support::JsonMap<String, JsonValue>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read recovery config {}: {error}", path.display()))?;
    match parse_value(
        &text,
        &format!("invalid recovery config {}", path.display()),
    )? {
        JsonValue::Object(object) => Ok(object),
        _ => Err(format!(
            "remote-head recovery config {} must be a JSON object",
            path.display()
        )),
    }
}

fn validate_jobs(jobs: usize) -> Result<(), String> {
    if !(1..=MAX_RECOVERY_JOBS).contains(&jobs) {
        return Err(format!(
            "remote-head recovery jobs must be between 1 and {MAX_RECOVERY_JOBS}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn recovery_manifest(
        snapshot_id: &str,
        parent_snapshot_id: Option<&str>,
    ) -> ZstdImportManifestPayload {
        recovery_manifest_with_parents(
            snapshot_id,
            parent_snapshot_id.into_iter().map(str::to_string).collect(),
        )
    }

    fn recovery_manifest_with_parents(
        snapshot_id: &str,
        parent_snapshot_ids: Vec<String>,
    ) -> ZstdImportManifestPayload {
        let primary_parent_snapshot_id = parent_snapshot_ids.first().cloned();
        ZstdImportManifestPayload {
            contract: "ait.repository.zstd_import_manifest.v1".to_string(),
            repo_name: "fixture".to_string(),
            snapshot_id: snapshot_id.to_string(),
            snapshots: vec![ait_core::repository_pack_json::ZstdBulkSnapshotRow {
                snapshot_id: snapshot_id.to_string(),
                parent_snapshot_ids,
                primary_parent_snapshot_id: primary_parent_snapshot_id.clone(),
                parent_snapshot_id: primary_parent_snapshot_id,
                root_tree_pack_id: None,
                root_entry_ordinal: None,
                manifest_hash: None,
                message: None,
                line_name: None,
                snapshot_kind: None,
                file_count: None,
                total_bytes: None,
                created_at: None,
            }],
            object_packs: Vec::new(),
            tree_packs: Vec::new(),
            blob_locators: Vec::new(),
            tree_locators: Vec::new(),
            line_update: None,
        }
    }

    fn recovery_line_head(
        line_name: &str,
        snapshot_id: &str,
        parent_snapshot_id: Option<&str>,
    ) -> RecoveryLineHead {
        RecoveryLineHead {
            line_name: line_name.to_string(),
            snapshot_id: snapshot_id.to_string(),
            manifest: recovery_manifest(snapshot_id, parent_snapshot_id),
        }
    }

    fn write_recovery_config(root: &Path) {
        let ait_dir = root.join(".ait");
        fs::create_dir_all(&ait_dir).unwrap();
        fs::write(
            ait_dir.join("config.json"),
            r#"{
                "default_line": "main",
                "default_remote": "origin",
                "remotes": {
                    "origin": {
                        "repo_name": "fixture",
                        "url": "http://127.0.0.1:8088"
                    }
                },
                "repository_index": 7,
                "repo_name": "fixture"
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn recovery_jobs_are_bounded() {
        assert!(validate_jobs(1).is_ok());
        assert!(validate_jobs(8).is_ok());
        assert!(validate_jobs(0).is_err());
        assert!(validate_jobs(MAX_RECOVERY_JOBS + 1).is_err());
    }

    #[test]
    fn recovery_lines_keep_primary_first_and_deduplicate_includes() {
        assert_eq!(
            recovery_line_names(
                "feature/task",
                &[
                    "main".to_string(),
                    " feature/task ".to_string(),
                    "".to_string(),
                    "release".to_string(),
                    "main".to_string(),
                ],
            ),
            vec![
                "feature/task".to_string(),
                "main".to_string(),
                "release".to_string(),
            ]
        );
    }

    #[test]
    fn included_line_anchors_primary_remote_ancestry() {
        let line_heads = vec![
            recovery_line_head("feature/task", "SNP-HEAD", Some("SNP-MID")),
            recovery_line_head("main", "SNP-FORK", Some("SNP-OLDER")),
        ];
        let remote_manifests = BTreeMap::from([(
            "SNP-MID".to_string(),
            recovery_manifest("SNP-MID", Some("SNP-FORK")),
        )]);
        let mut fetched = Vec::new();

        let ancestry = collect_primary_ancestry_with("fixture", &line_heads, |_, snapshot_id| {
            fetched.push(snapshot_id.to_string());
            remote_manifests
                .get(snapshot_id)
                .cloned()
                .ok_or_else(|| format!("missing fixture manifest {snapshot_id}"))
        })
        .unwrap();

        assert!(!ancestry.primary_is_boundary);
        assert_eq!(ancestry.anchor_snapshot_ids, vec!["SNP-FORK"]);
        assert_eq!(fetched, vec!["SNP-MID".to_string()]);
        assert_eq!(
            ancestry
                .intermediate_manifests_topological
                .iter()
                .map(|manifest| manifest.snapshot_id.as_str())
                .collect::<Vec<_>>(),
            vec!["SNP-MID"]
        );
        assert_eq!(ancestry.complete_ancestry_snapshot_count(), 2);
    }

    #[test]
    fn included_lines_anchor_every_parent_of_a_remote_merge() {
        let line_heads = vec![
            RecoveryLineHead {
                line_name: "feature/task".to_string(),
                snapshot_id: "SNP-MERGE".to_string(),
                manifest: recovery_manifest_with_parents(
                    "SNP-MERGE",
                    vec!["SNP-LEFT-MID".to_string(), "SNP-RIGHT-MID".to_string()],
                ),
            },
            recovery_line_head("left", "SNP-LEFT", None),
            recovery_line_head("right", "SNP-RIGHT", None),
        ];
        let remote_manifests = BTreeMap::from([
            (
                "SNP-LEFT-MID".to_string(),
                recovery_manifest("SNP-LEFT-MID", Some("SNP-LEFT")),
            ),
            (
                "SNP-RIGHT-MID".to_string(),
                recovery_manifest("SNP-RIGHT-MID", Some("SNP-RIGHT")),
            ),
        ]);
        let mut fetched = Vec::new();

        let ancestry = collect_primary_ancestry_with("fixture", &line_heads, |_, snapshot_id| {
            fetched.push(snapshot_id.to_string());
            remote_manifests
                .get(snapshot_id)
                .cloned()
                .ok_or_else(|| format!("missing fixture manifest {snapshot_id}"))
        })
        .expect("complete merge ancestry");

        fetched.sort();
        assert_eq!(fetched, vec!["SNP-LEFT-MID", "SNP-RIGHT-MID"]);
        assert_eq!(ancestry.anchor_snapshot_ids, vec!["SNP-LEFT", "SNP-RIGHT"]);
        assert_eq!(
            ancestry
                .intermediate_manifests_topological
                .iter()
                .map(|manifest| manifest.snapshot_id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["SNP-LEFT-MID", "SNP-RIGHT-MID"])
        );
        assert_eq!(ancestry.complete_ancestry_snapshot_count(), 3);
    }

    #[test]
    fn included_line_ancestry_fails_closed_when_disconnected_or_cyclic() {
        let line_heads = vec![
            recovery_line_head("feature/task", "SNP-HEAD", Some("SNP-MID")),
            recovery_line_head("main", "SNP-FORK", None),
        ];
        let disconnected =
            collect_primary_ancestry_with("fixture", &line_heads, |_, snapshot_id| {
                Ok(recovery_manifest(snapshot_id, None))
            })
            .unwrap_err();
        assert!(disconnected.contains("did not reach any included-line anchor"));

        let cyclic = collect_primary_ancestry_with("fixture", &line_heads, |_, snapshot_id| {
            Ok(recovery_manifest(snapshot_id, Some("SNP-HEAD")))
        })
        .unwrap_err();
        assert!(cyclic.contains("contains a cycle"));
    }

    #[test]
    fn recovery_line_identity_is_exact() {
        let line = json!({
            "repo_name": "ait-core",
            "line_name": "main",
            "head_snapshot_id": "SNP-1",
        });
        assert!(verify_remote_line(&line, "ait-core", "main").is_ok());
        assert!(verify_remote_line(&line, "other", "main").is_err());
        assert!(verify_remote_line(&line, "ait-core", "feature").is_err());
    }

    #[test]
    fn recovery_context_does_not_admit_or_open_local_authority() {
        let temp = TempDir::new().unwrap();
        write_recovery_config(temp.path());
        let polluted_authority = temp.path().join(".ait/binary-db/local");
        fs::create_dir_all(&polluted_authority).unwrap();
        fs::write(polluted_authority.join("undeclared.bin"), b"retired").unwrap();

        let context = RemoteHeadRecoveryContext::discover_from_path(temp.path()).unwrap();

        assert_eq!(context.repo_name(), "fixture");
        assert_eq!(context.repository_index(), Some(RepositoryIndex::new(7)));
        assert_eq!(context.default_line_name(), "main");
        assert_eq!(context.remote_row(None).unwrap().name, "origin");
        assert_eq!(
            context.authority_root(),
            temp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn recovery_context_targets_canonical_authority_from_worktree_overlay() {
        let temp = TempDir::new().unwrap();
        let worktree = temp.path().join("worktree");
        let authority = temp.path().join("canonical");
        fs::create_dir_all(&worktree).unwrap();
        write_recovery_config(&worktree);
        fs::write(
            worktree.join(WORKTREE_CONFIG_NAME),
            format!(
                "{{\"repo_root\":{}}}",
                ait_core::json_support::JsonCodec::encode_value(
                    &JsonValue::String(authority.to_string_lossy().to_string()),
                    ait_core::json_support::JsonEncodeOptions::compact(),
                )
                .unwrap()
            ),
        )
        .unwrap();

        let context = RemoteHeadRecoveryContext::discover_from_path(&worktree).unwrap();

        assert_eq!(context.authority_root(), authority);
    }
}
