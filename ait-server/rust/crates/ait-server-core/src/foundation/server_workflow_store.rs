use serde_json::Value as JsonValue;
use std::any::Any;

pub const DEFAULT_SERVER_WORKFLOW_BACKEND: &str = "binary";
pub const SERVER_WORKFLOW_BACKEND_ENV: &str = "AIT_NATIVE_SERVER_WORKFLOW_BACKEND";

pub fn patchset_ci_trigger_requests_new_run(trigger: Option<&str>) -> bool {
    matches!(
        trigger
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("manual_rerun"),
        "manual_rerun" | "base_stale_after_land_rerun"
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerWorkflowBackend {
    Binary,
}

impl ServerWorkflowBackend {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "" | "binary" => Ok(Self::Binary),
            other => Err(format!(
                "Unsupported {SERVER_WORKFLOW_BACKEND_ENV}: '{other}'. The release server supports only the registry-backed binary repository authority."
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Binary => "binary",
        }
    }
}

pub fn resolve_server_workflow_backend(
    raw_backend: Option<&str>,
) -> Result<ServerWorkflowBackend, String> {
    ServerWorkflowBackend::parse(raw_backend.unwrap_or(DEFAULT_SERVER_WORKFLOW_BACKEND))
}

pub trait ServerWorkflowTaskStore: Send + Sync {
    fn prepare_history_promotion(
        &self,
        _repo_name: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err("Workflow history promotion is unavailable for this workflow backend.".to_string())
    }

    fn start_plan_bound_task(
        &self,
        _repo_name: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err("Atomic Plan-bound Task start is unavailable for this workflow backend.".to_string())
    }

    fn create_task(&self, repo_name: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn list_tasks(&self, repo_name: &str) -> Result<JsonValue, String>;

    fn get_task(&self, repo_name: Option<&str>, task_ref: &str) -> Result<JsonValue, String>;

    fn close_task(&self, task_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn read_task_audit(
        &self,
        repo_name: &str,
        task_ref: &str,
        target_line: &str,
    ) -> Result<JsonValue, String>;
}

pub trait ServerWorkflowChangeStore: Send + Sync {
    fn create_change(&self, repo_name: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn list_changes(&self, repo_name: &str) -> Result<JsonValue, String>;

    fn get_change(&self, repo_name: Option<&str>, change_ref: &str) -> Result<JsonValue, String>;

    fn close_change(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;
}

pub trait ServerWorkflowReviewStore: Send + Sync {
    fn request_review(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn record_review(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn list_reviews(&self, change_id: &str) -> Result<JsonValue, String>;
}

pub trait ServerWorkflowPatchsetStore: Send + Sync {
    fn select_patchset(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn get_patchset(&self, repo_name: Option<&str>, patchset_id: &str)
        -> Result<JsonValue, String>;

    fn publish_patchset(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn list_patchsets(
        &self,
        repo_name: Option<&str>,
        change_ref: &str,
    ) -> Result<JsonValue, String>;
}

pub trait ServerWorkflowAttestationStore: Send + Sync {
    fn put_attestation(&self, patchset_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn get_attestation(&self, patchset_id: &str) -> Result<JsonValue, String>;
}

pub trait ServerWorkflowPolicyStore: Send + Sync {
    fn get_policy(&self, patchset_id: &str) -> Result<JsonValue, String>;

    fn evaluate_policy(&self, patchset_id: &str) -> Result<JsonValue, String>;

    fn run_patchset_ci(&self, patchset_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn complete_patchset_ci(
        &self,
        _patchset_id: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err("Patchset CI completion is unavailable for this workflow backend".to_string())
    }
}

pub trait ServerWorkflowLandStore: Send + Sync {
    fn resolve_task_land_change_ref(&self, _task_or_change_ref: &str) -> Result<String, String> {
        Err("Atomic Task Land resolution is unavailable for this workflow backend.".to_string())
    }

    fn submit_task_land(
        &self,
        _task_or_change_ref: &str,
        _payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err("Atomic Task Land is unavailable for this workflow backend.".to_string())
    }

    fn submit_land(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String>;

    fn get_land(&self, repo_name: Option<&str>, submission_id: &str) -> Result<JsonValue, String>;
}

pub trait ServerWorkflowStore:
    ServerWorkflowTaskStore
    + ServerWorkflowChangeStore
    + ServerWorkflowReviewStore
    + ServerWorkflowPatchsetStore
    + ServerWorkflowAttestationStore
    + ServerWorkflowPolicyStore
    + ServerWorkflowLandStore
    + Any
{
    fn as_any(&self) -> &dyn Any;
}

impl<T> ServerWorkflowStore for T
where
    T: ServerWorkflowTaskStore
        + ServerWorkflowChangeStore
        + ServerWorkflowReviewStore
        + ServerWorkflowPatchsetStore
        + ServerWorkflowAttestationStore
        + ServerWorkflowPolicyStore
        + ServerWorkflowLandStore
        + Any,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub trait ServerWorkflowRepository: ServerWorkflowStore {}

impl<T> ServerWorkflowRepository for T where T: ServerWorkflowStore + ?Sized {}

#[cfg(test)]
mod tests {
    use super::{
        patchset_ci_trigger_requests_new_run, resolve_server_workflow_backend,
        ServerWorkflowBackend,
    };

    #[test]
    fn patchset_ci_only_allocates_a_new_terminal_run_for_explicit_reruns() {
        for trigger in [None, Some(""), Some("manual_rerun")] {
            assert!(patchset_ci_trigger_requests_new_run(trigger));
        }
        assert!(patchset_ci_trigger_requests_new_run(Some(
            "base_stale_after_land_rerun"
        )));
        for trigger in [
            "workflow_ready_apply",
            "patchset_select",
            "existing_active",
            "unknown_ensure_trigger",
        ] {
            assert!(!patchset_ci_trigger_requests_new_run(Some(trigger)));
        }
    }

    #[test]
    fn workflow_backend_defaults_to_binary() {
        assert_eq!(
            resolve_server_workflow_backend(None),
            Ok(ServerWorkflowBackend::Binary)
        );
        assert_eq!(
            resolve_server_workflow_backend(Some("")),
            Ok(ServerWorkflowBackend::Binary)
        );
    }

    #[test]
    fn workflow_backend_rejects_postgres_and_shadow_escape_paths() {
        for backend in ["postgres", "postgres_binary_shadow", "binary_read_shadow"] {
            let error = resolve_server_workflow_backend(Some(backend))
                .expect_err("non-Binary workflow backend must fail closed");
            assert!(error.contains("supports only the registry-backed binary"));
        }
    }

    #[test]
    fn workflow_backend_accepts_current_binary_authority() {
        assert_eq!(
            resolve_server_workflow_backend(Some("binary")),
            Ok(ServerWorkflowBackend::Binary)
        );
    }

    #[test]
    fn workflow_backend_rejects_unknown_backend() {
        let err = resolve_server_workflow_backend(Some("local-file"))
            .expect_err("unknown backend should fail closed");
        assert!(err.contains("Unsupported AIT_NATIVE_SERVER_WORKFLOW_BACKEND"));
    }
}
