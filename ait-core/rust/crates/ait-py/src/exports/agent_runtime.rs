#[pyfunction(name = "extract_codex_usage_jsonl")]
fn extract_codex_usage_jsonl_py(
    py: Python<'_>,
    usage_jsonl_path: &str,
    usage_scope: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        rust_extract_codex_usage_jsonl(std::path::Path::new(usage_jsonl_path), usage_scope)
            .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "extract_codex_usage_bundle_jsonl", signature = (usage_jsonl_paths, *, usage_roles = None, usage_scope = "total"))]
fn extract_codex_usage_bundle_jsonl_py(
    py: Python<'_>,
    usage_jsonl_paths: Vec<String>,
    usage_roles: Option<Vec<String>>,
    usage_scope: &str,
) -> PyResult<Py<PyDict>> {
    let payload = rust_extract_codex_usage_bundle_jsonl(
        &usage_jsonl_paths,
        usage_roles.as_deref(),
        usage_scope,
    )
    .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "project_repo_retire_runtime_blockers")]
fn project_repo_retire_runtime_blockers_py(
    py: Python<'_>,
    payload: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let payload = parse_json_value(payload, "repo retirement runtime blocker payload")?;
    let projected =
        rust_project_repo_retire_runtime_blockers(&payload).map_err(PyValueError::new_err)?;
    render_json_dict(py, projected)
}

#[pyfunction(name = "language_binding_info")]
fn language_binding_info_py(py: Python<'_>) -> PyResult<Py<PyDict>> {
    render_json_dict(py, language_binding_info_json())
}

#[pyfunction(name = "ait_agent_worker_capabilities")]
fn ait_agent_worker_capabilities_py(py: Python<'_>) -> PyResult<Py<PyDict>> {
    let payload = py
        .detach(agent_worker_capabilities_binding_json)
        .map_err(PyRuntimeError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "ait_agent_worker_transaction")]
fn ait_agent_worker_transaction_py(
    py: Python<'_>,
    request: Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let request = parse_json_value(request, "request")?;
    let payload = py
        .detach(move || agent_worker_transaction_binding_json(&request))
        .map_err(PyRuntimeError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "transport_envelope_ir_version")]
fn transport_envelope_ir_version_py() -> &'static str {
    transport_envelope_ir_version()
}

#[pyfunction(name = "transport_envelope_schema")]
fn transport_envelope_schema_py(py: Python<'_>) -> PyResult<Py<PyDict>> {
    render_json_dict(py, transport_envelope_schema_json())
}

#[pyfunction(name = "runtime_binding_state_ir_version")]
fn runtime_binding_state_ir_version_py() -> &'static str {
    runtime_binding_state_ir_version()
}

#[pyfunction(name = "runtime_binding_state_schema")]
fn runtime_binding_state_schema_py(py: Python<'_>) -> PyResult<Py<PyDict>> {
    render_json_dict(py, runtime_binding_state_schema_json())
}

#[pyfunction(name = "runtime_binding_state_default_payload")]
fn runtime_binding_state_default_payload_py(py: Python<'_>) -> PyResult<Py<PyDict>> {
    render_json_dict(py, default_runtime_binding_state_payload_json())
}

#[pyfunction(name = "runtime_binding_state_normalize_document")]
#[pyo3(signature = (payload))]
fn runtime_binding_state_normalize_document_py(
    py: Python<'_>,
    payload: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let payload_value = parse_json_value(payload, "payload")?;
    render_json_dict(
        py,
        normalize_runtime_binding_state_document_json(&payload_value),
    )
}

#[pyfunction(name = "ait_agent_web_runtime_execute")]
#[pyo3(signature = (request))]
fn ait_agent_web_runtime_execute_py(
    py: Python<'_>,
    request: Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let request_value = parse_json_value(request, "request")?;
    let result = py
        .detach(move || agent_web_runtime_execute_json(&request_value))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, result)
}

#[pyfunction(name = "worker_manifest_ir_version")]
fn worker_manifest_ir_version_py() -> &'static str {
    worker_manifest_ir_version()
}

#[pyfunction(name = "worker_manifest_schema")]
fn worker_manifest_schema_py(py: Python<'_>) -> PyResult<Py<PyDict>> {
    render_json_dict(py, worker_manifest_schema_json())
}

#[pyfunction(name = "worker_manifest_default_config")]
fn worker_manifest_default_config_py(py: Python<'_>) -> PyResult<Py<PyDict>> {
    render_json_dict(py, default_worker_manifest_config_json())
}

#[pyfunction(name = "worker_manifest_normalize_document")]
#[pyo3(signature = (payload, *, path=None))]
fn worker_manifest_normalize_document_py(
    py: Python<'_>,
    payload: Bound<'_, PyAny>,
    path: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let payload_value = parse_json_value(payload, "payload")?;
    render_json_dict(
        py,
        normalize_worker_manifest_document_json(&payload_value, path),
    )
}

#[pyfunction(name = "worker_manifest_select_telegram_worker")]
#[pyo3(signature = (*, config, requested_name=None))]
fn worker_manifest_select_telegram_worker_py(
    py: Python<'_>,
    config: Bound<'_, PyAny>,
    requested_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let config_value = parse_json_value(config, "config")?;
    render_json_value(
        py,
        select_telegram_worker_json(&config_value, requested_name),
    )
}

#[pyfunction(name = "ait_agent_env_file_load")]
fn ait_agent_env_file_load_py(
    py: Python<'_>,
    request: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let request_value = parse_json_value(request, "request")?;
    let payload = py
        .detach(move || agent_env_file_load_json(&request_value))
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "ait_agent_telegram_workflow_notification_format")]
fn ait_agent_telegram_workflow_notification_format_py(
    py: Python<'_>,
    request: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let request_value = parse_json_value(request, "request")?;
    let payload = agent_telegram_workflow_notification_format_json(&request_value)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "ait_agent_telegram_turn_input_plan")]
fn ait_agent_telegram_turn_input_plan_py(
    py: Python<'_>,
    request: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let request_value = parse_json_value(request, "request")?;
    let payload =
        agent_telegram_turn_input_plan_json(&request_value).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "ait_agent_telegram_message_delivery_execute")]
fn ait_agent_telegram_message_delivery_execute_py(
    py: Python<'_>,
    request: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let request_value = parse_json_value(request, "request")?;
    let payload = py
        .detach(move || agent_telegram_message_delivery_execute_json(&request_value))
        .map_err(PyValueError::new_err)?;
    validate_telegram_message_delivery_export(&payload).map_err(PyRuntimeError::new_err)?;
    render_json_dict(py, payload)
}

fn validate_telegram_message_delivery_export(payload: &JsonValue) -> Result<(), &'static str> {
    const INVALID: &str =
        "Rust Telegram message delivery returned an invalid export contract.";
    let object = payload.as_object().ok_or(INVALID)?;
    if object.get("contract").and_then(JsonValue::as_str)
        != Some("ait_agent_core.event_loop.TelegramMessageDeliveryExecution.v1")
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some("rust_agent_telegram_message_delivery_execution")
        || object.get("stage").and_then(JsonValue::as_str) != Some("execute")
        || object
            .get("python_message_delivery_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object
            .get("python_message_formatting_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object
            .get("raw_api_result_exposed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object
            .get("telegram_description_exposed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object
            .get("token_bearing_url_exposed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object.get("chat_id_exposed").and_then(JsonValue::as_bool) != Some(false)
        || object
            .get("formatted_text_exposed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object.get("plain_text_exposed").and_then(JsonValue::as_bool) != Some(false)
    {
        return Err(INVALID);
    }
    Ok(())
}

#[pyfunction(name = "ait_agent_telegram_workflow_query_plan")]
fn ait_agent_telegram_workflow_query_plan_py(
    py: Python<'_>,
    request: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let request_value = parse_json_value(request, "request")?;
    let payload =
        agent_telegram_workflow_query_plan_json(&request_value).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "transport_envelope_build_binding_metadata")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (*, transport, surface_id, conversation_key, surface_title=None, surface_kind=None, thread_id=None, reply_target=None, metadata_extra=None))]
fn transport_envelope_build_binding_metadata_py(
    py: Python<'_>,
    transport: &str,
    surface_id: &str,
    conversation_key: &str,
    surface_title: Option<&str>,
    surface_kind: Option<&str>,
    thread_id: Option<&str>,
    reply_target: Option<Bound<'_, PyAny>>,
    metadata_extra: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let reply_target_value = match reply_target {
        Some(value) => Some(parse_json_value(value, "reply_target")?),
        None => None,
    };
    let metadata_extra_value = match metadata_extra {
        Some(value) => Some(parse_json_value(value, "metadata_extra")?),
        None => None,
    };
    render_json_dict(
        py,
        build_transport_binding_metadata_json(
            transport,
            surface_id,
            surface_title,
            surface_kind,
            thread_id,
            conversation_key,
            reply_target_value.as_ref(),
            metadata_extra_value.as_ref(),
        ),
    )
}

#[pyfunction(name = "transport_envelope_build_event_envelope")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (*, transport, actor_identity, channel_id, text, actor_transport_id=None, actor_username=None, actor_display_name=None, actor_is_bot=None, channel_title=None, channel_kind=None, thread_id=None, message_id=None, message_ids=None, occurred_at=None, event_id=None, dedupe_key=None, attachments=None, metadata=None))]
fn transport_envelope_build_event_envelope_py(
    py: Python<'_>,
    transport: &str,
    actor_identity: &str,
    channel_id: &str,
    text: &str,
    actor_transport_id: Option<&str>,
    actor_username: Option<&str>,
    actor_display_name: Option<&str>,
    actor_is_bot: Option<bool>,
    channel_title: Option<&str>,
    channel_kind: Option<&str>,
    thread_id: Option<&str>,
    message_id: Option<Bound<'_, PyAny>>,
    message_ids: Option<Bound<'_, PyAny>>,
    occurred_at: Option<&str>,
    event_id: Option<&str>,
    dedupe_key: Option<&str>,
    attachments: Option<Bound<'_, PyAny>>,
    metadata: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let message_id_value = match message_id {
        Some(value) => Some(parse_json_value(value, "message_id")?),
        None => None,
    };
    let message_ids_value = match message_ids {
        Some(value) => Some(parse_json_value(value, "message_ids")?),
        None => None,
    };
    let attachments_value = match attachments {
        Some(value) => Some(parse_json_value(value, "attachments")?),
        None => None,
    };
    let metadata_value = match metadata {
        Some(value) => Some(parse_json_value(value, "metadata")?),
        None => None,
    };
    render_json_dict(
        py,
        build_transport_event_envelope_json(
            transport,
            actor_identity,
            channel_id,
            text,
            actor_transport_id,
            actor_username,
            actor_display_name,
            actor_is_bot,
            channel_title,
            channel_kind,
            thread_id,
            message_id_value.as_ref(),
            message_ids_value.as_ref(),
            occurred_at,
            event_id,
            dedupe_key,
            attachments_value.as_ref(),
            metadata_value.as_ref(),
        ),
    )
}

#[pyfunction(name = "transport_envelope_build_reply_envelope")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (*, transport, channel_id, text, channel_title=None, channel_kind=None, thread_id=None, delivery_kind="chat_reply", reply_to_event_id=None, reply_to_message_id=None, reply_to_message_ids=None, attachments=None, metadata=None))]
fn transport_envelope_build_reply_envelope_py(
    py: Python<'_>,
    transport: &str,
    channel_id: &str,
    text: &str,
    channel_title: Option<&str>,
    channel_kind: Option<&str>,
    thread_id: Option<&str>,
    delivery_kind: &str,
    reply_to_event_id: Option<&str>,
    reply_to_message_id: Option<Bound<'_, PyAny>>,
    reply_to_message_ids: Option<Bound<'_, PyAny>>,
    attachments: Option<Bound<'_, PyAny>>,
    metadata: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let reply_to_message_id_value = match reply_to_message_id {
        Some(value) => Some(parse_json_value(value, "reply_to_message_id")?),
        None => None,
    };
    let reply_to_message_ids_value = match reply_to_message_ids {
        Some(value) => Some(parse_json_value(value, "reply_to_message_ids")?),
        None => None,
    };
    let attachments_value = match attachments {
        Some(value) => Some(parse_json_value(value, "attachments")?),
        None => None,
    };
    let metadata_value = match metadata {
        Some(value) => Some(parse_json_value(value, "metadata")?),
        None => None,
    };
    render_json_dict(
        py,
        build_transport_reply_envelope_json(
            transport,
            channel_id,
            text,
            channel_title,
            channel_kind,
            thread_id,
            Some(delivery_kind),
            reply_to_event_id,
            reply_to_message_id_value.as_ref(),
            reply_to_message_ids_value.as_ref(),
            attachments_value.as_ref(),
            metadata_value.as_ref(),
        ),
    )
}

#[pyfunction(name = "transport_envelope_compact_event_envelope")]
fn transport_envelope_compact_event_envelope_py(
    py: Python<'_>,
    envelope: Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let envelope_value = parse_json_value(envelope, "envelope")?;
    render_json_value(py, compact_transport_event_envelope_json(&envelope_value))
}

#[pyfunction(name = "transport_envelope_compact_reply_envelope")]
fn transport_envelope_compact_reply_envelope_py(
    py: Python<'_>,
    envelope: Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let envelope_value = parse_json_value(envelope, "envelope")?;
    render_json_value(py, compact_transport_reply_envelope_json(&envelope_value))
}

fn register_agent_runtime(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(language_binding_info_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        ait_agent_worker_capabilities_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        ait_agent_worker_transaction_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(transport_envelope_ir_version_py, module)?)?;
    module.add_function(wrap_pyfunction!(transport_envelope_schema_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        runtime_binding_state_ir_version_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(runtime_binding_state_schema_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        runtime_binding_state_default_payload_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        runtime_binding_state_normalize_document_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(ait_agent_web_runtime_execute_py, module)?)?;
    module.add_function(wrap_pyfunction!(worker_manifest_ir_version_py, module)?)?;
    module.add_function(wrap_pyfunction!(worker_manifest_schema_py, module)?)?;
    module.add_function(wrap_pyfunction!(worker_manifest_default_config_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        worker_manifest_normalize_document_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        worker_manifest_select_telegram_worker_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(ait_agent_env_file_load_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        ait_agent_telegram_workflow_notification_format_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        ait_agent_telegram_turn_input_plan_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        ait_agent_telegram_message_delivery_execute_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        ait_agent_telegram_workflow_query_plan_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        transport_envelope_build_binding_metadata_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        transport_envelope_build_event_envelope_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        transport_envelope_build_reply_envelope_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        transport_envelope_compact_event_envelope_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        transport_envelope_compact_reply_envelope_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(extract_codex_usage_jsonl_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        extract_codex_usage_bundle_jsonl_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        project_repo_retire_runtime_blockers_py,
        module
    )?)?;
    Ok(())
}
