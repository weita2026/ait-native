#[pyfunction(name = "review_show_operation")]
#[pyo3(signature = (repo_root, reference, *, remote_name=None))]
fn review_show_operation_py(py: Python<'_>, repo_root: &str, reference: &str, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::inspect_review(&repo, reference, remote_name.as_deref())).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_team_request_operation")]
#[pyo3(signature = (repo_root, reference, reviewer_groups, *, patchset_id=None, note=None, remote_name=None))]
fn review_team_request_operation_py(py: Python<'_>, repo_root: &str, reference: &str, reviewer_groups: Vec<String>, patchset_id: Option<String>, note: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::request_team_review(&repo, reference, &reviewer_groups, &ait_cli::binding_review::ReviewRequestOptions {patchset_id,note,remote_name})).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_task_approve_operation")]
#[pyo3(signature = (repo_root, reference, patchset_id, message, *, remote_name=None))]
fn review_task_approve_operation_py(py: Python<'_>, repo_root: &str, reference: &str, patchset_id: &str, message: &str, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::approve_task_review(&repo, reference, patchset_id, message, remote_name.as_deref())).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_code_submit_operation")]
#[pyo3(signature = (repo_root, reference, patchset_id, message, *, reviewer=None, remote_name=None))]
fn review_code_submit_operation_py(py: Python<'_>, repo_root: &str, reference: &str, patchset_id: &str, message: &str, reviewer: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::submit_code_review(&repo, reference, patchset_id, message, reviewer.as_deref(), remote_name.as_deref())).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_team_approve_operation")]
#[pyo3(signature = (repo_root, reference, *, patchset_id=None, reviewer=None, message=None, remote_name=None))]
fn review_team_approve_operation_py(py: Python<'_>, repo_root: &str, reference: &str, patchset_id: Option<String>, reviewer: Option<String>, message: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::approve_team_review(&repo, reference, &ait_cli::binding_review::TeamReviewOptions {patchset_id,reviewer,message,remote_name})).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_team_request_changes_operation")]
#[pyo3(signature = (repo_root, reference, *, patchset_id=None, reviewer=None, message=None, remote_name=None))]
fn review_team_request_changes_operation_py(py: Python<'_>, repo_root: &str, reference: &str, patchset_id: Option<String>, reviewer: Option<String>, message: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::request_team_changes(&repo, reference, &ait_cli::binding_review::TeamReviewOptions {patchset_id,reviewer,message,remote_name})).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_team_comment_operation")]
#[pyo3(signature = (repo_root, reference, *, patchset_id=None, reviewer=None, message=None, remote_name=None))]
fn review_team_comment_operation_py(py: Python<'_>, repo_root: &str, reference: &str, patchset_id: Option<String>, reviewer: Option<String>, message: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::comment_team_review(&repo, reference, &ait_cli::binding_review::TeamReviewOptions {patchset_id,reviewer,message,remote_name})).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_team_defer_operation")]
#[pyo3(signature = (repo_root, reference, *, patchset_id=None, reviewer=None, message=None, remote_name=None))]
fn review_team_defer_operation_py(py: Python<'_>, repo_root: &str, reference: &str, patchset_id: Option<String>, reviewer: Option<String>, message: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::defer_team_review(&repo, reference, &ait_cli::binding_review::TeamReviewOptions {patchset_id,reviewer,message,remote_name})).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_task_request_changes_operation")]
#[pyo3(signature = (repo_root, reference, *, patchset_id=None, message=None, remote_name=None))]
fn review_task_request_changes_operation_py(py: Python<'_>, repo_root: &str, reference: &str, patchset_id: Option<String>, message: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::request_task_changes(&repo, reference, &ait_cli::binding_review::TaskReviewOptions {patchset_id,message,remote_name})).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_task_comment_operation")]
#[pyo3(signature = (repo_root, reference, *, patchset_id=None, message=None, remote_name=None))]
fn review_task_comment_operation_py(py: Python<'_>, repo_root: &str, reference: &str, patchset_id: Option<String>, message: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::comment_task_review(&repo, reference, &ait_cli::binding_review::TaskReviewOptions {patchset_id,message,remote_name})).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "review_task_defer_operation")]
#[pyo3(signature = (repo_root, reference, *, patchset_id=None, message=None, remote_name=None))]
fn review_task_defer_operation_py(py: Python<'_>, repo_root: &str, reference: &str, patchset_id: Option<String>, message: Option<String>, remote_name: Option<String>) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| ait_cli::binding_review::defer_task_review(&repo, reference, &ait_cli::binding_review::TaskReviewOptions {patchset_id,message,remote_name})).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_review_team_request")]
fn format_review_team_request_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_team_request(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_team_approve")]
fn format_review_team_approve_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_team_approve(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_team_request_changes")]
fn format_review_team_request_changes_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_team_request_changes(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_team_comment")]
fn format_review_team_comment_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_team_comment(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_team_defer")]
fn format_review_team_defer_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_team_defer(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_task_approve")]
fn format_review_task_approve_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_task_approve(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_task_request_changes")]
fn format_review_task_request_changes_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_task_request_changes(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_task_comment")]
fn format_review_task_comment_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_task_comment(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_task_defer")]
fn format_review_task_defer_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_task_defer(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_show")]
fn format_review_show_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_show(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_code_submit")]
fn format_review_code_submit_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_code_submit(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_review_code_template")]
fn format_review_code_template_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Review result")?;
    ait_cli::binding_output::format_review_code_template(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "project_review_show")]
fn project_review_show_py(py: Python<'_>, payload: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let payload = parse_json_value(payload, "Review result")?;
    render_json_value(py, ait_cli::binding_output::project_review_show(&payload))
}
#[pyfunction(name = "project_review_task_operation")]
fn project_review_task_operation_py(py: Python<'_>, operation: Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let operation = parse_json_value(operation, "Review operation")?;
    render_json_value(py, ait_cli::binding_output::project_review_task_operation(&operation).map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "validate_review_patchset_selector")]
fn validate_review_patchset_selector_py(value: &str) -> PyResult<String> {
    ait_cli::binding_review::validate_review_patchset_selector(value).map_err(PyValueError::new_err)
}

#[pyfunction(name = "validate_public_patchset_argument")]
fn validate_public_patchset_argument_py(value: &str) -> PyResult<String> {
    ait_cli::binding_review::validate_public_patchset_argument(value).map_err(PyValueError::new_err)
}

fn register_binding_review(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(validate_review_patchset_selector_py, module)?)?;
    module.add_function(wrap_pyfunction!(validate_public_patchset_argument_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_show_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_team_request_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_task_approve_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_code_submit_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_team_approve_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_team_request_changes_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_team_comment_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_team_defer_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_task_request_changes_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_task_comment_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(review_task_defer_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_team_request_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_team_approve_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_team_request_changes_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_team_comment_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_team_defer_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_task_approve_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_task_request_changes_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_task_comment_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_task_defer_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_code_submit_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_review_code_template_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_review_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_review_task_operation_py, module)?)?;
    Ok(())
}
