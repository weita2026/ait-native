#[pyfunction(name = "git_migrate_operation")]
#[pyo3(signature = (repo_root, staging_root, *, apply=false))]
fn git_migrate_operation_py(
    py: Python<'_>,
    repo_root: &str,
    staging_root: &str,
    apply: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_git::migrate_git(&repo, Path::new(staging_root), apply))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_git_migrate")]
fn format_git_migrate_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Git result")?;
    ait_cli::binding_output::format_git_migrate(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "git_recover_operation")]
#[pyo3(signature = (repo_root, *, apply=false))]
fn git_recover_operation_py(py: Python<'_>, repo_root: &str, apply: bool) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_git::recover_git(&repo, apply))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_git_recover")]
fn format_git_recover_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Git result")?;
    ait_cli::binding_output::format_git_recover(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "git_import_operation")]
#[pyo3(signature = (repo_root, source, *, all_branches_and_tags=false, dry_run=false))]
fn git_import_operation_py(
    py: Python<'_>,
    repo_root: &str,
    source: &str,
    all_branches_and_tags: bool,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_git::import_git(&repo, source, all_branches_and_tags, dry_run))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_git_import")]
fn format_git_import_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Git result")?;
    ait_cli::binding_output::format_git_import(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "git_export_operation")]
#[pyo3(signature = (repo_root, target, *, all_lines_and_tags=false, dry_run=false, generation=None, git_object=None))]
fn git_export_operation_py(
    py: Python<'_>,
    repo_root: &str,
    target: &str,
    all_lines_and_tags: bool,
    dry_run: bool,
    generation: Option<&str>,
    git_object: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_git::export_git(
                &repo,
                target,
                all_lines_and_tags,
                dry_run,
                generation,
                git_object,
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_git_export")]
fn format_git_export_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Git result")?;
    ait_cli::binding_output::format_git_export(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "git_restore_operation")]
#[pyo3(signature = (repo_root, generation, git_object, target))]
fn git_restore_operation_py(
    py: Python<'_>,
    repo_root: &str,
    generation: &str,
    git_object: &str,
    target: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_git::restore_git(&repo, generation, git_object, Path::new(target))
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_git_restore")]
fn format_git_restore_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Git result")?;
    ait_cli::binding_output::format_git_restore(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "git_mirror_operation")]
#[pyo3(signature = (repo_root, endpoint, direction, *, dry_run=false))]
fn git_mirror_operation_py(
    py: Python<'_>,
    repo_root: &str,
    endpoint: &str,
    direction: &str,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_git::mirror_git(&repo, endpoint, direction, dry_run))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_git_mirror")]
fn format_git_mirror_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Git result")?;
    ait_cli::binding_output::format_git_mirror(&payload).map_err(PyValueError::new_err)
}

fn register_binding_git(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(git_migrate_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_git_migrate_py, module)?)?;
    module.add_function(wrap_pyfunction!(git_recover_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_git_recover_py, module)?)?;
    module.add_function(wrap_pyfunction!(git_import_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_git_import_py, module)?)?;
    module.add_function(wrap_pyfunction!(git_export_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_git_export_py, module)?)?;
    module.add_function(wrap_pyfunction!(git_restore_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_git_restore_py, module)?)?;
    module.add_function(wrap_pyfunction!(git_mirror_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_git_mirror_py, module)?)?;
    Ok(())
}
