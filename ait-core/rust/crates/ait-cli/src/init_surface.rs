use crate::agent_harness::converge_agent_workflow_harness;
use crate::config_surface::config_show;
use crate::json_support::{encode_value_pretty_with_newline_error_string, parse_value};
use crate::runtime::{RepoBinaryDbStoreFactory, RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT};
use crate::task_worktree_layout::detect_init_task_worktree_defaults;
use ait_core::binary_db::{AuthorityId, LocalStateScope};
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::line_store::LineStore;
use ait_core::policy::{
    parse_policy_yaml, POLICY_AUTHOR_CLASSES, POLICY_CONTENT_CLASSES, POLICY_REQUIREMENT_FLAGS,
};
use chrono::Utc;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::{Builder, NamedTempFile};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const APP_DIR: &str = ".ait";
const CONFIG_NAME: &str = "config.json";
const BINARY_DB_DIR: &str = "binary-db";
const DEFAULT_POLICY_PROFILE: &str = "prototype";
const REPOSITORY_DIRECTORIES: &[&str] = &[
    "objects/manifests",
    "objects/packs",
    "objects/tree-packs",
    "refs/lines",
    "workspace/locks",
    "workspace/worktrees",
    "worktrees",
    BINARY_DB_DIR,
];

#[derive(Clone, Debug)]
pub struct InitRequest {
    pub root: PathBuf,
    pub name: Option<String>,
    pub default_line: String,
    pub policy_profile: String,
    pub default_author_mode: String,
    pub default_model: Option<String>,
    pub repair_existing: bool,
}

#[derive(Clone, Debug)]
struct ValidatedInitRequest {
    name: Option<String>,
    default_line: String,
    policy_profile: String,
    policy_yaml: String,
    default_author_mode: String,
    default_model: Option<String>,
    sprint_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InitAction {
    Initialized,
    Reinitialized,
    Repaired,
}

impl InitAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initialized => "initialized",
            Self::Reinitialized => "reinitialized",
            Self::Repaired => "repaired",
        }
    }
}

#[derive(Debug)]
struct ExistingInitPlan {
    config: JsonMap<String, JsonValue>,
    config_changed: bool,
    policy_yaml: Option<String>,
    missing_directories: Vec<&'static str>,
    line_missing: bool,
    repairs: Vec<String>,
}

pub fn init_repo(request: &InitRequest) -> Result<JsonValue, String> {
    init_repo_with_agent_contract(request, true)
}

pub(crate) fn init_repo_for_install(
    request: &InitRequest,
    sprint_enabled: bool,
) -> Result<JsonValue, String> {
    init_repo_with_agent_contract(request, sprint_enabled)
}

/// Initializes an isolated recovery staging repository with only the
/// repository-authority portion of init. It deliberately omits working-tree
/// agent and sprint artifacts.
pub(crate) fn init_repo_for_remote_head_recovery(
    request: &InitRequest,
) -> Result<JsonValue, String> {
    init_repo_impl(request, true)
}

fn init_repo_with_agent_contract(
    request: &InitRequest,
    sprint_enabled: bool,
) -> Result<JsonValue, String> {
    let payload = init_repo_impl(request, sprint_enabled)?;
    let repo = RepoRuntime::discover_from_path(&request.root)?;
    converge_agent_workflow_harness(&repo)?;
    Ok(payload)
}

fn init_repo_impl(request: &InitRequest, sprint_enabled: bool) -> Result<JsonValue, String> {
    let root = request.root.canonicalize().map_err(|error| {
        format!(
            "Failed to resolve repository root {}: {error}",
            request.root.display()
        )
    })?;
    require_real_directory(&root, "Repository root")?;
    let validated = validate_init_request(request, sprint_enabled)?;
    let ait_dir = root.join(APP_DIR);
    let (action, repairs) = match classify_ait_directory(&ait_dir)? {
        None => {
            if request.repair_existing {
                return Err(format!(
                    "{} does not exist; --repair-existing only applies to an existing .ait directory",
                    ait_dir.display()
                ));
            }
            initialize_fresh_repo(&root, &ait_dir, &validated)?;
            (InitAction::Initialized, Vec::new())
        }
        Some(()) => {
            reinitialize_existing_repo(&root, &ait_dir, &validated, request.repair_existing)?
        }
    };

    let repo = RepoRuntime::discover_from_path(&root)?;
    let config_payload = config_show(&repo)?;
    let default_line = repo.default_line_name();
    Ok(json!({
        "action": action.as_str(),
        "repo_root": repo.authoritative_repo_root().to_string_lossy().to_string(),
        "authority_path": repo.ait_dir.to_string_lossy().to_string(),
        "repo_name": repo.repo_name(),
        "default_line": default_line,
        "workflow_mode": config_payload.get("workflow_mode").cloned().unwrap_or(JsonValue::Null),
        "task_worktree": config_payload.get("task_worktree").cloned().unwrap_or(JsonValue::Null),
        "policy_profile": repo.config.get("policy_profile").cloned().unwrap_or(JsonValue::Null),
        "default_author_mode": repo.config.get("default_author_mode").cloned().unwrap_or(JsonValue::Null),
        "default_model": repo.config.get("default_model").cloned().unwrap_or(JsonValue::Null),
        "repairs": repairs,
    }))
}

pub fn render_human_init(payload: &JsonValue) {
    let action = payload
        .get("action")
        .and_then(JsonValue::as_str)
        .unwrap_or("initialized");
    let verb = match action {
        "reinitialized" => "Reinitialized existing",
        "repaired" => "Repaired existing",
        _ => "Initialized empty",
    };
    println!(
        "{verb} AIT repository in {}",
        display_value(payload.get("authority_path"))
    );
}

fn ensure_repo_dirs(ait_dir: &Path) -> Result<(), String> {
    for relative in REPOSITORY_DIRECTORIES {
        create_real_directory_tree(ait_dir, relative)?;
    }
    Ok(())
}

fn initialize_binary_db(
    root: &Path,
    ait_dir: &Path,
    repo_name: &str,
    default_line: &str,
) -> Result<(), String> {
    let authority_root = ait_dir.join(BINARY_DB_DIR);
    create_real_directory_tree(ait_dir, BINARY_DB_DIR)?;
    let stores = RepoBinaryDbStoreFactory::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>::new(
        root,
        authority_root,
        AuthorityId::new(format!("local:{repo_name}")),
        LocalStateScope::Repository,
    );
    let lines = stores.lines();
    let now = now_string();
    let existing_head = read_line_ref(ait_dir, default_line)?;
    if lines.line_by_name(default_line)?.is_none() {
        lines.create_line(default_line, existing_head.as_deref(), &now)?;
    }
    Ok(())
}

fn validate_init_request(
    request: &InitRequest,
    sprint_enabled: bool,
) -> Result<ValidatedInitRequest, String> {
    let name = match request.name.as_deref() {
        Some(value) => Some(required_normalized_text(value, "Repository name")?),
        None => None,
    };
    let default_line = required_normalized_text(&request.default_line, "Default line")?;
    let policy_profile = normalize_policy_profile_name(&request.policy_profile)?;
    let policy_yaml = policy_profile_yaml(&policy_profile)?;
    let default_author_mode = normalize_author_mode(&request.default_author_mode)?;
    let default_model = match request.default_model.as_deref() {
        Some(value) => Some(required_normalized_text(value, "Default model")?),
        None => None,
    };
    Ok(ValidatedInitRequest {
        name,
        default_line,
        policy_profile,
        policy_yaml,
        default_author_mode,
        default_model,
        sprint_enabled,
    })
}

fn initialize_fresh_repo(
    root: &Path,
    ait_dir: &Path,
    request: &ValidatedInitRequest,
) -> Result<(), String> {
    let repo_name = requested_repo_name(root, request);
    let config = fresh_config(root, ait_dir, &repo_name, request);
    let staged = Builder::new()
        .prefix(".ait-init-")
        .tempdir_in(root)
        .map_err(|error| format!("Failed to stage repository initialization: {error}"))?;
    let staged_ait_dir = staged.path();
    ensure_repo_dirs(staged_ait_dir)?;
    write_json_pretty(
        &staged_ait_dir.join(CONFIG_NAME),
        &JsonValue::Object(config),
    )?;
    write_text_atomically(
        &staged_ait_dir.join("policy.yaml"),
        &request.policy_yaml,
        0o600,
    )?;
    initialize_binary_db(root, staged_ait_dir, &repo_name, &request.default_line)?;
    validate_no_symlinks_below(&staged_ait_dir.join(BINARY_DB_DIR))?;
    if classify_ait_directory(ait_dir)?.is_some() {
        return Err(format!(
            "Refusing to replace repository authority created concurrently at {}",
            ait_dir.display()
        ));
    }
    fs::rename(staged_ait_dir, ait_dir).map_err(|error| {
        format!(
            "Failed to atomically install repository authority {}: {error}",
            ait_dir.display()
        )
    })?;
    std::mem::forget(staged);
    sync_directory(root)?;
    Ok(())
}

fn fresh_config(
    root: &Path,
    ait_dir: &Path,
    repo_name: &str,
    request: &ValidatedInitRequest,
) -> JsonMap<String, JsonValue> {
    let mut config = JsonMap::new();
    config.insert(
        "repo_name".to_string(),
        JsonValue::String(repo_name.to_string()),
    );
    config.insert(
        "default_line".to_string(),
        JsonValue::String(request.default_line.clone()),
    );
    config.insert(
        "current_line".to_string(),
        JsonValue::String(request.default_line.clone()),
    );
    config.insert("default_remote".to_string(), JsonValue::Null);
    config.insert(
        "id_namespace_prefix".to_string(),
        JsonValue::String(String::new()),
    );
    config.insert(
        "policy_profile".to_string(),
        JsonValue::String(request.policy_profile.clone()),
    );
    config.insert(
        "default_author_mode".to_string(),
        JsonValue::String(request.default_author_mode.clone()),
    );
    config.insert(
        "sprint".to_string(),
        JsonValue::String(if request.sprint_enabled { "on" } else { "off" }.to_string()),
    );
    config.insert(
        "plan_task_binding".to_string(),
        json!({"mode": if request.sprint_enabled { "required" } else { "off" }}),
    );
    if let Some(model) = request.default_model.as_ref() {
        config.insert(
            "default_model".to_string(),
            JsonValue::String(model.clone()),
        );
    }
    let repo_for_detection = RepoRuntime {
        root: root.to_path_buf(),
        ait_dir: ait_dir.to_path_buf(),
        config: config.clone(),
        worktree_config_path: None,
    };
    if let Some(JsonValue::Object(defaults)) =
        detect_init_task_worktree_defaults(&repo_for_detection)
    {
        if !defaults.is_empty() {
            config.insert("task_worktree".to_string(), JsonValue::Object(defaults));
        }
    }
    config
}

fn reinitialize_existing_repo(
    root: &Path,
    ait_dir: &Path,
    request: &ValidatedInitRequest,
    repair_existing: bool,
) -> Result<(InitAction, Vec<String>), String> {
    let plan = prepare_existing_init(root, ait_dir, request)?;
    if plan.repairs.is_empty() {
        return Ok((InitAction::Reinitialized, Vec::new()));
    }
    if !repair_existing {
        return Err(format!(
            "AIT repository at {} is incomplete ({}); rerun with --repair-existing to complete only the missing structure",
            ait_dir.display(),
            plan.repairs.join(", ")
        ));
    }
    for relative in &plan.missing_directories {
        create_real_directory_tree(ait_dir, relative)?;
    }
    if plan.line_missing {
        let repo_name = required_config_text(&plan.config, "repo_name")?;
        let default_line = required_config_text(&plan.config, "default_line")?;
        initialize_binary_db(root, ait_dir, &repo_name, &default_line)?;
    }
    if let Some(policy_yaml) = plan.policy_yaml.as_deref() {
        write_text_atomically(&ait_dir.join("policy.yaml"), policy_yaml, 0o600)?;
    }
    if plan.config_changed {
        write_json_pretty(&ait_dir.join(CONFIG_NAME), &JsonValue::Object(plan.config))?;
    }
    Ok((InitAction::Repaired, plan.repairs))
}

fn prepare_existing_init(
    root: &Path,
    ait_dir: &Path,
    request: &ValidatedInitRequest,
) -> Result<ExistingInitPlan, String> {
    let mut missing_directories = Vec::new();
    for relative in REPOSITORY_DIRECTORIES {
        if directory_tree_is_missing(ait_dir, relative)? {
            missing_directories.push(*relative);
        }
    }
    let existing_config = read_json_object(&ait_dir.join(CONFIG_NAME))?;
    let existing_policy = read_policy_id(&ait_dir.join("policy.yaml"))?;
    let (config, config_changed, policy_yaml) =
        prepare_existing_config(root, ait_dir, existing_config, existing_policy, request)?;
    let repo_name = required_config_text(&config, "repo_name")?;
    let default_line = required_config_text(&config, "default_line")?;
    read_line_ref(ait_dir, &default_line)?;
    let binary_directory_missing = missing_directories.contains(&BINARY_DB_DIR);
    let line_missing = if binary_directory_missing {
        true
    } else {
        validate_no_symlinks_below(&ait_dir.join(BINARY_DB_DIR))?;
        let stores = RepoBinaryDbStoreFactory::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>::new(
            root,
            ait_dir.join(BINARY_DB_DIR),
            AuthorityId::new(format!("local:{repo_name}")),
            LocalStateScope::Repository,
        );
        stores.lines().line_by_name(&default_line)?.is_none()
    };
    let mut repairs = Vec::new();
    if config_changed {
        repairs.push(format!("missing or incomplete {CONFIG_NAME}"));
    }
    if policy_yaml.is_some() {
        repairs.push("missing policy.yaml".to_string());
    }
    if !missing_directories.is_empty() {
        repairs.push(format!(
            "missing directories: {}",
            missing_directories.join(", ")
        ));
    }
    if line_missing {
        repairs.push(format!("missing Binary line {default_line}"));
    }
    Ok(ExistingInitPlan {
        config,
        config_changed,
        policy_yaml,
        missing_directories,
        line_missing,
        repairs,
    })
}

fn prepare_existing_config(
    root: &Path,
    ait_dir: &Path,
    existing_config: Option<JsonMap<String, JsonValue>>,
    existing_policy_id: Option<String>,
    request: &ValidatedInitRequest,
) -> Result<(JsonMap<String, JsonValue>, bool, Option<String>), String> {
    let config_was_missing = existing_config.is_none();
    let policy_was_missing = existing_policy_id.is_none();
    let mut config = existing_config.unwrap_or_default();
    validate_known_config_fields(&config)?;
    let mut changed = config_was_missing;

    ensure_config_text(
        &mut config,
        "repo_name",
        requested_repo_name(root, request),
        &mut changed,
    )?;
    let default_line = ensure_config_text(
        &mut config,
        "default_line",
        request.default_line.clone(),
        &mut changed,
    )?;
    ensure_config_text(
        &mut config,
        "current_line",
        default_line.clone(),
        &mut changed,
    )?;
    ensure_config_value(&mut config, "default_remote", JsonValue::Null, &mut changed);
    ensure_config_value(
        &mut config,
        "id_namespace_prefix",
        JsonValue::String(String::new()),
        &mut changed,
    );

    let configured_policy_id = config
        .get("policy_profile")
        .map(|value| exact_nonempty_json_text(value, "config.policy_profile"))
        .transpose()?;
    let policy_id = match (configured_policy_id, existing_policy_id) {
        (Some(configured), Some(on_disk)) if configured != on_disk => {
            return Err(format!(
            "config.policy_profile `{configured}` does not match policy.yaml policy_id `{on_disk}`"
        ))
        }
        (Some(configured), _) => configured,
        (None, Some(on_disk)) => on_disk,
        (None, None) => request.policy_profile.clone(),
    };
    if !config.contains_key("policy_profile") {
        config.insert(
            "policy_profile".to_string(),
            JsonValue::String(policy_id.clone()),
        );
        changed = true;
    }

    let configured_author_mode = config
        .get("default_author_mode")
        .map(|value| exact_nonempty_json_text(value, "config.default_author_mode"))
        .transpose()?;
    match configured_author_mode {
        Some(value) => {
            let normalized = normalize_author_mode(&value)?;
            if normalized != value {
                return Err(
                    "config.default_author_mode must not contain surrounding whitespace"
                        .to_string(),
                );
            }
        }
        None => {
            config.insert(
                "default_author_mode".to_string(),
                JsonValue::String(request.default_author_mode.clone()),
            );
            changed = true;
        }
    }
    if config_was_missing {
        config.insert(
            "sprint".to_string(),
            JsonValue::String(if request.sprint_enabled { "on" } else { "off" }.to_string()),
        );
        config.insert(
            "plan_task_binding".to_string(),
            json!({"mode": if request.sprint_enabled { "required" } else { "off" }}),
        );
        if let Some(model) = request.default_model.as_ref() {
            config.insert(
                "default_model".to_string(),
                JsonValue::String(model.clone()),
            );
        }
        let repo_for_detection = RepoRuntime {
            root: root.to_path_buf(),
            ait_dir: ait_dir.to_path_buf(),
            config: config.clone(),
            worktree_config_path: None,
        };
        if let Some(JsonValue::Object(defaults)) =
            detect_init_task_worktree_defaults(&repo_for_detection)
        {
            if !defaults.is_empty() {
                config.insert("task_worktree".to_string(), JsonValue::Object(defaults));
            }
        }
    }
    let policy_yaml = if policy_was_missing {
        Some(policy_profile_yaml(&policy_id).map_err(|_| {
            format!(
                "Cannot reconstruct missing policy.yaml for custom policy `{policy_id}`; restore the policy file before repair"
            )
        })?)
    } else {
        None
    };
    Ok((config, changed, policy_yaml))
}

fn policy_profile_yaml(name: &str) -> Result<String, String> {
    let profile = normalize_policy_profile_name(name)?;
    let (require_lint, require_security_scan, require_license_scan) = match profile.as_str() {
        "prototype" => (false, false, false),
        "team" => (true, false, false),
        "release" => (true, true, true),
        _ => unreachable!("normalized policy profile"),
    };
    Ok(format!(
        r#"version: 1
policy_id: {profile}
defaults:
  require_attestation: true
  require_tests: true
  require_lint: {require_lint}
  require_security_scan: {require_security_scan}
  require_license_scan: {require_license_scan}
  require_ai_provenance: false
  require_code_review_summary: false
class_overrides:
  - when:
      content_class: docs_only
    set:
      require_tests: false
      require_lint: false
      require_security_scan: false
      require_license_scan: false
"#
    ))
}

fn normalize_policy_profile_name(name: &str) -> Result<String, String> {
    let normalized = required_normalized_text(name, "Policy profile")?;
    match normalized.to_ascii_lowercase().as_str() {
        "prototype" | "team" | "release" => Ok(normalized.to_ascii_lowercase()),
        _ => Err(format!("Unknown policy profile: {name}")),
    }
}

fn normalize_author_mode(value: &str) -> Result<String, String> {
    let normalized = required_normalized_text(value, "Default author mode")?;
    match normalized.as_str() {
        "human_only" | "human_with_ai_assist" | "ai_with_human_review"
        | "ai_only_experimental" => Ok(normalized),
        _ => Err(
            "Unknown author_mode. Expected one of: human_only, human_with_ai_assist, ai_with_human_review, ai_only_experimental"
                .to_string(),
        ),
    }
}

fn requested_repo_name(root: &Path, request: &ValidatedInitRequest) -> String {
    request
        .name
        .clone()
        .or_else(|| {
            root.file_name()
                .and_then(|value| value.to_str())
                .and_then(|value| normalize_text(Some(value)))
        })
        .unwrap_or_else(|| "repo".to_string())
}

fn required_normalized_text(value: &str, field: &str) -> Result<String, String> {
    let normalized =
        normalize_text(Some(value)).ok_or_else(|| format!("{field} must not be empty"))?;
    if normalized.chars().any(char::is_control) {
        return Err(format!("{field} must not contain control characters"));
    }
    Ok(normalized)
}

fn exact_nonempty_json_text(value: &JsonValue, field: &str) -> Result<String, String> {
    let text = value
        .as_str()
        .ok_or_else(|| format!("{field} must be a JSON string"))?;
    let normalized = required_normalized_text(text, field)?;
    if normalized != text {
        return Err(format!("{field} must not contain surrounding whitespace"));
    }
    Ok(normalized)
}

fn validate_known_config_fields(config: &JsonMap<String, JsonValue>) -> Result<(), String> {
    for key in [
        "repo_name",
        "default_line",
        "current_line",
        "policy_profile",
        "default_author_mode",
    ] {
        if let Some(value) = config.get(key) {
            exact_nonempty_json_text(value, &format!("config.{key}"))?;
        }
    }
    if let Some(value) = config.get("default_remote") {
        if !value.is_null() {
            exact_nonempty_json_text(value, "config.default_remote")?;
        }
    }
    if let Some(value) = config.get("id_namespace_prefix") {
        let text = value
            .as_str()
            .ok_or_else(|| "config.id_namespace_prefix must be a JSON string".to_string())?;
        if text.trim() != text {
            return Err(
                "config.id_namespace_prefix must not contain surrounding whitespace".to_string(),
            );
        }
    }
    if let Some(value) = config.get("default_model") {
        if !value.is_null() {
            exact_nonempty_json_text(value, "config.default_model")?;
        }
    }
    if let Some(value) = config.get("task_worktree") {
        if !value.is_null() && !value.is_object() {
            return Err("config.task_worktree must be a JSON object or null".to_string());
        }
    }
    Ok(())
}

fn required_config_text(config: &JsonMap<String, JsonValue>, key: &str) -> Result<String, String> {
    config
        .get(key)
        .ok_or_else(|| format!("config.{key} is required"))
        .and_then(|value| exact_nonempty_json_text(value, &format!("config.{key}")))
}

fn ensure_config_text(
    config: &mut JsonMap<String, JsonValue>,
    key: &str,
    fallback: String,
    changed: &mut bool,
) -> Result<String, String> {
    if let Some(value) = config.get(key) {
        return exact_nonempty_json_text(value, &format!("config.{key}"));
    }
    config.insert(key.to_string(), JsonValue::String(fallback.clone()));
    *changed = true;
    Ok(fallback)
}

fn ensure_config_value(
    config: &mut JsonMap<String, JsonValue>,
    key: &str,
    fallback: JsonValue,
    changed: &mut bool,
) {
    if !config.contains_key(key) {
        config.insert(key.to_string(), fallback);
        *changed = true;
    }
}

fn read_json_object(path: &Path) -> Result<Option<JsonMap<String, JsonValue>>, String> {
    if !regular_file_state(path, "Repository config")? {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed to read repository config {}: {error}",
            path.display()
        )
    })?;
    match parse_value(
        &content,
        &format!("Failed to parse repository config {}", path.display()),
    )? {
        JsonValue::Object(config) => Ok(Some(config)),
        _ => Err(format!(
            "Repository config must contain a JSON object: {}",
            path.display()
        )),
    }
}

fn read_policy_id(path: &Path) -> Result<Option<String>, String> {
    if !regular_file_state(path, "Repository policy")? {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed to read repository policy {}: {error}",
            path.display()
        )
    })?;
    validate_policy_document(&content, path).map(Some)
}

#[derive(Default)]
struct PolicyOverrideValidation {
    when_fields: BTreeSet<String>,
    set_fields: BTreeSet<String>,
}

fn validate_policy_document(content: &str, path: &Path) -> Result<String, String> {
    let mut root_fields = BTreeSet::new();
    let mut default_fields = BTreeSet::new();
    let mut section = "";
    let mut override_section = "";
    let mut current_override: Option<PolicyOverrideValidation> = None;
    let mut policy_id = None;
    for (index, raw_line) in content.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line
            .split_once('#')
            .map(|(left, _)| left)
            .unwrap_or(raw_line)
            .trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if line.contains('\t') {
            return Err(format!(
                "Repository policy {} line {line_number} contains a tab",
                path.display()
            ));
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent % 2 != 0 {
            return Err(format!(
                "Repository policy {} line {line_number} has invalid indentation",
                path.display()
            ));
        }
        let stripped = line.trim();
        match indent {
            0 => {
                finish_policy_override(current_override.take(), path)?;
                override_section = "";
                if matches!(stripped, "defaults:" | "class_overrides:") {
                    section = stripped.trim_end_matches(':');
                    if !root_fields.insert(section.to_string()) {
                        return Err(format!(
                            "Repository policy {} contains duplicate root field `{section}`",
                            path.display()
                        ));
                    }
                    continue;
                }
                section = "";
                let (key, value) = policy_key_value(stripped, path, line_number)?;
                if !matches!(key, "version" | "policy_id") {
                    return Err(format!(
                        "Repository policy {} contains unknown root field `{key}`",
                        path.display()
                    ));
                }
                if !root_fields.insert(key.to_string()) {
                    return Err(format!(
                        "Repository policy {} contains duplicate root field `{key}`",
                        path.display()
                    ));
                }
                if key == "version" && value != "1" {
                    return Err(format!(
                        "Repository policy {} version must be exact integer 1",
                        path.display()
                    ));
                }
                if key == "policy_id" {
                    policy_id = Some(policy_scalar_text(value, path, line_number)?);
                }
            }
            2 if section == "defaults" => {
                let (key, value) = policy_key_value(stripped, path, line_number)?;
                if !POLICY_REQUIREMENT_FLAGS.contains(&key) {
                    return Err(format!(
                        "Repository policy {} defaults contains unknown field `{key}`",
                        path.display()
                    ));
                }
                require_policy_bool(value, path, line_number)?;
                if !default_fields.insert(key.to_string()) {
                    return Err(format!(
                        "Repository policy {} contains duplicate defaults field `{key}`",
                        path.display()
                    ));
                }
            }
            2 if section == "class_overrides" && stripped == "- when:" => {
                finish_policy_override(current_override.take(), path)?;
                current_override = Some(PolicyOverrideValidation::default());
                override_section = "when";
            }
            4 if section == "class_overrides"
                && current_override.is_some()
                && matches!(stripped, "when:" | "set:") =>
            {
                override_section = stripped.trim_end_matches(':');
            }
            6 if section == "class_overrides" && current_override.is_some() => {
                let (key, value) = policy_key_value(stripped, path, line_number)?;
                let current = current_override.as_mut().expect("checked above");
                match override_section {
                    "when" => {
                        let supported = match key {
                            "content_class" => POLICY_CONTENT_CLASSES.contains(&value),
                            "author_class" => POLICY_AUTHOR_CLASSES.contains(&value),
                            _ => false,
                        };
                        if !supported || !current.when_fields.insert(key.to_string()) {
                            return Err(format!(
                                "Repository policy {} line {line_number} has an invalid or duplicate override predicate",
                                path.display()
                            ));
                        }
                    }
                    "set" => {
                        if !POLICY_REQUIREMENT_FLAGS.contains(&key) {
                            return Err(format!(
                                "Repository policy {} line {line_number} has an unknown override field `{key}`",
                                path.display()
                            ));
                        }
                        require_policy_bool(value, path, line_number)?;
                        if !current.set_fields.insert(key.to_string()) {
                            return Err(format!(
                                "Repository policy {} contains duplicate override field `{key}`",
                                path.display()
                            ));
                        }
                    }
                    _ => {
                        return Err(format!(
                        "Repository policy {} line {line_number} is outside an override section",
                        path.display()
                    ))
                    }
                }
            }
            _ => {
                return Err(format!(
                "Repository policy {} line {line_number} is outside the supported policy structure",
                path.display()
            ))
            }
        }
    }
    finish_policy_override(current_override.take(), path)?;
    for required in ["version", "policy_id", "defaults"] {
        if !root_fields.contains(required) {
            return Err(format!(
                "Repository policy {} is missing required root field `{required}`",
                path.display()
            ));
        }
    }
    if default_fields.is_empty() {
        return Err(format!(
            "Repository policy {} defaults must not be empty",
            path.display()
        ));
    }
    let policy_id = policy_id.ok_or_else(|| {
        format!(
            "Repository policy {} is missing a usable policy_id",
            path.display()
        )
    })?;
    let parsed = parse_policy_yaml(content, DEFAULT_POLICY_PROFILE).map_err(|error| {
        format!(
            "Failed to parse repository policy {}: {error}",
            path.display()
        )
    })?;
    if parsed.get("policy_id").and_then(JsonValue::as_str) != Some(policy_id.as_str()) {
        return Err(format!(
            "Repository policy {} did not preserve its policy_id during parsing",
            path.display()
        ));
    }
    Ok(policy_id)
}

fn finish_policy_override(
    current: Option<PolicyOverrideValidation>,
    path: &Path,
) -> Result<(), String> {
    let Some(current) = current else {
        return Ok(());
    };
    if current.when_fields.is_empty() || current.set_fields.is_empty() {
        return Err(format!(
            "Repository policy {} contains an incomplete class override",
            path.display()
        ));
    }
    Ok(())
}

fn policy_key_value<'a>(
    line: &'a str,
    path: &Path,
    line_number: usize,
) -> Result<(&'a str, &'a str), String> {
    let (key, value) = line.split_once(':').ok_or_else(|| {
        format!(
            "Repository policy {} line {line_number} must contain a key/value delimiter",
            path.display()
        )
    })?;
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        return Err(format!(
            "Repository policy {} line {line_number} must contain a non-empty key and value",
            path.display()
        ));
    }
    Ok((key, value))
}

fn policy_scalar_text(value: &str, path: &Path, line_number: usize) -> Result<String, String> {
    let unquoted = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };
    required_normalized_text(unquoted, "Policy scalar").map_err(|_| {
        format!(
            "Repository policy {} line {line_number} contains an empty scalar",
            path.display()
        )
    })
}

fn require_policy_bool(value: &str, path: &Path, line_number: usize) -> Result<(), String> {
    if matches!(value, "true" | "false") {
        Ok(())
    } else {
        Err(format!(
            "Repository policy {} line {line_number} must use exact boolean true or false",
            path.display()
        ))
    }
}

fn write_json_pretty(path: &Path, payload: &JsonValue) -> Result<(), String> {
    let encoded = encode_value_pretty_with_newline_error_string(payload)?;
    write_text_atomically(path, &encoded, 0o600)
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn encode_ref_name(name: &str) -> String {
    let mut out = String::new();
    for byte in name.as_bytes() {
        let ch = *byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

fn line_ref_path(ait_dir: &Path, line_name: &str) -> PathBuf {
    ait_dir
        .join("refs")
        .join("lines")
        .join(encode_ref_name(line_name))
}

fn read_line_ref(ait_dir: &Path, line_name: &str) -> Result<Option<String>, String> {
    let path = line_ref_path(ait_dir, line_name);
    if !regular_file_state(&path, "Line reference")? {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("Failed to read line reference {}: {error}", path.display()))?;
    let value = required_normalized_text(&content, "Line reference")
        .map_err(|_| format!("Line reference must not be empty: {}", path.display()))?;
    if value.chars().any(char::is_whitespace) {
        return Err(format!(
            "Line reference must contain one snapshot id: {}",
            path.display()
        ));
    }
    Ok(Some(value))
}

fn classify_ait_directory(path: &Path) -> Result<Option<()>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "Refusing symbolic-link repository authority: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_dir() => Err(format!(
            "Repository authority must be a directory: {}",
            path.display()
        )),
        Ok(_) => Ok(Some(())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Failed to inspect repository authority {}: {error}",
            path.display()
        )),
    }
}

fn require_real_directory(path: &Path, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "{label} must not be a symbolic link: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_dir() => {
            Err(format!("{label} must be a directory: {}", path.display()))
        }
        Ok(_) => Ok(()),
        Err(error) => Err(format!(
            "Failed to inspect {label} {}: {error}",
            path.display()
        )),
    }
}

fn directory_tree_is_missing(base: &Path, relative: &str) -> Result<bool, String> {
    require_real_directory(base, "Repository authority")?;
    let mut current = base.to_path_buf();
    for component in relative.split('/') {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing symbolic-link repository directory: {}",
                    current.display()
                ))
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!(
                    "Repository directory path has the wrong file kind: {}",
                    current.display()
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(error) => {
                return Err(format!(
                    "Failed to inspect repository directory {}: {error}",
                    current.display()
                ))
            }
        }
    }
    Ok(false)
}

fn create_real_directory_tree(base: &Path, relative: &str) -> Result<(), String> {
    require_real_directory(base, "Directory-tree root")?;
    let mut current = base.to_path_buf();
    for component in relative.split('/') {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "Refusing symbolic-link repository directory: {}",
                    current.display()
                ))
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!(
                    "Repository directory path has the wrong file kind: {}",
                    current.display()
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|create_error| {
                    format!(
                        "Failed to create repository directory {}: {create_error}",
                        current.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "Failed to inspect repository directory {}: {error}",
                    current.display()
                ))
            }
        }
    }
    Ok(())
}

fn regular_file_state(path: &Path, label: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "Refusing symbolic-link {label}: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "{label} must be a regular file: {}",
            path.display()
        )),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Failed to inspect {label} {}: {error}",
            path.display()
        )),
    }
}

fn validate_no_symlinks_below(path: &Path) -> Result<(), String> {
    require_real_directory(path, "Binary DB authority")?;
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| {
            format!(
                "Failed to inspect Binary DB authority {}: {error}",
                directory.display()
            )
        })? {
            let entry = entry.map_err(|error| error.to_string())?;
            let entry_path = entry.path();
            let metadata = fs::symlink_metadata(&entry_path).map_err(|error| {
                format!(
                    "Failed to inspect Binary DB entry {}: {error}",
                    entry_path.display()
                )
            })?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "Refusing symbolic-link Binary DB entry: {}",
                    entry_path.display()
                ));
            }
            if metadata.is_dir() {
                pending.push(entry_path);
            }
        }
    }
    Ok(())
}

fn write_text_atomically(path: &Path, content: &str, default_mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Atomic write path has no parent: {}", path.display()))?;
    require_real_directory(parent, "Atomic write parent")?;
    let existing_permissions = if regular_file_state(path, "Atomic write target")? {
        Some(
            fs::symlink_metadata(path)
                .map_err(|error| error.to_string())?
                .permissions(),
        )
    } else {
        None
    };
    let mut staged = NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "Failed to stage atomic write for {}: {error}",
            path.display()
        )
    })?;
    #[cfg(unix)]
    staged
        .as_file()
        .set_permissions(
            existing_permissions.unwrap_or_else(|| fs::Permissions::from_mode(default_mode)),
        )
        .map_err(|error| format!("Failed to set staged permissions: {error}"))?;
    #[cfg(not(unix))]
    let _ = (existing_permissions, default_mode);
    staged
        .as_file_mut()
        .write_all(content.as_bytes())
        .and_then(|_| staged.as_file_mut().flush())
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|error| format!("Failed to stage {}: {error}", path.display()))?;
    staged.persist(path).map_err(|error| {
        format!(
            "Failed to atomically publish {}: {}",
            path.display(),
            error.error
        )
    })?;
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Failed to sync directory {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn display_value(value: Option<&JsonValue>) -> String {
    value
        .and_then(|value| match value {
            JsonValue::String(text) => Some(text.clone()),
            JsonValue::Number(number) => Some(number.to_string()),
            JsonValue::Bool(flag) => Some(flag.to_string()),
            JsonValue::Null => Some("null".to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "null".to_string())
}

#[cfg(test)]
mod tests;
