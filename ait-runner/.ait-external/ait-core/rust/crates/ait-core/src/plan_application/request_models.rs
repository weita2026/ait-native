use super::*;

pub fn normalize_plan_list_service_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_list_service_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_list_service_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let mut normalized = normalize_query_request(&payload, "plan list service request")?;
    normalized.insert(
        "plans".to_string(),
        JsonValue::Array(
            require_object_list(payload.get("plans"), "plans")?
                .into_iter()
                .map(JsonValue::Object)
                .collect(),
        ),
    );
    Ok(JsonValue::Object(normalized))
}

pub fn normalize_plan_show_service_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_show_service_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_show_service_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let mut normalized = normalize_query_request(&payload, "plan show service request")?;
    normalized.insert(
        "plan".to_string(),
        JsonValue::Object(
            require_object(payload.get("plan"), "plan show service request plan")?.clone(),
        ),
    );
    normalized.insert(
        "revision".to_string(),
        match payload.get("revision") {
            None | Some(JsonValue::Null) => JsonValue::Null,
            Some(value) => JsonValue::Object(
                require_object(Some(value), "plan show service request revision")?.clone(),
            ),
        },
    );
    Ok(JsonValue::Object(normalized))
}

pub fn normalize_plan_revisions_service_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless()
        .normalize_plan_revisions_service_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_revisions_service_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let mut normalized = normalize_query_request(&payload, "plan revisions service request")?;
    normalized.insert(
        "plan_id".to_string(),
        JsonValue::String(require_nonempty_text(payload.get("plan_id"), "plan_id")?),
    );
    normalized.insert(
        "revisions".to_string(),
        JsonValue::Array(
            require_object_list(payload.get("revisions"), "revisions")?
                .into_iter()
                .map(JsonValue::Object)
                .collect(),
        ),
    );
    Ok(JsonValue::Object(normalized))
}

pub fn normalize_plan_items_service_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_items_service_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_items_service_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let mut normalized = normalize_query_request(&payload, "plan items service request")?;
    normalized.insert(
        "plan".to_string(),
        JsonValue::Object(
            require_object(payload.get("plan"), "plan items service request plan")?.clone(),
        ),
    );
    normalized.insert(
        "revision".to_string(),
        match payload.get("revision") {
            None | Some(JsonValue::Null) => JsonValue::Null,
            Some(value) => JsonValue::Object(
                require_object(Some(value), "plan items service request revision")?.clone(),
            ),
        },
    );
    Ok(JsonValue::Object(normalized))
}

pub fn normalize_plan_candidates_service_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless()
        .normalize_plan_candidates_service_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_candidates_service_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let mut normalized = normalize_query_request(&payload, "plan candidates service request")?;
    normalized.insert(
        "plans".to_string(),
        JsonValue::Array(
            normalize_object_list(payload.get("plans"), "plans")?
                .into_iter()
                .map(JsonValue::Object)
                .collect(),
        ),
    );
    normalized.insert(
        "tasks".to_string(),
        JsonValue::Array(
            normalize_object_list(payload.get("tasks"), "tasks")?
                .into_iter()
                .map(JsonValue::Object)
                .collect(),
        ),
    );
    normalized.insert(
        "include_all".to_string(),
        JsonValue::Bool(optional_bool_with_default(
            payload.get("include_all"),
            false,
            "include_all",
        )?),
    );
    normalized.insert(
        "contains_terms".to_string(),
        JsonValue::Array(
            normalize_text_list(payload.get("contains_terms"), "contains_terms")?
                .into_iter()
                .map(JsonValue::String)
                .collect(),
        ),
    );
    let local_shadow_index = match payload.get("local_shadow_index") {
        None | Some(JsonValue::Null) => JsonMap::new(),
        Some(value) => {
            let index = require_object(
                Some(value),
                "plan candidates service request local_shadow_index",
            )?;
            let mut normalized_index = JsonMap::new();
            for (key, entry) in index {
                let plan_id = normalize_optional_text(Some(key.as_str()))
                    .filter(|value| value != "null")
                    .ok_or_else(|| {
                        "Plan candidates service request local_shadow_index keys must be plan ids."
                            .to_string()
                    })?;
                normalized_index.insert(
                    plan_id,
                    JsonValue::Object(
                        require_object(
                            Some(entry),
                            "plan candidates service request local shadow",
                        )?
                        .clone(),
                    ),
                );
            }
            normalized_index
        }
    };
    normalized.insert(
        "local_shadow_index".to_string(),
        JsonValue::Object(local_shadow_index),
    );
    Ok(JsonValue::Object(normalized))
}

pub fn normalize_plan_inspect_service_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_inspect_service_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_inspect_service_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let mut normalized = normalize_query_request(&payload, "plan inspect service request")?;
    normalized.insert(
        "plan".to_string(),
        JsonValue::Object(
            require_object(payload.get("plan"), "plan inspect service request plan")?.clone(),
        ),
    );
    normalized.insert(
        "revision".to_string(),
        match payload.get("revision") {
            None | Some(JsonValue::Null) => JsonValue::Null,
            Some(value) => JsonValue::Object(
                require_object(Some(value), "plan inspect service request revision")?.clone(),
            ),
        },
    );
    normalized.insert(
        "tasks".to_string(),
        JsonValue::Array(
            normalize_object_list(payload.get("tasks"), "tasks")?
                .into_iter()
                .map(JsonValue::Object)
                .collect(),
        ),
    );
    normalized.insert(
        "local_shadow".to_string(),
        match payload.get("local_shadow") {
            None | Some(JsonValue::Null) => JsonValue::Null,
            Some(value) => JsonValue::Object(
                require_object(Some(value), "plan inspect service request local_shadow")?.clone(),
            ),
        },
    );
    Ok(JsonValue::Object(normalized))
}

pub fn normalize_plan_sync_service_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_sync_service_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_sync_service_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let advisory = match payload.get("advisory") {
        None | Some(JsonValue::Null) => JsonValue::Null,
        Some(JsonValue::Object(value)) => JsonValue::Object(value.clone()),
        Some(_) => {
            return Err(
                "Plan sync service request advisory must be an object when present.".to_string(),
            )
        }
    };
    let error = match payload.get("error") {
        None | Some(JsonValue::Null) => JsonValue::Null,
        Some(JsonValue::Object(value)) => JsonValue::Object(value.clone()),
        Some(_) => {
            return Err(
                "Plan sync service request error must be an object when present.".to_string(),
            )
        }
    };
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "target".to_string(),
            JsonValue::String(require_plan_sync_text(&payload, "target")?),
        ),
        (
            "scope".to_string(),
            JsonValue::String(require_plan_sync_text(&payload, "scope")?),
        ),
        (
            "mode".to_string(),
            JsonValue::String(require_plan_sync_text(&payload, "mode")?),
        ),
        (
            "status".to_string(),
            JsonValue::String(require_plan_sync_text(&payload, "status")?),
        ),
        (
            "results".to_string(),
            JsonValue::Array(
                normalize_plan_application_object_list(payload.get("results"), "results")?
                    .into_iter()
                    .map(JsonValue::Object)
                    .collect(),
            ),
        ),
        (
            "adoptions".to_string(),
            JsonValue::Array(
                normalize_plan_application_object_list(payload.get("adoptions"), "adoptions")?
                    .into_iter()
                    .map(JsonValue::Object)
                    .collect(),
            ),
        ),
        (
            "publish_results".to_string(),
            JsonValue::Array(
                normalize_plan_application_object_list(
                    payload.get("publish_results"),
                    "publish_results",
                )?
                .into_iter()
                .map(JsonValue::Object)
                .collect(),
            ),
        ),
        (
            "artifact_results".to_string(),
            JsonValue::Array(
                normalize_plan_application_object_list(
                    payload.get("artifact_results"),
                    "artifact_results",
                )?
                .into_iter()
                .map(JsonValue::Object)
                .collect(),
            ),
        ),
        ("advisory".to_string(), advisory),
        ("error".to_string(), error),
    ])))
}
