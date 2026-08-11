pub fn workflow_ready_apply_command(change_id: Option<&str>) -> String {
    match change_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(change_id) => format!("ait workflow ready {change_id} --apply"),
        None => "ait workflow ready --apply".to_string(),
    }
}
