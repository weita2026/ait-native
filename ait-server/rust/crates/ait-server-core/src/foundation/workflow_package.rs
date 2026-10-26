//! The released transient workflow-package/v1 container. It never commits DB data.
use crate::foundation::server_content_binary_db::server_snapshot_hash48_from_id;
use crate::foundation::workflow_package_contract::{parse_package_hash, PACKAGE_MANIFEST_CONTRACT};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

mod materialize;
pub use materialize::WorkflowPackageBaseContent;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageHistoryChange {
    pub local_change_id: String,
    pub local_change_ref: String,
    #[serde(deserialize_with = "required_nullable_string")]
    pub expected_remote_change_ref: Option<String>,
    pub change: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageHistorySnapshot {
    pub snapshot_id: String,
    pub created_at_s: u64,
    #[serde(deserialize_with = "required_nullable_string")]
    pub local_change_ref: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageHistoryEntry {
    pub local_task_id: String,
    #[serde(deserialize_with = "required_nullable_string")]
    pub expected_remote_task_id: Option<String>,
    pub task: Map<String, Value>,
    pub pre_land_target_snapshot_id: String,
    pub landed_snapshot_id: String,
    pub landed_at_s: u64,
    pub changes: Vec<PackageHistoryChange>,
    pub snapshots: Vec<PackageHistorySnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageContent {
    pub object_packs: Vec<Value>,
    pub tree_packs: Vec<Value>,
    pub blob_locators: Vec<Value>,
    pub tree_locators: Vec<Value>,
    pub snapshots: Vec<Value>,
    pub line_update: Value,
    pub capture_binding: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPackageManifest {
    pub contract: String,
    pub repository_index: u32,
    pub target_line: String,
    pub base_snapshot_id: String,
    pub revision_snapshot_id: String,
    pub author_mode: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_nonnull_string"
    )]
    pub summary: Option<String>,
    pub history_entries: Vec<PackageHistoryEntry>,
    pub content: PackageContent,
}

/// The reserved destination namespace belongs only to package materialization.
pub fn require_ordinary_remote_line(name: &str) -> Result<(), String> {
    if name.trim().starts_with("import/") {
        Err("Reserved Remote import/ Line namespace".into())
    } else {
        Ok(())
    }
}

/// Source labels use the existing trim-only Line normalization. Never shorten
/// this injective encoding, including when checking a future Task's maximum ID.
pub fn projected_package_line(task_id: &str, source: &str) -> Result<String, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("Source base Line must not be empty".into());
    }
    let hex = source
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let name = format!("import/{task_id}/base/{hex}");
    u16::try_from(name.len()).map_err(|_| "Projected Line exceeds u16 UTF-8 byte capacity")?;
    Ok(name)
}

impl WorkflowPackageManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("Invalid package manifest: {error}"))?;
        if serde_json::to_vec(&value).map_err(|error| error.to_string())? != bytes {
            return Err("Package manifest must be compact JSON with recursively sorted keys and no BOM/newline".into());
        }
        let manifest: Self = serde_json::from_value(value)
            .map_err(|error| format!("Invalid package manifest fields: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.contract != PACKAGE_MANIFEST_CONTRACT
            || self.target_line.trim().is_empty()
            || self.history_entries.is_empty()
            || !self.content.line_update.is_null()
            || !self.content.capture_binding.is_null()
        {
            return Err(
                "Package manifest contract, history, target or prepare-only content differs".into(),
            );
        }
        require_ordinary_remote_line(&self.target_line)?;
        if self.history_entries.len() > 64 {
            return Err("Package exceeds 64 source Tasks".into());
        }
        for snapshot in [&self.base_snapshot_id, &self.revision_snapshot_id] {
            server_snapshot_hash48_from_id(snapshot).map_err(|error| error.to_string())?;
        }
        if !matches!(
            self.author_mode.as_str(),
            "human_only" | "human_with_ai_assist" | "ai_with_human_review" | "ai_only_experimental"
        ) {
            return Err("Package author_mode is unsupported".into());
        }
        let mut tasks = BTreeSet::new();
        let mut snapshots = BTreeSet::new();
        for entry in &self.history_entries {
            for snapshot in [
                &entry.pre_land_target_snapshot_id,
                &entry.landed_snapshot_id,
            ] {
                server_snapshot_hash48_from_id(snapshot).map_err(|error| error.to_string())?;
            }
            if entry.local_task_id.trim().is_empty()
                || !tasks.insert(&entry.local_task_id)
                || entry.landed_at_s == 0
            {
                return Err("Package history requires distinct real completed source Tasks".into());
            }
            validate_object(
                &entry.task,
                &[
                    "title",
                    "intent",
                    "plan_id",
                    "origin_plan_revision_id",
                    "plan_item_ref",
                ],
                &["title", "intent"],
            )?;
            if entry.snapshots.len() > 64 || entry.changes.len() > 64 {
                return Err("Source receipt exceeds its Snapshot64 or Change64 capacity".into());
            }
            let mut change_refs = BTreeSet::new();
            let mut last_change = 0;
            for change in &entry.changes {
                let ordinal = parse_change_ordinal(&change.local_change_id)?;
                if ordinal <= last_change
                    || change.local_change_ref
                        != format!("{}/{}", entry.local_task_id, change.local_change_id)
                    || !change_refs.insert(change.local_change_ref.as_str())
                {
                    return Err(
                        "Package Changes must be distinct Task-scoped refs in source ordinal order"
                            .into(),
                    );
                }
                last_change = ordinal;
                validate_object(
                    &change.change,
                    &["title", "base_line", "fork_snapshot_id"],
                    &["title", "base_line", "fork_snapshot_id"],
                )?;
                server_snapshot_hash48_from_id(
                    change.change["fork_snapshot_id"]
                        .as_str()
                        .ok_or("Change fork Snapshot must be a string")?,
                )
                .map_err(|error| error.to_string())?;
            }
            for snapshot in &entry.snapshots {
                server_snapshot_hash48_from_id(&snapshot.snapshot_id)
                    .map_err(|error| error.to_string())?;
                if snapshot.created_at_s == 0
                    || !snapshots.insert(&snapshot.snapshot_id)
                    || snapshot
                        .local_change_ref
                        .as_ref()
                        .is_some_and(|owner| !change_refs.contains(owner.as_str()))
                {
                    return Err(
                        "Package Snapshot time, uniqueness or explicit Change owner differs".into(),
                    );
                }
            }
        }
        for (rows, key) in [
            (&self.content.object_packs, "pack_id"),
            (&self.content.tree_packs, "pack_id"),
            (&self.content.blob_locators, "blob_id"),
            (&self.content.tree_locators, "tree_id"),
        ] {
            ordered_ids(rows, key)?;
        }
        validate_snapshot_order(&self.content.snapshots)?;
        let rows = self
            .content
            .snapshots
            .iter()
            .filter_map(|row| row["snapshot_id"].as_str().map(|id| (id, row)))
            .collect::<BTreeMap<_, _>>();
        for entry in &self.history_entries {
            // Existing canonical content may be omitted from a delta. Validate
            // every entry whose parent metadata is supplied in this package.
            if let Some(owned) = entry
                .snapshots
                .iter()
                .map(|snapshot| {
                    rows.get(snapshot.snapshot_id.as_str())
                        .map(|row| (*row).clone())
                })
                .collect::<Option<Vec<_>>>()
            {
                validate_snapshot_order(&owned)?;
            }
        }
        Ok(())
    }

    /// The lossless fixed-owner readback path needs at least one Snapshot for
    /// each newly mapped Change. Known explicit mappings do not need inference.
    pub fn require_supported_change_readback(&self) -> Result<(), String> {
        for entry in &self.history_entries {
            for change in &entry.changes {
                if change.expected_remote_change_ref.is_none()
                    && !entry.snapshots.iter().any(|snapshot| {
                        snapshot.local_change_ref.as_deref()
                            == Some(change.local_change_ref.as_str())
                    })
                {
                    return Err(format!("Source Change {} has no Snapshot or explicit mapping; publication requires an approved recoverable representation",change.local_change_ref));
                }
            }
        }
        Ok(())
    }

    fn archive_paths(&self) -> Result<Vec<String>, String> {
        let mut paths = vec!["manifest.json".into()];
        for (prefix, rows) in [
            ("objects", &self.content.object_packs),
            ("trees", &self.content.tree_packs),
        ] {
            for id in ordered_ids(rows, "pack_id")? {
                if id.is_empty()
                    || !id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                {
                    return Err("Package pack filename is not a valid identity segment".into());
                }
                paths.push(format!("{prefix}/{id}"));
            }
        }
        Ok(paths)
    }
}

pub struct WorkflowPackage {
    pub manifest: WorkflowPackageManifest,
    pub hash: [u8; 32],
    pub inputs: PathBuf,
}

impl WorkflowPackage {
    /// Read the original sealed manifest during recovery/evidence/finish. The
    /// full container was validated on admission; never trust a mutable copy
    /// of manifest.json separately from its compressed hash identity.
    pub fn read_sealed_manifest(
        raw: &[u8],
        hash: [u8; 32],
        repository: u32,
        limit: usize,
    ) -> Result<WorkflowPackageManifest, String> {
        if <[u8; 32]>::from(Sha256::digest(raw)) != hash {
            return Err("Sealed package hash differs".into());
        }
        let mut reader = zstd::stream::read::Decoder::new(raw)
            .map_err(|error| error.to_string())?
            .single_frame();
        let mut header = [0; 512];
        reader
            .read_exact(&mut header)
            .map_err(|error| error.to_string())?;
        let (name, len) = tar_header(&header)?;
        if name != "manifest.json" || len > limit as u64 {
            return Err("Sealed package has no bounded first manifest".into());
        }
        let mut bytes = vec![0; len as usize];
        reader
            .read_exact(&mut bytes)
            .map_err(|error| error.to_string())?;
        let manifest = WorkflowPackageManifest::parse(&bytes)?;
        if manifest.repository_index != repository {
            return Err("Sealed package Repository differs".into());
        }
        Ok(manifest)
    }

    pub fn unpack(
        raw: &[u8],
        destination: &Path,
        repository_index: u32,
        expected_hash: &str,
        manifest_limit: usize,
    ) -> Result<Self, String> {
        let hash: [u8; 32] = Sha256::digest(raw).into();
        if hash != parse_package_hash(expected_hash)? {
            return Err("Package SHA-256 differs from its upload header".into());
        }
        if raw.get(..4) != Some(&[0x28, 0xb5, 0x2f, 0xfd])
            || raw
                .get(4)
                .is_none_or(|descriptor| descriptor & 0b0000_0111 != 0)
        {
            return Err(
                "Package requires one dictionary-free zstd frame with checksum disabled".into(),
            );
        }
        let compressed_size = zstd::zstd_safe::find_frame_compressed_size(raw)
            .map_err(|error| format!("Invalid package zstd frame: {error}"))?;
        let content_size = zstd::zstd_safe::get_frame_content_size(raw)
            .map_err(|error| format!("Invalid zstd content size: {error:?}"))?
            .ok_or("Package zstd frame must pledge its exact uncompressed length")?;
        if compressed_size != raw.len() {
            return Err("Package must contain exactly one zstd frame".into());
        }
        fs::create_dir(destination)
            .map_err(|error| format!("Create fresh package input directory: {error}"))?;
        let result = unpack_archive(
            raw,
            destination,
            repository_index,
            content_size,
            manifest_limit,
        );
        match result {
            Ok(manifest) => Ok(Self {
                manifest,
                hash,
                inputs: destination.to_path_buf(),
            }),
            Err(error) => {
                let _ = fs::remove_dir_all(destination);
                Err(error)
            }
        }
    }
}

fn unpack_archive(
    raw: &[u8],
    destination: &Path,
    repository_index: u32,
    content_size: u64,
    manifest_limit: usize,
) -> Result<WorkflowPackageManifest, String> {
    let mut reader = zstd::stream::read::Decoder::new(raw)
        .map_err(|error| error.to_string())?
        .single_frame();
    let mut consumed = 0_u64;
    let mut expected = vec!["manifest.json".to_string()];
    let mut manifest = None;
    let mut position = 0;
    loop {
        let mut header = [0; 512];
        read_exact(&mut reader, &mut header, &mut consumed)?;
        if header.iter().all(|byte| *byte == 0) {
            let mut end = [0; 512];
            read_exact(&mut reader, &mut end, &mut consumed)?;
            if end.iter().any(|byte| *byte != 0) {
                return Err("Tar requires two zero end blocks".into());
            }
            let mut padding = [0; 8192];
            loop {
                let count = reader
                    .read(&mut padding)
                    .map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                consumed += count as u64;
                if padding[..count].iter().any(|byte| *byte != 0) {
                    return Err("Tar has trailing nonzero data".into());
                }
            }
            break;
        }
        let (name, size) = tar_header(&header)?;
        if expected.get(position) != Some(&name) || size > content_size.saturating_sub(consumed) {
            return Err("Package archive order, filename, duplicate member or size differs from its manifest".into());
        }
        let output = destination.join(&name);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if position == 0 {
            let size = usize::try_from(size).map_err(|_| "Package manifest is too large")?;
            if size > manifest_limit {
                return Err("Package manifest exceeds the existing request body limit".into());
            }
            let mut bytes = vec![0; size];
            read_exact(&mut reader, &mut bytes, &mut consumed)?;
            let parsed = WorkflowPackageManifest::parse(&bytes)?;
            if parsed.repository_index != repository_index {
                return Err("Package Repository differs from its routed authority".into());
            }
            expected = parsed.archive_paths()?;
            fs::write(&output, &bytes).map_err(|error| error.to_string())?;
            manifest = Some(parsed);
        } else {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&output)
                .map_err(|error| error.to_string())?;
            let copied = std::io::copy(&mut reader.by_ref().take(size), &mut file)
                .map_err(|error| error.to_string())?;
            consumed += copied;
            if copied != size {
                return Err("Package member is truncated".into());
            }
            file.flush().map_err(|error| error.to_string())?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&output, fs::Permissions::from_mode(0o644))
                .map_err(|error| error.to_string())?;
        }
        let padding_size = ((512 - size % 512) % 512) as usize;
        let mut padding = vec![0; padding_size];
        read_exact(&mut reader, &mut padding, &mut consumed)?;
        if padding.iter().any(|byte| *byte != 0) {
            return Err("Package tar padding is nonzero".into());
        }
        position += 1;
    }
    if position != expected.len() || consumed != content_size {
        return Err("Package tar is incomplete or disagrees with its pledged length".into());
    }
    manifest.ok_or_else(|| "Package has no manifest".into())
}

fn tar_header(header: &[u8; 512]) -> Result<(String, u64), String> {
    if &header[257..263] != b"ustar\0"
        || &header[263..265] != b"00"
        || !matches!(header[156], 0 | b'0')
        || header[157..257].iter().any(|byte| *byte != 0)
        || header[265..329].iter().any(|byte| *byte != 0)
        || header[345..].iter().any(|byte| *byte != 0)
        || octal(&header[100..108])? != 0o644
        || octal(&header[108..116])? != 0
        || octal(&header[116..124])? != 0
        || octal(&header[136..148])? != 0
        || octal(&header[329..337])? != 0
        || octal(&header[337..345])? != 0
    {
        return Err("Package tar member is not the released regular POSIX ustar form".into());
    }
    let checksum = header
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                32_u64
            } else {
                *byte as u64
            }
        })
        .sum::<u64>();
    if checksum != octal(&header[148..156])? {
        return Err("Package tar header checksum differs".into());
    }
    let end = header[..100]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(100);
    if header[end..100].iter().any(|byte| *byte != 0) {
        return Err("Package tar filename padding is nonzero".into());
    }
    let name = std::str::from_utf8(&header[..end])
        .map_err(|error| error.to_string())?
        .to_string();
    if name.is_empty()
        || name.starts_with('/')
        || name
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || name.contains('\\')
    {
        return Err("Package tar filename escapes its input directory".into());
    }
    Ok((name, octal(&header[124..136])?))
}

fn octal(bytes: &[u8]) -> Result<u64, String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|error| error.to_string())?
        .trim_matches(|character| matches!(character, '\0' | ' '));
    if value.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(value, 8).map_err(|_| "Package tar number is not unsigned octal".into())
}
fn read_exact(reader: &mut impl Read, bytes: &mut [u8], consumed: &mut u64) -> Result<(), String> {
    reader
        .read_exact(bytes)
        .map_err(|error| format!("Truncated package archive: {error}"))?;
    *consumed = consumed
        .checked_add(bytes.len() as u64)
        .ok_or("Package size overflows")?;
    Ok(())
}
fn required_nullable_string<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}
fn optional_nonnull_string<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}
fn parse_change_ordinal(id: &str) -> Result<u8, String> {
    let ordinal = id
        .strip_prefix("C-")
        .and_then(|suffix| suffix.parse::<u8>().ok())
        .filter(|ordinal| (1..=64).contains(ordinal))
        .ok_or("Package Change ID is invalid")?;
    if id != format!("C-{ordinal:02}") {
        return Err("Package Change ID is not canonical".into());
    }
    Ok(ordinal)
}
fn validate_object(
    object: &Map<String, Value>,
    allowed: &[&str],
    required: &[&str],
) -> Result<(), String> {
    if object.keys().any(|key| !allowed.contains(&key.as_str()))
        || required.iter().any(|key| {
            object
                .get(*key)
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
        })
    {
        return Err("Package history object has unknown fields or missing required text".into());
    }
    Ok(())
}
fn ordered_ids<'a>(rows: &'a [Value], key: &str) -> Result<Vec<&'a str>, String> {
    let values = rows
        .iter()
        .map(|row| {
            row.get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("Package row requires {key}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.windows(2).any(|ids| ids[0] >= ids[1]) {
        return Err(format!("Package {key} rows must be unique and sorted"));
    }
    Ok(values)
}
pub(crate) fn validate_snapshot_order(rows: &[Value]) -> Result<(), String> {
    let mut remaining = BTreeMap::new();
    for row in rows {
        let id = row
            .get("snapshot_id")
            .and_then(Value::as_str)
            .ok_or("Package Snapshot row requires snapshot_id")?;
        if remaining.insert(id, row).is_some() {
            return Err("Package Snapshot rows are duplicated".into());
        }
    }
    let all = remaining.keys().copied().collect::<BTreeSet<_>>();
    let mut done = BTreeSet::new();
    for row in rows {
        let next = remaining
            .iter()
            .find(|(_, row)| {
                let parents = row
                    .get("parent_snapshot_ids")
                    .and_then(Value::as_array)
                    .map(|parents| parents.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                    .unwrap_or_else(|| {
                        row.get("parent_snapshot_id")
                            .and_then(Value::as_str)
                            .into_iter()
                            .collect()
                    });
                parents
                    .into_iter()
                    .all(|parent| !all.contains(parent) || done.contains(parent))
            })
            .map(|(id, _)| *id)
            .ok_or("Package Snapshot parents have a cycle")?;
        if row["snapshot_id"].as_str() != Some(next) {
            return Err(
                "Package Snapshots must be parent-first with Snapshot ID tie-breaking".into(),
            );
        }
        remaining.remove(next);
        done.insert(next);
    }
    Ok(())
}

#[cfg(test)]
#[path = "workflow_package/tests.rs"]
mod tests;
