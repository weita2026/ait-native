use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use ait_agent_core::{
    consume_worker_termination_context_json, AgentEvent, AgentEventLoopDriver,
    AgentEventLoopPollPort, AgentEventLoopReadWriteRegistrationPort,
    AgentEventLoopReadableRegistrationPort, AgentEventLoopUnregistrationPort, NativeSocket,
};
use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions, JsonValue};
use serde::Serialize;

use crate::diagnostic::{WorkerDiagnostic, EXIT_RUNTIME_UNAVAILABLE};
use crate::WorkerRunContext;

pub const WORKER_HOST_HEALTH_CONTRACT: &str = "ait.agent.worker.host.health.v1";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(10);
#[cfg(windows)]
const WORKER_SHUTDOWN_INTERRUPT: i32 = 2;
const WORKER_SHUTDOWN_TERMINATE: i32 = 15;

static PROCESS_SHUTDOWN_SIGNAL: AtomicI32 = AtomicI32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerHostHealthState {
    Starting,
    Ready,
    Stopping,
    Stopped,
    Failed,
}

impl WorkerHostHealthState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerHostHealthSnapshot {
    pub contract: &'static str,
    pub kind: &'static str,
    pub state: WorkerHostHealthState,
    pub worker_key: String,
    pub transport: String,
    pub event_loop_backend: String,
    pub shard_index: usize,
    pub inflight_work_count: usize,
    pub runtime_state: Option<String>,
    pub reconnect_attempt: Option<usize>,
    pub runtime_diagnostic_code: Option<String>,
    pub shutdown_signal: Option<i32>,
    pub termination_context_status: Option<String>,
    pub termination_context_suffix: Option<String>,
    pub failure_code: Option<String>,
    pub python_worker_execution_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerHostSettings {
    pub poll_interval: Duration,
    pub shutdown_grace: Duration,
}

impl Default for WorkerHostSettings {
    fn default() -> Self {
        Self {
            poll_interval: DEFAULT_POLL_INTERVAL,
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
        }
    }
}

pub trait WorkerHostRuntime {
    fn start(
        &mut self,
        context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic>;
    fn tick(
        &mut self,
        context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
        events: &[AgentEvent],
    ) -> Result<(), WorkerDiagnostic>;
    fn request_shutdown(
        &mut self,
        context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
        signal: i32,
    ) -> Result<(), WorkerDiagnostic>;
    fn inflight_work_count(&self) -> usize;
    fn finish_shutdown(
        &mut self,
        context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic>;
    fn force_shutdown(
        &mut self,
        context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic>;
    fn runtime_health_generation(&self) -> u64 {
        0
    }
    fn runtime_health_state(&self) -> Option<&str> {
        None
    }
    fn runtime_reconnect_attempt(&self) -> Option<usize> {
        None
    }
    fn runtime_diagnostic_code(&self) -> Option<&str> {
        None
    }
}

pub trait WorkerShutdownSource {
    fn shutdown_signal(&self) -> Option<i32>;
}

pub trait WorkerHostEventLoop {
    fn register_readable(&mut self, token: u64, fd: NativeSocket) -> Result<(), WorkerDiagnostic>;
    fn register_read_write(&mut self, token: u64, fd: NativeSocket)
        -> Result<(), WorkerDiagnostic>;
    fn unregister(&mut self, token: u64) -> Result<(), WorkerDiagnostic>;
    fn wait(&mut self, timeout: Duration) -> Result<Vec<AgentEvent>, WorkerDiagnostic>;
}

pub trait WorkerHostClock {
    fn now(&self) -> Duration;
}

#[derive(Debug)]
pub struct SystemWorkerHostClock {
    started_at: Instant,
}

impl SystemWorkerHostClock {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }
}

impl Default for SystemWorkerHostClock {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerHostClock for SystemWorkerHostClock {
    fn now(&self) -> Duration {
        self.started_at.elapsed()
    }
}

pub struct AgentEventLoopHostWait {
    event_loop: AgentEventLoopDriver,
}

impl AgentEventLoopHostWait {
    pub fn new(context: &WorkerRunContext) -> Result<Self, WorkerDiagnostic> {
        AgentEventLoopDriver::new_for_backend(context.event_loop_backend)
            .map(|event_loop| Self { event_loop })
            .map_err(|error| {
                WorkerDiagnostic::new(
                    "worker_host_event_loop_unavailable",
                    format!("Cannot start the Rust worker-host event loop: {error}"),
                    EXIT_RUNTIME_UNAVAILABLE,
                )
                .with_detail("event_loop_backend", context.event_loop_backend.label())
            })
    }
}

impl WorkerHostEventLoop for AgentEventLoopHostWait {
    fn register_readable(&mut self, token: u64, fd: NativeSocket) -> Result<(), WorkerDiagnostic> {
        self.event_loop
            .register_readable(token, fd)
            .map_err(|error| event_loop_registration_error("register readable", token, error))
    }

    fn register_read_write(
        &mut self,
        token: u64,
        fd: NativeSocket,
    ) -> Result<(), WorkerDiagnostic> {
        self.event_loop
            .register_read_write(token, fd)
            .map_err(|error| event_loop_registration_error("register read/write", token, error))
    }

    fn unregister(&mut self, token: u64) -> Result<(), WorkerDiagnostic> {
        self.event_loop
            .unregister(token)
            .map_err(|error| event_loop_registration_error("unregister", token, error))
    }

    fn wait(&mut self, timeout: Duration) -> Result<Vec<AgentEvent>, WorkerDiagnostic> {
        self.event_loop.poll(timeout).map_err(|error| {
            WorkerDiagnostic::new(
                "worker_host_event_loop_wait_failed",
                format!("Rust worker-host event-loop wait failed: {error}"),
                EXIT_RUNTIME_UNAVAILABLE,
            )
        })
    }
}

fn event_loop_registration_error(action: &str, token: u64, error: io::Error) -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "worker_host_event_loop_registration_failed",
        format!("Rust worker-host failed to {action} token {token}: {error}"),
        EXIT_RUNTIME_UNAVAILABLE,
    )
    .with_detail("token", token)
    .with_detail("action", action)
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessShutdownSource;

impl ProcessShutdownSource {
    pub fn install() -> Result<Self, WorkerDiagnostic> {
        PROCESS_SHUTDOWN_SIGNAL.store(0, Ordering::SeqCst);
        install_process_signal_handlers()?;
        Ok(Self)
    }
}

impl WorkerShutdownSource for ProcessShutdownSource {
    fn shutdown_signal(&self) -> Option<i32> {
        let signal = PROCESS_SHUTDOWN_SIGNAL.load(Ordering::SeqCst);
        (signal > 0).then_some(signal)
    }
}

struct ProcessAndContextShutdownSource {
    process: ProcessShutdownSource,
    termination_context_path: PathBuf,
}

impl ProcessAndContextShutdownSource {
    fn new(process: ProcessShutdownSource, context: &WorkerRunContext) -> Self {
        Self {
            process,
            termination_context_path: PathBuf::from(
                &context.config.shared().paths.termination_context_path,
            ),
        }
    }
}

impl WorkerShutdownSource for ProcessAndContextShutdownSource {
    fn shutdown_signal(&self) -> Option<i32> {
        self.process.shutdown_signal().or_else(|| {
            termination_context_requests_shutdown(
                &self.termination_context_path,
                i64::from(std::process::id()),
            )
            .then_some(WORKER_SHUTDOWN_TERMINATE)
        })
    }
}

pub fn run_worker_host<R>(
    context: &WorkerRunContext,
    runtime: &mut R,
) -> Result<(), WorkerDiagnostic>
where
    R: WorkerHostRuntime,
{
    let signals = ProcessShutdownSource::install()?;
    let shutdown = ProcessAndContextShutdownSource::new(signals, context);
    let mut wait = AgentEventLoopHostWait::new(context)?;
    let clock = SystemWorkerHostClock::new();
    let mut stderr = io::stderr().lock();
    run_worker_host_with_ports(
        context,
        runtime,
        &shutdown,
        &mut wait,
        &clock,
        WorkerHostSettings::default(),
        &mut stderr,
    )
}

fn termination_context_requests_shutdown(path: &Path, expected_pid: i64) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(payload) = JsonCodec::parse_value(&text, "worker termination context") else {
        return false;
    };
    payload.get("pid").and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
    }) == Some(expected_pid)
}

#[allow(clippy::too_many_arguments)]
pub fn run_worker_host_with_ports<R, S, E, C, O>(
    context: &WorkerRunContext,
    runtime: &mut R,
    shutdown_source: &S,
    event_loop: &mut E,
    clock: &C,
    settings: WorkerHostSettings,
    output: &mut O,
) -> Result<(), WorkerDiagnostic>
where
    R: WorkerHostRuntime,
    S: WorkerShutdownSource + ?Sized,
    E: WorkerHostEventLoop,
    C: WorkerHostClock + ?Sized,
    O: Write + ?Sized,
{
    emit_health(
        output,
        health_snapshot(
            context,
            runtime,
            WorkerHostHealthState::Starting,
            None,
            None,
            None,
        ),
    )?;
    if let Err(error) = runtime.start(context, event_loop) {
        let _ = runtime.force_shutdown(context, event_loop);
        emit_failure(output, context, runtime, None, None, &error)?;
        return Err(error);
    }
    emit_health(
        output,
        health_snapshot(
            context,
            runtime,
            WorkerHostHealthState::Ready,
            None,
            None,
            None,
        ),
    )?;
    let mut runtime_health_generation = runtime.runtime_health_generation();

    let poll_interval = settings.poll_interval.max(Duration::from_millis(1));
    let mut shutdown: Option<ShutdownState> = None;
    let mut events = Vec::new();
    loop {
        if shutdown.is_none() {
            if let Some(signal) = shutdown_source.shutdown_signal() {
                let termination = consume_termination_context(context, signal);
                if let Err(error) = runtime.request_shutdown(context, event_loop, signal) {
                    let state = ShutdownState {
                        signal,
                        started_at: clock.now(),
                        termination,
                    };
                    emit_failure(output, context, runtime, Some(&state), None, &error)?;
                    let _ = runtime.force_shutdown(context, event_loop);
                    return Err(error);
                }
                let state = ShutdownState {
                    signal,
                    started_at: clock.now(),
                    termination,
                };
                emit_health(
                    output,
                    health_snapshot(
                        context,
                        runtime,
                        WorkerHostHealthState::Stopping,
                        Some(&state),
                        None,
                        None,
                    ),
                )?;
                runtime_health_generation = runtime.runtime_health_generation();
                shutdown = Some(state);
            }
        }

        if let Some(state) = shutdown.as_ref() {
            if runtime.inflight_work_count() == 0 {
                if let Err(error) = runtime.finish_shutdown(context, event_loop) {
                    emit_failure(output, context, runtime, Some(state), None, &error)?;
                    let _ = runtime.force_shutdown(context, event_loop);
                    return Err(error);
                }
                emit_health(
                    output,
                    health_snapshot(
                        context,
                        runtime,
                        WorkerHostHealthState::Stopped,
                        Some(state),
                        None,
                        None,
                    ),
                )?;
                return Ok(());
            }
            if clock.now().saturating_sub(state.started_at) >= settings.shutdown_grace {
                let force_error = runtime.force_shutdown(context, event_loop).err();
                let error = WorkerDiagnostic::new(
                    "worker_host_graceful_shutdown_timeout",
                    "Rust worker-host graceful shutdown exceeded its deadline.",
                    EXIT_RUNTIME_UNAVAILABLE,
                )
                .with_detail("worker_key", context.worker_key.clone())
                .with_detail("shutdown_signal", state.signal as i64)
                .with_detail("inflight_work_count", runtime.inflight_work_count());
                emit_failure(
                    output,
                    context,
                    runtime,
                    Some(state),
                    force_error.as_ref().map(|value| value.code),
                    &error,
                )?;
                return Err(error);
            }
        }

        if let Err(error) = runtime.tick(context, event_loop, &events) {
            let _ = runtime.force_shutdown(context, event_loop);
            emit_failure(output, context, runtime, shutdown.as_ref(), None, &error)?;
            return Err(error);
        }
        let next_health_generation = runtime.runtime_health_generation();
        if next_health_generation != runtime_health_generation {
            emit_health(
                output,
                health_snapshot(
                    context,
                    runtime,
                    if shutdown.is_some() {
                        WorkerHostHealthState::Stopping
                    } else {
                        WorkerHostHealthState::Ready
                    },
                    shutdown.as_ref(),
                    None,
                    None,
                ),
            )?;
            runtime_health_generation = next_health_generation;
        }

        let timeout = shutdown
            .as_ref()
            .map(|state| {
                settings
                    .shutdown_grace
                    .saturating_sub(clock.now().saturating_sub(state.started_at))
                    .min(poll_interval)
            })
            .unwrap_or(poll_interval)
            .max(Duration::from_millis(1));
        match event_loop.wait(timeout) {
            Ok(next_events) => events = next_events,
            Err(error) => {
                let _ = runtime.force_shutdown(context, event_loop);
                emit_failure(output, context, runtime, shutdown.as_ref(), None, &error)?;
                return Err(error);
            }
        }
    }
}

#[derive(Debug, Clone)]
struct TerminationContextSummary {
    status: String,
    suffix: String,
}

#[derive(Debug, Clone)]
struct ShutdownState {
    signal: i32,
    started_at: Duration,
    termination: TerminationContextSummary,
}

fn consume_termination_context(
    context: &WorkerRunContext,
    signal: i32,
) -> TerminationContextSummary {
    let request = json!({
        "path": context.config.shared().paths.termination_context_path,
        "expected_pid": std::process::id(),
        "signal": signal,
        "include_issuer_details": false,
    });
    match consume_worker_termination_context_json(&request) {
        Ok(result) => TerminationContextSummary {
            status: result
                .get("status")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
                .to_string(),
            suffix: result
                .get("suffix")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        Err(_) => TerminationContextSummary {
            status: "consume_failed".to_string(),
            suffix: String::new(),
        },
    }
}

fn health_snapshot<R>(
    context: &WorkerRunContext,
    runtime: &R,
    state: WorkerHostHealthState,
    shutdown: Option<&ShutdownState>,
    failure_code: Option<&str>,
    termination_override: Option<&TerminationContextSummary>,
) -> WorkerHostHealthSnapshot
where
    R: WorkerHostRuntime + ?Sized,
{
    let termination = termination_override.or_else(|| shutdown.map(|value| &value.termination));
    WorkerHostHealthSnapshot {
        contract: WORKER_HOST_HEALTH_CONTRACT,
        kind: "worker_host_health",
        state,
        worker_key: context.worker_key.clone(),
        transport: context.transport.as_str().to_string(),
        event_loop_backend: context.event_loop_backend.label().to_string(),
        shard_index: context.shard_index,
        inflight_work_count: runtime.inflight_work_count(),
        runtime_state: runtime.runtime_health_state().map(str::to_string),
        reconnect_attempt: runtime.runtime_reconnect_attempt(),
        runtime_diagnostic_code: runtime.runtime_diagnostic_code().map(str::to_string),
        shutdown_signal: shutdown.map(|value| value.signal),
        termination_context_status: termination.map(|value| value.status.clone()),
        termination_context_suffix: termination
            .map(|value| value.suffix.clone())
            .filter(|value| !value.is_empty()),
        failure_code: failure_code.map(str::to_string),
        python_worker_execution_allowed: false,
    }
}

fn emit_failure<R, O>(
    output: &mut O,
    context: &WorkerRunContext,
    runtime: &R,
    shutdown: Option<&ShutdownState>,
    cleanup_failure_code: Option<&str>,
    error: &WorkerDiagnostic,
) -> Result<(), WorkerDiagnostic>
where
    R: WorkerHostRuntime + ?Sized,
    O: Write + ?Sized,
{
    let failure_code = cleanup_failure_code.unwrap_or(error.code);
    emit_health(
        output,
        health_snapshot(
            context,
            runtime,
            WorkerHostHealthState::Failed,
            shutdown,
            Some(failure_code),
            None,
        ),
    )
}

fn emit_health<O>(
    output: &mut O,
    snapshot: WorkerHostHealthSnapshot,
) -> Result<(), WorkerDiagnostic>
where
    O: Write + ?Sized,
{
    let encoded = JsonCodec::encode_serializable_with_error_prefix(
        &snapshot,
        JsonEncodeOptions::compact().with_trailing_newline(),
        "Failed to serialize Rust worker-host health",
    )
    .map_err(|error| {
        WorkerDiagnostic::new(
            "worker_host_health_serialization_failed",
            error.to_string(),
            EXIT_RUNTIME_UNAVAILABLE,
        )
    })?;
    output.write_all(encoded.as_bytes()).map_err(|error| {
        WorkerDiagnostic::new(
            "worker_host_health_output_failed",
            format!("Failed to write Rust worker-host health: {error}"),
            EXIT_RUNTIME_UNAVAILABLE,
        )
    })?;
    output.flush().map_err(|error| {
        WorkerDiagnostic::new(
            "worker_host_health_output_failed",
            format!("Failed to flush Rust worker-host health: {error}"),
            EXIT_RUNTIME_UNAVAILABLE,
        )
    })
}

#[cfg(unix)]
extern "C" fn process_shutdown_signal_handler(signal: libc::c_int) {
    let _ = PROCESS_SHUTDOWN_SIGNAL.compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst);
}

#[cfg(unix)]
fn install_process_signal_handlers() -> Result<(), WorkerDiagnostic> {
    for signal in [libc::SIGTERM, libc::SIGINT] {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = process_shutdown_signal_handler as *const () as usize;
        action.sa_flags = 0;
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
        }
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
            return Err(WorkerDiagnostic::new(
                "worker_host_signal_install_failed",
                format!(
                    "Failed to install Rust worker-host signal handler for signal {signal}: {}",
                    io::Error::last_os_error()
                ),
                EXIT_RUNTIME_UNAVAILABLE,
            )
            .with_detail("signal", signal as i64));
        }
    }
    Ok(())
}

#[cfg(windows)]
unsafe extern "system" fn process_shutdown_console_handler(control_type: u32) -> i32 {
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    };

    let signal = match control_type {
        CTRL_C_EVENT => WORKER_SHUTDOWN_INTERRUPT,
        CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            WORKER_SHUTDOWN_TERMINATE
        }
        _ => return 0,
    };
    let _ = PROCESS_SHUTDOWN_SIGNAL.compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst);
    1
}

#[cfg(windows)]
fn install_process_signal_handlers() -> Result<(), WorkerDiagnostic> {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

    if unsafe { SetConsoleCtrlHandler(Some(process_shutdown_console_handler), 1) } == 0 {
        return Err(WorkerDiagnostic::new(
            "worker_host_signal_install_failed",
            format!(
                "Failed to install the Windows worker-host console control handler: {}",
                io::Error::last_os_error()
            ),
            EXIT_RUNTIME_UNAVAILABLE,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::rc::Rc;

    use ait_agent_core::{
        agent_runtime_admission_plan_json, resolve_agent_worker_config, AgentEventLoopBackend,
        AgentWorkerConfigInput, AgentWorkerRuntimeConfig, TransportKind,
    };
    use ait_core::json_support::{json, JsonCodec};
    use tempfile::{tempdir, TempDir};

    use super::*;
    use crate::paths::ResolvedWorkerPaths;

    struct FakeClock {
        now: Rc<Cell<Duration>>,
    }

    impl WorkerHostClock for FakeClock {
        fn now(&self) -> Duration {
            self.now.get()
        }
    }

    struct FakeWait {
        now: Rc<Cell<Duration>>,
        calls: usize,
        fail_on_call: Option<usize>,
    }

    impl WorkerHostEventLoop for FakeWait {
        fn register_readable(
            &mut self,
            _token: u64,
            _fd: NativeSocket,
        ) -> Result<(), WorkerDiagnostic> {
            Ok(())
        }

        fn register_read_write(
            &mut self,
            _token: u64,
            _fd: NativeSocket,
        ) -> Result<(), WorkerDiagnostic> {
            Ok(())
        }

        fn unregister(&mut self, _token: u64) -> Result<(), WorkerDiagnostic> {
            Ok(())
        }

        fn wait(&mut self, timeout: Duration) -> Result<Vec<AgentEvent>, WorkerDiagnostic> {
            self.calls += 1;
            if self.fail_on_call == Some(self.calls) {
                return Err(WorkerDiagnostic::new(
                    "fake_wait_failed",
                    "fake wait failed",
                    EXIT_RUNTIME_UNAVAILABLE,
                ));
            }
            self.now.set(self.now.get() + timeout);
            Ok(Vec::new())
        }
    }

    struct SequenceShutdownSource {
        calls: Cell<usize>,
        signal_on_call: usize,
        signal: i32,
    }

    impl WorkerShutdownSource for SequenceShutdownSource {
        fn shutdown_signal(&self) -> Option<i32> {
            let calls = self.calls.get() + 1;
            self.calls.set(calls);
            (calls >= self.signal_on_call).then_some(self.signal)
        }
    }

    #[derive(Default)]
    struct FakeRuntime {
        started: bool,
        stopping: bool,
        finished: bool,
        forced: bool,
        inflight: usize,
        drain_per_tick: usize,
        fail_start: bool,
        fail_tick: bool,
    }

    impl WorkerHostRuntime for FakeRuntime {
        fn start(
            &mut self,
            _context: &WorkerRunContext,
            _event_loop: &mut dyn WorkerHostEventLoop,
        ) -> Result<(), WorkerDiagnostic> {
            if self.fail_start {
                return Err(fake_error("fake_start_failed"));
            }
            self.started = true;
            Ok(())
        }

        fn tick(
            &mut self,
            _context: &WorkerRunContext,
            _event_loop: &mut dyn WorkerHostEventLoop,
            _events: &[AgentEvent],
        ) -> Result<(), WorkerDiagnostic> {
            if self.fail_tick {
                return Err(fake_error("fake_tick_failed"));
            }
            if self.stopping {
                self.inflight = self.inflight.saturating_sub(self.drain_per_tick);
            }
            Ok(())
        }

        fn request_shutdown(
            &mut self,
            _context: &WorkerRunContext,
            _event_loop: &mut dyn WorkerHostEventLoop,
            _signal: i32,
        ) -> Result<(), WorkerDiagnostic> {
            self.stopping = true;
            Ok(())
        }

        fn inflight_work_count(&self) -> usize {
            self.inflight
        }

        fn finish_shutdown(
            &mut self,
            _context: &WorkerRunContext,
            _event_loop: &mut dyn WorkerHostEventLoop,
        ) -> Result<(), WorkerDiagnostic> {
            self.finished = true;
            Ok(())
        }

        fn force_shutdown(
            &mut self,
            _context: &WorkerRunContext,
            _event_loop: &mut dyn WorkerHostEventLoop,
        ) -> Result<(), WorkerDiagnostic> {
            self.forced = true;
            Ok(())
        }
    }

    fn fake_error(code: &'static str) -> WorkerDiagnostic {
        WorkerDiagnostic::new(code, code, EXIT_RUNTIME_UNAVAILABLE)
    }

    fn fixture_context() -> (TempDir, WorkerRunContext) {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
        let config = resolve_agent_worker_config(AgentWorkerConfigInput {
            repo_root: temp.path().to_path_buf(),
            worker_key: "telegram/main".to_string(),
            worker: json!({
                "kind": "telegram",
                "name": "main",
                "token": "must-not-leak"
            }),
            process_env: BTreeMap::new(),
        })
        .expect("worker config");
        assert!(matches!(config, AgentWorkerRuntimeConfig::Telegram(_)));
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

    fn run_fake_host(
        context: &WorkerRunContext,
        runtime: &mut FakeRuntime,
        signal_on_call: usize,
        wait_failure: Option<usize>,
        grace: Duration,
    ) -> (Result<(), WorkerDiagnostic>, FakeWait, String) {
        let now = Rc::new(Cell::new(Duration::ZERO));
        let clock = FakeClock { now: now.clone() };
        let signals = SequenceShutdownSource {
            calls: Cell::new(0),
            signal_on_call,
            signal: libc::SIGTERM,
        };
        let mut wait = FakeWait {
            now,
            calls: 0,
            fail_on_call: wait_failure,
        };
        let mut output = Vec::new();
        let result = run_worker_host_with_ports(
            context,
            runtime,
            &signals,
            &mut wait,
            &clock,
            WorkerHostSettings {
                poll_interval: Duration::from_millis(5),
                shutdown_grace: grace,
            },
            &mut output,
        );
        (result, wait, String::from_utf8(output).expect("UTF-8 logs"))
    }

    fn states(logs: &str) -> Vec<String> {
        logs.lines()
            .map(|line| JsonCodec::parse_value(line, "worker-host health").expect("health JSON"))
            .map(|value| value["state"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn host_transitions_ready_drains_inflight_and_consumes_termination_context() {
        let (_temp, context) = fixture_context();
        let termination_path =
            PathBuf::from(&context.config.shared().paths.termination_context_path);
        fs::write(
            &termination_path,
            format!(
                "{{\"pid\":{},\"reason\":\"operator stop\",\"secret\":\"must-not-log\"}}",
                std::process::id()
            ),
        )
        .expect("termination context");
        let mut runtime = FakeRuntime {
            inflight: 2,
            drain_per_tick: 1,
            ..FakeRuntime::default()
        };

        let (result, wait, logs) =
            run_fake_host(&context, &mut runtime, 2, None, Duration::from_millis(50));

        result.expect("host result");
        assert!(runtime.started);
        assert!(runtime.finished);
        assert!(!runtime.forced);
        assert!(wait.calls >= 2);
        assert_eq!(states(&logs), ["starting", "ready", "stopping", "stopped"]);
        assert!(logs.contains("\"termination_context_status\":\"consumed\""));
        assert!(logs.contains("operator stop"));
        assert!(!logs.contains("must-not-log"));
        assert!(!termination_path.exists());
    }

    #[test]
    fn host_reports_runtime_start_and_tick_failures() {
        let (_temp, context) = fixture_context();
        let mut start_failure = FakeRuntime {
            fail_start: true,
            ..FakeRuntime::default()
        };
        let (start_result, _, start_logs) = run_fake_host(
            &context,
            &mut start_failure,
            usize::MAX,
            None,
            Duration::from_secs(1),
        );
        assert_eq!(start_result.unwrap_err().code, "fake_start_failed");
        assert!(start_failure.forced);
        assert_eq!(states(&start_logs), ["starting", "failed"]);

        let mut tick_failure = FakeRuntime {
            fail_tick: true,
            ..FakeRuntime::default()
        };
        let (tick_result, _, tick_logs) = run_fake_host(
            &context,
            &mut tick_failure,
            usize::MAX,
            None,
            Duration::from_secs(1),
        );
        assert_eq!(tick_result.unwrap_err().code, "fake_tick_failed");
        assert!(tick_failure.forced);
        assert_eq!(states(&tick_logs), ["starting", "ready", "failed"]);
    }

    #[test]
    fn host_reports_event_loop_wait_failure() {
        let (_temp, context) = fixture_context();
        let mut runtime = FakeRuntime::default();

        let (result, wait, logs) = run_fake_host(
            &context,
            &mut runtime,
            usize::MAX,
            Some(1),
            Duration::from_secs(1),
        );

        assert_eq!(result.unwrap_err().code, "fake_wait_failed");
        assert_eq!(wait.calls, 1);
        assert!(runtime.forced);
        assert_eq!(states(&logs), ["starting", "ready", "failed"]);
    }

    #[test]
    fn host_forces_cleanup_after_graceful_shutdown_timeout() {
        let (_temp, context) = fixture_context();
        let mut runtime = FakeRuntime {
            inflight: 1,
            ..FakeRuntime::default()
        };

        let (result, _, logs) =
            run_fake_host(&context, &mut runtime, 1, None, Duration::from_millis(5));

        assert_eq!(
            result.unwrap_err().code,
            "worker_host_graceful_shutdown_timeout"
        );
        assert!(runtime.forced);
        assert_eq!(states(&logs), ["starting", "ready", "stopping", "failed"]);
    }

    #[test]
    fn health_logs_are_versioned_and_disallow_python_execution() {
        let (_temp, context) = fixture_context();
        let runtime = FakeRuntime::default();
        let snapshot = health_snapshot(
            &context,
            &runtime,
            WorkerHostHealthState::Ready,
            None,
            None,
            None,
        );

        assert_eq!(snapshot.contract, WORKER_HOST_HEALTH_CONTRACT);
        assert_eq!(snapshot.state.as_str(), "ready");
        assert!(!snapshot.python_worker_execution_allowed);
        assert_eq!(snapshot.worker_key, "telegram/main");
    }

    #[test]
    #[cfg(unix)]
    fn process_shutdown_latch_preserves_the_first_signal() {
        PROCESS_SHUTDOWN_SIGNAL.store(0, Ordering::SeqCst);
        process_shutdown_signal_handler(libc::SIGTERM);
        process_shutdown_signal_handler(libc::SIGINT);

        assert_eq!(ProcessShutdownSource.shutdown_signal(), Some(libc::SIGTERM));
    }

    #[test]
    #[cfg(windows)]
    fn windows_console_shutdown_latch_preserves_the_first_control_event() {
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};

        PROCESS_SHUTDOWN_SIGNAL.store(0, Ordering::SeqCst);
        assert_eq!(
            unsafe { process_shutdown_console_handler(CTRL_BREAK_EVENT) },
            1
        );
        assert_eq!(unsafe { process_shutdown_console_handler(CTRL_C_EVENT) }, 1);

        assert_eq!(
            ProcessShutdownSource.shutdown_signal(),
            Some(WORKER_SHUTDOWN_TERMINATE)
        );
    }
}
