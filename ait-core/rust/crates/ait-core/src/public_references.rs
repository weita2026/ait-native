fn is_numbered_component(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|ordinal| {
        !ordinal.is_empty() && ordinal.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn is_task_id(value: &str) -> bool {
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

/// Split the public Patchset reference used by CLI callers.
///
/// Durable and protocol identities may contain an owning Change component,
/// while public commands address the same Patchset as `TASK_ID/P-##`.
pub fn split_public_patchset_reference(value: &str) -> Option<(&str, &str)> {
    let mut components = value.split('/');
    let task_id = components.next()?;
    let patchset_ordinal = components.next()?;
    if components.next().is_some()
        || !is_task_id(task_id)
        || !is_numbered_component(patchset_ordinal, "P-")
    {
        return None;
    }
    Some((task_id, patchset_ordinal))
}

/// Collapse an internal `TASK_ID/C-##/P-##` identity for public display.
pub fn public_patchset_reference(value: &str) -> String {
    let mut components = value.split('/');
    let Some(task_id) = components.next() else {
        return value.to_string();
    };
    let Some(change_ordinal) = components.next() else {
        return value.to_string();
    };
    let Some(patchset_ordinal) = components.next() else {
        return value.to_string();
    };
    if components.next().is_none()
        && is_task_id(task_id)
        && is_numbered_component(change_ordinal, "C-")
        && is_numbered_component(patchset_ordinal, "P-")
    {
        format!("{task_id}/{patchset_ordinal}")
    } else {
        value.to_string()
    }
}

/// Hide internal Change components from user-facing diagnostics.
pub fn public_work_reference_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(relative_change_start) = value[cursor..].find("/C-") {
        let change_start = cursor + relative_change_start;
        let task_start = value[..change_start]
            .char_indices()
            .rev()
            .find_map(|(index, character)| {
                (!character.is_ascii_uppercase() && !character.is_ascii_digit() && character != '-')
                    .then_some(index + character.len_utf8())
            })
            .unwrap_or_default();
        let task_id = &value[task_start..change_start];
        let change_digits_start = change_start + 3;
        let change_digits_end = value[change_digits_start..]
            .find(|character: char| !character.is_ascii_digit())
            .map(|offset| change_digits_start + offset)
            .unwrap_or(value.len());
        if !is_task_id(task_id) || change_digits_end == change_digits_start {
            output.push_str(&value[cursor..change_start + 1]);
            cursor = change_start + 1;
            continue;
        }
        output.push_str(&value[cursor..change_start]);
        cursor = change_digits_end;
    }
    output.push_str(&value[cursor..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_patchset_reference_hides_only_the_internal_change_component() {
        assert_eq!(
            public_patchset_reference("RCT-1751/C-01/P-02"),
            "RCT-1751/P-02"
        );
        assert_eq!(public_patchset_reference("RCT-1751/P-02"), "RCT-1751/P-02");
        assert_eq!(public_patchset_reference("RCP-2"), "RCP-2");
    }

    #[test]
    fn public_parser_accepts_task_scoped_patchsets_only() {
        assert_eq!(
            split_public_patchset_reference("LCT-97/P-03"),
            Some(("LCT-97", "P-03"))
        );
        assert_eq!(split_public_patchset_reference("RCT-97/C-01/P-03"), None);
        assert_eq!(split_public_patchset_reference("C-01/P-03"), None);
    }

    #[test]
    fn diagnostic_text_hides_patchset_and_standalone_change_components() {
        assert_eq!(
            public_work_reference_text("Patchset `RCT-1751/C-01/P-02` belongs to `RCT-1751/C-01`."),
            "Patchset `RCT-1751/P-02` belongs to `RCT-1751`."
        );
        assert_eq!(
            public_work_reference_text("path/C-01/P-02 remains literal"),
            "path/C-01/P-02 remains literal"
        );
    }
}
