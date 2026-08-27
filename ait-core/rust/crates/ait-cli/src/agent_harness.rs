use crate::runtime::RepoRuntime;
use crate::task_land_contract::{task_land_scope_contract, TASK_LAND_CONTRACT_VERSION};
use crate::workspace_lock::run_locked_workspace_command;
use ait_core::json_support::{json, JsonValue};
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use tempfile::NamedTempFile;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const MANAGED_START: &str = "<!-- ait:workflow:start -->";
const MANAGED_END: &str = "<!-- ait:workflow:end -->";
const PLAN_BINARY_DB_WRITE_LAYOUT: u32 = 1;
const AGENT_HARNESS_PATH: &str = "AGENTS.md";

pub fn refresh_agent_workflow_harness(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let path = repo.authoritative_repo_root().join("AGENTS.md");
    let existing = read_optional_regular_text(&path, "Agent contract")?
        .unwrap_or_else(|| "# AGENTS\n".to_string());
    let managed = render_agent_workflow_block(repo);
    let updated = replace_or_insert_managed_block(&existing, &managed)?;
    let changed = updated != existing;
    if changed {
        write_text_atomically(&path, &updated, 0o644)?;
    }
    Ok(json!({
        "status": if changed { "updated" } else { "unchanged" },
        "changed": changed,
        "artifact_path": AGENT_HARNESS_PATH,
        "path": path,
    }))
}

pub fn converge_agent_workflow_harness(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let root_repo = RepoRuntime::discover_from_path(&repo.authoritative_repo_root())?;
    run_locked_workspace_command(&root_repo, "ait-cli agent harness converge", || {
        ensure_configured_sprint_directory(&root_repo)?;
        converge_agent_workflow_harness_with_executor(&root_repo, |request| {
            execute_plan_sync_command_request_json(&request.to_string())
        })
    })
}

fn converge_agent_workflow_harness_with_executor<F>(
    repo: &RepoRuntime,
    mut execute: F,
) -> Result<JsonValue, String>
where
    F: FnMut(&JsonValue) -> Result<JsonValue, String>,
{
    let refresh = refresh_agent_workflow_harness(repo)?;
    let scope = effective_agent_harness_scope(repo);
    let remote_name = if scope == "remote" {
        let Some(remote_name) = repo.default_remote_name() else {
            return Ok(pending_remote_convergence(
                refresh,
                "no_default_remote",
                None,
                None,
            ));
        };
        match repo.remote_row(Some(&remote_name)) {
            Ok(_) => Some(remote_name),
            Err(error) => {
                return Ok(pending_remote_convergence(
                    refresh,
                    "default_remote_unavailable",
                    Some(remote_name),
                    Some(error),
                ));
            }
        }
    } else {
        None
    };
    let request = agent_harness_plan_sync_request(repo, remote_name.as_deref())?;
    let plan_sync = execute(&request)?;
    if plan_sync.get("status").and_then(JsonValue::as_str) != Some("ok") {
        let error = plan_sync
            .get("error")
            .and_then(JsonValue::as_str)
            .unwrap_or("plan sync returned a non-ok result");
        return Err(format!(
            "Generated {AGENT_HARNESS_PATH} was refreshed, but automatic {scope} plan sync failed: {error}"
        ));
    }
    Ok(json!({
        "status": "synced",
        "scope": scope,
        "remote": remote_name,
        "artifact_path": AGENT_HARNESS_PATH,
        "refresh": refresh,
        "plan_sync": plan_sync,
    }))
}

fn pending_remote_convergence(
    refresh: JsonValue,
    reason: &str,
    remote_name: Option<String>,
    error: Option<String>,
) -> JsonValue {
    json!({
        "status": "pending",
        "scope": "remote",
        "remote": remote_name,
        "artifact_path": AGENT_HARNESS_PATH,
        "reason": reason,
        "error": error,
        "refresh": refresh,
        "plan_sync": JsonValue::Null,
        "next_action": "Connect or repair the default remote; the default-remote mutation path will retry AGENTS.md convergence automatically.",
    })
}

fn agent_harness_plan_sync_request(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let mut payload = json!({
        "root_path": repo.authoritative_repo_root(),
        "repo_name": repo.repo_name(),
        "repository_index": repo.repository_index(),
        "id_namespace_prefix": repo.id_namespace_prefix(),
        "created_by": repo.actor_identity(),
        "target": AGENT_HARNESS_PATH,
        "plan_ref": JsonValue::Null,
        "prune": false,
        "local": remote_name.is_none(),
        "remote_name": JsonValue::Null,
        "remote_repo_name": JsonValue::Null,
        "base_url": JsonValue::Null,
        "rebase": false,
        "reconcile": false,
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    });
    if let Some(remote_name) = remote_name {
        let remote = repo.remote_row(Some(remote_name))?;
        let object = payload
            .as_object_mut()
            .ok_or_else(|| "Agent harness plan sync request must be an object.".to_string())?;
        object.insert("remote_name".to_string(), JsonValue::String(remote.name));
        object.insert(
            "remote_repo_name".to_string(),
            JsonValue::String(remote.repo_name.unwrap_or_else(|| repo.repo_name())),
        );
        object.insert("base_url".to_string(), JsonValue::String(remote.url));
    }
    Ok(payload)
}

fn effective_agent_harness_scope(repo: &RepoRuntime) -> &'static str {
    match repo.effective_workflow_mode().as_str() {
        "solo_local" => "local",
        "solo_remote" | "team_remote" => "remote",
        _ => match repo
            .config
            .get("workflow_default_scope")
            .and_then(JsonValue::as_str)
        {
            Some("remote") => "remote",
            _ => "local",
        },
    }
}

fn markdown_code_span(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let longest_backtick_run = normalized
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest_backtick_run + 1);
    if normalized.starts_with('`') || normalized.ends_with('`') {
        format!("{fence} {normalized} {fence}")
    } else {
        format!("{fence}{normalized}{fence}")
    }
}

fn effective_plan_binding_mode(repo: &RepoRuntime, sprint_enabled: bool) -> String {
    repo.config
        .get("plan_task_binding")
        .and_then(|binding| binding.get("mode"))
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            if sprint_enabled {
                "required".to_string()
            } else {
                "off".to_string()
            }
        })
}

fn render_effective_workflow_admission(
    repo: &RepoRuntime,
    workflow_mode: &str,
    sprint_enabled: bool,
    scope_label: &str,
    land_contract: &crate::task_land_contract::TaskLandScopeContract,
) -> String {
    let mut satisfied = Vec::new();
    let mut action_required = Vec::new();
    let sprint = if sprint_enabled { "on" } else { "off" };
    let binding = effective_plan_binding_mode(repo, sprint_enabled);
    let expected_binding = if sprint_enabled { "required" } else { "off" };

    if workflow_mode == "custom" {
        action_required.push(format!(
            "- entry: mode={}; scopes={} (unsupported workflow combination)",
            markdown_code_span(workflow_mode),
            markdown_code_span(scope_label),
        ));
    } else {
        satisfied.push(format!(
            "- entry: mode={}; sprint={}; scopes={}",
            markdown_code_span(workflow_mode),
            markdown_code_span(sprint),
            markdown_code_span(scope_label),
        ));
    }
    let binding_fact = format!("plan-binding={}", markdown_code_span(&binding));
    if binding == expected_binding {
        satisfied.push(format!("- entry: {binding_fact}"));
    } else {
        action_required.push(format!(
            "- entry: {binding_fact} (expected {} for sprint={})",
            markdown_code_span(expected_binding),
            markdown_code_span(sprint),
        ));
    }

    let default_remote = repo.default_remote_name();
    if scope_label == "local" {
        let remote = match default_remote.as_deref() {
            Some(name) => format!("{} (inactive)", markdown_code_span(name)),
            None => markdown_code_span("none"),
        };
        satisfied.push(format!(
            "- entry: default-remote={remote}; transport={}; server-use={}",
            markdown_code_span("local-only"),
            markdown_code_span("none"),
        ));
    } else {
        match default_remote.as_deref() {
            Some(name) if repo.remote_row(Some(name)).is_ok() => satisfied.push(format!(
                "- entry: default-remote={}; transport={}",
                markdown_code_span(name),
                markdown_code_span("remote"),
            )),
            Some(name) => action_required.push(format!(
                "- entry: default-remote={} (configured remote is unavailable)",
                markdown_code_span(name),
            )),
            None => action_required.push(format!(
                "- entry: default-remote={} (required by {})",
                markdown_code_span("unset"),
                markdown_code_span(workflow_mode),
            )),
        }
    }

    let author_mode = repo.effective_author_mode(None);
    let model = repo
        .effective_model_name(None)
        .map(|value| markdown_code_span(&value))
        .unwrap_or_else(|| format!("{} (optional)", markdown_code_span("unset")));
    satisfied.push(format!(
        "- authoring: author-mode={}; model={model}",
        markdown_code_span(&author_mode),
    ));

    let task_review = if repo
        .config
        .get("task_review")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        "required"
    } else {
        "automatic"
    };
    if repo.task_review_reviewer_identity().is_some() {
        satisfied.push(format!(
            "- closeout: review={}; reviewer={}",
            markdown_code_span(task_review),
            markdown_code_span("configured"),
        ));
    } else {
        action_required.push(format!(
            "- closeout: review={}; reviewer={} (configure user-name)",
            markdown_code_span(task_review),
            markdown_code_span("unset"),
        ));
    }
    satisfied.push(format!(
        "- closeout: contract={}; readiness={}; plan-closeout={}",
        markdown_code_span(TASK_LAND_CONTRACT_VERSION),
        markdown_code_span(land_contract.readiness_policy),
        markdown_code_span(land_contract.plan_closeout_policy),
    ));

    let action_section = if action_required.is_empty() {
        "Action required: none.".to_string()
    } else {
        format!("Action required:\n\n{}", action_required.join("\n"))
    };
    format!(
        r#"### Effective workflow admission

Satisfied:

{}

{action_section}

`task start` revalidates entry; Snapshot creation and `task finish` revalidate
authoring and closeout. Inspect configuration only for an action-required item,
an explicit configuration task, or a validator-reported mismatch."#,
        satisfied.join("\n"),
    )
}

pub fn render_agent_workflow_block(repo: &RepoRuntime) -> String {
    let workflow_mode = repo.effective_workflow_mode();
    let sprint_enabled = repo.sprint_enabled();
    let remote_name = repo
        .default_remote_name()
        .unwrap_or_else(|| "origin".to_string());
    let scope_label = effective_agent_harness_scope(repo);
    let land_contract = task_land_scope_contract(scope_label == "local");
    let admission = render_effective_workflow_admission(
        repo,
        &workflow_mode,
        sprint_enabled,
        scope_label,
        &land_contract,
    );
    let plan_sync_command = if scope_label == "remote" {
        format!("ait plan sync <markdown-file-or-dir> --remote {remote_name}")
    } else {
        "ait plan sync <markdown-file-or-dir> --local".to_string()
    };
    let task_scope = if scope_label == "remote" {
        format!("remote-backed through `{remote_name}`")
    } else if scope_label == "local" {
        "local-only".to_string()
    } else {
        "the scopes reported by `ait config show`".to_string()
    };
    let markdown_sync_rule = if sprint_enabled {
        format!(
            r#"- Reconcile authored Markdown through `{plan_sync_command}`. The initial
  sprint card is the command-spelling exception: `task start --from` performs
  that exact-file Plan sync before code work. Do not hide Markdown lineage
  inside a code snapshot."#
        )
    } else {
        format!(
            r#"- Any authored Markdown change must be reconciled through
  `{plan_sync_command}`. Do not hide Markdown lineage inside a code snapshot."#
        )
    };
    let sprint_finish = if scope_label == "remote" {
        format!(
            r#"Prepare the exact Patchset with `ait workflow ready <change-id>
   --apply`, then hand it to the reviewer. The reviewer finishes with `ait
   workflow finish <change-id> --apply` (adding `--review-message` when asked).
   After successful land, mark the exact bound checklist item complete and
   sync its card separately:
   `ait plan sync <sprint-card-path> --remote {remote_name}`."#
        )
    } else {
        r#"Finish dirty work with `ait task finish <task-or-change-id> --message
   "<message>"`. If an explicit Snapshot already made the worktree clean, omit
   `--message`; finish reuses the current Line head. A successful final local
   Task finish closes and syncs the exact bound sprint checklist item locally."#
            .to_string()
    };
    let non_sprint_finish = if scope_label == "remote" {
        r#"Prepare the exact Patchset with `ait workflow ready <change-id>
   --apply`, then hand it to the reviewer. The reviewer finishes with `ait
   workflow finish <change-id> --apply` (adding `--review-message` when asked)."#
            .to_string()
    } else {
        r#"Finish dirty work with `ait task finish <task-or-change-id> --message
   "<message>"`. For clean work, omit `--message` and reuse the Line head."#
            .to_string()
    };
    let code_closeout = if scope_label == "remote" {
        "reviewer-owned `ait workflow finish <change-id> --apply`, which delegates final closeout to the atomic internal Land authority"
    } else {
        "`ait task finish <task-or-change-id>`"
    };
    let authoring_step = if scope_label == "remote" {
        r#"Enter the task worktree emitted by `task start`, author the code there,
   and create a Snapshot with `ait snapshot create --message "<message>"`."#
    } else {
        r#"Enter the task worktree emitted by `task start` and author the code there.
   `ait snapshot create --message "<message>"` remains available for optional
   intermediate checkpoints; final dirty work can be Snapshotted by Task finish."#
    };

    let sprint_path = if sprint_enabled {
        format!(
            r#"### Task path: sprint mode is on

For changes classified as `normal_task` or `fully_governed`:

1. Write a detailed Markdown sprint card under `docs/sprints/` with one stable
   `[plan-ref: ...]` root and an unchecked checklist item carrying an exact
   `[ref: ...]`.
2. Start the task and first change with `ait task start --from
   <sprint-card-path>#<exact-ref> --intent "<intent>"`.
   `task start --from` owns exact-file Plan sync in the configured scope,
   post-sync item taskability validation, canonical Plan binding, Task/Change
   creation, bound-worktree bootstrap, and the printed `cd` hint. The task is
   {task_scope}; do not run a separate pre-start Plan sync or copy Plan IDs.
3. {authoring_step}
4. {sprint_finish}

After every context-window compaction, re-read the bound sprint card before
continuing."#
        )
    } else {
        format!(
            r#"### Task path: sprint mode is off

For changes classified as `normal_task` or `fully_governed`:

1. Start a task and first change with `ait task start --title "<title>"
   --intent "<intent>"`. The task scope is {task_scope}; a
   sprint card is not required and `--from` is unavailable while sprint mode
   is off.
2. {authoring_step}
3. {non_sprint_finish}"#
        )
    };

    let closeout = if scope_label == "remote" {
        r#"### Remote readiness and land

- `task start` opens the remote task/change lineage. `snapshot create` records
  the reviewable code state.
- Prepare snapshot freshness, patchset publication/content synchronization, CI,
  and attestation explicitly with `ait workflow ready <change-id> --apply`.
- Hand the exact Patchset to the reviewer. `ait workflow finish <change-id>
  --apply` owns code Review, Task approval, final Policy, and then delegates the
  already-ready final mutation to the atomic internal Land authority. Supply `--review-message`
  when the decision requests structured code-review evidence; required human
  Review and blocking feedback remain manual stop points.
- Direct `ait task finish <task-or-change-id>` is the already-ready finalizer and
  recovery entry. It creates no Review evidence, does not publish/synchronize
  content, start/wait for CI, or sync Plan state. Success owns remote land, Task
  completion, target-Line sync, and bound-worktree cleanup."#
            .to_string()
    } else if scope_label == "local" {
        r#"### Local finish

- `task start`, its initial change, Snapshots, and `task finish` stay local unless
  a command explicitly requests remote promotion.
- `ait task finish <task-or-change-id> --message "<message>" --local` creates
  the final Snapshot for dirty work, applies it to the local target Line,
  completes the Task, cleans the bound worktree, and (when bound) closes the
  local sprint checklist item. Clean work omits `--message` and reuses the
  current Line-head Snapshot."#
            .to_string()
    } else {
        r#"### Effective custom closeout

- Run `ait config show` before mutating workflow state and follow its effective
  plan/task/change scopes explicitly.
- Use `ait task finish <task-or-change-id>` for the configured closeout path."#
            .to_string()
    };

    format!(
        r#"{MANAGED_START}
## Effective Ait Workflow (Generated)

`ait init`, relevant `ait config set`/`unset` changes, and default-remote setup
regenerate this authoritative block from `.ait/config.json` and sync its
configured target when available.

{admission}

### Rules for every repository mutation

- Read this block and `docs/plan.md` when it exists.
- When a regression is found, run `ait blame <path>` (narrow with `--line` or
  `--start`/`--end`) to identify the responsible Snapshot or Plan revision
  before choosing the repair.
{markdown_sync_rule}
- `ait workflow ready` and `ait workflow finish` are text-only decision surfaces;
  never append or recommend `--json` for either command.
- Every code change must start with a new `ait task start`, be authored in its
  bound worktree, and finish through {code_closeout}.
  There is no direct Snapshot-only closeout path.
- Prefer `ait queue summary` for current actionable inventory, `ait task list
  --all` and `ait change list --all` for history, and `ait task audit <task-id>`
  for one task's readiness.

{sprint_path}

{closeout}
{MANAGED_END}"#,
    )
}

fn replace_or_insert_managed_block(existing: &str, managed: &str) -> Result<String, String> {
    match (existing.find(MANAGED_START), existing.find(MANAGED_END)) {
        (Some(start), Some(end)) if start <= end => {
            let suffix_start = end + MANAGED_END.len();
            Ok(format!(
                "{}{}{}",
                &existing[..start],
                managed,
                &existing[suffix_start..]
            ))
        }
        (None, None) => {
            let insertion = existing.find('\n').map(|index| index + 1).unwrap_or(0);
            let mut output = String::with_capacity(existing.len() + managed.len() + 2);
            output.push_str(&existing[..insertion]);
            if insertion > 0 && !output.ends_with("\n\n") {
                output.push('\n');
            }
            output.push_str(managed);
            output.push_str("\n\n");
            output.push_str(&existing[insertion..]);
            Ok(output)
        }
        _ => Err(
            "AGENTS.md contains an incomplete ait-managed workflow block; restore both managed markers before refreshing config guidance."
                .to_string(),
        ),
    }
}

fn ensure_configured_sprint_directory(repo: &RepoRuntime) -> Result<(), String> {
    if !repo.sprint_enabled() {
        return Ok(());
    }
    create_real_directory_tree(&repo.authoritative_repo_root(), "docs/sprints")
}

fn create_real_directory_tree(base: &Path, relative: &str) -> Result<(), String> {
    require_real_directory(base, "Repository root")?;
    let mut current = base.to_path_buf();
    for component in relative.split('/') {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing symbolic-link workflow directory: {}",
                    current.display()
                ))
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!(
                    "Workflow directory path has the wrong file kind: {}",
                    current.display()
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|create_error| {
                    format!(
                        "Failed to create workflow directory {}: {create_error}",
                        current.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "Failed to inspect workflow directory {}: {error}",
                    current.display()
                ))
            }
        }
    }
    Ok(())
}

fn require_real_directory(path: &Path, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "{label} must not be a symbolic link: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_dir() => {
            Err(format!("{label} must be a directory: {}", path.display()))
        }
        Ok(_) => Ok(()),
        Err(error) => Err(format!(
            "Failed to inspect {label} {}: {error}",
            path.display()
        )),
    }
}

fn read_optional_regular_text(path: &Path, label: &str) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "Refusing symbolic-link {label}: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "{label} must be a regular file: {}",
            path.display()
        )),
        Ok(_) => fs::read_to_string(path)
            .map(Some)
            .map_err(|error| format!("Failed to read {label} {}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Failed to inspect {label} {}: {error}",
            path.display()
        )),
    }
}

fn write_text_atomically(path: &Path, content: &str, default_mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Atomic write path has no parent: {}", path.display()))?;
    require_real_directory(parent, "Atomic write parent")?;
    let existing_permissions = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "Refusing symbolic-link Agent contract: {}",
                path.display()
            ))
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!(
                "Agent contract must be a regular file: {}",
                path.display()
            ))
        }
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "Failed to inspect Agent contract {}: {error}",
                path.display()
            ))
        }
    };
    let mut staged = NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "Failed to stage atomic Agent contract write for {}: {error}",
            path.display()
        )
    })?;
    #[cfg(unix)]
    staged
        .as_file()
        .set_permissions(
            existing_permissions.unwrap_or_else(|| fs::Permissions::from_mode(default_mode)),
        )
        .map_err(|error| format!("Failed to set staged AGENTS.md permissions: {error}"))?;
    #[cfg(not(unix))]
    let _ = (existing_permissions, default_mode);
    staged
        .as_file_mut()
        .write_all(content.as_bytes())
        .and_then(|_| staged.as_file_mut().flush())
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|error| format!("Failed to stage {}: {error}", path.display()))?;
    staged.persist(path).map_err(|error| {
        format!(
            "Failed to atomically publish {}: {}",
            path.display(),
            error.error
        )
    })?;
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Failed to sync directory {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_surface::{init_repo, InitRequest};
    use ait_core::json_support::{json, JsonCodec, JsonMap, JsonValue};
    use ait_core::remote_store::{RemoteAddRecord, RemoteStore};
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn repo(mode: &str, sprint: &str) -> RepoRuntime {
        let mut config = JsonMap::new();
        config.insert("repo_name".to_string(), json!("demo"));
        config.insert("default_line".to_string(), json!("main"));
        config.insert("workflow_mode".to_string(), json!(mode));
        let scope = if matches!(mode, "solo_remote" | "team_remote") {
            "remote"
        } else {
            "local"
        };
        for key in [
            "workflow_default_scope",
            "task_default_scope",
            "change_default_scope",
        ] {
            config.insert(key.to_string(), json!(scope));
        }
        config.insert("sprint".to_string(), json!(sprint));
        config.insert(
            "plan_task_binding".to_string(),
            json!({"mode": if sprint == "on" { "required" } else { "off" }}),
        );
        config.insert("default_remote".to_string(), json!("upstream"));
        config.insert(
            "default_author_mode".to_string(),
            json!("ai_only_experimental"),
        );
        config.insert("default_model".to_string(), json!("test-model"));
        config.insert("user_name".to_string(), json!("Benchmark Agent"));
        RepoRuntime {
            root: PathBuf::from("/repo"),
            ait_dir: PathBuf::from("/repo/.ait"),
            config,
            worktree_config_path: None,
        }
    }

    #[test]
    fn renders_every_mode_and_sprint_combination_as_compact_nonconflicting_guidance() {
        for mode in ["solo_local", "solo_remote", "team_remote"] {
            for sprint in ["on", "off"] {
                let rendered = render_agent_workflow_block(&repo(mode, sprint));
                let scope = if mode == "solo_local" {
                    "local"
                } else {
                    "remote"
                };
                assert!(rendered.contains(&format!(
                    "entry: mode=`{mode}`; sprint=`{sprint}`; scopes=`{scope}`"
                )));
                assert!(rendered.contains(&format!(
                    "plan-binding=`{}`",
                    if sprint == "on" { "required" } else { "off" }
                )));
                assert!(rendered.contains(&format!("contract=`{TASK_LAND_CONTRACT_VERSION}`")));
                assert!(rendered.contains(&format!(
                    "plan-closeout=`{}`",
                    task_land_scope_contract(mode == "solo_local").plan_closeout_policy
                )));
                assert_eq!(rendered.matches(MANAGED_START).count(), 1);
                assert!(rendered.contains("`ait init`, relevant `ait config set`/`unset` changes"));
                assert!(rendered.contains("### Effective workflow admission"));
                assert!(rendered.contains("Satisfied:"));
                assert!(rendered.contains("`task start` revalidates entry"));
                assert!(rendered.contains("author-mode=`ai_only_experimental`; model=`test-model`"));
                assert!(rendered.contains("review=`automatic`; reviewer=`configured`"));
                assert!(!rendered.contains("They are currently satisfied"));
                assert!(!rendered.contains("ait install"));
                assert!(rendered.contains("regenerate this authoritative block"));
                assert!(rendered.contains("default-remote setup\nregenerate this authoritative"));
                assert!(rendered.contains("Read this block and `docs/plan.md` when it exists"));
                assert!(!rendered.contains("runtime state may have changed"));
                assert!(rendered.contains("ait blame <path>"));
                assert!(rendered.contains(
                    "`ait workflow ready` and `ait workflow finish` are text-only decision surfaces"
                ));
                assert!(rendered.contains("never append or recommend `--json`"));
                assert!(!rendered.contains("workflow tier"));
                assert!(!rendered.contains("--profile quick"));
                assert!(rendered.contains("Every code change must start"));
                assert!(rendered.contains("no direct Snapshot-only closeout path"));
                assert!(!rendered.contains("--base-line"));
                assert!(
                    rendered.split_whitespace().count() < 1_024,
                    "{mode}/{sprint} guidance exceeded the 1,024-token ceiling"
                );
                if matches!(mode, "solo_remote" | "team_remote") {
                    assert!(rendered.contains("plan sync <markdown-file-or-dir> --remote upstream"));
                    assert!(rendered.contains("does not publish/synchronize\n  content"));
                    assert!(rendered.contains("Hand the exact Patchset to the reviewer"));
                    assert!(rendered.contains("owns code Review, Task approval, final Policy"));
                    assert!(rendered.contains(
                        "delegates the\n  already-ready final mutation to the atomic internal Land authority"
                    ));
                    assert!(rendered.contains("It creates no Review evidence"));
                    assert!(!rendered.contains("attestation, policy, and review state"));
                    assert!(!rendered.contains("### Local finish"));
                } else {
                    assert!(rendered.contains("plan sync <markdown-file-or-dir> --local"));
                    assert!(!rendered.contains("### Remote readiness and land"));
                }
                if sprint == "on" {
                    assert!(rendered
                        .contains("ait task start --from\n   <sprint-card-path>#<exact-ref>"));
                    assert!(rendered.contains("owns exact-file Plan sync"));
                    assert!(rendered.contains("do not run a separate pre-start Plan sync"));
                    assert!(!rendered.contains("--plan <plan-id>"));
                    assert!(!rendered.contains("--revision"));
                    assert!(!rendered.contains("--plan-item-ref"));
                    assert!(rendered.contains("detailed Markdown sprint card"));
                    assert!(rendered.contains(
                        "After every context-window compaction, re-read the bound sprint card"
                    ));
                    if matches!(mode, "solo_remote" | "team_remote") {
                        assert!(
                            rendered.contains("ait plan sync <sprint-card-path> --remote upstream")
                        );
                    }
                } else {
                    assert!(rendered.contains("ait task start --title \"<title>\""));
                    assert!(rendered.contains("`--from` is unavailable"));
                    assert!(!rendered.contains("--plan-item-ref"));
                    assert!(!rendered.contains("context-window compaction"));
                    assert!(!rendered.contains("Plan ID"));
                }
            }
        }
    }

    #[test]
    fn binding_only_sprint_detection_matches_effective_config_contract() {
        for (binding_mode, expected_sprint) in [
            (None, "on"),
            (Some("required"), "on"),
            (Some("advisory"), "off"),
            (Some("strict"), "off"),
            (Some("off"), "off"),
        ] {
            let mut runtime = repo("solo_local", "on");
            runtime.config.remove("sprint");
            match binding_mode {
                Some(mode) => {
                    runtime
                        .config
                        .insert("plan_task_binding".to_string(), json!({"mode": mode}));
                }
                None => {
                    runtime.config.remove("plan_task_binding");
                }
            }

            let rendered = render_agent_workflow_block(&runtime);
            assert!(
                rendered.contains(&format!("sprint=`{expected_sprint}`")),
                "binding mode {binding_mode:?} rendered the wrong sprint mode"
            );
        }
    }

    #[test]
    fn managed_refresh_is_idempotent_and_preserves_surrounding_governance() {
        let first = replace_or_insert_managed_block(
            "# AGENTS\n\nCustom before.\n",
            &render_agent_workflow_block(&repo("solo_local", "on")),
        )
        .unwrap();
        let second = replace_or_insert_managed_block(
            &first,
            &render_agent_workflow_block(&repo("solo_remote", "off")),
        )
        .unwrap();
        assert!(second.contains("Custom before."));
        assert!(second.contains("entry: mode=`solo_remote`; sprint=`off`; scopes=`remote`"));
        assert_eq!(second.matches(MANAGED_START).count(), 1);
        assert_eq!(second.matches(MANAGED_END).count(), 1);
    }

    #[test]
    fn local_admission_distinguishes_absent_and_inactive_default_remotes() {
        let with_remote = render_agent_workflow_block(&repo("solo_local", "off"));
        assert!(with_remote.contains(
            "default-remote=`upstream` (inactive); transport=`local-only`; server-use=`none`"
        ));
        assert!(!with_remote.contains("configured remote is unavailable"));

        let mut without_remote = repo("solo_local", "off");
        without_remote.config.remove("default_remote");
        let rendered = render_agent_workflow_block(&without_remote);
        assert!(
            rendered.contains("default-remote=`none`; transport=`local-only`; server-use=`none`")
        );
        assert!(rendered.contains("Action required: none."));
    }

    #[test]
    fn mismatched_plan_binding_is_action_required_without_markdown_checkboxes() {
        let mut runtime = repo("solo_local", "off");
        runtime
            .config
            .insert("plan_task_binding".to_string(), json!({"mode": "required"}));
        let rendered = render_agent_workflow_block(&runtime);
        assert!(
            rendered.contains("entry: plan-binding=`required` (expected `off` for sprint=`off`)")
        );
        assert!(!rendered.contains("- [x]"));
        assert!(!rendered.contains("- [ ]"));
    }

    #[test]
    fn arbitrary_model_text_is_kept_inside_a_safe_code_span() {
        let mut runtime = repo("solo_local", "off");
        runtime.config.insert(
            "default_model".to_string(),
            json!("model`name\nwith whitespace"),
        );
        let rendered = render_agent_workflow_block(&runtime);
        assert!(rendered.contains("model=``model`name with whitespace``"));

        runtime
            .config
            .insert("default_model".to_string(), json!("`edge ticks`"));
        let rendered = render_agent_workflow_block(&runtime);
        assert!(rendered.contains("model=`` `edge ticks` ``"));
    }

    #[test]
    fn remote_convergence_uses_the_configured_default_remote_after_refresh() {
        let temp = tempdir().unwrap();
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("demo".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        let initialized = RepoRuntime::discover_from_path(temp.path()).unwrap();
        initialized
            .remote_store()
            .unwrap()
            .add_remote(&RemoteAddRecord {
                name: "mirror".to_string(),
                url: "https://example.test".to_string(),
                repo_name: Some("demo-remote".to_string()),
                make_default: true,
                created_at: "2026-07-10T00:00:00Z".to_string(),
            })
            .unwrap();
        let config_path = temp.path().join(".ait/config.json");
        let mut config = JsonCodec::parse_object(
            &fs::read_to_string(&config_path).unwrap(),
            "agent harness test config",
        )
        .unwrap();
        for (key, value) in [
            ("repo_name", json!("demo")),
            ("default_line", json!("main")),
            ("current_line", json!("main")),
            ("default_remote", json!("mirror")),
            ("workflow_mode", json!("solo_remote")),
            ("workflow_default_scope", json!("remote")),
            ("task_default_scope", json!("remote")),
            ("change_default_scope", json!("remote")),
            ("sprint", json!("on")),
            ("plan_task_binding", json!({"mode": "required"})),
            ("user_name", json!("Benchmark Agent")),
        ] {
            config.insert(key.to_string(), value);
        }
        fs::write(config_path, JsonValue::Object(config).to_string()).unwrap();
        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        let mut captured = None;

        let convergence = converge_agent_workflow_harness_with_executor(&repo, |request| {
            captured = Some(request.clone());
            Ok(json!({"status": "ok", "results": []}))
        })
        .unwrap();

        assert_eq!(convergence["status"], "synced");
        assert_eq!(convergence["scope"], "remote");
        assert_eq!(convergence["remote"], "mirror");
        let request = captured.expect("plan sync request");
        assert_eq!(request["local"], false);
        assert_eq!(request["remote_name"], "mirror");
        assert_eq!(request["remote_repo_name"], "demo-remote");
        assert_eq!(request["base_url"], "https://example.test");
        let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("--remote mirror"));
        assert!(agents.contains("default-remote=`mirror`; transport=`remote`"));
        assert!(agents.contains("Action required: none."));
    }

    #[test]
    fn remote_convergence_without_a_remote_is_explicitly_pending() {
        let temp = tempdir().unwrap();
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("demo".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .unwrap();
        fs::write(
            temp.path().join(".ait/config.json"),
            json!({
                "repo_name": "demo",
                "default_line": "main",
                "current_line": "main",
                "default_remote": null,
                "workflow_mode": "solo_remote",
                "workflow_default_scope": "remote",
                "task_default_scope": "remote",
                "change_default_scope": "remote",
                "sprint": "on",
                "plan_task_binding": {"mode": "required"},
            })
            .to_string(),
        )
        .unwrap();
        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        let mut executed = false;

        let convergence = converge_agent_workflow_harness_with_executor(&repo, |_| {
            executed = true;
            Ok(json!({"status": "ok"}))
        })
        .unwrap();

        assert_eq!(convergence["status"], "pending");
        assert_eq!(convergence["reason"], "no_default_remote");
        assert!(!executed);
        let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("entry: mode=`solo_remote`; sprint=`on`; scopes=`remote`"));
        assert!(agents.contains("default-remote=`unset` (required by `solo_remote`)"));
    }
}
