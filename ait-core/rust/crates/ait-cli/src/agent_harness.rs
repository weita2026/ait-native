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

pub fn render_agent_workflow_block(repo: &RepoRuntime) -> String {
    let workflow_mode = repo.effective_workflow_mode();
    let sprint_enabled = repo.sprint_enabled();
    let remote_name = repo
        .default_remote_name()
        .unwrap_or_else(|| "origin".to_string());
    let scope_label = effective_agent_harness_scope(repo);
    let land_contract = task_land_scope_contract(scope_label == "local");
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
            r#"Finish code closeout with `ait task land <task-or-change-id>`.
   After successful land, mark the exact bound checklist item complete, then
   sync its card separately:
   `ait plan sync <sprint-card-path> --remote {remote_name}`."#
        )
    } else {
        r#"Finish with `ait task land <task-or-change-id>`. A successful final
   local task land closes and syncs the exact bound sprint checklist item
   locally."#
            .to_string()
    };

    let sprint_path = if sprint_enabled {
        format!(
            r#"### Task path: sprint mode is on

For changes classified as `normal_task` or `fully_governed`:

1. Write a detailed Markdown sprint card under `docs/sprints/` with one stable
   `[plan-ref: ...]` root and an unchecked checklist item carrying an exact
   `[ref: ...]`.
2. Start the task and first change with `ait task start --from
   <sprint-card-path>#<exact-ref> --intent "<intent>" --base-line <line>`.
   `task start --from` owns exact-file Plan sync in the configured scope,
   post-sync item taskability validation, canonical Plan binding, Task/Change
   creation, bound-worktree bootstrap, and the printed `cd` hint. The task is
   {task_scope}; do not run a separate pre-start Plan sync or copy Plan IDs.
3. Enter the task worktree emitted by `task start`, author the code there, and
   create a snapshot with `ait snapshot create --message "<message>"`.
4. {sprint_finish}

After every context-window compaction, re-read the bound sprint card before
continuing."#
        )
    } else {
        format!(
            r#"### Task path: sprint mode is off

For changes classified as `normal_task` or `fully_governed`:

1. Start a task and first change with `ait task start --title "<title>"
   --intent "<intent>" --base-line <line>`. The task scope is {task_scope}; a
   sprint card is not required and `--from` is unavailable while sprint mode
   is off.
2. Enter the emitted task worktree, author the code there, and create a snapshot
   with `ait snapshot create --message "<message>"`.
3. Finish with `ait task land <task-or-change-id>`."#
        )
    };

    let closeout = if scope_label == "remote" {
        r#"### Remote readiness and land

- `task start` opens the remote task/change lineage. `snapshot create` records
  the reviewable code state.
- Prepare snapshot freshness, patchset publication/content synchronization, CI,
  attestation, policy, and review state explicitly with `ait workflow ready
  <change-id> --apply`.
- `ait task land <task-or-change-id>` consumes an already-ready patchset and
  fails immediately on a missing prerequisite. It does not publish/synchronize
  content, start/wait for CI, or sync Plan state. Success owns remote land, task
  completion, target-line sync, and bound-worktree cleanup.
- Use `ait workflow land <change-id> --apply` when inspecting or resuming the
  guided land phase separately."#
            .to_string()
    } else if scope_label == "local" {
        r#"### Local land

- `task start`, its initial change, snapshots, and `task land` stay local unless
  a command explicitly requests remote promotion.
- `ait task land <task-or-change-id> --local` lands the code onto the local
  target line, completes the task, cleans the bound worktree, and (when bound)
  closes the local sprint checklist item."#
            .to_string()
    } else {
        r#"### Effective custom closeout

- Run `ait config show` before mutating workflow state and follow its effective
  plan/task/change scopes explicitly.
- Use `ait task land <task-or-change-id>` for the configured closeout path."#
            .to_string()
    };

    format!(
        r#"{MANAGED_START}
## Effective Ait Workflow (Generated)

`ait init`, `ait install`, relevant `ait config set` changes, and default-remote
setup regenerate this block from `.ait/config.json` and sync it when the
configured target is available. The current values and commands are
authoritative; they replace stale context and generic examples.

- workflow mode: `{workflow_mode}`
- sprint mode: `{}`
- default mutation scope: `{scope_label}`
- task-land contract: `{TASK_LAND_CONTRACT_VERSION}`
- task-land readiness policy: `{}`
- task-land Plan closeout policy: `{}`

Commands below already reflect these values. Do not mix local and remote
variants.

### Rules for every repository mutation

- Read this block at the start of a session. Read `docs/plan.md` when it exists,
  and use `ait config show` if runtime state may have changed.
- When a regression is found, run `ait blame <path>` (narrow with `--line` or
  `--start`/`--end`) to identify the responsible Snapshot or Plan revision
  before choosing the repair.
{markdown_sync_rule}
- `ait workflow ready` and `ait workflow land` are text-only decision surfaces;
  never append or recommend `--json` for either command.
- Use `ait workflow tier --json` to evaluate an already bounded local edit
  before choosing its closeout path. `quick_modification` is an explicit local-
  only opt-in on a known non-default line and must finish with `ait snapshot
  create --profile quick --intent "<intent>" --validation "<evidence>"
  --message "<message>"`. If runtime risk escalates, leave the workspace on its
  current line and follow the reported Task command; never publish quick work
  directly to a governed remote.
- Every `normal_task` or `fully_governed` code change must start with a new `ait
  task start`, be authored in its bound worktree, and finish with `ait task land
  <task-or-change-id>`.
- Prefer `ait queue summary --all-changes` for inventory and `ait task audit
  <task-id>` for one task's readiness.

{sprint_path}

{closeout}
{MANAGED_END}"#,
        if sprint_enabled { "on" } else { "off" },
        land_contract.readiness_policy,
        land_contract.plan_closeout_policy,
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
                assert!(rendered.contains(&format!("workflow mode: `{mode}`")));
                assert!(rendered.contains(&format!("sprint mode: `{sprint}`")));
                assert!(rendered.contains(&format!(
                    "task-land contract: `{TASK_LAND_CONTRACT_VERSION}`"
                )));
                assert!(rendered.contains(&format!(
                    "task-land Plan closeout policy: `{}`",
                    task_land_scope_contract(mode == "solo_local").plan_closeout_policy
                )));
                assert_eq!(rendered.matches(MANAGED_START).count(), 1);
                assert!(rendered.contains("`ait init`, `ait install`, relevant `ait config set`"));
                assert!(rendered.contains("relevant `ait config set` changes"));
                assert!(rendered.contains("default-remote\nsetup regenerate this block"));
                assert!(rendered.contains("Read `docs/plan.md` when it exists"));
                assert!(rendered.contains("ait blame <path>"));
                assert!(rendered.contains(
                    "`ait workflow ready` and `ait workflow land` are text-only decision surfaces"
                ));
                assert!(rendered.contains("never append or recommend `--json`"));
                assert!(rendered.contains("ait workflow tier --json"));
                assert!(rendered.contains("snapshot\n  create --profile quick"));
                assert!(rendered.contains("Every `normal_task` or `fully_governed`"));
                assert!(
                    rendered.split_whitespace().count() < 1_024,
                    "{mode}/{sprint} guidance exceeded the 1,024-token ceiling"
                );
                if matches!(mode, "solo_remote" | "team_remote") {
                    assert!(rendered.contains("plan sync <markdown-file-or-dir> --remote upstream"));
                    assert!(rendered.contains("does not publish/synchronize\n  content"));
                    assert!(!rendered.contains("### Local land"));
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
                rendered.contains(&format!("sprint mode: `{expected_sprint}`")),
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
        assert!(second.contains("workflow mode: `solo_remote`"));
        assert_eq!(second.matches(MANAGED_START).count(), 1);
        assert_eq!(second.matches(MANAGED_END).count(), 1);
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
        assert!(agents.contains("workflow mode: `solo_remote`"));
    }
}
