use super::*;
use ait_core::line_store::LineStore;
use ait_core::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotOperationStore, LocalSnapshotWriteStore,
};
use ait_core::snapshot_merge::{merge_utf8_text_bytes, TextMergeOutcome};

const LINE_MERGE_CONTRACT: &str = "line-merge/v1";
const MERGE_MARKER_PREFIX: &str = "<<<<<<< AIT target:";
const MERGE_MARKER_SUFFIX: &str = ">>>>>>> AIT source:";

#[derive(Clone, Debug)]
enum MergePathAction {
    Keep,
    Remove,
    WriteRow(SnapshotFileRow),
    WriteBytes { bytes: Vec<u8>, mode: String },
    Conflict { kind: String },
}

#[derive(Clone, Debug)]
struct MergePathPlan {
    path: String,
    base: Option<SnapshotFileRow>,
    target: Option<SnapshotFileRow>,
    source: Option<SnapshotFileRow>,
    action: MergePathAction,
}

#[derive(Clone, Debug)]
struct LineMergeTreePlan {
    files: Vec<MergePathPlan>,
    conflict_paths: Vec<String>,
    conflict_kinds: BTreeMap<String, String>,
}

pub fn line_merge(
    repo: &RepoRuntime,
    source_line_name: Option<&str>,
    message: Option<&str>,
    continue_merge: bool,
    abort_merge: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli line merge", || {
        match (continue_merge, abort_merge) {
            (true, true) => Err("--continue and --abort cannot be used together.".to_string()),
            (true, false) => continue_line_merge_unlocked(repo, message),
            (false, true) => {
                if message.is_some() {
                    return Err("--message cannot be used with --abort.".to_string());
                }
                abort_line_merge_unlocked(repo)
            }
            (false, false) => {
                let source_line_name = normalized_text(source_line_name).ok_or_else(|| {
                    "A source line is required unless --continue or --abort is used.".to_string()
                })?;
                start_line_merge_unlocked(repo, &source_line_name, message)
            }
        }
    })
}

fn start_line_merge_unlocked(
    repo: &RepoRuntime,
    source_line_name: &str,
    message: Option<&str>,
) -> Result<JsonValue, String> {
    if source_line_name == repo.current_line_name()? {
        return Err(format!("Cannot merge line {source_line_name} into itself."));
    }
    let source_line = local_line_row(repo, source_line_name)?;
    require_active_line(&source_line, source_line_name)?;
    let source_snapshot_id = required_line_head(&source_line, source_line_name)?;
    start_line_merge_from_snapshot_unlocked(
        repo,
        source_line_name,
        &source_snapshot_id,
        Some(source_line_name),
        None,
        message,
    )
}

pub(in crate::primitives) fn start_line_merge_from_snapshot_unlocked(
    repo: &RepoRuntime,
    source_label: &str,
    source_snapshot_id: &str,
    source_verification_line: Option<&str>,
    target_line_name: Option<&str>,
    message: Option<&str>,
) -> Result<JsonValue, String> {
    guard_current_worktree_task_bound_authoring(repo, "line merge")?;
    guard_no_planning_only_artifact_drift(repo, "ait line merge")?;
    let current_line_name = repo.current_line_name()?;
    let target_line_name =
        normalized_text(target_line_name).unwrap_or_else(|| current_line_name.clone());
    if target_line_name != current_line_name {
        return Err(format!(
            "Merge target {target_line_name} is not the current line {current_line_name}. Switch with `ait line switch {target_line_name} --restore` before merging."
        ));
    }
    if source_verification_line == Some(target_line_name.as_str()) {
        return Err(format!("Cannot merge line {source_label} into itself."));
    }
    let worktree_name = require_merge_worktree(repo)?;
    let metadata = worktree_metadata_with_defaults(&load_worktree_metadata(repo, &worktree_name)?);
    guard_idle_merge_and_rebase_state(&metadata, &worktree_name)?;

    let target_line = local_line_row(repo, &target_line_name)?;
    require_active_line(&target_line, &target_line_name)?;
    let target_snapshot_id = required_line_head(&target_line, &target_line_name)?;
    require_clean_merge_workspace(repo, &target_snapshot_id)?;
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;

    if target_snapshot_id == source_snapshot_id {
        return Ok(line_merge_result(
            "already_equal",
            &target_line_name,
            source_label,
            &target_snapshot_id,
            source_snapshot_id,
            &[],
            None,
            &[],
            &BTreeMap::new(),
        ));
    }
    if snapshot_is_ancestor(
        &store,
        &target_snapshot_id,
        source_snapshot_id,
        SnapshotDagLimits::default(),
    )?
    .is_some()
    {
        restore_workspace_all(
            repo,
            Some(source_snapshot_id),
            Some(&target_snapshot_id),
            false,
            false,
        )?;
        if let Err(error) = store.compare_and_swap_line_head(
            &target_line_name,
            Some(&target_snapshot_id),
            Some(source_snapshot_id),
            &system_event_timestamp(),
        ) {
            let _ = restore_workspace_all(
                repo,
                Some(&target_snapshot_id),
                Some(source_snapshot_id),
                true,
                false,
            );
            return Err(error);
        }
        repo.set_worktree_materialized_snapshot(Some(source_snapshot_id))?;
        return Ok(line_merge_result(
            "fast_forward",
            &target_line_name,
            source_label,
            &target_snapshot_id,
            source_snapshot_id,
            std::slice::from_ref(&target_snapshot_id),
            None,
            &[],
            &BTreeMap::new(),
        ));
    }
    if snapshot_is_ancestor(
        &store,
        source_snapshot_id,
        &target_snapshot_id,
        SnapshotDagLimits::default(),
    )?
    .is_some()
    {
        return Ok(line_merge_result(
            "already_contains_source",
            &target_line_name,
            source_label,
            &target_snapshot_id,
            source_snapshot_id,
            &[source_snapshot_id.to_string()],
            None,
            &[],
            &BTreeMap::new(),
        ));
    }

    let merge_base_snapshot_ids = snapshot_merge_bases(
        &store,
        &target_snapshot_id,
        source_snapshot_id,
        SnapshotDagLimits::default(),
    )?;
    if merge_base_snapshot_ids.is_empty() {
        return Err(format!(
            "Line {target_line_name} and source {source_label} have unrelated Snapshot histories; an ordinary merge requires at least one common base."
        ));
    }
    let plan = compute_line_merge_tree_plan(
        &store,
        &merge_base_snapshot_ids,
        &target_snapshot_id,
        source_snapshot_id,
    )?;
    let resolved_message = normalized_text(message)
        .unwrap_or_else(|| format!("Merge {source_label} into {target_line_name}"));
    save_line_merge_state(
        repo,
        &worktree_name,
        "applying",
        &target_line_name,
        source_label,
        source_verification_line,
        &target_snapshot_id,
        source_snapshot_id,
        &merge_base_snapshot_ids,
        &resolved_message,
        &plan,
    )?;
    if let Err(error) = apply_line_merge_plan(
        repo,
        &plan,
        &target_line_name,
        source_label,
        &target_snapshot_id,
        source_snapshot_id,
    ) {
        let _ = restore_workspace_all(
            repo,
            Some(&target_snapshot_id),
            Some(&target_snapshot_id),
            true,
            false,
        );
        let _ = clear_line_merge_state(repo, &worktree_name);
        return Err(error);
    }
    if !plan.conflict_paths.is_empty() {
        save_line_merge_state(
            repo,
            &worktree_name,
            "conflicted",
            &target_line_name,
            source_label,
            source_verification_line,
            &target_snapshot_id,
            source_snapshot_id,
            &merge_base_snapshot_ids,
            &resolved_message,
            &plan,
        )?;
        return Ok(line_merge_result(
            "conflicted",
            &target_line_name,
            source_label,
            &target_snapshot_id,
            source_snapshot_id,
            &merge_base_snapshot_ids,
            None,
            &plan.conflict_paths,
            &plan.conflict_kinds,
        ));
    }

    let parent_snapshot_ids = vec![target_snapshot_id.clone(), source_snapshot_id.to_string()];
    let snapshot = match store.create_snapshot_with_parents(
        &repo.repo_name(),
        &target_line_name,
        &parent_snapshot_ids,
        Some(&resolved_message),
        repo.is_worktree(),
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = restore_workspace_all(
                repo,
                Some(&target_snapshot_id),
                Some(&target_snapshot_id),
                true,
                false,
            );
            let _ = clear_line_merge_state(repo, &worktree_name);
            return Err(error);
        }
    };
    let merge_snapshot_id = required_string_field(&snapshot, "snapshot_id")?;
    repo.set_worktree_materialized_snapshot(Some(&merge_snapshot_id))?;
    clear_line_merge_state(repo, &worktree_name)?;
    Ok(line_merge_result(
        "merged",
        &target_line_name,
        source_label,
        &target_snapshot_id,
        source_snapshot_id,
        &merge_base_snapshot_ids,
        Some(&merge_snapshot_id),
        &[],
        &BTreeMap::new(),
    ))
}

fn continue_line_merge_unlocked(
    repo: &RepoRuntime,
    message: Option<&str>,
) -> Result<JsonValue, String> {
    guard_current_worktree_task_bound_authoring(repo, "line merge --continue")?;
    guard_no_planning_only_artifact_drift(repo, "ait line merge --continue")?;
    let worktree_name = require_merge_worktree(repo)?;
    let metadata = load_worktree_metadata(repo, &worktree_name)?;
    if metadata_string(&metadata, "merge_state").as_deref() != Some("conflicted") {
        return Err(format!(
            "Worktree {worktree_name} has no conflicted line merge to continue."
        ));
    }
    let target_line_name = required_metadata_string(&metadata, "merge_target_line")?;
    let source_line_name = required_metadata_string(&metadata, "merge_source_line")?;
    let source_verification_line = metadata_string(&metadata, "merge_source_verification_line");
    let target_snapshot_id = required_metadata_string(&metadata, "merge_target_snapshot_id")?;
    let source_snapshot_id = required_metadata_string(&metadata, "merge_source_snapshot_id")?;
    let merge_base_snapshot_ids = metadata_string_list(&metadata, "merge_base_snapshot_ids");
    let conflict_paths = metadata_string_list(&metadata, "merge_conflict_paths");
    let conflict_kinds = metadata_string_map(&metadata, "merge_conflict_kinds");
    if repo.current_line_name()? != target_line_name {
        return Err(format!(
            "Conflicted merge targets line {target_line_name}; switch back to that line before continuing."
        ));
    }
    verify_merge_parent_heads(
        repo,
        &target_line_name,
        source_verification_line.as_deref(),
        &target_snapshot_id,
        &source_snapshot_id,
    )?;
    let unresolved = conflict_paths
        .iter()
        .filter(|path| workspace_path_has_merge_marker(repo, path))
        .cloned()
        .collect::<Vec<_>>();
    if !unresolved.is_empty() {
        return Err(format!(
            "Merge conflicts remain unresolved: {}. Remove every AIT merge marker after choosing the intended content, then rerun `ait line merge --continue`.",
            summarize_path_sample(&unresolved)
        ));
    }
    let resolved_message = normalized_text(message)
        .or_else(|| metadata_string(&metadata, "merge_message"))
        .unwrap_or_else(|| format!("Merge line {source_line_name} into {target_line_name}"));
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let parents = vec![target_snapshot_id.clone(), source_snapshot_id.clone()];
    let snapshot = store.create_snapshot_with_parents(
        &repo.repo_name(),
        &target_line_name,
        &parents,
        Some(&resolved_message),
        repo.is_worktree(),
    )?;
    let merge_snapshot_id = required_string_field(&snapshot, "snapshot_id")?;
    repo.set_worktree_materialized_snapshot(Some(&merge_snapshot_id))?;
    clear_line_merge_state(repo, &worktree_name)?;
    Ok(line_merge_result(
        "continued",
        &target_line_name,
        &source_line_name,
        &target_snapshot_id,
        &source_snapshot_id,
        &merge_base_snapshot_ids,
        Some(&merge_snapshot_id),
        &conflict_paths,
        &conflict_kinds,
    ))
}

fn abort_line_merge_unlocked(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let worktree_name = require_merge_worktree(repo)?;
    let metadata = load_worktree_metadata(repo, &worktree_name)?;
    let merge_state = metadata_string(&metadata, "merge_state").unwrap_or_else(|| "idle".into());
    if !matches!(merge_state.as_str(), "conflicted" | "applying") {
        return Err(format!(
            "Worktree {worktree_name} has no active line merge to abort."
        ));
    }
    let target_line_name = required_metadata_string(&metadata, "merge_target_line")?;
    let source_line_name = required_metadata_string(&metadata, "merge_source_line")?;
    let target_snapshot_id = required_metadata_string(&metadata, "merge_target_snapshot_id")?;
    let source_snapshot_id = required_metadata_string(&metadata, "merge_source_snapshot_id")?;
    if repo.current_line_name()? != target_line_name {
        return Err(format!(
            "Active merge targets line {target_line_name}; switch back before aborting."
        ));
    }
    let current_target_head = local_line_head_snapshot_id(repo, &target_line_name)?;
    if current_target_head.as_deref() != Some(target_snapshot_id.as_str()) {
        return Err(format!(
            "Cannot abort merge because target line {target_line_name} moved from {target_snapshot_id} to {}.",
            current_target_head.as_deref().unwrap_or("none")
        ));
    }
    restore_workspace_all(
        repo,
        Some(&target_snapshot_id),
        Some(&target_snapshot_id),
        true,
        false,
    )?;
    repo.set_worktree_materialized_snapshot(Some(&target_snapshot_id))?;
    clear_line_merge_state(repo, &worktree_name)?;
    Ok(line_merge_result(
        "aborted",
        &target_line_name,
        &source_line_name,
        &target_snapshot_id,
        &source_snapshot_id,
        &metadata_string_list(&metadata, "merge_base_snapshot_ids"),
        None,
        &metadata_string_list(&metadata, "merge_conflict_paths"),
        &metadata_string_map(&metadata, "merge_conflict_kinds"),
    ))
}

fn compute_line_merge_tree_plan<S>(
    store: &S,
    merge_base_snapshot_ids: &[String],
    target_snapshot_id: &str,
    source_snapshot_id: &str,
) -> Result<LineMergeTreePlan, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    let file_map = |snapshot_id: &str| {
        store
            .snapshot_tree_file_rows(Some(snapshot_id))
            .map(|rows| {
                rows.into_iter()
                    .map(|row| (row.path.clone(), row))
                    .collect::<BTreeMap<_, _>>()
            })
    };
    let base_maps = merge_base_snapshot_ids
        .iter()
        .map(|snapshot_id| file_map(snapshot_id))
        .collect::<Result<Vec<_>, _>>()?;
    let target_files = file_map(target_snapshot_id)?;
    let source_files = file_map(source_snapshot_id)?;
    let (base_files, ambiguous_base_paths) = agreed_merge_base_files(&base_maps);
    let rename_conflicts = classify_rename_conflicts(&base_files, &target_files, &source_files);

    let all_paths = base_files
        .keys()
        .chain(target_files.keys())
        .chain(source_files.keys())
        .chain(rename_conflicts.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut files = Vec::new();
    let mut conflict_paths = Vec::new();
    let mut conflict_kinds = BTreeMap::new();
    for path in all_paths {
        let base = base_files.get(&path).cloned().flatten();
        let target = target_files.get(&path).cloned();
        let source = source_files.get(&path).cloned();
        let action = if let Some(kind) = rename_conflicts.get(&path) {
            MergePathAction::Conflict { kind: kind.clone() }
        } else if ambiguous_base_paths.contains(&path) && target != source {
            MergePathAction::Conflict {
                kind: "multiple_merge_base".to_string(),
            }
        } else {
            classify_merge_path(store, base.as_ref(), target.as_ref(), source.as_ref())?
        };
        if let MergePathAction::Conflict { kind } = &action {
            conflict_paths.push(path.clone());
            conflict_kinds.insert(path.clone(), kind.clone());
        }
        files.push(MergePathPlan {
            path,
            base,
            target,
            source,
            action,
        });
    }
    Ok(LineMergeTreePlan {
        files,
        conflict_paths,
        conflict_kinds,
    })
}

fn classify_rename_conflicts(
    base_files: &BTreeMap<String, Option<SnapshotFileRow>>,
    target_files: &BTreeMap<String, SnapshotFileRow>,
    source_files: &BTreeMap<String, SnapshotFileRow>,
) -> BTreeMap<String, String> {
    let target_renames = exact_renames(base_files, target_files);
    let source_renames = exact_renames(base_files, source_files);
    let mut rename_conflicts = BTreeMap::new();
    for (old_path, target_new_path) in &target_renames {
        if let Some(source_new_path) = source_renames.get(old_path) {
            if target_new_path != source_new_path {
                rename_conflicts.insert(old_path.clone(), "rename_rename".to_string());
            }
        } else if source_files
            .get(old_path)
            .is_some_and(|source| base_files.get(old_path).and_then(Option::as_ref) != Some(source))
        {
            rename_conflicts.insert(old_path.clone(), "rename_modify".to_string());
        }
    }
    for old_path in source_renames.keys() {
        if !target_renames.contains_key(old_path)
            && target_files.get(old_path).is_some_and(|target| {
                base_files.get(old_path).and_then(Option::as_ref) != Some(target)
            })
        {
            rename_conflicts.insert(old_path.clone(), "modify_rename".to_string());
        }
    }
    rename_conflicts
}

fn classify_merge_path<S>(
    store: &S,
    base: Option<&SnapshotFileRow>,
    target: Option<&SnapshotFileRow>,
    source: Option<&SnapshotFileRow>,
) -> Result<MergePathAction, String>
where
    S: LocalSnapshotBlobReadStore + ?Sized,
{
    if target == source {
        return Ok(MergePathAction::Keep);
    }
    if target == base {
        return Ok(match source {
            Some(source) => MergePathAction::WriteRow(source.clone()),
            None => MergePathAction::Remove,
        });
    }
    if source == base {
        return Ok(MergePathAction::Keep);
    }
    let (Some(base), Some(target), Some(source)) = (base, target, source) else {
        let kind = match (base.is_some(), target.is_some(), source.is_some()) {
            (false, true, true) => "add_add",
            (true, false, true) => "delete_modify",
            (true, true, false) => "modify_delete",
            _ => "path_type",
        };
        return Ok(MergePathAction::Conflict {
            kind: kind.to_string(),
        });
    };
    if [base, target, source]
        .iter()
        .any(|row| snapshot_mode_is_symlink(&row.mode))
    {
        return Ok(MergePathAction::Conflict {
            kind: "symlink".to_string(),
        });
    }

    let target_content_changed = target.blob_id != base.blob_id;
    let source_content_changed = source.blob_id != base.blob_id;
    let target_mode_changed = target.mode != base.mode;
    let source_mode_changed = source.mode != base.mode;
    if target_content_changed
        && !target_mode_changed
        && !source_content_changed
        && source_mode_changed
    {
        let mut merged = target.clone();
        merged.mode = source.mode.clone();
        return Ok(MergePathAction::WriteRow(merged));
    }
    if source_content_changed
        && !source_mode_changed
        && !target_content_changed
        && target_mode_changed
    {
        let mut merged = source.clone();
        merged.mode = target.mode.clone();
        return Ok(MergePathAction::WriteRow(merged));
    }
    if target.mode != source.mode {
        return Ok(MergePathAction::Conflict {
            kind: "mode".to_string(),
        });
    }

    let base_bytes = store.read_blob_bytes(&base.blob_id)?;
    let target_bytes = store.read_blob_bytes(&target.blob_id)?;
    let source_bytes = store.read_blob_bytes(&source.blob_id)?;
    Ok(
        match merge_utf8_text_bytes(&base_bytes, &target_bytes, &source_bytes) {
            TextMergeOutcome::Merged(bytes) => MergePathAction::WriteBytes {
                bytes,
                mode: target.mode.clone(),
            },
            TextMergeOutcome::Conflict => MergePathAction::Conflict {
                kind: "text".to_string(),
            },
            TextMergeOutcome::NonText => MergePathAction::Conflict {
                kind: "binary".to_string(),
            },
        },
    )
}

fn agreed_merge_base_files(
    base_maps: &[BTreeMap<String, SnapshotFileRow>],
) -> (BTreeMap<String, Option<SnapshotFileRow>>, BTreeSet<String>) {
    let all_paths = base_maps
        .iter()
        .flat_map(|files| files.keys().cloned())
        .collect::<BTreeSet<_>>();
    let mut agreed = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for path in all_paths {
        let first = base_maps
            .first()
            .and_then(|files| files.get(&path))
            .cloned();
        if base_maps
            .iter()
            .all(|files| files.get(&path) == first.as_ref())
        {
            agreed.insert(path, first);
        } else {
            ambiguous.insert(path.clone());
            agreed.insert(path, first);
        }
    }
    (agreed, ambiguous)
}

fn exact_renames(
    base_files: &BTreeMap<String, Option<SnapshotFileRow>>,
    side_files: &BTreeMap<String, SnapshotFileRow>,
) -> BTreeMap<String, String> {
    let added = side_files
        .iter()
        .filter(|(path, _)| !base_files.get(*path).is_some_and(Option::is_some))
        .collect::<Vec<_>>();
    let mut renames = BTreeMap::new();
    for (old_path, base) in base_files {
        let Some(base) = base else {
            continue;
        };
        if side_files.contains_key(old_path) {
            continue;
        }
        let candidates = added
            .iter()
            .filter(|(_, candidate)| same_file_identity(base, candidate))
            .map(|(path, _)| (*path).clone())
            .collect::<Vec<_>>();
        if candidates.len() == 1 {
            renames.insert(old_path.clone(), candidates[0].clone());
        }
    }
    renames
}

fn same_file_identity(left: &SnapshotFileRow, right: &SnapshotFileRow) -> bool {
    left.blob_id == right.blob_id
        && left.sha256 == right.sha256
        && left.size_bytes == right.size_bytes
        && left.mode == right.mode
}

fn snapshot_mode_bits(mode: &str) -> Option<u32> {
    let value = mode.trim();
    let octal = value
        .strip_prefix("0o")
        .or_else(|| value.strip_prefix("0O"))
        .unwrap_or(value);
    u32::from_str_radix(octal, 8).ok()
}

fn snapshot_mode_is_symlink(mode: &str) -> bool {
    snapshot_mode_bits(mode).is_some_and(|bits| bits & 0o170000 == 0o120000)
}

fn apply_line_merge_plan(
    repo: &RepoRuntime,
    plan: &LineMergeTreePlan,
    target_line_name: &str,
    source_line_name: &str,
    target_snapshot_id: &str,
    source_snapshot_id: &str,
) -> Result<(), String> {
    for file in &plan.files {
        match &file.action {
            MergePathAction::Keep => {}
            MergePathAction::Remove => write_workspace_snapshot_row(repo, &file.path, None)?,
            MergePathAction::WriteRow(row) => {
                let row = snapshot_file_row_json(row);
                write_workspace_snapshot_row(repo, &file.path, Some(&row))?;
            }
            MergePathAction::WriteBytes { bytes, mode } => {
                write_workspace_bytes(repo, &file.path, bytes, mode)?;
            }
            MergePathAction::Conflict { kind } => {
                let marker = render_line_merge_conflict_marker(
                    repo,
                    file,
                    kind,
                    target_line_name,
                    source_line_name,
                    target_snapshot_id,
                    source_snapshot_id,
                )?;
                write_workspace_bytes(repo, &file.path, marker.as_bytes(), "0o644")?;
            }
        }
    }
    Ok(())
}

fn render_line_merge_conflict_marker(
    repo: &RepoRuntime,
    file: &MergePathPlan,
    kind: &str,
    target_line_name: &str,
    source_line_name: &str,
    target_snapshot_id: &str,
    source_snapshot_id: &str,
) -> Result<String, String> {
    if kind == "text" {
        let base = read_worktree_snapshot_blob_bytes(
            repo,
            file.base.as_ref().map(snapshot_file_row_json).as_ref(),
        )?;
        let target = read_worktree_snapshot_blob_bytes(
            repo,
            file.target.as_ref().map(snapshot_file_row_json).as_ref(),
        )?;
        let source = read_worktree_snapshot_blob_bytes(
            repo,
            file.source.as_ref().map(snapshot_file_row_json).as_ref(),
        )?;
        if let (Ok(base), Ok(target), Ok(source)) = (
            String::from_utf8(base),
            String::from_utf8(target),
            String::from_utf8(source),
        ) {
            return Ok(format!(
                "{MERGE_MARKER_PREFIX}{target_line_name}@{target_snapshot_id}\n{target}||||||| AIT base\n{base}=======\n{source}{MERGE_MARKER_SUFFIX}{source_line_name}@{source_snapshot_id}\n"
            ));
        }
    }
    Ok(format!(
        "{MERGE_MARKER_PREFIX}{target_line_name}@{target_snapshot_id}\nAIT merge conflict: {kind}\npath: {}\ntarget_blob: {}\n||||||| AIT base\nbase_blob: {}\n=======\nsource_blob: {}\n{MERGE_MARKER_SUFFIX}{source_line_name}@{source_snapshot_id}\n",
        file.path,
        file.target.as_ref().map(|row| row.blob_id.as_str()).unwrap_or("deleted"),
        file.base.as_ref().map(|row| row.blob_id.as_str()).unwrap_or("absent_or_ambiguous"),
        file.source.as_ref().map(|row| row.blob_id.as_str()).unwrap_or("deleted"),
    ))
}

fn write_workspace_bytes(
    repo: &RepoRuntime,
    path: &str,
    bytes: &[u8],
    mode: &str,
) -> Result<(), String> {
    let abs_path = repo.workspace_root().join(path);
    if let Ok(metadata) = fs::symlink_metadata(&abs_path) {
        if metadata.file_type().is_symlink() || metadata.is_dir() {
            if metadata.is_dir() {
                return Err(format!("Cannot write merge result over directory: {path}"));
            }
            fs::remove_file(&abs_path).map_err(|error| error.to_string())?;
        }
    }
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(&abs_path, bytes).map_err(|error| error.to_string())?;
    let mode = snapshot_mode_bits(mode)
        .ok_or_else(|| format!("Invalid Snapshot mode {mode:?} for {path}."))?;
    set_portable_mode(&abs_path, mode & 0o777).map_err(|error| error.to_string())
}

#[expect(
    clippy::too_many_arguments,
    reason = "merge state fields map directly to the resumable persisted record"
)]
fn save_line_merge_state(
    repo: &RepoRuntime,
    worktree_name: &str,
    state: &str,
    target_line_name: &str,
    source_line_name: &str,
    source_verification_line: Option<&str>,
    target_snapshot_id: &str,
    source_snapshot_id: &str,
    merge_base_snapshot_ids: &[String],
    message: &str,
    plan: &LineMergeTreePlan,
) -> Result<(), String> {
    let mut metadata = load_worktree_metadata(repo, worktree_name)?;
    metadata.insert(
        "merge_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    metadata.insert(
        "merge_started_at".to_string(),
        JsonValue::String(system_event_timestamp()),
    );
    for (key, value) in [
        ("merge_target_line", target_line_name),
        ("merge_source_line", source_line_name),
        ("merge_target_snapshot_id", target_snapshot_id),
        ("merge_source_snapshot_id", source_snapshot_id),
        ("merge_pre_workspace_snapshot_id", target_snapshot_id),
        ("merge_message", message),
    ] {
        metadata.insert(key.to_string(), JsonValue::String(value.to_string()));
    }
    match source_verification_line {
        Some(line_name) => {
            metadata.insert(
                "merge_source_verification_line".to_string(),
                JsonValue::String(line_name.to_string()),
            );
        }
        None => {
            metadata.remove("merge_source_verification_line");
        }
    }
    metadata.insert(
        "merge_base_snapshot_ids".to_string(),
        json_string_array(merge_base_snapshot_ids),
    );
    metadata.insert(
        "merge_conflict_paths".to_string(),
        json_string_array(&plan.conflict_paths),
    );
    metadata.insert(
        "merge_conflict_kinds".to_string(),
        JsonValue::Object(
            plan.conflict_kinds
                .iter()
                .map(|(path, kind)| (path.clone(), JsonValue::String(kind.clone())))
                .collect(),
        ),
    );
    metadata.insert(
        "merge_plan".to_string(),
        JsonValue::Array(plan.files.iter().map(merge_path_plan_json).collect()),
    );
    save_worktree_metadata(repo, worktree_name, &metadata)
}

fn clear_line_merge_state(repo: &RepoRuntime, worktree_name: &str) -> Result<(), String> {
    let mut metadata = load_worktree_metadata(repo, worktree_name)?;
    for key in [
        "merge_started_at",
        "merge_target_line",
        "merge_source_line",
        "merge_source_verification_line",
        "merge_target_snapshot_id",
        "merge_source_snapshot_id",
        "merge_pre_workspace_snapshot_id",
        "merge_message",
        "merge_base_snapshot_ids",
        "merge_conflict_paths",
        "merge_conflict_kinds",
        "merge_plan",
    ] {
        metadata.remove(key);
    }
    metadata.insert(
        "merge_state".to_string(),
        JsonValue::String("idle".to_string()),
    );
    save_worktree_metadata(repo, worktree_name, &metadata)
}

#[expect(
    clippy::too_many_arguments,
    reason = "arguments map directly to the stable line merge result payload"
)]
fn line_merge_result(
    status: &str,
    target_line_name: &str,
    source_line_name: &str,
    target_snapshot_id: &str,
    source_snapshot_id: &str,
    merge_base_snapshot_ids: &[String],
    merge_snapshot_id: Option<&str>,
    conflict_paths: &[String],
    conflict_kinds: &BTreeMap<String, String>,
) -> JsonValue {
    json!({
        "contract": LINE_MERGE_CONTRACT,
        "status": status,
        "target_line_name": target_line_name,
        "source_line_name": source_line_name,
        "target_snapshot_id": target_snapshot_id,
        "source_snapshot_id": source_snapshot_id,
        "merge_base_snapshot_ids": merge_base_snapshot_ids,
        "merge_snapshot_id": merge_snapshot_id,
        "merge_snapshot_created": merge_snapshot_id.is_some(),
        "parent_snapshot_ids": merge_snapshot_id.map(|_| vec![target_snapshot_id, source_snapshot_id]).unwrap_or_default(),
        "conflict_count": conflict_paths.len(),
        "conflict_paths": conflict_paths,
        "conflict_kinds": conflict_kinds,
    })
}

fn merge_path_plan_json(plan: &MergePathPlan) -> JsonValue {
    let (status, kind) = match &plan.action {
        MergePathAction::Keep => ("keep", None),
        MergePathAction::Remove => ("remove", None),
        MergePathAction::WriteRow(_) => ("write", None),
        MergePathAction::WriteBytes { .. } => ("write_merged_text", None),
        MergePathAction::Conflict { kind } => ("conflict", Some(kind.as_str())),
    };
    json!({
        "path": plan.path,
        "status": status,
        "conflict_kind": kind,
        "base": plan.base.as_ref().map(snapshot_file_row_json),
        "target": plan.target.as_ref().map(snapshot_file_row_json),
        "source": plan.source.as_ref().map(snapshot_file_row_json),
    })
}

fn snapshot_file_row_json(row: &SnapshotFileRow) -> JsonValue {
    json!({
        "path": row.path,
        "blob_id": row.blob_id,
        "size_bytes": row.size_bytes,
        "mode": row.mode,
        "sha256": row.sha256,
    })
}

fn require_merge_worktree(repo: &RepoRuntime) -> Result<String, String> {
    resolve_runtime_worktree_name(repo, None).map_err(|_| {
        "Divergent Line merge requires a managed worktree so conflicts can be resumed or aborted safely. Start or enter a Task worktree first.".to_string()
    })
}

pub(in crate::primitives) fn guard_no_active_line_merge(
    repo: &RepoRuntime,
    worktree_name: Option<&str>,
    operation: &str,
) -> Result<(), String> {
    let resolved_worktree_name = match resolve_runtime_worktree_name(repo, worktree_name) {
        Ok(value) => value,
        Err(_) if worktree_name.is_none() => return Ok(()),
        Err(error) => return Err(error),
    };
    let metadata = load_worktree_metadata(repo, &resolved_worktree_name)?;
    let merge_state = metadata_string(&metadata, "merge_state").unwrap_or_else(|| "idle".into());
    if merge_state == "idle" {
        return Ok(());
    }
    Err(format!(
        "Worktree {resolved_worktree_name} has an active line merge in state {merge_state}. Run `ait line merge --continue` or `ait line merge --abort` before {operation}."
    ))
}

fn guard_idle_merge_and_rebase_state(
    metadata: &JsonMap<String, JsonValue>,
    worktree_name: &str,
) -> Result<(), String> {
    if metadata_string(metadata, "merge_state").is_some_and(|state| state != "idle") {
        return Err(format!(
            "Worktree {worktree_name} already has an active line merge. Use `ait line merge --continue` or `--abort`."
        ));
    }
    if metadata_string(metadata, "rebase_state").as_deref() == Some("conflicted") {
        return Err(format!(
            "Worktree {worktree_name} has a conflicted rebase. Continue or abort it before merging."
        ));
    }
    Ok(())
}

fn require_active_line(row: &JsonValue, line_name: &str) -> Result<(), String> {
    let status = string_field(row, "status").unwrap_or_else(|| "active".to_string());
    if status == "archived" {
        Err(format!(
            "Line {line_name} is archived and cannot be merged."
        ))
    } else {
        Ok(())
    }
}

fn required_line_head(row: &JsonValue, line_name: &str) -> Result<String, String> {
    string_field(row, "head_snapshot_id")
        .ok_or_else(|| format!("Line {line_name} has no head Snapshot to merge."))
}

fn require_clean_merge_workspace(
    repo: &RepoRuntime,
    target_snapshot_id: &str,
) -> Result<(), String> {
    let workspace = workspace_delta_payload(repo, Some(target_snapshot_id), None)?;
    if workspace
        .get("clean")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return Ok(());
    }
    let paths = json_string_list(workspace.get("changed_paths"));
    Err(format!(
        "Line merge requires a clean workspace at target head {target_snapshot_id}; changed paths: {}.",
        summarize_path_sample(&paths)
    ))
}

fn verify_merge_parent_heads(
    repo: &RepoRuntime,
    target_line_name: &str,
    source_verification_line: Option<&str>,
    target_snapshot_id: &str,
    source_snapshot_id: &str,
) -> Result<(), String> {
    let mut expected_heads = vec![(target_line_name, target_snapshot_id)];
    if let Some(source_line_name) = source_verification_line {
        expected_heads.push((source_line_name, source_snapshot_id));
    }
    for (line_name, expected) in expected_heads {
        let actual = local_line_head_snapshot_id(repo, line_name)?;
        if actual.as_deref() != Some(expected) {
            return Err(format!(
                "Cannot continue merge because line {line_name} moved from {expected} to {}.",
                actual.as_deref().unwrap_or("none")
            ));
        }
    }
    Ok(())
}

fn workspace_path_has_merge_marker(repo: &RepoRuntime, path: &str) -> bool {
    fs::read(repo.workspace_root().join(path))
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some_and(|text| {
            text.contains(MERGE_MARKER_PREFIX) || text.contains(MERGE_MARKER_SUFFIX)
        })
}

fn required_metadata_string(
    metadata: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<String, String> {
    metadata_string(metadata, key).ok_or_else(|| {
        format!("Active merge metadata is missing {key}; abort recovery is required.")
    })
}

fn metadata_string_list(metadata: &JsonMap<String, JsonValue>, key: &str) -> Vec<String> {
    metadata
        .get(key)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::to_string)
        .collect()
}

fn metadata_string_map(
    metadata: &JsonMap<String, JsonValue>,
    key: &str,
) -> BTreeMap<String, String> {
    metadata
        .get(key)
        .and_then(JsonValue::as_object)
        .into_iter()
        .flat_map(|values| values.iter())
        .filter_map(|(path, value)| value.as_str().map(|kind| (path.clone(), kind.to_string())))
        .collect()
}

fn json_string_array(values: &[String]) -> JsonValue {
    JsonValue::Array(values.iter().cloned().map(JsonValue::String).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemoryBlobs(BTreeMap<String, Vec<u8>>);

    impl LocalSnapshotBlobReadStore for MemoryBlobs {
        fn read_blob_bytes(&self, blob_id: &str) -> Result<Vec<u8>, String> {
            self.0
                .get(blob_id)
                .cloned()
                .ok_or_else(|| format!("missing blob {blob_id}"))
        }
    }

    fn row(path: &str, blob: &str, mode: &str) -> SnapshotFileRow {
        SnapshotFileRow {
            path: path.to_string(),
            blob_id: blob.to_string(),
            size_bytes: 1,
            mode: mode.to_string(),
            sha256: blob.to_string(),
        }
    }

    #[test]
    fn agreed_merge_bases_mark_only_disputed_paths_ambiguous() {
        let (agreed, ambiguous) = agreed_merge_base_files(&[
            BTreeMap::from([
                ("same".to_string(), row("same", "A", "0o644")),
                ("different".to_string(), row("different", "B", "0o644")),
            ]),
            BTreeMap::from([
                ("same".to_string(), row("same", "A", "0o644")),
                ("different".to_string(), row("different", "C", "0o644")),
            ]),
        ]);
        assert_eq!(agreed["same"].as_ref().unwrap().blob_id, "A");
        assert_eq!(ambiguous, BTreeSet::from(["different".to_string()]));
    }

    #[test]
    fn exact_rename_detection_requires_one_content_identity_candidate() {
        let base = BTreeMap::from([("old".to_string(), Some(row("old", "A", "0o644")))]);
        let side = BTreeMap::from([("new".to_string(), row("new", "A", "0o644"))]);
        assert_eq!(
            exact_renames(&base, &side),
            BTreeMap::from([("old".to_string(), "new".to_string())])
        );
    }

    fn conflict_kind(action: MergePathAction) -> String {
        match action {
            MergePathAction::Conflict { kind } => kind,
            _ => panic!("expected conflict action"),
        }
    }

    #[test]
    fn merge_path_conflict_classifications_are_stable() {
        let store = MemoryBlobs(BTreeMap::from([
            ("B".to_string(), b"base\n".to_vec()),
            ("T".to_string(), b"target\n".to_vec()),
            ("S".to_string(), b"source\n".to_vec()),
            ("BB".to_string(), vec![0, 1]),
            ("BT".to_string(), vec![0, 2]),
            ("BS".to_string(), vec![0, 3]),
        ]));
        let base = row("file", "B", "0o644");
        let target = row("file", "T", "0o644");
        let source = row("file", "S", "0o644");
        assert_eq!(
            conflict_kind(classify_merge_path(&store, None, Some(&target), Some(&source)).unwrap()),
            "add_add"
        );
        assert_eq!(
            conflict_kind(classify_merge_path(&store, Some(&base), None, Some(&source)).unwrap()),
            "delete_modify"
        );
        assert_eq!(
            conflict_kind(classify_merge_path(&store, Some(&base), Some(&target), None).unwrap()),
            "modify_delete"
        );
        assert_eq!(
            conflict_kind(
                classify_merge_path(&store, Some(&base), Some(&target), Some(&source)).unwrap()
            ),
            "text"
        );

        let binary_base = row("binary", "BB", "0o644");
        let binary_target = row("binary", "BT", "0o644");
        let binary_source = row("binary", "BS", "0o644");
        assert_eq!(
            conflict_kind(
                classify_merge_path(
                    &store,
                    Some(&binary_base),
                    Some(&binary_target),
                    Some(&binary_source),
                )
                .unwrap()
            ),
            "binary"
        );

        let mode_target = row("file", "B", "0o755");
        let mode_source = row("file", "B", "0o700");
        assert_eq!(
            conflict_kind(
                classify_merge_path(&store, Some(&base), Some(&mode_target), Some(&mode_source),)
                    .unwrap()
            ),
            "mode"
        );

        let symlink_base = row("link", "B", "0o120777");
        let symlink_target = row("link", "T", "0o120777");
        let symlink_source = row("link", "S", "0o120777");
        assert_eq!(
            conflict_kind(
                classify_merge_path(
                    &store,
                    Some(&symlink_base),
                    Some(&symlink_target),
                    Some(&symlink_source),
                )
                .unwrap()
            ),
            "symlink"
        );
    }

    #[test]
    fn rename_conflict_classifications_are_stable() {
        let base = BTreeMap::from([("old".to_string(), Some(row("old", "A", "0o644")))]);
        let target_renamed =
            BTreeMap::from([("target-new".to_string(), row("target-new", "A", "0o644"))]);
        let source_modified = BTreeMap::from([("old".to_string(), row("old", "B", "0o644"))]);
        assert_eq!(
            classify_rename_conflicts(&base, &target_renamed, &source_modified)["old"],
            "rename_modify"
        );
        assert_eq!(
            classify_rename_conflicts(&base, &source_modified, &target_renamed)["old"],
            "modify_rename"
        );
        let source_renamed =
            BTreeMap::from([("source-new".to_string(), row("source-new", "A", "0o644"))]);
        assert_eq!(
            classify_rename_conflicts(&base, &target_renamed, &source_renamed)["old"],
            "rename_rename"
        );
    }

    #[test]
    fn snapshot_mode_classification_accepts_git_and_native_octal_forms() {
        assert!(snapshot_mode_is_symlink("0o120000"));
        assert!(snapshot_mode_is_symlink("120777"));
        assert!(!snapshot_mode_is_symlink("0o100644"));
        assert!(!snapshot_mode_is_symlink("0o755"));
        assert_eq!(snapshot_mode_bits("0o100755").unwrap() & 0o777, 0o755);
    }
}
