//! Named operation services shared by native and language frontends.
//! Domain primitives own validation, locking, hooks and recovery. No argv is parsed here.
use crate::config_surface::post_finish_hooks_for_repo;
use crate::post_finish_hooks::{
    acquire_hooks_lock, pre_finish_hooks_for_roots, pre_finish_hooks_text_lines,
    run_post_finish_hooks, PostFinishHookContext, POST_FINISH_HOOKS_PAYLOAD_KEY,
    PRE_FINISH_HOOKS_PAYLOAD_KEY,
};
use crate::primitives::*;
use crate::runner_hint::attach_runner_hint_best_effort;
use crate::runtime::RepoRuntime;
use crate::workspace_lock::run_locked_workspace_command;
use ait_core::json_support::{json, JsonValue};
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct WorkflowReadyRequest {
    pub task_id: String,
    pub apply: bool,
    pub remote_name: Option<String>,
    pub expected_identity: Option<PathBuf>,
    pub snapshot_message: Option<String>,
    pub summary: Option<String>,
    pub tests: Option<String>,
    pub lint: Option<String>,
    pub security: Option<String>,
    pub license: Option<String>,
    pub author_mode: Option<String>,
    pub model: Option<String>,
    pub run_ci_once: bool,
}

#[derive(Clone, Debug, Default)]
pub struct WorkflowFinishRequest {
    pub task_id: String,
    pub apply: bool,
    pub remote_name: Option<String>,
    pub expected_identity: Option<PathBuf>,
    pub reviewer: Option<String>,
    pub review_message: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorkflowOperationResult {
    pub payload: JsonValue,
    pub exit_code: u8,
}

pub fn prepare_workflow(
    repo: &RepoRuntime,
    request: &WorkflowReadyRequest,
) -> Result<WorkflowOperationResult, String> {
    if let Some(author_mode) = request.author_mode.as_deref() {
        ait_core::policy::normalize_author_mode(author_mode)?;
    }
    crate::main_app::workflow_ready_service(repo, request)
}

pub fn finish_workflow(
    repo: &RepoRuntime,
    request: &WorkflowFinishRequest,
) -> Result<WorkflowOperationResult, String> {
    crate::main_app::workflow_finish_service(repo, request)
}

#[derive(Clone, Debug)]
pub struct WorkflowReconcileRequest {
    pub remote_name: Option<String>,
    pub task_id: Option<String>,
    pub apply: bool,
    pub safe_only: bool,
    pub scheduled: bool,
    pub limit: usize,
}

pub fn reconcile_workflow(
    repo: &RepoRuntime,
    request: &WorkflowReconcileRequest,
) -> Result<JsonValue, String> {
    if request.scheduled {
        if !request.apply {
            return Err("Scheduled reconciliation requires apply.".into());
        }
        run_locked_workspace_command(repo, "ait-cli workflow reconcile --scheduled", || {
            workflow_reconcile_automatic(
                repo,
                AutomaticReconciliationScope::Remote(request.remote_name.clone()),
                request.task_id.as_deref(),
                AutomaticReconciliationTrigger::ScheduledRemote,
                Some(request.limit),
            )
        })
    } else if request.apply {
        run_locked_workspace_command(repo, "ait-cli workflow reconcile --apply", || {
            workflow_reconcile_apply(
                repo,
                request.remote_name.as_deref(),
                request.task_id.as_deref(),
                request.safe_only,
                Some(request.limit),
            )
        })
    } else {
        workflow_reconcile_inventory(
            repo,
            request.remote_name.as_deref(),
            request.task_id.as_deref(),
            request.safe_only,
            Some(request.limit),
        )
    }
}

pub fn workflow_guide(topic: Option<&str>) -> Result<JsonValue, String> {
    crate::main_app::workflow_guide_payload(topic)
}

pub fn plan_authority_diagnostics(root: &std::path::Path) -> Result<JsonValue, String> {
    match RepoRuntime::discover_from_path(root).ok() {
        Some(repo) => crate::doctor_surface::doctor_plan_authority_for_repository(
            &repo.authoritative_repo_root(),
        ),
        None => crate::doctor_surface::doctor_plan_authority(),
    }
}

pub fn audit_task(
    repo: &RepoRuntime,
    task_id: &str,
    scope: &PlanScope,
) -> Result<JsonValue, String> {
    task_audit(repo, task_id, scope.local, scope.remote_name.as_deref())
}

pub fn abandon_task(
    repo: &RepoRuntime,
    task_id: &str,
    scope: &PlanScope,
) -> Result<WorkflowOperationResult, String> {
    crate::main_app::task_abandon_service(repo, task_id, scope)
}

#[derive(Clone, Debug, Default)]
pub struct PlanScope {
    pub local: bool,
    pub remote_name: Option<String>,
}

pub fn line_inventory(
    repo: &RepoRuntime,
    include_all: bool,
    archived: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let current_line = repo.current_line_name()?;
    let payload = line_list(repo, include_all, archived, remote_name)?;
    Ok(json!({"payload": payload, "current_line": current_line}))
}

pub fn show_line(
    repo: &RepoRuntime,
    name: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if remote_name.is_some() {
        line_show_remote(repo, name, remote_name)
    } else {
        line_show(repo, name)
    }
}

#[derive(Clone, Debug)]
pub struct PlanSelection {
    pub plan_id: String,
    pub revision: Option<String>,
    pub scope: PlanScope,
}

#[derive(Clone, Debug)]
pub struct PlanCandidatesRequest {
    pub scope: PlanScope,
    pub include_all: bool,
    pub contains: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlanSyncRequest {
    pub target: PathBuf,
    pub plan_ref: Option<String>,
    pub prune: bool,
    pub scope: PlanScope,
    pub reconcile: bool,
}

pub fn list_plans(repo: &RepoRuntime, scope: &PlanScope) -> Result<JsonValue, String> {
    crate::main_app::plan_list_service(repo, scope)
}

pub fn show_plan(repo: &RepoRuntime, request: &PlanSelection) -> Result<JsonValue, String> {
    crate::main_app::plan_show_service(repo, request)
}

pub fn plan_revisions(
    repo: &RepoRuntime,
    plan_id: &str,
    scope: &PlanScope,
) -> Result<JsonValue, String> {
    crate::main_app::plan_revisions_service(repo, plan_id, scope)
}

pub fn plan_items(repo: &RepoRuntime, request: &PlanSelection) -> Result<JsonValue, String> {
    crate::main_app::plan_items_service(repo, request)
}

pub fn inspect_plan(repo: &RepoRuntime, request: &PlanSelection) -> Result<JsonValue, String> {
    crate::main_app::plan_inspect_service(repo, request)
}

pub fn plan_candidates(
    repo: &RepoRuntime,
    request: &PlanCandidatesRequest,
) -> Result<JsonValue, String> {
    crate::main_app::plan_candidates_service(repo, request)
}

pub fn sync_plan(repo: &RepoRuntime, request: &PlanSyncRequest) -> Result<JsonValue, String> {
    crate::main_app::plan_sync_service(repo, request)
}

#[derive(Clone, Debug)]
pub struct TaskFinishRequest {
    pub task_id: String,
    pub local: bool,
    pub remote_name: Option<String>,
    pub message: Option<String>,
    pub sync_plan_markdown: bool,
}

#[derive(Clone, Debug)]
pub struct TaskStartRequest {
    pub title: Option<String>,
    pub source: Option<String>,
    pub intent: String,
    pub local: bool,
    pub remote_name: Option<String>,
    pub edit_root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct TaskQuickRequest {
    pub title: Option<String>,
    pub intent: String,
    pub local: bool,
    pub remote_name: Option<String>,
    pub edit_root: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct SnapshotCaptureRequest {
    pub reference: String,
    pub message: Option<String>,
    pub resume_snapshot_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SnapshotAncestryRequest {
    pub selector: String,
    pub descendants: bool,
    pub first_parent: bool,
    pub max_depth: usize,
    pub limit: usize,
}

pub fn inspect_snapshot(
    repo: &RepoRuntime,
    selector: &str,
    include_delta: bool,
) -> Result<JsonValue, String> {
    let snapshot_id = crate::tag_surface::resolve_snapshot_ref(repo, selector)?;
    let payload = snapshot_show(repo, &snapshot_id)?;
    let parent = payload
        .get("parent_snapshot_id")
        .and_then(JsonValue::as_str)
        .filter(|parent| !parent.is_empty());
    let delta = if include_delta {
        parent
            .map(|parent| {
                snapshot_diff(
                    repo,
                    parent,
                    &snapshot_id,
                    false,
                    ait_core::object_diff::DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
                )
            })
            .transpose()?
    } else {
        None
    };
    Ok(json!({"payload": payload, "delta": delta}))
}

pub fn query_snapshot_ancestry(
    repo: &RepoRuntime,
    request: &SnapshotAncestryRequest,
) -> Result<JsonValue, String> {
    if request.max_depth == 0 || request.limit == 0 {
        return Err("Snapshot ancestry max_depth and limit must be positive.".into());
    }
    let snapshot = crate::tag_surface::resolve_snapshot_ref(repo, &request.selector)?;
    let direction = if request.descendants {
        SnapshotAncestryDirection::Descendants
    } else {
        SnapshotAncestryDirection::Ancestors
    };
    snapshot_ancestry(
        repo,
        &snapshot,
        direction,
        request.first_parent,
        request.max_depth,
        request.limit,
    )
}

pub fn diff_snapshot_selectors(
    repo: &RepoRuntime,
    older: &str,
    newer: &str,
    include_text: bool,
    max_bytes: usize,
) -> Result<JsonValue, String> {
    let older = crate::tag_surface::resolve_snapshot_ref(repo, older)?;
    let newer = crate::tag_surface::resolve_snapshot_ref(repo, newer)?;
    snapshot_diff(repo, &older, &newer, include_text, max_bytes)
}

pub fn revert_snapshot_selector(
    repo: &RepoRuntime,
    selector: &str,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let snapshot = crate::tag_surface::resolve_snapshot_ref(repo, selector)?;
    run_locked_workspace_command(repo, "ait-cli snapshot revert", || {
        snapshot_revert(repo, &snapshot, force, dry_run)
    })
}

pub fn replay_snapshot_selector(
    repo: &RepoRuntime,
    selector: &str,
    onto: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let snapshot = crate::tag_surface::resolve_snapshot_ref(repo, selector)?;
    run_locked_workspace_command(repo, "ait-cli snapshot replay", || {
        let onto = match onto {
            Some(line) => line.to_string(),
            None => repo.current_line_name()?,
        };
        snapshot_replay(repo, &snapshot, &onto, force, dry_run)
    })
}

pub fn query_snapshot_is_ancestor(
    repo: &RepoRuntime,
    older: &str,
    newer: &str,
) -> Result<WorkflowOperationResult, String> {
    let older = crate::tag_surface::resolve_snapshot_ref(repo, older)?;
    let newer = crate::tag_surface::resolve_snapshot_ref(repo, newer)?;
    let (payload, found) = snapshot_is_ancestor_query(repo, &older, &newer)?;
    Ok(WorkflowOperationResult {
        payload,
        exit_code: u8::from(!found),
    })
}

pub fn query_snapshot_merge_base(
    repo: &RepoRuntime,
    left: &str,
    right: &str,
    all: bool,
) -> Result<WorkflowOperationResult, String> {
    let left = crate::tag_surface::resolve_snapshot_ref(repo, left)?;
    let right = crate::tag_surface::resolve_snapshot_ref(repo, right)?;
    let (payload, found) = snapshot_merge_base_query(repo, &left, &right, all)?;
    Ok(WorkflowOperationResult {
        payload,
        exit_code: u8::from(!found),
    })
}

pub type TaskStartProgress<'a> = dyn FnMut(&JsonValue) -> Result<(), String> + 'a;

#[derive(Clone, Debug, Default)]
pub struct RemoteSyncRequest {
    pub remote_name: Option<String>,
    pub line_name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PullRequest {
    pub remote_name: Option<String>,
    pub line_name: Option<String>,
    pub merge: bool,
    pub restore: bool,
    pub force: bool,
}

pub fn pull_repository(repo: &RepoRuntime, request: &PullRequest) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli pull", || {
        pull(
            repo,
            request.remote_name.as_deref(),
            request.line_name.as_deref(),
            request.merge,
            request.restore,
            request.force,
        )
    })
}

pub fn push_repository(
    repo: &RepoRuntime,
    request: &RemoteSyncRequest,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli push", || {
        push(
            repo,
            request.remote_name.as_deref(),
            request.line_name.as_deref(),
        )
    })
}

pub fn save_stash(
    repo: &RepoRuntime,
    message: Option<&str>,
    keep_workspace: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli stash save", || {
        stash_save(repo, message, keep_workspace)
    })
}

fn stash_restore_error(error: String, force: bool) -> String {
    if !force && error.starts_with("Workspace has unsaved changes") {
        format!("{error}. Re-run with --force to replace those workspace changes.")
    } else {
        error
    }
}

pub fn apply_stash(repo: &RepoRuntime, stash_id: &str, force: bool) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli stash apply", || {
        stash_apply(repo, stash_id, force).map_err(|error| stash_restore_error(error, force))
    })
}

pub fn pop_stash(repo: &RepoRuntime, stash_id: &str, force: bool) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli stash pop", || {
        stash_pop(repo, stash_id, force).map_err(|error| stash_restore_error(error, force))
    })
}

pub fn drop_stash(repo: &RepoRuntime, stash_id: &str) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli stash drop", || stash_drop(repo, stash_id))
}

pub fn emit_task_start_progress(event: &JsonValue) -> Result<(), String> {
    if let Some(line) = crate::main_app::task_start_progress_line(event) {
        println!("{line}");
    }
    Ok(())
}

pub fn start_task(
    repo: &RepoRuntime,
    request: &TaskStartRequest,
    progress: Option<&mut TaskStartProgress<'_>>,
) -> Result<JsonValue, String> {
    if request.source.is_some() && request.title.is_some() {
        return Err("Task start accepts a title or Plan source, not both.".into());
    }
    let automatic_scope =
        if repo.task_uses_local_scope(request.local, request.remote_name.as_deref())? {
            AutomaticReconciliationScope::Local
        } else {
            AutomaticReconciliationScope::Remote(request.remote_name.clone())
        };
    run_locked_workspace_command(repo, "ait-cli task start", || {
        let reconciliation = workflow_reconcile_automatic_best_effort(
            repo,
            automatic_scope,
            None,
            AutomaticReconciliationTrigger::PreTaskStart,
            None,
        );
        let mut payload = if let Some(source) = request.source.as_deref() {
            task_start_from_with_edit_root_and_progress(
                repo,
                source,
                &request.intent,
                request.local,
                request.remote_name.as_deref(),
                request.edit_root.as_deref(),
                None,
                progress,
            )?
        } else {
            let title = request
                .title
                .as_deref()
                .ok_or_else(|| "`--title` is required unless `--from` is provided.".to_string())?;
            task_start_with_progress(
                repo,
                title,
                &request.intent,
                request.local,
                request.remote_name.as_deref(),
                None,
                None,
                None,
                request.edit_root.as_deref(),
                None,
                progress,
            )?
        };
        attach_automatic_reconciliation(&mut payload, reconciliation);
        attach_plan_backed_markdown_summary(repo, &mut payload);
        Ok(payload)
    })
}

pub fn quick_task(
    repo: &RepoRuntime,
    request: &TaskQuickRequest,
    progress: Option<&mut TaskStartProgress<'_>>,
) -> Result<JsonValue, String> {
    let automatic_scope =
        if repo.task_uses_local_scope(request.local, request.remote_name.as_deref())? {
            AutomaticReconciliationScope::Local
        } else {
            AutomaticReconciliationScope::Remote(request.remote_name.clone())
        };
    run_locked_workspace_command(repo, "ait-cli task quick", || {
        let reconciliation = workflow_reconcile_automatic_best_effort(
            repo,
            automatic_scope,
            None,
            AutomaticReconciliationTrigger::PreTaskStart,
            None,
        );
        let mut payload = task_quick_with_edit_root_and_progress(
            repo,
            request.title.as_deref(),
            &request.intent,
            request.local,
            request.remote_name.as_deref(),
            request.edit_root.as_deref(),
            progress,
        )?;
        attach_automatic_reconciliation(&mut payload, reconciliation);
        attach_plan_backed_markdown_summary(repo, &mut payload);
        Ok(payload)
    })
}

pub fn capture_snapshot(
    repo: &RepoRuntime,
    request: &SnapshotCaptureRequest,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli snapshot create", || {
        if let Some(id) = request.resume_snapshot_id.as_deref() {
            return resume_remote_snapshot_capture(repo, &request.reference, id);
        }
        snapshot_create_for_reference(repo, &request.reference, request.message.as_deref())
    })
}

pub fn initialize_repository(
    request: &crate::init_surface::InitRequest,
) -> Result<JsonValue, String> {
    let mut payload = crate::init_surface::init_repo(request)?;
    let repo = RepoRuntime::discover_from_path(&request.root)?;
    payload["agent_team_bootstrap"] = crate::agent_teams::bootstrap(&repo);
    Ok(payload)
}

pub fn finish_task(repo: &RepoRuntime, request: &TaskFinishRequest) -> Result<JsonValue, String> {
    let (use_local_scope, scoped_remote_name) =
        resolve_task_land_scope(repo, request.local, request.remote_name.as_deref())?;
    if !use_local_scope && request.message.is_some() {
        return Err("`--message` is available only for local `ait task finish`; remote finish consumes an already-ready selected Patchset.".to_string());
    }
    if !use_local_scope && request.sync_plan_markdown {
        return Err("`--sync-plan-markdown` is available only for local `ait task finish`; sync Plan-backed Markdown with `ait plan sync <path> --local` before a remote finish.".to_string());
    }
    let presync = if use_local_scope && request.sync_plan_markdown {
        Some(run_locked_workspace_command(
            repo,
            "ait-cli task finish --sync-plan-markdown",
            || presync_plan_markdown_drift(repo),
        )?)
    } else {
        None
    };
    let pre_finish = if use_local_scope {
        run_pre_finish_hooks_gate(repo, &request.task_id)?
    } else {
        None
    };
    let mut payload = run_task_scoped_workspace_command(
        repo,
        &request.task_id,
        use_local_scope,
        !use_local_scope,
        scoped_remote_name.as_deref(),
        "ait-cli task finish",
        |execution_repo| {
            task_land_apply_scoped(
                execution_repo,
                &request.task_id,
                use_local_scope,
                scoped_remote_name.as_deref(),
                request.message.as_deref(),
                None::<fn(&JsonValue) -> Result<(), String>>,
            )
        },
    )?;
    let task_id = workflow_payload_task_id(&payload);
    let scope = if use_local_scope {
        AutomaticReconciliationScope::Local
    } else {
        AutomaticReconciliationScope::Remote(scoped_remote_name.clone())
    };
    let incomplete_local_closeout = use_local_scope
        && (payload["apply_status"].as_str() != Some("done")
            || payload["repo_root_restore"]["status"].as_str() == Some("failed")
            || payload["bound_worktree_cleanup"]["status"].as_str() == Some("failed"));
    let reconciliation = if incomplete_local_closeout {
        json!({"contract":"workflow-automatic-reconciliation/v1","automatic":true,"status":"skipped",
            "scope":"local","mutated":false,"reason":"task_closeout_recovery_pending",
            "task_filter":task_id,"detail":"Preserve worktree registration until exact Task closeout recovery succeeds."})
    } else {
        run_automatic_reconciliation_locked(
            repo,
            scope,
            task_id.as_deref(),
            task_land_automatic_trigger(&payload),
        )
    };
    attach_automatic_reconciliation(&mut payload, reconciliation);
    if let (Some(presync), Some(object)) = (presync, payload.as_object_mut()) {
        object.insert(PLAN_MARKDOWN_PRESYNC_PAYLOAD_KEY.to_string(), presync);
    }
    if let (Some(pre_finish), Some(object)) = (pre_finish, payload.as_object_mut()) {
        object.insert(PRE_FINISH_HOOKS_PAYLOAD_KEY.to_string(), pre_finish);
    }
    if use_local_scope {
        attach_post_finish_hooks(repo, &mut payload);
    } else {
        attach_runner_hint_best_effort(repo, scoped_remote_name.as_deref(), &mut payload);
    }
    crate::team_remote::attach_finish_handoff(
        crate::agent_teams::enabled(repo),
        &repo.authoritative_repo_root(),
        if use_local_scope {
            crate::team_remote::FinishScope::Local
        } else {
            crate::team_remote::FinishScope::Remote
        },
        &mut payload,
        &crate::agent_teams::CoordinatorSource,
    );

    Ok(payload)
}

/// Run `hooks.post_finish` once after a fresh local apply. An already-finished
/// closeout recovery and every non-applied outcome skip the hooks.
pub(crate) fn attach_post_finish_hooks(repo: &RepoRuntime, payload: &mut JsonValue) {
    if payload.get("apply_status").and_then(JsonValue::as_str) != Some("done") {
        return;
    }
    if payload.get("execution_status").and_then(JsonValue::as_str) == Some("already_landed") {
        return;
    }
    let repo_root = repo.authoritative_repo_root();
    let result = match post_finish_hooks_for_repo(repo) {
        Ok(hooks) if hooks.is_empty() => {
            run_post_finish_hooks(&repo_root, &hooks, &PostFinishHookContext::default())
        }
        Ok(hooks) => match acquire_hooks_lock(&repo_root) {
            Ok(_lock) => {
                let mut context = PostFinishHookContext::from_finish_payload(payload);
                context.current_head_snapshot_id = line_show(repo, Some(&repo.default_line_name()))
                    .ok()
                    .and_then(|row| {
                        row.get("head_snapshot_id")
                            .and_then(JsonValue::as_str)
                            .map(ToString::to_string)
                    });
                let mut result = run_post_finish_hooks(&repo_root, &hooks, &context);
                if let Some(object) = result.as_object_mut() {
                    object.insert("serialized".to_string(), JsonValue::Bool(true));
                    object.insert(
                        "current_head_snapshot_id".to_string(),
                        context
                            .current_head_snapshot_id
                            .clone()
                            .map(JsonValue::String)
                            .unwrap_or(JsonValue::Null),
                    );
                }
                result
            }
            Err(error) => json!({
                "status": "failed",
                "error": error,
                "repo_root": repo_root.to_string_lossy(),
                "configured_count": hooks.len(),
                "failed_count": 0,
                "hooks": [],
                "detail": "The finish hook lock could not be acquired, so no hook ran. The Task finish itself is applied; rerun the hook commands manually.",
            }),
        },
        Err(error) => json!({
            "status": "failed",
            "error": error,
                "repo_root": repo_root.to_string_lossy(),
            "configured_count": 0,
            "failed_count": 0,
            "hooks": [],
            "detail": "hooks.post_finish in .ait/config.json could not be parsed, so no hook ran. The Task finish itself is applied; fix the configuration with `ait config set --post-finish-hooks <json>`.",
        }),
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert(POST_FINISH_HOOKS_PAYLOAD_KEY.to_string(), result);
    }
}

/// Tell the agent at start time which Markdown in the new worktree travels
/// through Plan lineage, so the finish gate is not the first place it learns.
pub(crate) fn attach_plan_backed_markdown_summary(repo: &RepoRuntime, payload: &mut JsonValue) {
    let Some(edit_root) = payload
        .get("worktree")
        .and_then(|worktree| worktree.get("path"))
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
    else {
        return;
    };
    let summary = plan_backed_markdown_summary(repo, &edit_root);
    if let Some(object) = payload.as_object_mut() {
        object.insert(PLAN_BACKED_MARKDOWN_PAYLOAD_KEY.to_string(), summary);
    }
}

/// Run tracked `pre_finish` hooks in the Task worktree before any Snapshot is
/// created. A `fail` hook refuses the finish with its output tail; nothing has
/// been applied yet, so the refusal is safe to retry after the fix.
pub(crate) fn run_pre_finish_hooks_gate(
    repo: &RepoRuntime,
    requested: &str,
) -> Result<Option<JsonValue>, String> {
    if !repo.is_worktree() {
        return Ok(None);
    }
    let workspace_root = repo.workspace_root();
    let Some((path, hooks)) =
        pre_finish_hooks_for_roots(&workspace_root, &repo.authoritative_repo_root())?
    else {
        return Ok(None);
    };
    let context = PostFinishHookContext {
        task_id: Some(requested.to_string()),
        ..PostFinishHookContext::default()
    };
    let mut result = run_post_finish_hooks(&workspace_root, &hooks, &context);
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "source".to_string(),
            JsonValue::String(path.to_string_lossy().to_string()),
        );
        object.insert(
            "phase".to_string(),
            JsonValue::String("pre_finish".to_string()),
        );
    }
    if result.get("status").and_then(JsonValue::as_str) == Some("failed") {
        let mut lines = vec![format!(
            "Refusing `ait task finish {requested}`: a pre-finish hook from {} failed. Nothing was applied; fix the reported command and rerun `ait task finish {requested} --local`.",
            path.display()
        )];
        lines.extend(
            pre_finish_hooks_text_lines(&json!({ PRE_FINISH_HOOKS_PAYLOAD_KEY: result }))
                .into_iter()
                .skip(1),
        );
        return Err(lines.join("\n"));
    }
    Ok(Some(result))
}

pub(crate) fn validate_scope(local: bool, remote_name: Option<&str>) -> Result<(), String> {
    if local && remote_name.is_some() {
        return Err("--local cannot be combined with --remote".to_string());
    }
    if remote_name.is_some_and(|name| name.trim().is_empty()) {
        return Err("--remote requires a non-empty remote name".to_string());
    }
    Ok(())
}

pub(crate) fn resolve_task_land_scope(
    repo: &RepoRuntime,
    local: bool,
    remote_name: Option<&str>,
) -> Result<(bool, Option<String>), String> {
    validate_scope(local, remote_name)?;
    let use_local_scope = repo.task_uses_local_scope(local, remote_name)?;
    if use_local_scope {
        return Ok((true, None));
    }
    let remote = repo.remote_row(remote_name)?;
    Ok((false, Some(remote.name)))
}

pub(crate) fn reconciliation_scope_label(scope: &AutomaticReconciliationScope) -> &'static str {
    match scope {
        AutomaticReconciliationScope::Local => "local",
        AutomaticReconciliationScope::Remote(_) => "remote",
    }
}

pub(crate) fn run_automatic_reconciliation_locked(
    repo: &RepoRuntime,
    scope: AutomaticReconciliationScope,
    task_filter: Option<&str>,
    trigger: AutomaticReconciliationTrigger,
) -> JsonValue {
    let scope_label = reconciliation_scope_label(&scope);
    let command = format!("ait-cli automatic reconciliation {}", trigger.label());
    let root_repo = match RepoRuntime::discover_from_path(&repo.authoritative_repo_root()) {
        Ok(root_repo) => root_repo,
        Err(error) => {
            return json!({
                "contract": "workflow-automatic-reconciliation/v1",
                "automatic": true,
                "status": "failed_non_blocking",
                "trigger": trigger.label(),
                "scope": scope_label,
                "task_filter": task_filter,
                "safe_only": true,
                "mutated": false,
                "error": format!("Failed to run automatic repair from the main repository root: {error}"),
                "next_command": "ait workflow reconcile --dry-run",
            });
        }
    };
    match run_locked_workspace_command(&root_repo, &command, || {
        Ok(workflow_reconcile_automatic_best_effort(
            &root_repo,
            scope,
            task_filter,
            trigger,
            None,
        ))
    }) {
        Ok(payload) => payload,
        Err(error) => json!({
            "contract": "workflow-automatic-reconciliation/v1",
            "automatic": true,
            "status": "failed_non_blocking",
            "trigger": trigger.label(),
            "scope": scope_label,
            "task_filter": task_filter,
            "safe_only": true,
            "mutated": false,
            "error": error,
            "next_command": "ait workflow reconcile --dry-run",
        }),
    }
}

pub(crate) fn attach_automatic_reconciliation(payload: &mut JsonValue, reconciliation: JsonValue) {
    if let Some(object) = payload.as_object_mut() {
        object.insert("automatic_reconciliation".to_string(), reconciliation);
    }
}

pub(crate) fn workflow_payload_task_id(payload: &JsonValue) -> Option<String> {
    payload
        .get("task_id")
        .and_then(JsonValue::as_str)
        .or_else(|| {
            payload
                .get("task")
                .and_then(JsonValue::as_object)
                .and_then(|task| task.get("task_id"))
                .and_then(JsonValue::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn task_land_automatic_trigger(payload: &JsonValue) -> AutomaticReconciliationTrigger {
    let recovery = payload
        .get("execution_status")
        .and_then(JsonValue::as_str)
        .is_some_and(|status| status == "already_landed")
        || payload
            .get("status")
            .and_then(JsonValue::as_str)
            .is_some_and(|status| status.contains("recovery"))
        || payload.get("closeout_recovery").is_some();
    if recovery {
        AutomaticReconciliationTrigger::LandRecovery
    } else {
        AutomaticReconciliationTrigger::LandTerminal
    }
}

pub fn recover_remote_head(
    root: &std::path::Path,
    request: &crate::remote_head_recovery::RemoteHeadRecoveryRequest,
) -> Result<JsonValue, String> {
    let context =
        crate::remote_head_recovery::RemoteHeadRecoveryContext::discover_from_path_or_environment(
            root,
        )?;
    crate::remote_head_recovery::recover_remote_head(&context, request)
}

/// Scalar native Snapshot ownership grammar; this does not execute a command.
pub fn parse_snapshot_authoring_reference(reference: &str) -> Result<String, String> {
    crate::primitives::parse_snapshot_authoring_reference(reference)
}
