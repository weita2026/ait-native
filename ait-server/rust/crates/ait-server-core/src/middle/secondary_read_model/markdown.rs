use super::*;

pub(super) fn markdown_metadata(markdown: &str) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        for key in ["Authority", "Status", "Scope"] {
            let prefix = format!("{key}:");
            if let Some(value) = trimmed.strip_prefix(&prefix) {
                metadata.insert(key.to_ascii_lowercase(), value.trim().to_string());
            }
        }
    }
    metadata
}

pub(super) fn markdown_title(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find_map(|line| {
            line.strip_prefix("# ")
                .map(str::trim)
                .map(ToOwned::to_owned)
        })
        .filter(|title| !title.is_empty())
}

pub(super) fn body_markdown(markdown: &str) -> String {
    let mut lines = markdown.lines();
    if markdown
        .lines()
        .next()
        .map(|line| line.starts_with("# "))
        .unwrap_or(false)
    {
        lines.next();
    }
    lines.collect::<Vec<_>>().join("\n").trim().to_string()
}

pub(super) fn markdown_link_targets(source_path: &str, markdown: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = markdown;
    while let Some(start) = rest.find("](") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else {
            break;
        };
        let target = &rest[..end];
        if let Some(normalized) = normalize_markdown_target(source_path, target) {
            if !targets.contains(&normalized) {
                targets.push(normalized);
            }
        }
        rest = &rest[end + 1..];
    }
    targets
}

pub(super) fn normalize_markdown_target(source_path: &str, target: &str) -> Option<String> {
    let mut cleaned = target.trim();
    if cleaned.is_empty()
        || cleaned.starts_with("http://")
        || cleaned.starts_with("https://")
        || cleaned.starts_with("mailto:")
        || cleaned.starts_with('#')
    {
        return None;
    }
    cleaned = cleaned.split('#').next()?.split('?').next()?.trim();
    if cleaned.is_empty() || !cleaned.to_ascii_lowercase().ends_with(".md") {
        return None;
    }
    let raw = if cleaned.starts_with('/') {
        cleaned.trim_start_matches('/').to_string()
    } else {
        let dir = source_path
            .rsplit_once('/')
            .map(|(dir, _)| dir)
            .unwrap_or("");
        if dir.is_empty() {
            cleaned.to_string()
        } else {
            format!("{dir}/{cleaned}")
        }
    };
    normalize_path(&raw)
}

pub(super) fn normalize_path(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}
