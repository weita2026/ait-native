use crate::agent_harness::converge_agent_workflow_harness;
use crate::json_support::{encode_value_pretty_with_newline_error_string, parse_value};
use crate::primitives::{snapshot_show, workflow_workspace_status};
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
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

const PATCH_CI_RELATIVE_PATH: &str = "ci/patch_ci.json";
const PATCH_CI_PLACEHOLDER_COMMAND: &str = "CONFIGURE_PATCHSET_TEST_COMMAND";

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
    let normalized_request = RemoteAddRequest {
        name: name.clone(),
        url: url.clone(),
        make_default: request.make_default,
    };
    let patch_ci = prepare_remote_registration_patch_ci(repo, &normalized_request)?;
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
        .insert("patch_ci".to_string(), patch_ci);
    if request.make_default {
        let agent_harness = converge_agent_workflow_harness(&refreshed)?;
        payload
            .as_object_mut()
            .ok_or_else(|| "Remote add payload must be an object.".to_string())?
            .insert("agent_harness".to_string(), agent_harness);
    }
    Ok(payload)
}

fn prepare_remote_registration_patch_ci(
    repo: &RepoRuntime,
    request: &RemoteAddRequest,
) -> Result<JsonValue, String> {
    let required = local_policy_requires_tests(repo)?;
    if !required {
        return Ok(patch_ci_readiness_payload(
            false,
            "not_required",
            None,
            Vec::new(),
        ));
    }

    let root = repo.workspace_root();
    let manifest_path = root.join(PATCH_CI_RELATIVE_PATH);
    let manifest_text = match read_patch_ci_manifest(&manifest_path) {
        Ok(Some(text)) => text,
        Ok(None) => {
            let template = patch_ci_template(&root);
            let encoded = encode_value_pretty_with_newline_error_string(&template.manifest)?;
            write_new_patch_ci_manifest(&manifest_path, encoded.as_bytes())?;
            return Err(generated_patch_ci_message(request, &template));
        }
        Err(err) => return Err(patch_ci_not_ready_message(request, &err)),
    };
    let blocking_suite_ids = validate_patch_ci_manifest(&manifest_text)
        .map_err(|err| patch_ci_not_ready_message(request, &err))?;
    let snapshot_id = patch_ci_snapshot_admission(repo, manifest_text.as_bytes())
        .map_err(|err| patch_ci_not_ready_message(request, &err))?;

    Ok(patch_ci_readiness_payload(
        true,
        "ready",
        Some(snapshot_id),
        blocking_suite_ids,
    ))
}

fn read_patch_ci_manifest(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "{} must be a regular repository file, not a symbolic link.",
                    PATCH_CI_RELATIVE_PATH
                ));
            }
            if !metadata.is_file() {
                return Err(format!(
                    "{} must be a regular file.",
                    PATCH_CI_RELATIVE_PATH
                ));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "Could not inspect {}: {err}",
                PATCH_CI_RELATIVE_PATH
            ));
        }
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|err| format!("Could not read {}: {err}", PATCH_CI_RELATIVE_PATH))
}

fn write_new_patch_ci_manifest(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory.", path.display()))?;
    match fs::symlink_metadata(parent) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "Refusing to create {} because {} is not a regular directory.",
                    PATCH_CI_RELATIVE_PATH,
                    parent.display()
                ));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        Err(err) => {
            return Err(format!("Could not inspect {}: {err}", parent.display()));
        }
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| {
            format!(
                "Could not create {} without overwriting existing content: {err}",
                PATCH_CI_RELATIVE_PATH
            )
        })?;
    if let Err(err) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(format!(
            "Could not finish writing {}: {err}",
            PATCH_CI_RELATIVE_PATH
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct PatchCiTemplate {
    manifest: JsonValue,
    commands: Vec<String>,
}

fn patch_ci_template(_: &Path) -> PatchCiTemplate {
    let commands = vec![PATCH_CI_PLACEHOLDER_COMMAND.to_string()];
    let manifest = json!({
        "schema_version": 1,
        "suites": [{
            "schema_version": 1,
            "suite_id": "patchset_gate",
            "display_name": "Patchset Gate",
            "plane": "patchset",
            "default_blocking": true,
            "mode": "gate",
            "purpose": "Validate this repository before remote finish.",
            "runner": {
                "kind": "command_bundle",
                "commands": commands.clone(),
            },
            "artifacts": {
                "log_path": ".ait/generated/ci/patchset_gate.log",
            },
        }],
    });

    PatchCiTemplate { manifest, commands }
}

fn validate_patch_ci_manifest(text: &str) -> Result<Vec<String>, String> {
    let manifest = parse_value(text, "Invalid ci/patch_ci.json")?;
    let object = manifest
        .as_object()
        .ok_or_else(|| "ci/patch_ci.json must contain a JSON object.".to_string())?;
    if object.get("schema_version").and_then(JsonValue::as_i64) != Some(1) {
        return Err("ci/patch_ci.json must set top-level `schema_version` to 1.".to_string());
    }
    let suites = object
        .get("suites")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "ci/patch_ci.json must contain a `suites` array.".to_string())?;
    let mut blocking_suite_ids = Vec::new();
    for suite in suites {
        let Some(suite_obj) = suite.as_object() else {
            continue;
        };
        if suite_obj.get("plane").and_then(JsonValue::as_str) != Some("patchset")
            || suite_obj.get("mode").and_then(JsonValue::as_str) != Some("gate")
            || suite_obj
                .get("default_blocking")
                .and_then(JsonValue::as_bool)
                != Some(true)
        {
            continue;
        }
        let suite_id = suite_obj
            .get("suite_id")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalize_optional_text(Some(value)))
            .ok_or_else(|| {
                "Every blocking Patchset gate must define a non-empty `suite_id`.".to_string()
            })?;
        validate_patch_ci_runner(suite_obj, &suite_id)?;
        blocking_suite_ids.push(suite_id);
    }
    blocking_suite_ids.sort();
    blocking_suite_ids.dedup();
    if blocking_suite_ids.is_empty() {
        return Err(
            "ci/patch_ci.json must contain at least one suite with `plane: patchset`, \
             `mode: gate`, and `default_blocking: true`."
                .to_string(),
        );
    }
    Ok(blocking_suite_ids)
}

fn validate_patch_ci_runner(
    suite: &JsonMap<String, JsonValue>,
    suite_id: &str,
) -> Result<(), String> {
    let runner = suite
        .get("runner")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Patchset CI suite {suite_id:?} must define a `runner` object."))?;
    let kind = runner
        .get("kind")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_optional_text(Some(value)))
        .ok_or_else(|| {
            format!("Patchset CI suite {suite_id:?} must define a non-empty `runner.kind`.")
        })?;
    if kind != "command_bundle" {
        return Ok(());
    }
    let commands = runner
        .get("commands")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            format!("Patchset CI command-bundle suite {suite_id:?} must define `runner.commands`.")
        })?;
    if commands.is_empty() {
        return Err(format!(
            "Patchset CI command-bundle suite {suite_id:?} must contain at least one command."
        ));
    }
    for command in commands {
        let command = command
            .as_str()
            .and_then(|value| normalize_optional_text(Some(value)))
            .ok_or_else(|| {
                format!(
                    "Patchset CI command-bundle suite {suite_id:?} contains an empty or non-string command."
                )
            })?;
        if command == PATCH_CI_PLACEHOLDER_COMMAND {
            return Err(format!(
                "Patchset CI suite {suite_id:?} still contains the generated placeholder \
                 {PATCH_CI_PLACEHOLDER_COMMAND:?}; replace it with the repository test command."
            ));
        }
    }
    Ok(())
}

fn patch_ci_snapshot_admission(
    repo: &RepoRuntime,
    manifest_bytes: &[u8],
) -> Result<String, String> {
    let status = workflow_workspace_status(repo, None, None).map_err(|err| {
        format!("Could not verify the current Line Snapshot for Patchset CI: {err}")
    })?;
    let snapshot_id = status
        .get("baseline_snapshot_id")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_optional_text(Some(value)))
        .ok_or_else(|| {
            format!(
                "{} is configured but the current Line has no head Snapshot.",
                PATCH_CI_RELATIVE_PATH
            )
        })?;
    let snapshot = snapshot_show(repo, &snapshot_id).map_err(|err| {
        format!("Could not inspect current Line head Snapshot {snapshot_id}: {err}")
    })?;
    let file = snapshot
        .get("files")
        .and_then(JsonValue::as_array)
        .and_then(|files| {
            files.iter().find(|file| {
                file.get("path").and_then(JsonValue::as_str) == Some(PATCH_CI_RELATIVE_PATH)
            })
        })
        .ok_or_else(|| {
            format!(
                "{} is configured but is not included in current Line head Snapshot {snapshot_id}.",
                PATCH_CI_RELATIVE_PATH
            )
        })?;
    let expected_sha256 = file
        .get("sha256")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            format!(
                "Current Line head Snapshot {snapshot_id} has no SHA-256 for {}.",
                PATCH_CI_RELATIVE_PATH
            )
        })?;
    let actual_sha256 = format!("{:x}", Sha256::digest(manifest_bytes));
    if expected_sha256 != actual_sha256 {
        return Err(format!(
            "{} differs from current Line head Snapshot {snapshot_id}.",
            PATCH_CI_RELATIVE_PATH
        ));
    }
    Ok(snapshot_id)
}

fn patch_ci_readiness_payload(
    required: bool,
    status: &str,
    snapshot_id: Option<String>,
    blocking_suite_ids: Vec<String>,
) -> JsonValue {
    json!({
        "status": status,
        "required": required,
        "manifest_path": PATCH_CI_RELATIVE_PATH,
        "snapshot_id": snapshot_id,
        "blocking_suite_ids": blocking_suite_ids,
        "configuration": {
            "commands_field": "suites[].runner.commands",
            "required_plane": "patchset",
            "required_mode": "gate",
            "default_blocking": true,
            "changes_require_new_snapshot": true,
        },
    })
}

fn generated_patch_ci_message(request: &RemoteAddRequest, template: &PatchCiTemplate) -> String {
    let commands = template
        .commands
        .iter()
        .map(|command| format!("  - {command}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Created a language-neutral {PATCH_CI_RELATIVE_PATH} starter.\n\
         No project manifests were inspected and no validation command was inferred.\n\
         Remote registration was not attempted. Replace \
         {PATCH_CI_PLACEHOLDER_COMMAND:?} before retrying.\n\n\
         Configure the repository-authored validation command:\n{commands}\n\n\
         Keep at least one suite configured with:\n\
           plane: patchset\n\
           mode: gate\n\
           default_blocking: true\n\
         Edit test commands at `suites[].runner.commands`, then run:\n\
           ait snapshot create --message \"Configure Patchset CI\"\n\
           {}",
        remote_add_retry_command(request)
    )
}

fn patch_ci_not_ready_message(request: &RemoteAddRequest, detail: &str) -> String {
    format!(
        "Patchset CI is not ready for remote registration: {detail}\n\
         Remote registration was not attempted and existing configuration was not overwritten.\n\n\
         Configure {PATCH_CI_RELATIVE_PATH} with at least one blocking Patchset gate:\n\
           plane: patchset\n\
           mode: gate\n\
           default_blocking: true\n\
           runner.commands: [\"your test command\"]\n\
         Include the exact manifest bytes in the current Line head, then retry:\n\
           ait snapshot create --message \"Configure Patchset CI\"\n\
           {}",
        remote_add_retry_command(request)
    )
}

fn remote_add_retry_command(request: &RemoteAddRequest) -> String {
    let mut command = vec![
        "ait".to_string(),
        "remote".to_string(),
        "add".to_string(),
        shell_quote(&request.name),
        shell_quote(&request.url),
    ];
    if request.make_default {
        command.push("--default".to_string());
    }
    command.join(" ")
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_./:@%+=".contains(&byte))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
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
