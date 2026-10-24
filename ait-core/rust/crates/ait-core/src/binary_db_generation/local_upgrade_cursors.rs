//! Transient disposable cursor outputs for the coordinated metadata batch.
use super::local_change_cleanup::Conversion;
use super::{GenerationResult, Path, PathBuf};
use serde_json::Value;
use std::fs;
use std::io::Write;

fn direct_bytes(path: &Path) -> GenerationResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || path.canonicalize().map_err(|e| e.to_string())? != path {
        return Err(format!("Aliased worktree cursor path: {}", path.display()));
    }
    fs::read(path).map_err(|e| e.to_string())
}
fn task_index(text: &str) -> GenerationResult<Option<u32>> {
    if !text.starts_with('L') {
        return Ok(None);
    }
    let (_, ordinal) = text
        .rsplit_once("T-")
        .ok_or("Invalid local worktree Task identity")?;
    let index = ordinal
        .parse::<u32>()
        .ok()
        .and_then(|x| x.checked_sub(1))
        .ok_or("Invalid local worktree Task ordinal")?;
    Ok(Some(index))
}
fn selection(
    conversion: &Conversion,
    task: u32,
    id: &str,
    renumber: bool,
) -> GenerationResult<Option<String>> {
    let short = id.rsplit('/').next().unwrap_or(id);
    let ordinal = short
        .strip_prefix("C-")
        .and_then(|x| x.parse::<u8>().ok())
        .and_then(|x| x.checked_sub(1))
        .ok_or("Invalid legacy worktree Change selection")?;
    let old = conversion
        .changes
        .iter()
        .position(|row| row.task_index == task && row.change_ordinal == ordinal)
        .ok_or("Worktree selects missing legacy Change")?;
    Ok(conversion.mapping[old].map(|_| {
        if renumber {
            format!("C-{:02}", ordinal)
        } else {
            format!("C-{:02}", ordinal + 1)
        }
    }))
}
fn converted(
    path: &Path,
    before: &[u8],
    conversion: &Conversion,
    renumber: bool,
    registered_task: &str,
) -> GenerationResult<Vec<u8>> {
    if path.file_name().and_then(|x| x.to_str()) == Some("cursor.bin") {
        if before.len() != 32
            || before[..4] != 1u32.to_le_bytes()
            || before[4] & !7 != 0
            || before[5..8] != [0; 3]
        {
            return Err("Unsupported worktree cursor profile".into());
        }
        let field = |at| u32::from_le_bytes(before[at..at + 4].try_into().unwrap());
        for (flag, at) in [(1, 12), (2, 16), (4, 20)] {
            if (before[4] & flag != 0) != (field(at) != 0) {
                return Err("Worktree cursor presence flags disagree".into());
            }
        }
        let mut after = before.to_vec();
        if let Some(old) = field(12).checked_sub(1) {
            let row = conversion
                .changes
                .get(old as usize)
                .ok_or("Worktree cursor Change reference is missing")?;
            if row.task_index != field(8) {
                return Err("Worktree cursor references another Task's Change".into());
            }
            let new = conversion.mapping[old as usize].map_or(0, |x| x + 1);
            after[12..16].copy_from_slice(&new.to_le_bytes());
            after[4] = (after[4] & !1) | u8::from(new != 0);
        }
        return Ok(after);
    }
    let mut value: Value = serde_json::from_slice(before).map_err(|e| e.to_string())?;
    let task_text = value
        .get("bound_task_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .or_else(|| value.get("task_id").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .unwrap_or(registered_task)
        .to_string();
    if let Some(task) = task_index(&task_text)? {
        for key in [
            "bound_change_id",
            "bound_change_ref",
            "change_id",
            "change_ref",
        ] {
            let Some(id) = value
                .get(key)
                .and_then(Value::as_str)
                .filter(|x| !x.is_empty())
            else {
                continue;
            };
            if let Some((owner, _)) = id.rsplit_once('/') {
                if owner != task_text {
                    return Err("Worktree textual Change has another Task owner".into());
                }
            }
            let new = selection(conversion, task, id, renumber)?;
            value[key] = match new {
                Some(short) => Value::String(if key.ends_with("_ref") || id.contains('/') {
                    format!("{task_text}/{short}")
                } else {
                    short
                }),
                None => Value::Null,
            };
        }
    }
    if serde_json::from_slice::<Value>(before).map_err(|e| e.to_string())? == value {
        return Ok(before.to_vec());
    }
    let mut bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}
struct RegisteredPath {
    path: PathBuf,
    task: String,
}
fn paths_with_namespace(
    repo: &Path,
    namespace: Option<&str>,
) -> GenerationResult<Vec<RegisteredPath>> {
    let registry = repo.join(".ait/worktrees");
    let mut paths = Vec::new();
    let registry_exists = match fs::symlink_metadata(&registry) {
        Ok(metadata) if metadata.file_type().is_dir() => true,
        Ok(_) => return Err("Worktree registry is not a direct directory".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.to_string()),
    };
    if registry_exists {
        if registry.canonicalize().map_err(|e| e.to_string())? != registry {
            return Err("Aliased worktree registry".into());
        }
        for entry in fs::read_dir(&registry).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let bytes = direct_bytes(&path)?;
            let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let task = value
                .get("bound_task_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            if task_index(task)?.is_none() {
                continue;
            }
            if let Some(namespace) = namespace {
                if task != format!("L{namespace}T-{:04}", task_index(task)?.unwrap() + 1) {
                    return Err("Worktree registry has another Local namespace".into());
                }
            }
            let declared = value
                .get("repo_root")
                .and_then(Value::as_str)
                .ok_or("Worktree registry has no Repository owner")?;
            if Path::new(declared)
                .canonicalize()
                .map_err(|e| e.to_string())?
                != repo
            {
                return Err("Worktree registry belongs to another Repository".into());
            }
            paths.push(RegisteredPath {
                path,
                task: task.to_string(),
            });
            if let Some(worktree) = value.get("path").and_then(Value::as_str) {
                let worktree = Path::new(worktree);
                for leaf in [".ait-worktree.json", ".ait-worktree/cursor.bin"] {
                    let path = worktree.join(leaf);
                    match fs::symlink_metadata(&path) {
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => return Err(e.to_string()),
                    }
                    {
                        let bytes = direct_bytes(&path)?;
                        if leaf == ".ait-worktree.json" {
                            let overlay: Value =
                                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                            let owner = overlay
                                .get("repo_root")
                                .and_then(Value::as_str)
                                .ok_or("Worktree overlay has no Repository owner")?;
                            if Path::new(owner).canonicalize().map_err(|e| e.to_string())? != repo {
                                return Err("Worktree overlay belongs to another Repository".into());
                            }
                            let mut explicit_task = false;
                            for key in ["bound_task_id", "task_id"] {
                                if let Some(identity) = overlay.get(key).filter(|v| !v.is_null()) {
                                    let identity =
                                        identity.as_str().ok_or("Invalid overlay Task identity")?;
                                    if !identity.is_empty() {
                                        explicit_task = true;
                                        if identity != task {
                                            return Err(
                                                "Worktree overlay differs from its registered Task"
                                                    .into(),
                                            );
                                        }
                                    }
                                }
                            }
                            let registered_root =
                                worktree.canonicalize().map_err(|e| e.to_string())?;
                            let mut explicit_path = false;
                            for key in ["workspace_root", "path"] {
                                if let Some(root) = overlay.get(key).filter(|v| !v.is_null()) {
                                    explicit_path = true;
                                    let root =
                                        root.as_str().ok_or("Invalid overlay workspace path")?;
                                    if Path::new(root).canonicalize().map_err(|e| e.to_string())?
                                        != registered_root
                                    {
                                        return Err(
                                            "Worktree overlay differs from its registered path"
                                                .into(),
                                        );
                                    }
                                }
                            }
                            let registered_name = value
                                .get("name")
                                .and_then(Value::as_str)
                                .or_else(|| worktree.file_name().and_then(|s| s.to_str()));
                            let overlay_name = match overlay.get("worktree_name") {
                                None | Some(Value::Null) => None,
                                Some(Value::String(name)) => Some(name.as_str()),
                                Some(_) => return Err("Invalid overlay worktree name".into()),
                            };
                            if overlay_name.is_some() && overlay_name != registered_name {
                                return Err(
                                    "Worktree overlay differs from its registered name".into()
                                );
                            }
                            if !explicit_task && (!explicit_path || overlay_name.is_none()) {
                                return Err(
                                    "Worktree overlay lacks registered workspace identity".into()
                                );
                            }
                        } else if bytes.len() != 32
                            || u32::from_le_bytes(bytes[8..12].try_into().unwrap())
                                != task_index(task)?.ok_or("Missing local Task")?
                        {
                            return Err("Worktree cursor differs from its registered Task".into());
                        }
                        paths.push(RegisteredPath {
                            path,
                            task: task.to_string(),
                        });
                    }
                }
            }
        }
    }
    paths.sort_by(|a, b| a.path.cmp(&b.path));
    for pair in paths.windows(2) {
        if pair[0].path == pair[1].path && pair[0].task != pair[1].task {
            return Err("Worktree path has conflicting registered Tasks".into());
        }
    }
    paths.dedup_by(|a, b| a.path == b.path);
    Ok(paths)
}

struct CursorOutput {
    path: PathBuf,
    before: Vec<u8>,
    temporary: Option<PathBuf>,
}
pub(super) struct PreparedCursors {
    outputs: Vec<CursorOutput>,
}
impl Drop for PreparedCursors {
    fn drop(&mut self) {
        for output in &self.outputs {
            if let Some(path) = &output.temporary {
                let _ = fs::remove_file(path);
            }
        }
    }
}
impl PreparedCursors {
    pub(super) fn publish(&self) -> GenerationResult<()> {
        for output in &self.outputs {
            if let Some(temporary) = &output.temporary {
                if direct_bytes(&output.path)? != output.before {
                    return Err(
                        "Worktree cursor/selection changed across metadata publication".into(),
                    );
                }
                fs::rename(temporary, &output.path).map_err(|e| e.to_string())?;
                super::generation_capture::sync_directory(
                    output.path.parent().ok_or("Cursor has no parent")?,
                )?;
            }
        }
        Ok(())
    }
}

pub(super) fn prepare(
    repo: &Path,
    conversion: &Conversion,
    namespace: &str,
) -> GenerationResult<PreparedCursors> {
    prepare_with_namespace(repo, conversion, Some(namespace))
}

fn prepare_with_namespace(
    repo: &Path,
    conversion: &Conversion,
    namespace: Option<&str>,
) -> GenerationResult<PreparedCursors> {
    let mut prepared = PreparedCursors {
        outputs: Vec::new(),
    };
    for registered in paths_with_namespace(repo, namespace)? {
        let path = registered.path;
        let before = direct_bytes(&path)?;
        if namespace.is_some() {
            let task = task_index(&registered.task)?.ok_or("Missing local worktree Task")?;
            if task as usize >= conversion.task_count {
                return Err("Worktree references a missing Local Task".into());
            }
        }
        let after = converted(&path, &before, conversion, true, &registered.task)?;
        let temporary = (before != after).then(|| {
            path.parent().unwrap().join(format!(
                ".local-receipt-cursor-{}",
                super::local_land_upgrade::temporary_suffix()
            ))
        });
        // Register before writing so failures discard only temporary outputs.
        prepared.outputs.push(CursorOutput {
            path,
            before,
            temporary,
        });
        if let Some(path) = &prepared.outputs.last().unwrap().temporary {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
                .map_err(|e| e.to_string())?;
            file.write_all(&after)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(prepared)
}

// No recovery action: stale numeric or textual selections fail before ordinary
// work. Identity mapping validates the already-current profile without cleanup.
pub(super) fn validate_current(repo: &Path, authority: &Path) -> GenerationResult<()> {
    use crate::workflow_binary_db::LocalChangeRecord;
    let bytes = match fs::read(authority.join("change.bin")) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 1u32.to_le_bytes().to_vec(),
        Err(e) => return Err(e.to_string()),
    };
    if bytes.len() < 4 || bytes[..4] != 1u32.to_le_bytes() || !(bytes.len() - 4).is_multiple_of(68)
    {
        return Err("Unsupported current Change profile".into());
    }
    let changes = bytes[4..]
        .chunks_exact(68)
        .map(LocalChangeRecord::decode)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let conversion = Conversion {
        mapping: (0..changes.len()).map(|index| Some(index as u32)).collect(),
        changes,
        files: Default::default(),
        task_count: 0,
        sources: Default::default(),
        core_history: false,
    };
    for registered in paths_with_namespace(repo, None)? {
        let before = direct_bytes(&registered.path)?;
        if converted(
            &registered.path,
            &before,
            &conversion,
            false,
            &registered.task,
        )? != before
        {
            return Err(
                "Incomplete workflow cursor profile; admission requires coordination".into(),
            );
        }
    }
    Ok(())
}
