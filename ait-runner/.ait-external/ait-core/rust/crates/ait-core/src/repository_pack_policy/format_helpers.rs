pub(super) fn normalize_optional_text(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub(super) fn require_non_empty<'a>(value: &'a str, label: &str) -> Result<&'a str, String> {
    normalize_optional_text(value).ok_or_else(|| format!("Missing {label}."))
}

pub(super) fn validate_optional_owner(
    inventory_repo_name: &str,
    row_kind: &str,
    row_id: &str,
    repo_name: &Option<String>,
    repo_id: &Option<String>,
) -> Result<(), String> {
    if let Some(repo_name) = repo_name.as_deref().and_then(normalize_optional_text) {
        if repo_name != inventory_repo_name {
            return Err(format!(
                "{row_kind} {row_id} belongs to repository {repo_name}, not {inventory_repo_name}."
            ));
        }
    }
    if let Some(repo_id) = repo_id.as_deref() {
        require_non_empty(repo_id, "repository owner id")?;
    }
    Ok(())
}
