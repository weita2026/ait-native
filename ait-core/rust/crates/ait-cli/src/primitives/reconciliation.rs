use super::*;

const RECONCILIATION_CONTRACT: &str = "workflow-reconciliation/v1";
const DEFAULT_RECONCILIATION_LIMIT: usize = 100;
const MAX_RECONCILIATION_LIMIT: usize = 10_000;

mod apply;
pub use apply::{
    workflow_reconcile_apply, workflow_reconcile_automatic,
    workflow_reconcile_automatic_best_effort, workflow_reconciliation_cached_summary,
    AutomaticReconciliationScope, AutomaticReconciliationTrigger,
};

#[derive(Clone, Debug, Default)]
struct JoinedRow {
    local: Option<JsonValue>,
    remote: Option<JsonValue>,
    publication_mapping_blocked: bool,
}

impl JoinedRow {
    fn authoritative(&self) -> Option<&JsonValue> {
        self.remote.as_ref().or(self.local.as_ref())
    }

    fn status(&self) -> Option<String> {
        self.authoritative()
            .and_then(|row| string_field(row, "status"))
    }
}

#[derive(Clone, Debug, Default)]
struct JoinedInventory {
    rows: BTreeMap<String, JoinedRow>,
    aliases: BTreeMap<String, String>,
    findings: Vec<JsonValue>,
}

impl JoinedInventory {
    fn canonical_id(&self, identity: &str) -> Option<String> {
        self.aliases.get(identity).cloned().or_else(|| {
            self.rows
                .contains_key(identity)
                .then(|| identity.to_string())
        })
    }

    fn get(&self, identity: &str) -> Option<&JoinedRow> {
        let canonical = self
            .aliases
            .get(identity)
            .map(String::as_str)
            .unwrap_or(identity);
        self.rows.get(canonical)
    }

    fn matches_filter(&self, task_filter: Option<&str>, identity: Option<&str>) -> bool {
        let Some(filter) = normalized_text(task_filter) else {
            return true;
        };
        let Some(identity) = normalized_text(identity) else {
            return false;
        };
        let filter = self.canonical_id(&filter).unwrap_or(filter);
        let identity = self.canonical_id(&identity).unwrap_or(identity);
        filter == identity
    }

    fn insert_alias(&mut self, identity: &str, canonical: &str, kind: &str) -> Result<(), String> {
        if let Some(existing) = self.aliases.get(identity) {
            if existing != canonical {
                return Err(format!(
                    "Reconciliation {kind} identity {identity} resolves to both {existing} and {canonical}; refusing ambiguous publication mapping."
                ));
            }
            return Ok(());
        }
        self.aliases
            .insert(identity.to_string(), canonical.to_string());
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct RemoteReadError {
    source: &'static str,
    message: String,
}

#[derive(Clone, Debug)]
struct ReconciliationInventoryInput {
    repo_name: String,
    captured_at: String,
    remote_name: Option<String>,
    task_filter: Option<String>,
    current_line: String,
    default_line: String,
    local_tasks: Vec<JsonValue>,
    remote_tasks: Vec<JsonValue>,
    local_changes: Vec<JsonValue>,
    remote_changes: Vec<JsonValue>,
    local_lines: Vec<JsonValue>,
    remote_lines: Vec<JsonValue>,
    worktrees: Vec<JsonValue>,
    mutation_receipts: Vec<JsonValue>,
    workspace_lock: JsonValue,
    remote_errors: Vec<RemoteReadError>,
}

fn nested_string_field(row: &JsonValue, path: &[&str]) -> Option<String> {
    let mut current = row;
    for key in path {
        current = current.get(*key)?;
    }
    current
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn landed_snapshot_id(row: &JsonValue) -> Option<String> {
    string_field(row, "landed_snapshot_id")
        .or_else(|| nested_string_field(row, &["landing_summary", "result", "landed_snapshot_id"]))
        .or_else(|| {
            nested_string_field(
                row,
                &["landing_summary", "result", "target_line_head_snapshot_id"],
            )
        })
        .or_else(|| nested_string_field(row, &["landing_summary", "result", "target_line_head"]))
}

fn land_base_snapshot_id(row: &JsonValue) -> Option<String> {
    string_field(row, "base_snapshot_id")
        .or_else(|| string_field(row, "base_line_head_snapshot_id"))
        .or_else(|| {
            nested_string_field(
                row,
                &["landing_summary", "result", "expected_base_snapshot_id"],
            )
        })
}

fn collect_mutation_receipts<'a>(
    rows: impl IntoIterator<Item = (&'static str, &'static str, &'a JsonValue)>,
) -> Vec<JsonValue> {
    let mut receipts = Vec::new();
    for (scope, object_kind, row) in rows {
        let object_id = string_field(row, "change_ref")
            .or_else(|| string_field(row, "change_id"))
            .or_else(|| string_field(row, "task_id"));
        for key in ["mutation_receipts", "receipts"] {
            if let Some(values) = row.get(key).and_then(JsonValue::as_array) {
                for value in values {
                    receipts.push(json!({
                        "scope": scope,
                        "object_kind": object_kind,
                        "object_id": object_id,
                        "receipt_kind": "mutation",
                        "receipt": value,
                    }));
                }
            }
        }
        for key in ["mutation_receipt", "land_receipt"] {
            if let Some(value) = row.get(key).filter(|value| value.is_object()) {
                receipts.push(json!({
                    "scope": scope,
                    "object_kind": object_kind,
                    "object_id": object_id,
                    "receipt_kind": key,
                    "receipt": value,
                }));
            }
        }
        if let Some(summary) = row.get("landing_summary").filter(|value| value.is_object()) {
            receipts.push(json!({
                "scope": scope,
                "object_kind": object_kind,
                "object_id": object_id,
                "receipt_kind": "land_submission",
                "receipt": summary,
            }));
        }
    }
    receipts
}

fn local_land_closeout_evidence(
    input: &ReconciliationInventoryInput,
    local_change_ref: &str,
    local_change: &JsonValue,
) -> Vec<JsonValue> {
    let mut evidence = input
        .mutation_receipts
        .iter()
        .filter(|receipt| {
            string_field(receipt, "scope").as_deref() == Some("local")
                && string_field(receipt, "object_kind").as_deref() == Some("change")
                && string_field(receipt, "object_id").as_deref() == Some(local_change_ref)
                && (matches!(
                    string_field(receipt, "receipt_kind").as_deref(),
                    Some("land_receipt" | "land_submission")
                ) || matches!(
                    receipt
                        .get("receipt")
                        .and_then(|payload| string_field(payload, "action"))
                        .as_deref(),
                    Some("submit_land" | "local_land")
                ))
        })
        .cloned()
        .collect::<Vec<_>>();
    let landed_snapshot_id = landed_snapshot_id(local_change);
    let pre_land_target_snapshot_id = string_field(local_change, "pre_land_target_snapshot_id");
    let landed_at = string_field(local_change, "landed_at");
    if landed_snapshot_id.is_some() || pre_land_target_snapshot_id.is_some() || landed_at.is_some()
    {
        evidence.push(json!({
            "scope": "local",
            "object_kind": "change",
            "object_id": local_change_ref,
            "receipt_kind": "local_land_payload",
            "receipt": {
                "landed_snapshot_id": landed_snapshot_id,
                "pre_land_target_snapshot_id": pre_land_target_snapshot_id,
                "landed_at": landed_at,
            },
        }));
    }
    evidence
}

fn read_workspace_lock_evidence(repo: &RepoRuntime) -> JsonValue {
    let path = crate::workspace_lock::workspace_command_lock_path(repo);
    if !path.is_file() {
        return json!({
            "path": path,
            "state": "absent",
            "active": false,
            "owned_by_current_process": false,
            "blocks_reconciliation": false,
            "metadata": JsonValue::Null,
        });
    }
    let metadata = foundation::read_json_document(&path);
    let active = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .ok()
        .is_some_and(|file| match file.try_lock_exclusive() {
            Ok(()) => {
                let _ = fs2::FileExt::unlock(&file);
                false
            }
            Err(_) => true,
        });
    let owned_by_current_process = active
        && metadata.get("pid").and_then(JsonValue::as_u64) == Some(u64::from(std::process::id()));
    json!({
        "path": path,
        "state": if active { "held" } else { "idle" },
        "active": active,
        "owned_by_current_process": owned_by_current_process,
        "blocks_reconciliation": active && !owned_by_current_process,
        "metadata": if metadata.as_object().is_some_and(|object| !object.is_empty()) { metadata } else { JsonValue::Null },
    })
}

fn json_array(payload: JsonValue, label: &str) -> Result<Vec<JsonValue>, String> {
    payload
        .as_array()
        .cloned()
        .ok_or_else(|| format!("{label} inventory must decode to an array."))
}

fn remote_array(
    source: &'static str,
    result: Result<JsonValue, String>,
    errors: &mut Vec<RemoteReadError>,
) -> Vec<JsonValue> {
    match result {
        Ok(payload) => match payload.as_array() {
            Some(rows) => rows.clone(),
            None => {
                errors.push(RemoteReadError {
                    source,
                    message: format!("Remote {source} inventory did not return an array."),
                });
                Vec::new()
            }
        },
        Err(message) => {
            errors.push(RemoteReadError { source, message });
            Vec::new()
        }
    }
}

fn verified_worktree_inventory(repo: &RepoRuntime) -> Result<Vec<JsonValue>, String> {
    let rows = json_array(worktree_list(repo, false)?, "worktree")?;
    rows.into_iter()
        .map(|row| {
            let mut object = row
                .as_object()
                .cloned()
                .ok_or_else(|| "Worktree inventory row must decode to an object.".to_string())?;
            let name = string_field(&row, "name").unwrap_or_default();
            if let Ok(metadata) = worktree::load_worktree_metadata(repo, &name) {
                for key in [
                    "cleanup_policy",
                    "creation_kind",
                    "merge_state",
                    "rebase_state",
                ] {
                    if let Some(value) = metadata.get(key) {
                        object.insert(key.to_string(), value.clone());
                    }
                }
            }
            object
                .entry("merge_state".to_string())
                .or_insert_with(|| JsonValue::String("idle".to_string()));
            object
                .entry("rebase_state".to_string())
                .or_insert_with(|| JsonValue::String("idle".to_string()));
            let path_is_present = row
                .get("exists")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            let cached_status = string_field(&row, "workspace_status").unwrap_or_default();
            if !path_is_present || matches!(cached_status.as_str(), "missing" | "detached") {
                object.insert(
                    "reconciliation_status_source".to_string(),
                    JsonValue::String("filesystem_metadata".to_string()),
                );
                return Ok(JsonValue::Object(object));
            }
            match worktree_status(repo, Some(&name), None, None) {
                Ok(status) => {
                    let clean = status
                        .get("clean")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false);
                    object.insert("clean".to_string(), JsonValue::Bool(clean));
                    object.insert(
                        "workspace_status".to_string(),
                        JsonValue::String(if clean { "clean" } else { "dirty" }.to_string()),
                    );
                    for key in [
                        "changed_count",
                        "changed_paths",
                        "modified_paths",
                        "missing_paths",
                        "untracked_paths",
                    ] {
                        if let Some(value) = status.get(key) {
                            object.insert(key.to_string(), value.clone());
                        }
                    }
                    object.insert(
                        "reconciliation_status_source".to_string(),
                        JsonValue::String("verified_read_only".to_string()),
                    );
                }
                Err(message) => {
                    object.insert(
                        "workspace_status".to_string(),
                        JsonValue::String("detached".to_string()),
                    );
                    object.insert("clean".to_string(), JsonValue::Null);
                    object.insert(
                        "reconciliation_status_source".to_string(),
                        JsonValue::String("inspection_failed".to_string()),
                    );
                    object.insert(
                        "reconciliation_inspection_error".to_string(),
                        JsonValue::String(message),
                    );
                }
            }
            Ok(JsonValue::Object(object))
        })
        .collect()
}

fn read_reconciliation_inventory(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    task_filter: Option<&str>,
    use_default_remote: bool,
) -> Result<ReconciliationInventoryInput, String> {
    let local_tasks = json_array(task_list(repo, true, None)?, "local task")?;
    let local_changes = json_array(change_list(repo, true, None)?, "local change")?;
    let local_lines = json_array(line_list(repo, true, false, None)?, "local line")?;
    let worktrees = verified_worktree_inventory(repo)?;
    let selected_remote = normalized_text(remote_name).or_else(|| {
        if use_default_remote {
            repo.default_remote_name()
        } else {
            None
        }
    });
    let mut remote_errors = Vec::new();
    let (remote_tasks, remote_changes, remote_lines) = match selected_remote.as_deref() {
        Some(remote) => (
            remote_array(
                "tasks",
                task_list(repo, false, Some(remote)),
                &mut remote_errors,
            ),
            remote_array(
                "changes",
                change_list(repo, false, Some(remote)),
                &mut remote_errors,
            ),
            remote_array(
                "lines",
                line_list(repo, true, false, Some(remote)),
                &mut remote_errors,
            ),
        ),
        None => (Vec::new(), Vec::new(), Vec::new()),
    };
    let mutation_receipts = collect_mutation_receipts(
        local_tasks
            .iter()
            .map(|row| ("local", "task", row))
            .chain(local_changes.iter().map(|row| ("local", "change", row)))
            .chain(remote_tasks.iter().map(|row| ("remote", "task", row)))
            .chain(remote_changes.iter().map(|row| ("remote", "change", row))),
    );
    Ok(ReconciliationInventoryInput {
        repo_name: repo.repo_name(),
        captured_at: system_event_timestamp(),
        remote_name: selected_remote,
        task_filter: normalized_text(task_filter),
        current_line: repo.current_line_name()?,
        default_line: repo.default_line_name(),
        local_tasks,
        remote_tasks,
        local_changes,
        remote_changes,
        local_lines,
        remote_lines,
        worktrees,
        mutation_receipts,
        workspace_lock: read_workspace_lock_evidence(repo),
        remote_errors,
    })
}

fn selected_publication_target(
    row: &JsonValue,
    selected_remote: Option<&str>,
    target_field: &str,
    kind: &str,
    local_identity: &str,
) -> Result<Option<String>, String> {
    if string_field(row, "publication_state").as_deref() != Some("published") {
        return Ok(None);
    }
    let Some(selected_remote) = normalized_text(selected_remote) else {
        return Ok(None);
    };
    let published_remote = string_field(row, "published_remote_name").ok_or_else(|| {
        format!(
            "Published Local {kind} {local_identity} is missing published_remote_name; refusing reconciliation publication inference."
        )
    })?;
    if published_remote != selected_remote {
        return Ok(None);
    }
    string_field(row, target_field).map(Some).ok_or_else(|| {
        format!(
            "Published Local {kind} {local_identity} is missing {target_field}; refusing reconciliation publication inference."
        )
    })
}

fn joined_task_rows(input: &ReconciliationInventoryInput) -> Result<JoinedInventory, String> {
    let remote_inventory_complete = !remote_source_failed(input, "tasks");
    let mut remote_by_id = BTreeMap::<String, JsonValue>::new();
    for row in &input.remote_tasks {
        let Some(task_id) = string_field(row, "task_id") else {
            continue;
        };
        if remote_by_id.insert(task_id.clone(), row.clone()).is_some() {
            return Err(format!(
                "Remote Task inventory contains duplicate identity {task_id}; refusing reconciliation."
            ));
        }
    }

    let mut joined = JoinedInventory::default();
    for row in &input.local_tasks {
        let Some(local_task_id) = string_field(row, "task_id") else {
            continue;
        };
        let published_task_id = selected_publication_target(
            row,
            input.remote_name.as_deref(),
            "published_task_id",
            "Task",
            &local_task_id,
        )?;
        let missing_remote_target = remote_inventory_complete
            && published_task_id
                .as_ref()
                .is_some_and(|remote_task_id| !remote_by_id.contains_key(remote_task_id));
        let publication_mapping_unverified =
            published_task_id.is_some() && !remote_inventory_complete;
        let canonical = published_task_id
            .clone()
            .unwrap_or_else(|| local_task_id.clone());
        if joined
            .rows
            .get(&canonical)
            .is_some_and(|entry| entry.local.is_some())
        {
            return Err(format!(
                "Multiple Local Tasks resolve to Remote Task {canonical}; refusing ambiguous publication mapping."
            ));
        }
        joined.insert_alias(&local_task_id, &canonical, "Task")?;
        joined.insert_alias(&canonical, &canonical, "Task")?;
        let entry = joined.rows.entry(canonical).or_default();
        entry.local = Some(row.clone());
        entry.publication_mapping_blocked = missing_remote_target || publication_mapping_unverified;
        if missing_remote_target {
            let remote_task_id = published_task_id.unwrap_or_default();
            push_finding(
                &mut joined.findings,
                "publication.task_target_missing",
                "protected",
                "error",
                identity_map([
                    ("task_id", Some(local_task_id.clone())),
                    ("local_task_id", Some(local_task_id.clone())),
                    ("remote_task_id", Some(remote_task_id.clone())),
                ]),
                json!({
                    "publication_state": "published",
                    "published_remote_name": string_field(row, "published_remote_name"),
                    "published_task_id": remote_task_id,
                    "remote_inventory_complete": true,
                }),
                "repair_or_explain_missing_published_task",
                format!("ait task audit {local_task_id}"),
                "The exact published Remote Task target is absent from a complete selected-Remote inventory; no lifecycle inference is safe for this mapping.",
            );
        }
    }
    for (remote_task_id, row) in remote_by_id {
        let canonical = remote_task_id.clone();
        if joined
            .rows
            .get(&canonical)
            .is_some_and(|entry| entry.remote.is_some())
        {
            return Err(format!(
                "Multiple Remote Tasks resolve to reconciliation identity {canonical}; refusing ambiguous inventory."
            ));
        }
        joined.insert_alias(&remote_task_id, &canonical, "Task")?;
        joined.rows.entry(canonical).or_default().remote = Some(row);
    }
    Ok(joined)
}

fn change_reference(row: &JsonValue) -> Option<String> {
    string_field(row, "change_ref").or_else(|| {
        let task_id = string_field(row, "task_id")?;
        let change_id = string_field(row, "change_id")?;
        Some(format!("{task_id}/{change_id}"))
    })
}

fn joined_change_rows(
    input: &ReconciliationInventoryInput,
    tasks: &JoinedInventory,
) -> Result<JoinedInventory, String> {
    let remote_inventory_complete = !remote_source_failed(input, "changes");
    let mut remote_by_ref = BTreeMap::<String, JsonValue>::new();
    for row in &input.remote_changes {
        let Some(change_ref) = change_reference(row) else {
            continue;
        };
        if remote_by_ref
            .insert(change_ref.clone(), row.clone())
            .is_some()
        {
            return Err(format!(
                "Remote Change inventory contains duplicate identity {change_ref}; refusing reconciliation."
            ));
        }
    }

    let mut joined = JoinedInventory::default();
    for row in &input.local_changes {
        let Some(local_change_ref) = change_reference(row) else {
            continue;
        };
        let published_change_ref = selected_publication_target(
            row,
            input.remote_name.as_deref(),
            "published_change_id",
            "Change",
            &local_change_ref,
        )?;
        let missing_remote_target = remote_inventory_complete
            && published_change_ref
                .as_ref()
                .is_some_and(|remote_change_ref| !remote_by_ref.contains_key(remote_change_ref));
        let publication_mapping_unverified =
            published_change_ref.is_some() && !remote_inventory_complete;
        let canonical = published_change_ref
            .clone()
            .unwrap_or_else(|| local_change_ref.clone());
        if joined
            .rows
            .get(&canonical)
            .is_some_and(|entry| entry.local.is_some())
        {
            return Err(format!(
                "Multiple Local Changes resolve to Remote Change {canonical}; refusing ambiguous publication mapping."
            ));
        }
        joined.insert_alias(&local_change_ref, &canonical, "Change")?;
        joined.insert_alias(&canonical, &canonical, "Change")?;
        let entry = joined.rows.entry(canonical).or_default();
        entry.local = Some(row.clone());
        entry.publication_mapping_blocked = missing_remote_target || publication_mapping_unverified;
        if missing_remote_target {
            let remote_change_ref = published_change_ref.unwrap_or_default();
            let local_task_id = string_field(row, "task_id");
            let remote_task_id = local_task_id
                .as_ref()
                .and_then(|task_id| tasks.canonical_id(task_id))
                .filter(|task_id| task_id != local_task_id.as_deref().unwrap_or_default());
            push_finding(
                &mut joined.findings,
                "publication.change_target_missing",
                "protected",
                "error",
                identity_map([
                    ("task_id", local_task_id.clone()),
                    ("local_task_id", local_task_id.clone()),
                    ("remote_task_id", remote_task_id),
                    ("change_ref", Some(local_change_ref.clone())),
                    ("remote_change_ref", Some(remote_change_ref.clone())),
                ]),
                json!({
                    "publication_state": "published",
                    "published_remote_name": string_field(row, "published_remote_name"),
                    "published_change_id": remote_change_ref,
                    "remote_inventory_complete": true,
                }),
                "repair_or_explain_missing_published_change",
                format!(
                    "ait task audit {}",
                    local_task_id.as_deref().unwrap_or("<task-id>")
                ),
                "The exact published Remote Change target is absent from a complete selected-Remote inventory; no lifecycle inference is safe for this mapping.",
            );
        }
    }
    for (remote_change_ref, row) in remote_by_ref {
        let canonical = remote_change_ref.clone();
        if joined
            .rows
            .get(&canonical)
            .is_some_and(|entry| entry.remote.is_some())
        {
            return Err(format!(
                "Multiple Remote Changes resolve to reconciliation identity {canonical}; refusing ambiguous inventory."
            ));
        }
        joined.insert_alias(&remote_change_ref, &canonical, "Change")?;
        joined.rows.entry(canonical).or_default().remote = Some(row);
    }

    for (change_ref, change) in &mut joined.rows {
        let (Some(local), Some(remote)) = (change.local.as_ref(), change.remote.as_ref()) else {
            continue;
        };
        let local_task_id = string_field(local, "task_id").ok_or_else(|| {
            format!("Local Change {change_ref} is missing its owning Task identity.")
        })?;
        let remote_task_id = string_field(remote, "task_id").ok_or_else(|| {
            format!("Remote Change {change_ref} is missing its owning Task identity.")
        })?;
        if tasks
            .get(&local_task_id)
            .is_some_and(|task| task.publication_mapping_blocked)
        {
            change.publication_mapping_blocked = true;
            continue;
        }
        let local_owner = tasks
            .canonical_id(&local_task_id)
            .unwrap_or(local_task_id.clone());
        let remote_owner = tasks
            .canonical_id(&remote_task_id)
            .unwrap_or(remote_task_id.clone());
        if local_owner != remote_owner {
            return Err(format!(
                "Published Change mapping {change_ref} disagrees on Task ownership: Local {local_task_id} resolves to {local_owner}, Remote owner is {remote_task_id}; refusing reconciliation."
            ));
        }
    }
    Ok(joined)
}

fn joined_change_for_binding<'a>(
    changes: &'a JoinedInventory,
    bound_change_ref: &str,
    task_id: Option<&str>,
) -> Option<&'a JoinedRow> {
    if let Some(change) = changes.get(bound_change_ref) {
        return Some(change);
    }
    let short_id = bound_change_ref
        .rsplit_once('/')
        .map(|(_, suffix)| suffix)
        .unwrap_or(bound_change_ref);
    changes.rows.values().find(|change| {
        [change.local.as_ref(), change.remote.as_ref()]
            .into_iter()
            .flatten()
            .any(|row| {
                let row_task_id = string_field(row, "task_id");
                let task_matches = task_id.is_none() || row_task_id.as_deref() == task_id;
                let id_matches = string_field(row, "change_id").as_deref() == Some(short_id)
                    || string_field(row, "change_ref").as_deref() == Some(bound_change_ref);
                task_matches && id_matches
            })
    })
}

fn remote_source_failed(input: &ReconciliationInventoryInput, source: &str) -> bool {
    input
        .remote_errors
        .iter()
        .any(|error| error.source == source)
}

fn task_status_is_terminal(status: Option<&str>) -> bool {
    matches!(
        status,
        Some("completed" | "canceled" | "cancelled" | "abandoned" | "stopped")
    )
}

fn task_status_is_active(status: Option<&str>) -> bool {
    matches!(status, Some("active" | "planned" | "in_progress" | "open"))
}

fn change_status_is_terminal(status: Option<&str>) -> bool {
    matches!(
        status,
        Some(
            "landed"
                | "closed"
                | "archived"
                | "canceled"
                | "cancelled"
                | "abandoned"
                | "superseded"
        )
    )
}

fn change_status_is_open(status: Option<&str>) -> bool {
    status.is_some_and(|status| !change_status_is_terminal(Some(status)))
}

fn task_id_from_feature_line(line_name: &str) -> Option<String> {
    let suffix = line_name.strip_prefix("feature/")?;
    let token = suffix.split('/').next()?.trim();
    let uppercase = token.to_ascii_uppercase();
    let (family, ordinal) = uppercase.split_once('-')?;
    if ordinal.is_empty() || !ordinal.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let family = family.as_bytes();
    if !(2..=4).contains(&family.len())
        || !matches!(family.first(), Some(b'L' | b'R'))
        || family.last() != Some(&b'T')
        || !family[1..family.len() - 1]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(uppercase)
}

fn task_id_has_remote_origin(task_id: &str) -> bool {
    task_id.as_bytes().first() == Some(&b'R')
}

fn identity_map(
    entries: impl IntoIterator<Item = (&'static str, Option<String>)>,
) -> JsonMap<String, JsonValue> {
    entries
        .into_iter()
        .filter_map(|(key, value)| normalized_text(value.as_deref()).map(|value| (key, value)))
        .map(|(key, value)| (key.to_string(), JsonValue::String(value)))
        .collect()
}

fn finding_id(code: &str, identities: &JsonMap<String, JsonValue>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    hasher.update([0]);
    for (key, value) in identities {
        hasher.update(key.as_bytes());
        hasher.update([0]);
        if let Some(text) = value.as_str() {
            hasher.update(text.as_bytes());
        }
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let suffix = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    format!("RCF-{suffix}")
}

fn reconcile_command(remote_name: Option<&str>, task_id: Option<&str>) -> String {
    let mut command = "ait workflow reconcile".to_string();
    if let Some(remote) = normalized_text(remote_name) {
        command.push_str(" --remote ");
        command.push_str(&remote);
    }
    if let Some(task_id) = normalized_text(task_id) {
        command.push_str(" --task ");
        command.push_str(&task_id);
    }
    command.push_str(" --apply --safe-only");
    command
}

#[allow(clippy::too_many_arguments)]
fn push_finding(
    findings: &mut Vec<JsonValue>,
    code: &'static str,
    disposition: &'static str,
    severity: &'static str,
    identities: JsonMap<String, JsonValue>,
    evidence: JsonValue,
    action_code: &'static str,
    action_command: String,
    detail: impl Into<String>,
) {
    let id = finding_id(code, &identities);
    findings.push(json!({
        "finding_id": id,
        "code": code,
        "disposition": disposition,
        "severity": severity,
        "identities": identities,
        "evidence": evidence,
        "recommended_action": {
            "code": action_code,
            "command": action_command,
            "automatic": matches!(disposition, "safe_metadata_repair" | "safe_auto_cleanup"),
        },
        "detail": detail.into(),
    }));
}

fn worktree_operation_in_progress(worktree: &JsonValue) -> bool {
    string_field(worktree, "merge_state").is_some_and(|state| state != "idle")
        || string_field(worktree, "rebase_state").is_some_and(|state| state != "idle")
}

fn finding_matches_task_filter(
    finding: &JsonValue,
    task_filter: Option<&str>,
    tasks: &JoinedInventory,
) -> bool {
    let Some(filter) = normalized_text(task_filter) else {
        return true;
    };
    let identities = finding.get("identities").unwrap_or(&JsonValue::Null);
    ["task_id", "local_task_id", "remote_task_id"]
        .into_iter()
        .filter_map(|field| string_field(identities, field))
        .any(|identity| tasks.matches_filter(Some(&filter), Some(&identity)))
}

fn line_rows_by_name(rows: &[JsonValue]) -> BTreeMap<String, JsonValue> {
    rows.iter()
        .filter_map(|row| string_field(row, "line_name").map(|name| (name, row.clone())))
        .collect()
}

fn plan_drift_is_present(row: &JsonValue) -> bool {
    match row.get("plan_drift_state") {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(value)) => *value,
        Some(JsonValue::String(value)) => {
            let value = value.trim().to_ascii_lowercase();
            !value.is_empty() && !matches!(value.as_str(), "none" | "clean" | "in_sync")
        }
        Some(JsonValue::Array(values)) => !values.is_empty(),
        Some(JsonValue::Object(values)) => !values.is_empty(),
        Some(_) => true,
    }
}

fn build_reconciliation_inventory(
    input: ReconciliationInventoryInput,
    safe_only: bool,
    limit: usize,
) -> Result<JsonValue, String> {
    if limit == 0 || limit > MAX_RECONCILIATION_LIMIT {
        return Err(format!(
            "--limit must be between 1 and {MAX_RECONCILIATION_LIMIT}."
        ));
    }
    let tasks = joined_task_rows(&input)?;
    let changes = joined_change_rows(&input, &tasks)?;
    let local_lines = line_rows_by_name(&input.local_lines);
    let remote_lines = line_rows_by_name(&input.remote_lines);
    let remote_task_inventory_complete = !remote_source_failed(&input, "tasks");
    let remote_change_inventory_complete = !remote_source_failed(&input, "changes");
    let remote_line_inventory_complete = !remote_source_failed(&input, "lines");
    let mut changes_by_task = BTreeMap::<String, Vec<(String, JoinedRow)>>::new();
    for (change_ref, joined) in &changes.rows {
        if let Some(task_id) = joined
            .authoritative()
            .and_then(|row| string_field(row, "task_id"))
        {
            let task_id = tasks.canonical_id(&task_id).unwrap_or(task_id);
            changes_by_task
                .entry(task_id)
                .or_default()
                .push((change_ref.clone(), joined.clone()));
        }
    }
    let worktrees_by_task = input
        .worktrees
        .iter()
        .filter_map(|row| {
            string_field(row, "bound_task_id").map(|task| {
                let task = tasks.canonical_id(&task).unwrap_or(task);
                (task, row)
            })
        })
        .fold(
            BTreeMap::<String, Vec<&JsonValue>>::new(),
            |mut map, (task, row)| {
                map.entry(task).or_default().push(row);
                map
            },
        );
    let worktree_lines = input
        .worktrees
        .iter()
        .flat_map(|row| {
            [
                string_field(row, "current_line"),
                string_field(row, "registered_line_name"),
            ]
        })
        .flatten()
        .collect::<BTreeSet<_>>();
    let mut findings = tasks
        .findings
        .iter()
        .chain(changes.findings.iter())
        .cloned()
        .collect::<Vec<JsonValue>>();

    for error in &input.remote_errors {
        push_finding(
            &mut findings,
            "remote.inventory_unavailable",
            "manual_resolution",
            "error",
            identity_map([
                ("remote_name", input.remote_name.clone()),
                ("source", Some(error.source.to_string())),
            ]),
            json!({"error": error.message, "source": error.source}),
            "restore_remote_inventory",
            format!(
                "ait workflow reconcile{} --dry-run",
                input
                    .remote_name
                    .as_ref()
                    .map(|name| format!(" --remote {name}"))
                    .unwrap_or_default()
            ),
            "The selected remote inventory could not be read completely; no remote-derived repair is safe until the read succeeds.",
        );
    }
    let workspace_lock_active = input
        .workspace_lock
        .get("active")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let workspace_lock_blocks_reconciliation = input
        .workspace_lock
        .get("blocks_reconciliation")
        .and_then(JsonValue::as_bool)
        .unwrap_or(workspace_lock_active);
    if workspace_lock_blocks_reconciliation {
        push_finding(
            &mut findings,
            "reconciliation.mutation_lock_active",
            "protected",
            "warning",
            identity_map([(
                "lock_path",
                string_field(&input.workspace_lock, "path"),
            )]),
            input.workspace_lock.clone(),
            "wait_for_active_mutation",
            "ait workflow reconcile --dry-run".to_string(),
            "Another workflow operation is changing the workspace; wait for it to finish before repairing anything.",
        );
    }

    for (task_id, task) in &tasks.rows {
        if !tasks.matches_filter(input.task_filter.as_deref(), Some(task_id)) {
            continue;
        }
        let local_task_id = task
            .local
            .as_ref()
            .and_then(|row| string_field(row, "task_id"));
        let remote_task_id = task
            .remote
            .as_ref()
            .and_then(|row| string_field(row, "task_id"));
        let authoritative_task_id = remote_task_id
            .clone()
            .or_else(|| local_task_id.clone())
            .unwrap_or_else(|| task_id.clone());
        let operational_task_id = local_task_id
            .clone()
            .or_else(|| remote_task_id.clone())
            .unwrap_or_else(|| task_id.clone());
        let task_status = task.status();
        let task_changes = changes_by_task.get(task_id).cloned().unwrap_or_default();
        let lifecycle_mapping_blocked = task.publication_mapping_blocked
            || task_changes
                .iter()
                .any(|(_, change)| change.publication_mapping_blocked);
        let has_open_change = task_changes
            .iter()
            .any(|(_, change)| change_status_is_open(change.status().as_deref()));
        let all_changes_landed = !task_changes.is_empty()
            && task_changes
                .iter()
                .all(|(_, change)| change.status().as_deref() == Some("landed"));
        if task_status_is_terminal(task_status.as_deref()) && has_open_change {
            let open_refs = task_changes
                .iter()
                .filter(|(_, change)| change_status_is_open(change.status().as_deref()))
                .map(|(change_ref, _)| JsonValue::String(change_ref.clone()))
                .collect::<Vec<_>>();
            push_finding(
                &mut findings,
                "task.terminal_with_open_change",
                "manual_resolution",
                "error",
                identity_map([
                    ("task_id", Some(authoritative_task_id.clone())),
                    ("local_task_id", local_task_id.clone()),
                    ("remote_task_id", remote_task_id.clone()),
                ]),
                json!({"task_status": task_status, "open_change_refs": open_refs}),
                "resolve_terminal_task_open_change",
                format!("ait task audit {authoritative_task_id}"),
                "A terminal Task still has an open Change. Reconciliation will not infer a Change outcome.",
            );
        }
        if task_status_is_active(task_status.as_deref())
            && all_changes_landed
            && remote_change_inventory_complete
            && !lifecycle_mapping_blocked
        {
            push_finding(
                &mut findings,
                "task.active_after_all_changes_landed",
                "safe_metadata_repair",
                "warning",
                identity_map([
                    ("task_id", Some(authoritative_task_id.clone())),
                    ("local_task_id", local_task_id.clone()),
                    ("remote_task_id", remote_task_id.clone()),
                ]),
                json!({
                    "task_status": task_status,
                    "authoritative_scope": if task.remote.is_some() { "remote" } else { "local" },
                    "landed_change_refs": task_changes.iter().map(|(change_ref, _)| JsonValue::String(change_ref.clone())).collect::<Vec<_>>(),
                }),
                "close_task_from_immutable_land_evidence",
                reconcile_command(input.remote_name.as_deref(), Some(&authoritative_task_id)),
                "Every linked Change is authoritatively landed while the Task remains active.",
            );
        }
        // Local and Remote Task lifecycle records are independent authorities. In
        // particular, a published Local Task is immutable source lineage rather
        // than a mutable projection whose status should mirror its Remote target.
        if task.authoritative().is_some_and(plan_drift_is_present) {
            let task_row = task.authoritative().unwrap_or(&JsonValue::Null);
            push_finding(
                &mut findings,
                "plan.checklist_drift",
                "manual_resolution",
                "warning",
                identity_map([
                    ("task_id", Some(authoritative_task_id.clone())),
                    ("local_task_id", local_task_id.clone()),
                    ("remote_task_id", remote_task_id.clone()),
                    ("plan_id", string_field(task_row, "plan_id")),
                    ("plan_item_ref", string_field(task_row, "plan_item_ref")),
                ]),
                json!({
                    "plan_drift_state": task_row.get("plan_drift_state").cloned().unwrap_or(JsonValue::Null),
                    "plan_closeout_policy": "separate_after_remote_land",
                }),
                "sync_bound_plan_separately",
                format!(
                    "ait plan sync <bound-sprint-card-path>{}",
                    input
                        .remote_name
                        .as_ref()
                        .map(|name| format!(" --remote {name}"))
                        .unwrap_or_else(|| " --local".to_string())
                ),
                "Plan checklist drift is read-only reconciliation evidence and requires a separate Plan sync.",
            );
        }
        if task_status_is_active(task_status.as_deref())
            && task_changes
                .iter()
                .any(|(_, change)| change_status_is_open(change.status().as_deref()))
            && !worktrees_by_task.contains_key(task_id)
        {
            let expected_line = format!("feature/{}", operational_task_id.to_ascii_lowercase());
            let line_present = local_lines
                .get(&expected_line)
                .or_else(|| remote_lines.get(&expected_line))
                .is_some();
            push_finding(
                &mut findings,
                "worktree.registration_missing",
                "manual_resolution",
                "warning",
                identity_map([
                    ("task_id", Some(operational_task_id.clone())),
                    ("local_task_id", local_task_id.clone()),
                    ("remote_task_id", remote_task_id.clone()),
                    ("line_name", line_present.then_some(expected_line)),
                ]),
                json!({
                    "task_status": task_status,
                    "open_change_refs": task_changes.iter().filter(|(_, change)| change_status_is_open(change.status().as_deref())).map(|(change_ref, _)| JsonValue::String(change_ref.clone())).collect::<Vec<_>>(),
                    "expected_feature_line_present": line_present,
                }),
                "recover_or_recreate_task_worktree",
                format!(
                    "ait worktree recover-task {operational_task_id} --dry-run"
                ),
                "An active Task with open work has no bound worktree registration; completion is not inferred from the missing registration.",
            );
        }
    }

    for worktree in &input.worktrees {
        let task_id = string_field(worktree, "bound_task_id");
        if !tasks.matches_filter(input.task_filter.as_deref(), task_id.as_deref()) {
            continue;
        }
        let paired_task = task_id.as_ref().and_then(|task_id| tasks.get(task_id));
        let local_task_id = paired_task
            .and_then(|task| task.local.as_ref())
            .and_then(|row| string_field(row, "task_id"));
        let remote_task_id = paired_task
            .and_then(|task| task.remote.as_ref())
            .and_then(|row| string_field(row, "task_id"));
        let task_status = task_id
            .as_ref()
            .and_then(|task_id| tasks.get(task_id))
            .and_then(JoinedRow::status);
        let bound_change_ref = string_field(worktree, "bound_change_ref").or_else(|| {
            Some(format!(
                "{}/{}",
                task_id.as_deref()?,
                string_field(worktree, "bound_change_id")?
            ))
        });
        let change_status = bound_change_ref
            .as_ref()
            .and_then(|change_ref| {
                joined_change_for_binding(&changes, change_ref, task_id.as_deref())
            })
            .and_then(JoinedRow::status);
        let owner_mapping_blocked = paired_task
            .is_some_and(|task| task.publication_mapping_blocked)
            || bound_change_ref
                .as_ref()
                .and_then(|change_ref| changes.get(change_ref))
                .is_some_and(|change| change.publication_mapping_blocked);
        let name = string_field(worktree, "name");
        let line_name = string_field(worktree, "registered_line_name")
            .or_else(|| string_field(worktree, "current_line"));
        let status =
            string_field(worktree, "workspace_status").unwrap_or_else(|| "unknown".to_string());
        let clean = worktree.get("clean").and_then(JsonValue::as_bool);
        let is_current = worktree
            .get("is_current")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let auto_created = worktree
            .get("auto_created_for_task")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
            || string_field(worktree, "creation_kind").as_deref() == Some("task_auto_created");
        let manual_only =
            string_field(worktree, "cleanup_policy").as_deref() == Some("manual_only");
        let operation_in_progress = worktree_operation_in_progress(worktree);
        let worktree_is_protected = manual_only
            || is_current
            || line_name.as_deref() == Some(input.default_line.as_str())
            || clean == Some(false)
            || operation_in_progress;
        let identities = || {
            identity_map([
                ("task_id", task_id.clone()),
                ("local_task_id", local_task_id.clone()),
                ("remote_task_id", remote_task_id.clone()),
                ("change_ref", bound_change_ref.clone()),
                ("worktree_name", name.clone()),
                ("line_name", line_name.clone()),
            ])
        };

        if status == "missing" {
            let owner_inventory_complete =
                remote_task_inventory_complete && remote_change_inventory_complete;
            let safe = owner_inventory_complete
                && !worktree_is_protected
                && !owner_mapping_blocked
                && (task_status_is_terminal(task_status.as_deref())
                    || (task_id.is_none() && bound_change_ref.is_none()));
            push_finding(
                &mut findings,
                "worktree.materialization_missing",
                if safe {
                    "safe_metadata_repair"
                } else {
                    "manual_resolution"
                },
                "warning",
                identities(),
                json!({
                    "workspace_status": status,
                    "task_status": task_status,
                    "change_status": change_status,
                    "path": string_field(worktree, "path"),
                }),
                if safe {
                    "prune_missing_worktree_registration"
                } else {
                    "recover_missing_task_worktree"
                },
                if safe {
                    reconcile_command(input.remote_name.as_deref(), task_id.as_deref())
                } else {
                    format!(
                        "ait worktree recover-task {} --dry-run",
                        task_id.clone().unwrap_or_else(|| "<task-id>".to_string())
                    )
                },
                "The registered worktree directory is missing. An active owner prevents automatic pruning.",
            );
        }
        if status == "detached" {
            let safe = remote_task_inventory_complete
                && !worktree_is_protected
                && !owner_mapping_blocked
                && task_status_is_terminal(task_status.as_deref())
                && clean != Some(false);
            push_finding(
                &mut findings,
                "worktree.overlay_detached",
                if safe {
                    "safe_metadata_repair"
                } else {
                    "manual_resolution"
                },
                "error",
                identities(),
                json!({
                    "workspace_status": status,
                    "task_status": task_status,
                    "inspection_error": worktree.get("reconciliation_inspection_error").cloned().unwrap_or(JsonValue::Null),
                }),
                if safe {
                    "prune_detached_terminal_registration"
                } else {
                    "repair_detached_worktree_overlay"
                },
                if safe {
                    reconcile_command(input.remote_name.as_deref(), task_id.as_deref())
                } else {
                    format!(
                        "ait worktree recreate {} --dry-run",
                        name.clone().unwrap_or_else(|| "<worktree>".to_string())
                    )
                },
                "The registered worktree cannot be opened as a valid repository overlay.",
            );
        }
        if task_status_is_terminal(task_status.as_deref()) && status == "clean" && auto_created {
            let cleanup_safe = remote_task_inventory_complete
                && remote_change_inventory_complete
                && !worktree_is_protected;
            let cleanup_safe = cleanup_safe && !owner_mapping_blocked;
            push_finding(
                &mut findings,
                "worktree.terminal_owner_clean",
                if is_current {
                    "protected"
                } else if cleanup_safe {
                    "safe_auto_cleanup"
                } else {
                    "manual_resolution"
                },
                "warning",
                identities(),
                json!({
                    "workspace_status": status,
                    "task_status": task_status,
                    "change_status": change_status,
                    "auto_created": true,
                    "is_current": is_current,
                }),
                if is_current {
                    "leave_current_worktree_untouched"
                } else if cleanup_safe {
                    "remove_clean_terminal_worktree"
                } else {
                    "restore_authoritative_inventory_before_cleanup"
                },
                if is_current {
                    "ait worktree path".to_string()
                } else if cleanup_safe {
                    reconcile_command(input.remote_name.as_deref(), task_id.as_deref())
                } else {
                    "ait workflow reconcile --dry-run".to_string()
                },
                "A clean auto-created worktree remains after its authoritative Task reached a terminal state.",
            );
        }
        if clean == Some(false) {
            push_finding(
                &mut findings,
                "worktree.dirty_protected",
                "protected",
                "warning",
                identities(),
                json!({
                    "workspace_status": status,
                    "changed_count": worktree.get("changed_count").cloned().unwrap_or(JsonValue::Null),
                    "changed_paths_sample": worktree.get("changed_paths").and_then(JsonValue::as_array).map(|rows| rows.iter().take(10).cloned().collect::<Vec<_>>()).unwrap_or_default(),
                }),
                "inspect_dirty_worktree",
                format!(
                    "ait worktree status {}",
                    name.clone().unwrap_or_else(|| "<worktree>".to_string())
                ),
                "Dirty or untracked content is protected from automatic reconciliation.",
            );
        }
        if manual_only || is_current || line_name.as_deref() == Some(input.default_line.as_str()) {
            push_finding(
                &mut findings,
                "worktree.protected_state",
                "protected",
                "info",
                identities(),
                json!({
                    "manual_only": manual_only,
                    "is_current": is_current,
                    "is_default_line": line_name.as_deref() == Some(input.default_line.as_str()),
                }),
                "leave_protected_worktree_untouched",
                "ait worktree cleanup-candidates --include-protected".to_string(),
                "Current, default-line, or manual-only worktrees require explicit operator action.",
            );
        }
        if operation_in_progress {
            push_finding(
                &mut findings,
                "worktree.operation_in_progress",
                "protected",
                "error",
                identities(),
                json!({
                    "merge_state": string_field(worktree, "merge_state"),
                    "rebase_state": string_field(worktree, "rebase_state"),
                }),
                "resume_or_abort_active_operation",
                format!(
                    "ait worktree show {}",
                    name.clone().unwrap_or_else(|| "<worktree>".to_string())
                ),
                "An unresolved merge or rebase operation blocks cleanup and metadata rewriting.",
            );
        }
        if let Some(change_ref) = bound_change_ref.as_ref() {
            match joined_change_for_binding(&changes, change_ref, task_id.as_deref())
                .and_then(JoinedRow::authoritative)
            {
                Some(change) => {
                    let change_task_id = string_field(change, "task_id");
                    let bound_owner = task_id.as_ref().map(|task_id| {
                        tasks
                            .canonical_id(task_id)
                            .unwrap_or_else(|| task_id.clone())
                    });
                    let change_owner = change_task_id.as_ref().map(|task_id| {
                        tasks
                            .canonical_id(task_id)
                            .unwrap_or_else(|| task_id.clone())
                    });
                    if bound_owner != change_owner {
                        push_finding(
                            &mut findings,
                            "binding.task_change_disagreement",
                            "manual_resolution",
                            "error",
                            identities(),
                            json!({
                                "worktree_task_id": task_id,
                                "change_task_id": change_task_id,
                            }),
                            "repair_task_change_binding",
                            format!(
                                "ait worktree show {} --json",
                                name.clone().unwrap_or_else(|| "<worktree>".to_string())
                            ),
                            "The worktree Task binding disagrees with the authoritative Change owner.",
                        );
                    }
                }
                None if remote_change_inventory_complete => {
                    push_finding(
                        &mut findings,
                        "binding.change_missing",
                        "manual_resolution",
                        "error",
                        identities(),
                        json!({"bound_change_ref": change_ref}),
                        "recover_missing_change_binding",
                        format!(
                            "ait worktree recover-task {} --dry-run",
                            task_id.clone().unwrap_or_else(|| "<task-id>".to_string())
                        ),
                        "The worktree references a Change absent from both local and selected-remote inventory.",
                    );
                }
                _ => {}
            }
        }
        let current_line = string_field(worktree, "current_line");
        let registered_line = string_field(worktree, "registered_line_name");
        if current_line.is_some() && registered_line.is_some() && current_line != registered_line {
            push_finding(
                &mut findings,
                "binding.line_disagreement",
                "manual_resolution",
                "error",
                identities(),
                json!({
                    "current_line": current_line,
                    "registered_line_name": registered_line,
                }),
                "repair_worktree_line_binding",
                format!(
                    "ait worktree recover-task {} --dry-run",
                    task_id.clone().unwrap_or_else(|| "<task-id>".to_string())
                ),
                "The worktree overlay and registry point at different line names.",
            );
        }
        if let Some(line_name) = line_name.as_ref() {
            let line = local_lines.get(line_name);
            let line_active =
                line.and_then(|row| string_field(row, "status")).as_deref() == Some("active");
            if !line_active {
                push_finding(
                    &mut findings,
                    "line.stale_name_reference",
                    "manual_resolution",
                    "error",
                    identities(),
                    json!({
                        "referenced_line_status": line.and_then(|row| string_field(row, "status")),
                        "stable_line_identity_available": line.and_then(|row| string_field(row, "line_id")).is_some(),
                    }),
                    "repair_reference_through_stable_line_identity",
                    format!(
                        "ait worktree recover-task {} --dry-run",
                        task_id.clone().unwrap_or_else(|| "<task-id>".to_string())
                    ),
                    "The worktree retains a missing, archived, or deleted line name reference.",
                );
            }
        }
    }

    for (line_name, line) in &local_lines {
        let Some(task_id) = task_id_from_feature_line(line_name) else {
            continue;
        };
        if !tasks.matches_filter(input.task_filter.as_deref(), Some(&task_id)) {
            continue;
        }
        if line_name == &input.current_line
            || line_name == &input.default_line
            || worktree_lines.contains(line_name)
            || string_field(line, "status").as_deref() != Some("active")
        {
            continue;
        }
        let Some(task) = tasks.get(&task_id) else {
            if !remote_task_inventory_complete
                || (task_id_has_remote_origin(&task_id) && input.remote_name.is_none())
            {
                continue;
            }
            push_finding(
                &mut findings,
                "line.owner_missing",
                "manual_resolution",
                "warning",
                identity_map([
                    ("task_id", Some(task_id)),
                    ("line_id", string_field(line, "line_id")),
                    ("line_name", Some(line_name.clone())),
                ]),
                json!({"line_status": string_field(line, "status")}),
                "identify_or_archive_orphan_feature_line",
                "ait line cleanup --include-protected".to_string(),
                "An active feature line has no Task in local or selected-remote inventory.",
            );
            continue;
        };
        let local_task_id = task
            .local
            .as_ref()
            .and_then(|row| string_field(row, "task_id"));
        let remote_task_id = task
            .remote
            .as_ref()
            .and_then(|row| string_field(row, "task_id"));
        let task_status = task.status();
        let canonical_task_id = tasks
            .canonical_id(&task_id)
            .unwrap_or_else(|| task_id.clone());
        let task_changes = changes_by_task
            .get(&canonical_task_id)
            .cloned()
            .unwrap_or_default();
        let every_change_terminal = task_changes
            .iter()
            .all(|(_, change)| change_status_is_terminal(change.status().as_deref()));
        if task_status_is_terminal(task_status.as_deref())
            && every_change_terminal
            && remote_change_inventory_complete
            && remote_line_inventory_complete
            && !task.publication_mapping_blocked
            && task_changes
                .iter()
                .all(|(_, change)| !change.publication_mapping_blocked)
        {
            push_finding(
                &mut findings,
                "line.terminal_owner_orphaned",
                "safe_auto_cleanup",
                "warning",
                identity_map([
                    ("task_id", Some(task_id.clone())),
                    ("local_task_id", local_task_id),
                    ("remote_task_id", remote_task_id),
                    ("line_id", string_field(line, "line_id")),
                    ("line_name", Some(line_name.clone())),
                ]),
                json!({
                    "task_status": task_status,
                    "line_status": string_field(line, "status"),
                    "remaining_worktree_count": 0,
                    "all_changes_terminal": every_change_terminal,
                }),
                "archive_terminal_owner_feature_line",
                reconcile_command(input.remote_name.as_deref(), Some(&task_id)),
                "The auto-created feature line has a terminal owner and no remaining worktree or open Change.",
            );
        }
    }

    for (change_ref, change) in &changes.rows {
        let Some(authoritative) = change.authoritative() else {
            continue;
        };
        let authoritative_task_id = string_field(authoritative, "task_id");
        if !tasks.matches_filter(
            input.task_filter.as_deref(),
            authoritative_task_id.as_deref(),
        ) {
            continue;
        }
        let local_task_id = change
            .local
            .as_ref()
            .and_then(|row| string_field(row, "task_id"));
        let remote_task_id = change
            .remote
            .as_ref()
            .and_then(|row| string_field(row, "task_id"));
        let local_change_ref = change.local.as_ref().and_then(change_reference);
        let remote_change_ref = change.remote.as_ref().and_then(change_reference);
        if let (Some(local), Some(remote)) = (change.local.as_ref(), change.remote.as_ref()) {
            let local_status = string_field(local, "status");
            let remote_status = string_field(remote, "status");
            let local_is_published =
                string_field(local, "publication_state").as_deref() == Some("published");
            let local_land_evidence = local_change_ref
                .as_deref()
                .map(|change_ref| local_land_closeout_evidence(&input, change_ref, local))
                .unwrap_or_default();
            // A Remote Land proves only the Remote lifecycle. It cannot prove that
            // Local Land closeout started, and published Local rows are immutable
            // publication lineage rather than incomplete Local Land records.
            if remote_status.as_deref() == Some("landed")
                && local_status.as_deref() != Some("landed")
                && !local_is_published
                && !local_land_evidence.is_empty()
                && !change.publication_mapping_blocked
            {
                push_finding(
                    &mut findings,
                    "land.local_closeout_interrupted",
                    "manual_resolution",
                    "error",
                    identity_map([
                        ("task_id", local_task_id.clone()),
                        ("local_task_id", local_task_id.clone()),
                        ("remote_task_id", remote_task_id.clone()),
                        ("change_ref", local_change_ref.clone()),
                        ("remote_change_ref", remote_change_ref.clone()),
                    ]),
                    json!({
                        "local_change_status": local_status,
                        "remote_change_status": remote_status,
                        "remote_landed_at": remote.get("landed_at").cloned().unwrap_or(JsonValue::Null),
                        "remote_target_line": string_field(remote, "target_line"),
                        "remote_landed_snapshot_id": landed_snapshot_id(remote),
                        "local_land_evidence": local_land_evidence,
                    }),
                    "audit_local_closeout_without_complete_land_payload",
                    format!(
                        "ait task audit {}",
                        local_task_id.as_deref().unwrap_or("<task-id>")
                    ),
                    "Independent Local Land evidence exists, but the local Change did not finish. The available records do not contain everything needed to rebuild the missing Local Land safely, so repair will not invent it.",
                );
            }
        }
        if let Some(remote) = change.remote.as_ref() {
            let remote_status = string_field(remote, "status");
            let target_line =
                string_field(remote, "target_line").or_else(|| string_field(remote, "base_line"));
            let landed_snapshot = landed_snapshot_id(remote);
            if remote_status.as_deref() == Some("landed")
                && remote_line_inventory_complete
                && !change.publication_mapping_blocked
                && target_line.is_some()
                && landed_snapshot.is_some()
            {
                let target_line = target_line.unwrap_or_default();
                let landed_snapshot = landed_snapshot.unwrap_or_default();
                let local_target_head = local_lines
                    .get(&target_line)
                    .and_then(|line| string_field(line, "head_snapshot_id"));
                let remote_target_head = remote_lines
                    .get(&target_line)
                    .and_then(|line| string_field(line, "head_snapshot_id"));
                if remote_target_head.as_deref() == Some(landed_snapshot.as_str())
                    && local_target_head.as_deref() != Some(landed_snapshot.as_str())
                {
                    let expected_previous_head = land_base_snapshot_id(remote);
                    let cas_precondition_holds = expected_previous_head.is_some()
                        && local_target_head == expected_previous_head;
                    push_finding(
                        &mut findings,
                        "land.target_sync_interrupted",
                        if cas_precondition_holds {
                            "safe_metadata_repair"
                        } else {
                            "manual_resolution"
                        },
                        "error",
                        identity_map([
                            ("task_id", remote_task_id.clone().or(authoritative_task_id.clone())),
                            ("local_task_id", local_task_id.clone()),
                            ("remote_task_id", remote_task_id.clone()),
                            ("change_ref", remote_change_ref.clone().or_else(|| Some(change_ref.clone()))),
                            ("local_change_ref", local_change_ref.clone()),
                            ("line_name", Some(target_line.clone())),
                            ("snapshot_id", Some(landed_snapshot.clone())),
                        ]),
                        json!({
                            "local_target_head_snapshot_id": local_target_head,
                            "remote_target_head_snapshot_id": remote_target_head,
                            "landed_snapshot_id": landed_snapshot,
                            "expected_previous_head_snapshot_id": expected_previous_head,
                            "cas_precondition_holds": cas_precondition_holds,
                        }),
                        if cas_precondition_holds {
                            "resume_target_line_sync_with_cas"
                        } else {
                            "resolve_diverged_target_line"
                        },
                        if cas_precondition_holds {
                            reconcile_command(
                                input.remote_name.as_deref(),
                                remote_task_id
                                    .as_deref()
                                    .or(authoritative_task_id.as_deref()),
                            )
                        } else {
                            format!(
                                "ait pull --line {target_line}{} --dry-run",
                                input
                                    .remote_name
                                    .as_ref()
                                    .map(|name| format!(" --remote {name}"))
                                    .unwrap_or_default()
                            )
                        },
                        "Remote land moved the authoritative target line, but local target-line synchronization did not finish. Automatic repair requires the recorded compare-and-swap base head.",
                    );
                }
            }
        }
    }

    findings.retain(|finding| {
        finding_matches_task_filter(finding, input.task_filter.as_deref(), &tasks)
            || string_field(finding, "code").as_deref() == Some("remote.inventory_unavailable")
    });
    findings.sort_by(|left, right| {
        (
            string_field(left, "code").unwrap_or_default(),
            string_field(left, "finding_id").unwrap_or_default(),
        )
            .cmp(&(
                string_field(right, "code").unwrap_or_default(),
                string_field(right, "finding_id").unwrap_or_default(),
            ))
    });
    let all_finding_count = findings.len();
    let disposition_counts = [
        "healthy",
        "safe_metadata_repair",
        "safe_auto_cleanup",
        "manual_resolution",
        "protected",
    ]
    .into_iter()
    .map(|disposition| {
        (
            disposition.to_string(),
            JsonValue::from(
                findings
                    .iter()
                    .filter(|finding| {
                        string_field(finding, "disposition").as_deref() == Some(disposition)
                    })
                    .count(),
            ),
        )
    })
    .collect::<JsonMap<String, JsonValue>>();
    if safe_only {
        findings.retain(|finding| {
            matches!(
                string_field(finding, "disposition").as_deref(),
                Some("safe_metadata_repair" | "safe_auto_cleanup")
            )
        });
    }
    let selected_finding_count = findings.len();
    let returned_findings = findings.into_iter().take(limit).collect::<Vec<_>>();
    let remaining_count = selected_finding_count.saturating_sub(returned_findings.len());
    let digest_material = returned_findings
        .iter()
        .filter_map(|finding| string_field(finding, "finding_id"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut digest = Sha256::new();
    digest.update(digest_material.as_bytes());
    let inventory_digest = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let task_selected_count = tasks
        .rows
        .keys()
        .filter(|task_id| tasks.matches_filter(input.task_filter.as_deref(), Some(task_id)))
        .count();
    let change_selected_count = changes
        .rows
        .values()
        .filter(|change| {
            tasks.matches_filter(
                input.task_filter.as_deref(),
                change
                    .authoritative()
                    .and_then(|row| string_field(row, "task_id"))
                    .as_deref(),
            )
        })
        .count();
    let worktree_selected_count = input
        .worktrees
        .iter()
        .filter(|row| {
            tasks.matches_filter(
                input.task_filter.as_deref(),
                string_field(row, "bound_task_id").as_deref(),
            )
        })
        .count();
    let remote_status = if input.remote_name.is_none() {
        "not_configured"
    } else if input.remote_errors.is_empty() {
        "available"
    } else {
        "partial"
    };
    Ok(json!({
        "contract": RECONCILIATION_CONTRACT,
        "operation": "inventory",
        "status": if remaining_count > 0 { "findings_truncated" } else { "completed" },
        "mode": "dry_run",
        "captured_at": input.captured_at,
        "repo_name": input.repo_name,
        "remote_name": input.remote_name,
        "task_filter": input.task_filter,
        "safe_only": safe_only,
        "limit": limit,
        "inventory_digest": inventory_digest,
        "sources": {
            "local": {
                "status": "available",
                "task_count": input.local_tasks.len(),
                "change_count": input.local_changes.len(),
                "line_count": input.local_lines.len(),
                "worktree_count": input.worktrees.len(),
                "worktree_status_mode": "verified_read_only",
            },
            "remote": {
                "status": remote_status,
                "task_count": input.remote_tasks.len(),
                "change_count": input.remote_changes.len(),
                "line_count": input.remote_lines.len(),
                "error_count": input.remote_errors.len(),
            },
            "plan": {
                "mode": "read_only_evidence",
                "mutation_owned_by_reconcile": false,
            },
            "mutation_receipts": {
                "mode": "read_only_evidence",
                "count": input.mutation_receipts.len(),
                "evidence": input.mutation_receipts.iter().take(limit).cloned().collect::<Vec<_>>(),
            },
            "workspace_lock": input.workspace_lock,
        },
        "inventory": {
            "joined_task_count": tasks.rows.len(),
            "joined_change_count": changes.rows.len(),
            "local_line_count": local_lines.len(),
            "remote_line_count": remote_lines.len(),
            "worktree_count": input.worktrees.len(),
            "selected_task_count": task_selected_count,
            "selected_change_count": change_selected_count,
            "selected_worktree_count": worktree_selected_count,
            "mutation_receipt_count": input.mutation_receipts.len(),
            "interrupted_operation_count": input.worktrees.iter().filter(|worktree| {
                worktree_operation_in_progress(worktree)
            }).count(),
        },
        "summary": {
            "total_finding_count": all_finding_count,
            "selected_finding_count": selected_finding_count,
            "returned_finding_count": returned_findings.len(),
            "remaining_count": remaining_count,
            "truncated": remaining_count > 0,
            "disposition_counts": disposition_counts,
            "healthy": disposition_counts.get("safe_metadata_repair").and_then(JsonValue::as_u64).unwrap_or(0) == 0
                && disposition_counts.get("safe_auto_cleanup").and_then(JsonValue::as_u64).unwrap_or(0) == 0
                && disposition_counts.get("manual_resolution").and_then(JsonValue::as_u64).unwrap_or(0) == 0
                && disposition_counts.get("protected").and_then(JsonValue::as_u64).unwrap_or(0) == 0,
        },
        "findings": returned_findings,
        "apply_available": true,
        "mutated": false,
        "receipts_created": 0,
        "next_command": reconcile_command(input.remote_name.as_deref(), input.task_filter.as_deref()),
    }))
}

pub fn workflow_reconcile_inventory(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    task_filter: Option<&str>,
    safe_only: bool,
    limit: Option<usize>,
) -> Result<JsonValue, String> {
    workflow_reconcile_inventory_with_remote_policy(
        repo,
        remote_name,
        task_filter,
        safe_only,
        limit,
        true,
    )
}

pub(super) fn workflow_reconcile_inventory_with_remote_policy(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    task_filter: Option<&str>,
    safe_only: bool,
    limit: Option<usize>,
    use_default_remote: bool,
) -> Result<JsonValue, String> {
    let input = read_reconciliation_inventory(repo, remote_name, task_filter, use_default_remote)?;
    build_reconciliation_inventory(
        input,
        safe_only,
        limit.unwrap_or(DEFAULT_RECONCILIATION_LIMIT),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_input() -> ReconciliationInventoryInput {
        ReconciliationInventoryInput {
            repo_name: "fixture".to_string(),
            captured_at: "2026-07-19T00:00:00Z".to_string(),
            remote_name: Some("origin".to_string()),
            task_filter: None,
            current_line: "main".to_string(),
            default_line: "main".to_string(),
            local_tasks: vec![json!({"task_id": "RCT-1", "status": "active"})],
            remote_tasks: vec![
                json!({"task_id": "RCT-1", "status": "active"}),
                json!({"task_id": "RCT-2", "status": "completed"}),
                json!({"task_id": "RCT-3", "status": "completed", "plan_drift_state": "checklist_open", "plan_id": "PR-3", "plan_item_ref": "card/three"}),
                json!({"task_id": "RCT-4", "status": "active"}),
                json!({"task_id": "RCT-5", "status": "active"}),
            ],
            local_changes: vec![json!({
                "task_id": "RCT-1",
                "change_id": "C-01",
                "change_ref": "RCT-1/C-01",
                "status": "draft"
            })],
            remote_changes: vec![
                json!({"task_id": "RCT-1", "change_id": "C-01", "change_ref": "RCT-1/C-01", "status": "landed", "landed_at": "2026-07-19T00:00:00Z"}),
                json!({"task_id": "RCT-2", "change_id": "C-01", "change_ref": "RCT-2/C-01", "status": "draft"}),
                json!({"task_id": "RCT-3", "change_id": "C-01", "change_ref": "RCT-3/C-01", "status": "landed"}),
                json!({"task_id": "RCT-4", "change_id": "C-01", "change_ref": "RCT-4/C-01", "status": "draft"}),
                json!({"task_id": "RCT-5", "change_id": "C-01", "change_ref": "RCT-5/C-01", "status": "draft"}),
            ],
            local_lines: vec![
                json!({"line_id": "LNE-1", "line_name": "main", "status": "active"}),
                json!({"line_id": "LNE-2", "line_name": "feature/rct-3", "status": "active"}),
                json!({"line_id": "LNE-4", "line_name": "feature/rct-4", "status": "archived"}),
            ],
            remote_lines: vec![json!({"line_name": "main", "status": "active"})],
            worktrees: vec![
                json!({
                    "name": "rct-3",
                    "path": "/tmp/rct-3",
                    "bound_task_id": "RCT-3",
                    "bound_change_id": "C-01",
                    "bound_change_ref": "RCT-3/C-01",
                    "registered_line_name": "feature/rct-3",
                    "current_line": "feature/rct-3",
                    "workspace_status": "clean",
                    "clean": true,
                    "auto_created_for_task": true,
                    "creation_kind": "task_auto_created",
                    "cleanup_policy": "after_remote_land",
                    "is_current": false,
                    "merge_state": "idle",
                    "rebase_state": "idle"
                }),
                json!({
                    "name": "rct-4",
                    "path": "/tmp/rct-4",
                    "bound_task_id": "RCT-4",
                    "bound_change_id": "C-01",
                    "bound_change_ref": "RCT-4/C-01",
                    "registered_line_name": "feature/rct-4",
                    "current_line": "feature/wrong",
                    "workspace_status": "dirty",
                    "clean": false,
                    "changed_count": 1,
                    "changed_paths": ["dirty.txt"],
                    "creation_kind": "manual_add",
                    "cleanup_policy": "manual_only",
                    "is_current": false,
                    "merge_state": "conflicted",
                    "rebase_state": "idle"
                }),
                json!({
                    "name": "missing",
                    "path": "/tmp/missing",
                    "workspace_status": "missing",
                    "exists": false,
                    "creation_kind": "manual_add",
                    "cleanup_policy": "manual_only",
                    "is_current": false,
                    "merge_state": "idle",
                    "rebase_state": "idle"
                }),
            ],
            mutation_receipts: vec![json!({
                "scope": "local",
                "object_kind": "change",
                "object_id": "RCT-1/C-01",
                "receipt_kind": "land_submission",
                "receipt": {"submission_id": "LAND-1", "status": "succeeded"},
            })],
            workspace_lock: json!({
                "path": "/tmp/reconcile.lock",
                "state": "idle",
                "active": false,
                "metadata": JsonValue::Null,
            }),
            remote_errors: vec![RemoteReadError {
                source: "lines",
                message: "fixture remote line read failed".to_string(),
            }],
        }
    }

    fn finding_codes(payload: &JsonValue) -> BTreeSet<String> {
        payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|finding| string_field(finding, "code"))
            .collect()
    }

    fn published_pair_input() -> ReconciliationInventoryInput {
        ReconciliationInventoryInput {
            repo_name: "fixture".to_string(),
            captured_at: "2026-08-14T00:00:00Z".to_string(),
            remote_name: Some("origin".to_string()),
            task_filter: None,
            current_line: "main".to_string(),
            default_line: "main".to_string(),
            local_tasks: vec![json!({
                "task_id": "LCT-100",
                "status": "active",
                "publication_state": "published",
                "published_remote_name": "origin",
                "published_task_id": "RCT-900"
            })],
            remote_tasks: vec![json!({
                "task_id": "RCT-900",
                "status": "completed"
            })],
            local_changes: vec![json!({
                "task_id": "LCT-100",
                "change_id": "C-01",
                "change_ref": "LCT-100/C-01",
                "status": "draft",
                "publication_state": "published",
                "published_remote_name": "origin",
                "published_change_id": "RCT-900/C-01"
            })],
            remote_changes: vec![json!({
                "task_id": "RCT-900",
                "change_id": "C-01",
                "change_ref": "RCT-900/C-01",
                "status": "landed",
                "landed_at": "2026-08-14T00:00:00Z"
            })],
            local_lines: vec![
                json!({"line_id": "LNE-MAIN", "line_name": "main", "status": "active"}),
                json!({"line_id": "LNE-LOCAL", "line_name": "feature/lct-100", "status": "active"}),
            ],
            remote_lines: vec![
                json!({"line_id": "LNE-REMOTE-MAIN", "line_name": "main", "status": "active"}),
            ],
            worktrees: Vec::new(),
            mutation_receipts: Vec::new(),
            workspace_lock: json!({
                "path": "/tmp/reconcile.lock",
                "state": "idle",
                "active": false,
                "metadata": JsonValue::Null,
            }),
            remote_errors: Vec::new(),
        }
    }

    #[test]
    fn inventory_classifies_cross_object_findings_without_mutation() {
        let payload = build_reconciliation_inventory(fixture_input(), false, 100).unwrap();
        let codes = finding_codes(&payload);
        for expected in [
            "binding.line_disagreement",
            "land.local_closeout_interrupted",
            "line.stale_name_reference",
            "plan.checklist_drift",
            "remote.inventory_unavailable",
            "task.active_after_all_changes_landed",
            "task.terminal_with_open_change",
            "worktree.dirty_protected",
            "worktree.materialization_missing",
            "worktree.operation_in_progress",
            "worktree.protected_state",
            "worktree.registration_missing",
            "worktree.terminal_owner_clean",
        ] {
            assert!(codes.contains(expected), "missing {expected}: {codes:?}");
        }
        assert_eq!(payload["contract"], json!(RECONCILIATION_CONTRACT));
        assert_eq!(payload["mode"], json!("dry_run"));
        assert_eq!(payload["mutated"], json!(false));
        assert_eq!(payload["apply_available"], json!(true));
        assert_eq!(payload["receipts_created"], json!(0));
        assert_eq!(payload["inventory"]["mutation_receipt_count"], json!(1));
        assert_eq!(
            payload["sources"]["plan"]["mutation_owned_by_reconcile"],
            json!(false)
        );
    }

    #[test]
    fn published_local_and_remote_lifecycles_remain_independent() {
        let payload = build_reconciliation_inventory(published_pair_input(), false, 100).unwrap();
        let codes = finding_codes(&payload);
        assert!(codes.contains("line.terminal_owner_orphaned"));
        assert!(!codes.contains("land.local_closeout_interrupted"));
        assert!(!codes.contains("task.local_status_stale"));
        assert!(!codes.contains("worktree.registration_missing"));
        assert_eq!(payload["inventory"]["joined_task_count"], json!(1));
        assert_eq!(payload["inventory"]["joined_change_count"], json!(1));
    }

    #[test]
    fn remote_land_receipt_does_not_prove_local_closeout_interruption() {
        let mut input = fixture_input();
        input.mutation_receipts[0]["scope"] = json!("remote");
        let payload = build_reconciliation_inventory(input, false, 100).unwrap();
        assert!(!finding_codes(&payload).contains("land.local_closeout_interrupted"));
    }

    #[test]
    fn terminal_remote_task_never_authorizes_local_task_status_repair() {
        let mut input = fixture_input();
        input.remote_tasks[0]["status"] = json!("completed");
        let payload = build_reconciliation_inventory(input, false, 100).unwrap();
        assert!(!finding_codes(&payload).contains("task.local_status_stale"));
        assert!(payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| {
                string_field(&finding["recommended_action"], "code").as_deref()
                    != Some("refresh_local_task_status")
            }));
    }

    #[test]
    fn explicit_local_land_receipt_reports_interrupted_closeout() {
        let payload = build_reconciliation_inventory(fixture_input(), false, 100).unwrap();
        let finding = payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| {
                string_field(finding, "code").as_deref() == Some("land.local_closeout_interrupted")
            })
            .unwrap();
        assert_eq!(finding["disposition"], json!("manual_resolution"));
        assert_eq!(finding["recommended_action"]["automatic"], json!(false));
        assert_eq!(
            finding["evidence"]["local_land_evidence"][0]["scope"],
            json!("local")
        );
    }

    #[test]
    fn explicit_local_land_payload_reports_interrupted_closeout() {
        let mut input = fixture_input();
        input.mutation_receipts.clear();
        input.local_changes[0]["landed_snapshot_id"] = json!("SNP-LOCAL-LAND");
        let payload = build_reconciliation_inventory(input, false, 100).unwrap();
        let finding = payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| {
                string_field(finding, "code").as_deref() == Some("land.local_closeout_interrupted")
            })
            .unwrap();
        assert_eq!(
            finding["evidence"]["local_land_evidence"][0]["receipt_kind"],
            json!("local_land_payload")
        );
    }

    #[test]
    fn either_publication_identity_filters_the_same_lifecycle() {
        let mut local_filter = published_pair_input();
        local_filter.task_filter = Some("LCT-100".to_string());
        let local = build_reconciliation_inventory(local_filter, false, 100).unwrap();

        let mut remote_filter = published_pair_input();
        remote_filter.task_filter = Some("RCT-900".to_string());
        let remote = build_reconciliation_inventory(remote_filter, false, 100).unwrap();

        let ids = |payload: &JsonValue| {
            payload["findings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|finding| string_field(finding, "finding_id").unwrap())
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(ids(&local), ids(&remote));
        assert_eq!(local["inventory"]["selected_task_count"], json!(1));
        assert_eq!(remote["inventory"]["selected_task_count"], json!(1));
    }

    #[test]
    fn active_remote_publication_is_not_duplicated() {
        let mut input = published_pair_input();
        input.remote_tasks[0]["status"] = json!("active");
        input.remote_changes[0]["status"] = json!("draft");
        input.remote_changes[0]
            .as_object_mut()
            .unwrap()
            .remove("landed_at");
        let payload = build_reconciliation_inventory(input, false, 100).unwrap();
        let missing = payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| {
                string_field(finding, "code").as_deref() == Some("worktree.registration_missing")
            })
            .collect::<Vec<_>>();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0]["identities"]["task_id"], json!("LCT-100"));
        assert_eq!(missing[0]["identities"]["remote_task_id"], json!("RCT-900"));
    }

    #[test]
    fn invalid_or_unavailable_publication_mapping_fails_closed() {
        let mut missing = published_pair_input();
        missing.remote_tasks.clear();
        let payload = build_reconciliation_inventory(missing, false, 100).unwrap();
        let missing_task = payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| {
                string_field(finding, "code").as_deref() == Some("publication.task_target_missing")
            })
            .unwrap();
        assert_eq!(missing_task["disposition"], json!("protected"));

        let mut missing_change = published_pair_input();
        missing_change.remote_changes.clear();
        let payload = build_reconciliation_inventory(missing_change, false, 100).unwrap();
        let missing_change = payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| {
                string_field(finding, "code").as_deref()
                    == Some("publication.change_target_missing")
            })
            .unwrap();
        assert_eq!(missing_change["disposition"], json!("protected"));
        assert_eq!(payload["summary"]["healthy"], json!(false));

        let mut duplicate = published_pair_input();
        duplicate.local_tasks.push(json!({
            "task_id": "LCT-101",
            "status": "active",
            "publication_state": "published",
            "published_remote_name": "origin",
            "published_task_id": "RCT-900"
        }));
        let error = build_reconciliation_inventory(duplicate, false, 100).unwrap_err();
        assert!(error.contains("Multiple Local Tasks resolve to Remote Task RCT-900"));

        let mut alias_collision = published_pair_input();
        alias_collision.remote_tasks = vec![json!({
            "task_id": "LCT-100",
            "status": "completed"
        })];
        let error = build_reconciliation_inventory(alias_collision, false, 100).unwrap_err();
        assert!(error
            .contains("Reconciliation Task identity LCT-100 resolves to both RCT-900 and LCT-100"));

        let mut partial = published_pair_input();
        partial.remote_errors = vec![RemoteReadError {
            source: "tasks",
            message: "partial task inventory".to_string(),
        }];
        let payload = build_reconciliation_inventory(partial, false, 100).unwrap();
        assert!(payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| {
                !matches!(
                    string_field(finding, "disposition").as_deref(),
                    Some("safe_metadata_repair" | "safe_auto_cleanup")
                )
            }));

        let mut unavailable = published_pair_input();
        unavailable.remote_tasks.clear();
        unavailable.remote_changes.clear();
        unavailable.remote_errors = vec![
            RemoteReadError {
                source: "tasks",
                message: "task inventory unavailable".to_string(),
            },
            RemoteReadError {
                source: "changes",
                message: "change inventory unavailable".to_string(),
            },
        ];
        let payload = build_reconciliation_inventory(unavailable, false, 100).unwrap();
        assert!(payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| {
                !matches!(
                    string_field(finding, "disposition").as_deref(),
                    Some("safe_metadata_repair" | "safe_auto_cleanup")
                )
            }));
    }

    #[test]
    fn finding_ids_are_stable_and_safe_only_limit_is_bounded() {
        let first = build_reconciliation_inventory(fixture_input(), false, 100).unwrap();
        let second = build_reconciliation_inventory(fixture_input(), false, 100).unwrap();
        let first_ids = first["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| string_field(finding, "finding_id").unwrap())
            .collect::<Vec<_>>();
        let second_ids = second["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| string_field(finding, "finding_id").unwrap())
            .collect::<Vec<_>>();
        assert_eq!(first_ids, second_ids);

        let safe = build_reconciliation_inventory(fixture_input(), true, 1).unwrap();
        assert_eq!(safe["safe_only"], json!(true));
        assert_eq!(safe["summary"]["returned_finding_count"], json!(1));
        assert_eq!(safe["summary"]["truncated"], json!(true));
        assert!(safe["findings"].as_array().unwrap().iter().all(|finding| {
            matches!(
                string_field(finding, "disposition").as_deref(),
                Some("safe_metadata_repair" | "safe_auto_cleanup")
            )
        }));
        assert!(build_reconciliation_inventory(fixture_input(), false, 0).is_err());
        assert!(build_reconciliation_inventory(
            fixture_input(),
            false,
            MAX_RECONCILIATION_LIMIT + 1
        )
        .is_err());
    }

    #[test]
    fn task_filter_keeps_only_exact_task_findings_plus_remote_availability() {
        let mut input = fixture_input();
        input.task_filter = Some("RCT-3".to_string());
        let payload = build_reconciliation_inventory(input, false, 100).unwrap();
        assert!(payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| {
                string_field(finding, "code").as_deref() == Some("remote.inventory_unavailable")
                    || string_field(&finding["identities"], "task_id").as_deref() == Some("RCT-3")
            }));
    }

    #[test]
    fn target_sync_requires_recorded_compare_and_swap_head() {
        let mut safe_input = fixture_input();
        safe_input.remote_errors.clear();
        safe_input.local_lines[0]["head_snapshot_id"] = json!("SNP-BASE");
        safe_input.remote_lines[0]["head_snapshot_id"] = json!("SNP-LANDED");
        safe_input.remote_changes[0]["base_line"] = json!("main");
        safe_input.remote_changes[0]["base_snapshot_id"] = json!("SNP-BASE");
        safe_input.remote_changes[0]["landed_snapshot_id"] = json!("SNP-LANDED");
        let safe = build_reconciliation_inventory(safe_input, false, 100).unwrap();
        let finding = safe["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| {
                string_field(finding, "code").as_deref() == Some("land.target_sync_interrupted")
            })
            .expect("target sync finding");
        assert_eq!(finding["disposition"], json!("safe_metadata_repair"));
        assert_eq!(finding["evidence"]["cas_precondition_holds"], json!(true));

        let mut diverged_input = fixture_input();
        diverged_input.remote_errors.clear();
        diverged_input.local_lines[0]["head_snapshot_id"] = json!("SNP-DIVERGED");
        diverged_input.remote_lines[0]["head_snapshot_id"] = json!("SNP-LANDED");
        diverged_input.remote_changes[0]["base_line"] = json!("main");
        diverged_input.remote_changes[0]["base_snapshot_id"] = json!("SNP-BASE");
        diverged_input.remote_changes[0]["landed_snapshot_id"] = json!("SNP-LANDED");
        let diverged = build_reconciliation_inventory(diverged_input, false, 100).unwrap();
        let finding = diverged["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|finding| {
                string_field(finding, "code").as_deref() == Some("land.target_sync_interrupted")
            })
            .expect("diverged target sync finding");
        assert_eq!(finding["disposition"], json!("manual_resolution"));
        assert_eq!(finding["evidence"]["cas_precondition_holds"], json!(false));
    }

    #[test]
    fn feature_line_owner_recognizes_empty_one_and_two_byte_task_namespaces() {
        assert_eq!(
            task_id_from_feature_line("feature/rct-42/extra").as_deref(),
            Some("RCT-42")
        );
        assert_eq!(
            task_id_from_feature_line("feature/lct-100").as_deref(),
            Some("LCT-100")
        );
        assert_eq!(
            task_id_from_feature_line("feature/rt-7").as_deref(),
            Some("RT-7")
        );
        assert_eq!(
            task_id_from_feature_line("feature/lt-3").as_deref(),
            Some("LT-3")
        );
        assert_eq!(
            task_id_from_feature_line("feature/lwtt-0006").as_deref(),
            Some("LWTT-0006")
        );
        assert_eq!(
            task_id_from_feature_line("feature/rwtt-9").as_deref(),
            Some("RWTT-9")
        );
        assert_eq!(task_id_from_feature_line("feature/manual"), None);
        assert_eq!(task_id_from_feature_line("feature/laitt-1"), None);
        assert_eq!(task_id_from_feature_line("feature/wtt-1"), None);
        assert_eq!(task_id_from_feature_line("feature/lwtt-not-a-number"), None);
        assert_eq!(task_id_from_feature_line("feature/lwtt-1-extra"), None);
        assert!(change_status_is_terminal(Some("archived")));
        assert!(!change_status_is_open(Some("archived")));
    }

    #[test]
    fn remote_origin_line_missing_owner_requires_selected_complete_remote_tasks() {
        let mut input = fixture_input();
        input.task_filter = None;
        input.current_line = "main".to_string();
        input.default_line = "main".to_string();
        input.local_tasks.clear();
        input.remote_tasks.clear();
        input.local_changes.clear();
        input.remote_changes.clear();
        input.local_lines = vec![
            json!({"line_id":"LNE-1","line_name":"main","status":"active"}),
            json!({"line_id":"LNE-2","line_name":"feature/lwtt-0001","status":"active"}),
            json!({"line_id":"LNE-3","line_name":"feature/rwtt-0001","status":"active"}),
        ];
        input.remote_lines.clear();
        input.worktrees.clear();
        input.mutation_receipts.clear();
        input.remote_errors.clear();

        let missing_owner_ids = |result: &JsonValue| {
            result["findings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|finding| finding["code"] == json!("line.owner_missing"))
                .filter_map(|finding| finding["identities"]["task_id"].as_str())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        };

        input.remote_name = None;
        let local_only = build_reconciliation_inventory(input.clone(), false, 100).unwrap();
        assert_eq!(
            missing_owner_ids(&local_only),
            BTreeSet::from(["LWTT-0001".to_string()])
        );

        input.remote_name = Some("origin".to_string());
        let selected_remote = build_reconciliation_inventory(input.clone(), false, 100).unwrap();
        assert_eq!(
            missing_owner_ids(&selected_remote),
            BTreeSet::from(["LWTT-0001".to_string(), "RWTT-0001".to_string()])
        );

        input.remote_errors.push(RemoteReadError {
            source: "tasks",
            message: "remote Task read failed".to_string(),
        });
        let failed_remote = build_reconciliation_inventory(input, false, 100).unwrap();
        assert!(missing_owner_ids(&failed_remote).is_empty());
    }
}
