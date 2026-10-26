use std::collections::BTreeMap;

use ait_agent_core::TransportKind;
use serde::Serialize;

use crate::diagnostic::{WorkerDiagnostic, EXIT_RUNTIME_UNAVAILABLE};
use crate::{
    run_discord_transport, run_line_transport, run_slack_transport, run_telegram_transport,
    WorkerRunContext,
};

pub type TransportRunner = fn(&WorkerRunContext) -> Result<(), WorkerDiagnostic>;

#[derive(Clone, Copy)]
pub struct TransportRunnerRegistration {
    pub transport: TransportKind,
    runner: Option<TransportRunner>,
    unavailable_diagnostic: Option<&'static str>,
}

impl TransportRunnerRegistration {
    pub const fn available(transport: TransportKind, runner: TransportRunner) -> Self {
        Self {
            transport,
            runner: Some(runner),
            unavailable_diagnostic: None,
        }
    }

    pub const fn unavailable(
        transport: TransportKind,
        unavailable_diagnostic: &'static str,
    ) -> Self {
        Self {
            transport,
            runner: None,
            unavailable_diagnostic: Some(unavailable_diagnostic),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransportRunnerCapability {
    pub transport: TransportKind,
    pub runner_available: bool,
    pub diagnostic: Option<String>,
}

#[derive(Clone)]
pub struct TransportRunnerRegistry {
    registrations: BTreeMap<TransportKind, TransportRunnerRegistration>,
}

impl TransportRunnerRegistry {
    pub fn new(registrations: impl IntoIterator<Item = TransportRunnerRegistration>) -> Self {
        Self {
            registrations: registrations
                .into_iter()
                .map(|registration| (registration.transport, registration))
                .collect(),
        }
    }

    pub fn compiled() -> Self {
        Self::new([
            TransportRunnerRegistration::available(TransportKind::Telegram, run_telegram_transport),
            TransportRunnerRegistration::available(TransportKind::Discord, run_discord_transport),
            TransportRunnerRegistration::available(TransportKind::Slack, run_slack_transport),
            TransportRunnerRegistration::available(TransportKind::Line, run_line_transport),
        ])
    }

    pub fn capabilities(&self) -> Vec<TransportRunnerCapability> {
        TransportKind::ALL
            .into_iter()
            .map(|transport| match self.registrations.get(&transport) {
                Some(registration) => TransportRunnerCapability {
                    transport,
                    runner_available: registration.runner.is_some(),
                    diagnostic: registration
                        .runner
                        .is_none()
                        .then(|| {
                            registration
                                .unavailable_diagnostic
                                .unwrap_or("The transport runner is unavailable in this build.")
                        })
                        .map(str::to_string),
                },
                None => TransportRunnerCapability {
                    transport,
                    runner_available: false,
                    diagnostic: Some(
                        "The transport is known but is not registered in this build.".to_string(),
                    ),
                },
            })
            .collect()
    }

    pub fn supported_transports(&self) -> Vec<TransportKind> {
        self.capabilities()
            .into_iter()
            .filter_map(|capability| capability.runner_available.then_some(capability.transport))
            .collect()
    }

    pub fn run(&self, context: &WorkerRunContext) -> Result<(), WorkerDiagnostic> {
        let registration = self.registrations.get(&context.transport).ok_or_else(|| {
            WorkerDiagnostic::new(
                "transport_runner_unregistered",
                format!(
                    "Rust {} transport is known but has no runner registration.",
                    context.transport
                ),
                EXIT_RUNTIME_UNAVAILABLE,
            )
            .with_detail("transport", context.transport.as_str())
        })?;
        let Some(runner) = registration.runner else {
            return Err(WorkerDiagnostic::new(
                "unsupported_transport_runtime",
                registration
                    .unavailable_diagnostic
                    .unwrap_or("The requested Rust transport runner is unavailable."),
                EXIT_RUNTIME_UNAVAILABLE,
            )
            .with_detail("transport", context.transport.as_str())
            .with_detail("worker", context.worker_name.clone())
            .with_detail("runner_state", "unavailable"));
        };
        runner(context)
    }
}

impl Default for TransportRunnerRegistry {
    fn default() -> Self {
        Self::compiled()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};

    use ait_agent_core::{
        agent_runtime_admission_plan_json, resolve_agent_worker_config, AgentEventLoopBackend,
        AgentWorkerConfigInput,
    };
    use ait_core::json_support::json;
    use tempfile::{tempdir, TempDir};

    use super::*;
    use crate::paths::ResolvedWorkerPaths;

    static RUNNER_CALLED: AtomicBool = AtomicBool::new(false);

    fn test_runner(_context: &WorkerRunContext) -> Result<(), WorkerDiagnostic> {
        RUNNER_CALLED.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn context() -> (TempDir, WorkerRunContext) {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".ait")).expect("ait dir");
        fs::write(
            temp.path().join(".ait/config.json"),
            r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
        )
        .expect("repo config");
        let config = resolve_agent_worker_config(AgentWorkerConfigInput {
            repo_root: temp.path().to_path_buf(),
            worker_key: "telegram/main".to_string(),
            worker: json!({
                "kind": "telegram",
                "name": "main",
                "token": "test-token"
            }),
            process_env: BTreeMap::new(),
        })
        .expect("worker config");
        let context = WorkerRunContext {
            paths: ResolvedWorkerPaths {
                repo_root: temp.path().to_path_buf(),
                manifest_path: temp.path().join(".ait/agent-workers.json"),
            },
            transport: TransportKind::Telegram,
            worker_key: "telegram/main".to_string(),
            worker_name: "main".to_string(),
            event_loop_backend: AgentEventLoopBackend::PortablePoll,
            shard_index: 0,
            runtime_admission_plan: agent_runtime_admission_plan_json(&json!({
                "worker_manifest": {
                    "version": 1,
                    "workers": {"telegram/main": {"kind": "telegram", "name": "main"}}
                },
                "backend": "portable_poll",
                "transport_runtime": "rust",
                "allow_python_fallback": false,
                "requested_worker_keys": ["telegram/main"],
            }))
            .expect("runtime admission"),
            config,
        };
        (temp, context)
    }

    #[test]
    fn compiled_registry_reports_all_native_product_runners_available() {
        let registry = TransportRunnerRegistry::compiled();

        assert_eq!(
            registry.supported_transports(),
            vec![
                TransportKind::Telegram,
                TransportKind::Discord,
                TransportKind::Slack,
                TransportKind::Line
            ]
        );
        assert_eq!(registry.capabilities().len(), TransportKind::ALL.len());
        assert_eq!(
            registry
                .capabilities()
                .iter()
                .filter(|capability| capability.runner_available)
                .map(|capability| capability.transport)
                .collect::<Vec<_>>(),
            vec![
                TransportKind::Telegram,
                TransportKind::Discord,
                TransportKind::Slack,
                TransportKind::Line
            ]
        );
    }

    #[test]
    fn available_registration_executes_only_its_rust_runner() {
        RUNNER_CALLED.store(false, Ordering::SeqCst);
        let registry = TransportRunnerRegistry::new([TransportRunnerRegistration::available(
            TransportKind::Telegram,
            test_runner,
        )]);

        let (_temp, context) = context();
        registry.run(&context).expect("runner result");

        assert!(RUNNER_CALLED.load(Ordering::SeqCst));
    }
}
