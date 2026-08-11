use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

pub const DEFERRED_REPLY_SCHEDULER_KERNEL_VERSION: &str = "ait.deferred_reply_scheduler_kernel.v1";
pub const DEFERRED_REPLY_SHARED_SCHEDULER_CORE_ID: &str = "ait.client_epoll.shared.v1";
static LOCAL_SCHEDULER_COUNTER: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeferredReplyScheduledWatchState {
    pub kernel_version: String,
    pub watch_id: String,
    pub deadline_at: f64,
    pub next_poll_at: f64,
    pub inflight: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeferredReplyScheduledWatchResolution {
    pub kernel_version: String,
    pub remove_watch: bool,
    pub callback_kind: Option<String>,
    pub state: Option<DeferredReplyScheduledWatchState>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecoveryOutcome {
    Recovered,
    Retryable,
    Terminal,
}

impl RecoveryOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recovered => "recovered",
            Self::Retryable => "retryable",
            Self::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryResult<T> {
    pub outcome: RecoveryOutcome,
    pub payload: Option<T>,
}

impl<T> RecoveryResult<T> {
    pub fn recovered(payload: T) -> Self {
        Self {
            outcome: RecoveryOutcome::Recovered,
            payload: Some(payload),
        }
    }

    pub fn retryable() -> Self {
        Self {
            outcome: RecoveryOutcome::Retryable,
            payload: None,
        }
    }

    pub fn terminal() -> Self {
        Self {
            outcome: RecoveryOutcome::Terminal,
            payload: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WatchRecoveryPolicy {
    pub attempts: usize,
    pub delay_schedule_seconds: Vec<f64>,
    pub watch_max_wait_seconds: f64,
    pub watch_poll_interval_seconds: f64,
}

pub trait WatchRecoveryRuntime<P, T>: Send + Sync {
    fn still_pending(&self, pending: &P) -> bool;
    fn watch_max_wait_seconds(&self, pending: &P) -> f64;
    fn watch_poll_interval_seconds(&self, pending: &P) -> f64;
    fn recover_completed_pending_reply_result(&self, pending: &P) -> RecoveryResult<T>;
}

type RecoveryFn<P, T> = dyn Fn(&P) -> RecoveryResult<T> + Send + Sync;
type StillPendingFn<P> = dyn Fn(&P) -> bool + Send + Sync;
type AttemptsFn = dyn Fn() -> usize + Send + Sync;
type DelayFn = dyn Fn(usize) -> f64 + Send + Sync;
type ScalarFn = dyn Fn() -> f64 + Send + Sync;
type PendingPolicyFn<P> = dyn Fn(&P) -> Option<WatchRecoveryPolicy> + Send + Sync;
type SleepFn = dyn Fn(f64) + Send + Sync;
type MonotonicFn = dyn Fn() -> f64 + Send + Sync;

#[derive(Clone)]
pub struct ClientEpollRuntime<P, T> {
    recover_once: Arc<RecoveryFn<P, T>>,
    still_pending: Arc<StillPendingFn<P>>,
    recovery_attempts: Arc<AttemptsFn>,
    recovery_delay_seconds: Arc<DelayFn>,
    watch_max_wait_seconds: Arc<ScalarFn>,
    watch_poll_interval_seconds: Arc<ScalarFn>,
    pending_policy: Arc<PendingPolicyFn<P>>,
    sleep: Arc<SleepFn>,
    monotonic: Arc<MonotonicFn>,
}

impl<P, T> ClientEpollRuntime<P, T> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        recover_once: Arc<RecoveryFn<P, T>>,
        still_pending: Arc<StillPendingFn<P>>,
        recovery_attempts: Arc<AttemptsFn>,
        recovery_delay_seconds: Arc<DelayFn>,
        watch_max_wait_seconds: Arc<ScalarFn>,
        watch_poll_interval_seconds: Arc<ScalarFn>,
        pending_policy: Arc<PendingPolicyFn<P>>,
        sleep: Arc<SleepFn>,
        monotonic: Arc<MonotonicFn>,
    ) -> Self {
        Self {
            recover_once,
            still_pending,
            recovery_attempts,
            recovery_delay_seconds,
            watch_max_wait_seconds,
            watch_poll_interval_seconds,
            pending_policy,
            sleep,
            monotonic,
        }
    }

    fn recovery_policy(&self, pending: &P) -> Option<WatchRecoveryPolicy> {
        (self.pending_policy)(pending)
    }

    fn recovery_attempt_count(&self, pending: &P) -> usize {
        match self.recovery_policy(pending) {
            Some(policy) => policy.attempts.max(1),
            None => (self.recovery_attempts)().max(1),
        }
    }

    fn recovery_delay(&self, pending: &P, attempt: usize) -> f64 {
        match self.recovery_policy(pending) {
            Some(policy) if !policy.delay_schedule_seconds.is_empty() => {
                let index = attempt.min(policy.delay_schedule_seconds.len() - 1);
                policy.delay_schedule_seconds[index]
            }
            _ => (self.recovery_delay_seconds)(attempt),
        }
    }

    pub fn still_pending(&self, pending: &P) -> bool {
        (self.still_pending)(pending)
    }

    pub fn watch_max_wait_seconds(&self, pending: &P) -> f64 {
        match self.recovery_policy(pending) {
            Some(policy) => policy.watch_max_wait_seconds,
            None => (self.watch_max_wait_seconds)(),
        }
    }

    pub fn watch_poll_interval_seconds(&self, pending: &P) -> f64 {
        match self.recovery_policy(pending) {
            Some(policy) => policy.watch_poll_interval_seconds,
            None => (self.watch_poll_interval_seconds)(),
        }
    }

    pub fn recover_completed_pending_reply_result(&self, pending: &P) -> RecoveryResult<T> {
        let attempts = self.recovery_attempt_count(pending);
        let mut last_result = RecoveryResult::retryable();
        for attempt in 0..attempts {
            last_result = (self.recover_once)(pending);
            if last_result.outcome == RecoveryOutcome::Recovered {
                return last_result;
            }
            if last_result.outcome == RecoveryOutcome::Terminal || attempt + 1 >= attempts {
                return last_result;
            }
            (self.sleep)(self.recovery_delay(pending, attempt));
        }
        last_result
    }

    pub fn recover_completed_pending_reply(&self, pending: &P) -> bool {
        self.recover_completed_pending_reply_result(pending).outcome == RecoveryOutcome::Recovered
    }

    pub fn watch_for_completed_pending_reply(&self, pending: &P) -> bool {
        let max_wait_seconds = self.watch_max_wait_seconds(pending);
        if max_wait_seconds <= 0.0 {
            return false;
        }
        let deadline = (self.monotonic)() + max_wait_seconds;
        while (self.monotonic)() < deadline {
            if !self.still_pending(pending) {
                return false;
            }
            let remaining_seconds = deadline - (self.monotonic)();
            if remaining_seconds <= 0.0 {
                break;
            }
            (self.sleep)(
                self.watch_poll_interval_seconds(pending)
                    .min(remaining_seconds),
            );
            if self.recover_completed_pending_reply_result(pending).outcome
                == RecoveryOutcome::Recovered
            {
                return true;
            }
        }
        false
    }
}

impl<P, T> WatchRecoveryRuntime<P, T> for ClientEpollRuntime<P, T> {
    fn still_pending(&self, pending: &P) -> bool {
        self.still_pending(pending)
    }

    fn watch_max_wait_seconds(&self, pending: &P) -> f64 {
        self.watch_max_wait_seconds(pending)
    }

    fn watch_poll_interval_seconds(&self, pending: &P) -> f64 {
        self.watch_poll_interval_seconds(pending)
    }

    fn recover_completed_pending_reply_result(&self, pending: &P) -> RecoveryResult<T> {
        self.recover_completed_pending_reply_result(pending)
    }
}

type RecoveredCallback<P, T> = dyn Fn(P, Option<T>) + Send + Sync;
type ExhaustedCallback<P> = dyn Fn(P) + Send + Sync;

struct ScheduledWatch<P, T> {
    watch_id: String,
    runtime: Arc<dyn WatchRecoveryRuntime<P, T>>,
    pending: P,
    kernel_state: DeferredReplyScheduledWatchState,
    on_recovered: Option<Arc<RecoveredCallback<P, T>>>,
    on_exhausted: Option<Arc<ExhaustedCallback<P>>>,
}

impl<P, T> Clone for ScheduledWatch<P, T>
where
    P: Clone,
{
    fn clone(&self) -> Self {
        Self {
            watch_id: self.watch_id.clone(),
            runtime: Arc::clone(&self.runtime),
            pending: self.pending.clone(),
            kernel_state: self.kernel_state.clone(),
            on_recovered: self.on_recovered.clone(),
            on_exhausted: self.on_exhausted.clone(),
        }
    }
}

struct SchedulerState<P, T> {
    watches: HashMap<String, ScheduledWatch<P, T>>,
    active_attempts: usize,
    stop_requested: bool,
    thread_running: bool,
}

impl<P, T> Default for SchedulerState<P, T> {
    fn default() -> Self {
        Self {
            watches: HashMap::new(),
            active_attempts: 0,
            stop_requested: false,
            thread_running: false,
        }
    }
}

struct SchedulerInner<P, T> {
    default_runtime: Option<Arc<dyn WatchRecoveryRuntime<P, T>>>,
    monotonic: Arc<MonotonicFn>,
    condition: Condvar,
    state: Mutex<SchedulerState<P, T>>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    watch_counter: AtomicUsize,
    shared: bool,
    core_id: String,
}

pub struct ClientEpollWatchScheduler<P, T> {
    inner: Arc<SchedulerInner<P, T>>,
}

impl<P, T> Clone for ClientEpollWatchScheduler<P, T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<P, T> ClientEpollWatchScheduler<P, T>
where
    P: Clone + Send + 'static,
    T: Send + 'static,
{
    pub fn new(
        runtime: Option<Arc<dyn WatchRecoveryRuntime<P, T>>>,
        monotonic: Arc<MonotonicFn>,
    ) -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                default_runtime: runtime,
                monotonic,
                condition: Condvar::new(),
                state: Mutex::new(SchedulerState::default()),
                thread_handle: Mutex::new(None),
                watch_counter: AtomicUsize::new(1),
                shared: false,
                core_id: format!(
                    "ait.client_epoll.local.{}",
                    LOCAL_SCHEDULER_COUNTER.fetch_add(1, Ordering::Relaxed)
                ),
            }),
        }
    }

    pub fn new_shared(
        runtime: Option<Arc<dyn WatchRecoveryRuntime<P, T>>>,
        monotonic: Arc<MonotonicFn>,
    ) -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                default_runtime: runtime,
                monotonic,
                condition: Condvar::new(),
                state: Mutex::new(SchedulerState::default()),
                thread_handle: Mutex::new(None),
                watch_counter: AtomicUsize::new(1),
                shared: true,
                core_id: DEFERRED_REPLY_SHARED_SCHEDULER_CORE_ID.to_string(),
            }),
        }
    }

    pub fn core_id(&self) -> &str {
        &self.inner.core_id
    }

    pub fn shared(&self) -> bool {
        self.inner.shared
    }

    #[allow(clippy::type_complexity)]
    pub fn schedule_watch(
        &self,
        pending: P,
        runtime: Option<Arc<dyn WatchRecoveryRuntime<P, T>>>,
        watch_id: Option<String>,
        on_recovered: Option<Arc<RecoveredCallback<P, T>>>,
        on_exhausted: Option<Arc<ExhaustedCallback<P>>>,
    ) -> Result<bool, String> {
        let active_runtime = match runtime.or_else(|| self.inner.default_runtime.clone()) {
            Some(runtime) => runtime,
            None => {
                return Err(
                    "ClientEpollWatchScheduler.schedule_watch requires a runtime for unbound schedulers."
                        .to_string(),
                )
            }
        };
        let max_wait_seconds = active_runtime.watch_max_wait_seconds(&pending);
        if max_wait_seconds <= 0.0 {
            return Ok(false);
        }
        let watch_id = watch_id.unwrap_or_else(|| {
            let next = self.inner.watch_counter.fetch_add(1, Ordering::Relaxed);
            format!("watch-{next}")
        });
        let now = (self.inner.monotonic)();
        let poll_interval_seconds = active_runtime
            .watch_poll_interval_seconds(&pending)
            .max(0.0);
        let watch = ScheduledWatch {
            watch_id: watch_id.clone(),
            runtime: active_runtime,
            pending,
            kernel_state: build_watch_state(
                &watch_id,
                now,
                max_wait_seconds,
                poll_interval_seconds,
            ),
            on_recovered,
            on_exhausted,
        };
        let mut state = self
            .inner
            .state
            .lock()
            .expect("watch scheduler mutex poisoned");
        if state.stop_requested {
            return Ok(false);
        }
        if state.watches.contains_key(&watch_id) {
            return Ok(false);
        }
        state.watches.insert(watch_id, watch);
        self.ensure_thread_locked(&mut state);
        self.inner.condition.notify_all();
        Ok(true)
    }

    pub fn wait_for_idle(&self, timeout: Option<f64>) -> bool {
        let deadline = timeout.map(|value| (self.inner.monotonic)() + value);
        let mut state = self
            .inner
            .state
            .lock()
            .expect("watch scheduler mutex poisoned");
        while !state.watches.is_empty() || state.active_attempts > 0 {
            let remaining = deadline.map(|limit| (limit - (self.inner.monotonic)()).max(0.0));
            if let Some(value) = remaining {
                if value <= 0.0 {
                    return false;
                }
                let (next_state, _) = self
                    .inner
                    .condition
                    .wait_timeout(state, std::time::Duration::from_secs_f64(value))
                    .expect("watch scheduler wait should succeed");
                state = next_state;
                continue;
            }
            state = self
                .inner
                .condition
                .wait(state)
                .expect("watch scheduler wait should succeed");
        }
        true
    }

    pub fn active_watch_count(&self) -> usize {
        let state = self
            .inner
            .state
            .lock()
            .expect("watch scheduler mutex poisoned");
        state.watches.len()
    }

    pub fn stop(&self) {
        if self.inner.shared {
            return;
        }
        self.force_stop();
    }

    pub fn force_stop(&self) {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("watch scheduler mutex poisoned");
            state.stop_requested = true;
            state.watches.clear();
            self.inner.condition.notify_all();
        }
        if let Some(handle) = self
            .inner
            .thread_handle
            .lock()
            .expect("watch scheduler handle mutex poisoned")
            .take()
        {
            let _ = handle.join();
        }
    }

    fn ensure_thread_locked(&self, state: &mut SchedulerState<P, T>) {
        if state.thread_running {
            return;
        }
        state.thread_running = true;
        let scheduler = self.clone();
        let handle = thread::Builder::new()
            .name(format!("ait-client-epoll-scheduler-{}", self.inner.core_id))
            .spawn(move || scheduler.run_scheduler_loop())
            .expect("watch scheduler thread should spawn");
        *self
            .inner
            .thread_handle
            .lock()
            .expect("watch scheduler handle mutex poisoned") = Some(handle);
    }

    fn run_scheduler_loop(&self) {
        loop {
            let mut recovered_callback: Option<(Arc<RecoveredCallback<P, T>>, P, Option<T>)> = None;
            let mut exhausted_callback: Option<(Arc<ExhaustedCallback<P>>, P)> = None;
            {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .expect("watch scheduler mutex poisoned");
                if state.stop_requested {
                    state.thread_running = false;
                    self.inner.condition.notify_all();
                    return;
                }
                let now = (self.inner.monotonic)();
                let exhausted_watch = state
                    .watches
                    .values()
                    .find(|watch| {
                        watch_is_exhausted(
                            &watch.kernel_state,
                            now,
                            watch.runtime.still_pending(&watch.pending),
                        )
                    })
                    .cloned();
                if let Some(watch) = exhausted_watch {
                    state.watches.remove(&watch.watch_id);
                    if let Some(callback) = watch.on_exhausted {
                        exhausted_callback = Some((callback, watch.pending));
                    }
                    self.inner.condition.notify_all();
                } else {
                    let due_watch = state
                        .watches
                        .values()
                        .find(|watch| watch_is_due(&watch.kernel_state, now))
                        .cloned();
                    if let Some(watch) = due_watch {
                        if let Some(current) = state.watches.get_mut(&watch.watch_id) {
                            current.kernel_state = mark_watch_inflight(&current.kernel_state);
                        }
                        state.active_attempts += 1;
                        let scheduler = self.clone();
                        let watch_id = watch.watch_id.clone();
                        thread::Builder::new()
                            .name(format!("ait-client-epoll-attempt-{}", watch.watch_id))
                            .spawn(move || scheduler.run_watch_attempt(watch_id))
                            .expect("watch attempt thread should spawn");
                    } else {
                        let timeout = self.next_wait_timeout_locked(&state, now);
                        if timeout > 0.0 {
                            let (next_state, _) = self
                                .inner
                                .condition
                                .wait_timeout(state, std::time::Duration::from_secs_f64(timeout))
                                .expect("watch scheduler wait should succeed");
                            state = next_state;
                        } else {
                            state = self
                                .inner
                                .condition
                                .wait(state)
                                .expect("watch scheduler wait should succeed");
                        }
                        drop(state);
                        continue;
                    }
                    self.inner.condition.notify_all();
                }
            }
            if let Some((callback, pending, payload)) = recovered_callback.take() {
                callback(pending, payload);
            }
            if let Some((callback, pending)) = exhausted_callback.take() {
                callback(pending);
            }
        }
    }

    fn next_wait_timeout_locked(&self, state: &SchedulerState<P, T>, now: f64) -> f64 {
        next_wait_timeout(
            &state
                .watches
                .values()
                .map(|watch| watch.kernel_state.clone())
                .collect::<Vec<_>>(),
            now,
        )
    }

    fn run_watch_attempt(&self, watch_id: String) {
        let watch = {
            let state = self
                .inner
                .state
                .lock()
                .expect("watch scheduler mutex poisoned");
            state.watches.get(&watch_id).cloned()
        };
        let Some(watch) = watch else {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("watch scheduler mutex poisoned");
            state.active_attempts = state.active_attempts.saturating_sub(1);
            self.inner.condition.notify_all();
            return;
        };

        let result = watch
            .runtime
            .recover_completed_pending_reply_result(&watch.pending);
        let mut recovered_callback: Option<(Arc<RecoveredCallback<P, T>>, P, Option<T>)> = None;
        let mut exhausted_callback: Option<(Arc<ExhaustedCallback<P>>, P)> = None;
        {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("watch scheduler mutex poisoned");
            if let Some(current) = state.watches.get_mut(&watch_id) {
                let now = (self.inner.monotonic)();
                let resolution = resolve_watch_attempt(
                    &current.kernel_state,
                    now,
                    result.outcome.as_str(),
                    current.runtime.still_pending(&current.pending),
                    current
                        .runtime
                        .watch_poll_interval_seconds(&current.pending),
                );
                if resolution.remove_watch {
                    let finished = state
                        .watches
                        .remove(&watch_id)
                        .expect("scheduled watch should still exist");
                    match resolution.callback_kind.as_deref() {
                        Some("recovered") => {
                            if let Some(callback) = finished.on_recovered {
                                recovered_callback =
                                    Some((callback, finished.pending, result.payload));
                            }
                        }
                        Some("exhausted") => {
                            if let Some(callback) = finished.on_exhausted {
                                exhausted_callback = Some((callback, finished.pending));
                            }
                        }
                        _ => {}
                    }
                } else if let Some(next_state) = resolution.state {
                    current.kernel_state = next_state;
                }
            }
        }
        if let Some((callback, pending, payload)) = recovered_callback {
            callback(pending, payload);
        }
        if let Some((callback, pending)) = exhausted_callback {
            callback(pending);
        }
        let mut state = self
            .inner
            .state
            .lock()
            .expect("watch scheduler mutex poisoned");
        state.active_attempts = state.active_attempts.saturating_sub(1);
        self.inner.condition.notify_all();
    }
}

pub fn build_watch_state(
    watch_id: &str,
    now: f64,
    max_wait_seconds: f64,
    poll_interval_seconds: f64,
) -> DeferredReplyScheduledWatchState {
    let normalized_max_wait = max_wait_seconds.max(0.0);
    let normalized_poll_interval = poll_interval_seconds.max(0.0);
    DeferredReplyScheduledWatchState {
        kernel_version: DEFERRED_REPLY_SCHEDULER_KERNEL_VERSION.to_string(),
        watch_id: watch_id.to_string(),
        deadline_at: now + normalized_max_wait,
        next_poll_at: now + normalized_poll_interval.min(normalized_max_wait),
        inflight: false,
    }
}

pub fn watch_is_due(state: &DeferredReplyScheduledWatchState, now: f64) -> bool {
    !state.inflight && state.next_poll_at <= now
}

pub fn watch_is_exhausted(
    state: &DeferredReplyScheduledWatchState,
    now: f64,
    still_pending: bool,
) -> bool {
    !state.inflight && (state.deadline_at <= now || !still_pending)
}

pub fn mark_watch_inflight(
    state: &DeferredReplyScheduledWatchState,
) -> DeferredReplyScheduledWatchState {
    let mut updated = state.clone();
    updated.inflight = true;
    updated
}

pub fn next_wait_timeout(states: &[DeferredReplyScheduledWatchState], now: f64) -> f64 {
    let next_time = states
        .iter()
        .filter(|state| !state.inflight)
        .map(|state| state.next_poll_at.min(state.deadline_at))
        .min_by(|left, right| left.total_cmp(right));
    match next_time {
        Some(deadline) => (deadline - now).max(0.0),
        None => 0.1,
    }
}

pub fn resolve_watch_attempt(
    state: &DeferredReplyScheduledWatchState,
    now: f64,
    outcome: &str,
    still_pending: bool,
    next_poll_interval_seconds: f64,
) -> DeferredReplyScheduledWatchResolution {
    let normalized_outcome = outcome.trim().to_lowercase();
    if normalized_outcome == "recovered" {
        return DeferredReplyScheduledWatchResolution {
            kernel_version: DEFERRED_REPLY_SCHEDULER_KERNEL_VERSION.to_string(),
            remove_watch: true,
            callback_kind: Some("recovered".to_string()),
            state: None,
        };
    }
    if normalized_outcome == "terminal" {
        return DeferredReplyScheduledWatchResolution {
            kernel_version: DEFERRED_REPLY_SCHEDULER_KERNEL_VERSION.to_string(),
            remove_watch: true,
            callback_kind: Some("exhausted".to_string()),
            state: None,
        };
    }
    if now >= state.deadline_at || !still_pending {
        return DeferredReplyScheduledWatchResolution {
            kernel_version: DEFERRED_REPLY_SCHEDULER_KERNEL_VERSION.to_string(),
            remove_watch: true,
            callback_kind: Some("exhausted".to_string()),
            state: None,
        };
    }
    let mut updated = state.clone();
    updated.inflight = false;
    updated.next_poll_at = now + next_poll_interval_seconds.max(0.0);
    DeferredReplyScheduledWatchResolution {
        kernel_version: DEFERRED_REPLY_SCHEDULER_KERNEL_VERSION.to_string(),
        remove_watch: false,
        callback_kind: None,
        state: Some(updated),
    }
}
