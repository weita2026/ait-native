use super::*;
use crate::json_support::encode_value_pretty_with_newline_error_string;
#[cfg(test)]
use ait_core::line_store::{line_by_name_with_line_store, LineStore};
use ait_core::remote_store::{list_remotes_with_remote_store, RemoteStore};
use ait_core::repo_status_store::{storage_counts_with_repo_status_store, RepoStatusStore};

pub fn line_list(
    repo: &RepoRuntime,
    include_all: bool,
    archived_only: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if include_all && archived_only {
        return Err("--all and --archived cannot be used together".to_string());
    }
    let mut rows = if remote_name.is_some() {
        let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
        let mut task_remote = http_task_remote(repo, &remote_row)?;
        remote_line_list_with_task_remote(&mut task_remote, &repo_name)?
    } else {
        JsonValue::Array(list_local_lines(repo)?)
    };
    if let Some(items) = rows.as_array_mut() {
        items.retain(|row| {
            let status = string_field(row, "status").unwrap_or_else(|| "active".to_string());
            if archived_only {
                status == "archived"
            } else if include_all {
                true
            } else {
                status != "archived"
            }
        });
    }
    Ok(rows)
}

pub fn line_show(repo: &RepoRuntime, name: Option<&str>) -> Result<JsonValue, String> {
    let resolved_name = match normalized_text(name) {
        Some(name) => name,
        None => repo.current_line_name()?,
    };
    local_line_row(repo, &resolved_name)
}

#[cfg(test)]
pub(super) fn line_show_with_line_store<S>(store: &S, line_name: &str) -> Result<JsonValue, String>
where
    S: LineStore + ?Sized,
{
    line_by_name_with_line_store(store, line_name)?
        .map(|line| line_record_json(&line))
        .ok_or_else(|| format!("Unknown line: {line_name}"))
}

fn line_create_unlocked(
    repo: &RepoRuntime,
    name: &str,
    from_snapshot: Option<&str>,
    switch: bool,
) -> Result<JsonValue, String> {
    if switch {
        guard_no_active_line_merge(repo, None, "switching lines")?;
    }
    let current_before = repo.current_line_name()?;
    let resolved_snapshot_id = match normalized_text(from_snapshot) {
        Some(snapshot_id) => Some(snapshot_id),
        None => string_field(&local_line_row(repo, &current_before)?, "head_snapshot_id"),
    };
    let created = create_local_line(repo, name, resolved_snapshot_id.as_deref())?;
    if !switch {
        return Ok(created);
    }
    set_runtime_current_line(repo, name)?;
    let mut payload = created
        .as_object()
        .cloned()
        .ok_or_else(|| "line create payload must decode to an object".to_string())?;
    payload.insert(
        "current_line_before".to_string(),
        JsonValue::String(current_before),
    );
    payload.insert(
        "current_line".to_string(),
        JsonValue::String(name.to_string()),
    );
    payload.insert("switched".to_string(), JsonValue::Bool(true));
    Ok(JsonValue::Object(payload))
}

pub fn line_create(
    repo: &RepoRuntime,
    name: &str,
    from_snapshot: Option<&str>,
    switch: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli line create", || {
        line_create_unlocked(repo, name, from_snapshot, switch)
    })
}

fn line_switch_unlocked(
    repo: &RepoRuntime,
    name: &str,
    restore: bool,
    force: bool,
) -> Result<JsonValue, String> {
    if force && !restore {
        return Err("--force only applies together with --restore".to_string());
    }
    guard_no_active_line_merge(repo, None, "switching lines")?;
    if restore {
        return worktree_restore(repo, None, None, Some(name), &[], force, false);
    }
    let row = local_line_row(repo, name)?;
    set_runtime_current_line(repo, name)?;
    Ok(row)
}

pub fn line_switch(
    repo: &RepoRuntime,
    name: &str,
    restore: bool,
    force: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli line switch", || {
        line_switch_unlocked(repo, name, restore, force)
    })
}

fn line_archive_unlocked(
    repo: &RepoRuntime,
    name: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if remote_name.is_some() {
        let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
        let mut task_remote = http_task_remote(repo, &remote_row)?;
        return remote_line_archive_with_task_remote(&mut task_remote, &repo_name, name);
    }
    let default_line = repo.default_line_name();
    if name == default_line {
        return Err(format!("Default line {name} cannot be archived"));
    }
    let current_line = repo.current_line_name()?;
    if name == current_line {
        return Err(format!(
            "Current line {name} cannot be archived; switch to another line first"
        ));
    }
    let archived = archive_local_line(repo, name)?;
    Ok(archived)
}

pub fn line_archive(
    repo: &RepoRuntime,
    name: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if remote_name.is_some() {
        return line_archive_unlocked(repo, name, remote_name);
    }
    run_locked_workspace_command(repo, "ait-cli line archive", || {
        line_archive_unlocked(repo, name, None)
    })
}

const LINE_LIFECYCLE_CONTRACT: &str = "line-lifecycle/v1";
const LINE_LIFECYCLE_TRANSACTION_CONTRACT: &str = "line-lifecycle-transaction/v1";

fn line_lifecycle_journal_path(repo: &RepoRuntime) -> PathBuf {
    repo.authoritative_repo_root()
        .join(".ait")
        .join("line-lifecycle-transaction.json")
}

fn write_json_atomic(path: &Path, payload: &JsonValue) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("JSON path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    let temp_path = parent.join(format!(
        ".line-lifecycle-{}-{}.tmp",
        std::process::id(),
        system_event_timestamp().replace([':', '.', '+'], "-")
    ));
    let encoded = encode_value_pretty_with_newline_error_string(payload)?;
    fs::write(&temp_path, encoded).map_err(|err| err.to_string())?;
    fs::rename(&temp_path, path).map_err(|err| {
        let _ = fs::remove_file(&temp_path);
        err.to_string()
    })
}

fn replace_string_field(
    object: &mut JsonMap<String, JsonValue>,
    field: &str,
    old_line_name: &str,
    new_line_name: &str,
) -> bool {
    if object.get(field).and_then(JsonValue::as_str) != Some(old_line_name) {
        return false;
    }
    object.insert(
        field.to_string(),
        JsonValue::String(new_line_name.to_string()),
    );
    true
}

fn update_line_reference_object(
    object: &mut JsonMap<String, JsonValue>,
    old_line_name: &str,
    new_line_name: &str,
    line_id: &str,
) -> bool {
    let mut changed = false;
    for (name_field, id_field) in [
        ("line_name", "line_id"),
        ("current_line", "current_line_id"),
        ("default_line", "default_line_id"),
        ("registered_line_name", "registered_line_id"),
        ("target_base_line", "target_base_line_id"),
        ("forked_from_line", "forked_from_line_id"),
        ("merge_target_line", "merge_target_line_id"),
        ("merge_source_line", "merge_source_line_id"),
    ] {
        if replace_string_field(object, name_field, old_line_name, new_line_name) {
            object.insert(id_field.to_string(), JsonValue::String(line_id.to_string()));
            changed = true;
        }
    }
    if let Some(cache) = object
        .get_mut("workspace_status_cache")
        .and_then(JsonValue::as_object_mut)
    {
        changed |= update_line_reference_object(cache, old_line_name, new_line_name, line_id);
    }
    changed
}

fn registered_worktree_documents(repo: &RepoRuntime) -> Result<Vec<(PathBuf, JsonValue)>, String> {
    let registry_dir = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("worktrees");
    let Ok(entries) = fs::read_dir(&registry_dir) else {
        return Ok(Vec::new());
    };
    let mut documents = Vec::new();
    for entry in entries {
        let path = entry.map_err(|err| err.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let payload = read_json_value(&path);
        if !payload.is_object() {
            return Err(format!(
                "Worktree registry payload is not an object: {}",
                path.display()
            ));
        }
        documents.push((path, payload));
    }
    documents.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(documents)
}

fn preflight_line_rename_references(repo: &RepoRuntime, old_line_name: &str) -> Result<(), String> {
    let change_store = repo.change_store()?;
    let changes = line_change_usage_index_with_change_store(&change_store)?;
    if let Some(change_ids) = changes.get(old_line_name).filter(|rows| !rows.is_empty()) {
        return Err(format!(
            "Line {old_line_name} is referenced by active Change(s): {}. Close, finish, or retarget them before rename.",
            change_ids.join(", ")
        ));
    }
    let remote_changes = remote_line_change_usage_index(repo)?;
    if let Some(change_ids) = remote_changes
        .get(old_line_name)
        .filter(|rows| !rows.is_empty())
    {
        return Err(format!(
            "Line {old_line_name} is referenced by active Change(s) on the configured remote: {}. Close, finish, or retarget them before rename.",
            change_ids.join(", ")
        ));
    }
    for (path, payload) in registered_worktree_documents(repo)? {
        let Some(object) = payload.as_object() else {
            continue;
        };
        let references_line = [
            "line_name",
            "current_line",
            "registered_line_name",
            "target_base_line",
            "forked_from_line",
            "merge_target_line",
            "merge_source_line",
        ]
        .iter()
        .any(|field| object.get(*field).and_then(JsonValue::as_str) == Some(old_line_name));
        if !references_line {
            continue;
        }
        let merge_state = object
            .get("merge_state")
            .and_then(JsonValue::as_str)
            .unwrap_or("idle");
        let rebase_state = object
            .get("rebase_state")
            .and_then(JsonValue::as_str)
            .unwrap_or("idle");
        if merge_state != "idle" || rebase_state != "idle" {
            return Err(format!(
                "Line {old_line_name} has an active operation in {} (merge={merge_state}, rebase={rebase_state}); continue or abort it before rename.",
                path.display()
            ));
        }
    }
    Ok(())
}

fn apply_line_reference_rename(
    repo: &RepoRuntime,
    old_line_name: &str,
    new_line_name: &str,
    line_id: &str,
) -> Result<(), String> {
    let root_config_path = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("config.json");
    let mut root_config = read_json_object_value(&root_config_path);
    let default_was_old = repo.default_line_name() == old_line_name;
    let root_current_was_old = root_config
        .get("current_line")
        .and_then(JsonValue::as_str)
        .map_or(default_was_old, |value| value == old_line_name);
    let mut root_changed =
        update_line_reference_object(&mut root_config, old_line_name, new_line_name, line_id);
    if default_was_old {
        root_config.insert(
            "default_line".to_string(),
            JsonValue::String(new_line_name.to_string()),
        );
        root_config.insert(
            "default_line_id".to_string(),
            JsonValue::String(line_id.to_string()),
        );
        root_changed = true;
    }
    if root_current_was_old {
        root_config.insert(
            "current_line".to_string(),
            JsonValue::String(new_line_name.to_string()),
        );
        root_config.insert(
            "current_line_id".to_string(),
            JsonValue::String(line_id.to_string()),
        );
        root_changed = true;
    }
    if root_changed {
        write_json_atomic(&root_config_path, &JsonValue::Object(root_config))?;
    }

    for (registry_path, mut payload) in registered_worktree_documents(repo)? {
        let object = payload
            .as_object_mut()
            .ok_or_else(|| format!("Invalid worktree metadata: {}", registry_path.display()))?;
        let changed = update_line_reference_object(object, old_line_name, new_line_name, line_id);
        let marker_path = object
            .get("path")
            .and_then(JsonValue::as_str)
            .map(PathBuf::from)
            .map(|path| path.join(WORKTREE_CONFIG_NAME));
        if changed {
            write_json_atomic(&registry_path, &payload)?;
        }
        let Some(marker_path) = marker_path.filter(|path| path.is_file()) else {
            continue;
        };
        let mut marker = read_json_object_value(&marker_path);
        if update_line_reference_object(&mut marker, old_line_name, new_line_name, line_id) {
            write_json_atomic(&marker_path, &JsonValue::Object(marker))?;
        }
    }
    Ok(())
}

fn recover_line_lifecycle_transaction(repo: &RepoRuntime) -> Result<(), String> {
    let journal_path = line_lifecycle_journal_path(repo);
    if !journal_path.is_file() {
        return Ok(());
    }
    let journal = read_json_value(&journal_path);
    if string_field(&journal, "contract").as_deref() != Some(LINE_LIFECYCLE_TRANSACTION_CONTRACT)
        || string_field(&journal, "operation").as_deref() != Some("rename")
    {
        return Err(format!(
            "Unknown line lifecycle recovery journal: {}",
            journal_path.display()
        ));
    }
    let line_id = required_string_field(&journal, "line_id")?;
    let old_line_name = required_string_field(&journal, "old_line_name")?;
    let new_line_name = required_string_field(&journal, "new_line_name")?;
    let store = repo.line_store()?;
    let old = store.line_by_name(&old_line_name)?;
    let new = store.line_by_name(&new_line_name)?;
    if new.as_ref().is_some_and(|line| line.line_id == line_id) {
        apply_line_reference_rename(repo, &old_line_name, &new_line_name, &line_id)?;
    } else if old.as_ref().is_some_and(|line| line.line_id == line_id) {
        apply_line_reference_rename(repo, &new_line_name, &old_line_name, &line_id)?;
    } else {
        return Err(format!(
            "Cannot recover line rename transaction for stable identity {line_id}; neither {old_line_name} nor {new_line_name} owns it."
        ));
    }
    fs::remove_file(&journal_path).map_err(|err| err.to_string())
}

fn line_rename_local_unlocked(
    repo: &RepoRuntime,
    old_line_name: &str,
    new_line_name: &str,
) -> Result<JsonValue, String> {
    recover_line_lifecycle_transaction(repo)?;
    if old_line_name.trim() == new_line_name.trim() {
        return Err("Old and new line names must differ.".to_string());
    }
    let store = repo.line_store()?;
    let source = store
        .line_by_name(old_line_name)?
        .ok_or_else(|| format!("Unknown line: {old_line_name}"))?;
    if store.line_by_name(new_line_name)?.is_some() {
        return Err(format!("Line already exists: {new_line_name}"));
    }
    preflight_line_rename_references(repo, old_line_name)?;
    let updated_at = system_event_timestamp();
    let journal_path = line_lifecycle_journal_path(repo);
    write_json_atomic(
        &journal_path,
        &json!({
            "contract": LINE_LIFECYCLE_TRANSACTION_CONTRACT,
            "operation": "rename",
            "line_id": &source.line_id,
            "old_line_name": old_line_name,
            "new_line_name": new_line_name,
            "prepared_at": &updated_at,
        }),
    )?;
    let renamed = match store.rename_line(old_line_name, new_line_name, &updated_at) {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_file(&journal_path);
            return Err(error);
        }
    };
    if let Err(error) =
        apply_line_reference_rename(repo, old_line_name, new_line_name, &source.line_id)
    {
        let pointer_rollback =
            apply_line_reference_rename(repo, new_line_name, old_line_name, &source.line_id);
        let store_rollback = store.rename_line(new_line_name, old_line_name, &updated_at);
        if pointer_rollback.is_ok() && store_rollback.is_ok() {
            let _ = fs::remove_file(&journal_path);
        }
        return Err(format!(
            "Line rename pointer reconciliation failed: {error}. Recovery journal: {}",
            journal_path.display()
        ));
    }
    fs::remove_file(&journal_path).map_err(|err| err.to_string())?;
    Ok(json!({
        "contract": LINE_LIFECYCLE_CONTRACT,
        "operation": "rename",
        "line_id": renamed.line_id,
        "old_line_name": old_line_name,
        "new_line_name": renamed.line_name,
        "status": renamed.status,
        "head_snapshot_id": renamed.head_snapshot_id,
        "updated_at": renamed.updated_at,
        "references_reconciled": true,
        "transaction_recovered": false,
    }))
}

fn line_history_preservation(
    repo: &RepoRuntime,
    line_name: &str,
    head_snapshot_id: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(head_snapshot_id) = normalized_text(head_snapshot_id) else {
        return Ok(Some("empty-line".to_string()));
    };
    for row in list_local_lines(repo)? {
        let candidate_name = string_field(&row, "line_name").unwrap_or_default();
        if candidate_name == line_name {
            continue;
        }
        let candidate_head = string_field(&row, "head_snapshot_id");
        if snapshot_distance_if_ancestor(
            repo,
            Some(head_snapshot_id.as_str()),
            candidate_head.as_deref(),
        )?
        .is_some()
        {
            return Ok(Some(format!("local-line:{candidate_name}")));
        }
    }
    let tag_store =
        FilesystemTagStore::new(repo.authoritative_repo_root().to_string_lossy().as_ref())?;
    for tag in tag_store.list_tags()? {
        if snapshot_distance_if_ancestor(
            repo,
            Some(head_snapshot_id.as_str()),
            Some(tag.snapshot_id.as_str()),
        )?
        .is_some()
        {
            return Ok(Some(format!("tag:{}", tag.name)));
        }
    }
    for remote_name in list_remote_names(repo)? {
        let Ok((remote_row, repo_name)) = remote_context(repo, Some(&remote_name), None) else {
            continue;
        };
        let Ok(mut remote) = http_task_remote(repo, &remote_row) else {
            continue;
        };
        let Ok(row) = remote.get_line(&repo_name, line_name) else {
            continue;
        };
        if string_field(&row, "head_snapshot_id").as_deref() == Some(head_snapshot_id.as_str()) {
            return Ok(Some(format!("remote:{remote_name}/{line_name}")));
        }
    }
    Ok(None)
}

pub(in crate::primitives) fn line_delete_local_unlocked(
    repo: &RepoRuntime,
    line_name: &str,
    confirm: bool,
) -> Result<JsonValue, String> {
    if !confirm {
        return Err("Pass --yes to delete a line ref.".to_string());
    }
    recover_line_lifecycle_transaction(repo)?;
    if line_name == repo.default_line_name() {
        return Err(format!("Default line {line_name} cannot be deleted."));
    }
    if line_name == repo.current_line_name()? {
        return Err(format!(
            "Current line {line_name} cannot be deleted; switch to another line first."
        ));
    }
    if line_name.starts_with("review/") || line_name.starts_with("review-base/") {
        return Err(format!(
            "Protected review line {line_name} cannot be deleted through the general line lifecycle."
        ));
    }
    let store = repo.line_store()?;
    let line = store
        .line_by_name(line_name)?
        .ok_or_else(|| format!("Unknown line: {line_name}"))?;
    let indexes = collect_line_usage_indexes(repo)?;
    let usage = line_usage_summary(line_name, &indexes);
    let worktree_names = json_string_list(usage.get("worktree_names"));
    let change_ids = json_string_list(usage.get("active_change_ids"));
    if !worktree_names.is_empty() || !change_ids.is_empty() {
        return Err(format!(
            "Line {line_name} is still bound (worktrees: {}; active changes: {}). Remove or retarget every binding before delete.",
            if worktree_names.is_empty() { "none".to_string() } else { worktree_names.join(", ") },
            if change_ids.is_empty() { "none".to_string() } else { change_ids.join(", ") },
        ));
    }
    let preserved_by = line_history_preservation(repo, line_name, line.head_snapshot_id.as_deref())?
        .ok_or_else(|| format!(
            "Line {line_name} has unique history that is not verified on another local line or configured remote. Preserve or push head {} before delete.",
            line.head_snapshot_id.as_deref().unwrap_or("none")
        ))?;
    let deleted_at = system_event_timestamp();
    let deleted = store.delete_line(line_name, &deleted_at)?;
    Ok(json!({
        "contract": LINE_LIFECYCLE_CONTRACT,
        "operation": "delete",
        "line_id": deleted.line_id,
        "line_name": deleted.line_name,
        "status": deleted.status,
        "deleted_at": deleted.updated_at,
        "head_snapshot_id": deleted.head_snapshot_id,
        "history_preserved": true,
        "history_preserved_by": preserved_by,
        "tombstone": true,
        "snapshots_deleted": 0,
    }))
}

fn line_lifecycle_idempotency_key(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(LINE_LIFECYCLE_CONTRACT.as_bytes());
    for part in parts {
        digest.update([0]);
        digest.update(part.as_bytes());
    }
    format!("line-lifecycle-{:x}", digest.finalize())
}

fn require_remote_line_identity(row: &JsonValue, line_name: &str) -> Result<String, String> {
    string_field(row, "line_id")
        .and_then(|value| normalized_text(Some(&value)))
        .ok_or_else(|| {
            format!(
                "Remote line {line_name} does not expose stable line_id; upgrade the server before remote rename/delete."
            )
        })
}

fn enrich_remote_line_lifecycle_receipt(
    mut receipt: JsonValue,
    operation: &str,
    remote_name: &str,
    idempotency_key: &str,
    expected_line_id: &str,
) -> Result<JsonValue, String> {
    let object = receipt
        .as_object_mut()
        .ok_or_else(|| "Remote line lifecycle receipt must be an object.".to_string())?;
    if let Some(actual_line_id) = object.get("line_id").and_then(JsonValue::as_str) {
        if actual_line_id != expected_line_id {
            return Err(format!(
                "Remote line lifecycle receipt identity mismatch: expected {expected_line_id}, got {actual_line_id}."
            ));
        }
    } else {
        return Err("Remote line lifecycle receipt is missing line_id.".to_string());
    }
    object
        .entry("contract".to_string())
        .or_insert_with(|| JsonValue::String(LINE_LIFECYCLE_CONTRACT.to_string()));
    object.insert(
        "operation".to_string(),
        JsonValue::String(operation.to_string()),
    );
    object.insert(
        "remote".to_string(),
        JsonValue::String(remote_name.to_string()),
    );
    object.insert(
        "idempotency_key".to_string(),
        JsonValue::String(idempotency_key.to_string()),
    );
    Ok(receipt)
}

fn line_rename_remote(
    repo: &RepoRuntime,
    old_line_name: &str,
    new_line_name: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if old_line_name.trim() == new_line_name.trim() {
        return Err("Old and new line names must differ.".to_string());
    }
    let resolved_remote_name = normalized_text(remote_name)
        .or_else(|| repo.default_remote_name())
        .ok_or_else(|| "Remote name is required.".to_string())?;
    let (remote_row, repo_name) = remote_context(repo, Some(&resolved_remote_name), None)?;
    let mut remote = http_task_remote(repo, &remote_row)?;
    let row = remote
        .get_line(&repo_name, old_line_name)
        .map_err(|err| err.to_string())?;
    let line_id = require_remote_line_identity(&row, old_line_name)?;
    let head_snapshot_id = string_field(&row, "head_snapshot_id");
    let idempotency_key = line_lifecycle_idempotency_key(&[
        "rename",
        &repo_name,
        &line_id,
        old_line_name,
        new_line_name,
        head_snapshot_id.as_deref().unwrap_or("none"),
    ]);
    let receipt = remote
        .rename_remote_line(
            &repo_name,
            old_line_name,
            new_line_name,
            &line_id,
            head_snapshot_id.as_deref(),
            &idempotency_key,
        )
        .map_err(|err| err.to_string())?;
    enrich_remote_line_lifecycle_receipt(
        receipt,
        "rename",
        &resolved_remote_name,
        &idempotency_key,
        &line_id,
    )
}

fn line_delete_remote(
    repo: &RepoRuntime,
    line_name: &str,
    remote_name: Option<&str>,
    confirm: bool,
) -> Result<JsonValue, String> {
    if !confirm {
        return Err("Pass --yes to delete a remote line ref.".to_string());
    }
    if line_name.starts_with("review/") || line_name.starts_with("review-base/") {
        return Err(format!(
            "Protected review line {line_name} cannot be deleted through the general line lifecycle."
        ));
    }
    let resolved_remote_name = normalized_text(remote_name)
        .or_else(|| repo.default_remote_name())
        .ok_or_else(|| "Remote name is required.".to_string())?;
    let (remote_row, repo_name) = remote_context(repo, Some(&resolved_remote_name), None)?;
    let mut remote = http_task_remote(repo, &remote_row)?;
    let row = remote
        .get_line(&repo_name, line_name)
        .map_err(|err| err.to_string())?;
    let line_id = require_remote_line_identity(&row, line_name)?;
    let head_snapshot_id = string_field(&row, "head_snapshot_id");
    let idempotency_key = line_lifecycle_idempotency_key(&[
        "delete",
        &repo_name,
        &line_id,
        line_name,
        head_snapshot_id.as_deref().unwrap_or("none"),
    ]);
    let receipt = remote
        .delete_remote_line(
            &repo_name,
            line_name,
            &line_id,
            head_snapshot_id.as_deref(),
            &idempotency_key,
        )
        .map_err(|err| err.to_string())?;
    enrich_remote_line_lifecycle_receipt(
        receipt,
        "delete",
        &resolved_remote_name,
        &idempotency_key,
        &line_id,
    )
}

pub fn line_rename(
    repo: &RepoRuntime,
    old_line_name: &str,
    new_line_name: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if remote_name.is_some() {
        return line_rename_remote(repo, old_line_name, new_line_name, remote_name);
    }
    run_locked_workspace_command(repo, "ait-cli line rename", || {
        line_rename_local_unlocked(repo, old_line_name, new_line_name)
    })
}

pub fn line_delete(
    repo: &RepoRuntime,
    line_name: &str,
    remote_name: Option<&str>,
    confirm: bool,
) -> Result<JsonValue, String> {
    if remote_name.is_some() {
        return line_delete_remote(repo, line_name, remote_name, confirm);
    }
    run_locked_workspace_command(repo, "ait-cli line delete", || {
        line_delete_local_unlocked(repo, line_name, confirm)
    })
}

pub fn line_set_head(
    repo: &RepoRuntime,
    name: &str,
    snapshot_id: Option<&str>,
) -> Result<JsonValue, String> {
    guard_current_worktree_task_bound_authoring(repo, "line set-head")?;
    run_locked_workspace_command(repo, "ait-cli line set-head", || {
        set_or_create_local_line_head(repo, name, snapshot_id)
    })
}

pub(super) fn remote_line_list_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineLister + ?Sized,
{
    task_remote
        .list_lines(repo_name)
        .map(JsonValue::Array)
        .map_err(|err| err.to_string())
}

pub(super) fn remote_line_archive_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineCloser + ?Sized,
{
    task_remote
        .close_line(repo_name, name, "archived")
        .map_err(|err| err.to_string())
}

pub(super) fn list_remote_names(repo: &RepoRuntime) -> Result<Vec<String>, String> {
    let store = repo.remote_store()?;
    configured_remote_names_with_store(&store)
}

pub fn repo_status(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let _status_range = perfetto_range!("ait.cli.status.read");
    let repo_name = repo.repo_name();
    let (current_line, head_snapshot_id) = {
        let _range = perfetto_range!("ait.cli.status.line_head");
        let current_line = repo.current_line_name()?;
        let head_snapshot_id = local_line_head_snapshot_id(repo, &current_line)?;
        (current_line, head_snapshot_id)
    };
    let snapshot_count = {
        let _range = perfetto_range!("ait.cli.status.storage_counts");
        repo_status_snapshot_count(repo)?
    };
    let remote_count = {
        let _range = perfetto_range!("ait.cli.status.remote_list");
        list_remote_names(repo)?.len() as i64
    };
    let workspace = {
        let _range = perfetto_range!("ait.cli.status.workspace_delta");
        workspace_delta_payload(repo, head_snapshot_id.as_deref(), None)?
    };
    let worktree_hygiene = {
        let _range = perfetto_range!("ait.cli.status.worktree_hygiene");
        repo_status_worktree_hygiene(repo)?
    };
    let line_count = {
        let _range = perfetto_range!("ait.cli.status.line_count");
        count_local_lines(repo)? as i64
    };
    let reconciliation = {
        let _range = perfetto_range!("ait.cli.status.reconciliation_cache");
        workflow_reconciliation_cached_summary(repo)
    };
    let workspace_clean = workspace
        .get("clean")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let worktree_name = if repo.is_worktree() {
        repo.config
            .get("worktree_name")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value)))
    } else {
        None
    };

    let _range = perfetto_range!("ait.cli.status.assemble");
    Ok(json!({
        "repo_name": repo_name,
        "workspace_root": repo.workspace_root().to_string_lossy().to_string(),
        "current_line": current_line,
        "head_snapshot_id": head_snapshot_id,
        "snapshot_count": snapshot_count,
        "remote_count": remote_count,
        "default_remote": repo.default_remote_name(),
        "workspace_status": if workspace_clean { "clean" } else { "dirty" },
        "workspace_dirty": !workspace_clean,
        "workspace_changed_count": workspace.get("changed_count").cloned().unwrap_or(JsonValue::from(0)),
        "workspace_modified_count": workspace.get("modified_paths").and_then(JsonValue::as_array).map(|rows| rows.len()).unwrap_or(0),
        "workspace_missing_count": workspace.get("missing_paths").and_then(JsonValue::as_array).map(|rows| rows.len()).unwrap_or(0),
        "workspace_untracked_count": workspace.get("untracked_paths").and_then(JsonValue::as_array).map(|rows| rows.len()).unwrap_or(0),
        "workspace_changed_paths_sample": json_string_list(workspace.get("changed_paths")).into_iter().take(10).collect::<Vec<_>>(),
        "is_worktree": repo.is_worktree(),
        "worktree_name": worktree_name,
        "worktree_hygiene": {
            "total_count": worktree_hygiene.get("total_count").cloned().unwrap_or(JsonValue::from(0)),
            "stale_count": worktree_hygiene.get("stale_count").cloned().unwrap_or(JsonValue::from(0)),
            "cleanup_candidate_count": worktree_hygiene.get("safe_auto_remove_count").and_then(JsonValue::as_i64).unwrap_or(0)
                + worktree_hygiene.get("safe_cleanup_candidate_count").and_then(JsonValue::as_i64).unwrap_or(0),
            "manual_review_candidate_count": worktree_hygiene.get("manual_review_candidate_count").cloned().unwrap_or(JsonValue::from(0)),
            "protected_count": worktree_hygiene.get("protected_count").cloned().unwrap_or(JsonValue::from(0)),
        },
        "line_hygiene": {
            "mode": "metadata_only",
            "idle_for": JsonValue::Null,
            "candidate_count": JsonValue::Null,
            "protected_count": JsonValue::Null,
            "inspected_count": line_count,
            "detail_command": "ait line cleanup --include-protected",
        },
        "reconciliation": reconciliation,
    }))
}

fn repo_status_snapshot_count(repo: &RepoRuntime) -> Result<i64, String> {
    let store = repo.repo_status_store()?;
    repo_status_snapshot_count_with_store(&store)
}

pub(super) fn configured_remote_names_with_store<S>(store: &S) -> Result<Vec<String>, String>
where
    S: RemoteStore + ?Sized,
{
    let mut names = list_remotes_with_remote_store(store)?
        .into_iter()
        .map(|remote| remote.name)
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

pub(super) fn repo_status_snapshot_count_with_store<S>(store: &S) -> Result<i64, String>
where
    S: RepoStatusStore + ?Sized,
{
    let counts = storage_counts_with_repo_status_store(store).map_err(|err| err.to_string())?;
    Ok(counts.snapshot_count)
}

fn line_cleanup_inventory(
    repo: &RepoRuntime,
    idle_for: Option<&str>,
    cleanup_kind: Option<&str>,
    include_protected: bool,
) -> Result<JsonValue, String> {
    let normalized_kind = normalize_line_cleanup_kind(cleanup_kind)?;
    let rows = list_local_lines(repo)?;
    let indexes = collect_line_usage_indexes(repo)?;
    let current_line_name = repo.current_line_name()?;
    let default_line_name = repo.default_line_name();
    let (idle_for_delta, idle_for_label) = normalize_line_idle_for(idle_for)?;
    let reference_now = Utc::now();
    let mut candidates = Vec::new();
    let mut protected = Vec::new();
    let mut inspected_count = 0_i64;
    let mut protected_count = 0_i64;

    for row in rows {
        let decision = line_cleanup_decision(
            repo,
            &row,
            idle_for_delta,
            &idle_for_label,
            normalized_kind.as_deref(),
            &indexes,
            &current_line_name,
            &default_line_name,
            reference_now,
        );
        let mut enriched = row
            .as_object()
            .cloned()
            .ok_or_else(|| "line cleanup row must decode to an object".to_string())?;
        for (key, value) in decision {
            enriched.insert(key, value);
        }
        let enriched_value = JsonValue::Object(enriched.clone());
        inspected_count += 1;
        if enriched_value
            .get("cleanup_candidate")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            candidates.push(JsonValue::Object(enriched));
        } else {
            protected_count += 1;
            if include_protected {
                protected.push(JsonValue::Object(enriched));
            }
        }
    }

    candidates.sort_by(|left, right| {
        (
            string_field(left, "last_activity_at").unwrap_or_default(),
            string_field(left, "line_name").unwrap_or_default(),
        )
            .cmp(&(
                string_field(right, "last_activity_at").unwrap_or_default(),
                string_field(right, "line_name").unwrap_or_default(),
            ))
    });
    protected.sort_by(|left, right| {
        (
            string_field(left, "line_name").unwrap_or_default(),
            string_field(left, "protected_reason").unwrap_or_default(),
        )
            .cmp(&(
                string_field(right, "line_name").unwrap_or_default(),
                string_field(right, "protected_reason").unwrap_or_default(),
            ))
    });

    Ok(json!({
        "idle_for": idle_for_label,
        "cleanup_kind": normalized_kind,
        "include_protected": include_protected,
        "inspected_count": inspected_count,
        "candidate_count": candidates.len(),
        "protected_count": protected_count,
        "candidates": candidates,
        "protected": if include_protected { protected } else { Vec::new() },
    }))
}

fn line_cleanup_unlocked(
    repo: &RepoRuntime,
    idle_for: Option<&str>,
    cleanup_kind: Option<&str>,
    limit: Option<usize>,
    include_protected: bool,
    apply: bool,
) -> Result<JsonValue, String> {
    if limit == Some(0) {
        return Err("`--limit` must be greater than zero.".to_string());
    }
    let payload = line_cleanup_inventory(repo, idle_for, cleanup_kind, include_protected)?;
    let candidates = payload
        .get("candidates")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let selected = match limit {
        Some(value) => candidates.into_iter().take(value).collect::<Vec<_>>(),
        None => candidates,
    };
    let planned_rows = selected
        .iter()
        .map(|row| {
            json!({
                "line_id": string_field(row, "line_id"),
                "line_name": string_field(row, "line_name"),
                "lifecycle_kind": string_field(row, "lifecycle_kind"),
                "cleanup_policy": string_field(row, "cleanup_policy"),
                "last_activity_at": string_field(row, "last_activity_at"),
                "cleanup_reason": string_field(row, "cleanup_reason"),
            })
        })
        .collect::<Vec<_>>();
    let mut archived_rows = Vec::new();
    if apply {
        for row in selected {
            let line_name = required_string_field(&row, "line_name")?;
            archived_rows.push(line_archive_unlocked(repo, &line_name, None)?);
        }
    }
    let mut result = payload
        .as_object()
        .cloned()
        .ok_or_else(|| "line cleanup inventory must decode to an object".to_string())?;
    result.insert(
        "mode".to_string(),
        JsonValue::String(if apply { "applied" } else { "preview" }.to_string()),
    );
    result.insert("applied".to_string(), JsonValue::Bool(apply));
    result.insert(
        "limit".to_string(),
        limit.map(JsonValue::from).unwrap_or(JsonValue::Null),
    );
    result.insert(
        "planned_count".to_string(),
        JsonValue::from(planned_rows.len()),
    );
    result.insert("planned_rows".to_string(), JsonValue::Array(planned_rows));
    result.insert(
        "archived_count".to_string(),
        JsonValue::from(archived_rows.len()),
    );
    result.insert("archived_rows".to_string(), JsonValue::Array(archived_rows));
    Ok(JsonValue::Object(result))
}

pub fn line_cleanup(
    repo: &RepoRuntime,
    idle_for: Option<&str>,
    cleanup_kind: Option<&str>,
    limit: Option<usize>,
    include_protected: bool,
    apply: bool,
) -> Result<JsonValue, String> {
    if !apply {
        return line_cleanup_unlocked(
            repo,
            idle_for,
            cleanup_kind,
            limit,
            include_protected,
            false,
        );
    }
    run_locked_workspace_command(repo, "ait-cli line cleanup", || {
        line_cleanup_unlocked(repo, idle_for, cleanup_kind, limit, include_protected, true)
    })
}
