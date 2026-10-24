use super::*;
use crate::json_support::{
    encode_value, encode_value_pretty_with_newline_error_string, parse_value,
};
use ait_core::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotTreeReadStore, SnapshotAuthoringOptions,
};
use ait_core::tag_store::{new_tag_record, FilesystemTagStore, TagStore};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::{Builder as TempBuilder, TempDir};

const INTEROP_CONTRACT: &str = "git-interop-operation/v1";
const MAPPING_CONTRACT: &str = "git-identity-map/v1";
const CHECKPOINT_CONTRACT: &str = "git-interop-checkpoint/v1";
const OBJECT_FORMAT_SHA1: &str = "sha1";
const ZERO_SHA1: &str = "0000000000000000000000000000000000000000";

mod mirror;
pub use mirror::git_mirror;

#[derive(Clone, Debug)]
struct SourceInfo {
    source: String,
    source_identity: String,
    fingerprint: String,
    object_format: String,
    generation_id: String,
    head_symbolic_ref: Option<String>,
    head_object_id: Option<String>,
    refs: Vec<SourceRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceRef {
    name: String,
    object_id: String,
}

#[derive(Clone, Debug)]
struct ImportedRef {
    source_name: String,
    object_id: String,
    retained_name: String,
}

#[derive(Clone, Debug)]
struct CommitData {
    object_id: String,
    tree_object_id: String,
    parent_object_ids: Vec<String>,
    author: GitIdentity,
    committer: GitIdentity,
    message_bytes: Vec<u8>,
    raw_bytes: Vec<u8>,
    files: Vec<GitTreeEntry>,
    signed: bool,
    lfs_pointer_count: usize,
}

#[derive(Clone, Debug)]
struct GitIdentity {
    raw: String,
    name: String,
    email: String,
    timestamp: String,
    timezone: String,
}

#[derive(Clone, Debug)]
struct GitTreeEntry {
    mode: String,
    object_type: String,
    object_id: String,
    path: String,
}

#[derive(Clone, Debug)]
struct TagData {
    source_ref: String,
    name: String,
    object_id: String,
    object_type: String,
    peeled_commit_id: String,
    raw_bytes: Vec<u8>,
    message_bytes: Vec<u8>,
    signed: bool,
}

#[derive(Clone, Debug)]
struct ExportRef {
    git_ref_name: String,
    snapshot_id: String,
    ait_kind: String,
    ait_name: String,
    ait_identity: Option<String>,
    message: Option<String>,
    created_at: Option<String>,
}

#[derive(Clone, Debug)]
struct TargetInfo {
    requested: String,
    path: PathBuf,
    git_dir: PathBuf,
    fingerprint: String,
    object_format: String,
    existed: bool,
    bare: bool,
}

#[derive(Clone, Debug)]
struct InteropStore {
    root: PathBuf,
}

impl InteropStore {
    fn new(repo: &RepoRuntime) -> Self {
        Self {
            root: repo
                .authoritative_repo_root()
                .join(".ait")
                .join("git-interop")
                .join("v1"),
        }
    }

    fn retained_repository(&self, fingerprint: &str) -> PathBuf {
        self.root
            .join("repositories")
            .join(format!("{}.git", fingerprint.to_ascii_lowercase()))
    }

    fn generated_repository(&self) -> PathBuf {
        self.root.join("generated.git")
    }

    fn operation_path(&self, operation_id: &str) -> PathBuf {
        self.root
            .join("operations")
            .join(format!("{}.json", operation_id.to_ascii_lowercase()))
    }

    fn mappings_root(&self) -> PathBuf {
        self.root.join("mappings")
    }

    fn read_operation(&self, operation_id: &str) -> Result<Option<JsonValue>, String> {
        let path = self.operation_path(operation_id);
        match fs::read_to_string(&path) {
            Ok(text) => parse_value(
                &text,
                &format!("Failed to decode Git interop checkpoint {}", path.display()),
            )
            .map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!(
                "Failed to read Git interop checkpoint {}: {error}",
                path.display()
            )),
        }
    }

    fn write_operation(&self, operation_id: &str, payload: &JsonValue) -> Result<(), String> {
        atomic_write_json(&self.operation_path(operation_id), payload)
    }

    fn load_mappings(&self) -> Result<Vec<JsonValue>, String> {
        let root = self.mappings_root();
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!(
                    "Failed to list Git identity mappings {}: {error}",
                    root.display()
                ))
            }
        };
        let mut paths = entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Failed to list Git identity mappings: {error}"))?;
        paths.sort();
        let mut rows = Vec::new();
        for path in paths {
            if path.extension().and_then(OsStr::to_str) != Some("json") {
                continue;
            }
            let text = fs::read_to_string(&path).map_err(|error| {
                format!(
                    "Failed to read Git identity mapping {}: {error}",
                    path.display()
                )
            })?;
            let row = parse_value(
                &text,
                &format!("Failed to decode Git identity mapping {}", path.display()),
            )?;
            if json_text(&row, "contract") != Some(MAPPING_CONTRACT) {
                return Err(format!(
                    "Unsupported Git identity mapping contract in {}.",
                    path.display()
                ));
            }
            rows.push(row);
        }
        Ok(rows)
    }

    fn write_mapping(&self, mut payload: JsonValue) -> Result<(String, bool), String> {
        let recorded_at_unix_nanos = unix_time_nanos()?;
        {
            let object = payload
                .as_object_mut()
                .ok_or_else(|| "Git identity mapping must be an object.".to_string())?;
            object.insert(
                "contract".to_string(),
                JsonValue::String(MAPPING_CONTRACT.to_string()),
            );
            object.remove("record_id");
            object.insert(
                "recorded_at_unix_nanos".to_string(),
                JsonValue::String(recorded_at_unix_nanos.to_string()),
            );
        }
        let logical_payload = logical_mapping_payload(&payload)?;
        let canonical = encode_value(
            &logical_payload,
            "Failed to encode logical Git identity mapping",
        )?;
        let record_id = sha256_prefixed("GIM", canonical.as_bytes(), 20);
        payload.as_object_mut().unwrap().insert(
            "record_id".to_string(),
            JsonValue::String(record_id.clone()),
        );
        let path = self
            .mappings_root()
            .join(format!("{}.json", record_id.to_ascii_lowercase()));
        let encoded = encode_value_pretty_with_newline_error_string(&payload)?;
        if let Ok(existing) = fs::read_to_string(&path) {
            let existing_payload = parse_value(
                &existing,
                &format!("Failed to decode Git identity mapping {}", path.display()),
            )?;
            if logical_mapping_payload(&existing_payload)? == logical_payload {
                return Ok((record_id, false));
            }
            return Err(format!(
                "Immutable Git identity mapping collision at {}.",
                path.display()
            ));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create Git mapping directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let temp = path.with_extension(format!(
            "json.tmp-{}-{recorded_at_unix_nanos}",
            std::process::id()
        ));
        {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)
                .map_err(|error| {
                    format!(
                        "Failed to stage Git identity mapping {}: {error}",
                        temp.display()
                    )
                })?;
            file.write_all(encoded.as_bytes()).map_err(|error| {
                format!(
                    "Failed to write Git identity mapping {}: {error}",
                    temp.display()
                )
            })?;
            file.sync_all().map_err(|error| {
                format!(
                    "Failed to sync Git identity mapping {}: {error}",
                    temp.display()
                )
            })?;
        }
        match fs::hard_link(&temp, &path) {
            Ok(()) => {
                fs::remove_file(&temp).map_err(|error| {
                    format!(
                        "Failed to remove Git mapping staging file {}: {error}",
                        temp.display()
                    )
                })?;
                Ok((record_id, true))
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&temp);
                let existing = fs::read_to_string(&path).map_err(|read_error| {
                    format!(
                        "Failed to verify concurrent Git mapping {} after {error}: {read_error}",
                        path.display()
                    )
                })?;
                let existing_payload = parse_value(
                    &existing,
                    &format!("Failed to decode Git identity mapping {}", path.display()),
                )?;
                if logical_mapping_payload(&existing_payload)? == logical_payload {
                    Ok((record_id, false))
                } else {
                    Err(format!(
                        "Immutable Git identity mapping collision at {}.",
                        path.display()
                    ))
                }
            }
            Err(error) => {
                let _ = fs::remove_file(&temp);
                Err(format!(
                    "Failed to publish immutable Git mapping {}: {error}",
                    path.display()
                ))
            }
        }
    }
}

fn logical_mapping_payload(payload: &JsonValue) -> Result<JsonValue, String> {
    let mut logical = payload.clone();
    let object = logical
        .as_object_mut()
        .ok_or_else(|| "Git identity mapping must be an object.".to_string())?;
    object.remove("record_id");
    object.remove("created_at");
    object.remove("recorded_at_unix_nanos");
    Ok(logical)
}

fn unix_time_nanos() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| format!("System clock is before the Unix epoch: {error}"))
}

fn atomic_write_json(path: &Path, payload: &JsonValue) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Git interop directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = encode_value_pretty_with_newline_error_string(payload)?;
    let temp = path.with_extension(format!(
        "json.tmp-{}-{}",
        std::process::id(),
        unix_time_nanos()?
    ));
    {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)
            .map_err(|error| {
                format!(
                    "Failed to stage Git interop state {}: {error}",
                    temp.display()
                )
            })?;
        file.write_all(encoded.as_bytes()).map_err(|error| {
            format!(
                "Failed to write Git interop state {}: {error}",
                temp.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "Failed to sync Git interop state {}: {error}",
                temp.display()
            )
        })?;
    }
    fs::rename(&temp, path).map_err(|error| {
        format!(
            "Failed to publish Git interop state {} -> {}: {error}",
            temp.display(),
            path.display()
        )
    })
}

fn inspect_source(source: &str) -> Result<SourceInfo, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("git source must not be empty.".to_string());
    }
    if source.starts_with('-') {
        return Err("git source must not begin with '-'".to_string());
    }
    let source_path = PathBuf::from(source);
    let source_identity = if source_path.exists() {
        source_path
            .canonicalize()
            .map_err(|error| format!("Failed to resolve Git source {source}: {error}"))?
            .to_string_lossy()
            .to_string()
    } else {
        source.to_string()
    };
    let output = run_git(
        [
            "ls-remote",
            "--symref",
            source,
            "HEAD",
            "refs/heads/*",
            "refs/tags/*",
            "refs/notes/*",
            "refs/replace/*",
        ],
        &[],
        None,
    )?;
    let text = String::from_utf8(output)
        .map_err(|_| "git ls-remote returned non-UTF-8 ref metadata.".to_string())?;
    let mut refs = Vec::new();
    let mut head_symbolic_ref = None;
    let mut head_object_id = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("ref: ") {
            let mut fields = rest.split('\t');
            let target = fields.next().unwrap_or_default();
            let name = fields.next().unwrap_or_default();
            if name == "HEAD" {
                head_symbolic_ref = Some(target.to_string());
            }
            continue;
        }
        let mut fields = line.split('\t');
        let object_id = fields.next().unwrap_or_default();
        let name = fields.next().unwrap_or_default();
        if object_id.is_empty() || name.is_empty() || name.ends_with("^{}") {
            continue;
        }
        if name == "HEAD" {
            head_object_id = Some(object_id.to_string());
            continue;
        }
        refs.push(SourceRef {
            name: name.to_string(),
            object_id: object_id.to_string(),
        });
    }
    refs.sort_by(|left, right| left.name.cmp(&right.name));
    refs.dedup_by(|left, right| left.name == right.name && left.object_id == right.object_id);
    let object_format = if source_path.exists() {
        let output = run_git(
            [
                "-C",
                source_identity.as_str(),
                "rev-parse",
                "--show-object-format",
            ],
            &[],
            None,
        )?;
        String::from_utf8(output)
            .map_err(|_| "Git object format output was not UTF-8.".to_string())?
            .trim()
            .to_string()
    } else {
        let oid_length = refs
            .first()
            .map(|row| row.object_id.len())
            .or_else(|| head_object_id.as_ref().map(String::len))
            .unwrap_or(40);
        match oid_length {
            40 => OBJECT_FORMAT_SHA1.to_string(),
            64 => "sha256".to_string(),
            other => format!("unknown-{other}"),
        }
    };
    let fingerprint = sha256_prefixed(
        "GSR",
        format!("git-source-fingerprint/v1\n{object_format}\n{source_identity}\n").as_bytes(),
        24,
    );
    let ref_generation_material = refs
        .iter()
        .map(|row| format!("{} {}", row.name, row.object_id))
        .collect::<Vec<_>>()
        .join("\n");
    let generation_material = format!(
        "HEAD symbolic={} object={}\n{}",
        head_symbolic_ref.as_deref().unwrap_or("none"),
        head_object_id.as_deref().unwrap_or("none"),
        ref_generation_material
    );
    let generation_id = sha256_prefixed(
        "GIT-IMP",
        format!("{fingerprint}\n{generation_material}\n").as_bytes(),
        16,
    );
    Ok(SourceInfo {
        source: source.to_string(),
        source_identity,
        fingerprint,
        object_format,
        generation_id,
        head_symbolic_ref,
        head_object_id,
        refs,
    })
}

fn prepare_retained_repository(
    path: &Path,
    source: &SourceInfo,
) -> Result<Vec<ImportedRef>, String> {
    ensure_bare_repository(path, &source.object_format)?;
    let mut refspecs = Vec::new();
    let mut imported_refs = Vec::new();
    for source_ref in &source.refs {
        let Some(suffix) = source_ref.name.strip_prefix("refs/") else {
            continue;
        };
        let retained_name = format!(
            "refs/ait/import/{}/{suffix}",
            source.generation_id.to_ascii_lowercase()
        );
        refspecs.push(format!("+{}:{retained_name}", source_ref.name));
        imported_refs.push(ImportedRef {
            source_name: source_ref.name.clone(),
            object_id: source_ref.object_id.clone(),
            retained_name,
        });
    }
    if let Some(head_object_id) = source.head_object_id.as_deref() {
        let retained_name = format!(
            "refs/ait/import/{}/HEAD",
            source.generation_id.to_ascii_lowercase()
        );
        refspecs.push(format!("+HEAD:{retained_name}"));
        imported_refs.push(ImportedRef {
            source_name: "HEAD".to_string(),
            object_id: head_object_id.to_string(),
            retained_name,
        });
    }
    if !refspecs.is_empty() {
        let mut args = vec![
            OsString::from(format!("--git-dir={}", path.display())),
            OsString::from("fetch"),
            OsString::from("--no-tags"),
            OsString::from("--no-write-fetch-head"),
            OsString::from(&source.source),
        ];
        args.extend(refspecs.into_iter().map(OsString::from));
        run_git_os(args, &[], None)?;
    }
    for imported in &imported_refs {
        let retained_object_id = git_ref_object_id(path, &imported.retained_name)?;
        if retained_object_id.as_deref() != Some(imported.object_id.as_str()) {
            return Err(format!(
                "Retained Git ref {} did not resolve to expected object {} after fetch.",
                imported.retained_name, imported.object_id
            ));
        }
    }
    imported_refs.sort_by(|left, right| left.source_name.cmp(&right.source_name));
    Ok(imported_refs)
}

fn ensure_bare_repository(path: &Path, object_format: &str) -> Result<(), String> {
    if object_format != OBJECT_FORMAT_SHA1 {
        return Err(format!(
            "Unsupported Git object format {object_format:?}; this build supports sha1 only."
        ));
    }
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Failed to create Git object-store directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        run_git_os(
            vec![
                OsString::from("init"),
                OsString::from("--bare"),
                OsString::from("--object-format=sha1"),
                path.as_os_str().to_os_string(),
            ],
            &[],
            None,
        )?;
    }
    let format = git_repo_text(path, ["rev-parse", "--show-object-format"])?;
    if format.trim() != object_format {
        return Err(format!(
            "Git object-store format mismatch at {}: expected {object_format}, found {}.",
            path.display(),
            format.trim()
        ));
    }
    Ok(())
}

fn selected_import_refs(
    source: &SourceInfo,
    retained_refs: &[ImportedRef],
    all_branches_and_tags: bool,
) -> Result<(Vec<ImportedRef>, Vec<ImportedRef>), String> {
    let heads = retained_refs
        .iter()
        .filter(|row| row.source_name.starts_with("refs/heads/"))
        .cloned()
        .collect::<Vec<_>>();
    let selected_heads = if all_branches_and_tags {
        heads
    } else if let Some(symbolic) = source.head_symbolic_ref.as_deref() {
        heads
            .into_iter()
            .filter(|row| row.source_name == symbolic)
            .collect::<Vec<_>>()
    } else if let Some(head) = source.head_object_id.as_deref() {
        let mut matching = heads
            .into_iter()
            .filter(|row| row.object_id == head)
            .collect::<Vec<_>>();
        matching.truncate(1);
        matching
    } else {
        Vec::new()
    };
    if !all_branches_and_tags && selected_heads.is_empty() && source.head_object_id.is_some() {
        let detached = retained_refs
            .iter()
            .find(|row| row.source_name == "HEAD")
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        if !detached.is_empty() {
            return Ok((detached, Vec::new()));
        }
    }
    let selected_tags = if all_branches_and_tags {
        retained_refs
            .iter()
            .filter(|row| row.source_name.starts_with("refs/tags/"))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    Ok((selected_heads, selected_tags))
}

fn load_import_commit_plan(
    git_dir: &Path,
    heads: &[ImportedRef],
    tags: &[ImportedRef],
) -> Result<(Vec<CommitData>, Vec<TagData>), String> {
    let mut tag_rows = Vec::new();
    let mut roots = heads
        .iter()
        .map(|row| row.object_id.clone())
        .collect::<Vec<_>>();
    for tag in tags {
        let object_type = git_repo_text(git_dir, ["cat-file", "-t", tag.object_id.as_str()])?
            .trim()
            .to_string();
        let peeled_commit_id = git_repo_text(
            git_dir,
            ["rev-parse", &format!("{}^{{commit}}", tag.object_id)],
        )?
        .trim()
        .to_string();
        roots.push(peeled_commit_id.clone());
        let raw_bytes = if object_type == "tag" {
            git_repo_bytes(git_dir, ["cat-file", "tag", tag.object_id.as_str()])?
        } else {
            Vec::new()
        };
        let message_bytes = split_object_message(&raw_bytes).to_vec();
        tag_rows.push(TagData {
            source_ref: tag.source_name.clone(),
            name: tag
                .source_name
                .strip_prefix("refs/tags/")
                .unwrap_or(tag.source_name.as_str())
                .to_string(),
            object_id: tag.object_id.clone(),
            object_type,
            peeled_commit_id,
            signed: raw_bytes
                .windows(b"BEGIN PGP SIGNATURE".len())
                .any(|window| window == b"BEGIN PGP SIGNATURE"),
            raw_bytes,
            message_bytes,
        });
    }
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return Ok((Vec::new(), tag_rows));
    }
    let mut args = vec![
        OsString::from("rev-list"),
        OsString::from("--topo-order"),
        OsString::from("--reverse"),
    ];
    args.extend(roots.into_iter().map(OsString::from));
    let output = git_repo_bytes_os(git_dir, args, &[], None)?;
    let text = String::from_utf8(output)
        .map_err(|_| "git rev-list returned non-UTF-8 object IDs.".to_string())?;
    let mut commits = Vec::new();
    for object_id in text
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        commits.push(read_commit_data(git_dir, object_id)?);
    }
    Ok((commits, tag_rows))
}

fn read_commit_data(git_dir: &Path, object_id: &str) -> Result<CommitData, String> {
    let raw_bytes = git_repo_bytes(git_dir, ["cat-file", "commit", object_id])?;
    let header_end = find_subslice(&raw_bytes, b"\n\n").unwrap_or(raw_bytes.len());
    let header = &raw_bytes[..header_end];
    let header_text = String::from_utf8_lossy(header);
    let tree_object_id = header_text
        .lines()
        .find_map(|line| line.strip_prefix("tree "))
        .ok_or_else(|| format!("Git commit {object_id} is missing a tree header."))?
        .to_string();
    let parent_object_ids = header_text
        .lines()
        .filter_map(|line| line.strip_prefix("parent ").map(str::to_string))
        .collect::<Vec<_>>();
    let author = parse_git_identity(
        header_text
            .lines()
            .find_map(|line| line.strip_prefix("author "))
            .ok_or_else(|| format!("Git commit {object_id} is missing author metadata."))?,
    )?;
    let committer = parse_git_identity(
        header_text
            .lines()
            .find_map(|line| line.strip_prefix("committer "))
            .ok_or_else(|| format!("Git commit {object_id} is missing committer metadata."))?,
    )?;
    let tree_output = git_repo_bytes(git_dir, ["ls-tree", "-r", "-z", object_id])?;
    let mut files = Vec::new();
    let mut lfs_pointer_count = 0_usize;
    for raw_row in tree_output
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        let Some(tab) = raw_row.iter().position(|byte| *byte == b'\t') else {
            return Err(format!(
                "Git tree row for commit {object_id} is missing a path separator."
            ));
        };
        let metadata = std::str::from_utf8(&raw_row[..tab])
            .map_err(|_| format!("Git tree metadata for commit {object_id} is not UTF-8."))?;
        let path = std::str::from_utf8(&raw_row[tab + 1..]).map_err(|_| {
            format!(
                "Git commit {object_id} contains a non-UTF-8 path; this build preserves Unicode UTF-8 paths only."
            )
        })?;
        let mut fields = metadata.split_whitespace();
        let mode = fields.next().unwrap_or_default();
        let object_type = fields.next().unwrap_or_default();
        let entry_object_id = fields.next().unwrap_or_default();
        if mode.is_empty() || object_type.is_empty() || entry_object_id.is_empty() {
            return Err(format!("Malformed Git tree row in commit {object_id}."));
        }
        if object_type == "blob" {
            let size = git_repo_text(git_dir, ["cat-file", "-s", entry_object_id])?
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("Invalid Git blob size for {entry_object_id}."))?;
            if size <= 1024 {
                let bytes = git_repo_bytes(git_dir, ["cat-file", "blob", entry_object_id])?;
                if bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n") {
                    lfs_pointer_count += 1;
                }
            }
        }
        files.push(GitTreeEntry {
            mode: mode.to_string(),
            object_type: object_type.to_string(),
            object_id: entry_object_id.to_string(),
            path: path.to_string(),
        });
    }
    Ok(CommitData {
        object_id: object_id.to_string(),
        tree_object_id,
        parent_object_ids,
        author,
        committer,
        message_bytes: split_object_message(&raw_bytes).to_vec(),
        signed: header_text.lines().any(|line| line.starts_with("gpgsig ")),
        raw_bytes,
        files,
        lfs_pointer_count,
    })
}

fn parse_git_identity(raw: &str) -> Result<GitIdentity, String> {
    let close = raw
        .rfind('>')
        .ok_or_else(|| format!("Malformed Git identity {raw:?}: missing '>'."))?;
    let open = raw[..close]
        .rfind('<')
        .ok_or_else(|| format!("Malformed Git identity {raw:?}: missing '<'."))?;
    let name = raw[..open].trim().to_string();
    let email = raw[open + 1..close].to_string();
    let mut tail = raw[close + 1..].split_whitespace();
    let timestamp = tail
        .next()
        .ok_or_else(|| format!("Malformed Git identity {raw:?}: missing timestamp."))?
        .to_string();
    let timezone = tail
        .next()
        .ok_or_else(|| format!("Malformed Git identity {raw:?}: missing timezone."))?
        .to_string();
    Ok(GitIdentity {
        raw: raw.to_string(),
        name,
        email,
        timestamp,
        timezone,
    })
}

fn materialize_commit(git_dir: &Path, object_id: &str) -> Result<TempDir, String> {
    let temp = TempBuilder::new()
        .prefix("ait-git-import-")
        .tempdir()
        .map_err(|error| format!("Failed to create Git import workspace: {error}"))?;
    let index_path = temp.path().join("index");
    let env = vec![(
        OsString::from("GIT_INDEX_FILE"),
        index_path.as_os_str().to_os_string(),
    )];
    git_repo_bytes_os(
        git_dir,
        vec![OsString::from("read-tree"), OsString::from(object_id)],
        &env,
        None,
    )?;
    let args = vec![
        OsString::from(format!("--git-dir={}", git_dir.display())),
        OsString::from(format!("--work-tree={}", temp.path().display())),
        OsString::from("checkout-index"),
        OsString::from("-a"),
        OsString::from("-f"),
    ];
    run_git_os(args, &env, None)?;
    let _ = fs::remove_file(index_path);
    Ok(temp)
}

fn run_git<'a, I>(
    args: I,
    env: &[(OsString, OsString)],
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, String>
where
    I: IntoIterator<Item = &'a str>,
{
    run_git_os(args.into_iter().map(OsString::from).collect(), env, stdin)
}

fn run_git_os(
    args: Vec<OsString>,
    env: &[(OsString, OsString)],
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let display = args
        .iter()
        .map(|value| value.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let mut command = Command::new("git");
    command
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to execute Git (`git {display}`): {error}"))?;
    if let Some(input) = stdin {
        child
            .stdin
            .as_mut()
            .ok_or_else(|| "Git subprocess stdin was unavailable.".to_string())?
            .write_all(input)
            .map_err(|error| format!("Failed to write Git subprocess stdin: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Failed to wait for Git (`git {display}`): {error}"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "Git command failed (`git {display}`): {}",
        stderr.trim()
    ))
}

fn git_repo_bytes<'a, I>(git_dir: &Path, args: I) -> Result<Vec<u8>, String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut owned = vec![OsString::from(format!("--git-dir={}", git_dir.display()))];
    owned.extend(args.into_iter().map(OsString::from));
    run_git_os(owned, &[], None)
}

fn git_repo_bytes_os(
    git_dir: &Path,
    args: Vec<OsString>,
    env: &[(OsString, OsString)],
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let mut owned = vec![OsString::from(format!("--git-dir={}", git_dir.display()))];
    owned.extend(args);
    run_git_os(owned, env, stdin)
}

fn git_repo_text<'a, I>(git_dir: &Path, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = &'a str>,
{
    String::from_utf8(git_repo_bytes(git_dir, args)?)
        .map_err(|_| "Git command returned non-UTF-8 metadata.".to_string())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn split_object_message(raw: &[u8]) -> &[u8] {
    find_subslice(raw, b"\n\n")
        .map(|index| &raw[index + 2..])
        .unwrap_or_default()
}

fn sha256_prefixed(prefix: &str, bytes: &[u8], hex_length: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = format!("{:X}", hasher.finalize());
    format!("{prefix}-{}", &digest[..hex_length.min(digest.len())])
}

fn json_text<'a>(value: &'a JsonValue, field: &str) -> Option<&'a str> {
    value.get(field).and_then(JsonValue::as_str)
}

fn json_usize(value: &JsonValue, field: &str) -> Option<usize> {
    value
        .get(field)
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn strings_json(values: &[String]) -> JsonValue {
    JsonValue::Array(values.iter().cloned().map(JsonValue::String).collect())
}

fn bytes_base64(bytes: &[u8]) -> String {
    BASE64_STANDARD.encode(bytes)
}

fn identity_json(identity: &GitIdentity) -> JsonValue {
    json!({
        "raw": identity.raw,
        "name": identity.name,
        "email": identity.email,
        "timestamp": identity.timestamp,
        "timezone": identity.timezone,
    })
}

fn tree_entries_json(entries: &[GitTreeEntry]) -> JsonValue {
    JsonValue::Array(
        entries
            .iter()
            .map(|entry| {
                json!({
                    "path": entry.path,
                    "mode": entry.mode,
                    "object_type": entry.object_type,
                    "git_object_id": entry.object_id,
                })
            })
            .collect(),
    )
}

fn latest_mapping<'a>(rows: impl IntoIterator<Item = &'a JsonValue>) -> Option<&'a JsonValue> {
    rows.into_iter().max_by(|left, right| {
        let left_nanos = json_text(left, "recorded_at_unix_nanos")
            .and_then(|value| value.parse::<u128>().ok())
            .unwrap_or_default();
        let right_nanos = json_text(right, "recorded_at_unix_nanos")
            .and_then(|value| value.parse::<u128>().ok())
            .unwrap_or_default();
        left_nanos.cmp(&right_nanos).then_with(|| {
            json_text(left, "created_at")
                .unwrap_or_default()
                .cmp(json_text(right, "created_at").unwrap_or_default())
                .then_with(|| {
                    json_text(left, "record_id")
                        .unwrap_or_default()
                        .cmp(json_text(right, "record_id").unwrap_or_default())
                })
        })
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "checkpoint fields map directly to the durable interop record"
)]
fn operation_checkpoint(
    operation: &str,
    operation_id: &str,
    generation_id: &str,
    plan_hash: &str,
    state: &str,
    next_commit_index: usize,
    next_ref_index: usize,
    result: Option<JsonValue>,
) -> JsonValue {
    json!({
        "contract": CHECKPOINT_CONTRACT,
        "operation": operation,
        "operation_id": operation_id,
        "generation_id": generation_id,
        "plan_hash": plan_hash,
        "state": state,
        "next_commit_index": next_commit_index,
        "next_ref_index": next_ref_index,
        "updated_at": system_event_timestamp(),
        "result": result,
    })
}

fn existing_operation_result(
    store: &InteropStore,
    operation_id: &str,
    plan_hash: &str,
) -> Result<(Option<JsonValue>, usize, usize), String> {
    let Some(checkpoint) = store.read_operation(operation_id)? else {
        return Ok((None, 0, 0));
    };
    if json_text(&checkpoint, "contract") != Some(CHECKPOINT_CONTRACT)
        || json_text(&checkpoint, "plan_hash") != Some(plan_hash)
    {
        return Err(format!(
            "Git interop checkpoint {operation_id} does not match the current immutable plan."
        ));
    }
    if json_text(&checkpoint, "state") == Some("completed") {
        return checkpoint
            .get("result")
            .cloned()
            .filter(|value| !value.is_null())
            .map(|result| (Some(result), 0, 0))
            .ok_or_else(|| {
                format!("Completed Git interop checkpoint {operation_id} is missing its result.")
            });
    }
    Ok((
        None,
        json_usize(&checkpoint, "next_commit_index").unwrap_or(0),
        json_usize(&checkpoint, "next_ref_index").unwrap_or(0),
    ))
}

pub fn git_import(
    repo: &RepoRuntime,
    source: &str,
    all_branches_and_tags: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let source_info = inspect_source(source)?;
    let format_blocked = source_info.object_format != OBJECT_FORMAT_SHA1;
    if format_blocked {
        let report = json!({
            "contract": INTEROP_CONTRACT,
            "operation": "import",
            "status": "blocked",
            "source": source_info.source_identity,
            "source_repository_fingerprint": source_info.fingerprint,
            "git_object_format": source_info.object_format,
            "supported_object_formats": [OBJECT_FORMAT_SHA1],
            "blockers": [{
                "kind": "unsupported_object_format",
                "count": 1,
                "disposition": "fail_closed",
            }],
            "dry_run": dry_run,
            "mutated": false,
        });
        if dry_run {
            return Ok(report);
        }
        return Err(format!(
            "Git import blocked: object format {:?} is unsupported; this build supports sha1 only.",
            source_info.object_format
        ));
    }

    let replace_ref_count = source_info
        .refs
        .iter()
        .filter(|row| row.name.starts_with("refs/replace/"))
        .count();
    let notes_ref_count = source_info
        .refs
        .iter()
        .filter(|row| row.name.starts_with("refs/notes/"))
        .count();
    if replace_ref_count > 0 && !dry_run {
        return Err(format!(
            "Git import stopped before changing AIT history: {replace_ref_count} replace ref(s) are unsupported. Remove the replace refs or expand their replacement history, then retry."
        ));
    }

    let interop = InteropStore::new(repo);
    let mut dry_temp = None;
    let git_dir = if dry_run {
        let temp = TempBuilder::new()
            .prefix("ait-git-import-inspect-")
            .tempdir()
            .map_err(|error| format!("Failed to create Git inspection directory: {error}"))?;
        let path = temp.path().join("source.git");
        dry_temp = Some(temp);
        path
    } else {
        interop.retained_repository(&source_info.fingerprint)
    };
    let retained_refs = prepare_retained_repository(&git_dir, &source_info)?;
    let (selected_heads, selected_tags) =
        selected_import_refs(&source_info, &retained_refs, all_branches_and_tags)?;
    let (commits, tags) = load_import_commit_plan(&git_dir, &selected_heads, &selected_tags)?;
    let _dry_temp_guard = dry_temp;

    let submodule_rows = commits
        .iter()
        .flat_map(|commit| {
            commit
                .files
                .iter()
                .filter(|entry| entry.mode == "160000" || entry.object_type == "commit")
                .map(move |entry| {
                    json!({
                        "commit": commit.object_id,
                        "path": entry.path,
                        "git_object_id": entry.object_id,
                    })
                })
        })
        .collect::<Vec<_>>();
    let unsupported_tree_rows = commits
        .iter()
        .flat_map(|commit| {
            commit.files.iter().filter_map(move |entry| {
                let supported = matches!(entry.mode.as_str(), "100644" | "100755" | "120000")
                    && entry.object_type == "blob";
                (!supported && entry.mode != "160000" && entry.object_type != "commit").then(|| {
                    json!({
                        "commit": commit.object_id,
                        "path": entry.path,
                        "mode": entry.mode,
                        "object_type": entry.object_type,
                    })
                })
            })
        })
        .collect::<Vec<_>>();
    let signed_commit_count = commits.iter().filter(|commit| commit.signed).count();
    let signed_tag_count = tags.iter().filter(|tag| tag.signed).count();
    let lfs_pointer_count = commits
        .iter()
        .map(|commit| commit.lfs_pointer_count)
        .sum::<usize>();
    let mut blockers = Vec::new();
    if replace_ref_count > 0 {
        blockers.push(json!({
            "kind": "replace_refs",
            "count": replace_ref_count,
            "disposition": "fail_closed",
        }));
    }
    if !submodule_rows.is_empty() {
        blockers.push(json!({
            "kind": "submodules",
            "count": submodule_rows.len(),
            "disposition": "fail_closed",
            "entries": submodule_rows,
        }));
    }
    if !unsupported_tree_rows.is_empty() {
        blockers.push(json!({
            "kind": "unsupported_tree_entries",
            "count": unsupported_tree_rows.len(),
            "disposition": "fail_closed",
            "entries": unsupported_tree_rows,
        }));
    }
    let classifications = json!([
        {
            "kind": "signed_commits",
            "count": signed_commit_count,
            "disposition": "preserved_raw_unverified",
        },
        {
            "kind": "signed_tags",
            "count": signed_tag_count,
            "disposition": "preserved_raw_unverified",
        },
        {
            "kind": "notes_refs",
            "count": notes_ref_count,
            "disposition": "preserved_git_only",
        },
        {
            "kind": "lfs_pointers",
            "count": lfs_pointer_count,
            "disposition": "pointer_content_preserved",
        }
    ]);
    let plan_material = format!(
        "{}\n{}\n{}\n{}\n{}",
        source_info.generation_id,
        all_branches_and_tags,
        commits
            .iter()
            .map(|commit| commit.object_id.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        selected_heads
            .iter()
            .map(|row| format!("{} {}", row.source_name, row.object_id))
            .collect::<Vec<_>>()
            .join("\n"),
        selected_tags
            .iter()
            .map(|row| format!("{} {}", row.source_name, row.object_id))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let plan_hash = sha256_prefixed("GIP", plan_material.as_bytes(), 24);
    let operation_id = sha256_prefixed(
        "GIO-IMPORT",
        format!("{}\n{plan_hash}", source_info.fingerprint).as_bytes(),
        16,
    );
    if dry_run {
        return Ok(json!({
            "contract": INTEROP_CONTRACT,
            "operation": "import",
            "status": if blockers.is_empty() { "dry_run" } else { "blocked" },
            "operation_id": operation_id,
            "generation_id": source_info.generation_id,
            "plan_hash": plan_hash,
            "source": source_info.source_identity,
            "source_repository_fingerprint": source_info.fingerprint,
            "git_object_format": source_info.object_format,
            "head_symbolic_ref": source_info.head_symbolic_ref,
            "head_object_id": source_info.head_object_id,
            "commit_count": commits.len(),
            "line_count": selected_heads.len(),
            "tag_count": tags.len(),
            "blockers": blockers,
            "classifications": classifications,
            "dry_run": true,
            "resume_supported": true,
            "mutated": false,
        }));
    }
    if !blockers.is_empty() {
        return Err(format!(
            "Git import stopped before changing any AIT Snapshot or ref: {}",
            encode_value(&JsonValue::Array(blockers), "Failed to encode Git blockers")?
        ));
    }

    let (completed, mut next_commit_index, mut next_ref_index) =
        existing_operation_result(&interop, &operation_id, &plan_hash)?;
    if let Some(mut result) = completed {
        if let Some(object) = result.as_object_mut() {
            object.insert("status".to_string(), JsonValue::String("no_op".to_string()));
            object.insert("replayed".to_string(), JsonValue::Bool(true));
        }
        return Ok(result);
    }
    interop.write_operation(
        &operation_id,
        &operation_checkpoint(
            "import",
            &operation_id,
            &source_info.generation_id,
            &plan_hash,
            "running",
            next_commit_index,
            next_ref_index,
            None,
        ),
    )?;

    let mut mappings = interop.load_mappings()?;
    let snapshot_store = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    let mut git_to_snapshot = BTreeMap::<String, String>::new();
    for row in &mappings {
        if json_text(row, "kind") == Some("commit")
            && json_text(row, "direction") == Some("import")
            && json_text(row, "source_repository_fingerprint")
                == Some(source_info.fingerprint.as_str())
        {
            if let (Some(git_id), Some(snapshot_id)) = (
                json_text(row, "git_object_id"),
                json_text(row, "snapshot_id"),
            ) {
                git_to_snapshot.insert(git_id.to_string(), snapshot_id.to_string());
            }
        }
    }
    let mut imported_commit_count = 0_usize;
    let mut reused_commit_count = 0_usize;
    for (index, commit) in commits.iter().enumerate() {
        if index < next_commit_index {
            continue;
        }
        if let Some(snapshot_id) = git_to_snapshot.get(&commit.object_id) {
            if !snapshot_store.snapshot_exists(snapshot_id)? {
                return Err(format!(
                    "Git mapping for commit {} points at missing Snapshot {}.",
                    commit.object_id, snapshot_id
                ));
            }
            reused_commit_count += 1;
        } else {
            let parent_snapshot_ids = commit
                .parent_object_ids
                .iter()
                .map(|parent| {
                    git_to_snapshot.get(parent).cloned().ok_or_else(|| {
                        format!(
                            "Git import plan lost parent mapping {parent} before commit {}.",
                            commit.object_id
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let workspace = materialize_commit(&git_dir, &commit.object_id)?;
            let content = repo
                .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
                .content_for_root(workspace.path().to_path_buf());
            let message = String::from_utf8_lossy(&commit.message_bytes)
                .trim_end_matches(['\n', '\r'])
                .to_string();
            let line_name = format!(
                "git/import/{}",
                source_info.fingerprint.to_ascii_lowercase()
            );
            let payload = content.create_snapshot_content_with_parents_and_options(
                &repo.repo_name(),
                &line_name,
                &parent_snapshot_ids,
                (!message.is_empty()).then_some(message.as_str()),
                false,
                SnapshotAuthoringOptions {
                    allow_unchanged_tree: true,
                },
            )?;
            let snapshot_id = json_text(&payload, "snapshot_id")
                .ok_or_else(|| "Git import Snapshot payload is missing snapshot_id.".to_string())?
                .to_string();
            let locator = content.snapshot_tree_root_locator(&snapshot_id)?;
            let mapping = json!({
                "kind": "commit",
                "direction": "import",
                "created_at": system_event_timestamp(),
                "generation_id": source_info.generation_id,
                "source_repository_fingerprint": source_info.fingerprint,
                "git_object_format": source_info.object_format,
                "git_object_id": commit.object_id,
                "git_tree_object_id": commit.tree_object_id,
                "snapshot_id": snapshot_id,
                "ait_root_tree_id": locator.root_tree_id,
                "parent_git_object_ids": strings_json(&commit.parent_object_ids),
                "parent_snapshot_ids": strings_json(&parent_snapshot_ids),
                "author": identity_json(&commit.author),
                "committer": identity_json(&commit.committer),
                "message_base64": bytes_base64(&commit.message_bytes),
                "raw_commit_base64": bytes_base64(&commit.raw_bytes),
                "file_modes": tree_entries_json(&commit.files),
                "signed": commit.signed,
                "lfs_pointer_count": commit.lfs_pointer_count,
                "imported_unchanged": true,
            });
            interop.write_mapping(mapping.clone())?;
            mappings.push(mapping);
            git_to_snapshot.insert(commit.object_id.clone(), snapshot_id);
            imported_commit_count += 1;
        }
        next_commit_index = index + 1;
        interop.write_operation(
            &operation_id,
            &operation_checkpoint(
                "import",
                &operation_id,
                &source_info.generation_id,
                &plan_hash,
                "running",
                next_commit_index,
                next_ref_index,
                None,
            ),
        )?;
    }

    let timestamp = system_event_timestamp();
    let line_store = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .lines();
    let tag_store =
        FilesystemTagStore::new(repo.authoritative_repo_root().to_string_lossy().as_ref())?;
    let mut ref_index = 0_usize;
    let mut imported_lines = Vec::new();
    for head in &selected_heads {
        if ref_index < next_ref_index {
            ref_index += 1;
            continue;
        }
        let line_name = if head.source_name == "HEAD" {
            "git-head".to_string()
        } else {
            head.source_name
                .strip_prefix("refs/heads/")
                .unwrap_or(head.source_name.as_str())
                .to_string()
        };
        let snapshot_id = git_to_snapshot.get(&head.object_id).ok_or_else(|| {
            format!(
                "Git branch {} points at commit {} without an imported Snapshot.",
                head.source_name, head.object_id
            )
        })?;
        let line = match line_store.line_by_name(&line_name)? {
            None => line_store.create_line(&line_name, Some(snapshot_id), &timestamp)?,
            Some(line) if line.head_snapshot_id.as_deref() == Some(snapshot_id.as_str()) => line,
            Some(line) if line.head_snapshot_id.is_none() => line_store
                .compare_and_swap_line_head(&line_name, None, Some(snapshot_id), &timestamp)?,
            Some(line) => {
                let previous = latest_mapping(mappings.iter().filter(|row| {
                    json_text(row, "kind") == Some("ref")
                        && json_text(row, "direction") == Some("import")
                        && json_text(row, "source_repository_fingerprint")
                            == Some(source_info.fingerprint.as_str())
                        && json_text(row, "git_ref_name") == Some(head.source_name.as_str())
                        && json_text(row, "ait_line_name") == Some(line_name.as_str())
                }));
                let expected = previous.and_then(|row| json_text(row, "snapshot_id"));
                if expected != line.head_snapshot_id.as_deref() {
                    return Err(format!(
                        "Git import refuses to overwrite AIT line {line_name}: current head {} is not the last imported head {}.",
                        line.head_snapshot_id.as_deref().unwrap_or("none"),
                        expected.unwrap_or("none")
                    ));
                }
                line_store.compare_and_swap_line_head(
                    &line_name,
                    line.head_snapshot_id.as_deref(),
                    Some(snapshot_id),
                    &timestamp,
                )?
            }
        };
        let mapping = json!({
            "kind": "ref",
            "direction": "import",
            "created_at": system_event_timestamp(),
            "generation_id": source_info.generation_id,
            "source_repository_fingerprint": source_info.fingerprint,
            "git_object_format": source_info.object_format,
            "git_ref_name": head.source_name,
            "git_object_id": head.object_id,
            "ait_line_id": line.line_id,
            "ait_line_name": line.line_name,
            "snapshot_id": snapshot_id,
        });
        interop.write_mapping(mapping.clone())?;
        mappings.push(mapping);
        imported_lines.push(json!({
            "git_ref_name": head.source_name,
            "line_id": line.line_id,
            "line_name": line.line_name,
            "snapshot_id": snapshot_id,
        }));
        ref_index += 1;
        next_ref_index = ref_index;
        interop.write_operation(
            &operation_id,
            &operation_checkpoint(
                "import",
                &operation_id,
                &source_info.generation_id,
                &plan_hash,
                "running",
                next_commit_index,
                next_ref_index,
                None,
            ),
        )?;
    }

    let mut imported_tags = Vec::new();
    for tag in &tags {
        if ref_index < next_ref_index {
            ref_index += 1;
            continue;
        }
        let snapshot_id = git_to_snapshot.get(&tag.peeled_commit_id).ok_or_else(|| {
            format!(
                "Git tag {} points at commit {} without an imported Snapshot.",
                tag.source_ref, tag.peeled_commit_id
            )
        })?;
        let message_text = String::from_utf8_lossy(&tag.message_bytes);
        let message = message_text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Imported Git tag {}", tag.name));
        let existing = tag_store.tag_by_name(&tag.name)?;
        let record = match existing {
            Some(existing) if existing.snapshot_id == *snapshot_id => existing,
            Some(existing) => {
                return Err(format!(
                    "Git import refuses to replace AIT tag {} because Tag bindings are immutable: it points to {}, but {} points to {}.",
                    tag.name, existing.snapshot_id, tag.source_ref, snapshot_id
                ));
            }
            None => tag_store.create_tag(&new_tag_record(
                &tag.name,
                snapshot_id,
                &message,
                &timestamp,
            )?)?,
        };
        let mapping = json!({
            "kind": "tag",
            "direction": "import",
            "created_at": system_event_timestamp(),
            "generation_id": source_info.generation_id,
            "source_repository_fingerprint": source_info.fingerprint,
            "git_object_format": source_info.object_format,
            "git_ref_name": tag.source_ref,
            "git_object_id": tag.object_id,
            "git_object_type": tag.object_type,
            "peeled_commit_git_object_id": tag.peeled_commit_id,
            "snapshot_id": snapshot_id,
            "ait_tag_name": record.name,
            "message_base64": bytes_base64(&tag.message_bytes),
            "raw_tag_base64": bytes_base64(&tag.raw_bytes),
            "signed": tag.signed,
        });
        interop.write_mapping(mapping.clone())?;
        mappings.push(mapping);
        imported_tags.push(json!({
            "git_ref_name": tag.source_ref,
            "git_object_id": tag.object_id,
            "git_object_type": tag.object_type,
            "name": record.name,
            "snapshot_id": record.snapshot_id,
        }));
        ref_index += 1;
        next_ref_index = ref_index;
        interop.write_operation(
            &operation_id,
            &operation_checkpoint(
                "import",
                &operation_id,
                &source_info.generation_id,
                &plan_hash,
                "running",
                next_commit_index,
                next_ref_index,
                None,
            ),
        )?;
    }

    let imported_symbolic_head = source_info
        .head_symbolic_ref
        .as_deref()
        .and_then(|symbolic_ref| {
            selected_heads
                .iter()
                .find(|head| head.source_name == symbolic_ref)
        })
        .map(|head| {
            let line_name = head
                .source_name
                .strip_prefix("refs/heads/")
                .unwrap_or(head.source_name.as_str());
            json!({
                "kind": "symbolic_ref",
                "direction": "import",
                "created_at": system_event_timestamp(),
                "generation_id": source_info.generation_id,
                "source_repository_fingerprint": source_info.fingerprint,
                "git_object_format": source_info.object_format,
                "git_ref_name": "HEAD",
                "git_symbolic_target": head.source_name,
                "git_object_id": head.object_id,
                "ait_line_name": line_name,
            })
        });
    if let Some(mapping) = imported_symbolic_head.as_ref() {
        let (_, created) = interop.write_mapping(mapping.clone())?;
        if created {
            mappings.push(mapping.clone());
        }
    }

    let result = json!({
        "contract": INTEROP_CONTRACT,
        "operation": "import",
        "status": "completed",
        "operation_id": operation_id,
        "generation_id": source_info.generation_id,
        "plan_hash": plan_hash,
        "source": source_info.source_identity,
        "source_repository_fingerprint": source_info.fingerprint,
        "git_object_format": source_info.object_format,
        "head_symbolic_ref": source_info.head_symbolic_ref,
        "head_object_id": source_info.head_object_id,
        "symbolic_head_mapped": imported_symbolic_head.is_some(),
        "retained_repository": git_dir.to_string_lossy(),
        "commit_count": commits.len(),
        "imported_commit_count": imported_commit_count,
        "reused_commit_count": reused_commit_count,
        "line_count": imported_lines.len(),
        "tag_count": imported_tags.len(),
        "lines": imported_lines,
        "tags": imported_tags,
        "blockers": [],
        "classifications": classifications,
        "dry_run": false,
        "resume_supported": true,
        "mutated": true,
    });
    interop.write_operation(
        &operation_id,
        &operation_checkpoint(
            "import",
            &operation_id,
            &source_info.generation_id,
            &plan_hash,
            "completed",
            commits.len(),
            selected_heads.len() + tags.len(),
            Some(result.clone()),
        ),
    )?;
    Ok(result)
}

pub fn git_export(
    repo: &RepoRuntime,
    target: &str,
    all_lines_and_tags: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let target_info = inspect_target(target)?;
    if target_info.object_format != OBJECT_FORMAT_SHA1 {
        let report = json!({
            "contract": INTEROP_CONTRACT,
            "operation": "export",
            "status": "blocked",
            "target": target_info.path.to_string_lossy(),
            "target_repository_fingerprint": target_info.fingerprint,
            "git_object_format": target_info.object_format,
            "supported_object_formats": [OBJECT_FORMAT_SHA1],
            "blockers": [{
                "kind": "unsupported_object_format",
                "count": 1,
                "disposition": "fail_closed",
            }],
            "dry_run": dry_run,
            "mutated": false,
        });
        if dry_run {
            return Ok(report);
        }
        return Err(format!(
            "Git export blocked: target object format {:?} is unsupported; this build supports sha1 only.",
            target_info.object_format
        ));
    }
    let export_refs = selected_export_refs(repo, all_lines_and_tags)?;
    if export_refs.is_empty() {
        return Err("Git export selected no AIT lines or tags with Snapshot heads.".to_string());
    }
    let mut invalid_refs = Vec::new();
    for reference in &export_refs {
        if let Err(error) = run_git(
            ["check-ref-format", reference.git_ref_name.as_str()],
            &[],
            None,
        ) {
            invalid_refs.push(json!({
                "git_ref_name": reference.git_ref_name,
                "ait_kind": reference.ait_kind,
                "ait_name": reference.ait_name,
                "error": error,
            }));
        }
    }
    if !invalid_refs.is_empty() {
        if dry_run {
            return Ok(json!({
                "contract": INTEROP_CONTRACT,
                "operation": "export",
                "status": "blocked",
                "target": target_info.path.to_string_lossy(),
                "target_repository_fingerprint": target_info.fingerprint,
                "git_object_format": target_info.object_format,
                "blockers": [{
                    "kind": "invalid_git_ref_names",
                    "count": invalid_refs.len(),
                    "disposition": "fail_closed",
                    "entries": invalid_refs,
                }],
                "dry_run": true,
                "mutated": false,
            }));
        }
        return Err(format!(
            "Git export blocked by invalid ref names: {}",
            encode_value(
                &JsonValue::Array(invalid_refs),
                "Failed to encode Git ref blockers"
            )?
        ));
    }

    let snapshot_store = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    let head_snapshot_ids = export_refs
        .iter()
        .map(|reference| reference.snapshot_id.clone())
        .collect::<Vec<_>>();
    let traversal = snapshot_ancestor_closure(
        &snapshot_store,
        &head_snapshot_ids,
        &BTreeSet::new(),
        SnapshotParentMode::AllParents,
        SnapshotDagLimits::default(),
    )?;
    let interop = InteropStore::new(repo);
    let mut mappings = interop.load_mappings()?;
    let preferred_head_ref = preferred_export_head_ref(repo, &export_refs, &mappings);
    let exact_reuse_count = traversal
        .topological_snapshot_ids
        .iter()
        .filter(|snapshot_id| import_commit_mapping(&mappings, snapshot_id).is_some())
        .count();
    let native_commit_count = traversal
        .topological_snapshot_ids
        .len()
        .saturating_sub(exact_reuse_count);
    let refs_material = export_refs
        .iter()
        .map(|reference| {
            format!(
                "{} {} {} {}",
                reference.git_ref_name,
                reference.snapshot_id,
                reference.ait_kind,
                reference.ait_name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let plan_hash = sha256_prefixed(
        "GEP",
        format!(
            "{}\n{}\n{}\nHEAD symbolic={}\n{}",
            target_info.fingerprint,
            all_lines_and_tags,
            traversal.topological_snapshot_ids.join("\n"),
            preferred_head_ref.as_deref().unwrap_or("none"),
            refs_material
        )
        .as_bytes(),
        24,
    );
    let generation_id = sha256_prefixed(
        "GIT-EXP",
        format!("{}\n{plan_hash}", target_info.fingerprint).as_bytes(),
        16,
    );
    let operation_id = sha256_prefixed(
        "GIO-EXPORT",
        format!("{}\n{plan_hash}", target_info.fingerprint).as_bytes(),
        16,
    );
    if dry_run {
        return Ok(json!({
            "contract": INTEROP_CONTRACT,
            "operation": "export",
            "status": "dry_run",
            "operation_id": operation_id,
            "generation_id": generation_id,
            "plan_hash": plan_hash,
            "target": target_info.path.to_string_lossy(),
            "target_repository_fingerprint": target_info.fingerprint,
            "target_exists": target_info.existed,
            "target_bare": target_info.bare,
            "git_object_format": target_info.object_format,
            "snapshot_count": traversal.topological_snapshot_ids.len(),
            "exact_git_object_reuse_count": exact_reuse_count,
            "native_commit_count": native_commit_count,
            "ref_count": export_refs.len(),
            "refs": export_refs_json(&export_refs),
            "head_symbolic_ref": preferred_head_ref,
            "blockers": [],
            "dry_run": true,
            "resume_supported": true,
            "mutated": false,
        }));
    }

    let (completed, mut next_commit_index, mut next_ref_index) =
        existing_operation_result(&interop, &operation_id, &plan_hash)?;
    if let Some(mut result) = completed {
        if let Some(object) = result.as_object_mut() {
            object.insert("status".to_string(), JsonValue::String("no_op".to_string()));
            object.insert("replayed".to_string(), JsonValue::Bool(true));
        }
        return Ok(result);
    }
    interop.write_operation(
        &operation_id,
        &operation_checkpoint(
            "export",
            &operation_id,
            &generation_id,
            &plan_hash,
            "running",
            next_commit_index,
            next_ref_index,
            None,
        ),
    )?;

    let target_git_dir = prepare_export_target(&target_info)?;
    let generated_git_dir = interop.generated_repository();
    ensure_bare_repository(&generated_git_dir, OBJECT_FORMAT_SHA1)?;
    let ait_fingerprint = sha256_prefixed(
        "ASR",
        format!(
            "ait-repository-fingerprint/v1\n{}\n{}\n",
            repo.repo_name(),
            repo.authoritative_repo_root().to_string_lossy()
        )
        .as_bytes(),
        24,
    );
    let mut snapshot_to_git = BTreeMap::<String, String>::new();
    let mut fetched_import_generations = BTreeSet::new();
    for (index, snapshot_id) in traversal.topological_snapshot_ids.iter().enumerate() {
        if index < next_commit_index {
            let mapping = preferred_commit_mapping(&mappings, snapshot_id).ok_or_else(|| {
                format!(
                    "Git export checkpoint skipped Snapshot {snapshot_id} without a persisted mapping."
                )
            })?;
            let object_id = json_text(mapping, "git_object_id").ok_or_else(|| {
                format!("Git mapping for {snapshot_id} is missing git_object_id.")
            })?;
            snapshot_to_git.insert(snapshot_id.clone(), object_id.to_string());
            continue;
        }
        let parent_git_object_ids = traversal
            .parent_snapshot_ids
            .get(snapshot_id)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|parent| {
                snapshot_to_git.get(parent).cloned().ok_or_else(|| {
                    format!(
                        "Git export lost parent object mapping for Snapshot {parent} before {snapshot_id}."
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let git_object_id = if let Some(mapping) = import_commit_mapping(&mappings, snapshot_id) {
            ensure_import_mapping_in_generated(
                &interop,
                &generated_git_dir,
                mapping,
                snapshot_id,
                &mut fetched_import_generations,
            )?
        } else {
            let (object_id, mapping) = build_native_git_commit(
                repo,
                &snapshot_store,
                &generated_git_dir,
                snapshot_id,
                &parent_git_object_ids,
                &generation_id,
                &ait_fingerprint,
            )?;
            let (_, created) = interop.write_mapping(mapping.clone())?;
            if created {
                mappings.push(mapping);
            }
            object_id
        };
        let cache_ref = generated_snapshot_ref(snapshot_id);
        git_update_ref(&generated_git_dir, &cache_ref, &git_object_id, None)?;
        snapshot_to_git.insert(snapshot_id.clone(), git_object_id);
        next_commit_index = index + 1;
        interop.write_operation(
            &operation_id,
            &operation_checkpoint(
                "export",
                &operation_id,
                &generation_id,
                &plan_hash,
                "running",
                next_commit_index,
                next_ref_index,
                None,
            ),
        )?;
    }

    let mut exported_refs = Vec::new();
    let mut staging_refs = Vec::new();
    for (index, reference) in export_refs.iter().enumerate() {
        if index < next_ref_index {
            continue;
        }
        let snapshot_commit_id = snapshot_to_git
            .get(&reference.snapshot_id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Git export ref {} lost Snapshot mapping {}.",
                    reference.git_ref_name, reference.snapshot_id
                )
            })?;
        let (desired_object_id, cache_ref, object_type) = if reference.ait_kind == "tag" {
            export_tag_object(
                &interop,
                &mut mappings,
                &generated_git_dir,
                &mut fetched_import_generations,
                reference,
                &snapshot_commit_id,
                &generation_id,
                &ait_fingerprint,
            )?
        } else {
            (
                snapshot_commit_id,
                generated_snapshot_ref(&reference.snapshot_id),
                "commit".to_string(),
            )
        };
        let staging_ref = format!(
            "refs/ait/transfer/{}/{}",
            operation_id.to_ascii_lowercase(),
            index
        );
        fetch_cache_ref(
            &target_git_dir,
            &generated_git_dir,
            &cache_ref,
            &staging_ref,
        )?;
        staging_refs.push(staging_ref.clone());
        let current = git_ref_object_id(&target_git_dir, &reference.git_ref_name)?;
        let previous = latest_mapping(mappings.iter().filter(|row| {
            json_text(row, "kind") == Some("ref")
                && json_text(row, "direction") == Some("export")
                && json_text(row, "target_repository_fingerprint")
                    == Some(target_info.fingerprint.as_str())
                && json_text(row, "git_ref_name") == Some(reference.git_ref_name.as_str())
        }));
        let expected = previous.and_then(|row| json_text(row, "git_object_id"));
        match current.as_deref() {
            Some(current) if current == desired_object_id => {}
            None => git_update_ref(
                &target_git_dir,
                &reference.git_ref_name,
                &desired_object_id,
                Some(ZERO_SHA1),
            )?,
            Some(current) if expected == Some(current) => git_update_ref(
                &target_git_dir,
                &reference.git_ref_name,
                &desired_object_id,
                Some(current),
            )?,
            Some(current) => {
                return Err(format!(
                    "Git export refuses to overwrite {}: target is at {}, but the last exported object is {}.",
                    reference.git_ref_name,
                    current,
                    expected.unwrap_or("none")
                ))
            }
        }
        let mapping = json!({
            "kind": "ref",
            "direction": "export",
            "created_at": system_event_timestamp(),
            "generation_id": generation_id,
            "source_repository_fingerprint": ait_fingerprint,
            "target_repository_fingerprint": target_info.fingerprint,
            "git_object_format": OBJECT_FORMAT_SHA1,
            "git_ref_name": reference.git_ref_name,
            "git_object_id": desired_object_id,
            "git_object_type": object_type,
            "snapshot_id": reference.snapshot_id,
            "ait_kind": reference.ait_kind,
            "ait_name": reference.ait_name,
            "ait_identity": reference.ait_identity,
            "expected_previous_git_object_id": current,
        });
        interop.write_mapping(mapping.clone())?;
        mappings.push(mapping);
        exported_refs.push(json!({
            "git_ref_name": reference.git_ref_name,
            "git_object_id": desired_object_id,
            "git_object_type": object_type,
            "snapshot_id": reference.snapshot_id,
            "ait_kind": reference.ait_kind,
            "ait_name": reference.ait_name,
        }));
        next_ref_index = index + 1;
        interop.write_operation(
            &operation_id,
            &operation_checkpoint(
                "export",
                &operation_id,
                &generation_id,
                &plan_hash,
                "running",
                next_commit_index,
                next_ref_index,
                None,
            ),
        )?;
    }

    let exported_head = if !target_info.existed {
        preferred_head_ref.as_deref().and_then(|preferred| {
            export_refs
                .iter()
                .find(|reference| reference.git_ref_name == preferred)
        })
    } else {
        None
    };
    if let Some(branch) = exported_head {
        git_repo_bytes(
            &target_git_dir,
            ["symbolic-ref", "HEAD", branch.git_ref_name.as_str()],
        )?;
    }
    git_repo_bytes(&target_git_dir, ["fsck", "--full", "--no-dangling"])?;
    for staging_ref in &staging_refs {
        let _ = git_repo_bytes(&target_git_dir, ["update-ref", "-d", staging_ref.as_str()]);
    }
    let result = json!({
        "contract": INTEROP_CONTRACT,
        "operation": "export",
        "status": "completed",
        "operation_id": operation_id,
        "generation_id": generation_id,
        "plan_hash": plan_hash,
        "target": target_info.path.to_string_lossy(),
        "target_git_dir": target_git_dir.to_string_lossy(),
        "target_repository_fingerprint": target_info.fingerprint,
        "target_created": !target_info.existed,
        "target_bare": if target_info.existed { target_info.bare } else { true },
        "git_object_format": OBJECT_FORMAT_SHA1,
        "snapshot_count": traversal.topological_snapshot_ids.len(),
        "exact_git_object_reuse_count": exact_reuse_count,
        "native_commit_count": native_commit_count,
        "ref_count": exported_refs.len(),
        "refs": exported_refs,
        "head_symbolic_ref": exported_head.map(|branch| branch.git_ref_name.as_str()),
        "compare_and_swap": true,
        "force_updated": false,
        "fsck": "passed",
        "working_tree_updated": false,
        "dry_run": false,
        "resume_supported": true,
        "mutated": true,
    });
    interop.write_operation(
        &operation_id,
        &operation_checkpoint(
            "export",
            &operation_id,
            &generation_id,
            &plan_hash,
            "completed",
            traversal.topological_snapshot_ids.len(),
            export_refs.len(),
            Some(result.clone()),
        ),
    )?;
    Ok(result)
}

fn inspect_target(target: &str) -> Result<TargetInfo, String> {
    let requested = target.trim();
    if requested.is_empty() {
        return Err("git target must not be empty.".to_string());
    }
    if requested.starts_with('-') {
        return Err("git target must not begin with '-'".to_string());
    }
    let raw_path = PathBuf::from(requested);
    let path = if raw_path.is_absolute() {
        raw_path
    } else {
        env::current_dir()
            .map_err(|error| format!("Failed to resolve current directory: {error}"))?
            .join(raw_path)
    };
    let (git_dir, object_format, bare, existed) = if path.exists() {
        let git_dir_text = String::from_utf8(run_git(
            [
                "-C",
                path.to_string_lossy().as_ref(),
                "rev-parse",
                "--absolute-git-dir",
            ],
            &[],
            None,
        )?)
        .map_err(|_| "Git target path metadata was not UTF-8.".to_string())?;
        let git_dir = PathBuf::from(git_dir_text.trim())
            .canonicalize()
            .map_err(|error| format!("Failed to resolve target Git directory: {error}"))?;
        let object_format = git_repo_text(&git_dir, ["rev-parse", "--show-object-format"])?
            .trim()
            .to_string();
        let bare = git_repo_text(&git_dir, ["rev-parse", "--is-bare-repository"])?.trim() == "true";
        (git_dir, object_format, bare, true)
    } else {
        let parent = path.parent().ok_or_else(|| {
            format!(
                "Git target {} does not have a resolvable parent.",
                path.display()
            )
        })?;
        let resolved_parent = parent.canonicalize().map_err(|error| {
            format!(
                "Git target parent {} must exist and be resolvable: {error}",
                parent.display()
            )
        })?;
        let file_name = path.file_name().ok_or_else(|| {
            format!(
                "Git target {} is missing its final path component.",
                path.display()
            )
        })?;
        let resolved = resolved_parent.join(file_name);
        (
            resolved.clone(),
            OBJECT_FORMAT_SHA1.to_string(),
            true,
            false,
        )
    };
    let canonical_path = if existed {
        path.canonicalize()
            .map_err(|error| format!("Failed to resolve Git target {}: {error}", path.display()))?
    } else {
        git_dir.clone()
    };
    let fingerprint = sha256_prefixed(
        "GTR",
        format!(
            "git-target-fingerprint/v1\n{object_format}\n{}\n",
            canonical_path.to_string_lossy()
        )
        .as_bytes(),
        24,
    );
    Ok(TargetInfo {
        requested: requested.to_string(),
        path: canonical_path,
        git_dir,
        fingerprint,
        object_format,
        existed,
        bare,
    })
}

fn prepare_export_target(target: &TargetInfo) -> Result<PathBuf, String> {
    if !target.existed {
        ensure_bare_repository(&target.path, OBJECT_FORMAT_SHA1)?;
        return target.path.canonicalize().map_err(|error| {
            format!(
                "Failed to resolve newly created Git target {}: {error}",
                target.path.display()
            )
        });
    }
    let format = git_repo_text(&target.git_dir, ["rev-parse", "--show-object-format"])?;
    if format.trim() != OBJECT_FORMAT_SHA1 {
        return Err(format!(
            "Git target {} uses unsupported object format {}.",
            target.requested,
            format.trim()
        ));
    }
    Ok(target.git_dir.clone())
}

fn selected_export_refs(
    repo: &RepoRuntime,
    all_lines_and_tags: bool,
) -> Result<Vec<ExportRef>, String> {
    let line_store = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .lines();
    let mut refs = Vec::new();
    if all_lines_and_tags {
        for line in line_store.list_lines()? {
            let Some(snapshot_id) = line.head_snapshot_id else {
                continue;
            };
            refs.push(ExportRef {
                git_ref_name: format!("refs/heads/{}", line.line_name),
                snapshot_id,
                ait_kind: "line".to_string(),
                ait_name: line.line_name,
                ait_identity: Some(line.line_id),
                message: None,
                created_at: line.created_at,
            });
        }
        let tags =
            FilesystemTagStore::new(repo.authoritative_repo_root().to_string_lossy().as_ref())?
                .list_tags()?;
        for tag in tags {
            refs.push(ExportRef {
                git_ref_name: format!("refs/tags/{}", tag.name),
                snapshot_id: tag.snapshot_id,
                ait_kind: "tag".to_string(),
                ait_name: tag.name,
                ait_identity: None,
                message: Some(tag.message),
                created_at: Some(tag.created_at),
            });
        }
    } else {
        let line_name = repo.current_line_name()?;
        let line = line_store
            .line_by_name(&line_name)?
            .ok_or_else(|| format!("Current AIT line does not exist: {line_name}"))?;
        let snapshot_id = line
            .head_snapshot_id
            .ok_or_else(|| format!("Current AIT line {line_name} has no Snapshot to export."))?;
        refs.push(ExportRef {
            git_ref_name: format!("refs/heads/{line_name}"),
            snapshot_id,
            ait_kind: "line".to_string(),
            ait_name: line.line_name,
            ait_identity: Some(line.line_id),
            message: None,
            created_at: line.created_at,
        });
    }
    refs.sort_by(|left, right| left.git_ref_name.cmp(&right.git_ref_name));
    Ok(refs)
}

fn preferred_export_head_ref(
    repo: &RepoRuntime,
    refs: &[ExportRef],
    mappings: &[JsonValue],
) -> Option<String> {
    let imported_source_fingerprints = refs
        .iter()
        .filter(|reference| reference.ait_kind == "line")
        .filter_map(|reference| import_commit_mapping(mappings, &reference.snapshot_id))
        .filter_map(|mapping| json_text(mapping, "source_repository_fingerprint"))
        .collect::<BTreeSet<_>>();
    let imported_head = latest_mapping(mappings.iter().filter(|mapping| {
        if json_text(mapping, "kind") != Some("symbolic_ref")
            || json_text(mapping, "direction") != Some("import")
        {
            return false;
        }
        let Some(fingerprint) = json_text(mapping, "source_repository_fingerprint") else {
            return false;
        };
        if !imported_source_fingerprints.contains(fingerprint) {
            return false;
        }
        let Some(target) = json_text(mapping, "git_symbolic_target") else {
            return false;
        };
        refs.iter()
            .any(|reference| reference.ait_kind == "line" && reference.git_ref_name == target)
    }))
    .and_then(|mapping| json_text(mapping, "git_symbolic_target"))
    .map(str::to_string);
    if imported_head.is_some() {
        return imported_head;
    }

    let current_line_name = repo.current_line_name().ok();
    refs.iter()
        .find(|reference| {
            reference.ait_kind == "line"
                && current_line_name.as_deref() == Some(reference.ait_name.as_str())
        })
        .or_else(|| refs.iter().find(|reference| reference.ait_kind == "line"))
        .map(|reference| reference.git_ref_name.clone())
}

fn export_refs_json(refs: &[ExportRef]) -> JsonValue {
    JsonValue::Array(
        refs.iter()
            .map(|reference| {
                json!({
                    "git_ref_name": reference.git_ref_name,
                    "snapshot_id": reference.snapshot_id,
                    "ait_kind": reference.ait_kind,
                    "ait_name": reference.ait_name,
                    "ait_identity": reference.ait_identity,
                })
            })
            .collect(),
    )
}

fn import_commit_mapping<'a>(
    mappings: &'a [JsonValue],
    snapshot_id: &str,
) -> Option<&'a JsonValue> {
    latest_mapping(mappings.iter().filter(|row| {
        json_text(row, "kind") == Some("commit")
            && json_text(row, "direction") == Some("import")
            && json_text(row, "snapshot_id") == Some(snapshot_id)
            && json_text(row, "git_object_format") == Some(OBJECT_FORMAT_SHA1)
    }))
}

fn preferred_commit_mapping<'a>(
    mappings: &'a [JsonValue],
    snapshot_id: &str,
) -> Option<&'a JsonValue> {
    import_commit_mapping(mappings, snapshot_id).or_else(|| {
        latest_mapping(mappings.iter().filter(|row| {
            json_text(row, "kind") == Some("commit")
                && json_text(row, "direction") == Some("export")
                && json_text(row, "snapshot_id") == Some(snapshot_id)
                && json_text(row, "git_object_format") == Some(OBJECT_FORMAT_SHA1)
        }))
    })
}

fn generated_snapshot_ref(snapshot_id: &str) -> String {
    format!("refs/ait/snapshots/{}", snapshot_id.to_ascii_lowercase())
}

fn generated_tag_ref(name: &str) -> String {
    format!(
        "refs/ait/tags/{}",
        sha256_prefixed("tag", name.as_bytes(), 20).to_ascii_lowercase()
    )
}

fn ensure_import_mapping_in_generated(
    interop: &InteropStore,
    generated_git_dir: &Path,
    mapping: &JsonValue,
    snapshot_id: &str,
    fetched_generations: &mut BTreeSet<(String, String)>,
) -> Result<String, String> {
    let source_fingerprint =
        json_text(mapping, "source_repository_fingerprint").ok_or_else(|| {
            format!("Imported mapping for {snapshot_id} is missing source fingerprint.")
        })?;
    let generation_id = json_text(mapping, "generation_id")
        .ok_or_else(|| format!("Imported mapping for {snapshot_id} is missing generation_id."))?;
    let object_id = json_text(mapping, "git_object_id")
        .ok_or_else(|| format!("Imported mapping for {snapshot_id} is missing git_object_id."))?
        .to_string();
    let key = (source_fingerprint.to_string(), generation_id.to_string());
    if !fetched_generations.contains(&key) {
        let source_git_dir = interop.retained_repository(source_fingerprint);
        if !source_git_dir.exists() {
            return Err(format!(
                "Retained Git object store for imported Snapshot {snapshot_id} is missing: {}.",
                source_git_dir.display()
            ));
        }
        let source_prefix = format!("refs/ait/import/{}/", generation_id.to_ascii_lowercase());
        let destination_prefix = format!(
            "refs/ait/import-cache/{}/{}/",
            source_fingerprint.to_ascii_lowercase(),
            generation_id.to_ascii_lowercase()
        );
        let refspec = format!("+{}*:{}*", source_prefix, destination_prefix);
        let fetch = git_repo_bytes_os(
            generated_git_dir,
            vec![
                OsString::from("fetch"),
                OsString::from("--no-tags"),
                OsString::from("--no-write-fetch-head"),
                source_git_dir.as_os_str().to_os_string(),
                OsString::from(refspec),
            ],
            &[],
            None,
        );
        if let Err(prefix_error) = fetch {
            let cache_ref = generated_snapshot_ref(snapshot_id);
            git_repo_bytes_os(
                generated_git_dir,
                vec![
                    OsString::from("fetch"),
                    OsString::from("--no-tags"),
                    OsString::from("--no-write-fetch-head"),
                    source_git_dir.as_os_str().to_os_string(),
                    OsString::from(format!("+{object_id}:{cache_ref}")),
                ],
                &[],
                None,
            )
            .map_err(|fallback_error| {
                format!(
                    "Failed to restore imported Git generation {generation_id}: {prefix_error}; exact-object fallback also failed: {fallback_error}"
                )
            })?;
        }
        fetched_generations.insert(key);
    }
    git_repo_bytes(
        generated_git_dir,
        ["cat-file", "-e", &format!("{object_id}^{{commit}}")],
    )?;
    git_update_ref(
        generated_git_dir,
        &generated_snapshot_ref(snapshot_id),
        &object_id,
        None,
    )?;
    Ok(object_id)
}

fn build_native_git_commit<S>(
    _repo: &RepoRuntime,
    snapshot_store: &S,
    generated_git_dir: &Path,
    snapshot_id: &str,
    parent_git_object_ids: &[String],
    generation_id: &str,
    ait_fingerprint: &str,
) -> Result<(String, JsonValue), String>
where
    S: SnapshotStore + LocalSnapshotTreeReadStore + LocalSnapshotBlobReadStore + ?Sized,
{
    let snapshot = snapshot_store
        .snapshot_by_id(snapshot_id)?
        .ok_or_else(|| format!("Unknown AIT Snapshot: {snapshot_id}"))?;
    let rows = snapshot_store.snapshot_tree_file_rows(Some(snapshot_id))?;
    let temp = TempBuilder::new()
        .prefix("ait-git-export-index-")
        .tempdir()
        .map_err(|error| format!("Failed to create Git export index directory: {error}"))?;
    let index_path = temp.path().join("index");
    let env = vec![(
        OsString::from("GIT_INDEX_FILE"),
        index_path.as_os_str().to_os_string(),
    )];
    git_repo_bytes_os(
        generated_git_dir,
        vec![OsString::from("read-tree"), OsString::from("--empty")],
        &env,
        None,
    )?;
    let mut file_modes = Vec::new();
    for row in &rows {
        let bytes = snapshot_store.read_blob_bytes(&row.blob_id)?;
        let blob_object_id = String::from_utf8(git_repo_bytes_os(
            generated_git_dir,
            vec![
                OsString::from("hash-object"),
                OsString::from("-w"),
                OsString::from("--stdin"),
            ],
            &[],
            Some(&bytes),
        )?)
        .map_err(|_| "git hash-object returned non-UTF-8 object ID.".to_string())?
        .trim()
        .to_string();
        let git_mode = snapshot_mode_to_git_mode(&row.mode)?;
        git_repo_bytes_os(
            generated_git_dir,
            vec![
                OsString::from("update-index"),
                OsString::from("--add"),
                OsString::from("--cacheinfo"),
                OsString::from(format!("{git_mode},{blob_object_id},{}", row.path)),
            ],
            &env,
            None,
        )?;
        file_modes.push(json!({
            "path": row.path,
            "ait_mode": row.mode,
            "git_mode": git_mode,
            "ait_blob_id": row.blob_id,
            "git_object_id": blob_object_id,
        }));
    }
    let tree_object_id = String::from_utf8(git_repo_bytes_os(
        generated_git_dir,
        vec![OsString::from("write-tree")],
        &env,
        None,
    )?)
    .map_err(|_| "git write-tree returned non-UTF-8 object ID.".to_string())?
    .trim()
    .to_string();
    let date = git_date(&snapshot.created_at)?;
    let identity_env = git_identity_env(&date);
    let mut args = vec![
        OsString::from("commit-tree"),
        OsString::from(&tree_object_id),
    ];
    for parent in parent_git_object_ids {
        args.push(OsString::from("-p"));
        args.push(OsString::from(parent));
    }
    let mut message = snapshot
        .message
        .clone()
        .unwrap_or_else(|| format!("AIT Snapshot {snapshot_id}"))
        .into_bytes();
    if !message.ends_with(b"\n") {
        message.push(b'\n');
    }
    let object_id = String::from_utf8(git_repo_bytes_os(
        generated_git_dir,
        args,
        &identity_env,
        Some(&message),
    )?)
    .map_err(|_| "git commit-tree returned non-UTF-8 object ID.".to_string())?
    .trim()
    .to_string();
    let raw_commit = git_repo_bytes(generated_git_dir, ["cat-file", "commit", &object_id])?;
    let locator = snapshot_store.snapshot_tree_root_locator(snapshot_id)?;
    let mapping = json!({
        "kind": "commit",
        "direction": "export",
        "created_at": system_event_timestamp(),
        "generation_id": generation_id,
        "source_repository_fingerprint": ait_fingerprint,
        "git_object_format": OBJECT_FORMAT_SHA1,
        "git_object_id": object_id,
        "git_tree_object_id": tree_object_id,
        "snapshot_id": snapshot_id,
        "ait_root_tree_id": locator.root_tree_id,
        "parent_git_object_ids": strings_json(parent_git_object_ids),
        "parent_snapshot_ids": strings_json(&snapshot.parent_snapshot_ids),
        "author": {
            "name": "AIT Snapshot",
            "email": "ait@local",
            "timestamp": date.split_whitespace().next().unwrap_or_default(),
            "timezone": "+0000",
        },
        "committer": {
            "name": "AIT Snapshot",
            "email": "ait@local",
            "timestamp": date.split_whitespace().next().unwrap_or_default(),
            "timezone": "+0000",
        },
        "message_base64": bytes_base64(&message),
        "raw_commit_base64": bytes_base64(&raw_commit),
        "file_modes": file_modes,
        "imported_unchanged": false,
        "deterministic": true,
    });
    Ok((object_id, mapping))
}

fn snapshot_mode_to_git_mode(mode: &str) -> Result<&'static str, String> {
    let normalized = mode.trim().trim_start_matches("0o");
    let bits = u32::from_str_radix(normalized, 8)
        .map_err(|_| format!("Invalid AIT Snapshot mode {mode:?}."))?;
    if bits & 0o170000 == 0o120000 {
        return Ok("120000");
    }
    if bits & 0o170000 != 0 && bits & 0o170000 != 0o100000 {
        return Err(format!(
            "Unsupported AIT Snapshot file type in mode {mode:?}."
        ));
    }
    Ok(if bits & 0o111 != 0 {
        "100755"
    } else {
        "100644"
    })
}

fn git_date(created_at: &str) -> Result<String, String> {
    let trimmed = created_at.trim();
    let timestamp = match trimmed.parse::<i64>() {
        Ok(epoch_seconds) => DateTime::<Utc>::from_timestamp(epoch_seconds, 0)
            .ok_or_else(|| format!("AIT Snapshot epoch timestamp {created_at:?} is out of range."))?
            .timestamp(),
        Err(_) => DateTime::parse_from_rfc3339(trimmed)
            .map_err(|error| {
                format!(
                    "AIT Snapshot timestamp {created_at:?} is neither epoch seconds nor RFC3339: {error}"
                )
            })?
            .timestamp(),
    };
    Ok(format!("{timestamp} +0000"))
}

fn git_identity_env(date: &str) -> Vec<(OsString, OsString)> {
    [
        ("GIT_AUTHOR_NAME", "AIT Snapshot"),
        ("GIT_AUTHOR_EMAIL", "ait@local"),
        ("GIT_AUTHOR_DATE", date),
        ("GIT_COMMITTER_NAME", "AIT Snapshot"),
        ("GIT_COMMITTER_EMAIL", "ait@local"),
        ("GIT_COMMITTER_DATE", date),
    ]
    .into_iter()
    .map(|(key, value)| (OsString::from(key), OsString::from(value)))
    .collect()
}

#[expect(
    clippy::too_many_arguments,
    reason = "tag export keeps repository, generation, and identity inputs explicit"
)]
fn export_tag_object(
    interop: &InteropStore,
    mappings: &mut Vec<JsonValue>,
    generated_git_dir: &Path,
    fetched_generations: &mut BTreeSet<(String, String)>,
    reference: &ExportRef,
    snapshot_commit_id: &str,
    generation_id: &str,
    ait_fingerprint: &str,
) -> Result<(String, String, String), String> {
    if let Some(mapping) = latest_mapping(mappings.iter().filter(|row| {
        json_text(row, "kind") == Some("tag")
            && json_text(row, "direction") == Some("import")
            && json_text(row, "ait_tag_name") == Some(reference.ait_name.as_str())
            && json_text(row, "snapshot_id") == Some(reference.snapshot_id.as_str())
    })) {
        let object_id = json_text(mapping, "git_object_id")
            .ok_or_else(|| {
                format!(
                    "Imported tag {} is missing git_object_id.",
                    reference.ait_name
                )
            })?
            .to_string();
        let object_type = json_text(mapping, "git_object_type")
            .unwrap_or("commit")
            .to_string();
        if object_type == "tag" {
            let source_fingerprint = json_text(mapping, "source_repository_fingerprint")
                .ok_or_else(|| {
                    format!(
                        "Imported tag {} is missing source fingerprint.",
                        reference.ait_name
                    )
                })?;
            let source_git_dir = interop.retained_repository(source_fingerprint);
            let generation = json_text(mapping, "generation_id").ok_or_else(|| {
                format!(
                    "Imported tag {} is missing generation_id.",
                    reference.ait_name
                )
            })?;
            let key = (source_fingerprint.to_string(), generation.to_string());
            if !fetched_generations.contains(&key) {
                let dummy_mapping = json!({
                    "source_repository_fingerprint": source_fingerprint,
                    "generation_id": generation,
                    "git_object_id": snapshot_commit_id,
                });
                ensure_import_mapping_in_generated(
                    interop,
                    generated_git_dir,
                    &dummy_mapping,
                    &reference.snapshot_id,
                    fetched_generations,
                )?;
            }
            git_repo_bytes(
                generated_git_dir,
                ["cat-file", "-e", &format!("{object_id}^{{tag}}")],
            )
            .or_else(|_| {
                git_repo_bytes_os(
                    generated_git_dir,
                    vec![
                        OsString::from("fetch"),
                        OsString::from("--no-tags"),
                        OsString::from("--no-write-fetch-head"),
                        source_git_dir.as_os_str().to_os_string(),
                        OsString::from(format!(
                            "+{object_id}:{}",
                            generated_tag_ref(&reference.ait_name)
                        )),
                    ],
                    &[],
                    None,
                )
            })?;
        }
        let cache_ref = generated_tag_ref(&reference.ait_name);
        git_update_ref(generated_git_dir, &cache_ref, &object_id, None)?;
        return Ok((object_id, cache_ref, object_type));
    }
    let date = git_date(
        reference
            .created_at
            .as_deref()
            .unwrap_or("1970-01-01T00:00:00Z"),
    )?;
    let message = reference
        .message
        .as_deref()
        .unwrap_or(reference.ait_name.as_str());
    let tag_object = format!(
        "object {snapshot_commit_id}\ntype commit\ntag {}\ntagger AIT Snapshot <ait@local> {date}\n\n{}\n",
        reference.ait_name, message
    );
    let object_id = String::from_utf8(git_repo_bytes_os(
        generated_git_dir,
        vec![OsString::from("mktag")],
        &[],
        Some(tag_object.as_bytes()),
    )?)
    .map_err(|_| "git mktag returned non-UTF-8 object ID.".to_string())?
    .trim()
    .to_string();
    let cache_ref = generated_tag_ref(&reference.ait_name);
    git_update_ref(generated_git_dir, &cache_ref, &object_id, None)?;
    let mapping = json!({
        "kind": "tag",
        "direction": "export",
        "created_at": system_event_timestamp(),
        "generation_id": generation_id,
        "source_repository_fingerprint": ait_fingerprint,
        "git_object_format": OBJECT_FORMAT_SHA1,
        "git_ref_name": reference.git_ref_name,
        "git_object_id": object_id,
        "git_object_type": "tag",
        "peeled_commit_git_object_id": snapshot_commit_id,
        "snapshot_id": reference.snapshot_id,
        "ait_tag_name": reference.ait_name,
        "message_base64": bytes_base64(message.as_bytes()),
        "raw_tag_base64": bytes_base64(tag_object.as_bytes()),
        "deterministic": true,
    });
    let (_, created) = interop.write_mapping(mapping.clone())?;
    if created {
        mappings.push(mapping);
    }
    Ok((object_id, cache_ref, "tag".to_string()))
}

fn fetch_cache_ref(
    target_git_dir: &Path,
    generated_git_dir: &Path,
    source_ref: &str,
    destination_ref: &str,
) -> Result<(), String> {
    git_repo_bytes_os(
        target_git_dir,
        vec![
            OsString::from("fetch"),
            OsString::from("--no-tags"),
            OsString::from("--no-write-fetch-head"),
            generated_git_dir.as_os_str().to_os_string(),
            OsString::from(format!("+{source_ref}:{destination_ref}")),
        ],
        &[],
        None,
    )?;
    Ok(())
}

fn git_ref_object_id(git_dir: &Path, reference: &str) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", git_dir.display()))
        .args(["rev-parse", "--verify", "--quiet", reference])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("Failed to read Git ref {reference}: {error}"))?;
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map(|text| Some(text.trim().to_string()))
            .map_err(|_| format!("Git ref {reference} returned a non-UTF-8 object ID."));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(format!(
        "Failed to read Git ref {reference}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn git_update_ref(
    git_dir: &Path,
    reference: &str,
    new_object_id: &str,
    expected_object_id: Option<&str>,
) -> Result<(), String> {
    let mut args = vec!["update-ref", reference, new_object_id];
    if let Some(expected) = expected_object_id {
        args.push(expected);
    }
    git_repo_bytes(git_dir, args)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_mapping_identity_ignores_observation_time_and_deduplicates_resume() {
        let temp = TempDir::new().unwrap();
        let store = InteropStore {
            root: temp.path().join("interop"),
        };
        let first = json!({
            "kind": "ref",
            "direction": "export",
            "generation_id": "GIT-EXP-1",
            "git_ref_name": "refs/heads/main",
            "git_object_id": "1111111111111111111111111111111111111111",
            "created_at": "2026-07-19T00:00:00Z",
        });
        let second = json!({
            "kind": "ref",
            "direction": "export",
            "generation_id": "GIT-EXP-1",
            "git_ref_name": "refs/heads/main",
            "git_object_id": "1111111111111111111111111111111111111111",
            "created_at": "2026-07-19T00:00:01Z",
        });

        let (first_id, first_created) = store.write_mapping(first).unwrap();
        let (second_id, second_created) = store.write_mapping(second).unwrap();
        assert!(first_created);
        assert!(!second_created);
        assert_eq!(first_id, second_id);
        assert_eq!(store.load_mappings().unwrap().len(), 1);
    }

    #[test]
    fn latest_mapping_uses_nanosecond_record_order_before_second_precision_time() {
        let older = json!({
            "record_id": "GIM-Z",
            "created_at": "2026-07-19T00:00:00Z",
            "recorded_at_unix_nanos": "100",
        });
        let newer = json!({
            "record_id": "GIM-A",
            "created_at": "2026-07-19T00:00:00Z",
            "recorded_at_unix_nanos": "101",
        });
        assert_eq!(
            latest_mapping([&newer, &older]),
            Some(&newer),
            "record ID ordering must not override actual write order"
        );
    }
}
