use std::cell::RefCell;
use std::collections::VecDeque;

use ait_core::json_support::{json, JsonValue};

use super::*;

struct ScriptedCycle {
    results: RefCell<VecDeque<Result<JsonValue, String>>>,
    requests: RefCell<Vec<JsonValue>>,
}

impl ScriptedCycle {
    fn new(results: Vec<Result<JsonValue, String>>) -> Self {
        Self {
            results: RefCell::new(results.into()),
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramServiceRunCyclePort for ScriptedCycle {
    fn execute_cycle(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.borrow_mut().push(request.clone());
        self.results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("unexpected-cycle-secret".to_string()))
    }
}

struct ScriptedRuntime {
    times: RefCell<VecDeque<Result<f64, String>>>,
    stop_results: RefCell<VecDeque<Result<bool, String>>>,
    sleep_results: RefCell<VecDeque<Result<(), String>>>,
    sleeps: RefCell<Vec<f64>>,
}

impl ScriptedRuntime {
    fn new(times: Vec<Result<f64, String>>, stop_results: Vec<Result<bool, String>>) -> Self {
        Self {
            times: RefCell::new(times.into()),
            stop_results: RefCell::new(stop_results.into()),
            sleep_results: RefCell::new(VecDeque::new()),
            sleeps: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramServiceRunClockPort for ScriptedRuntime {
    fn monotonic_seconds(&self) -> Result<f64, String> {
        self.times
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("unexpected-clock-secret".to_string()))
    }
}

impl TelegramServiceRunStopPort for ScriptedRuntime {
    fn stop_requested(&self) -> Result<bool, String> {
        self.stop_results
            .borrow_mut()
            .pop_front()
            .unwrap_or(Ok(false))
    }
}

impl TelegramServiceRunSleepPort for ScriptedRuntime {
    fn sleep_seconds(&self, seconds: f64) -> Result<(), String> {
        self.sleeps.borrow_mut().push(seconds);
        self.sleep_results
            .borrow_mut()
            .pop_front()
            .unwrap_or(Ok(()))
    }
}

fn request(max_cycles: u64) -> JsonValue {
    json!({
        "state_path": "/tmp/telegram-service-run-state.json",
        "poll_timeout_seconds": 30,
        "background_sync_enabled": true,
        "background_sync_interval_seconds": 60.0,
        "retry_backoff_seconds": 1.0,
        "max_cycles": max_cycles,
    })
}

fn cycle_result(
    state: &str,
    ok: bool,
    error_kind: Option<&str>,
    next_background_sync_at: Option<f64>,
    update_count: u64,
    dispatched_count: u64,
    background_sync_ran: bool,
) -> JsonValue {
    json!({
        "contract": SERVICE_CYCLE_CONTRACT,
        "migration_stage": SERVICE_CYCLE_MIGRATION_STAGE,
        "stage": "execute",
        "service_cycle_state": state,
        "ok": ok,
        "completed": ok,
        "error_kind": error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "update_count": update_count,
        "dispatched_count": dispatched_count,
        "background_sync_ran": background_sync_ran,
        "next_background_sync_at": optional_f64_json(next_background_sync_at),
        "python_service_loop_allowed": false,
        "python_callback_execution_allowed": false,
        "python_state_mutation_allowed": false,
        "python_telegram_api_allowed": false,
        "python_update_dispatch_allowed": false,
        "python_background_sync_allowed": false,
    })
}

fn execute(cycle: &ScriptedCycle, runtime: &ScriptedRuntime, request: &JsonValue) -> JsonValue {
    execute_with_telegram_service_run_ports(cycle, runtime, runtime, runtime, request)
        .expect("service-run execution")
}

#[test]
fn graceful_stop_before_first_cycle_does_not_observe_clock_or_sleep() {
    let cycle = ScriptedCycle::new(Vec::new());
    let runtime = ScriptedRuntime::new(Vec::new(), vec![Ok(true)]);

    let outcome = execute(&cycle, &runtime, &request(3));

    assert_eq!(outcome["contract"], CONTRACT);
    assert_eq!(outcome["service_run_state"], "stopped");
    assert_eq!(outcome["stop_reason"], "stop_requested");
    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["graceful_stop"], true);
    assert_eq!(outcome["cycle_count"], 0);
    assert_eq!(outcome["production_stop_observed"], true);
    assert!(cycle.requests.borrow().is_empty());
    assert!(runtime.times.borrow().is_empty());
    assert!(runtime.sleeps.borrow().is_empty());
}

#[test]
fn graceful_stop_between_successful_cycles_preserves_last_public_deadline() {
    let cycle = ScriptedCycle::new(vec![Ok(cycle_result(
        "empty_poll",
        true,
        None,
        Some(66.0),
        0,
        0,
        false,
    ))]);
    let runtime = ScriptedRuntime::new(vec![Ok(6.0)], vec![Ok(false), Ok(true)]);

    let outcome = execute(&cycle, &runtime, &request(3));

    assert_eq!(outcome["service_run_state"], "stopped");
    assert_eq!(outcome["graceful_stop"], true);
    assert_eq!(outcome["cycle_count"], 1);
    assert_eq!(outcome["successful_cycle_count"], 1);
    assert_eq!(outcome["empty_poll_count"], 1);
    assert_eq!(outcome["next_background_sync_at"], 66.0);
    assert_eq!(cycle.requests.borrow().len(), 1);
    assert!(runtime.sleeps.borrow().is_empty());
}

#[test]
fn successful_and_empty_cycles_carry_deadline_and_finish_at_explicit_limit() {
    let cycle = ScriptedCycle::new(vec![
        Ok(cycle_result(
            "empty_poll",
            true,
            None,
            Some(70.0),
            0,
            0,
            false,
        )),
        Ok(cycle_result(
            "completed",
            true,
            None,
            Some(70.0),
            2,
            2,
            true,
        )),
    ]);
    let runtime = ScriptedRuntime::new(vec![Ok(10.0), Ok(11.5)], vec![Ok(false), Ok(false)]);

    let outcome = execute(&cycle, &runtime, &request(2));

    assert_eq!(outcome["service_run_state"], "bounded_cycle_limit_reached");
    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["bounded_cycle_limit_reached"], true);
    assert_eq!(outcome["production_stop_observed"], false);
    assert_eq!(outcome["cycle_count"], 2);
    assert_eq!(outcome["successful_cycle_count"], 2);
    assert_eq!(outcome["empty_poll_count"], 1);
    assert_eq!(outcome["update_count"], 2);
    assert_eq!(outcome["dispatched_count"], 2);
    assert_eq!(outcome["background_sync_run_count"], 1);
    assert_eq!(outcome["next_background_sync_at"], 70.0);
    assert_eq!(cycle.requests.borrow()[0]["now_monotonic_seconds"], 10.0);
    assert_eq!(
        cycle.requests.borrow()[0]["next_background_sync_at"],
        JsonValue::Null
    );
    assert_eq!(cycle.requests.borrow()[1]["now_monotonic_seconds"], 11.5);
    assert_eq!(cycle.requests.borrow()[1]["next_background_sync_at"], 70.0);
    assert!(runtime.sleeps.borrow().is_empty());
}

#[test]
fn retryable_failure_carries_deadline_and_sleeps_exactly_once_before_recovery() {
    let cycle = ScriptedCycle::new(vec![
        Ok(cycle_result(
            "poll_failed",
            false,
            Some("poll"),
            Some(75.0),
            0,
            0,
            false,
        )),
        Ok(cycle_result(
            "empty_poll",
            true,
            None,
            Some(75.0),
            0,
            0,
            false,
        )),
    ]);
    let runtime = ScriptedRuntime::new(
        vec![Ok(12.0), Ok(13.0)],
        vec![Ok(false), Ok(false), Ok(false)],
    );

    let outcome = execute(&cycle, &runtime, &request(2));

    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["cycle_count"], 2);
    assert_eq!(outcome["successful_cycle_count"], 1);
    assert_eq!(outcome["retryable_failure_count"], 1);
    assert_eq!(outcome["retry_sleep_count"], 1);
    assert_eq!(outcome["retry_sleep_seconds"], 1.0);
    assert_eq!(&*runtime.sleeps.borrow(), &[1.0]);
    assert_eq!(cycle.requests.borrow()[1]["next_background_sync_at"], 75.0);
}

#[test]
fn stop_observed_after_retryable_failure_skips_retry_sleep() {
    let cycle = ScriptedCycle::new(vec![Ok(cycle_result(
        "dispatch_failed",
        false,
        Some("dispatch"),
        Some(88.0),
        2,
        1,
        false,
    ))]);
    let runtime = ScriptedRuntime::new(vec![Ok(20.0)], vec![Ok(false), Ok(true)]);

    let outcome = execute(&cycle, &runtime, &request(4));

    assert_eq!(outcome["service_run_state"], "stopped");
    assert_eq!(outcome["graceful_stop"], true);
    assert_eq!(outcome["retryable_failure_count"], 1);
    assert_eq!(outcome["retry_sleep_count"], 0);
    assert_eq!(outcome["update_count"], 2);
    assert_eq!(outcome["dispatched_count"], 1);
    assert_eq!(outcome["next_background_sync_at"], 88.0);
    assert!(runtime.sleeps.borrow().is_empty());
}

#[test]
fn invalid_cycle_contract_is_fatal_and_never_retries_or_leaks_payload() {
    let cycle = ScriptedCycle::new(vec![Ok(json!({
        "contract": "wrong-contract",
        "secret": "bot-token-contract-secret",
    }))]);
    let runtime = ScriptedRuntime::new(vec![Ok(1.0)], vec![Ok(false)]);

    let outcome = execute(&cycle, &runtime, &request(5));

    assert_eq!(outcome["service_run_state"], "failed_closed");
    assert_eq!(outcome["stop_reason"], "cycle_contract_invalid");
    assert_eq!(outcome["error_kind"], "contract");
    assert_eq!(outcome["fatal_failure_count"], 1);
    assert_eq!(outcome["cycle_count"], 1);
    assert!(runtime.sleeps.borrow().is_empty());
    assert!(!outcome.to_string().contains("bot-token-contract-secret"));

    let mut fallback_cycle_result = cycle_result("empty_poll", true, None, Some(10.0), 0, 0, false);
    fallback_cycle_result["python_service_loop_allowed"] = JsonValue::Bool(true);
    fallback_cycle_result["secret"] = json!("bot-token-fallback-secret");
    let fallback_cycle = ScriptedCycle::new(vec![Ok(fallback_cycle_result)]);
    let fallback_runtime = ScriptedRuntime::new(vec![Ok(2.0)], vec![Ok(false)]);
    let fallback_outcome = execute(&fallback_cycle, &fallback_runtime, &request(2));
    assert_eq!(fallback_outcome["stop_reason"], "cycle_contract_invalid");
    assert_eq!(fallback_outcome["python_service_loop_allowed"], false);
    assert!(!fallback_outcome
        .to_string()
        .contains("bot-token-fallback-secret"));
}

#[test]
fn cycle_execution_error_is_fatal_and_secret_safe() {
    let cycle = ScriptedCycle::new(vec![Err("bot-token-cycle-secret".to_string())]);
    let runtime = ScriptedRuntime::new(vec![Ok(1.0)], vec![Ok(false)]);

    let outcome = execute(&cycle, &runtime, &request(2));

    assert_eq!(outcome["stop_reason"], "cycle_execution_failed");
    assert_eq!(outcome["error_kind"], "cycle");
    assert_eq!(outcome["cycle_count"], 1);
    assert_eq!(outcome["fatal_failure_count"], 1);
    assert!(!outcome.to_string().contains("bot-token-cycle-secret"));
    assert!(runtime.sleeps.borrow().is_empty());
}

#[test]
fn stop_and_clock_observation_failures_are_secret_safe_and_prevent_cycle_execution() {
    let stop_cycle = ScriptedCycle::new(Vec::new());
    let stop_runtime = ScriptedRuntime::new(
        Vec::new(),
        vec![Err("shutdown-observation-secret".to_string())],
    );
    let stop_outcome = execute(&stop_cycle, &stop_runtime, &request(2));
    assert_eq!(stop_outcome["stop_reason"], "stop_observation_failed");
    assert_eq!(stop_outcome["error_kind"], "control");
    assert_eq!(stop_outcome["fatal_failure_count"], 1);
    assert_eq!(stop_outcome["cycle_count"], 0);
    assert!(!stop_outcome
        .to_string()
        .contains("shutdown-observation-secret"));

    let clock_cycle = ScriptedCycle::new(Vec::new());
    let clock_runtime =
        ScriptedRuntime::new(vec![Err("clock-secret".to_string())], vec![Ok(false)]);
    let clock_outcome = execute(&clock_cycle, &clock_runtime, &request(2));
    assert_eq!(clock_outcome["stop_reason"], "clock_observation_failed");
    assert_eq!(clock_outcome["error_kind"], "clock");
    assert_eq!(clock_outcome["fatal_failure_count"], 1);
    assert_eq!(clock_outcome["cycle_count"], 0);
    assert!(!clock_outcome.to_string().contains("clock-secret"));
    assert!(stop_cycle.requests.borrow().is_empty());
    assert!(clock_cycle.requests.borrow().is_empty());
}

#[test]
fn retry_sleep_failure_is_fatal_secret_safe_and_not_counted_as_completed_sleep() {
    let cycle = ScriptedCycle::new(vec![Ok(cycle_result(
        "state_load_failed",
        false,
        Some("state"),
        Some(55.0),
        0,
        0,
        false,
    ))]);
    let runtime = ScriptedRuntime::new(vec![Ok(2.0)], vec![Ok(false), Ok(false)]);
    runtime
        .sleep_results
        .borrow_mut()
        .push_back(Err("sleep-secret".to_string()));

    let outcome = execute(&cycle, &runtime, &request(3));

    assert_eq!(outcome["stop_reason"], "retry_sleep_failed");
    assert_eq!(outcome["error_kind"], "sleep");
    assert_eq!(outcome["retryable_failure_count"], 1);
    assert_eq!(outcome["retry_sleep_count"], 0);
    assert_eq!(outcome["fatal_failure_count"], 1);
    assert_eq!(&*runtime.sleeps.borrow(), &[1.0]);
    assert!(!outcome.to_string().contains("sleep-secret"));
}

#[test]
fn invalid_run_limits_are_rejected_before_ports_execute() {
    let cycle = ScriptedCycle::new(Vec::new());
    let runtime = ScriptedRuntime::new(Vec::new(), Vec::new());
    let mut invalid = request(1);
    invalid["max_cycles"] = json!(0);
    invalid["retry_backoff_seconds"] = json!(61.0);

    let error =
        execute_with_telegram_service_run_ports(&cycle, &runtime, &runtime, &runtime, &invalid)
            .unwrap_err();

    assert!(error.contains("retry_backoff_seconds"));
    assert!(cycle.requests.borrow().is_empty());
    assert!(runtime.sleeps.borrow().is_empty());
}

struct EmptyState;

impl TelegramServiceCycleStatePort for EmptyState {
    fn execute_state(
        &self,
        _path: &str,
        operation: &str,
        _request: &JsonValue,
    ) -> Result<JsonValue, String> {
        match operation {
            "load" => Ok(json!({"last_update_id": 3})),
            _ => Err("unexpected state operation".to_string()),
        }
    }
}

struct EmptyPoll;

impl TelegramServiceCyclePollPort for EmptyPoll {
    fn poll_updates(&self, _request: &JsonValue) -> Result<Vec<JsonValue>, String> {
        Ok(Vec::new())
    }
}

struct NoDispatch;

impl TelegramServiceCycleDispatchPort for NoDispatch {
    fn dispatch_update(&self, _request: &JsonValue) -> Result<(), String> {
        Err("dispatch must not run".to_string())
    }
}

struct NoBackground;

impl TelegramServiceCycleBackgroundSyncPort for NoBackground {
    fn run_background_sync_once(&self, _request: &JsonValue) -> Result<usize, String> {
        Err("background must not run".to_string())
    }
}

#[test]
fn concrete_cycle_executor_delegates_to_existing_service_cycle_transaction() {
    let state = EmptyState;
    let poll = EmptyPoll;
    let dispatch = NoDispatch;
    let background = NoBackground;
    let executor = TelegramServiceRunCycleExecutor::new(&state, &poll, &dispatch, &background);

    let outcome = executor
        .execute_cycle(&json!({
            "state_path": "/tmp/telegram-service-run-adapter.json",
            "poll_timeout_seconds": 9,
            "background_sync_enabled": false,
            "background_sync_interval_seconds": 60.0,
            "now_monotonic_seconds": 5.0,
            "next_background_sync_at": null,
        }))
        .expect("cycle adapter");

    assert_eq!(outcome["contract"], SERVICE_CYCLE_CONTRACT);
    assert_eq!(outcome["service_cycle_state"], "empty_poll");
    assert_eq!(outcome["last_update_id_before"], 3);
}
