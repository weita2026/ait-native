#[pyfunction(name = "extract_plan_items")]
fn extract_plan_items_py(py: Python<'_>, markdown: Option<&str>) -> PyResult<Py<PyList>> {
    Ok(render_plan_items(py, extract_plan_items(markdown))?.unbind())
}

#[pyfunction(name = "list_plan_section_refs")]
fn list_plan_section_refs_py(py: Python<'_>, markdown: Option<&str>) -> PyResult<Py<PyList>> {
    let output = PyList::empty(py);
    for item in list_plan_section_refs(markdown) {
        let row = PyDict::new(py);
        row.set_item("plan_ref", item.plan_ref)?;
        row.set_item("heading_title", item.heading_title)?;
        row.set_item("heading_level", item.heading_level)?;
        row.set_item("line_number", item.line_number)?;
        output.append(row)?;
    }
    Ok(output.unbind())
}

#[pyfunction(name = "extract_plan_section")]
fn extract_plan_section_py(
    py: Python<'_>,
    markdown: Option<&str>,
    plan_ref: Option<&str>,
) -> PyResult<Option<Py<PyDict>>> {
    let Some(section) = extract_plan_section(markdown, plan_ref) else {
        return Ok(None);
    };
    Ok(Some(render_plan_section(py, section)?.unbind()))
}

#[pyfunction(name = "find_plan_item")]
fn find_plan_item_py(
    py: Python<'_>,
    markdown: Option<&str>,
    plan_item_ref: Option<&str>,
) -> PyResult<Option<Py<PyDict>>> {
    let Some(item) = find_plan_item(markdown, plan_item_ref) else {
        return Ok(None);
    };
    Ok(Some(render_plan_item(py, item)?.unbind()))
}

#[pyfunction(name = "normalize_plan_items")]
fn normalize_plan_items_py(
    py: Python<'_>,
    items: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyList>> {
    let seeds = parse_plan_item_seeds(items)?;
    let normalized = normalize_plan_items(&seeds).map_err(PyValueError::new_err)?;
    Ok(render_plan_items(py, normalized)?.unbind())
}

#[pyfunction(name = "find_plan_item_in_items")]
fn find_plan_item_in_items_py(
    py: Python<'_>,
    items: Option<Bound<'_, PyAny>>,
    plan_item_ref: Option<&str>,
) -> PyResult<Option<Py<PyDict>>> {
    let seeds = parse_plan_item_seeds(items)?;
    let Some(item) =
        find_plan_item_in_items(&seeds, plan_item_ref).map_err(PyValueError::new_err)?
    else {
        return Ok(None);
    };
    Ok(Some(render_plan_item(py, item)?.unbind()))
}

#[pyfunction(name = "plan_items_payload")]
fn plan_items_payload_py(
    py: Python<'_>,
    plan: Bound<'_, PyDict>,
    revision: Option<Bound<'_, PyDict>>,
) -> PyResult<Py<PyDict>> {
    let plan = parse_dispatch_plan(plan)?;
    let revision = revision
        .as_ref()
        .map(parse_dispatch_revision)
        .transpose()?;
    Ok(render_plan_items_payload(py, plan_items_payload(&plan, revision.as_ref()))?.unbind())
}

#[pyfunction(name = "local_plan_publish_shadow")]
fn local_plan_publish_shadow_py(
    py: Python<'_>,
    plan: Option<Bound<'_, PyDict>>,
) -> PyResult<Option<Py<PyDict>>> {
    let parsed_plan = plan
        .map(parse_dispatch_plan)
        .transpose()?;
    let Some(shadow) = local_plan_publish_shadow(parsed_plan.as_ref()) else {
        return Ok(None);
    };
    Ok(Some(render_local_plan_publish_shadow(py, shadow)?.unbind()))
}

#[pyfunction(name = "plan_dispatch_summary")]
fn plan_dispatch_summary_py(
    py: Python<'_>,
    plan: Bound<'_, PyDict>,
    tasks: Option<Bound<'_, PyAny>>,
    revision: Option<Bound<'_, PyDict>>,
    local_shadow: Option<Bound<'_, PyDict>>,
) -> PyResult<Py<PyDict>> {
    let plan = parse_dispatch_plan(plan)?;
    let tasks = parse_dispatch_tasks(tasks)?;
    let revision = revision
        .as_ref()
        .map(parse_dispatch_revision)
        .transpose()?;
    let local_shadow = local_shadow
        .as_ref()
        .map(parse_local_plan_publish_shadow)
        .transpose()?;
    Ok(render_plan_dispatch_summary(
        py,
        plan_dispatch_summary(&plan, &tasks, revision.as_ref(), local_shadow.as_ref()),
    )?
    .unbind())
}

#[pyfunction(name = "plan_task_link_indexes")]
fn plan_task_link_indexes_py(
    py: Python<'_>,
    tasks: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyTuple>> {
    let tasks = parse_dispatch_tasks(tasks)?;
    Ok(render_plan_task_link_indexes(py, plan_task_link_indexes(&tasks))?.unbind())
}

#[pyfunction(name = "compute_taskable_items")]
fn compute_taskable_items_py(
    py: Python<'_>,
    plan: Bound<'_, PyDict>,
    tasks: Option<Bound<'_, PyAny>>,
    revision: Option<Bound<'_, PyDict>>,
    local_shadow: Option<Bound<'_, PyDict>>,
) -> PyResult<Py<PyList>> {
    let plan = parse_dispatch_plan(plan)?;
    let tasks = parse_dispatch_tasks(tasks)?;
    let revision = revision
        .as_ref()
        .map(parse_dispatch_revision)
        .transpose()?;
    let local_shadow = local_shadow
        .as_ref()
        .map(parse_local_plan_publish_shadow)
        .transpose()?;
    Ok(render_dispatch_summary_items(
        py,
        compute_taskable_items(&plan, &tasks, revision.as_ref(), local_shadow.as_ref()),
    )?
    .unbind())
}

#[pyfunction(name = "validate_dispatch_legality")]
fn validate_dispatch_legality_py(
    py: Python<'_>,
    plan: Bound<'_, PyDict>,
    tasks: Option<Bound<'_, PyAny>>,
    plan_item_ref: Option<&str>,
    revision: Option<Bound<'_, PyDict>>,
    local_shadow: Option<Bound<'_, PyDict>>,
) -> PyResult<Py<PyDict>> {
    let plan = parse_dispatch_plan(plan)?;
    let tasks = parse_dispatch_tasks(tasks)?;
    let revision = revision
        .as_ref()
        .map(parse_dispatch_revision)
        .transpose()?;
    let local_shadow = local_shadow
        .as_ref()
        .map(parse_local_plan_publish_shadow)
        .transpose()?;
    Ok(render_dispatch_legality_decision(
        py,
        validate_dispatch_legality(
            &plan,
            &tasks,
            plan_item_ref,
            revision.as_ref(),
            local_shadow.as_ref(),
        ),
    )?
    .unbind())
}

#[pyfunction(name = "plan_candidates_payload")]
fn plan_candidates_payload_py(
    py: Python<'_>,
    summaries: Option<Bound<'_, PyAny>>,
    scope: Option<&str>,
    repo_name: Option<&str>,
    remote: Option<&str>,
    include_all: Option<bool>,
) -> PyResult<Py<PyDict>> {
    let summaries = parse_plan_dispatch_summaries(summaries)?;
    Ok(render_plan_candidates_payload(
        py,
        plan_candidates_payload(
            &summaries,
            scope,
            repo_name,
            remote,
            include_all.unwrap_or(false),
        ),
    )?
    .unbind())
}


#[pyfunction(name = "normalize_task_workflow_http_compatibility")]
fn normalize_task_workflow_http_compatibility_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_task_workflow_http_compatibility_payload_json(payload_json)
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "normalize_task_workflow_http_readiness")]
fn normalize_task_workflow_http_readiness_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_task_workflow_http_readiness_payload_json(payload_json)
        .map_err(plan_http_py_error)?;
    render_json_dict(py, payload)
}


fn register_plan_store(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(extract_plan_items_py, module)?)?;
    module.add_function(wrap_pyfunction!(list_plan_section_refs_py, module)?)?;
    module.add_function(wrap_pyfunction!(extract_plan_section_py, module)?)?;
    module.add_function(wrap_pyfunction!(find_plan_item_py, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_plan_items_py, module)?)?;
    module.add_function(wrap_pyfunction!(find_plan_item_in_items_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_items_payload_py, module)?)?;
    module.add_function(wrap_pyfunction!(local_plan_publish_shadow_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_dispatch_summary_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_task_link_indexes_py, module)?)?;
    module.add_function(wrap_pyfunction!(compute_taskable_items_py, module)?)?;
    module.add_function(wrap_pyfunction!(validate_dispatch_legality_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_candidates_payload_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        normalize_task_workflow_http_compatibility_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        normalize_task_workflow_http_readiness_py,
        module
    )?)?;
    Ok(())
}
