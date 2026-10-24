use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{
    execute_with_telegram_service_cycle_ports, TelegramServiceCycleBackgroundSyncPort,
    TelegramServiceCycleDispatchPort, TelegramServiceCyclePollPort, TelegramServiceCycleStatePort,
};

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramServiceRunExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_service_run_execution";
const SERVICE_CYCLE_CONTRACT: &str = "ait_agent_core.event_loop.TelegramServiceCycleExecution.v1";
const SERVICE_CYCLE_MIGRATION_STAGE: &str = "rust_agent_telegram_service_cycle_execution";
const DEFAULT_RETRY_BACKOFF_SECONDS: f64 = 1.0;
const MAX_RETRY_BACKOFF_SECONDS: f64 = 60.0;
const HARD_MAX_CYCLES: u64 = 1_000_000;

pub trait TelegramServiceRunCyclePort {
    fn execute_cycle(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait TelegramServiceRunClockPort {
    fn monotonic_seconds(&self) -> Result<f64, String>;
}

pub trait TelegramServiceRunStopPort {
    fn stop_requested(&self) -> Result<bool, String>;
}

pub trait TelegramServiceRunSleepPort {
    fn sleep_seconds(&self, seconds: f64) -> Result<(), String>;
}

pub struct TelegramServiceRunCycleExecutor<'a, S: ?Sized, P: ?Sized, D: ?Sized, B: ?Sized> {
    state: &'a S,
    poll: &'a P,
    dispatch: &'a D,
    background_sync: &'a B,
}

impl<'a, S: ?Sized, P: ?Sized, D: ?Sized, B: ?Sized>
    TelegramServiceRunCycleExecutor<'a, S, P, D, B>
{
    pub fn new(state: &'a S, poll: &'a P, dispatch: &'a D, background_sync: &'a B) -> Self {
        Self {
            state,
            poll,
            dispatch,
            background_sync,
        }
    }
}

impl<S, P, D, B> TelegramServiceRunCyclePort for TelegramServiceRunCycleExecutor<'_, S, P, D, B>
where
    S: TelegramServiceCycleStatePort + ?Sized,
    P: TelegramServiceCyclePollPort + ?Sized,
    D: TelegramServiceCycleDispatchPort + ?Sized,
    B: TelegramServiceCycleBackgroundSyncPort + ?Sized,
{
    fn execute_cycle(&self, request: &JsonValue) -> Result<JsonValue, String> {
        execute_with_telegram_service_cycle_ports(
            self.state,
            self.poll,
            self.dispatch,
            self.background_sync,
            request,
        )
    }
}

pub fn execute_with_telegram_service_run_ports<C, K, T, S>(
    cycle: &C,
    clock: &K,
    stop: &T,
    sleeper: &S,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    C: TelegramServiceRunCyclePort + ?Sized,
    K: TelegramServiceRunClockPort + ?Sized,
    T: TelegramServiceRunStopPort + ?Sized,
    S: TelegramServiceRunSleepPort + ?Sized,
{
    let input = RunInput::parse(request)?;
    let mut progress = RunProgress {
        next_background_sync_at: input.initial_next_background_sync_at,
        ..RunProgress::default()
    };

    loop {
        match stop.stop_requested() {
            Ok(true) => return Ok(run_payload(&input, &progress, Terminal::graceful_stop())),
            Ok(false) => {}
            Err(_) => {
                progress.fatal_failure_count += 1;
                return Ok(run_payload(
                    &input,
                    &progress,
                    Terminal::failed(
                        "stop_observation_failed",
                        "control",
                        "Telegram service-run stop observation failed.",
                    ),
                ));
            }
        }

        let now_monotonic_seconds = match clock.monotonic_seconds() {
            Ok(value) if value.is_finite() && value >= 0.0 => value,
            Ok(_) | Err(_) => {
                progress.fatal_failure_count += 1;
                return Ok(run_payload(
                    &input,
                    &progress,
                    Terminal::failed(
                        "clock_observation_failed",
                        "clock",
                        "Telegram service-run monotonic clock observation failed.",
                    ),
                ));
            }
        };
        let cycle_request =
            input.cycle_request(now_monotonic_seconds, progress.next_background_sync_at);
        progress.cycle_count += 1;
        let raw_cycle = match cycle.execute_cycle(&cycle_request) {
            Ok(value) => value,
            Err(_) => {
                progress.fatal_failure_count += 1;
                return Ok(run_payload(
                    &input,
                    &progress,
                    Terminal::failed(
                        "cycle_execution_failed",
                        "cycle",
                        "Telegram service-run cycle execution failed.",
                    ),
                ));
            }
        };
        let cycle = match CycleSummary::parse(&raw_cycle) {
            Ok(cycle) => cycle,
            Err(_) => {
                progress.fatal_failure_count += 1;
                return Ok(run_payload(
                    &input,
                    &progress,
                    Terminal::failed(
                        "cycle_contract_invalid",
                        "contract",
                        "Telegram service-run cycle contract validation failed.",
                    ),
                ));
            }
        };
        progress.observe(&cycle);

        if cycle.ok {
            progress.successful_cycle_count += 1;
            if cycle.state == "empty_poll" {
                progress.empty_poll_count += 1;
            }
            if input.limit_reached(progress.cycle_count) {
                return Ok(run_payload(&input, &progress, Terminal::bounded_limit()));
            }
            continue;
        }

        if !is_retryable_error(cycle.error_kind.as_deref()) {
            progress.fatal_failure_count += 1;
            return Ok(run_payload(
                &input,
                &progress,
                Terminal::failed(
                    "fatal_cycle_failure",
                    "contract",
                    "Telegram service-run encountered a fatal cycle failure.",
                ),
            ));
        }

        progress.retryable_failure_count += 1;
        if input.limit_reached(progress.cycle_count) {
            return Ok(run_payload(&input, &progress, Terminal::bounded_limit()));
        }
        match stop.stop_requested() {
            Ok(true) => return Ok(run_payload(&input, &progress, Terminal::graceful_stop())),
            Ok(false) => {}
            Err(_) => {
                progress.fatal_failure_count += 1;
                return Ok(run_payload(
                    &input,
                    &progress,
                    Terminal::failed(
                        "stop_observation_failed",
                        "control",
                        "Telegram service-run stop observation failed.",
                    ),
                ));
            }
        }
        if sleeper.sleep_seconds(input.retry_backoff_seconds).is_err() {
            progress.fatal_failure_count += 1;
            return Ok(run_payload(
                &input,
                &progress,
                Terminal::failed(
                    "retry_sleep_failed",
                    "sleep",
                    "Telegram service-run retry sleep failed.",
                ),
            ));
        }
        progress.retry_sleep_count += 1;
        progress.retry_sleep_seconds += input.retry_backoff_seconds;
    }
}

fn is_retryable_error(error_kind: Option<&str>) -> bool {
    matches!(
        error_kind,
        Some("state" | "poll" | "dispatch" | "background_sync")
    )
}

#[derive(Default)]
struct RunProgress {
    cycle_count: u64,
    successful_cycle_count: u64,
    empty_poll_count: u64,
    update_count: u64,
    dispatched_count: u64,
    background_sync_run_count: u64,
    retryable_failure_count: u64,
    fatal_failure_count: u64,
    retry_sleep_count: u64,
    retry_sleep_seconds: f64,
    last_cycle_ok: Option<bool>,
    last_service_cycle_state: Option<String>,
    next_background_sync_at: Option<f64>,
}

impl RunProgress {
    fn observe(&mut self, cycle: &CycleSummary) {
        self.update_count = self.update_count.saturating_add(cycle.update_count);
        self.dispatched_count = self.dispatched_count.saturating_add(cycle.dispatched_count);
        self.background_sync_run_count = self
            .background_sync_run_count
            .saturating_add(u64::from(cycle.background_sync_ran));
        self.last_cycle_ok = Some(cycle.ok);
        self.last_service_cycle_state = Some(cycle.state.clone());
        self.next_background_sync_at = cycle.next_background_sync_at;
    }
}

struct CycleSummary {
    ok: bool,
    state: String,
    error_kind: Option<String>,
    update_count: u64,
    dispatched_count: u64,
    background_sync_ran: bool,
    next_background_sync_at: Option<f64>,
}

impl CycleSummary {
    fn parse(value: &JsonValue) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "Telegram service cycle result must be an object.".to_string())?;
        if clean_text(object.get("contract")).as_deref() != Some(SERVICE_CYCLE_CONTRACT) {
            return Err("Telegram service cycle result contract is invalid.".to_string());
        }
        if clean_text(object.get("migration_stage")).as_deref()
            != Some(SERVICE_CYCLE_MIGRATION_STAGE)
            || clean_text(object.get("stage")).as_deref() != Some("execute")
        {
            return Err("Telegram service cycle result identity is invalid.".to_string());
        }
        let ok = object
            .get("ok")
            .and_then(JsonValue::as_bool)
            .ok_or_else(|| "Telegram service cycle result ok flag is required.".to_string())?;
        if object.get("completed").and_then(JsonValue::as_bool) != Some(ok) {
            return Err("Telegram service cycle completed flag is invalid.".to_string());
        }
        for key in [
            "python_service_loop_allowed",
            "python_callback_execution_allowed",
            "python_state_mutation_allowed",
            "python_telegram_api_allowed",
            "python_update_dispatch_allowed",
            "python_background_sync_allowed",
        ] {
            if object.get(key).and_then(JsonValue::as_bool) != Some(false) {
                return Err(format!(
                    "Telegram service cycle result `{key}` must be false."
                ));
            }
        }
        let state = clean_text(object.get("service_cycle_state")).ok_or_else(|| {
            "Telegram service cycle result service_cycle_state is required.".to_string()
        })?;
        let error_kind = optional_clean_text(object.get("error_kind"))?;
        if ok && error_kind.is_some() {
            return Err("Successful Telegram service cycle cannot contain error_kind.".to_string());
        }
        if !ok && error_kind.is_none() {
            return Err("Failed Telegram service cycle must contain error_kind.".to_string());
        }
        let update_count = required_u64(object, "update_count")?;
        let dispatched_count = required_u64(object, "dispatched_count")?;
        if dispatched_count > update_count {
            return Err(
                "Telegram service cycle dispatched_count cannot exceed update_count.".to_string(),
            );
        }
        Ok(Self {
            ok,
            state,
            error_kind,
            update_count,
            dispatched_count,
            background_sync_ran: object
                .get("background_sync_ran")
                .and_then(JsonValue::as_bool)
                .ok_or_else(|| {
                    "Telegram service cycle background_sync_ran flag is required.".to_string()
                })?,
            next_background_sync_at: optional_f64_field(object, "next_background_sync_at")?,
        })
    }
}

struct RunInput {
    cycle_request: Map<String, JsonValue>,
    retry_backoff_seconds: f64,
    max_cycles: Option<u64>,
    initial_next_background_sync_at: Option<f64>,
}

impl RunInput {
    fn parse(request: &JsonValue) -> Result<Self, String> {
        let object = request
            .as_object()
            .ok_or_else(|| "Telegram service-run request must be an object.".to_string())?;
        let retry_backoff_seconds = optional_f64_field(object, "retry_backoff_seconds")?
            .unwrap_or(DEFAULT_RETRY_BACKOFF_SECONDS);
        if !retry_backoff_seconds.is_finite()
            || !(0.0..=MAX_RETRY_BACKOFF_SECONDS).contains(&retry_backoff_seconds)
        {
            return Err(format!(
                "Telegram service-run retry_backoff_seconds must be finite and between 0 and {MAX_RETRY_BACKOFF_SECONDS}."
            ));
        }
        let max_cycles = optional_u64_field(object, "max_cycles")?;
        if max_cycles == Some(0) || max_cycles.is_some_and(|value| value > HARD_MAX_CYCLES) {
            return Err(format!(
                "Telegram service-run max_cycles must be between 1 and {HARD_MAX_CYCLES}."
            ));
        }
        let initial_next_background_sync_at =
            optional_f64_field(object, "next_background_sync_at")?;
        let mut cycle_request = object.clone();
        cycle_request.remove("retry_backoff_seconds");
        cycle_request.remove("max_cycles");
        cycle_request.remove("now_monotonic_seconds");
        cycle_request.remove("next_background_sync_at");
        Ok(Self {
            cycle_request,
            retry_backoff_seconds,
            max_cycles,
            initial_next_background_sync_at,
        })
    }

    fn cycle_request(
        &self,
        now_monotonic_seconds: f64,
        next_background_sync_at: Option<f64>,
    ) -> JsonValue {
        let mut request = self.cycle_request.clone();
        request.insert(
            "now_monotonic_seconds".to_string(),
            finite_f64_json(now_monotonic_seconds),
        );
        request.insert(
            "next_background_sync_at".to_string(),
            optional_f64_json(next_background_sync_at),
        );
        JsonValue::Object(request)
    }

    fn limit_reached(&self, cycle_count: u64) -> bool {
        self.max_cycles
            .is_some_and(|max_cycles| cycle_count >= max_cycles)
    }
}

struct Terminal<'a> {
    state: &'a str,
    stop_reason: &'a str,
    ok: bool,
    graceful_stop: bool,
    bounded_cycle_limit_reached: bool,
    error_kind: Option<&'a str>,
    error: Option<&'a str>,
}

impl Terminal<'static> {
    fn graceful_stop() -> Self {
        Self {
            state: "stopped",
            stop_reason: "stop_requested",
            ok: true,
            graceful_stop: true,
            bounded_cycle_limit_reached: false,
            error_kind: None,
            error: None,
        }
    }

    fn bounded_limit() -> Self {
        Self {
            state: "bounded_cycle_limit_reached",
            stop_reason: "max_cycles_reached",
            ok: true,
            graceful_stop: false,
            bounded_cycle_limit_reached: true,
            error_kind: None,
            error: None,
        }
    }

    fn failed(stop_reason: &'static str, error_kind: &'static str, error: &'static str) -> Self {
        Self {
            state: "failed_closed",
            stop_reason,
            ok: false,
            graceful_stop: false,
            bounded_cycle_limit_reached: false,
            error_kind: Some(error_kind),
            error: Some(error),
        }
    }
}

fn run_payload(input: &RunInput, progress: &RunProgress, terminal: Terminal<'_>) -> JsonValue {
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "run",
        "service_run_state": terminal.state,
        "stop_reason": terminal.stop_reason,
        "ok": terminal.ok,
        "completed": true,
        "graceful_stop": terminal.graceful_stop,
        "production_stop_observed": terminal.graceful_stop,
        "bounded_cycle_limit_reached": terminal.bounded_cycle_limit_reached,
        "unbounded_run_requested": input.max_cycles.is_none(),
        "configured_max_cycles": input.max_cycles.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "retry_backoff_seconds": finite_f64_json(input.retry_backoff_seconds),
        "cycle_count": progress.cycle_count,
        "successful_cycle_count": progress.successful_cycle_count,
        "empty_poll_count": progress.empty_poll_count,
        "update_count": progress.update_count,
        "dispatched_count": progress.dispatched_count,
        "background_sync_run_count": progress.background_sync_run_count,
        "retryable_failure_count": progress.retryable_failure_count,
        "fatal_failure_count": progress.fatal_failure_count,
        "retry_sleep_count": progress.retry_sleep_count,
        "retry_sleep_seconds": finite_f64_json(progress.retry_sleep_seconds),
        "last_cycle_ok": progress.last_cycle_ok.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "last_service_cycle_state": progress.last_service_cycle_state.as_deref().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "next_background_sync_at": optional_f64_json(progress.next_background_sync_at),
        "error_kind": terminal.error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "error": terminal.error.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "python_service_loop_allowed": false,
        "python_cycle_execution_allowed": false,
        "python_retry_sleep_allowed": false,
        "python_stop_control_allowed": false,
        "python_monotonic_clock_allowed": false,
    })
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn optional_clean_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) if value.trim().is_empty() => Ok(None),
        Some(JsonValue::String(value)) => Ok(Some(value.trim().to_string())),
        Some(_) => Err("Telegram service cycle error_kind must be text or null.".to_string()),
    }
}

fn required_u64(object: &Map<String, JsonValue>, key: &str) -> Result<u64, String> {
    optional_u64(object.get(key))
        .ok_or_else(|| format!("Telegram service cycle {key} must be a non-negative integer."))
}

fn optional_u64_field(object: &Map<String, JsonValue>, key: &str) -> Result<Option<u64>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => optional_u64(Some(value))
            .map(Some)
            .ok_or_else(|| format!("Telegram service-run {key} must be a non-negative integer.")),
    }
}

fn optional_u64(value: Option<&JsonValue>) -> Option<u64> {
    match value? {
        JsonValue::Number(value) => value.as_u64(),
        JsonValue::String(value) => value.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn optional_f64_field(object: &Map<String, JsonValue>, key: &str) -> Result<Option<f64>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => {
            let parsed = optional_f64(Some(value)).ok_or_else(|| {
                format!("Telegram service-run {key} must be a finite non-negative number or null.")
            })?;
            if !parsed.is_finite() || parsed < 0.0 {
                return Err(format!(
                    "Telegram service-run {key} must be a finite non-negative number or null."
                ));
            }
            Ok(Some(parsed))
        }
    }
}

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(value) => value.as_f64(),
        JsonValue::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn finite_f64_json(value: f64) -> JsonValue {
    ait_core::json_support::JsonNumber::from_f64(value)
        .map(JsonValue::Number)
        .unwrap_or(JsonValue::Null)
}

fn optional_f64_json(value: Option<f64>) -> JsonValue {
    value.map(finite_f64_json).unwrap_or(JsonValue::Null)
}

#[cfg(test)]
mod tests;
