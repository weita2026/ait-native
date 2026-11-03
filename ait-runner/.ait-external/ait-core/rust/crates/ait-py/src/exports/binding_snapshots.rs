#[pyfunction(name = "inspect_snapshot_operation")]
#[pyo3(signature = (repo_root, selector, *, include_delta=false))]
fn inspect_snapshot_operation_py(
    py: Python<'_>,
    repo_root: &str,
    selector: &str,
    include_delta: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_services::inspect_snapshot(&repo, selector, include_delta))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "project_snapshot_list")]
#[pyo3(signature = (payload, *, all=false))]
fn project_snapshot_list_py(
    py: Python<'_>,
    payload: Bound<'_, PyAny>,
    all: bool,
) -> PyResult<Py<PyAny>> {
    let payload = parse_json_value(payload, "Snapshot inventory")?;
    render_json_value(
        py,
        ait_cli::binding_output::project_snapshot_list(&payload, all),
    )
}

#[pyfunction(name = "format_snapshot_list")]
#[pyo3(signature = (payload, *, all=false))]
fn format_snapshot_list_py(payload: Bound<'_, PyAny>, all: bool) -> PyResult<String> {
    let payload = parse_json_value(payload, "Snapshot inventory")?;
    ait_cli::binding_output::format_snapshot_list(&payload, all).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_snapshot_show")]
#[pyo3(signature = (inspection, *, files=false))]
fn format_snapshot_show_py(inspection: Bound<'_, PyAny>, files: bool) -> PyResult<String> {
    let payload = parse_json_value(inspection, "Snapshot inspection")?;
    ait_cli::binding_output::format_snapshot_show(&payload, files).map_err(PyValueError::new_err)
}

#[pyfunction(name = "snapshot_ancestry_operation")]
#[pyo3(signature = (repo_root, selector, *, descendants=false, first_parent=false, max_depth=10_000, limit=10_000))]
fn snapshot_ancestry_operation_py(
    py: Python<'_>,
    repo_root: &str,
    selector: &str,
    descendants: bool,
    first_parent: bool,
    max_depth: usize,
    limit: usize,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::SnapshotAncestryRequest {
        selector: selector.into(),
        descendants,
        first_parent,
        max_depth,
        limit,
    };
    let payload = py
        .detach(|| ait_cli::binding_services::query_snapshot_ancestry(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "snapshot_is_ancestor_operation")]
fn snapshot_is_ancestor_operation_py(
    py: Python<'_>,
    repo_root: &str,
    older: &str,
    newer: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let result = py
        .detach(|| ait_cli::binding_services::query_snapshot_is_ancestor(&repo, older, newer))
        .map_err(PyValueError::new_err)?;
    render_workflow_operation(py, result)
}

#[pyfunction(name = "snapshot_merge_base_operation")]
#[pyo3(signature = (repo_root, left, right, *, all=false))]
fn snapshot_merge_base_operation_py(
    py: Python<'_>,
    repo_root: &str,
    left: &str,
    right: &str,
    all: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let result = py
        .detach(|| ait_cli::binding_services::query_snapshot_merge_base(&repo, left, right, all))
        .map_err(PyValueError::new_err)?;
    render_workflow_operation(py, result)
}

#[pyfunction(name = "format_snapshot_ancestry")]
#[pyo3(signature = (payload, *, all=false))]
fn format_snapshot_ancestry_py(payload: Bound<'_, PyAny>, all: bool) -> PyResult<String> {
    let payload = parse_json_value(payload, "Snapshot ancestry result")?;
    ait_cli::binding_output::format_snapshot_ancestry(&payload, all).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_snapshot_is_ancestor")]
fn format_snapshot_is_ancestor_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Snapshot ancestry result")?;
    ait_cli::binding_output::format_snapshot_is_ancestor(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_snapshot_merge_base")]
fn format_snapshot_merge_base_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Snapshot merge-base result")?;
    ait_cli::binding_output::format_snapshot_merge_base(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "snapshot_query_error_exit_code")]
fn snapshot_query_error_exit_code_py() -> u8 {
    ait_cli::binding_output::SNAPSHOT_QUERY_ERROR_EXIT_CODE
}

#[pyfunction(name = "format_snapshot_query_error")]
fn format_snapshot_query_error_py(message: &str) -> String {
    ait_cli::binding_output::format_snapshot_query_error(message)
}

macro_rules! snapshot_text_output {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        fn $name(payload: Bound<'_, PyAny>) -> PyResult<String> {
            let payload = parse_json_value(payload, "Snapshot result")?;
            $target(&payload).map_err(PyValueError::new_err)
        }
    )*};
}
snapshot_text_output! {
    (format_snapshot_restore_lines_py, "format_snapshot_restore_lines", ait_cli::binding_output::format_snapshot_restore_lines),
    (format_snapshot_revert_py, "format_snapshot_revert", ait_cli::binding_output::format_snapshot_revert),
    (format_snapshot_replay_py, "format_snapshot_replay", ait_cli::binding_output::format_snapshot_replay),
    (format_snapshot_diff_py, "format_snapshot_diff", ait_cli::binding_output::format_snapshot_diff),
}

#[pyfunction(name = "snapshot_diff_operation")]
#[pyo3(signature = (repo_root, older, newer, *, include_text=false, max_bytes=DEFAULT_SNAPSHOT_DIFF_MAX_BYTES))]
fn snapshot_diff_operation_py(
    py: Python<'_>,
    repo_root: &str,
    older: &str,
    newer: &str,
    include_text: bool,
    max_bytes: usize,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_services::diff_snapshot_selectors(
                &repo,
                older,
                newer,
                include_text,
                max_bytes,
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "snapshot_revert_operation")]
#[pyo3(signature = (repo_root, selector, *, force=false, dry_run=false))]
fn snapshot_revert_operation_py(
    py: Python<'_>,
    repo_root: &str,
    selector: &str,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_services::revert_snapshot_selector(&repo, selector, force, dry_run)
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "snapshot_replay_operation")]
#[pyo3(signature = (repo_root, selector, *, onto=None, force=false, dry_run=false))]
fn snapshot_replay_operation_py(
    py: Python<'_>,
    repo_root: &str,
    selector: &str,
    onto: Option<&str>,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_services::replay_snapshot_selector(
                &repo, selector, onto, force, dry_run,
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

fn register_binding_snapshots(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(
        parse_snapshot_authoring_reference_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(snapshot_diff_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(snapshot_revert_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(snapshot_replay_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(inspect_snapshot_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(project_snapshot_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(snapshot_ancestry_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(snapshot_is_ancestor_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(snapshot_merge_base_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_ancestry_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_is_ancestor_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_merge_base_py, module)?)?;
    module.add_function(wrap_pyfunction!(snapshot_query_error_exit_code_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_query_error_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_restore_lines_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_revert_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_replay_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_snapshot_diff_py, module)?)?;
    Ok(())
}

#[pyfunction(name = "parse_snapshot_authoring_reference")]
fn parse_snapshot_authoring_reference_py(reference: &str) -> PyResult<String> {
    ait_cli::binding_services::parse_snapshot_authoring_reference(reference)
        .map_err(PyValueError::new_err)
}
