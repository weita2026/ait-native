macro_rules! binding_projection {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        #[pyo3(signature = (payload, *, full=false))]
        fn $name(py: Python<'_>, payload: Bound<'_, PyAny>, full: bool) -> PyResult<Py<PyAny>> {
            let payload = parse_json_value(payload, "native result")?;
            render_json_value(py, $target(&payload, full))
        }
    )*};
}

binding_projection! {
    (project_status_py, "project_status", ait_cli::binding_output::project_status),
    (project_task_start_py, "project_task_start", ait_cli::binding_output::project_task_start),
    (project_task_finish_py, "project_task_finish", ait_cli::binding_output::project_task_finish),
    (project_snapshot_capture_py, "project_snapshot_capture", ait_cli::binding_output::project_snapshot_capture),
}

macro_rules! binding_text_output {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        fn $name(payload: Bound<'_, PyAny>) -> PyResult<String> {
            let payload = parse_json_value(payload, "native result")?;
            $target(&payload).map_err(PyValueError::new_err)
        }
    )*};
}

binding_text_output! {
    (format_task_show_py, "format_task_show", ait_cli::binding_output::format_task_show),
    (format_remote_add_py, "format_remote_add", ait_cli::binding_output::format_remote_add),
    (format_remote_list_py, "format_remote_list", ait_cli::binding_output::format_remote_list),
    (format_queue_summary_py, "format_queue_summary", ait_cli::binding_output::format_queue_summary),
    (format_doctor_memory_root_py, "format_doctor_memory_root", ait_cli::binding_output::format_doctor_memory_root),
    (format_doctor_runtime_root_py, "format_doctor_runtime_root", ait_cli::binding_output::format_doctor_runtime_root),
    (format_doctor_plan_authority_py, "format_doctor_plan_authority", ait_cli::binding_output::format_doctor_plan_authority),
    (format_configuration_show_py, "format_configuration_show", ait_cli::binding_output::format_configuration_show),
    (format_status_py, "format_status", ait_cli::binding_output::format_status),
    (format_task_start_py, "format_task_start", ait_cli::binding_output::format_task_start),
    (format_task_quick_py, "format_task_quick", ait_cli::binding_output::format_task_quick),
    (format_snapshot_capture_py, "format_snapshot_capture", ait_cli::binding_output::format_snapshot_capture),
}

#[pyfunction(name = "project_task_list")]
#[pyo3(signature = (payload, *, all=false))]
fn project_task_list_py(
    py: Python<'_>,
    payload: Bound<'_, PyAny>,
    all: bool,
) -> PyResult<Py<PyAny>> {
    let payload = parse_json_value(payload, "Task inventory")?;
    render_json_value(
        py,
        ait_cli::binding_output::project_task_list(&payload, all),
    )
}

#[pyfunction(name = "format_task_list")]
#[pyo3(signature = (payload, *, all=false, local=false, remote_name=None))]
fn format_task_list_py(
    payload: Bound<'_, PyAny>,
    all: bool,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<String> {
    let payload = parse_json_value(payload, "Task inventory")?;
    ait_cli::binding_output::format_task_list(
        &payload,
        all,
        &binding_plan_scope(local, remote_name),
    )
    .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_initialize_repository")]
fn format_initialize_repository_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "initialization result")?;
    Ok(ait_cli::binding_output::format_initialize_repository(
        &payload,
    ))
}

#[pyfunction(name = "format_configuration")]
fn format_configuration_py(
    payload: Bound<'_, PyAny>,
    request: Bound<'_, PyAny>,
) -> PyResult<String> {
    let payload = parse_json_value(payload, "configuration result")?;
    let request = parse_json_value(request, "configuration request")?;
    ait_cli::binding_output::format_configuration(&payload, &request).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_configuration_unset")]
fn format_configuration_unset_py(payload: Bound<'_, PyAny>, key: &str) -> PyResult<String> {
    let payload = parse_json_value(payload, "configuration result")?;
    ait_cli::binding_output::format_configuration_unset(&payload, key)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_task_finish")]
fn format_task_finish_py(payload: Bound<'_, PyAny>, requested: &str) -> PyResult<String> {
    let payload = parse_json_value(payload, "native Task finish result")?;
    ait_cli::binding_output::format_task_finish(&payload, requested).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_task_audit")]
fn format_task_audit_py(payload: Bound<'_, PyAny>, task_id: &str) -> PyResult<String> {
    let payload = parse_json_value(payload, "Task audit result")?;
    ait_cli::binding_output::format_task_audit(&payload, task_id).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_task_abandon")]
fn format_task_abandon_py(payload: Bound<'_, PyAny>, task_id: &str) -> PyResult<String> {
    let payload = parse_json_value(payload, "Task abandonment result")?;
    ait_cli::binding_output::format_task_abandon(&payload, task_id).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_domain_error")]
fn format_domain_error_py(message: &str) -> String {
    ait_cli::render::domain_error_text(message)
}

#[pyfunction(name = "domain_error_exit_code")]
fn domain_error_exit_code_py() -> u8 {
    ait_cli::render::DOMAIN_ERROR_EXIT_CODE
}

#[pyfunction(name = "encode_json")]
#[pyo3(signature = (value, *, pretty=false))]
fn encode_json_py(value: Bound<'_, PyAny>, pretty: bool) -> PyResult<String> {
    let value = parse_json_value(value, "JSON value")?;
    let options = if pretty {
        ait_core::json_support::JsonEncodeOptions::pretty()
    } else {
        ait_core::json_support::JsonEncodeOptions::compact()
    };
    ait_core::json_support::JsonCodec::encode_value(&value, options)
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

#[pyfunction(name = "decode_json")]
fn decode_json_py(py: Python<'_>, text: &str) -> PyResult<Py<PyAny>> {
    let value =
        ait_core::json_support::JsonCodec::parse_value_with_error_prefix(text, "Invalid JSON")
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
    render_json_value(py, value)
}

#[pyfunction(name = "format_auth_whoami")]
fn format_auth_whoami_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Auth result")?;
    ait_cli::binding_output::format_auth_whoami(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_auth_grant")]
fn format_auth_grant_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Auth result")?;
    ait_cli::binding_output::format_auth_grant(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_auth_bindings")]
fn format_auth_bindings_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Auth result")?;
    ait_cli::binding_output::format_auth_bindings(&payload).map_err(PyValueError::new_err)
}

fn register_binding_output(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(format_task_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_task_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_task_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_remote_add_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_remote_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_queue_summary_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_doctor_memory_root_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_doctor_runtime_root_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_doctor_plan_authority_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_initialize_repository_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_configuration_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_configuration_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_configuration_unset_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_status_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_task_start_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_task_finish_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_snapshot_capture_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_status_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_task_start_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_task_quick_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_task_audit_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_task_abandon_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_task_finish_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_capture_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_domain_error_py, module)?)?;
    module.add_function(wrap_pyfunction!(domain_error_exit_code_py, module)?)?;
    module.add_function(wrap_pyfunction!(encode_json_py, module)?)?;
    module.add_function(wrap_pyfunction!(decode_json_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_auth_whoami_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_auth_grant_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_auth_bindings_py, module)?)?;
    Ok(())
}
