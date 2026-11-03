#[pyfunction(name = "workspace_diff_operation")]
#[pyo3(signature = (repo_root, paths=Vec::new()))]
fn workspace_diff_operation_py(py: Python<'_>, repo_root: &str, paths: Vec<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::primitives::workspace_dirty_diff(&repo, &paths, DEFAULT_SNAPSHOT_DIFF_MAX_BYTES)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_blame")]
fn format_blame_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Inspection result")?;
    Ok(ait_cli::binding_output::format_blame(&payload))
}

#[pyfunction(name = "render_workspace_diff_names")]
fn render_workspace_diff_names_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Inspection result")?;
    ait_cli::binding_output::render_workspace_diff_names(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "render_workspace_diff_stat")]
fn render_workspace_diff_stat_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Inspection result")?;
    ait_cli::binding_output::render_workspace_diff_stat(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "render_workspace_diff_text")]
fn render_workspace_diff_text_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Inspection result")?;
    ait_cli::binding_output::render_workspace_diff_text(&payload).map_err(PyValueError::new_err)
}

fn register_binding_inspection(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(workspace_diff_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_blame_py, module)?)?;
    module.add_function(wrap_pyfunction!(render_workspace_diff_names_py, module)?)?;
    module.add_function(wrap_pyfunction!(render_workspace_diff_stat_py, module)?)?;
    module.add_function(wrap_pyfunction!(render_workspace_diff_text_py, module)?)?;
    Ok(())
}
