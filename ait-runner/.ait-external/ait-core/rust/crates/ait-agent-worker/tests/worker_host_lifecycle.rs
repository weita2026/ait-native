#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ait_agent_core::AgentEvent;
use ait_agent_worker::{
    prepare_worker_run_with_env, run_worker_host_with_ports, AgentEventLoopHostWait,
    ProcessShutdownSource, SystemWorkerHostClock, WorkerHostEventLoop, WorkerHostRuntime,
    WorkerHostSettings, WorkerPathInputs, WorkerRunContext, WorkerRunRequest,
};
use tempfile::tempdir;

use ait_agent_worker::{WorkerDiagnostic, EXIT_RUNTIME_UNAVAILABLE};

#[derive(Default)]
struct ProbeRuntime {
    ready_marker: Option<std::path::PathBuf>,
    stopped: bool,
}

impl WorkerHostRuntime for ProbeRuntime {
    fn start(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        let marker = self.ready_marker.as_ref().ok_or_else(|| {
            WorkerDiagnostic::new(
                "signal_probe_marker_missing",
                "signal probe marker is missing",
                EXIT_RUNTIME_UNAVAILABLE,
            )
        })?;
        fs::write(marker, b"ready").map_err(|error| {
            WorkerDiagnostic::new(
                "signal_probe_marker_write_failed",
                error.to_string(),
                EXIT_RUNTIME_UNAVAILABLE,
            )
        })
    }

    fn tick(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
        _events: &[AgentEvent],
    ) -> Result<(), WorkerDiagnostic> {
        Ok(())
    }

    fn request_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
        _signal: i32,
    ) -> Result<(), WorkerDiagnostic> {
        self.stopped = true;
        Ok(())
    }

    fn inflight_work_count(&self) -> usize {
        0
    }

    fn finish_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        Ok(())
    }

    fn force_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        self.stopped = true;
        Ok(())
    }
}

fn fixture_context() -> (tempfile::TempDir, WorkerRunContext) {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    fs::write(
        temp.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    fs::write(
        temp.path().join(".ait/agent-workers.json"),
        r#"{"version":1,"workers":{"telegram/main":{"kind":"telegram","name":"main","token":"secret"}}}"#,
    )
    .expect("manifest");
    let context = prepare_worker_run_with_env(
        &WorkerRunRequest {
            transport: "telegram".to_string(),
            worker: "main".to_string(),
            event_loop_backend: "portable_poll".to_string(),
            shard: "0".to_string(),
        },
        &WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: Some(temp.path().to_path_buf()),
            manifest_path_override: None,
        },
        BTreeMap::new(),
    )
    .expect("worker context");
    (temp, context)
}

#[test]
#[ignore = "subprocess helper selected explicitly by the signal lifecycle test"]
fn process_signal_child_helper() {
    let (_temp, context) = fixture_context();
    let signals = ProcessShutdownSource::install().expect("install process signals");
    let mut wait = AgentEventLoopHostWait::new(&context).expect("event loop wait");
    let clock = SystemWorkerHostClock::new();
    let mut runtime = ProbeRuntime {
        ready_marker: Some(
            std::env::current_dir()
                .expect("signal helper current directory")
                .join("ready"),
        ),
        stopped: false,
    };
    let mut stdout = std::io::stdout().lock();

    run_worker_host_with_ports(
        &context,
        &mut runtime,
        &signals,
        &mut wait,
        &clock,
        WorkerHostSettings {
            poll_interval: Duration::from_millis(10),
            shutdown_grace: Duration::from_secs(1),
        },
        &mut stdout,
    )
    .expect("worker host result");
    assert!(runtime.stopped);
}

#[test]
fn real_sigterm_stops_child_host_with_structured_health_events() {
    let temp = tempdir().expect("tempdir");
    let marker = temp.path().join("ready");
    let mut child = Command::new(std::env::current_exe().expect("current test executable"))
        .current_dir(temp.path())
        .args([
            "--ignored",
            "--exact",
            "process_signal_child_helper",
            "--nocapture",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn signal child");

    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() {
        assert!(
            Instant::now() < ready_deadline,
            "signal child did not become ready"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);

    let exit_deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll signal child") {
            break status;
        }
        if Instant::now() >= exit_deadline {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGKILL);
            }
            panic!("signal child did not stop within its grace interval");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("child stdout")
        .read_to_string(&mut stdout)
        .expect("read child stdout");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("child stderr")
        .read_to_string(&mut stderr)
        .expect("read child stderr");

    assert!(status.success(), "child stderr: {stderr}");
    assert!(stdout.contains("\"state\":\"ready\""), "{stdout}");
    assert!(stdout.contains("\"state\":\"stopping\""), "{stdout}");
    assert!(stdout.contains("\"state\":\"stopped\""), "{stdout}");
    assert!(stdout.contains("\"shutdown_signal\":15"), "{stdout}");
    assert!(stdout.contains("\"python_worker_execution_allowed\":false"));
}
