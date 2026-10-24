use crate::plan_preferences::PlanPreferences;
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
const TEAM_REVIEW_GUIDANCE: &str = "\n\nFresh team candidates default to independent agent code review through\n`ai_only_experimental`. A valid explicit author mode takes precedence.\nKeep the saved solo default and every sealed candidate's original mode and\nidentity. Use an independent Reviewer; a missing Reviewer requires assignment.\nReal CI, Task approval and mandatory Policy still apply. Do not fabricate human\napproval or add per-batch human approval solely because of the saved default.";
const GUIDANCE_REUSE: &str = "\
Use complete applicable guidance already in context without fetching it again.
A new Task in the same repository and scope needs its new card and necessary
references, not unchanged instructions. Read missing, changed or newly applicable
guidance once. After context compaction, re-read applicable repository guidance
and the current work order. Existing ownership and finish gates still apply.";
const TOOL_DISCIPLINE: &str = "\
Before a query, identify the missing fact and reuse authoritative facts still
valid in context. Batch independent searches and reads; filter results before
returning them to the model. After truncation, retrieve only the missing segment.";
const WORKSPACE_IGNORE_GUIDANCE: &str = "\
Before the first build/test or Snapshot-producing finish, review `.aitignore`
in `edit_root`. AIT does not automatically load `.gitignore`. Exclude only
confirmed generated/cache/local-secret paths, not broad data categories.
New rules can exclude previously snapshotted paths from new Snapshots;
inspect affected paths before capture. Old Snapshot history remains intact.";
/// Facts an agent otherwise learns from a refusal. Kept short and stable so
/// every session reads them once at start; the sync command follows the
/// effective scope.
pub fn render_worktree_facts(scope_label: &str, remote_name: &str) -> String {
    let (sync_command, finish_clause) = if scope_label == "remote" {
        (
            format!("ait plan sync <path> --remote {remote_name}"),
            String::new(),
        )
    } else {
        (
            "ait plan sync <path> --local".to_string(),
            "; `ait task finish` refuses unsynced drift unless given\n  `--sync-plan-markdown`"
                .to_string(),
        )
    };
    let quick_rule =
        "`ait task quick --intent \"<intent>\" --edit-root <absolute-path>` covers small changes.";
    format!(
        r#"### Worktree facts

- `docs/` in a Task worktree is a shared link to the repository root.
- Plan-backed Markdown moves through `{sync_command}`, never code
  Snapshots{finish_clause}.
- Plan-backed Markdown outside `docs/` is absent from worktrees; edit it at the root.
- One Markdown file may bind several Plans; `ait plan list` shows `head_match`.
- {quick_rule}
- `hooks.post_finish` (see `ait config show`) runs after each successful local finish."#
    )
}
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
    let agents_block = render_agent_workflow_block(repo)?;
    let claude_block = render_claude_workflow_block(repo)?;
    let (agents, agents_changed, _) = refresh_workflow_document(
        repo,
        AGENT_HARNESS_PATH,
        "AGENTS",
        "Agent contract",
        &agents_block,
        &[],
    )?;
    let (claude, claude_changed, claude_existed) = refresh_workflow_document(
        repo,
        CLAUDE_HARNESS_PATH,
        "CLAUDE",
        "Claude agent contract",
        &claude_block,
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

/// Import only fills absent guidance. Explicit init/config convergence keeps
/// its existing refresh semantics; import must not replace custom or old pins.
pub(crate) fn validate_import_guidance(repo: &RepoRuntime) -> Result<(), String> {
    render_agent_workflow_block(repo)?;
    render_claude_workflow_block(repo)?;
    for name in [AGENT_HARNESS_PATH, CLAUDE_HARNESS_PATH] {
        if let Some(body) =
            read_optional_regular_text(&repo.authoritative_repo_root().join(name), name)?
        {
            replace_or_insert_managed_block(&body, "", name)?;
            if body.matches(MANAGED_START).count() > 1 || body.matches(MANAGED_END).count() > 1 {
                return Err(format!("{name} contains duplicate ait-managed workflow blocks; existing content was preserved"));
            }
        }
    }
    Ok(())
}

pub(crate) fn create_import_guidance_if_absent(
    repo: &RepoRuntime,
) -> Result<Vec<(String, bool)>, String> {
    let mut result = Vec::new();
    for (path, heading, managed) in [
        (
            AGENT_HARNESS_PATH,
            "AGENTS",
            render_agent_workflow_block(repo)?,
        ),
        (
            CLAUDE_HARNESS_PATH,
            "CLAUDE",
            render_claude_workflow_block(repo)?,
        ),
    ] {
        let body = replace_or_insert_managed_block(&format!("# {heading}\n"), &managed, path)?;
        let created = crate::init_surface::write_workspace_file_if_absent(
            &repo.authoritative_repo_root().join(path),
            body.as_bytes(),
            0o644,
        )?;
        result.push((path.to_string(), created));
    }
    validate_import_guidance(repo)?;
    Ok(result)
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
        PlanPreferences::for_repo(&root_repo)?;
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
    let scope = "local";
    let mut plan_syncs = Vec::new();
    let mut agents_plan_sync = JsonValue::Null;
    for artifact_path in [AGENT_HARNESS_PATH, CLAUDE_HARNESS_PATH] {
        let request = agent_harness_plan_sync_request(repo, artifact_path)?;
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
        "remote": JsonValue::Null,
        "artifact_path": AGENT_HARNESS_PATH,
        "artifact_paths": [AGENT_HARNESS_PATH, CLAUDE_HARNESS_PATH],
        "refresh": refresh,
        // Preserve the admitted AGENTS.md receipt while adding the complete
        // two-artifact inventory for direct consumers.
        "plan_sync": agents_plan_sync,
        "plan_syncs": plan_syncs,
    }))
}

fn agent_harness_plan_sync_request(
    repo: &RepoRuntime,
    artifact_path: &str,
) -> Result<JsonValue, String> {
    Ok(json!({
        "root_path": repo.authoritative_repo_root(),
        "repo_name": repo.repo_name(),
        "repository_index": repo.repository_index(),
        "id_namespace_prefix": repo.id_namespace_prefix(),
        "created_by": repo.actor_identity(),
        "target": artifact_path,
        "plan_ref": JsonValue::Null,
        "prune": false,
        "local": true,
        "remote_name": JsonValue::Null,
        "remote_repo_name": JsonValue::Null,
        "base_url": JsonValue::Null,
        "rebase": false,
        "reconcile": false,
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    }))
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

fn render_effective_workflow_admission(repo: &RepoRuntime, sprint_enabled: bool) -> String {
    let mut action_required = Vec::new();
    let sprint = if sprint_enabled { "on" } else { "off" };
    let binding = effective_plan_binding_mode(repo, sprint_enabled);
    let expected_binding = if sprint_enabled { "required" } else { "off" };

    let binding_fact = format!("plan-binding={}", markdown_code_span(&binding));
    if binding != expected_binding {
        action_required.push(format!(
            "- {binding_fact} (expected {} for sprint={})",
            markdown_code_span(expected_binding),
            markdown_code_span(sprint),
        ));
    }

    let author_mode = repo.effective_author_mode(None);

    let route = format!(
        "Route: sprint={}; scope=`local`; {binding_fact}; author-mode={}.",
        markdown_code_span(sprint),
        markdown_code_span(&author_mode),
    );
    let review = if repo.team_review_enabled() {
        TEAM_REVIEW_GUIDANCE
    } else {
        ""
    };

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

{route}{review}

{admission}"#,
    )
}

pub fn render_agent_workflow_block(repo: &RepoRuntime) -> Result<String, String> {
    render_workflow_block(repo, GuidanceAudience::Agents)
}

fn render_claude_workflow_block(repo: &RepoRuntime) -> Result<String, String> {
    render_workflow_block(repo, GuidanceAudience::Claude)
}

fn render_workflow_block(repo: &RepoRuntime, audience: GuidanceAudience) -> Result<String, String> {
    let prose_guidance = PlanPreferences::for_repo(repo)?.guidance();
    let sprint_enabled = repo.sprint_enabled();
    // Shared command routing remains local; team review affects fresh author mode.
    let worktree_facts = render_worktree_facts("local", "");
    let admission = render_effective_workflow_admission(repo, sprint_enabled);
    let plan_sync_command = "ait plan sync <markdown-file-or-dir> --local";
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
   <task-id> --message "<message>" --local`; if clean, omit `--message`.
   Successful Task finish output is authoritative proof of local apply,
   Task completion, worktree cleanup, and bound-card closeout.
   Do not follow it with `status`, `diff`, or `audit`
   except for failure, required action, unexpected state, or requested evidence."#;
    let (task_start_verb, edit_root_argument, enter_worktree_suffix) = match audience {
        GuidanceAudience::Agents => ("Run", "", ""),
        GuidanceAudience::Claude => (
            "Select a safe absolute, absent-or-empty Task worktree path outside the\n   canonical repository, then run",
            " --edit-root <absolute-path>",
            " && cd <absolute-path>",
        ),
    };

    let task_path = if sprint_enabled {
        format!(
            r#"### Code-change path

1. Create a complete card under `docs/sprints/` using prose defaults: one stable
   `[plan-ref: ...]` and one exact unchecked `[ref: ...]` item.
2. {task_start_verb} `ait task start --from <sprint-card-path>#<exact-ref> --intent
   "<intent>"{edit_root_argument}{enter_worktree_suffix}`. `--from` syncs and binds the card; do not
   pre-sync it or copy Plan IDs.
3. Work only in the returned `edit_root`. Optional checkpoints: `ait snapshot create
   <task-id> --message "<message>"`.
4. {local_finish}

After every context-window compaction, re-read the bound sprint card."#
        )
    } else {
        format!(
            r#"### Code-change path

1. {task_start_verb} `ait task start --title "<title>" --intent "<intent>"{edit_root_argument}{enter_worktree_suffix}`; sprint
   mode is off, so `--from` is unavailable.
2. Work only in the returned `edit_root`. Optional checkpoints: `ait snapshot create
   <task-id> --message "<message>"`.
3. {local_finish}"#
        )
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
    let task_path = format!("{task_path}\n\n{edit_root_guidance}\n\nInitial Task work creates no Change. Revisions need a meaningfully titled\nChange from the exact reviewed Snapshot; reuse for corrections. Finish\napplies once; recovery reuses its receipt.");

    let supplemental_guidance = if !sprint_enabled {
        format!(
            r#"### Worktree and recovery rules

- Stay inside `edit_root`; do not search parent directories. `docs/` there is
  shared with the repository root.
- Sync authored Markdown with `{plan_sync_command}`; never hide it in a code
  Snapshot.
- For regressions, run `ait blame <path>` before repair. If AIT reports
  `action_required` or ambiguity, follow its recovery and use `ait task audit`
  only when directed or when evidence is requested."#
        )
    } else {
        format!(
            r#"{worktree_facts}

### Conditional references

- Read `docs/plan.md` Current Priorities only for product-scope, release, or
  public-contract changes.
- For a regression, use `ait blame <path>` before choosing a repair.
{markdown_sync_rule}
- A Snapshot is a checkpoint, not a substitute for the listed closeout.
- As needed: `ait queue summary` shows actionable work,
  `ait task audit <task-id>` shows readiness, and `ait task list --all` plus
  `ait snapshot list --all` show history.
- Change IDs are internal to normal Task work. Do not create another Change
  for checkpoints, review corrections, or checklist steps. Patchset CI uses
  the public `TASK_ID/P-##` Patchset reference. If a Task reports ambiguous work, inspect `ait task
  audit <task-id>`."#
        )
    };

    Ok(format!(
        r#"{MANAGED_START}
## Effective Ait Workflow (Generated)

{admission}

{prose_guidance}

{GUIDANCE_REUSE}

{TOOL_DISCIPLINE}

{task_path}

{WORKSPACE_IGNORE_GUIDANCE}

{supplemental_guidance}
{MANAGED_END}"#,
    ))
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

pub(crate) fn ensure_configured_sprint_directory(repo: &RepoRuntime) -> Result<(), String> {
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

    fn repo(teams: bool, sprint: &str) -> RepoRuntime {
        let mut config = JsonMap::new();
        config.insert("repo_name".to_string(), json!("demo"));
        config.insert("default_line".to_string(), json!("main"));
        config.insert("agent_teams".to_string(), json!(teams));
        let scope = "local";
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
    fn renders_teams_and_sprint_combinations_as_compact_nonconflicting_guidance() {
        for teams in [false, true] {
            for sprint in ["on", "off"] {
                let rendered = render_agent_workflow_block(&repo(teams, sprint)).unwrap();
                let remote = false;
                let scope = if remote { "remote" } else { "local" };
                let lean_local_sprint_off = !remote && sprint == "off";
                assert!(rendered.contains(&format!("Route: sprint=`{sprint}`; scope=`{scope}`")));
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
                    "{teams}/{sprint} guidance must use finish terminology"
                );
                assert!(!rendered.contains("plan-closeout="));
                assert!(!rendered.contains("ait install"));
                assert!(!rendered.contains("regenerate this authoritative block"));
                assert!(!rendered.contains("Read `docs/plan.md` when it exists"));
                assert_eq!(
                    rendered.contains("Read `docs/plan.md` Current Priorities only"),
                    !lean_local_sprint_off
                );
                assert!(rendered.contains("ait blame <path>"));
                assert!(!rendered.contains("workflow tier"));
                assert!(!rendered.contains("--profile quick"));
                assert!(rendered.contains("### Code-change path"));
                assert_eq!(
                    rendered.contains("### Worktree facts"),
                    !lean_local_sprint_off
                );
                assert_eq!(
                    rendered.contains("### Worktree and recovery rules"),
                    lean_local_sprint_off
                );
                assert_eq!(
                    rendered.contains("do not search parent directories"),
                    lean_local_sprint_off
                );
                assert_eq!(
                    rendered.contains("--sync-plan-markdown"),
                    !remote && !lean_local_sprint_off
                );
                assert!(rendered
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .contains("ait snapshot create <task-id>"));
                assert!(!rendered.contains("--task-id"));
                assert!(!rendered.contains("--change-id"));
                assert_eq!(
                    rendered.contains("A Snapshot is a checkpoint, not a substitute"),
                    !lean_local_sprint_off
                );
                assert!(!rendered.contains("--base-line"));
                assert!(
                    !rendered.to_ascii_lowercase().contains("json"),
                    "{teams}/{sprint} guidance must not mention JSON output"
                );
                let (base_byte_limit, base_word_limit) = match (remote, sprint) {
                    (false, "on") => (2_600, 340),
                    (false, _) => (2_200, 285),
                    (true, "on") => (3_200, 420),
                    (true, _) => (2_800, 365),
                };
                let prose = PlanPreferences::from_config(&JsonMap::new())
                    .unwrap()
                    .guidance();
                let facts =
                    render_worktree_facts(if remote { "remote" } else { "local" }, "upstream");
                let byte_limit = base_byte_limit
                    + prose.len()
                    + facts.len()
                    + WORKSPACE_IGNORE_GUIDANCE.len()
                    + GUIDANCE_REUSE.len()
                    + TOOL_DISCIPLINE.len()
                    + if teams { TEAM_REVIEW_GUIDANCE.len() } else { 0 }
                    + 40;
                let word_limit = base_word_limit
                    + prose.split_whitespace().count()
                    + facts.split_whitespace().count()
                    + WORKSPACE_IGNORE_GUIDANCE.split_whitespace().count()
                    + GUIDANCE_REUSE.split_whitespace().count()
                    + TOOL_DISCIPLINE.split_whitespace().count()
                    + if teams {
                        TEAM_REVIEW_GUIDANCE.split_whitespace().count()
                    } else {
                        0
                    }
                    + 6;
                assert!(
                    rendered.len() < byte_limit,
                    "{teams}/{sprint} guidance was {} bytes; limit is {byte_limit}",
                    rendered.len()
                );
                assert!(
                    rendered.split_whitespace().count() < word_limit,
                    "{teams}/{sprint} guidance was {} words; limit is {word_limit}",
                    rendered.split_whitespace().count()
                );
                if remote {
                    assert!(rendered.contains("remote=`upstream`"));
                    assert!(rendered.contains("plan sync <markdown-file-or-dir> --remote upstream"));
                    assert!(rendered.contains("ait workflow ready <task-id> --apply"));
                    assert!(rendered.contains("ait workflow finish\n   <task-id> --apply"));
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
                    assert!(rendered.contains("Create a complete card under `docs/sprints/`"));
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

                let claude = render_claude_workflow_block(&repo(teams, sprint)).unwrap();
                for guidance in [&rendered, &claude] {
                    assert_eq!(guidance.matches(WORKSPACE_IGNORE_GUIDANCE).count(), 1);
                    assert_eq!(guidance.matches(GUIDANCE_REUSE).count(), 1);
                    assert_eq!(guidance.matches(TOOL_DISCIPLINE).count(), 1);
                    for team_rule in [
                        "docs/roles/",
                        "ait team",
                        "ASSIGN",
                        "READY",
                        "role_revision",
                        "contacts",
                        "usage reviews",
                    ] {
                        assert!(!guidance.contains(team_rule), "{team_rule}: {guidance}");
                    }
                    assert!(guidance.contains("review `.aitignore`\nin `edit_root`"));
                    assert!(guidance.contains("first build/test or Snapshot-producing finish"));
                    assert!(guidance.contains("does not automatically load `.gitignore`"));
                    assert!(guidance.contains("previously snapshotted paths from new Snapshots"));
                    assert!(guidance.contains("not broad data categories"));
                }
                assert!(claude
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .contains("ait snapshot create <task-id>"));
                let shared_route = format!("Route: sprint=`{sprint}`; scope=`{scope}`");
                let mut shared_guidance = vec![
                    shared_route.as_str(),
                    "## Effective Ait Workflow (Generated)",
                    "### Code-change path",
                ];
                if lean_local_sprint_off {
                    shared_guidance.push("### Worktree and recovery rules");
                    shared_guidance.push("do not search parent directories");
                    assert!(!claude.contains("### Conditional references"));
                    assert!(!claude.contains("Read `docs/plan.md` Current Priorities only"));
                } else {
                    shared_guidance.push("### Conditional references");
                    shared_guidance.push("Read `docs/plan.md` Current Priorities only");
                }
                for shared in shared_guidance {
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
                    "{teams}/{sprint} Claude guidance must not mention JSON output"
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
    fn teams_preserve_shared_local_commands_and_add_independent_review_guidance() {
        for sprint in ["on", "off"] {
            for default in [Some("upstream"), None] {
                let mut runtime = repo(true, sprint);
                if default.is_none() {
                    runtime.config.remove("default_remote");
                }
                for audience in [GuidanceAudience::Agents, GuidanceAudience::Claude] {
                    let team = render_workflow_block(&runtime, audience).unwrap();
                    runtime.config.insert("agent_teams".into(), json!(false));
                    let solo = render_workflow_block(&runtime, audience).unwrap();
                    runtime.config.insert("agent_teams".into(), json!(true));
                    assert_eq!(team.replace(TEAM_REVIEW_GUIDANCE, ""), solo);
                    assert!(team.contains("a missing Reviewer requires assignment"));
                    assert!(!solo.contains("Fresh team candidates"));
                    assert!(team.contains("scope=`local`"));
                    assert!(team.contains("Admission: ready."));
                    assert!(!team.contains("configured remote is unavailable"));
                    for absent in [
                        "Newton",
                        "Coordinator",
                        "<scope-args>",
                        "work-order",
                        "role",
                        "ait team",
                        "ait workflow ready",
                        "notification",
                        "successful JSON receipt",
                    ] {
                        assert!(!team.contains(absent), "shared guidance contains {absent}");
                    }
                    assert_eq!(team.matches(MANAGED_START).count(), 1);
                    assert_eq!(team.matches(WORKSPACE_IGNORE_GUIDANCE).count(), 1);
                    assert_eq!(
                        team.contains("--from <sprint-card-path>#<exact-ref>"),
                        sprint == "on"
                    );
                    assert_eq!(team.contains("--title \"<title>\""), sprint == "off");
                    assert_eq!(
                        team.contains("&& cd <absolute-path>"),
                        matches!(audience, GuidanceAudience::Claude)
                    );
                    assert!(runtime.task_uses_local_scope(false, None).unwrap());
                    assert!(!runtime
                        .task_uses_local_scope(false, Some("upstream"))
                        .unwrap());
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
            let mut runtime = repo(false, "on");
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

            let rendered = render_agent_workflow_block(&runtime).unwrap();
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
            &render_agent_workflow_block(&repo(false, "on")).unwrap(),
            AGENT_HARNESS_PATH,
        )
        .unwrap();
        let second = replace_or_insert_managed_block(
            &first,
            &render_agent_workflow_block(&repo(false, "off")).unwrap(),
            AGENT_HARNESS_PATH,
        )
        .unwrap();
        assert!(second.contains("Custom before."));
        assert!(second.contains("Route: sprint=`off`; scope=`local`"));
        assert_eq!(second.matches(MANAGED_START).count(), 1);
        assert_eq!(second.matches(MANAGED_END).count(), 1);
    }

    #[test]
    fn local_route_omits_absent_and_inactive_default_remotes() {
        let with_remote = render_agent_workflow_block(&repo(false, "off")).unwrap();
        assert!(!with_remote.contains("remote=`upstream`"));
        assert!(!with_remote.contains("configured remote is unavailable"));

        let mut without_remote = repo(false, "off");
        without_remote.config.remove("default_remote");
        let rendered = render_agent_workflow_block(&without_remote).unwrap();
        assert!(!rendered.contains("remote=`none`"));
        assert!(rendered.contains("Admission: ready."));
    }

    #[test]
    fn mismatched_plan_binding_is_action_required_without_markdown_checkboxes() {
        let mut runtime = repo(false, "off");
        runtime
            .config
            .insert("plan_task_binding".to_string(), json!({"mode": "required"}));
        let rendered = render_agent_workflow_block(&runtime).unwrap();
        assert!(rendered.contains("- plan-binding=`required` (expected `off` for sprint=`off`)"));
        assert!(rendered.contains("Action required before mutation:"));
        assert!(!rendered.contains("- [x]"));
        assert!(!rendered.contains("- [ ]"));
    }

    #[test]
    fn absent_review_identity_does_not_block_admission_or_hide_binding_mismatches() {
        for task_review in [false, true] {
            for sprint in ["on", "off"] {
                for mismatch in [false, true] {
                    let mut runtime = repo(true, sprint);
                    runtime.config.remove("user_name");
                    runtime.config.remove("user_email");
                    runtime
                        .config
                        .insert("task_review".into(), json!(task_review));
                    assert!(runtime.task_review_reviewer_identity().is_none());
                    if mismatch {
                        runtime.config.insert(
                            "plan_task_binding".into(),
                            json!({"mode":if sprint=="on"{"off"}else{"required"}}),
                        );
                    }
                    for rendered in [
                        render_agent_workflow_block(&runtime).unwrap(),
                        render_claude_workflow_block(&runtime).unwrap(),
                    ] {
                        assert_eq!(
                            rendered.contains("Admission: ready."),
                            !mismatch,
                            "{rendered}"
                        );
                        assert_eq!(
                            rendered.contains("Action required before mutation:"),
                            mismatch,
                            "{rendered}"
                        );
                        if mismatch {
                            assert!(rendered.contains("(expected"));
                        }
                        assert!(!rendered.contains("configure user-name"), "{rendered}");
                        assert!(!rendered.contains("reviewer=`unset`"), "{rendered}");
                        assert!(!rendered.contains("user_email"), "{rendered}");
                    }
                }
            }
        }
    }

    #[test]
    fn arbitrary_author_mode_text_is_kept_inside_a_safe_code_span() {
        let mut runtime = repo(false, "off");
        runtime.config.insert(
            "default_author_mode".to_string(),
            json!("mode`name\nwith whitespace"),
        );
        let rendered = render_agent_workflow_block(&runtime).unwrap();
        assert!(rendered.contains("author-mode=``mode`name with whitespace``"));

        runtime
            .config
            .insert("default_author_mode".to_string(), json!("`edge ticks`"));
        let rendered = render_agent_workflow_block(&runtime).unwrap();
        assert!(rendered.contains("author-mode=`` `edge ticks` ``"));
    }

    #[test]
    fn local_convergence_ignores_inactive_remote_defaults_after_refresh() {
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
        assert_eq!(convergence["scope"], "local");
        assert!(convergence["remote"].is_null());
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0]["target"], AGENT_HARNESS_PATH);
        assert_eq!(captured[1]["target"], CLAUDE_HARNESS_PATH);
        for request in captured {
            assert_eq!(request["local"], true);
            assert!(request["remote_name"].is_null());
            assert!(request["remote_repo_name"].is_null());
            assert!(request["base_url"].is_null());
        }
        assert_eq!(convergence["plan_syncs"].as_array().unwrap().len(), 2);
        let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("--local"));
        assert!(!agents.contains("remote=`mirror`"));
        assert!(agents.contains("Admission: ready."));
        let claude = fs::read_to_string(temp.path().join("CLAUDE.md")).unwrap();
        assert!(claude.contains("--local"));
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
                render_claude_workflow_block(&repo).unwrap()
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
    fn local_convergence_does_not_require_a_default_remote() {
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

        assert_eq!(convergence["status"], "synced");
        assert!(executed);
        let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("Route: sprint=`on`; scope=`local`"));
        assert!(!agents.contains("required by `solo_remote`"));
    }
}
