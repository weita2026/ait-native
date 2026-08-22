use super::{RoutedBinaryNativeRepositoryService, RoutedBinaryWorkflowStore};
use crate::operational_binary_runtime::OperationalBinaryRuntime;
use crate::runtime_service::{BinaryServerRuntimeService, ServerRuntimeService};
use ait_server_core::foundation::native_repositories::NativeRepositoryService;
use ait_server_core::foundation::server_operational_repository_registry::OperationalRepositoryEntry;
use ait_server_core::foundation::server_workflow_store::{
    patchset_ci_trigger_requests_new_run, ServerWorkflowStore,
};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

const DEFAULT_MAX_JOB_ATTEMPTS: u16 = 3;

#[derive(Clone)]
pub(crate) struct BinaryServingServices {
    pub(crate) repository: Arc<dyn NativeRepositoryService>,
    pub(crate) workflow: Arc<dyn ServerWorkflowStore>,
    pub(crate) runtime: Arc<dyn ServerRuntimeService>,
}

impl BinaryServingServices {
    pub(crate) fn new(operational: Arc<OperationalBinaryRuntime>) -> Result<Self, String> {
        let repository_router = Arc::new(RoutedBinaryNativeRepositoryService::new(
            operational.clone(),
        ));
        let workflow_router = Arc::new(RoutedBinaryWorkflowStore::new(repository_router.clone()));
        let runtime = Arc::new(RoutedBinaryRuntimeService::new(
            operational,
            repository_router.clone(),
            workflow_router.clone(),
        )?);
        Ok(Self {
            repository: repository_router,
            workflow: workflow_router,
            runtime,
        })
    }
}

#[derive(Clone)]
struct RepositoryRuntime {
    entry: OperationalRepositoryEntry,
    service: Arc<BinaryServerRuntimeService>,
}

#[derive(Clone)]
pub(crate) struct RoutedBinaryRuntimeService {
    operational: Arc<OperationalBinaryRuntime>,
    repository: Arc<RoutedBinaryNativeRepositoryService>,
    workflow: Arc<RoutedBinaryWorkflowStore>,
    services: Arc<RwLock<BTreeMap<u32, RepositoryRuntime>>>,
}

impl RoutedBinaryRuntimeService {
    fn new(
        operational: Arc<OperationalBinaryRuntime>,
        repository: Arc<RoutedBinaryNativeRepositoryService>,
        workflow: Arc<RoutedBinaryWorkflowStore>,
    ) -> Result<Self, String> {
        let mut services = BTreeMap::new();
        for repository_index in operational.serving_repository_indexes() {
            let (entry, db) = operational
                .repository_db(repository_index)
                .map_err(|error| error.to_string())?;
            let (_, workflow_store) = workflow.store_for_repo(&repository_index.to_string())?;
            let service = Arc::new(BinaryServerRuntimeService::new(db, workflow_store));
            services.insert(repository_index, RepositoryRuntime { entry, service });
        }
        Ok(Self {
            operational,
            repository,
            workflow,
            services: Arc::new(RwLock::new(services)),
        })
    }

    fn service(&self, repository_index: &str) -> Result<RepositoryRuntime, String> {
        let index = parse_repository_index(repository_index)?;
        if let Some(runtime) = self
            .services
            .read()
            .map_err(|_| "Binary runtime service cache lock is poisoned".to_string())?
            .get(&index)
            .cloned()
        {
            self.operational
                .repository_db(index)
                .map_err(|error| error.to_string())?;
            return Ok(runtime);
        }
        let runtime = self.build_service(index)?;
        Ok(self
            .services
            .write()
            .map_err(|_| "Binary runtime service cache lock is poisoned".to_string())?
            .entry(index)
            .or_insert(runtime)
            .clone())
    }

    fn build_service(&self, repository_index: u32) -> Result<RepositoryRuntime, String> {
        let (entry, db) = self
            .operational
            .repository_db(repository_index)
            .map_err(|error| error.to_string())?;
        let (_, workflow_store) = self
            .workflow
            .store_for_repo(&repository_index.to_string())?;
        let service = Arc::new(BinaryServerRuntimeService::new(db, workflow_store));
        Ok(RepositoryRuntime { entry, service })
    }

    fn all_services(&self) -> Result<Vec<RepositoryRuntime>, String> {
        self.operational
            .serving_repository_indexes()
            .into_iter()
            .map(|repository_index| self.service(&repository_index.to_string()))
            .collect()
    }

    fn unique_service_for_plan(&self, plan_id: &str) -> Result<RepositoryRuntime, String> {
        let mut matches = Vec::new();
        for runtime in self.all_services()? {
            match runtime.service.get_plan(plan_id) {
                Ok(_) => matches.push(runtime),
                Err(error) if is_missing_plan_error(&error) => {}
                Err(error) => return Err(error),
            }
        }
        match matches.as_slice() {
            [runtime] => Ok(runtime.clone()),
            [] => Err(format!("Unknown plan: {plan_id}")),
            _ => Err(format!(
                "Plan {plan_id} is ambiguous across numeric Repository authorities"
            )),
        }
    }

    fn workflow_store(
        &self,
        repository_index: &str,
    ) -> Result<(u32, String, Arc<dyn ServerWorkflowStore>), String> {
        let index = parse_repository_index(repository_index)?;
        let (repo_name, store) = self.workflow.store_for_repo(repository_index)?;
        Ok((index, repo_name, store))
    }

    fn patchset_ci_status(
        &self,
        repository_index: &str,
        patchset_id: &str,
        recent_limit: i64,
        projection: Option<&str>,
    ) -> Result<JsonValue, String> {
        let recent_limit = patchset_ci_status_recent_limit(recent_limit, projection)?;
        let (index, repo_name, workflow) = self.workflow_store(repository_index)?;
        let patchset = workflow.get_patchset(None, patchset_id)?;
        let patchset_index = self
            .workflow
            .physical_patchset_index(repository_index, patchset_id)?;
        let jobs = self
            .operational
            .patchset_ci_jobs(index, patchset_index, recent_limit)
            .map_err(|error| error.to_string())?;
        let completed_at_s = patchset
            .get("ci_completed_at_s")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let run_seq = patchset
            .get("ci_run_seq")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let tests_status = if completed_at_s == 0 {
            "pending"
        } else {
            patchset
                .pointer("/ci/tests_status")
                .and_then(JsonValue::as_str)
                .unwrap_or("none")
        };
        let latest_job = jobs["latest_job"].clone();
        let recent_jobs = jobs["recent_jobs"].clone();
        let status = json!({
            "available": true,
            "repository_index": index,
            "patchset_index": patchset_index,
            "patchset_id": patchset_id,
            "ci_run_seq": run_seq,
            "ci_completed_at_s": if completed_at_s == 0 {
                JsonValue::Null
            } else {
                json!(completed_at_s)
            },
            "tests_status": tests_status,
            "overall_status": patchset.pointer("/ci/overall_status").cloned().unwrap_or(json!("none")),
            "lint_status": patchset.pointer("/ci/lint_status").cloned().unwrap_or(json!("none")),
            "selected_suite_count": patchset.pointer("/ci/selected_suite_count").cloned().unwrap_or(json!(0)),
            "suite_result_count": patchset.pointer("/ci/suite_result_count").cloned().unwrap_or(json!(0)),
            "blocking_failure_count": patchset.pointer("/ci/blocking_failure_count").cloned().unwrap_or(json!(0)),
            "has_runnable_evidence": !latest_job.is_null() || completed_at_s > 0,
            "selected_suite_ids": [],
            "suite_results": [],
            "latest_job": latest_job,
            "recent_jobs": recent_jobs,
        });
        project_patchset_ci_status(
            status,
            projection,
            patchset.get("change_id").and_then(JsonValue::as_str),
            patchset.get("change_ref").and_then(JsonValue::as_str),
            &repo_name,
            recent_limit,
        )
    }

    fn run_scoped_patchset_ci(
        &self,
        repository_index: &str,
        patchset_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let (index, _, workflow) = self.workflow_store(repository_index)?;
        let patchset = workflow.run_patchset_ci(patchset_id, payload)?;
        let trigger = payload
            .get("trigger")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("manual_rerun");
        let completed_at_s = patchset
            .get("ci_completed_at_s")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        if completed_at_s > 0 && !patchset_ci_trigger_requests_new_run(Some(trigger)) {
            return Ok(json!({
                "repository_index": index,
                "patchset_id": patchset_id,
                "queued": false,
                "job": JsonValue::Null,
                "trigger": trigger,
                "delivery": "existing_terminal",
                "patchset_ci": patchset,
            }));
        }
        let patchset_index = self
            .workflow
            .physical_patchset_index(repository_index, patchset_id)?;
        let job = self
            .operational
            .enqueue_patchset_ci(index, patchset_index, false, DEFAULT_MAX_JOB_ATTEMPTS)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "repository_index": index,
            "patchset_id": patchset_id,
            "queued": true,
            "job": job,
            "trigger": trigger,
            "delivery": "binary_worker_job",
            "patchset_ci": patchset,
        }))
    }

    fn run_scoped_repo_ci(
        &self,
        repository_index: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let index = parse_repository_index(repository_index)?;
        self.service(repository_index)?;
        let snapshot_id = payload
            .get("snapshot_id")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                self.repository
                    .get_line(repository_index, "main")
                    .ok()
                    .and_then(|line| {
                        line.get("head_snapshot_id")
                            .and_then(JsonValue::as_str)
                            .map(str::to_string)
                    })
            })
            .ok_or_else(|| {
                format!(
                    "Repository {repository_index} logical main has no Snapshot selected for repo.ci"
                )
            })?;
        let snapshot_index = self
            .operational
            .snapshot_index_for_id(index, &snapshot_id)
            .map_err(|error| error.to_string())?;
        let job = self
            .operational
            .enqueue_repo_ci(index, snapshot_index, DEFAULT_MAX_JOB_ATTEMPTS)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "repository_index": index,
            "snapshot_id": snapshot_id,
            "snapshot_index": snapshot_index,
            "queued": true,
            "job": job,
            "delivery": "binary_worker_job",
        }))
    }
}

impl ServerRuntimeService for RoutedBinaryRuntimeService {
    fn request_queue_read_models_refresh(&self, repo_name: Option<&str>) {
        let Ok(services) = self.all_services() else {
            return;
        };
        for runtime in services {
            if repo_name.is_none() || repo_name == Some(runtime.entry.repo_name.as_str()) {
                runtime
                    .service
                    .request_queue_read_models_refresh(Some(&runtime.entry.repo_name));
            }
        }
    }

    fn read_repository_queue_summary(
        &self,
        repository_index: &str,
        status: Option<&str>,
    ) -> Result<JsonValue, String> {
        let runtime = self.service(repository_index)?;
        runtime
            .service
            .read_repository_queue_summary(repository_index, status)
    }

    fn read_repository_task_queue(
        &self,
        repository_index: &str,
        status: Option<&str>,
    ) -> Result<JsonValue, String> {
        let runtime = self.service(repository_index)?;
        runtime
            .service
            .read_repository_task_queue(repository_index, status)
    }

    fn read_repository_reviewer_inbox(&self, repository_index: &str) -> Result<JsonValue, String> {
        let runtime = self.service(repository_index)?;
        runtime
            .service
            .read_repository_reviewer_inbox(repository_index)
    }

    fn run_repo_ci(
        &self,
        repository_index: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.run_scoped_repo_ci(repository_index, payload)
    }

    fn run_patchset_ci(
        &self,
        _patchset_id: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err("Patchset CI requires an explicit numeric repository_index".to_string())
    }

    fn run_repository_authority_patchset_ci(
        &self,
        repository_index: &str,
        patchset_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.run_scoped_patchset_ci(repository_index, patchset_id, payload)
    }

    fn read_patchset_ci_status(
        &self,
        _patchset_id: &str,
        _recent_limit: i64,
        _projection: Option<&str>,
    ) -> Result<JsonValue, String> {
        Err("Patchset CI status requires an explicit numeric repository_index".to_string())
    }

    fn read_repository_authority_patchset_ci_status(
        &self,
        repository_index: &str,
        patchset_id: &str,
        recent_limit: i64,
        projection: Option<&str>,
    ) -> Result<JsonValue, String> {
        self.patchset_ci_status(repository_index, patchset_id, recent_limit, projection)
    }

    fn plan_repository_zstd_bulk(
        &self,
        repository_index: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.repository
            .zstd_bulk_plan(repository_index, payload.clone())
            .map_err(|error| error.to_string())
    }

    fn get_repository_zstd_object_pack(
        &self,
        repository_index: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, String> {
        self.repository
            .get_zstd_bulk_object_pack(repository_index, pack_id)
            .map_err(|error| error.to_string())
    }

    fn get_repository_zstd_tree_pack(
        &self,
        repository_index: &str,
        pack_id: &str,
    ) -> Result<Vec<u8>, String> {
        self.repository
            .get_zstd_bulk_tree_pack(repository_index, pack_id)
            .map_err(|error| error.to_string())
    }

    fn commit_repository_zstd_bulk(
        &self,
        repository_index: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.repository
            .commit_zstd_bulk(repository_index, payload.clone())
            .map_err(|error| error.to_string())
    }

    fn get_repository_zstd_import_manifest(
        &self,
        repository_index: &str,
        snapshot_id: &str,
    ) -> Result<JsonValue, String> {
        self.repository
            .get_zstd_import_manifest(repository_index, snapshot_id)
            .map_err(|error| error.to_string())
    }

    fn get_repository_zstd_pull_manifest(
        &self,
        repository_index: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.repository
            .get_zstd_pull_manifest(repository_index, payload.clone())
            .map_err(|error| error.to_string())
    }

    fn list_plans(
        &self,
        repository_index: &str,
        artifact_path: Option<&str>,
    ) -> Result<JsonValue, String> {
        let runtime = self.service(repository_index)?;
        runtime
            .service
            .list_plans(&runtime.entry.repo_name, artifact_path)
    }

    fn create_plan(
        &self,
        repository_index: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let runtime = self.service(repository_index)?;
        runtime
            .service
            .create_plan(&runtime.entry.repo_name, payload)
    }

    fn list_repository_plans(
        &self,
        repository_index: &str,
        artifact_path: Option<&str>,
    ) -> Result<JsonValue, String> {
        self.list_plans(repository_index, artifact_path)
    }

    fn create_repository_plan(
        &self,
        repository_index: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.create_plan(repository_index, payload)
    }

    fn get_plan(&self, plan_id: &str) -> Result<JsonValue, String> {
        self.unique_service_for_plan(plan_id)?
            .service
            .get_plan(plan_id)
    }

    fn get_repository_plan(
        &self,
        repository_index: &str,
        plan_id: &str,
    ) -> Result<JsonValue, String> {
        self.service(repository_index)?.service.get_plan(plan_id)
    }

    fn update_plan_status(&self, plan_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.unique_service_for_plan(plan_id)?
            .service
            .update_plan_status(plan_id, payload)
    }

    fn update_repository_plan_status(
        &self,
        repository_index: &str,
        plan_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.service(repository_index)?
            .service
            .update_plan_status(plan_id, payload)
    }

    fn list_plan_revisions(&self, plan_id: &str) -> Result<JsonValue, String> {
        self.unique_service_for_plan(plan_id)?
            .service
            .list_plan_revisions(plan_id)
    }

    fn list_repository_plan_revisions(
        &self,
        repository_index: &str,
        plan_id: &str,
    ) -> Result<JsonValue, String> {
        self.service(repository_index)?
            .service
            .list_plan_revisions(plan_id)
    }

    fn get_plan_revision(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        self.unique_service_for_plan(plan_id)?
            .service
            .get_plan_revision(plan_id, plan_revision_id)
    }

    fn get_repository_plan_revision(
        &self,
        repository_index: &str,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String> {
        self.service(repository_index)?
            .service
            .get_plan_revision(plan_id, plan_revision_id)
    }

    fn resolve_task_plan_linkage(
        &self,
        repository_index: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let runtime = self.service(repository_index)?;
        runtime
            .service
            .resolve_task_plan_linkage(&runtime.entry.repo_name, payload)
    }

    fn list_plan_ids_matching_contains(
        &self,
        repository_index: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let runtime = self.service(repository_index)?;
        runtime
            .service
            .list_plan_ids_matching_contains(&runtime.entry.repo_name, payload)
    }

    fn revise_plan(&self, plan_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.unique_service_for_plan(plan_id)?
            .service
            .revise_plan(plan_id, payload)
    }

    fn revise_repository_plan(
        &self,
        repository_index: &str,
        plan_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.service(repository_index)?
            .service
            .revise_plan(plan_id, payload)
    }

    fn put_plan_revision_artifacts(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.unique_service_for_plan(plan_id)?
            .service
            .put_plan_revision_artifacts(plan_id, plan_revision_id, payload)
    }

    fn put_repository_plan_revision_artifacts(
        &self,
        repository_index: &str,
        plan_id: &str,
        plan_revision_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.service(repository_index)?
            .service
            .put_plan_revision_artifacts(plan_id, plan_revision_id, payload)
    }
}

fn parse_repository_index(value: &str) -> Result<u32, String> {
    let value = value.trim();
    if value.is_empty()
        || value.bytes().any(|byte| !byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(
            "repository_index must be canonical unsigned base-10 without leading zeroes"
                .to_string(),
        );
    }
    value
        .parse::<u32>()
        .map_err(|_| "repository_index exceeds u32".to_string())
}

fn normalize_limit(limit: i64) -> Result<usize, String> {
    usize::try_from(limit)
        .ok()
        .filter(|value| (1..=1_000).contains(value))
        .ok_or_else(|| "list limit must be between 1 and 1000".to_string())
}

fn patchset_ci_status_recent_limit(
    recent_limit: i64,
    projection: Option<&str>,
) -> Result<usize, String> {
    let recent_limit = normalize_limit(recent_limit)?;
    match projection {
        None => Ok(recent_limit),
        Some("readiness") => Ok(recent_limit.min(20)),
        Some(value) => Err(format!(
            "Unsupported patchset CI status projection `{value}`. Expected `readiness`."
        )),
    }
}

fn project_patchset_ci_status(
    mut status: JsonValue,
    projection: Option<&str>,
    change_id: Option<&str>,
    change_ref: Option<&str>,
    repo_name: &str,
    recent_limit: usize,
) -> Result<JsonValue, String> {
    let Some(projection) = projection else {
        return Ok(status);
    };
    if projection != "readiness" {
        return Err(format!(
            "Unsupported patchset CI status projection `{projection}`. Expected `readiness`."
        ));
    }
    let required_identity = |value: Option<&str>, field: &str| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                format!("Patchset CI readiness projection is missing non-empty {field}.")
            })
    };
    let change_id = required_identity(change_id, "change_id")?;
    let change_ref = required_identity(change_ref, "change_ref")?;
    let repo_name = required_identity(Some(repo_name), "repo_name")?;
    let object = status
        .as_object_mut()
        .ok_or_else(|| "Patchset CI status projection requires a JSON object.".to_string())?;
    let completed = object
        .get("ci_completed_at_s")
        .and_then(JsonValue::as_u64)
        .is_some_and(|value| value > 0);
    let suite_result_count = object
        .get("suite_result_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default();
    let blocking_failure_count = object
        .get("blocking_failure_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default();
    object.insert(
        "contract".to_string(),
        json!("ait.server.patchset_ci.readiness.v1"),
    );
    object.insert("projection".to_string(), json!("readiness"));
    object.insert("change_id".to_string(), json!(change_id));
    object.insert("change_ref".to_string(), json!(change_ref));
    object.insert("repo_name".to_string(), json!(repo_name));
    object.insert("recent_limit_applied".to_string(), json!(recent_limit));
    object.insert(
        "has_runnable_evidence".to_string(),
        json!(completed && (suite_result_count > 0 || blocking_failure_count > 0)),
    );
    Ok(status)
}

fn is_missing_plan_error(error: &str) -> bool {
    error.contains("Unknown plan") || error.contains("did not match any Binary DB records")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_indexes_are_canonical_decimal_only() {
        assert_eq!(parse_repository_index("0").unwrap(), 0);
        assert_eq!(parse_repository_index("3").unwrap(), 3);
        assert!(parse_repository_index("03").is_err());
        assert!(parse_repository_index("ait-core").is_err());
    }

    #[test]
    fn patchset_ci_readiness_projection_is_bounded_and_complete() {
        let ordinary = json!({
            "available": true,
            "patchset_id": "RWTT-0057/C-01/P-01",
            "ci_completed_at_s": 1_787_326_949_u64,
            "tests_status": "pass",
            "selected_suite_ids": [],
            "suite_result_count": 1,
            "blocking_failure_count": 0,
            "has_runnable_evidence": true,
            "latest_job": {"worker_job_index": 6, "state": "succeeded"},
            "recent_jobs": [{"worker_job_index": 6, "state": "succeeded"}],
        });

        assert_eq!(
            project_patchset_ci_status(ordinary.clone(), None, None, None, "ait-web-test", 1_000,)
                .expect("ordinary projection"),
            ordinary
        );
        let readiness = project_patchset_ci_status(
            ordinary,
            Some("readiness"),
            Some("C-01"),
            Some("RWTT-0057/C-01"),
            "ait-web-test",
            20,
        )
        .expect("readiness projection");
        assert_eq!(
            readiness["contract"],
            json!("ait.server.patchset_ci.readiness.v1")
        );
        assert_eq!(readiness["projection"], json!("readiness"));
        assert_eq!(readiness["patchset_id"], json!("RWTT-0057/C-01/P-01"));
        assert_eq!(readiness["change_id"], json!("C-01"));
        assert_eq!(readiness["change_ref"], json!("RWTT-0057/C-01"));
        assert_eq!(readiness["repo_name"], json!("ait-web-test"));
        assert_eq!(readiness["recent_limit_applied"], json!(20));
        assert_eq!(readiness["has_runnable_evidence"], json!(true));
        assert_eq!(readiness["latest_job"]["worker_job_index"], json!(6));
        assert_eq!(readiness["recent_jobs"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn patchset_ci_readiness_limit_and_projection_fail_closed() {
        assert_eq!(patchset_ci_status_recent_limit(1_000, None).unwrap(), 1_000);
        assert_eq!(
            patchset_ci_status_recent_limit(1_000, Some("readiness")).unwrap(),
            20
        );
        assert!(patchset_ci_status_recent_limit(10, Some("diagnostics")).is_err());
        assert!(project_patchset_ci_status(
            json!({}),
            Some("readiness"),
            None,
            Some("RWTT-0057/C-01"),
            "ait-web-test",
            10,
        )
        .is_err());
    }
}
