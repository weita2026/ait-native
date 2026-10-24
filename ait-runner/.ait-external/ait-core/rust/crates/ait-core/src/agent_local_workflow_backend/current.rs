use std::collections::BTreeMap;

use crate::binary_db::{
    AuthorityId, LocalBinaryDbFs, LocalStateScope, StorePath, REPOSITORY_BINARY_DB_BIN_PATHS,
    REPOSITORY_BINARY_DB_INDEX_PATHS,
};
use crate::binary_db_generation::admit_activated_binary_db_generation_for_runtime;
use crate::json_support::{json, JsonMap, JsonValue};
use crate::task_workflow_store_traits::{TaskWorkflowChangeLister, TaskWorkflowTaskLister};
use crate::workflow_binary_db::{BinaryDbWorkflowStore, BINARY_DB_WORKFLOW_LAYOUT_ID};

pub const AGENT_LOCAL_CURRENT_WORKFLOW_CONTRACT: &str = "ait.agent.local_current_workflow.v1";
pub const AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION: &str = "read_current_workflow";

pub fn agent_local_current_workflow_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "local current-workflow request must be an object".to_string())?;
    let operation = required_text(request.get("operation"), "operation")?;
    if operation != AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION {
        return Err(format!(
            "Unsupported local current-workflow operation `{operation}`."
        ));
    }
    let target = request
        .get("target")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "local current-workflow request field `target` is required".to_string())?;
    let (repo_root, repo_name) = validate_target(target)?;
    validate_arguments(request.get("arguments"))?;

    let authority = repo_root.join(".ait").join("binary-db");
    let (generation, guard) =
        admit_activated_binary_db_generation_for_runtime(&repo_root, &authority, &repo_name)?;
    let db = LocalBinaryDbFs::new(
        StorePath::from(generation.authority_root),
        StorePath::from(generation.generation_root),
        AuthorityId::new(format!("local:{repo_name}")),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS)
    .with_generation_guard(Some(guard));
    let store = BinaryDbWorkflowStore::<_, BINARY_DB_WORKFLOW_LAYOUT_ID>::new(db, &repo_name);
    let tasks = store.list_tasks().map_err(|error| error.to_string())?;
    let changes = store.list_changes().map_err(|error| error.to_string())?;
    let payload = project_current_workflow(&repo_name, &tasks, &changes)?;

    Ok(json!({
        "contract": AGENT_LOCAL_CURRENT_WORKFLOW_CONTRACT,
        "ok": true,
        "operation": AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION,
        "retryable": false,
        "payload": payload,
    }))
}

fn project_current_workflow(
    repo_name: &str,
    tasks: &[JsonValue],
    changes: &[JsonValue],
) -> Result<JsonValue, String> {
    let mut latest_change_updates = BTreeMap::<String, String>::new();
    for change in changes.iter().filter_map(JsonValue::as_object) {
        if text(change, "repo_name").as_deref() != Some(repo_name) {
            continue;
        }
        let Some(task_id) = text(change, "task_id") else {
            continue;
        };
        let updated_at = text(change, "updated_at").unwrap_or_default();
        latest_change_updates
            .entry(task_id)
            .and_modify(|current| {
                if updated_at > *current {
                    current.clone_from(&updated_at);
                }
            })
            .or_insert(updated_at);
    }
    let current_task = tasks
        .iter()
        .filter_map(JsonValue::as_object)
        .filter(|task| text(task, "repo_name").as_deref() == Some(repo_name))
        .filter(|task| text(task, "status").as_deref() == Some("active"))
        .max_by(|left, right| {
            task_order(left, &latest_change_updates).cmp(&task_order(right, &latest_change_updates))
        });

    let Some(task) = current_task else {
        return Ok(json!({
            "notification_source": "local_current",
            "items": [],
            "summary": {"current": 0},
        }));
    };
    let task_id = text(task, "task_id")
        .ok_or_else(|| "Local current workflow Task is missing `task_id`.".to_string())?;

    let linked_changes = changes
        .iter()
        .filter_map(JsonValue::as_object)
        .filter(|change| text(change, "repo_name").as_deref() == Some(repo_name))
        .filter(|change| text(change, "task_id").as_deref() == Some(task_id.as_str()))
        .collect::<Vec<_>>();
    let focus_change = linked_changes
        .iter()
        .copied()
        .filter(|change| !terminal_change_status(text(change, "status").as_deref()))
        .max_by(|left, right| row_order(left, "change_id").cmp(&row_order(right, "change_id")));
    let has_landed_change = linked_changes
        .iter()
        .any(|change| text(change, "status").as_deref() == Some("landed"));
    let (state, reason, action_code, action_label, action_detail) = match focus_change
        .and_then(|change| text(change, "status"))
        .as_deref()
    {
        Some("review") => (
            "in_review",
            "The current local Change is in review.",
            "inspect_review",
            "Inspect review",
            "Inspect the current local Change review state.",
        ),
        Some(_) => (
            "in_progress",
            "The current local Change is still in progress.",
            "continue_change",
            "Continue change",
            "Continue the current local Change.",
        ),
        None if has_landed_change => (
            "ready_to_complete",
            "All linked local Changes are terminal and at least one is applied.",
            "complete_task",
            "Complete task",
            "Complete the current local Task after its applied Change is verified.",
        ),
        None => (
            "planning",
            "The current local Task has no active Change.",
            "create_change",
            "Create change",
            "Create the next local Change for this Task.",
        ),
    };
    let focus_change_value = focus_change
        .cloned()
        .map(JsonValue::Object)
        .unwrap_or(JsonValue::Null);
    let updated_at = linked_changes
        .iter()
        .filter_map(|change| text(change, "updated_at"))
        .chain(text(task, "updated_at"))
        .max()
        .unwrap_or_default();

    Ok(json!({
        "notification_source": "local_current",
        "items": [{
            "task": JsonValue::Object(task.clone()),
            "focus_change": focus_change_value,
            "workflow": {
                "state": state,
                "reason": reason,
            },
            "next_action": {
                "code": action_code,
                "label": action_label,
                "detail": action_detail,
                "target_ref": focus_change
                    .and_then(|change| text(change, "change_id"))
                    .unwrap_or_else(|| task_id.clone()),
            },
            "updated_at": updated_at,
        }],
        "summary": {"current": 1},
    }))
}

fn validate_target(
    target: &JsonMap<String, JsonValue>,
) -> Result<(std::path::PathBuf, String), String> {
    if required_text(target.get("mode"), "target.mode")? != "local" {
        return Err("local current-workflow target mode must be `local`".to_string());
    }
    if required_text(target.get("workflow_mode"), "target.workflow_mode")? != "solo_local" {
        return Err("local current-workflow target workflow_mode must be `solo_local`".to_string());
    }
    let repo_root =
        std::path::PathBuf::from(required_text(target.get("repo_root"), "target.repo_root")?);
    let repo_name = required_text(target.get("repo_name"), "target.repo_name")?;
    if !repo_root.is_dir() || !repo_root.join(".ait").is_dir() {
        return Err(format!(
            "Local current-workflow repository root '{}' is invalid.",
            repo_root.display()
        ));
    }
    Ok((repo_root, repo_name))
}

fn validate_arguments(value: Option<&JsonValue>) -> Result<(), String> {
    match value {
        None | Some(JsonValue::Null) => Ok(()),
        Some(JsonValue::Object(arguments)) if arguments.is_empty() => Ok(()),
        _ => Err("local current-workflow arguments must be an empty object".to_string()),
    }
}

fn terminal_change_status(status: Option<&str>) -> bool {
    matches!(status, Some("landed" | "archived" | "superseded"))
}

fn row_order(row: &JsonMap<String, JsonValue>, id_field: &str) -> (String, String) {
    (
        text(row, "updated_at").unwrap_or_default(),
        text(row, id_field).unwrap_or_default(),
    )
}

fn task_order(
    task: &JsonMap<String, JsonValue>,
    latest_change_updates: &BTreeMap<String, String>,
) -> (String, String) {
    let task_id = text(task, "task_id").unwrap_or_default();
    let updated_at = latest_change_updates
        .get(&task_id)
        .cloned()
        .into_iter()
        .chain(text(task, "updated_at"))
        .max()
        .unwrap_or_default();
    (updated_at, task_id)
}

fn text(row: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    row.get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            format!("local current-workflow request field `{field}` must be a non-empty string")
        })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::change_store::ChangeStore;
    use crate::line_binary_db::BinaryDbLineStore;
    use crate::line_store::LineStore;
    use crate::task_store::TaskStore;

    use super::*;

    #[test]
    fn projection_selects_exactly_one_latest_active_task_and_focus_change() {
        let tasks = json!([
            {"task_id": "LT-1", "repo_name": "fixture", "title": "older", "status": "active", "updated_at": "2026-07-19T00:00:00Z"},
            {"task_id": "LT-2", "repo_name": "fixture", "title": "current", "status": "active", "updated_at": "2026-07-18T00:00:00Z"},
            {"task_id": "LT-3", "repo_name": "fixture", "title": "closed", "status": "completed", "updated_at": "2026-07-20T00:00:00Z"}
        ]);
        let changes = json!([
            {"change_id": "LC-1", "task_id": "LT-2", "repo_name": "fixture", "status": "draft", "updated_at": "2026-07-19T01:00:00Z"},
            {"change_id": "LC-2", "task_id": "LT-2", "repo_name": "fixture", "status": "review", "updated_at": "2026-07-19T02:00:00Z"},
            {"change_id": "LC-3", "task_id": "LT-2", "repo_name": "fixture", "status": "landed", "updated_at": "2026-07-19T03:00:00Z"}
        ]);

        let payload = project_current_workflow(
            "fixture",
            tasks.as_array().unwrap(),
            changes.as_array().unwrap(),
        )
        .expect("projection");

        assert_eq!(payload["notification_source"], "local_current");
        assert_eq!(payload["items"].as_array().unwrap().len(), 1);
        assert_eq!(payload["items"][0]["task"]["task_id"], "LT-2");
        assert_eq!(payload["items"][0]["focus_change"]["change_id"], "LC-2");
        assert_eq!(payload["items"][0]["workflow"]["state"], "in_review");
        assert_eq!(payload["items"][0]["next_action"]["code"], "inspect_review");
        assert_eq!(payload["items"][0]["updated_at"], "2026-07-19T03:00:00Z");
    }

    #[test]
    fn projection_distinguishes_planning_ready_to_complete_and_no_current_workflow() {
        let active = json!([{
            "task_id": "LT-1",
            "repo_name": "fixture",
            "title": "Current",
            "status": "active",
            "updated_at": "2026-07-19T00:00:00Z"
        }]);
        let planning =
            project_current_workflow("fixture", active.as_array().unwrap(), &[]).expect("planning");
        assert_eq!(planning["items"][0]["workflow"]["state"], "planning");
        assert_eq!(planning["items"][0]["next_action"]["code"], "create_change");

        let landed = json!([{
            "change_id": "LC-1",
            "task_id": "LT-1",
            "repo_name": "fixture",
            "status": "landed",
            "updated_at": "2026-07-19T01:00:00Z"
        }]);
        let ready = project_current_workflow(
            "fixture",
            active.as_array().unwrap(),
            landed.as_array().unwrap(),
        )
        .expect("ready");
        assert_eq!(ready["items"][0]["workflow"]["state"], "ready_to_complete");
        assert_eq!(ready["items"][0]["next_action"]["code"], "complete_task");

        let completed = json!([{
            "task_id": "LT-1",
            "repo_name": "fixture",
            "status": "completed",
            "updated_at": "2026-07-19T02:00:00Z"
        }]);
        let none = project_current_workflow("fixture", completed.as_array().unwrap(), &[])
            .expect("no current workflow");
        assert!(none["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn production_reader_uses_admitted_binary_db_without_building_a_queue() {
        let temp = tempdir().expect("tempdir");
        let ait = temp.path().join(".ait");
        let authority = ait.join("binary-db");
        fs::create_dir_all(&authority).expect("authority");
        fs::create_dir_all(ait.join("objects")).expect("objects");
        fs::write(
            ait.join("config.json"),
            json!({"repo_name": "fixture"}).to_string(),
        )
        .expect("config");
        let db = LocalBinaryDbFs::new(
            StorePath::from(authority),
            StorePath::from(temp.path()),
            AuthorityId::new("local:fixture"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
        BinaryDbLineStore::<_, BINARY_DB_WORKFLOW_LAYOUT_ID>::new(db.clone())
            .create_line("main", None, "2026-07-25T00:00:00Z")
            .expect("main line");
        let store = BinaryDbWorkflowStore::<_, BINARY_DB_WORKFLOW_LAYOUT_ID>::new(db, "fixture");
        let task = store
            .create_task("fixture", "Current task", "Ship it", None, None, None, None)
            .expect("task");
        store
            .create_change(
                task["task_id"].as_str().unwrap(),
                "fixture",
                "Current change",
                "main",
                None,
                None,
            )
            .expect("change");
        drop(store);

        let response = agent_local_current_workflow_execute_json(&json!({
            "operation": AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION,
            "target": {
                "mode": "local",
                "workflow_mode": "solo_local",
                "repo_root": temp.path(),
                "repo_name": "fixture",
            },
            "arguments": {},
        }))
        .expect("current workflow");

        assert_eq!(response["contract"], AGENT_LOCAL_CURRENT_WORKFLOW_CONTRACT);
        assert_eq!(response["ok"], true);
        assert_eq!(response["payload"]["notification_source"], "local_current");
        assert_eq!(response["payload"]["items"].as_array().unwrap().len(), 1);
        assert!(response["payload"].get("queue").is_none());
    }
}
