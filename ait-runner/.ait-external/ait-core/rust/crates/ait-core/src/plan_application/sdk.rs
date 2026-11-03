//! JSON adapters for retained pure Plan SDK capabilities.
//! Existing Plan parsers, domain functions and result projections own semantics.
use super::{text_rendering::*, validation_helpers::*};
use crate::{
    json_support::{json, JsonMap, JsonValue},
    plan_dispatch::{
        self as dispatch, DispatchPlanInput, DispatchRevisionInput, DispatchSummaryItem,
        DispatchTaskInput, LinkedTaskSummary, LocalPlanPublishShadow, PlanDispatchSummary,
    },
};
use std::collections::BTreeMap;

fn revision_with_default_items(value: &JsonValue) -> JsonValue {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        if object.get("items").is_none_or(JsonValue::is_null) {
            object.insert("items".to_string(), json!([]));
        }
    }
    value
}

fn parse_plan(value: &JsonValue) -> Result<DispatchPlanInput, String> {
    let mut value = value.clone();
    if let Some(revision) = value
        .get_mut("head_revision")
        .filter(|value| !value.is_null())
    {
        *revision = revision_with_default_items(revision);
    }
    parse_dispatch_plan_from_value(&value)
}

fn parse_revision(value: Option<&JsonValue>) -> Result<Option<DispatchRevisionInput>, String> {
    value
        .filter(|value| !value.is_null())
        .map(|value| parse_dispatch_revision_from_value(&revision_with_default_items(value)))
        .transpose()
}

fn parse_tasks(value: Option<&JsonValue>) -> Result<Vec<DispatchTaskInput>, String> {
    match value.filter(|value| !value.is_null()) {
        Some(value) => parse_dispatch_tasks_from_value(Some(value)),
        None => Ok(Vec::new()),
    }
}

fn parse_shadow(value: Option<&JsonValue>) -> Result<Option<LocalPlanPublishShadow>, String> {
    value
        .filter(|value| !value.is_null())
        .map(parse_local_plan_publish_shadow_from_value)
        .transpose()
}

fn parse_list<T>(
    value: Option<&JsonValue>,
    label: &str,
    parse: impl Fn(&JsonValue) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    match value.filter(|value| !value.is_null()) {
        Some(value) => require_array(Some(value), label)?
            .iter()
            .map(parse)
            .collect(),
        None => Ok(Vec::new()),
    }
}

fn parse_linked_task(value: &JsonValue) -> Result<LinkedTaskSummary, String> {
    let task = parse_dispatch_task_from_value(value)?;
    Ok(LinkedTaskSummary {
        task_id: task.task_id,
        title: task.title,
        status: task.status,
        planning_state: task.planning_state,
        origin_plan_revision_id: task.origin_plan_revision_id,
        plan_drift_state: task.plan_drift_state,
    })
}

fn parse_summary_item(value: &JsonValue) -> Result<DispatchSummaryItem, String> {
    let row = require_object(Some(value), "plan dispatch summary item")?;
    let item = parse_dispatch_plan_item_from_value(value)?;
    Ok(DispatchSummaryItem {
        plan_item_ref: item.plan_item_ref,
        text: item.text,
        checkbox_state: item.checkbox_state,
        heading_path: item.heading_path,
        line_number: item.line_number,
        linked_tasks: parse_list(row.get("linked_tasks"), "linked_tasks", parse_linked_task)?,
        taskable: optional_bool_with_default(row.get("taskable"), false, "taskable")?,
        taskable_blocker: optional_text(row.get("taskable_blocker"))?,
    })
}

fn count(value: Option<&JsonValue>, label: &str) -> Result<usize, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(0),
        Some(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| format!("{label} must be a nonnegative native-size integer.")),
    }
}

fn parse_summary(value: &JsonValue) -> Result<PlanDispatchSummary, String> {
    let row = require_object(Some(value), "plan dispatch summary")?;
    let mut linked_task_status_counts = BTreeMap::new();
    if let Some(value) = row
        .get("linked_task_status_counts")
        .filter(|value| !value.is_null())
    {
        for (key, value) in require_object(Some(value), "linked_task_status_counts")? {
            let key = normalize_optional_text(Some(key)).unwrap_or_else(|| "unknown".to_string());
            linked_task_status_counts.insert(key, count(Some(value), "linked task status count")?);
        }
    }
    Ok(PlanDispatchSummary {
        plan_id: optional_text(row.get("plan_id"))?,
        title: optional_text(row.get("title"))?,
        status: optional_text(row.get("status"))?,
        repo_name: optional_text(row.get("repo_name"))?,
        artifact_path: optional_text(row.get("artifact_path"))?,
        artifact_selector: optional_text(row.get("artifact_selector"))?,
        artifact_heading: optional_text(row.get("artifact_heading"))?,
        plan_revision_id: optional_text(row.get("plan_revision_id"))?,
        revision_number: optional_i64(row.get("revision_number"), "revision_number")?,
        publication_state: optional_text(row.get("publication_state"))?,
        head_publication_state: optional_text(row.get("head_publication_state"))?,
        published_plan_id: optional_text(row.get("published_plan_id"))?,
        published_head_revision_id: optional_text(row.get("published_head_revision_id"))?,
        local_publication: parse_shadow(row.get("local_publication"))?,
        local_unpublished_head: optional_bool_with_default(
            row.get("local_unpublished_head"),
            false,
            "local_unpublished_head",
        )?,
        item_count: count(row.get("item_count"), "item_count")?,
        open_item_count: count(row.get("open_item_count"), "open_item_count")?,
        done_item_count: count(row.get("done_item_count"), "done_item_count")?,
        unref_open_item_count: count(row.get("unref_open_item_count"), "unref_open_item_count")?,
        linked_open_item_count: count(row.get("linked_open_item_count"), "linked_open_item_count")?,
        taskable_item_count: count(row.get("taskable_item_count"), "taskable_item_count")?,
        linked_task_count: count(row.get("linked_task_count"), "linked_task_count")?,
        linked_task_status_counts,
        items: parse_list(row.get("items"), "items", parse_summary_item)?,
        open_items: parse_list(row.get("open_items"), "open_items", parse_summary_item)?,
        taskable_items: parse_list(
            row.get("taskable_items"),
            "taskable_items",
            parse_summary_item,
        )?,
    })
}

pub fn plan_items_payload_json(
    plan: &JsonValue,
    revision: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    Ok(render_plan_items_payload_json(
        &dispatch::plan_items_payload(&parse_plan(plan)?, parse_revision(revision)?.as_ref()),
    ))
}

pub fn local_plan_publish_shadow_json(plan: Option<&JsonValue>) -> Result<JsonValue, String> {
    let plan = plan
        .filter(|value| !value.is_null())
        .map(parse_plan)
        .transpose()?;
    Ok(dispatch::local_plan_publish_shadow(plan.as_ref())
        .as_ref()
        .map(render_local_plan_publish_shadow_json)
        .unwrap_or(JsonValue::Null))
}

pub fn plan_dispatch_summary_json(
    plan: &JsonValue,
    tasks: Option<&JsonValue>,
    revision: Option<&JsonValue>,
    local_shadow: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    Ok(render_plan_dispatch_summary_json(
        &dispatch::plan_dispatch_summary(
            &parse_plan(plan)?,
            &parse_tasks(tasks)?,
            parse_revision(revision)?.as_ref(),
            parse_shadow(local_shadow)?.as_ref(),
        ),
    ))
}

pub fn compute_taskable_items_json(
    plan: &JsonValue,
    tasks: Option<&JsonValue>,
    revision: Option<&JsonValue>,
    local_shadow: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    Ok(JsonValue::Array(
        dispatch::compute_taskable_items(
            &parse_plan(plan)?,
            &parse_tasks(tasks)?,
            parse_revision(revision)?.as_ref(),
            parse_shadow(local_shadow)?.as_ref(),
        )
        .iter()
        .map(render_dispatch_summary_item_json)
        .collect(),
    ))
}

pub fn validate_dispatch_legality_json(
    plan: &JsonValue,
    tasks: Option<&JsonValue>,
    plan_item_ref: Option<&str>,
    revision: Option<&JsonValue>,
    local_shadow: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    let result = dispatch::validate_dispatch_legality(
        &parse_plan(plan)?,
        &parse_tasks(tasks)?,
        plan_item_ref,
        parse_revision(revision)?.as_ref(),
        parse_shadow(local_shadow)?.as_ref(),
    );
    Ok(
        json!({"plan_item_ref":result.plan_item_ref,"taskable":result.taskable,"taskable_blocker":result.taskable_blocker,"item":result.item.as_ref().map(render_dispatch_summary_item_json),"dispatch_validation_required":result.dispatch_validation_required,"dispatch_validation_hint":result.dispatch_validation_hint}),
    )
}

pub fn plan_task_link_indexes_json(tasks: Option<&JsonValue>) -> Result<JsonValue, String> {
    let indexes = dispatch::plan_task_link_indexes(&parse_tasks(tasks)?);
    let by_item = indexes.by_item.into_iter().map(|((plan_id, plan_item_ref), tasks)| json!({"plan_id":plan_id,"plan_item_ref":plan_item_ref,"tasks":tasks.iter().map(render_linked_task_summary_json).collect::<Vec<_>>()})).collect::<Vec<_>>();
    let by_plan = indexes
        .by_plan
        .into_iter()
        .map(|(id, tasks)| {
            (
                id,
                JsonValue::Array(tasks.iter().map(render_linked_task_summary_json).collect()),
            )
        })
        .collect::<JsonMap<_, _>>();
    Ok(json!({"by_item":by_item,"by_plan":by_plan}))
}

pub fn plan_candidates_payload_json(
    summaries: Option<&JsonValue>,
    scope: Option<&str>,
    repo_name: Option<&str>,
    remote: Option<&str>,
    include_all: bool,
) -> Result<JsonValue, String> {
    Ok(render_plan_candidates_payload_json(
        &dispatch::plan_candidates_payload(
            &parse_list(summaries, "plan candidate summaries", parse_summary)?,
            scope,
            repo_name,
            remote,
            include_all,
        ),
    ))
}
