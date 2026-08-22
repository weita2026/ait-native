use super::*;

#[derive(Debug, Deserialize)]
pub(in super::super) struct NativeTaskAuditQuery {
    target_line: Option<String>,
}

pub(super) async fn run_workflow_call<F>(
    context: &'static str,
    callback: F,
) -> Result<(StatusCode, Json<JsonValue>), ApiError>
where
    F: FnOnce() -> Result<JsonValue, String> + Send + 'static,
{
    let result = task::spawn_blocking(callback)
        .await
        .map_err(|exc| ApiError::internal(format!("{context} worker failed: {exc}")))?;
    map_json_result(result)
}

pub(super) async fn run_workflow_mutation<F>(
    context: &'static str,
    runtime: Arc<dyn ServerRuntimeService>,
    callback: F,
) -> Result<(StatusCode, Json<JsonValue>), ApiError>
where
    F: FnOnce() -> Result<JsonValue, String> + Send + 'static,
{
    run_workflow_call(context, move || {
        #[cfg(feature = "perfetto-tracing")]
        let _trace = ait_server_core::perfetto_trace::PerfettoRange::new(context);
        let result = callback()?;
        runtime.request_queue_read_models_refresh(workflow_mutation_repo_name(&result));
        Ok(result)
    })
    .await
}

fn workflow_mutation_repo_name(result: &JsonValue) -> Option<&str> {
    const REPO_NAME_POINTERS: &[&str] = &[
        "/repo_name",
        "/task/repo_name",
        "/change/repo_name",
        "/patchset/repo_name",
        "/review/repo_name",
        "/attestation/repo_name",
        "/policy/repo_name",
        "/land/repo_name",
        "/job/repo_name",
        "/job/payload/repo_name",
    ];
    REPO_NAME_POINTERS.iter().find_map(|pointer| {
        result
            .pointer(pointer)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

pub(in super::super) async fn native_start_plan_bound_task(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_mutation(
        "native atomic task start",
        state.runtime_service.clone(),
        move || {
            let (repo_name, workflow) =
                repository_authority_workflow_store(routed_workflow.as_ref(), &repository_index)?;
            workflow.start_plan_bound_task(&repo_name, &payload)
        },
    )
    .await
}

pub(in super::super) async fn native_create_task(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    let runtime = state.runtime_service.clone();
    let linkage_runtime = runtime.clone();
    run_workflow_mutation("native task create", runtime, move || {
        let (repo_name, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repository_index)?;
        let payload = resolve_native_task_plan_linkage_payload(
            linkage_runtime.as_ref(),
            &repository_index,
            payload,
        )?;
        workflow.create_task(&repo_name, &payload)
    })
    .await
}

pub(super) fn resolve_native_task_plan_linkage_payload(
    runtime: &dyn ServerRuntimeService,
    repository_index: &str,
    payload: JsonValue,
) -> Result<JsonValue, String> {
    let mut object = payload
        .as_object()
        .cloned()
        .ok_or_else(|| "task create payload must be a JSON object.".to_string())?;
    for retired_field in ["repo_id", "repository_index"] {
        if object.contains_key(retired_field) {
            return Err(format!(
                "task create payload must not contain {retired_field}; Repository authority comes from the numeric route"
            ));
        }
    }
    let mut plan_id = json_optional_text(&object, "plan_id");
    let mut revision_id = json_optional_text(&object, "origin_plan_revision_id");
    let plan_item_ref = json_optional_text(&object, "plan_item_ref");
    if plan_id.is_none() && revision_id.is_none() {
        if plan_item_ref.is_some() {
            return Err("plan_item_ref requires plan linkage".to_string());
        }
        return Ok(JsonValue::Object(object));
    }

    let mut plan = None;
    if let Some(plan_id) = plan_id.as_ref() {
        let fetched = get_plan_for_repository(runtime, Some(repository_index), plan_id)?;
        plan = Some(fetched);
    }

    let mut revision = None;
    if let Some(current_revision_id) = revision_id.as_ref() {
        if let Some(current_plan_id) = plan_id.as_ref() {
            revision = Some(get_plan_revision_for_repository(
                runtime,
                Some(repository_index),
                current_plan_id,
                current_revision_id,
            )?);
        } else {
            let (resolved_plan_id, fetched_revision) =
                find_revision_in_repo(runtime, repository_index, current_revision_id)?;
            plan_id = Some(resolved_plan_id);
            revision = Some(fetched_revision);
        }
    } else if let (Some(current_plan_id), Some(current_plan)) = (plan_id.as_ref(), plan.as_ref()) {
        let head_revision_id = head_revision_id(current_plan)
            .ok_or_else(|| format!("Plan {current_plan_id} has no head revision to link from"))?;
        revision = Some(get_plan_revision_for_repository(
            runtime,
            Some(repository_index),
            current_plan_id,
            &head_revision_id,
        )?);
        revision_id = Some(head_revision_id);
    }

    if let (Some(current_revision), Some(current_plan_id)) = (revision.as_ref(), plan_id.as_ref()) {
        if let Some(revision_plan_id) = json_value_optional_text(current_revision.get("plan_id")) {
            if revision_plan_id != *current_plan_id {
                let current_revision_id = revision_id.as_deref().unwrap_or("<unknown>");
                return Err(format!(
                    "Plan revision {current_revision_id} does not belong to plan {current_plan_id}"
                ));
            }
        }
    }

    if let Some(current_ref) = plan_item_ref.as_ref() {
        let current_revision = revision.as_ref().ok_or_else(|| {
            format!(
                "Unknown plan revision: {}",
                revision_id.as_deref().unwrap_or("")
            )
        })?;
        let items = revision_items(current_revision);
        if !plan_item_exists(&items, current_ref) {
            let known_refs = known_plan_item_refs(&items);
            let current_revision_id = revision_id.as_deref().unwrap_or("<unknown>");
            if known_refs.is_empty() {
                return Err(format!(
                    "Plan revision {current_revision_id} does not expose any explicit `[ref: ...]` plan items yet. Add refs to the file-backed plan section before binding a task to one."
                ));
            }
            return Err(format!(
                "Plan item ref {current_ref:?} is not present in plan revision {current_revision_id}. Known refs: {}",
                known_refs.join(", ")
            ));
        }
    }

    object.insert("plan_id".to_string(), json!(plan_id));
    object.insert("origin_plan_revision_id".to_string(), json!(revision_id));
    object.insert("plan_item_ref".to_string(), json!(plan_item_ref));
    Ok(JsonValue::Object(object))
}

pub(super) fn json_optional_text(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Option<String> {
    json_value_optional_text(object.get(field))
}

pub(super) fn json_value_optional_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    let text = match value {
        JsonValue::String(raw) => raw.trim().to_string(),
        _ => value.to_string().trim().to_string(),
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn get_plan_for_repository(
    runtime: &dyn ServerRuntimeService,
    repository_index: Option<&str>,
    plan_id: &str,
) -> Result<JsonValue, String> {
    match repository_index {
        Some(repository_index) => runtime.get_repository_plan(repository_index, plan_id),
        None => runtime.get_plan(plan_id),
    }
}

fn get_plan_revision_for_repository(
    runtime: &dyn ServerRuntimeService,
    repository_index: Option<&str>,
    plan_id: &str,
    revision_id: &str,
) -> Result<JsonValue, String> {
    match repository_index {
        Some(repository_index) => {
            runtime.get_repository_plan_revision(repository_index, plan_id, revision_id)
        }
        None => runtime.get_plan_revision(plan_id, revision_id),
    }
}

pub(super) fn head_revision_id(plan: &JsonValue) -> Option<String> {
    let object = plan.as_object()?;
    object
        .get("head_revision")
        .and_then(JsonValue::as_object)
        .and_then(|head| json_optional_text(head, "plan_revision_id"))
        .or_else(|| json_optional_text(object, "head_revision_id"))
}

pub(super) fn find_revision_in_repo(
    runtime: &dyn ServerRuntimeService,
    repository_index: &str,
    revision_id: &str,
) -> Result<(String, JsonValue), String> {
    let plans = runtime.list_repository_plans(repository_index, None)?;
    let Some(plan_rows) = plans.as_array() else {
        return Err("Rust plan runtime returned a non-list plan payload.".to_string());
    };
    for plan in plan_rows {
        let Some(plan_id) = plan
            .as_object()
            .and_then(|object| json_optional_text(object, "plan_id"))
        else {
            continue;
        };
        match get_plan_revision_for_repository(
            runtime,
            Some(repository_index),
            &plan_id,
            revision_id,
        ) {
            Ok(revision) => return Ok((plan_id, revision)),
            Err(message) if message.contains("Unknown plan revision") => continue,
            Err(message) => return Err(message),
        }
    }
    Err(format!("Unknown plan revision: {revision_id}"))
}

pub(super) fn revision_items(revision: &JsonValue) -> Vec<JsonValue> {
    if let Some(items) = revision.get("items").and_then(JsonValue::as_array) {
        return items.to_vec();
    }
    revision
        .get("items_json")
        .and_then(JsonValue::as_str)
        .and_then(|text| serde_json::from_str::<JsonValue>(text).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

pub(super) fn plan_item_exists(items: &[JsonValue], plan_item_ref: &str) -> bool {
    items
        .iter()
        .any(|item| plan_item_ref_in_value(item, plan_item_ref))
}

pub(super) fn plan_item_ref_in_value(value: &JsonValue, plan_item_ref: &str) -> bool {
    match value {
        JsonValue::Object(object) => {
            json_optional_text(object, "plan_item_ref").as_deref() == Some(plan_item_ref)
                || object
                    .values()
                    .any(|child| plan_item_ref_in_value(child, plan_item_ref))
        }
        JsonValue::Array(items) => items
            .iter()
            .any(|item| plan_item_ref_in_value(item, plan_item_ref)),
        _ => false,
    }
}

pub(super) fn known_plan_item_refs(items: &[JsonValue]) -> Vec<String> {
    let mut refs = BTreeSet::new();
    for item in items {
        collect_plan_item_refs(item, &mut refs);
    }
    refs.into_iter().collect()
}

pub(super) fn collect_plan_item_refs(value: &JsonValue, refs: &mut BTreeSet<String>) {
    match value {
        JsonValue::Object(object) => {
            if let Some(plan_item_ref) = json_optional_text(object, "plan_item_ref") {
                refs.insert(plan_item_ref);
            }
            for child in object.values() {
                collect_plan_item_refs(child, refs);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_plan_item_refs(item, refs);
            }
        }
        _ => {}
    }
}

pub(in super::super) async fn native_list_tasks(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native task list", move || {
        let (repo_name, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repository_index)?;
        workflow.list_tasks(&repo_name)
    })
    .await
}

pub(in super::super) async fn native_get_repository_authority_task(
    State(state): State<ServerState>,
    Path((repo_id, task_ref)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native repository authority task read", move || {
        let (_, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
        workflow.get_task(None, &task_ref)
    })
    .await
}

pub(in super::super) async fn native_repository_authority_task_action(
    State(state): State<ServerState>,
    Path((repo_id, task_tail)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let task_id = parse_suffixed_tail(&task_tail, ":close", "task action")?;
    let routed_workflow = state.workflow_service.clone();
    run_workflow_mutation(
        "native repository authority task close",
        state.runtime_service.clone(),
        move || {
            let (_, workflow) =
                repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
            workflow.close_task(&task_id, &payload)
        },
    )
    .await
}

pub(in super::super) async fn native_read_task_audit(
    State(state): State<ServerState>,
    Path((repository_index, task_ref)): Path<(String, String)>,
    Query(query): Query<NativeTaskAuditQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let target_line = query.target_line.unwrap_or_else(|| "main".to_string());
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native task audit", move || {
        let (repo_name, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repository_index)?;
        workflow.read_task_audit(&repo_name, &task_ref, &target_line)
    })
    .await
}
