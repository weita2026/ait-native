#[pyfunction(name = "format_attest_put")]
fn format_attest_put_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Governance result")?;
    ait_cli::binding_output::format_attest_put(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_attest_show")]
fn format_attest_show_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Governance result")?;
    ait_cli::binding_output::format_attest_show(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_policy_eval")]
fn format_policy_eval_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Governance result")?;
    ait_cli::binding_output::format_policy_eval(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_policy_waive")]
fn format_policy_waive_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Governance result")?;
    ait_cli::binding_output::format_policy_waive(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_policy_show")]
fn format_policy_show_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Governance result")?;
    ait_cli::binding_output::format_policy_show(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "project_policy_show")]
fn project_policy_show_py(py: Python<'_>, payload: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let payload = parse_json_value(payload, "Policy result")?;
    render_json_value(py, ait_cli::binding_output::project_policy_show(&payload))
}

fn register_binding_governance(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(format_attest_put_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_attest_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_policy_eval_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_policy_waive_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_policy_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_policy_show_py, module)?)?;
    Ok(())
}
