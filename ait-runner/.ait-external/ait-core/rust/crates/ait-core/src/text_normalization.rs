pub fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let text = raw.trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    })
}
