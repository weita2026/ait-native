use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::agent_telegram_webhook_ingress_plan_json;

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramWebhookTransaction.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_webhook_dispatch_transaction";
const INGRESS_CONTRACT: &str = "ait_agent_core.event_loop.TelegramWebhookIngress.v1";
const INGRESS_MIGRATION_STAGE: &str = "rust_agent_telegram_webhook_ingress";

pub trait TelegramWebhookTransactionIngressPort {
    fn plan_ingress(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait TelegramWebhookTransactionDispatchPort {
    fn dispatch_update(&self, request: &JsonValue) -> Result<(), String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramWebhookTransactionIngressPort;

impl TelegramWebhookTransactionIngressPort for DefaultTelegramWebhookTransactionIngressPort {
    fn plan_ingress(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_webhook_ingress_plan_json(request)
    }
}

pub fn execute_with_telegram_webhook_transaction_ports<I, D>(
    ingress: &I,
    dispatch: &D,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    I: TelegramWebhookTransactionIngressPort + ?Sized,
    D: TelegramWebhookTransactionDispatchPort + ?Sized,
{
    let request = request
        .as_object()
        .ok_or_else(|| "Telegram webhook transaction request must be an object.".to_string())?;
    let ingress_request = selected_fields(request, &["raw_payload", "fallback_update_key_prefix"]);
    let planned = match ingress.plan_ingress(&ingress_request) {
        Ok(value) => value,
        Err(_) => {
            return Ok(TransactionOutcome::failure(
                "ingress_failed",
                400,
                "ingress",
                "Telegram webhook ingress failed.",
            )
            .payload())
        }
    };
    let batch = match ValidatedIngressBatch::parse(&planned) {
        Ok(batch) => batch,
        Err(_) => {
            return Ok(TransactionOutcome::failure(
                "ingress_contract_invalid",
                500,
                "contract",
                "Telegram webhook ingress returned an invalid contract.",
            )
            .payload())
        }
    };
    let mut counts = TransactionCounts {
        planned: batch.items.len(),
        ..TransactionCounts::default()
    };

    for item in &batch.items {
        counts.attempted += 1;
        if dispatch.dispatch_update(&item.request).is_err() {
            counts.failed += 1;
            return Ok(TransactionOutcome::failure(
                "dispatch_failed",
                500,
                "dispatch",
                "Telegram webhook update dispatch failed.",
            )
            .with_counts(counts)
            .with_ingress(&batch)
            .payload());
        }
        counts.dispatched += 1;
    }

    Ok(TransactionOutcome::success(counts, &batch).payload())
}

struct ValidatedIngressBatch {
    items: Vec<ValidatedDispatchItem>,
    last_update_id: i64,
}

impl ValidatedIngressBatch {
    fn parse(value: &JsonValue) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "Telegram webhook ingress result must be an object.".to_string())?;
        for (key, expected) in [
            ("migration_stage", INGRESS_MIGRATION_STAGE),
            ("ingress_contract", INGRESS_CONTRACT),
            ("source", "telegram_webhook"),
            ("transport", "telegram"),
            ("ingress_state", "accepted"),
        ] {
            if clean_text(object.get(key)).as_deref() != Some(expected) {
                return Err(format!(
                    "Telegram webhook ingress field `{key}` is invalid."
                ));
            }
        }
        for key in [
            "rust_event_loop_required",
            "webhook_runtime_required",
            "raw_payload_present",
        ] {
            if object.get(key).and_then(JsonValue::as_bool) != Some(true) {
                return Err(format!(
                    "Telegram webhook ingress field `{key}` must be true."
                ));
            }
        }
        if object
            .get("python_ingress_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        {
            return Err(
                "Telegram webhook ingress python_ingress_allowed must be false.".to_string(),
            );
        }
        let updates = object
            .get("updates")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| "Telegram webhook ingress updates must be an array.".to_string())?;
        let dispatch_items = object
            .get("dispatch_items")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| {
                "Telegram webhook ingress dispatch_items must be an array.".to_string()
            })?;
        let fallback_keys = object
            .get("fallback_update_keys")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| {
                "Telegram webhook ingress fallback_update_keys must be an array.".to_string()
            })?;
        if updates.is_empty()
            || dispatch_items.len() != updates.len()
            || fallback_keys.len() != updates.len()
            || required_usize(object, "update_count")? != updates.len()
            || required_usize(object, "dispatch_count")? != dispatch_items.len()
        {
            return Err("Telegram webhook ingress batch counts are inconsistent.".to_string());
        }
        if object
            .get("rejection_reasons")
            .and_then(JsonValue::as_array)
            .is_none_or(|reasons| !reasons.is_empty())
        {
            return Err("Accepted Telegram webhook ingress cannot contain rejections.".to_string());
        }
        let last_update_id = optional_i64(object.get("last_update_id")).ok_or_else(|| {
            "Telegram webhook ingress last_update_id must be an integer.".to_string()
        })?;
        if object
            .get("should_update_last_update_id")
            .and_then(JsonValue::as_bool)
            != Some(last_update_id != 0)
        {
            return Err(
                "Telegram webhook ingress cursor observation contract is invalid.".to_string(),
            );
        }
        let fallback_prefix =
            clean_text(object.get("fallback_update_key_prefix")).ok_or_else(|| {
                "Telegram webhook ingress fallback_update_key_prefix must be non-empty.".to_string()
            })?;

        let mut items = Vec::with_capacity(updates.len());
        let mut expected_last_update_id = 0_i64;
        for (index, ((update, dispatch_item), fallback_key)) in updates
            .iter()
            .zip(dispatch_items)
            .zip(fallback_keys)
            .enumerate()
        {
            let update = update
                .as_object()
                .ok_or_else(|| "Telegram webhook ingress update must be an object.".to_string())?;
            let dispatch_item = dispatch_item.as_object().ok_or_else(|| {
                "Telegram webhook ingress dispatch item must be an object.".to_string()
            })?;
            let fallback_key = clean_text(Some(fallback_key)).ok_or_else(|| {
                "Telegram webhook ingress fallback key must be non-empty.".to_string()
            })?;
            if optional_usize(dispatch_item.get("index")) != Some(index) {
                return Err("Telegram webhook ingress dispatch item index is invalid.".to_string());
            }
            let dispatch_key = clean_text(dispatch_item.get("dispatch_key")).ok_or_else(|| {
                "Telegram webhook ingress dispatch key must be non-empty.".to_string()
            })?;
            let update_key = clean_text(dispatch_item.get("update_key")).ok_or_else(|| {
                "Telegram webhook ingress update key must be non-empty.".to_string()
            })?;
            let update_id = optional_i64(dispatch_item.get("update_id")).ok_or_else(|| {
                "Telegram webhook ingress dispatch update_id must be an integer.".to_string()
            })?;
            let expected_update_id = optional_i64(update.get("update_id")).unwrap_or(0);
            let expected_message_id = update_message_id(update).unwrap_or(0);
            let expected_chat_id_value = update_chat_id(update);
            let expected_chat_id = expected_chat_id_value.cloned().unwrap_or(JsonValue::Null);
            let expected_fallback_key = update_identity_suffix(update)
                .map(|suffix| format!("{fallback_prefix}-{suffix}"))
                .unwrap_or_else(|| format!("{fallback_prefix}-{index}"));
            let expected_update_key = if expected_update_id != 0 {
                format!("update-{expected_update_id}")
            } else if expected_message_id != 0 {
                format!("message-{expected_message_id}")
            } else {
                expected_fallback_key.clone()
            };
            let expected_dispatch_key = if let Some(chat_id) = expected_chat_id_value {
                format!("chat-{}", pythonish_text(chat_id))
            } else if expected_update_id != 0 {
                format!("update-{expected_update_id}")
            } else {
                "update-unknown".to_string()
            };
            if update_id != expected_update_id
                || optional_i64(dispatch_item.get("message_id")) != Some(expected_message_id)
                || dispatch_item.get("chat_id") != Some(&expected_chat_id)
                || fallback_key != expected_fallback_key
                || update_key != expected_update_key
                || dispatch_key != expected_dispatch_key
                || dispatch_item
                    .get("should_update_last_update_id")
                    .and_then(JsonValue::as_bool)
                    != Some(update_id != 0)
            {
                return Err(
                    "Telegram webhook ingress dispatch cursor contract is invalid.".to_string(),
                );
            }
            if expected_update_id != 0 {
                expected_last_update_id = expected_update_id;
            }
            items.push(ValidatedDispatchItem {
                request: json!({
                    "source": "telegram_webhook",
                    "index": index,
                    "update": JsonValue::Object(update.clone()),
                    "dispatch_item": JsonValue::Object(dispatch_item.clone()),
                    "dispatch_key": dispatch_key,
                    "queue_key": dispatch_key,
                    "update_key": update_key,
                    "fallback_update_key": fallback_key,
                }),
            });
        }
        if last_update_id != expected_last_update_id {
            return Err(
                "Telegram webhook ingress last_update_id does not match its update batch."
                    .to_string(),
            );
        }
        Ok(Self {
            items,
            last_update_id,
        })
    }
}

struct ValidatedDispatchItem {
    request: JsonValue,
}

#[derive(Debug, Default, Clone, Copy)]
struct TransactionCounts {
    planned: usize,
    attempted: usize,
    dispatched: usize,
    failed: usize,
}

struct TransactionOutcome {
    state: &'static str,
    http_status: u16,
    ok: bool,
    counts: TransactionCounts,
    last_update_id: i64,
    error_kind: Option<&'static str>,
    error: Option<&'static str>,
}

impl TransactionOutcome {
    fn success(counts: TransactionCounts, batch: &ValidatedIngressBatch) -> Self {
        Self {
            state: "completed",
            http_status: 200,
            ok: true,
            counts,
            last_update_id: batch.last_update_id,
            error_kind: None,
            error: None,
        }
    }

    fn failure(
        state: &'static str,
        http_status: u16,
        error_kind: &'static str,
        error: &'static str,
    ) -> Self {
        Self {
            state,
            http_status,
            ok: false,
            counts: TransactionCounts::default(),
            last_update_id: 0,
            error_kind: Some(error_kind),
            error: Some(error),
        }
    }

    fn with_counts(mut self, counts: TransactionCounts) -> Self {
        self.counts = counts;
        self
    }

    fn with_ingress(mut self, batch: &ValidatedIngressBatch) -> Self {
        self.last_update_id = batch.last_update_id;
        self
    }

    fn payload(self) -> JsonValue {
        let unattempted = self.counts.planned.saturating_sub(self.counts.attempted);
        let remaining = self.counts.planned.saturating_sub(self.counts.dispatched);
        let response = if self.ok {
            json!({
                "ok": true,
                "processed_updates": self.counts.dispatched,
            })
        } else {
            json!({
                "ok": false,
                "error": self.error.unwrap_or("Telegram webhook transaction failed."),
            })
        };
        let ingress_state = match self.state {
            "completed" | "dispatch_failed" => "accepted",
            "ingress_failed" => "rejected",
            _ => "invalid",
        };
        json!({
            "contract": CONTRACT,
            "migration_stage": MIGRATION_STAGE,
            "stage": "execute",
            "transaction_state": self.state,
            "ingress_state": ingress_state,
            "ok": self.ok,
            "completed": self.ok,
            "http_status": self.http_status,
            "write_json_response": true,
            "response": response,
            "planned_update_count": self.counts.planned,
            "attempted_update_count": self.counts.attempted,
            "dispatched_update_count": self.counts.dispatched,
            "failed_update_count": self.counts.failed,
            "unattempted_update_count": unattempted,
            "remaining_update_count": remaining,
            "last_update_id_observed": self.last_update_id,
            "cursor_mutated": false,
            "retryable": self.http_status >= 500,
            "error_kind": self.error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "error": self.error.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "python_ingress_allowed": false,
            "python_service_entry_loop_allowed": false,
            "python_update_dispatch_allowed": false,
            "python_http_response_allowed": false,
        })
    }
}

fn selected_fields(request: &Map<String, JsonValue>, keys: &[&str]) -> JsonValue {
    JsonValue::Object(
        keys.iter()
            .filter_map(|key| {
                request
                    .get(*key)
                    .map(|value| ((*key).to_string(), value.clone()))
            })
            .collect(),
    )
}

fn required_usize(object: &Map<String, JsonValue>, key: &str) -> Result<usize, String> {
    optional_usize(object.get(key))
        .ok_or_else(|| format!("Telegram webhook ingress `{key}` must be a non-negative integer."))
}

fn optional_usize(value: Option<&JsonValue>) -> Option<usize> {
    match value? {
        JsonValue::Number(value) => value.as_u64().and_then(|value| usize::try_from(value).ok()),
        JsonValue::String(value) => value.trim().parse::<usize>().ok(),
        _ => None,
    }
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(value) => value.as_i64(),
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn update_message_id(update: &Map<String, JsonValue>) -> Option<i64> {
    update
        .get("message")
        .and_then(JsonValue::as_object)
        .and_then(|message| optional_i64(message.get("message_id")))
}

fn update_chat_id(update: &Map<String, JsonValue>) -> Option<&JsonValue> {
    update
        .get("message")
        .and_then(JsonValue::as_object)
        .and_then(|message| message.get("chat"))
        .and_then(JsonValue::as_object)
        .and_then(|chat| chat.get("id"))
}

fn update_identity_suffix(update: &Map<String, JsonValue>) -> Option<String> {
    update
        .get("update_id")
        .and_then(|value| {
            if let Some(number) = value.as_i64() {
                Some(number.to_string())
            } else {
                value.as_str().map(|text| text.trim().to_string())
            }
        })
        .filter(|value| !value.is_empty())
}

fn pythonish_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "None".to_string(),
        JsonValue::Bool(true) => "True".to_string(),
        JsonValue::Bool(false) => "False".to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => value.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
    }
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests;
