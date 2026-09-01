use ait_cli::init_surface::{init_repo as initialize_repo, InitRequest};
use ait_cli::primitives::{
    task_land_apply, task_land_payload, workflow_land_apply, workflow_ready_payload,
};
use ait_cli::runtime::RepoRuntime;
use ait_core::line_store::LineStore;
use ait_core::local_snapshot::LocalSnapshotTreeReadStore;
use ait_core::pack_substrate::{PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1};
use ait_core::remote_sync_local_store::{
    RemoteSyncLocalStoreContext, RemoteSyncZstdLocalPlanSource, ZstdBulkLocalPlan,
};
use ait_core::repository_pack_json::{
    JsonPayloadContract, ZstdBulkCommitRequest, ZstdImportManifestJson,
    ZstdImportManifestPayload, ZSTD_IMPORT_MANIFEST_CONTRACT_NAME,
};
use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions, JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tiny_http::{Response, Server};

const FIXTURE_BASE_SNAPSHOT_ID: &str = "SNP-C95DCC8C7848";
const FIXTURE_REVISION_SNAPSHOT_ID: &str = "SNP-A11CE5EED001";
const FIXTURE_FINISHED_SNAPSHOT_ID: &str = "SNP-A11CE5EED002";
const FIXTURE_REPOSITORY_INDEX: u32 = 7;

struct WritableTreeOnDrop(PathBuf);

impl WritableTreeOnDrop {
    fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

impl Drop for WritableTreeOnDrop {
    fn drop(&mut self) {
        fn make_writeable(path: &Path) {
            let Ok(metadata) = fs::symlink_metadata(path) else {
                return;
            };
            if metadata.file_type().is_symlink() {
                return;
            }
            if metadata.is_dir() {
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.flatten() {
                        make_writeable(&entry.path());
                    }
                }
            }
            let mut permissions = metadata.permissions();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let minimum_mode = if metadata.is_dir() { 0o700 } else { 0o200 };
                permissions.set_mode(permissions.mode() | minimum_mode);
            }
            #[cfg(not(unix))]
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }

        make_writeable(&self.0);
    }
}

fn parse_json(text: &str) -> JsonValue {
    JsonCodec::parse_value_with_error_prefix(text, "Invalid JSON").unwrap()
}

fn parse_json_bytes(bytes: &[u8]) -> JsonValue {
    JsonCodec::parse_slice_with_error_prefix(bytes, "Invalid JSON").unwrap()
}

fn parse_json_file(path: impl AsRef<Path>) -> JsonValue {
    parse_json(&fs::read_to_string(path).unwrap())
}

fn encode_json(value: &JsonValue) -> String {
    JsonCodec::encode_value(value, JsonEncodeOptions::compact()).unwrap()
}

fn encode_json_pretty(value: &JsonValue) -> String {
    JsonCodec::encode_value(value, JsonEncodeOptions::pretty()).unwrap()
}

fn encode_json_vec(value: &JsonValue) -> Vec<u8> {
    JsonCodec::encode_value_to_vec(value, JsonEncodeOptions::compact()).unwrap()
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    url: String,
    body: String,
}

#[derive(Default)]
struct RecoveryRemoteState {
    remote_head_snapshot_id: Option<String>,
    published_base_snapshot_id: Option<String>,
    published_revision_snapshot_id: Option<String>,
}

#[derive(Default)]
struct BoundedSnapshotRemoteState {
    remote_head_snapshot_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloseoutMutationBoundary {
    MutateBeforeResponse,
    MutationInFlight,
    RetryableBusyAfterMutation,
    TimeoutBeforeMutation,
}

struct CloseoutRecoveryRemoteState {
    land_submitted: bool,
    task_completed: bool,
    enforce_reviewer_workflow: bool,
    code_review_recorded: bool,
    task_review_recorded: bool,
    policy_evaluated: bool,
    patchset_revision_snapshot_id: Option<String>,
    land_boundary: CloseoutMutationBoundary,
    task_boundary: CloseoutMutationBoundary,
    land_response_delay: Duration,
    task_response_delay: Duration,
    land_submit_attempts: usize,
    land_submit_mutations: usize,
    task_close_attempts: usize,
    task_close_mutations: usize,
    land_mutation_in_flight: bool,
    atomic_task_land_idempotency_key: Option<String>,
    fixture_seed: u64,
}

impl Default for CloseoutRecoveryRemoteState {
    fn default() -> Self {
        Self {
            land_submitted: false,
            task_completed: false,
            enforce_reviewer_workflow: false,
            code_review_recorded: true,
            task_review_recorded: true,
            policy_evaluated: true,
            patchset_revision_snapshot_id: None,
            land_boundary: CloseoutMutationBoundary::MutateBeforeResponse,
            task_boundary: CloseoutMutationBoundary::MutateBeforeResponse,
            land_response_delay: Duration::from_millis(150),
            task_response_delay: Duration::from_millis(150),
            land_submit_attempts: 0,
            land_submit_mutations: 0,
            task_close_attempts: 0,
            task_close_mutations: 0,
            land_mutation_in_flight: false,
            atomic_task_land_idempotency_key: None,
            fixture_seed: 0,
        }
    }
}

impl CloseoutRecoveryRemoteState {
    fn reset_iteration(
        &mut self,
        fixture_seed: u64,
        land_boundary: CloseoutMutationBoundary,
        land_response_delay: Duration,
        task_boundary: CloseoutMutationBoundary,
        task_response_delay: Duration,
    ) {
        self.land_submitted = false;
        self.task_completed = false;
        self.enforce_reviewer_workflow = false;
        self.code_review_recorded = true;
        self.task_review_recorded = true;
        self.policy_evaluated = true;
        self.patchset_revision_snapshot_id = None;
        self.land_boundary = land_boundary;
        self.task_boundary = task_boundary;
        self.land_response_delay = land_response_delay;
        self.task_response_delay = task_response_delay;
        self.land_submit_attempts = 0;
        self.land_submit_mutations = 0;
        self.task_close_attempts = 0;
        self.task_close_mutations = 0;
        self.land_mutation_in_flight = false;
        self.atomic_task_land_idempotency_key = None;
        self.fixture_seed = fixture_seed;
    }
}

struct CloseoutRecoveryServerHandle {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl CloseoutRecoveryServerHandle {
    fn join(mut self) -> thread::Result<()> {
        self.stop.store(true, Ordering::Release);
        self.handle.take().expect("closeout server handle").join()
    }
}

impl Drop for CloseoutRecoveryServerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[derive(Default)]
struct FakeRemoteState {
    remote_head_snapshot_id: Option<String>,
    selected_patchset_id: Option<String>,
    selected_patchset_base_snapshot_id: Option<String>,
    selected_patchset_revision_snapshot_id: Option<String>,
    force_no_selected_patchset: bool,
    land_submit_base_stale_converged: bool,
    base_stale_converged_submitted: bool,
    omit_landing_summary_after_base_stale_converged: bool,
    land_submitted: bool,
    task_completed: bool,
    atomic_plan: Option<JsonValue>,
    atomic_task_start: Option<JsonValue>,
    fail_atomic_task_start: bool,
    zstd_import_fixture: Option<ZstdRemoteImportFixture>,
}

#[derive(Clone)]
struct ZstdRemoteImportFixture {
    manifests: BTreeMap<String, ZstdImportManifestPayload>,
    object_packs: BTreeMap<String, Vec<u8>>,
    tree_packs: BTreeMap<String, Vec<u8>>,
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut handle = fs::File::create(path).unwrap();
    handle.write_all(content.as_bytes()).unwrap();
}

fn encode_ref_name(name: &str) -> String {
    let mut out = String::new();
    for byte in name.as_bytes() {
        let ch = *byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

fn local_line_head(root: &Path, line_name: &str) -> Option<String> {
    RepoRuntime::discover_from_path(root)
        .unwrap()
        .line_store()
        .unwrap()
        .line_by_name(line_name)
        .unwrap()
        .and_then(|line| line.head_snapshot_id)
}

fn seed_binary_line(root: &Path, line_name: &str, snapshot_id: &str) {
    let repo = RepoRuntime::discover_from_path(root).unwrap();
    let lines = repo.line_store().unwrap();
    if lines.line_by_name(line_name).unwrap().is_none() {
        lines
            .create_line(
                line_name,
                Some(snapshot_id),
                "2026-06-08T00:00:00Z",
            )
            .unwrap();
    } else {
        lines
            .set_line_head(
                line_name,
                Some(snapshot_id),
                "2026-06-08T00:00:00Z",
            )
            .unwrap();
    }
}

fn init_repo_with_fixture_workflow(base_url: &str) -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"ok\" }\n",
    );
    initialize_repo(&InitRequest {
        root: root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "repository_index": 7,
  "id_namespace_prefix": "",
  "default_line": "main",
  "default_remote": "origin",
  "workflow_mode": "team_remote",
  "remotes": {
    "origin": {
      "remote_id": 1,
      "url": "BASE_URL",
      "repo_name": "fixture-ait",
      "created_at": "2026-06-08T00:00:00Z"
    }
  },
  "workflow_default_scope": "remote",
  "task_default_scope": "remote",
  "sprint": "off",
  "plan_task_binding": {"mode": "off"},
  "user_name": "Fixture User",
  "user_email": "fixture@example.com"
}"#
        .replace("BASE_URL", base_url)
        .as_str(),
    );
    let base_snapshot = json_output(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "fixture base snapshot",
            "--json",
        ],
    );
    assert_eq!(
        base_snapshot["snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID),
        "fixture base Snapshot identity changed"
    );

    temp
}

fn init_repo(base_url: &str) -> TempDir {
    init_repo_with_fixture_workflow(base_url)
}

fn init_repo_without_workflow_rows(base_url: &str) -> TempDir {
    init_repo_with_fixture_workflow(base_url)
}

fn assert_fixture_repo_is_zstd_only_compatible(root: &Path) {
    let repo = RepoRuntime::discover_from_path(root).unwrap();
    let store = repo.remote_sync_local_store::<1>().unwrap();
    let plan = store
        .zstd_bulk_local_plan(
            &RemoteSyncLocalStoreContext::new(root),
            &[FIXTURE_BASE_SNAPSHOT_ID.to_string()],
            &BTreeSet::new(),
        )
        .unwrap();
    let object_formats = plan
        .object_packs
        .values()
        .filter_map(|pack| pack.metadata["pack_format"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let tree_formats = plan
        .tree_packs
        .values()
        .filter_map(|pack| pack.metadata["pack_format"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    assert!(
        object_formats
            .iter()
            .all(|format| format == PACK_FORMAT_ZSTD_CHUNKED_V1),
        "fixture object packs must be zstd-only: {object_formats:?}"
    );
    assert!(
        tree_formats
            .iter()
            .all(|format| format == TREE_PACK_FORMAT_ZSTD_CHUNKED_V1),
        "fixture tree packs must be zstd-only: {tree_formats:?}"
    );
}

fn remove_manifest_forbidden_fields(row: &mut JsonValue, fields: &[&str]) {
    if let Some(object) = row.as_object_mut() {
        for field in fields {
            object.remove(*field);
        }
    }
}

fn zstd_pack_entry_names_by_blob_id(
    local_plan: &ZstdBulkLocalPlan,
) -> BTreeMap<String, String> {
    let mut entry_names = BTreeMap::new();
    for pack in local_plan.object_packs.values() {
        let Some(entries) = pack
            .metadata
            .get("pack_index")
            .and_then(|index| index.get("entries"))
            .and_then(JsonValue::as_array)
        else {
            continue;
        };
        for entry in entries {
            let Some(blob_id) = entry.get("blob_id").and_then(JsonValue::as_str) else {
                continue;
            };
            let Some(entry_name) = entry.get("entry_name").and_then(JsonValue::as_str) else {
                continue;
            };
            entry_names.insert(blob_id.to_string(), entry_name.to_string());
        }
    }
    entry_names
}

fn snapshot_blob_ids_for_manifest(repo_root: &Path, snapshot_id: &str) -> BTreeSet<String> {
    let repo = RepoRuntime::discover_from_path(repo_root).unwrap();
    repo.local_snapshot_operation_store::<1>(repo_root)
        .unwrap()
        .snapshot_tree_file_rows(Some(snapshot_id))
        .unwrap()
        .into_iter()
        .map(|row| row.blob_id)
        .collect()
}

fn zstd_import_manifest_from_local_plan(
    repo_root: &Path,
    repo_name: &str,
    snapshot_id: &str,
    local_plan: &ZstdBulkLocalPlan,
) -> ZstdImportManifestPayload {
    let entry_names = zstd_pack_entry_names_by_blob_id(local_plan);
    let snapshot_blob_ids = snapshot_blob_ids_for_manifest(repo_root, snapshot_id);
    let object_packs = local_plan
        .object_packs
        .values()
        .map(|pack| {
            let mut row = pack.metadata.clone();
            remove_manifest_forbidden_fields(
                &mut row,
                &[
                    "generation_key",
                    "repo_name",
                    "repo_id",
                    "status",
                    "pack_path",
                    "pack_index",
                ],
            );
            row
        })
        .collect::<Vec<_>>();
    let tree_packs = local_plan
        .tree_packs
        .values()
        .map(|pack| {
            let mut row = pack.metadata.clone();
            remove_manifest_forbidden_fields(
                &mut row,
                &[
                    "generation_key",
                    "repo_name",
                    "repo_id",
                    "status",
                    "pack_path",
                    "pack_index",
                ],
            );
            row
        })
        .collect::<Vec<_>>();
    let blob_locators = local_plan
        .blob_locators
        .iter()
        .filter(|(blob_id, _)| snapshot_blob_ids.contains(*blob_id))
        .map(|(blob_id, locator)| {
            let mut row = locator.clone();
            remove_manifest_forbidden_fields(
                &mut row,
                &["generation_key", "storage_path", "storage_kind"],
            );
            row.as_object_mut().unwrap().insert(
                "pack_entry_name".to_string(),
                json!(entry_names.get(blob_id).unwrap()),
            );
            row
        })
        .collect::<Vec<_>>();
    let tree_locators = local_plan
        .tree_locators
        .values()
        .map(|locator| {
            let mut row = locator.clone();
            remove_manifest_forbidden_fields(&mut row, &["generation_key"]);
            row
        })
        .collect::<Vec<_>>();
    let mut snapshot_row = local_plan.snapshots.get(snapshot_id).unwrap().clone();
    snapshot_row
        .as_object_mut()
        .unwrap()
        .entry("snapshot_kind".to_string())
        .or_insert_with(|| json!("line"));
    let commit = ZstdBulkCommitRequest::from_json_rows(
        Some("ait.remote_sync.zstd_bulk.commit.v1".to_string()),
        None,
        object_packs,
        tree_packs,
        blob_locators,
        tree_locators,
        vec![snapshot_row],
        None,
    )
    .unwrap();
    let manifest = ZstdImportManifestPayload {
        contract: ZSTD_IMPORT_MANIFEST_CONTRACT_NAME.to_string(),
        repo_name: repo_name.to_string(),
        snapshot_id: snapshot_id.to_string(),
        snapshots: commit.snapshots,
        object_packs: commit.object_packs,
        tree_packs: commit.tree_packs,
        blob_locators: commit.blob_locators,
        tree_locators: commit.tree_locators,
        line_update: None,
    };
    ZstdImportManifestJson::stateless()
        .validate_domain(&manifest)
        .unwrap();
    manifest
}

fn zstd_remote_import_fixture_from_repo(
    repo_root: &Path,
    snapshot_id: &str,
) -> ZstdRemoteImportFixture {
    let ctx = RemoteSyncLocalStoreContext::new(repo_root);
    let repo = RepoRuntime::discover_from_path(repo_root).unwrap();
    let store = repo.remote_sync_local_store::<1>().unwrap();
    let local_plan = store
        .zstd_bulk_local_plan(&ctx, std::slice::from_ref(&snapshot_id.to_string()), &BTreeSet::new())
        .unwrap();
    let manifest =
        zstd_import_manifest_from_local_plan(repo_root, "fixture-ait", snapshot_id, &local_plan);
    let object_packs = local_plan
        .object_packs
        .values()
        .map(|pack| (pack.pack_id.clone(), fs::read(&pack.pack_abs_path).unwrap()))
        .collect::<BTreeMap<_, _>>();
    let tree_packs = local_plan
        .tree_packs
        .values()
        .map(|pack| (pack.pack_id.clone(), fs::read(&pack.pack_abs_path).unwrap()))
        .collect::<BTreeMap<_, _>>();
    ZstdRemoteImportFixture {
        manifests: BTreeMap::from_iter([(snapshot_id.to_string(), manifest)]),
        object_packs,
        tree_packs,
    }
}

fn assert_zstd_snapshot_download_logged(logged: &[RecordedRequest], snapshot_id: &str) {
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url
            == format!(
                "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/import-manifests/{snapshot_id}"
            )));
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url.starts_with(
            "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/object-packs/"
        )));
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url.starts_with(
            "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/tree-packs/"
        )));
    assert!(!logged.iter().any(|row| row.url.ends_with(":pack")));
}

fn init_worktree_repo_with_rt1_publication(
    base_url: &str,
    _publish_rt1: bool,
) -> (TempDir, std::path::PathBuf) {
    let temp = init_repo_with_fixture_workflow(base_url);
    let root = temp.path();
    let worktree = root.join("rt-1");
    fs::create_dir_all(worktree.join("src")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join(".ait"), worktree.join(".ait")).unwrap();
    write_file(
        &worktree.join(".ait-worktree.json"),
        &format!(
            "{{\n  \"current_line\": \"feature/rt-1\",\n  \"repo_root\": \"{}\",\n  \"workspace_root\": \"{}\",\n  \"worktree_name\": \"rt-1\"\n}}\n",
            root.display(),
            worktree.display()
        ),
    );
    write_file(
        &root.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"root\" }\n",
    );
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn worktree_version() -> &'static str { \"worktree override\" }\n",
    );
    seed_binary_line(root, "feature/rt-1", FIXTURE_BASE_SNAPSHOT_ID);
    write_file(
        &root.join(".ait/worktrees/rt-1.json"),
        &format!(
            concat!(
                "{{\n",
                "  \"name\": \"rt-1\",\n",
                "  \"path\": \"{}\",\n",
                "  \"repo_root\": \"{}\",\n",
                "  \"line_name\": \"feature/rt-1\",\n",
                "  \"bound_task_id\": \"RT-1\",\n",
                "  \"bound_change_id\": \"RC-1\",\n",
                "  \"auto_created_for_task\": true,\n",
                "  \"fork_snapshot_id\": \"{}\",\n",
                "  \"forked_from_line\": \"main\",\n",
                "  \"target_base_line\": \"main\",\n",
                "  \"rebase_state\": \"idle\",\n",
                "  \"rebase_conflict_paths\": []\n",
                "}}\n"
            ),
            worktree.display(),
            root.display(),
            FIXTURE_BASE_SNAPSHOT_ID,
        ),
    );
    (temp, worktree)
}

fn init_worktree_repo(base_url: &str) -> (TempDir, std::path::PathBuf) {
    init_worktree_repo_with_rt1_publication(base_url, true)
}

fn init_local_draft_worktree_repo(base_url: &str) -> (TempDir, std::path::PathBuf) {
    init_worktree_repo_with_rt1_publication(base_url, false)
}

fn init_cli_local_draft_worktree_repo(base_url: &str) -> (TempDir, PathBuf, JsonValue) {
    let temp = init_repo(base_url);
    let started = json_output(
        temp.path(),
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Native local task",
            "--intent",
            "exercise authoritative local workflow state",
            "--json",
        ],
    );
    assert_eq!(started["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(started["change"]["change_id"].as_str(), Some("C-01"));
    let worktree = started["worktree"]["open_path"]
        .as_str()
        .or_else(|| started["worktree"]["path"].as_str())
        .map(PathBuf::from)
        .expect("local task start worktree path");
    (temp, worktree, started)
}

fn init_registered_worktree(
    root: &Path,
    name: &str,
    line_name: &str,
    bound_task_id: Option<&str>,
    bound_change_id: Option<&str>,
    auto_created_for_task: bool,
    cleanup_policy: Option<&str>,
) -> std::path::PathBuf {
    let worktree = root.join(name);
    fs::create_dir_all(worktree.join("src")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join(".ait"), worktree.join(".ait")).unwrap();
    write_file(
        &worktree.join(".ait-worktree.json"),
        &format!(
            "{{\n  \"current_line\": \"{}\",\n  \"repo_root\": \"{}\",\n  \"workspace_root\": \"{}\",\n  \"worktree_name\": \"{}\"\n}}\n",
            line_name,
            root.display(),
            worktree.display(),
            name,
        ),
    );
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn auxiliary_worktree() -> &'static str { \"ok\" }\n",
    );
    seed_binary_line(root, line_name, FIXTURE_BASE_SNAPSHOT_ID);

    let mut metadata = json!({
        "name": name,
        "path": worktree.display().to_string(),
        "repo_root": root.display().to_string(),
        "line_name": line_name,
        "fork_snapshot_id": FIXTURE_BASE_SNAPSHOT_ID,
        "forked_from_line": "main",
        "target_base_line": "main",
        "rebase_state": "idle",
        "rebase_conflict_paths": [],
        "created_at": "2026-06-08T00:00:00Z",
        "auto_created_for_task": auto_created_for_task,
    });
    if let Some(task_id) = bound_task_id {
        metadata["bound_task_id"] = JsonValue::String(task_id.to_string());
    }
    if let Some(change_id) = bound_change_id {
        metadata["bound_change_id"] = JsonValue::String(change_id.to_string());
    }
    if let Some(policy) = cleanup_policy {
        metadata["cleanup_policy"] = JsonValue::String(policy.to_string());
    }
    write_file(
        &root
            .join(".ait")
            .join("worktrees")
            .join(format!("{name}.json")),
        &(encode_json_pretty(&metadata) + "\n"),
    );
    worktree
}

fn snapshot_blob_size(workspace_root: &Path, snapshot_id: &str, path: &str) -> i64 {
    let repo = RepoRuntime::discover_from_path(workspace_root).unwrap();
    repo.local_snapshot_operation_store::<1>(workspace_root)
        .unwrap()
        .snapshot_tree_path_file_rows(snapshot_id, &[path.to_string()])
        .unwrap()
        .get(path)
        .unwrap_or_else(|| panic!("snapshot {snapshot_id} does not contain {path}"))
        .size_bytes
}

fn cargo_bin() -> Command {
    if let Some(path) = option_env!("CARGO_BIN_EXE_ait-cli") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Command::new(candidate);
        }
    }
    let current_exe = std::env::current_exe().unwrap();
    let target_dir = current_exe
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Command::new(target_dir.join("ait-cli"))
}

fn command_output_with_env(root: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = cargo_bin();
    command.current_dir(root).args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().unwrap()
}

fn json_output_with_env(root: &Path, args: &[&str], envs: &[(&str, &str)]) -> JsonValue {
    // Most native primitive tests assert the complete historical receipts. Keep
    // those consumers explicit about compatibility mode while dedicated compact
    // contract tests call `compact_json_output` below.
    let mut effective_args = args.to_vec();
    let command = args.first().copied();
    let subcommand = args.get(1).copied();
    let agent_action_command = command == Some("status")
        || (command == Some("task")
            && (subcommand == Some("start") || subcommand == Some("finish")))
        || (command == Some("snapshot") && subcommand == Some("create"));
    if agent_action_command && args.contains(&"--json") && !args.contains(&"--full") {
        effective_args.push("--full");
    }
    let output = command_output_with_env(root, &effective_args, envs);
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json_bytes(&output.stdout)
}

fn json_output(root: &Path, args: &[&str]) -> JsonValue {
    json_output_with_env(root, args, &[])
}

fn compact_json_output(root: &Path, args: &[&str]) -> JsonValue {
    let output = command_output_with_env(root, args, &[]);
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json_bytes(&output.stdout)
}

fn action_result<'a>(payload: &'a JsonValue, code: &str) -> &'a JsonValue {
    payload["applied_actions"]
        .as_array()
        .and_then(|actions| {
            actions
                .iter()
                .find(|row| row["code"].as_str() == Some(code))
        })
        .and_then(|row| row.get("result"))
        .unwrap_or_else(|| {
            panic!(
                "missing applied action {code} in payload:\n{}",
                encode_json_pretty(payload)
            )
        })
}

fn seed_snapshot(root: &Path, message: &str) -> String {
    json_output(
        root,
        &["snapshot", "create", "--message", message, "--json"],
    )["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn disable_default_remote(root: &Path) {
    let config_path = root.join(".ait/config.json");
    let mut config = parse_json_file(&config_path);
    config
        .as_object_mut()
        .expect("fixture config must be an object")
        .remove("default_remote");
    write_file(&config_path, &(encode_json_pretty(&config) + "\n"));
}
