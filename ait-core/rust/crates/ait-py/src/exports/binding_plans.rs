fn binding_plan_scope(
    local: bool,
    remote_name: Option<&str>,
) -> ait_cli::binding_services::PlanScope {
    ait_cli::binding_services::PlanScope {
        local,
        remote_name: remote_name.map(str::to_string),
    }
}

#[pyfunction(name = "plan_list_operation")]
#[pyo3(signature = (repo_root, *, local=false, remote_name=None))]
fn plan_list_operation_py(
    py: Python<'_>,
    repo_root: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let scope = binding_plan_scope(local, remote_name);
    let payload = py
        .detach(|| ait_cli::binding_services::list_plans(&repo, &scope))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

macro_rules! plan_selection_operation {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        #[pyo3(signature = (repo_root, plan_id, *, revision=None, local=false, remote_name=None))]
        fn $name(py: Python<'_>, repo_root: &str, plan_id: &str, revision: Option<&str>, local: bool, remote_name: Option<&str>) -> PyResult<Py<PyAny>> {
            let repo = discover_task_workflow_repo(repo_root)?;
            let request = ait_cli::binding_services::PlanSelection { plan_id: plan_id.to_string(), revision: revision.map(str::to_string), scope: binding_plan_scope(local, remote_name) };
            let payload = py.detach(|| $target(&repo, &request)).map_err(PyValueError::new_err)?;
            render_json_value(py, payload)
        }
    )*};
}

plan_selection_operation! {
    (plan_show_operation_py, "plan_show_operation", ait_cli::binding_services::show_plan),
    (plan_items_operation_py, "plan_items_operation", ait_cli::binding_services::plan_items),
    (plan_inspect_operation_py, "plan_inspect_operation", ait_cli::binding_services::inspect_plan),
}

#[pyfunction(name = "plan_revisions_operation")]
#[pyo3(signature = (repo_root, plan_id, *, local=false, remote_name=None))]
fn plan_revisions_operation_py(
    py: Python<'_>,
    repo_root: &str,
    plan_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let scope = binding_plan_scope(local, remote_name);
    let payload = py
        .detach(|| ait_cli::binding_services::plan_revisions(&repo, plan_id, &scope))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "plan_candidates_operation")]
#[pyo3(signature = (repo_root, *, include_all=false, contains=None, local=false, remote_name=None))]
fn plan_candidates_operation_py(
    py: Python<'_>,
    repo_root: &str,
    include_all: bool,
    contains: Option<&str>,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::PlanCandidatesRequest {
        scope: binding_plan_scope(local, remote_name),
        include_all,
        contains: contains.map(str::to_string),
    };
    let payload = py
        .detach(|| ait_cli::binding_services::plan_candidates(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "plan_sync_operation")]
#[pyo3(signature = (repo_root, target, *, plan_ref=None, prune=false, local=false, remote_name=None, reconcile=false))]
#[allow(clippy::too_many_arguments)]
fn plan_sync_operation_py(
    py: Python<'_>,
    repo_root: &str,
    target: &str,
    plan_ref: Option<&str>,
    prune: bool,
    local: bool,
    remote_name: Option<&str>,
    reconcile: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_services::PlanSyncRequest {
        target: PathBuf::from(target),
        plan_ref: plan_ref.map(str::to_string),
        prune,
        scope: binding_plan_scope(local, remote_name),
        reconcile,
    };
    let payload = py
        .detach(|| ait_cli::binding_services::sync_plan(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "plan_sync_error")]
fn plan_sync_error_py(payload: Bound<'_, PyAny>) -> PyResult<Option<String>> {
    let payload = parse_json_value(payload, "Plan sync result")?;
    Ok(ait_cli::binding_output::plan_sync_error(&payload))
}

#[pyfunction(name = "format_plan_list")]
#[pyo3(signature = (payload, *, all=false, local=false, remote_name=None))]
fn format_plan_list_py(
    payload: Bound<'_, PyAny>,
    all: bool,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<String> {
    let payload = parse_json_value(payload, "Plan list result")?;
    ait_cli::binding_output::format_plan_list(
        &payload,
        all,
        &binding_plan_scope(local, remote_name),
    )
    .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_plan_revisions")]
#[pyo3(signature = (payload, plan_id, *, all=false, local=false, remote_name=None))]
fn format_plan_revisions_py(
    payload: Bound<'_, PyAny>,
    plan_id: &str,
    all: bool,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<String> {
    let payload = parse_json_value(payload, "Plan revisions result")?;
    ait_cli::binding_output::format_plan_revisions(
        &payload,
        plan_id,
        all,
        &binding_plan_scope(local, remote_name),
    )
    .map_err(PyValueError::new_err)
}

macro_rules! plan_text_output {
    ($(($name:ident, $python_name:literal, $target:path)),* $(,)?) => {$(
        #[pyfunction(name = $python_name)]
        fn $name(payload: Bound<'_, PyAny>) -> PyResult<String> {
            let payload = parse_json_value(payload, "Plan result")?;
            $target(&payload).map_err(PyValueError::new_err)
        }
    )*};
}

plan_text_output! {
    (format_plan_show_py, "format_plan_show", ait_cli::binding_output::format_plan_show),
    (format_plan_items_py, "format_plan_items", ait_cli::binding_output::format_plan_items),
    (format_plan_candidates_py, "format_plan_candidates", ait_cli::binding_output::format_plan_candidates),
    (format_plan_inspect_py, "format_plan_inspect", ait_cli::binding_output::format_plan_inspect),
    (format_plan_sync_py, "format_plan_sync", ait_cli::binding_output::format_plan_sync),
}

fn register_binding_plans(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(plan_list_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_show_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_revisions_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_items_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_inspect_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_candidates_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_sync_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_sync_error_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_plan_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_plan_revisions_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_plan_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_plan_items_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_plan_candidates_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_plan_inspect_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_plan_sync_py, module)?)?;
    Ok(())
}
