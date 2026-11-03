#[pyfunction(name = "team_contacts_operation")]
#[pyo3(signature = (repo_root))]
fn team_contacts_operation_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let result = py.detach(|| ait_cli::binding_team::team_contacts(&repo));
    render_workflow_operation(py, result)
}

#[pyfunction(name = "team_session_acknowledge_operation")]
#[pyo3(signature = (repo_root, activation))]
fn team_session_acknowledge_operation_py(
    py: Python<'_>,
    repo_root: &str,
    activation: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let result = py.detach(|| ait_cli::binding_team::acknowledge_team_session(&repo, activation));
    render_workflow_operation(py, result)
}

#[pyfunction(name = "team_session_ensure_operation")]
#[pyo3(signature = (repo_root, agent, *, role=None))]
fn team_session_ensure_operation_py(
    py: Python<'_>,
    repo_root: &str,
    agent: u32,
    role: Option<u32>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let result = py.detach(|| ait_cli::binding_team::ensure_team_session(&repo, agent, role));
    render_workflow_operation(py, result)
}

#[pyfunction(name = "format_team_contacts")]
fn format_team_contacts_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "team payload")?;
    Ok(ait_cli::binding_team::format_team_contacts(&payload))
}

#[pyfunction(name = "format_team_session_acknowledge")]
fn format_team_session_acknowledge_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "team payload")?;
    Ok(ait_cli::binding_team::format_team_session_acknowledge(
        &payload,
    ))
}

#[pyfunction(name = "format_team_session_ensure")]
fn format_team_session_ensure_py(payload: Bound<'_, PyAny>, agent: u32) -> PyResult<String> {
    let payload = parse_json_value(payload, "team payload")?;
    Ok(ait_cli::binding_team::format_team_session_ensure(
        &payload, agent,
    ))
}

fn register_binding_team(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(team_contacts_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        team_session_acknowledge_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(team_session_ensure_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_team_contacts_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        format_team_session_acknowledge_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(format_team_session_ensure_py, module)?)?;
    Ok(())
}
