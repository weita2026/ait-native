use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    build_agent_token_report, build_agent_token_schedule, digest_workspace,
    extract_agent_token_secondary_metrics, extract_and_validate_codex_transcript,
    import_codex_usage, load_agent_token_campaign, materialize_game_fixture,
    render_agent_token_report_markdown, sha256_digest, write_json_new, write_text_new,
    AgentTokenBrowserReport, AgentTokenCampaignManifest, AgentTokenCommandTranscript,
    AgentTokenEnvironment, AgentTokenMode, AgentTokenRunSummary, AgentTokenScheduleEntry,
    AGENT_TOKEN_BROWSER_REPORT_CONTRACT, AGENT_TOKEN_ENVIRONMENT_CONTRACT,
    AGENT_TOKEN_PROTOCOL_V1_JSON, AGENT_TOKEN_RUN_SUMMARY_CONTRACT,
};

pub const AGENT_TOKEN_CAMPAIGN_EXECUTION_CONTRACT: &str = "ait-agent-token-benchmark-execution/v1";
pub const AGENT_TOKEN_RUN_MANIFEST_CONTRACT: &str = "ait-agent-token-run-manifest/v1";
pub const AGENT_TOKEN_RUN_INDEX_CONTRACT: &str = "ait-agent-token-run-index/v1";
pub const AGENT_TOKEN_WORKFLOW_VERIFICATION_CONTRACT: &str =
    "ait-agent-token-workflow-verification/v1";

#[derive(Clone, Debug, Serialize)]
pub struct AgentTokenCampaignExecution {
    pub contract: &'static str,
    pub campaign_id: String,
    pub output_dir: PathBuf,
    pub scheduled_run_count: usize,
    pub executed_run_count: usize,
    pub accepted_run_count: usize,
    pub invalid_run_count: usize,
    pub failed_run_count: usize,
    pub stopped_early: bool,
    pub stop_reason: Option<String>,
    pub claim_eligible: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenRunManifest {
    pub contract: String,
    pub campaign_id: String,
    pub run_id: String,
    pub workload_id: String,
    pub mode: AgentTokenMode,
    pub accounting_profile: String,
    pub attempt: usize,
    pub block_index: usize,
    pub randomized_order: usize,
    pub fixture_revision: String,
    pub fixture_content_digest: String,
    pub shared_task_prompt_digest: String,
    pub measured_prompt_digest: String,
    pub workspace: String,
    pub network_policy: String,
    pub tool_policy: String,
    pub project_document_loading: String,
    pub workflow_mode: String,
    pub sprint_mode: String,
    pub ait_server_allowed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenWorkflowVerification {
    pub contract: String,
    pub mode: AgentTokenMode,
    pub closed: bool,
    pub workflow_mode: String,
    pub sprint_mode: String,
    pub default_remote_present: bool,
    pub remote_count: Option<u64>,
    pub ait_server_configured: bool,
    pub workspace_dirty: Option<bool>,
    pub current_line: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenRunIndex {
    pub contract: String,
    pub campaign_id: String,
    pub generated_at: String,
    pub scheduled_run_count: usize,
    pub executed_run_count: usize,
    pub runs: Vec<AgentTokenRunIndexEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentTokenRunIndexEntry {
    pub run_id: String,
    pub workload_id: String,
    pub mode: AgentTokenMode,
    pub attempt: usize,
    pub valid_attempt: bool,
    pub accepted_equivalent: bool,
    pub provider_total_tokens: Option<u64>,
    pub run_summary: String,
}

#[derive(Clone, Debug, Serialize)]
struct ExternalCommandEvent {
    sequence: usize,
    phase: String,
    program: String,
    args: Vec<String>,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

struct TimedProcessResult {
    exit_code: Option<i32>,
    timed_out: bool,
    elapsed_ms: u64,
}

struct AitBootstrap {
    worktree_add_dir: PathBuf,
}

pub fn run_agent_token_campaign(
    manifest_path: &Path,
    output_dir: &Path,
    max_runs: Option<usize>,
) -> Result<AgentTokenCampaignExecution, String> {
    if max_runs == Some(0) {
        return Err("max_runs must be greater than zero when supplied".to_string());
    }
    let manifest = load_agent_token_campaign(manifest_path)?;
    if manifest.model.model_id.starts_with("REPLACE_")
        || manifest.model.model_revision.starts_with("REPLACE_")
    {
        return Err(
            "Campaign execution requires a real pinned model id and revision, not template placeholders"
                .to_string(),
        );
    }
    prepare_empty_directory(output_dir, "campaign output")?;
    fs::create_dir(output_dir.join("runs")).map_err(|error| {
        format!(
            "Failed to create campaign runs directory {}: {error}",
            output_dir.join("runs").display()
        )
    })?;

    copy_file_new(manifest_path, &output_dir.join("campaign-manifest.json"))?;
    copy_file_new(
        &manifest.runtime.fixture_manifest,
        &output_dir.join("fixture-manifest.json"),
    )?;
    let protocol = serde_json::from_str::<serde_json::Value>(AGENT_TOKEN_PROTOCOL_V1_JSON)
        .map_err(|error| format!("Compiled agent-token protocol is invalid: {error}"))?;
    write_json_new(&output_dir.join("protocol.json"), &protocol)?;
    let schedule = build_agent_token_schedule(&manifest);
    write_json_new(&output_dir.join("randomization-schedule.json"), &schedule)?;

    let versions = capture_versions(&manifest)?;
    let limit = max_runs.unwrap_or(schedule.entries.len());
    let mut runs = Vec::new();
    let mut stop_reason = None;
    for entry in schedule.entries.iter().take(limit) {
        let run = run_one(&manifest, entry, output_dir, &versions)?;
        if let Some(reason) = run.infrastructure_failure.as_deref() {
            stop_reason = Some(format!("{}: {reason}", run.run_id));
        }
        runs.push(run);
        if stop_reason.is_some() {
            break;
        }
    }

    let index = AgentTokenRunIndex {
        contract: AGENT_TOKEN_RUN_INDEX_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        generated_at: Utc::now().to_rfc3339(),
        scheduled_run_count: schedule.entries.len(),
        executed_run_count: runs.len(),
        runs: runs
            .iter()
            .map(|run| AgentTokenRunIndexEntry {
                run_id: run.run_id.clone(),
                workload_id: run.workload_id.clone(),
                mode: run.mode,
                attempt: run.attempt,
                valid_attempt: run.valid_attempt,
                accepted_equivalent: run.accepted_equivalent,
                provider_total_tokens: run.usage.as_ref().map(|usage| usage.provider_total_tokens),
                run_summary: format!("runs/{}/run-summary.json", run.run_id),
            })
            .collect(),
    };
    write_json_new(&output_dir.join("raw-run-index.json"), &index)?;
    let report = build_agent_token_report(&manifest, &schedule, &runs)?;
    write_json_new(&output_dir.join("aggregate-report.json"), &report)?;
    write_json_new(
        &output_dir.join("comparison-report.json"),
        &serde_json::json!({
            "contract": "ait-agent-token-mode-comparison-report/v1",
            "campaign_id": report.campaign_id,
            "protocol_revision": report.protocol_revision,
            "claim_eligible": report.claim_eligible,
            "comparisons": report.comparisons,
            "blockers": report.blockers,
        }),
    )?;
    let mut claim_boundary = render_agent_token_report_markdown(&report);
    claim_boundary.push_str(
        "\n## Claim Boundary\n\nThis campaign compares only the pinned game-development workloads, model, accounting profile, and single-session local topology. It does not connect to `ait-server` and does not support a general AIT-versus-Git product claim.\n",
    );
    write_text_new(&output_dir.join("claim-boundary.md"), &claim_boundary)?;

    Ok(AgentTokenCampaignExecution {
        contract: AGENT_TOKEN_CAMPAIGN_EXECUTION_CONTRACT,
        campaign_id: manifest.campaign_id,
        output_dir: output_dir.to_path_buf(),
        scheduled_run_count: schedule.entries.len(),
        executed_run_count: runs.len(),
        accepted_run_count: runs.iter().filter(|run| run.accepted_equivalent).count(),
        invalid_run_count: runs.iter().filter(|run| !run.valid_attempt).count(),
        failed_run_count: runs
            .iter()
            .filter(|run| run.valid_attempt && !run.accepted_equivalent)
            .count(),
        stopped_early: runs.len() != schedule.entries.len(),
        stop_reason,
        claim_eligible: report.claim_eligible,
    })
}

fn run_one(
    manifest: &AgentTokenCampaignManifest,
    entry: &AgentTokenScheduleEntry,
    campaign_dir: &Path,
    versions: &CapturedVersions,
) -> Result<AgentTokenRunSummary, String> {
    let run_dir = campaign_dir.join("runs").join(&entry.run_id);
    prepare_empty_directory(&run_dir, "run evidence")?;
    let workspace = run_dir.join("workspace");
    let receipt = materialize_game_fixture(
        &manifest.runtime.fixture_manifest,
        &entry.workload_id,
        &workspace,
    )?;
    copy_file_new(
        &campaign_dir.join("campaign-manifest.json"),
        &run_dir.join("campaign-manifest.json"),
    )?;
    copy_file_new(
        &campaign_dir.join("fixture-manifest.json"),
        &run_dir.join("fixture-manifest.json"),
    )?;

    let shared_task = fs::read_to_string(workspace.join("TASK.txt")).map_err(|error| {
        format!(
            "Failed to read workload task {}: {error}",
            workspace.join("TASK.txt").display()
        )
    })?;
    let prompt = build_measured_prompt(manifest, entry, &shared_task);
    write_text_new(&run_dir.join("prompt.txt"), &prompt)?;
    let shared_task_prompt_digest = sha256_digest(shared_task.as_bytes());
    let measured_prompt_digest = sha256_digest(prompt.as_bytes());

    let mut bootstrap_events = Vec::new();
    let mut sequence = 1_usize;
    let git_metadata_path = run_dir.join("private/git-metadata");
    let (add_dir, git_metadata_dir) = match (manifest.accounting_profile, entry.mode) {
        (
            crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            AgentTokenMode::GitLinearSingleSession,
        ) => {
            prepare_empty_directory(&git_metadata_path, "Git metadata")?;
            bootstrap_git(
                manifest,
                &workspace,
                &git_metadata_path,
                &mut bootstrap_events,
                &mut sequence,
            )?;
            (
                Some(git_metadata_path.clone()),
                Some(git_metadata_path.clone()),
            )
        }
        (
            crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            AgentTokenMode::AitLinearSingleSession,
        ) => (
            Some(
                bootstrap_ait(manifest, &workspace, &mut bootstrap_events, &mut sequence)?
                    .worktree_add_dir,
            ),
            None,
        ),
        (
            crate::AgentTokenAccountingProfile::FirstUseTotalCost,
            AgentTokenMode::AitLinearSingleSession,
        ) => (
            manifest.runtime.ait_first_use_worktree_add_dir.clone(),
            None,
        ),
        (
            crate::AgentTokenAccountingProfile::FirstUseTotalCost,
            AgentTokenMode::GitLinearSingleSession,
        ) => {
            prepare_empty_directory(&git_metadata_path, "Git metadata")?;
            (
                Some(git_metadata_path.clone()),
                Some(git_metadata_path.clone()),
            )
        }
    };
    write_json_lines_new(
        &run_dir.join("private/bootstrap-events.jsonl"),
        &bootstrap_events,
    )?;

    let run_manifest = AgentTokenRunManifest {
        contract: AGENT_TOKEN_RUN_MANIFEST_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        run_id: entry.run_id.clone(),
        workload_id: entry.workload_id.clone(),
        mode: entry.mode,
        accounting_profile: manifest.accounting_profile.as_str().to_string(),
        attempt: entry.attempt,
        block_index: entry.block_index,
        randomized_order: entry.randomized_order,
        fixture_revision: receipt.fixture_revision.clone(),
        fixture_content_digest: receipt.content_digest.clone(),
        shared_task_prompt_digest,
        measured_prompt_digest,
        workspace: "workspace".to_string(),
        network_policy: manifest.network_policy.clone(),
        tool_policy: manifest.tool_policy.clone(),
        project_document_loading: "disabled_symmetrically_project_doc_max_bytes_0".to_string(),
        workflow_mode: match entry.mode {
            AgentTokenMode::GitLinearSingleSession => "git_local".to_string(),
            AgentTokenMode::AitLinearSingleSession => "solo_local".to_string(),
        },
        sprint_mode: match entry.mode {
            AgentTokenMode::GitLinearSingleSession => "not_applicable".to_string(),
            AgentTokenMode::AitLinearSingleSession => "off".to_string(),
        },
        ait_server_allowed: false,
    };
    write_json_new(&run_dir.join("run-manifest.json"), &run_manifest)?;

    let raw_events = run_dir.join("private/codex-events.raw.jsonl");
    let codex_stderr = run_dir.join("private/codex.stderr.txt");
    let codex = run_codex(
        manifest,
        &workspace,
        add_dir.as_deref(),
        git_metadata_dir.as_deref(),
        &prompt,
        &raw_events,
        &codex_stderr,
    )?;
    let usage_result = import_codex_usage(
        &raw_events,
        &entry.run_id,
        &entry.workload_id,
        entry.mode,
        manifest.accounting_profile,
        &manifest.model,
    );
    let transcript_result = extract_and_validate_codex_transcript(
        &raw_events,
        &entry.run_id,
        entry.mode,
        manifest.accounting_profile,
    );
    let transcript = transcript_result.unwrap_or_else(|error| AgentTokenCommandTranscript {
        contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
        run_id: entry.run_id.clone(),
        mode: entry.mode,
        accounting_profile: manifest.accounting_profile,
        command_count: 0,
        commands: Vec::new(),
        valid: false,
        errors: vec![error],
        observed_required_commands: Vec::new(),
    });
    let secondary_metrics =
        extract_agent_token_secondary_metrics(&raw_events, &transcript).unwrap_or_default();
    write_command_events(&run_dir.join("command-events.jsonl"), &transcript)?;
    let usage = usage_result.ok();
    let infrastructure_failure =
        classify_codex_infrastructure_failure(&raw_events, &codex, &transcript, usage.as_ref());
    if let Some(value) = &usage {
        write_json_line_new(&run_dir.join("provider-usage.jsonl"), value)?;
    } else {
        write_text_new(&run_dir.join("provider-usage.jsonl"), "")?;
    }

    let acceptance = run_acceptance(manifest, &receipt, &workspace, &run_dir)?;
    let browser = run_browser_acceptance(manifest, &receipt, &workspace, &run_dir)?;
    let workflow = verify_workflow(
        manifest,
        entry.mode,
        &workspace,
        git_metadata_dir.as_deref(),
    )?;
    write_json_new(&run_dir.join("workflow-verification.json"), &workflow)?;

    let environment = AgentTokenEnvironment {
        contract: AGENT_TOKEN_ENVIRONMENT_CONTRACT.to_string(),
        captured_at: Utc::now().to_rfc3339(),
        os: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        codex_version: versions.codex.clone(),
        ait_version: versions.ait.clone(),
        git_version: versions.git.clone(),
        node_version: versions.node.clone(),
        browser_version: versions.browser.clone(),
        workflow_mode: workflow.workflow_mode.clone(),
        sprint_mode: workflow.sprint_mode.clone(),
        ait_server_connected: workflow.ait_server_configured,
        network_policy: manifest.network_policy.clone(),
        cache_class: manifest.cache_class.clone(),
    };
    write_json_new(&run_dir.join("environment.json"), &environment)?;

    let final_content_digest = digest_workspace(
        &workspace,
        &[
            ".ait".to_string(),
            ".git".to_string(),
            ".ait-worktree-links".to_string(),
            "AGENTS.md".to_string(),
        ],
    )
    .ok();
    let mut invalid_reasons = Vec::new();
    if usage.is_none() {
        invalid_reasons.push("provider usage is missing or has an unknown schema".to_string());
    }
    if let Some(reason) = infrastructure_failure.as_deref() {
        invalid_reasons.push(format!("candidate infrastructure unavailable: {reason}"));
    }
    if !transcript.valid {
        invalid_reasons.extend(transcript.errors.iter().cloned());
    }
    if receipt.content_digest != run_manifest.fixture_content_digest {
        invalid_reasons.push("fixture digest linkage drifted".to_string());
    }
    if entry.mode == AgentTokenMode::AitLinearSingleSession {
        if workflow.workflow_mode != "solo_local" {
            invalid_reasons.push("AIT workflow mode is not solo_local".to_string());
        }
        if workflow.sprint_mode != "off" {
            invalid_reasons.push("AIT sprint mode is not off".to_string());
        }
        if workflow.default_remote_present
            || workflow.remote_count.unwrap_or_default() != 0
            || workflow.ait_server_configured
        {
            invalid_reasons.push(
                "AIT run configured a default remote, remote authority, or ait-server".to_string(),
            );
        }
    }

    let evaluator_accepted = acceptance
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let evaluator_score = acceptance.get("score").and_then(serde_json::Value::as_u64);
    let mut failure_reasons = Vec::new();
    if codex.timed_out {
        failure_reasons.push("candidate agent timed out".to_string());
    }
    if codex.exit_code != Some(0) {
        failure_reasons.push(format!("candidate agent exited with {:?}", codex.exit_code));
    }
    if !evaluator_accepted {
        failure_reasons.push("functional acceptance rejected the candidate".to_string());
    }
    if browser.status != "passed" {
        failure_reasons.push(format!("browser acceptance status is {}", browser.status));
    }
    if !workflow.closed {
        failure_reasons.push("repository workflow did not close cleanly".to_string());
    }
    let valid_attempt = invalid_reasons.is_empty();
    let accepted_equivalent = valid_attempt
        && !codex.timed_out
        && codex.exit_code == Some(0)
        && evaluator_accepted
        && browser.status == "passed"
        && workflow.closed;
    let summary = AgentTokenRunSummary {
        contract: AGENT_TOKEN_RUN_SUMMARY_CONTRACT.to_string(),
        campaign_id: manifest.campaign_id.clone(),
        run_id: entry.run_id.clone(),
        workload_id: entry.workload_id.clone(),
        mode: entry.mode,
        accounting_profile: manifest.accounting_profile,
        attempt: entry.attempt,
        block_index: entry.block_index,
        randomized_order: entry.randomized_order,
        initial_content_digest: receipt.content_digest,
        final_content_digest,
        codex_exit_code: codex.exit_code,
        codex_timed_out: codex.timed_out,
        elapsed_ms: codex.elapsed_ms,
        infrastructure_failure,
        usage,
        transcript,
        secondary_metrics,
        evaluator_exit_code: acceptance
            .get("_evaluator_exit_code")
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok()),
        evaluator_score,
        evaluator_accepted,
        browser,
        workflow_closed: workflow.closed,
        valid_attempt,
        accepted_equivalent,
        invalid_reasons,
        failure_reasons,
    };
    write_json_new(&run_dir.join("run-summary.json"), &summary)?;
    Ok(summary)
}

fn build_measured_prompt(
    manifest: &AgentTokenCampaignManifest,
    entry: &AgentTokenScheduleEntry,
    shared_task: &str,
) -> String {
    let profile = manifest.accounting_profile.as_str();
    let workflow = match entry.mode {
        AgentTokenMode::GitLinearSingleSession => format!(
            "Use only the local Git repository workflow. Inspect repository state and relevant history with explicit `{git} status`, diff, log, show, branch, or rev-parse commands. Do not invoke `ait`. The runner supplies isolated writable Git metadata through the inherited standard Git environment; use ordinary `{git}` commands in the candidate worktree and do not copy `.git`, set `GIT_DIR` or `GIT_WORK_TREE`, or redirect repository metadata. In first-use profile initialize Git, set repository-local `user.name` to `AIT Benchmark Agent` and `user.email` to `benchmark-agent@example.invalid`, then create a baseline commit before editing; in steady-state profile the initialized baseline and identity already exist. Commit the completed candidate and leave the working tree clean.",
            git = manifest.runtime.git_program.display()
        ),
        AgentTokenMode::AitLinearSingleSession => format!(
            "Use only the current local AIT workflow through `{ait}`. The effective mode must be `solo_local`, sprint must be off, and no default remote or ait-server may be configured or contacted. Never invoke raw Git, Plan commands, `--from`, push, pull, remote CI/review/land, or any `--remote` option. In first-use profile run `{ait} init`, then `{ait} config set --workflow-mode solo_local --sprint off --task-review automatic --default-author-mode ai_only_experimental --default-model {model} --user-name benchmark-agent --user-email benchmark-agent@example.invalid --json`, leave the generated root `AGENTS.md` unchanged, and create the immutable baseline Snapshot before starting the measured task. Start an unbound task with `{ait} task start --title ... --intent ... --local --json`, enter the compact response's physical `edit_root` using `next_action.command`, edit and test there, create `{ait} snapshot create --message ... --json`, then finish with `{ait} task land <task-or-change-id> --local --json`. In steady-state profile initialization and the baseline Snapshot already exist.",
            ait = manifest.runtime.ait_program.display(),
            model = manifest.model.model_id,
        ),
    };
    format!(
        "You are one fresh measured coding session in the AIT agent-token game-development benchmark. Complete the shared task below without asking for help or receiving human repair.\n\nFairness rules:\n- Work only inside this fresh candidate repository and its declared local task worktree.\n- Do not access the public network, install dependencies, use hosted assets, or inspect benchmark evaluator source outside the candidate repository.\n- Do not add Python. Use project-local Node validation and preserve unrelated behavior.\n- The accounting profile is `{profile}` and the workload is `{workload}`.\n- Run the project-local validation before repository closeout.\n\nRepository workflow:\n{workflow}\n\nShared task (identical outcome contract in both modes):\n\n{shared_task}",
        workload = entry.workload_id,
    )
}

fn bootstrap_git(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    metadata_dir: &Path,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<(), String> {
    run_checked_git_event(
        &manifest.runtime.git_program,
        &["init", "--initial-branch=main"],
        workspace,
        metadata_dir,
        "bootstrap",
        events,
        sequence,
    )?;
    for args in [
        vec!["config", "user.name", "AIT Benchmark Agent"],
        vec!["config", "user.email", "benchmark-agent@example.invalid"],
        vec!["add", "--all"],
        vec!["commit", "-m", "Benchmark fixture baseline"],
    ] {
        run_checked_git_event(
            &manifest.runtime.git_program,
            &args,
            workspace,
            metadata_dir,
            "bootstrap",
            events,
            sequence,
        )?;
    }
    Ok(())
}

fn bootstrap_ait(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<AitBootstrap, String> {
    run_checked_event(
        &manifest.runtime.ait_program,
        &["init", "--json"],
        workspace,
        "bootstrap",
        events,
        sequence,
    )?;
    let model = manifest.model.model_id.as_str();
    run_checked_event(
        &manifest.runtime.ait_program,
        &[
            "config",
            "set",
            "--workflow-mode",
            "solo_local",
            "--sprint",
            "off",
            "--task-review",
            "automatic",
            "--default-author-mode",
            "ai_only_experimental",
            "--default-model",
            model,
            "--user-name",
            "benchmark-agent",
            "--user-email",
            "benchmark-agent@example.invalid",
            "--json",
        ],
        workspace,
        "bootstrap",
        events,
        sequence,
    )?;
    run_checked_event(
        &manifest.runtime.ait_program,
        &[
            "snapshot",
            "create",
            "--message",
            "Benchmark fixture baseline",
            "--json",
        ],
        workspace,
        "bootstrap",
        events,
        sequence,
    )?;
    let config = command_json(
        &manifest.runtime.ait_program,
        &["config", "show", "--json"],
        workspace,
    )?;
    validate_solo_local_config(&config)?;
    let worktree_add_dir = config
        .pointer("/task_worktree/ephemeral_root/value")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| {
            "AIT config did not expose task_worktree.ephemeral_root.value".to_string()
        })?;
    Ok(AitBootstrap { worktree_add_dir })
}

fn run_codex(
    manifest: &AgentTokenCampaignManifest,
    workspace: &Path,
    add_dir: Option<&Path>,
    git_metadata_dir: Option<&Path>,
    prompt: &str,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<TimedProcessResult, String> {
    ensure_parent(stdout_path)?;
    ensure_parent(stderr_path)?;
    let stdout = create_new_file(stdout_path)?;
    let stderr = create_new_file(stderr_path)?;
    let mut command = Command::new(&manifest.runtime.codex_program);
    command
        .args([
            "exec",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
            "--json",
            "--skip-git-repo-check",
            "--sandbox",
            "workspace-write",
            "--model",
            manifest.model.model_id.as_str(),
            "--config",
            &format!(
                "model_reasoning_effort=\"{}\"",
                manifest.model.reasoning_effort
            ),
            "--config",
            "sandbox_workspace_write.network_access=false",
            "--config",
            "project_doc_max_bytes=0",
            "--cd",
        ])
        .arg(workspace);
    if let Some(path) = add_dir {
        command.arg("--add-dir").arg(path);
    }
    if let Some(path) = git_metadata_dir {
        configure_git_environment(&mut command, path, workspace);
    }
    command
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .env("NO_COLOR", "1");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let start = Instant::now();
    let mut child = command.spawn().map_err(|error| {
        format!(
            "Failed to launch Codex benchmark subject {}: {error}",
            manifest.runtime.codex_program.display()
        )
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex benchmark subject has no stdin pipe".to_string())?;
    stdin
        .write_all(prompt.as_bytes())
        .map_err(|error| format!("Failed to write Codex benchmark prompt: {error}"))?;
    drop(stdin);
    wait_for_child(
        &mut child,
        Duration::from_secs(manifest.runtime.run_timeout_seconds),
        start,
    )
}

fn classify_codex_infrastructure_failure(
    raw_events: &Path,
    process: &TimedProcessResult,
    transcript: &AgentTokenCommandTranscript,
    usage: Option<&crate::NormalizedAgentTokenUsage>,
) -> Option<String> {
    if process.timed_out
        || process.exit_code == Some(0)
        || usage.is_some()
        || transcript.command_count != 0
    {
        return None;
    }
    let source = fs::read_to_string(raw_events).ok()?;
    let mut messages = Vec::new();
    for line in source.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match event.get("type").and_then(serde_json::Value::as_str) {
            Some("error") => {
                if let Some(message) = event.get("message").and_then(serde_json::Value::as_str) {
                    messages.push(message.to_string());
                }
            }
            Some("turn.failed") => {
                if let Some(message) = event
                    .pointer("/error/message")
                    .and_then(serde_json::Value::as_str)
                {
                    messages.push(message.to_string());
                }
            }
            _ => {}
        }
    }
    if messages.is_empty() {
        return None;
    }
    let combined = messages.join(" ").to_ascii_lowercase();
    let classification =
        if combined.contains("usage limit") || combined.contains("purchase more credits") {
            "provider_usage_limit"
        } else if combined.contains("rate limit") || combined.contains("too many requests") {
            "provider_rate_limit"
        } else if combined.contains("unauthorized")
            || combined.contains("authentication")
            || combined.contains("log in")
        {
            "provider_authentication_failure"
        } else if combined.contains("model")
            && (combined.contains("not found") || combined.contains("unavailable"))
        {
            "provider_model_unavailable"
        } else {
            "provider_session_failed_before_candidate_execution"
        };
    Some(classification.to_string())
}

fn run_acceptance(
    manifest: &AgentTokenCampaignManifest,
    receipt: &crate::GameFixtureReceipt,
    workspace: &Path,
    run_dir: &Path,
) -> Result<serde_json::Value, String> {
    let stdout = run_dir.join("private/acceptance.raw.json");
    let stderr = run_dir.join("private/acceptance.stderr.txt");
    let result = run_process_to_files(
        &manifest.runtime.node_program,
        &[
            receipt.evaluator_path.as_os_str(),
            "--workload".as_ref(),
            receipt.workload_id.as_ref(),
            "--candidate".as_ref(),
            workspace.as_os_str(),
            "--acceptance".as_ref(),
            receipt.acceptance_path.as_os_str(),
        ],
        workspace,
        &stdout,
        &stderr,
        Duration::from_secs(60),
    )?;
    let bytes = fs::read(&stdout).map_err(|error| {
        format!(
            "Failed to read game acceptance report {}: {error}",
            stdout.display()
        )
    })?;
    let mut report = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| {
        format!(
            "Game acceptance evaluator emitted invalid JSON for {}: {error}",
            receipt.workload_id
        )
    })?;
    if result.timed_out || !matches!(result.exit_code, Some(0 | 1)) {
        return Err(format!(
            "Game acceptance evaluator failed as a harness with exit {:?}, timeout={}",
            result.exit_code, result.timed_out
        ));
    }
    if let Some(object) = report.as_object_mut() {
        object.insert("candidate".to_string(), serde_json::json!("workspace"));
        object.insert(
            "_evaluator_exit_code".to_string(),
            serde_json::json!(result.exit_code),
        );
    }
    write_json_new(&run_dir.join("acceptance-report.json"), &report)?;
    Ok(report)
}

fn run_browser_acceptance(
    manifest: &AgentTokenCampaignManifest,
    receipt: &crate::GameFixtureReceipt,
    workspace: &Path,
    run_dir: &Path,
) -> Result<AgentTokenBrowserReport, String> {
    let Some(browser_program) = manifest.runtime.browser_program.as_deref() else {
        let report = AgentTokenBrowserReport {
            contract: AGENT_TOKEN_BROWSER_REPORT_CONTRACT.to_string(),
            workload_id: receipt.workload_id.clone(),
            required_for_equivalent_completion: true,
            status: "unavailable".to_string(),
            desktop_passed: None,
            mobile_passed: None,
            console_errors: None,
            failed_requests: None,
            horizontal_overflow: None,
            notes: vec!["Campaign runtime did not pin a browser program".to_string()],
        };
        write_json_new(&run_dir.join("browser-report.json"), &report)?;
        return Ok(report);
    };
    let stdout = run_dir.join("private/browser.raw.json");
    let stderr = run_dir.join("private/browser.stderr.txt");
    let result = run_process_to_files(
        &manifest.runtime.node_program,
        &[
            receipt.browser_evaluator_path.as_os_str(),
            "--workload".as_ref(),
            receipt.workload_id.as_ref(),
            "--candidate".as_ref(),
            workspace.as_os_str(),
            "--browser".as_ref(),
            browser_program.as_os_str(),
        ],
        workspace,
        &stdout,
        &stderr,
        Duration::from_secs(90),
    )?;
    let report =
        serde_json::from_slice::<AgentTokenBrowserReport>(&fs::read(&stdout).map_err(|error| {
            format!(
                "Failed to read browser acceptance report {}: {error}",
                stdout.display()
            )
        })?)
        .map_err(|error| {
            format!(
                "Browser evaluator emitted invalid JSON for {}: {error}",
                receipt.workload_id
            )
        })?;
    if result.timed_out || !matches!(result.exit_code, Some(0 | 1)) {
        return Err(format!(
            "Browser evaluator failed as a harness with exit {:?}, timeout={}, status={}",
            result.exit_code, result.timed_out, report.status
        ));
    }
    write_json_new(&run_dir.join("browser-report.json"), &report)?;
    Ok(report)
}

fn verify_workflow(
    manifest: &AgentTokenCampaignManifest,
    mode: AgentTokenMode,
    workspace: &Path,
    git_metadata_dir: Option<&Path>,
) -> Result<AgentTokenWorkflowVerification, String> {
    match mode {
        AgentTokenMode::GitLinearSingleSession => {
            let metadata_dir = git_metadata_dir.ok_or_else(|| {
                "Git workflow verification is missing its isolated metadata directory".to_string()
            })?;
            let status = command_output_in_git_context(
                &manifest.runtime.git_program,
                &["status", "--porcelain"],
                workspace,
                metadata_dir,
            );
            let head = command_output_in_git_context(
                &manifest.runtime.git_program,
                &["rev-parse", "--verify", "HEAD"],
                workspace,
                metadata_dir,
            );
            let mut reasons = Vec::new();
            let workspace_dirty = match status {
                Ok(output) => {
                    let dirty = !output.trim().is_empty();
                    if dirty {
                        reasons
                            .push("Git working tree is dirty after candidate closeout".to_string());
                    }
                    Some(dirty)
                }
                Err(error) => {
                    reasons.push(error);
                    None
                }
            };
            if let Err(error) = head {
                reasons.push(error);
            }
            Ok(AgentTokenWorkflowVerification {
                contract: AGENT_TOKEN_WORKFLOW_VERIFICATION_CONTRACT.to_string(),
                mode,
                closed: reasons.is_empty(),
                workflow_mode: "git_local".to_string(),
                sprint_mode: "not_applicable".to_string(),
                default_remote_present: false,
                remote_count: None,
                ait_server_configured: false,
                workspace_dirty,
                current_line: None,
                reasons,
            })
        }
        AgentTokenMode::AitLinearSingleSession => {
            let config = command_json(
                &manifest.runtime.ait_program,
                &["config", "show", "--json"],
                workspace,
            );
            let status = command_json(
                &manifest.runtime.ait_program,
                &["status", "--json", "--full"],
                workspace,
            );
            let mut reasons = Vec::new();
            if let Err(error) = config.as_ref() {
                reasons.push(error.clone());
            }
            if let Err(error) = status.as_ref() {
                reasons.push(error.clone());
            }
            let workflow_mode = config
                .as_ref()
                .ok()
                .and_then(|value| value.pointer("/workflow_mode/value"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let sprint_mode = config
                .as_ref()
                .ok()
                .and_then(|value| value.pointer("/sprint/value"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let default_remote_present = config
                .as_ref()
                .ok()
                .and_then(|value| value.get("default_remote"))
                .is_some_and(|value| !value.is_null());
            let ait_server_configured = config
                .as_ref()
                .ok()
                .and_then(|value| value.pointer("/agent_runtime/server_url"))
                .is_some_and(|value| !value.is_null());
            let remote_count = status
                .as_ref()
                .ok()
                .and_then(|value| value.get("remote_count"))
                .and_then(serde_json::Value::as_u64);
            let workspace_dirty = status
                .as_ref()
                .ok()
                .and_then(|value| value.get("workspace_dirty"))
                .and_then(serde_json::Value::as_bool);
            let current_line = status
                .as_ref()
                .ok()
                .and_then(|value| value.get("current_line"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            if workflow_mode != "solo_local" {
                reasons.push(format!(
                    "AIT workflow_mode is {workflow_mode}, expected solo_local"
                ));
            }
            if sprint_mode != "off" {
                reasons.push(format!("AIT sprint mode is {sprint_mode}, expected off"));
            }
            if default_remote_present {
                reasons.push("AIT default_remote is configured".to_string());
            }
            if remote_count.unwrap_or_default() != 0 {
                reasons.push("AIT remote_count is not zero".to_string());
            }
            if ait_server_configured {
                reasons.push("AIT agent_runtime.server_url is configured".to_string());
            }
            if workspace_dirty != Some(false) {
                reasons.push("AIT target workspace is not clean after local land".to_string());
            }
            if current_line.as_deref() != Some("main") {
                reasons.push("AIT target workspace did not return to logical main".to_string());
            }
            Ok(AgentTokenWorkflowVerification {
                contract: AGENT_TOKEN_WORKFLOW_VERIFICATION_CONTRACT.to_string(),
                mode,
                closed: reasons.is_empty(),
                workflow_mode,
                sprint_mode,
                default_remote_present,
                remote_count,
                ait_server_configured,
                workspace_dirty,
                current_line,
                reasons,
            })
        }
    }
}

fn validate_solo_local_config(config: &serde_json::Value) -> Result<(), String> {
    let workflow = config
        .pointer("/workflow_mode/value")
        .and_then(serde_json::Value::as_str);
    let sprint = config
        .pointer("/sprint/value")
        .and_then(serde_json::Value::as_str);
    let default_remote = config.get("default_remote");
    let server_url = config.pointer("/agent_runtime/server_url");
    if workflow != Some("solo_local")
        || sprint != Some("off")
        || default_remote.is_some_and(|value| !value.is_null())
        || server_url.is_some_and(|value| !value.is_null())
    {
        return Err(
            "AIT bootstrap did not resolve to solo_local, sprint off, null default_remote, and null server_url"
                .to_string(),
        );
    }
    Ok(())
}

#[derive(Clone)]
struct CapturedVersions {
    codex: String,
    ait: String,
    git: String,
    node: String,
    browser: Option<String>,
}

fn capture_versions(manifest: &AgentTokenCampaignManifest) -> Result<CapturedVersions, String> {
    Ok(CapturedVersions {
        codex: program_version(&manifest.runtime.codex_program)?,
        ait: program_version(&manifest.runtime.ait_program)?,
        git: program_version(&manifest.runtime.git_program)?,
        node: program_version(&manifest.runtime.node_program)?,
        browser: manifest
            .runtime
            .browser_program
            .as_deref()
            .map(program_version)
            .transpose()?,
    })
}

fn program_version(program: &Path) -> Result<String, String> {
    let output = Command::new(program)
        .arg("--version")
        .output()
        .map_err(|error| {
            format!(
                "Failed to launch pinned program {} --version: {error}",
                program.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "Pinned program {} --version exited with {:?}: {}",
            program.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let version = if stdout.is_empty() { stderr } else { stdout };
    if version.is_empty() {
        Err(format!(
            "Pinned program {} emitted no version",
            program.display()
        ))
    } else {
        Ok(version)
    }
}

fn run_checked_event(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    phase: &str,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<(), String> {
    run_checked_event_with_git_context(program, args, cwd, None, phase, events, sequence)
}

fn run_checked_git_event(
    program: &Path,
    args: &[&str],
    workspace: &Path,
    metadata_dir: &Path,
    phase: &str,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<(), String> {
    run_checked_event_with_git_context(
        program,
        args,
        workspace,
        Some((metadata_dir, workspace)),
        phase,
        events,
        sequence,
    )
}

fn run_checked_event_with_git_context(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    git_context: Option<(&Path, &Path)>,
    phase: &str,
    events: &mut Vec<ExternalCommandEvent>,
    sequence: &mut usize,
) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    if let Some((metadata_dir, workspace)) = git_context {
        configure_git_environment(&mut command, metadata_dir, workspace);
    }
    let output = command.output().map_err(|error| {
        format!(
            "Failed to launch bootstrap command {} in {}: {error}",
            program.display(),
            cwd.display()
        )
    })?;
    events.push(ExternalCommandEvent {
        sequence: *sequence,
        phase: phase.to_string(),
        program: program.display().to_string(),
        args: args.iter().map(|value| (*value).to_string()).collect(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    });
    *sequence += 1;
    if !output.status.success() {
        return Err(format!(
            "Bootstrap command {} {:?} failed with {:?}: {}",
            program.display(),
            args,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn command_output(program: &Path, args: &[&str], cwd: &Path) -> Result<String, String> {
    command_output_with_git_context(program, args, cwd, None)
}

fn command_output_in_git_context(
    program: &Path,
    args: &[&str],
    workspace: &Path,
    metadata_dir: &Path,
) -> Result<String, String> {
    command_output_with_git_context(program, args, workspace, Some((metadata_dir, workspace)))
}

fn command_output_with_git_context(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    git_context: Option<(&Path, &Path)>,
) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    if let Some((metadata_dir, workspace)) = git_context {
        configure_git_environment(&mut command, metadata_dir, workspace);
    }
    let output = command.output().map_err(|error| {
        format!(
            "Failed to launch {} {:?} in {}: {error}",
            program.display(),
            args,
            cwd.display()
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "Command {} {:?} failed with {:?}: {}",
            program.display(),
            args,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn configure_git_environment(command: &mut Command, metadata_dir: &Path, workspace: &Path) {
    command
        .env("GIT_DIR", metadata_dir)
        .env("GIT_WORK_TREE", workspace);
}

fn command_json(program: &Path, args: &[&str], cwd: &Path) -> Result<serde_json::Value, String> {
    let output = command_output(program, args, cwd)?;
    serde_json::from_str(&output).map_err(|error| {
        format!(
            "Command {} {:?} emitted invalid JSON: {error}",
            program.display(),
            args
        )
    })
}

fn run_process_to_files(
    program: &Path,
    args: &[&std::ffi::OsStr],
    cwd: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
    timeout: Duration,
) -> Result<TimedProcessResult, String> {
    ensure_parent(stdout_path)?;
    ensure_parent(stderr_path)?;
    let stdout = create_new_file(stdout_path)?;
    let stderr = create_new_file(stderr_path)?;
    let start = Instant::now();
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| {
        format!(
            "Failed to launch evaluator {} in {}: {error}",
            program.display(),
            cwd.display()
        )
    })?;
    wait_for_child(&mut child, timeout, start)
}

fn wait_for_child(
    child: &mut std::process::Child,
    timeout: Duration,
    start: Instant,
) -> Result<TimedProcessResult, String> {
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("Failed to poll benchmark process: {error}"))?
        {
            return Ok(TimedProcessResult {
                exit_code: status.code(),
                timed_out: false,
                elapsed_ms: elapsed_millis(start),
            });
        }
        if start.elapsed() >= timeout {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGTERM);
            }
            let grace = Instant::now();
            while grace.elapsed() < Duration::from_millis(500) {
                if child
                    .try_wait()
                    .map_err(|error| format!("Failed to poll timed-out process: {error}"))?
                    .is_some()
                {
                    return Ok(TimedProcessResult {
                        exit_code: None,
                        timed_out: true,
                        elapsed_ms: elapsed_millis(start),
                    });
                }
                thread::sleep(Duration::from_millis(20));
            }
            child
                .kill()
                .map_err(|error| format!("Failed to kill timed-out benchmark process: {error}"))?;
            let _ = child.wait();
            return Ok(TimedProcessResult {
                exit_code: None,
                timed_out: true,
                elapsed_ms: elapsed_millis(start),
            });
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn elapsed_millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn prepare_empty_directory(path: &Path, kind: &str) -> Result<(), String> {
    if path.exists() {
        if !path.is_dir() {
            return Err(format!(
                "{kind} path is not a directory: {}",
                path.display()
            ));
        }
        if fs::read_dir(path)
            .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?
            .next()
            .is_some()
        {
            return Err(format!(
                "{kind} {} must be absent or empty; evidence is never overwritten",
                path.display()
            ));
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|error| format!("Failed to create {}: {error}", path.display()))?;
    }
    Ok(())
}

fn copy_file_new(source: &Path, destination: &Path) -> Result<(), String> {
    let bytes = fs::read(source)
        .map_err(|error| format!("Failed to read {}: {error}", source.display()))?;
    ensure_parent(destination)?;
    let mut output = create_new_file(destination)?;
    output
        .write_all(&bytes)
        .map_err(|error| format!("Failed to write {}: {error}", destination.display()))
}

fn create_new_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "Failed to create {} without overwriting: {error}",
                path.display()
            )
        })
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    }
    Ok(())
}

fn write_json_line_new(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut text = serde_json::to_string(value)
        .map_err(|error| format!("Failed to encode JSONL for {}: {error}", path.display()))?;
    text.push('\n');
    write_text_new(path, &text)
}

fn write_json_lines_new<T: Serialize>(path: &Path, values: &[T]) -> Result<(), String> {
    let mut output = String::new();
    for value in values {
        output.push_str(
            &serde_json::to_string(value).map_err(|error| {
                format!("Failed to encode JSONL for {}: {error}", path.display())
            })?,
        );
        output.push('\n');
    }
    write_text_new(path, &output)
}

fn write_command_events(
    path: &Path,
    transcript: &AgentTokenCommandTranscript,
) -> Result<(), String> {
    let events = transcript
        .commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            serde_json::json!({
                "contract": "ait-agent-token-command-event/v1",
                "sequence": index + 1,
                "command": command,
            })
        })
        .collect::<Vec<_>>();
    write_json_lines_new(path, &events)
}

pub fn validate_agent_token_campaign_evidence(
    manifest: &AgentTokenCampaignManifest,
    campaign_dir: &Path,
) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for required in [
        "campaign-manifest.json",
        "fixture-manifest.json",
        "protocol.json",
        "randomization-schedule.json",
        "raw-run-index.json",
        "aggregate-report.json",
        "comparison-report.json",
        "claim-boundary.md",
    ] {
        let path = campaign_dir.join(required);
        let metadata = fs::symlink_metadata(&path);
        if !metadata
            .as_ref()
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            errors.push(format!(
                "campaign is missing regular evidence file {required}"
            ));
        }
    }
    let schedule =
        crate::load_agent_token_schedule(&campaign_dir.join("randomization-schedule.json"))?;
    let runs = crate::load_agent_token_run_summaries(campaign_dir)?;
    if schedule.contract != crate::AGENT_TOKEN_SCHEDULE_CONTRACT {
        errors.push("schedule contract is unsupported".to_string());
    }
    if schedule.campaign_id != manifest.campaign_id {
        errors.push("schedule campaign_id does not match manifest".to_string());
    }
    if schedule.protocol_revision != manifest.protocol_revision {
        errors.push("schedule protocol_revision does not match manifest".to_string());
    }
    if schedule.entry_count != schedule.entries.len() {
        errors.push("schedule entry_count does not match entries".to_string());
    }
    if runs.len() != schedule.entries.len() {
        errors.push(format!(
            "observed {} run summaries for {} scheduled entries",
            runs.len(),
            schedule.entries.len()
        ));
    }
    let mut seen = BTreeSet::new();
    for run in &runs {
        if run.contract != AGENT_TOKEN_RUN_SUMMARY_CONTRACT {
            errors.push(format!("run {} contract is unsupported", run.run_id));
        }
        if run.campaign_id != manifest.campaign_id {
            errors.push(format!("run {} campaign linkage differs", run.run_id));
        }
        if run.accounting_profile != manifest.accounting_profile {
            errors.push(format!("run {} accounting profile differs", run.run_id));
        }
        if run.transcript.accounting_profile != manifest.accounting_profile {
            errors.push(format!(
                "run {} transcript accounting profile differs",
                run.run_id
            ));
        }
        if !seen.insert(run.run_id.as_str()) {
            errors.push(format!("duplicate run summary id: {}", run.run_id));
        }
        let Some(entry) = schedule
            .entries
            .iter()
            .find(|entry| entry.run_id == run.run_id)
        else {
            errors.push(format!("run {} is absent from schedule", run.run_id));
            continue;
        };
        if entry.workload_id != run.workload_id
            || entry.mode != run.mode
            || entry.attempt != run.attempt
            || entry.block_index != run.block_index
            || entry.randomized_order != run.randomized_order
        {
            errors.push(format!("run {} metadata differs from schedule", run.run_id));
        }
        let run_dir = campaign_dir.join("runs").join(&run.run_id);
        for required in [
            "campaign-manifest.json",
            "fixture-manifest.json",
            "prompt.txt",
            "run-manifest.json",
            "provider-usage.jsonl",
            "command-events.jsonl",
            "acceptance-report.json",
            "browser-report.json",
            "environment.json",
            "workflow-verification.json",
            "run-summary.json",
        ] {
            let path = run_dir.join(required);
            let metadata = fs::symlink_metadata(&path);
            if !metadata
                .as_ref()
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                errors.push(format!(
                    "run {} is missing regular evidence file {required}",
                    run.run_id
                ));
            }
        }
        if let Some(usage) = &run.usage {
            if usage.contract != crate::AGENT_TOKEN_USAGE_CONTRACT
                || usage.run_id != run.run_id
                || usage.workload_id != run.workload_id
                || usage.mode != run.mode
                || usage.accounting_profile != manifest.accounting_profile
            {
                errors.push(format!(
                    "run {} normalized usage linkage differs",
                    run.run_id
                ));
            }
        }
        if run.browser.contract != AGENT_TOKEN_BROWSER_REPORT_CONTRACT
            || run.browser.workload_id != run.workload_id
        {
            errors.push(format!(
                "run {} browser evidence linkage differs",
                run.run_id
            ));
        }
        let environment = run_dir.join("environment.json");
        if environment.is_file() {
            match fs::read(&environment)
                .map_err(|error| format!("Failed to read {}: {error}", environment.display()))
                .and_then(|bytes| {
                    serde_json::from_slice::<AgentTokenEnvironment>(&bytes).map_err(|error| {
                        format!("Failed to decode {}: {error}", environment.display())
                    })
                }) {
                Ok(environment) => {
                    if run.mode == AgentTokenMode::AitLinearSingleSession
                        && (environment.workflow_mode != "solo_local"
                            || environment.sprint_mode != "off"
                            || environment.ait_server_connected)
                    {
                        errors.push(format!(
                            "run {} environment violates solo_local/sprint-off/no-server",
                            run.run_id
                        ));
                    }
                }
                Err(error) => errors.push(error),
            }
        }
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest() -> AgentTokenCampaignManifest {
        AgentTokenCampaignManifest {
            contract: crate::AGENT_TOKEN_CAMPAIGN_CONTRACT.to_string(),
            campaign_id: "test".to_string(),
            protocol_revision: crate::AGENT_TOKEN_PROTOCOL_REVISION.to_string(),
            campaign_scope: crate::AgentTokenCampaignScope::Smoke,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            seed: 1,
            attempts_per_cell: 1,
            workload_ids: vec!["GD-01".to_string()],
            modes: vec![
                AgentTokenMode::GitLinearSingleSession,
                AgentTokenMode::AitLinearSingleSession,
            ],
            model: crate::AgentTokenModelPin {
                provider: "openai".to_string(),
                model_id: "test-model".to_string(),
                model_revision: "test".to_string(),
                reasoning_effort: "medium".to_string(),
            },
            runtime: crate::AgentTokenRuntime {
                codex_program: PathBuf::from("codex"),
                ait_program: PathBuf::from("ait"),
                git_program: PathBuf::from("git"),
                node_program: PathBuf::from("node"),
                browser_program: None,
                fixture_manifest: PathBuf::from("fixture.json"),
                run_timeout_seconds: 60,
                ait_first_use_worktree_add_dir: None,
            },
            cache_class: "provider-default".to_string(),
            network_policy: "disabled_except_loopback".to_string(),
            tool_policy: "codex_shell_only".to_string(),
            bootstrap_resamples: 1_000,
            limitations: Vec::new(),
        }
    }

    #[test]
    fn measured_prompt_keeps_server_remote_and_plan_out_of_ait_core_mode() {
        let manifest = test_manifest();
        let entry = AgentTokenScheduleEntry {
            run_id: "test-b001-gd-01-ait".to_string(),
            workload_id: "GD-01".to_string(),
            mode: AgentTokenMode::AitLinearSingleSession,
            attempt: 1,
            block_index: 1,
            randomized_order: 1,
        };
        let prompt = build_measured_prompt(&manifest, &entry, "repair the game");
        assert!(prompt.contains("solo_local"));
        assert!(prompt.contains("sprint must be off"));
        assert!(prompt.contains("no default remote or ait-server"));
        assert!(prompt.contains("Never invoke raw Git, Plan commands"));
        assert!(prompt.contains("task land <task-or-change-id> --local"));

        let git_entry = AgentTokenScheduleEntry {
            mode: AgentTokenMode::GitLinearSingleSession,
            ..entry
        };
        let git_prompt = build_measured_prompt(&manifest, &git_entry, "repair the game");
        assert!(git_prompt.contains("isolated writable Git metadata"));
        assert!(git_prompt.contains("do not copy `.git`"));
        assert!(git_prompt.contains("do not copy `.git`, set `GIT_DIR` or `GIT_WORK_TREE`"));
        assert!(git_prompt.contains("`user.name` to `AIT Benchmark Agent`"));
        assert!(git_prompt.contains("`user.email` to `benchmark-agent@example.invalid`"));
    }

    #[test]
    fn git_bootstrap_and_closeout_share_external_writable_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let metadata = temp.path().join("git-metadata");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("game.txt"), "baseline\n").unwrap();
        prepare_empty_directory(&metadata, "Git metadata").unwrap();

        let manifest = test_manifest();
        let mut events = Vec::new();
        let mut sequence = 1;
        bootstrap_git(&manifest, &workspace, &metadata, &mut events, &mut sequence).unwrap();
        assert!(!workspace.join(".git").exists());
        assert!(metadata.join("HEAD").is_file());

        fs::write(workspace.join("game.txt"), "repaired\n").unwrap();
        run_checked_git_event(
            &manifest.runtime.git_program,
            &["add", "--all"],
            &workspace,
            &metadata,
            "candidate",
            &mut events,
            &mut sequence,
        )
        .unwrap();
        run_checked_git_event(
            &manifest.runtime.git_program,
            &["commit", "-m", "Repair game"],
            &workspace,
            &metadata,
            "candidate",
            &mut events,
            &mut sequence,
        )
        .unwrap();

        let verification = verify_workflow(
            &manifest,
            AgentTokenMode::GitLinearSingleSession,
            &workspace,
            Some(&metadata),
        )
        .unwrap();
        assert!(verification.closed, "{:?}", verification.reasons);
        assert_eq!(verification.workspace_dirty, Some(false));
        assert!(command_output_in_git_context(
            &manifest.runtime.git_program,
            &["log", "--format=%s", "-1"],
            &workspace,
            &metadata,
        )
        .unwrap()
        .contains("Repair game"));
    }

    #[test]
    fn provider_failure_before_candidate_execution_is_classified_for_fail_fast() {
        let temp = tempfile::tempdir().unwrap();
        let raw_events = temp.path().join("codex-events.raw.jsonl");
        fs::write(
            &raw_events,
            concat!(
                "{\"type\":\"thread.started\",\"thread_id\":\"test\"}\n",
                "{\"type\":\"turn.started\"}\n",
                "{\"type\":\"error\",\"message\":\"You've hit your usage limit.\"}\n",
                "{\"type\":\"turn.failed\",\"error\":{\"message\":\"Purchase more credits.\"}}\n"
            ),
        )
        .unwrap();
        let transcript = AgentTokenCommandTranscript {
            contract: crate::AGENT_TOKEN_TRANSCRIPT_CONTRACT.to_string(),
            run_id: "test".to_string(),
            mode: AgentTokenMode::GitLinearSingleSession,
            accounting_profile: crate::AgentTokenAccountingProfile::SteadyStateTaskCost,
            command_count: 0,
            commands: Vec::new(),
            valid: false,
            errors: Vec::new(),
            observed_required_commands: Vec::new(),
        };
        let process = TimedProcessResult {
            exit_code: Some(1),
            timed_out: false,
            elapsed_ms: 1,
        };

        assert_eq!(
            classify_codex_infrastructure_failure(&raw_events, &process, &transcript, None)
                .as_deref(),
            Some("provider_usage_limit")
        );
    }

    #[test]
    fn evidence_output_refuses_nonempty_directory() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("existing"), "evidence").unwrap();
        let error = prepare_empty_directory(temp.path(), "campaign").unwrap_err();
        assert!(error.contains("never overwritten"));
    }
}
