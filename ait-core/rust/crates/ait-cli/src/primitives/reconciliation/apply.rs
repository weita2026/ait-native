use super::*;
use crate::json_support::{
    encode_value, encode_value_pretty_with_newline_error_string, parse_value_option,
};
use std::io::{Seek, SeekFrom, Write};

const RECONCILIATION_RECEIPT_CONTRACT: &str = "workflow-reconciliation-receipt/v1";
const RECONCILIATION_SUMMARY_CONTRACT: &str = "workflow-reconciliation-summary/v1";
const AUTOMATIC_RECONCILIATION_CONTRACT: &str = "workflow-automatic-reconciliation/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutomaticReconciliationScope {
    Local,
    Remote(Option<String>),
}

impl AutomaticReconciliationScope {
    fn selection(&self) -> (Option<&str>, bool) {
        match self {
            Self::Local => (None, false),
            Self::Remote(remote_name) => (remote_name.as_deref(), true),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote(_) => "remote",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutomaticReconciliationTrigger {
    PreTaskStart,
    TaskTerminal,
    ChangeTerminal,
    LandTerminal,
    LandRecovery,
    ScheduledRemote,
}

impl AutomaticReconciliationTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::PreTaskStart => "pre_task_start",
            Self::TaskTerminal => "task_terminal",
            Self::ChangeTerminal => "change_terminal",
            Self::LandTerminal => "land_terminal",
            Self::LandRecovery => "land_recovery",
            Self::ScheduledRemote => "scheduled_remote",
        }
    }

    fn default_action_limit(self) -> usize {
        match self {
            Self::PreTaskStart => 4,
            Self::TaskTerminal | Self::ChangeTerminal | Self::LandTerminal => 8,
            Self::LandRecovery => 12,
            Self::ScheduledRemote => 100,
        }
    }

    fn cooperative_time_budget(self) -> Duration {
        match self {
            Self::PreTaskStart => Duration::from_millis(1_500),
            Self::TaskTerminal | Self::ChangeTerminal | Self::LandTerminal => {
                Duration::from_millis(3_000)
            }
            Self::LandRecovery => Duration::from_millis(5_000),
            Self::ScheduledRemote => Duration::from_millis(30_000),
        }
    }
}

struct ReconciliationLease {
    file: File,
    path: PathBuf,
}

impl ReconciliationLease {
    fn acquire(repo: &RepoRuntime) -> Result<Self, String> {
        let path = reconciliation_state_root(repo).join("reconcile.lock");
        let parent = path
            .parent()
            .ok_or_else(|| "Reconciliation lease path has no parent.".to_string())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut file = OpenOptions::new()
            .create(true)
            // Preserve lease metadata until exclusive ownership is confirmed.
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| error.to_string())?;
        file.try_lock_exclusive().map_err(|error| {
            format!(
                "Another reconciler holds the repository lease at {}: {error}",
                path.display()
            )
        })?;
        let metadata = json!({
            "contract": "workflow-reconciliation-lease/v1",
            "pid": std::process::id(),
            "repo_name": repo.repo_name(),
            "started_at": system_event_timestamp(),
        });
        file.set_len(0).map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        file.write_all(encode_value_pretty_with_newline_error_string(&metadata)?.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        Ok(Self { file, path })
    }

    fn evidence(&self) -> JsonValue {
        json!({
            "path": self.path,
            "state": "held",
            "pid": std::process::id(),
        })
    }
}

impl Drop for ReconciliationLease {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
        let _ = self.file.seek(SeekFrom::Start(0));
        let _ = self.file.flush();
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn reconciliation_state_root(repo: &RepoRuntime) -> PathBuf {
    repo.authoritative_repo_root()
        .join(".ait")
        .join("reconciliation")
        .join("v1")
}

fn reconciliation_receipts_root(repo: &RepoRuntime) -> PathBuf {
    reconciliation_state_root(repo).join("receipts")
}

fn receipt_path(repo: &RepoRuntime, receipt_id: &str) -> PathBuf {
    reconciliation_receipts_root(repo).join(format!("{receipt_id}.json"))
}

fn reconciliation_summary_path(repo: &RepoRuntime) -> PathBuf {
    reconciliation_state_root(repo).join("summary.json")
}

fn receipt_id(finding: &JsonValue) -> Result<String, String> {
    let finding_id = required_string_field(finding, "finding_id")?;
    let action_code = finding
        .get("recommended_action")
        .and_then(|value| string_field(value, "code"))
        .ok_or_else(|| format!("Finding {finding_id} is missing a recommended action code."))?;
    let evidence = finding.get("evidence").cloned().unwrap_or(JsonValue::Null);
    let encoded = encode_value(&evidence, "Failed to encode reconciliation preconditions")?;
    let mut hasher = Sha256::new();
    hasher.update(finding_id.as_bytes());
    hasher.update([0]);
    hasher.update(action_code.as_bytes());
    hasher.update([0]);
    hasher.update(encoded.as_bytes());
    let digest = hasher.finalize();
    Ok(format!(
        "RCR-{}",
        digest[..10]
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>()
    ))
}

fn write_reconciliation_json_atomic(path: &Path, payload: &JsonValue) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Receipt path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let target_name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("state");
    let temp = parent.join(format!(
        ".reconciliation-{target_name}-{}-{}.tmp",
        std::process::id(),
        system_event_timestamp().replace([':', '.', '+'], "-")
    ));
    let encoded = encode_value_pretty_with_newline_error_string(payload)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|error| error.to_string())?;
    file.write_all(encoded.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        error.to_string()
    })?;
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn write_receipt_atomic(path: &Path, payload: &JsonValue) -> Result<(), String> {
    write_reconciliation_json_atomic(path, payload)
}

fn reconciliation_summary_unavailable(
    repo: &RepoRuntime,
    state: &str,
    error: Option<String>,
) -> JsonValue {
    json!({
        "contract": RECONCILIATION_SUMMARY_CONTRACT,
        "state": state,
        "captured_at": JsonValue::Null,
        "total_finding_count": JsonValue::Null,
        "safe_finding_count": JsonValue::Null,
        "manual_resolution_count": JsonValue::Null,
        "protected_count": JsonValue::Null,
        "oldest_finding_first_observed_at": JsonValue::Null,
        "oldest_finding_age_seconds": JsonValue::Null,
        "next_command": reconcile_command(repo.default_remote_name().as_deref(), None),
        "error": error,
    })
}

fn load_reconciliation_summary_document(repo: &RepoRuntime) -> Result<Option<JsonValue>, String> {
    let path = reconciliation_summary_path(repo);
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let payload = parse_value_option(&content)
        .ok_or_else(|| format!("Invalid reconciliation summary JSON: {}", path.display()))?;
    if payload.get("contract").and_then(JsonValue::as_str) != Some(RECONCILIATION_SUMMARY_CONTRACT)
    {
        return Err(format!(
            "Unsupported reconciliation summary contract at {}.",
            path.display()
        ));
    }
    Ok(Some(payload))
}

fn compact_reconciliation_summary(payload: &JsonValue) -> JsonValue {
    let mut compact = JsonMap::new();
    for key in [
        "contract",
        "state",
        "captured_at",
        "total_finding_count",
        "safe_finding_count",
        "manual_resolution_count",
        "protected_count",
        "oldest_finding_first_observed_at",
        "oldest_finding_age_seconds",
        "next_command",
        "last_trigger",
    ] {
        if let Some(value) = payload.get(key) {
            compact.insert(key.to_string(), value.clone());
        }
    }
    JsonValue::Object(compact)
}

pub fn workflow_reconciliation_cached_summary(repo: &RepoRuntime) -> JsonValue {
    match load_reconciliation_summary_document(repo) {
        Ok(Some(payload)) => compact_reconciliation_summary(&payload),
        Ok(None) => reconciliation_summary_unavailable(repo, "never_observed", None),
        Err(error) => reconciliation_summary_unavailable(repo, "invalid", Some(error)),
    }
}

fn reconciliation_finding_observations(
    previous: Option<&JsonValue>,
    findings: &[JsonValue],
    observed_at: &str,
) -> JsonMap<String, JsonValue> {
    let previous = previous
        .and_then(|payload| payload.get("finding_first_observed_at"))
        .and_then(JsonValue::as_object);
    findings
        .iter()
        .filter_map(|finding| {
            let finding_id = string_field(finding, "finding_id")?;
            let first_observed_at = previous
                .and_then(|observations| observations.get(&finding_id))
                .and_then(JsonValue::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| observed_at.to_string());
            Some((finding_id, JsonValue::String(first_observed_at)))
        })
        .collect()
}

fn oldest_observation(observations: &JsonMap<String, JsonValue>) -> (JsonValue, JsonValue) {
    let oldest = observations
        .values()
        .filter_map(JsonValue::as_str)
        .filter_map(|value| {
            DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|parsed| (parsed, value.to_string()))
        })
        .min_by_key(|(parsed, _)| *parsed);
    let Some((parsed, text)) = oldest else {
        return (JsonValue::Null, JsonValue::Null);
    };
    let age_seconds = Utc::now()
        .signed_duration_since(parsed.with_timezone(&Utc))
        .num_seconds()
        .max(0);
    (JsonValue::String(text), JsonValue::from(age_seconds))
}

fn update_reconciliation_summary_cache(
    repo: &RepoRuntime,
    inventory: &JsonValue,
    last_trigger: Option<&str>,
) -> Result<JsonValue, String> {
    // A cached projection is never authoritative. A corrupt or unsupported cache must remain
    // visible to read-only status, but an explicit reconciliation pass can safely replace it
    // from the freshly computed inventory instead of becoming permanently wedged.
    let previous = load_reconciliation_summary_document(repo)
        .ok()
        .flatten()
        .unwrap_or(JsonValue::Null);
    let findings = inventory_findings(inventory);
    let captured_at = string_field(inventory, "captured_at").unwrap_or_else(system_event_timestamp);
    let observations = reconciliation_finding_observations(
        (!previous.is_null()).then_some(&previous),
        &findings,
        &captured_at,
    );
    let (oldest_first_observed_at, oldest_age_seconds) = oldest_observation(&observations);
    let disposition_counts = inventory
        .get("summary")
        .and_then(|summary| summary.get("disposition_counts"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let safe_finding_count = disposition_counts
        .get("safe_metadata_repair")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0)
        + disposition_counts
            .get("safe_auto_cleanup")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
    let payload = json!({
        "contract": RECONCILIATION_SUMMARY_CONTRACT,
        "state": "available",
        "captured_at": captured_at,
        "repo_name": repo.repo_name(),
        "remote_name": inventory.get("remote_name").cloned().unwrap_or(JsonValue::Null),
        "task_filter": inventory.get("task_filter").cloned().unwrap_or(JsonValue::Null),
        "total_finding_count": inventory.get("summary").and_then(|summary| summary.get("total_finding_count")).cloned().unwrap_or(JsonValue::from(findings.len())),
        "safe_finding_count": safe_finding_count,
        "manual_resolution_count": disposition_counts.get("manual_resolution").cloned().unwrap_or(JsonValue::from(0)),
        "protected_count": disposition_counts.get("protected").cloned().unwrap_or(JsonValue::from(0)),
        "oldest_finding_first_observed_at": oldest_first_observed_at,
        "oldest_finding_age_seconds": oldest_age_seconds,
        "next_command": reconcile_command(
            inventory.get("remote_name").and_then(JsonValue::as_str),
            inventory.get("task_filter").and_then(JsonValue::as_str),
        ),
        "last_trigger": last_trigger,
        "finding_first_observed_at": observations,
    });
    write_reconciliation_json_atomic(&reconciliation_summary_path(repo), &payload)?;
    Ok(compact_reconciliation_summary(&payload))
}

fn load_receipt(path: &Path) -> Result<JsonValue, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let receipt = parse_value_option(&content)
        .ok_or_else(|| format!("Invalid reconciliation receipt JSON: {}", path.display()))?;
    if receipt.get("contract").and_then(JsonValue::as_str) != Some(RECONCILIATION_RECEIPT_CONTRACT)
    {
        return Err(format!(
            "Unsupported reconciliation receipt contract at {}.",
            path.display()
        ));
    }
    Ok(receipt)
}

fn load_receipts(repo: &RepoRuntime) -> Result<BTreeMap<String, JsonValue>, String> {
    let root = reconciliation_receipts_root(repo);
    if !root.is_dir() {
        return Ok(BTreeMap::new());
    }
    let mut paths = fs::read_dir(&root)
        .map_err(|error| error.to_string())?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut receipts = BTreeMap::new();
    for path in paths {
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let receipt = load_receipt(&path)?;
        let id = required_string_field(&receipt, "receipt_id")?;
        receipts.insert(id, receipt);
    }
    Ok(receipts)
}

fn finding_identity(finding: &JsonValue, key: &str) -> Result<String, String> {
    finding
        .get("identities")
        .and_then(|identities| string_field(identities, key))
        .ok_or_else(|| {
            format!(
                "Finding {} is missing identity `{key}`.",
                string_field(finding, "finding_id").unwrap_or_else(|| "unknown".to_string())
            )
        })
}

fn apply_safe_finding(
    repo: &RepoRuntime,
    finding: &JsonValue,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let code = required_string_field(finding, "code")?;
    match code.as_str() {
        "task.active_after_all_changes_landed" => {
            let task_id = finding_identity(finding, "task_id")?;
            let authoritative_scope = finding
                .get("evidence")
                .and_then(|evidence| string_field(evidence, "authoritative_scope"))
                .unwrap_or_else(|| {
                    if remote_name.is_some() {
                        "remote".to_string()
                    } else {
                        "local".to_string()
                    }
                });
            let result = task_close(
                repo,
                &task_id,
                "completed",
                authoritative_scope == "local",
                (authoritative_scope == "remote")
                    .then_some(remote_name)
                    .flatten(),
                None,
            )?;
            Ok(json!({"mutated": true, "operation": "task_close", "result": result}))
        }
        "task.local_status_stale" => {
            let task_id = finding_identity(finding, "task_id")?;
            let status = finding
                .get("evidence")
                .and_then(|evidence| string_field(evidence, "remote_status"))
                .ok_or_else(|| format!("Finding for {task_id} is missing remote status."))?;
            let result = task_close(repo, &task_id, &status, true, None, None)?;
            Ok(json!({"mutated": true, "operation": "refresh_local_task_status", "result": result}))
        }
        "worktree.materialization_missing" | "worktree.overlay_detached" => {
            let worktree_name = finding_identity(finding, "worktree_name")?;
            let worktree_name = worktree::normalize_worktree_name(&worktree_name)?;
            let path = foundation::worktree_registry_path(repo, &worktree_name);
            if path.exists() {
                fs::remove_file(&path).map_err(|error| error.to_string())?;
                Ok(json!({
                    "mutated": true,
                    "operation": "prune_worktree_registration",
                    "worktree_name": worktree_name,
                    "metadata_path": path,
                }))
            } else {
                Ok(json!({
                    "mutated": false,
                    "operation": "prune_worktree_registration",
                    "worktree_name": worktree_name,
                    "status": "already_absent",
                }))
            }
        }
        "worktree.terminal_owner_clean" => {
            let worktree_name = finding_identity(finding, "worktree_name")?;
            let result = worktree_remove(repo, &[worktree_name], false, true, false, false)?;
            Ok(json!({"mutated": true, "operation": "remove_terminal_worktree", "result": result}))
        }
        "line.terminal_owner_orphaned" => {
            let line_name = finding_identity(finding, "line_name")?;
            let result = line_archive(repo, &line_name, None)?;
            Ok(json!({"mutated": true, "operation": "archive_feature_line", "result": result}))
        }
        "land.target_sync_interrupted" => {
            let line_name = finding_identity(finding, "line_name")?;
            let remote = normalized_text(remote_name).ok_or_else(|| {
                "Target-line reconciliation requires a selected remote.".to_string()
            })?;
            let result = pull(repo, Some(&remote), Some(&line_name), false, false, false)?;
            Ok(json!({"mutated": true, "operation": "resume_target_line_sync", "result": result}))
        }
        _ => Err(format!(
            "Finding `{code}` is marked safe but has no reconciliation action implementation."
        )),
    }
}

fn receipt_base(receipt_id: &str, finding: &JsonValue, attempt: u64, state: &str) -> JsonValue {
    json!({
        "contract": RECONCILIATION_RECEIPT_CONTRACT,
        "receipt_id": receipt_id,
        "retry_identity": {
            "finding_id": string_field(finding, "finding_id"),
            "finding_code": string_field(finding, "code"),
            "action_code": finding.get("recommended_action").and_then(|value| string_field(value, "code")),
        },
        "identities": finding.get("identities").cloned().unwrap_or_else(|| json!({})),
        "preconditions": finding.get("evidence").cloned().unwrap_or(JsonValue::Null),
        "disposition": string_field(finding, "disposition"),
        "attempt": attempt,
        "state": state,
        "started_at": system_event_timestamp(),
        "completed_at": JsonValue::Null,
        "result": JsonValue::Null,
        "remaining_findings": JsonValue::Null,
    })
}

fn set_receipt_completion(
    receipt: &mut JsonValue,
    state: &str,
    result: JsonValue,
    remaining_findings: Option<usize>,
) {
    let Some(object) = receipt.as_object_mut() else {
        return;
    };
    object.insert("state".to_string(), JsonValue::String(state.to_string()));
    object.insert(
        "completed_at".to_string(),
        JsonValue::String(system_event_timestamp()),
    );
    object.insert("result".to_string(), result);
    if let Some(count) = remaining_findings {
        object.insert("remaining_findings".to_string(), JsonValue::from(count));
    }
}

fn inventory_findings(payload: &JsonValue) -> Vec<JsonValue> {
    payload
        .get("findings")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
}

fn finding_is_safe(finding: &JsonValue) -> bool {
    matches!(
        string_field(finding, "disposition").as_deref(),
        Some("safe_metadata_repair" | "safe_auto_cleanup")
    )
}

pub fn workflow_reconcile_apply(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    task_filter: Option<&str>,
    safe_only: bool,
    limit: Option<usize>,
) -> Result<JsonValue, String> {
    workflow_reconcile_apply_with_budget(
        repo,
        remote_name,
        task_filter,
        safe_only,
        limit,
        true,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn workflow_reconcile_apply_with_budget(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    task_filter: Option<&str>,
    safe_only: bool,
    limit: Option<usize>,
    use_default_remote: bool,
    cooperative_time_budget: Option<Duration>,
    trigger: Option<AutomaticReconciliationTrigger>,
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let limit = limit.unwrap_or(DEFAULT_RECONCILIATION_LIMIT);
    if limit == 0 || limit > MAX_RECONCILIATION_LIMIT {
        return Err(format!(
            "--limit must be between 1 and {MAX_RECONCILIATION_LIMIT}."
        ));
    }
    let lease = ReconciliationLease::acquire(repo)?;
    let initial = workflow_reconcile_inventory_with_remote_policy(
        repo,
        remote_name,
        task_filter,
        false,
        Some(MAX_RECONCILIATION_LIMIT),
        use_default_remote,
    )?;
    let selected_remote = initial
        .get("remote_name")
        .and_then(JsonValue::as_str)
        .map(ToString::to_string);
    let mut current_findings = inventory_findings(&initial);
    let mut receipts = load_receipts(repo)?;
    let mut receipt_updates = Vec::<JsonValue>::new();
    let mut actions = Vec::<JsonValue>::new();
    let mut receipts_created = 0_usize;
    let mut replayed_receipt_count = 0_usize;
    let mut recovered_receipt_count = 0_usize;
    let mut state_mutated = false;
    let mut cooperative_budget_exhausted = false;

    let current_ids = current_findings
        .iter()
        .filter_map(|finding| string_field(finding, "finding_id"))
        .collect::<BTreeSet<_>>();
    for (id, receipt) in receipts.iter_mut() {
        if string_field(receipt, "state").as_deref() != Some("started") {
            continue;
        }
        let finding_id = receipt
            .get("retry_identity")
            .and_then(|identity| string_field(identity, "finding_id"));
        if finding_id.is_some_and(|finding_id| !current_ids.contains(&finding_id)) {
            set_receipt_completion(
                receipt,
                "completed_recovered",
                json!({
                    "mutated": false,
                    "operation": "authoritative_recovery",
                    "reason": "finding_absent_after_interrupted_attempt",
                }),
                Some(current_findings.len()),
            );
            write_receipt_atomic(&receipt_path(repo, id), receipt)?;
            receipt_updates.push(receipt.clone());
            recovered_receipt_count += 1;
        }
    }

    let selected = current_findings
        .iter()
        .filter(|finding| !safe_only || finding_is_safe(finding))
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    for selected_finding in selected {
        if cooperative_time_budget.is_some_and(|budget| started.elapsed() >= budget) {
            cooperative_budget_exhausted = true;
            break;
        }
        let finding_id = required_string_field(&selected_finding, "finding_id")?;
        let current = current_findings.iter().find(|finding| {
            string_field(finding, "finding_id").as_deref() == Some(finding_id.as_str())
        });
        let effective_finding = current.cloned().unwrap_or_else(|| selected_finding.clone());
        let id = receipt_id(&effective_finding)?;
        if current.is_none() {
            let mut receipt = receipt_base(&id, &effective_finding, 1, "started");
            set_receipt_completion(
                &mut receipt,
                "completed_noop",
                json!({"mutated": false, "reason": "finding_already_converged"}),
                Some(current_findings.len()),
            );
            write_receipt_atomic(&receipt_path(repo, &id), &receipt)?;
            receipts.insert(id.clone(), receipt.clone());
            receipt_updates.push(receipt);
            receipts_created += 1;
            actions.push(json!({
                "finding_id": finding_id,
                "receipt_id": id,
                "state": "completed_noop",
                "mutated": false,
            }));
            continue;
        }
        if let Some(existing) = receipts.get(&id) {
            if matches!(
                string_field(existing, "state").as_deref(),
                Some("completed" | "completed_noop" | "completed_recovered")
            ) {
                replayed_receipt_count += 1;
                actions.push(json!({
                    "finding_id": finding_id,
                    "receipt_id": id,
                    "state": "replayed_success_noop",
                    "mutated": false,
                }));
                continue;
            }
        }
        let previous_attempt = receipts
            .get(&id)
            .and_then(|receipt| receipt.get("attempt"))
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let safe = finding_is_safe(&effective_finding);
        let mut receipt = receipt_base(&id, &effective_finding, previous_attempt + 1, "started");
        write_receipt_atomic(&receipt_path(repo, &id), &receipt)?;
        if !receipts.contains_key(&id) {
            receipts_created += 1;
        }
        if !safe {
            set_receipt_completion(
                &mut receipt,
                "refused",
                json!({
                    "mutated": false,
                    "reason": "finding_is_not_safe_for_automatic_repair",
                    "disposition": string_field(&effective_finding, "disposition"),
                }),
                Some(current_findings.len()),
            );
            write_receipt_atomic(&receipt_path(repo, &id), &receipt)?;
            receipts.insert(id.clone(), receipt.clone());
            receipt_updates.push(receipt);
            actions.push(json!({
                "finding_id": finding_id,
                "receipt_id": id,
                "state": "refused",
                "mutated": false,
            }));
            continue;
        }

        match apply_safe_finding(repo, &effective_finding, selected_remote.as_deref()) {
            Ok(result) => {
                let mutated = result
                    .get("mutated")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false);
                state_mutated |= mutated;
                set_receipt_completion(&mut receipt, "completed", result.clone(), None);
                write_receipt_atomic(&receipt_path(repo, &id), &receipt)?;
                receipts.insert(id.clone(), receipt.clone());
                receipt_updates.push(receipt);
                actions.push(json!({
                    "finding_id": finding_id,
                    "receipt_id": id,
                    "state": "completed",
                    "mutated": mutated,
                    "result": result,
                }));
                let refreshed = workflow_reconcile_inventory_with_remote_policy(
                    repo,
                    selected_remote.as_deref(),
                    task_filter,
                    false,
                    Some(MAX_RECONCILIATION_LIMIT),
                    false,
                )?;
                current_findings = inventory_findings(&refreshed);
            }
            Err(error) => {
                set_receipt_completion(
                    &mut receipt,
                    "failed",
                    json!({"mutated": false, "error": error}),
                    None,
                );
                write_receipt_atomic(&receipt_path(repo, &id), &receipt)?;
                receipts.insert(id.clone(), receipt.clone());
                receipt_updates.push(receipt);
                actions.push(json!({
                    "finding_id": finding_id,
                    "receipt_id": id,
                    "state": "failed",
                    "mutated": false,
                    "error": error,
                }));
            }
        }
    }

    let final_inventory = workflow_reconcile_inventory_with_remote_policy(
        repo,
        selected_remote.as_deref(),
        task_filter,
        false,
        Some(MAX_RECONCILIATION_LIMIT),
        false,
    )?;
    let remaining_findings = inventory_findings(&final_inventory);
    let remaining_safe_count = remaining_findings
        .iter()
        .filter(|finding| finding_is_safe(finding))
        .count();
    for receipt in &mut receipt_updates {
        if matches!(
            string_field(receipt, "state").as_deref(),
            Some("completed" | "failed")
        ) {
            if let Some(object) = receipt.as_object_mut() {
                object.insert(
                    "remaining_findings".to_string(),
                    JsonValue::from(remaining_findings.len()),
                );
            }
            let id = required_string_field(receipt, "receipt_id")?;
            write_receipt_atomic(&receipt_path(repo, &id), receipt)?;
        }
    }
    let failed_count = actions
        .iter()
        .filter(|action| string_field(action, "state").as_deref() == Some("failed"))
        .count();
    let refused_count = actions
        .iter()
        .filter(|action| string_field(action, "state").as_deref() == Some("refused"))
        .count();
    let status = if failed_count > 0 {
        "partial_failure"
    } else if remaining_safe_count > 0 {
        "continuation_required"
    } else {
        "completed"
    };
    let summary_cache = match update_reconciliation_summary_cache(
        repo,
        &final_inventory,
        trigger.map(AutomaticReconciliationTrigger::label),
    ) {
        Ok(summary) => json!({
            "status": "updated",
            "path": reconciliation_summary_path(repo),
            "summary": summary,
        }),
        Err(error) => json!({
            "status": "failed",
            "path": reconciliation_summary_path(repo),
            "error": error,
        }),
    };
    let mut output = final_inventory
        .as_object()
        .cloned()
        .ok_or_else(|| "Final reconciliation inventory must be an object.".to_string())?;
    output.insert(
        "operation".to_string(),
        JsonValue::String("apply".to_string()),
    );
    output.insert("mode".to_string(), JsonValue::String("apply".to_string()));
    output.insert("status".to_string(), JsonValue::String(status.to_string()));
    output.insert("safe_only".to_string(), JsonValue::Bool(safe_only));
    output.insert("limit".to_string(), JsonValue::from(limit));
    output.insert("mutated".to_string(), JsonValue::Bool(state_mutated));
    output.insert(
        "receipt_store_mutated".to_string(),
        JsonValue::Bool(!receipt_updates.is_empty()),
    );
    output.insert("summary_cache".to_string(), summary_cache);
    output.insert(
        "receipts_created".to_string(),
        JsonValue::from(receipts_created),
    );
    output.insert(
        "receipt_updates".to_string(),
        JsonValue::Array(receipt_updates),
    );
    output.insert("actions".to_string(), JsonValue::Array(actions.clone()));
    output.insert(
        "apply_summary".to_string(),
        json!({
            "attempted_count": actions.len(),
            "failed_count": failed_count,
            "refused_count": refused_count,
            "remaining_safe_count": remaining_safe_count,
            "replayed_receipt_count": replayed_receipt_count,
            "recovered_receipt_count": recovered_receipt_count,
        }),
    );
    output.insert(
        "initial_inventory_digest".to_string(),
        initial
            .get("inventory_digest")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    output.insert("lease".to_string(), lease.evidence());
    output.insert(
        "cooperative_budget".to_string(),
        json!({
            "time_budget_ms": cooperative_time_budget.map(|budget| budget.as_millis() as u64),
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "exhausted": cooperative_budget_exhausted,
            "action_limit": limit,
        }),
    );
    output.insert(
        "next_command".to_string(),
        JsonValue::String(if remaining_safe_count > 0 {
            reconcile_command(selected_remote.as_deref(), task_filter)
        } else {
            let mut command = "ait workflow reconcile".to_string();
            if let Some(remote) = selected_remote.as_ref() {
                command.push_str(" --remote ");
                command.push_str(remote);
            }
            if let Some(task_id) = normalized_text(task_filter) {
                command.push_str(" --task ");
                command.push_str(&task_id);
            }
            command.push_str(" --dry-run");
            command
        }),
    );
    Ok(JsonValue::Object(output))
}

pub fn workflow_reconcile_automatic(
    repo: &RepoRuntime,
    scope: AutomaticReconciliationScope,
    task_filter: Option<&str>,
    trigger: AutomaticReconciliationTrigger,
    limit: Option<usize>,
) -> Result<JsonValue, String> {
    if trigger == AutomaticReconciliationTrigger::ScheduledRemote {
        if scope == AutomaticReconciliationScope::Local {
            return Err("Scheduled reconciliation requires remote scope.".to_string());
        }
        let (requested_remote, use_default_remote) = scope.selection();
        if normalized_text(requested_remote).is_none()
            && (!use_default_remote || repo.default_remote_name().is_none())
        {
            return Err(
                "Scheduled reconciliation requires `--remote <name>` or a configured default remote."
                    .to_string(),
            );
        }
    }
    let action_limit = limit.unwrap_or_else(|| trigger.default_action_limit());
    let time_budget = trigger.cooperative_time_budget();
    let (remote_name, use_default_remote) = scope.selection();
    let mut output = workflow_reconcile_apply_with_budget(
        repo,
        remote_name,
        task_filter,
        true,
        Some(action_limit),
        use_default_remote,
        Some(time_budget),
        Some(trigger),
    )?;
    let object = output
        .as_object_mut()
        .ok_or_else(|| "Automatic reconciliation payload must be an object.".to_string())?;
    object.insert("automatic".to_string(), JsonValue::Bool(true));
    object.insert(
        "automatic_trigger".to_string(),
        json!({
            "contract": AUTOMATIC_RECONCILIATION_CONTRACT,
            "trigger": trigger.label(),
            "scope": scope.label(),
            "safe_only": true,
            "action_limit": action_limit,
            "cooperative_time_budget_ms": time_budget.as_millis() as u64,
            "continuation_required": object.get("apply_summary")
                .and_then(|summary| summary.get("remaining_safe_count"))
                .and_then(JsonValue::as_u64)
                .unwrap_or(0) > 0,
        }),
    );
    Ok(output)
}

pub fn workflow_reconcile_automatic_best_effort(
    repo: &RepoRuntime,
    scope: AutomaticReconciliationScope,
    task_filter: Option<&str>,
    trigger: AutomaticReconciliationTrigger,
    limit: Option<usize>,
) -> JsonValue {
    let scope_label = scope.label();
    match workflow_reconcile_automatic(repo, scope, task_filter, trigger, limit) {
        Ok(output) => output,
        Err(error) => json!({
            "contract": AUTOMATIC_RECONCILIATION_CONTRACT,
            "automatic": true,
            "status": "failed_non_blocking",
            "trigger": trigger.label(),
            "scope": scope_label,
            "task_filter": task_filter,
            "safe_only": true,
            "mutated": false,
            "error": error,
            "next_command": reconcile_command(repo.default_remote_name().as_deref(), task_filter),
        }),
    }
}
