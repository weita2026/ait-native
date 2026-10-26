#[pyfunction(name = "repository_authority_root")]
fn repository_authority_root_py(repo_root: &str) -> PyResult<String> {
    Ok(discover_task_workflow_repo(repo_root)?
        .authoritative_repo_root()
        .to_string_lossy()
        .into_owned())
}

#[pyfunction(name = "unset_configuration")]
fn unset_configuration_py(py: Python<'_>, repo_root: &str, key: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::config_surface::config_unset_from_key(&repo, key))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "doctor_plan_authority_operation")]
fn doctor_plan_authority_operation_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let payload = py
        .detach(|| ait_cli::binding_services::plan_authority_diagnostics(Path::new(repo_root)))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "doctor_memory_root_operation")]
fn doctor_memory_root_operation_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::doctor_surface::doctor_memory_root(&repo))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "task_audit_operation")]
#[pyo3(signature = (repo_root, task_id, *, local=false, remote_name=None))]
fn task_audit_operation_py(
    py: Python<'_>,
    repo_root: &str,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let scope = binding_plan_scope(local, remote_name);
    let payload = py
        .detach(|| ait_cli::binding_services::audit_task(&repo, task_id, &scope))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "task_abandon_operation")]
#[pyo3(signature = (repo_root, task_id, *, local=false, remote_name=None))]
fn task_abandon_operation_py(
    py: Python<'_>,
    repo_root: &str,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let scope = binding_plan_scope(local, remote_name);
    let result = py
        .detach(|| ait_cli::binding_services::abandon_task(&repo, task_id, &scope))
        .map_err(PyValueError::new_err)?;
    render_workflow_operation(py, result)
}

#[pyfunction(name = "initialize_repository")]
#[pyo3(signature = (repo_root, *, name=None, default_line="main", policy_profile="prototype", default_author_mode="ai_with_human_review", default_model=None, repair_existing=false))]
#[allow(clippy::too_many_arguments)]
fn initialize_repository_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    default_line: &str,
    policy_profile: &str,
    default_author_mode: &str,
    default_model: Option<&str>,
    repair_existing: bool,
) -> PyResult<Py<PyDict>> {
    let request = InitRequest {
        root: PathBuf::from(repo_root),
        name: name.map(str::to_string),
        default_line: default_line.to_string(),
        policy_profile: policy_profile.to_string(),
        default_author_mode: default_author_mode.to_string(),
        default_model: default_model.map(str::to_string),
        repair_existing,
    };
    let payload = py.detach(|| ait_cli::binding_services::initialize_repository(&request));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "configure_repository")]
fn configure_repository_py(
    py: Python<'_>,
    repo_root: &str,
    payload: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = parse_json_value(payload, "configuration payload")?;
    let result = py.detach(|| {
        ait_cli::config_surface::config_set_from_payload_with_team_bootstrap(&repo, &payload)
    });
    render_task_workflow_primitive_result(py, result)
}

#[pyfunction(name = "configuration_error")]
fn configuration_error_py(payload: Bound<'_, PyAny>) -> PyResult<Option<String>> {
    let payload = parse_json_value(payload, "configuration result")?;
    Ok(ait_cli::config_surface::team_setup_error(&payload))
}

#[pyfunction(name = "task_start_operation")]
#[pyo3(signature = (repo_root, intent, *, title=None, source=None, local=false, remote_name=None, edit_root=None, human_progress=false))]
#[allow(clippy::too_many_arguments)]
fn task_start_operation_py(
    py: Python<'_>,
    repo_root: &str,
    intent: &str,
    title: Option<&str>,
    source: Option<&str>,
    local: bool,
    remote_name: Option<&str>,
    edit_root: Option<&str>,
    human_progress: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::TaskStartRequest {
        title: title.map(str::to_string),
        source: source.map(str::to_string),
        intent: intent.to_string(),
        local,
        remote_name: remote_name.map(str::to_string),
        edit_root: edit_root.map(PathBuf::from),
    };
    let mut progress = ait_cli::binding_services::emit_task_start_progress;
    let result = py.detach(|| {
        ait_cli::binding_services::start_task(
            &repo,
            &request,
            human_progress.then_some(&mut progress),
        )
    });
    render_task_workflow_primitive_result(py, result)
}

#[pyfunction(name = "task_quick_operation")]
#[pyo3(signature = (repo_root, intent, *, title=None, local=false, remote_name=None, edit_root=None, human_progress=false))]
#[allow(clippy::too_many_arguments)]
fn task_quick_operation_py(
    py: Python<'_>,
    repo_root: &str,
    intent: &str,
    title: Option<&str>,
    local: bool,
    remote_name: Option<&str>,
    edit_root: Option<&str>,
    human_progress: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::TaskQuickRequest {
        title: title.map(str::to_string),
        intent: intent.to_string(),
        local,
        remote_name: remote_name.map(str::to_string),
        edit_root: edit_root.map(PathBuf::from),
    };
    let mut progress = ait_cli::binding_services::emit_task_start_progress;
    let result = py.detach(|| {
        ait_cli::binding_services::quick_task(
            &repo,
            &request,
            human_progress.then_some(&mut progress),
        )
    });
    render_task_workflow_primitive_result(py, result)
}

#[pyfunction(name = "task_finish_operation")]
#[pyo3(signature = (repo_root, task_id, *, local=false, remote_name=None, message=None, sync_plan_markdown=false))]
fn task_finish_operation_py(
    py: Python<'_>,
    repo_root: &str,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
    message: Option<&str>,
    sync_plan_markdown: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::TaskFinishRequest {
        task_id: task_id.to_string(),
        local,
        remote_name: remote_name.map(str::to_string),
        message: message.map(str::to_string),
        sync_plan_markdown,
    };
    let result = py.detach(|| ait_cli::binding_services::finish_task(&repo, &request));
    render_task_workflow_primitive_result(py, result)
}

#[pyfunction(name = "task_finish_exit_code")]
fn task_finish_exit_code_py(payload: Bound<'_, PyAny>) -> PyResult<u8> {
    let payload = parse_json_value(payload, "Task finish result")?;
    Ok(ait_cli::post_finish_hooks::task_finish_exit_code(&payload))
}

#[pyfunction(name = "snapshot_capture")]
#[pyo3(signature = (repo_root, reference, *, message=None, resume_snapshot_id=None))]
fn snapshot_capture_py(
    py: Python<'_>,
    repo_root: &str,
    reference: &str,
    message: Option<&str>,
    resume_snapshot_id: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::SnapshotCaptureRequest {
        reference: reference.to_string(),
        message: message.map(str::to_string),
        resume_snapshot_id: resume_snapshot_id.map(str::to_string),
    };
    let result = py.detach(|| ait_cli::binding_services::capture_snapshot(&repo, &request));
    render_task_workflow_primitive_result(py, result)
}

fn register_binding_operations(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(
        doctor_plan_authority_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(doctor_memory_root_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(unset_configuration_py, module)?)?;
    module.add_function(wrap_pyfunction!(repository_authority_root_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_audit_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_abandon_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(initialize_repository_py, module)?)?;
    module.add_function(wrap_pyfunction!(configure_repository_py, module)?)?;
    module.add_function(wrap_pyfunction!(configuration_error_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_start_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_quick_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_finish_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_finish_exit_code_py, module)?)?;
    module.add_function(wrap_pyfunction!(snapshot_capture_py, module)?)?;
    Ok(())
}
