use super::repositories::RoutedBinaryNativeRepositoryService;
use ait_server_core::foundation::server_workflow_store::{
    ServerWorkflowAttestationStore, ServerWorkflowChangeStore, ServerWorkflowLandStore,
    ServerWorkflowPatchsetStore, ServerWorkflowPolicyStore, ServerWorkflowReviewStore,
    ServerWorkflowStore, ServerWorkflowTaskStore,
};
use ait_server_core::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store;
use serde_json::Value as JsonValue;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct RoutedBinaryWorkflowStore {
    repositories: Arc<RoutedBinaryNativeRepositoryService>,
}

impl RoutedBinaryWorkflowStore {
    pub(crate) fn new(repositories: Arc<RoutedBinaryNativeRepositoryService>) -> Self {
        Self { repositories }
    }

    pub(crate) fn store_for_repo(
        &self,
        repository_index: &str,
    ) -> Result<(String, Arc<dyn ServerWorkflowStore>), String> {
        let (entry, namespace, db) = self
            .repositories
            .resolve_workflow_runtime(repository_index)
            .map_err(|error| error.to_string())?;
        let store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db, &namespace)?.into_arc();
        Ok((entry.repo_name, store))
    }

    pub(crate) fn physical_patchset_index(
        &self,
        repository_index: &str,
        patchset_id: &str,
    ) -> Result<u32, String> {
        let (_, namespace, db) = self
            .repositories
            .resolve_workflow_runtime(repository_index)
            .map_err(|error| error.to_string())?;
        BinaryDbServerWorkflowV0Store::new_remote_frozen(db, &namespace)?
            .physical_patchset_index(patchset_id)
    }

    fn search_json<F>(&self, label: &str, mut operation: F) -> Result<JsonValue, String>
    where
        F: FnMut(&dyn ServerWorkflowStore) -> Result<JsonValue, String>,
    {
        let mut found = Vec::new();
        for (repository_index, repo_name) in self
            .repositories
            .authority_repositories()
            .map_err(|error| error.to_string())?
        {
            let (_, store) = self.store_for_repo(&repository_index)?;
            match operation(store.as_ref()) {
                Ok(value) => found.push((repository_index.clone(), value)),
                Err(error) if is_missing_workflow_entity(&error) => {}
                Err(error) => {
                    return Err(format!(
                        "Binary DB Repository {repository_index} ({repo_name}) failed while resolving {label}: {error}"
                    ));
                }
            }
        }
        match found.len() {
            1 => Ok(found.remove(0).1),
            0 => Err(format!("Unknown {label} in Binary DB repository registry")),
            _ => Err(format!(
                "Ambiguous {label} exists in multiple Binary DB repositories: {}",
                found
                    .iter()
                    .map(|(repository_index, _)| repository_index.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            )),
        }
    }

    fn locate_store<F>(
        &self,
        label: &str,
        mut probe: F,
    ) -> Result<Arc<dyn ServerWorkflowStore>, String>
    where
        F: FnMut(&dyn ServerWorkflowStore) -> Result<JsonValue, String>,
    {
        let mut found = Vec::new();
        for (repository_index, repo_name) in self
            .repositories
            .authority_repositories()
            .map_err(|error| error.to_string())?
        {
            let (_, store) = self.store_for_repo(&repository_index)?;
            match probe(store.as_ref()) {
                Ok(_) => found.push((repository_index.clone(), store.clone())),
                Err(error) if is_missing_workflow_entity(&error) => {}
                Err(error) => {
                    return Err(format!(
                        "Binary DB Repository {repository_index} ({repo_name}) failed while resolving {label}: {error}"
                    ));
                }
            }
        }
        match found.len() {
            1 => Ok(found.remove(0).1),
            0 => Err(format!("Unknown {label} in Binary DB repository registry")),
            _ => Err(format!(
                "Ambiguous {label} exists in multiple Binary DB repositories: {}",
                found
                    .iter()
                    .map(|(repository_index, _)| repository_index.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            )),
        }
    }

    fn store_for_task(&self, task_id: &str) -> Result<Arc<dyn ServerWorkflowStore>, String> {
        self.locate_store(&format!("task {task_id}"), |store| {
            store.get_task(None, task_id)
        })
    }

    fn store_for_change(&self, change_id: &str) -> Result<Arc<dyn ServerWorkflowStore>, String> {
        self.locate_store(&format!("change {change_id}"), |store| {
            store.get_change(None, change_id)
        })
    }

    pub(crate) fn store_for_patchset(
        &self,
        patchset_id: &str,
    ) -> Result<Arc<dyn ServerWorkflowStore>, String> {
        self.locate_store(&format!("patchset {patchset_id}"), |store| {
            store.get_patchset(None, patchset_id)
        })
    }

    pub(crate) fn patchset_ci_workflow_rows(
        &self,
        patchset_id: &str,
    ) -> Result<(JsonValue, JsonValue), String> {
        let store = self.store_for_patchset(patchset_id)?;
        Self::patchset_ci_workflow_rows_from_store(store.as_ref(), patchset_id)
    }

    pub(crate) fn repository_patchset_ci_workflow_rows(
        &self,
        repo_name: &str,
        patchset_id: &str,
    ) -> Result<(JsonValue, JsonValue), String> {
        let (_, store) = self.store_for_repo(repo_name)?;
        Self::patchset_ci_workflow_rows_from_store(store.as_ref(), patchset_id)
    }

    pub(crate) fn repository_patchset_ci_workflow_rows_by_id(
        &self,
        repository_index: &str,
        patchset_id: &str,
    ) -> Result<(JsonValue, JsonValue), String> {
        let (_, store) = self.store_for_repo(repository_index)?;
        Self::patchset_ci_workflow_rows_from_store(store.as_ref(), patchset_id)
    }

    fn patchset_ci_workflow_rows_from_store(
        store: &dyn ServerWorkflowStore,
        patchset_id: &str,
    ) -> Result<(JsonValue, JsonValue), String> {
        let patchset = store.get_patchset(None, patchset_id)?;
        let change_ref = patchset
            .get("change_ref")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Patchset {patchset_id} is missing change_ref"))?;
        let change = store.get_change(None, change_ref)?;
        Ok((patchset, change))
    }
}

fn is_missing_workflow_entity(error: &str) -> bool {
    error.starts_with("Unknown ")
        || error.contains(": Unknown ")
        || error.contains(" did not match any Binary DB records")
        || error.contains(" is not a Task in this repository namespace")
}

#[cfg(test)]
mod tests {
    use super::is_missing_workflow_entity;

    #[test]
    fn binary_workflow_lookup_treats_empty_adapter_matches_as_repository_local_misses() {
        assert!(is_missing_workflow_entity(
            "Binary DB workflow adapter ServerWorkflowPatchsetStore::get_patchset failed: PatchsetById did not match any Binary DB records"
        ));
        assert!(is_missing_workflow_entity("Unknown patchset RSEP-1"));
        assert!(is_missing_workflow_entity(
            "\"RAST-0001\" is not a Task in this repository namespace"
        ));
        assert!(!is_missing_workflow_entity(
            "patchset.bin layout is corrupt"
        ));
    }
}

impl ServerWorkflowTaskStore for RoutedBinaryWorkflowStore {
    fn prepare_history_promotion(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let (repo_name, store) = self.store_for_repo(repo_name)?;
        store.prepare_history_promotion(&repo_name, payload)
    }

    fn start_plan_bound_task(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let (repo_name, store) = self.store_for_repo(repo_name)?;
        store.start_plan_bound_task(&repo_name, payload)
    }

    fn create_task(&self, repo_name: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let (repo_name, store) = self.store_for_repo(repo_name)?;
        store.create_task(&repo_name, payload)
    }

    fn list_tasks(&self, repo_name: &str) -> Result<JsonValue, String> {
        let (repo_name, store) = self.store_for_repo(repo_name)?;
        store.list_tasks(&repo_name)
    }

    fn get_task(&self, repo_name: Option<&str>, task_ref: &str) -> Result<JsonValue, String> {
        match repo_name {
            Some(repo_name) => {
                let (repo_name, store) = self.store_for_repo(repo_name)?;
                store.get_task(Some(&repo_name), task_ref)
            }
            None => self.search_json(&format!("task {task_ref}"), |store| {
                store.get_task(None, task_ref)
            }),
        }
    }

    fn close_task(&self, task_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store_for_task(task_id)?.close_task(task_id, payload)
    }

    fn read_task_audit(
        &self,
        repo_name: &str,
        task_ref: &str,
        target_line: &str,
    ) -> Result<JsonValue, String> {
        let (repo_name, store) = self.store_for_repo(repo_name)?;
        store.read_task_audit(&repo_name, task_ref, target_line)
    }
}

impl ServerWorkflowChangeStore for RoutedBinaryWorkflowStore {
    fn create_change(&self, repo_name: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let (repo_name, store) = self.store_for_repo(repo_name)?;
        store.create_change(&repo_name, payload)
    }

    fn list_changes(&self, repo_name: &str) -> Result<JsonValue, String> {
        let (repo_name, store) = self.store_for_repo(repo_name)?;
        store.list_changes(&repo_name)
    }

    fn get_change(&self, repo_name: Option<&str>, change_ref: &str) -> Result<JsonValue, String> {
        match repo_name {
            Some(repo_name) => {
                let (repo_name, store) = self.store_for_repo(repo_name)?;
                store.get_change(Some(&repo_name), change_ref)
            }
            None => self.search_json(&format!("change {change_ref}"), |store| {
                store.get_change(None, change_ref)
            }),
        }
    }

    fn close_change(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store_for_change(change_id)?
            .close_change(change_id, payload)
    }
}

impl ServerWorkflowReviewStore for RoutedBinaryWorkflowStore {
    fn request_review(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store_for_change(change_id)?
            .request_review(change_id, payload)
    }

    fn record_review(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store_for_change(change_id)?
            .record_review(change_id, payload)
    }

    fn list_reviews(&self, change_id: &str) -> Result<JsonValue, String> {
        self.store_for_change(change_id)?.list_reviews(change_id)
    }
}

impl ServerWorkflowPatchsetStore for RoutedBinaryWorkflowStore {
    fn select_patchset(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store_for_change(change_id)?
            .select_patchset(change_id, payload)
    }

    fn get_patchset(
        &self,
        repo_name: Option<&str>,
        patchset_id: &str,
    ) -> Result<JsonValue, String> {
        match repo_name {
            Some(repo_name) => {
                let (repo_name, store) = self.store_for_repo(repo_name)?;
                store.get_patchset(Some(&repo_name), patchset_id)
            }
            None => self.search_json(&format!("patchset {patchset_id}"), |store| {
                store.get_patchset(None, patchset_id)
            }),
        }
    }

    fn publish_patchset(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store_for_change(change_id)?
            .publish_patchset(change_id, payload)
    }

    fn list_patchsets(
        &self,
        repo_name: Option<&str>,
        change_ref: &str,
    ) -> Result<JsonValue, String> {
        match repo_name {
            Some(repo_name) => {
                let (repo_name, store) = self.store_for_repo(repo_name)?;
                store.list_patchsets(Some(&repo_name), change_ref)
            }
            None => self
                .store_for_change(change_ref)?
                .list_patchsets(None, change_ref),
        }
    }
}

impl ServerWorkflowAttestationStore for RoutedBinaryWorkflowStore {
    fn put_attestation(&self, patchset_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store_for_patchset(patchset_id)?
            .put_attestation(patchset_id, payload)
    }

    fn get_attestation(&self, patchset_id: &str) -> Result<JsonValue, String> {
        self.store_for_patchset(patchset_id)?
            .get_attestation(patchset_id)
    }
}

impl ServerWorkflowPolicyStore for RoutedBinaryWorkflowStore {
    fn get_policy(&self, patchset_id: &str) -> Result<JsonValue, String> {
        self.store_for_patchset(patchset_id)?
            .get_policy(patchset_id)
    }

    fn evaluate_policy(&self, patchset_id: &str) -> Result<JsonValue, String> {
        self.store_for_patchset(patchset_id)?
            .evaluate_policy(patchset_id)
    }

    fn run_patchset_ci(&self, patchset_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store_for_patchset(patchset_id)?
            .run_patchset_ci(patchset_id, payload)
    }

    fn complete_patchset_ci(
        &self,
        patchset_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.store_for_patchset(patchset_id)?
            .complete_patchset_ci(patchset_id, payload)
    }
}

impl ServerWorkflowLandStore for RoutedBinaryWorkflowStore {
    fn resolve_task_land_change_ref(&self, task_or_change_ref: &str) -> Result<String, String> {
        let store = if task_or_change_ref.contains("/C-") {
            self.store_for_change(task_or_change_ref)?
        } else {
            self.store_for_task(task_or_change_ref)?
        };
        store.resolve_task_land_change_ref(task_or_change_ref)
    }

    fn submit_task_land(
        &self,
        task_or_change_ref: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let store = if task_or_change_ref.contains("/C-") {
            self.store_for_change(task_or_change_ref)?
        } else {
            self.store_for_task(task_or_change_ref)?
        };
        store.submit_task_land(task_or_change_ref, payload)
    }

    fn submit_land(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.store_for_change(change_id)?
            .submit_land(change_id, payload)
    }

    fn get_land(&self, repo_name: Option<&str>, submission_id: &str) -> Result<JsonValue, String> {
        match repo_name {
            Some(repo_name) => {
                let (repo_name, store) = self.store_for_repo(repo_name)?;
                store.get_land(Some(&repo_name), submission_id)
            }
            None => self.search_json(&format!("land {submission_id}"), |store| {
                store.get_land(None, submission_id)
            }),
        }
    }
}
