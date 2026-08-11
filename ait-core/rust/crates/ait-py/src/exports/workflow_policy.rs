#[pyfunction(name = "normalize_task_status")]
fn normalize_task_status_py(py: Python<'_>, value: Option<&str>) -> PyResult<Py<PyAny>> {
    let normalized = task_status_value(value).map_err(PyValueError::new_err)?;
    Ok(match normalized {
        Some(value) => value.into_pyobject(py)?.unbind().into_any(),
        None => py.None(),
    })
}

#[pyfunction(name = "normalize_optional_text")]
fn normalize_optional_text_py(
    py: Python<'_>,
    value: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyAny>> {
    Ok(match normalize_optional_text_value(value) {
        Some(value) => value.into_pyobject(py)?.unbind().into_any(),
        None => py.None(),
    })
}

#[pyfunction(name = "utc_now")]
fn utc_now_py() -> PyResult<String> {
    let payload = build_plan_timestamp_payload_json("{}").map_err(PyValueError::new_err)?;
    payload
        .get("timestamp")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| PyRuntimeError::new_err("Rust utc_now did not return a timestamp."))
}

#[pyfunction(name = "generate_namespaced_workflow_id")]
fn generate_namespaced_workflow_id_py(
    family: &str,
    namespace_prefix: Option<&str>,
) -> PyResult<String> {
    let payload = build_plan_workflow_id_payload_json(
        &json!({
            "family": family,
            "namespace_prefix": namespace_prefix,
            "timestamp_ms": null,
            "randomness_hex": null,
        })
        .to_string(),
    )
    .map_err(PyValueError::new_err)?;
    payload
        .get("generated_id")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            PyRuntimeError::new_err("Rust generate_namespaced_workflow_id did not return an id.")
        })
}

#[pyfunction(name = "normalize_id_namespace_prefix")]
#[pyo3(signature = (value, *, default=None))]
fn normalize_id_namespace_prefix_py(
    value: Option<Bound<'_, PyAny>>,
    default: Option<&str>,
) -> PyResult<String> {
    let normalized = normalize_namespace_prefix_value(value);
    normalize_id_namespace_prefix(
        normalized.as_deref(),
        default.or(Some(DEFAULT_ID_NAMESPACE_PREFIX)),
    )
    .map_err(PyValueError::new_err)
}

#[pyfunction(name = "workflow_id_tokens")]
#[pyo3(signature = (family, namespace_prefix=None, *, include_legacy=true))]
fn workflow_id_tokens_py(
    family: &str,
    namespace_prefix: Option<&str>,
    include_legacy: bool,
) -> PyResult<Vec<String>> {
    workflow_id_tokens(family, namespace_prefix, include_legacy).map_err(PyValueError::new_err)
}

#[pyfunction(name = "workflow_origin_namespace_prefix")]
fn workflow_origin_namespace_prefix_py(
    origin_prefix: &str,
    namespace_prefix: Option<&str>,
) -> PyResult<String> {
    workflow_origin_namespace_prefix(origin_prefix, namespace_prefix).map_err(PyValueError::new_err)
}

#[pyfunction(name = "workflow_id_namespace_prefix_candidates")]
#[pyo3(signature = (namespace_prefix=None, *, include_legacy=true, include_task_change_origins=false))]
fn workflow_id_namespace_prefix_candidates_py(
    namespace_prefix: Option<&str>,
    include_legacy: bool,
    include_task_change_origins: bool,
) -> PyResult<Vec<String>> {
    workflow_id_namespace_prefix_candidates(
        namespace_prefix,
        include_legacy,
        include_task_change_origins,
    )
    .map_err(PyValueError::new_err)
}

#[pyfunction(name = "workflow_id_namespace_prefix_for_value")]
#[pyo3(signature = (value, family, namespace_prefix=None, *, include_legacy=true, include_task_change_origins=false))]
fn workflow_id_namespace_prefix_for_value_py(
    py: Python<'_>,
    value: Option<&str>,
    family: &str,
    namespace_prefix: Option<&str>,
    include_legacy: bool,
    include_task_change_origins: bool,
) -> PyResult<Py<PyAny>> {
    let result = workflow_id_namespace_prefix_for_value(
        value,
        family,
        namespace_prefix,
        include_legacy,
        include_task_change_origins,
    )
    .map_err(PyValueError::new_err)?;
    Ok(match result {
        Some(value) => value.into_pyobject(py)?.unbind().into_any(),
        None => py.None(),
    })
}

#[pyfunction(name = "workflow_id_matches_any_namespace_prefix")]
#[pyo3(signature = (value, family, namespace_prefix=None, *, include_legacy=true, include_task_change_origins=false))]
fn workflow_id_matches_any_namespace_prefix_py(
    value: Option<&str>,
    family: &str,
    namespace_prefix: Option<&str>,
    include_legacy: bool,
    include_task_change_origins: bool,
) -> PyResult<bool> {
    workflow_id_matches_any_namespace_prefix(
        value,
        family,
        namespace_prefix,
        include_legacy,
        include_task_change_origins,
    )
    .map_err(PyValueError::new_err)
}

#[pyfunction(name = "derive_patchset_id")]
fn derive_patchset_id_py(
    change_id: &str,
    patchset_number: i64,
    namespace_prefix: Option<&str>,
) -> PyResult<String> {
    derive_patchset_id(change_id, patchset_number, namespace_prefix).map_err(PyValueError::new_err)
}

#[pyfunction(name = "generate_workflow_id")]
fn generate_workflow_id_py(prefix: &str) -> PyResult<String> {
    generate_workflow_id(prefix).map_err(PyValueError::new_err)
}

#[pyfunction(name = "policy_profile_names")]
fn policy_profile_names_py() -> Vec<String> {
    rust_policy::policy_profile_names()
}

#[pyfunction(name = "author_mode_values")]
fn author_mode_values_py() -> Vec<String> {
    rust_policy::author_mode_values()
}

#[pyfunction(name = "policy_content_class_values")]
fn policy_content_class_values_py() -> Vec<String> {
    rust_policy::policy_content_class_values()
}

#[pyfunction(name = "policy_author_class_values")]
fn policy_author_class_values_py() -> Vec<String> {
    rust_policy::policy_author_class_values()
}

#[pyfunction(name = "normalize_author_mode")]
fn normalize_author_mode_py(value: &str) -> PyResult<String> {
    rust_policy::normalize_author_mode(value).map_err(PyValueError::new_err)
}

#[pyfunction(name = "missing_code_review_summary_sections")]
fn missing_code_review_summary_sections_py(value: Option<Bound<'_, PyAny>>) -> Vec<String> {
    let text = normalize_optional_text_value(value);
    rust_policy::missing_code_review_summary_sections(text.as_deref())
}

#[pyfunction(name = "is_structured_code_review_summary")]
fn is_structured_code_review_summary_py(value: Option<Bound<'_, PyAny>>) -> bool {
    let text = normalize_optional_text_value(value);
    rust_policy::is_structured_code_review_summary(text.as_deref())
}

#[pyfunction(name = "render_code_review_summary_template")]
#[pyo3(signature = (style="inline"))]
fn render_code_review_summary_template_py(style: &str) -> PyResult<String> {
    rust_policy::render_code_review_summary_template(Some(style))
        .map(str::to_string)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "code_review_summary_requirement_text")]
fn code_review_summary_requirement_text_py(value: Option<Bound<'_, PyAny>>) -> String {
    let text = normalize_optional_text_value(value);
    rust_policy::code_review_summary_requirement_text(text.as_deref())
}

#[pyfunction(name = "derive_policy_content_class")]
fn derive_policy_content_class_py(changed_paths: Option<Bound<'_, PyAny>>) -> PyResult<String> {
    let payload = match changed_paths {
        Some(value) if !value.is_none() => parse_json_value(value, "changed_paths")?,
        _ => JsonValue::Null,
    };
    Ok(rust_policy::derive_policy_content_class(Some(&payload)))
}

#[pyfunction(name = "derive_policy_author_class")]
fn derive_policy_author_class_py(py: Python<'_>, author_mode: Option<&str>) -> PyResult<Py<PyAny>> {
    Ok(match rust_policy::derive_policy_author_class(author_mode) {
        Some(value) => value.into_pyobject(py)?.unbind().into_any(),
        None => py.None(),
    })
}

#[pyfunction(name = "build_minimum_provenance")]
#[pyo3(signature = (author_mode, *, model_name=None))]
fn build_minimum_provenance_py(
    py: Python<'_>,
    author_mode: &str,
    model_name: Option<&str>,
) -> PyResult<Py<PyTuple>> {
    let (summary, detail) = rust_policy::build_minimum_provenance(author_mode, model_name)
        .map_err(PyValueError::new_err)?;
    let tuple = PyTuple::new(
        py,
        [
            json_value_to_py(py, &summary)?,
            json_value_to_py(py, &detail)?,
        ],
    )?;
    Ok(tuple.unbind())
}

#[pyfunction(name = "policy_profile")]
fn policy_profile_py(py: Python<'_>, name: &str) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        rust_policy::policy_profile(name).map_err(PyKeyError::new_err)?,
    )
}

#[pyfunction(name = "normalize_policy")]
#[pyo3(signature = (policy=None, *, fallback_profile="prototype"))]
fn normalize_policy_py(
    py: Python<'_>,
    policy: Option<Bound<'_, PyAny>>,
    fallback_profile: &str,
) -> PyResult<Py<PyDict>> {
    let payload = optional_json_value(policy, "policy")?;
    render_json_dict(
        py,
        rust_policy::normalize_policy(payload.as_ref(), fallback_profile)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "resolve_effective_policy")]
#[pyo3(signature = (policy=None, *, content_class=None, author_class=None, fallback_profile="prototype"))]
fn resolve_effective_policy_py(
    py: Python<'_>,
    policy: Option<Bound<'_, PyAny>>,
    content_class: Option<&str>,
    author_class: Option<&str>,
    fallback_profile: &str,
) -> PyResult<Py<PyDict>> {
    let payload = optional_json_value(policy, "policy")?;
    render_json_dict(
        py,
        rust_policy::resolve_effective_policy(
            payload.as_ref(),
            content_class,
            author_class,
            fallback_profile,
        )
        .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "policy_to_yaml")]
#[pyo3(signature = (policy=None, *, fallback_profile="prototype"))]
fn policy_to_yaml_py(policy: Option<Bound<'_, PyAny>>, fallback_profile: &str) -> PyResult<String> {
    let payload = optional_json_value(policy, "policy")?;
    rust_policy::policy_to_yaml(payload.as_ref(), fallback_profile).map_err(PyValueError::new_err)
}

#[pyfunction(name = "parse_policy_yaml")]
#[pyo3(signature = (text, *, fallback_profile="prototype"))]
fn parse_policy_yaml_py(
    py: Python<'_>,
    text: &str,
    fallback_profile: &str,
) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        rust_policy::parse_policy_yaml(text, fallback_profile).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "runtime_data_env_value")]
fn runtime_data_env_value_py(py: Python<'_>) -> PyResult<Py<PyAny>> {
    Ok(match rust_runtime_roots::runtime_data_env_value() {
        Some(value) => value.into_pyobject(py)?.unbind().into_any(),
        None => py.None(),
    })
}

#[pyfunction(name = "runtime_data_env_name")]
fn runtime_data_env_name_py(py: Python<'_>) -> PyResult<Py<PyAny>> {
    Ok(match rust_runtime_roots::runtime_data_env_name() {
        Some(value) => value.into_pyobject(py)?.unbind().into_any(),
        None => py.None(),
    })
}

#[pyfunction(name = "resolve_runtime_data_root_with_source")]
fn resolve_runtime_data_root_with_source_py(
    py: Python<'_>,
    path: Option<&str>,
) -> PyResult<Py<PyTuple>> {
    let (root, source) = rust_runtime_roots::resolve_runtime_data_root_with_source(path)
        .map_err(PyRuntimeError::new_err)?;
    Ok(PyTuple::new(py, [root.to_string_lossy().to_string(), source.to_string()])?.unbind())
}

#[pyfunction(name = "resolve_runtime_data_root")]
fn resolve_runtime_data_root_py(path: Option<&str>) -> PyResult<String> {
    rust_runtime_roots::resolve_runtime_data_root(path)
        .map(|root| root.to_string_lossy().to_string())
        .map_err(PyRuntimeError::new_err)
}

#[pyfunction(name = "resolve_server_runtime_root_with_source")]
fn resolve_server_runtime_root_with_source_py(
    py: Python<'_>,
    path: Option<&str>,
) -> PyResult<Py<PyTuple>> {
    let (root, source) = rust_runtime_roots::resolve_server_runtime_root_with_source(path)
        .map_err(PyRuntimeError::new_err)?;
    Ok(PyTuple::new(py, [root.to_string_lossy().to_string(), source.to_string()])?.unbind())
}

#[pyfunction(name = "resolve_server_runtime_root")]
fn resolve_server_runtime_root_py(path: Option<&str>) -> PyResult<String> {
    rust_runtime_roots::resolve_server_runtime_root(path)
        .map(|root| root.to_string_lossy().to_string())
        .map_err(PyRuntimeError::new_err)
}

#[pyfunction(name = "encode_ref_name")]
fn encode_ref_name_py(name: &str) -> String {
    rust_ref_names::encode_ref_name(name)
}

#[pyfunction(name = "decode_ref_name")]
fn decode_ref_name_py(name: &str) -> String {
    rust_ref_names::decode_ref_name(name)
}

#[pyfunction(name = "task_status_details")]
fn task_status_details_py(py: Python<'_>, value: Option<&str>) -> PyResult<Py<PyDict>> {
    Ok(render_workflow_status_details(
        py,
        task_status_details(value).map_err(PyValueError::new_err)?,
    )?
    .unbind())
}

#[pyfunction(name = "normalize_workflow_mode")]
fn normalize_workflow_mode_py(py: Python<'_>, value: Option<&str>) -> PyResult<Py<PyAny>> {
    let normalized = workflow_mode_value(value).map_err(PyValueError::new_err)?;
    Ok(match normalized {
        Some(value) => value.into_pyobject(py)?.unbind().into_any(),
        None => py.None(),
    })
}

#[pyfunction(name = "normalize_publication_state")]
fn normalize_publication_state_py(py: Python<'_>, value: Option<&str>) -> PyResult<Py<PyAny>> {
    let normalized = publication_state_value(value).map_err(PyValueError::new_err)?;
    Ok(match normalized {
        Some(value) => value.into_pyobject(py)?.unbind().into_any(),
        None => py.None(),
    })
}

#[pyfunction(name = "publication_state_has_unpublished_head")]
fn publication_state_has_unpublished_head_py(value: Option<&str>) -> PyResult<bool> {
    publication_state_has_unpublished_head(value).map_err(PyValueError::new_err)
}

#[pyfunction(name = "workflow_id_token")]
fn workflow_id_token_py(family: &str, namespace_prefix: Option<&str>) -> PyResult<String> {
    workflow_id_token(family, namespace_prefix).map_err(PyValueError::new_err)
}

#[pyfunction(name = "workflow_id_matches")]
fn workflow_id_matches_py(
    value: Option<&str>,
    family: &str,
    namespace_prefix: Option<&str>,
    include_legacy: Option<bool>,
) -> PyResult<bool> {
    workflow_id_matches(
        value,
        family,
        namespace_prefix,
        include_legacy.unwrap_or(true),
    )
    .map_err(PyValueError::new_err)
}

#[pyfunction(name = "generate_namespaced_sequence_id")]
fn generate_namespaced_sequence_id_py(
    family: &str,
    number: i64,
    namespace_prefix: Option<&str>,
    width: Option<usize>,
) -> PyResult<String> {
    generate_namespaced_sequence_id(family, number, namespace_prefix, width.unwrap_or(4))
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "workflow_success_envelope")]
fn workflow_success_envelope_py(
    py: Python<'_>,
    kind: &str,
    value: Option<&str>,
) -> PyResult<Py<PyDict>> {
    Ok(render_workflow_result_envelope(
        py,
        workflow_success_envelope(kind, value).map_err(PyValueError::new_err)?,
    )?
    .unbind())
}

#[pyfunction(name = "workflow_error_envelope")]
fn workflow_error_envelope_py(
    py: Python<'_>,
    kind: &str,
    code: &str,
    message: &str,
    detail: Option<&str>,
) -> PyResult<Py<PyDict>> {
    Ok(render_workflow_result_envelope(
        py,
        workflow_error_envelope(kind, code, message, detail).map_err(PyValueError::new_err)?,
    )?
    .unbind())
}

#[pyfunction(name = "parse_plan_markdown")]
fn parse_plan_markdown_py(py: Python<'_>, markdown: Option<&str>) -> PyResult<Py<PyDict>> {
    Ok(render_parsed_plan(py, parse_plan_markdown(markdown))?.unbind())
}

#[pyfunction(name = "extract_plan_refs")]
fn extract_plan_refs_py(py: Python<'_>, parsed_plan: Bound<'_, PyDict>) -> PyResult<Py<PyDict>> {
    let parsed = parse_parsed_plan(parsed_plan)?;
    Ok(render_plan_ref_identity_payload(py, extract_plan_refs(&parsed))?.unbind())
}

#[pyfunction(name = "compute_sync_prune_decisions")]
fn compute_sync_prune_decisions_py(
    py: Python<'_>,
    scope: Option<&str>,
    tracked_artifacts: Option<Bound<'_, PyAny>>,
    synced_artifacts: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let tracked = parse_string_list(tracked_artifacts)?;
    let synced = parse_string_list(synced_artifacts)?;
    Ok(render_sync_prune_decisions(
        py,
        compute_sync_prune_decisions(scope, &tracked, &synced).map_err(PyValueError::new_err)?,
    )?
    .unbind())
}

#[pyfunction(name = "diff_snapshot_manifests")]
fn diff_snapshot_manifests_py(
    py: Python<'_>,
    old_files: Bound<'_, PyAny>,
    new_files: Bound<'_, PyAny>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let old_payload = parse_json_value(old_files, "old_files")?;
    let new_payload = parse_json_value(new_files, "new_files")?;
    render_json_dict(
        py,
        diff_snapshot_manifests(&old_payload, &new_payload, old_snapshot_id, new_snapshot_id)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "snapshot_diff_from_manifests")]
#[pyo3(signature = (old_files, new_files, blob_bytes_by_id=None, old_snapshot_id=None, new_snapshot_id=None, include_text=false, max_bytes=None))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn snapshot_diff_from_manifests_py(
    py: Python<'_>,
    old_files: Bound<'_, PyAny>,
    new_files: Bound<'_, PyAny>,
    blob_bytes_by_id: Option<Bound<'_, PyAny>>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    include_text: bool,
    max_bytes: Option<usize>,
) -> PyResult<Py<PyDict>> {
    let old_payload = parse_json_value(old_files, "old_files")?;
    let new_payload = parse_json_value(new_files, "new_files")?;
    let blob_bytes = parse_blob_bytes_map(blob_bytes_by_id)?;
    render_json_dict(
        py,
        snapshot_diff_from_manifests(
            &old_payload,
            &new_payload,
            &blob_bytes,
            old_snapshot_id,
            new_snapshot_id,
            include_text,
            max_bytes.unwrap_or(DEFAULT_SNAPSHOT_DIFF_MAX_BYTES),
        )
        .map_err(PyValueError::new_err)?,
    )
}

fn register_workflow_policy(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(normalize_optional_text_py, module)?)?;
    module.add_function(wrap_pyfunction!(utc_now_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        generate_namespaced_workflow_id_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(normalize_id_namespace_prefix_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_id_tokens_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        workflow_origin_namespace_prefix_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        workflow_id_namespace_prefix_candidates_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        workflow_id_namespace_prefix_for_value_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        workflow_id_matches_any_namespace_prefix_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(derive_patchset_id_py, module)?)?;
    module.add_function(wrap_pyfunction!(generate_workflow_id_py, module)?)?;
    module.add_function(wrap_pyfunction!(policy_profile_names_py, module)?)?;
    module.add_function(wrap_pyfunction!(author_mode_values_py, module)?)?;
    module.add_function(wrap_pyfunction!(policy_content_class_values_py, module)?)?;
    module.add_function(wrap_pyfunction!(policy_author_class_values_py, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_author_mode_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        missing_code_review_summary_sections_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        is_structured_code_review_summary_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        render_code_review_summary_template_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        code_review_summary_requirement_text_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(derive_policy_content_class_py, module)?)?;
    module.add_function(wrap_pyfunction!(derive_policy_author_class_py, module)?)?;
    module.add_function(wrap_pyfunction!(build_minimum_provenance_py, module)?)?;
    module.add_function(wrap_pyfunction!(policy_profile_py, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_policy_py, module)?)?;
    module.add_function(wrap_pyfunction!(resolve_effective_policy_py, module)?)?;
    module.add_function(wrap_pyfunction!(policy_to_yaml_py, module)?)?;
    module.add_function(wrap_pyfunction!(parse_policy_yaml_py, module)?)?;
    module.add_function(wrap_pyfunction!(runtime_data_env_value_py, module)?)?;
    module.add_function(wrap_pyfunction!(runtime_data_env_name_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        resolve_runtime_data_root_with_source_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(resolve_runtime_data_root_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        resolve_server_runtime_root_with_source_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(resolve_server_runtime_root_py, module)?)?;
    module.add_function(wrap_pyfunction!(encode_ref_name_py, module)?)?;
    module.add_function(wrap_pyfunction!(decode_ref_name_py, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_task_status_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_status_details_py, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_workflow_mode_py, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_publication_state_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        publication_state_has_unpublished_head_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(workflow_id_token_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_id_matches_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        generate_namespaced_sequence_id_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(workflow_success_envelope_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_error_envelope_py, module)?)?;
    module.add_function(wrap_pyfunction!(parse_plan_markdown_py, module)?)?;
    module.add_function(wrap_pyfunction!(extract_plan_refs_py, module)?)?;
    module.add_function(wrap_pyfunction!(compute_sync_prune_decisions_py, module)?)?;
    module.add_function(wrap_pyfunction!(diff_snapshot_manifests_py, module)?)?;
    module.add_function(wrap_pyfunction!(snapshot_diff_from_manifests_py, module)?)?;
    Ok(())
}
