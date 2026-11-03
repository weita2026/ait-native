#[pyfunction(name = "change_create_operation")]
#[pyo3(signature = (repo_root, task_id, title, *, base_line=None, fork_snapshot=None, local=false, remote_name=None))]
#[expect(clippy::too_many_arguments, reason = "matches the existing operation options")]
fn change_create_operation_py(py: Python<'_>, repo_root: &str, task_id: &str, title: &str, base_line: Option<String>, fork_snapshot: Option<String>, local: bool, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_changes::create_change(&repo, &ait_cli::binding_changes::ChangeCreateRequest { task_id:task_id.into(), title:title.into(), base_line, fork_snapshot, scope:ait_cli::binding_changes::ChangeScope { local, remote_name } })).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "change_show_operation")]
#[pyo3(signature = (repo_root, reference, *, local=false, remote_name=None))]
fn change_show_operation_py(py: Python<'_>, repo_root: &str, reference: &str, local: bool, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_changes::inspect_change(&repo, reference, &ait_cli::binding_changes::ChangeScope { local, remote_name })).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "change_close_operation")]
#[pyo3(signature = (repo_root, reference, *, local=false, remote_name=None))]
fn change_close_operation_py(py: Python<'_>, repo_root: &str, reference: &str, local: bool, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_changes::close_change(&repo, reference, &ait_cli::binding_changes::ChangeScope { local, remote_name })).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "change_revert_operation")]
#[pyo3(signature = (repo_root, reference, *, force=false, dry_run=false, local=false, remote_name=None))]
fn change_revert_operation_py(py: Python<'_>, repo_root: &str, reference: &str, force: bool, dry_run: bool, local: bool, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_changes::revert_change(&repo, reference, &ait_cli::binding_changes::ChangeMutationOptions { force, dry_run, scope: ait_cli::binding_changes::ChangeScope { local, remote_name } })).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "change_replay_operation")]
#[pyo3(signature = (repo_root, reference, *, onto=None, force=false, dry_run=false, local=false, remote_name=None))]
#[expect(clippy::too_many_arguments, reason = "matches the existing operation options")]
fn change_replay_operation_py(py: Python<'_>, repo_root: &str, reference: &str, onto: Option<String>, force: bool, dry_run: bool, local: bool, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_changes::replay_change(&repo, reference, onto.as_deref(), &ait_cli::binding_changes::ChangeMutationOptions { force, dry_run, scope: ait_cli::binding_changes::ChangeScope { local, remote_name } })).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "change_publish_operation")]
#[pyo3(signature = (repo_root, reference, *, remote_name=None))]
fn change_publish_operation_py(py: Python<'_>, repo_root: &str, reference: &str, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_changes::publish_change(&repo, reference, remote_name.as_deref())).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "patchset_publish_operation")]
#[pyo3(signature = (repo_root, reference, summary, *, author_mode=None, remote_name=None))]
fn patchset_publish_operation_py(py: Python<'_>, repo_root: &str, reference: &str, summary: &str, author_mode: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_changes::publish_patchset(&repo, reference, summary, author_mode.as_deref(), remote_name.as_deref())).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "patchset_list_operation")]
#[pyo3(signature = (repo_root, reference, *, remote_name=None))]
fn patchset_list_operation_py(py: Python<'_>, repo_root: &str, reference: &str, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_changes::list_patchsets(&repo, reference, remote_name.as_deref())).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

macro_rules! change_text_output {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        fn $name(payload: Bound<'_, PyAny>) -> PyResult<String> {
            let payload = parse_json_value(payload, "Change/Patchset result")?;
            $target(&payload).map_err(PyValueError::new_err)
        }
    )*};
}
change_text_output! {
    (format_patchset_show_py, "format_patchset_show", ait_cli::binding_output::format_patchset_show),
    (format_patchset_ci_status_py, "format_patchset_ci_status", ait_cli::binding_output::format_patchset_ci_status),
    (format_patchset_list_py, "format_patchset_list", ait_cli::binding_output::format_patchset_list),
    (format_change_create_py, "format_change_create", ait_cli::binding_output::format_change_create),
    (format_change_show_py, "format_change_show", ait_cli::binding_output::format_change_show),
    (format_change_revert_py, "format_change_revert", ait_cli::binding_output::format_change_revert),
    (format_change_replay_py, "format_change_replay", ait_cli::binding_output::format_change_replay),
    (format_change_close_py, "format_change_close", ait_cli::binding_output::format_change_close),
    (format_change_publish_py, "format_change_publish", ait_cli::binding_output::format_change_publish),
    (format_patchset_publish_py, "format_patchset_publish", ait_cli::binding_output::format_patchset_publish),
    (format_patchset_select_py, "format_patchset_select", ait_cli::binding_output::format_patchset_select),
    (format_patchset_rerun_ci_py, "format_patchset_rerun_ci", ait_cli::binding_output::format_patchset_rerun_ci),
}

#[pyfunction(name = "project_change_list")]
#[pyo3(signature = (payload, *, all=false))]
fn project_change_list_py(py: Python<'_>, payload: Bound<'_, PyAny>, all: bool) -> PyResult<Py<PyAny>> {
    let payload = parse_json_value(payload, "Change inventory")?;
    render_json_value(py, ait_cli::binding_output::project_change_list(&payload, all))
}

#[pyfunction(name = "format_change_list")]
#[pyo3(signature = (payload, *, all=false, local=false, remote_name=None))]
fn format_change_list_py(payload: Bound<'_, PyAny>, all: bool, local: bool, remote_name: Option<&str>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Change inventory")?;
    ait_cli::binding_output::format_change_list(&payload, all, &binding_plan_scope(local, remote_name)).map_err(PyValueError::new_err)
}

fn register_binding_changes(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(change_create_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(change_show_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(change_close_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(change_revert_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(change_replay_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(change_publish_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(patchset_publish_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(patchset_list_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_change_create_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_change_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_change_revert_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_change_replay_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_change_close_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_change_publish_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_patchset_publish_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_patchset_select_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_patchset_rerun_ci_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_patchset_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_patchset_ci_status_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_patchset_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_change_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_change_list_py, module)?)?;
    Ok(())
}
