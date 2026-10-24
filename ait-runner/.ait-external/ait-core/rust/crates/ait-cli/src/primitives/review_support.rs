use super::*;

pub(super) fn missing_code_review_summary_sections(message: &str) -> Vec<String> {
    let normalized = normalized_text(Some(message)).unwrap_or_default();
    if normalized.is_empty() {
        return REVIEW_SECTION_LABELS
            .iter()
            .map(|(section, _)| (*section).to_string())
            .collect();
    }
    let lowered = normalized.to_lowercase();
    let mut missing = Vec::new();
    for (section, aliases) in REVIEW_SECTION_LABELS {
        let mut present = false;
        for alias in *aliases {
            if let Some(body) = section_body_for_alias(&lowered, alias) {
                if section_has_content(body) {
                    present = true;
                    break;
                }
            }
        }
        if !present {
            missing.push((*section).to_string());
        }
    }
    missing
}

pub(super) fn section_body_for_alias<'a>(text: &'a str, alias: &str) -> Option<&'a str> {
    let candidates = [
        format!("{alias}:"),
        format!("{alias}\n"),
        format!("{alias} -"),
        format!("{alias}-"),
        format!("{alias};"),
    ];
    let mut match_index = None;
    let mut match_len = 0usize;
    for candidate in &candidates {
        if let Some(index) = text.find(candidate) {
            match_index = Some(index);
            match_len = candidate.len();
            break;
        }
    }
    let start = match_index?;
    let body_start = start + match_len;
    let remainder = &text[body_start..];
    let mut next_label_index = remainder.len();
    for (_section, aliases) in REVIEW_SECTION_LABELS {
        for next_alias in *aliases {
            let candidate = format!("\n{next_alias}");
            if let Some(index) = remainder.find(&candidate) {
                next_label_index = next_label_index.min(index);
            }
            let candidate = format!("; {next_alias}");
            if let Some(index) = remainder.find(&candidate) {
                next_label_index = next_label_index.min(index);
            }
        }
    }
    Some(remainder[..next_label_index].trim())
}

pub(super) fn section_has_content(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    let placeholder = trimmed.trim_matches('.').trim().to_lowercase();
    if matches!(
        placeholder.as_str(),
        "" | "todo" | "tbd" | "replace me" | "replace_me"
    ) {
        return false;
    }
    !(trimmed.starts_with('<') && trimmed.ends_with('>'))
}

pub(super) fn required_string_field(value: &JsonValue, key: &str) -> Result<String, String> {
    string_field(value, key).ok_or_else(|| format!("Missing required field `{key}`."))
}

pub(super) fn string_field(value: &JsonValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .and_then(|text| normalized_text(Some(text)))
}

pub(super) fn normalized_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}
