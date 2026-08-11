//! Plan taskability and dispatch semantics remain concrete to plan-linked task
//! authoring. They are not shared-foundation traits or shared-domain kernels.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchPlanItemInput {
    pub plan_item_ref: Option<String>,
    pub text: String,
    pub checkbox_state: String,
    pub heading_path: Vec<String>,
    pub line_number: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchRevisionInput {
    pub plan_revision_id: Option<String>,
    pub revision_number: Option<i64>,
    pub artifact_path: Option<String>,
    pub artifact_selector: Option<String>,
    pub artifact_heading: Option<String>,
    pub publication_state: Option<String>,
    pub items: Vec<DispatchPlanItemInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchPlanInput {
    pub plan_id: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub repo_name: Option<String>,
    pub publication_state: Option<String>,
    pub published_plan_id: Option<String>,
    pub published_head_revision_id: Option<String>,
    pub head_revision_id: Option<String>,
    pub head_revision: Option<DispatchRevisionInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchTaskInput {
    pub task_id: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub planning_state: Option<String>,
    pub origin_plan_revision_id: Option<String>,
    pub plan_drift_state: Option<String>,
    pub plan_id: Option<String>,
    pub plan_item_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedTaskSummary {
    pub task_id: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub planning_state: Option<String>,
    pub origin_plan_revision_id: Option<String>,
    pub plan_drift_state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPlanPublishShadow {
    pub plan_id: Option<String>,
    pub publication_state: Option<String>,
    pub head_publication_state: Option<String>,
    pub head_revision_id: Option<String>,
    pub head_revision_number: Option<i64>,
    pub published_plan_id: Option<String>,
    pub published_head_revision_id: Option<String>,
    pub unpublished_head: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItemsPayload {
    pub plan_id: Option<String>,
    pub plan_title: Option<String>,
    pub plan_revision_id: Option<String>,
    pub revision_number: Option<i64>,
    pub identity_only: bool,
    pub dispatch_validation_required: bool,
    pub dispatch_validation_hint: String,
    pub item_count: usize,
    pub items: Vec<DispatchPlanItemInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchSummaryItem {
    pub plan_item_ref: Option<String>,
    pub text: String,
    pub checkbox_state: String,
    pub heading_path: Vec<String>,
    pub line_number: i64,
    pub linked_tasks: Vec<LinkedTaskSummary>,
    pub taskable: bool,
    pub taskable_blocker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanDispatchSummary {
    pub plan_id: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub repo_name: Option<String>,
    pub artifact_path: Option<String>,
    pub artifact_selector: Option<String>,
    pub artifact_heading: Option<String>,
    pub plan_revision_id: Option<String>,
    pub revision_number: Option<i64>,
    pub publication_state: Option<String>,
    pub head_publication_state: Option<String>,
    pub published_plan_id: Option<String>,
    pub published_head_revision_id: Option<String>,
    pub local_publication: Option<LocalPlanPublishShadow>,
    pub local_unpublished_head: bool,
    pub item_count: usize,
    pub open_item_count: usize,
    pub done_item_count: usize,
    pub unref_open_item_count: usize,
    pub linked_open_item_count: usize,
    pub taskable_item_count: usize,
    pub linked_task_count: usize,
    pub linked_task_status_counts: BTreeMap<String, usize>,
    pub items: Vec<DispatchSummaryItem>,
    pub open_items: Vec<DispatchSummaryItem>,
    pub taskable_items: Vec<DispatchSummaryItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchLegalityDecision {
    pub plan_item_ref: Option<String>,
    pub taskable: bool,
    pub taskable_blocker: Option<String>,
    pub item: Option<DispatchSummaryItem>,
    pub dispatch_validation_required: bool,
    pub dispatch_validation_hint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanCandidatesAggregateSummary {
    pub scanned_plan_count: usize,
    pub candidate_plan_count: usize,
    pub open_item_count: usize,
    pub taskable_item_count: usize,
    pub linked_task_count: usize,
    pub local_unpublished_head_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanCandidatesPayload {
    pub scope: String,
    pub remote: Option<String>,
    pub repo_name: Option<String>,
    pub summary: PlanCandidatesAggregateSummary,
    pub candidates: Vec<PlanDispatchSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanTaskLinkIndexes {
    pub by_item: BTreeMap<(String, String), Vec<LinkedTaskSummary>>,
    pub by_plan: BTreeMap<String, Vec<LinkedTaskSummary>>,
}

pub fn plan_items_payload(
    plan: &DispatchPlanInput,
    revision: Option<&DispatchRevisionInput>,
) -> PlanItemsPayload {
    let current_revision = revision.or(plan.head_revision.as_ref());
    let items = current_revision
        .map(|value| value.items.clone())
        .unwrap_or_default();
    PlanItemsPayload {
        plan_id: plan.plan_id.clone(),
        plan_title: plan.title.clone(),
        plan_revision_id: current_revision.and_then(|value| value.plan_revision_id.clone()),
        revision_number: current_revision.and_then(|value| value.revision_number),
        identity_only: true,
        dispatch_validation_required: true,
        dispatch_validation_hint: "Use `ait plan inspect <plan-id>` or `ait plan candidates` before `ait task start` to confirm the ref is still taskable.".to_string(),
        item_count: items.len(),
        items,
    }
}

pub fn local_plan_publish_shadow(
    plan: Option<&DispatchPlanInput>,
) -> Option<LocalPlanPublishShadow> {
    let plan = plan?;
    let head_revision = plan.head_revision.as_ref();
    let head_publication_state = head_revision.and_then(|value| value.publication_state.clone());
    Some(LocalPlanPublishShadow {
        plan_id: plan.plan_id.clone(),
        publication_state: plan.publication_state.clone(),
        head_publication_state: head_publication_state.clone(),
        head_revision_id: plan
            .head_revision_id
            .clone()
            .or_else(|| head_revision.and_then(|value| value.plan_revision_id.clone())),
        head_revision_number: head_revision.and_then(|value| value.revision_number),
        published_plan_id: plan.published_plan_id.clone(),
        published_head_revision_id: plan.published_head_revision_id.clone(),
        unpublished_head: !matches!(head_publication_state.as_deref(), None | Some("published")),
    })
}

pub fn plan_task_link_indexes(tasks: &[DispatchTaskInput]) -> PlanTaskLinkIndexes {
    let mut by_item: BTreeMap<(String, String), Vec<LinkedTaskSummary>> = BTreeMap::new();
    let mut by_plan: BTreeMap<String, Vec<LinkedTaskSummary>> = BTreeMap::new();
    for task in tasks {
        let Some(plan_id) = normalize_optional_text(task.plan_id.as_deref()) else {
            continue;
        };
        let plan_item_ref = normalize_optional_text(task.plan_item_ref.as_deref());
        let summarized = summarize_plan_linked_task(task);
        by_plan
            .entry(plan_id.clone())
            .or_default()
            .push(summarized.clone());
        if let Some(plan_item_ref) = plan_item_ref {
            by_item
                .entry((plan_id, plan_item_ref))
                .or_default()
                .push(summarized);
        }
    }
    PlanTaskLinkIndexes { by_item, by_plan }
}

pub fn plan_dispatch_summary(
    plan: &DispatchPlanInput,
    tasks: &[DispatchTaskInput],
    revision: Option<&DispatchRevisionInput>,
    local_shadow_override: Option<&LocalPlanPublishShadow>,
) -> PlanDispatchSummary {
    let plan_id = plan.plan_id.clone().unwrap_or_default();
    let current_revision = revision.or(plan.head_revision.as_ref());
    let raw_items = current_revision
        .map(|value| value.items.clone())
        .unwrap_or_default();
    let indexes = plan_task_link_indexes(tasks);
    let linked_plan_tasks = indexes
        .by_plan
        .get(plan_id.as_str())
        .cloned()
        .unwrap_or_default();

    let mut enriched_items = Vec::new();
    let mut open_items = Vec::new();
    let mut taskable_items = Vec::new();
    let mut unref_open_item_count = 0usize;
    let mut linked_open_item_count = 0usize;

    for item in raw_items {
        let plan_item_ref = normalize_optional_text(item.plan_item_ref.as_deref());
        let linked_tasks: Vec<LinkedTaskSummary> = match plan_item_ref.as_deref() {
            Some(target_ref) => indexes
                .by_item
                .get(&(plan_id.clone(), target_ref.to_string()))
                .cloned()
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let checkbox_state = item.checkbox_state.clone();
        let mut taskable_blocker = None;
        if checkbox_state != "open" {
            taskable_blocker = Some("not_open".to_string());
        } else if plan_item_ref.is_none() {
            taskable_blocker = Some("missing_plan_item_ref".to_string());
            unref_open_item_count += 1;
        } else if !linked_tasks.is_empty() {
            taskable_blocker = Some("linked_task_exists".to_string());
            linked_open_item_count += 1;
        }

        let enriched = DispatchSummaryItem {
            plan_item_ref,
            text: item.text,
            checkbox_state: checkbox_state.clone(),
            heading_path: item.heading_path,
            line_number: item.line_number,
            linked_tasks,
            taskable: taskable_blocker.is_none(),
            taskable_blocker,
        };
        if checkbox_state == "open" {
            open_items.push(enriched.clone());
        }
        if enriched.taskable {
            taskable_items.push(enriched.clone());
        }
        enriched_items.push(enriched);
    }

    let done_item_count = enriched_items
        .iter()
        .filter(|item| item.checkbox_state == "done")
        .count();
    let local_shadow = local_shadow_override
        .cloned()
        .or_else(|| local_plan_publish_shadow(Some(plan)));

    PlanDispatchSummary {
        plan_id: plan.plan_id.clone(),
        title: plan.title.clone(),
        status: plan.status.clone(),
        repo_name: plan.repo_name.clone(),
        artifact_path: current_revision.and_then(|value| value.artifact_path.clone()),
        artifact_selector: current_revision.and_then(|value| value.artifact_selector.clone()),
        artifact_heading: current_revision.and_then(|value| value.artifact_heading.clone()),
        plan_revision_id: current_revision.and_then(|value| value.plan_revision_id.clone()),
        revision_number: current_revision.and_then(|value| value.revision_number),
        publication_state: plan.publication_state.clone(),
        head_publication_state: current_revision.and_then(|value| value.publication_state.clone()),
        published_plan_id: plan.published_plan_id.clone(),
        published_head_revision_id: plan.published_head_revision_id.clone(),
        local_unpublished_head: local_shadow
            .as_ref()
            .map(|value| value.unpublished_head)
            .unwrap_or(false),
        local_publication: local_shadow,
        item_count: enriched_items.len(),
        open_item_count: open_items.len(),
        done_item_count,
        unref_open_item_count,
        linked_open_item_count,
        taskable_item_count: taskable_items.len(),
        linked_task_count: linked_plan_tasks.len(),
        linked_task_status_counts: status_counts(&linked_plan_tasks),
        items: enriched_items,
        open_items,
        taskable_items,
    }
}

pub fn compute_taskable_items(
    plan: &DispatchPlanInput,
    tasks: &[DispatchTaskInput],
    revision: Option<&DispatchRevisionInput>,
    local_shadow_override: Option<&LocalPlanPublishShadow>,
) -> Vec<DispatchSummaryItem> {
    plan_dispatch_summary(plan, tasks, revision, local_shadow_override).taskable_items
}

pub fn validate_dispatch_legality(
    plan: &DispatchPlanInput,
    tasks: &[DispatchTaskInput],
    plan_item_ref: Option<&str>,
    revision: Option<&DispatchRevisionInput>,
    local_shadow_override: Option<&LocalPlanPublishShadow>,
) -> DispatchLegalityDecision {
    let summary = plan_dispatch_summary(plan, tasks, revision, local_shadow_override);
    let normalized_ref = normalize_optional_text(plan_item_ref);
    let item = normalized_ref.as_ref().and_then(|target_ref| {
        summary
            .items
            .iter()
            .find(|item| item.plan_item_ref.as_deref() == Some(target_ref.as_str()))
            .cloned()
    });
    let (taskable, taskable_blocker) = match (normalized_ref.as_ref(), item.as_ref()) {
        (None, _) => (false, Some("missing_requested_plan_item_ref".to_string())),
        (Some(_), None) => (false, Some("plan_item_not_found".to_string())),
        (Some(_), Some(item)) => (item.taskable, item.taskable_blocker.clone()),
    };
    DispatchLegalityDecision {
        plan_item_ref: normalized_ref,
        taskable,
        taskable_blocker,
        item,
        dispatch_validation_required: true,
        dispatch_validation_hint:
            "Use `ait plan inspect <plan-id>` or `ait plan candidates` before `ait task start` to confirm the ref is still taskable.".to_string(),
    }
}

pub fn plan_candidates_payload(
    summaries: &[PlanDispatchSummary],
    scope: Option<&str>,
    repo_name: Option<&str>,
    remote: Option<&str>,
    include_all: bool,
) -> PlanCandidatesPayload {
    let mut candidates: Vec<PlanDispatchSummary> = summaries
        .iter()
        .filter(|summary| include_all || summary.taskable_item_count > 0)
        .cloned()
        .collect();
    candidates.sort_by(|left, right| {
        right
            .taskable_item_count
            .cmp(&left.taskable_item_count)
            .then_with(|| right.open_item_count.cmp(&left.open_item_count))
            .then_with(|| {
                left.artifact_path
                    .as_deref()
                    .unwrap_or("")
                    .cmp(right.artifact_path.as_deref().unwrap_or(""))
            })
            .then_with(|| {
                left.title
                    .as_deref()
                    .unwrap_or("")
                    .cmp(right.title.as_deref().unwrap_or(""))
            })
    });

    PlanCandidatesPayload {
        scope: scope.unwrap_or("local").trim().to_string(),
        remote: normalize_optional_text(remote),
        repo_name: normalize_optional_text(repo_name),
        summary: PlanCandidatesAggregateSummary {
            scanned_plan_count: summaries.len(),
            candidate_plan_count: candidates.len(),
            open_item_count: summaries.iter().map(|row| row.open_item_count).sum(),
            taskable_item_count: summaries.iter().map(|row| row.taskable_item_count).sum(),
            linked_task_count: summaries.iter().map(|row| row.linked_task_count).sum(),
            local_unpublished_head_count: summaries
                .iter()
                .filter(|row| row.local_unpublished_head)
                .count(),
        },
        candidates,
    }
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim().to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn summarize_plan_linked_task(task: &DispatchTaskInput) -> LinkedTaskSummary {
    LinkedTaskSummary {
        task_id: task.task_id.clone(),
        title: task.title.clone(),
        status: task.status.clone(),
        planning_state: task.planning_state.clone(),
        origin_plan_revision_id: task.origin_plan_revision_id.clone(),
        plan_drift_state: task.plan_drift_state.clone(),
    }
}

fn status_counts(rows: &[LinkedTaskSummary]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        let status = row.status.clone().unwrap_or_else(|| "unknown".to_string());
        *counts.entry(status).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests;
