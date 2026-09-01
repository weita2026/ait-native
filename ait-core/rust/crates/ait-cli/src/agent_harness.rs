use crate::runtime::RepoRuntime;
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
const CLAUDE_HARNESS_PATH: &str = "CLAUDE.md";
const LEGACY_CLAUDE_POINTER_BODIES: [&str; 2] = [
    "# CLAUDE\n\nThis repository's agent guidance lives in the file imported below; this\npointer exists because Claude Code auto-loads CLAUDE.md but not AGENTS.md.\n\n@AGENTS.md\n",
    "# CLAUDE\n\nThis repository's workflow rules live in the file imported below. Read it as\nauthoritative; this file only imports it and never restates its content.\n\n@AGENTS.md\n",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuidanceAudience {
    Agents,
    Claude,
}

pub fn refresh_agent_workflow_harness(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let (agents, agents_changed, _) = refresh_workflow_document(
        repo,
        AGENT_HARNESS_PATH,
        "AGENTS",
        "Agent contract",
        &render_agent_workflow_block(repo),
        &[],
    )?;
    let (claude, claude_changed, claude_existed) = refresh_workflow_document(
        repo,
        CLAUDE_HARNESS_PATH,
        "CLAUDE",
        "Claude agent contract",
        &render_claude_workflow_block(repo),
        &LEGACY_CLAUDE_POINTER_BODIES,
    )?;
    let changed = agents_changed || claude_changed;
    Ok(json!({
        "status": if changed { "updated" } else { "unchanged" },
        "changed": changed,
        "artifact_path": AGENT_HARNESS_PATH,
        "artifact_paths": [AGENT_HARNESS_PATH, CLAUDE_HARNESS_PATH],
        "path": repo.authoritative_repo_root().join(AGENT_HARNESS_PATH),
        "artifacts": [agents, claude],
        // Compatibility field retained for consumers of the previous pointer
        // receipt. It now reports only whether CLAUDE.md predated this refresh.
        "claude_pointer": if claude_existed { "existing" } else { "created" },
    }))
}

fn refresh_workflow_document(
    repo: &RepoRuntime,
    artifact_path: &str,
    heading: &str,
    label: &str,
    managed: &str,
    legacy_bodies: &[&str],
) -> Result<(JsonValue, bool, bool), String> {
    let path = repo.authoritative_repo_root().join(artifact_path);
    let original = read_optional_regular_text(&path, label)?;
    let existed = original.is_some();
    let existing = match original.as_deref() {
        Some(body) => strip_legacy_pointer_bodies(body, heading, legacy_bodies),
        None => format!("# {heading}\n"),
    };
    let updated = replace_or_insert_managed_block(&existing, managed, artifact_path)?;
    let changed = original.as_deref() != Some(updated.as_str());
    if changed {
        write_text_atomically(&path, &updated, 0o644)?;
    }
    let status = if !existed {
        "created"
    } else if changed {
        "updated"
    } else {
        "unchanged"
    };
    Ok((
        json!({
            "artifact_path": artifact_path,
            "path": path,
            "status": status,
            "changed": changed,
        }),
        changed,
        existed,
    ))
}

fn strip_legacy_pointer_bodies(body: &str, heading: &str, legacy_bodies: &[&str]) -> String {
    let mut cleaned = body.to_string();
    for legacy_body in legacy_bodies {
        let Some((legacy_heading, legacy_payload)) = legacy_body.split_once("\n\n") else {
            continue;
        };
        if legacy_heading != format!("# {heading}") {
            continue;
        }
        while let Some(start) = cleaned.find(legacy_payload) {
            cleaned.replace_range(start..start + legacy_payload.len(), "");
        }
    }
    if cleaned.trim() == format!("# {heading}") {
        format!("# {heading}\n")
    } else {
        cleaned
    }
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
    let mut plan_syncs = Vec::new();
    let mut agents_plan_sync = JsonValue::Null;
    for artifact_path in [AGENT_HARNESS_PATH, CLAUDE_HARNESS_PATH] {
        let request = agent_harness_plan_sync_request(repo, artifact_path, remote_name.as_deref())?;
        let plan_sync = execute(&request)?;
        if plan_sync.get("status").and_then(JsonValue::as_str) != Some("ok") {
            let error = plan_sync
                .get("error")
                .and_then(JsonValue::as_str)
                .unwrap_or("plan sync returned a non-ok result");
            return Err(format!(
                "Generated {artifact_path} was refreshed, but automatic {scope} plan sync failed: {error}"
            ));
        }
        if artifact_path == AGENT_HARNESS_PATH {
            agents_plan_sync = plan_sync.clone();
        }
        plan_syncs.push(json!({
            "artifact_path": artifact_path,
            "result": plan_sync,
        }));
    }
    Ok(json!({
        "status": "synced",
        "scope": scope,
        "remote": remote_name,
        "artifact_path": AGENT_HARNESS_PATH,
        "artifact_paths": [AGENT_HARNESS_PATH, CLAUDE_HARNESS_PATH],
        "refresh": refresh,
        // Preserve the admitted AGENTS.md receipt while adding the complete
        // two-artifact inventory for direct consumers.
        "plan_sync": agents_plan_sync,
        "plan_syncs": plan_syncs,
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
        "next_action": "Connect or repair the default remote; the default-remote mutation path will retry AGENTS.md and CLAUDE.md convergence automatically.",
    })
}

fn agent_harness_plan_sync_request(
    repo: &RepoRuntime,
    artifact_path: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let mut payload = json!({
        "root_path": repo.authoritative_repo_root(),
        "repo_name": repo.repo_name(),
        "repository_index": repo.repository_index(),
        "id_namespace_prefix": repo.id_namespace_prefix(),
        "created_by": repo.actor_identity(),
        "target": artifact_path,
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
) -> String {
    let mut action_required = Vec::new();
    let sprint = if sprint_enabled { "on" } else { "off" };
    let binding = effective_plan_binding_mode(repo, sprint_enabled);
    let expected_binding = if sprint_enabled { "required" } else { "off" };

    if workflow_mode == "custom" {
        action_required.push(format!(
            "- mode={}; scope={} (unsupported workflow combination)",
            markdown_code_span(workflow_mode),
            markdown_code_span(scope_label),
        ));
    }
    let binding_fact = format!("plan-binding={}", markdown_code_span(&binding));
    if binding != expected_binding {
        action_required.push(format!(
            "- {binding_fact} (expected {} for sprint={})",
            markdown_code_span(expected_binding),
            markdown_code_span(sprint),
        ));
    }

    let default_remote = repo.default_remote_name();
    let route_remote = if scope_label == "remote" {
        match default_remote.as_deref() {
            Some(name) if repo.remote_row(Some(name)).is_ok() => Some(markdown_code_span(name)),
            Some(name) => {
                action_required.push(format!(
                    "- remote={} (configured remote is unavailable)",
                    markdown_code_span(name),
                ));
                Some(markdown_code_span(name))
            }
            None => {
                action_required.push(format!(
                    "- remote={} (required by {})",
                    markdown_code_span("unset"),
                    markdown_code_span(workflow_mode),
                ));
                Some(markdown_code_span("unset"))
            }
        }
    } else {
        None
    };

    let author_mode = repo.effective_author_mode(None);
    if repo.task_review_reviewer_identity().is_none() {
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
        action_required.push(format!(
            "- review={}; reviewer={} (configure user-name)",
            markdown_code_span(task_review),
            markdown_code_span("unset"),
        ));
    }

    let mut route = format!(
        "Route: mode={}; sprint={}; scope={}; {binding_fact}; author-mode={}",
        markdown_code_span(workflow_mode),
        markdown_code_span(sprint),
        markdown_code_span(scope_label),
        markdown_code_span(&author_mode),
    );
    if let Some(remote) = route_remote {
        route.push_str(&format!("; remote={remote}"));
    }
    route.push('.');

    let admission = if action_required.is_empty() {
        r#"Admission: ready. Do not inspect configuration unless an AIT command
reports `action_required` or the task explicitly changes configuration."#
            .to_string()
    } else {
        format!(
            "Action required before mutation:\n\n{}",
            action_required.join("\n")
        )
    };
    format!(
        r#"### Effective route

{route}

{admission}"#,
    )
}

pub fn render_agent_workflow_block(repo: &RepoRuntime) -> String {
    render_workflow_block(repo, GuidanceAudience::Agents)
}

fn render_claude_workflow_block(repo: &RepoRuntime) -> String {
    render_workflow_block(repo, GuidanceAudience::Claude)
}

fn render_workflow_block(repo: &RepoRuntime, audience: GuidanceAudience) -> String {
    let workflow_mode = repo.effective_workflow_mode();
    let sprint_enabled = repo.sprint_enabled();
    let remote_name = repo
        .default_remote_name()
        .unwrap_or_else(|| "origin".to_string());
    let scope_label = effective_agent_harness_scope(repo);
    let admission =
        render_effective_workflow_admission(repo, &workflow_mode, sprint_enabled, scope_label);
    let plan_sync_command = if scope_label == "remote" {
        format!("ait plan sync <markdown-file-or-dir> --remote {remote_name}")
    } else {
        "ait plan sync <markdown-file-or-dir> --local".to_string()
    };
    let markdown_sync_rule = if sprint_enabled {
        format!(
            r#"- Sync authored Markdown other than the initial sprint card with
  `{plan_sync_command}`; do not hide Markdown lineage in a code Snapshot."#
        )
    } else {
        format!(
            r#"- Sync authored Markdown with `{plan_sync_command}`; do not hide
  Markdown lineage in a code Snapshot."#
        )
    };
    let local_finish = r#"For dirty work, run `ait task finish
   <task-or-change-id> --message "<message>" --local`; when already
   clean, omit `--message`. Successful Task finish output is authoritative
   proof of local apply, Task completion, worktree cleanup, and
   applicable bound-card closeout. Do not follow it with `status`, `diff`, or `audit`
   unless it fails, reports required action, state is unexpected, or evidence
   was requested."#;
    let remote_finish = r#"Create the reviewable Snapshot with `ait snapshot create --message
   "<message>"`, then run `ait workflow ready <change-id> --apply`. Give the
   exact Patchset to the reviewer; the reviewer runs `ait workflow finish
   <change-id> --apply` (and `--review-message` when requested).
   Workflow finish owns Review, approval, final Policy, and atomic Task closeout.
   Use direct `ait task finish` only as an already-ready finalizer or a reported
   recovery command; it creates no Review evidence, publishes no content, runs no
   CI, and closes no Plan."#;

    let (task_start_verb, edit_root_argument, enter_worktree_suffix) = match audience {
        GuidanceAudience::Agents => ("Run", "", ""),
        GuidanceAudience::Claude => (
            "Select a safe absolute, absent-or-empty Task worktree path outside the\n   canonical repository, then run",
            " --edit-root <absolute-path>",
            " && cd <absolute-path>",
        ),
    };

    let task_path = match (sprint_enabled, scope_label) {
        (true, "remote") => format!(
            r#"### Code-change path

1. Create a detailed card under `docs/sprints/` with one stable
   `[plan-ref: ...]` and one unchecked item carrying an exact `[ref: ...]`.
2. {task_start_verb} `ait task start --from <sprint-card-path>#<exact-ref> --intent
   "<intent>"{edit_root_argument} --remote {remote_name}{enter_worktree_suffix}`. `--from` syncs and binds the
   initial card; do not pre-sync it or copy Plan IDs.
3. Work only in the returned `edit_root`.
4. {remote_finish}
5. After a successful finish, mark the bound item complete and run `ait plan sync
   <sprint-card-path> --remote {remote_name}`.

After every context-window compaction, re-read the bound sprint card before
continuing."#
        ),
        (true, _) => format!(
            r#"### Code-change path

1. Create a detailed card under `docs/sprints/` with one stable
   `[plan-ref: ...]` and one unchecked item carrying an exact `[ref: ...]`.
2. {task_start_verb} `ait task start --from <sprint-card-path>#<exact-ref> --intent
   "<intent>"{edit_root_argument}{enter_worktree_suffix}`. `--from` syncs and binds the initial card; do not
   pre-sync it or copy Plan IDs.
3. Work only in the returned `edit_root`. Intermediate `ait snapshot create
   --message "<message>"` checkpoints are optional.
4. {local_finish}

After every context-window compaction, re-read the bound sprint card before
continuing."#
        ),
        (false, "remote") => format!(
            r#"### Code-change path

1. {task_start_verb} `ait task start --title "<title>" --intent "<intent>"{edit_root_argument} --remote
   {remote_name}{enter_worktree_suffix}`; sprint mode is off, so `--from` is unavailable.
2. Work only in the returned `edit_root`.
3. {remote_finish}"#
        ),
        (false, _) => format!(
            r#"### Code-change path

1. {task_start_verb} `ait task start --title "<title>" --intent "<intent>"{edit_root_argument}{enter_worktree_suffix}`; sprint
   mode is off, so `--from` is unavailable.
2. Work only in the returned `edit_root`. Intermediate `ait snapshot create
   --message "<message>"` checkpoints are optional.
3. {local_finish}"#
        ),
    };
    let edit_root_guidance = match audience {
        GuidanceAudience::Agents => {
            r#"If the caller already chose a safe absolute worktree path, add `--edit-root
<absolute-path>` to Task start; otherwise omit it and use the returned `edit_root`."#
        }
        GuidanceAudience::Claude => {
            r#"The two `<absolute-path>` values must be identical. Do not omit `--edit-root`;
retain the returned Task ID and verify that its `edit_root` is the selected path."#
        }
    };
    let task_path = format!("{task_path}\n\n{edit_root_guidance}");

    format!(
        r#"{MANAGED_START}
## Effective Ait Workflow (Generated)

{admission}

{task_path}

### Conditional references

- Read `docs/plan.md` when it exists.
- For a regression, use `ait blame <path>` before choosing a repair.
{markdown_sync_rule}
- A Snapshot is a checkpoint, not a substitute for the listed closeout.
- Only when that question arises: `ait queue summary` shows actionable work,
  `ait task audit <task-id>` shows readiness, and `ait task list --all` plus
  `ait change list --all` show history.
{MANAGED_END}"#,
    )
}

fn replace_or_insert_managed_block(
    existing: &str,
    managed: &str,
    artifact_path: &str,
) -> Result<String, String> {
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
        _ => Err(format!(
            "{artifact_path} contains an incomplete ait-managed workflow block; restore both managed markers before refreshing config guidance."
        )),
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
                let remote = matches!(mode, "solo_remote" | "team_remote");
                let scope = if remote { "remote" } else { "local" };
                assert!(rendered.contains(&format!(
                    "Route: mode=`{mode}`; sprint=`{sprint}`; scope=`{scope}`"
                )));
                assert!(rendered.contains(&format!(
                    "plan-binding=`{}`",
                    if sprint == "on" { "required" } else { "off" }
                )));
                assert!(rendered.contains("author-mode=`ai_only_experimental`"));
                assert_eq!(rendered.matches(MANAGED_START).count(), 1);
                assert!(rendered.contains("### Effective route"));
                if remote {
                    assert!(rendered.contains("Action required before mutation:"));
                    assert!(rendered.contains("configured remote is unavailable"));
                } else {
                    assert!(rendered.contains("Admission: ready."));
                    assert!(rendered.contains("reports `action_required`"));
                }
                assert!(!rendered.contains("Satisfied:"));
                assert!(!rendered.contains("model=`test-model`"));
                assert!(!rendered.contains("reviewer=`configured`"));
                assert!(!rendered.contains("task-land-plan-closeout"));
                assert!(
                    !rendered.to_ascii_lowercase().contains("land"),
                    "{mode}/{sprint} guidance must use finish terminology"
                );
                assert!(!rendered.contains("plan-closeout="));
                assert!(!rendered.contains("ait install"));
                assert!(!rendered.contains("regenerate this authoritative block"));
                assert!(rendered.contains("Read `docs/plan.md` when it exists"));
                assert!(rendered.contains("ait blame <path>"));
                assert!(!rendered.contains("workflow tier"));
                assert!(!rendered.contains("--profile quick"));
                assert!(rendered.contains("### Code-change path"));
                assert!(rendered.contains("A Snapshot is a checkpoint, not a substitute"));
                assert!(!rendered.contains("--base-line"));
                assert!(
                    !rendered.to_ascii_lowercase().contains("json"),
                    "{mode}/{sprint} guidance must not mention JSON output"
                );
                let (byte_limit, word_limit) = match (remote, sprint) {
                    (false, "on") => (2_600, 340),
                    (false, _) => (2_200, 285),
                    (true, "on") => (3_200, 420),
                    (true, _) => (2_800, 365),
                };
                assert!(
                    rendered.len() < byte_limit,
                    "{mode}/{sprint} guidance was {} bytes; limit is {byte_limit}",
                    rendered.len()
                );
                assert!(
                    rendered.split_whitespace().count() < word_limit,
                    "{mode}/{sprint} guidance was {} words; limit is {word_limit}",
                    rendered.split_whitespace().count()
                );
                if remote {
                    assert!(rendered.contains("remote=`upstream`"));
                    assert!(rendered.contains("plan sync <markdown-file-or-dir> --remote upstream"));
                    assert!(rendered.contains("ait workflow ready <change-id> --apply"));
                    assert!(rendered.contains("ait workflow finish\n   <change-id> --apply"));
                    assert!(rendered.contains("Workflow finish owns Review, approval"));
                    assert!(rendered.contains("atomic Task closeout"));
                    assert!(!rendered.contains("atomic land"));
                    assert!(!rendered.contains("After land"));
                    assert!(rendered.contains("it creates no Review evidence"));
                    assert!(!rendered.contains("--local"));
                    assert!(!rendered.contains("authoritative proof of local apply"));
                } else {
                    assert!(!rendered.contains("remote=`upstream`"));
                    assert!(rendered.contains("plan sync <markdown-file-or-dir> --local"));
                    assert!(!rendered.contains("ait workflow ready"));
                    assert!(!rendered.contains("ait workflow finish"));
                    assert!(rendered.contains("--local`"));
                    assert!(rendered.contains("Successful Task finish output is"));
                    assert!(rendered.contains("Do not follow it with `status`, `diff`, or `audit`"));
                }
                if sprint == "on" {
                    assert!(rendered
                        .contains("ait task start --from <sprint-card-path>#<exact-ref> --intent"));
                    assert!(rendered.contains("`. `--from` syncs and binds"));
                    assert!(rendered.contains("pre-sync it or copy Plan IDs"));
                    assert!(!rendered.contains("--plan <plan-id>"));
                    assert!(!rendered.contains("--revision"));
                    assert!(!rendered.contains("--plan-item-ref"));
                    assert!(rendered.contains("Create a detailed card under `docs/sprints/`"));
                    assert!(rendered.contains(
                        "After every context-window compaction, re-read the bound sprint card"
                    ));
                    if remote {
                        assert!(rendered
                            .contains("ait plan sync\n   <sprint-card-path> --remote upstream"));
                    }
                } else {
                    assert!(rendered.contains("ait task start --title \"<title>\""));
                    assert!(rendered.contains("mode is off, so `--from` is unavailable"));
                    assert!(rendered.contains("`--from` is unavailable"));
                    assert!(!rendered.contains("--plan-item-ref"));
                    assert!(!rendered.contains("context-window compaction"));
                    assert!(!rendered.contains("Plan ID"));
                }
                assert!(rendered.contains("otherwise omit it and use the returned `edit_root`"));

                let claude = render_claude_workflow_block(&repo(mode, sprint));
                let shared_route =
                    format!("Route: mode=`{mode}`; sprint=`{sprint}`; scope=`{scope}`");
                for shared in [
                    shared_route.as_str(),
                    "## Effective Ait Workflow (Generated)",
                    "### Code-change path",
                    "### Conditional references",
                    "Read `docs/plan.md` when it exists",
                ] {
                    assert!(claude.contains(shared), "missing shared guidance: {shared}");
                }
                assert_eq!(claude.matches(MANAGED_START).count(), 1);
                assert_eq!(claude.matches(MANAGED_END).count(), 1);
                assert!(claude.contains(
                    "Select a safe absolute, absent-or-empty Task worktree path outside the"
                ));
                assert!(claude.contains("--edit-root <absolute-path>"));
                assert!(claude.contains("&& cd <absolute-path>"));
                assert!(
                    !claude.to_ascii_lowercase().contains("json"),
                    "{mode}/{sprint} Claude guidance must not mention JSON output"
                );
                assert!(claude.contains("Do not omit `--edit-root`"));
                assert!(!claude.contains("otherwise omit it"));
                assert!(!claude.contains("@AGENTS.md"));
                assert!(claude.len() < byte_limit + 400);
                assert!(claude.split_whitespace().count() < word_limit + 55);
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
            AGENT_HARNESS_PATH,
        )
        .unwrap();
        let second = replace_or_insert_managed_block(
            &first,
            &render_agent_workflow_block(&repo("solo_remote", "off")),
            AGENT_HARNESS_PATH,
        )
        .unwrap();
        assert!(second.contains("Custom before."));
        assert!(second.contains("Route: mode=`solo_remote`; sprint=`off`; scope=`remote`"));
        assert_eq!(second.matches(MANAGED_START).count(), 1);
        assert_eq!(second.matches(MANAGED_END).count(), 1);
    }

    #[test]
    fn local_route_omits_absent_and_inactive_default_remotes() {
        let with_remote = render_agent_workflow_block(&repo("solo_local", "off"));
        assert!(!with_remote.contains("remote=`upstream`"));
        assert!(!with_remote.contains("configured remote is unavailable"));

        let mut without_remote = repo("solo_local", "off");
        without_remote.config.remove("default_remote");
        let rendered = render_agent_workflow_block(&without_remote);
        assert!(!rendered.contains("remote=`none`"));
        assert!(rendered.contains("Admission: ready."));
    }

    #[test]
    fn mismatched_plan_binding_is_action_required_without_markdown_checkboxes() {
        let mut runtime = repo("solo_local", "off");
        runtime
            .config
            .insert("plan_task_binding".to_string(), json!({"mode": "required"}));
        let rendered = render_agent_workflow_block(&runtime);
        assert!(rendered.contains("- plan-binding=`required` (expected `off` for sprint=`off`)"));
        assert!(rendered.contains("Action required before mutation:"));
        assert!(!rendered.contains("- [x]"));
        assert!(!rendered.contains("- [ ]"));
    }

    #[test]
    fn arbitrary_author_mode_text_is_kept_inside_a_safe_code_span() {
        let mut runtime = repo("solo_local", "off");
        runtime.config.insert(
            "default_author_mode".to_string(),
            json!("mode`name\nwith whitespace"),
        );
        let rendered = render_agent_workflow_block(&runtime);
        assert!(rendered.contains("author-mode=``mode`name with whitespace``"));

        runtime
            .config
            .insert("default_author_mode".to_string(), json!("`edge ticks`"));
        let rendered = render_agent_workflow_block(&runtime);
        assert!(rendered.contains("author-mode=`` `edge ticks` ``"));
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
        let mut captured = Vec::new();

        let convergence = converge_agent_workflow_harness_with_executor(&repo, |request| {
            captured.push(request.clone());
            Ok(json!({"status": "ok", "results": []}))
        })
        .unwrap();

        assert_eq!(convergence["status"], "synced");
        assert_eq!(convergence["scope"], "remote");
        assert_eq!(convergence["remote"], "mirror");
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0]["target"], AGENT_HARNESS_PATH);
        assert_eq!(captured[1]["target"], CLAUDE_HARNESS_PATH);
        for request in captured {
            assert_eq!(request["local"], false);
            assert_eq!(request["remote_name"], "mirror");
            assert_eq!(request["remote_repo_name"], "demo-remote");
            assert_eq!(request["base_url"], "https://example.test");
        }
        assert_eq!(convergence["plan_syncs"].as_array().unwrap().len(), 2);
        let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("--remote mirror"));
        assert!(agents.contains("remote=`mirror`"));
        assert!(agents.contains("Admission: ready."));
        let claude = fs::read_to_string(temp.path().join("CLAUDE.md")).unwrap();
        assert!(claude.contains("--remote mirror"));
        assert!(claude.contains("--edit-root <absolute-path>"));
        assert!(!claude.contains("@AGENTS.md"));
    }

    #[test]
    fn claude_mirror_is_created_migrated_and_preserves_user_content() {
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
        let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
        let claude_path = temp.path().join("CLAUDE.md");

        // init_repo itself runs the harness refresh, so remove the mirror to
        // exercise fresh creation explicitly.
        if claude_path.exists() {
            fs::remove_file(&claude_path).unwrap();
        }
        let first = refresh_agent_workflow_harness(&repo).unwrap();
        assert_eq!(first["claude_pointer"], "created");
        let body = fs::read_to_string(&claude_path).unwrap();
        assert!(body.contains("## Effective Ait Workflow (Generated)"));
        assert!(body.contains("--edit-root <absolute-path>"));
        assert!(body.contains("&& cd <absolute-path>"));
        assert!(!body.contains("@AGENTS.md"));

        // Idempotent: a second refresh reports existing and changes nothing.
        let second = refresh_agent_workflow_harness(&repo).unwrap();
        assert_eq!(second["claude_pointer"], "existing");
        assert_eq!(second["status"], "unchanged");
        assert_eq!(fs::read_to_string(&claude_path).unwrap(), body);

        // User-owned content survives while the managed mirror is inserted.
        fs::write(
            &claude_path,
            "# My own instructions\n\nKeep this Claude-specific rule.\n",
        )
        .unwrap();
        let third = refresh_agent_workflow_harness(&repo).unwrap();
        assert_eq!(third["claude_pointer"], "existing");
        let with_user_content = fs::read_to_string(&claude_path).unwrap();
        assert!(with_user_content.starts_with("# My own instructions\n"));
        assert!(with_user_content.contains("Keep this Claude-specific rule."));
        assert_eq!(with_user_content.matches(MANAGED_START).count(), 1);
        let fourth = refresh_agent_workflow_harness(&repo).unwrap();
        assert_eq!(fourth["status"], "unchanged");
        assert_eq!(fs::read_to_string(&claude_path).unwrap(), with_user_content);

        // Every shipped AIT-created legacy pointer is replaced rather than
        // kept as duplicate or dangling guidance.
        for legacy_body in LEGACY_CLAUDE_POINTER_BODIES {
            fs::write(&claude_path, legacy_body).unwrap();
            let migrated = refresh_agent_workflow_harness(&repo).unwrap();
            assert_eq!(migrated["claude_pointer"], "existing");
            let migrated_body = fs::read_to_string(&claude_path).unwrap();
            assert!(migrated_body.starts_with("# CLAUDE\n"));
            assert!(migrated_body.contains("## Effective Ait Workflow (Generated)"));
            assert!(!migrated_body.contains("@AGENTS.md"));
            assert!(!migrated_body.contains("pointer exists because"));
            assert!(!migrated_body.contains("only imports it"));
        }

        // Repair the intermediate state produced when an unrecognized legacy
        // pointer was preserved below an already inserted managed block.
        let deployed_payload = LEGACY_CLAUDE_POINTER_BODIES[1]
            .split_once("\n\n")
            .unwrap()
            .1;
        fs::write(
            &claude_path,
            format!(
                "# CLAUDE\n\n{}\n\n{deployed_payload}",
                render_claude_workflow_block(&repo)
            ),
        )
        .unwrap();
        refresh_agent_workflow_harness(&repo).unwrap();
        let repaired = fs::read_to_string(&claude_path).unwrap();
        assert_eq!(repaired.matches(MANAGED_START).count(), 1);
        assert!(!repaired.contains("@AGENTS.md"));

        // Removing a known AIT pointer does not remove surrounding user-owned
        // instructions or arbitrary references with different prose.
        fs::write(
            &claude_path,
            format!("# CLAUDE\n\nKeep my rule.\n\n{deployed_payload}"),
        )
        .unwrap();
        refresh_agent_workflow_harness(&repo).unwrap();
        let preserved = fs::read_to_string(&claude_path).unwrap();
        assert!(preserved.contains("Keep my rule."));
        assert!(!preserved.contains("only imports it"));
        assert!(!preserved.contains("@AGENTS.md"));

        fs::write(
            &claude_path,
            "# CLAUDE\n\nUse @AGENTS.md as optional background for this custom rule.\n",
        )
        .unwrap();
        refresh_agent_workflow_harness(&repo).unwrap();
        let arbitrary_reference = fs::read_to_string(&claude_path).unwrap();
        assert!(arbitrary_reference
            .contains("Use @AGENTS.md as optional background for this custom rule."));
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
        assert!(agents.contains("Route: mode=`solo_remote`; sprint=`on`; scope=`remote`"));
        assert!(agents.contains("remote=`unset` (required by `solo_remote`)"));
    }
}
