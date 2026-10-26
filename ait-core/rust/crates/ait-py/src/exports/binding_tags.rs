#[pyfunction(name = "tag_create_operation")]
#[pyo3(signature = (repo_root, name, message, *, snapshot_id=None))]
fn tag_create_operation_py(py: Python<'_>, repo_root: &str, name: &str, message: &str, snapshot_id: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::tag_surface::tag_create(&repo, ait_cli::tag_surface::TagCreateRequest { name:name.into(), message:message.into(), snapshot_id })).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "tag_list_operation")]
#[pyo3(signature = (repo_root))]
fn tag_list_operation_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::tag_surface::tag_list(&repo)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "tag_show_operation")]
#[pyo3(signature = (repo_root, name))]
fn tag_show_operation_py(py: Python<'_>, repo_root: &str, name: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::tag_surface::tag_show(&repo, name)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "tag_delete_operation")]
#[pyo3(signature = (repo_root, name))]
fn tag_delete_operation_py(py: Python<'_>, repo_root: &str, name: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::tag_surface::tag_delete(&repo, name)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "resolve_snapshot_ref_operation")]
fn resolve_snapshot_ref_operation_py(py: Python<'_>, repo_root: &str, value: &str) -> PyResult<String> {
    let repo = discover_task_workflow_repo(repo_root)?;
    py.detach(|| ait_cli::tag_surface::resolve_snapshot_ref(&repo, value)).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_tag_create")]
fn format_tag_create_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Tag result")?;
    ait_cli::binding_output::format_tag_create(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_tag_list")]
fn format_tag_list_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Tag result")?;
    ait_cli::binding_output::format_tag_list(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_tag_show")]
fn format_tag_show_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Tag result")?;
    ait_cli::binding_output::format_tag_show(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_tag_delete")]
fn format_tag_delete_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Tag result")?;
    ait_cli::binding_output::format_tag_delete(&payload).map_err(PyValueError::new_err)
}

fn register_binding_tags(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(tag_create_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(tag_list_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(tag_show_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(tag_delete_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(resolve_snapshot_ref_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_tag_create_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_tag_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_tag_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_tag_delete_py, module)?)?;
    Ok(())
}
