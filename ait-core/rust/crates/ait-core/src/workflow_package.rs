//! Released workflow-package/v1 transient archive. Physical DB records and
//! indexes never enter this format; row encodings reuse the bulk transport.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::repository_pack_json::{ZstdBulkCommitRequest, ZstdBulkCommitRequestJson};

mod abandonment;
mod completion;
mod execution;
pub use abandonment::{PackageAbandonment, PackageAbandonmentState};
pub use completion::{
    lookup_package_completion, PackageChangeMapping, PackageChangeMappingExpectation,
    PackageCompletion, PackageCompletionOutcome, PackageMapping, PackageMappingExpectation,
    PackageReadResource, PackageSnapshotOwnerExpectation,
};
pub use execution::{
    package_response_data, PackageAvailability, PackageCiState, PackageExecution,
    PreparedPackageExecution,
};

pub const WORKFLOW_PACKAGE_CONTRACT: &str = "workflow-package/v1";
pub const WORKFLOW_PACKAGE_MEDIA_TYPE: &str = "application/vnd.ait.workflow-package.v1+zstd";

/// Inspect the leading manifest without extracting files. This does not verify
/// the remaining archive inventory or pack bodies; Server validation still owns
/// full package admission. The inspection has a 16 MiB manifest bound.
pub fn inspect_workflow_package_manifest(bytes: &[u8]) -> Result<Value, String> {
    const LIMIT: u64 = 16 * 1024 * 1024;
    let error = |error| format!("Workflow package manifest: {error}");
    let mut decoder = zstd::stream::read::Decoder::new(bytes).map_err(error)?;
    decoder.window_log_max(27).map_err(error)?;
    let mut archive = tar::Archive::new(decoder.single_frame());
    let mut entries = archive.entries().map_err(error)?;
    let mut entry = entries
        .next()
        .ok_or("Workflow package manifest is missing")?
        .map_err(error)?;
    if entry.path_bytes().as_ref() != b"manifest.json" || !entry.header().entry_type().is_file() {
        return Err("Workflow package must begin with its regular manifest.json".into());
    }
    if entry.size() == 0 || entry.size() > LIMIT {
        return Err("Workflow package manifest size is outside inspection bounds".into());
    }
    let mut body = Vec::new();
    entry
        .by_ref()
        .take(LIMIT + 1)
        .read_to_end(&mut body)
        .map_err(error)?;
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    if value["contract"].as_str() != Some(WORKFLOW_PACKAGE_CONTRACT) {
        return Err("Unsupported workflow package manifest contract".into());
    }
    Ok(value)
}

#[derive(Clone, Debug)]
pub struct WorkflowPackageManifest {
    pub repository_index: u32,
    pub target_line: String,
    pub base_snapshot_id: String,
    pub revision_snapshot_id: String,
    pub author_mode: String,
    pub summary: Option<String>,
    pub history_entries: Vec<Value>,
    pub content: ZstdBulkCommitRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedWorkflowPackage {
    pub bytes: Vec<u8>,
    pub package_hash: String,
}

impl WorkflowPackageManifest {
    pub fn to_value(&self) -> Result<Value, String> {
        for (field, value) in [
            ("target_line", &self.target_line),
            ("author_mode", &self.author_mode),
        ] {
            if value.is_empty() || value.trim() != value {
                return Err(format!("Workflow package requires exact {field}"));
            }
        }
        for id in [&self.base_snapshot_id, &self.revision_snapshot_id] {
            if !id.is_ascii()
                || id.trim() != id
                || crate::content_binary_db::snapshot_hash48_from_id(id).is_err()
            {
                return Err("Workflow package requires existing Snapshot ID syntax".into());
            }
        }
        validate_package_source_count(self.history_entries.len())?;
        validate_history(&self.history_entries)?;
        let mut content = self.content.clone();
        if content.contract.is_some()
            || content.generation_key.is_some()
            || content.line_update.is_some()
            || content.capture_binding.is_some()
        {
            return Err("Workflow package content cannot publish metadata or carry capture/generation state".into());
        }
        content.object_packs = unique_rows(content.object_packs, |row| &row.pack_id)?;
        content.tree_packs = unique_rows(content.tree_packs, |row| &row.pack_id)?;
        content.blob_locators = unique_rows(content.blob_locators, |row| &row.blob_id)?;
        content.tree_locators = unique_rows(content.tree_locators, |row| &row.tree_id)?;
        content.snapshots = unique_rows(content.snapshots, |row| &row.snapshot_id)?;
        let mut ordered = Vec::with_capacity(content.snapshots.len());
        let mut remaining = content
            .snapshots
            .into_iter()
            .map(|row| (row.snapshot_id.clone(), row))
            .collect::<BTreeMap<_, _>>();
        while !remaining.is_empty() {
            let next = remaining
                .iter()
                .find(|(_, row)| {
                    row.parent_snapshot_ids
                        .iter()
                        .all(|parent| !remaining.contains_key(parent))
                })
                .map(|(id, _)| id.clone())
                .ok_or("Workflow package Snapshot ancestry contains a cycle")?;
            ordered.push(
                remaining
                    .remove(&next)
                    .expect("selected remaining Snapshot"),
            );
        }
        content.snapshots = ordered;
        let graph = content
            .snapshots
            .iter()
            .map(|row| (row.snapshot_id.as_str(), row.parent_snapshot_ids.as_slice()))
            .collect::<BTreeMap<_, _>>();
        let mut previous = None;
        for entry in &self.history_entries {
            let start = entry["pre_land_target_snapshot_id"]
                .as_str()
                .expect("validated receipt base");
            let end = entry["landed_snapshot_id"]
                .as_str()
                .expect("validated receipt revision");
            if let Some(previous) = previous {
                require_package_ancestry(&graph, previous, start)?;
            }
            require_package_ancestry(&graph, start, end)?;
            previous = Some(end);
            for change in entry["changes"].as_array().expect("validated Changes") {
                if !graph.contains_key(
                    change["change"]["fork_snapshot_id"]
                        .as_str()
                        .expect("validated fork"),
                ) {
                    return Err("Package is missing an original Change fork Snapshot".into());
                }
            }
        }
        require_package_ancestry(&graph, &self.base_snapshot_id, &self.revision_snapshot_id)?;
        let mut history = self.history_entries.clone();
        for entry in &mut history {
            let snapshots = entry["snapshots"]
                .as_array_mut()
                .expect("validated Snapshot inventory");
            let mut remaining = snapshots
                .iter()
                .map(|row| {
                    row["snapshot_id"]
                        .as_str()
                        .expect("validated Snapshot")
                        .to_string()
                })
                .collect::<BTreeSet<_>>();
            if remaining.iter().any(|id| !graph.contains_key(id.as_str())) {
                return Err("Package is missing governed Snapshot content".into());
            }
            // Each array has its own canonical domain. Parents outside this
            // entry do not delay its independent roots as global content can.
            let mut order = BTreeMap::new();
            while !remaining.is_empty() {
                let next = remaining
                    .iter()
                    .find(|id| {
                        graph[id.as_str()]
                            .iter()
                            .all(|parent| !remaining.contains(parent))
                    })
                    .cloned()
                    .ok_or("Workflow package owned Snapshot ancestry contains a cycle")?;
                order.insert(next.clone(), order.len());
                remaining.remove(&next);
            }
            snapshots
                .sort_by_key(|row| order[row["snapshot_id"].as_str().expect("validated Snapshot")]);
        }
        let mut content_value = ZstdBulkCommitRequestJson::stateless().encode_value(&content)?;
        content_value["capture_binding"] = Value::Null;
        let mut manifest = json!({
            "contract":WORKFLOW_PACKAGE_CONTRACT,
            "repository_index":self.repository_index,
            "target_line":self.target_line,
            "base_snapshot_id":self.base_snapshot_id,
            "revision_snapshot_id":self.revision_snapshot_id,
            "author_mode":self.author_mode,
            "history_entries":history,
            "content":content_value,
        });
        if let Some(summary) = &self.summary {
            if summary.trim().is_empty() || summary.len() > usize::from(u16::MAX) {
                return Err("Package summary must contain 1..65535 UTF-8 bytes".into());
            }
            manifest["summary"] = Value::String(summary.clone());
        }
        canonical_json(&manifest)
    }

    pub fn encode(
        &self,
        objects: &BTreeMap<String, Vec<u8>>,
        trees: &BTreeMap<String, Vec<u8>>,
    ) -> Result<EncodedWorkflowPackage, String> {
        validate_pack_names(objects.keys(), true)?;
        validate_pack_names(trees.keys(), false)?;
        let value = self.to_value()?;
        for (field, supplied) in [("object_packs", objects), ("tree_packs", trees)] {
            let declared = value["content"][field]
                .as_array()
                .ok_or("Package pack rows missing")?
                .iter()
                .map(|row| row["pack_id"].as_str().ok_or("Package pack ID missing"))
                .collect::<Result<BTreeSet<_>, _>>()?;
            if declared != supplied.keys().map(String::as_str).collect() {
                return Err(format!(
                    "Workflow package {field} bytes must exactly match declared pack IDs"
                ));
            }
        }
        let manifest = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
        let mut archive = tar::Builder::new(Vec::new());
        append_regular(&mut archive, "manifest.json", &manifest)?;
        for (prefix, packs) in [("objects", objects), ("trees", trees)] {
            for (id, bytes) in packs {
                append_regular(&mut archive, &format!("{prefix}/{id}"), bytes)?;
            }
        }
        let tar_bytes = archive.into_inner().map_err(|error| error.to_string())?;
        let mut encoder =
            zstd::stream::write::Encoder::new(Vec::new(), 3).map_err(|error| error.to_string())?;
        encoder
            .include_contentsize(true)
            .map_err(|error| error.to_string())?;
        encoder
            .include_checksum(false)
            .map_err(|error| error.to_string())?;
        encoder
            .set_pledged_src_size(Some(tar_bytes.len() as u64))
            .map_err(|error| error.to_string())?;
        encoder
            .write_all(&tar_bytes)
            .map_err(|error| error.to_string())?;
        let bytes = encoder.finish().map_err(|error| error.to_string())?;
        let package_hash = format!("{:x}", Sha256::digest(&bytes));
        Ok(EncodedWorkflowPackage {
            bytes,
            package_hash,
        })
    }
}

/// Package capacity is independent of Repository Task-key capacity.
pub fn validate_package_source_count(count: usize) -> Result<(), String> {
    if !(1..=64).contains(&count) {
        return Err(
            "Workflow package requires 1..64 source Tasks; do not split or omit history".into(),
        );
    }
    Ok(())
}

fn unique_rows<T: Eq>(rows: Vec<T>, key: impl Fn(&T) -> &String) -> Result<Vec<T>, String> {
    let mut ordered = BTreeMap::new();
    for row in rows {
        let id = key(&row).clone();
        if let Some(previous) = ordered.get(&id) {
            if previous != &row {
                return Err(format!("Conflicting workflow package rows for {id}"));
            }
        } else {
            ordered.insert(id, row);
        }
    }
    Ok(ordered.into_values().collect())
}

fn require_package_ancestry(
    graph: &BTreeMap<&str, &[String]>,
    ancestor: &str,
    descendant: &str,
) -> Result<(), String> {
    if !graph.contains_key(ancestor) || !graph.contains_key(descendant) {
        return Err("Package is missing an original receipt or tested Snapshot endpoint".into());
    }
    let mut pending = vec![descendant];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if id == ancestor {
            return Ok(());
        }
        if seen.insert(id) {
            if let Some(parents) = graph.get(id) {
                pending.extend(parents.iter().map(String::as_str));
            }
        }
    }
    Err("Package receipt or tested Snapshot chain cannot resolve its exact ancestry".into())
}

fn is_package_change_id(id: &str) -> bool {
    id.strip_prefix("C-")
        .and_then(|value| value.parse::<u8>().ok())
        .is_some_and(|ordinal| (1..=64).contains(&ordinal) && id == format!("C-{ordinal:02}"))
}

fn validate_history(entries: &[Value]) -> Result<(), String> {
    fn exact<'a>(
        value: &'a Value,
        keys: &[&str],
    ) -> Result<&'a serde_json::Map<String, Value>, String> {
        let object = value
            .as_object()
            .ok_or("Package history row must be an object")?;
        if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
            return Err("Package history row has missing, legacy or unknown fields".into());
        }
        Ok(object)
    }
    fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
        value[key]
            .as_str()
            .filter(|text| !text.is_empty() && text.trim() == *text)
            .ok_or_else(|| format!("Package history requires exact {key}"))
    }
    fn nullable(value: &Value, key: &str) -> Result<(), String> {
        if value[key].is_null() {
            Ok(())
        } else {
            text(value, key).map(|_| ())
        }
    }
    let mut tasks = BTreeSet::new();
    let mut snapshots = BTreeSet::new();
    let mut previous_time = 0;
    for entry in entries {
        exact(
            entry,
            &[
                "local_task_id",
                "expected_remote_task_id",
                "task",
                "changes",
                "pre_land_target_snapshot_id",
                "landed_snapshot_id",
                "landed_at_s",
                "snapshots",
            ],
        )?;
        let task = text(entry, "local_task_id")?;
        nullable(entry, "expected_remote_task_id")?;
        if !tasks.insert(task) {
            return Err("Package history repeats a Task".into());
        }
        text(entry, "pre_land_target_snapshot_id")?;
        text(entry, "landed_snapshot_id")?;
        let landed_at = entry["landed_at_s"]
            .as_u64()
            .filter(|time| *time > 0 && *time >= previous_time)
            .ok_or("Package history requires real ordered successful receipt times")?;
        previous_time = landed_at;
        let mut changes = BTreeSet::new();
        let mut previous_ordinal = String::new();
        for change in entry["changes"]
            .as_array()
            .ok_or("Package history requires changes array")?
        {
            exact(
                change,
                &[
                    "local_change_id",
                    "local_change_ref",
                    "expected_remote_change_ref",
                    "change",
                ],
            )?;
            let id = text(change, "local_change_id")?;
            if !is_package_change_id(id)
                || id <= previous_ordinal.as_str()
                || text(change, "local_change_ref")? != format!("{task}/{id}")
                || !changes.insert(text(change, "local_change_ref")?)
            {
                return Err(
                    "Package Change history is reordered, duplicated or differently owned".into(),
                );
            }
            previous_ordinal = id.to_string();
            nullable(change, "expected_remote_change_ref")?;
            exact(
                &change["change"],
                &["title", "base_line", "fork_snapshot_id"],
            )?;
            for field in ["title", "base_line", "fork_snapshot_id"] {
                text(&change["change"], field)?;
            }
        }
        let source = entry["snapshots"]
            .as_array()
            .filter(|rows| !rows.is_empty())
            .ok_or("Package history requires owned Snapshot inventory")?;
        for snapshot in source {
            exact(
                snapshot,
                &["snapshot_id", "created_at_s", "local_change_ref"],
            )?;
            if !snapshots.insert(text(snapshot, "snapshot_id")?)
                || snapshot["created_at_s"].as_u64().is_none()
            {
                return Err("Package Snapshot history is duplicated or has no domain time".into());
            }
            nullable(snapshot, "local_change_ref")?;
            if let Some(reference) = snapshot["local_change_ref"].as_str() {
                if !changes.contains(reference) {
                    return Err(
                        "Package Snapshot Change owner is absent from its Task history".into(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn canonical_json(value: &Value) -> Result<Value, String> {
    match value {
        Value::Object(object) => {
            let sorted = object.iter().collect::<BTreeMap<_, _>>();
            sorted
                .into_iter()
                .map(|(key, value)| Ok((key.clone(), canonical_json(value)?)))
                .collect::<Result<serde_json::Map<_, _>, String>>()
                .map(Value::Object)
        }
        Value::Array(values) => values
            .iter()
            .map(canonical_json)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Number(number) if !number.is_i64() && !number.is_u64() => {
            Err("Workflow package numbers must be integers".into())
        }
        _ => Ok(value.clone()),
    }
}

fn validate_pack_names<'a>(
    names: impl Iterator<Item = &'a String>,
    objects: bool,
) -> Result<(), String> {
    for name in names {
        if !name.is_ascii() || name.trim() != name {
            return Err("Invalid workflow package pack filename".into());
        }
        if objects {
            crate::content_binary_db::object_pack_hash48_from_id(name)
        } else {
            crate::content_binary_db::tree_pack_hash48_from_id(name)
        }
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn append_regular(
    archive: &mut tar::Builder<Vec<u8>>,
    path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let mut header = tar::Header::new_ustar();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(bytes.len() as u64);
    header.set_cksum();
    archive
        .append_data(&mut header, path, Cursor::new(bytes))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
