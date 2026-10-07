//! Best-effort guidance when Patchset CI waits with no runner claiming work.
//!
//! The server does not expose runner liveness. Queued Worker Jobs with zero
//! running Worker Jobs is the observable signal that no `ait-runner` is
//! serving this repository, so the CLI derives an exact one-shot runner
//! command from it. Listing failures are ignored: the hint is advice, never a
//! gate.

use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonValue};
use ait_core::plan_http_client::{PlanHttpClientConfig, PlanHttpClientManager};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const RUNNER_HINT_PAYLOAD_KEY: &str = "runner_hint";
pub const RUNNER_HINT_CODE: &str = "no_runner_claiming_ci";
pub const CI_RUNNER_ONCE_PAYLOAD_KEY: &str = "ci_runner_once";
const WORKER_JOB_LIST_LIMIT: u32 = 100;

fn job_state(job: &JsonValue) -> String {
    for key in ["state", "diagnostic_status"] {
        if let Some(text) = job
            .get(key)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return text.to_ascii_lowercase();
        }
    }
    String::new()
}

fn worker_jobs(payload: &JsonValue) -> Vec<&JsonValue> {
    payload
        .get("jobs")
        .and_then(JsonValue::as_array)
        .or_else(|| payload.as_array())
        .map(|rows| rows.iter().collect())
        .unwrap_or_default()
}

/// Exact runner command for the observed server and repository.
pub fn runner_once_command(server_url: &str, repository_index: Option<u32>) -> String {
    let repository_index = repository_index
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<repository-index>".to_string());
    format!(
        "ait-runner serve --server {server_url} --worker-id <worker-name> --repository-index {repository_index} --once --attempt-root <fresh-empty-dir>"
    )
}

/// Derive the hint from a Worker Job listing. `None` when a runner is
/// active (a running job exists) or nothing is queued.
pub fn derive_runner_hint(
    jobs_payload: &JsonValue,
    server_url: &str,
    repository_index: Option<u32>,
) -> Option<JsonValue> {
    let jobs = worker_jobs(jobs_payload);
    let queued = jobs.iter().filter(|job| job_state(job) == "queued").count();
    let running = jobs
        .iter()
        .filter(|job| job_state(job) == "running")
        .count();
    if queued == 0 || running > 0 {
        return None;
    }
    Some(json!({
        "code": RUNNER_HINT_CODE,
        "queued_jobs": queued,
        "running_jobs": running,
        "server_url": server_url,
        "repository_index": repository_index,
        "detail": format!(
            "{queued} queued Worker Job(s) and no running Worker Job: no ait-runner is claiming CI for this repository, so Patchset CI will stay pending until one runs."
        ),
        "command": runner_once_command(server_url, repository_index),
    }))
}

fn payload_waits_for_ci(payload: &JsonValue) -> bool {
    let code = |value: Option<&JsonValue>| {
        value
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    code(payload.get("apply_status")) == "waiting_for_ci"
        || code(payload.get("next_action").and_then(|next| next.get("code"))) == "waiting_for_ci"
}

/// Attach `runner_hint` to a workflow payload that stopped at `waiting_for_ci`.
/// Every failure to observe the server is ignored.
pub fn attach_runner_hint_best_effort(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    payload: &mut JsonValue,
) {
    if !payload_waits_for_ci(payload) {
        return;
    }
    let Ok(remote_row) = repo.remote_row(remote_name) else {
        return;
    };
    let Some(repository_index) = repo.repository_index() else {
        return;
    };
    let Ok(mut client) = PlanHttpClientManager::new(PlanHttpClientConfig {
        base_url: remote_row.url.clone(),
        repository_index: Some(repository_index),
        headers: repo.auth_headers(),
        ..PlanHttpClientConfig::default()
    }) else {
        return;
    };
    let Ok(jobs) = client.list_worker_jobs(repository_index, None, WORKER_JOB_LIST_LIMIT) else {
        return;
    };
    if let Some(hint) = derive_runner_hint(&jobs, &remote_row.url, Some(repository_index.get())) {
        if let Some(object) = payload.as_object_mut() {
            object.insert(RUNNER_HINT_PAYLOAD_KEY.to_string(), hint);
        }
    }
}

/// Text appended to a rendered workflow result when a hint is present.
pub fn runner_hint_text(payload: &JsonValue) -> Option<String> {
    let hint = payload
        .get(RUNNER_HINT_PAYLOAD_KEY)
        .and_then(JsonValue::as_object)?;
    let detail = hint.get("detail").and_then(JsonValue::as_str)?;
    let command = hint.get("command").and_then(JsonValue::as_str)?;
    Some(format!("runner: {detail}\nRun `{command}`"))
}

/// Locate `ait-runner`: a sibling of the running `ait` executable first, then
/// the PATH lookup the hint command itself relies on.
pub fn resolve_runner_executable() -> PathBuf {
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            for name in ["ait-runner", "ait-runner.exe"] {
                let candidate = parent.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("ait-runner")
}

fn output_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end();
    let count = text.chars().count();
    if count <= 2000 {
        text.to_string()
    } else {
        text.chars().skip(count - 2000).collect()
    }
}

/// Run one scoped `ait-runner serve --once` for the repository named by a
/// runner hint. Never returns an error: the caller re-applies ready afterwards
/// and the authoritative CI state decides what happened.
pub fn run_ci_once(hint: &JsonValue) -> JsonValue {
    let Some(server_url) = hint.get("server_url").and_then(JsonValue::as_str) else {
        return json!({"status": "skipped", "reason": "hint_missing_server_url"});
    };
    let Some(repository_index) = hint.get("repository_index").and_then(JsonValue::as_u64) else {
        return json!({"status": "skipped", "reason": "hint_missing_repository_index"});
    };
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default();
    let worker_id = format!("ait-ready-once-{}-{stamp}", std::process::id());
    let attempt_root = std::env::temp_dir().join(format!("ait-runner-once-{worker_id}"));
    let executable = resolve_runner_executable();
    let argv = vec![
        "serve".to_string(),
        "--server".to_string(),
        server_url.to_string(),
        "--worker-id".to_string(),
        worker_id.clone(),
        "--repository-index".to_string(),
        repository_index.to_string(),
        "--once".to_string(),
        "--attempt-root".to_string(),
        attempt_root.to_string_lossy().to_string(),
    ];
    let started = Instant::now();
    let outcome = Command::new(&executable).args(&argv).output();
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let command_text = format!("{} {}", executable.display(), argv.join(" "));
    match outcome {
        Ok(output) => json!({
            "status": if output.status.success() { "succeeded" } else { "failed" },
            "exit_code": output.status.code(),
            "worker_id": worker_id,
            "attempt_root": attempt_root.to_string_lossy().to_string(),
            "command": command_text,
            "duration_ms": duration_ms,
            "stdout_tail": output_tail(&output.stdout),
            "stderr_tail": output_tail(&output.stderr),
        }),
        Err(error) => json!({
            "status": "spawn_failed",
            "error": format!("{}: {error}", executable.display()),
            "command": command_text,
            "detail": "ait-runner was not found beside `ait` or on PATH. Install the runner, or run the hinted command from a host that has it.",
        }),
    }
}

/// Text line for a `ci_runner_once` result; none when absent.
pub fn ci_runner_once_text(payload: &JsonValue) -> Option<String> {
    let result = payload
        .get(CI_RUNNER_ONCE_PAYLOAD_KEY)
        .and_then(JsonValue::as_object)?;
    let status = result
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    let mut line = format!("ci runner once: {status}");
    if let Some(code) = result.get("exit_code").and_then(JsonValue::as_i64) {
        line.push_str(&format!(" (exit {code})"));
    }
    if let Some(reason) = result.get("reason").and_then(JsonValue::as_str) {
        line.push_str(&format!(" ({reason})"));
    }
    if status != "succeeded" {
        if let Some(tail) = result
            .get("stderr_tail")
            .and_then(JsonValue::as_str)
            .and_then(|text| text.lines().last())
            .or_else(|| result.get("error").and_then(JsonValue::as_str))
        {
            line.push_str(&format!("\nci runner output: {tail}"));
        }
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_without_running_yields_exact_command() {
        let jobs = json!({"jobs": [
            {"worker_job_index": 1, "state": "queued"},
            {"worker_job_index": 2, "state": "succeeded"}
        ]});
        let hint = derive_runner_hint(&jobs, "http://127.0.0.1:8088", Some(7)).unwrap();
        assert_eq!(hint["code"], json!(RUNNER_HINT_CODE));
        assert_eq!(hint["queued_jobs"], json!(1));
        assert_eq!(
            hint["command"],
            json!("ait-runner serve --server http://127.0.0.1:8088 --worker-id <worker-name> --repository-index 7 --once --attempt-root <fresh-empty-dir>")
        );
        let text = runner_hint_text(&json!({RUNNER_HINT_PAYLOAD_KEY: hint})).unwrap();
        assert!(text.starts_with("runner: 1 queued Worker Job(s)"), "{text}");
        assert!(
            text.ends_with("--attempt-root <fresh-empty-dir>`"),
            "{text}"
        );
    }

    #[test]
    fn running_job_or_empty_queue_yields_no_hint() {
        let active = json!({"jobs": [
            {"state": "queued"},
            {"state": "running"}
        ]});
        assert!(derive_runner_hint(&active, "http://x", Some(1)).is_none());
        let idle = json!({"jobs": [{"state": "succeeded"}]});
        assert!(derive_runner_hint(&idle, "http://x", Some(1)).is_none());
        assert!(derive_runner_hint(&json!({}), "http://x", None).is_none());
        assert!(runner_hint_text(&json!({})).is_none());
    }

    #[test]
    fn diagnostic_status_and_bare_arrays_are_accepted() {
        let jobs = json!([{"diagnostic_status": "Queued"}]);
        let hint = derive_runner_hint(&jobs, "http://x", None).unwrap();
        assert_eq!(hint["repository_index"], JsonValue::Null);
        assert!(hint["command"]
            .as_str()
            .unwrap()
            .contains("--repository-index <repository-index>"));
    }

    #[test]
    fn run_ci_once_reports_missing_hint_fields_without_spawning() {
        assert_eq!(
            run_ci_once(&json!({}))["reason"],
            json!("hint_missing_server_url")
        );
        assert_eq!(
            run_ci_once(&json!({"server_url": "http://x"}))["reason"],
            json!("hint_missing_repository_index")
        );
        let executable = resolve_runner_executable();
        assert!(executable.to_string_lossy().contains("ait-runner"));
        let text = ci_runner_once_text(&json!({
            CI_RUNNER_ONCE_PAYLOAD_KEY: {"status": "failed", "exit_code": 2, "stderr_tail": "a\nboom"}
        }))
        .unwrap();
        assert_eq!(
            text,
            "ci runner once: failed (exit 2)\nci runner output: boom"
        );
        assert!(ci_runner_once_text(&json!({})).is_none());
    }

    #[test]
    fn only_waiting_for_ci_payloads_are_eligible() {
        assert!(payload_waits_for_ci(
            &json!({"apply_status": "waiting_for_ci"})
        ));
        assert!(payload_waits_for_ci(
            &json!({"next_action": {"code": "waiting_for_ci"}})
        ));
        assert!(!payload_waits_for_ci(&json!({"apply_status": "done"})));
    }
}
