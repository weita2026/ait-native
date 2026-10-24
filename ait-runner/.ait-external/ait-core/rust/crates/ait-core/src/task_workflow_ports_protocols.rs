use crate::json_support::JsonValue;

use crate::change_json::ChangeJson;
use crate::task_json::TaskJson;

pub fn build_linked_task_lookup_payload(
    task_links_by_item_rows: Option<&JsonValue>,
    tasks_by_plan_rows: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    TaskJson::stateless()
        .build_linked_task_lookup_payload(task_links_by_item_rows, tasks_by_plan_rows)
}

pub fn normalize_linked_change_lookup_payload(payload: &JsonValue) -> Result<JsonValue, String> {
    ChangeJson::stateless().normalize_linked_change_lookup_payload(payload)
}

pub fn build_linked_change_lookup_payload(
    change_links_by_task_rows: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    ChangeJson::stateless().build_linked_change_lookup_payload(change_links_by_task_rows)
}

pub fn build_task_tracking_title_payload(task: &JsonValue) -> Result<JsonValue, String> {
    TaskJson::stateless().build_task_tracking_title_payload(task)
}

pub fn build_task_tracking_metadata_payload(
    task: &JsonValue,
    author_mode: &str,
    tracking_policy: &str,
) -> Result<JsonValue, String> {
    TaskJson::stateless().build_task_tracking_metadata_payload(task, author_mode, tracking_policy)
}

#[cfg(test)]
mod tests;
