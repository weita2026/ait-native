use super::*;
use crate::json_support::encode_value;
use ait_core::json_support::{JsonCodec, JsonEncodeOptions};
use ait_core::workflow_tier::{
    evaluate_workflow_tier, WorkflowTierInput, WorkflowTierLimits, DEFAULT_QUICK_MAX_BYTES,
    DEFAULT_QUICK_MAX_FILES,
};
use std::io::Read;

const QUICK_PROVENANCE_CONTRACT: &str = "ait.quick-snapshot-provenance/v1";
const QUICK_EVIDENCE_MAX_CHARS: usize = 512;

pub fn workflow_tier_payload(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let status = workflow_workspace_status(repo, None, None)?;
    let changed_paths = json_string_list(status.get("changed_paths"));
    let dirty_diff = workspace_dirty_diff(repo, &changed_paths, 0)?;
    let changed_bytes = dirty_diff
        .get("summary")
        .and_then(JsonValue::as_object)
        .and_then(|summary| summary.get("changed_bytes"))
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let missing_path_count = status
        .get("missing_paths")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let (binary_paths, special_paths) =
        workspace_quick_file_facts(&repo.workspace_root(), &changed_paths);
    let (quick_limits, extra_forbidden_prefixes, limits_source) = quick_config(repo);
    let current_line = string_field(&status, "current_line").unwrap_or_default();
    let default_line = repo.default_line_name();
    let workflow_mode = repo.effective_workflow_mode();
    let policy_profile = repo
        .config
        .get("policy_profile")
        .and_then(JsonValue::as_str)
        .unwrap_or("prototype")
        .trim()
        .to_string();
    let known_base = status
        .get("baseline_snapshot_id")
        .and_then(JsonValue::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    let worktree_metadata = current_worktree_metadata(repo)?;
    let bound_task_id = worktree_metadata
        .as_ref()
        .and_then(|metadata| metadata.bound_task_id.clone());
    let bound_change_id = worktree_metadata
        .as_ref()
        .and_then(|metadata| metadata.bound_change_id.clone());

    let evaluation = evaluate_workflow_tier(WorkflowTierInput {
        changed_paths: changed_paths.clone(),
        changed_bytes,
        missing_path_count,
        binary_paths: binary_paths.clone(),
        special_paths: special_paths.clone(),
        is_worktree: repo.is_worktree(),
        known_base,
        current_line: current_line.clone(),
        default_line: default_line.clone(),
        workflow_mode: workflow_mode.clone(),
        policy_profile: policy_profile.clone(),
        quick_limits,
        extra_forbidden_prefixes: extra_forbidden_prefixes.clone(),
    });
    let encoded = JsonCodec::encode_serializable(&evaluation, JsonEncodeOptions::compact())
        .map_err(String::from)?;
    let mut payload =
        JsonCodec::parse_value(&encoded, "workflow tier evaluation").map_err(String::from)?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "workflow tier evaluation must encode to an object".to_string())?;
    if let Some(command) = object
        .get("escalation_command")
        .and_then(JsonValue::as_str)
        .map(|value| value.replace("<line>", &current_line))
    {
        object.insert("escalation_command".to_string(), JsonValue::String(command));
    }
    object.insert(
        "facts".to_string(),
        json!({
            "workspace_root": repo.workspace_root().to_string_lossy().to_string(),
            "is_worktree": repo.is_worktree(),
            "bound_task_id": bound_task_id,
            "bound_change_id": bound_change_id,
            "known_base": known_base,
            "current_line": current_line,
            "default_line": default_line,
            "workflow_mode": workflow_mode,
            "policy_profile": policy_profile,
            "missing_path_count": missing_path_count,
            "binary_paths": binary_paths,
            "special_paths": special_paths,
            "extra_forbidden_prefixes": extra_forbidden_prefixes,
            "limits_source": limits_source,
            "target_scope": "local_only_for_quick",
        }),
    );
    Ok(payload)
}

pub fn snapshot_create_quick(
    repo: &RepoRuntime,
    message: Option<&str>,
    intent: Option<&str>,
    validation: Option<&str>,
) -> Result<JsonValue, String> {
    let message = quick_evidence_text(message, "--message")?;
    let intent = quick_evidence_text(intent, "--intent")?;
    let validation = quick_evidence_text(validation, "--validation")?;
    let evaluation = workflow_tier_payload(repo)?;
    if evaluation.get("quick_allowed").and_then(JsonValue::as_bool) != Some(true) {
        let tier = evaluation
            .get("recommended_tier")
            .and_then(JsonValue::as_str)
            .unwrap_or("normal_task");
        let reasons = evaluation
            .get("reasons")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(|reason| reason.get("detail").and_then(JsonValue::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        let escalation = evaluation
            .get("escalation_command")
            .and_then(JsonValue::as_str)
            .unwrap_or("ait workflow tier --json");
        return Err(format!(
            "Quick Snapshot is not allowed; recommended tier is `{tier}`. {reasons} The workspace and current local line were left unchanged. Escalate with: {escalation}"
        ));
    }

    let provenance = json!({
        "contract": QUICK_PROVENANCE_CONTRACT,
        "profile": "quick",
        "intent": intent,
        "validation": validation,
    });
    let encoded_provenance = encode_value(&provenance, "Failed to encode quick provenance")?;
    let persisted_message = format!("{message}\n\nAIT-Quick-Provenance: {encoded_provenance}");
    let mut snapshot = snapshot_create(repo, Some(&persisted_message))?;
    let object = snapshot
        .as_object_mut()
        .ok_or_else(|| "quick Snapshot payload must decode to an object".to_string())?;
    object.insert(
        "profile".to_string(),
        JsonValue::String("quick".to_string()),
    );
    object.insert("intent".to_string(), JsonValue::String(intent));
    object.insert("validation".to_string(), JsonValue::String(validation));
    object.insert("quick_provenance".to_string(), provenance);
    object.insert("workflow_tier".to_string(), evaluation);
    Ok(snapshot)
}

fn quick_config(repo: &RepoRuntime) -> (WorkflowTierLimits, Vec<String>, &'static str) {
    let Some(config) = repo
        .config
        .get("workflow_quick")
        .and_then(JsonValue::as_object)
    else {
        return (WorkflowTierLimits::default(), Vec::new(), "defaults");
    };
    let max_files = config
        .get("max_files")
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_QUICK_MAX_FILES);
    let max_bytes = config
        .get("max_bytes")
        .and_then(JsonValue::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_QUICK_MAX_BYTES);
    let extra_forbidden_prefixes = config
        .get("forbidden_prefixes")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect();
    (
        WorkflowTierLimits {
            max_files,
            max_bytes,
        },
        extra_forbidden_prefixes,
        "repository_config",
    )
}

fn workspace_quick_file_facts(
    workspace_root: &Path,
    changed_paths: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut binary_paths = Vec::new();
    let mut special_paths = Vec::new();
    for relative_path in changed_paths {
        let path = workspace_root.join(relative_path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.file_type().is_file() {
            special_paths.push(relative_path.clone());
            continue;
        }
        if workspace_file_looks_binary(&path) {
            binary_paths.push(relative_path.clone());
        }
    }
    (binary_paths, special_paths)
}

fn workspace_file_looks_binary(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return true;
    };
    let mut prefix = [0_u8; 8192];
    match file.read(&mut prefix) {
        Ok(length) => prefix[..length].contains(&0),
        Err(_) => true,
    }
}

fn quick_evidence_text(value: Option<&str>, option: &str) -> Result<String, String> {
    let normalized = value
        .map(|raw| raw.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("{option} is required with `--profile quick`."))?;
    if normalized.chars().count() > QUICK_EVIDENCE_MAX_CHARS {
        return Err(format!(
            "{option} exceeds the {QUICK_EVIDENCE_MAX_CHARS}-character quick provenance limit."
        ));
    }
    Ok(normalized)
}
