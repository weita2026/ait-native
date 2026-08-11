use super::helpers::{bool_value, int_value, object_text, string_list, update_latest_activity};
use super::*;

pub fn repository_index_read_model(input: &RepositoryIndexInput) -> Result<JsonValue, String> {
    let mut repositories = input.repositories.clone();
    repositories.sort_by(|left, right| {
        object_text(left, "repo_name").cmp(&object_text(right, "repo_name"))
    });

    let mut line_counts: BTreeMap<String, usize> = BTreeMap::new();
    for line in &input.lines {
        if let Some(repo_name) = object_text(line, "repo_name") {
            *line_counts.entry(repo_name).or_default() += 1;
        }
    }

    let mut entries = Vec::new();
    let mut entries_by_name: HashMap<String, JsonValue> = HashMap::new();
    let mut latest_activity: Option<JsonValue> = None;
    let mut total_lines = 0_usize;
    for row in repositories {
        let repo_name = object_text(&row, "repo_name").unwrap_or_default();
        let line_count = line_counts.get(&repo_name).copied().unwrap_or(0);
        total_lines += line_count;
        let entry = json!({
            "repo_name": repo_name,
            "repo_id": row.get("repo_id").cloned().unwrap_or(JsonValue::Null),
            "default_line": row.get("default_line").cloned().unwrap_or(JsonValue::Null),
            "created_at": row.get("created_at").cloned().unwrap_or(JsonValue::Null),
            "updated_at": row.get("updated_at").cloned().unwrap_or(JsonValue::Null),
            "line_count": line_count,
        });
        update_latest_activity(&mut latest_activity, &entry, "repository", "repo_name");
        entries_by_name.insert(repo_name, entry.clone());
        entries.push(entry);
    }

    let groups = input
        .groups
        .iter()
        .map(|group| {
            let repo_entries = string_list(group.get("repo_names"))
                .into_iter()
                .filter_map(|repo_name| entries_by_name.get(&repo_name).cloned())
                .collect::<Vec<_>>();
            json!({
                "group_id": object_text(group, "group_id").unwrap_or_default(),
                "title": object_text(group, "title").unwrap_or_default(),
                "sort_index": group.get("sort_index").and_then(int_value).unwrap_or(0),
                "system_slug": object_text(group, "system_slug").unwrap_or_default(),
                "is_main": group.get("is_main").and_then(bool_value).unwrap_or(false),
                "repo_count": repo_entries.len(),
                "repositories": repo_entries,
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "count": entries.len(),
        "total_lines": total_lines,
        "repositories": entries,
        "groups": groups,
        "group_count": groups.len(),
        "latest_activity": latest_activity,
    }))
}
