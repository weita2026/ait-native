#[pyfunction(name = "stash_save_operation")]
#[pyo3(signature = (repo_root, *, message=None, keep_workspace=false))]
fn stash_save_operation_py(py: Python<'_>, repo_root: &str, message: Option<String>, keep_workspace: bool) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_services::save_stash(&repo, message.as_deref(), keep_workspace)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "stash_apply_operation")]
#[pyo3(signature = (repo_root, stash_id, *, force=false))]
fn stash_apply_operation_py(py: Python<'_>, repo_root: &str, stash_id: &str, force: bool) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_services::apply_stash(&repo, stash_id, force)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "stash_pop_operation")]
#[pyo3(signature = (repo_root, stash_id, *, force=false))]
fn stash_pop_operation_py(py: Python<'_>, repo_root: &str, stash_id: &str, force: bool) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_services::pop_stash(&repo, stash_id, force)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "stash_drop_operation")]
#[pyo3(signature = (repo_root, stash_id))]
fn stash_drop_operation_py(py: Python<'_>, repo_root: &str, stash_id: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_services::drop_stash(&repo, stash_id)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_stash_save")]
fn format_stash_save_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Stash result")?;
    ait_cli::binding_output::format_stash_save(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_stash_list")]
fn format_stash_list_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Stash result")?;
    ait_cli::binding_output::format_stash_list(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_stash_show")]
fn format_stash_show_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Stash result")?;
    ait_cli::binding_output::format_stash_show(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_stash_apply")]
fn format_stash_apply_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Stash result")?;
    ait_cli::binding_output::format_stash_apply(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_stash_pop")]
fn format_stash_pop_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Stash result")?;
    ait_cli::binding_output::format_stash_pop(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_stash_drop")]
fn format_stash_drop_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Stash result")?;
    ait_cli::binding_output::format_stash_drop(&payload).map_err(PyValueError::new_err)
}

fn register_binding_stash(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(stash_save_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(stash_apply_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(stash_pop_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(stash_drop_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_stash_save_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_stash_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_stash_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_stash_apply_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_stash_pop_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_stash_drop_py, module)?)?;
    Ok(())
}
