use super::*;
use crate::json_support::{
    encode_value_pretty_with_newline_error_string, parse_object_or_empty, parse_value_or,
};
use crate::runtime::SNAPSHOT_BINARY_DB_WRITE_LAYOUT;
use ait_core::local_snapshot::{LocalSnapshotBlobReadStore, LocalSnapshotTreeReadStore};

pub(super) const AI_RELATED_AUTHOR_MODES: &[&str] = &[
    "ai_generated",
    "ai_with_human_review",
    "human_with_ai_assist",
];
pub(super) const CODE_REVIEW_SUMMARY_TEMPLATE: &str =
    "Reviewed files: <paths reviewed>; Findings: <blocking/non-blocking findings>; Risks: <residual risks>; Tests: <checks run>; Recommendation: <land/defer/request changes>";
pub(super) const CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE: &str =
    "1. Reviewed files\n<paths reviewed>\n2. Findings\n<blocking/non-blocking findings>\n3. Risks\n<residual risks>\n4. Tests\n<checks run>\n5. Recommendation\n<land/defer/request changes>";
pub(super) const CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND: &str =
    "ait review code template --style numbered";
pub(super) const DEFAULT_WORKFLOW_SCOPE: &str = "local";
pub(super) const COMPLETED_LOCAL_FINAL_SNAPSHOT_PROMOTION_GUIDANCE: &str =
    "To promote completed `solo_local` work, select the latest landed local change and run `ait workflow ready <local-change-id> --apply --remote <name>` once, then `ait task land <local-change-id> --remote <name>`. This publishes the consecutive local Task/Change/Snapshot/Land history while gating only one aggregate Patchset; do not replay earlier local rows with `task publish`, `change publish`, or `--all-completed-local`.";
pub(super) const COMPLETED_LOCAL_BATCH_RETIRED_ERROR: &str =
    "`--all-completed-local` is retired because completed local rows must not be replayed as separate remote patchsets. Select the latest landed local change, run `ait workflow ready <local-change-id> --apply --remote <name>`, then run `ait task land <local-change-id> --remote <name>`.";
pub(super) const APP_DIR: &str = ".ait";
pub(super) const WORKTREE_CONFIG_NAME: &str = ".ait-worktree.json";
pub(super) const WORKFLOW_READY_POLL_SECONDS_KEY: &str = "workflow_ready_poll_seconds";
pub(super) const WORKFLOW_LAND_POLL_SECONDS_KEY: &str = "workflow_land_poll_seconds";
pub(super) const PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND: &str = "workflow_ready_foreground";
pub(super) const WORKFLOW_WAIT_HINT_BOOTSTRAP_MISS: i64 = 0;
pub(super) const WORKFLOW_WAIT_HINT_ALPHA: f64 = 0.5;
pub(super) const WORKFLOW_WAIT_HINT_MIN_SECONDS: i64 = 5;
pub(super) const WORKFLOW_WAIT_HINT_MAX_SECONDS: i64 = 900;
pub(super) const WORKFLOW_WAIT_HINT_HISTORY_LIMIT: usize = 40;
pub(super) const WORKFLOW_WAIT_HINT_SAMPLE_LIMIT: usize = 12;
pub(super) const WORKFLOW_APPLY_FOREGROUND_WAIT_MAX_SECONDS: f64 = 900.0;
pub(super) const WORKFLOW_APPLY_FOREGROUND_WAIT_POLL_SECONDS: f64 = 0.25;
pub(super) const INTERNAL_WORKTREE_ROLE_MAIN_SEED: &str = "main_seed_mirror";
pub(super) const MAIN_SEED_LAYOUT_VERSION: i64 = 1;
pub(super) const MAIN_SEED_COPY_EXCLUDE_NAMES: &[&str] = &[APP_DIR, WORKTREE_CONFIG_NAME, ".venv"];
#[cfg(target_os = "macos")]
pub(super) const CLONEFILE_FALLBACK_ERRNOS: &[i32] = &[1, 18, 22, 45, 78];
#[cfg(target_os = "linux")]
pub(super) const LINUX_FICLONE: libc::c_ulong = 0x40049409;
#[cfg(target_os = "linux")]
pub(super) const REFLINK_FALLBACK_ERRNOS: &[i32] = &[
    libc::EOPNOTSUPP,
    libc::ENOTTY,
    libc::EXDEV,
    libc::EINVAL,
    libc::ENOSYS,
    libc::EPERM,
];
pub(super) const REVIEW_SECTION_LABELS: &[(&str, &[&str])] = &[
    (
        "Reviewed files",
        &[
            "reviewed files",
            "files reviewed",
            "reviewed file",
            "files",
            "paths reviewed",
        ],
    ),
    ("Findings", &["findings", "issues", "observations"]),
    ("Risks", &["risks", "residual risks", "risk"]),
    (
        "Tests",
        &["tests", "checks run", "validation", "verification"],
    ),
    (
        "Recommendation",
        &[
            "recommendation",
            "promotion recommendation",
            "land recommendation",
            "verdict",
            "decision",
        ],
    ),
];

#[derive(Clone, Debug)]
pub struct TaskStartBootstrapRequest<'a> {
    pub task: &'a JsonValue,
    pub change: Option<&'a JsonValue>,
    pub title_hint: &'a str,
    pub intent_hint: &'a str,
    pub base_line_name: &'a str,
    pub local: bool,
    pub remote_name: Option<&'a str>,
    pub worktree_name: &'a str,
    pub worktree_path: &'a str,
    pub worktree_alias_path: Option<&'a str>,
    pub worktree_root_source: Option<&'a str>,
    pub worktree_fallback_reason: Option<&'a str>,
    pub worktree_default_line: Option<&'a str>,
    pub worktree_seed_snapshot_id: Option<&'a str>,
    pub worktree_seed_snapshot_total_bytes: Option<i64>,
    pub worktree_main_seed_ram_max_bytes: Option<i64>,
}

pub(super) type TaskStartProgressEmitter<'a> = dyn FnMut(&JsonValue) -> Result<(), String> + 'a;

#[derive(Clone, Debug, Default)]
pub(super) struct MainSeedState {
    pub(super) exists: bool,
    pub(super) internal_role: Option<String>,
    pub(super) workspace_root: Option<String>,
    pub(super) seed_line_name: Option<String>,
    pub(super) seed_snapshot_id: Option<String>,
    pub(super) seed_ignore_rules_blob_id: Option<String>,
    pub(super) worktree_name: Option<String>,
    pub(super) seed_refreshed_at: Option<String>,
    pub(super) seed_content_fingerprint: Option<String>,
    pub(super) seed_content_row_count: Option<usize>,
    pub(super) layout_version: Option<i64>,
}

#[derive(Clone, Debug)]
pub(super) struct MainSeedMirrorResult {
    pub(super) status: String,
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) line_name: String,
    pub(super) seed_snapshot_id: String,
    pub(super) root_source: Option<String>,
    pub(super) seed_refreshed_at: Option<String>,
    pub(super) baseline_seed_snapshot_id: Option<String>,
    pub(super) refresh_strategy: String,
    pub(super) copy_strategy: Option<String>,
    pub(super) copy_error: Option<String>,
    pub(super) materialized_write_count: Option<usize>,
    pub(super) materialized_remove_count: Option<usize>,
    pub(super) materialized_unchanged_count: Option<usize>,
    pub(super) phase_timings_ms: Option<JsonValue>,
    pub(super) error: Option<String>,
}

impl MainSeedMirrorResult {
    pub(super) fn to_json(&self) -> JsonValue {
        json!({
            "status": self.status,
            "name": self.name,
            "path": self.path.to_string_lossy().to_string(),
            "line_name": self.line_name,
            "seed_snapshot_id": self.seed_snapshot_id,
            "root_source": self.root_source,
            "seed_refreshed_at": self.seed_refreshed_at,
            "baseline_seed_snapshot_id": self.baseline_seed_snapshot_id,
            "refresh_strategy": self.refresh_strategy,
            "copy_strategy": self.copy_strategy,
            "copy_error": self.copy_error,
            "materialized_write_count": self.materialized_write_count,
            "materialized_remove_count": self.materialized_remove_count,
            "materialized_unchanged_count": self.materialized_unchanged_count,
            "phase_timings_ms": self.phase_timings_ms,
            "error": self.error,
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct MainSeedMaterializationPlan {
    pub(super) write_count: usize,
    pub(super) remove_count: usize,
    pub(super) unchanged_count: usize,
    pub(super) baseline_integrity_reset: bool,
    pub(super) content_fingerprint: String,
    pub(super) content_row_count: usize,
    pub(super) phase_timings_ms: JsonValue,
}

#[derive(Clone, Debug, Default)]
pub(super) struct SnapshotRulesState {
    pub(super) blob_id: Option<String>,
    pub(super) text: Option<String>,
}

pub(super) fn task_identity_slug(task_id: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in task_id.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let normalized = out.trim_matches('-').to_string();
    if normalized.is_empty() {
        Err("Task id is required to derive a task worktree name.".to_string())
    } else {
        Ok(normalized)
    }
}

pub(super) fn task_bound_worktree_name(task_id: &str) -> Result<String, String> {
    task_identity_slug(task_id)
}

pub(super) fn legacy_task_bound_worktree_name(task_id: &str) -> Result<String, String> {
    Ok(format!("task-{}", task_bound_worktree_name(task_id)?))
}

pub(super) fn task_feature_line_name(task_id: &str) -> Result<String, String> {
    Ok(format!("feature/{}", task_bound_worktree_name(task_id)?))
}

pub(super) fn legacy_task_feature_line_name(task_id: &str) -> Result<String, String> {
    Ok(format!(
        "feature/{}",
        legacy_task_bound_worktree_name(task_id)?
    ))
}

pub(super) fn task_feature_line_candidates(task_id: &str) -> Result<Vec<String>, String> {
    let primary = task_feature_line_name(task_id)?;
    let legacy = legacy_task_feature_line_name(task_id)?;
    if primary == legacy {
        Ok(vec![primary])
    } else {
        Ok(vec![primary, legacy])
    }
}

pub(super) fn path_exists_or_directory_link(path: &Path) -> bool {
    path.exists() || fs::symlink_metadata(path).is_ok()
}

pub(super) fn remove_path_entry(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|err| err.to_string())?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path).map_err(|err| err.to_string())
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|err| err.to_string())
    } else {
        fs::remove_file(path).map_err(|err| err.to_string())
    }
}

pub(super) fn create_directory_link(link_path: &Path, target_path: &Path) -> Result<(), String> {
    if path_exists_or_directory_link(link_path) {
        return Err(format!(
            "Worktree alias path is already in use: {}",
            link_path.display()
        ));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target_path, link_path).map_err(|err| err.to_string())
    }
    #[cfg(not(unix))]
    {
        std::os::windows::fs::symlink_dir(target_path, link_path).map_err(|err| err.to_string())
    }
}

pub(super) fn directory_link_points_at(
    link_path: &Path,
    target_path: &Path,
) -> Result<bool, String> {
    let link_target = fs::read_link(link_path).map_err(|err| err.to_string())?;
    let resolved_link_target = if link_target.is_absolute() {
        link_target
    } else {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(link_target)
    };
    if let (Ok(canonical_link_target), Ok(canonical_target)) = (
        resolved_link_target.canonicalize(),
        target_path.canonicalize(),
    ) {
        return Ok(canonical_link_target == canonical_target);
    }
    Ok(lexical_normalize(&resolved_link_target) == lexical_normalize(target_path))
}

pub(super) fn write_json_pretty(path: &Path, payload: &JsonValue) -> Result<(), String> {
    let encoded = encode_value_pretty_with_newline_error_string(payload)?;
    fs::write(path, encoded).map_err(|err| err.to_string())
}

pub(super) fn read_json_document(path: &Path) -> JsonValue {
    let Ok(content) = fs::read_to_string(path) else {
        return json!({});
    };
    parse_value_or(&content, json!({}))
}

pub(super) fn read_json_object_value(path: &Path) -> JsonMap<String, JsonValue> {
    let Ok(content) = fs::read_to_string(path) else {
        return JsonMap::new();
    };
    parse_object_or_empty(&content)
}

pub(super) fn main_seed_worktree_name(line_name: &str) -> String {
    let trimmed = normalized_text(Some(line_name)).unwrap_or_else(|| "main".to_string());
    format!("{trimmed}-seed")
}

pub(super) fn main_seed_local_config(seed_path: &Path) -> JsonMap<String, JsonValue> {
    read_json_object_value(&seed_path.join(WORKTREE_CONFIG_NAME))
}

pub(super) fn main_seed_state(seed_path: &Path) -> MainSeedState {
    if !seed_path.is_dir() {
        return MainSeedState::default();
    }
    let payload = main_seed_local_config(seed_path);
    MainSeedState {
        exists: true,
        internal_role: payload
            .get("internal_role")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value))),
        workspace_root: payload
            .get("workspace_root")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value))),
        seed_line_name: payload
            .get("seed_line_name")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value))),
        seed_snapshot_id: payload
            .get("seed_snapshot_id")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value)))
            .or_else(|| {
                payload
                    .get("materialized_snapshot_id")
                    .and_then(JsonValue::as_str)
                    .and_then(|value| normalized_text(Some(value)))
            }),
        seed_ignore_rules_blob_id: payload
            .get("seed_ignore_rules_blob_id")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value))),
        worktree_name: payload
            .get("worktree_name")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value))),
        seed_refreshed_at: payload
            .get("seed_refreshed_at")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value))),
        seed_content_fingerprint: payload
            .get("seed_content_fingerprint")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value))),
        seed_content_row_count: payload
            .get("seed_content_row_count")
            .and_then(JsonValue::as_u64)
            .and_then(|value| usize::try_from(value).ok()),
        layout_version: payload.get("layout_version").and_then(JsonValue::as_i64),
    }
}

pub(super) fn is_seed_state_aligned(
    repo: &RepoRuntime,
    seed_state: &MainSeedState,
    seed_path: &Path,
    line_name: &str,
    snapshot_id: &str,
) -> bool {
    let workspace_root = seed_path.to_string_lossy();
    let metadata_aligned = seed_state.exists
        && seed_state.internal_role.as_deref() == Some(INTERNAL_WORKTREE_ROLE_MAIN_SEED)
        && seed_state.workspace_root.as_deref() == Some(workspace_root.as_ref())
        && seed_state.seed_line_name.as_deref() == Some(line_name)
        && seed_state.seed_snapshot_id.as_deref() == Some(snapshot_id)
        && seed_state.layout_version == Some(MAIN_SEED_LAYOUT_VERSION);
    if !metadata_aligned {
        return false;
    }
    if seed_state.seed_content_fingerprint.is_none() || seed_state.seed_content_row_count.is_none()
    {
        return false;
    }
    let Ok(seed_repo) = RepoRuntime::discover_from_path(seed_path) else {
        return false;
    };
    let Ok(snapshot_rules) = snapshot_rules_state_from_snapshot_id(repo, Some(snapshot_id)) else {
        return false;
    };
    validate_main_seed_baseline(&seed_repo, snapshot_id, snapshot_rules.text.as_deref())
        .map(|integrity| integrity.clean)
        .unwrap_or(false)
}

pub(super) fn main_seed_content_fingerprint(seed_path: &Path) -> Result<(String, usize), String> {
    let mut ignore_parts = Vec::new();
    let ignore_path = seed_path.join(".aitignore");
    if ignore_path.is_file() {
        let text = fs::read_to_string(&ignore_path)
            .map_err(|err| format!("Failed to read {}: {err}", ignore_path.display()))?;
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            ignore_parts.push(trimmed.to_string());
        }
    }
    ignore_parts.push("/docs/".to_string());
    let ignore_rules = ignore_parts.join("\n") + "\n";
    let entries = list_visible_workspace_entries(
        seed_path.to_string_lossy().as_ref(),
        Some(ignore_rules.as_str()),
        None,
    )
    .map_err(|err| format!("{err:?}"))?;
    let workspace_root_text = seed_path.to_string_lossy().to_string();
    let mut rows = Vec::new();
    for rel in entries.files {
        if rel == WORKTREE_CARGO_CONFIG_RELATIVE_PATH
            || path_is_projected_out_for_workspace(&workspace_root_text, &rel, true)
        {
            continue;
        }
        let abs_path = seed_path.join(&rel);
        let metadata = abs_path
            .metadata()
            .map_err(|err| format!("Failed to read metadata for {}: {err}", abs_path.display()))?;
        if !metadata.is_file() {
            continue;
        }
        let data = fs::read(&abs_path)
            .map_err(|err| format!("Failed to read {}: {err}", abs_path.display()))?;
        rows.push((
            rel,
            sha256_hex_bytes(&data),
            format!("{:#o}", metadata.permissions().mode() & 0o777),
        ));
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (path, sha256, mode) in &rows {
        for value in [path.as_str(), sha256.as_str(), mode.as_str()] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
    }
    Ok((format!("{:x}", digest.finalize()), rows.len()))
}

pub(super) fn main_seed_refresh_baseline_snapshot_id(
    seed_state: &MainSeedState,
    line_name: &str,
) -> Option<String> {
    if !seed_state.exists {
        return None;
    }
    if seed_state.internal_role.as_deref() != Some(INTERNAL_WORKTREE_ROLE_MAIN_SEED) {
        return None;
    }
    if seed_state.seed_line_name.as_deref() != Some(line_name) {
        return None;
    }
    if seed_state.layout_version != Some(MAIN_SEED_LAYOUT_VERSION) {
        return None;
    }
    seed_state.seed_snapshot_id.clone()
}

pub(super) fn main_seed_refresh_lock_path(repo: &RepoRuntime, line_name: &str) -> PathBuf {
    repo.authoritative_repo_root()
        .join(".ait")
        .join("workspace")
        .join("locks")
        .join(format!(
            "{}.refresh.lock",
            main_seed_worktree_name(line_name)
        ))
}

pub(super) fn normalized_ignore_rules_text_for_hash(
    ignore_rules_text: Option<&str>,
) -> Option<String> {
    let trimmed = ignore_rules_text.unwrap_or("").trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(format!("{trimmed}\n"))
    }
}

pub(super) fn status_ignore_rules_hash(ignore_rules_text: Option<&str>) -> String {
    let normalized = normalized_ignore_rules_text_for_hash(ignore_rules_text).unwrap_or_default();
    sha256_hex_bytes(normalized.as_bytes())
}

pub(super) fn status_ignore_rules_hash_without_worktree_docs(
    ignore_rules_text: Option<&str>,
) -> String {
    let Some(text) = ignore_rules_text else {
        return status_ignore_rules_hash(None);
    };
    let filtered = text
        .lines()
        .filter(|line| line.trim() != "/docs/")
        .collect::<Vec<_>>()
        .join("\n");
    status_ignore_rules_hash(Some(&filtered))
}

fn snapshot_file_row_json(row: SnapshotFileRow) -> JsonValue {
    json!({
        "path": row.path,
        "blob_id": row.blob_id,
        "size_bytes": row.size_bytes,
        "mode": row.mode,
        "sha256": row.sha256,
    })
}

fn read_selected_snapshot_blob_bytes_batch(
    repo: &RepoRuntime,
    blob_ids: &[String],
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    store.read_blob_bytes_batch(blob_ids)
}

pub(super) fn status_manifest_ignore_rules_hash(
    repo: &RepoRuntime,
    ignore_rules_text: Option<&str>,
) -> String {
    if repo.is_worktree() {
        status_ignore_rules_hash_without_worktree_docs(ignore_rules_text)
    } else {
        status_ignore_rules_hash(ignore_rules_text)
    }
}

pub fn ensure_status_manifest(repo: &RepoRuntime, snapshot_id: &str) -> Result<JsonValue, String> {
    let snapshot_id = normalized_text(Some(snapshot_id))
        .ok_or_else(|| "snapshot_id is required to load snapshot status metadata".to_string())?;
    let effective_ignore_rules = effective_ignore_rules_text(repo, None)?;
    let index = filtered_snapshot_manifest_index(
        repo,
        Some(snapshot_id.as_str()),
        effective_ignore_rules.as_deref(),
    )?;
    Ok(json!({
        "snapshot_id": snapshot_id,
        "source": "snapshot_binary_metadata",
        "row_count": index.rows.len(),
    }))
}

pub(super) struct LocalRepoSnapshotReader<'a> {
    pub(super) snapshot_store: &'a dyn SnapshotStore,
    pub(super) tree_read_store: &'a dyn LocalSnapshotTreeReadStore,
    pub(super) tree_pack_store: Option<&'a dyn TreePackStore>,
}

impl SnapshotReader for LocalRepoSnapshotReader<'_> {
    fn read_snapshot_manifest(&self, snapshot_id: &str) -> Result<JsonValue, String> {
        let rows = self
            .tree_read_store
            .snapshot_tree_file_rows(Some(snapshot_id))?;
        Ok(JsonValue::Object(JsonMap::from_iter(rows.into_iter().map(
            |row| {
                let path = row.path.clone();
                (path, snapshot_file_row_json(row))
            },
        ))))
    }

    fn read_snapshot_payload(&self, snapshot_id: &str) -> Result<Option<JsonValue>, String> {
        snapshot_payload_with_snapshot_store(self.snapshot_store, snapshot_id)
    }

    fn read_tree_payload(&self, tree_id: &str) -> Result<Option<JsonValue>, String> {
        let Some(tree_pack_store) = self.tree_pack_store else {
            return Ok(None);
        };
        tree_pack_store.read_tree_payload(tree_id)
    }
}

pub(super) fn snapshot_payload_with_snapshot_store<S>(
    snapshot_store: &S,
    snapshot_id: &str,
) -> Result<Option<JsonValue>, String>
where
    S: SnapshotStore + ?Sized,
{
    let Some(snapshot) = snapshot_by_id_with_snapshot_store(snapshot_store, snapshot_id)? else {
        return Ok(None);
    };
    Ok(Some(snapshot_payload_json(&snapshot)))
}

fn snapshot_payload_json(snapshot: &SnapshotRecord) -> JsonValue {
    json!({
        "snapshot_id": &snapshot.snapshot_id,
        "parent_snapshot_id": &snapshot.parent_snapshot_id,
        "root_tree_pack_id": &snapshot.root_tree_pack_id,
        "root_entry_ordinal": snapshot.root_entry_ordinal,
        "line_name": &snapshot.line_name,
        "snapshot_kind": &snapshot.snapshot_kind,
        "created_at": &snapshot.created_at,
    })
}

pub(super) struct LocalRepoBlobReader<'a> {
    pub(super) blob_store: &'a dyn LocalSnapshotBlobReadStore,
}

impl BlobReader for LocalRepoBlobReader<'_> {
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(Some(self.blob_store.read_blob_bytes(blob_id)?))
    }
}

pub(super) fn snapshot_row_path(row: &JsonValue) -> Option<String> {
    row.get("path")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
}

pub(super) fn snapshot_row_visible(
    repo: &RepoRuntime,
    row: &JsonValue,
    ignore_matcher: Option<&WorkspaceIgnoreMatcher>,
) -> Result<bool, String> {
    let Some(path) = snapshot_row_path(row) else {
        return Ok(false);
    };
    let workspace_root = repo.workspace_root();
    let workspace_root_text = workspace_root.to_string_lossy().to_string();
    if path_is_projected_out_for_workspace(&workspace_root_text, &path, repo.is_worktree()) {
        return Ok(false);
    }
    if ignore_matcher
        .map(|matcher| workspace_relative_path_is_ignored_with_matcher(&path, matcher))
        .unwrap_or(false)
    {
        return Ok(false);
    }
    if path == WORKTREE_CARGO_CONFIG_RELATIVE_PATH {
        if let Some(blob_id) = file_map_row_blob_id(row) {
            let contents = read_snapshot_blob_text(repo, &blob_id)?;
            if repo.is_worktree()
                && (matches_source_worktree_cargo_config_text(contents.as_str())
                    || matches_generated_worktree_cargo_config_text(
                        &workspace_root,
                        contents.as_str(),
                    ))
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(super) fn snapshot_file_row_visible(
    repo: &RepoRuntime,
    row: &SnapshotFileRow,
    ignore_matcher: Option<&WorkspaceIgnoreMatcher>,
) -> Result<bool, String> {
    snapshot_entry_visible_for_workspace(
        repo,
        &repo.workspace_root(),
        repo.is_worktree(),
        &row.path,
        &row.blob_id,
        ignore_matcher,
    )
}

pub(super) fn snapshot_entry_visible_for_workspace(
    repo: &RepoRuntime,
    workspace_root: &Path,
    is_worktree: bool,
    path: &str,
    blob_id: &str,
    ignore_matcher: Option<&WorkspaceIgnoreMatcher>,
) -> Result<bool, String> {
    let workspace_root_text = workspace_root.to_string_lossy().to_string();
    if path_is_projected_out_for_workspace(&workspace_root_text, path, is_worktree) {
        return Ok(false);
    }
    if ignore_matcher
        .map(|matcher| workspace_relative_path_is_ignored_with_matcher(path, matcher))
        .unwrap_or(false)
    {
        return Ok(false);
    }
    if path == WORKTREE_CARGO_CONFIG_RELATIVE_PATH {
        let contents = read_snapshot_blob_text(repo, blob_id)?;
        if is_worktree
            && (matches_source_worktree_cargo_config_text(contents.as_str())
                || matches_generated_worktree_cargo_config_text(workspace_root, contents.as_str()))
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn filtered_snapshot_manifest_index(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    ignore_rules_text: Option<&str>,
) -> Result<SnapshotTreeManifestIndex, String> {
    filtered_snapshot_manifest_index_for_workspace(
        repo,
        &repo.workspace_root(),
        repo.is_worktree(),
        snapshot_id,
        ignore_rules_text,
    )
}

pub(super) fn filtered_snapshot_manifest_index_for_workspace(
    repo: &RepoRuntime,
    workspace_root: &Path,
    is_worktree: bool,
    snapshot_id: Option<&str>,
    ignore_rules_text: Option<&str>,
) -> Result<SnapshotTreeManifestIndex, String> {
    let ignore_matcher = ignore_rules_text.map(parse_workspace_ignore_matcher);
    let repo_workspace_root = repo.workspace_root();
    let store = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&repo_workspace_root)?;
    let mut index =
        SnapshotTreeManifestIndex::from_file_rows(store.snapshot_tree_file_rows(snapshot_id)?)?;
    let rows = std::mem::take(&mut index.rows);
    index.rows = rows
        .into_iter()
        .filter_map(|row| {
            let visible = (|| {
                let path = index.row_path(&row)?;
                let blob_id = index.row_blob_id(&row)?;
                snapshot_entry_visible_for_workspace(
                    repo,
                    workspace_root,
                    is_worktree,
                    path,
                    blob_id,
                    ignore_matcher.as_ref(),
                )
            })();
            match visible {
                Ok(true) => Some(Ok(row)),
                Ok(false) => None,
                Err(err) => Some(Err(err)),
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(index)
}

pub(super) fn filtered_snapshot_path_file_rows(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    paths: &[String],
    ignore_rules_text: Option<&str>,
) -> Result<BTreeMap<String, SnapshotFileRow>, String> {
    let Some(snapshot_id) = snapshot_id.and_then(|value| normalized_text(Some(value))) else {
        return Ok(BTreeMap::new());
    };
    let ignore_matcher = ignore_rules_text.map(parse_workspace_ignore_matcher);
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let mut rows = BTreeMap::new();
    for (path, row) in store.snapshot_tree_path_file_rows(&snapshot_id, paths)? {
        if snapshot_file_row_visible(repo, &row, ignore_matcher.as_ref())? {
            rows.insert(path, row);
        }
    }
    Ok(rows)
}

pub(super) fn filtered_snapshot_path_manifest_index(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    paths: &[String],
    ignore_rules_text: Option<&str>,
) -> Result<SnapshotTreeManifestIndex, String> {
    SnapshotTreeManifestIndex::from_file_rows(
        filtered_snapshot_path_file_rows(repo, snapshot_id, paths, ignore_rules_text)?
            .into_values()
            .collect(),
    )
}

pub(super) fn filtered_snapshot_rows_json(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    ignore_rules_text: Option<&str>,
) -> Result<Vec<JsonValue>, String> {
    let ignore_matcher = ignore_rules_text.map(parse_workspace_ignore_matcher);
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let mut rows = Vec::new();
    for row in store
        .snapshot_tree_file_rows(snapshot_id)?
        .into_iter()
        .map(snapshot_file_row_json)
    {
        if snapshot_row_visible(repo, &row, ignore_matcher.as_ref())? {
            rows.push(row);
        }
    }
    Ok(rows)
}

pub(super) fn filtered_snapshot_path_rows(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    paths: &[String],
    ignore_rules_text: Option<&str>,
) -> Result<BTreeMap<String, JsonValue>, String> {
    let Some(snapshot_id) = snapshot_id.and_then(|value| normalized_text(Some(value))) else {
        return Ok(BTreeMap::new());
    };
    let ignore_matcher = ignore_rules_text.map(parse_workspace_ignore_matcher);
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let mut rows = BTreeMap::new();
    for (path, row) in store.snapshot_tree_path_rows(&snapshot_id, paths)? {
        if snapshot_row_visible(repo, &row, ignore_matcher.as_ref())? {
            rows.insert(path, row);
        }
    }
    Ok(rows)
}

pub(super) fn snapshot_rules_state_from_snapshot_id(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
) -> Result<SnapshotRulesState, String> {
    let Some(snapshot_id) = snapshot_id.and_then(|value| normalized_text(Some(value))) else {
        return Ok(SnapshotRulesState::default());
    };
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let Some(row) = store.snapshot_tree_path_row(&snapshot_id, WORKSPACE_IGNORE_FILE)? else {
        return Ok(SnapshotRulesState::default());
    };
    let blob_id = file_map_row_blob_id(&row);
    let text = blob_id
        .as_ref()
        .map(|blob_id| read_snapshot_blob_text(repo, blob_id))
        .transpose()?;
    Ok(SnapshotRulesState { blob_id, text })
}

#[derive(Debug)]
pub(super) struct MainSeedBaselineIntegrity {
    pub(super) clean: bool,
    pub(super) changed_count: usize,
    pub(super) untracked_paths: Vec<String>,
}

pub(super) fn validate_main_seed_cargo_projection(
    seed_repo: &RepoRuntime,
    baseline_snapshot_id: &str,
) -> Result<(), String> {
    let workspace_root = seed_repo.workspace_root();
    let cargo_config_path = workspace_root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
    let metadata = fs::symlink_metadata(&cargo_config_path)
        .map_err(|err| format!("Cargo projection metadata is unavailable: {err}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Cargo projection is not a physical file.".to_string());
    }
    let store = seed_repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let Some(row) =
        store.snapshot_tree_path_row(baseline_snapshot_id, WORKTREE_CARGO_CONFIG_RELATIVE_PATH)?
    else {
        return Err("Snapshot has no source Cargo configuration.".to_string());
    };
    let Some(blob_id) = file_map_row_blob_id(&row) else {
        return Err("Snapshot Cargo configuration has no Blob identity.".to_string());
    };
    let source = read_snapshot_blob_text(seed_repo, &blob_id)?;
    let Some(expected) = upgrade_generated_worktree_cargo_config_text(&workspace_root, &source)
    else {
        return Err("Snapshot Cargo configuration is not a projectable source policy.".to_string());
    };
    let actual = fs::read_to_string(&cargo_config_path).map_err(|err| err.to_string())?;
    if actual != expected {
        return Err(format!(
            "Cargo projection content does not match the seed workspace path: expected_sha256={}, actual_sha256={}.",
            sha256_hex_bytes(expected.as_bytes()),
            sha256_hex_bytes(actual.as_bytes())
        ));
    }
    let fingerprint = workspace_file_fingerprint(&cargo_config_path)?;
    let expected_mode = readonly_file_mode(parse_mode_bits(
        row.get("mode").and_then(JsonValue::as_str),
    )?) & 0o777;
    if fingerprint.file_kind != "file" || fingerprint.mode_bits & 0o777 != expected_mode {
        return Err(format!(
            "Cargo projection mode does not match: expected {expected_mode:#o}, got {:#o}.",
            fingerprint.mode_bits & 0o777
        ));
    }
    Ok(())
}

pub(super) fn validate_main_seed_baseline(
    seed_repo: &RepoRuntime,
    baseline_snapshot_id: &str,
    snapshot_rules_text: Option<&str>,
) -> Result<MainSeedBaselineIntegrity, String> {
    let effective_ignore_rules = effective_ignore_rules_text(seed_repo, snapshot_rules_text)?;
    let ignore_rules_hash =
        status_manifest_ignore_rules_hash(seed_repo, effective_ignore_rules.as_deref());
    let baseline_manifest = filtered_snapshot_manifest_index(
        seed_repo,
        Some(baseline_snapshot_id),
        effective_ignore_rules.as_deref(),
    )?;
    let workspace_scan = workspace_state_for_status(
        seed_repo,
        baseline_snapshot_id,
        &ignore_rules_hash,
        &baseline_manifest,
        effective_ignore_rules.as_deref(),
        None,
    )?;
    let mut remaining = workspace_scan.files;
    if remaining.contains_key(WORKTREE_CARGO_CONFIG_RELATIVE_PATH)
        && validate_main_seed_cargo_projection(seed_repo, baseline_snapshot_id).is_ok()
    {
        remaining.remove(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
    }
    let mut changed_count = 0_usize;
    for row in &baseline_manifest.rows {
        let path = baseline_manifest.row_path(row)?;
        let Some(current) = remaining.remove(path) else {
            changed_count += 1;
            continue;
        };
        let expected_mode = format!(
            "{:#o}",
            readonly_file_mode(parse_mode_bits(Some(row.mode.as_str()))?) & 0o777
        );
        if current.sha256 != row.sha256 || current.mode != expected_mode {
            changed_count += 1;
        }
    }
    let untracked_paths = remaining.into_keys().collect::<Vec<_>>();
    changed_count = changed_count.saturating_add(untracked_paths.len());
    Ok(MainSeedBaselineIntegrity {
        clean: changed_count == 0,
        changed_count,
        untracked_paths,
    })
}

pub(super) fn materialize_main_seed_snapshot(
    seed_repo: &RepoRuntime,
    target_snapshot_id: &str,
    baseline_snapshot_id: Option<&str>,
    snapshot_rules_state: &SnapshotRulesState,
    snapshot_rules_elapsed_ms: f64,
) -> Result<MainSeedMaterializationPlan, String> {
    let total_started = Instant::now();
    let workspace_root = seed_repo.workspace_root();
    let mut delta_elapsed = 0.0;
    let target_rows_elapsed: f64;
    let mut target_row_lookup_elapsed = 0.0;
    let mut write_paths = Vec::new();
    let mut remove_paths = Vec::new();
    let mut baseline_integrity_reset = false;
    let mut baseline_integrity_changed_count = 0_usize;
    let mut baseline_validation_elapsed = 0.0;
    let target_manifest: SnapshotTreeManifestIndex;
    let target_write_rows: BTreeMap<u32, SnapshotTreeManifestRow>;
    let visible_row_count_strategy: String;

    let effective_baseline_snapshot_id = if let Some(baseline_snapshot_id) = baseline_snapshot_id {
        let baseline_validation_started = Instant::now();
        let integrity = validate_main_seed_baseline(
            seed_repo,
            baseline_snapshot_id,
            snapshot_rules_state.text.as_deref(),
        )?;
        baseline_validation_elapsed = elapsed_ms(baseline_validation_started);
        baseline_integrity_changed_count = integrity.changed_count;
        if integrity.clean {
            Some(baseline_snapshot_id)
        } else {
            baseline_integrity_reset = true;
            remove_paths = reverse_depth_sort_paths(integrity.untracked_paths);
            None
        }
    } else {
        None
    };

    if effective_baseline_snapshot_id.is_none() {
        let target_rows_started = Instant::now();
        let mut rows = BTreeMap::new();
        target_manifest = filtered_snapshot_manifest_index(
            seed_repo,
            Some(target_snapshot_id),
            snapshot_rules_state.text.as_deref(),
        )?;
        for row in &target_manifest.rows {
            let path = target_manifest.row_path(row)?.to_string();
            write_paths.push(path.clone());
            rows.insert(row.path_id, row.clone());
        }
        target_rows_elapsed = elapsed_ms(target_rows_started);
        target_write_rows = rows;
        // Preserve the public timing label while avoiding the redundant delta+lookup pass.
        visible_row_count_strategy = if baseline_integrity_reset {
            "baseline_integrity_reset_full_projection".to_string()
        } else {
            "empty_seed_full_delta".to_string()
        };
    } else {
        let delta_started = Instant::now();
        let (_affected_paths, delta_status_by_path) = filtered_snapshot_tree_delta(
            seed_repo,
            effective_baseline_snapshot_id,
            Some(target_snapshot_id),
            snapshot_rules_state.text.as_deref(),
        )?;
        delta_elapsed = elapsed_ms(delta_started);
        write_paths = delta_status_by_path
            .iter()
            .filter_map(|(path, status)| {
                if status == "deleted" {
                    None
                } else {
                    Some(path.clone())
                }
            })
            .collect::<Vec<_>>();
        remove_paths = reverse_depth_sort_paths(
            delta_status_by_path
                .iter()
                .filter_map(|(path, status)| {
                    if status == "deleted" {
                        Some(path.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>(),
        );
        visible_row_count_strategy = "baseline_delta_with_post_apply_fingerprint".to_string();
        target_rows_elapsed = 0.0;
        let target_row_lookup_started = Instant::now();
        target_manifest = filtered_snapshot_path_manifest_index(
            seed_repo,
            Some(target_snapshot_id),
            &write_paths,
            snapshot_rules_state.text.as_deref(),
        )?;
        for rel in &write_paths {
            if !target_manifest.path_id_by_path.contains_key(rel.as_str()) {
                return Err(format!("Snapshot row is missing `{rel}`."));
            }
        }
        target_write_rows = target_manifest
            .rows
            .iter()
            .map(|row| (row.path_id, row.clone()))
            .collect();
        target_row_lookup_elapsed = elapsed_ms(target_row_lookup_started);
    }

    let remove_apply_started = Instant::now();
    for rel in &remove_paths {
        let abs_path = workspace_root.join(rel);
        if path_exists_or_directory_link(&abs_path) {
            remove_path_entry(&abs_path)?;
            prune_empty_parent_dirs(&workspace_root, &abs_path)?;
        }
    }
    let remove_apply_elapsed = elapsed_ms(remove_apply_started);

    let blob_plan_started = Instant::now();
    let mut blob_ids = BTreeSet::new();
    for rel in &write_paths {
        let path_id = *target_manifest
            .path_id_by_path
            .get(rel.as_str())
            .ok_or_else(|| format!("Snapshot row is missing `{rel}`."))?;
        let target_row = target_write_rows
            .get(&path_id)
            .ok_or_else(|| format!("Snapshot row is missing `{rel}`."))?;
        let blob_id = target_manifest.row_blob_id(target_row)?;
        blob_ids.insert(blob_id.to_string());
    }
    let blob_plan_elapsed = elapsed_ms(blob_plan_started);
    let blob_read_started = Instant::now();
    let blob_bytes_by_id = read_selected_snapshot_blob_bytes_batch(
        seed_repo,
        &blob_ids.into_iter().collect::<Vec<_>>(),
    )?;
    let blob_read_elapsed = elapsed_ms(blob_read_started);

    let write_apply_started = Instant::now();
    for rel in sort_paths(write_paths.clone()) {
        let path_id = *target_manifest
            .path_id_by_path
            .get(rel.as_str())
            .ok_or_else(|| format!("Snapshot row is missing `{rel}`."))?;
        let target_row = target_write_rows
            .get(&path_id)
            .ok_or_else(|| format!("Snapshot row is missing `{rel}`."))?;
        let blob_id = target_manifest.row_blob_id(target_row)?;
        let abs_path = workspace_root.join(&rel);
        if path_exists_or_directory_link(&abs_path) {
            remove_path_entry(&abs_path)?;
        }
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let data = blob_bytes_by_id
            .get(blob_id)
            .ok_or_else(|| format!("Blob payload missing for `{blob_id}`."))?;
        fs::write(&abs_path, data).map_err(|err| err.to_string())?;
        let mode = parse_mode_bits(Some(target_row.mode.as_str()))?;
        fs::set_permissions(
            &abs_path,
            fs::Permissions::from_mode(readonly_file_mode(mode)),
        )
        .map_err(|err| err.to_string())?;
    }
    let write_apply_elapsed = elapsed_ms(write_apply_started);
    let fingerprint_started = Instant::now();
    let (content_fingerprint, content_row_count) = main_seed_content_fingerprint(&workspace_root)?;
    let fingerprint_elapsed = elapsed_ms(fingerprint_started);
    let unchanged_count = content_row_count.saturating_sub(write_paths.len());

    Ok(MainSeedMaterializationPlan {
        write_count: write_paths.len(),
        remove_count: remove_paths.len(),
        unchanged_count,
        baseline_integrity_reset,
        content_fingerprint,
        content_row_count,
        phase_timings_ms: json!({
            "snapshot_rules_lookup": snapshot_rules_elapsed_ms,
            "baseline_seed_validation": baseline_validation_elapsed,
            "baseline_seed_validation_changed_count": baseline_integrity_changed_count,
            "baseline_integrity_reset": baseline_integrity_reset,
            "visible_row_count_strategy": visible_row_count_strategy,
            "target_filtered_rows": target_rows_elapsed,
            "changed_path_delta": delta_elapsed,
            "target_row_lookup": target_row_lookup_elapsed,
            "remove_apply": remove_apply_elapsed,
            "blob_read_plan": blob_plan_elapsed,
            "blob_read": blob_read_elapsed,
            "write_apply": write_apply_elapsed,
            "content_fingerprint": fingerprint_elapsed,
            "total": elapsed_ms(total_started),
        }),
    })
}

pub(super) struct RepoFileLock {
    pub(super) file: File,
}

impl RepoFileLock {
    pub(super) fn acquire_blocking(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Repo file lock path has no parent.".to_string())?;
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        let file = OpenOptions::new()
            .create(true)
            // Lock acquisition must not erase another owner's payload.
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|err| err.to_string())?;
        file.lock_exclusive().map_err(|err| err.to_string())?;
        Ok(Self { file })
    }
}

impl Drop for RepoFileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(super) fn copy_seed_tree(
    source_path: &Path,
    target_path: &Path,
    exclude_names: &[&str],
) -> Result<String, String> {
    copy_seed_tree_with_file_copy(
        source_path,
        target_path,
        exclude_names,
        copy_seed_file_platform,
    )
}

pub(super) fn copy_seed_tree_with_file_copy(
    source_path: &Path,
    target_path: &Path,
    exclude_names: &[&str],
    file_copy: fn(&Path, &Path, u32) -> Result<SeedCopyFileStrategy, String>,
) -> Result<String, String> {
    fs::create_dir_all(target_path).map_err(|err| err.to_string())?;
    let mut strategy = SeedCopyStrategy::default();
    copy_seed_tree_recursive(
        source_path,
        target_path,
        exclude_names,
        &mut strategy,
        file_copy,
    )?;
    Ok(strategy.reported_name().to_string())
}

pub(super) fn copy_seed_tree_recursive(
    source_root: &Path,
    target_root: &Path,
    exclude_names: &[&str],
    strategy: &mut SeedCopyStrategy,
    file_copy: fn(&Path, &Path, u32) -> Result<SeedCopyFileStrategy, String>,
) -> Result<(), String> {
    for entry in fs::read_dir(source_root).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if exclude_names.iter().any(|excluded| *excluded == name_text) {
            continue;
        }
        let source_path = entry.path();
        let target_path = target_root.join(&name);
        let metadata = fs::symlink_metadata(&source_path).map_err(|err| err.to_string())?;
        if metadata.file_type().is_symlink() {
            copy_seed_symlink(&source_path, &target_path)?;
            strategy.note(SeedCopyFileStrategy::Symlink);
            continue;
        }
        if metadata.is_dir() {
            fs::create_dir_all(&target_path).map_err(|err| err.to_string())?;
            copy_seed_tree_recursive(
                &source_path,
                &target_path,
                exclude_names,
                strategy,
                file_copy,
            )?;
            continue;
        }
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let file_strategy = file_copy(&source_path, &target_path, metadata.permissions().mode())?;
        strategy.note(file_strategy);
        let permissions = metadata.permissions();
        fs::set_permissions(&target_path, permissions).map_err(|err| err.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SeedCopyFileStrategy {
    Clonefile,
    #[cfg(any(target_os = "linux", test))]
    Reflink,
    Copy2,
    Symlink,
}

#[derive(Default)]
pub(super) struct SeedCopyStrategy {
    pub(super) used_clonefile: bool,
    pub(super) used_reflink: bool,
    pub(super) used_copy2: bool,
    pub(super) used_symlink: bool,
}

impl SeedCopyStrategy {
    fn note(&mut self, strategy: SeedCopyFileStrategy) {
        match strategy {
            SeedCopyFileStrategy::Clonefile => self.used_clonefile = true,
            #[cfg(any(target_os = "linux", test))]
            SeedCopyFileStrategy::Reflink => self.used_reflink = true,
            SeedCopyFileStrategy::Copy2 => self.used_copy2 = true,
            SeedCopyFileStrategy::Symlink => self.used_symlink = true,
        }
    }

    fn reported_name(&self) -> &'static str {
        if self.used_clonefile {
            "clonefile"
        } else if self.used_reflink {
            "reflink"
        } else if self.used_copy2 {
            "copy2"
        } else if self.used_symlink {
            "symlink"
        } else {
            "copy2"
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn clonefile(src: *const c_char, dst: *const c_char, flags: c_int) -> c_int;
}

#[cfg(target_os = "macos")]
pub(super) fn try_clonefile_macos(
    source_path: &Path,
    target_path: &Path,
) -> Result<Option<SeedCopyFileStrategy>, String> {
    let source = CString::new(source_path.as_os_str().as_bytes()).map_err(|_| {
        format!(
            "clonefile source path contains an interior NUL: {}",
            source_path.display()
        )
    })?;
    let target = CString::new(target_path.as_os_str().as_bytes()).map_err(|_| {
        format!(
            "clonefile target path contains an interior NUL: {}",
            target_path.display()
        )
    })?;
    let result = unsafe { clonefile(source.as_ptr(), target.as_ptr(), 0) };
    if result == 0 {
        return Ok(Some(SeedCopyFileStrategy::Clonefile));
    }
    let error = std::io::Error::last_os_error();
    let raw = error.raw_os_error();
    if raw.is_some_and(|code| CLONEFILE_FALLBACK_ERRNOS.contains(&code)) {
        if target_path.exists() {
            let _ = fs::remove_file(target_path);
        }
        return Ok(None);
    }
    Err(format!(
        "clonefile {} -> {} failed: {}",
        source_path.display(),
        target_path.display(),
        error
    ))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn try_clonefile_macos(
    _source_path: &Path,
    _target_path: &Path,
) -> Result<Option<SeedCopyFileStrategy>, String> {
    Ok(None)
}

#[cfg(target_os = "linux")]
pub(super) fn try_reflink_linux(
    source_path: &Path,
    target_path: &Path,
    mode: u32,
) -> Result<Option<SeedCopyFileStrategy>, String> {
    let source = File::open(source_path).map_err(|err| err.to_string())?;
    let target = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode & 0o777)
        .open(target_path)
        .map_err(|err| err.to_string())?;
    let result = unsafe { libc::ioctl(target.as_raw_fd(), LINUX_FICLONE, source.as_raw_fd()) };
    if result == 0 {
        return Ok(Some(SeedCopyFileStrategy::Reflink));
    }
    let error = std::io::Error::last_os_error();
    let raw = error.raw_os_error();
    drop(target);
    if target_path.exists() {
        let _ = fs::remove_file(target_path);
    }
    if raw.is_some_and(|code| REFLINK_FALLBACK_ERRNOS.contains(&code)) {
        return Ok(None);
    }
    Err(format!(
        "reflink {} -> {} failed: {}",
        source_path.display(),
        target_path.display(),
        error
    ))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn try_reflink_linux(
    _source_path: &Path,
    _target_path: &Path,
    _mode: u32,
) -> Result<Option<SeedCopyFileStrategy>, String> {
    Ok(None)
}

pub(super) fn copy_seed_file_platform(
    source_path: &Path,
    target_path: &Path,
    mode: u32,
) -> Result<SeedCopyFileStrategy, String> {
    if let Some(strategy) = try_clonefile_macos(source_path, target_path)? {
        return Ok(strategy);
    }
    if let Some(strategy) = try_reflink_linux(source_path, target_path, mode)? {
        return Ok(strategy);
    }
    fs::copy(source_path, target_path).map_err(|err| err.to_string())?;
    Ok(SeedCopyFileStrategy::Copy2)
}

pub(super) fn copy_seed_symlink(source_path: &Path, target_path: &Path) -> Result<(), String> {
    let target = fs::read_link(source_path).map_err(|err| err.to_string())?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target, target_path).map_err(|err| err.to_string())
    }
    #[cfg(not(unix))]
    {
        if source_path
            .metadata()
            .map_err(|err| err.to_string())?
            .is_dir()
        {
            std::os::windows::fs::symlink_dir(&target, target_path).map_err(|err| err.to_string())
        } else {
            std::os::windows::fs::symlink_file(&target, target_path).map_err(|err| err.to_string())
        }
    }
}

pub(super) fn readonly_file_mode(mode: u32) -> u32 {
    mode & !0o222
}

pub(super) fn set_tree_directories_writeable(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(|err| err.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        set_tree_directories_writeable(&child)?;
    }
    let metadata = fs::metadata(path).map_err(|err| err.to_string())?;
    let mut permissions = metadata.permissions();
    let mode = permissions.mode();
    permissions.set_mode(mode | 0o700);
    fs::set_permissions(path, permissions).map_err(|err| err.to_string())
}

pub(super) fn set_tree_directories_readonly(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(|err| err.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        set_tree_directories_readonly(&child)?;
    }
    let metadata = fs::metadata(path).map_err(|err| err.to_string())?;
    let mut permissions = metadata.permissions();
    let mode = permissions.mode();
    permissions.set_mode((mode | 0o555) & !0o222);
    fs::set_permissions(path, permissions).map_err(|err| err.to_string())
}

pub(super) fn set_tree_readonly(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(|err| err.to_string())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            set_tree_readonly(&child)?;
        }
        let mut permissions = metadata.permissions();
        let mode = permissions.mode();
        permissions.set_mode(if metadata.is_dir() {
            (mode | 0o555) & !0o222
        } else {
            readonly_file_mode(mode)
        });
        fs::set_permissions(&child, permissions).map_err(|err| err.to_string())?;
    }
    let metadata = fs::metadata(path).map_err(|err| err.to_string())?;
    let mut permissions = metadata.permissions();
    let mode = permissions.mode();
    permissions.set_mode((mode | 0o555) & !0o222);
    fs::set_permissions(path, permissions).map_err(|err| err.to_string())
}

pub(super) fn set_tree_writeable(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(|err| err.to_string())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            set_tree_writeable(&child)?;
        }
        let mut permissions = metadata.permissions();
        let mode = permissions.mode();
        permissions.set_mode(mode | 0o200);
        fs::set_permissions(&child, permissions).map_err(|err| err.to_string())?;
    }
    let metadata = fs::metadata(path).map_err(|err| err.to_string())?;
    let mut permissions = metadata.permissions();
    let mode = permissions.mode();
    permissions.set_mode(mode | 0o700);
    fs::set_permissions(path, permissions).map_err(|err| err.to_string())
}

pub(super) fn remove_tree_force(path: &Path) -> Result<(), String> {
    if !path_exists_or_directory_link(path) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path).map_err(|err| err.to_string())?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return remove_path_entry(path);
    }
    set_tree_writeable(path)?;
    fs::remove_dir_all(path).map_err(|err| err.to_string())
}

pub(super) fn worktree_registry_path(repo: &RepoRuntime, worktree_name: &str) -> PathBuf {
    repo.authoritative_repo_root()
        .join(".ait")
        .join("worktrees")
        .join(format!("{worktree_name}.json"))
}

pub(super) fn update_root_config(
    repo: &RepoRuntime,
    updater: impl FnOnce(&mut JsonMap<String, JsonValue>),
) -> Result<(), String> {
    let config_path = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("config.json");
    let mut config = read_json_object_value(&config_path);
    updater(&mut config);
    write_json_pretty(&config_path, &JsonValue::Object(config))
}

pub(super) fn worktree_name_registered(repo: &RepoRuntime, worktree_name: &str) -> bool {
    worktree_registry_path(repo, worktree_name).exists()
}

pub(super) fn resolve_next_task_worktree_name(
    repo: &RepoRuntime,
    task_id: &str,
) -> Result<String, String> {
    let base = task_bound_worktree_name(task_id)?;
    let mut suffix = 1_i64;
    loop {
        let candidate = if suffix == 1 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        if !worktree_name_registered(repo, &candidate) {
            return Ok(candidate);
        }
        suffix += 1;
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct CurrentWorktreeMetadata {
    pub(super) name: String,
    pub(super) bound_task_id: Option<String>,
    pub(super) bound_change_id: Option<String>,
    pub(super) bound_change_ref: Option<String>,
    pub(super) auto_created_for_task: bool,
    pub(super) created_at: Option<String>,
    pub(super) fork_snapshot_id: Option<String>,
    pub(super) target_base_line: Option<String>,
    pub(super) rebase_state: String,
    pub(super) rebase_conflict_paths: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct WorkspaceFileState {
    pub(super) sha256: String,
    pub(super) mode: String,
}

#[derive(Clone, Debug)]
pub(super) struct StatusBaselineManifest {
    pub(super) index: SnapshotTreeManifestIndex,
    pub(super) source: String,
    pub(super) manifest_path: Option<PathBuf>,
    pub(super) root_tree_id: Option<String>,
    pub(super) hash_cache: Option<WorkspaceHashCacheLoad>,
}

#[derive(Clone, Debug)]
pub(super) struct WorkspaceStatusScan {
    pub(super) files: BTreeMap<String, WorkspaceFileState>,
    pub(super) tracked_fingerprints: BTreeMap<String, WorkspaceFileFingerprint>,
    pub(super) operational_external_roots: Vec<String>,
    pub(super) reused_paths: usize,
    pub(super) rehashed_paths: usize,
    pub(super) cache_read: String,
}
