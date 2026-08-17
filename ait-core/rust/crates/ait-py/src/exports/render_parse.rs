fn render_plan_item<'py>(py: Python<'py>, item: PlanItem) -> PyResult<Bound<'py, PyDict>> {
    let row = PyDict::new(py);
    row.set_item("plan_item_ref", item.plan_item_ref)?;
    row.set_item("text", item.text)?;
    row.set_item("checkbox_state", item.checkbox_state.as_str())?;
    row.set_item("heading_path", item.heading_path)?;
    row.set_item("line_number", item.line_number)?;
    Ok(row)
}

fn render_plan_items<'py>(
    py: Python<'py>,
    items: Vec<ait_core::plan_items::PlanItem>,
) -> PyResult<Bound<'py, PyList>> {
    let output = PyList::empty(py);
    for item in items {
        output.append(render_plan_item(py, item)?)?;
    }
    Ok(output)
}

fn render_plan_section<'py>(py: Python<'py>, section: PlanSection) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("plan_ref", section.plan_ref)?;
    output.set_item("heading_title", section.heading_title)?;
    output.set_item("heading_level", section.heading_level)?;
    output.set_item("line_number", section.line_number)?;
    output.set_item("section_markdown", section.section_markdown)?;
    output.set_item("items", render_plan_items(py, section.items)?)?;
    Ok(output)
}

fn render_dispatch_item<'py>(
    py: Python<'py>,
    item: DispatchPlanItemInput,
) -> PyResult<Bound<'py, PyDict>> {
    let row = PyDict::new(py);
    row.set_item("plan_item_ref", item.plan_item_ref)?;
    row.set_item("text", item.text)?;
    row.set_item("checkbox_state", item.checkbox_state)?;
    row.set_item("heading_path", item.heading_path)?;
    row.set_item("line_number", item.line_number)?;
    Ok(row)
}

fn render_linked_task_summary<'py>(
    py: Python<'py>,
    task: LinkedTaskSummary,
) -> PyResult<Bound<'py, PyDict>> {
    let row = PyDict::new(py);
    row.set_item("task_id", task.task_id)?;
    row.set_item("title", task.title)?;
    row.set_item("status", task.status)?;
    row.set_item("planning_state", task.planning_state)?;
    row.set_item("origin_plan_revision_id", task.origin_plan_revision_id)?;
    row.set_item("plan_drift_state", task.plan_drift_state)?;
    Ok(row)
}

fn render_local_plan_publish_shadow<'py>(
    py: Python<'py>,
    shadow: LocalPlanPublishShadow,
) -> PyResult<Bound<'py, PyDict>> {
    let row = PyDict::new(py);
    row.set_item("plan_id", shadow.plan_id)?;
    row.set_item("publication_state", shadow.publication_state)?;
    row.set_item("head_publication_state", shadow.head_publication_state)?;
    row.set_item("head_revision_id", shadow.head_revision_id)?;
    row.set_item("head_revision_number", shadow.head_revision_number)?;
    row.set_item("published_plan_id", shadow.published_plan_id)?;
    row.set_item(
        "published_head_revision_id",
        shadow.published_head_revision_id,
    )?;
    row.set_item("unpublished_head", shadow.unpublished_head)?;
    Ok(row)
}

fn render_plan_items_payload<'py>(
    py: Python<'py>,
    payload: PlanItemsPayload,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("plan_id", payload.plan_id)?;
    output.set_item("plan_title", payload.plan_title)?;
    output.set_item("plan_revision_id", payload.plan_revision_id)?;
    output.set_item("revision_number", payload.revision_number)?;
    output.set_item("identity_only", payload.identity_only)?;
    output.set_item(
        "dispatch_validation_required",
        payload.dispatch_validation_required,
    )?;
    output.set_item("dispatch_validation_hint", payload.dispatch_validation_hint)?;
    output.set_item("item_count", payload.item_count)?;
    let items = PyList::empty(py);
    for item in payload.items {
        items.append(render_dispatch_item(py, item)?)?;
    }
    output.set_item("items", items)?;
    Ok(output)
}

fn render_dispatch_summary_item<'py>(
    py: Python<'py>,
    item: DispatchSummaryItem,
) -> PyResult<Bound<'py, PyDict>> {
    let row = PyDict::new(py);
    row.set_item("plan_item_ref", item.plan_item_ref)?;
    row.set_item("text", item.text)?;
    row.set_item("checkbox_state", item.checkbox_state)?;
    row.set_item("heading_path", item.heading_path)?;
    row.set_item("line_number", item.line_number)?;
    let linked_tasks = PyList::empty(py);
    for task in item.linked_tasks {
        linked_tasks.append(render_linked_task_summary(py, task)?)?;
    }
    row.set_item("linked_tasks", linked_tasks)?;
    row.set_item("taskable", item.taskable)?;
    row.set_item("taskable_blocker", item.taskable_blocker)?;
    Ok(row)
}

fn render_plan_dispatch_summary<'py>(
    py: Python<'py>,
    summary: PlanDispatchSummary,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("plan_id", summary.plan_id)?;
    output.set_item("title", summary.title)?;
    output.set_item("status", summary.status)?;
    output.set_item("repo_name", summary.repo_name)?;
    output.set_item("artifact_path", summary.artifact_path)?;
    output.set_item("artifact_selector", summary.artifact_selector)?;
    output.set_item("artifact_heading", summary.artifact_heading)?;
    output.set_item("plan_revision_id", summary.plan_revision_id)?;
    output.set_item("revision_number", summary.revision_number)?;
    output.set_item("publication_state", summary.publication_state)?;
    output.set_item("head_publication_state", summary.head_publication_state)?;
    output.set_item("published_plan_id", summary.published_plan_id)?;
    output.set_item(
        "published_head_revision_id",
        summary.published_head_revision_id,
    )?;
    output.set_item("local_unpublished_head", summary.local_unpublished_head)?;
    match summary.local_publication {
        Some(shadow) => output.set_item(
            "local_publication",
            render_local_plan_publish_shadow(py, shadow)?,
        )?,
        None => output.set_item("local_publication", py.None())?,
    }
    output.set_item("item_count", summary.item_count)?;
    output.set_item("open_item_count", summary.open_item_count)?;
    output.set_item("done_item_count", summary.done_item_count)?;
    output.set_item("unref_open_item_count", summary.unref_open_item_count)?;
    output.set_item("linked_open_item_count", summary.linked_open_item_count)?;
    output.set_item("taskable_item_count", summary.taskable_item_count)?;
    output.set_item("linked_task_count", summary.linked_task_count)?;
    let status_counts = PyDict::new(py);
    for (status, count) in summary.linked_task_status_counts {
        status_counts.set_item(status, count)?;
    }
    output.set_item("linked_task_status_counts", status_counts)?;
    output.set_item("items", render_dispatch_summary_items(py, summary.items)?)?;
    output.set_item(
        "open_items",
        render_dispatch_summary_items(py, summary.open_items)?,
    )?;
    output.set_item(
        "taskable_items",
        render_dispatch_summary_items(py, summary.taskable_items)?,
    )?;
    Ok(output)
}

fn render_dispatch_summary_items<'py>(
    py: Python<'py>,
    items: Vec<DispatchSummaryItem>,
) -> PyResult<Bound<'py, PyList>> {
    let output = PyList::empty(py);
    for item in items {
        output.append(render_dispatch_summary_item(py, item)?)?;
    }
    Ok(output)
}

fn render_dispatch_legality_decision<'py>(
    py: Python<'py>,
    decision: DispatchLegalityDecision,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("plan_item_ref", decision.plan_item_ref)?;
    output.set_item("taskable", decision.taskable)?;
    output.set_item("taskable_blocker", decision.taskable_blocker)?;
    match decision.item {
        Some(item) => output.set_item("item", render_dispatch_summary_item(py, item)?)?,
        None => output.set_item("item", py.None())?,
    }
    output.set_item(
        "dispatch_validation_required",
        decision.dispatch_validation_required,
    )?;
    output.set_item(
        "dispatch_validation_hint",
        decision.dispatch_validation_hint,
    )?;
    Ok(output)
}

fn render_plan_candidates_aggregate_summary<'py>(
    py: Python<'py>,
    summary: PlanCandidatesAggregateSummary,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("scanned_plan_count", summary.scanned_plan_count)?;
    output.set_item("candidate_plan_count", summary.candidate_plan_count)?;
    output.set_item("open_item_count", summary.open_item_count)?;
    output.set_item("taskable_item_count", summary.taskable_item_count)?;
    output.set_item("linked_task_count", summary.linked_task_count)?;
    output.set_item(
        "local_unpublished_head_count",
        summary.local_unpublished_head_count,
    )?;
    Ok(output)
}

fn render_plan_candidates_payload<'py>(
    py: Python<'py>,
    payload: PlanCandidatesPayload,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("scope", payload.scope)?;
    output.set_item("remote", payload.remote)?;
    output.set_item("repo_name", payload.repo_name)?;
    output.set_item(
        "summary",
        render_plan_candidates_aggregate_summary(py, payload.summary)?,
    )?;
    let candidates = PyList::empty(py);
    for candidate in payload.candidates {
        candidates.append(render_plan_dispatch_summary(py, candidate)?)?;
    }
    output.set_item("candidates", candidates)?;
    Ok(output)
}

fn render_plan_task_link_indexes<'py>(
    py: Python<'py>,
    indexes: PlanTaskLinkIndexes,
) -> PyResult<Bound<'py, PyTuple>> {
    let by_item = PyDict::new(py);
    for ((plan_id, plan_item_ref), tasks) in indexes.by_item {
        let key = PyTuple::new(py, [plan_id, plan_item_ref])?;
        let task_rows = PyList::empty(py);
        for task in tasks {
            task_rows.append(render_linked_task_summary(py, task)?)?;
        }
        by_item.set_item(key, task_rows)?;
    }
    let by_plan = PyDict::new(py);
    for (plan_id, tasks) in indexes.by_plan {
        let task_rows = PyList::empty(py);
        for task in tasks {
            task_rows.append(render_linked_task_summary(py, task)?)?;
        }
        by_plan.set_item(plan_id, task_rows)?;
    }
    PyTuple::new(py, [by_item.into_any(), by_plan.into_any()])
}

fn render_workflow_status_details<'py>(
    py: Python<'py>,
    details: WorkflowStatusDetails,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("normalized_status", details.normalized_status)?;
    output.set_item("display_label", details.display_label)?;
    output.set_item("closed", details.closed)?;
    Ok(output)
}

fn render_workflow_result_envelope<'py>(
    py: Python<'py>,
    envelope: WorkflowResultEnvelope,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("ok", envelope.ok)?;
    output.set_item("kind", envelope.kind)?;
    output.set_item("value", envelope.value)?;
    match envelope.error {
        Some(error) => {
            let error_row = PyDict::new(py);
            error_row.set_item("code", error.code)?;
            error_row.set_item("message", error.message)?;
            error_row.set_item("detail", error.detail)?;
            output.set_item("error", error_row)?;
        }
        None => output.set_item("error", py.None())?,
    }
    Ok(output)
}

fn render_parsed_plan<'py>(py: Python<'py>, parsed: ParsedPlan) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("markdown_text", parsed.markdown_text)?;
    output.set_item("plan_ref_count", parsed.plan_ref_count)?;
    output.set_item("item_count", parsed.item_count)?;
    let refs = PyList::empty(py);
    for item in parsed.plan_refs {
        let row = PyDict::new(py);
        row.set_item("plan_ref", item.plan_ref)?;
        row.set_item("heading_title", item.heading_title)?;
        row.set_item("heading_level", item.heading_level)?;
        row.set_item("line_number", item.line_number)?;
        refs.append(row)?;
    }
    output.set_item("plan_refs", refs)?;
    output.set_item("items", render_plan_items(py, parsed.items)?)?;
    Ok(output)
}

fn render_plan_ref_identity_payload<'py>(
    py: Python<'py>,
    payload: PlanRefIdentityPayload,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("plan_ref_count", payload.plan_ref_count)?;
    let refs = PyList::empty(py);
    for item in payload.plan_refs {
        let row = PyDict::new(py);
        row.set_item("plan_ref", item.plan_ref)?;
        row.set_item("heading_title", item.heading_title)?;
        row.set_item("heading_level", item.heading_level)?;
        row.set_item("line_number", item.line_number)?;
        refs.append(row)?;
    }
    output.set_item("plan_refs", refs)?;
    Ok(output)
}

fn render_sync_prune_decisions<'py>(
    py: Python<'py>,
    payload: SyncPruneDecisionPayload,
) -> PyResult<Bound<'py, PyDict>> {
    let output = PyDict::new(py);
    output.set_item("scope", payload.scope)?;
    output.set_item("tracked_artifact_count", payload.tracked_artifact_count)?;
    output.set_item("synced_artifact_count", payload.synced_artifact_count)?;
    output.set_item("retained_paths", payload.retained_paths)?;
    output.set_item("prune_paths", payload.prune_paths)?;
    output.set_item("prune_count", payload.prune_count)?;
    Ok(output)
}

fn parse_plan_item_seeds(
    items: Option<Bound<'_, PyAny>>,
) -> PyResult<Vec<NormalizedPlanItemSeed>> {
    let Some(items) = items else {
        return Ok(Vec::new());
    };
    if items.is_none() {
        return Ok(Vec::new());
    }
    let list = items
        .cast::<PyList>()
        .map_err(|_| PyValueError::new_err("Plan items must be a list."))?;
    let mut seeds = Vec::with_capacity(list.len());
    for entry in list.iter() {
        let item = entry
            .cast::<PyDict>()
            .map_err(|_| PyValueError::new_err("Plan items must be objects."))?;
        let plan_item_ref = normalize_optional_text_value(item.get_item("plan_item_ref")?)
            .ok_or_else(|| PyValueError::new_err("Plan items must include plan_item_ref."))?;
        let checkbox_state = normalize_optional_text_value(item.get_item("checkbox_state")?)
            .unwrap_or_else(|| "none".to_string());
        let heading_path = heading_path_value(item, plan_item_ref.as_str())?;
        let line_number = line_number_value(item, plan_item_ref.as_str())?;
        seeds.push(NormalizedPlanItemSeed {
            plan_item_ref,
            text: normalize_optional_text_value(item.get_item("text")?).unwrap_or_default(),
            checkbox_state,
            heading_path,
            line_number,
        });
    }
    Ok(seeds)
}

fn parse_dispatch_tasks(items: Option<Bound<'_, PyAny>>) -> PyResult<Vec<DispatchTaskInput>> {
    let Some(items) = items else {
        return Ok(Vec::new());
    };
    if items.is_none() {
        return Ok(Vec::new());
    }
    let list = items
        .cast::<PyList>()
        .map_err(|_| PyValueError::new_err("Plan dispatch tasks must be a list."))?;
    let mut tasks = Vec::with_capacity(list.len());
    for entry in list.iter() {
        let item = entry
            .cast::<PyDict>()
            .map_err(|_| PyValueError::new_err("Plan dispatch tasks must be objects."))?;
        tasks.push(DispatchTaskInput {
            task_id: normalize_optional_text_value(item.get_item("task_id")?),
            title: normalize_optional_text_value(item.get_item("title")?),
            status: normalize_optional_text_value(item.get_item("status")?),
            planning_state: normalize_optional_text_value(item.get_item("planning_state")?),
            origin_plan_revision_id: normalize_optional_text_value(
                item.get_item("origin_plan_revision_id")?,
            ),
            plan_drift_state: normalize_optional_text_value(item.get_item("plan_drift_state")?),
            plan_id: normalize_optional_text_value(item.get_item("plan_id")?),
            plan_item_ref: normalize_optional_text_value(item.get_item("plan_item_ref")?),
        });
    }
    Ok(tasks)
}

fn parse_dispatch_plan_item(item: &Bound<'_, PyDict>) -> PyResult<DispatchPlanItemInput> {
    Ok(DispatchPlanItemInput {
        plan_item_ref: normalize_optional_text_value(item.get_item("plan_item_ref")?),
        text: normalize_optional_text_value(item.get_item("text")?).unwrap_or_default(),
        checkbox_state: normalize_optional_text_value(item.get_item("checkbox_state")?)
            .unwrap_or_default(),
        heading_path: heading_path_value(item, "dispatch-item")?,
        line_number: line_number_value(item, "dispatch-item")?,
    })
}

fn parse_dispatch_revision(revision: &Bound<'_, PyDict>) -> PyResult<DispatchRevisionInput> {
    Ok(DispatchRevisionInput {
        plan_revision_id: normalize_optional_text_value(revision.get_item("plan_revision_id")?),
        revision_number: optional_int_value(
            revision.get_item("revision_number")?,
            "plan revision revision_number",
        )?,
        artifact_path: normalize_optional_text_value(revision.get_item("artifact_path")?),
        artifact_selector: normalize_optional_text_value(revision.get_item("artifact_selector")?),
        artifact_heading: normalize_optional_text_value(revision.get_item("artifact_heading")?),
        publication_state: normalize_optional_text_value(revision.get_item("publication_state")?),
        items: parse_dispatch_plan_items(revision.get_item("items")?)?,
    })
}

fn parse_dispatch_plan(plan: Bound<'_, PyDict>) -> PyResult<DispatchPlanInput> {
    let head_revision = match plan.get_item("head_revision")? {
        Some(value) if !value.is_none() => Some(parse_dispatch_revision(
            value
                .cast::<PyDict>()
                .map_err(|_| PyValueError::new_err("Plan head_revision must be an object."))?,
        )?),
        _ => None,
    };
    Ok(DispatchPlanInput {
        plan_id: normalize_optional_text_value(plan.get_item("plan_id")?),
        title: normalize_optional_text_value(plan.get_item("title")?),
        status: normalize_optional_text_value(plan.get_item("status")?),
        repo_name: normalize_optional_text_value(plan.get_item("repo_name")?),
        publication_state: normalize_optional_text_value(plan.get_item("publication_state")?),
        published_plan_id: normalize_optional_text_value(plan.get_item("published_plan_id")?),
        published_head_revision_id: normalize_optional_text_value(
            plan.get_item("published_head_revision_id")?,
        ),
        head_revision_id: normalize_optional_text_value(plan.get_item("head_revision_id")?),
        head_revision,
    })
}

fn parse_local_plan_publish_shadow(shadow: &Bound<'_, PyDict>) -> PyResult<LocalPlanPublishShadow> {
    Ok(LocalPlanPublishShadow {
        plan_id: normalize_optional_text_value(shadow.get_item("plan_id")?),
        publication_state: normalize_optional_text_value(shadow.get_item("publication_state")?),
        head_publication_state: normalize_optional_text_value(
            shadow.get_item("head_publication_state")?,
        ),
        head_revision_id: normalize_optional_text_value(shadow.get_item("head_revision_id")?),
        head_revision_number: optional_int_value(
            shadow.get_item("head_revision_number")?,
            "local publication head_revision_number",
        )?,
        published_plan_id: normalize_optional_text_value(shadow.get_item("published_plan_id")?),
        published_head_revision_id: normalize_optional_text_value(
            shadow.get_item("published_head_revision_id")?,
        ),
        unpublished_head: optional_bool_value(shadow.get_item("unpublished_head")?)?,
    })
}

fn parse_linked_task_summary(task: &Bound<'_, PyDict>) -> LinkedTaskSummary {
    LinkedTaskSummary {
        task_id: normalize_optional_text_value(task.get_item("task_id").ok().flatten()),
        title: normalize_optional_text_value(task.get_item("title").ok().flatten()),
        status: normalize_optional_text_value(task.get_item("status").ok().flatten()),
        planning_state: normalize_optional_text_value(
            task.get_item("planning_state").ok().flatten(),
        ),
        origin_plan_revision_id: normalize_optional_text_value(
            task.get_item("origin_plan_revision_id").ok().flatten(),
        ),
        plan_drift_state: normalize_optional_text_value(
            task.get_item("plan_drift_state").ok().flatten(),
        ),
    }
}

fn parse_dispatch_summary_item(item: &Bound<'_, PyDict>) -> PyResult<DispatchSummaryItem> {
    let mut linked_tasks = Vec::new();
    if let Some(value) = item.get_item("linked_tasks")? {
        if !value.is_none() {
            let linked_tasks_list = value
                .cast::<PyList>()
                .map_err(|_| PyValueError::new_err("Plan dispatch linked_tasks must be a list."))?;
            linked_tasks.reserve(linked_tasks_list.len());
            for entry in linked_tasks_list.iter() {
                let task = entry.cast::<PyDict>().map_err(|_| {
                    PyValueError::new_err("Plan dispatch linked_tasks entries must be objects.")
                })?;
                linked_tasks.push(parse_linked_task_summary(task));
            }
        }
    }

    Ok(DispatchSummaryItem {
        plan_item_ref: normalize_optional_text_value(item.get_item("plan_item_ref")?),
        text: normalize_optional_text_value(item.get_item("text")?).unwrap_or_default(),
        checkbox_state: normalize_optional_text_value(item.get_item("checkbox_state")?)
            .unwrap_or_default(),
        heading_path: heading_path_value(item, "dispatch-summary-item")?,
        line_number: line_number_value(item, "dispatch-summary-item")?,
        linked_tasks,
        taskable: optional_bool_value(item.get_item("taskable")?)?,
        taskable_blocker: normalize_optional_text_value(item.get_item("taskable_blocker")?),
    })
}

fn parse_plan_dispatch_summary(summary: &Bound<'_, PyDict>) -> PyResult<PlanDispatchSummary> {
    let mut linked_task_status_counts = std::collections::BTreeMap::new();
    if let Some(value) = summary.get_item("linked_task_status_counts")? {
        if !value.is_none() {
            let status_counts_dict = value.cast::<PyDict>().map_err(|_| {
                PyValueError::new_err(
                    "Plan dispatch summary linked_task_status_counts must be an object.",
                )
            })?;
            for (key, value) in status_counts_dict.iter() {
                let status = normalize_text_from_any(key).unwrap_or_else(|| "unknown".to_string());
                let count = value.extract::<usize>().map_err(|_| {
                    PyValueError::new_err("Plan dispatch status counts must be integers.")
                })?;
                linked_task_status_counts.insert(status, count);
            }
        }
    }

    Ok(PlanDispatchSummary {
        plan_id: normalize_optional_text_value(summary.get_item("plan_id")?),
        title: normalize_optional_text_value(summary.get_item("title")?),
        status: normalize_optional_text_value(summary.get_item("status")?),
        repo_name: normalize_optional_text_value(summary.get_item("repo_name")?),
        artifact_path: normalize_optional_text_value(summary.get_item("artifact_path")?),
        artifact_selector: normalize_optional_text_value(summary.get_item("artifact_selector")?),
        artifact_heading: normalize_optional_text_value(summary.get_item("artifact_heading")?),
        plan_revision_id: normalize_optional_text_value(summary.get_item("plan_revision_id")?),
        revision_number: optional_int_value(
            summary.get_item("revision_number")?,
            "plan dispatch summary revision_number",
        )?,
        publication_state: normalize_optional_text_value(summary.get_item("publication_state")?),
        head_publication_state: normalize_optional_text_value(
            summary.get_item("head_publication_state")?,
        ),
        published_plan_id: normalize_optional_text_value(summary.get_item("published_plan_id")?),
        published_head_revision_id: normalize_optional_text_value(
            summary.get_item("published_head_revision_id")?,
        ),
        local_publication: match summary.get_item("local_publication")? {
            Some(value) if !value.is_none() => Some(parse_local_plan_publish_shadow(
                value.cast::<PyDict>().map_err(|_| {
                    PyValueError::new_err("Plan dispatch local_publication must be an object.")
                })?,
            )?),
            _ => None,
        },
        local_unpublished_head: optional_bool_value(summary.get_item("local_unpublished_head")?)?,
        item_count: optional_int_value(
            summary.get_item("item_count")?,
            "plan dispatch summary item_count",
        )?
        .unwrap_or(0) as usize,
        open_item_count: optional_int_value(
            summary.get_item("open_item_count")?,
            "plan dispatch summary open_item_count",
        )?
        .unwrap_or(0) as usize,
        done_item_count: optional_int_value(
            summary.get_item("done_item_count")?,
            "plan dispatch summary done_item_count",
        )?
        .unwrap_or(0) as usize,
        unref_open_item_count: optional_int_value(
            summary.get_item("unref_open_item_count")?,
            "plan dispatch summary unref_open_item_count",
        )?
        .unwrap_or(0) as usize,
        linked_open_item_count: optional_int_value(
            summary.get_item("linked_open_item_count")?,
            "plan dispatch summary linked_open_item_count",
        )?
        .unwrap_or(0) as usize,
        taskable_item_count: optional_int_value(
            summary.get_item("taskable_item_count")?,
            "plan dispatch summary taskable_item_count",
        )?
        .unwrap_or(0) as usize,
        linked_task_count: optional_int_value(
            summary.get_item("linked_task_count")?,
            "plan dispatch summary linked_task_count",
        )?
        .unwrap_or(0) as usize,
        linked_task_status_counts,
        items: parse_dispatch_summary_items(summary.get_item("items")?)?,
        open_items: parse_dispatch_summary_items(summary.get_item("open_items")?)?,
        taskable_items: parse_dispatch_summary_items(summary.get_item("taskable_items")?)?,
    })
}

fn parse_dispatch_summary_items(
    items: Option<Bound<'_, PyAny>>,
) -> PyResult<Vec<DispatchSummaryItem>> {
    let Some(items) = items else {
        return Ok(Vec::new());
    };
    if items.is_none() {
        return Ok(Vec::new());
    }
    let list = items
        .cast::<PyList>()
        .map_err(|_| PyValueError::new_err("Plan dispatch summary items must be a list."))?;
    let mut parsed = Vec::with_capacity(list.len());
    for entry in list.iter() {
        let item = entry
            .cast::<PyDict>()
            .map_err(|_| PyValueError::new_err("Plan dispatch summary items must be objects."))?;
        parsed.push(parse_dispatch_summary_item(item)?);
    }
    Ok(parsed)
}

fn parse_plan_dispatch_summaries(
    summaries: Option<Bound<'_, PyAny>>,
) -> PyResult<Vec<PlanDispatchSummary>> {
    let Some(summaries) = summaries else {
        return Ok(Vec::new());
    };
    if summaries.is_none() {
        return Ok(Vec::new());
    }
    let list = summaries
        .cast::<PyList>()
        .map_err(|_| PyValueError::new_err("Plan candidates summaries must be a list."))?;
    let mut parsed = Vec::with_capacity(list.len());
    for entry in list.iter() {
        let summary = entry
            .cast::<PyDict>()
            .map_err(|_| PyValueError::new_err("Plan candidates summaries must be objects."))?;
        parsed.push(parse_plan_dispatch_summary(summary)?);
    }
    Ok(parsed)
}

fn parse_parsed_plan(payload: Bound<'_, PyDict>) -> PyResult<ParsedPlan> {
    let markdown_text =
        normalize_optional_text_value(payload.get_item("markdown_text")?).unwrap_or_default();
    let plan_refs = parse_plan_section_refs(payload.get_item("plan_refs")?)?;
    let items = parse_plan_items_from_payload(payload.get_item("items")?)?;
    Ok(ParsedPlan {
        markdown_text,
        plan_ref_count: optional_int_value(
            payload.get_item("plan_ref_count")?,
            "parsed plan plan_ref_count",
        )?
        .unwrap_or(plan_refs.len() as i64) as usize,
        item_count: optional_int_value(payload.get_item("item_count")?, "parsed plan item_count")?
            .unwrap_or(items.len() as i64) as usize,
        plan_refs,
        items,
    })
}

fn parse_plan_section_refs(
    items: Option<Bound<'_, PyAny>>,
) -> PyResult<Vec<ait_core::plan_items::PlanSectionRef>> {
    let Some(items) = items else {
        return Ok(Vec::new());
    };
    if items.is_none() {
        return Ok(Vec::new());
    }
    let list = items
        .cast::<PyList>()
        .map_err(|_| PyValueError::new_err("Parsed plan refs must be a list."))?;
    let mut parsed = Vec::with_capacity(list.len());
    for entry in list.iter() {
        let item = entry
            .cast::<PyDict>()
            .map_err(|_| PyValueError::new_err("Parsed plan refs must be objects."))?;
        let plan_ref = normalize_optional_text_value(item.get_item("plan_ref")?)
            .ok_or_else(|| PyValueError::new_err("Parsed plan refs must include plan_ref."))?;
        let heading_title =
            normalize_optional_text_value(item.get_item("heading_title")?).unwrap_or_default();
        let heading_level =
            optional_int_value(item.get_item("heading_level")?, "parsed plan heading_level")?
                .unwrap_or(0) as usize;
        let line_number =
            optional_int_value(item.get_item("line_number")?, "parsed plan line_number")?
                .unwrap_or(0) as usize;
        parsed.push(ait_core::plan_items::PlanSectionRef {
            plan_ref,
            heading_title,
            heading_level,
            line_number,
        });
    }
    Ok(parsed)
}

fn parse_plan_items_from_payload(items: Option<Bound<'_, PyAny>>) -> PyResult<Vec<PlanItem>> {
    let seeds = parse_plan_item_seeds(items)?;
    normalize_plan_items(&seeds).map_err(PyValueError::new_err)
}

fn parse_string_list(values: Option<Bound<'_, PyAny>>) -> PyResult<Vec<String>> {
    let Some(values) = values else {
        return Ok(Vec::new());
    };
    if values.is_none() {
        return Ok(Vec::new());
    }
    let list = values
        .cast::<PyList>()
        .map_err(|_| PyValueError::new_err("String-list payloads must be lists."))?;
    let mut parsed = Vec::with_capacity(list.len());
    for entry in list.iter() {
        let text = entry
            .extract::<String>()
            .map_err(|_| PyValueError::new_err("String-list payloads must contain strings."))?;
        parsed.push(text);
    }
    Ok(parsed)
}

fn parse_dispatch_plan_items(
    items: Option<Bound<'_, PyAny>>,
) -> PyResult<Vec<DispatchPlanItemInput>> {
    let Some(items) = items else {
        return Ok(Vec::new());
    };
    if items.is_none() {
        return Ok(Vec::new());
    }
    let list = items
        .cast::<PyList>()
        .map_err(|_| PyValueError::new_err("Plan dispatch items must be a list."))?;
    let mut parsed = Vec::with_capacity(list.len());
    for entry in list.iter() {
        let item = entry
            .cast::<PyDict>()
            .map_err(|_| PyValueError::new_err("Plan dispatch items must be objects."))?;
        parsed.push(parse_dispatch_plan_item(item)?);
    }
    Ok(parsed)
}

fn heading_path_value(item: &Bound<'_, PyDict>, plan_item_ref: &str) -> PyResult<Vec<String>> {
    let Some(value) = item.get_item("heading_path")? else {
        return Ok(Vec::new());
    };
    if value.is_none() {
        return Ok(Vec::new());
    }
    let list = value.cast::<PyList>().map_err(|_| {
        PyValueError::new_err(format!(
            "Plan item {} heading_path must be a list.",
            plan_item_ref
        ))
    })?;
    let mut heading_path = Vec::new();
    for entry in list.iter() {
        if let Some(text) = normalize_text_from_any(entry) {
            heading_path.push(text);
        }
    }
    Ok(heading_path)
}

fn line_number_value(
    item: &Bound<'_, PyDict>,
    plan_item_ref: &str,
) -> PyResult<i64> {
    let Some(value) = item.get_item("line_number")? else {
        return Ok(0);
    };
    if value.is_none() || !value.is_truthy()? {
        return Ok(0);
    }
    if let Ok(number) = value.extract::<i64>() {
        return Ok(number);
    }
    if let Ok(number) = value.extract::<f64>() {
        const I64_UPPER_EXCLUSIVE_F64: f64 = 9_223_372_036_854_775_808.0;
        if number.is_finite()
            && number >= i64::MIN as f64
            && number < I64_UPPER_EXCLUSIVE_F64
        {
            return Ok(number.trunc() as i64);
        }
    }
    if let Ok(text) = value.extract::<String>() {
        if let Ok(number) = text.trim().parse::<i64>() {
            return Ok(number);
        }
    }
    Err(PyValueError::new_err(format!(
        "Plan item {} line_number must be an integer.",
        plan_item_ref
    )))
}

fn optional_int_value(value: Option<Bound<'_, PyAny>>, field_name: &str) -> PyResult<Option<i64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_none() {
        return Ok(None);
    }
    value
        .extract::<i64>()
        .map(Some)
        .map_err(|_| PyValueError::new_err(format!("{} must be an integer.", field_name)))
}

fn optional_bool_value(value: Option<Bound<'_, PyAny>>) -> PyResult<bool> {
    let Some(value) = value else {
        return Ok(false);
    };
    if value.is_none() {
        return Ok(false);
    }
    value
        .extract::<bool>()
        .map_err(|_| PyValueError::new_err("Dispatch boolean fields must be booleans."))
}

fn normalize_optional_text_value(value: Option<Bound<'_, PyAny>>) -> Option<String> {
    value.and_then(normalize_text_from_any)
}

fn normalize_namespace_prefix_value(value: Option<Bound<'_, PyAny>>) -> Option<String> {
    value.and_then(normalize_namespace_prefix_from_any)
}

fn string_or_empty(root: &Map<String, JsonValue>, object_key: &str, field_key: &str) -> String {
    root.get(object_key)
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get(field_key))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

fn string_or_empty_root(root: &Map<String, JsonValue>, field_key: &str) -> String {
    root.get(field_key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

fn int_or_zero_root(root: &Map<String, JsonValue>, field_key: &str) -> i64 {
    root.get(field_key).and_then(JsonValue::as_i64).unwrap_or(0)
}

fn bool_or_false_root(root: &Map<String, JsonValue>, field_key: &str) -> bool {
    root.get(field_key)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn normalize_text_from_any(value: Bound<'_, PyAny>) -> Option<String> {
    if value.is_none() {
        return None;
    }
    let text = value.str().ok()?.to_string_lossy().trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn normalize_namespace_prefix_from_any(value: Bound<'_, PyAny>) -> Option<String> {
    if value.is_none() {
        return None;
    }
    Some(value.str().ok()?.to_string_lossy().trim().to_string())
}

fn render_json_dict(py: Python<'_>, value: JsonValue) -> PyResult<Py<PyDict>> {
    match value {
        JsonValue::Object(values) => {
            let output = PyDict::new(py);
            for (key, entry) in values {
                output.set_item(key, json_value_to_py(py, &entry)?)?;
            }
            Ok(output.unbind())
        }
        _ => Err(PyRuntimeError::new_err(
            "Rust backend returned a non-dict payload.",
        )),
    }
}

fn render_json_value(py: Python<'_>, value: JsonValue) -> PyResult<Py<PyAny>> {
    json_value_to_py(py, &value)
}


fn render_plan_http_client_stats(
    py: Python<'_>,
    stats: PlanHttpClientStats,
) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        JsonValue::Object(ait_core::json_support::JsonMap::from_iter([
            ("base_url".to_string(), JsonValue::String(stats.base_url)),
            (
                "default_timeout_ms".to_string(),
                JsonValue::Number(stats.default_timeout_ms.into()),
            ),
            (
                "retry_attempts".to_string(),
                JsonValue::Number((stats.retry_attempts as u64).into()),
            ),
            (
                "retry_backoff_ms".to_string(),
                JsonValue::Number(stats.retry_backoff_ms.into()),
            ),
            (
                "pool_max_idle_per_host".to_string(),
                JsonValue::Number((stats.pool_max_idle_per_host as u64).into()),
            ),
            (
                "request_count".to_string(),
                JsonValue::Number((stats.request_count as u64).into()),
            ),
            (
                "retry_count".to_string(),
                JsonValue::Number((stats.retry_count as u64).into()),
            ),
            ("closed".to_string(), JsonValue::Bool(stats.closed)),
        ])),
    )
}

#[pyfunction(name = "plan_ports_normalize_plan_store_read_request")]
fn plan_ports_normalize_plan_store_read_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_store_read_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "validate_planning_session_join_payload")]
fn validate_planning_session_join_payload_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        validate_planning_session_join_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_ports_normalize_plan_remote_transport")]
fn plan_ports_normalize_plan_remote_transport_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_remote_transport_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_ports_normalize_plan_remote_request")]
fn plan_ports_normalize_plan_remote_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        normalize_plan_remote_request_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_ports_normalize_artifact_resolver_request")]
fn plan_ports_normalize_artifact_resolver_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_artifact_resolver_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_ports_normalize_artifact_publish_request")]
fn plan_ports_normalize_artifact_publish_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_artifact_publish_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_ports_normalize_linked_task_lookup_payload")]
fn plan_ports_normalize_linked_task_lookup_payload_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        normalize_linked_task_lookup_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_ports_normalize_plan_config_runtime_facts")]
fn plan_ports_normalize_plan_config_runtime_facts_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_config_runtime_facts_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_ports_normalize_plan_connection_manager_stats")]
fn plan_ports_normalize_plan_connection_manager_stats_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_connection_manager_stats_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_config_runtime_normalize_selection_request")]
fn plan_config_runtime_normalize_selection_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_runtime_selection_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_config_runtime_build_selection_facts")]
fn plan_config_runtime_build_selection_facts_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_runtime_selection_facts_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_config_runtime_normalize_selection_facts")]
fn plan_config_runtime_normalize_selection_facts_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_runtime_selection_facts_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_config_runtime_normalize_compatibility")]
fn plan_config_runtime_normalize_compatibility_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_runtime_compatibility_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_config_runtime_normalize_readiness")]
fn plan_config_runtime_normalize_readiness_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_runtime_readiness_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_config_runtime_normalize_doctor")]
fn plan_config_runtime_normalize_doctor_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        normalize_plan_runtime_doctor_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_diagnostics_normalize_request")]
fn plan_diagnostics_normalize_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_diagnostics_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_diagnostics_normalize_backend_identity")]
fn plan_diagnostics_normalize_backend_identity_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_backend_identity_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_diagnostics_normalize_compatibility")]
fn plan_diagnostics_normalize_compatibility_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_diagnostics_compatibility_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_diagnostics_normalize_readiness")]
fn plan_diagnostics_normalize_readiness_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_diagnostics_readiness_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_diagnostics_normalize_doctor")]
fn plan_diagnostics_normalize_doctor_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_diagnostics_doctor_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_diagnostics_build_backend_identity_facts")]
fn plan_diagnostics_build_backend_identity_facts_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_backend_identity_facts_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_diagnostics_build_storage_readiness_facts")]
fn plan_diagnostics_build_storage_readiness_facts_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyAny>> {
    let payload =
        build_plan_storage_readiness_facts_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "plan_diagnostics_build_compatibility_status")]
fn plan_diagnostics_build_compatibility_status_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = build_plan_diagnostics_compatibility_status_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_diagnostics_build_readiness_status")]
fn plan_diagnostics_build_readiness_status_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = build_plan_diagnostics_readiness_status_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_diagnostics_build_doctor_facts")]
fn plan_diagnostics_build_doctor_facts_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_diagnostics_doctor_facts_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_normalize_list_request")]
fn plan_application_normalize_list_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_list_service_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_build_list")]
fn plan_application_build_list_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_list_service_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_normalize_show_request")]
fn plan_application_normalize_show_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_show_service_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_build_show")]
fn plan_application_build_show_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_show_service_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_normalize_revisions_request")]
fn plan_application_normalize_revisions_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_revisions_service_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_build_revisions")]
fn plan_application_build_revisions_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_revisions_service_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_normalize_items_request")]
fn plan_application_normalize_items_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_items_service_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_build_items")]
fn plan_application_build_items_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_items_service_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_normalize_candidates_request")]
fn plan_application_normalize_candidates_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_candidates_service_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_build_candidates")]
fn plan_application_build_candidates_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_candidates_service_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_normalize_inspect_request")]
fn plan_application_normalize_inspect_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_inspect_service_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_build_inspect")]
fn plan_application_build_inspect_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_inspect_service_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_normalize_sync_request")]
fn plan_application_normalize_sync_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_sync_service_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_application_build_sync")]
fn plan_application_build_sync_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_sync_service_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_normalize_list_request")]
fn plan_command_normalize_list_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_list_command_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_build_list")]
fn plan_command_build_list_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyList>> {
    let payload =
        build_plan_list_command_payload_json(payload_json).map_err(PyValueError::new_err)?;
    let values = match payload {
        JsonValue::Array(values) => values,
        _ => {
            return Err(PyRuntimeError::new_err(
                "Rust plan command surface backend returned a non-list payload for `plan_command_build_list`.",
            ))
        }
    };
    render_json_list(py, values)
}

#[pyfunction(name = "plan_command_normalize_show_request")]
fn plan_command_normalize_show_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_show_command_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_build_show")]
fn plan_command_build_show_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_show_command_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_normalize_revisions_request")]
fn plan_command_normalize_revisions_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_revisions_command_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_build_revisions")]
fn plan_command_build_revisions_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyList>> {
    let payload =
        build_plan_revisions_command_payload_json(payload_json).map_err(PyValueError::new_err)?;
    let values = match payload {
        JsonValue::Array(values) => values,
        _ => {
            return Err(PyRuntimeError::new_err(
                "Rust plan command surface backend returned a non-list payload for `plan_command_build_revisions`.",
            ))
        }
    };
    render_json_list(py, values)
}

#[pyfunction(name = "plan_command_normalize_items_request")]
fn plan_command_normalize_items_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_items_command_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_build_items")]
fn plan_command_build_items_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_items_command_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_normalize_candidates_request")]
fn plan_command_normalize_candidates_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_candidates_command_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_build_candidates")]
fn plan_command_build_candidates_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_candidates_command_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_normalize_inspect_request")]
fn plan_command_normalize_inspect_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_inspect_command_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_build_inspect")]
fn plan_command_build_inspect_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_inspect_command_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_normalize_sync_request")]
fn plan_command_normalize_sync_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_sync_command_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_build_sync")]
fn plan_command_build_sync_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_sync_command_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_execute_list")]
fn plan_command_execute_list_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyAny>> {
    let payload_json = payload_json.to_string();
    let payload = py
        .detach(move || execute_plan_list_command_request_json(payload_json.as_str()))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "plan_command_execute_show")]
fn plan_command_execute_show_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload_json = payload_json.to_string();
    let payload = py
        .detach(move || execute_plan_show_command_request_json(payload_json.as_str()))
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_execute_revisions")]
fn plan_command_execute_revisions_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyAny>> {
    let payload_json = payload_json.to_string();
    let payload = py
        .detach(move || execute_plan_revisions_command_request_json(payload_json.as_str()))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "plan_command_execute_items")]
fn plan_command_execute_items_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload_json = payload_json.to_string();
    let payload = py
        .detach(move || execute_plan_items_command_request_json(payload_json.as_str()))
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_execute_candidates")]
fn plan_command_execute_candidates_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload_json = payload_json.to_string();
    let payload = py
        .detach(move || execute_plan_candidates_command_request_json(payload_json.as_str()))
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_execute_inspect")]
fn plan_command_execute_inspect_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload_json = payload_json.to_string();
    let payload = py
        .detach(move || execute_plan_inspect_command_request_json(payload_json.as_str()))
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_command_execute_sync")]
fn plan_command_execute_sync_py(py: Python<'_>, payload_json: &str) -> PyResult<Py<PyDict>> {
    let payload_json = payload_json.to_string();
    let payload = py
        .detach(move || execute_plan_sync_command_request_json(payload_json.as_str()))
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}


#[pyfunction(name = "plan_provenance_normalize_revision_provenance")]
fn plan_provenance_normalize_revision_provenance_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_revision_provenance_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_provenance_build_revision_provenance")]
fn plan_provenance_build_revision_provenance_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_revision_provenance_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_normalize_timestamp_request")]
fn plan_time_identity_normalize_timestamp_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_timestamp_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_normalize_timestamp")]
fn plan_time_identity_normalize_timestamp_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        normalize_plan_timestamp_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_build_timestamp")]
fn plan_time_identity_build_timestamp_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = build_plan_timestamp_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_normalize_sequence_identity_request")]
fn plan_time_identity_normalize_sequence_identity_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_sequence_identity_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_normalize_sequence_identity")]
fn plan_time_identity_normalize_sequence_identity_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_sequence_identity_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_build_sequence_identity")]
fn plan_time_identity_build_sequence_identity_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_sequence_identity_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_normalize_workflow_id_request")]
fn plan_time_identity_normalize_workflow_id_request_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_workflow_id_request_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_normalize_workflow_id")]
fn plan_time_identity_normalize_workflow_id_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        normalize_plan_workflow_id_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_build_workflow_id")]
fn plan_time_identity_build_workflow_id_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_workflow_id_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_normalize_temporal_ordering")]
fn plan_time_identity_normalize_temporal_ordering_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload = normalize_plan_temporal_ordering_payload_json(payload_json)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "plan_time_identity_build_temporal_ordering")]
fn plan_time_identity_build_temporal_ordering_py(
    py: Python<'_>,
    payload_json: &str,
) -> PyResult<Py<PyDict>> {
    let payload =
        build_plan_temporal_ordering_payload_json(payload_json).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

fn render_json_list(py: Python<'_>, values: Vec<JsonValue>) -> PyResult<Py<PyList>> {
    let output = PyList::empty(py);
    for value in values {
        output.append(json_value_to_py(py, &value)?)?;
    }
    Ok(output.unbind())
}

fn json_value_to_py(py: Python<'_>, value: &JsonValue) -> PyResult<Py<PyAny>> {
    match value {
        JsonValue::Null => Ok(py.None()),
        JsonValue::Bool(value) => Ok(PyBool::new(py, *value).to_owned().unbind().into_any()),
        JsonValue::Number(value) => {
            if let Some(number) = value.as_i64() {
                Ok(number.into_pyobject(py)?.unbind().into_any())
            } else if let Some(number) = value.as_u64() {
                Ok(number.into_pyobject(py)?.unbind().into_any())
            } else if let Some(number) = value.as_f64() {
                Ok(number.into_pyobject(py)?.unbind().into_any())
            } else {
                Err(PyRuntimeError::new_err(
                    "Rust JSON number could not be converted to Python.",
                ))
            }
        }
        JsonValue::String(value) => Ok(value.into_pyobject(py)?.unbind().into_any()),
        JsonValue::Array(values) => {
            let output = PyList::empty(py);
            for entry in values {
                output.append(json_value_to_py(py, entry)?)?;
            }
            Ok(output.unbind().into_any())
        }
        JsonValue::Object(values) => {
            let output = PyDict::new(py);
            for (key, entry) in values {
                output.set_item(key, json_value_to_py(py, entry)?)?;
            }
            Ok(output.unbind().into_any())
        }
    }
}


fn release_store_py_error(message: String) -> PyErr {
    if message.starts_with("Unknown local release") || message.starts_with("Unknown release") {
        PyKeyError::new_err(message)
    } else {
        PyValueError::new_err(message)
    }
}

fn plan_http_py_error(err: PlanHttpClientError) -> PyErr {
    match err {
        PlanHttpClientError::Invalid(message) => PyValueError::new_err(message),
        PlanHttpClientError::Remote(message)
        | PlanHttpClientError::Transport(message)
        | PlanHttpClientError::Closed(message) => PyRuntimeError::new_err(message),
        response @ PlanHttpClientError::RemoteResponse { .. } => {
            PyRuntimeError::new_err(response.to_string())
        }
    }
}

fn plan_filesystem_py_error(err: PlanFilesystemError) -> PyErr {
    match err {
        PlanFilesystemError::Invalid(message) => PyValueError::new_err(message),
        PlanFilesystemError::NotFound(message) => PyFileNotFoundError::new_err(message),
        PlanFilesystemError::MissingEntry(message) => PyKeyError::new_err(message),
        PlanFilesystemError::Io(message) => PyRuntimeError::new_err(message),
    }
}

fn register_render_parse(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(
        plan_ports_normalize_plan_store_read_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        validate_planning_session_join_payload_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_ports_normalize_plan_remote_transport_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_ports_normalize_plan_remote_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_ports_normalize_artifact_resolver_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_ports_normalize_artifact_publish_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_ports_normalize_linked_task_lookup_payload_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_ports_normalize_plan_config_runtime_facts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_ports_normalize_plan_connection_manager_stats_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_config_runtime_normalize_selection_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_config_runtime_build_selection_facts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_config_runtime_normalize_selection_facts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_config_runtime_normalize_compatibility_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_config_runtime_normalize_readiness_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_config_runtime_normalize_doctor_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_normalize_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_normalize_backend_identity_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_normalize_compatibility_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_normalize_readiness_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_normalize_doctor_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_build_backend_identity_facts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_build_storage_readiness_facts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_build_compatibility_status_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_build_readiness_status_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_diagnostics_build_doctor_facts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_application_normalize_list_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_application_build_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_application_normalize_show_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_application_build_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_application_normalize_revisions_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_application_build_revisions_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_application_normalize_items_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_application_build_items_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_application_normalize_candidates_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_application_build_candidates_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_application_normalize_inspect_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_application_build_inspect_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_application_normalize_sync_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_application_build_sync_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_command_normalize_list_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_command_build_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_command_normalize_show_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_command_build_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_command_normalize_revisions_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_command_build_revisions_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_command_normalize_items_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_command_build_items_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_command_normalize_candidates_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_command_build_candidates_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_command_normalize_inspect_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_command_build_inspect_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_command_normalize_sync_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_command_build_sync_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_command_execute_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_command_execute_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_command_execute_revisions_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_command_execute_items_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_command_execute_candidates_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_command_execute_inspect_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_command_execute_sync_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_provenance_normalize_revision_provenance_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_provenance_build_revision_provenance_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_normalize_timestamp_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_normalize_timestamp_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_build_timestamp_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_normalize_sequence_identity_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_normalize_sequence_identity_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_build_sequence_identity_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_normalize_workflow_id_request_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_normalize_workflow_id_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_build_workflow_id_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_normalize_temporal_ordering_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_time_identity_build_temporal_ordering_py,
        module
    )?)?;
    Ok(())
}
