use crate::json_support::JsonValue;
use crate::task_json::TaskJson;

pub fn build_task_audit_verdict_payload(
    task: &JsonValue,
    change_rows: &JsonValue,
    target_line: &str,
) -> Result<JsonValue, String> {
    TaskJson::stateless().build_task_audit_verdict_payload(task, change_rows, target_line)
}
