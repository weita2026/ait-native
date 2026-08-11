use super::helpers::{optional_text, required_text_from_object};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};

const SUITE_CATALOG_CONTRACT: &str = "ait.server.patchset_ci.suite_catalog.v1";

pub fn patchset_ci_suite_catalog_json(request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "patchset-ci suite-catalog payload must be a JSON object.".to_string())?;
    let files = snapshot_file_payloads(payload)?;
    let mut suites_by_id = BTreeMap::new();
    let mut catalog_paths = BTreeSet::new();

    if files.contains_key("ci/patch_ci.json") {
        ingest_catalog_path(
            &files,
            "ci/patch_ci.json",
            &mut suites_by_id,
            &mut catalog_paths,
        )?;
    }

    let suites = suites_by_id.into_values().collect::<Vec<_>>();
    Ok(json!({
        "contract": SUITE_CATALOG_CONTRACT,
        "suite_count": suites.len(),
        "catalog_paths": catalog_paths.into_iter().collect::<Vec<_>>(),
        "suites": suites,
    }))
}

fn snapshot_file_payloads(
    payload: &JsonMap<String, JsonValue>,
) -> Result<BTreeMap<String, String>, String> {
    let values = payload
        .get("snapshot_files")
        .or_else(|| payload.get("materialized_files"))
        .or_else(|| payload.get("files"))
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            "patchset-ci suite-catalog payload requires `snapshot_files` or `materialized_files`."
                .to_string()
        })?;
    let mut files = BTreeMap::new();
    for (index, value) in values.iter().enumerate() {
        let object = value
            .as_object()
            .ok_or_else(|| format!("snapshot_files[{index}] must be an object."))?;
        let path = required_text_from_object(object, "path")?;
        let normalized_path = normalized_snapshot_path(&path);
        if normalized_path.is_empty() {
            continue;
        }
        let content = snapshot_file_content(object, index)?;
        files.insert(normalized_path, content);
    }
    Ok(files)
}

fn snapshot_file_content(
    object: &JsonMap<String, JsonValue>,
    index: usize,
) -> Result<String, String> {
    let retired_binary_key = ["content", "base64"].join("_");
    if object.contains_key(retired_binary_key.as_str()) {
        return Err(format!(
            "snapshot_files[{index}] must use text content or pack-backed materialization."
        ));
    }
    if let Some(content) = object
        .get("content")
        .or_else(|| object.get("content_utf8"))
        .and_then(JsonValue::as_str)
    {
        return Ok(content.to_string());
    }
    Ok(String::new())
}

fn normalized_snapshot_path(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn snapshot_json_file(
    files: &BTreeMap<String, String>,
    path: &str,
) -> Result<Option<JsonValue>, String> {
    let Some(content) = files.get(path) else {
        return Ok(None);
    };
    serde_json::from_str(content)
        .map(Some)
        .map_err(|exc| format!("CI suite catalog `{path}` is invalid JSON: {exc}"))
}

fn ingest_catalog_path(
    files: &BTreeMap<String, String>,
    path: &str,
    suites_by_id: &mut BTreeMap<String, JsonValue>,
    catalog_paths: &mut BTreeSet<String>,
) -> Result<(), String> {
    let Some(value) = snapshot_json_file(files, path)? else {
        return Ok(());
    };
    let suites = suite_values_from_catalog_value(value, path)?;
    if suites.is_empty() {
        return Ok(());
    }
    catalog_paths.insert(path.to_string());
    for mut suite in suites {
        let suite_id = suite
            .get("suite_id")
            .and_then(optional_text)
            .ok_or_else(|| format!("CI suite manifest `{path}` requires `suite_id`."))?;
        if suite_id.is_empty() {
            return Err(format!("CI suite manifest `{path}` requires `suite_id`."));
        }
        suite.insert(
            "_artifact_path".to_string(),
            JsonValue::String(path.to_string()),
        );
        suites_by_id.insert(suite_id, JsonValue::Object(suite));
    }
    Ok(())
}

fn suite_values_from_catalog_value(
    value: JsonValue,
    path: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    match value {
        JsonValue::Array(values) => suite_values_from_array(values, path),
        JsonValue::Object(mut object) => {
            if let Some(JsonValue::Array(values)) = object.remove("suites") {
                suite_values_from_array(values, path)
            } else if object.contains_key("suite_id") {
                Ok(vec![object])
            } else {
                Ok(Vec::new())
            }
        }
        _ => Ok(Vec::new()),
    }
}

fn suite_values_from_array(
    values: Vec<JsonValue>,
    path: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    let mut suites = Vec::new();
    for (index, value) in values.into_iter().enumerate() {
        let JsonValue::Object(object) = value else {
            return Err(format!(
                "CI suite catalog `{path}` suite[{index}] must be an object."
            ));
        };
        suites.push(object);
    }
    Ok(suites)
}
