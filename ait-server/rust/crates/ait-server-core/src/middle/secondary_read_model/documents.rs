use super::*;

pub(super) fn authority_doc(row: &JsonMap<String, JsonValue>, fallback_layer: i64) -> JsonValue {
    let path = object_text(row, "path")
        .or_else(|| object_text(row, "document_path"))
        .unwrap_or_else(|| "unknown".to_string());
    let markdown = object_text(row, "markdown").unwrap_or_default();
    let metadata = markdown_metadata(&markdown);
    let title = object_text(row, "title")
        .or_else(|| markdown_title(&markdown))
        .unwrap_or_else(|| filename_stem(&path));
    let layer = row
        .get("layer")
        .and_then(int_value)
        .unwrap_or(fallback_layer);
    let related_paths = string_list(row.get("related_paths"));
    let authority_link_paths = string_list(row.get("authority_link_paths"));
    json!({
        "doc_id": object_text(row, "doc_id").unwrap_or_else(|| path.clone()),
        "path": path,
        "filename": filename(&path),
        "title": title,
        "short_title": object_text(row, "short_title").unwrap_or_else(|| document_short_title(&title, &path)),
        "layer": layer,
        "status": object_text(row, "status").or_else(|| metadata.get("status").cloned()).unwrap_or_else(|| "current".to_string()),
        "scope": object_text(row, "scope").or_else(|| metadata.get("scope").cloned()).unwrap_or_default(),
        "authority": object_text(row, "authority").or_else(|| metadata.get("authority").cloned()).unwrap_or_default(),
        "markdown": markdown,
        "body_markdown": object_text(row, "body_markdown").unwrap_or_else(|| body_markdown(&markdown)),
        "related_paths": if related_paths.is_empty() { markdown_link_targets(&path, &markdown) } else { related_paths },
        "authority_link_paths": if authority_link_paths.is_empty() {
            metadata.get("authority").map(|text| markdown_link_targets(&path, text)).unwrap_or_default()
        } else {
            authority_link_paths
        },
    })
}

pub(super) fn authority_doc_or_missing(
    docs: &HashMap<String, &JsonMap<String, JsonValue>>,
    path: &str,
    layer: i64,
    title: Option<&str>,
) -> JsonValue {
    docs.get(path)
        .map(|row| authority_doc(row, layer))
        .unwrap_or_else(|| authority_missing_doc(path, layer, title))
}

pub(super) fn authority_missing_doc(path: &str, layer: i64, title: Option<&str>) -> JsonValue {
    let title = title.unwrap_or(path);
    json!({
        "doc_id": path,
        "path": path,
        "filename": filename(path),
        "title": title,
        "short_title": document_short_title(title, path),
        "layer": layer,
        "status": "current",
        "scope": "",
        "authority": "",
        "markdown": "",
        "body_markdown": "",
        "related_paths": [],
        "authority_link_paths": [],
    })
}

pub(super) fn merge_node_fields(doc: &mut JsonValue, row: &JsonMap<String, JsonValue>) {
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    for field in [
        "authority_node_id",
        "authority_map_id",
        "node_kind",
        "parent_node_id",
        "sort_index",
        "slug",
        "connection_mode",
    ] {
        if let Some(value) = row.get(field) {
            obj.insert(field.to_string(), value.clone());
        }
    }
    if let Some(title) = object_text(row, "title") {
        obj.insert("title".to_string(), json!(title));
        let path = obj
            .get("path")
            .and_then(json_value_to_text)
            .unwrap_or_default();
        obj.insert(
            "short_title".to_string(),
            json!(document_short_title(&title, &path)),
        );
    }
}

pub(super) fn add_related_documents(documents_by_path: &mut BTreeMap<String, JsonValue>) {
    let snapshot = documents_by_path.clone();
    for doc in documents_by_path.values_mut() {
        let related = doc
            .get("related_paths")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(json_value_to_text)
            .filter_map(|path| {
                let target = snapshot.get(&path)?;
                if value_text(doc, "path").as_deref() == Some(path.as_str()) {
                    return None;
                }
                Some(json!({
                    "path": path,
                    "title": value_text(target, "short_title"),
                    "layer": target.get("layer").cloned().unwrap_or(JsonValue::Null),
                }))
            })
            .collect::<Vec<_>>();
        doc.as_object_mut()
            .expect("authority docs are objects")
            .insert("related_documents".to_string(), JsonValue::Array(related));
    }
}

pub(super) fn sync_related_documents(
    doc: &mut JsonValue,
    documents_by_path: &BTreeMap<String, JsonValue>,
) {
    let related = value_text(doc, "path")
        .and_then(|path| documents_by_path.get(&path).cloned())
        .and_then(|source| source.get("related_documents").cloned())
        .unwrap_or_else(|| json!([]));
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    obj.insert("related_documents".to_string(), related);
    if let Some(children) = obj.get_mut("children").and_then(JsonValue::as_array_mut) {
        for child in children {
            sync_related_documents(child, documents_by_path);
        }
    }
}

pub(super) fn authority_node_layer(node_kind: Option<&str>) -> i64 {
    match node_kind.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "layer1" => 1,
        "layer2" => 2,
        "milestone" | "layer3" => 3,
        _ => 3,
    }
}

pub(super) fn authority_parent_path(doc: &JsonValue, legal_paths: &BTreeSet<String>) -> String {
    let path = value_text(doc, "path").unwrap_or_default();
    if let Some(override_path) = authority_parent_override(&path) {
        return override_path.to_string();
    }
    for related_path in doc
        .get("authority_link_paths")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(json_value_to_text)
    {
        if legal_paths.contains(&related_path) {
            return related_path;
        }
    }
    let lowered = path.to_ascii_lowercase();
    if ["financial", "economics", "runway", "cost"]
        .iter()
        .any(|token| lowered.contains(token))
    {
        return "docs/financial_plan.md".to_string();
    }
    if ["market", "benchmark", "launch"]
        .iter()
        .any(|token| lowered.contains(token))
    {
        return "docs/market_strategy.md".to_string();
    }
    if [
        "legal",
        "license",
        "privacy",
        "security",
        "attestation",
        "policy",
    ]
    .iter()
    .any(|token| lowered.contains(token))
    {
        return "docs/legal_plan.md".to_string();
    }
    if ["product", "segmentation", "ux_principles", "roadmap"]
        .iter()
        .any(|token| lowered.contains(token))
    {
        return "docs/product_plan.md".to_string();
    }
    "docs/engineering_plan.md".to_string()
}

pub(super) fn authority_parent_override(path: &str) -> Option<&'static str> {
    match path {
        "AGENTS.md" | "docs/ait_team_remote.md" => Some("docs/engineering_plan.md"),
        _ => None,
    }
}

pub(super) fn sort_docs(docs: &mut [JsonValue]) {
    docs.sort_by(|left, right| {
        value_int(left, "sort_index")
            .cmp(&value_int(right, "sort_index"))
            .then_with(|| value_text(left, "short_title").cmp(&value_text(right, "short_title")))
            .then_with(|| value_text(left, "path").cmp(&value_text(right, "path")))
    });
}

pub(super) fn document_short_title(title: &str, path: &str) -> String {
    if title.trim().is_empty() {
        filename_stem(path)
    } else {
        title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::authority_parent_override;

    #[test]
    fn authority_parent_override_excludes_retired_static_workflow_documents() {
        assert_eq!(
            authority_parent_override("AGENTS.md"),
            Some("docs/engineering_plan.md")
        );
        assert_eq!(
            authority_parent_override("docs/ait_team_remote.md"),
            Some("docs/engineering_plan.md")
        );
        for retired_path in [
            "ait.md",
            "docs/ait.md",
            "docs/ait_solo_local.md",
            "docs/ait_solo_remote.md",
        ] {
            assert_eq!(authority_parent_override(retired_path), None);
        }
    }
}
