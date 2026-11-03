#[pyfunction(name = "gc_stats_operation")]
#[pyo3(signature = (repo_root))]
fn gc_stats_operation_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_maintenance::gc_stats(&repo))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}
#[pyfunction(name = "format_gc_stats")]
fn format_gc_stats_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "GC result")?;
    ait_cli::binding_output::format_gc_stats(&payload).map_err(PyValueError::new_err)
}
#[pyfunction(name = "gc_validate_operation")]
#[pyo3(signature = (repo_root))]
fn gc_validate_operation_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_maintenance::gc_validate(&repo))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}
#[pyfunction(name = "format_gc_validate")]
fn format_gc_validate_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "GC result")?;
    ait_cli::binding_output::format_gc_validate(&payload).map_err(PyValueError::new_err)
}
#[pyfunction(name = "gc_prune_operation")]
#[pyo3(signature = (repo_root, *, apply=false))]
fn gc_prune_operation_py(py: Python<'_>, repo_root: &str, apply: bool) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_maintenance::gc_prune(&repo, apply))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}
#[pyfunction(name = "format_gc_prune")]
fn format_gc_prune_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "GC result")?;
    ait_cli::binding_output::format_gc_prune(&payload).map_err(PyValueError::new_err)
}
#[pyfunction(name = "gc_validate_exit_code")]
fn gc_validate_exit_code_py(payload: Bound<'_, PyAny>) -> PyResult<u8> {
    let payload = parse_json_value(payload, "GC validate result")?;
    ait_cli::binding_maintenance::gc_validate_exit_code(&payload).map_err(PyValueError::new_err)
}
fn register_binding_maintenance(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(gc_stats_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_gc_stats_py, module)?)?;
    module.add_function(wrap_pyfunction!(gc_validate_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_gc_validate_py, module)?)?;
    module.add_function(wrap_pyfunction!(gc_prune_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_gc_prune_py, module)?)?;
    module.add_function(wrap_pyfunction!(gc_validate_exit_code_py, module)?)?;
    Ok(())
}
