pub fn workflow_ready_apply_command(task_id: Option<&str>) -> String {
    match task_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(task_id) => format!("ait workflow ready {task_id} --apply"),
        None => "ait workflow ready --apply".to_string(),
    }
}

/// Project a stored generated command onto public Task and Patchset inputs.
pub fn workflow_task_owned_command(
    command: &str,
    task_id: Option<&str>,
    change_id: Option<&str>,
) -> String {
    let Some(task_id) = task_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return command.to_string();
    };
    let change_id = change_id.map(str::trim).filter(|value| !value.is_empty());
    let mut projected = command.to_string();
    for prefix in [
        "ait snapshot create ",
        "ait commit ",
        "ait task finish ",
        "ait workflow ready ",
        "ait workflow finish ",
        "ait change show ",
        "ait change revert ",
        "ait change replay ",
        "ait change close ",
        "ait change publish ",
        "ait patchset publish ",
        "ait patchset list ",
        "ait review show ",
        "ait review team request ",
        "ait review team approve ",
        "ait review team request-changes ",
        "ait review team comment ",
        "ait review team defer ",
        "ait review task approve ",
        "ait review task request-changes ",
        "ait review task comment ",
        "ait review task defer ",
        "ait review code submit ",
        "ait worktree recover-task ",
    ] {
        let Some(argument_tail) = projected.strip_prefix(prefix) else {
            continue;
        };
        let argument_end = argument_tail
            .find(char::is_whitespace)
            .unwrap_or(argument_tail.len());
        let argument = &argument_tail[..argument_end];
        let composite_owner = argument
            .split_once("/C-")
            .filter(|(_, ordinal)| {
                !ordinal.is_empty() && ordinal.bytes().all(|byte| byte.is_ascii_digit())
            })
            .map(|(owner, _)| owner);
        if change_id == Some(argument) || composite_owner == Some(task_id) {
            projected = format!("{prefix}{task_id}{}", &argument_tail[argument_end..]);
        }
        break;
    }

    while let Some(selector_start) = projected.find(" --change ") {
        let value_start = selector_start + " --change ".len();
        let value_end = projected[value_start..]
            .find(char::is_whitespace)
            .map(|offset| value_start + offset)
            .unwrap_or(projected.len());
        projected.replace_range(selector_start..value_end, "");
    }
    crate::public_references::public_work_reference_text(&projected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_task_commands_hide_change_input_and_patchset_change_segments() {
        assert_eq!(
            workflow_task_owned_command(
                "ait workflow ready RCT-1/C-02 --apply --remote origin",
                Some("RCT-1"),
                Some("RCT-1/C-02"),
            ),
            "ait workflow ready RCT-1 --apply --remote origin"
        );
        assert_eq!(
            workflow_task_owned_command(
                "ait workflow ready RCT-1 --apply --change RCT-1/C-02 --remote origin",
                Some("RCT-1"),
                Some("RCT-1/C-02"),
            ),
            "ait workflow ready RCT-1 --apply --remote origin"
        );
        assert_eq!(
            workflow_task_owned_command(
                "ait patchset rerun-ci RCT-1/C-02/P-03",
                Some("RCT-1"),
                Some("RCT-1/C-02"),
            ),
            "ait patchset rerun-ci RCT-1/P-03"
        );
    }
}
