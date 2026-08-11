use crate::runtime::{RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT};
use ait_core::json_support::{json, JsonValue};
use ait_core::line_store::{line_by_name_with_line_store, LineStore};
use ait_core::snapshot_store::{snapshot_exists_with_snapshot_store, SnapshotStore};
use ait_core::tag_store::{
    create_tag_with_store, delete_tag_with_store, list_tags_with_store, new_tag_record,
    tag_by_name_with_store, FilesystemTagStore, TagRecord,
};
use chrono::{SecondsFormat, Utc};

#[derive(Clone, Debug)]
pub struct TagCreateRequest {
    pub name: String,
    pub snapshot_id: Option<String>,
    pub message: String,
    pub force: bool,
}

pub fn tag_create(repo: &RepoRuntime, request: TagCreateRequest) -> Result<JsonValue, String> {
    let (snapshot_id, source_line) = match normalized_text(request.snapshot_id.as_deref()) {
        Some(snapshot_id) => {
            require_snapshot_exists(repo, &snapshot_id)?;
            (snapshot_id, None)
        }
        None => {
            let (line_name, snapshot_id) = current_line_head_snapshot_id(repo)?;
            require_snapshot_exists(repo, &snapshot_id)?;
            (snapshot_id, Some(line_name))
        }
    };
    let created_at = current_timestamp();
    let record = new_tag_record(&request.name, &snapshot_id, &request.message, &created_at)?;
    let tag_store = tag_store(repo)?;
    let persisted = create_tag_with_store(&tag_store, &record, request.force)?;
    let mut payload = tag_record_payload(&persisted);
    if let Some(line_name) = source_line {
        payload["source_line"] = JsonValue::String(line_name);
    }
    Ok(payload)
}

pub fn tag_list(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let tag_store = tag_store(repo)?;
    Ok(JsonValue::Array(
        list_tags_with_store(&tag_store)?
            .into_iter()
            .map(|record| tag_record_payload(&record))
            .collect(),
    ))
}

pub fn tag_show(repo: &RepoRuntime, name: &str) -> Result<JsonValue, String> {
    let tag_store = tag_store(repo)?;
    let record = tag_by_name_with_store(&tag_store, name)?
        .ok_or_else(|| format!("Unknown tag: {}", name.trim()))?;
    Ok(tag_record_payload(&record))
}

pub fn tag_delete(repo: &RepoRuntime, name: &str) -> Result<JsonValue, String> {
    let tag_store = tag_store(repo)?;
    let record = delete_tag_with_store(&tag_store, name)?
        .ok_or_else(|| format!("Unknown tag: {}", name.trim()))?;
    let mut payload = tag_record_payload(&record);
    payload["deleted"] = JsonValue::Bool(true);
    Ok(payload)
}

pub fn resolve_snapshot_ref(repo: &RepoRuntime, value: &str) -> Result<String, String> {
    let value =
        normalized_text(Some(value)).ok_or_else(|| "snapshot ref must not be empty".to_string())?;
    let snapshot_store = snapshot_store(repo)?;
    let direct_lookup_error = match snapshot_exists_with_snapshot_store(&snapshot_store, &value) {
        Ok(true) => return Ok(value),
        Ok(false) => None,
        Err(error) => Some(error),
    };
    let tag_store = tag_store(repo)?;
    let Some(tag) = tag_by_name_with_store(&tag_store, &value)? else {
        if let Some(error) = direct_lookup_error {
            return Err(error);
        }
        return Err(format!("Unknown snapshot or tag: {value}"));
    };
    if !snapshot_exists_with_snapshot_store(&snapshot_store, &tag.snapshot_id)? {
        return Err(format!(
            "Tag {value} points to unavailable snapshot {}.",
            tag.snapshot_id
        ));
    }
    Ok(tag.snapshot_id)
}

fn tag_record_payload(record: &TagRecord) -> JsonValue {
    json!({
        "schema": record.schema,
        "name": record.name,
        "snapshot_id": record.snapshot_id,
        "message": record.message,
        "created_at": record.created_at,
    })
}

fn current_line_head_snapshot_id(repo: &RepoRuntime) -> Result<(String, String), String> {
    let line_name = repo.current_line_name()?;
    let store = line_store(repo)?;
    let line = line_by_name_with_line_store(&store, &line_name)?
        .ok_or_else(|| format!("Unknown current line: {line_name}"))?;
    let snapshot_id = line
        .head_snapshot_id
        .ok_or_else(|| format!("Current line {line_name} does not have a head snapshot."))?;
    Ok((line_name, snapshot_id))
}

fn require_snapshot_exists(repo: &RepoRuntime, snapshot_id: &str) -> Result<(), String> {
    let snapshot_id = normalized_text(Some(snapshot_id))
        .ok_or_else(|| "snapshot_id must not be empty".to_string())?;
    let store = snapshot_store(repo)?;
    if snapshot_exists_with_snapshot_store(&store, &snapshot_id)? {
        Ok(())
    } else {
        Err(format!("Unknown snapshot: {snapshot_id}"))
    }
}

fn tag_store(repo: &RepoRuntime) -> Result<FilesystemTagStore, String> {
    FilesystemTagStore::new(repo.authoritative_repo_root().to_string_lossy().as_ref())
}

fn line_store(repo: &RepoRuntime) -> Result<impl LineStore, String> {
    repo.line_store()
}

fn snapshot_store(repo: &RepoRuntime) -> Result<impl SnapshotStore, String> {
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)
}

fn normalized_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn current_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
