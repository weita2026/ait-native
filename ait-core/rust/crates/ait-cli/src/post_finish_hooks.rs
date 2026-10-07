//! `hooks.post_finish` configuration and execution.
//!
//! A repository may declare commands that run in the canonical repository
//! root after a successful local `ait task finish` apply. Hooks never change
//! the applied finish; a `fail` policy only changes the process exit code.

use crate::task_land_contract::task_land_exit_code;
use ait_core::json_support::{json, JsonMap, JsonValue};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

pub const HOOKS_CONFIG_KEY: &str = "hooks";
pub const POST_FINISH_HOOKS_CONFIG_FIELD: &str = "post_finish";
pub const POST_FINISH_HOOKS_PAYLOAD_KEY: &str = "post_finish_hooks";
pub const POST_FINISH_HOOK_FAILED_EXIT_CODE: u8 = 3;
/// Repository-tracked hook declarations; travels in Snapshots unlike `.ait/config.json`.
pub const TRACKED_HOOKS_FILE: &str = ".ait-hooks.json";
pub const PRE_FINISH_HOOKS_FIELD: &str = "pre_finish";
pub const PRE_FINISH_HOOKS_PAYLOAD_KEY: &str = "pre_finish_hooks";
const OUTPUT_TAIL_CHARS: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookFailurePolicy {
    Warn,
    Fail,
}

impl HookFailurePolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "warn" => Ok(Self::Warn),
            "fail" => Ok(Self::Fail),
            other => Err(format!(
                "hooks.post_finish[].on_failure must be `warn` or `fail`, not `{other}`."
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostFinishHook {
    pub run: Vec<String>,
    pub on_failure: HookFailurePolicy,
    /// Skip this hook when the default Line head is no longer the Snapshot this
    /// finish applied; a later finish owns the rebuild then.
    pub run_if_head_unchanged: bool,
}

impl PostFinishHook {
    pub fn to_json(&self) -> JsonValue {
        json!({
            "run": self.run,
            "on_failure": self.on_failure.as_str(),
            "run_if_head_unchanged": self.run_if_head_unchanged,
        })
    }

    fn display(&self) -> String {
        self.run.join(" ")
    }
}

/// Parse the `hooks.post_finish` value: a list of `{run: [argv...], on_failure?}`.
pub fn parse_post_finish_hooks(value: &JsonValue) -> Result<Vec<PostFinishHook>, String> {
    parse_hook_list(value, "hooks.post_finish", HookFailurePolicy::Warn)
}

/// Parse one hook list under `label`; `default_policy` applies when an entry
/// omits `on_failure`.
pub fn parse_hook_list(
    value: &JsonValue,
    label: &str,
    default_policy: HookFailurePolicy,
) -> Result<Vec<PostFinishHook>, String> {
    let rows = value.as_array().ok_or_else(|| {
        format!(
            "{label} must be a JSON array of {{\"run\": [argv...], \"on_failure\": \"warn\"|\"fail\"}} entries."
        )
    })?;
    rows.iter()
        .enumerate()
        .map(|(index, row)| parse_hook_entry(label, index, row, default_policy))
        .collect()
}

fn parse_hook_entry(
    label: &str,
    index: usize,
    row: &JsonValue,
    default_policy: HookFailurePolicy,
) -> Result<PostFinishHook, String> {
    let object = row
        .as_object()
        .ok_or_else(|| format!("{label}[{index}] must be an object with a `run` argv array."))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "run" | "on_failure" | "run_if_head_unchanged") {
            return Err(format!(
                "{label}[{index}] has unknown field `{key}`; only `run`, `on_failure`, and `run_if_head_unchanged` are supported."
            ));
        }
    }
    let run_if_head_unchanged = match object.get("run_if_head_unchanged") {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(value)) => *value,
        Some(_) => {
            return Err(format!(
                "{label}[{index}].run_if_head_unchanged must be a boolean."
            ))
        }
    };
    let run = object
        .get("run")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{label}[{index}].run must be a non-empty argv array."))?;
    let run = run
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string)
                .ok_or_else(|| format!("{label}[{index}].run entries must be non-empty strings."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if run.is_empty() {
        return Err(format!(
            "{label}[{index}].run must be a non-empty argv array."
        ));
    }
    let on_failure = match object.get("on_failure") {
        None | Some(JsonValue::Null) => default_policy,
        Some(JsonValue::String(text)) => HookFailurePolicy::parse(text)?,
        Some(_) => {
            return Err(format!(
                "{label}[{index}].on_failure must be the string `warn` or `fail`."
            ))
        }
    };
    Ok(PostFinishHook {
        run,
        on_failure,
        run_if_head_unchanged,
    })
}

/// Serialize hook execution across concurrent finishes of the same repository
/// so two rebuilds never interleave on the canonical source tree. Blocks until
/// the previous holder releases; the guard releases on drop.
pub fn acquire_hooks_lock(
    repo_root: &Path,
) -> Result<ait_core::file_io::BoxedFileIoProcessLockGuard, String> {
    use ait_core::file_io::{
        FileIoByteStore, FileIoLockMode, FileIoLockStore, FileIoLockWait, FilesystemFileIoStore,
    };
    let path = repo_root
        .join(".ait")
        .join("locks")
        .join("finish-hooks.lock");
    let store = FilesystemFileIoStore;
    store
        .create_parent_dirs(&path)
        .map_err(|error| format!("create hook lock directory: {error}"))?;
    store
        .acquire_process_lock(&path, FileIoLockMode::Exclusive, FileIoLockWait::Blocking)
        .map_err(|error| format!("acquire hook lock {}: {error}", path.display()))?
        .ok_or_else(|| format!("hook lock {} was not granted", path.display()))
}

/// Locate the tracked hooks file: the active workspace first (a Task worktree
/// carries the Snapshot copy), then the canonical root.
pub fn tracked_hooks_file(
    workspace_root: &Path,
    authoritative_root: &Path,
) -> Option<std::path::PathBuf> {
    [workspace_root, authoritative_root]
        .iter()
        .map(|root| root.join(TRACKED_HOOKS_FILE))
        .find(|candidate| candidate.is_file())
}

/// Read `pre_finish` hooks from a tracked hooks file. Entries default to
/// `on_failure: fail` because a pre-finish gate that cannot block is not a gate.
pub fn pre_finish_hooks_from_tracked_file(path: &Path) -> Result<Vec<PostFinishHook>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    let value = crate::json_support::parse_value(&text, TRACKED_HOOKS_FILE)?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{TRACKED_HOOKS_FILE} must be a JSON object."))?;
    for key in object.keys() {
        if key != PRE_FINISH_HOOKS_FIELD {
            return Err(format!(
                "{TRACKED_HOOKS_FILE} has unknown field `{key}`; only `{PRE_FINISH_HOOKS_FIELD}` is supported."
            ));
        }
    }
    match object.get(PRE_FINISH_HOOKS_FIELD) {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(value) => parse_hook_list(
            value,
            &format!("{TRACKED_HOOKS_FILE}.{PRE_FINISH_HOOKS_FIELD}"),
            HookFailurePolicy::Fail,
        ),
    }
}

/// Load pre-finish hooks for a repository runtime; `Ok(None)` when no tracked
/// hooks file exists.
pub fn pre_finish_hooks_for_roots(
    workspace_root: &Path,
    authoritative_root: &Path,
) -> Result<Option<(std::path::PathBuf, Vec<PostFinishHook>)>, String> {
    let Some(path) = tracked_hooks_file(workspace_root, authoritative_root) else {
        return Ok(None);
    };
    let hooks = pre_finish_hooks_from_tracked_file(&path)?;
    Ok(Some((path, hooks)))
}

/// Human text lines for a `pre_finish_hooks` result.
pub fn pre_finish_hooks_text_lines(payload: &JsonValue) -> Vec<String> {
    hook_result_text_lines(payload, PRE_FINISH_HOOKS_PAYLOAD_KEY, "pre-finish hooks")
}

/// Read `hooks.post_finish` from a repository config object; absent means none.
pub fn post_finish_hooks_from_config(
    config: &JsonMap<String, JsonValue>,
) -> Result<Vec<PostFinishHook>, String> {
    let Some(hooks) = config.get(HOOKS_CONFIG_KEY) else {
        return Ok(Vec::new());
    };
    let hooks = hooks
        .as_object()
        .ok_or_else(|| "config.hooks must be an object.".to_string())?;
    match hooks.get(POST_FINISH_HOOKS_CONFIG_FIELD) {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(value) => parse_post_finish_hooks(value),
    }
}

pub fn post_finish_hooks_json(hooks: &[PostFinishHook]) -> JsonValue {
    JsonValue::Array(hooks.iter().map(PostFinishHook::to_json).collect())
}

/// Environment exported to every hook process.
#[derive(Clone, Debug, Default)]
pub struct PostFinishHookContext {
    pub task_id: Option<String>,
    pub change_ref: Option<String>,
    pub finished_snapshot_id: Option<String>,
    pub target_line: Option<String>,
    /// Paths changed by the applied Snapshot, from `repo_root_restore.landed_diff_paths`.
    pub changed_paths: Vec<String>,
    /// Default Line head observed after acquiring the hook lock; compared with
    /// `finished_snapshot_id` for `run_if_head_unchanged` hooks.
    pub current_head_snapshot_id: Option<String>,
}

impl PostFinishHookContext {
    pub fn head_moved(&self) -> bool {
        match (&self.finished_snapshot_id, &self.current_head_snapshot_id) {
            (Some(finished), Some(current)) => finished != current,
            _ => false,
        }
    }
}

impl PostFinishHookContext {
    pub fn from_finish_payload(payload: &JsonValue) -> Self {
        let text = |key: &str| {
            payload
                .get(key)
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        };
        let changed_paths = payload
            .get("repo_root_restore")
            .and_then(|value| value.get("landed_diff_paths"))
            .and_then(JsonValue::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(JsonValue::as_str)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self {
            task_id: text("task_id"),
            change_ref: text("change_ref"),
            finished_snapshot_id: text("landed_snapshot_id"),
            target_line: text("target_line"),
            changed_paths,
            current_head_snapshot_id: None,
        }
    }

    /// Write the changed-path list for hooks; one path per line. Returns the
    /// file path, or `None` when the file could not be written.
    fn write_changed_paths_file(&self) -> Option<std::path::PathBuf> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "ait-post-finish-{}-{}-{stamp}.paths",
            self.task_id.as_deref().unwrap_or("task"),
            std::process::id()
        ));
        let mut body = self.changed_paths.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        std::fs::write(&path, body).ok().map(|_| path)
    }

    fn env_pairs(
        &self,
        repo_root: &Path,
    ) -> (Vec<(&'static str, String)>, Option<std::path::PathBuf>) {
        use ait_core::environment_contract::names;
        let mut pairs = vec![
            (
                names::AIT_REPO_ROOT,
                repo_root.to_string_lossy().to_string(),
            ),
            (
                names::AIT_CHANGED_PATH_COUNT,
                self.changed_paths.len().to_string(),
            ),
        ];
        let changed_paths_file = self.write_changed_paths_file();
        if let Some(file) = changed_paths_file.as_ref() {
            pairs.push((
                names::AIT_CHANGED_PATHS_FILE,
                file.to_string_lossy().to_string(),
            ));
        }
        for (key, value) in [
            (names::AIT_TASK_ID, &self.task_id),
            (names::AIT_CHANGE_REF, &self.change_ref),
            (names::AIT_FINISHED_SNAPSHOT_ID, &self.finished_snapshot_id),
            (names::AIT_TARGET_LINE, &self.target_line),
        ] {
            if let Some(value) = value {
                pairs.push((key, value.clone()));
            }
        }
        (pairs, changed_paths_file)
    }
}

fn output_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end();
    let char_count = text.chars().count();
    if char_count <= OUTPUT_TAIL_CHARS {
        return text.to_string();
    }
    text.chars()
        .skip(char_count - OUTPUT_TAIL_CHARS)
        .collect::<String>()
}

/// Run every configured hook in order inside `repo_root`.
///
/// A `warn` failure is recorded and the next hook still runs. A `fail`
/// failure is recorded and the remaining hooks are skipped. The result is
/// always a JSON object; this function never returns an error because the
/// finish that preceded it is already applied.
pub fn run_post_finish_hooks(
    repo_root: &Path,
    hooks: &[PostFinishHook],
    context: &PostFinishHookContext,
) -> JsonValue {
    if hooks.is_empty() {
        return json!({
            "status": "skipped",
            "reason": "no_hooks_configured",
            "repo_root": repo_root.to_string_lossy().to_string(),
            "hooks": [],
        });
    }
    let (env, changed_paths_file) = context.env_pairs(repo_root);
    let mut entries = Vec::with_capacity(hooks.len());
    let mut warn_failures = 0_u64;
    let mut fail_failures = 0_u64;
    let mut stop = false;
    for (index, hook) in hooks.iter().enumerate() {
        if stop {
            entries.push(json!({
                "index": index,
                "run": hook.run,
                "on_failure": hook.on_failure.as_str(),
                "status": "skipped",
                "reason": "earlier_fail_hook_failed",
            }));
            continue;
        }
        if hook.run_if_head_unchanged && context.head_moved() {
            entries.push(json!({
                "index": index,
                "run": hook.run,
                "on_failure": hook.on_failure.as_str(),
                "status": "skipped",
                "reason": "head_moved",
                "finished_snapshot_id": context.finished_snapshot_id,
                "current_head_snapshot_id": context.current_head_snapshot_id,
                "detail": "The default Line head advanced past this finish before the hook ran; the finish that owns the current head runs it.",
            }));
            continue;
        }
        let started = Instant::now();
        let mut command = Command::new(&hook.run[0]);
        command.args(&hook.run[1..]).current_dir(repo_root);
        for (key, value) in &env {
            command.env(key, value);
        }
        let mut entry = JsonMap::new();
        entry.insert("index".to_string(), json!(index));
        entry.insert("run".to_string(), json!(hook.run));
        entry.insert(
            "on_failure".to_string(),
            JsonValue::String(hook.on_failure.as_str().to_string()),
        );
        let failed = match command.output() {
            Ok(output) => {
                let exit_code = output.status.code();
                let succeeded = output.status.success();
                entry.insert(
                    "status".to_string(),
                    JsonValue::String(if succeeded { "succeeded" } else { "failed" }.to_string()),
                );
                entry.insert("exit_code".to_string(), json!(exit_code));
                entry.insert(
                    "stdout_tail".to_string(),
                    JsonValue::String(output_tail(&output.stdout)),
                );
                entry.insert(
                    "stderr_tail".to_string(),
                    JsonValue::String(output_tail(&output.stderr)),
                );
                !succeeded
            }
            Err(error) => {
                entry.insert(
                    "status".to_string(),
                    JsonValue::String("spawn_failed".to_string()),
                );
                entry.insert("exit_code".to_string(), JsonValue::Null);
                entry.insert(
                    "error".to_string(),
                    JsonValue::String(format!("{}: {error}", hook.display())),
                );
                true
            }
        };
        entry.insert(
            "duration_ms".to_string(),
            json!(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        );
        if failed {
            match hook.on_failure {
                HookFailurePolicy::Warn => warn_failures += 1,
                HookFailurePolicy::Fail => {
                    fail_failures += 1;
                    stop = true;
                }
            }
        }
        entries.push(JsonValue::Object(entry));
    }
    if let Some(file) = changed_paths_file {
        let _ = std::fs::remove_file(file);
    }
    let status = if fail_failures > 0 {
        "failed"
    } else if warn_failures > 0 {
        "partial"
    } else {
        "complete"
    };
    json!({
        "status": status,
        "repo_root": repo_root.to_string_lossy().to_string(),
        "configured_count": hooks.len(),
        "failed_count": warn_failures + fail_failures,
        "fail_policy_failure_count": fail_failures,
        "hooks": entries,
        "detail": match status {
            "failed" => "A post-finish hook with on_failure=fail did not succeed. The Task finish itself is applied; inspect the hook output, then rerun the hook command manually.",
            "partial" => "A post-finish hook with on_failure=warn did not succeed. The Task finish itself is applied.",
            _ => "Every configured post-finish hook succeeded.",
        },
    })
}

pub fn post_finish_hooks_failed(payload: &JsonValue) -> bool {
    payload
        .get(POST_FINISH_HOOKS_PAYLOAD_KEY)
        .and_then(|value| value.get("status"))
        .and_then(JsonValue::as_str)
        == Some("failed")
}

/// Task finish exit code: the closeout contract decides first, then a
/// `fail` hook failure maps to a distinct non-zero code.
pub fn task_finish_exit_code(payload: &JsonValue) -> u8 {
    let base = task_land_exit_code(payload);
    if base != 0 {
        return base;
    }
    if post_finish_hooks_failed(payload) {
        POST_FINISH_HOOK_FAILED_EXIT_CODE
    } else {
        0
    }
}

/// Human text lines describing a `post_finish_hooks` result; empty when the
/// result is absent or no hook is configured.
pub fn post_finish_hooks_text_lines(payload: &JsonValue) -> Vec<String> {
    hook_result_text_lines(payload, POST_FINISH_HOOKS_PAYLOAD_KEY, "hooks")
}

/// Select the recorded failure tail without requiring another state query.
/// Hook capture already bounds each output tail to OUTPUT_TAIL_CHARS.
pub(crate) fn hook_failure_diagnostic(entry: &JsonValue) -> Option<&str> {
    ["error", "stderr_tail", "stdout_tail"]
        .into_iter()
        .find_map(|field| {
            entry
                .get(field)
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
        })
}

fn hook_result_text_lines(payload: &JsonValue, key: &str, label: &str) -> Vec<String> {
    let Some(result) = payload.get(key).and_then(JsonValue::as_object) else {
        return Vec::new();
    };
    let status = result
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    if status == "skipped" {
        return Vec::new();
    }
    let configured = result
        .get("configured_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let failed = result
        .get("failed_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let mut lines = vec![format!(
        "{label}: {status} ({configured} configured, {failed} failed)"
    )];
    if matches!(status, "failed" | "partial") {
        if key == POST_FINISH_HOOKS_PAYLOAD_KEY {
            lines.push(
                "Task apply succeeded; repair the hook separately from closeout.".to_string(),
            );
        }
        if let Some(root) = result.get("repo_root").and_then(JsonValue::as_str) {
            lines.push(format!("hook cwd: {root}"));
        }
    }
    if let Some(error) = result.get("error").and_then(JsonValue::as_str) {
        lines.push(format!("hook error: {error}"));
    }
    for entry in result
        .get("hooks")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let entry_status = entry
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown");
        if matches!(entry_status, "succeeded" | "skipped") {
            continue;
        }
        let run = entry
            .get("run")
            .and_then(JsonValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let policy = entry
            .get("on_failure")
            .and_then(JsonValue::as_str)
            .unwrap_or("warn");
        let exit_code = entry
            .get("exit_code")
            .and_then(JsonValue::as_i64)
            .map(|code| code.to_string())
            .unwrap_or_else(|| "none".to_string());
        lines.push(format!(
            "hook {entry_status} ({policy}): {run} -> exit {exit_code}"
        ));
        if let Some(tail) = hook_failure_diagnostic(entry) {
            lines.push(format!("hook output: {tail}"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_text_retains_stdout_cause_and_working_directory() {
        let payload = json!({"post_finish_hooks": {
            "status": "failed", "configured_count": 1, "failed_count": 1,
            "repo_root": "/repo", "hooks": [{"status": "failed",
                "run": ["build"], "exit_code": 1, "stderr_tail": "",
                "stdout_tail": "missing config.toml\nbuild failed"}]
        }});
        let text = post_finish_hooks_text_lines(&payload).join("\n");
        assert!(text.contains("hook cwd: /repo"));
        assert!(text.contains("missing config.toml\nbuild failed"));
        assert!(text.contains("repair the hook separately from closeout"));
        assert_eq!(
            hook_failure_diagnostic(&json!({"error": "spawn error", "stderr_tail": "other"})),
            Some("spawn error")
        );
        assert_eq!(hook_failure_diagnostic(&json!({"stdout_tail": " "})), None);
    }

    fn hook(argv: &[&str], on_failure: HookFailurePolicy) -> PostFinishHook {
        PostFinishHook {
            run: argv.iter().map(ToString::to_string).collect(),
            on_failure,
            run_if_head_unchanged: false,
        }
    }

    #[test]
    fn head_bound_hooks_skip_when_the_default_line_moved() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut bound = hook(&["sh", "-c", "touch ran.txt"], HookFailurePolicy::Warn);
        bound.run_if_head_unchanged = true;
        let moved = PostFinishHookContext {
            finished_snapshot_id: Some("SNP-A".to_string()),
            current_head_snapshot_id: Some("SNP-B".to_string()),
            ..PostFinishHookContext::default()
        };
        assert!(moved.head_moved());
        let result = run_post_finish_hooks(temp.path(), std::slice::from_ref(&bound), &moved);
        assert_eq!(result["status"], json!("complete"));
        assert_eq!(result["hooks"][0]["status"], json!("skipped"));
        assert_eq!(result["hooks"][0]["reason"], json!("head_moved"));
        assert!(!temp.path().join("ran.txt").exists());
        let same = PostFinishHookContext {
            finished_snapshot_id: Some("SNP-A".to_string()),
            current_head_snapshot_id: Some("SNP-A".to_string()),
            ..PostFinishHookContext::default()
        };
        assert!(!same.head_moved());
        let result = run_post_finish_hooks(temp.path(), std::slice::from_ref(&bound), &same);
        assert_eq!(result["hooks"][0]["status"], json!("succeeded"));
        let parsed =
            parse_post_finish_hooks(&json!([{"run": ["x"], "run_if_head_unchanged": true}]))
                .unwrap();
        assert!(parsed[0].run_if_head_unchanged);
        assert!(
            parse_post_finish_hooks(&json!([{"run": ["x"], "run_if_head_unchanged": "yes"}]))
                .is_err()
        );
    }

    #[test]
    fn hooks_lock_is_exclusive_and_released_on_drop() {
        let temp = tempfile::TempDir::new().unwrap();
        let first = acquire_hooks_lock(temp.path()).unwrap();
        let root = temp.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let _second = acquire_hooks_lock(&root).unwrap();
            started.elapsed()
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(first);
        let waited = handle.join().unwrap();
        assert!(
            waited >= std::time::Duration::from_millis(150),
            "{waited:?}"
        );
    }

    #[test]
    fn parse_accepts_argv_entries_and_defaults_to_warn() {
        let parsed = parse_post_finish_hooks(&json!([
            {"run": ["./ait.sh", "core", "build"]},
            {"run": ["sh", "-c", "echo ok"], "on_failure": "fail"}
        ]))
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].on_failure, HookFailurePolicy::Warn);
        assert_eq!(parsed[1].on_failure, HookFailurePolicy::Fail);
        assert_eq!(parsed[0].run, vec!["./ait.sh", "core", "build"]);
        assert_eq!(
            post_finish_hooks_json(&parsed)[1]["on_failure"],
            json!("fail")
        );
    }

    #[test]
    fn parse_rejects_malformed_entries_with_exact_messages() {
        let cases: [(JsonValue, &str); 5] = [
            (json!({"run": ["x"]}), "must be a JSON array"),
            (json!([{"run": []}]), "non-empty argv array"),
            (
                json!([{"run": ["x"], "on_failure": "retry"}]),
                "`warn` or `fail`",
            ),
            (json!([{"run": ["x"], "cwd": "."}]), "unknown field `cwd`"),
            (json!([{"run": ["x", ""]}]), "non-empty strings"),
        ];
        for (value, expected) in cases {
            let error = parse_post_finish_hooks(&value).unwrap_err();
            assert!(error.contains(expected), "{value}: {error}");
        }
    }

    #[test]
    fn config_without_hooks_yields_none() {
        assert!(post_finish_hooks_from_config(&JsonMap::new())
            .unwrap()
            .is_empty());
        let mut config = JsonMap::new();
        config.insert("hooks".to_string(), json!({}));
        assert!(post_finish_hooks_from_config(&config).unwrap().is_empty());
        config.insert("hooks".to_string(), json!("bad"));
        assert!(post_finish_hooks_from_config(&config).is_err());
    }

    #[test]
    fn no_hooks_is_skipped_and_exit_code_stays_zero() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = run_post_finish_hooks(temp.path(), &[], &PostFinishHookContext::default());
        assert_eq!(result["status"], json!("skipped"));
        let payload = json!({ POST_FINISH_HOOKS_PAYLOAD_KEY: result });
        assert!(!post_finish_hooks_failed(&payload));
        assert_eq!(task_finish_exit_code(&payload), 0);
        assert!(post_finish_hooks_text_lines(&payload).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn hooks_run_in_repo_root_with_exported_context() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = PostFinishHookContext {
            task_id: Some("LT-0001".to_string()),
            change_ref: Some("LT-0001/C-01".to_string()),
            finished_snapshot_id: Some("SNP-ABC".to_string()),
            target_line: Some("main".to_string()),
            changed_paths: vec!["src/a.rs".to_string(), "docs/b.md".to_string()],
            current_head_snapshot_id: None,
        };
        let result = run_post_finish_hooks(
            temp.path(),
            &[hook(
                &["sh", "-c", "printf '%s|%s|%s|%s|%s|' \"$AIT_TASK_ID\" \"$AIT_FINISHED_SNAPSHOT_ID\" \"$AIT_TARGET_LINE\" \"$(pwd)\" \"$AIT_CHANGED_PATH_COUNT\" > marker.txt; cat \"$AIT_CHANGED_PATHS_FILE\" | tr '\\n' ',' >> marker.txt"],
                HookFailurePolicy::Warn,
            )],
            &context,
        );
        assert_eq!(result["status"], json!("complete"), "{result}");
        let marker = std::fs::read_to_string(temp.path().join("marker.txt")).unwrap();
        let parts = marker.split('|').collect::<Vec<_>>();
        assert_eq!(parts[0], "LT-0001");
        assert_eq!(parts[1], "SNP-ABC");
        assert_eq!(parts[2], "main");
        assert_eq!(
            std::fs::canonicalize(parts[3]).unwrap(),
            std::fs::canonicalize(temp.path()).unwrap()
        );
        assert_eq!(parts[4], "2");
        assert_eq!(parts[5], "src/a.rs,docs/b.md,");
        let leftovers = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("ait-post-finish-LT-0001-{}-", std::process::id()))
            })
            .count();
        assert_eq!(
            leftovers, 0,
            "changed-paths temp file must be removed after hooks run"
        );
    }

    #[test]
    fn tracked_hooks_file_parses_pre_finish_with_fail_default_and_workspace_precedence() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("root");
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        assert!(tracked_hooks_file(&worktree, &root).is_none());
        std::fs::write(
            root.join(TRACKED_HOOKS_FILE),
            r#"{"pre_finish": [{"run": ["cargo", "fmt", "--all", "--check"]}]}"#,
        )
        .unwrap();
        assert_eq!(
            tracked_hooks_file(&worktree, &root),
            Some(root.join(TRACKED_HOOKS_FILE))
        );
        std::fs::write(
            worktree.join(TRACKED_HOOKS_FILE),
            r#"{"pre_finish": [{"run": ["true"], "on_failure": "warn"}]}"#,
        )
        .unwrap();
        let (path, hooks) = pre_finish_hooks_for_roots(&worktree, &root)
            .unwrap()
            .unwrap();
        assert_eq!(path, worktree.join(TRACKED_HOOKS_FILE));
        assert_eq!(hooks[0].on_failure, HookFailurePolicy::Warn);
        let root_hooks =
            pre_finish_hooks_from_tracked_file(&root.join(TRACKED_HOOKS_FILE)).unwrap();
        assert_eq!(root_hooks[0].on_failure, HookFailurePolicy::Fail);
        std::fs::write(root.join(TRACKED_HOOKS_FILE), r#"{"post_finish": []}"#).unwrap();
        let error = pre_finish_hooks_from_tracked_file(&root.join(TRACKED_HOOKS_FILE)).unwrap_err();
        assert!(error.contains("unknown field `post_finish`"), "{error}");
        std::fs::write(root.join(TRACKED_HOOKS_FILE), "{}").unwrap();
        assert!(
            pre_finish_hooks_from_tracked_file(&root.join(TRACKED_HOOKS_FILE))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn context_reads_changed_paths_from_the_finish_payload() {
        let context = PostFinishHookContext::from_finish_payload(&json!({
            "task_id": "LT-9",
            "repo_root_restore": {"landed_diff_paths": ["a.rs", "b.md"]},
        }));
        assert_eq!(context.changed_paths, vec!["a.rs", "b.md"]);
        assert!(PostFinishHookContext::from_finish_payload(&json!({}))
            .changed_paths
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn warn_failure_is_partial_and_keeps_running() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = run_post_finish_hooks(
            temp.path(),
            &[
                hook(
                    &["sh", "-c", "echo boom >&2; exit 3"],
                    HookFailurePolicy::Warn,
                ),
                hook(&["sh", "-c", "touch second.txt"], HookFailurePolicy::Warn),
            ],
            &PostFinishHookContext::default(),
        );
        assert_eq!(result["status"], json!("partial"));
        assert_eq!(result["failed_count"], json!(1));
        assert_eq!(result["hooks"][0]["exit_code"], json!(3));
        assert_eq!(result["hooks"][0]["stderr_tail"], json!("boom"));
        assert_eq!(result["hooks"][1]["status"], json!("succeeded"));
        assert!(temp.path().join("second.txt").exists());
        let payload = json!({ POST_FINISH_HOOKS_PAYLOAD_KEY: result });
        assert_eq!(task_finish_exit_code(&payload), 0);
        let lines = post_finish_hooks_text_lines(&payload);
        assert_eq!(lines[0], "hooks: partial (2 configured, 1 failed)");
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("hook failed (warn): sh -c")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|line| line == "hook output: boom"));
    }

    #[cfg(unix)]
    #[test]
    fn fail_failure_stops_remaining_hooks_and_sets_exit_code() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = run_post_finish_hooks(
            temp.path(),
            &[
                hook(&["sh", "-c", "exit 7"], HookFailurePolicy::Fail),
                hook(&["sh", "-c", "touch never.txt"], HookFailurePolicy::Warn),
            ],
            &PostFinishHookContext::default(),
        );
        assert_eq!(result["status"], json!("failed"));
        assert_eq!(result["hooks"][1]["status"], json!("skipped"));
        assert!(!temp.path().join("never.txt").exists());
        let payload = json!({ POST_FINISH_HOOKS_PAYLOAD_KEY: result });
        assert!(post_finish_hooks_failed(&payload));
        assert_eq!(
            task_finish_exit_code(&payload),
            POST_FINISH_HOOK_FAILED_EXIT_CODE
        );
        let partial = json!({
            "closeout_status": "partial",
            POST_FINISH_HOOKS_PAYLOAD_KEY: payload[POST_FINISH_HOOKS_PAYLOAD_KEY].clone(),
        });
        assert_eq!(task_finish_exit_code(&partial), 2);
    }

    #[test]
    fn spawn_failure_is_reported_not_raised() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = run_post_finish_hooks(
            temp.path(),
            &[hook(
                &["./definitely-missing-ait-hook-binary"],
                HookFailurePolicy::Warn,
            )],
            &PostFinishHookContext::default(),
        );
        assert_eq!(result["status"], json!("partial"));
        assert_eq!(result["hooks"][0]["status"], json!("spawn_failed"));
    }
}
