#[pyfunction(name = "line_inventory_operation")]
#[pyo3(signature = (repo_root, *, include_all=false, archived=false, remote_name=None))]
fn line_inventory_operation_py(
    py: Python<'_>,
    repo_root: &str,
    include_all: bool,
    archived: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::binding_services::line_inventory(&repo, include_all, archived, remote_name)
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "line_show_operation")]
#[pyo3(signature = (repo_root, *, name=None, remote_name=None))]
fn line_show_operation_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| ait_cli::binding_services::show_line(&repo, name, remote_name))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "line_merge_operation")]
#[pyo3(signature = (repo_root, *, source=None, message=None, continue_merge=false, abort_merge=false))]
fn line_merge_operation_py(
    py: Python<'_>,
    repo_root: &str,
    source: Option<&str>,
    message: Option<&str>,
    continue_merge: bool,
    abort_merge: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| {
            ait_cli::primitives::line_merge(&repo, source, message, continue_merge, abort_merge)
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_line_list")]
#[pyo3(signature = (inventory, *, include_all=false, archived=false, remote_name=None))]
fn format_line_list_py(
    inventory: Bound<'_, PyAny>,
    include_all: bool,
    archived: bool,
    remote_name: Option<&str>,
) -> PyResult<String> {
    let inventory = parse_json_value(inventory, "Line inventory")?;
    ait_cli::binding_output::format_line_list(&inventory, include_all || archived, remote_name)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_line_cleanup")]
#[pyo3(signature = (payload, idle_for, *, all=false, include_protected=false, cleanup_kind=None, limit=None))]
fn format_line_cleanup_py(
    payload: Bound<'_, PyAny>,
    idle_for: &str,
    all: bool,
    include_protected: bool,
    cleanup_kind: Option<&str>,
    limit: Option<usize>,
) -> PyResult<String> {
    let payload = parse_json_value(payload, "Line cleanup result")?;
    ait_cli::binding_output::format_line_cleanup(
        &payload,
        all,
        include_protected,
        idle_for,
        cleanup_kind,
        limit,
    )
    .map_err(PyValueError::new_err)
}

macro_rules! line_text_output {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        fn $name(payload: Bound<'_, PyAny>) -> PyResult<String> {
            let payload = parse_json_value(payload, "Line result")?;
            $target(&payload).map_err(PyValueError::new_err)
        }
    )*};
}

line_text_output! {
    (format_line_create_py, "format_line_create", ait_cli::binding_output::format_line_create),
    (format_line_switch_py, "format_line_switch", ait_cli::binding_output::format_line_switch),
    (format_line_show_py, "format_line_show", ait_cli::binding_output::format_line_show),
    (format_line_archive_py, "format_line_archive", ait_cli::binding_output::format_line_archive),
    (format_line_rename_py, "format_line_rename", ait_cli::binding_output::format_line_rename),
    (format_line_delete_py, "format_line_delete", ait_cli::binding_output::format_line_delete),
    (format_line_merge_py, "format_line_merge", ait_cli::binding_output::format_line_merge),
}

fn register_binding_lines(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(line_inventory_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(line_show_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(line_merge_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_line_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_line_cleanup_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_line_create_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_line_switch_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_line_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_line_archive_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_line_rename_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_line_delete_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_line_merge_py, module)?)?;
    Ok(())
}
