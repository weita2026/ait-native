use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use crate::foundation::scheduler::{
    admit_next, SchedulerAdmissionDecision, SchedulerJobSpec, SchedulerPolicy, SchedulerQueuedJob,
    SchedulerRunningJob,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorError {
    Stopped,
    WorkerClosed,
    Panic,
}

pub struct ExecutorFuture<R> {
    receiver: mpsc::Receiver<Result<R, ExecutorError>>,
    done: Arc<AtomicBool>,
}

impl<R> ExecutorFuture<R> {
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    pub fn wait(self) -> Result<R, ExecutorError> {
        self.receiver
            .recv()
            .expect("executor job result channel should stay open")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduledExecutorAdmission {
    Submitted {
        job_id: String,
        queue_key: String,
        cpu_tokens: usize,
        token_pools: Vec<String>,
    },
    Attached {
        job_id: String,
        active_job_id: String,
        singleflight_key: String,
    },
    Waiting {
        job_id: String,
        reason: String,
    },
}

pub enum ScheduledExecutorSubmission<R> {
    Submitted {
        admission: ScheduledExecutorAdmission,
        future: ExecutorFuture<R>,
    },
    Attached {
        admission: ScheduledExecutorAdmission,
    },
    Waiting {
        admission: ScheduledExecutorAdmission,
    },
}

struct ScheduledExecutorState {
    running: HashMap<String, SchedulerRunningJob>,
    stop_requested: bool,
}

impl Default for ScheduledExecutorState {
    fn default() -> Self {
        Self {
            running: HashMap::new(),
            stop_requested: false,
        }
    }
}

#[derive(Clone)]
pub struct ScheduledExecutorPool {
    executor: SerializedExecutorPool,
    policy: SchedulerPolicy,
    state: Arc<Mutex<ScheduledExecutorState>>,
    condition: Arc<Condvar>,
}

impl ScheduledExecutorPool {
    pub fn new(
        thread_name_prefix: impl Into<String>,
        monotonic: Arc<MonotonicFn>,
        policy: SchedulerPolicy,
    ) -> Self {
        Self {
            executor: SerializedExecutorPool::new(thread_name_prefix, monotonic),
            policy,
            state: Arc::new(Mutex::new(ScheduledExecutorState::default())),
            condition: Arc::new(Condvar::new()),
        }
    }

    pub fn policy(&self) -> SchedulerPolicy {
        self.policy.clone()
    }

    pub fn submit_scheduled<R, F>(
        &self,
        job_id: impl Into<String>,
        spec: SchedulerJobSpec,
        job: F,
    ) -> Result<ScheduledExecutorSubmission<R>, ExecutorError>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        self.submit_scheduled_with_wait(job_id.into(), spec, job, false)
    }

    pub fn submit_scheduled_wait<R, F>(
        &self,
        job_id: impl Into<String>,
        spec: SchedulerJobSpec,
        job: F,
    ) -> Result<ScheduledExecutorSubmission<R>, ExecutorError>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        self.submit_scheduled_with_wait(job_id.into(), spec, job, true)
    }

    fn submit_scheduled_with_wait<R, F>(
        &self,
        job_id: String,
        spec: SchedulerJobSpec,
        job: F,
        wait_for_admission: bool,
    ) -> Result<ScheduledExecutorSubmission<R>, ExecutorError>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        let queued = SchedulerQueuedJob {
            job_id: job_id.clone(),
            spec: spec.clone(),
            queued_ordinal: 0,
        };
        let decision = self.reserve_admission(&queued, wait_for_admission)?;

        match decision {
            SchedulerAdmissionDecision::Admit { job_id } => {
                let runner = self.clone();
                let lease_job_id = job_id.clone();
                let queue_key = spec.queue_key.clone();
                let cpu_tokens = spec.cpu_tokens;
                let token_pools = spec.token_pools.clone();
                let future = self.executor.submit_serialized(&queue_key, move || {
                    let _lease = ScheduledExecutorLease {
                        pool: runner,
                        job_id: lease_job_id,
                    };
                    job()
                });
                match future {
                    Ok(future) => Ok(ScheduledExecutorSubmission::Submitted {
                        admission: ScheduledExecutorAdmission::Submitted {
                            job_id,
                            queue_key,
                            cpu_tokens,
                            token_pools,
                        },
                        future,
                    }),
                    Err(error) => {
                        self.release_running(&job_id);
                        Err(error)
                    }
                }
            }
            SchedulerAdmissionDecision::Attach {
                job_id,
                active_job_id,
                singleflight_key,
            } => Ok(ScheduledExecutorSubmission::Attached {
                admission: ScheduledExecutorAdmission::Attached {
                    job_id,
                    active_job_id,
                    singleflight_key,
                },
            }),
            SchedulerAdmissionDecision::Wait { reason } => {
                Ok(ScheduledExecutorSubmission::Waiting {
                    admission: ScheduledExecutorAdmission::Waiting { job_id, reason },
                })
            }
        }
    }

    fn reserve_admission(
        &self,
        queued: &SchedulerQueuedJob,
        wait_for_admission: bool,
    ) -> Result<SchedulerAdmissionDecision, ExecutorError> {
        let mut state = self
            .state
            .lock()
            .expect("scheduled executor mutex poisoned");
        loop {
            if state.stop_requested {
                return Err(ExecutorError::Stopped);
            }
            let running: Vec<SchedulerRunningJob> = state.running.values().cloned().collect();
            let decision = admit_next(std::slice::from_ref(queued), &running, &self.policy);
            match &decision {
                SchedulerAdmissionDecision::Admit { job_id } => {
                    state.running.insert(
                        job_id.clone(),
                        SchedulerRunningJob {
                            job_id: job_id.clone(),
                            spec: queued.spec.clone(),
                        },
                    );
                    return Ok(decision);
                }
                SchedulerAdmissionDecision::Wait { .. } if wait_for_admission => {
                    state = self
                        .condition
                        .wait(state)
                        .expect("scheduled executor admission wait should succeed");
                }
                _ => return Ok(decision),
            }
        }
    }

    pub fn admit_next_queued(&self, queued: &[SchedulerQueuedJob]) -> SchedulerAdmissionDecision {
        let state = self
            .state
            .lock()
            .expect("scheduled executor mutex poisoned");
        let running: Vec<SchedulerRunningJob> = state.running.values().cloned().collect();
        admit_next(queued, &running, &self.policy)
    }

    pub fn running_jobs(&self) -> Vec<SchedulerRunningJob> {
        let state = self
            .state
            .lock()
            .expect("scheduled executor mutex poisoned");
        let mut jobs: Vec<SchedulerRunningJob> = state.running.values().cloned().collect();
        jobs.sort_by(|left, right| left.job_id.cmp(&right.job_id));
        jobs
    }

    pub fn running_job_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .expect("scheduled executor mutex poisoned");
        state.running.len()
    }

    pub fn wait_for_idle(&self, timeout: Option<f64>) -> bool {
        self.executor.wait_for_idle(timeout)
    }

    pub fn worker_count(&self) -> usize {
        self.executor.worker_count()
    }

    pub fn stop(&self) {
        {
            let mut state = self
                .state
                .lock()
                .expect("scheduled executor mutex poisoned");
            state.stop_requested = true;
        }
        self.condition.notify_all();
        self.executor.stop();
    }

    fn release_running(&self, job_id: &str) {
        let mut state = self
            .state
            .lock()
            .expect("scheduled executor mutex poisoned");
        state.running.remove(job_id);
        drop(state);
        self.condition.notify_all();
    }
}

struct ScheduledExecutorLease {
    pool: ScheduledExecutorPool,
    job_id: String,
}

impl Drop for ScheduledExecutorLease {
    fn drop(&mut self) {
        self.pool.release_running(&self.job_id);
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;
type MonotonicFn = dyn Fn() -> f64 + Send + Sync;

struct Worker {
    sender: mpsc::Sender<Job>,
    handle: JoinHandle<()>,
}

struct ExecutorState {
    workers: HashMap<String, Worker>,
    active_jobs: usize,
    stop_requested: bool,
}

impl Default for ExecutorState {
    fn default() -> Self {
        Self {
            workers: HashMap::new(),
            active_jobs: 0,
            stop_requested: false,
        }
    }
}

#[derive(Clone)]
pub struct SerializedExecutorPool {
    thread_name_prefix: String,
    monotonic: Arc<MonotonicFn>,
    state: Arc<Mutex<ExecutorState>>,
    condition: Arc<Condvar>,
}

impl SerializedExecutorPool {
    pub fn new(thread_name_prefix: impl Into<String>, monotonic: Arc<MonotonicFn>) -> Self {
        Self {
            thread_name_prefix: thread_name_prefix.into(),
            monotonic,
            state: Arc::new(Mutex::new(ExecutorState::default())),
            condition: Arc::new(Condvar::new()),
        }
    }

    pub fn submit_serialized<R, F>(
        &self,
        queue_key: &str,
        job: F,
    ) -> Result<ExecutorFuture<R>, ExecutorError>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel();
        let done = Arc::new(AtomicBool::new(false));
        let tracked_done = Arc::clone(&done);
        let tracker = self.clone();
        let wrapped: Job = Box::new(move || {
            let result =
                panic::catch_unwind(AssertUnwindSafe(job)).map_err(|_| ExecutorError::Panic);
            let _ = sender.send(result);
            tracked_done.store(true, Ordering::SeqCst);
            tracker.finish_job();
        });

        let mut state = self.state.lock().expect("executor mutex poisoned");
        if state.stop_requested {
            return Err(ExecutorError::Stopped);
        }
        let worker_sender = Self::ensure_worker_locked(
            &self.thread_name_prefix,
            Arc::clone(&self.condition),
            &mut state,
            queue_key,
        );
        state.active_jobs += 1;
        drop(state);
        if worker_sender.send(wrapped).is_err() {
            let mut state = self.state.lock().expect("executor mutex poisoned");
            state.active_jobs = state.active_jobs.saturating_sub(1);
            self.condition.notify_all();
            done.store(true, Ordering::SeqCst);
            return Err(ExecutorError::WorkerClosed);
        }
        Ok(ExecutorFuture { receiver, done })
    }

    pub fn wait_for_idle(&self, timeout: Option<f64>) -> bool {
        let deadline = timeout.map(|value| (self.monotonic)() + value);
        let mut state = self.state.lock().expect("executor mutex poisoned");
        while state.active_jobs > 0 {
            let remaining = deadline.map(|limit| (limit - (self.monotonic)()).max(0.0));
            if let Some(value) = remaining {
                if value <= 0.0 {
                    return false;
                }
                let (next_state, _) = self
                    .condition
                    .wait_timeout(state, std::time::Duration::from_secs_f64(value))
                    .expect("executor wait should succeed");
                state = next_state;
                continue;
            }
            state = self
                .condition
                .wait(state)
                .expect("executor wait should succeed");
        }
        true
    }

    pub fn worker_count(&self) -> usize {
        let state = self.state.lock().expect("executor mutex poisoned");
        state.workers.len()
    }

    pub fn stop(&self) {
        let workers = {
            let mut state = self.state.lock().expect("executor mutex poisoned");
            state.stop_requested = true;
            std::mem::take(&mut state.workers)
        };
        for worker in workers.into_values() {
            drop(worker.sender);
            let _ = worker.handle.join();
        }
        self.condition.notify_all();
    }

    fn finish_job(&self) {
        let mut state = self.state.lock().expect("executor mutex poisoned");
        state.active_jobs = state.active_jobs.saturating_sub(1);
        self.condition.notify_all();
    }

    fn ensure_worker_locked(
        thread_name_prefix: &str,
        condition: Arc<Condvar>,
        state: &mut ExecutorState,
        queue_key: &str,
    ) -> mpsc::Sender<Job> {
        state
            .workers
            .entry(queue_key.to_string())
            .or_insert_with(|| {
                let (sender, receiver) = mpsc::channel::<Job>();
                let thread_name = format!("{thread_name_prefix}-{queue_key}");
                let handle = thread::Builder::new()
                    .name(thread_name)
                    .spawn(move || {
                        while let Ok(job) = receiver.recv() {
                            job();
                        }
                        condition.notify_all();
                    })
                    .expect("executor worker thread should spawn");
                Worker { sender, handle }
            })
            .sender
            .clone()
    }
}
