use super::*;

pub fn task_abandon(
    repo: &RepoRuntime,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name)? {
        let task_store = repo.task_store()?;
        let change_store = repo.change_store()?;
        return task_abandon_local_with_stores(&task_store, &change_store, task_id);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    task_abandon_remote_with_remotes(&mut task_remote, &mut closeout_remote, &repo_name, task_id)
}

fn task_abandon_preflight(task: &JsonValue, task_id: &str) -> Result<bool, String> {
    let returned_task_id = required_string_field(task, "task_id")?;
    if returned_task_id != task_id {
        return Err(format!(
            "Task abandon resolved task `{returned_task_id}`, not requested task `{task_id}`."
        ));
    }
    let status = required_string_field(task, "status")?;
    match status.as_str() {
        "active" | "open" | "planned" | "in_progress" => Ok(false),
        "abandoned" => Ok(true),
        "completed" | "canceled" | "cancelled" | "later_promotion_excluded" | "stopped" => {
            Err(format!(
                "Task {task_id} is already terminal with status `{status}`; only an already-abandoned Task can be repaired by rerunning task abandon."
            ))
        }
        _ => Err(format!(
            "Task {task_id} has unsupported status `{status}` for abandonment."
        )),
    }
}

fn task_abandon_change_is_terminal(status: &str) -> bool {
    matches!(
        status,
        "landed" | "closed" | "archived" | "canceled" | "cancelled" | "abandoned" | "superseded"
    )
}

fn task_abandon_change_target(change: &JsonValue, task_id: &str) -> Result<Option<String>, String> {
    let change_task_id = required_string_field(change, "task_id")?;
    if change_task_id != task_id {
        return Ok(None);
    }
    let status = required_string_field(change, "status")?;
    if task_abandon_change_is_terminal(&status) {
        return Ok(None);
    }
    if !matches!(
        status.as_str(),
        "draft" | "active" | "review" | "open" | "planned" | "in_progress"
    ) {
        return Err(format!(
            "Task {task_id} owns Change with unsupported status `{status}`; refusing partial abandonment."
        ));
    }
    let change_id = required_string_field(change, "change_id")?;
    let change_ref = string_field(change, "change_ref");
    if let Some(change_ref) = change_ref.as_deref() {
        let expected_prefix = format!("{task_id}/");
        if !change_ref.starts_with(&expected_prefix) {
            return Err(format!(
                "Task {task_id} owns Change `{change_id}` with mismatched change_ref `{change_ref}`."
            ));
        }
    }
    if change_id.starts_with("C-") {
        return change_ref.map(Some).ok_or_else(|| {
            format!(
                "Task {task_id} owns short Change `{change_id}` without the exact change_ref required for abandonment."
            )
        });
    }
    if let Some((prefix, _)) = change_id.rsplit_once('/') {
        if prefix != task_id {
            return Err(format!(
                "Task {task_id} owns mismatched composite Change id `{change_id}`."
            ));
        }
    }
    Ok(Some(change_id))
}

fn task_abandon_change_targets(
    changes: Vec<JsonValue>,
    task_id: &str,
) -> Result<Vec<String>, String> {
    let mut targets = BTreeSet::new();
    for change in changes {
        if let Some(target) = task_abandon_change_target(&change, task_id)? {
            targets.insert(target);
        }
    }
    Ok(targets.into_iter().collect())
}

fn task_abandon_local_with_stores<T, C>(
    task_store: &T,
    change_store: &C,
    task_id: &str,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowTaskReader + TaskWorkflowTaskCloser + ?Sized,
    C: TaskWorkflowChangeLister + TaskWorkflowChangeCloser + ?Sized,
{
    let task = task_local_read_with_task_store(task_store, task_id)?;
    let already_abandoned = task_abandon_preflight(&task, task_id)?;
    let changes = change_local_list_with_change_store(change_store)?;
    let targets = task_abandon_change_targets(changes, task_id)?;
    for change_ref in targets {
        change_local_close_with_change_store(change_store, &change_ref, "canceled")?;
    }
    if already_abandoned {
        task_local_read_with_task_store(task_store, task_id)
    } else {
        task_local_close_with_task_store(task_store, task_id, "abandoned")
    }
}

fn task_abandon_remote_with_remotes<C, T>(
    change_remote: &mut C,
    task_closer: &mut T,
    repo_name: &str,
    task_id: &str,
) -> Result<JsonValue, String>
where
    C: TaskWorkflowRemoteTaskReader
        + TaskWorkflowRemoteChangeLister
        + TaskWorkflowRemoteChangeCloser
        + ?Sized,
    T: TaskWorkflowRemoteTaskCloser + ?Sized,
{
    let task = task_remote_read_with_task_remote(change_remote, repo_name, task_id)?;
    let already_abandoned = task_abandon_preflight(&task, task_id)?;
    let changes = change_list_with_task_remote(change_remote, repo_name)?;
    let changes = changes
        .as_array()
        .cloned()
        .ok_or_else(|| "Remote Change list must be an array.".to_string())?;
    let targets = task_abandon_change_targets(changes, task_id)?;
    for change_ref in targets {
        change_remote
            .close_change(&change_ref, "canceled", Some(repo_name))
            .map_err(|err| err.to_string())?;
    }
    if already_abandoned {
        task_remote_read_with_task_remote(change_remote, repo_name, task_id)
    } else {
        task_close_with_closeout_remote(task_closer, task_id, "abandoned", repo_name)
    }
}

pub(in crate::primitives) fn task_close(
    repo: &RepoRuntime,
    task_id: &str,
    status: &str,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    if repo.task_uses_local_scope(local, remote_name)? {
        let store = repo.task_store()?;
        return task_local_close_with_task_store(&store, task_id, status);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    task_close_with_closeout_remote(&mut closeout_remote, task_id, status, &repo_name)
}

pub(in crate::primitives) fn task_close_with_closeout_remote<R>(
    closeout_remote: &mut R,
    task_id: &str,
    status: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskCloser + ?Sized,
{
    closeout_remote
        .close_task(task_id, status, Some(repo_name))
        .map_err(|err| err.to_string())
}

#[cfg(test)]
pub(in crate::primitives) fn task_complete_with_closeout_remote<R>(
    closeout_remote: &mut R,
    task_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskCloser + ?Sized,
{
    task_close_with_closeout_remote(closeout_remote, task_id, "completed", repo_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ait_core::plan_store::{PlanStoreError, PlanStoreResult};
    use ait_core::task_workflow_http_adapter::{
        TaskWorkflowHttpClientError, TaskWorkflowHttpClientResult,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    struct RecordingTaskStore {
        task: RefCell<JsonValue>,
        events: Rc<RefCell<Vec<String>>>,
    }

    impl TaskWorkflowTaskReader for RecordingTaskStore {
        fn get_task(&self, task_id: &str) -> PlanStoreResult<JsonValue> {
            let task = self.task.borrow();
            if string_field(&task, "task_id").as_deref() == Some(task_id) {
                Ok(task.clone())
            } else {
                Err(PlanStoreError::NotFound(format!("Unknown Task {task_id}")))
            }
        }
    }

    impl TaskWorkflowTaskCloser for RecordingTaskStore {
        fn close_task(&self, task_id: &str, status: &str) -> PlanStoreResult<JsonValue> {
            self.events
                .borrow_mut()
                .push(format!("task:{task_id}:{status}"));
            let mut task = self.task.borrow_mut();
            task["status"] = JsonValue::String(status.to_string());
            Ok(task.clone())
        }
    }

    struct RecordingChangeStore {
        changes: RefCell<Vec<JsonValue>>,
        events: Rc<RefCell<Vec<String>>>,
    }

    impl TaskWorkflowChangeLister for RecordingChangeStore {
        fn list_changes(&self) -> PlanStoreResult<Vec<JsonValue>> {
            Ok(self.changes.borrow().clone())
        }
    }

    impl TaskWorkflowChangeCloser for RecordingChangeStore {
        fn close_change(&self, change_ref: &str, status: &str) -> PlanStoreResult<JsonValue> {
            self.events
                .borrow_mut()
                .push(format!("change:{change_ref}:{status}"));
            let mut changes = self.changes.borrow_mut();
            let change = changes
                .iter_mut()
                .find(|change| {
                    string_field(change, "change_ref").as_deref() == Some(change_ref)
                        || string_field(change, "change_id").as_deref() == Some(change_ref)
                })
                .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown Change {change_ref}")))?;
            change["status"] = JsonValue::String(status.to_string());
            Ok(change.clone())
        }
    }

    struct RecordingRemote {
        task: JsonValue,
        changes: Vec<JsonValue>,
        events: Rc<RefCell<Vec<String>>>,
    }

    impl TaskWorkflowRemoteTaskReader for RecordingRemote {
        fn get_task(
            &mut self,
            task_id: &str,
            _repo_name: Option<&str>,
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            if string_field(&self.task, "task_id").as_deref() == Some(task_id) {
                Ok(self.task.clone())
            } else {
                Err(TaskWorkflowHttpClientError::Remote(format!(
                    "Unknown Task {task_id}"
                )))
            }
        }
    }

    impl TaskWorkflowRemoteChangeLister for RecordingRemote {
        fn list_changes(
            &mut self,
            _repo_name: &str,
        ) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
            Ok(self.changes.clone())
        }
    }

    impl TaskWorkflowRemoteChangeCloser for RecordingRemote {
        fn close_change(
            &mut self,
            change_ref: &str,
            status: &str,
            _repo_name: Option<&str>,
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            self.events
                .borrow_mut()
                .push(format!("change:{change_ref}:{status}"));
            let change = self
                .changes
                .iter_mut()
                .find(|change| {
                    string_field(change, "change_ref").as_deref() == Some(change_ref)
                        || string_field(change, "change_id").as_deref() == Some(change_ref)
                })
                .ok_or_else(|| {
                    TaskWorkflowHttpClientError::Remote(format!("Unknown Change {change_ref}"))
                })?;
            change["status"] = JsonValue::String(status.to_string());
            Ok(change.clone())
        }
    }

    struct RecordingRemoteTaskCloser {
        events: Rc<RefCell<Vec<String>>>,
    }

    impl TaskWorkflowRemoteTaskCloser for RecordingRemoteTaskCloser {
        fn close_task(
            &mut self,
            task_id: &str,
            status: &str,
            repo_name: Option<&str>,
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            self.events
                .borrow_mut()
                .push(format!("task:{task_id}:{status}"));
            Ok(json!({
                "repo_name": repo_name,
                "task_id": task_id,
                "status": status,
            }))
        }
    }

    #[test]
    fn local_abandon_cancels_only_open_owned_changes_before_task() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let task_store = RecordingTaskStore {
            task: RefCell::new(json!({"task_id":"LCT-7","status":"active"})),
            events: events.clone(),
        };
        let change_store = RecordingChangeStore {
            changes: RefCell::new(vec![
                json!({"task_id":"LCT-7","change_id":"C-01","change_ref":"LCT-7/C-01","status":"draft"}),
                json!({"task_id":"LCT-7","change_id":"C-02","change_ref":"LCT-7/C-02","status":"landed"}),
                json!({"task_id":"LCT-7","change_id":"C-03","change_ref":"LCT-7/C-03","status":"archived"}),
                json!({"task_id":"LCT-8","change_id":"C-01","change_ref":"LCT-8/C-01","status":"draft"}),
            ]),
            events: events.clone(),
        };

        let abandoned =
            task_abandon_local_with_stores(&task_store, &change_store, "LCT-7").unwrap();
        assert_eq!(abandoned["status"], json!("abandoned"));
        assert_eq!(
            *events.borrow(),
            vec!["change:LCT-7/C-01:canceled", "task:LCT-7:abandoned"]
        );
        let statuses = change_store
            .changes
            .borrow()
            .iter()
            .map(|change| {
                (
                    string_field(change, "change_ref").unwrap(),
                    string_field(change, "status").unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(statuses["LCT-7/C-01"], "canceled");
        assert_eq!(statuses["LCT-7/C-02"], "landed");
        assert_eq!(statuses["LCT-7/C-03"], "archived");
        assert_eq!(statuses["LCT-8/C-01"], "draft");

        let repeated = task_abandon_local_with_stores(&task_store, &change_store, "LCT-7").unwrap();
        assert_eq!(repeated["status"], json!("abandoned"));
        assert_eq!(
            *events.borrow(),
            vec!["change:LCT-7/C-01:canceled", "task:LCT-7:abandoned"]
        );
    }

    #[test]
    fn abandon_preflight_rejects_malformed_inventory_before_mutation() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let task_store = RecordingTaskStore {
            task: RefCell::new(json!({"task_id":"LCT-7","status":"active"})),
            events: events.clone(),
        };
        let change_store = RecordingChangeStore {
            changes: RefCell::new(vec![
                json!({"task_id":"LCT-7","change_id":"C-01","change_ref":"LCT-7/C-01","status":"draft"}),
                json!({"change_id":"C-02","change_ref":"LCT-7/C-02","status":"draft"}),
            ]),
            events: events.clone(),
        };

        let error = task_abandon_local_with_stores(&task_store, &change_store, "LCT-7")
            .expect_err("malformed complete inventory must fail closed");
        assert!(error.contains("task_id"));
        assert!(events.borrow().is_empty());
        assert_eq!(change_store.changes.borrow()[0]["status"], json!("draft"));
        assert_eq!(task_store.task.borrow()["status"], json!("active"));
    }

    #[test]
    fn abandon_rejects_a_different_terminal_task_outcome_before_mutation() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let task_store = RecordingTaskStore {
            task: RefCell::new(json!({"task_id":"LCT-7","status":"completed"})),
            events: events.clone(),
        };
        let change_store = RecordingChangeStore {
            changes: RefCell::new(vec![
                json!({"task_id":"LCT-7","change_id":"C-01","change_ref":"LCT-7/C-01","status":"draft"}),
            ]),
            events: events.clone(),
        };

        let error = task_abandon_local_with_stores(&task_store, &change_store, "LCT-7")
            .expect_err("completed Task must not be rewritten as abandoned");
        assert!(error.contains("already terminal with status `completed`"));
        assert!(events.borrow().is_empty());
        assert_eq!(change_store.changes.borrow()[0]["status"], json!("draft"));
        assert_eq!(task_store.task.borrow()["status"], json!("completed"));
    }

    #[test]
    fn remote_abandon_cancels_open_change_before_task() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut remote = RecordingRemote {
            task: json!({"task_id":"RCT-9","status":"active"}),
            changes: vec![
                json!({"task_id":"RCT-9","change_id":"RCT-9/C-01","status":"review"}),
                json!({"task_id":"RCT-9","change_id":"RCT-9/C-02","status":"landed"}),
                json!({"task_id":"RCT-10","change_id":"RCT-10/C-01","status":"draft"}),
            ],
            events: events.clone(),
        };
        let mut task_closer = RecordingRemoteTaskCloser {
            events: events.clone(),
        };

        let abandoned =
            task_abandon_remote_with_remotes(&mut remote, &mut task_closer, "fixture-ait", "RCT-9")
                .unwrap();
        assert_eq!(abandoned["status"], json!("abandoned"));
        assert_eq!(
            *events.borrow(),
            vec!["change:RCT-9/C-01:canceled", "task:RCT-9:abandoned"]
        );
        assert_eq!(remote.changes[0]["status"], json!("canceled"));
        assert_eq!(remote.changes[1]["status"], json!("landed"));
        assert_eq!(remote.changes[2]["status"], json!("draft"));
    }

    #[test]
    fn already_abandoned_remote_task_repairs_open_change_without_reclosing_task() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut remote = RecordingRemote {
            task: json!({"task_id":"RCT-9","status":"abandoned"}),
            changes: vec![json!({"task_id":"RCT-9","change_id":"RCT-9/C-01","status":"draft"})],
            events: events.clone(),
        };
        let mut task_closer = RecordingRemoteTaskCloser {
            events: events.clone(),
        };

        let repaired =
            task_abandon_remote_with_remotes(&mut remote, &mut task_closer, "fixture-ait", "RCT-9")
                .unwrap();
        assert_eq!(repaired["status"], json!("abandoned"));
        assert_eq!(*events.borrow(), vec!["change:RCT-9/C-01:canceled"]);
        assert_eq!(remote.changes[0]["status"], json!("canceled"));
    }
}
