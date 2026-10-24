//! End-to-end `ait plan` execution stays concrete because it still owns
//! plan-specific command orchestration while delegating local/remote reads to
//! narrow data-source ports.

mod data_source;
mod local_shadow_ports;
mod local_source;
mod remote_source;

#[cfg(test)]
mod tests;

use self::data_source::{
    candidate_inputs_with_plan_command_data_source,
    get_plan_revision_with_plan_command_data_source, get_plan_with_plan_command_data_source,
    list_plan_revisions_with_plan_command_data_source, list_plans_with_plan_command_data_source,
    list_tasks_with_plan_command_data_source, PlanCommandCandidateInputSource,
    PlanCommandCandidateInputs, PlanCommandInspectSource, PlanCommandPlanLister,
    PlanCommandPlanRevisionReader, PlanCommandRevisionLister,
};
use self::local_shadow_ports::{
    local_shadow_for_plan_with_plan_command_local_shadow_source,
    local_shadow_index_with_plan_command_local_shadow_source, PlanCommandLocalShadowSource,
};
use self::local_source::{local_state_scope_from_text, LocalBinaryPlanCommandSource};
use self::remote_source::RemotePlanCommandSource;
use crate::binary_db::{AuthorityId, LocalBinaryDbFs, StorePath};
use crate::json_support::JsonCodec;
use crate::json_support::{json, JsonMap, JsonValue};
use crate::plan_command::{
    build_plan_candidates_command_payload_json, build_plan_inspect_command_payload_json,
    build_plan_items_command_payload_json, build_plan_list_command_payload_json,
    build_plan_revisions_command_payload_json, build_plan_show_command_payload_json,
};
use crate::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use crate::server_operational::RepositoryIndex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct PlanCommandExecutionJson<S> {
    store: S,
}

impl<S> PlanCommandExecutionJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl PlanCommandExecutionJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> PlanCommandExecutionJson<S> {
    pub fn execute_plan_list_command_request_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let request =
            self.parse_object_payload(payload_json, "plan list command execution request")?;
        execute_plan_list_command_request_map(request)
    }

    pub fn execute_plan_show_command_request_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let request =
            self.parse_object_payload(payload_json, "plan show command execution request")?;
        execute_plan_show_command_request_map(request)
    }

    pub fn execute_plan_revisions_command_request_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let request =
            self.parse_object_payload(payload_json, "plan revisions command execution request")?;
        execute_plan_revisions_command_request_map(request)
    }

    pub fn execute_plan_items_command_request_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let request =
            self.parse_object_payload(payload_json, "plan items command execution request")?;
        execute_plan_items_command_request_map(request)
    }

    pub fn execute_plan_candidates_command_request_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let request =
            self.parse_object_payload(payload_json, "plan candidates command execution request")?;
        execute_plan_candidates_command_request_map(request)
    }

    pub fn execute_plan_inspect_command_request_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let request =
            self.parse_object_payload(payload_json, "plan inspect command execution request")?;
        execute_plan_inspect_command_request_map(request)
    }

    fn parse_object_payload(
        &self,
        payload_json: &str,
        label: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        JsonCodec::parse_object_with_error_prefix(
            payload_json,
            &format!("{label} must be valid JSON"),
            &format!("{label} must be a JSON object."),
        )
        .map_err(String::from)
    }
}

pub fn execute_plan_list_command_request_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanCommandExecutionJson::stateless().execute_plan_list_command_request_json(payload_json)
}

fn execute_plan_list_command_request_map(
    request: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let scope = require_scope(&request)?;
    let repo_name = require_nonempty_text(request.get("repo_name"), "repo_name")?;
    let remote = optional_text(request.get("remote"))?;
    let remote_payload = command_remote_payload(scope.as_str(), remote);
    with_plan_command_plan_lister(scope.as_str(), &request, |source| {
        execute_plan_list_from_source(
            source,
            scope.as_str(),
            repo_name.as_str(),
            remote_payload.clone(),
        )
    })
}

pub fn execute_plan_show_command_request_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanCommandExecutionJson::stateless().execute_plan_show_command_request_json(payload_json)
}

fn execute_plan_show_command_request_map(
    request: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let scope = require_scope(&request)?;
    let default_repo_name = require_nonempty_text(request.get("repo_name"), "repo_name")?;
    let plan_id = require_nonempty_text(request.get("plan_id"), "plan_id")?;
    let revision_id = optional_text(request.get("revision"))?;
    let remote = optional_text(request.get("remote"))?;
    let remote_payload = command_remote_payload(scope.as_str(), remote);
    with_plan_command_plan_revision_reader(scope.as_str(), &request, |source| {
        execute_plan_show_from_source(
            source,
            scope.as_str(),
            default_repo_name.as_str(),
            plan_id.as_str(),
            revision_id.as_deref(),
            remote_payload.clone(),
        )
    })
}

pub fn execute_plan_revisions_command_request_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanCommandExecutionJson::stateless().execute_plan_revisions_command_request_json(payload_json)
}

fn execute_plan_revisions_command_request_map(
    request: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let scope = require_scope(&request)?;
    let default_repo_name = require_nonempty_text(request.get("repo_name"), "repo_name")?;
    let plan_id = require_nonempty_text(request.get("plan_id"), "plan_id")?;
    let remote = optional_text(request.get("remote"))?;
    let remote_payload = command_remote_payload(scope.as_str(), remote);
    with_plan_command_revision_lister(scope.as_str(), &request, |source| {
        execute_plan_revisions_from_source(
            source,
            scope.as_str(),
            default_repo_name.as_str(),
            plan_id.as_str(),
            remote_payload.clone(),
        )
    })
}

pub fn execute_plan_items_command_request_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanCommandExecutionJson::stateless().execute_plan_items_command_request_json(payload_json)
}

fn execute_plan_items_command_request_map(
    request: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let scope = require_scope(&request)?;
    let default_repo_name = require_nonempty_text(request.get("repo_name"), "repo_name")?;
    let plan_id = require_nonempty_text(request.get("plan_id"), "plan_id")?;
    let revision_id = optional_text(request.get("revision"))?;
    let remote = optional_text(request.get("remote"))?;
    let remote_payload = command_remote_payload(scope.as_str(), remote);
    with_plan_command_plan_revision_reader(scope.as_str(), &request, |source| {
        execute_plan_items_from_source(
            source,
            scope.as_str(),
            default_repo_name.as_str(),
            plan_id.as_str(),
            revision_id.as_deref(),
            remote_payload.clone(),
        )
    })
}

pub fn execute_plan_candidates_command_request_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanCommandExecutionJson::stateless().execute_plan_candidates_command_request_json(payload_json)
}

fn execute_plan_candidates_command_request_map(
    request: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let scope = require_scope(&request)?;
    let repo_name = require_nonempty_text(request.get("repo_name"), "repo_name")?;
    let remote = optional_text(request.get("remote"))?;
    let include_all = optional_bool(request.get("include_all"), false)?;
    let contains_terms = optional_text_list(request.get("contains_terms"))?;
    let remote_payload = command_remote_payload(scope.as_str(), remote);
    let local_shadow_index = if scope == "remote" {
        with_plan_command_local_shadow_source(&request, |source| {
            local_shadow_index_with_plan_command_local_shadow_source(source)
        })?
    } else {
        JsonMap::new()
    };
    with_plan_command_candidate_input_source(scope.as_str(), &request, |source| {
        execute_plan_candidates_from_source(
            source,
            scope.as_str(),
            repo_name.as_str(),
            remote_payload.clone(),
            include_all,
            &contains_terms,
            local_shadow_index.clone(),
        )
    })
}

pub fn execute_plan_inspect_command_request_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanCommandExecutionJson::stateless().execute_plan_inspect_command_request_json(payload_json)
}

fn execute_plan_inspect_command_request_map(
    request: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let scope = require_scope(&request)?;
    let default_repo_name = require_nonempty_text(request.get("repo_name"), "repo_name")?;
    let plan_id = require_nonempty_text(request.get("plan_id"), "plan_id")?;
    let revision_id = optional_text(request.get("revision"))?;
    let remote = optional_text(request.get("remote"))?;
    let remote_payload = command_remote_payload(scope.as_str(), remote);
    let local_shadow = if scope == "remote" {
        with_plan_command_local_shadow_source(&request, |source| {
            local_shadow_for_plan_with_plan_command_local_shadow_source(source, plan_id.as_str())
        })?
    } else {
        JsonValue::Null
    };
    with_plan_command_inspect_source(scope.as_str(), &request, |source| {
        execute_plan_inspect_from_source(
            source,
            scope.as_str(),
            default_repo_name.as_str(),
            plan_id.as_str(),
            revision_id.as_deref(),
            remote_payload.clone(),
            local_shadow.clone(),
        )
    })
}

macro_rules! define_plan_command_source_port {
    ($name:ident, $port:ty) => {
        fn $name<T>(
            scope: &str,
            request: &JsonMap<String, JsonValue>,
            action: impl FnOnce(&mut $port) -> Result<T, String>,
        ) -> Result<T, String> {
            match scope {
                "local" => with_local_plan_command_source_port!(request, $port, action),
                "remote" => {
                    let manager = build_http_client_manager(request)?;
                    let mut source = RemotePlanCommandSource::new(manager);
                    let source_port: &mut $port = &mut source;
                    action(source_port)
                }
                _ => Err("Plan command execution scope must be local or remote.".to_string()),
            }
        }
    };
}

macro_rules! with_local_plan_command_source_port {
    ($request:expr, $port:ty, $action:expr) => {{
        let config = local_plan_binary_storage($request)?;
        match config.write_layout {
            1 => {
                let repo_name = config.repo_name.clone();
                let mut source = LocalBinaryPlanCommandSource::<1>::from_db(
                    repo_name,
                    config.binary_db()?,
                );
                let source_port: &mut $port = &mut source;
                $action(source_port)
            }
            other => Err(format!(
                "Unsupported Plan Binary DB write_layout `{other}` for command reads; this build supports layout 1."
            )),
        }
    }};
}

define_plan_command_source_port!(with_plan_command_plan_lister, dyn PlanCommandPlanLister);
define_plan_command_source_port!(
    with_plan_command_plan_revision_reader,
    dyn PlanCommandPlanRevisionReader
);
define_plan_command_source_port!(
    with_plan_command_revision_lister,
    dyn PlanCommandRevisionLister
);
define_plan_command_source_port!(
    with_plan_command_candidate_input_source,
    dyn PlanCommandCandidateInputSource
);
define_plan_command_source_port!(
    with_plan_command_inspect_source,
    dyn PlanCommandInspectSource
);

fn with_plan_command_local_shadow_source<T>(
    request: &JsonMap<String, JsonValue>,
    action: impl FnOnce(&mut dyn PlanCommandLocalShadowSource) -> Result<T, String>,
) -> Result<T, String> {
    with_local_plan_command_source_port!(request, dyn PlanCommandLocalShadowSource, action)
}

#[derive(Clone, Debug)]
struct LocalPlanBinaryStorageConfig {
    write_layout: u32,
    repo_name: String,
    authority_root: String,
    activation_pointer: Option<String>,
    repo_root: String,
    local_authority_id: String,
    current_line_state_scope: crate::binary_db::LocalStateScope,
}

impl LocalPlanBinaryStorageConfig {
    fn binary_db(&self) -> Result<LocalBinaryDbFs, String> {
        if let Some(pointer) = self.activation_pointer.as_deref() {
            let (generation, guard) =
                crate::binary_db_generation::admit_activated_binary_db_generation_for_runtime(
                    Path::new(&self.repo_root),
                    Path::new(pointer),
                    &self.repo_name,
                )?;
            if generation.authority_root != self.authority_root {
                return Err(
                    "plan_storage.authority_root does not match the admitted activation pointer."
                        .to_string(),
                );
            }
            return Ok(LocalBinaryDbFs::new(
                StorePath::from(generation.authority_root),
                StorePath::from(PathBuf::from(&self.repo_root)),
                AuthorityId::new(self.local_authority_id.clone()),
                self.current_line_state_scope,
            )
            .with_declared_bin_paths(crate::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS)
            .with_declared_index_paths(crate::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS)
            .with_generation_guard(Some(guard)));
        }
        if !cfg!(test) {
            return Err(
                "plan_storage.activation_pointer is required for selected Binary DB plan commands."
                    .to_string(),
            );
        }
        Ok(LocalBinaryDbFs::new(
            StorePath::from(PathBuf::from(&self.authority_root)),
            StorePath::from(PathBuf::from(&self.repo_root)),
            AuthorityId::new(self.local_authority_id.clone()),
            self.current_line_state_scope,
        )
        .with_declared_bin_paths(crate::binary_db::REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(crate::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS))
    }
}

fn local_plan_binary_storage(
    request: &JsonMap<String, JsonValue>,
) -> Result<LocalPlanBinaryStorageConfig, String> {
    let storage = request.get("plan_storage").ok_or_else(|| {
        "Plan command execution requires a Binary DB `plan_storage` object.".to_string()
    })?;
    let storage = storage.as_object().ok_or_else(|| {
        "Plan command execution field `plan_storage` must be an object.".to_string()
    })?;
    const FIELDS: &[&str] = &[
        "write_layout",
        "repo_name",
        "authority_root",
        "activation_pointer",
        "pack_root",
        "repo_root",
        "local_authority_id",
        "current_line_state_scope",
    ];
    if let Some(field) = storage
        .keys()
        .find(|field| !FIELDS.contains(&field.as_str()))
    {
        return Err(format!(
            "Plan command execution does not support plan_storage field `{field}`."
        ));
    }
    local_plan_binary_storage_config(request, storage)
}

fn local_plan_binary_storage_config(
    request: &JsonMap<String, JsonValue>,
    storage: &JsonMap<String, JsonValue>,
) -> Result<LocalPlanBinaryStorageConfig, String> {
    let write_layout = require_local_plan_write_layout(storage)?;
    let repo_name = require_nonempty_text(request.get("repo_name"), "repo_name")?;
    let authority_root =
        require_nonempty_text(storage.get("authority_root"), "plan_storage.authority_root")?;
    let activation_pointer = optional_text(storage.get("activation_pointer"))?;
    let repo_root = require_nonempty_text(storage.get("repo_root"), "plan_storage.repo_root")?;
    let local_authority_id = require_nonempty_text(
        storage.get("local_authority_id"),
        "plan_storage.local_authority_id",
    )?;
    let current_line_state_scope = require_nonempty_text(
        storage.get("current_line_state_scope"),
        "plan_storage.current_line_state_scope",
    )?;
    Ok(LocalPlanBinaryStorageConfig {
        write_layout,
        repo_name,
        authority_root,
        activation_pointer,
        repo_root,
        local_authority_id,
        current_line_state_scope: local_state_scope_from_text(current_line_state_scope.as_str())?,
    })
}

fn require_local_plan_write_layout(storage: &JsonMap<String, JsonValue>) -> Result<u32, String> {
    let layout = optional_u64(storage.get("write_layout"))?.ok_or_else(|| {
        "Plan Binary DB command requests must include numeric `plan_storage.write_layout`."
            .to_string()
    })?;
    if layout > u64::from(u32::MAX) {
        return Err(
            "Plan Binary DB command request `plan_storage.write_layout` must fit in u32."
                .to_string(),
        );
    }
    Ok(layout as u32)
}

fn execute_plan_list_from_source<S>(
    source: &mut S,
    scope: &str,
    repo_name: &str,
    remote: JsonValue,
) -> Result<JsonValue, String>
where
    S: PlanCommandPlanLister + ?Sized,
{
    let plans = list_plans_with_plan_command_data_source(source, repo_name)?;
    build_plan_list_command_payload_json(
        &json!({
            "scope": scope,
            "repo_name": repo_name,
            "remote": remote,
            "plans": plans,
        })
        .to_string(),
    )
}

fn execute_plan_show_from_source<S>(
    source: &mut S,
    scope: &str,
    default_repo_name: &str,
    plan_id: &str,
    revision_id: Option<&str>,
    remote: JsonValue,
) -> Result<JsonValue, String>
where
    S: PlanCommandPlanRevisionReader + ?Sized,
{
    let plan = get_plan_with_plan_command_data_source(source, plan_id)?;
    let revision = revision_id
        .map(|value| get_plan_revision_with_plan_command_data_source(source, plan_id, value))
        .transpose()?;
    let repo_name = if scope == "remote" {
        repo_name_from_plan(&plan, default_repo_name)
    } else {
        default_repo_name.to_string()
    };
    build_plan_show_command_payload_json(
        &json!({
            "scope": scope,
            "repo_name": repo_name,
            "remote": remote,
            "plan": plan,
            "revision": revision,
        })
        .to_string(),
    )
}

fn execute_plan_revisions_from_source<S>(
    source: &mut S,
    scope: &str,
    default_repo_name: &str,
    plan_id: &str,
    remote: JsonValue,
) -> Result<JsonValue, String>
where
    S: PlanCommandRevisionLister + ?Sized,
{
    let revisions = list_plan_revisions_with_plan_command_data_source(source, plan_id)?;
    let repo_name = if scope == "remote" {
        repo_name_from_first_revision(&revisions, default_repo_name)
    } else {
        default_repo_name.to_string()
    };
    build_plan_revisions_command_payload_json(
        &json!({
            "scope": scope,
            "repo_name": repo_name,
            "remote": remote,
            "plan_id": plan_id,
            "revisions": revisions,
        })
        .to_string(),
    )
}

fn execute_plan_items_from_source<S>(
    source: &mut S,
    scope: &str,
    default_repo_name: &str,
    plan_id: &str,
    revision_id: Option<&str>,
    remote: JsonValue,
) -> Result<JsonValue, String>
where
    S: PlanCommandPlanRevisionReader + ?Sized,
{
    let plan = get_plan_with_plan_command_data_source(source, plan_id)?;
    let revision = revision_id
        .map(|value| get_plan_revision_with_plan_command_data_source(source, plan_id, value))
        .transpose()?;
    let repo_name = if scope == "remote" {
        repo_name_from_plan(&plan, default_repo_name)
    } else {
        default_repo_name.to_string()
    };
    build_plan_items_command_payload_json(
        &json!({
            "scope": scope,
            "repo_name": repo_name,
            "remote": remote,
            "plan": plan,
            "revision": revision,
        })
        .to_string(),
    )
}

fn execute_plan_candidates_from_source<S>(
    source: &mut S,
    scope: &str,
    repo_name: &str,
    remote: JsonValue,
    include_all: bool,
    contains_terms: &[String],
    local_shadow_index: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String>
where
    S: PlanCommandCandidateInputSource + ?Sized,
{
    let PlanCommandCandidateInputs { plans, tasks } =
        candidate_inputs_with_plan_command_data_source(source, repo_name, contains_terms)?;
    build_plan_candidates_command_payload_json(
        &json!({
            "scope": scope,
            "repo_name": repo_name,
            "remote": remote,
            "plans": plans,
            "tasks": tasks,
            "include_all": include_all,
            "contains_terms": contains_terms,
            "local_shadow_index": local_shadow_index,
        })
        .to_string(),
    )
}

fn execute_plan_inspect_from_source<S>(
    source: &mut S,
    scope: &str,
    default_repo_name: &str,
    plan_id: &str,
    revision_id: Option<&str>,
    remote: JsonValue,
    local_shadow: JsonValue,
) -> Result<JsonValue, String>
where
    S: PlanCommandInspectSource + ?Sized,
{
    let plan = get_plan_with_plan_command_data_source(source, plan_id)?;
    let revision = revision_id
        .map(|value| get_plan_revision_with_plan_command_data_source(source, plan_id, value))
        .transpose()?;
    let tasks = list_tasks_with_plan_command_data_source(source, default_repo_name)?;
    let repo_name = if scope == "remote" {
        repo_name_from_plan(&plan, default_repo_name)
    } else {
        default_repo_name.to_string()
    };
    build_plan_inspect_command_payload_json(
        &json!({
            "scope": scope,
            "repo_name": repo_name,
            "remote": remote,
            "plan": plan,
            "revision": revision,
            "tasks": tasks,
            "local_shadow": local_shadow,
        })
        .to_string(),
    )
}

fn command_remote_payload(scope: &str, remote: Option<String>) -> JsonValue {
    if scope == "remote" {
        remote.map(JsonValue::String).unwrap_or(JsonValue::Null)
    } else {
        JsonValue::Null
    }
}

pub(super) fn local_plan_publish_shadow_from_plan(
    plan: &JsonValue,
) -> Result<Option<JsonValue>, String> {
    let Some(plan_object) = plan.as_object() else {
        return Ok(None);
    };
    let head_revision = plan_object
        .get("head_revision")
        .and_then(|value| value.as_object());
    let head_publication_state = head_revision
        .and_then(|value| value.get("publication_state"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let unpublished_head = match &head_publication_state {
        JsonValue::String(value) => !value.trim().eq_ignore_ascii_case("published"),
        JsonValue::Null => false,
        _ => true,
    };
    let head_revision_id = match optional_text(plan_object.get("head_revision_id"))? {
        Some(value) => JsonValue::String(value),
        None => head_revision
            .and_then(|value| value.get("plan_revision_id"))
            .cloned()
            .unwrap_or(JsonValue::Null),
    };
    Ok(Some(JsonValue::Object(JsonMap::from_iter([
        (
            "plan_id".to_string(),
            plan_object
                .get("plan_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "publication_state".to_string(),
            plan_object
                .get("publication_state")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        ("head_publication_state".to_string(), head_publication_state),
        ("head_revision_id".to_string(), head_revision_id),
        (
            "head_revision_number".to_string(),
            head_revision
                .and_then(|value| value.get("revision_number"))
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "published_plan_id".to_string(),
            plan_object
                .get("published_plan_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "published_head_revision_id".to_string(),
            plan_object
                .get("published_head_revision_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
        (
            "unpublished_head".to_string(),
            JsonValue::Bool(unpublished_head),
        ),
    ]))))
}

fn build_http_client_manager(
    request: &JsonMap<String, JsonValue>,
) -> Result<PlanHttpClientManager, String> {
    let base_url = require_nonempty_text(request.get("base_url"), "base_url")?;
    let headers = parse_header_map(request.get("headers"))?;
    let timeout_ms = optional_u64(request.get("timeout_ms"))?.unwrap_or(DEFAULT_TIMEOUT_MS);
    let retry_attempts = optional_u64(request.get("retry_attempts"))?.unwrap_or(0) as usize;
    let retry_backoff_ms = optional_u64(request.get("retry_backoff_ms"))?.unwrap_or(0);
    let pool_max_idle_per_host =
        optional_u64(request.get("pool_max_idle_per_host"))?.unwrap_or(1) as usize;
    PlanHttpClientManager::new(PlanHttpClientConfig {
        base_url,
        repository_index: optional_u64(request.get("repository_index"))?
            .map(|value| {
                u32::try_from(value).map(RepositoryIndex::new).map_err(|_| {
                    format!("repository_index must fit an unsigned 32-bit integer: {value}")
                })
            })
            .transpose()?,
        headers,
        default_timeout_ms: timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    })
    .map_err(|err| err.to_string())
}

fn parse_header_map(value: Option<&JsonValue>) -> Result<BTreeMap<String, String>, String> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    if value.is_null() {
        return Ok(BTreeMap::new());
    }
    let headers = value
        .as_object()
        .ok_or_else(|| "plan command execution headers must be an object.".to_string())?;
    let mut normalized = BTreeMap::new();
    for (key, entry) in headers {
        let text = entry
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                "plan command execution headers values must be non-empty strings.".to_string()
            })?;
        normalized.insert(key.to_string(), text.to_string());
    }
    Ok(normalized)
}

fn require_scope(payload: &JsonMap<String, JsonValue>) -> Result<String, String> {
    let scope = require_nonempty_text(payload.get("scope"), "scope")?;
    match scope.as_str() {
        "local" | "remote" => Ok(scope),
        _ => Err("Plan command execution scope must be `local` or `remote`.".to_string()),
    }
}

fn require_nonempty_text(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    optional_text(value)?.ok_or_else(|| {
        format!("Plan command execution field `{field_name}` must be a non-empty string.")
    })
}

pub(super) fn optional_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let text = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "Plan command execution text fields must be strings.".to_string())?;
    Ok(Some(text.to_string()))
}

fn optional_text_list(value: Option<&JsonValue>) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        JsonValue::Null => Ok(Vec::new()),
        JsonValue::Array(entries) => {
            let mut normalized = Vec::new();
            for entry in entries {
                let Some(text) = optional_text(Some(entry))? else {
                    return Err(
                        "Plan command execution text-list fields must contain non-empty strings."
                            .to_string(),
                    );
                };
                if !normalized.contains(&text) {
                    normalized.push(text);
                }
            }
            Ok(normalized)
        }
        _ => Err("Plan command execution text-list fields must be lists.".to_string()),
    }
}

fn optional_bool(value: Option<&JsonValue>, default: bool) -> Result<bool, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(default),
        Some(JsonValue::Bool(flag)) => Ok(*flag),
        Some(_) => Err("Plan command execution boolean fields must be booleans.".to_string()),
    }
}

fn optional_u64(value: Option<&JsonValue>) -> Result<Option<u64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value.as_u64().map(Some).ok_or_else(|| {
        "Plan command execution numeric fields must be unsigned integers.".to_string()
    })
}

fn repo_name_from_plan(plan: &JsonValue, fallback: &str) -> String {
    value_get(plan, "repo_name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn repo_name_from_first_revision(revisions: &[JsonValue], fallback: &str) -> String {
    revisions
        .first()
        .and_then(|row| value_get(row, "repo_name"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

pub(super) fn value_get<'a>(value: &'a JsonValue, key: &str) -> Option<&'a JsonValue> {
    value.as_object().and_then(|object| object.get(key))
}
