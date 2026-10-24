use crate::json_support::parse_value;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::super::telegram_polling::plan_telegram_update_batch_dispatch;

const MIGRATION_STAGE: &str = "rust_agent_telegram_webhook_ingress";
const WEBHOOK_INGRESS_CONTRACT: &str = "ait_agent_core.event_loop.TelegramWebhookIngress.v1";

pub trait TelegramWebhookIngressPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramWebhookIngressPlanner;

impl TelegramWebhookIngressPlanner for DefaultTelegramWebhookIngressPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_webhook_ingress_json(request)
    }
}

pub fn agent_telegram_webhook_ingress_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_telegram_webhook_ingress_planner(&DefaultTelegramWebhookIngressPlanner, request)
}

pub fn plan_with_telegram_webhook_ingress_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramWebhookIngressPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_webhook_ingress_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())?;
    let raw_payload = object
        .get("raw_payload")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "raw_payload is required".to_string())?;
    let fallback_key_prefix = clean_text(object.get("fallback_update_key_prefix"))
        .unwrap_or_else(|| "webhook".to_string());
    let updates = parse_telegram_webhook_updates(raw_payload)?;
    let fallback_update_keys = updates
        .iter()
        .enumerate()
        .map(|(index, update)| {
            JsonValue::String(webhook_fallback_update_key(
                &fallback_key_prefix,
                index,
                update,
            ))
        })
        .collect::<Vec<_>>();
    let (dispatch_items, last_update_id) =
        plan_telegram_update_batch_dispatch(&updates, Some(&fallback_update_keys))?;
    let update_count = fallback_update_keys.len();
    let dispatch_count = dispatch_items.len();

    Ok(json!({
        "migration_stage": MIGRATION_STAGE,
        "ingress_contract": WEBHOOK_INGRESS_CONTRACT,
        "source": "telegram_webhook",
        "transport": "telegram",
        "ingress_state": "accepted",
        "rust_event_loop_required": true,
        "python_ingress_allowed": false,
        "webhook_runtime_required": true,
        "raw_payload_present": true,
        "fallback_update_key_prefix": fallback_key_prefix,
        "updates": updates,
        "update_count": update_count,
        "fallback_update_keys": fallback_update_keys,
        "dispatch_items": dispatch_items,
        "dispatch_count": dispatch_count,
        "last_update_id": last_update_id,
        "should_update_last_update_id": last_update_id != 0,
        "rejection_reasons": [],
    }))
}

fn parse_telegram_webhook_updates(raw_payload: &str) -> Result<Vec<JsonValue>, String> {
    if raw_payload.trim().is_empty() {
        return Err("No Telegram webhook payload provided on stdin.".to_string());
    }
    let payload = parse_value(raw_payload, "failed to parse Telegram webhook payload")
        .map_err(|_| "Telegram webhook payload must be valid JSON.".to_string())?;
    let updates = match payload {
        JsonValue::Array(values) => values,
        JsonValue::Object(_) => vec![payload],
        _ => {
            return Err("Telegram webhook payload must be a JSON object or array.".to_string());
        }
    };
    if updates.is_empty() {
        return Err("Telegram webhook payload must contain at least one update.".to_string());
    }
    for (index, update) in updates.iter().enumerate() {
        if !update.is_object() {
            return Err(format!(
                "Telegram webhook update payload item #{index} must be a JSON object."
            ));
        }
    }
    Ok(updates)
}

fn webhook_fallback_update_key(prefix: &str, index: usize, update: &JsonValue) -> String {
    if let Some(update_id) = update.as_object().and_then(update_identity_suffix) {
        return format!("{prefix}-{update_id}");
    }
    format!("{prefix}-{index}")
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

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = value?.as_str()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests;
