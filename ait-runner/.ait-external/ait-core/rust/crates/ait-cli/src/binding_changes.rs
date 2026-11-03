//! Concrete Change and Patchset operations shared by terminal frontends.
use crate::binding_services::{attach_automatic_reconciliation, workflow_payload_task_id};
use crate::primitives::*;
use crate::runtime::RepoRuntime;
use crate::workspace_lock::run_locked_workspace_command;
use ait_core::json_support::JsonValue;

#[derive(Clone, Debug, Default)]
pub struct ChangeScope {
    pub local: bool,
    pub remote_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ChangeCreateRequest {
    pub task_id: String,
    pub title: String,
    pub base_line: Option<String>,
    pub fork_snapshot: Option<String>,
    pub scope: ChangeScope,
}

#[derive(Clone, Debug, Default)]
pub struct ChangeMutationOptions {
    pub force: bool,
    pub dry_run: bool,
    pub scope: ChangeScope,
}

pub fn create_change(
    repo: &RepoRuntime,
    request: &ChangeCreateRequest,
) -> Result<JsonValue, String> {
    run_task_scoped_workspace_command(
        repo,
        &request.task_id,
        false,
        false,
        request.scope.remote_name.as_deref(),
        "ait-cli change create",
        |execution_repo| {
            change_create_revision(
                execution_repo,
                &request.task_id,
                &request.title,
                request.base_line.as_deref(),
                request.fork_snapshot.as_deref(),
                request.scope.local,
                request.scope.remote_name.as_deref(),
            )
        },
    )
}

pub fn inspect_change(
    repo: &RepoRuntime,
    reference: &str,
    scope: &ChangeScope,
) -> Result<JsonValue, String> {
    let reference = resolve_task_inspect_change_input(
        repo,
        reference,
        scope.local,
        scope.remote_name.as_deref(),
    )?;
    change_show(
        repo,
        &reference,
        scope.local,
        scope.remote_name.as_deref(),
        None,
    )
}

pub fn revert_change(
    repo: &RepoRuntime,
    reference: &str,
    options: &ChangeMutationOptions,
) -> Result<JsonValue, String> {
    let reference = resolve_task_author_change_input(
        repo,
        reference,
        options.scope.local,
        options.scope.remote_name.as_deref(),
    )?;
    run_locked_workspace_command(repo, "ait-cli change revert", || {
        change_revert(
            repo,
            &reference,
            options.force,
            options.dry_run,
            options.scope.local,
            options.scope.remote_name.as_deref(),
            None,
        )
    })
}

pub fn replay_change(
    repo: &RepoRuntime,
    reference: &str,
    onto: Option<&str>,
    options: &ChangeMutationOptions,
) -> Result<JsonValue, String> {
    let reference = resolve_task_author_change_input(
        repo,
        reference,
        options.scope.local,
        options.scope.remote_name.as_deref(),
    )?;
    run_locked_workspace_command(repo, "ait-cli change replay", || {
        let onto = match onto {
            Some(line) => line.to_string(),
            None => repo.current_line_name()?,
        };
        change_replay(
            repo,
            &reference,
            &onto,
            options.force,
            options.dry_run,
            options.scope.local,
            options.scope.remote_name.as_deref(),
            None,
        )
    })
}

pub fn close_change(
    repo: &RepoRuntime,
    reference: &str,
    scope: &ChangeScope,
) -> Result<JsonValue, String> {
    let reference = resolve_task_finish_change_input(
        repo,
        reference,
        scope.local,
        scope.remote_name.as_deref(),
    )?;
    let automatic_scope = if repo.change_uses_local_scope(scope.local, scope.remote_name.as_deref())
    {
        AutomaticReconciliationScope::Local
    } else {
        AutomaticReconciliationScope::Remote(scope.remote_name.clone())
    };
    run_locked_workspace_command(repo, "ait-cli change close", || {
        let mut payload =
            change_close(repo, &reference, scope.local, scope.remote_name.as_deref())?;
        let task = workflow_payload_task_id(&payload);
        let reconciliation = workflow_reconcile_automatic_best_effort(
            repo,
            automatic_scope,
            task.as_deref(),
            AutomaticReconciliationTrigger::ChangeTerminal,
            None,
        );
        attach_automatic_reconciliation(&mut payload, reconciliation);
        Ok(payload)
    })
}

pub fn publish_change(
    repo: &RepoRuntime,
    reference: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let reference = resolve_task_finish_change_input(repo, reference, true, None)?;
    change_publish(repo, &reference, remote_name)
}

pub fn publish_patchset(
    repo: &RepoRuntime,
    reference: &str,
    summary: &str,
    author_mode: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if let Some(mode) = author_mode {
        ait_core::policy::normalize_author_mode(mode)?;
    }
    let resolved = resolve_task_remote_change_input(repo, reference, remote_name)?;
    run_task_scoped_workspace_command(
        repo,
        reference,
        true,
        true,
        remote_name,
        "ait-cli patchset publish",
        |execution_repo| {
            patchset_publish(execution_repo, &resolved, summary, author_mode, remote_name)
        },
    )
}

pub fn list_patchsets(
    repo: &RepoRuntime,
    reference: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let reference = resolve_task_remote_change_input(repo, reference, remote_name)?;
    patchset_list(repo, &reference, remote_name)
}
