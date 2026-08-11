use super::*;

const AUTHORITY_LAYER2_PATHS: &[&str] = &[
    "docs/product_plan.md",
    "docs/market_strategy.md",
    "docs/engineering_plan.md",
    "docs/financial_plan.md",
    "docs/legal_plan.md",
];
const AUTHORITY_CENTER_NODE_PATHS: &[&str] = &["docs/milestone.md"];

pub fn authority_map_read_model(input: &AuthorityMapInput) -> Result<JsonValue, String> {
    let document_index = input
        .documents
        .iter()
        .filter_map(|row| object_text(row, "path").map(|path| (path, row)))
        .collect::<HashMap<_, _>>();
    let mut payload = if input.authority_nodes.is_empty() {
        authority_map_from_documents(&document_index)
    } else {
        authority_map_from_nodes(&document_index, &input.authority_nodes)
    };
    let interactive = input
        .local_repo_name
        .as_deref()
        .map(|local| local == input.repo_name)
        .unwrap_or(true);
    let obj = payload
        .as_object_mut()
        .ok_or_else(|| "authority map projection must be an object.".to_string())?;
    obj.insert("repo_name".to_string(), json!(input.repo_name));
    obj.insert("interactive".to_string(), json!(interactive));
    obj.insert(
        "authority_summary".to_string(),
        json!({
            "actor_count": input.actors.len(),
            "role_count": input.roles.len(),
            "permission_count": input.permissions.len(),
        }),
    );
    Ok(payload)
}

fn authority_map_from_documents(
    document_index: &HashMap<String, &JsonMap<String, JsonValue>>,
) -> JsonValue {
    let mut layer1 = authority_doc_or_missing(document_index, "docs/plan.md", 1, Some("Plan"));
    let mut layer2_docs = AUTHORITY_LAYER2_PATHS
        .iter()
        .filter_map(|path| document_index.get(*path).map(|row| authority_doc(row, 2)))
        .collect::<Vec<_>>();
    let mut center_nodes = AUTHORITY_CENTER_NODE_PATHS
        .iter()
        .filter_map(|path| document_index.get(*path).map(|row| authority_doc(row, 3)))
        .collect::<Vec<_>>();
    if center_nodes.is_empty() {
        center_nodes.push(authority_missing_doc(
            "docs/milestone.md",
            3,
            Some("Milestone Index"),
        ));
    }
    for doc in &mut center_nodes {
        insert_string(
            doc,
            "node_role",
            if value_text(doc, "path").as_deref() == Some("docs/milestone.md") {
                "milestone"
            } else {
                "center"
            },
        );
        insert_string(doc, "display_parent_path", "docs/engineering_plan.md");
    }
    let legal_paths = layer2_docs
        .iter()
        .filter_map(|doc| value_text(doc, "path"))
        .collect::<BTreeSet<_>>();
    let center_paths = center_nodes
        .iter()
        .filter_map(|doc| value_text(doc, "path"))
        .collect::<BTreeSet<_>>();
    let layer3_docs = document_index
        .iter()
        .filter(|(path, _)| {
            path.as_str() != "docs/plan.md"
                && !legal_paths.contains(path.as_str())
                && !center_paths.contains(path.as_str())
        })
        .map(|(_, row)| authority_doc(row, 3))
        .collect::<Vec<_>>();

    let mut children_by_parent: BTreeMap<String, Vec<JsonValue>> = BTreeMap::new();
    for mut doc in layer3_docs {
        let parent_path = authority_parent_path(&doc, &legal_paths);
        insert_string(&mut doc, "display_parent_path", &parent_path);
        children_by_parent.entry(parent_path).or_default().push(doc);
    }
    for doc in &mut layer2_docs {
        let path = value_text(doc, "path").unwrap_or_default();
        let mut children = children_by_parent.remove(&path).unwrap_or_default();
        sort_docs(&mut children);
        if let Some(obj) = doc.as_object_mut() {
            obj.insert("children".to_string(), JsonValue::Array(children));
        }
    }
    sort_docs(&mut layer2_docs);
    let mut documents_by_path = BTreeMap::new();
    documents_by_path.insert(
        value_text(&layer1, "path").unwrap_or_default(),
        layer1.clone(),
    );
    for doc in center_nodes.iter().chain(layer2_docs.iter()) {
        documents_by_path.insert(value_text(doc, "path").unwrap_or_default(), doc.clone());
        for child in doc
            .get("children")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            documents_by_path.insert(value_text(child, "path").unwrap_or_default(), child.clone());
        }
    }
    add_related_documents(&mut documents_by_path);
    sync_related_documents(&mut layer1, &documents_by_path);
    for doc in center_nodes.iter_mut().chain(layer2_docs.iter_mut()) {
        sync_related_documents(doc, &documents_by_path);
    }
    let layer3_count = documents_by_path
        .values()
        .filter(|doc| value_int(doc, "layer") == 3)
        .count();
    let relationship_count = layer2_docs.len()
        + layer2_docs
            .iter()
            .map(|doc| {
                doc.get("children")
                    .and_then(JsonValue::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
            })
            .sum::<usize>()
        + documents_by_path
            .values()
            .map(|doc| {
                doc.get("related_documents")
                    .and_then(JsonValue::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
            })
            .sum::<usize>();
    json!({
        "layer1": layer1,
        "center_nodes": center_nodes,
        "layer2": layer2_docs,
        "linked_documents": [],
        "summary": {
            "center_node_count": center_nodes.len(),
            "layer2_count": layer2_docs.len(),
            "layer3_count": layer3_count,
            "relationship_count": relationship_count,
        },
    })
}

fn authority_map_from_nodes(
    document_index: &HashMap<String, &JsonMap<String, JsonValue>>,
    nodes: &[JsonMap<String, JsonValue>],
) -> JsonValue {
    let mut all_nodes = BTreeMap::<String, JsonValue>::new();
    let mut nodes_by_id = HashMap::<String, JsonValue>::new();
    for row in nodes {
        let path = object_text(row, "document_path")
            .or_else(|| object_text(row, "path"))
            .unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        let layer = authority_node_layer(object_text(row, "node_kind").as_deref());
        let mut doc = document_index
            .get(&path)
            .map(|source| authority_doc(source, layer))
            .unwrap_or_else(|| {
                authority_missing_doc(&path, layer, object_text(row, "title").as_deref())
            });
        merge_node_fields(&mut doc, row);
        let node_id = value_text(&doc, "authority_node_id").unwrap_or_else(|| format!("db:{path}"));
        all_nodes.insert(path, doc.clone());
        nodes_by_id.insert(node_id, doc);
    }
    if !all_nodes.contains_key("docs/plan.md") {
        let doc = authority_missing_doc("docs/plan.md", 1, Some("Plan"));
        nodes_by_id.insert("db:docs/plan.md".to_string(), doc.clone());
        all_nodes.insert("docs/plan.md".to_string(), doc);
    }
    if !all_nodes.contains_key("docs/milestone.md") {
        let doc = authority_missing_doc("docs/milestone.md", 3, Some("Milestone"));
        nodes_by_id.insert("db:docs/milestone.md".to_string(), doc.clone());
        all_nodes.insert("docs/milestone.md".to_string(), doc);
    }
    add_related_documents(&mut all_nodes);
    let mut layer1 = all_nodes
        .values()
        .find(|doc| value_text(doc, "node_kind").as_deref() == Some("layer1"))
        .cloned()
        .or_else(|| all_nodes.get("docs/plan.md").cloned())
        .unwrap_or_else(|| authority_missing_doc("docs/plan.md", 1, Some("Plan")));
    layer1["layer"] = json!(1);
    let mut center_nodes = all_nodes
        .values()
        .filter(|doc| value_text(doc, "node_kind").as_deref() == Some("milestone"))
        .cloned()
        .collect::<Vec<_>>();
    if center_nodes.is_empty() {
        center_nodes.push(authority_missing_doc(
            "docs/milestone.md",
            3,
            Some("Milestone"),
        ));
    }
    for doc in &mut center_nodes {
        insert_string(
            doc,
            "node_role",
            if value_text(doc, "path").as_deref() == Some("docs/milestone.md") {
                "milestone"
            } else {
                "center"
            },
        );
        insert_string(doc, "display_parent_path", "docs/engineering_plan.md");
    }
    let mut layer2_docs = all_nodes
        .values()
        .filter(|doc| value_text(doc, "node_kind").as_deref() == Some("layer2"))
        .cloned()
        .collect::<Vec<_>>();
    let legal_paths = layer2_docs
        .iter()
        .filter_map(|doc| value_text(doc, "path"))
        .collect::<BTreeSet<_>>();
    let mut children_by_parent = BTreeMap::<String, Vec<JsonValue>>::new();
    let mut linked_documents = Vec::new();
    for mut node in all_nodes
        .values()
        .filter(|doc| value_text(doc, "node_kind").as_deref() == Some("layer3"))
        .cloned()
    {
        let parent = value_text(&node, "parent_node_id").and_then(|parent_id| {
            nodes_by_id
                .get(&parent_id)
                .and_then(|doc| value_text(doc, "path"))
        });
        let parent_path = parent.unwrap_or_else(|| authority_parent_path(&node, &legal_paths));
        insert_string(&mut node, "display_parent_path", &parent_path);
        if legal_paths.contains(&parent_path) {
            children_by_parent
                .entry(parent_path)
                .or_default()
                .push(node);
        } else {
            linked_documents.push(node);
        }
    }
    for doc in &mut layer2_docs {
        let path = value_text(doc, "path").unwrap_or_default();
        let mut children = children_by_parent.remove(&path).unwrap_or_default();
        sort_docs(&mut children);
        doc.as_object_mut()
            .expect("authority docs are objects")
            .insert("children".to_string(), JsonValue::Array(children));
    }
    sort_docs(&mut layer2_docs);
    sort_docs(&mut linked_documents);
    let layer3_count = all_nodes
        .values()
        .filter(|doc| value_text(doc, "node_kind").as_deref() == Some("layer3"))
        .count();
    let relationship_count = layer2_docs.len()
        + layer2_docs
            .iter()
            .map(|doc| {
                doc.get("children")
                    .and_then(JsonValue::as_array)
                    .map(Vec::len)
                    .unwrap_or(0)
            })
            .sum::<usize>();
    json!({
        "layer1": layer1,
        "center_nodes": center_nodes,
        "layer2": layer2_docs,
        "linked_documents": linked_documents,
        "summary": {
            "center_node_count": center_nodes.len(),
            "layer2_count": layer2_docs.len(),
            "layer3_count": layer3_count + center_nodes.len(),
            "relationship_count": relationship_count,
        },
    })
}
