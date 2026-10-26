use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use ait_core::content_binary_db::{object_pack_hash48_from_id, tree_pack_hash48_from_id};
use ait_core::server_operational::WorkerJobKey;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::RunnerError;
use crate::control::ExecutionControl;
use crate::materialize::{
    IMPORT_MANIFEST_CONTRACT, MAX_IMPORT_MANIFEST_BYTES, RemoteMaterialization, RemotePackKind,
    RemoteSnapshotProvider, RemoteSnapshotReference, materialize_remote_snapshot,
};

pub const WORKFLOW_PACKAGE_MEDIA_TYPE: &str = "application/vnd.ait.workflow-package.v1+zstd";
pub const MAX_WORKFLOW_PACKAGE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const MAX_PACKAGE_MEMBERS: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowPackageReference {
    pub key: WorkerJobKey,
    pub package_hash: String,
    pub base_snapshot_id: String,
    pub revision_snapshot_id: String,
}

pub(crate) fn materialize_workflow_package(
    provider: &dyn RemoteSnapshotProvider,
    source: &WorkflowPackageReference,
    workspace: &Path,
    package_root: &Path,
    pack_root: &Path,
    control: &ExecutionControl,
) -> Result<RemoteMaterialization, RunnerError> {
    control.check()?;
    fs::create_dir(package_root)
        .map_err(|error| RunnerError::fs("create package root", package_root, error))?;
    let archive = package_root.join("source.zst");
    provider.download_workflow_package(source, &archive, MAX_WORKFLOW_PACKAGE_BYTES)?;
    control.check()?;
    let mut file =
        File::open(&archive).map_err(|error| RunnerError::fs("open package", &archive, error))?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        control.check()?;
        let read = file
            .read(&mut buffer)
            .map_err(|error| RunnerError::fs("hash package", &archive, error))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| invalid("package size overflow"))?;
        if total > MAX_WORKFLOW_PACKAGE_BYTES {
            return Err(invalid("package exceeds download bound"));
        }
        hash.update(&buffer[..read]);
    }
    if format!("{:x}", hash.finalize()) != source.package_hash {
        return Err(invalid(
            "downloaded bytes do not match the claimed package hash",
        ));
    }
    let manifest = unpack(&archive, package_root, source, control)?;
    let root_source = RemoteSnapshotReference {
        repository_index: Some(source.key.repository_index),
        repository_name: "workflow-package".into(),
        legacy_repo_id: None,
        snapshot_id: source.revision_snapshot_id.clone(),
        external_repository_indexes: Default::default(),
    };
    let packaged = PackageProvider {
        delegate: provider,
        root_source: root_source.clone(),
        manifest,
        root: package_root.to_path_buf(),
        control,
    };
    materialize_remote_snapshot(&packaged, &root_source, workspace, pack_root)
}

struct PackageProvider<'a> {
    delegate: &'a dyn RemoteSnapshotProvider,
    root_source: RemoteSnapshotReference,
    manifest: Vec<u8>,
    root: PathBuf,
    control: &'a ExecutionControl,
}

impl RemoteSnapshotProvider for PackageProvider<'_> {
    fn fetch_import_manifest(
        &self,
        source: &RemoteSnapshotReference,
    ) -> Result<Vec<u8>, RunnerError> {
        self.control.check()?;
        if source == &self.root_source {
            return Ok(self.manifest.clone());
        }
        // Declared externals retain their own locked numeric Repository routes.
        self.delegate.fetch_import_manifest(source)
    }

    fn download_pack(
        &self,
        source: &RemoteSnapshotReference,
        kind: RemotePackKind,
        id: &str,
        destination: &Path,
        maximum_bytes: u64,
    ) -> Result<u64, RunnerError> {
        self.control.check()?;
        if source != &self.root_source {
            return self
                .delegate
                .download_pack(source, kind, id, destination, maximum_bytes);
        }
        let prefix = match kind {
            RemotePackKind::Object => "objects",
            RemotePackKind::Tree => "trees",
        };
        validate_member(&format!("{prefix}/{id}"))?;
        let path = self.root.join(prefix).join(id);
        let size = fs::metadata(&path)
            .map_err(|error| RunnerError::fs("read packaged pack", &path, error))?
            .len();
        if size > maximum_bytes {
            return Err(invalid("packaged pack exceeds materialization bound"));
        }
        let mut input = File::open(&path)
            .map_err(|error| RunnerError::fs("open packaged pack", &path, error))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| RunnerError::fs("create materialization pack", destination, error))?;
        let mut copied = 0;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            self.control.check()?;
            let read = input
                .read(&mut buffer)
                .map_err(|error| RunnerError::fs("read packaged pack", &path, error))?;
            if read == 0 {
                break;
            }
            copied += read as u64;
            if copied > maximum_bytes {
                return Err(invalid("packaged pack exceeds materialization bound"));
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| RunnerError::fs("copy packaged pack", destination, error))?;
        }
        Ok(copied)
    }
}

fn unpack(
    archive: &Path,
    root: &Path,
    source: &WorkflowPackageReference,
    control: &ExecutionControl,
) -> Result<Vec<u8>, RunnerError> {
    let mut file =
        File::open(archive).map_err(|error| RunnerError::fs("open package", archive, error))?;
    let mut prefix = [0u8; 18];
    let length = file
        .read(&mut prefix)
        .map_err(|error| RunnerError::fs("read package frame", archive, error))?;
    if length < 5 || prefix[..4] != [0x28, 0xb5, 0x2f, 0xfd] || prefix[4] & 0x1f != 0 {
        return Err(invalid(
            "package requires one ordinary zstd frame without dictionary or checksum",
        ));
    }
    let declared = zstd::zstd_safe::get_frame_content_size(&prefix[..length])
        .map_err(|_| invalid("invalid package frame header"))?
        .ok_or_else(|| invalid("package frame must declare its content size"))?;
    if declared > MAX_ARCHIVE_BYTES {
        return Err(invalid("package archive exceeds extraction bound"));
    }
    let file =
        File::open(archive).map_err(|error| RunnerError::fs("open package", archive, error))?;
    let mut decoder = zstd::stream::read::Decoder::new(file)
        .map_err(decode_error)?
        .single_frame();
    decoder.window_log_max(27).map_err(decode_error)?;
    let mut consumed = 0u64;
    let mut expected = None;
    let mut seen = BTreeSet::new();
    let mut previous = String::new();
    let mut import = None;
    loop {
        control.check()?;
        let mut header = [0u8; 512];
        read_exact(&mut decoder, &mut header, &mut consumed)?;
        if header.iter().all(|byte| *byte == 0) {
            let mut end = [0u8; 512];
            read_exact(&mut decoder, &mut end, &mut consumed)?;
            if end.iter().any(|byte| *byte != 0) {
                return Err(invalid("invalid ustar end blocks"));
            }
            let mut extra = [0u8; 1];
            if decoder.read(&mut extra).map_err(decode_error)? != 0 || consumed != declared {
                return Err(invalid(
                    "package archive has trailing data or incorrect frame size",
                ));
            }
            if decoder.finish().read(&mut extra).map_err(decode_error)? != 0 {
                return Err(invalid(
                    "package has another frame or trailing compressed data",
                ));
            }
            break;
        }
        let (name, size) = header_fields(&header)?;
        if size > MAX_WORKFLOW_PACKAGE_BYTES
            || consumed
                .checked_add(size)
                .is_none_or(|sum| sum > MAX_ARCHIVE_BYTES)
        {
            return Err(invalid("package member exceeds extraction bound"));
        }
        if import.is_none() {
            if name != "manifest.json" || size > MAX_IMPORT_MANIFEST_BYTES as u64 {
                return Err(invalid("package must begin with a bounded manifest.json"));
            }
            let mut bytes = vec![0; size as usize];
            read_exact(&mut decoder, &mut bytes, &mut consumed)?;
            let (manifest, members) = import_manifest(&bytes, source)?;
            import = Some(manifest);
            expected = Some(members);
        } else {
            validate_member(&name)?;
            if name <= previous
                || !expected.as_ref().unwrap().contains(&name)
                || !seen.insert(name.clone())
            {
                return Err(invalid("unexpected, duplicate or unordered package member"));
            }
            previous = name.clone();
            let destination = root.join(&name);
            fs::create_dir_all(destination.parent().unwrap()).map_err(|error| {
                RunnerError::fs("create package pack directory", &destination, error)
            })?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map_err(|error| RunnerError::fs("create packaged pack", &destination, error))?;
            let mut remaining = size;
            let mut buffer = [0u8; 64 * 1024];
            while remaining != 0 {
                control.check()?;
                let length = remaining.min(buffer.len() as u64) as usize;
                read_exact(&mut decoder, &mut buffer[..length], &mut consumed)?;
                output
                    .write_all(&buffer[..length])
                    .map_err(|error| RunnerError::fs("write packaged pack", &destination, error))?;
                remaining -= length as u64;
            }
        }
        let padding = ((512 - size % 512) % 512) as usize;
        let mut bytes = [0u8; 512];
        read_exact(&mut decoder, &mut bytes[..padding], &mut consumed)?;
        if bytes[..padding].iter().any(|byte| *byte != 0) {
            return Err(invalid("nonzero package padding"));
        }
    }
    if expected.as_ref() != Some(&seen) {
        return Err(invalid("package is missing declared packs"));
    }
    import.ok_or_else(|| invalid("package has no manifest"))
}

fn read_exact(
    reader: &mut impl Read,
    bytes: &mut [u8],
    consumed: &mut u64,
) -> Result<(), RunnerError> {
    *consumed = consumed
        .checked_add(bytes.len() as u64)
        .ok_or_else(|| invalid("archive size overflow"))?;
    if *consumed > MAX_ARCHIVE_BYTES {
        return Err(invalid("archive exceeds extraction bound"));
    }
    reader.read_exact(bytes).map_err(decode_error)
}

fn octal(bytes: &[u8]) -> Result<u64, RunnerError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| invalid("invalid ustar number"))?
        .trim_matches(['\0', ' ']);
    if text.is_empty() {
        return Ok(0);
    }
    if !text.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
        return Err(invalid("invalid ustar number"));
    }
    u64::from_str_radix(text, 8).map_err(|_| invalid("ustar number overflow"))
}

fn header_fields(header: &[u8; 512]) -> Result<(String, u64), RunnerError> {
    let checksum: u64 = header
        .iter()
        .enumerate()
        .map(|(at, byte)| {
            if (148..156).contains(&at) {
                32
            } else {
                u64::from(*byte)
            }
        })
        .sum();
    if octal(&header[148..156])? != checksum
        || &header[257..263] != b"ustar\0"
        || &header[263..265] != b"00"
        || header[156] != b'0'
        || octal(&header[100..108])? != 0o644
        || octal(&header[108..116])? != 0
        || octal(&header[116..124])? != 0
        || octal(&header[136..148])? != 0
        || header[157..257].iter().any(|byte| *byte != 0)
        || header[265..329].iter().any(|byte| *byte != 0)
        || octal(&header[329..337])? != 0
        || octal(&header[337..345])? != 0
        || header[345..].iter().any(|byte| *byte != 0)
    {
        return Err(invalid("package requires canonical regular ustar members"));
    }
    let end = header[..100]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(100);
    if header[end..100].iter().any(|byte| *byte != 0) {
        return Err(invalid("invalid package member name padding"));
    }
    let name = std::str::from_utf8(&header[..end])
        .map_err(|_| invalid("package member name is not UTF-8"))?
        .to_string();
    Ok((name, octal(&header[124..136])?))
}

fn validate_member(name: &str) -> Result<(), RunnerError> {
    let (prefix, id) = name
        .split_once('/')
        .ok_or_else(|| invalid("invalid package pack path"))?;
    let valid = match prefix {
        "objects" => object_pack_hash48_from_id(id),
        "trees" => tree_pack_hash48_from_id(id),
        _ => return Err(invalid("invalid package pack path")),
    };
    valid.map_err(|_| invalid("invalid package pack filename"))?;
    Ok(())
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonical(&object[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}

fn import_manifest(
    bytes: &[u8],
    source: &WorkflowPackageReference,
) -> Result<(Vec<u8>, BTreeSet<String>), RunnerError> {
    let manifest: Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("invalid package manifest: {error}")))?;
    if serde_json::to_vec(&canonical(&manifest)).map_err(|error| invalid(error.to_string()))?
        != bytes
    {
        return Err(invalid("package manifest must be canonical compact JSON"));
    }
    let object = manifest
        .as_object()
        .ok_or_else(|| invalid("package manifest must be an object"))?;
    let required = [
        "contract",
        "repository_index",
        "target_line",
        "base_snapshot_id",
        "revision_snapshot_id",
        "author_mode",
        "history_entries",
        "content",
    ];
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && key != "summary")
        || manifest["contract"] != "workflow-package/v1"
        || manifest["repository_index"].as_u64()
            != Some(u64::from(source.key.repository_index.get()))
        || manifest["base_snapshot_id"] != source.base_snapshot_id
        || manifest["revision_snapshot_id"] != source.revision_snapshot_id
        || manifest["target_line"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        || manifest["author_mode"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        || !manifest["history_entries"].is_array()
        || object.get("summary").is_some_and(|value| {
            value
                .as_str()
                .is_none_or(|text| text.trim().is_empty() || text.len() > 65535)
        })
    {
        return Err(invalid(
            "package manifest identity or grammar does not match the claim",
        ));
    }
    let content = manifest["content"]
        .as_object()
        .ok_or_else(|| invalid("package content must be an object"))?;
    if content.contains_key("contract")
        || content.contains_key("generation_key")
        || !content.get("line_update").is_some_and(Value::is_null)
        || !content.get("capture_binding").is_some_and(Value::is_null)
    {
        return Err(invalid(
            "package content must not publish a Line or capture binding",
        ));
    }
    let mut members = BTreeSet::new();
    for (field, prefix) in [("object_packs", "objects"), ("tree_packs", "trees")] {
        let rows = content
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("package pack rows missing"))?;
        for row in rows {
            let id = row["pack_id"]
                .as_str()
                .ok_or_else(|| invalid("package pack ID missing"))?;
            let member = format!("{prefix}/{id}");
            validate_member(&member)?;
            if !members.insert(member) || members.len() > MAX_PACKAGE_MEMBERS {
                return Err(invalid("duplicate or excessive package pack IDs"));
            }
        }
    }
    let snapshots = content
        .get("snapshots")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("package Snapshot rows missing"))?;
    let mut identities = BTreeSet::new();
    let mut selected = None;
    for row in snapshots {
        let id = row["snapshot_id"]
            .as_str()
            .ok_or_else(|| invalid("package Snapshot ID missing"))?;
        if !identities.insert(id) {
            return Err(invalid("duplicate package Snapshot ID"));
        }
        if id == source.revision_snapshot_id {
            selected = Some(row.clone());
        }
    }
    if !identities.contains(source.base_snapshot_id.as_str()) {
        return Err(invalid("package base Snapshot is missing"));
    }
    let selected = selected.ok_or_else(|| invalid("package final tested Snapshot is missing"))?;
    let mut import = Value::Object(content.clone());
    import["contract"] = json!(IMPORT_MANIFEST_CONTRACT);
    import["repo_name"] = json!("workflow-package");
    import["snapshot_id"] = json!(source.revision_snapshot_id);
    // The native materializer walks only this final tree and reachable packs;
    // original receipt/history Snapshot rows are never imported as an execution.
    import["snapshots"] = json!([selected]);
    Ok((
        serde_json::to_vec(&import).map_err(|error| invalid(error.to_string()))?,
        members,
    ))
}

fn decode_error(error: std::io::Error) -> RunnerError {
    invalid(format!("invalid package archive: {error}"))
}
fn invalid(message: impl Into<String>) -> RunnerError {
    RunnerError::InvalidRequest(format!("workflow_package: {}", message.into()))
}

#[cfg(test)]
mod tests;
