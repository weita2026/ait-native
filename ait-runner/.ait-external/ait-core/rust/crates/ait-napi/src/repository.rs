use crate::JsonInput;
use ait_cli::runtime::RepoRuntime;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;
use std::path::{Path, PathBuf};

#[napi(object)]
#[derive(Default)]
pub struct InitOptions {
    pub name: Option<String>,
    pub default_line: Option<String>,
    pub policy_profile: Option<String>,
    pub default_author_mode: Option<String>,
    pub default_model: Option<String>,
    pub repair_existing: Option<bool>,
}

#[napi(object)]
#[derive(Default)]
pub struct ScopeOptions {
    pub local: Option<bool>,
    pub remote_name: Option<String>,
}

#[napi(object)]
#[derive(Default)]
pub struct TaskListOptions {
    pub local: Option<bool>,
    pub remote_name: Option<String>,
    pub all: Option<bool>,
}

pub(crate) fn repository(root: &str) -> Result<RepoRuntime> {
    RepoRuntime::discover_from_path(Path::new(root)).map_err(Error::from_reason)
}

#[napi]
pub fn repository_authority_root(root: String) -> Result<String> {
    Ok(repository(&root)?
        .authoritative_repo_root()
        .to_string_lossy()
        .into_owned())
}

#[napi]
pub fn doctor_plan_authority(root: String) -> Result<JsonValue> {
    ait_cli::binding_services::plan_authority_diagnostics(Path::new(&root))
        .map_err(Error::from_reason)
}

#[napi]
pub fn doctor_memory_root(root: String) -> Result<JsonValue> {
    ait_cli::doctor_surface::doctor_memory_root(&repository(&root)?).map_err(Error::from_reason)
}

#[napi]
pub fn queue_summary(root: String, remote_name: Option<String>) -> Result<JsonValue> {
    ait_cli::primitives::queue_summary(&repository(&root)?, remote_name.as_deref())
        .map_err(Error::from_reason)
}

pub(crate) fn init_request(
    root: String,
    options: Option<InitOptions>,
) -> ait_cli::init_surface::InitRequest {
    let options = options.unwrap_or_default();

    ait_cli::init_surface::InitRequest {
        root: PathBuf::from(root),
        name: options.name,
        default_line: options.default_line.unwrap_or_else(|| "main".into()),
        policy_profile: options.policy_profile.unwrap_or_else(|| "prototype".into()),
        default_author_mode: options
            .default_author_mode
            .unwrap_or_else(|| "ai_with_human_review".into()),
        default_model: options.default_model,
        repair_existing: options.repair_existing.unwrap_or(false),
    }
}

#[napi]
pub fn task_workflow_init(root: String, options: Option<InitOptions>) -> Result<JsonValue> {
    ait_cli::init_surface::init_repo(&init_request(root, options)).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_config_show(root: String) -> Result<JsonValue> {
    ait_cli::config_surface::config_show(&repository(&root)?).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_config_set(root: String, payload: JsonInput) -> Result<JsonValue> {
    ait_cli::config_surface::config_set_from_payload(&repository(&root)?, &payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_status(root: String) -> Result<JsonValue> {
    ait_cli::primitives::repo_status(&repository(&root)?).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_remote_add(root: String, payload: JsonInput) -> Result<JsonValue> {
    ait_cli::remote_surface::remote_add_from_payload(&repository(&root)?, &payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_remote_list(root: String) -> Result<JsonValue> {
    ait_cli::remote_surface::remote_list(&repository(&root)?).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_doctor_runtime_root(root: String) -> Result<JsonValue> {
    ait_cli::doctor_surface::doctor_runtime_root(Path::new(&root)).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_doctor_plan_authority() -> Result<JsonValue> {
    ait_cli::doctor_surface::doctor_plan_authority().map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_task_start(
    root: String,
    title: String,
    intent: String,
    options: Option<ScopeOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    ait_cli::primitives::task_start(
        &repository(&root)?,
        &title,
        &intent,
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_task_list(
    root: String,
    options: Option<TaskListOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let payload = ait_cli::primitives::task_list(
        &repository(&root)?,
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)?;
    match payload {
        JsonValue::Array(mut rows) => {
            if !options.all.unwrap_or(false) {
                rows.retain(|row| {
                    !matches!(
                        row.get("status").and_then(JsonValue::as_str),
                        Some("completed" | "abandoned" | "canceled" | "later_promotion_excluded")
                    )
                });
            }
            Ok(JsonValue::Array(rows))
        }
        _ => Err(Error::from_reason(
            "task list payload must decode to a JSON array.",
        )),
    }
}

#[napi]
pub fn task_workflow_task_show(
    root: String,
    task_id: String,
    options: Option<ScopeOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    ait_cli::primitives::task_show(
        &repository(&root)?,
        &task_id,
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_snapshot_create(root: String, message: Option<String>) -> Result<JsonValue> {
    ait_cli::primitives::snapshot_create(&repository(&root)?, message.as_deref())
        .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_snapshot_list(root: String) -> Result<JsonValue> {
    ait_cli::primitives::snapshot_list(&repository(&root)?).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_snapshot_show(root: String, snapshot_id: String) -> Result<JsonValue> {
    ait_cli::primitives::snapshot_show(&repository(&root)?, &snapshot_id)
        .map_err(Error::from_reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ait_core::json_support::json;

    #[test]
    fn direct_repository_services_preserve_domain_results_and_failures() {
        let directory = tempfile::tempdir().expect("isolated repository");
        let root = directory
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let initialized = task_workflow_init(root.clone(), None).expect("direct init");
        assert_eq!(initialized["action"], "initialized");
        let config = task_workflow_config_set(
            root.clone(),
            json!({"agent_teams":"disabled","sprint":"off","user_name":"binding fixture"}).into(),
        )
        .expect("direct config");
        assert_eq!(config["agent_teams"], false);
        let readback = task_workflow_config_show(root.clone()).unwrap();
        assert_eq!(readback["agent_teams"], false);
        assert_eq!(readback["user_name"], "binding fixture");
        assert_eq!(task_workflow_remote_list(root.clone()).unwrap(), json!([]));
        assert_eq!(
            task_workflow_task_list(root.clone(), None).unwrap(),
            json!([])
        );
        assert!(task_workflow_status(root.clone()).is_ok());
        let blocked =
            task_workflow_snapshot_create(root.clone(), Some("direct binding fixture".into()))
                .expect_err("main cannot bypass the native Task-worktree gate");
        assert!(blocked.reason.contains("active Task worktree"));
        assert_eq!(
            task_workflow_snapshot_list(root.clone()).unwrap(),
            json!([])
        );
        assert!(task_workflow_snapshot_show(root.clone(), "missing-snapshot".into()).is_err());
        let before = std::fs::read(directory.path().join(".ait/config.json")).unwrap();
        let rejected =
            task_workflow_config_set(root.clone(), json!({"workflow_mode":"solo_local"}).into())
                .expect_err("retired config fails in Rust");
        assert!(!rejected.reason.is_empty());
        assert_eq!(
            std::fs::read(directory.path().join(".ait/config.json")).unwrap(),
            before
        );
        let missing = task_workflow_task_show(root, "missing-task".into(), None)
            .expect_err("native Task error");
        assert!(!missing.reason.is_empty());
    }
}
