#[pyfunction(name = "external_status_operation")]
#[pyo3(signature = (repo_root))]
#[allow(clippy::too_many_arguments)]
fn external_status_operation_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let result = py
        .detach(|| ait_cli::external_surface::external_status(&repo))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, result)
}

#[pyfunction(name = "external_doctor_operation")]
#[pyo3(signature = (repo_root, *, fail_on_blocking=false))]
#[allow(clippy::too_many_arguments)]
fn external_doctor_operation_py(
    py: Python<'_>,
    repo_root: &str,
    fail_on_blocking: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let result = py
        .detach(|| ait_cli::binding_external::diagnose_external(&repo, fail_on_blocking))
        .map_err(PyValueError::new_err)?;
    render_workflow_operation(py, result)
}

#[pyfunction(name = "external_update_operation")]
#[pyo3(signature = (repo_root, *, name=None, snapshot=None, latest=false, locked=false, validate=false, no_recursive=false))]
#[allow(clippy::too_many_arguments)]
fn external_update_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    snapshot: Option<&str>,
    latest: bool,
    locked: bool,
    validate: bool,
    no_recursive: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_external::ExternalUpdateRequest {
        name: name.map(str::to_string),
        snapshot: snapshot.map(str::to_string),
        latest,
        locked,
        validate,
        no_recursive,
    };
    let result = py
        .detach(|| ait_cli::binding_external::update_external(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, result)
}

#[pyfunction(name = "external_link_operation")]
#[pyo3(signature = (repo_root, name, path))]
#[allow(clippy::too_many_arguments)]
fn external_link_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: &str,
    path: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let result = py
        .detach(|| ait_cli::external_surface::external_link(&repo, name, path))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, result)
}

#[pyfunction(name = "external_unlink_operation")]
#[pyo3(signature = (repo_root, name))]
#[allow(clippy::too_many_arguments)]
fn external_unlink_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let result = py
        .detach(|| ait_cli::external_surface::external_unlink(&repo, name))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, result)
}

#[pyfunction(name = "format_external_status")]
fn format_external_status_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "external payload")?;
    ait_cli::external_surface::render_external_status_text(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_external_doctor")]
fn format_external_doctor_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "external payload")?;
    ait_cli::external_surface::render_external_doctor_text(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_external_update")]
fn format_external_update_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "external payload")?;
    ait_cli::external_surface::render_external_update_text(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_external_link")]
fn format_external_link_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "external payload")?;
    ait_cli::external_surface::render_external_link_text(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_external_unlink")]
fn format_external_unlink_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "external payload")?;
    ait_cli::external_surface::render_external_unlink_text(&payload).map_err(PyValueError::new_err)
}

fn register_binding_external(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(external_status_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(external_doctor_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(external_update_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(external_link_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(external_unlink_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_external_status_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_external_doctor_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_external_update_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_external_link_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_external_unlink_py, module)?)?;
    Ok(())
}
