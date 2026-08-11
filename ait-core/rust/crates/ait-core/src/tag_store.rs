use crate::json_support::{JsonCodec, JsonEncodeOptions};
use crate::ref_names::encode_ref_name;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const TAG_RECORD_SCHEMA: u32 = 1;

pub type TagStoreResult<T> = Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRecord {
    pub schema: u32,
    pub name: String,
    pub snapshot_id: String,
    pub message: String,
    pub created_at: String,
}

pub trait TagStore {
    fn create_tag(&self, record: &TagRecord, force: bool) -> TagStoreResult<TagRecord>;
    fn list_tags(&self) -> TagStoreResult<Vec<TagRecord>>;
    fn tag_by_name(&self, name: &str) -> TagStoreResult<Option<TagRecord>>;
    fn delete_tag(&self, name: &str) -> TagStoreResult<Option<TagRecord>>;
}

pub fn create_tag_with_store<S>(
    store: &S,
    record: &TagRecord,
    force: bool,
) -> TagStoreResult<TagRecord>
where
    S: TagStore + ?Sized,
{
    store.create_tag(record, force)
}

pub fn list_tags_with_store<S>(store: &S) -> TagStoreResult<Vec<TagRecord>>
where
    S: TagStore + ?Sized,
{
    store.list_tags()
}

pub fn tag_by_name_with_store<S>(store: &S, name: &str) -> TagStoreResult<Option<TagRecord>>
where
    S: TagStore + ?Sized,
{
    store.tag_by_name(name)
}

pub fn delete_tag_with_store<S>(store: &S, name: &str) -> TagStoreResult<Option<TagRecord>>
where
    S: TagStore + ?Sized,
{
    store.delete_tag(name)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesystemTagStore {
    repo_root: PathBuf,
}

impl FilesystemTagStore {
    pub fn new(repo_root: &str) -> TagStoreResult<Self> {
        if repo_root.trim().is_empty() {
            return Err("repo_root must not be empty.".to_string());
        }
        let repo_root = PathBuf::from(repo_root).canonicalize().map_err(io_error)?;
        Ok(Self { repo_root })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

impl TagStore for FilesystemTagStore {
    fn create_tag(&self, record: &TagRecord, force: bool) -> TagStoreResult<TagRecord> {
        let record = validate_record(record)?;
        let path = tag_record_path(&self.repo_root, &record.name)?;
        if path.exists() && !force {
            return Err(format!(
                "Tag {} already exists. Use --force to replace it.",
                record.name
            ));
        }
        write_tag_record(&path, &record)?;
        read_tag_record(&path)
    }

    fn list_tags(&self) -> TagStoreResult<Vec<TagRecord>> {
        let root = tags_root(&self.repo_root);
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(io_error(err)),
        };
        let mut rows = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            rows.push(read_tag_record(&path)?);
        }
        rows.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.created_at.cmp(&right.created_at))
        });
        Ok(rows)
    }

    fn tag_by_name(&self, name: &str) -> TagStoreResult<Option<TagRecord>> {
        let name = require_non_empty(name, "tag name")?;
        let path = tag_record_path(&self.repo_root, &name)?;
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => read_tag_record(&path).map(Some),
            Ok(_) => Err(format!("Tag path is not a file: {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(io_error(err)),
        }
    }

    fn delete_tag(&self, name: &str) -> TagStoreResult<Option<TagRecord>> {
        let name = require_non_empty(name, "tag name")?;
        let path = tag_record_path(&self.repo_root, &name)?;
        let record = match self.tag_by_name(&name)? {
            Some(record) => record,
            None => return Ok(None),
        };
        fs::remove_file(&path).map_err(io_error)?;
        Ok(Some(record))
    }
}

pub fn new_tag_record(
    name: &str,
    snapshot_id: &str,
    message: &str,
    created_at: &str,
) -> TagStoreResult<TagRecord> {
    validate_record(&TagRecord {
        schema: TAG_RECORD_SCHEMA,
        name: name.to_string(),
        snapshot_id: snapshot_id.to_string(),
        message: message.to_string(),
        created_at: created_at.to_string(),
    })
}

fn validate_record(record: &TagRecord) -> TagStoreResult<TagRecord> {
    if record.schema != TAG_RECORD_SCHEMA {
        return Err(format!("Unsupported tag record schema: {}", record.schema));
    }
    let name = require_non_empty(&record.name, "tag name")?;
    let snapshot_id = require_non_empty(&record.snapshot_id, "snapshot_id")?;
    let message = require_single_line(&record.message, "message")?;
    let created_at = require_non_empty(&record.created_at, "created_at")?;
    Ok(TagRecord {
        schema: TAG_RECORD_SCHEMA,
        name,
        snapshot_id,
        message,
        created_at,
    })
}

fn write_tag_record(path: &Path, record: &TagRecord) -> TagStoreResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let encoded = JsonCodec::encode_serializable_with_error_prefix(
        record,
        JsonEncodeOptions::pretty(),
        "JSON tag store operation failed",
    )
    .map_err(String::from)?;
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, format!("{encoded}\n")).map_err(io_error)?;
    fs::rename(temp_path, path).map_err(io_error)
}

fn read_tag_record(path: &Path) -> TagStoreResult<TagRecord> {
    let text = fs::read_to_string(path).map_err(io_error)?;
    let record = JsonCodec::parse_deserializable_with_error_prefix::<TagRecord>(
        &text,
        &format!("Failed to decode tag record {}", path.display()),
    )
    .map_err(String::from)?;
    validate_record(&record)
}

fn tag_record_path(repo_root: &Path, name: &str) -> TagStoreResult<PathBuf> {
    let name = require_non_empty(name, "tag name")?;
    Ok(tags_root(repo_root).join(format!("{}.json", encode_ref_name(&name))))
}

fn tags_root(repo_root: &Path) -> PathBuf {
    repo_root.join(".ait").join("refs").join("tags")
}

fn require_single_line(value: &str, field: &str) -> TagStoreResult<String> {
    let value = require_non_empty(value, field)?;
    if value.contains('\n') || value.contains('\r') {
        return Err(format!("{field} must be a single line"));
    }
    Ok(value)
}

fn require_non_empty(value: &str, field: &str) -> TagStoreResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    Ok(trimmed.to_string())
}

fn io_error(err: std::io::Error) -> String {
    format!("Filesystem tag store operation failed: {err}")
}

#[cfg(test)]
mod tests;
