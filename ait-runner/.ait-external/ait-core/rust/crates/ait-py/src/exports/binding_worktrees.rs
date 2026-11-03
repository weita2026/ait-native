#[pyfunction(name = "worktree_show_operation")]
#[pyo3(signature = (repo_root, name=None))]
fn worktree_show_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_worktrees::show_worktree(&repo, name))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_path_operation")]
#[pyo3(signature = (repo_root, name=None))]
fn worktree_path_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_worktrees::open_worktree_path(&repo, name))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_restore_operation")]
#[pyo3(signature = (repo_root, *, name=None, snapshot_id=None, line_name=None, paths=None, force=false, dry_run=false))]
#[allow(clippy::too_many_arguments)]
fn worktree_restore_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<String>,
    snapshot_id: Option<String>,
    line_name: Option<String>,
    paths: Option<Vec<String>>,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_worktrees::restore_worktree(
                &repo,
                &ait_cli::binding_worktrees::WorktreeRestoreRequest {
                    name,
                    snapshot_id,
                    line_name,
                    paths: paths.unwrap_or_default(),
                    force,
                    dry_run,
                },
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_cleanup_operation")]
#[pyo3(signature = (repo_root, *, older_than=None, cleanup_policy=None, allow_manual_only=false, limit=None, dry_run=false, yes=false))]
#[allow(clippy::too_many_arguments)]
fn worktree_cleanup_operation_py(
    py: Python<'_>,
    repo_root: &str,
    older_than: Option<String>,
    cleanup_policy: Option<String>,
    allow_manual_only: bool,
    limit: Option<usize>,
    dry_run: bool,
    yes: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_worktrees::cleanup_worktrees(
                &repo,
                &ait_cli::binding_worktrees::WorktreeCleanupRequest {
                    older_than,
                    cleanup_policy,
                    allow_manual_only,
                    limit,
                    dry_run,
                    yes,
                },
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_prune_stale_operation")]
#[pyo3(signature = (repo_root, *, dry_run=false, yes=false))]
fn worktree_prune_stale_operation_py(
    py: Python<'_>,
    repo_root: &str,
    dry_run: bool,
    yes: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_worktrees::prune_stale_worktrees(&repo, dry_run, yes))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_sync_operation")]
#[pyo3(signature = (repo_root, *, name=None, line_name=None, all_worktrees=false, force=false, dry_run=false))]
fn worktree_sync_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<String>,
    line_name: Option<String>,
    all_worktrees: bool,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_worktrees::sync_worktree(
                &repo,
                &ait_cli::binding_worktrees::WorktreeSyncRequest {
                    name,
                    line_name,
                    all_worktrees,
                    force,
                    dry_run,
                },
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_recreate_operation")]
#[pyo3(signature = (repo_root, *, name=None, dry_run=false))]
fn worktree_recreate_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_worktrees::recreate_worktree(&repo, name, dry_run))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_recover_task_operation")]
#[pyo3(signature = (repo_root, task_id, *, change_ref=None, remote_name=None, dry_run=false))]
fn worktree_recover_task_operation_py(
    py: Python<'_>,
    repo_root: &str,
    task_id: &str,
    change_ref: Option<&str>,
    remote_name: Option<&str>,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_worktrees::recover_task_worktree(
                &repo,
                task_id,
                change_ref,
                remote_name,
                dry_run,
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_restore_owned_head_operation")]
#[pyo3(signature = (repo_root, *, name=None, dry_run=false))]
fn worktree_restore_owned_head_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_worktrees::restore_worktree_owned_head(&repo, name, dry_run))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_rebase_operation")]
#[pyo3(signature = (repo_root, *, name=None, onto_line=None, continue_rebase=false, abort_rebase=false, dry_run=false))]
fn worktree_rebase_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<String>,
    onto_line: Option<String>,
    continue_rebase: bool,
    abort_rebase: bool,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_worktrees::rebase_worktree(
                &repo,
                &ait_cli::binding_worktrees::WorktreeRebaseRequest {
                    name,
                    onto_line,
                    continue_rebase,
                    abort_rebase,
                    dry_run,
                },
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_remove_operation")]
#[pyo3(signature = (repo_root, names, *, all_stale=false, delete_path=false, force=false, dry_run=false, yes=false))]
#[allow(clippy::too_many_arguments)]
fn worktree_remove_operation_py(
    py: Python<'_>,
    repo_root: &str,
    names: Vec<String>,
    all_stale: bool,
    delete_path: bool,
    force: bool,
    dry_run: bool,
    yes: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_worktrees::remove_worktrees(
                &repo,
                &ait_cli::binding_worktrees::WorktreeRemoveRequest {
                    names,
                    all_stale,
                    delete_path,
                    force,
                    dry_run,
                    yes,
                },
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_worktree_restore")]
fn format_worktree_restore_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_restore(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_show")]
fn format_worktree_show_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_show(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_recreate")]
fn format_worktree_recreate_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_recreate(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_recover_task")]
fn format_worktree_recover_task_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_recover_task(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_restore_owned_head")]
fn format_worktree_restore_owned_head_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_restore_owned_head(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_rebase")]
fn format_worktree_rebase_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_rebase(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_list")]
fn format_worktree_list_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_list(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_doctor")]
fn format_worktree_doctor_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_doctor(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_cleanup_candidates")]
fn format_worktree_cleanup_candidates_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_cleanup_candidates(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_sync")]
fn format_worktree_sync_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_sync(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_prune_stale")]
fn format_worktree_prune_stale_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_prune_stale(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_cleanup")]
fn format_worktree_cleanup_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_cleanup(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_status")]
#[pyo3(signature = (payload, verbose=false))]
fn format_worktree_status_py(payload: Bound<'_, PyAny>, verbose: bool) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_status(&payload, verbose)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_path")]
#[pyo3(signature = (payload, shell_output=false))]
fn format_worktree_path_py(payload: Bound<'_, PyAny>, shell_output: bool) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_path(&payload, shell_output)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_worktree_remove")]
#[pyo3(signature = (payload, all_stale, dry_run, multiple))]
fn format_worktree_remove_py(
    payload: Bound<'_, PyAny>,
    all_stale: bool,
    dry_run: bool,
    multiple: bool,
) -> PyResult<String> {
    let payload = parse_json_value(payload, "Worktree result")?;
    ait_cli::binding_output::format_worktree_remove(&payload, all_stale, dry_run, multiple)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "project_worktree_path")]
fn project_worktree_path_py(py: Python<'_>, payload: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let payload = parse_json_value(payload, "Worktree result")?;
    let payload =
        ait_cli::binding_output::project_worktree_path(&payload).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "worktree_sync_exit_code")]
fn worktree_sync_exit_code_py(payload: Bound<'_, PyAny>, all_worktrees: bool) -> PyResult<u8> {
    let payload = parse_json_value(payload, "Worktree sync result")?;
    Ok(ait_cli::binding_worktrees::worktree_sync_exit_code(
        &payload,
        all_worktrees,
    ))
}

fn register_binding_worktrees(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(worktree_show_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(worktree_path_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(worktree_restore_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(worktree_cleanup_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(worktree_prune_stale_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(worktree_sync_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(worktree_recreate_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        worktree_recover_task_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        worktree_restore_owned_head_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(worktree_rebase_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(worktree_remove_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_restore_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_recreate_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_recover_task_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        format_worktree_restore_owned_head_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(format_worktree_rebase_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_doctor_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        format_worktree_cleanup_candidates_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(format_worktree_sync_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_prune_stale_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_cleanup_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_status_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_path_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_worktree_remove_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_worktree_path_py, module)?)?;
    module.add_function(wrap_pyfunction!(worktree_sync_exit_code_py, module)?)?;
    Ok(())
}
