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
