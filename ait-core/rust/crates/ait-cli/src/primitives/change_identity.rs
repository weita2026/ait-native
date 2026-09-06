use super::*;

pub(super) fn canonical_change_id(value: &str) -> Result<String, String> {
    ChangeJson::stateless().canonical_change_id(value)
}

pub(super) fn is_short_change_id(value: &str) -> bool {
    let Some(ordinal) = value.strip_prefix("C-") else {
        return false;
    };
    !ordinal.is_empty() && ordinal.bytes().all(|byte| byte.is_ascii_digit())
}

pub(super) fn is_task_id(value: &str) -> bool {
    let Some((prefix, ordinal)) = value.rsplit_once('-') else {
        return false;
    };
    prefix.ends_with('T')
        && prefix
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && !ordinal.is_empty()
        && ordinal.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Clone, Copy)]
pub(super) enum TaskChangeSelection {
    Author,
    Finish,
}

/// Select from the complete authority inventory, never from a bounded UI list
/// or the worktree's initial Change hint. Explicit Change inputs bypass this.
pub(super) fn select_task_change_reference(
    rows: &[JsonValue],
    task_id: &str,
    selection: TaskChangeSelection,
) -> Result<Option<String>, String> {
    let mut candidates = Vec::new();
    for row in rows {
        if change_task_id_from_payload(row).as_deref() != Some(task_id) {
            continue;
        }
        let status = required_string_field(row, "status")?;
        let accepted = match status.as_str() {
            "archived" | "superseded" | "canceled" | "abandoned" => continue,
            "draft" | "active" | "review" => false,
            "ready" | "review_pending" | "blocked" | "gated" | "approved" | "landable" => {
                if matches!(selection, TaskChangeSelection::Author) {
                    continue;
                }
                false
            }
            "landed" => {
                if matches!(selection, TaskChangeSelection::Author) {
                    continue;
                }
                true
            }
            _ => return Err(format!("Task {task_id} has an unsupported work status: {status}. Inspect `ait task audit {task_id}` before continuing.")),
        };
        let reference = change_reference_from_payload(row, None)?;
        // Validate a supplied compound reference against its owning Task too.
        change_reference_for_context(Some(task_id), &reference)?;
        candidates.push((reference, accepted));
    }
    if candidates.iter().any(|(_, accepted)| !accepted) {
        candidates.retain(|(_, accepted)| !accepted);
    }
    if candidates.len() > 1 {
        let ids = candidates
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let kind = match selection {
            TaskChangeSelection::Author => "writable",
            TaskChangeSelection::Finish => "finishable",
        };
        return Err(format!("Task {task_id} has multiple {kind} changes ({ids}); repeat the same command with the intended exact TASK_ID/C-## reference. Advanced selection: `ait change --help`."));
    }
    Ok(candidates.pop().map(|(reference, _)| reference))
}

pub(super) fn change_reference_for_context(
    task_id: Option<&str>,
    change_id: &str,
) -> Result<String, String> {
    let requested = normalized_text(Some(change_id))
        .ok_or_else(|| "change_id must not be empty.".to_string())?;
    let canonical = canonical_change_id(&requested)?;
    if requested != canonical {
        if let (Some(expected_task_id), Some((actual_task_id, child))) =
            (normalized_text(task_id), requested.rsplit_once('/'))
        {
            if child == canonical && actual_task_id != expected_task_id {
                return Err(format!(
                    "Change reference `{requested}` belongs to task `{actual_task_id}`, not `{expected_task_id}`."
                ));
            }
        }
        return Ok(requested);
    }
    ChangeJson::stateless().rolling_server_change_id(task_id, &canonical)
}

pub(super) fn change_reference_from_payload(
    payload: &JsonValue,
    fallback: Option<&str>,
) -> Result<String, String> {
    if let Some(change_ref) = string_field(payload, "change_ref") {
        return Ok(change_ref);
    }
    let change_id = required_string_field(payload, "change_id")?;
    let canonical = canonical_change_id(&change_id)?;
    if change_id != canonical {
        return Ok(change_id);
    }
    if let Some(task_id) = string_field(payload, "task_id") {
        return change_reference_for_context(Some(&task_id), &canonical);
    }
    if let Some(fallback) = normalized_text(fallback) {
        if canonical_change_id(&fallback)? == canonical && fallback != canonical {
            return Ok(fallback);
        }
    }
    ChangeJson::stateless().rolling_server_change_id(None, &canonical)
}

pub(super) fn payload_belongs_to_change(
    payload: &JsonValue,
    expected_change_id: &str,
    expected_change_ref: &str,
) -> bool {
    if let Some(change_ref) = string_field(payload, "change_ref") {
        return change_ref == expected_change_ref;
    }
    let Some(raw_change_id) = string_field(payload, "change_id") else {
        return false;
    };
    if raw_change_id == expected_change_ref {
        return true;
    }
    if expected_change_id == expected_change_ref && raw_change_id == expected_change_id {
        return true;
    }
    let Some(task_id) = string_field(payload, "task_id") else {
        return false;
    };
    change_reference_for_context(Some(&task_id), &raw_change_id)
        .is_ok_and(|actual| actual == expected_change_ref)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_selection_uses_complete_inventory_and_ignores_terminal_siblings() {
        let mut rows = (1..=30)
            .map(|i| {
                json!({
                    "task_id": format!("RT-{i}"), "change_id": "C-01", "status": "active",
                })
            })
            .collect::<Vec<_>>();
        rows.extend([
            json!({"task_id":"RT-31", "change_id":"C-01", "status":"superseded"}),
            json!({"task_id":"RT-31", "change_id":"C-02", "status":"landed"}),
            json!({"task_id":"RT-31", "change_id":"C-03", "status":"active"}),
        ]);
        for selection in [TaskChangeSelection::Author, TaskChangeSelection::Finish] {
            assert_eq!(
                select_task_change_reference(&rows, "RT-31", selection)
                    .unwrap()
                    .as_deref(),
                Some("RT-31/C-03")
            );
        }
        rows.push(json!({"task_id":"RT-31", "change_id":"C-04", "status":"draft"}));
        assert!(
            select_task_change_reference(&rows, "RT-31", TaskChangeSelection::Finish)
                .unwrap_err()
                .contains("multiple finishable")
        );
        rows.pop();
        rows.last_mut().unwrap()["change_ref"] = json!("RT-32/C-03");
        assert!(select_task_change_reference(&rows, "RT-31", TaskChangeSelection::Author).is_err());
    }

    #[test]
    fn completed_task_selection_does_not_guess_among_accepted_changes() {
        let rows = vec![
            json!({"task_id":"LT-1", "change_id":"C-01", "status":"landed"}),
            json!({"task_id":"LT-1", "change_id":"C-02", "status":"landed"}),
        ];
        assert!(select_task_change_reference(&rows, "LT-1", TaskChangeSelection::Finish).is_err());
        assert_eq!(
            select_task_change_reference(&rows, "LT-1", TaskChangeSelection::Author).unwrap(),
            None
        );
        let unknown = vec![json!({"task_id":"LT-1", "change_id":"C-01", "status":"unknown"})];
        assert!(
            select_task_change_reference(&unknown, "LT-1", TaskChangeSelection::Finish).is_err()
        );
    }

    #[test]
    fn same_short_change_id_isolated_by_derived_reference() {
        let first = json!({
            "change_id": "C-01",
            "change_ref": "RT-1/C-01",
        });
        assert!(payload_belongs_to_change(&first, "C-01", "RT-1/C-01"));
        assert!(!payload_belongs_to_change(&first, "C-01", "RT-2/C-01"));
    }

    #[test]
    fn raw_composite_owner_is_accepted_but_unscoped_short_owner_fails_closed() {
        assert!(payload_belongs_to_change(
            &json!({"change_id": "RT-1/C-01"}),
            "C-01",
            "RT-1/C-01"
        ));
        assert!(!payload_belongs_to_change(
            &json!({"change_id": "C-01"}),
            "C-01",
            "RT-1/C-01"
        ));
        assert!(payload_belongs_to_change(
            &json!({"change_id": "C-01", "task_id": "RT-1"}),
            "C-01",
            "RT-1/C-01"
        ));
    }

    #[test]
    fn explicit_reference_rejects_conflicting_task_context() {
        let err = change_reference_for_context(Some("RT-1"), "RT-2/C-01")
            .expect_err("task mismatch must fail closed");
        assert!(err.contains("belongs to task `RT-2`, not `RT-1`"));
    }
}
