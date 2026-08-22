use crate::json_support::encode_value_pretty_with_newline_error_string;
use crate::runtime::{RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT};
use ait_core::json_support::{json, JsonValue};
use ait_core::line_store::{LineRecord, LineStore};
use ait_core::local_snapshot::{LocalSnapshotTreeReadStore, SnapshotPathDelta};
use ait_core::snapshot_store::{SnapshotRecord, SnapshotStore};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use clap::{error::ErrorKind, Parser};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{Builder, NamedTempFile};

const DEFAULT_HISTORY_LIMIT: usize = 200;
const MAX_HISTORY_LIMIT: usize = 5_000;
const MAX_LINE_ROWS: usize = 1_000;
const MAX_DIFF_PRELOAD_SNAPSHOTS: usize = 10;
const MAX_CHANGED_PATHS_PER_SNAPSHOT: usize = 200;
const AITK_PAYLOAD_SCHEMA: &str = "aitk-history/v1";
const AITK_TK_SCRIPT: &str = include_str!("aitk_surface/aitk.tcl");

#[derive(Clone, Debug, Parser)]
#[command(
    name = "aitk",
    version,
    about = "Browse AIT Snapshot history and Line health in the current repository",
    long_about = "Browse AIT Snapshot history and Line health in a local read-only Tcl/Tk window.\n\nRun aitk inside the target AIT repository, like gitk, or use -C <path>. The repository is never modified."
)]
struct AitkArgs {
    /// Run as if aitk was started in this directory
    #[arg(short = 'C', value_name = "PATH")]
    directory: Option<PathBuf>,

    /// Emit the versioned history payload without opening a window
    #[arg(long)]
    json_only: bool,

    /// Also write the JSON payload to this file
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Maximum number of Snapshot rows to load
    #[arg(long, default_value_t = DEFAULT_HISTORY_LIMIT, value_name = "COUNT")]
    limit: usize,

    /// Tcl/Tk wish executable or path
    #[arg(long, value_name = "COMMAND")]
    wish: Option<OsString>,

    /// Internal bounded transport used by the embedded UI for one parent diff
    #[arg(long, hide = true, value_name = "SNAPSHOT_ID")]
    ui_diff_tsv: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LineHealth {
    state: &'static str,
    ahead_by: Option<usize>,
    behind_by: Option<usize>,
    head_present: bool,
}

pub fn entry() -> u8 {
    entry_with_args(env::args_os().collect())
}

pub fn entry_with_args(args: Vec<OsString>) -> u8 {
    let args = match AitkArgs::try_parse_from(args) {
        Ok(args) => args,
        Err(error) => {
            let kind = error.kind();
            let _ = error.print();
            return if matches!(kind, ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                0
            } else {
                2
            };
        }
    };
    match run(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("aitk: {error}");
            1
        }
    }
}

fn run(args: AitkArgs) -> Result<(), String> {
    validate_limit(args.limit)?;
    let start = match args.directory {
        Some(path) => path,
        None => {
            env::current_dir().map_err(|error| format!("cannot read current directory: {error}"))?
        }
    };
    if !start.is_dir() {
        return Err(format!("-C target is not a directory: {}", start.display()));
    }
    let repo = RepoRuntime::discover_from_path(&start).map_err(|error| {
        format!(
            "no initialized AIT repository was found from {} or its parents; run `ait init` in the target repository or pass `-C <path>` ({error})",
            start.display()
        )
    })?;
    if let Some(snapshot_id) = args.ui_diff_tsv.as_deref() {
        print!("{}", ui_diff_tsv(&repo, snapshot_id)?);
        return Ok(());
    }
    let payload = build_history_payload(&repo, args.limit)?;
    let encoded = encode_value_pretty_with_newline_error_string(&payload)?;

    if let Some(output) = args.output.as_deref() {
        write_json_output(output, encoded.as_bytes())?;
    }
    if args.json_only {
        if args.output.is_none() {
            print!("{encoded}");
        }
        return Ok(());
    }

    launch_tk(&repo, &payload, args.wish.as_deref())
}

fn validate_limit(limit: usize) -> Result<(), String> {
    if limit == 0 || limit > MAX_HISTORY_LIMIT {
        return Err(format!(
            "--limit must be between 1 and {MAX_HISTORY_LIMIT}, got {limit}"
        ));
    }
    Ok(())
}

fn write_json_output(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.is_dir() {
        return Err(format!(
            "--output target is a directory: {}",
            path.display()
        ));
    }
    fs::write(path, bytes).map_err(|error| {
        format!(
            "failed to write JSON payload to {}: {error}",
            path.display()
        )
    })
}

pub fn build_history_payload(repo: &RepoRuntime, limit: usize) -> Result<JsonValue, String> {
    validate_limit(limit)?;
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let lines = store.list_lines()?;
    let snapshots = store.list_line_snapshots()?;
    build_history_payload_from_records(
        &repo.repo_name(),
        &workspace_root,
        &repo.default_line_name(),
        &repo.current_line_name()?,
        lines,
        snapshots,
        limit,
        |snapshot| {
            store.snapshot_tree_path_delta(
                snapshot.primary_parent_snapshot_id.as_deref(),
                Some(&snapshot.snapshot_id),
            )
        },
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the arguments are the explicit, independently testable aitk read-model inputs"
)]
fn build_history_payload_from_records<F>(
    repo_name: &str,
    repo_root: &Path,
    default_line_name: &str,
    current_line_name: &str,
    mut lines: Vec<LineRecord>,
    mut snapshots: Vec<SnapshotRecord>,
    limit: usize,
    mut diff_reader: F,
) -> Result<JsonValue, String>
where
    F: FnMut(&SnapshotRecord) -> Result<SnapshotPathDelta, String>,
{
    validate_limit(limit)?;
    snapshots.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.snapshot_id.cmp(&left.snapshot_id))
    });
    let total_snapshot_count = snapshots.len();
    let parent_map = snapshots
        .iter()
        .map(|snapshot| {
            (
                snapshot.snapshot_id.clone(),
                snapshot.parent_snapshot_ids.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    lines.retain(|line| line.status != "archived" && line.status != "deleted");
    let total_active_line_count = lines.len();
    let default_head = lines
        .iter()
        .find(|line| line.line_name == default_line_name)
        .and_then(|line| line.head_snapshot_id.clone());
    let mut health_by_line = BTreeMap::new();
    for line in &lines {
        health_by_line.insert(
            line.line_name.clone(),
            classify_line_health(
                line,
                default_line_name,
                default_head.as_deref(),
                &parent_map,
            ),
        );
    }
    lines.sort_by(|left, right| {
        let left_default = left.line_name == default_line_name;
        let right_default = right.line_name == default_line_name;
        right_default
            .cmp(&left_default)
            .then_with(|| {
                health_rank(health_by_line[&left.line_name].state)
                    .cmp(&health_rank(health_by_line[&right.line_name].state))
            })
            .then_with(|| left.line_name.cmp(&right.line_name))
    });
    lines.truncate(MAX_LINE_ROWS);

    let mut head_labels = BTreeMap::<String, Vec<String>>::new();
    for line in &lines {
        if let Some(head) = line.head_snapshot_id.as_ref() {
            head_labels
                .entry(head.clone())
                .or_default()
                .push(line.line_name.clone());
        }
    }
    for labels in head_labels.values_mut() {
        labels.sort();
    }

    let selected_snapshots = snapshots.iter().take(limit).cloned().collect::<Vec<_>>();
    let active_heads = lines
        .iter()
        .filter_map(|line| line.head_snapshot_id.clone())
        .collect::<Vec<_>>();
    let graph_columns = active_column_layout(&selected_snapshots, &active_heads);

    let line_rows = lines
        .iter()
        .map(|line| {
            let health = &health_by_line[&line.line_name];
            json!({
                "line_id": &line.line_id,
                "line_name": &line.line_name,
                "status": &line.status,
                "head_snapshot_id": &line.head_snapshot_id,
                "created_at": &line.created_at,
                "updated_at": &line.updated_at,
                "health": health.state,
                "ahead_by": health.ahead_by,
                "behind_by": health.behind_by,
                "head_present": health.head_present,
                "is_default": line.line_name == default_line_name,
                "is_current": line.line_name == current_line_name,
            })
        })
        .collect::<Vec<_>>();

    let mut snapshot_rows = Vec::with_capacity(selected_snapshots.len());
    for (snapshot_index, snapshot) in selected_snapshots.iter().enumerate() {
        let (changed_paths, changed_path_count, changed_paths_truncated, diff_state, diff_error) =
            if snapshot_index >= MAX_DIFF_PRELOAD_SNAPSHOTS {
                (Vec::new(), None, false, "not_preloaded", None)
            } else {
                match diff_reader(snapshot) {
                    Ok(delta) => {
                        let total = delta.status_by_path.len();
                        let rows = delta
                            .status_by_path
                            .into_iter()
                            .take(MAX_CHANGED_PATHS_PER_SNAPSHOT)
                            .map(|(path, status)| json!({"path": path, "status": status}))
                            .collect::<Vec<_>>();
                        (
                            rows,
                            Some(total),
                            total > MAX_CHANGED_PATHS_PER_SNAPSHOT,
                            "loaded",
                            None,
                        )
                    }
                    Err(error) => (Vec::new(), None, false, "unavailable", Some(error)),
                }
            };
        let labels = head_labels
            .get(&snapshot.snapshot_id)
            .cloned()
            .unwrap_or_default();
        let line_health = health_by_line
            .get(&snapshot.line_name)
            .map(|health| health.state)
            .unwrap_or("historical");
        snapshot_rows.push(json!({
            "snapshot_id": &snapshot.snapshot_id,
            "parent_snapshot_ids": &snapshot.parent_snapshot_ids,
            "primary_parent_snapshot_id": &snapshot.primary_parent_snapshot_id,
            "line_name": &snapshot.line_name,
            "snapshot_kind": &snapshot.snapshot_kind,
            "message": &snapshot.message,
            "created_at": &snapshot.created_at,
            "file_count": snapshot.file_count,
            "total_bytes": snapshot.total_bytes,
            "head_labels": labels,
            "line_health": line_health,
            "graph_column": graph_columns.get(&snapshot.snapshot_id).copied().unwrap_or(0),
            "changed_path_count": changed_path_count,
            "changed_paths_truncated": changed_paths_truncated,
            "changed_paths": changed_paths,
            "diff_state": diff_state,
            "diff_error": diff_error,
        }));
    }

    Ok(json!({
        "schema": AITK_PAYLOAD_SCHEMA,
        "payload_type": "aitk-history",
        "read_only": true,
        "repository": {
            "name": repo_name,
            "root": repo_root.to_string_lossy().to_string(),
            "default_line": default_line_name,
            "current_line": current_line_name,
        },
        "history": {
            "limit": limit,
            "total_snapshot_count": total_snapshot_count,
            "returned_snapshot_count": snapshot_rows.len(),
            "truncated": total_snapshot_count > snapshot_rows.len(),
            "total_active_line_count": total_active_line_count,
            "returned_line_count": line_rows.len(),
            "lines_truncated": total_active_line_count > line_rows.len(),
            "diff_preload_snapshot_limit": MAX_DIFF_PRELOAD_SNAPSHOTS,
            "changed_path_limit_per_snapshot": MAX_CHANGED_PATHS_PER_SNAPSHOT,
        },
        "lines": line_rows,
        "snapshots": snapshot_rows,
    }))
}

fn health_rank(state: &str) -> usize {
    match state {
        "current_main" => 0,
        "uncontained" => 1,
        "contained" => 2,
        "empty" => 3,
        "missing_snapshot" => 4,
        _ => 5,
    }
}

fn classify_line_health(
    line: &LineRecord,
    default_line_name: &str,
    default_head: Option<&str>,
    parent_map: &BTreeMap<String, Vec<String>>,
) -> LineHealth {
    let Some(head) = line.head_snapshot_id.as_deref() else {
        return LineHealth {
            state: "empty",
            ahead_by: Some(0),
            behind_by: default_head.map(|_| 0),
            head_present: true,
        };
    };
    let head_present = parent_map.contains_key(head);
    if line.line_name == default_line_name {
        return LineHealth {
            state: if head_present {
                "current_main"
            } else {
                "missing_snapshot"
            },
            ahead_by: Some(0),
            behind_by: Some(0),
            head_present,
        };
    }
    if !head_present {
        return LineHealth {
            state: "missing_snapshot",
            ahead_by: None,
            behind_by: None,
            head_present: false,
        };
    }
    let Some(default_head) = default_head else {
        return LineHealth {
            state: "unknown",
            ahead_by: None,
            behind_by: None,
            head_present: true,
        };
    };
    if !parent_map.contains_key(default_head) {
        return LineHealth {
            state: "unknown",
            ahead_by: None,
            behind_by: None,
            head_present: true,
        };
    }
    let head_distances = ancestor_distances(head, parent_map);
    let default_distances = ancestor_distances(default_head, parent_map);
    let common = head_distances
        .iter()
        .filter_map(|(snapshot_id, head_distance)| {
            default_distances.get(snapshot_id).map(|default_distance| {
                (
                    head_distance + default_distance,
                    *head_distance,
                    *default_distance,
                    snapshot_id,
                )
            })
        })
        .min_by(|left, right| left.cmp(right));
    let (ahead_by, behind_by) = common
        .map(|(_, ahead, behind, _)| (Some(ahead), Some(behind)))
        .unwrap_or((None, None));
    LineHealth {
        state: if default_distances.contains_key(head) {
            "contained"
        } else {
            "uncontained"
        },
        ahead_by,
        behind_by,
        head_present: true,
    }
}

fn ancestor_distances(
    start: &str,
    parent_map: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, usize> {
    let mut distances = BTreeMap::new();
    let mut queue = VecDeque::from([(start.to_string(), 0usize)]);
    while let Some((snapshot_id, distance)) = queue.pop_front() {
        if distances.contains_key(&snapshot_id) {
            continue;
        }
        distances.insert(snapshot_id.clone(), distance);
        if let Some(parents) = parent_map.get(&snapshot_id) {
            for parent in parents {
                if !distances.contains_key(parent) {
                    queue.push_back((parent.clone(), distance.saturating_add(1)));
                }
            }
        }
    }
    distances
}

fn active_column_layout(
    snapshots: &[SnapshotRecord],
    active_heads: &[String],
) -> BTreeMap<String, usize> {
    let mut columns = active_heads
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut result = BTreeMap::new();
    for snapshot in snapshots {
        let column = match columns.iter().position(|id| id == &snapshot.snapshot_id) {
            Some(column) => column,
            None => {
                columns.push(snapshot.snapshot_id.clone());
                columns.len() - 1
            }
        };
        result.insert(snapshot.snapshot_id.clone(), column);
        columns.remove(column);
        let mut insertion = column.min(columns.len());
        for parent in &snapshot.parent_snapshot_ids {
            if columns.iter().any(|id| id == parent) {
                continue;
            }
            columns.insert(insertion, parent.clone());
            insertion += 1;
        }
    }
    result
}

fn launch_tk(
    repo: &RepoRuntime,
    payload: &JsonValue,
    requested_wish: Option<&OsStr>,
) -> Result<(), String> {
    let wish = resolve_wish(requested_wish)?;
    let mut script = private_tempfile("aitk-", ".tcl")?;
    script
        .write_all(AITK_TK_SCRIPT.as_bytes())
        .map_err(|error| format!("failed to write embedded Tcl/Tk script: {error}"))?;
    script
        .flush()
        .map_err(|error| format!("failed to flush embedded Tcl/Tk script: {error}"))?;
    let mut data = private_tempfile("aitk-", ".tsv")?;
    let current_executable = env::current_exe()
        .map_err(|error| format!("failed to resolve the running aitk executable: {error}"))?;
    data.write_all(ui_tsv(payload, Some(&current_executable))?.as_bytes())
        .map_err(|error| format!("failed to write temporary UI payload: {error}"))?;
    data.flush()
        .map_err(|error| format!("failed to flush temporary UI payload: {error}"))?;

    let status = Command::new(&wish)
        .arg(script.path())
        .arg(data.path())
        .current_dir(repo.workspace_root())
        .status()
        .map_err(|error| format!("failed to start {}: {error}", wish.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with status {status}", wish.display()))
    }
}

fn private_tempfile(prefix: &str, suffix: &str) -> Result<NamedTempFile, String> {
    Builder::new()
        .prefix(prefix)
        .suffix(suffix)
        .tempfile()
        .map_err(|error| format!("failed to create private system temporary file: {error}"))
}

fn resolve_wish(requested: Option<&OsStr>) -> Result<PathBuf, String> {
    if let Some(requested) = requested {
        return find_executable(requested).ok_or_else(|| {
            format!(
                "--wish executable was not found or is not a file: {}",
                Path::new(requested).display()
            )
        });
    }
    let mut candidates = Vec::<OsString>::new();
    #[cfg(target_os = "macos")]
    {
        candidates.push("/opt/homebrew/opt/tcl-tk/bin/wish".into());
        candidates.push("/usr/local/opt/tcl-tk/bin/wish".into());
    }
    candidates.extend(["wish9.0".into(), "wish8.6".into(), "wish".into()]);
    for candidate in candidates {
        if let Some(path) = find_executable(&candidate) {
            return Ok(path);
        }
    }
    Err(
        "no Tcl/Tk `wish` executable was found. Install Tcl/Tk (for example `brew install tcl-tk` on macOS) or pass `--wish <path>`. `aitk --json-only` does not require Tk."
            .to_string(),
    )
}

fn find_executable(candidate: &OsStr) -> Option<PathBuf> {
    let candidate_path = Path::new(candidate);
    if candidate_path.components().count() > 1 {
        return candidate_path
            .is_file()
            .then(|| candidate_path.to_path_buf());
    }
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        let direct = directory.join(candidate);
        if direct.is_file() {
            return Some(direct);
        }
        #[cfg(windows)]
        {
            let executable = directory.join(format!("{}.exe", candidate.to_string_lossy()));
            if executable.is_file() {
                return Some(executable);
            }
        }
    }
    None
}

fn ui_tsv(payload: &JsonValue, aitk_command: Option<&Path>) -> Result<String, String> {
    let repository = payload
        .get("repository")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "aitk payload is missing repository metadata".to_string())?;
    let mut rows = vec!["aitk-tsv-v1".to_string()];
    for key in ["name", "root", "default_line", "current_line"] {
        rows.push(tsv_row(
            "meta",
            &[key.to_string(), json_text(repository.get(key))],
        ));
    }
    if let Some(command) = aitk_command {
        rows.push(tsv_row(
            "meta",
            &[
                "aitk_command".to_string(),
                command.to_string_lossy().to_string(),
            ],
        ));
    }
    for line in payload
        .get("lines")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        rows.push(tsv_row(
            "line",
            &[
                json_text(line.get("line_name")),
                json_text(line.get("head_snapshot_id")),
                json_text(line.get("status")),
                json_text(line.get("health")),
                json_text(line.get("ahead_by")),
                json_text(line.get("behind_by")),
            ],
        ));
    }
    for snapshot in payload
        .get("snapshots")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let snapshot_id = json_text(snapshot.get("snapshot_id"));
        rows.push(tsv_row(
            "snapshot",
            &[
                snapshot_id.clone(),
                json_text(snapshot.get("line_name")),
                json_text(snapshot.get("created_at")),
                json_text(snapshot.get("message")),
                json_text(snapshot.get("snapshot_kind")),
                json_list(snapshot.get("parent_snapshot_ids"), " | "),
                json_list(snapshot.get("head_labels"), ", "),
                json_text(snapshot.get("line_health")),
                json_text(snapshot.get("graph_column")),
                json_text(snapshot.get("file_count")),
                json_text(snapshot.get("total_bytes")),
                json_text(snapshot.get("changed_path_count")),
                json_text(snapshot.get("changed_paths_truncated")),
                json_text(snapshot.get("diff_error")),
                json_text(snapshot.get("diff_state")),
            ],
        ));
        for changed in snapshot
            .get("changed_paths")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            rows.push(tsv_row(
                "path",
                &[
                    snapshot_id.clone(),
                    json_text(changed.get("status")),
                    json_text(changed.get("path")),
                ],
            ));
        }
    }
    rows.push(String::new());
    Ok(rows.join("\n"))
}

fn ui_diff_tsv(repo: &RepoRuntime, snapshot_id: &str) -> Result<String, String> {
    let snapshot_id = snapshot_id.trim();
    if snapshot_id.is_empty() {
        return Err("--ui-diff-tsv requires a non-empty Snapshot id".to_string());
    }
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let snapshot = store
        .snapshot_by_id(snapshot_id)?
        .ok_or_else(|| format!("unknown Snapshot: {snapshot_id}"))?;
    let delta = store.snapshot_tree_path_delta(
        snapshot.primary_parent_snapshot_id.as_deref(),
        Some(snapshot_id),
    )?;
    let total = delta.status_by_path.len();
    let mut rows = vec!["aitk-diff-tsv-v1".to_string()];
    rows.push(tsv_row(
        "meta",
        &["changed_path_count".to_string(), total.to_string()],
    ));
    rows.push(tsv_row(
        "meta",
        &[
            "truncated".to_string(),
            (total > MAX_CHANGED_PATHS_PER_SNAPSHOT).to_string(),
        ],
    ));
    for (path, status) in delta
        .status_by_path
        .into_iter()
        .take(MAX_CHANGED_PATHS_PER_SNAPSHOT)
    {
        rows.push(tsv_row("path", &[status, path]));
    }
    rows.push(String::new());
    Ok(rows.join("\n"))
}

fn tsv_row(kind: &str, fields: &[String]) -> String {
    std::iter::once(kind.to_string())
        .chain(
            fields
                .iter()
                .map(|field| BASE64_STANDARD.encode(field.as_bytes())),
        )
        .collect::<Vec<_>>()
        .join("\t")
}

fn json_text(value: Option<&JsonValue>) -> String {
    match value {
        Some(JsonValue::String(value)) => value.clone(),
        Some(JsonValue::Number(value)) => value.to_string(),
        Some(JsonValue::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn json_list(value: Option<&JsonValue>, separator: &str) -> String {
    value
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(JsonValue::as_str)
                .collect::<Vec<_>>()
                .join(separator)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(id: &str, parents: &[&str], line: &str, created_at: &str) -> SnapshotRecord {
        SnapshotRecord {
            snapshot_id: id.to_string(),
            parent_snapshot_ids: parents.iter().map(|value| (*value).to_string()).collect(),
            primary_parent_snapshot_id: parents.first().map(|value| (*value).to_string()),
            parent_snapshot_id: parents.first().map(|value| (*value).to_string()),
            root_tree_pack_id: None,
            root_entry_ordinal: None,
            manifest_hash: format!("hash-{id}"),
            message: Some(format!("message {id}")),
            line_name: line.to_string(),
            snapshot_kind: "line".to_string(),
            file_count: 1,
            total_bytes: 10,
            created_at: created_at.to_string(),
        }
    }

    fn line(name: &str, head: Option<&str>) -> LineRecord {
        LineRecord {
            line_id: format!("line-{name}"),
            line_name: name.to_string(),
            status: "active".to_string(),
            archived_at: None,
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: Some("2026-01-02T00:00:00Z".to_string()),
            head_snapshot_id: head.map(str::to_string),
        }
    }

    #[test]
    fn payload_is_bounded_newest_first_and_classifies_line_health() {
        let payload = build_history_payload_from_records(
            "fixture",
            Path::new("/fixture"),
            "main",
            "feature/a",
            vec![
                line("main", Some("SNP-MAIN")),
                line("feature/a", Some("SNP-FEATURE")),
                line("feature/contained", Some("SNP-ROOT")),
                line("feature/empty", None),
            ],
            vec![
                snapshot("SNP-ROOT", &[], "main", "2026-01-01T00:00:00Z"),
                snapshot("SNP-MAIN", &["SNP-ROOT"], "main", "2026-01-03T00:00:00Z"),
                snapshot(
                    "SNP-FEATURE",
                    &["SNP-ROOT"],
                    "feature/a",
                    "2026-01-04T00:00:00Z",
                ),
            ],
            2,
            |_| {
                Ok(SnapshotPathDelta {
                    affected_paths: vec!["src/lib.rs".to_string()],
                    status_by_path: BTreeMap::from([(
                        "src/lib.rs".to_string(),
                        "modified".to_string(),
                    )]),
                })
            },
        )
        .unwrap();
        let snapshots = payload["snapshots"].as_array().unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0]["snapshot_id"].as_str(), Some("SNP-FEATURE"));
        assert_eq!(snapshots[1]["snapshot_id"].as_str(), Some("SNP-MAIN"));
        assert_eq!(payload["history"]["truncated"].as_bool(), Some(true));
        assert_eq!(snapshots[0]["changed_path_count"].as_u64(), Some(1));

        let states = payload["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["line_name"].as_str().unwrap(),
                    row["health"].as_str().unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(states["main"], "current_main");
        assert_eq!(states["feature/a"], "uncontained");
        assert_eq!(states["feature/contained"], "contained");
        assert_eq!(states["feature/empty"], "empty");
    }

    #[test]
    fn payload_preserves_ordered_multi_parent_identity_and_graph_columns() {
        let payload = build_history_payload_from_records(
            "fixture",
            Path::new("/fixture"),
            "main",
            "main",
            vec![line("main", Some("SNP-MERGE"))],
            vec![
                snapshot(
                    "SNP-MERGE",
                    &["SNP-LEFT", "SNP-RIGHT"],
                    "main",
                    "2026-01-03T00:00:00Z",
                ),
                snapshot("SNP-LEFT", &[], "main", "2026-01-02T00:00:00Z"),
                snapshot("SNP-RIGHT", &[], "feature", "2026-01-01T00:00:00Z"),
            ],
            3,
            |_| {
                Ok(SnapshotPathDelta {
                    affected_paths: Vec::new(),
                    status_by_path: BTreeMap::new(),
                })
            },
        )
        .unwrap();
        let rows = payload["snapshots"].as_array().unwrap();
        assert_eq!(
            rows[0]["parent_snapshot_ids"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(JsonValue::as_str)
                .collect::<Vec<_>>(),
            vec!["SNP-LEFT", "SNP-RIGHT"]
        );
        assert_eq!(rows[0]["graph_column"].as_u64(), Some(0));
    }

    #[test]
    fn tsv_transport_is_base64_safe_and_contains_read_only_rows() {
        let payload = build_history_payload_from_records(
            "repo\t名",
            Path::new("/tmp/repo with spaces"),
            "main",
            "main",
            vec![line("main", Some("SNP-1"))],
            vec![snapshot("SNP-1", &[], "main", "2026-01-01T00:00:00Z")],
            1,
            |_| {
                Ok(SnapshotPathDelta {
                    affected_paths: Vec::new(),
                    status_by_path: BTreeMap::new(),
                })
            },
        )
        .unwrap();
        let encoded = ui_tsv(&payload, None).unwrap();
        assert!(encoded.starts_with("aitk-tsv-v1\nmeta\t"));
        assert!(!encoded.contains("repo\t名"));
        assert!(encoded.lines().any(|line| line.starts_with("snapshot\t")));
        assert!(AITK_TK_SCRIPT.contains("--ui-diff-tsv"));
        for mutation in [" task ", " change ", " snapshot create", " land "] {
            assert!(!AITK_TK_SCRIPT.contains(mutation));
        }
    }

    #[test]
    fn invalid_history_bounds_fail_before_repository_access() {
        assert!(validate_limit(0).unwrap_err().contains("between 1"));
        assert!(validate_limit(MAX_HISTORY_LIMIT + 1)
            .unwrap_err()
            .contains(&MAX_HISTORY_LIMIT.to_string()));
    }
}
