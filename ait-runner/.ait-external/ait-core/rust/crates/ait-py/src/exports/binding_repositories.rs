#[pyfunction(name = "repository_show_operation")]
#[pyo3(signature = (repo_root, *, remote_name=None))]
fn repository_show_operation_py(py: Python<'_>, repo_root: &str, remote_name: Option<&str>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::repo_surface::inspect_repository(&repo, remote_name)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "repository_jobs_operation")]
#[pyo3(signature = (repo_root, *, remote_name=None, worker_job_index=None, state=None, limit=None))]
fn repository_jobs_operation_py(py: Python<'_>, repo_root: &str, remote_name: Option<String>, worker_job_index: Option<u32>, state: Option<String>, limit: Option<u32>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::repo_surface::RepositoryJobsRequest { remote_name, worker_job_index, state, limit };
    let payload = py.detach(|| ait_cli::repo_surface::repository_jobs(&repo, &request)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "repository_ci_capabilities_operation")]
#[pyo3(signature = (repo_root, *, remote_name=None))]
fn repository_ci_capabilities_operation_py(py: Python<'_>, repo_root: &str, remote_name: Option<&str>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::repo_surface::repository_ci_capabilities(&repo, remote_name)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "repository_retire_operation")]
#[pyo3(signature = (repo_root, *, remote_name=None, abort=false))]
fn repository_retire_operation_py(py: Python<'_>, repo_root: &str, remote_name: Option<String>, abort: bool) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::repository_retirement::RepoRetireRequest { remote_name, abort };
    let payload = py.detach(|| ait_cli::repository_retirement::retire_repository(&repo, &request)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "repository_restore_operation")]
#[pyo3(signature = (repo_root, *, remote_name=None))]
fn repository_restore_operation_py(py: Python<'_>, repo_root: &str, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::repository_retirement::RepoRestoreRequest { remote_name };
    let payload = py.detach(|| ait_cli::repository_retirement::restore_repository(&repo, &request)).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_repository_show")]
#[pyo3(signature = (payload, *, remote_name=None))]
fn format_repository_show_py(payload: Bound<'_, PyAny>, remote_name: Option<&str>) -> PyResult<String> {
    Ok(ait_cli::repo_surface::format_repository_show(&parse_json_value(payload, "Repository result")?, remote_name))
}

#[pyfunction(name = "format_repository_ci_capabilities")]
#[pyo3(signature = (payload, *, remote_name=None))]
fn format_repository_ci_capabilities_py(payload: Bound<'_, PyAny>, remote_name: Option<&str>) -> PyResult<String> {
    Ok(ait_cli::repo_surface::format_repository_ci_capabilities(&parse_json_value(payload, "Repository CI capabilities")?, remote_name))
}

#[pyfunction(name = "format_repository_jobs")]
#[pyo3(signature = (payload, *, remote_name=None, worker_job_index=None, state=None, limit=None))]
fn format_repository_jobs_py(payload: Bound<'_, PyAny>, remote_name: Option<String>, worker_job_index: Option<u32>, state: Option<String>, limit: Option<u32>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Repository Jobs")?;
    ait_cli::repo_surface::format_repository_jobs(&payload, &ait_cli::repo_surface::RepositoryJobsRequest { remote_name, worker_job_index, state, limit }).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_repository_retire")]
fn format_repository_retire_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    Ok(ait_cli::repo_surface::format_repository_retire(&parse_json_value(payload, "Repository retirement")?))
}

#[pyfunction(name = "format_repository_restore")]
fn format_repository_restore_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    Ok(ait_cli::repo_surface::format_repository_restore(&parse_json_value(payload, "Repository restoration")?))
}

fn register_binding_repositories(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(repository_show_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(repository_jobs_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(repository_ci_capabilities_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(repository_retire_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(repository_restore_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_repository_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_repository_ci_capabilities_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_repository_jobs_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_repository_retire_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_repository_restore_py, module)?)?;
    Ok(())
}
