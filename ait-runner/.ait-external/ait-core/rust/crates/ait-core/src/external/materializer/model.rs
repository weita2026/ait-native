use crate::file_io::{FileIoStore, FilesystemFileIoStore};
use std::path::Path;

use crate::json_support::{json, JsonValue};

use crate::external::lockfile::{ExternalLockNode, ExternalLockfile};
use crate::external::{ExternalError, ExternalResult};
use crate::json_support::{JsonCodec, JsonEncodeOptions};

pub const EXTERNAL_MATERIALIZER_MARKER: &str = ".ait-external-marker.json";
pub const EXTERNAL_MATERIALIZER_MARKER_FORMAT: &str = "ait.external.materialized";
pub const EXTERNAL_MATERIALIZER_MARKER_VERSION: u64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalMaterializerMarkerFileEntry {
    pub path: String,
    pub sha256: String,
}

impl ExternalMaterializerMarkerFileEntry {
    pub fn new(path: impl Into<String>, sha256: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            sha256: sha256.into(),
        }
    }

    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "path": self.path,
            "sha256": self.sha256,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalMaterializerMarkerV3 {
    pub name: String,
    pub repo_name: String,
    pub repository_index: u32,
    pub remote: String,
    pub line: String,
    pub snapshot: String,
    pub parent_path: String,
    pub materialize_to: String,
    pub files: Vec<ExternalMaterializerMarkerFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalMaterializerMarkerRecord {
    V3(ExternalMaterializerMarkerV3),
    Legacy {
        format: Option<String>,
        version: Option<u64>,
        snapshot: Option<String>,
    },
}

impl ExternalMaterializerMarkerRecord {
    pub fn snapshot(&self) -> Option<&str> {
        match self {
            Self::V3(marker) => Some(marker.snapshot.as_str()),
            Self::Legacy { snapshot, .. } => snapshot.as_deref(),
        }
    }
}

pub struct ExternalMaterializerMarkerJson<S> {
    store: S,
}

impl<S> ExternalMaterializerMarkerJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl ExternalMaterializerMarkerJson<FilesystemFileIoStore> {
    pub fn filesystem() -> Self {
        Self::new(FilesystemFileIoStore)
    }
}

impl<S> ExternalMaterializerMarkerJson<S>
where
    S: FileIoStore,
{
    pub fn read_marker(&self, path: &Path) -> ExternalResult<ExternalMaterializerMarkerRecord> {
        let text = self.store.read_to_string(path).map_err(|err| {
            ExternalError::with_code(
                "external_status_marker",
                format!("failed to read external materialization marker: {err}"),
            )
        })?;
        let marker =
            JsonCodec::parse_value(&text, "external materialization marker").map_err(|err| {
                let detail = err
                    .message()
                    .strip_prefix("Invalid external materialization marker JSON: ")
                    .unwrap_or_else(|| err.message());
                ExternalError::with_code(
                    "external_status_marker",
                    format!("failed to parse external materialization marker: {detail}"),
                )
            })?;
        parse_external_materializer_marker(&marker)
    }

    pub fn write_marker(
        &self,
        path: &Path,
        node: &ExternalLockNode,
        files: &[ExternalMaterializerMarkerFileEntry],
    ) -> ExternalResult<()> {
        let text = JsonCodec::encode_value(
            &external_materializer_marker_value(node, files),
            JsonEncodeOptions::pretty(),
        )
        .map_err(|err| {
            let detail = err
                .message()
                .strip_prefix("Failed to encode JSON: ")
                .unwrap_or_else(|| err.message());
            ExternalError::with_code(
                "external_materializer_marker",
                format!("failed to encode external materialization marker: {detail}"),
            )
        })?;
        self.store.write_string(path, &text).map_err(|err| {
            ExternalError::with_code(
                "external_materializer_marker",
                format!("failed to write external materialization marker: {err}"),
            )
        })
    }
}

fn external_materializer_marker_value(
    node: &ExternalLockNode,
    files: &[ExternalMaterializerMarkerFileEntry],
) -> JsonValue {
    json!({
        "format": EXTERNAL_MATERIALIZER_MARKER_FORMAT,
        "version": EXTERNAL_MATERIALIZER_MARKER_VERSION,
        "name": node.name,
        "repo_name": node.repo_name,
        "repository_index": node.repository_index,
        "remote": node.remote,
        "line": node.line,
        "snapshot": node.snapshot,
        "parent_path": node.parent_path,
        "materialize_to": node.materialize_to,
        "files": files.iter().map(ExternalMaterializerMarkerFileEntry::to_json_value).collect::<Vec<_>>(),
    })
}

fn parse_external_materializer_marker(
    marker: &JsonValue,
) -> ExternalResult<ExternalMaterializerMarkerRecord> {
    let format = marker
        .get("format")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let version = marker.get("version").and_then(JsonValue::as_u64);
    let snapshot = marker
        .get("snapshot")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    if format.as_deref() == Some(EXTERNAL_MATERIALIZER_MARKER_FORMAT)
        && version == Some(EXTERNAL_MATERIALIZER_MARKER_VERSION)
    {
        return Ok(ExternalMaterializerMarkerRecord::V3(
            ExternalMaterializerMarkerV3 {
                name: required_marker_string_field(marker, "name")?,
                repo_name: required_marker_string_field(marker, "repo_name")?,
                repository_index: required_marker_u32_field(marker, "repository_index")?,
                remote: required_marker_string_field(marker, "remote")?,
                line: required_marker_string_field(marker, "line")?,
                snapshot: required_marker_string_field(marker, "snapshot")?,
                parent_path: required_marker_string_field(marker, "parent_path")?,
                materialize_to: required_marker_string_field(marker, "materialize_to")?,
                files: parse_marker_file_entries(marker)?,
            },
        ));
    }
    Ok(ExternalMaterializerMarkerRecord::Legacy {
        format,
        version,
        snapshot,
    })
}

fn required_marker_u32_field(marker: &JsonValue, field: &str) -> ExternalResult<u32> {
    let value = marker
        .get(field)
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    value.ok_or_else(|| {
        ExternalError::with_code(
            "external_status_marker",
            format!(
                "failed to parse external materialization marker: missing unsigned 32-bit integer field `{field}`"
            ),
        )
    })
}

fn required_marker_string_field(marker: &JsonValue, field: &str) -> ExternalResult<String> {
    marker
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            ExternalError::with_code(
                "external_status_marker",
                format!(
                    "failed to parse external materialization marker: missing string field `{field}`"
                ),
            )
        })
}

fn parse_marker_file_entries(
    marker: &JsonValue,
) -> ExternalResult<Vec<ExternalMaterializerMarkerFileEntry>> {
    let files = marker
        .get("files")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            ExternalError::with_code(
                "external_status_marker",
                "failed to parse external materialization marker: missing file manifest"
                    .to_string(),
            )
        })?;
    let mut entries = Vec::with_capacity(files.len());
    for file in files {
        let path = file
            .get("path")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                ExternalError::with_code(
                    "external_status_marker",
                    "failed to parse external materialization marker: file entry is missing `path`"
                        .to_string(),
                )
            })?;
        let sha256 = file
            .get("sha256")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                ExternalError::with_code(
                    "external_status_marker",
                    "failed to parse external materialization marker: file entry is missing `sha256`"
                        .to_string(),
                )
            })?;
        entries.push(ExternalMaterializerMarkerFileEntry::new(path, sha256));
    }
    Ok(entries)
}

pub trait ExternalContentSource {
    fn materialize_content(
        &self,
        node: &ExternalLockNode,
        destination: &Path,
    ) -> ExternalResult<()>;
}

pub trait ExternalMaterializer {
    fn materialize_lockfile(
        &self,
        lockfile: &ExternalLockfile,
        options: &ExternalMaterializationOptions,
    ) -> ExternalResult<ExternalMaterializationReport>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalMaterializationOptions {
    pub no_recursive: bool,
    pub locked: bool,
    pub release_ready: bool,
    pub local_link_overrides: Vec<ExternalLocalLinkOverride>,
}

impl ExternalMaterializationOptions {
    pub fn recursive() -> Self {
        Self::default()
    }

    pub fn no_recursive() -> Self {
        Self {
            no_recursive: true,
            ..Self::default()
        }
    }

    pub fn with_locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    pub fn with_release_ready(mut self, release_ready: bool) -> Self {
        self.release_ready = release_ready;
        self
    }

    pub fn with_local_link_override(
        mut self,
        name: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        self.local_link_overrides.push(ExternalLocalLinkOverride {
            name: name.into(),
            path: path.into(),
        });
        self
    }

    pub(crate) fn reject_forbidden_local_links(&self) -> ExternalResult<()> {
        if (self.locked || self.release_ready) && !self.local_link_overrides.is_empty() {
            let names = self
                .local_link_overrides
                .iter()
                .map(|link| format!("{} -> {}", link.name, link.path))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ExternalError::with_code(
                "external_local_link_forbidden",
                format!(
                    "local external links are not accepted for locked or release-ready materialization: {names}"
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLocalLinkOverride {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalMaterializationReport {
    pub entries: Vec<ExternalMaterializationEntry>,
}

impl ExternalMaterializationReport {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "entries": self.entries.iter().map(ExternalMaterializationEntry::to_json_value).collect::<Vec<_>>(),
            "summary": {
                "entry_count": self.entries.len(),
                "materialized_count": self.entries.iter().filter(|entry| entry.state == ExternalMaterializationState::Materialized).count(),
                "skipped_count": self.entries.iter().filter(|entry| entry.state != ExternalMaterializationState::Materialized).count(),
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalMaterializationEntry {
    pub name: String,
    pub repo_name: String,
    pub repository_index: u32,
    pub snapshot: String,
    pub parent_path: String,
    pub materialize_to: String,
    pub state: ExternalMaterializationState,
}

impl ExternalMaterializationEntry {
    pub(crate) fn from_node(node: &ExternalLockNode, state: ExternalMaterializationState) -> Self {
        Self {
            name: node.name.clone(),
            repo_name: node.repo_name.clone(),
            repository_index: node.repository_index,
            snapshot: node.snapshot.clone(),
            parent_path: node.parent_path.clone(),
            materialize_to: node.materialize_to.clone(),
            state,
        }
    }

    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "name": self.name,
            "repo_name": self.repo_name,
            "repository_index": self.repository_index,
            "snapshot": self.snapshot,
            "parent_path": self.parent_path,
            "materialize_to": self.materialize_to,
            "state": self.state.as_str(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalMaterializationState {
    Materialized,
    SkippedNoRecursive,
    SkippedLocalLink,
}

impl ExternalMaterializationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Materialized => "materialized",
            Self::SkippedNoRecursive => "skipped_no_recursive",
            Self::SkippedLocalLink => "skipped_local_link",
        }
    }
}
