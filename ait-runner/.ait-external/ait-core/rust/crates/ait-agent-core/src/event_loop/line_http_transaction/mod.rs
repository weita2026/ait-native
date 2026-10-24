use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{agent_line_event_job_execute_json, agent_line_webhook_ingress_plan_json};

const MIGRATION_STAGE: &str = "rust_agent_line_http_transaction";
const LINE_HTTP_TRANSACTION_CONTRACT: &str = "ait_agent_core.event_loop.LineHttpTransaction.v1";
const REDACTED: &str = "[redacted]";

pub trait LineHttpTransactionIngressPort {
    fn plan_ingress(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait LineHttpTransactionEventJobPort {
    fn execute_event_job(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultLineHttpTransactionIngressPort;

impl LineHttpTransactionIngressPort for DefaultLineHttpTransactionIngressPort {
    fn plan_ingress(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_line_webhook_ingress_plan_json(request)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultLineHttpTransactionEventJobPort;

impl LineHttpTransactionEventJobPort for DefaultLineHttpTransactionEventJobPort {
    fn execute_event_job(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_line_event_job_execute_json(request)
    }
}

pub fn agent_line_http_transaction_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    execute_with_line_http_transaction_ports(
        &DefaultLineHttpTransactionIngressPort,
        &DefaultLineHttpTransactionEventJobPort,
        request,
    )
}

pub fn execute_with_line_http_transaction_ports<I, E>(
    ingress: &I,
    event_jobs: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    I: LineHttpTransactionIngressPort + ?Sized,
    E: LineHttpTransactionEventJobPort + ?Sized,
{
    let request = request_object(request)?;
    let secrets = request_secrets(request);
    let ingress_request = ingress_request(request);
    let planned = match ingress.plan_ingress(&ingress_request) {
        Ok(value) => value,
        Err(_) => {
            return Ok(TransactionOutcome::failure(
                "ingress_failed",
                400,
                "ingress",
                "LINE webhook ingress failed.",
            )
            .payload())
        }
    };
    let Some(planned) = planned.as_object() else {
        return Ok(TransactionOutcome::failure(
            "ingress_contract_invalid",
            500,
            "ingress_contract",
            "LINE webhook ingress returned an invalid payload.",
        )
        .payload());
    };
    let should_handle = match planned
        .get("should_handle_webhook")
        .and_then(JsonValue::as_bool)
    {
        Some(value) => value,
        None => {
            return Ok(TransactionOutcome::failure(
                "ingress_contract_invalid",
                500,
                "ingress_contract",
                "LINE webhook ingress returned an invalid payload.",
            )
            .payload())
        }
    };
    if !should_handle {
        return Ok(rejected_ingress_outcome(planned, &secrets).payload());
    }

    let event_plans = match planned.get("event_plans").and_then(JsonValue::as_array) {
        Some(value) => value,
        None => {
            return Ok(TransactionOutcome::failure(
                "ingress_contract_invalid",
                500,
                "ingress_contract",
                "LINE webhook ingress returned an invalid event-plan list.",
            )
            .payload())
        }
    };
    let mut counts = TransactionCounts {
        planned: event_plans.len(),
        ..TransactionCounts::default()
    };
    for event_plan in event_plans {
        let Some(event_plan_object) = event_plan.as_object() else {
            return Ok(transaction_contract_failure(
                "event_plan_invalid",
                counts,
                "LINE webhook ingress returned an invalid event plan.",
            )
            .payload());
        };
        let should_submit = match event_plan_object
            .get("should_submit_turn")
            .and_then(JsonValue::as_bool)
        {
            Some(value) => value,
            None => {
                return Ok(transaction_contract_failure(
                    "event_plan_invalid",
                    counts,
                    "LINE webhook ingress returned an invalid event plan.",
                )
                .payload())
            }
        };
        if !should_submit {
            counts.ignored += 1;
            continue;
        }
        counts.submitted += 1;
        counts.attempted += 1;
        let event_request = event_job_request(request, event_plan);
        let result = match event_jobs.execute_event_job(&event_request) {
            Ok(value) => value,
            Err(_) => {
                return Ok(event_job_failure(
                    "event_job_failed",
                    counts,
                    "event_job",
                    "LINE event transaction failed.",
                )
                .payload())
            }
        };
        let Some(result) = result.as_object() else {
            return Ok(transaction_contract_failure(
                "event_job_contract_invalid",
                counts,
                "LINE event transaction returned an invalid payload.",
            )
            .payload());
        };
        let ok = result.get("ok").and_then(JsonValue::as_bool);
        let processed = result.get("processed").and_then(JsonValue::as_bool);
        let duplicate = result.get("duplicate").and_then(JsonValue::as_bool);
        let (Some(ok), Some(processed), Some(duplicate)) = (ok, processed, duplicate) else {
            return Ok(transaction_contract_failure(
                "event_job_contract_invalid",
                counts,
                "LINE event transaction returned an invalid payload.",
            )
            .payload());
        };
        if !ok {
            let state = clean_text(result.get("event_job_state"))
                .unwrap_or_else(|| "event_job_failed".to_string());
            return Ok(event_job_failure(
                "event_job_failed",
                counts,
                "event_job",
                &format!("LINE event transaction did not complete ({state})."),
            )
            .redacted(&secrets)
            .payload());
        }
        if processed {
            counts.processed += 1;
        } else if duplicate {
            counts.duplicates += 1;
        } else {
            return Ok(transaction_contract_failure(
                "event_job_contract_invalid",
                counts,
                "LINE event transaction returned neither processed nor duplicate state.",
            )
            .payload());
        }
    }

    let mut outcome = TransactionOutcome::new("completed", 200, true);
    outcome.write_json_response = true;
    outcome.response = json!({
        "ok": true,
        "processed_events": counts.processed,
    });
    outcome.counts = counts;
    Ok(outcome.redacted(&secrets).payload())
}

fn ingress_request(request: &Map<String, JsonValue>) -> JsonValue {
    selected_request_fields(
        request,
        &[
            "raw_payload",
            "signature",
            "channel_secret",
            "request_path",
            "path",
            "webhook_path",
            "now_iso",
        ],
    )
}

fn event_job_request(request: &Map<String, JsonValue>, event_plan: &JsonValue) -> JsonValue {
    let mut result = selected_request_fields(
        request,
        &[
            "state_path",
            "runtime_target",
            "channel_access_token",
            "api_base_url",
            "timeout_seconds",
        ],
    );
    result["event_plan"] = event_plan.clone();
    result
}

fn selected_request_fields(request: &Map<String, JsonValue>, fields: &[&str]) -> JsonValue {
    let mut selected = Map::new();
    for field in fields {
        if let Some(value) = request.get(*field) {
            selected.insert((*field).to_string(), value.clone());
        }
    }
    JsonValue::Object(selected)
}

fn rejected_ingress_outcome(
    planned: &Map<String, JsonValue>,
    secrets: &[String],
) -> TransactionOutcome {
    let http_status = planned
        .get("http_status")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| (100..=599).contains(value))
        .unwrap_or(400);
    let write_json_response = planned
        .get("write_json_response")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    let response = planned
        .get("response")
        .cloned()
        .unwrap_or_else(|| json!({"ok": false, "error": "LINE webhook request rejected."}));
    let state =
        clean_text(planned.get("webhook_ingress_state")).unwrap_or_else(|| "rejected".to_string());
    let error_kind =
        clean_text(planned.get("error_kind")).unwrap_or_else(|| "ingress_rejected".to_string());
    let error = clean_text(planned.get("error"))
        .unwrap_or_else(|| "LINE webhook request rejected.".to_string());
    let mut outcome =
        TransactionOutcome::failure("ingress_rejected", http_status, &error_kind, &error);
    outcome.ingress_state = Some(state);
    outcome.write_json_response = write_json_response;
    outcome.response = response;
    outcome.redacted(secrets)
}

fn transaction_contract_failure(
    state: &'static str,
    counts: TransactionCounts,
    error: &str,
) -> TransactionOutcome {
    let mut outcome = TransactionOutcome::failure(state, 500, "contract", error);
    outcome.counts = counts;
    outcome
}

fn event_job_failure(
    state: &'static str,
    counts: TransactionCounts,
    error_kind: &str,
    error: &str,
) -> TransactionOutcome {
    let mut outcome = TransactionOutcome::failure(state, 400, error_kind, error);
    outcome.counts = counts;
    outcome
}

#[derive(Debug, Default, Clone, Copy)]
struct TransactionCounts {
    planned: usize,
    submitted: usize,
    attempted: usize,
    ignored: usize,
    duplicates: usize,
    processed: usize,
}

struct TransactionOutcome {
    state: &'static str,
    ingress_state: Option<String>,
    http_status: u16,
    ok: bool,
    write_json_response: bool,
    response: JsonValue,
    counts: TransactionCounts,
    error_kind: Option<String>,
    error: Option<String>,
}

impl TransactionOutcome {
    fn new(state: &'static str, http_status: u16, ok: bool) -> Self {
        Self {
            state,
            ingress_state: None,
            http_status,
            ok,
            write_json_response: true,
            response: JsonValue::Null,
            counts: TransactionCounts::default(),
            error_kind: None,
            error: None,
        }
    }

    fn failure(state: &'static str, http_status: u16, error_kind: &str, error: &str) -> Self {
        let mut outcome = Self::new(state, http_status, false);
        outcome.response = json!({"ok": false, "error": error});
        outcome.error_kind = Some(error_kind.to_string());
        outcome.error = Some(error.to_string());
        outcome
    }

    fn redacted(mut self, secrets: &[String]) -> Self {
        self.ingress_state = self.ingress_state.map(|value| redact_text(&value, secrets));
        self.response = redact_json(&self.response, secrets);
        self.error_kind = self.error_kind.map(|value| redact_text(&value, secrets));
        self.error = self.error.map(|value| redact_text(&value, secrets));
        self
    }

    fn payload(self) -> JsonValue {
        json!({
            "contract": LINE_HTTP_TRANSACTION_CONTRACT,
            "migration_stage": MIGRATION_STAGE,
            "stage": "execute",
            "transaction_state": self.state,
            "ingress_state": optional_string_json(self.ingress_state.as_deref()),
            "ok": self.ok,
            "http_status": self.http_status,
            "write_json_response": self.write_json_response,
            "response": self.response,
            "planned_event_count": self.counts.planned,
            "submitted_event_count": self.counts.submitted,
            "attempted_event_count": self.counts.attempted,
            "ignored_event_count": self.counts.ignored,
            "duplicate_event_count": self.counts.duplicates,
            "processed_events": self.counts.processed,
            "error_kind": optional_string_json(self.error_kind.as_deref()),
            "error": optional_string_json(self.error.as_deref()),
            "python_signature_verification_allowed": false,
            "python_json_parsing_allowed": false,
            "python_event_execution_allowed": false,
            "python_http_response_allowed": false,
        })
    }
}

fn request_secrets(request: &Map<String, JsonValue>) -> Vec<String> {
    ["channel_secret", "channel_access_token", "signature"]
        .iter()
        .filter_map(|key| clean_text(request.get(*key)))
        .collect()
}

fn redact_json(value: &JsonValue, secrets: &[String]) -> JsonValue {
    match value {
        JsonValue::String(value) => JsonValue::String(redact_text(value, secrets)),
        JsonValue::Array(values) => JsonValue::Array(
            values
                .iter()
                .map(|value| redact_json(value, secrets))
                .collect(),
        ),
        JsonValue::Object(values) => JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(key) {
                        JsonValue::String(REDACTED.to_string())
                    } else {
                        redact_json(value, secrets)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn redact_text(value: &str, secrets: &[String]) -> String {
    secrets
        .iter()
        .filter(|secret| !secret.is_empty())
        .fold(value.to_string(), |text, secret| {
            text.replace(secret, REDACTED)
        })
}

fn sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization"
            | "channel_access_token"
            | "channel_secret"
            | "access_token"
            | "reply_token"
            | "replytoken"
            | "signature"
    )
}

fn request_object(value: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| "LINE HTTP transaction request must be an object.".to_string())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}

#[cfg(test)]
mod tests;
