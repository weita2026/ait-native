#[pyfunction(name = "binary_db_agent_pool_50_operation")]
#[pyo3(signature = (repo_root, generation_root, *, activate=false))]
fn binary_db_agent_pool_50_operation_py(
    py: Python<'_>,
    repo_root: &str,
    generation_root: &str,
    activate: bool,
) -> PyResult<Py<PyAny>> {
    let repo_root = PathBuf::from(repo_root);
    let generation_root = PathBuf::from(generation_root);
    let payload = py
        .detach(|| {
            ait_cli::binding_binary_db::convert_agent_pool_50(repo_root, generation_root, activate)
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}
#[pyfunction(name = "binary_db_upgrade_u64_seconds_operation")]
#[pyo3(signature = (repo_root, generation_root, source_time_width, *, activate=false))]
fn binary_db_upgrade_u64_seconds_operation_py(
    py: Python<'_>,
    repo_root: &str,
    generation_root: &str,
    source_time_width: &str,
    activate: bool,
) -> PyResult<Py<PyAny>> {
    let repo_root = PathBuf::from(repo_root);
    let generation_root = PathBuf::from(generation_root);
    let source_time_width = source_time_width.to_string();
    let payload = py
        .detach(|| {
            ait_cli::binding_binary_db::upgrade_u64_seconds(
                repo_root,
                generation_root,
                source_time_width,
                activate,
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}
#[pyfunction(name = "format_binary_db_agent_pool_50")]
fn format_binary_db_agent_pool_50_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Agent pool conversion result")?;
    Ok(ait_cli::binding_binary_db::format_agent_pool_50(&payload))
}
#[pyfunction(name = "format_binary_db_upgrade_u64_seconds")]
fn format_binary_db_upgrade_u64_seconds_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "u64-second upgrade result")?;
    Ok(ait_cli::binding_binary_db::format_upgrade_u64_seconds(
        &payload,
    ))
}
fn register_binding_binary_db(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(
        binary_db_agent_pool_50_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        binary_db_upgrade_u64_seconds_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(format_binary_db_agent_pool_50_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        format_binary_db_upgrade_u64_seconds_py,
        module
    )?)?;
    Ok(())
}
