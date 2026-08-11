use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ait_core::json_support::{json, JsonValue};

use crate::cli::{plan_agent_cli_launch, AgentCliPlanInput, AgentWorkerLaunchState};
use crate::json_support::encode_to_value;
use crate::manifest::{AgentWorkerManifestDocument, AgentWorkerManifestStore};
use crate::supervisor::{
    acquire_worker_lifecycle_lock, agent_supervisor_public_worker_payload_json,
    plan_worker_supervisor_lifecycle, release_worker_lifecycle_lock, runtime_env_value,
    AgentWorkerLifecycleLockAcquireInput, AgentWorkerLifecycleLockReleaseInput,
    AgentWorkerLifecycleOperation, AgentWorkerLifecyclePlan, AgentWorkerLifecyclePlanInput,
    AgentWorkerLifecycleSpec, AgentWorkerLogTailInput, AgentWorkerProcessLogTailPort,
    AgentWorkerProcessPaths, AgentWorkerProcessStartPort, AgentWorkerProcessStatusInput,
    AgentWorkerProcessStatusPort, AgentWorkerProcessStopPort, AgentWorkerStartInput,
    AgentWorkerStartSpec, AgentWorkerStopInput, NativeAgentWorkerProcessPort,
};
use crate::transport::TransportKind;

mod capability;

pub use capability::{
    parse_capability_report, AgentWorkerCapabilityProbe, AgentWorkerCapabilityReport,
    NativeAgentWorkerCapabilityProbe, AGENT_WORKER_CAPABILITY_CONTRACT,
};

const DEFAULT_STOP_TIMEOUT_SECONDS: f64 = 10.0;
const DEFAULT_KILL_GRACE_SECONDS: f64 = 2.0;
const STOP_SUCCESS_STATES: &[&str] = &["already_stopped", "stopped", "killed"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSupervisorAction {
    Status,
    Start,
    Run,
    Stop,
    Restart,
}

impl AgentSupervisorAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Start => "start",
            Self::Run => "run",
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }
}

pub struct AgentManagementRuntime<
    P = NativeAgentWorkerProcessPort,
    C = NativeAgentWorkerCapabilityProbe,
> {
    repo_root: PathBuf,
    manifest_store: AgentWorkerManifestStore,
    worker_binary: String,
    parent_env: BTreeMap<String, String>,
    process_port: P,
    capability_probe: C,
}

impl AgentManagementRuntime<NativeAgentWorkerProcessPort, NativeAgentWorkerCapabilityProbe> {
    pub fn filesystem(
        repo_root: impl Into<PathBuf>,
        manifest_path: impl Into<PathBuf>,
        worker_binary: impl Into<String>,
        parent_env: BTreeMap<String, String>,
    ) -> Self {
        Self::with_ports(
            repo_root,
            manifest_path,
            worker_binary,
            parent_env,
            NativeAgentWorkerProcessPort,
            NativeAgentWorkerCapabilityProbe,
        )
    }
}

impl<P, C> AgentManagementRuntime<P, C>
where
    P: AgentWorkerProcessStatusPort
        + AgentWorkerProcessLogTailPort
        + AgentWorkerProcessStopPort
        + AgentWorkerProcessStartPort,
    C: AgentWorkerCapabilityProbe,
{
    #[allow(clippy::too_many_arguments)]
    pub fn with_ports(
        repo_root: impl Into<PathBuf>,
        manifest_path: impl Into<PathBuf>,
        worker_binary: impl Into<String>,
        parent_env: BTreeMap<String, String>,
        process_port: P,
        capability_probe: C,
    ) -> Self {
        Self {
            repo_root: repo_root.into(),
            manifest_store: AgentWorkerManifestStore::filesystem(manifest_path),
            worker_binary: worker_binary.into(),
            parent_env,
            process_port,
            capability_probe,
        }
    }

    pub fn add_worker(&self, worker: JsonValue) -> Result<JsonValue, String> {
        let mutation = self.manifest_store.upsert(worker, None)?;
        public_worker_payload(&mutation.worker, None, None, None, None)
    }

    pub fn list_workers(&self, transport: TransportKind) -> Result<Vec<JsonValue>, String> {
        let document = self.manifest_store.load();
        manifest_workers(&document.config, transport)
            .into_iter()
            .map(|worker| public_worker_payload(&worker, None, None, None, None))
            .collect()
    }

    pub fn status_workers(
        &self,
        transport: TransportKind,
        name: Option<&str>,
    ) -> Result<JsonValue, String> {
        let document = self.manifest_store.load();
        if let Some(name) = name {
            let worker = get_worker(&document.config, transport, name)?;
            return self.status_worker(&worker, &document);
        }
        let workers = manifest_workers(&document.config, transport)
            .iter()
            .map(|worker| self.status_worker(worker, &document))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(JsonValue::Array(workers))
    }

    pub fn foreground_worker_command(
        &self,
        transport: TransportKind,
        name: &str,
    ) -> Result<Vec<String>, String> {
        let document = self.manifest_store.load();
        let worker = get_worker(&document.config, transport, name)?;
        let launch = self.launch_decision(&document.config, &worker);
        if launch.state != "ready" {
            return Err(launch.diagnostic.unwrap_or_else(|| {
                format!(
                    "Rust {} worker runtime is unavailable; refusing Python fallback.",
                    transport.as_str()
                )
            }));
        }
        if launch.argv.is_empty() {
            return Err(
                "Rust launch contract returned an empty command; refusing Python fallback."
                    .to_string(),
            );
        }
        Ok(launch.argv)
    }

    pub fn start_worker(&self, transport: TransportKind, name: &str) -> Result<JsonValue, String> {
        let action = format!("{}/start/{name}", transport.as_str());
        self.with_transport_lock(transport, &action, || {
            let document = self.manifest_store.load();
            let worker = get_worker(&document.config, transport, name)?;
            let mut status = self.status_worker(&worker, &document)?;
            if bool_field(&status, "running") {
                insert_value(&mut status, "started", JsonValue::Bool(false))?;
                return Ok(status);
            }
            self.start_worker_unlocked(&worker, &document)
        })
    }

    pub fn stop_worker(&self, transport: TransportKind, name: &str) -> Result<JsonValue, String> {
        let action = format!("{}/stop/{name}", transport.as_str());
        self.with_transport_lock(transport, &action, || {
            let document = self.manifest_store.load();
            let worker = get_worker(&document.config, transport, name)?;
            self.stop_worker_unlocked(
                &worker,
                &document,
                &format!("cli_{}_stop", transport.as_str()),
            )
            .map(|(payload, _)| payload)
        })
    }

    pub fn restart_worker(
        &self,
        transport: TransportKind,
        name: &str,
    ) -> Result<JsonValue, String> {
        let action = format!("{}/restart/{name}", transport.as_str());
        self.with_transport_lock(transport, &action, || {
            let document = self.manifest_store.load();
            let worker = get_worker(&document.config, transport, name)?;
            self.restart_worker_unlocked(
                &worker,
                &document,
                &format!("cli_{}_restart", transport.as_str()),
            )
        })
    }

    pub fn worker_logs(
        &self,
        transport: TransportKind,
        name: &str,
        lines: usize,
    ) -> Result<JsonValue, String> {
        let document = self.manifest_store.load();
        let worker = get_worker(&document.config, transport, name)?;
        let plan = self.lifecycle_plan(&worker, AgentWorkerLifecycleOperation::Status)?;
        let log_tail = self
            .process_port
            .read_worker_log_tail(AgentWorkerLogTailInput {
                log_file: plan.paths.log_file.clone(),
                lines: Some(lines),
            });
        let mut status = self.status_worker(&worker, &document)?;
        insert_value(
            &mut status,
            "lines",
            JsonValue::Array(log_tail.lines.into_iter().map(JsonValue::String).collect()),
        )?;
        insert_value(
            &mut status,
            "log_exists",
            JsonValue::Bool(log_tail.log_exists),
        )?;
        insert_value(
            &mut status,
            "lines_requested",
            JsonValue::from(log_tail.lines_requested),
        )?;
        Ok(status)
    }

    pub fn remove_worker(&self, transport: TransportKind, name: &str) -> Result<JsonValue, String> {
        let removal = self.manifest_store.remove(transport.as_str(), name)?;
        Ok(json!({
            "removed": removal.removed,
            "kind": transport.as_str(),
            "name": name.trim(),
        }))
    }

    pub fn telegram_supervisor(
        &self,
        action: AgentSupervisorAction,
        interval_seconds: Option<f64>,
        cycle: Option<usize>,
    ) -> Result<JsonValue, String> {
        let execute = || {
            let document = self.manifest_store.load();
            let mut workers = Vec::new();
            for worker in manifest_workers(&document.config, TransportKind::Telegram) {
                let payload = match action {
                    AgentSupervisorAction::Status => self.status_worker(&worker, &document)?,
                    AgentSupervisorAction::Start | AgentSupervisorAction::Run => {
                        let mut status = self.status_worker(&worker, &document)?;
                        if bool_field(&status, "running") {
                            insert_value(&mut status, "started", JsonValue::Bool(false))?;
                            insert_value(
                                &mut status,
                                "start_state",
                                JsonValue::String("already_running".to_string()),
                            )?;
                            status
                        } else {
                            self.start_worker_unlocked(&worker, &document)?
                        }
                    }
                    AgentSupervisorAction::Stop => {
                        self.stop_worker_unlocked(&worker, &document, "supervisor_telegram_stop")?
                            .0
                    }
                    AgentSupervisorAction::Restart => self.restart_worker_unlocked(
                        &worker,
                        &document,
                        "supervisor_telegram_restart",
                    )?,
                };
                workers.push(payload);
            }
            let running_count = workers
                .iter()
                .filter(|worker| bool_field(worker, "running"))
                .count();
            let started_count = workers
                .iter()
                .filter(|worker| text_field(worker, "start_state") == Some("started"))
                .count();
            let mut payload = json!({
                "kind": "telegram-supervisor",
                "action": action.as_str(),
                "worker_count": workers.len(),
                "running_count": running_count,
                "workers": workers,
            });
            merge_config_diagnostics(&mut payload, &document)?;
            if matches!(
                action,
                AgentSupervisorAction::Start | AgentSupervisorAction::Run
            ) {
                insert_value(
                    &mut payload,
                    "started_count",
                    JsonValue::from(started_count),
                )?;
            }
            if let Some(cycle) = cycle {
                insert_value(&mut payload, "cycle", JsonValue::from(cycle))?;
            }
            if let Some(interval_seconds) = interval_seconds {
                insert_value(
                    &mut payload,
                    "interval_seconds",
                    JsonValue::from(interval_seconds),
                )?;
            }
            Ok(payload)
        };
        if action == AgentSupervisorAction::Status {
            execute()
        } else {
            self.with_transport_lock(
                TransportKind::Telegram,
                &format!("telegram/supervisor/{}", action.as_str()),
                execute,
            )
        }
    }

    fn status_worker(
        &self,
        worker: &JsonValue,
        document: &AgentWorkerManifestDocument,
    ) -> Result<JsonValue, String> {
        let plan = self.lifecycle_plan(worker, AgentWorkerLifecycleOperation::Status)?;
        let paths = process_paths(&plan);
        let status =
            self.process_port
                .inspect_worker_process_status(AgentWorkerProcessStatusInput {
                    paths: paths.clone(),
                });
        let env_bot_token = if plan.transport == TransportKind::Discord {
            runtime_env_value(
                Path::new(&paths.env_path),
                &["AIT_DISCORD_BOT_TOKEN", "DISCORD_BOT_TOKEN"],
            )
        } else {
            None
        };
        public_worker_payload(
            worker,
            Some(document),
            Some(&plan),
            Some(&status),
            env_bot_token.as_deref(),
        )
    }

    fn start_worker_unlocked(
        &self,
        worker: &JsonValue,
        document: &AgentWorkerManifestDocument,
    ) -> Result<JsonValue, String> {
        let launch = self.launch_decision(&document.config, worker);
        if launch.state != "ready" {
            let mut payload = self.status_worker(worker, document)?;
            insert_value(&mut payload, "started", JsonValue::Bool(false))?;
            insert_value(&mut payload, "start_state", JsonValue::String(launch.state))?;
            insert_value(&mut payload, "rust_launch_blocked", JsonValue::Bool(true))?;
            insert_value(
                &mut payload,
                "rust_launch_diagnostic",
                JsonValue::String(launch.diagnostic.unwrap_or_else(|| {
                    "Rust worker runtime is unavailable; refusing Python fallback.".to_string()
                })),
            )?;
            insert_value(
                &mut payload,
                "planned_command",
                json_string_array(&launch.argv),
            )?;
            return Ok(payload);
        }
        if launch.argv.is_empty() {
            let mut payload = self.status_worker(worker, document)?;
            insert_value(&mut payload, "started", JsonValue::Bool(false))?;
            insert_value(
                &mut payload,
                "start_state",
                JsonValue::String("rust_launch_command_missing".to_string()),
            )?;
            insert_value(&mut payload, "rust_launch_blocked", JsonValue::Bool(true))?;
            insert_value(
                &mut payload,
                "rust_launch_diagnostic",
                JsonValue::String(
                    "Rust launch contract returned an empty command; refusing Python fallback."
                        .to_string(),
                ),
            )?;
            insert_value(
                &mut payload,
                "planned_command",
                JsonValue::Array(Vec::new()),
            )?;
            return Ok(payload);
        }
        let plan = self.lifecycle_plan(worker, AgentWorkerLifecycleOperation::Start)?;
        let start_result = self
            .process_port
            .start_worker_process(AgentWorkerStartInput {
                repo_root: self.repo_root.to_string_lossy().into_owned(),
                paths: process_paths(&plan),
                worker: decode_start_spec(worker)?,
                argv: launch.argv.clone(),
                parent_env: self.parent_env.clone(),
            })?;
        let mut payload = self.status_worker(worker, document)?;
        let start_state = if start_result.start_state.is_empty() {
            if start_result.started {
                "started".to_string()
            } else {
                "rust_launch_blocked".to_string()
            }
        } else {
            start_result.start_state.clone()
        };
        let rust_launch_blocked = !start_result.started && start_state != "already_running";
        insert_value(
            &mut payload,
            "started",
            JsonValue::Bool(start_result.started),
        )?;
        insert_value(&mut payload, "start_state", JsonValue::String(start_state))?;
        insert_value(
            &mut payload,
            "command",
            json_string_array(if start_result.command.is_empty() {
                &launch.argv
            } else {
                &start_result.command
            }),
        )?;
        insert_value(
            &mut payload,
            "rust_launch_blocked",
            JsonValue::Bool(rust_launch_blocked),
        )?;
        if let Some(diagnostic) = start_result.diagnostic {
            insert_value(
                &mut payload,
                "rust_launch_diagnostic",
                JsonValue::String(diagnostic),
            )?;
        }
        if rust_launch_blocked {
            insert_value(
                &mut payload,
                "planned_command",
                json_string_array(&launch.argv),
            )?;
        }
        Ok(payload)
    }

    fn stop_worker_unlocked(
        &self,
        worker: &JsonValue,
        document: &AgentWorkerManifestDocument,
        reason: &str,
    ) -> Result<(JsonValue, String), String> {
        let plan = self.lifecycle_plan(worker, AgentWorkerLifecycleOperation::Stop)?;
        let stop_result = self
            .process_port
            .stop_worker_process(AgentWorkerStopInput {
                paths: process_paths(&plan),
                reason: Some(reason.to_string()),
                worker_name: worker
                    .get("name")
                    .and_then(JsonValue::as_str)
                    .map(ToString::to_string),
                stop_timeout_seconds: Some(DEFAULT_STOP_TIMEOUT_SECONDS),
                kill_grace_seconds: Some(DEFAULT_KILL_GRACE_SECONDS),
            })?;
        let stop_state = stop_result.stop_state;
        let mut payload = self.status_worker(worker, document)?;
        insert_value(
            &mut payload,
            "stopped",
            JsonValue::Bool(stop_result.stopped),
        )?;
        insert_value(
            &mut payload,
            "stop_state",
            JsonValue::String(stop_state.clone()),
        )?;
        Ok((payload, stop_state))
    }

    fn restart_worker_unlocked(
        &self,
        worker: &JsonValue,
        document: &AgentWorkerManifestDocument,
        reason: &str,
    ) -> Result<JsonValue, String> {
        let status = self.status_worker(worker, document)?;
        let (stopped, stop_state) = if bool_field(&status, "running") {
            let (mut stopped_payload, stop_state) =
                self.stop_worker_unlocked(worker, document, reason)?;
            if !stop_success_state(&stop_state) {
                insert_value(&mut stopped_payload, "started", JsonValue::Bool(false))?;
                insert_value(&mut stopped_payload, "restarted", JsonValue::Bool(false))?;
                insert_value(
                    &mut stopped_payload,
                    "restart_blocked",
                    JsonValue::Bool(true),
                )?;
                return Ok(stopped_payload);
            }
            (bool_field(&stopped_payload, "stopped"), stop_state)
        } else {
            (false, "not_running".to_string())
        };
        let mut payload = self.start_worker_unlocked(worker, document)?;
        let start_success = bool_field(&payload, "started");
        insert_value(&mut payload, "stopped", JsonValue::Bool(stopped))?;
        insert_value(
            &mut payload,
            "stop_state",
            JsonValue::String(stop_state.clone()),
        )?;
        insert_value(
            &mut payload,
            "restarted",
            JsonValue::Bool(start_success && stop_success_state(&stop_state)),
        )?;
        insert_value(
            &mut payload,
            "restart_blocked",
            JsonValue::Bool(!start_success),
        )?;
        Ok(payload)
    }

    fn lifecycle_plan(
        &self,
        worker: &JsonValue,
        operation: AgentWorkerLifecycleOperation,
    ) -> Result<AgentWorkerLifecyclePlan, String> {
        let spec: AgentWorkerLifecycleSpec = crate::json_support::decode_from_value(
            worker,
            "invalid ait-agent worker lifecycle spec",
        )?;
        plan_worker_supervisor_lifecycle(AgentWorkerLifecyclePlanInput {
            repo_root: self.repo_root.to_string_lossy().into_owned(),
            operation,
            worker: spec,
            runtime_root: None,
            stop_timeout_seconds: None,
            kill_grace_seconds: None,
        })
    }

    fn launch_decision(&self, config: &JsonValue, worker: &JsonValue) -> LaunchDecision {
        let capability = self.capability_probe.probe(&self.worker_binary);
        let (supported_transports, probe_error) = match capability {
            Ok(report) => (report.supported_transports, None),
            Err(err) => (Vec::new(), Some(err)),
        };
        let plan = plan_agent_cli_launch(AgentCliPlanInput {
            worker_manifest: config.clone(),
            expected_concurrent_workers: None,
            rust_worker_binary: Some(self.worker_binary.clone()),
            available_rust_transports: supported_transports,
        });
        let transport = worker
            .get("kind")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        let name = worker
            .get("name")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        let worker_key = format!("{transport}/{name}");
        let row = plan
            .workers
            .into_iter()
            .find(|candidate| candidate.worker_key == worker_key);
        if probe_error.is_some() {
            return LaunchDecision {
                argv: row.map(|candidate| candidate.argv).unwrap_or_default(),
                state: "rust_worker_capability_probe_failed".to_string(),
                diagnostic: Some(
                    "Rust worker capability probe failed; rebuild or reinstall the native \
                     ait-agent-worker artifact. Refusing Python fallback."
                        .to_string(),
                ),
            };
        }
        match row {
            Some(row) => LaunchDecision {
                argv: row.argv,
                state: if row.launch_state == AgentWorkerLaunchState::Ready {
                    "ready".to_string()
                } else {
                    "missing_rust_transport_runtime".to_string()
                },
                diagnostic: row.diagnostic,
            },
            None => LaunchDecision {
                argv: Vec::new(),
                state: "rust_launch_contract_unavailable".to_string(),
                diagnostic: Some(format!(
                    "Rust launch plan did not include worker {worker_key}; refusing Python fallback."
                )),
            },
        }
    }

    fn with_transport_lock<T, F>(
        &self,
        transport: TransportKind,
        action: &str,
        operation: F,
    ) -> Result<T, String>
    where
        F: FnOnce() -> Result<T, String>,
    {
        let lock = acquire_worker_lifecycle_lock(AgentWorkerLifecycleLockAcquireInput {
            repo_root: self.repo_root.to_string_lossy().into_owned(),
            transport,
            action: action.to_string(),
            runtime_root: None,
        })?;
        let result = operation();
        let release = release_worker_lifecycle_lock(AgentWorkerLifecycleLockReleaseInput {
            lifecycle_lock_path: lock.lifecycle_lock_path,
            lock_token: lock.lock_token,
        });
        match (result, release) {
            (Ok(value), Ok(_)) => Ok(value),
            (Err(error), Ok(_)) => Err(error),
            (Ok(_), Err(release_error)) => Err(release_error),
            (Err(error), Err(release_error)) => Err(format!(
                "{error}; additionally failed to release lifecycle lock: {release_error}"
            )),
        }
    }
}

#[derive(Debug)]
struct LaunchDecision {
    argv: Vec<String>,
    state: String,
    diagnostic: Option<String>,
}

fn manifest_workers(config: &JsonValue, transport: TransportKind) -> Vec<JsonValue> {
    config
        .get("workers")
        .and_then(JsonValue::as_object)
        .map(|workers| {
            workers
                .iter()
                .filter(|(key, worker)| {
                    key.starts_with(&format!("{}/", transport.as_str())) && worker.is_object()
                })
                .map(|(_, worker)| worker.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn get_worker(
    config: &JsonValue,
    transport: TransportKind,
    name: &str,
) -> Result<JsonValue, String> {
    let name = normalize_worker_name(name)?;
    let key = format!("{}/{name}", transport.as_str());
    config
        .get("workers")
        .and_then(JsonValue::as_object)
        .and_then(|workers| workers.get(&key))
        .filter(|worker| worker.is_object())
        .cloned()
        .ok_or_else(|| format!("Unknown {} worker: {name}", transport.as_str()))
}

fn normalize_worker_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Worker name must not be empty.".to_string());
    }
    if name.contains('/') {
        return Err("Worker name must not contain '/'.".to_string());
    }
    Ok(name.to_string())
}

fn process_paths(plan: &AgentWorkerLifecyclePlan) -> AgentWorkerProcessPaths {
    AgentWorkerProcessPaths {
        pid_file: plan.paths.pid_file.clone(),
        log_file: plan.paths.log_file.clone(),
        sync_state_path: plan.paths.sync_state_path.clone(),
        env_path: plan.paths.env_path.clone(),
        termination_context_path: plan.paths.termination_context_path.clone(),
    }
}

fn decode_start_spec(worker: &JsonValue) -> Result<AgentWorkerStartSpec, String> {
    crate::json_support::decode_from_value(worker, "invalid ait-agent worker start spec")
}

fn public_worker_payload(
    worker: &JsonValue,
    document: Option<&AgentWorkerManifestDocument>,
    plan: Option<&AgentWorkerLifecyclePlan>,
    status: Option<&crate::supervisor::AgentWorkerProcessStatus>,
    env_bot_token: Option<&str>,
) -> Result<JsonValue, String> {
    let mut request = json!({"worker": worker});
    if let Some(document) = document {
        request["config"] = document.config.clone();
        request["config_issues"] = json_string_array(&document.issues);
    }
    if let Some(plan) = plan {
        request["paths"] = encode_to_value(
            &plan.paths,
            "failed to serialize ait-agent supervisor paths",
        )?;
    }
    if let Some(status) = status {
        request["process_status"] = encode_to_value(
            status,
            "failed to serialize ait-agent supervisor process status",
        )?;
    }
    if let Some(env_bot_token) = env_bot_token {
        request["env_bot_token"] = JsonValue::String(env_bot_token.to_string());
    }
    agent_supervisor_public_worker_payload_json(&request)
}

fn merge_config_diagnostics(
    payload: &mut JsonValue,
    document: &AgentWorkerManifestDocument,
) -> Result<(), String> {
    let diagnostics = public_worker_payload(
        &json!({"kind": "supervisor", "name": "config"}),
        Some(document),
        None,
        None,
        None,
    )?;
    for key in ["config_version", "config_valid", "config_issues"] {
        if let Some(value) = diagnostics.get(key) {
            insert_value(payload, key, value.clone())?;
        }
    }
    Ok(())
}

fn insert_value(payload: &mut JsonValue, key: &str, value: JsonValue) -> Result<(), String> {
    payload
        .as_object_mut()
        .ok_or_else(|| "ait-agent management payload must be an object".to_string())?
        .insert(key.to_string(), value);
    Ok(())
}

fn bool_field(payload: &JsonValue, key: &str) -> bool {
    payload
        .get(key)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn text_field<'a>(payload: &'a JsonValue, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(JsonValue::as_str)
}

fn json_string_array(values: &[impl AsRef<str>]) -> JsonValue {
    JsonValue::Array(
        values
            .iter()
            .map(|value| JsonValue::String(value.as_ref().to_string()))
            .collect(),
    )
}

fn stop_success_state(state: &str) -> bool {
    STOP_SUCCESS_STATES.contains(&state)
}

#[cfg(test)]
mod tests;
