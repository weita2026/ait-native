use super::helpers::*;
use super::*;

pub(super) fn normalize_task_filter(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    let normalized = if normalized.is_empty() {
        TASK_STATUS_ACTIVE
    } else {
        normalized.as_str()
    };
    match normalized {
        TASK_STATUS_ACTIVE
        | TASK_STATUS_COMPLETED
        | TASK_STATUS_ABANDONED
        | TASK_STATUS_LATER_PROMOTION_EXCLUDED
        | TASK_STATUS_LEGACY_CANCELED
        | "all" => Ok(normalized.to_string()),
        _ => Err(format!(
            "Unsupported task queue status filter: {normalized}"
        )),
    }
}

pub(super) fn task_statuses_for_filter(status: &str) -> Vec<&'static str> {
    match status {
        "all" => Vec::new(),
        TASK_STATUS_LEGACY_CANCELED => vec![TASK_STATUS_ABANDONED, TASK_STATUS_LEGACY_CANCELED],
        TASK_STATUS_ACTIVE => vec![TASK_STATUS_ACTIVE],
        TASK_STATUS_COMPLETED => vec![TASK_STATUS_COMPLETED],
        TASK_STATUS_ABANDONED => vec![TASK_STATUS_ABANDONED],
        TASK_STATUS_LATER_PROMOTION_EXCLUDED => vec![TASK_STATUS_LATER_PROMOTION_EXCLUDED],
        _ => Vec::new(),
    }
}

fn task_matches_filter(task: &JsonMap<String, JsonValue>, status: &str) -> bool {
    if status == "all" {
        return true;
    }
    let task_status = object_text(task, "status")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if status == TASK_STATUS_LEGACY_CANCELED {
        return task_status == TASK_STATUS_ABANDONED || task_status == TASK_STATUS_LEGACY_CANCELED;
    }
    task_status == status
}

pub(super) fn repo_matches(repo_name: Option<&str>, row: &JsonMap<String, JsonValue>) -> bool {
    let Some(repo_name) = repo_name else {
        return true;
    };
    object_text(row, "repo_name").as_deref() == Some(repo_name)
}

pub(super) fn selected_tasks<'a>(
    input: &'a QueueReadModelInput,
    normalized_status: &str,
) -> Vec<&'a JsonMap<String, JsonValue>> {
    let mut tasks = input
        .tasks
        .iter()
        .filter(|task| repo_matches(input.repo_name.as_deref(), task))
        .filter(|task| task_matches_filter(task, normalized_status))
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        object_text(right, "created_at").cmp(&object_text(left, "created_at"))
    });
    tasks
}
