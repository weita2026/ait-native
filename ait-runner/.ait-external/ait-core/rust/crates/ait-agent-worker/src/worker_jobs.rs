use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::{WorkerDiagnostic, EXIT_INVALID_CONFIGURATION, EXIT_RUNTIME_UNAVAILABLE};

const DEFAULT_MAX_INFLIGHT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerJobExecutorConfig {
    pub max_inflight: usize,
}

impl Default for WorkerJobExecutorConfig {
    fn default() -> Self {
        Self {
            max_inflight: DEFAULT_MAX_INFLIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerJobExecutorState {
    Open,
    Draining,
    Detached,
}

#[derive(Debug)]
pub struct WorkerJobCompletion<T> {
    pub job_id: u64,
    pub kind: String,
    pub result: Result<T, WorkerDiagnostic>,
}

pub struct BoundedWorkerJobExecutor<T> {
    config: WorkerJobExecutorConfig,
    state: WorkerJobExecutorState,
    next_job_id: Option<u64>,
    sender: Sender<WorkerJobCompletion<T>>,
    receiver: Receiver<WorkerJobCompletion<T>>,
    active: BTreeMap<u64, ActiveWorkerJob>,
    pending_completions: BTreeMap<u64, WorkerJobCompletion<T>>,
}

struct ActiveWorkerJob {
    kind: String,
    handle: JoinHandle<()>,
}

impl<T> BoundedWorkerJobExecutor<T>
where
    T: Send + 'static,
{
    pub fn new(config: WorkerJobExecutorConfig) -> Result<Self, WorkerDiagnostic> {
        if config.max_inflight == 0 {
            return Err(WorkerDiagnostic::new(
                "worker_job_executor_config_invalid",
                "The Rust worker job concurrency limit must be greater than zero.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("field", "max_inflight"));
        }
        let (sender, receiver) = mpsc::channel();
        Ok(Self {
            config,
            state: WorkerJobExecutorState::Open,
            next_job_id: Some(1),
            sender,
            receiver,
            active: BTreeMap::new(),
            pending_completions: BTreeMap::new(),
        })
    }

    pub fn state(&self) -> WorkerJobExecutorState {
        self.state
    }

    pub fn max_inflight(&self) -> usize {
        self.config.max_inflight
    }

    pub fn inflight_count(&self) -> usize {
        self.active.len()
    }

    pub fn available_capacity(&self) -> usize {
        if self.state != WorkerJobExecutorState::Open {
            return 0;
        }
        self.config.max_inflight.saturating_sub(self.active.len())
    }

    pub fn close_admission(&mut self) {
        if self.state == WorkerJobExecutorState::Open {
            self.state = WorkerJobExecutorState::Draining;
        }
    }

    pub fn submit<F>(&mut self, kind: impl Into<String>, job: F) -> Result<u64, WorkerDiagnostic>
    where
        F: FnOnce() -> Result<T, WorkerDiagnostic> + Send + 'static,
    {
        let kind = validate_job_kind(kind.into())?;
        if self.state != WorkerJobExecutorState::Open {
            return Err(WorkerDiagnostic::new(
                "worker_job_executor_closed",
                "The Rust worker job executor is not accepting new work.",
                EXIT_RUNTIME_UNAVAILABLE,
            )
            .with_detail("state", executor_state_label(self.state))
            .with_detail("kind", kind));
        }
        if self.active.len() >= self.config.max_inflight {
            return Err(WorkerDiagnostic::new(
                "worker_job_capacity_exhausted",
                "The Rust worker job concurrency limit is exhausted.",
                EXIT_RUNTIME_UNAVAILABLE,
            )
            .with_detail("kind", kind)
            .with_detail("max_inflight", self.config.max_inflight)
            .with_detail("inflight_count", self.active.len()));
        }
        let job_id = self.next_job_id.ok_or_else(|| {
            WorkerDiagnostic::new(
                "worker_job_id_exhausted",
                "The Rust worker job identifier space is exhausted.",
                EXIT_RUNTIME_UNAVAILABLE,
            )
        })?;
        self.next_job_id = job_id.checked_add(1);
        let sender = self.sender.clone();
        let completion_kind = kind.clone();
        let handle = thread::Builder::new()
            .name(format!("ait-worker-job-{job_id}"))
            .spawn(move || {
                let result = match catch_unwind(AssertUnwindSafe(job)) {
                    Ok(result) => result,
                    Err(_) => Err(WorkerDiagnostic::new(
                        "worker_job_panicked",
                        "A Rust worker job panicked; its panic payload was suppressed.",
                        EXIT_RUNTIME_UNAVAILABLE,
                    )
                    .with_detail("job_id", job_id)
                    .with_detail("kind", completion_kind.clone())),
                };
                let _ = sender.send(WorkerJobCompletion {
                    job_id,
                    kind: completion_kind,
                    result,
                });
            })
            .map_err(|error| {
                WorkerDiagnostic::new(
                    "worker_job_spawn_failed",
                    format!("Cannot spawn bounded Rust worker job {job_id}: {error}"),
                    EXIT_RUNTIME_UNAVAILABLE,
                )
                .with_detail("job_id", job_id)
                .with_detail("kind", kind.clone())
            })?;
        self.active.insert(job_id, ActiveWorkerJob { kind, handle });
        Ok(job_id)
    }

    pub fn poll_completed(&mut self) -> Vec<WorkerJobCompletion<T>> {
        while let Ok(completion) = self.receiver.try_recv() {
            if self.state != WorkerJobExecutorState::Detached
                && self.active.contains_key(&completion.job_id)
            {
                self.pending_completions
                    .insert(completion.job_id, completion);
            }
        }
        if self.state == WorkerJobExecutorState::Detached {
            self.pending_completions.clear();
            return Vec::new();
        }

        let finished: Vec<u64> = self
            .active
            .iter()
            .filter_map(|(job_id, active)| active.handle.is_finished().then_some(*job_id))
            .collect();
        let mut completions = Vec::with_capacity(finished.len());
        for job_id in finished {
            let Some(active) = self.active.remove(&job_id) else {
                continue;
            };
            let join_result = active.handle.join();
            // Joining establishes that this worker has completed its channel send. Reap the
            // channel again after that synchronization point so a completion that raced with
            // the first non-blocking drain is not misclassified as missing.
            while let Ok(completion) = self.receiver.try_recv() {
                if self.state != WorkerJobExecutorState::Detached
                    && (completion.job_id == job_id || self.active.contains_key(&completion.job_id))
                {
                    self.pending_completions
                        .insert(completion.job_id, completion);
                }
            }
            let completion =
                self.pending_completions
                    .remove(&job_id)
                    .unwrap_or_else(|| WorkerJobCompletion {
                        job_id,
                        kind: active.kind,
                        result: Err(WorkerDiagnostic::new(
                            "worker_job_completion_missing",
                            "A Rust worker job exited without publishing a completion.",
                            EXIT_RUNTIME_UNAVAILABLE,
                        )
                        .with_detail("job_id", job_id)
                        .with_detail("thread_panicked", join_result.is_err())),
                    });
            completions.push(completion);
        }
        completions
    }

    pub fn force_detach(&mut self) {
        self.state = WorkerJobExecutorState::Detached;
        self.active.clear();
        self.pending_completions.clear();
        while self.receiver.try_recv().is_ok() {}
    }
}

fn validate_job_kind(value: String) -> Result<String, WorkerDiagnostic> {
    let kind = value.trim();
    if kind.is_empty()
        || kind.len() > 64
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(WorkerDiagnostic::new(
            "worker_job_kind_invalid",
            "The Rust worker job kind must be a short ASCII identifier.",
            EXIT_INVALID_CONFIGURATION,
        ));
    }
    Ok(kind.to_string())
}

fn executor_state_label(state: WorkerJobExecutorState) -> &'static str {
    match state {
        WorkerJobExecutorState::Open => "open",
        WorkerJobExecutorState::Draining => "draining",
        WorkerJobExecutorState::Detached => "detached",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use super::*;

    fn executor<T: Send + 'static>(max_inflight: usize) -> BoundedWorkerJobExecutor<T> {
        BoundedWorkerJobExecutor::new(WorkerJobExecutorConfig { max_inflight })
            .expect("job executor")
    }

    fn poll_until<T: Send + 'static>(
        executor: &mut BoundedWorkerJobExecutor<T>,
        expected: usize,
    ) -> Vec<WorkerJobCompletion<T>> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut completed = Vec::new();
        while completed.len() < expected {
            completed.extend(executor.poll_completed());
            assert!(Instant::now() < deadline, "job completion timed out");
            thread::yield_now();
        }
        completed
    }

    #[test]
    fn bounded_jobs_hold_capacity_until_reaped_and_use_monotonic_ids() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let mut executor = executor::<u64>(2);

        for value in [10_u64, 20] {
            let gate = gate.clone();
            let started_tx = started_tx.clone();
            let done_tx = done_tx.clone();
            let job_id = executor
                .submit("line.turn", move || {
                    started_tx.send(value).expect("started signal");
                    let (lock, ready) = &*gate;
                    let released = lock.lock().expect("gate lock");
                    drop(
                        ready
                            .wait_while(released, |released| !*released)
                            .expect("gate wait"),
                    );
                    done_tx.send(value).expect("done signal");
                    Ok(value)
                })
                .expect("submit job");
            assert_eq!(job_id, value / 10);
        }
        started_rx.recv().expect("first started");
        started_rx.recv().expect("second started");
        let capacity_error = executor
            .submit("line.turn", || Ok(30))
            .expect_err("capacity must reject");
        assert_eq!(capacity_error.code, "worker_job_capacity_exhausted");
        assert_eq!(executor.inflight_count(), 2);

        let (lock, ready) = &*gate;
        *lock.lock().expect("gate lock") = true;
        ready.notify_all();
        done_rx.recv().expect("first done");
        done_rx.recv().expect("second done");
        assert_eq!(
            executor
                .submit("line.turn", || Ok(30))
                .expect_err("finished but unreaped jobs still consume capacity")
                .code,
            "worker_job_capacity_exhausted"
        );

        let mut completed = poll_until(&mut executor, 2);
        completed.sort_by_key(|completion| completion.job_id);
        assert_eq!(
            completed
                .into_iter()
                .map(|completion| completion.result.expect("job result"))
                .collect::<Vec<_>>(),
            [10, 20]
        );
        assert_eq!(executor.inflight_count(), 0);
        assert_eq!(
            executor.submit("line.turn", || Ok(30)).expect("third job"),
            3
        );
        assert_eq!(
            poll_until(&mut executor, 1)
                .pop()
                .expect("third completion")
                .result
                .expect("third result"),
            30
        );
    }

    #[test]
    fn job_errors_and_panics_are_typed_and_do_not_poison_the_executor() {
        let mut executor = executor::<u64>(1);
        executor
            .submit("line.failure", || {
                Err(WorkerDiagnostic::new(
                    "line_job_failed",
                    "line job failed",
                    EXIT_RUNTIME_UNAVAILABLE,
                ))
            })
            .expect("error job");
        assert_eq!(
            poll_until(&mut executor, 1)[0]
                .result
                .as_ref()
                .expect_err("job error")
                .code,
            "line_job_failed"
        );

        executor
            .submit("line.panic", || panic!("secret panic payload"))
            .expect("panic job");
        let completion = poll_until(&mut executor, 1).pop().expect("panic result");
        let error = completion.result.expect_err("panic must be an error");
        assert_eq!(error.code, "worker_job_panicked");
        assert!(!error.message.contains("secret panic payload"));

        executor
            .submit("line.recovery", || Ok(42))
            .expect("recovery job");
        assert_eq!(
            poll_until(&mut executor, 1)[0]
                .result
                .as_ref()
                .expect("recovery result"),
            &42
        );
    }

    #[test]
    fn forced_detach_is_nonblocking_rejects_new_jobs_and_discards_late_results() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let mut executor = executor::<u64>(1);
        let job_gate = gate.clone();
        executor
            .submit("line.stuck", move || {
                started_tx.send(()).expect("started signal");
                let (lock, ready) = &*job_gate;
                let released = lock.lock().expect("gate lock");
                drop(
                    ready
                        .wait_while(released, |released| !*released)
                        .expect("gate wait"),
                );
                done_tx.send(()).expect("done signal");
                Ok(7)
            })
            .expect("stuck job");
        started_rx.recv().expect("job started");

        let started_at = Instant::now();
        executor.force_detach();
        assert!(started_at.elapsed() < Duration::from_secs(1));
        assert_eq!(executor.state(), WorkerJobExecutorState::Detached);
        assert_eq!(executor.inflight_count(), 0);
        assert_eq!(executor.available_capacity(), 0);
        assert_eq!(
            executor
                .submit("line.late", || Ok(8))
                .expect_err("detached executor must reject")
                .code,
            "worker_job_executor_closed"
        );

        let (lock, ready) = &*gate;
        *lock.lock().expect("gate lock") = true;
        ready.notify_all();
        done_rx.recv().expect("detached job finished");
        thread::yield_now();
        assert!(executor.poll_completed().is_empty());
        assert_eq!(executor.inflight_count(), 0);
    }

    #[test]
    fn draining_and_invalid_configuration_close_admission_fail_closed() {
        let config_error =
            match BoundedWorkerJobExecutor::<u64>::new(WorkerJobExecutorConfig { max_inflight: 0 })
            {
                Ok(_) => panic!("zero capacity must fail"),
                Err(error) => error,
            };
        assert_eq!(config_error.code, "worker_job_executor_config_invalid");
        let mut executor = executor::<u64>(1);
        assert_eq!(executor.max_inflight(), 1);
        assert_eq!(executor.available_capacity(), 1);
        executor.close_admission();
        assert_eq!(executor.state(), WorkerJobExecutorState::Draining);
        assert_eq!(executor.available_capacity(), 0);
        assert_eq!(
            executor
                .submit("line.closed", || Ok(1))
                .expect_err("closed admission")
                .code,
            "worker_job_executor_closed"
        );
        assert_eq!(
            executor
                .submit("not a kind", || Ok(1))
                .expect_err("invalid kind")
                .code,
            "worker_job_kind_invalid"
        );
    }
}
