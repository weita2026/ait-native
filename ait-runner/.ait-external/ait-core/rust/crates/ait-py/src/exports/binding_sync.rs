#[pyfunction(name = "pull_operation")]
#[pyo3(signature = (repo_root, *, remote_name=None, line_name=None, merge=false, restore=false, force=false))]
fn pull_operation_py(
    py: Python<'_>,
    repo_root: &str,
    remote_name: Option<String>,
    line_name: Option<String>,
    merge: bool,
    restore: bool,
    force: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_services::pull_repository(
                &repo,
                &ait_cli::binding_services::PullRequest {
                    remote_name,
                    line_name,
                    merge,
                    restore,
                    force,
                },
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "push_operation")]
#[pyo3(signature = (repo_root, *, remote_name=None, line_name=None))]
fn push_operation_py(
    py: Python<'_>,
    repo_root: &str,
    remote_name: Option<String>,
    line_name: Option<String>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_services::push_repository(
                &repo,
                &ait_cli::binding_services::RemoteSyncRequest {
                    remote_name,
                    line_name,
                },
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_pull")]
fn format_pull_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Remote sync result")?;
    ait_cli::binding_output::format_pull(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_push")]
fn format_push_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Remote sync result")?;
    ait_cli::binding_output::format_push(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "project_remote_sync")]
fn project_remote_sync_py(py: Python<'_>, payload: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let payload = parse_json_value(payload, "Remote sync result")?;
    render_json_value(py, ait_cli::binding_output::project_remote_sync(&payload))
}

fn register_binding_sync(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(pull_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(push_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_pull_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_push_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_remote_sync_py, module)?)?;
    Ok(())
}
