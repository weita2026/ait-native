use super::*;

pub(super) fn load_workspace_ignore_rules(
    root: &Path,
    ignore_rules_text: Option<&str>,
) -> Result<Vec<WorkspaceIgnoreRule>, PlanFilesystemError> {
    if let Some(text) = ignore_rules_text {
        return Ok(parse_workspace_ignore_rules(text));
    }
    let path = root.join(WORKSPACE_IGNORE_FILE);
    let store = FilesystemFileIoStore;
    if !store.path_exists(&path) {
        return Ok(Vec::new());
    }
    let text = store
        .read_to_string(&path)
        .map_err(|err| file_io_error_for_path("read workspace ignore file", &path, err))?;
    Ok(parse_workspace_ignore_rules(&text))
}

pub(super) fn load_workspace_ignore_matcher(
    root: &Path,
    ignore_rules_text: Option<&str>,
) -> Result<WorkspaceIgnoreMatcher, PlanFilesystemError> {
    Ok(WorkspaceIgnoreMatcher {
        rules: load_workspace_ignore_rules(root, ignore_rules_text)?,
    })
}

pub(super) fn parse_workspace_ignore_rules(text: &str) -> Vec<WorkspaceIgnoreRule> {
    text.lines()
        .filter_map(parse_workspace_ignore_rule)
        .collect()
}

pub(super) fn parse_workspace_ignore_rule(line: &str) -> Option<WorkspaceIgnoreRule> {
    let mut text = line.trim().to_string();
    if text.is_empty() || text.starts_with('#') {
        return None;
    }
    let escaped = text.starts_with("\\#") || text.starts_with("\\!");
    if escaped {
        text = text[1..].to_string();
    }
    let negated = text.starts_with('!') && !escaped;
    if negated {
        text = text[1..].to_string();
    }
    while let Some(rest) = text.strip_prefix("./") {
        text = rest.to_string();
    }
    let anchored = text.starts_with('/');
    if anchored {
        text = text[1..].to_string();
    }
    let directory_only = text.ends_with('/');
    text = text.trim_end_matches('/').to_string();
    if text.is_empty() {
        return None;
    }
    Some(WorkspaceIgnoreRule {
        basename_only: !text.contains('/'),
        pattern: text.clone(),
        regex: compile_glob_regex(&text),
        negated,
        directory_only,
        anchored,
    })
}

pub(super) fn workspace_path_is_ignored_for_rules(
    rel_path: &Path,
    rules: &[WorkspaceIgnoreRule],
) -> bool {
    workspace_path_is_ignored_for_rules_with_kind(rel_path, rules, false)
}

pub(super) fn workspace_path_is_ignored_for_rules_with_kind(
    rel_path: &Path,
    rules: &[WorkspaceIgnoreRule],
    is_dir: bool,
) -> bool {
    let mut ignored = false;
    for rule in rules {
        if workspace_ignore_rule_matches(rel_path, rule, is_dir) {
            ignored = !rule.negated;
        }
    }
    ignored
}

pub(super) fn workspace_ignore_rule_matches(
    rel_path: &Path,
    rule: &WorkspaceIgnoreRule,
    is_dir: bool,
) -> bool {
    let parts: Vec<String> = rel_path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return false;
    }
    let max_parts = if rule.directory_only {
        if is_dir {
            parts.len()
        } else {
            parts.len().saturating_sub(1)
        }
    } else {
        parts.len()
    };
    if max_parts == 0 {
        return false;
    }
    if rule.basename_only {
        return parts[..max_parts].iter().any(|part| glob_match(part, rule));
    }
    let starts: Vec<usize> = if rule.anchored {
        vec![0]
    } else {
        (0..max_parts).collect()
    };
    for start in starts {
        for end in (start + 1)..=max_parts {
            let candidate = parts[start..end].join("/");
            if glob_match(&candidate, rule) {
                return true;
            }
        }
    }
    false
}

pub(super) fn ignored_directory_may_contain_negated_match(
    rel_path: &Path,
    rules: &[WorkspaceIgnoreRule],
) -> bool {
    let rel_text = rel_path.to_string_lossy().replace('\\', "/");
    let rel_text = rel_text.trim_end_matches('/');
    if rel_text.is_empty() {
        return true;
    }
    let rel_prefix = format!("{rel_text}/");
    for rule in rules {
        if !rule.negated {
            continue;
        }
        if rule.basename_only || !rule.anchored {
            return true;
        }
        let pattern = rule.pattern.trim_end_matches('/');
        if pattern == rel_text || pattern.starts_with(&rel_prefix) {
            return true;
        }
    }
    false
}

pub(super) fn compile_glob_regex(pattern: &str) -> Option<Regex> {
    let mut regex_text = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex_text.push_str(".*"),
            '?' => regex_text.push('.'),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                regex_text.push('\\');
                regex_text.push(ch);
            }
            _ => regex_text.push(ch),
        }
    }
    regex_text.push('$');
    Regex::new(&regex_text).ok()
}

pub(super) fn glob_match(candidate: &str, rule: &WorkspaceIgnoreRule) -> bool {
    rule.regex
        .as_ref()
        .map(|regex| regex.is_match(candidate))
        .unwrap_or(false)
}
