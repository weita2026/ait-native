use regex::Regex;
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const PLAN_REVISION_CONTRACT: &str = "ait.server.plan_revision.v1";
pub const PLAN_REVISION_REFERENCE_MODULE: &str =
    "rust/crates/ait-server-core/src/foundation/plan_revision.rs";

pub fn plan_revision_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "plan-revision payload must be a JSON object.".to_string())?;
    match operation {
        "contract" => Ok(json!({
            "contract": PLAN_REVISION_CONTRACT,
            "reference_module": PLAN_REVISION_REFERENCE_MODULE,
            "mutates_state": false,
            "excluded_reference_behaviors": [
                "database reads/writes",
                "Markdown blob reads",
                "planning-session mutation"
            ],
            "operations": [
                "normalize-artifact",
                "surface-entries",
                "surface-hash",
                "changed-count",
                "metadata",
                "revision-view"
            ],
        })),
        "normalize-artifact" => Ok(json!({
            "contract": PLAN_REVISION_CONTRACT,
            "artifact": normalize_plan_revision_artifact(payload)?,
        })),
        "surface-entries" => {
            let items = payload.get("items").or_else(|| payload.get("items_json"));
            Ok(json!({
                "contract": PLAN_REVISION_CONTRACT,
                "entries": plan_link_surface_entries_value(
                    items,
                    optional_text(payload.get("artifact_body")).as_deref(),
                )?,
            }))
        }
        "surface-hash" => {
            let entries = required_object(payload.get("entries"), "entries")?;
            Ok(json!({
                "contract": PLAN_REVISION_CONTRACT,
                "plan_links_surface_hash": plan_link_surface_hash(entries)?,
            }))
        }
        "changed-count" => {
            let previous_entries = optional_object(payload.get("previous_entries"));
            let current_entries =
                required_object(payload.get("current_entries"), "current_entries")?;
            Ok(json!({
                "contract": PLAN_REVISION_CONTRACT,
                "plan_links_changed_count_to_prev": plan_link_changed_count(previous_entries, current_entries),
            }))
        }
        "metadata" => {
            let items = payload.get("items").or_else(|| payload.get("items_json"));
            let previous_entries = optional_object(payload.get("previous_entries"));
            let entries = plan_link_surface_entries_value(
                items,
                optional_text(payload.get("artifact_body")).as_deref(),
            )?;
            Ok(json!({
                "contract": PLAN_REVISION_CONTRACT,
                "plan_links_surface_hash": plan_link_surface_hash(&entries)?,
                "plan_links_changed_count_to_prev": plan_link_changed_count(previous_entries, &entries),
                "entries": entries,
            }))
        }
        "revision-view" => {
            let row = required_object(payload.get("row"), "row")?;
            let blob = optional_object(payload.get("blob"));
            let artifacts = payload
                .get("artifacts")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            let include_artifact_body = optional_bool(payload.get("include_artifact_body"));
            let preserve_items_json = optional_bool(payload.get("preserve_items_json"));
            Ok(json!({
                "contract": PLAN_REVISION_CONTRACT,
                "revision": plan_revision_view(
                    row,
                    blob,
                    Some(artifacts),
                    optional_text(payload.get("artifact_body")),
                    PlanRevisionViewOptions {
                        include_artifact_body,
                        preserve_items_json,
                        include_blob_object: optional_bool(payload.get("include_blob_object")),
                    },
                )?,
            }))
        }
        other => Err(format!("Unsupported plan-revision operation `{other}`.")),
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PlanRevisionViewOptions {
    pub include_artifact_body: bool,
    pub preserve_items_json: bool,
    pub include_blob_object: bool,
}

pub fn normalize_plan_revision_artifact(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let artifact_path = required_text(payload.get("artifact_path"), "Plan artifact_path")?;
    let artifact_selector = optional_text(payload.get("artifact_selector"));
    let artifact_heading = required_text(payload.get("artifact_heading"), "Plan artifact_heading")?;
    let items = normalized_plan_items(payload.get("items").or_else(|| payload.get("items_json")))?;
    let items_json = serde_json::to_string(&JsonValue::Array(items.clone()))
        .map_err(|exc| format!("Plan items must serialize as JSON: {exc}"))?;
    Ok(JsonMap::from_iter([
        ("artifact_path".to_string(), json!(artifact_path)),
        (
            "artifact_selector".to_string(),
            artifact_selector
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        ("artifact_heading".to_string(), json!(artifact_heading)),
        ("items".to_string(), JsonValue::Array(items)),
        ("items_json".to_string(), json!(items_json)),
    ]))
}

pub fn normalized_plan_items(value: Option<&JsonValue>) -> Result<Vec<JsonValue>, String> {
    let items = match value {
        Some(JsonValue::Array(items)) => items.clone(),
        Some(JsonValue::String(text)) if text.trim().is_empty() => Vec::new(),
        Some(JsonValue::String(text)) => {
            let parsed: JsonValue = serde_json::from_str(text)
                .map_err(|exc| format!("items_json must be valid JSON: {exc}"))?;
            match parsed {
                JsonValue::Array(items) => items,
                _ => return Err("items_json must encode a JSON array.".to_string()),
            }
        }
        Some(_) => return Err("Plan items must be a JSON array.".to_string()),
        None => Vec::new(),
    };
    Ok(items
        .into_iter()
        .filter_map(|item| normalize_plan_item(item).transpose())
        .collect::<Result<Vec<_>, _>>()?)
}

pub fn plan_link_surface_entries_value(
    items: Option<&JsonValue>,
    artifact_body: Option<&str>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let normalized_items = normalized_plan_items(items)?;
    Ok(plan_link_surface_entries_from_items(
        &normalized_items,
        artifact_body,
    ))
}

pub fn plan_link_surface_entries_from_items(
    items: &[JsonValue],
    artifact_body: Option<&str>,
) -> JsonMap<String, JsonValue> {
    let details_by_ref = plan_link_item_details_by_ref(artifact_body, items);
    let mut entries = JsonMap::new();
    for item in items {
        let Some(item) = item.as_object() else {
            continue;
        };
        let Some(plan_item_ref) = optional_text(item.get("plan_item_ref")) else {
            continue;
        };
        entries.insert(
            plan_item_ref.clone(),
            json!({
                "plan_item_ref": plan_item_ref,
                "text": plan_link_inline_text(item.get("text")),
                "checkbox_state": optional_text(item.get("checkbox_state")).unwrap_or_else(|| "none".to_string()),
                "heading_path": plan_link_heading_path(item.get("heading_path")),
                "details": optional_text(details_by_ref.get(&plan_item_ref)).unwrap_or_default(),
            }),
        );
    }
    entries
}

pub fn plan_link_surface_hash(entries: &JsonMap<String, JsonValue>) -> Result<String, String> {
    let payload = entries
        .keys()
        .map(|key| entries.get(key).cloned().unwrap_or(JsonValue::Null))
        .collect::<Vec<_>>();
    let canonical = serde_json::to_string(&JsonValue::Array(payload))
        .map_err(|exc| format!("Plan link surface entries must serialize as JSON: {exc}"))?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn plan_link_changed_count(
    previous_entries: Option<&JsonMap<String, JsonValue>>,
    current_entries: &JsonMap<String, JsonValue>,
) -> i64 {
    let Some(previous_entries) = previous_entries else {
        return 0;
    };
    let keys = previous_entries
        .keys()
        .chain(current_entries.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    keys.into_iter()
        .filter(|key| previous_entries.get(key) != current_entries.get(key))
        .count() as i64
}

pub fn plan_link_metadata(
    items: Option<&JsonValue>,
    artifact_body: Option<&str>,
    previous_entries: Option<&JsonMap<String, JsonValue>>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let entries = plan_link_surface_entries_value(items, artifact_body)?;
    Ok(JsonMap::from_iter([
        (
            "plan_links_surface_hash".to_string(),
            json!(plan_link_surface_hash(&entries)?),
        ),
        (
            "plan_links_changed_count_to_prev".to_string(),
            json!(plan_link_changed_count(previous_entries, &entries)),
        ),
        ("entries".to_string(), JsonValue::Object(entries)),
    ]))
}

pub fn plan_revision_view(
    row: &JsonMap<String, JsonValue>,
    blob: Option<&JsonMap<String, JsonValue>>,
    artifacts: Option<Vec<JsonValue>>,
    artifact_body: Option<String>,
    options: PlanRevisionViewOptions,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = row.clone();
    let raw_items_json = out.remove("items_json").unwrap_or_else(|| json!("[]"));
    let items = normalized_plan_items(Some(&raw_items_json)).unwrap_or_default();
    if options.preserve_items_json {
        out.insert("items_json".to_string(), raw_items_json);
    }
    out.insert("items".to_string(), JsonValue::Array(items));
    out.entry("plan_links_surface_hash".to_string())
        .or_insert(JsonValue::Null);
    let changed_count = out
        .get("plan_links_changed_count_to_prev")
        .and_then(json_i64)
        .unwrap_or(0);
    out.insert(
        "plan_links_changed_count_to_prev".to_string(),
        json!(changed_count),
    );
    out.entry("artifact_blob_id".to_string())
        .or_insert(JsonValue::Null);

    if let Some(blob) = blob {
        copy_blob_field(&mut out, blob, "blob_id", "artifact_blob_id");
        copy_blob_field(&mut out, blob, "media_type", "artifact_media_type");
        copy_blob_field(&mut out, blob, "encoding", "artifact_encoding");
        copy_blob_field(&mut out, blob, "byte_count", "artifact_byte_count");
        copy_blob_field(&mut out, blob, "created_at", "artifact_blob_created_at");
        copy_blob_field(
            &mut out,
            blob,
            "storage_authority",
            "artifact_storage_authority",
        );
        copy_blob_field(&mut out, blob, "object_pack_id", "artifact_object_pack_id");
        copy_blob_field(&mut out, blob, "tree_id", "artifact_tree_id");
        copy_blob_field(&mut out, blob, "tree_pack_id", "artifact_tree_pack_id");
        if options.include_blob_object {
            out.insert("blob".to_string(), JsonValue::Object(blob.clone()));
        }
    } else {
        out.entry("artifact_media_type".to_string())
            .or_insert(JsonValue::Null);
        out.entry("artifact_encoding".to_string())
            .or_insert(JsonValue::Null);
        out.entry("artifact_byte_count".to_string())
            .or_insert(JsonValue::Null);
        out.entry("artifact_blob_created_at".to_string())
            .or_insert(JsonValue::Null);
        if options.include_blob_object {
            out.insert("blob".to_string(), JsonValue::Null);
        }
    }
    if let Some(artifacts) = artifacts {
        out.insert("artifacts".to_string(), JsonValue::Array(artifacts));
    }
    if options.include_artifact_body {
        out.insert(
            "artifact_body".to_string(),
            artifact_body
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        );
    }
    Ok(out)
}

pub fn plan_revision_artifact_view(row: &JsonMap<String, JsonValue>) -> JsonMap<String, JsonValue> {
    let mut out = row.clone();
    let metadata = match out.remove("metadata") {
        Some(JsonValue::Object(map)) => JsonValue::Object(map),
        _ => match out.remove("metadata_json") {
            Some(JsonValue::String(text)) => serde_json::from_str::<JsonValue>(&text)
                .ok()
                .filter(JsonValue::is_object)
                .unwrap_or_else(|| json!({})),
            Some(JsonValue::Object(map)) => JsonValue::Object(map),
            _ => json!({}),
        },
    };
    out.insert("metadata".to_string(), metadata);
    out
}

fn normalize_plan_item(value: JsonValue) -> Result<Option<JsonValue>, String> {
    let JsonValue::Object(mut item) = value else {
        return Ok(None);
    };
    normalize_optional_item_text(&mut item, "plan_item_ref");
    normalize_optional_item_text(&mut item, "checkbox_state");
    if item.get("checkbox_state").and_then(JsonValue::as_str) == Some("none") {
        item.remove("checkbox_state");
    }
    if let Some(text) = item.get("text") {
        item.insert("text".to_string(), json!(plan_link_inline_text(Some(text))));
    }
    let heading_path = plan_link_heading_path(item.get("heading_path"));
    if !heading_path.is_empty() {
        item.insert("heading_path".to_string(), JsonValue::Array(heading_path));
    } else {
        item.remove("heading_path");
    }
    if let Some(line_number) = item.get("line_number").and_then(json_i64) {
        if line_number == 0 {
            item.remove("line_number");
        } else {
            item.insert("line_number".to_string(), json!(line_number));
        }
    } else {
        item.remove("line_number");
    }
    Ok(Some(JsonValue::Object(item)))
}

fn normalize_optional_item_text(item: &mut JsonMap<String, JsonValue>, key: &str) {
    match optional_text(item.get(key)) {
        Some(value) => {
            item.insert(key.to_string(), JsonValue::String(value));
        }
        None => {
            item.remove(key);
        }
    }
}

fn plan_link_item_details_by_ref(
    artifact_body: Option<&str>,
    items: &[JsonValue],
) -> JsonMap<String, JsonValue> {
    let lines = artifact_body
        .unwrap_or_default()
        .lines()
        .collect::<Vec<_>>();
    let list_item_re = Regex::new(r"^(?P<indent>\s*)(?:[-*+]|\d+\.)\s+(?:\[[ xX]\]\s+)?")
        .expect("plan-link list item regex should compile");
    let mut details_by_ref = JsonMap::new();
    for item in items {
        let Some(item) = item.as_object() else {
            continue;
        };
        let Some(plan_item_ref) = optional_text(item.get("plan_item_ref")) else {
            continue;
        };
        let Some(line_number) = item.get("line_number").and_then(json_i64) else {
            continue;
        };
        if line_number <= 0 || line_number as usize > lines.len() {
            continue;
        }
        let line_index = line_number as usize - 1;
        let raw_line = lines[line_index];
        let Some(list_match) = list_item_re.captures(raw_line) else {
            continue;
        };
        let item_indent = list_match
            .name("indent")
            .map(|matched| matched.as_str().len())
            .unwrap_or(0);
        let mut detail_lines = Vec::new();
        let mut pending_blank = false;
        for raw_following in lines.iter().skip(line_index + 1) {
            if raw_following.trim_start().starts_with('#') {
                break;
            }
            let following_match = list_item_re.captures(raw_following);
            let current_indent = raw_following.len() - raw_following.trim_start().len();
            if following_match.is_some() && current_indent <= item_indent {
                break;
            }
            if raw_following.trim().is_empty() {
                pending_blank = true;
                continue;
            }
            if current_indent <= item_indent && following_match.is_none() {
                break;
            }
            if pending_blank && !detail_lines.is_empty() {
                detail_lines.push(String::new());
            }
            detail_lines.push(raw_following.trim().to_string());
            pending_blank = false;
        }
        let detail_text = detail_lines.join("\n").trim().to_string();
        if !detail_text.is_empty() {
            details_by_ref.insert(plan_item_ref, JsonValue::String(detail_text));
        }
    }
    details_by_ref
}

fn plan_link_inline_text(value: Option<&JsonValue>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    value_to_text(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn plan_link_heading_path(value: Option<&JsonValue>) -> Vec<JsonValue> {
    let Some(JsonValue::Array(values)) = value else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|value| {
            let normalized = plan_link_inline_text(Some(value));
            if normalized.is_empty() {
                None
            } else {
                Some(JsonValue::String(normalized))
            }
        })
        .collect()
}

fn copy_blob_field(
    out: &mut JsonMap<String, JsonValue>,
    blob: &JsonMap<String, JsonValue>,
    source: &str,
    target: &str,
) {
    out.insert(
        target.to_string(),
        blob.get(source).cloned().unwrap_or(JsonValue::Null),
    );
}

fn required_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("plan-revision payload requires object field `{field}`."))
}

fn optional_object(value: Option<&JsonValue>) -> Option<&JsonMap<String, JsonValue>> {
    value.and_then(JsonValue::as_object)
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text(value).ok_or_else(|| format!("{field} must not be empty"))
}

fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    let text = value_to_text(value).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn optional_bool(value: Option<&JsonValue>) -> bool {
    value.and_then(JsonValue::as_bool).unwrap_or(false)
}

fn json_i64(value: &JsonValue) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

fn value_to_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => text.to_string(),
        JsonValue::Null => String::new(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(value: JsonValue) -> JsonMap<String, JsonValue> {
        value
            .as_object()
            .cloned()
            .expect("value should be an object")
    }

    fn sample_items() -> JsonValue {
        json!([
            {
                "plan_item_ref": " demo/first ",
                "text": " First   task ",
                "checkbox_state": "todo",
                "heading_path": [" Sprint ", "", "Now"],
                "line_number": "3"
            },
            {
                "plan_item_ref": "demo/second",
                "text": "Second task",
                "checkbox_state": "done",
                "heading_path": ["Sprint"],
                "line_number": 8
            }
        ])
    }

    fn sample_markdown() -> &'static str {
        "# Sprint\n\n- [ ] First task\n  First detail line\n  continuation\n\n  second paragraph\n- [x] Second task"
    }

    #[test]
    fn normalize_plan_revision_artifact_trims_fields_and_serializes_items() {
        let payload = object(json!({
            "artifact_path": " docs/sprints/demo.md ",
            "artifact_selector": " #target ",
            "artifact_heading": " Demo Sprint ",
            "items": sample_items()
        }));

        let artifact = normalize_plan_revision_artifact(&payload).expect("normalized artifact");

        assert_eq!(artifact["artifact_path"], json!("docs/sprints/demo.md"));
        assert_eq!(artifact["artifact_selector"], json!("#target"));
        assert_eq!(artifact["artifact_heading"], json!("Demo Sprint"));
        assert_eq!(artifact["items"][0]["plan_item_ref"], json!("demo/first"));
        assert_eq!(artifact["items"][0]["text"], json!("First task"));
        assert_eq!(
            artifact["items"][0]["heading_path"],
            json!(["Sprint", "Now"])
        );
        assert_eq!(artifact["items"][0]["line_number"], json!(3));
        assert!(artifact["items_json"]
            .as_str()
            .expect("items_json should be text")
            .contains("\"demo/first\""));

        let error = normalize_plan_revision_artifact(&object(json!({
            "artifact_path": "",
            "artifact_heading": "Demo",
            "items": []
        })))
        .expect_err("empty artifact_path should fail");
        assert_eq!(error, "Plan artifact_path must not be empty");
    }

    #[test]
    fn plan_link_entries_hash_and_changed_count_match_reference_surface() {
        let entries =
            plan_link_surface_entries_value(Some(&sample_items()), Some(sample_markdown()))
                .expect("entries");

        assert_eq!(entries["demo/first"]["text"], json!("First task"));
        assert_eq!(
            entries["demo/first"]["details"],
            json!("First detail line\ncontinuation\n\nsecond paragraph")
        );
        assert_eq!(entries["demo/second"]["details"], json!(""));
        assert_eq!(
            plan_link_surface_hash(&entries).expect("surface hash"),
            "ebb270cd23b5d2fb3762e8d9a2949d31b7d1ecdf088761bfa2535629e6bf9843"
        );
        assert_eq!(plan_link_changed_count(None, &entries), 0);

        let previous = plan_link_surface_entries_value(
            Some(&json!([{
                "plan_item_ref": "demo/first",
                "text": "Old task",
                "checkbox_state": "todo",
                "heading_path": ["Sprint"],
                "line_number": 3
            }])),
            Some(sample_markdown()),
        )
        .expect("previous entries");
        assert_eq!(plan_link_changed_count(Some(&previous), &entries), 2);

        let metadata = plan_link_metadata(
            Some(&sample_items()),
            Some(sample_markdown()),
            Some(&previous),
        )
        .expect("metadata");
        assert_eq!(metadata["plan_links_changed_count_to_prev"], json!(2));
        assert_eq!(
            metadata["plan_links_surface_hash"],
            json!("ebb270cd23b5d2fb3762e8d9a2949d31b7d1ecdf088761bfa2535629e6bf9843")
        );
    }

    #[test]
    fn plan_revision_view_parses_items_and_preserves_bridge_compatibility_fields() {
        let row = object(json!({
            "plan_revision_id": "PR-1",
            "plan_id": "PL-1",
            "revision_number": 2,
            "items_json": "[{\"plan_item_ref\":\"demo/first\",\"text\":\"First\"}]",
            "plan_links_surface_hash": null,
            "plan_links_changed_count_to_prev": "bad",
            "created_at": "2026-07-08T10:00:00Z"
        }));
        let blob = object(json!({
            "blob_id": "BLB-1",
            "media_type": "text/markdown",
            "encoding": "utf-8",
            "byte_count": 42,
            "created_at": "2026-07-08T10:00:01Z",
            "storage_authority": "remote_zstd_pack",
            "object_pack_id": "PCK-1",
            "tree_id": "TREE-1",
            "tree_pack_id": "TPK-1"
        }));
        let view = plan_revision_view(
            &row,
            Some(&blob),
            Some(vec![json!({"artifact_path": "task.json"})]),
            Some("# Sprint".to_string()),
            PlanRevisionViewOptions {
                include_artifact_body: true,
                preserve_items_json: true,
                include_blob_object: true,
            },
        )
        .expect("revision view");

        assert_eq!(view["items"][0]["plan_item_ref"], json!("demo/first"));
        assert_eq!(view["items_json"], row["items_json"]);
        assert_eq!(view["plan_links_surface_hash"], JsonValue::Null);
        assert_eq!(view["plan_links_changed_count_to_prev"], json!(0));
        assert_eq!(view["artifact_blob_id"], json!("BLB-1"));
        assert_eq!(
            view["artifact_storage_authority"],
            json!("remote_zstd_pack")
        );
        assert_eq!(view["blob"]["object_pack_id"], json!("PCK-1"));
        assert_eq!(view["artifacts"][0]["artifact_path"], json!("task.json"));
        assert_eq!(view["artifact_body"], json!("# Sprint"));

        let artifact = plan_revision_artifact_view(&object(json!({
            "artifact_path": "task.json",
            "metadata_json": "{bad json"
        })));
        assert_eq!(artifact["metadata"], json!({}));
        assert!(artifact.get("metadata_json").is_none());
    }

    #[test]
    fn plan_revision_json_exposes_seam_operations() {
        let contract = plan_revision_json("contract", &json!({})).expect("contract");
        assert_eq!(contract["contract"], json!("ait.server.plan_revision.v1"));
        assert_eq!(contract["mutates_state"], json!(false));

        let metadata = plan_revision_json(
            "metadata",
            &json!({
                "items": sample_items(),
                "artifact_body": sample_markdown()
            }),
        )
        .expect("metadata operation");
        assert_eq!(
            metadata["plan_links_surface_hash"],
            json!("ebb270cd23b5d2fb3762e8d9a2949d31b7d1ecdf088761bfa2535629e6bf9843")
        );
        assert_eq!(metadata["plan_links_changed_count_to_prev"], json!(0));
    }
}
