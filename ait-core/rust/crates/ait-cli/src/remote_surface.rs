use crate::agent_harness::converge_agent_workflow_harness;
use crate::json_support::{encode_value_pretty_with_newline_error_string, parse_value};
use crate::remote_repository::{
    ensure_or_read_remote_repository_authority_for_url, local_policy_requires_tests,
    remote_repository_index,
};
use crate::repository_retirement::ensure_fresh_registration_has_no_archive;
use crate::runtime::{canonical_repository_directory_name, RepoRuntime};
use crate::workspace_lock::run_locked_workspace_command;
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::remote_store::{
    add_remote_with_remote_store, list_remotes_with_remote_store, remote_by_name_with_remote_store,
    remote_exists_with_remote_store, RemoteAddRecord, RemoteRecord, RemoteStore,
};
use chrono::Utc;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct RemoteAddRequest {
    pub name: String,
    pub url: String,
    pub make_default: bool,
}

pub fn remote_add(repo: &RepoRuntime, request: &RemoteAddRequest) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli remote add", || {
        let refreshed = RepoRuntime::discover_from_path(&repo.workspace_root())?;
        remote_add_unlocked(&refreshed, request)
    })
}

fn remote_add_unlocked(
    repo: &RepoRuntime,
    request: &RemoteAddRequest,
) -> Result<JsonValue, String> {
    let name = normalize_required_text(&request.name, "remote name")?;
    let url = normalize_required_text(&request.url, "remote URL")?;
    let store = repo.remote_store()?;
    ensure_remote_name_available_with_remote_store(&store, &name)?;
    ensure_fresh_registration_has_no_archive(repo, &name)?;
    let repo_name = canonical_repository_directory_name(&repo.authoritative_repo_root())?;
    let delivery_policy_default_requires_tests = local_policy_requires_tests(repo)?;
    let remote_repository = ensure_or_read_remote_repository_authority_for_url(
        repo, &url, &repo_name,
    )
    .map_err(|err| {
        format!(
            "Remote repository {} could not be ensured for remote {}: {}",
            repo_name, name, err
        )
    })?;
    let repository_index = remote_repository_index(&remote_repository)?;
    if repo.repository_index().is_none() {
        persist_repository_index(repo, repository_index)?;
    }
    let now = Utc::now().to_rfc3339();

    remote_add_record_with_remote_store(
        &store,
        &RemoteAddRecord {
            name: name.clone(),
            url,
            repo_name: Some(repo_name),
            make_default: request.make_default,
            created_at: now,
        },
    )?;

    if request.make_default {
        set_default_remote(repo, &name)?;
    }
    let refreshed = RepoRuntime::discover_from_path(&repo.authoritative_repo_root())?;
    let mut payload = remote_get(&refreshed, Some(&name))?;
    payload
        .as_object_mut()
        .ok_or_else(|| "Remote add payload must be an object.".to_string())?
        .insert(
            "patch_ci".to_string(),
            json!({
                "status": "deferred_to_workflow",
                "required": false,
                "delivery_policy_default_requires_tests": delivery_policy_default_requires_tests,
            }),
        );
    if request.make_default {
        let agent_harness = converge_agent_workflow_harness(&refreshed)?;
        payload
            .as_object_mut()
            .ok_or_else(|| "Remote add payload must be an object.".to_string())?
            .insert("agent_harness".to_string(), agent_harness);
    }
    Ok(payload)
}

fn ensure_remote_name_available_with_remote_store<S>(store: &S, name: &str) -> Result<(), String>
where
    S: RemoteStore + ?Sized,
{
    let name = normalize_required_text(name, "remote name")?;
    if remote_exists_with_remote_store(store, &name)? {
        return Err(format!("Remote {name} already exists."));
    }
    Ok(())
}

fn remote_add_record_with_remote_store<S>(
    store: &S,
    request: &RemoteAddRecord,
) -> Result<(), String>
where
    S: RemoteStore + ?Sized,
{
    add_remote_with_remote_store(store, request)
}

pub fn remote_list(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let store = repo.remote_store()?;
    remote_list_with_remote_store(&store)
}

fn remote_list_with_remote_store<S>(store: &S) -> Result<JsonValue, String>
where
    S: RemoteStore + ?Sized,
{
    let rows = list_remotes_with_remote_store(store)?
        .iter()
        .map(remote_record_to_json)
        .collect();
    Ok(JsonValue::Array(rows))
}

pub fn remote_get(repo: &RepoRuntime, requested: Option<&str>) -> Result<JsonValue, String> {
    let remote_name = requested
        .and_then(|value| normalize_optional_text(Some(value)))
        .or_else(|| repo.default_remote_name())
        .ok_or_else(|| {
            "No remote configured. Run `ait remote add ... --default` first.".to_string()
        })?;
    let store = repo.remote_store()?;
    remote_get_with_remote_store(&store, &remote_name)
}

fn remote_get_with_remote_store<S>(store: &S, remote_name: &str) -> Result<JsonValue, String>
where
    S: RemoteStore + ?Sized,
{
    let remote_name = normalize_required_text(remote_name, "remote name")?;
    remote_by_name_with_remote_store(store, &remote_name)?
        .map(|record| remote_record_to_json(&record))
        .ok_or_else(|| format!("Unknown remote: {remote_name}"))
}

pub fn remote_add_from_payload(
    repo: &RepoRuntime,
    payload: &JsonValue,
) -> Result<JsonValue, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "remote add payload must decode to an object.".to_string())?;
    let unsupported = obj
        .keys()
        .filter(|field| !matches!(field.as_str(), "name" | "url" | "make_default"))
        .cloned()
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        return Err(format!(
            "remote add payload contains retired or unknown field(s): {}.",
            unsupported.join(", ")
        ));
    }
    remote_add(
        repo,
        &RemoteAddRequest {
            name: string_field(obj, "name")?,
            url: string_field(obj, "url")?,
            make_default: bool_field(obj, "make_default")?,
        },
    )
}

fn remote_record_to_json(record: &RemoteRecord) -> JsonValue {
    json!({
        "remote_id": record.remote_id,
        "name": record.name.as_str(),
        "url": record.url.as_str(),
        "repo_name": record.repo_name.as_deref(),
        "is_default_push": record.is_default_push,
        "is_default_pull": record.is_default_pull,
        "created_at": record.created_at.as_str(),
    })
}

pub(crate) fn set_default_remote(repo: &RepoRuntime, name: &str) -> Result<(), String> {
    let path = repo.root.join(".ait").join("config.json");
    update_json_object(&path, |config| {
        config.insert(
            "default_remote".to_string(),
            JsonValue::String(name.to_string()),
        );
    })
}

fn persist_repository_index(repo: &RepoRuntime, repository_index: u32) -> Result<(), String> {
    let path = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("config.json");
    let mut config = read_json_object(&path)?;
    match config.get("repository_index") {
        Some(value) if value.as_u64() == Some(u64::from(repository_index)) => return Ok(()),
        Some(value) => {
            return Err(format!(
                "Refusing to replace existing repository_index {} with newly registered index {repository_index}.",
                value
            ))
        }
        None => {}
    }
    config.insert(
        "repository_index".to_string(),
        JsonValue::from(repository_index),
    );
    write_json_object_atomically(&path, config)
}

fn update_json_object<F>(path: &Path, apply: F) -> Result<(), String>
where
    F: FnOnce(&mut JsonMap<String, JsonValue>),
{
    let mut config = read_json_object(path)?;
    apply(&mut config);
    write_json_object_atomically(path, config)
}

fn write_json_object_atomically(
    path: &Path,
    config: JsonMap<String, JsonValue>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Config path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|err| {
        format!(
            "Failed to create config directory {}: {err}",
            parent.display()
        )
    })?;
    let encoded = encode_value_pretty_with_newline_error_string(&JsonValue::Object(config))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config.json");
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_nanos();
    let mut staged = None;
    for attempt in 0_u8..=16 {
        let candidate = parent.join(format!(
            ".{file_name}.{}-{nonce}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                staged = Some((candidate, file));
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(format!(
                    "Failed to stage config update for {}: {err}",
                    path.display()
                ))
            }
        }
    }
    let (staged_path, mut staged_file) = staged.ok_or_else(|| {
        format!(
            "Failed to allocate a unique staged config path for {}.",
            path.display()
        )
    })?;
    if let Ok(metadata) = fs::metadata(path) {
        if let Err(err) = staged_file.set_permissions(metadata.permissions()) {
            let _ = fs::remove_file(&staged_path);
            return Err(format!(
                "Failed to preserve config permissions for {}: {err}",
                path.display()
            ));
        }
    }
    let stage_result = staged_file
        .write_all(encoded.as_bytes())
        .and_then(|_| staged_file.sync_all());
    if let Err(err) = stage_result {
        let _ = fs::remove_file(&staged_path);
        return Err(format!(
            "Failed to durably stage config update for {}: {err}",
            path.display()
        ));
    }
    drop(staged_file);
    fs::rename(&staged_path, path).map_err(|err| {
        let _ = fs::remove_file(&staged_path);
        format!(
            "Failed to atomically replace config {}: {err}",
            path.display()
        )
    })?;
    if let Ok(parent_directory) = fs::File::open(parent) {
        let _ = parent_directory.sync_all();
    }
    Ok(())
}

fn read_json_object(path: &Path) -> Result<JsonMap<String, JsonValue>, String> {
    match fs::read_to_string(path) {
        Ok(content) => {
            match parse_value(&content, &format!("Failed to parse {}", path.display())) {
                Ok(JsonValue::Object(obj)) => Ok(obj),
                Ok(_) => Err(format!(
                    "Config file {} must contain a JSON object.",
                    path.display()
                )),
                Err(err) => Err(err),
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(JsonMap::new()),
        Err(err) => Err(err.to_string()),
    }
}

fn normalize_required_text(value: &str, field: &str) -> Result<String, String> {
    normalize_optional_text(Some(value)).ok_or_else(|| format!("{field} is required."))
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn string_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Result<String, String> {
    obj.get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("remote add payload requires `{field}`."))
}

fn bool_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Result<bool, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(false),
        Some(JsonValue::Bool(value)) => Ok(*value),
        Some(_) => Err(format!(
            "remote add payload field `{field}` must be a boolean."
        )),
    }
}

#[cfg(test)]
mod tests;
