use crate::json_support::{
    parse_json_array_with_error_prefix, parse_json_object_with_error_prefix,
};
use ait_core::plan_http_sdk::{
    build_query_string, join_path as join_remote_path, normalize_remote_text,
    require_remote_non_empty,
};

#[pyfunction(name = "join_path")]
fn join_path_py(base_url: &str, path: &str) -> String {
    join_remote_path(base_url, path)
}

#[pyfunction(name = "build_query_string")]
fn build_query_string_py(params: Bound<'_, PyDict>) -> PyResult<Option<String>> {
    let mut parsed = BTreeMap::new();
    for (key, value) in params.iter() {
        let key = key
            .extract::<String>()
            .map_err(|_| PyValueError::new_err("query parameter keys must be strings."))?;
        if value.is_none() {
            return Err(PyValueError::new_err(
                "query parameter values must be strings.",
            ));
        }
        parsed.insert(
            key,
            value
                .extract::<String>()
                .map_err(|_| PyValueError::new_err("query parameter values must be strings."))?,
        );
    }
    Ok(build_query_string(&parsed))
}

#[pyfunction(name = "build_list_plans_request")]
fn build_list_plans_request_py(
    py: Python<'_>,
    base_url: &str,
    repo_name: &str,
    artifact_path: Option<&str>,
) -> PyResult<Py<PyDict>> {
    require_remote_non_empty(repo_name, "repo_name").map_err(PyValueError::new_err)?;
    render_json_dict(
        py,
        ait_core::plan_http_sdk::build_list_plans_request(base_url, repo_name, artifact_path)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_get_plan_request")]
fn build_get_plan_request_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
) -> PyResult<Py<PyDict>> {
    require_remote_non_empty(plan_id, "plan_id").map_err(PyValueError::new_err)?;
    render_json_dict(
        py,
        ait_core::plan_http_sdk::build_get_plan_request(base_url, plan_id)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_list_plan_revisions_request")]
fn build_list_plan_revisions_request_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
) -> PyResult<Py<PyDict>> {
    require_remote_non_empty(plan_id, "plan_id").map_err(PyValueError::new_err)?;
    render_json_dict(
        py,
        ait_core::plan_http_sdk::build_list_plan_revisions_request(base_url, plan_id)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_get_plan_revision_request")]
fn build_get_plan_revision_request_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    plan_revision_id: &str,
) -> PyResult<Py<PyDict>> {
    require_remote_non_empty(plan_id, "plan_id").map_err(PyValueError::new_err)?;
    require_remote_non_empty(plan_revision_id, "plan_revision_id")
        .map_err(PyValueError::new_err)?;
    render_json_dict(
        py,
        ait_core::plan_http_sdk::build_get_plan_revision_request(
            base_url,
            plan_id,
            plan_revision_id,
        )
        .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_create_plan_request")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    base_url,
    repo_name,
    title,
    artifact_path,
    artifact_selector,
    artifact_heading,
    items,
    summary=None,
    status="draft",
    plan_id=None,
    source_kind=None,
    artifact_body=None
))]
fn build_create_plan_request_py(
    py: Python<'_>,
    base_url: &str,
    repo_name: &str,
    title: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: Bound<'_, PyAny>,
    summary: Option<&str>,
    status: &str,
    plan_id: Option<&str>,
    source_kind: Option<&str>,
    artifact_body: Option<&str>,
) -> PyResult<Py<PyDict>> {
    require_remote_non_empty(repo_name, "repo_name").map_err(PyValueError::new_err)?;
    require_remote_non_empty(title, "title").map_err(PyValueError::new_err)?;
    require_remote_non_empty(artifact_path, "artifact_path").map_err(PyValueError::new_err)?;
    require_remote_non_empty(artifact_heading, "artifact_heading")
        .map_err(PyValueError::new_err)?;
    let items = JsonValue::Array(parse_json_array(items, "items")?);
    render_json_dict(
        py,
        ait_core::plan_http_sdk::build_create_plan_request(
            base_url,
            repo_name,
            title,
            artifact_path,
            artifact_selector,
            artifact_heading,
            &items,
            summary,
            status,
            plan_id,
            source_kind,
            artifact_body,
        )
        .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_revise_plan_request")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    base_url,
    plan_id,
    artifact_path,
    artifact_selector,
    artifact_heading,
    items,
    title=None,
    summary=None,
    source_kind=None,
    artifact_body=None,
    expected_head_revision_id=None
))]
fn build_revise_plan_request_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: Bound<'_, PyAny>,
    title: Option<&str>,
    summary: Option<&str>,
    source_kind: Option<&str>,
    artifact_body: Option<&str>,
    expected_head_revision_id: Option<&str>,
) -> PyResult<Py<PyDict>> {
    require_remote_non_empty(plan_id, "plan_id").map_err(PyValueError::new_err)?;
    require_remote_non_empty(artifact_path, "artifact_path").map_err(PyValueError::new_err)?;
    require_remote_non_empty(artifact_heading, "artifact_heading")
        .map_err(PyValueError::new_err)?;
    let items = JsonValue::Array(parse_json_array(items, "items")?);
    render_json_dict(
        py,
        ait_core::plan_http_sdk::build_revise_plan_request(
            base_url,
            plan_id,
            artifact_path,
            artifact_selector,
            artifact_heading,
            &items,
            title,
            summary,
            source_kind,
            artifact_body,
            expected_head_revision_id,
        )
        .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_update_plan_status_request")]
fn build_update_plan_status_request_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    status: &str,
) -> PyResult<Py<PyDict>> {
    require_remote_non_empty(plan_id, "plan_id").map_err(PyValueError::new_err)?;
    require_remote_non_empty(status, "status").map_err(PyValueError::new_err)?;
    render_json_dict(
        py,
        ait_core::plan_http_sdk::build_update_plan_status_request(base_url, plan_id, status)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_put_plan_revision_artifacts_request")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    base_url,
    plan_id,
    plan_revision_id,
    artifacts
))]
fn build_put_plan_revision_artifacts_request_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    plan_revision_id: &str,
    artifacts: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    require_remote_non_empty(plan_id, "plan_id").map_err(PyValueError::new_err)?;
    require_remote_non_empty(plan_revision_id, "plan_revision_id")
        .map_err(PyValueError::new_err)?;
    let artifacts = JsonValue::Array(parse_json_array(artifacts, "artifacts")?);
    render_json_dict(
        py,
        ait_core::plan_http_sdk::build_put_plan_revision_artifacts_request(
            base_url,
            plan_id,
            plan_revision_id,
            &artifacts,
        )
        .map_err(PyValueError::new_err)?,
    )
}

fn parse_remote_timeout(field: &str, value: Option<f64>) -> PyResult<f64> {
    ait_core::plan_http_sdk::remote_timeout(field, value).map_err(PyValueError::new_err)
}

fn parse_json_array(items: Bound<'_, PyAny>, field_name: &str) -> PyResult<Vec<JsonValue>> {
    let list = items
        .cast::<PyList>()
        .map_err(|_| PyValueError::new_err(format!("{field_name} must be a list.")))?;
    let mut parsed = Vec::with_capacity(list.len());
    for value in list.iter() {
        parsed.push(parse_json_value(value, field_name)?);
    }
    Ok(parsed)
}

fn parse_json_value(value: Bound<'_, PyAny>, field_name: &str) -> PyResult<JsonValue> {
    if value.is_none() {
        return Ok(JsonValue::Null);
    }
    if let Ok(value) = value.extract::<bool>() {
        return Ok(JsonValue::Bool(value));
    }
    if let Ok(value) = value.extract::<i64>() {
        return Ok(JsonValue::Number(value.into()));
    }
    if let Ok(value) = value.extract::<u64>() {
        return Ok(JsonValue::Number(value.into()));
    }
    if let Ok(value) = value.extract::<f64>() {
        return ait_core::json_support::JsonNumber::from_f64(value)
            .map(JsonValue::Number)
            .ok_or_else(|| {
                PyValueError::new_err(format!("{field_name} contains a non-finite numeric value."))
            });
    }
    if let Ok(value) = value.extract::<Vec<u8>>() {
        return Ok(JsonValue::Array(
            value
                .into_iter()
                .map(|byte| JsonValue::Number(Number::from(byte)))
                .collect(),
        ));
    }
    if let Ok(value) = value.extract::<String>() {
        return Ok(JsonValue::String(value));
    }
    if let Ok(value) = value.cast::<PyList>() {
        let mut output = Vec::with_capacity(value.len());
        for entry in value.iter() {
            output.push(parse_json_value(entry, field_name)?);
        }
        return Ok(JsonValue::Array(output));
    }
    if let Ok(value) = value.cast::<PyTuple>() {
        let mut output = Vec::with_capacity(value.len());
        for entry in value.iter() {
            output.push(parse_json_value(entry, field_name)?);
        }
        return Ok(JsonValue::Array(output));
    }
    if let Ok(value) = value.cast::<PyDict>() {
        let mut output = ait_core::json_support::JsonMap::new();
        for (entry_key, entry_value) in value.iter() {
            let key = entry_key.extract::<String>().map_err(|_| {
                PyValueError::new_err(format!("{field_name} keys must be strings."))
            })?;
            output.insert(key, parse_json_value(entry_value, field_name)?);
        }
        return Ok(JsonValue::Object(output));
    }
    Err(PyValueError::new_err(format!(
        "{field_name} must contain JSON-serializable values."
    )))
}

fn optional_json_value(
    value: Option<Bound<'_, PyAny>>,
    field_name: &str,
) -> PyResult<Option<JsonValue>> {
    match value {
        Some(value) if !value.is_none() => parse_json_value(value, field_name).map(Some),
        _ => Ok(None),
    }
}

fn parse_json_text_array(text: &str, field_name: &str) -> PyResult<Vec<JsonValue>> {
    parse_json_array_with_error_prefix(
        text,
        &format!("{field_name} must contain a JSON array"),
        &format!("{field_name} must contain a JSON array."),
    )
    .map_err(PyValueError::new_err)
}

fn parse_json_text_string_array(text: &str, field_name: &str) -> PyResult<Vec<String>> {
    parse_json_text_array(text, field_name)?
        .into_iter()
        .map(|value| match value {
            JsonValue::String(text) => Ok(text),
            _ => Err(PyValueError::new_err(format!(
                "{field_name} must contain only strings."
            ))),
        })
        .collect()
}

fn parse_json_text_object(
    text: &str,
    field_name: &str,
) -> PyResult<ait_core::json_support::JsonMap<String, JsonValue>> {
    parse_json_object_with_error_prefix(
        text,
        &format!("{field_name} must contain a JSON object"),
        &format!("{field_name} must contain a JSON object."),
    )
    .map_err(PyValueError::new_err)
}

fn parse_blob_bytes_map(value: Option<Bound<'_, PyAny>>) -> PyResult<BTreeMap<String, Vec<u8>>> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    if value.is_none() {
        return Ok(BTreeMap::new());
    }
    let mapping = value
        .cast::<PyDict>()
        .map_err(|_| PyValueError::new_err("blob_bytes_by_id must be a dictionary."))?;
    let mut output = BTreeMap::new();
    for (entry_key, entry_value) in mapping.iter() {
        let key = entry_key
            .extract::<String>()
            .map_err(|_| PyValueError::new_err("blob_bytes_by_id keys must be strings."))?;
        if key.trim().is_empty() {
            return Err(PyValueError::new_err(
                "blob_bytes_by_id keys must be non-empty strings.",
            ));
        }
        if let Ok(bytes) = entry_value.extract::<Vec<u8>>() {
            output.insert(key, bytes);
            continue;
        }
        if let Ok(text) = entry_value.extract::<String>() {
            output.insert(key, text.into_bytes());
            continue;
        }
        return Err(PyValueError::new_err(
            "blob_bytes_by_id values must be bytes or strings.",
        ));
    }
    Ok(output)
}

fn build_plan_http_client_config(
    base_url: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<PlanHttpClientConfig> {
    ait_core::plan_http_sdk::client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
        None,
    )
    .map_err(PyValueError::new_err)
}

#[pyfunction(name = "list_plans")]
#[pyo3(signature = (base_url, repo_name, artifact_path=None, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn list_plans_py(
    py: Python<'_>,
    base_url: &str,
    repo_name: &str,
    artifact_path: Option<&str>,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyList>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| list_http_plans(config, repo_name, artifact_path))
        .map_err(plan_http_py_error)?;
    render_json_list(py, payload)
}

#[pyfunction(name = "get_plan")]
#[pyo3(signature = (base_url, plan_id, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn get_plan_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| get_http_plan(config, plan_id))
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "list_plan_revisions")]
#[pyo3(signature = (base_url, plan_id, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn list_plan_revisions_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyList>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| list_http_plan_revisions(config, plan_id))
        .map_err(plan_http_py_error)?;
    render_json_list(py, payload)
}

#[pyfunction(name = "resolve_task_plan_linkage")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (base_url, repo_name, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
fn resolve_task_plan_linkage_py(
    py: Python<'_>,
    base_url: &str,
    repo_name: &str,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| {
            resolve_http_task_plan_linkage(
                config,
                repo_name,
                plan_id,
                origin_plan_revision_id,
                plan_item_ref,
            )
        })
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "list_plan_ids_matching_contains")]
#[pyo3(signature = (base_url, repo_name, contains_terms_json, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn list_plan_ids_matching_contains_py(
    py: Python<'_>,
    base_url: &str,
    repo_name: &str,
    contains_terms_json: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyList>> {
    let contains_terms = parse_json_text_string_array(contains_terms_json, "contains_terms_json")?;
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| list_http_plan_ids_matching_contains(config, repo_name, &contains_terms))
        .map_err(plan_http_py_error)?;
    render_json_list(py, payload)
}

#[pyfunction(name = "get_plan_revision")]
#[pyo3(signature = (base_url, plan_id, plan_revision_id, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn get_plan_revision_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    plan_revision_id: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| get_http_plan_revision(config, plan_id, plan_revision_id))
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "create_plan")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    base_url,
    repo_name,
    title,
    artifact_path,
    artifact_selector,
    artifact_heading,
    items_json,
    summary=None,
    status="draft",
    plan_id=None,
    source_kind="manual_edit",
    artifact_body=None,
    headers_json=None,
    timeout_ms=None,
    retry_attempts=0,
    retry_backoff_ms=0,
    pool_max_idle_per_host=1
))]
fn create_plan_py(
    py: Python<'_>,
    base_url: &str,
    repo_name: &str,
    title: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items_json: &str,
    summary: Option<&str>,
    status: &str,
    plan_id: Option<&str>,
    source_kind: &str,
    artifact_body: Option<&str>,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let items = parse_json_text_array(items_json, "items_json")?;
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| {
            create_http_plan(
                config,
                repo_name,
                title,
                artifact_path,
                artifact_selector,
                artifact_heading,
                &items,
                summary,
                status,
                plan_id,
                source_kind,
                artifact_body,
            )
        })
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "revise_plan")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    base_url,
    plan_id,
    artifact_path,
    artifact_selector,
    artifact_heading,
    items_json,
    title=None,
    summary=None,
    source_kind="manual_edit",
    artifact_body=None,
    expected_head_revision_id=None,
    headers_json=None,
    timeout_ms=None,
    retry_attempts=0,
    retry_backoff_ms=0,
    pool_max_idle_per_host=1
))]
fn revise_plan_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items_json: &str,
    title: Option<&str>,
    summary: Option<&str>,
    source_kind: &str,
    artifact_body: Option<&str>,
    expected_head_revision_id: Option<&str>,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let items = parse_json_text_array(items_json, "items_json")?;
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| {
            revise_http_plan(
                config,
                plan_id,
                artifact_path,
                artifact_selector,
                artifact_heading,
                &items,
                title,
                summary,
                source_kind,
                artifact_body,
                expected_head_revision_id,
            )
        })
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "update_plan_status")]
#[pyo3(signature = (base_url, plan_id, status, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn update_plan_status_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    status: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| update_http_plan_status(config, plan_id, status))
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "put_plan_revision_artifacts")]
#[pyo3(signature = (base_url, plan_id, plan_revision_id, artifacts_json, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn put_plan_revision_artifacts_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    plan_revision_id: &str,
    artifacts_json: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let artifacts = parse_json_text_array(artifacts_json, "artifacts_json")?;
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| put_http_plan_revision_artifacts(config, plan_id, plan_revision_id, &artifacts))
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "create_planning_session")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (base_url, plan_id, title=None, mode="connected_local", preferred_agent=None, resume_if_active=true, planning_session_id=None, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
fn create_planning_session_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    title: Option<&str>,
    mode: &str,
    preferred_agent: Option<&str>,
    resume_if_active: bool,
    planning_session_id: Option<&str>,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| {
            create_http_planning_session(
                config,
                plan_id,
                title,
                mode,
                preferred_agent,
                resume_if_active,
                planning_session_id,
            )
        })
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "list_planning_sessions")]
#[pyo3(signature = (base_url, plan_id, status=None, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn list_planning_sessions_py(
    py: Python<'_>,
    base_url: &str,
    plan_id: &str,
    status: Option<&str>,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyList>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| list_http_planning_sessions(config, plan_id, status))
        .map_err(plan_http_py_error)?;
    render_json_list(py, payload)
}

#[pyfunction(name = "get_planning_session")]
#[pyo3(signature = (base_url, planning_session_id, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn get_planning_session_py(
    py: Python<'_>,
    base_url: &str,
    planning_session_id: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| get_http_planning_session(config, planning_session_id))
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "append_planning_session_event")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (base_url, planning_session_id, event_type, payload_json=None, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
fn append_planning_session_event_py(
    py: Python<'_>,
    base_url: &str,
    planning_session_id: &str,
    event_type: &str,
    payload_json: Option<&str>,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = JsonValue::Object(parse_json_text_object(
        payload_json.unwrap_or("{}"),
        "payload_json",
    )?);
    let result = py
        .detach(|| {
            append_http_planning_session_event(config, planning_session_id, event_type, &payload)
        })
        .map_err(plan_http_py_error)?;
    render_json_dict(py, result)
}

#[pyfunction(name = "list_planning_session_events")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (base_url, planning_session_id, after_sequence=0, limit=200, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
fn list_planning_session_events_py(
    py: Python<'_>,
    base_url: &str,
    planning_session_id: &str,
    after_sequence: i64,
    limit: i64,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyList>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| {
            list_http_planning_session_events(config, planning_session_id, after_sequence, limit)
        })
        .map_err(plan_http_py_error)?;
    render_json_list(py, payload)
}

#[pyfunction(name = "join_planning_session")]
#[pyo3(signature = (base_url, planning_session_id, surface="cli", title=None, model_name=None, resume_if_active=true, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn join_planning_session_py(
    py: Python<'_>,
    base_url: &str,
    planning_session_id: &str,
    surface: &str,
    title: Option<&str>,
    model_name: Option<&str>,
    resume_if_active: bool,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| {
            join_http_planning_session(
                config,
                planning_session_id,
                surface,
                title,
                model_name,
                resume_if_active,
            )
        })
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "promote_planning_session")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (base_url, planning_session_id, artifact_path, artifact_selector, artifact_heading, items_json, title=None, summary=None, artifact_body=None, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
fn promote_planning_session_py(
    py: Python<'_>,
    base_url: &str,
    planning_session_id: &str,
    artifact_path: &str,
    artifact_selector: &str,
    artifact_heading: &str,
    items_json: &str,
    title: Option<&str>,
    summary: Option<&str>,
    artifact_body: Option<&str>,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let items = parse_json_text_array(items_json, "items_json")?;
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| {
            promote_http_planning_session(
                config,
                planning_session_id,
                artifact_path,
                artifact_selector,
                artifact_heading,
                &items,
                title,
                summary,
                artifact_body,
            )
        })
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "close_planning_session")]
#[pyo3(signature = (base_url, planning_session_id, status="closed", headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn close_planning_session_py(
    py: Python<'_>,
    base_url: &str,
    planning_session_id: &str,
    status: &str,
    headers_json: Option<&str>,
    timeout_ms: Option<u64>,
    retry_attempts: usize,
    retry_backoff_ms: u64,
    pool_max_idle_per_host: usize,
) -> PyResult<Py<PyDict>> {
    let config = build_plan_http_client_config(
        base_url,
        headers_json,
        timeout_ms,
        retry_attempts,
        retry_backoff_ms,
        pool_max_idle_per_host,
    )?;
    let payload = py
        .detach(|| close_http_planning_session(config, planning_session_id, status))
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

fn register_plan_http(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(join_path_py, module)?)?;
    module.add_function(wrap_pyfunction!(build_query_string_py, module)?)?;
    module.add_function(wrap_pyfunction!(build_list_plans_request_py, module)?)?;
    module.add_function(wrap_pyfunction!(build_get_plan_request_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        build_list_plan_revisions_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        build_get_plan_revision_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(build_create_plan_request_py, module)?)?;
    module.add_function(wrap_pyfunction!(build_revise_plan_request_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        build_update_plan_status_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        build_put_plan_revision_artifacts_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(list_plans_py, module)?)?;
    module.add_function(wrap_pyfunction!(get_plan_py, module)?)?;
    module.add_function(wrap_pyfunction!(list_plan_revisions_py, module)?)?;
    module.add_function(wrap_pyfunction!(resolve_task_plan_linkage_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        list_plan_ids_matching_contains_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(get_plan_revision_py, module)?)?;
    module.add_function(wrap_pyfunction!(create_plan_py, module)?)?;
    module.add_function(wrap_pyfunction!(revise_plan_py, module)?)?;
    module.add_function(wrap_pyfunction!(update_plan_status_py, module)?)?;
    module.add_function(wrap_pyfunction!(put_plan_revision_artifacts_py, module)?)?;
    module.add_function(wrap_pyfunction!(create_planning_session_py, module)?)?;
    module.add_function(wrap_pyfunction!(list_planning_sessions_py, module)?)?;
    module.add_function(wrap_pyfunction!(get_planning_session_py, module)?)?;
    module.add_function(wrap_pyfunction!(append_planning_session_event_py, module)?)?;
    module.add_function(wrap_pyfunction!(list_planning_session_events_py, module)?)?;
    module.add_function(wrap_pyfunction!(join_planning_session_py, module)?)?;
    module.add_function(wrap_pyfunction!(promote_planning_session_py, module)?)?;
    module.add_function(wrap_pyfunction!(close_planning_session_py, module)?)?;
    Ok(())
}
