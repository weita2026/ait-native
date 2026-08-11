use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::str::FromStr;

use ait_agent_core::{
    list_manifest_workers, normalize_agent_worker_manifest, plan_agent_runtime_admission,
    resolve_agent_worker_config, AgentEventLoopBackend, AgentRuntimeAdmissionInput,
    AgentWorkerConfigInput, AgentWorkerRuntimeConfig, TransportKind,
};
use ait_core::json_support::{json, JsonCodec, JsonValue};

use crate::diagnostic::{
    WorkerDiagnostic, EXIT_INVALID_CONFIGURATION, EXIT_INVALID_REQUEST, EXIT_RUNTIME_UNAVAILABLE,
};
use crate::paths::{resolve_worker_paths, ResolvedWorkerPaths, WorkerPathInputs};
use crate::registry::TransportRunnerRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerRunRequest {
    pub transport: String,
    pub worker: String,
    pub event_loop_backend: String,
    pub shard: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkerRunContext {
    pub paths: ResolvedWorkerPaths,
    pub transport: TransportKind,
    pub worker_key: String,
    pub worker_name: String,
    pub event_loop_backend: AgentEventLoopBackend,
    pub shard_index: usize,
    pub runtime_admission_plan: JsonValue,
    pub config: AgentWorkerRuntimeConfig,
}

pub(crate) struct ResolvedWorkerSelection {
    pub(crate) normalized_manifest: JsonValue,
    pub(crate) worker_key: String,
    pub(crate) config: AgentWorkerRuntimeConfig,
}

pub fn execute_worker_request(
    request: &WorkerRunRequest,
    path_inputs: &WorkerPathInputs,
    registry: &TransportRunnerRegistry,
) -> Result<(), WorkerDiagnostic> {
    let context = prepare_worker_run(request, path_inputs)?;
    registry.run(&context)
}

pub fn prepare_worker_run(
    request: &WorkerRunRequest,
    path_inputs: &WorkerPathInputs,
) -> Result<WorkerRunContext, WorkerDiagnostic> {
    prepare_worker_run_with_env(request, path_inputs, env::vars().collect())
}

pub fn prepare_worker_run_with_env(
    request: &WorkerRunRequest,
    path_inputs: &WorkerPathInputs,
    process_env: BTreeMap<String, String>,
) -> Result<WorkerRunContext, WorkerDiagnostic> {
    validate_platform_available()?;
    let transport = TransportKind::from_str(&request.transport).map_err(|_| {
        WorkerDiagnostic::new(
            "unknown_transport",
            format!(
                "Unknown ait-agent transport `{}`. Expected one of: telegram, discord, slack, line.",
                request.transport.trim()
            ),
            EXIT_INVALID_REQUEST,
        )
        .with_detail("transport", request.transport.trim().to_string())
    })?;
    let worker_name = validate_worker_name(&request.worker)?;
    let event_loop_backend = AgentEventLoopBackend::from_label(&request.event_loop_backend)
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "unknown_event_loop_backend",
                format!(
                    "Unknown ait-agent event-loop backend `{}`. Expected linux_epoll or portable_poll.",
                    request.event_loop_backend.trim()
                ),
                EXIT_INVALID_REQUEST,
            )
            .with_detail(
                "event_loop_backend",
                request.event_loop_backend.trim().to_string(),
            )
        })?;
    validate_backend_available(event_loop_backend)?;
    let shard_index = request.shard.trim().parse::<usize>().map_err(|_| {
        WorkerDiagnostic::new(
            "invalid_shard_index",
            format!(
                "Invalid ait-agent shard index `{}`. Expected a non-negative integer.",
                request.shard.trim()
            ),
            EXIT_INVALID_REQUEST,
        )
        .with_detail("shard", request.shard.trim().to_string())
    })?;
    let paths = resolve_worker_paths(path_inputs)?;
    let ResolvedWorkerSelection {
        normalized_manifest,
        worker_key,
        config,
    } = resolve_worker_selection(&paths, transport, &worker_name, process_env)?;
    let (expected_concurrent_workers, workers_per_shard) =
        runtime_admission_overrides(&config, event_loop_backend)?;
    let admission_plan = plan_agent_runtime_admission(AgentRuntimeAdmissionInput {
        worker_manifest: normalized_manifest,
        expected_concurrent_workers,
        backend: Some(event_loop_backend.label().to_string()),
        workers_per_shard,
        transport_runtime: "rust".to_string(),
        allow_python_fallback: false,
        requested_worker_keys: vec![worker_key.clone()],
    })
    .map_err(|message| {
        WorkerDiagnostic::new(
            "event_loop_plan_invalid",
            message,
            EXIT_INVALID_CONFIGURATION,
        )
    })?;
    if !admission_plan.launch_allowed {
        let message = admission_plan
            .rejection_reasons
            .first()
            .or_else(|| admission_plan.diagnostics.first())
            .cloned()
            .unwrap_or_else(|| {
                "The selected event-loop backend cannot admit this worker set.".to_string()
            });
        return Err(WorkerDiagnostic::new(
            "event_loop_capacity_unavailable",
            message,
            EXIT_INVALID_CONFIGURATION,
        )
        .with_detail("event_loop_backend", event_loop_backend.label())
        .with_detail(
            "expected_concurrent_workers",
            admission_plan.expected_concurrent_workers,
        ));
    }
    let lease = admission_plan
        .worker_leases
        .iter()
        .find(|lease| lease.worker_key == worker_key)
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "worker_shard_assignment_missing",
                format!("Rust reactor planning omitted worker '{worker_key}'."),
                EXIT_INVALID_CONFIGURATION,
            )
        })?;
    if shard_index != lease.shard_index {
        return Err(WorkerDiagnostic::new(
            "invalid_shard_assignment",
            format!(
                "Worker `{}` belongs to shard {}, not shard {shard_index}.",
                worker_key, lease.shard_index
            ),
            EXIT_INVALID_REQUEST,
        )
        .with_detail("worker_key", worker_key.clone())
        .with_detail("expected_shard", lease.shard_index)
        .with_detail("requested_shard", shard_index));
    }
    Ok(WorkerRunContext {
        paths,
        transport,
        worker_key,
        worker_name,
        event_loop_backend,
        shard_index,
        runtime_admission_plan: json!(admission_plan),
        config,
    })
}

fn validate_platform_available() -> Result<(), WorkerDiagnostic> {
    if !cfg!(any(unix, windows)) {
        return Err(WorkerDiagnostic::new(
            "worker_platform_unsupported",
            format!(
                "ait-agent-worker does not have a native runtime backend for {}/{}.",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            EXIT_RUNTIME_UNAVAILABLE,
        )
        .with_detail("platform", std::env::consts::OS)
        .with_detail("architecture", std::env::consts::ARCH));
    }
    Ok(())
}

fn runtime_admission_overrides(
    config: &AgentWorkerRuntimeConfig,
    requested_backend: AgentEventLoopBackend,
) -> Result<(Option<usize>, Option<usize>), WorkerDiagnostic> {
    let AgentWorkerRuntimeConfig::Telegram(config) = config else {
        return Ok((None, None));
    };
    if let Some(configured) = config.event_loop_backend.as_deref() {
        let configured_backend =
            AgentEventLoopBackend::from_label(configured).ok_or_else(|| {
                WorkerDiagnostic::new(
                    "telegram_event_loop_backend_invalid",
                    "The Telegram worker event-loop backend configuration is invalid.",
                    EXIT_INVALID_CONFIGURATION,
                )
            })?;
        if configured_backend != requested_backend {
            return Err(WorkerDiagnostic::new(
                "telegram_event_loop_backend_mismatch",
                "The Telegram worker event-loop backend does not match its launch assignment.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("configured_backend", configured_backend.label())
            .with_detail("launch_backend", requested_backend.label()));
        }
    }
    Ok((config.expected_concurrent_workers, config.workers_per_shard))
}

pub(crate) fn resolve_worker_selection(
    paths: &ResolvedWorkerPaths,
    transport: TransportKind,
    worker_name: &str,
    process_env: BTreeMap<String, String>,
) -> Result<ResolvedWorkerSelection, WorkerDiagnostic> {
    let manifest = load_worker_manifest(paths)?;
    let normalized_manifest = normalize_agent_worker_manifest(
        &manifest,
        Some(paths.manifest_path.to_string_lossy().as_ref()),
    );
    let issue_count = normalized_manifest
        .get("issues")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    if issue_count > 0 {
        return Err(WorkerDiagnostic::new(
            "worker_manifest_invalid",
            "The ait-agent worker manifest failed Rust normalization.",
            EXIT_INVALID_CONFIGURATION,
        )
        .with_detail("manifest_path", paths.manifest_path.display().to_string())
        .with_detail("issue_count", issue_count));
    }
    let workers = list_manifest_workers(&normalized_manifest);
    let worker = workers
        .iter()
        .find(|candidate| candidate.transport == transport && candidate.name == worker_name)
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "unknown_worker",
                format!("Unknown {transport} worker `{worker_name}`."),
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("transport", transport.as_str())
            .with_detail("worker", worker_name)
            .with_detail("manifest_path", paths.manifest_path.display().to_string())
        })?;
    let worker_key = worker.key.clone();
    let worker_config_payload = normalized_manifest
        .get("config")
        .and_then(|config| config.get("workers"))
        .and_then(JsonValue::as_object)
        .and_then(|workers| workers.get(&worker_key))
        .cloned()
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "worker_config_missing",
                "The normalized ait-agent worker manifest omitted the selected worker config.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("worker_key", worker_key.clone())
        })?;
    let config = resolve_agent_worker_config(AgentWorkerConfigInput {
        repo_root: paths.repo_root.clone(),
        worker_key: worker_key.clone(),
        worker: worker_config_payload,
        process_env,
    })
    .map_err(|reason| {
        WorkerDiagnostic::new(
            "worker_config_invalid",
            "The selected ait-agent worker configuration is invalid.",
            EXIT_INVALID_CONFIGURATION,
        )
        .with_detail("worker_key", worker_key.clone())
        .with_detail("reason", reason)
    })?;
    Ok(ResolvedWorkerSelection {
        normalized_manifest,
        worker_key,
        config,
    })
}

pub(crate) fn validate_worker_name(value: &str) -> Result<String, WorkerDiagnostic> {
    let worker_name = value.trim();
    if worker_name.is_empty() || worker_name.contains('/') {
        return Err(WorkerDiagnostic::new(
            "invalid_worker_name",
            "The ait-agent worker name must be non-empty and must not contain `/`.",
            EXIT_INVALID_REQUEST,
        )
        .with_detail("worker", worker_name.to_string()));
    }
    Ok(worker_name.to_string())
}

fn validate_backend_available(backend: AgentEventLoopBackend) -> Result<(), WorkerDiagnostic> {
    if backend == AgentEventLoopBackend::LinuxEpoll && !cfg!(target_os = "linux") {
        return Err(WorkerDiagnostic::new(
            "event_loop_backend_unavailable",
            "The linux_epoll event-loop backend is unavailable on this platform.",
            EXIT_INVALID_CONFIGURATION,
        )
        .with_detail("event_loop_backend", backend.label()));
    }
    Ok(())
}

fn load_worker_manifest(paths: &ResolvedWorkerPaths) -> Result<JsonValue, WorkerDiagnostic> {
    let content = fs::read_to_string(&paths.manifest_path).map_err(|error| {
        let code = if error.kind() == std::io::ErrorKind::NotFound {
            "worker_manifest_not_found"
        } else {
            "worker_manifest_read_failed"
        };
        WorkerDiagnostic::new(
            code,
            format!(
                "Cannot read the ait-agent worker manifest at `{}`: {error}",
                paths.manifest_path.display()
            ),
            EXIT_INVALID_CONFIGURATION,
        )
        .with_detail("manifest_path", paths.manifest_path.display().to_string())
    })?;
    JsonCodec::parse_value_with_error_prefix(&content, "Invalid ait-agent worker manifest").map_err(
        |error| {
            WorkerDiagnostic::new(
                "worker_manifest_json_invalid",
                error.to_string(),
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("manifest_path", paths.manifest_path.display().to_string())
        },
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn fixture(
        manifest: &str,
        request: WorkerRunRequest,
    ) -> (tempfile::TempDir, WorkerPathInputs, WorkerRunRequest) {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".ait")).expect("ait dir");
        fs::write(
            temp.path().join(".ait/config.json"),
            r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
        )
        .expect("repo config");
        fs::write(temp.path().join(".ait/agent-workers.json"), manifest).expect("manifest");
        let inputs = WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: None,
            manifest_path_override: None,
        };
        (temp, inputs, request)
    }

    fn telegram_request() -> WorkerRunRequest {
        WorkerRunRequest {
            transport: "telegram".to_string(),
            worker: "main".to_string(),
            event_loop_backend: "portable_poll".to_string(),
            shard: "0".to_string(),
        }
    }

    #[test]
    fn prepares_known_worker_without_starting_a_runner() {
        let (_temp, inputs, request) = fixture(
            r#"{"version":1,"workers":{"telegram/main":{"kind":"telegram","name":"main","token":"secret"}}}"#,
            telegram_request(),
        );

        let context = prepare_worker_run(&request, &inputs).expect("context");

        assert_eq!(context.worker_key, "telegram/main");
        assert_eq!(context.shard_index, 0);
        assert!(matches!(
            context.config,
            AgentWorkerRuntimeConfig::Telegram(_)
        ));
        assert_eq!(
            context.runtime_admission_plan["admission_contract"],
            "ait_agent_core.event_loop.AgentRuntimeAdmission.v1"
        );
        assert_eq!(
            context.runtime_admission_plan["admission_state"],
            "admitted"
        );
        assert_eq!(context.runtime_admission_plan["launch_allowed"], true);
        assert_eq!(context.runtime_admission_plan["backend"], "portable_poll");
        assert_eq!(context.runtime_admission_plan["transport_runtime"], "rust");
        assert_eq!(
            context.runtime_admission_plan["python_worker_execution_allowed"],
            false
        );
        assert_eq!(
            context.runtime_admission_plan["python_fallback_requested"],
            false
        );
        assert_eq!(
            context.runtime_admission_plan["worker_leases"][0]["worker_key"],
            "telegram/main"
        );
        assert_eq!(
            context.runtime_admission_plan["worker_leases"][0]["shard_index"],
            0
        );
    }

    #[test]
    fn prepares_typed_worker_config_from_rust_env_loading() {
        let (temp, inputs, request) = fixture(
            r#"{"version":1,"workers":{"telegram/main":{"kind":"telegram","name":"main"}}}"#,
            telegram_request(),
        );
        fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
        fs::write(
            temp.path().join(".ait/agent-runtime/telegram.env"),
            "AIT_TELEGRAM_BOT_TOKEN=env-only-secret\nAIT_TELEGRAM_POLL_TIMEOUT_SECONDS=12\n",
        )
        .expect("env config");

        let context =
            prepare_worker_run_with_env(&request, &inputs, BTreeMap::new()).expect("context");
        let AgentWorkerRuntimeConfig::Telegram(config) = context.config else {
            panic!("Telegram config");
        };

        assert_eq!(config.token.expose(), "env-only-secret");
        assert_eq!(config.poll_timeout_seconds, 12);
    }

    #[test]
    fn rejects_unknown_worker() {
        let (_temp, inputs, mut request) = fixture(
            r#"{"version":1,"workers":{"telegram/main":{"kind":"telegram","name":"main","token":"secret"}}}"#,
            telegram_request(),
        );
        request.worker = "missing".to_string();

        let error = prepare_worker_run(&request, &inputs).expect_err("unknown worker");

        assert_eq!(error.code, "unknown_worker");
        assert!(!error.render_json().contains("secret"));
    }

    #[test]
    fn rejects_wrong_shard_assignment() {
        let (_temp, inputs, mut request) = fixture(
            r#"{"version":1,"workers":{"telegram/main":{"kind":"telegram","name":"main","token":"secret"}}}"#,
            telegram_request(),
        );
        request.shard = "1".to_string();

        let error = prepare_worker_run(&request, &inputs).expect_err("wrong shard");

        assert_eq!(error.code, "invalid_shard_assignment");
    }

    #[test]
    fn rejects_telegram_backend_configuration_that_differs_from_launch_assignment() {
        let (_temp, inputs, request) = fixture(
            r#"{"version":1,"workers":{"telegram/main":{"kind":"telegram","name":"main","token":"secret"}}}"#,
            telegram_request(),
        );

        let error = prepare_worker_run_with_env(
            &request,
            &inputs,
            BTreeMap::from([(
                "AIT_AGENT_EVENT_LOOP_BACKEND".to_string(),
                "linux_epoll".to_string(),
            )]),
        )
        .expect_err("configured backend mismatch");

        assert_eq!(error.code, "telegram_event_loop_backend_mismatch");
        assert_eq!(error.details["configured_backend"], "linux_epoll");
        assert_eq!(error.details["launch_backend"], "portable_poll");
        assert!(!error.render_json().contains("secret"));
    }

    #[test]
    fn rejects_high_concurrency_telegram_admission_on_portable_poll() {
        let (_temp, inputs, request) = fixture(
            r#"{"version":1,"workers":{"telegram/main":{"kind":"telegram","name":"main","token":"secret"}}}"#,
            telegram_request(),
        );

        let error = prepare_worker_run_with_env(
            &request,
            &inputs,
            BTreeMap::from([
                (
                    "AIT_AGENT_EXPECTED_CONCURRENT_WORKERS".to_string(),
                    "64".to_string(),
                ),
                ("AIT_AGENT_WORKERS_PER_SHARD".to_string(), "32".to_string()),
            ]),
        )
        .expect_err("portable poll cannot admit high concurrency");

        assert_eq!(error.code, "event_loop_capacity_unavailable");
        assert_eq!(error.details["event_loop_backend"], "portable_poll");
        assert_eq!(error.details["expected_concurrent_workers"], 64);
        assert!(!error.render_json().contains("secret"));
    }

    #[test]
    fn rejects_invalid_manifest_before_runner_execution() {
        let (_temp, inputs, request) = fixture("{not-json", telegram_request());

        let error = prepare_worker_run(&request, &inputs).expect_err("invalid manifest");

        assert_eq!(error.code, "worker_manifest_json_invalid");
    }
}
