fn render_workflow_operation(
    py: Python<'_>,
    result: ait_cli::binding_services::WorkflowOperationResult,
) -> PyResult<Py<PyAny>> {
    render_json_value(
        py,
        ait_core::json_support::json!({"payload": result.payload, "exit_code": result.exit_code}),
    )
}

#[pyfunction(name = "workflow_ready_operation")]
#[pyo3(signature = (repo_root, task_id, *, apply=false, remote_name=None, expected_identity_path=None, snapshot_message=None, summary=None, tests=None, lint=None, security=None, license=None, author_mode=None, model=None, run_ci_once=false))]
#[allow(clippy::too_many_arguments)]
fn workflow_ready_operation_py(
    py: Python<'_>,
    repo_root: &str,
    task_id: &str,
    apply: bool,
    remote_name: Option<&str>,
    expected_identity_path: Option<&str>,
    snapshot_message: Option<&str>,
    summary: Option<&str>,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: Option<&str>,
    model: Option<&str>,
    run_ci_once: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::WorkflowReadyRequest {
        task_id: task_id.into(),
        apply,
        remote_name: remote_name.map(str::to_string),
        expected_identity: expected_identity_path.map(PathBuf::from),
        snapshot_message: snapshot_message.map(str::to_string),
        summary: summary.map(str::to_string),
        tests: tests.map(str::to_string),
        lint: lint.map(str::to_string),
        security: security.map(str::to_string),
        license: license.map(str::to_string),
        author_mode: author_mode.map(str::to_string),
        model: model.map(str::to_string),
        run_ci_once,
    };
    let result = py
        .detach(|| ait_cli::binding_services::prepare_workflow(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_workflow_operation(py, result)
}

#[pyfunction(name = "workflow_finish_operation")]
#[pyo3(signature = (repo_root, task_id, *, apply=false, remote_name=None, expected_identity_path=None, reviewer=None, review_message=None))]
#[allow(clippy::too_many_arguments)]
fn workflow_finish_operation_py(
    py: Python<'_>,
    repo_root: &str,
    task_id: &str,
    apply: bool,
    remote_name: Option<&str>,
    expected_identity_path: Option<&str>,
    reviewer: Option<&str>,
    review_message: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::WorkflowFinishRequest {
        task_id: task_id.into(),
        apply,
        remote_name: remote_name.map(str::to_string),
        expected_identity: expected_identity_path.map(PathBuf::from),
        reviewer: reviewer.map(str::to_string),
        review_message: review_message.map(str::to_string),
    };
    let result = py
        .detach(|| ait_cli::binding_services::finish_workflow(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_workflow_operation(py, result)
}

#[pyfunction(name = "workflow_reconcile_operation")]
#[pyo3(signature = (repo_root, *, remote_name=None, task_id=None, apply=false, safe_only=false, scheduled=false, limit=100))]
#[allow(clippy::too_many_arguments)]
fn workflow_reconcile_operation_py(
    py: Python<'_>,
    repo_root: &str,
    remote_name: Option<&str>,
    task_id: Option<&str>,
    apply: bool,
    safe_only: bool,
    scheduled: bool,
    limit: usize,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::WorkflowReconcileRequest {
        remote_name: remote_name.map(str::to_string),
        task_id: task_id.map(str::to_string),
        apply,
        safe_only,
        scheduled,
        limit,
    };
    let payload = py
        .detach(|| ait_cli::binding_services::reconcile_workflow(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "workflow_guide")]
#[pyo3(signature = (topic=None))]
fn workflow_guide_py(py: Python<'_>, topic: Option<&str>) -> PyResult<Py<PyAny>> {
    let payload =
        ait_cli::binding_services::workflow_guide(topic).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

macro_rules! workflow_output_projection {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        #[pyo3(signature = (repo_root, payload, task_id, *, remote_name=None, full=false))]
        fn $name(py: Python<'_>, repo_root: &str, payload: Bound<'_, PyAny>, task_id: &str, remote_name: Option<&str>, full: bool) -> PyResult<Py<PyAny>> {
            let repo = discover_task_workflow_repo(repo_root)?;
            let payload = parse_json_value(payload, "workflow result")?;
            render_json_value(py, $target(&repo, &payload, task_id, remote_name, full))
        }
    )*};
}

workflow_output_projection! {
    (project_workflow_ready_py, "project_workflow_ready", ait_cli::binding_output::project_workflow_ready),
    (project_workflow_finish_py, "project_workflow_finish", ait_cli::binding_output::project_workflow_finish),
}

macro_rules! workflow_phase_text {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        fn $name(payload: Bound<'_, PyAny>, task_id: &str) -> PyResult<String> {
            let payload = parse_json_value(payload, "workflow result")?;
            $target(&payload, task_id).map_err(PyValueError::new_err)
        }
    )*};
}

workflow_phase_text! {
    (format_workflow_ready_py, "format_workflow_ready", ait_cli::binding_output::format_workflow_ready),
    (format_workflow_finish_py, "format_workflow_finish", ait_cli::binding_output::format_workflow_finish),
}

macro_rules! workflow_text {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        fn $name(payload: Bound<'_, PyAny>) -> PyResult<String> {
            let payload = parse_json_value(payload, "workflow result")?;
            $target(&payload).map_err(PyValueError::new_err)
        }
    )*};
}

workflow_text! {
    (format_workflow_reconcile_py, "format_workflow_reconcile", ait_cli::binding_output::format_workflow_reconcile),
    (format_workflow_guide_py, "format_workflow_guide", ait_cli::binding_output::format_workflow_guide),
}

fn register_binding_workflows(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(workflow_ready_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_finish_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_reconcile_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_guide_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_workflow_ready_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_workflow_finish_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_workflow_ready_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_workflow_finish_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_workflow_reconcile_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_workflow_guide_py, module)?)?;
    Ok(())
}
