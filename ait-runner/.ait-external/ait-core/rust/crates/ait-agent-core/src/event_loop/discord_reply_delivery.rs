use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const EXECUTION_KIND: &str = "discord_reply_delivery";
const DELIVERY_KIND: &str = "discord_assistant_reply";
const REPLY_MODE_CHANNEL_MESSAGE: &str = "channel_message";
const REPLY_MODE_INTERACTION: &str = "interaction";

pub fn agent_discord_reply_delivery_execution_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "Discord reply delivery execution request must be an object.".to_string())?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| {
        if object.contains_key("callback_result") || object.contains_key("operation_results") {
            "result".to_string()
        } else {
            "request".to_string()
        }
    });
    match stage.as_str() {
        "request" => plan_request(object),
        "result" => plan_result(object),
        other => Err(format!(
            "unsupported Discord reply delivery execution stage: {other}"
        )),
    }
}

pub fn agent_discord_reply_delivery_callback_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "Discord reply delivery callback request must be an object.".to_string())?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| {
        if object.contains_key("callback_result") || object.contains_key("operation_results") {
            "result".to_string()
        } else {
            "request".to_string()
        }
    });
    match stage.as_str() {
        "request" => {
            let execution_request = object
                .get("execution_request")
                .cloned()
                .unwrap_or_else(|| JsonValue::Object(object.clone()));
            let execution = agent_discord_reply_delivery_execution_plan_json(&json!({
                "stage": "request",
                "execution_request": execution_request,
            }))?;
            Ok(json!({
                "stage": "request",
                "execution_kind": "discord_reply_delivery_callback",
                "delivery_kind": DELIVERY_KIND,
                "callback_group": "reply_delivery",
                "callback_kind": "deliver_discord_reply",
                "reply_mode": execution["reply_mode"],
                "should_execute": execution["should_execute"],
                "expects_result": true,
                "completed": false,
                "requires_post_operations": false,
                "request": execution["request"],
                "result": JsonValue::Null,
                "reply_delivery_execution": execution,
            }))
        }
        "result" => {
            let callback_result = object
                .get("callback_result")
                .cloned()
                .unwrap_or_else(|| JsonValue::Object(object.clone()));
            let execution = agent_discord_reply_delivery_execution_plan_json(&json!({
                "stage": "result",
                "callback_result": callback_result,
            }))?;
            Ok(json!({
                "stage": "result",
                "execution_kind": "discord_reply_delivery_callback",
                "delivery_kind": DELIVERY_KIND,
                "callback_group": "reply_delivery",
                "callback_kind": "deliver_discord_reply",
                "reply_mode": execution["reply_mode"],
                "should_execute": false,
                "expects_result": false,
                "completed": execution["completed"],
                "requires_post_operations": false,
                "request": JsonValue::Null,
                "result": execution["result"],
                "reply_delivery_execution": execution,
            }))
        }
        other => Err(format!(
            "unsupported Discord reply delivery callback stage: {other}"
        )),
    }
}

fn plan_request(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = object
        .get("execution_request")
        .and_then(JsonValue::as_object)
        .unwrap_or(object);
    let reply_mode = normalize_reply_mode(
        clean_text(source.get("reply_mode"))
            .or_else(|| clean_text(source.get("delivery_mode")))
            .as_deref(),
    );
    let channel_id = clean_text(source.get("channel_id")).unwrap_or_default();
    let application_id = clean_text(source.get("application_id")).unwrap_or_default();
    let interaction_token = clean_text(source.get("interaction_token")).unwrap_or_default();
    let reply_text =
        clean_text(source.get("reply_text").or_else(|| source.get("text"))).unwrap_or_default();
    let attachments = source
        .get("attachments")
        .or_else(|| source.get("reply_attachments"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let assistant_sequence = non_negative_i64(source.get("assistant_sequence")).unwrap_or(0);
    let through_sequence = non_negative_i64(source.get("through_sequence"))
        .unwrap_or(assistant_sequence)
        .max(assistant_sequence);
    let requested = source
        .get("should_execute")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    let operations = delivery_operations(
        &reply_mode,
        &channel_id,
        &application_id,
        &interaction_token,
        &reply_text,
        &attachments,
    );
    let target_valid = match reply_mode.as_str() {
        REPLY_MODE_INTERACTION => !application_id.is_empty() && !interaction_token.is_empty(),
        _ => !channel_id.is_empty(),
    };
    let error = if !target_valid {
        Some("Discord reply target is incomplete.")
    } else if operations.is_empty() {
        Some("Discord reply payload is empty.")
    } else {
        None
    };
    let should_execute = requested && error.is_none();
    let request = json!({
        "execution_kind": EXECUTION_KIND,
        "delivery_kind": DELIVERY_KIND,
        "callback_group": "reply_delivery",
        "operation": "deliver_discord_reply",
        "ok": error.is_none(),
        "error": error,
        "reply_mode": reply_mode,
        "channel_id": channel_id,
        "application_id": application_id,
        "interaction_token": interaction_token,
        "assistant_sequence": assistant_sequence,
        "through_sequence": through_sequence,
        "reply_text": reply_text,
        "attachments": attachments,
        "operations": operations,
        "operation_count": operations.len(),
    });
    Ok(json!({
        "stage": "request",
        "execution_kind": EXECUTION_KIND,
        "delivery_kind": DELIVERY_KIND,
        "reply_mode": reply_mode,
        "should_execute": should_execute,
        "expects_result": true,
        "completed": false,
        "requires_post_operations": false,
        "request": request,
        "result": JsonValue::Null,
    }))
}

fn plan_result(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let result = object
        .get("callback_result")
        .or_else(|| object.get("result"))
        .and_then(JsonValue::as_object)
        .unwrap_or(object);
    let operation_results = result
        .get("operation_results")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let post_operation_results = result
        .get("post_operation_results")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let operation_count =
        non_negative_i64(result.get("operation_count")).unwrap_or(operation_results.len() as i64);
    let delivered_count = operation_results
        .iter()
        .filter(|value| operation_succeeded(value))
        .count() as i64;
    let failed_count = operation_results.len() as i64 - delivered_count;
    let reply_mode = normalize_reply_mode(clean_text(result.get("reply_mode")).as_deref());
    let assistant_sequence = non_negative_i64(result.get("assistant_sequence")).unwrap_or(0);
    let through_sequence = non_negative_i64(result.get("through_sequence"))
        .unwrap_or(assistant_sequence)
        .max(assistant_sequence);
    let attachments = result
        .get("attachments")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let channel_id = clean_text(result.get("channel_id")).unwrap_or_default();
    let application_id = clean_text(result.get("application_id")).unwrap_or_default();
    let interaction_token = clean_text(result.get("interaction_token")).unwrap_or_default();
    let post_operations = if post_operation_results.is_empty() {
        attachment_fallback_operations(
            &operation_results,
            &attachments,
            &reply_mode,
            &channel_id,
            &application_id,
            &interaction_token,
        )
    } else {
        Vec::new()
    };
    let requires_post_operations = !post_operations.is_empty();
    let delivered_post_operation_count = post_operation_results
        .iter()
        .filter(|value| operation_succeeded(value))
        .count() as i64;
    let failed_post_operation_count =
        post_operation_results.len() as i64 - delivered_post_operation_count;
    let failed_unrecoverable_operation_count = operation_results
        .iter()
        .filter(|value| !operation_succeeded(value) && !is_recoverable_attachment_failure(value))
        .count() as i64;
    let message_ids = collect_message_ids(&operation_results, &post_operation_results);
    let recovered = !post_operation_results.is_empty()
        && failed_post_operation_count == 0
        && post_operation_results.iter().all(operation_succeeded);
    let delivered = operation_count > 0
        && operation_results.len() as i64 == operation_count
        && (failed_count == 0 || (failed_unrecoverable_operation_count == 0 && recovered));
    let failure_texts = operation_results
        .iter()
        .chain(post_operation_results.iter())
        .filter(|value| !operation_succeeded(value))
        .filter_map(|value| clean_text(value.get("error")))
        .collect::<Vec<_>>();
    let error = if delivered {
        JsonValue::Null
    } else {
        failure_texts
            .first()
            .cloned()
            .map(JsonValue::String)
            .unwrap_or_else(|| JsonValue::String("Discord reply delivery failed.".to_string()))
    };
    let result = json!({
        "execution_kind": EXECUTION_KIND,
        "delivery_kind": DELIVERY_KIND,
        "ok": delivered,
        "delivered": delivered,
        "reply_mode": reply_mode,
        "assistant_sequence": assistant_sequence,
        "through_sequence": through_sequence,
        "operation_results": operation_results,
        "operation_count": operation_count,
        "delivered_operation_count": delivered_count,
        "failed_operation_count": failed_count,
        "failed_unrecoverable_operation_count": failed_unrecoverable_operation_count,
        "post_operations": post_operations,
        "post_operation_count": post_operations.len().max(post_operation_results.len()),
        "post_operation_results": post_operation_results,
        "delivered_post_operation_count": delivered_post_operation_count,
        "failed_post_operation_count": failed_post_operation_count,
        "summary_text": if delivered { "Discord reply delivered." } else { "Discord reply delivery failed." },
        "failure_texts": failure_texts,
        "message_ids": message_ids,
        "error": error,
    });
    Ok(json!({
        "stage": "result",
        "execution_kind": EXECUTION_KIND,
        "delivery_kind": DELIVERY_KIND,
        "reply_mode": reply_mode,
        "should_execute": false,
        "expects_result": false,
        "completed": delivered,
        "requires_post_operations": requires_post_operations,
        "request": JsonValue::Null,
        "result": result,
    }))
}

fn delivery_operations(
    reply_mode: &str,
    channel_id: &str,
    application_id: &str,
    interaction_token: &str,
    reply_text: &str,
    attachments: &[JsonValue],
) -> Vec<JsonValue> {
    let mut operations = Vec::new();
    if !reply_text.is_empty() {
        operations.push(match reply_mode {
            REPLY_MODE_INTERACTION => json!({
                "kind": "edit_original_response",
                "application_id": application_id,
                "interaction_token": interaction_token,
                "text": reply_text,
            }),
            _ => json!({
                "kind": "send_channel_message",
                "channel_id": channel_id,
                "text": reply_text,
            }),
        });
    }
    for (index, attachment) in attachments.iter().enumerate() {
        operations.push(match reply_mode {
            REPLY_MODE_INTERACTION => json!({
                "kind": "send_followup_attachment",
                "application_id": application_id,
                "interaction_token": interaction_token,
                "attachment_index": index,
                "attachment": attachment,
            }),
            _ => json!({
                "kind": "send_channel_attachment",
                "channel_id": channel_id,
                "attachment_index": index,
                "attachment": attachment,
            }),
        });
    }
    operations
}

fn normalize_reply_mode(value: Option<&str>) -> String {
    match value
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        REPLY_MODE_INTERACTION => REPLY_MODE_INTERACTION.to_string(),
        _ => REPLY_MODE_CHANNEL_MESSAGE.to_string(),
    }
}

fn operation_succeeded(value: &JsonValue) -> bool {
    value
        .get("ok")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
        && value
            .get("delivered")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true)
}

fn attachment_fallback_operations(
    operation_results: &[JsonValue],
    attachments: &[JsonValue],
    reply_mode: &str,
    channel_id: &str,
    application_id: &str,
    interaction_token: &str,
) -> Vec<JsonValue> {
    operation_results
        .iter()
        .filter(|result| !operation_succeeded(result))
        .filter(|result| is_recoverable_attachment_failure(result))
        .map(|result| {
            let text = attachment_failure_text(result, attachments);
            match reply_mode {
                REPLY_MODE_INTERACTION => json!({
                    "kind": "send_followup",
                    "application_id": application_id,
                    "interaction_token": interaction_token,
                    "text": text,
                }),
                _ => json!({
                    "kind": "send_channel_message",
                    "channel_id": channel_id,
                    "text": text,
                }),
            }
        })
        .collect()
}

fn attachment_failure_text(result: &JsonValue, attachments: &[JsonValue]) -> String {
    let attachment = result
        .get("attachment")
        .filter(|value| value.is_object())
        .or_else(|| {
            non_negative_i64(result.get("attachment_index"))
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| attachments.get(index))
        });
    let file_name = attachment
        .and_then(|attachment| {
            clean_text(attachment.get("file_name"))
                .or_else(|| clean_text(attachment.get("name")))
                .or_else(|| clean_text(attachment.get("local_path")).map(file_name_from_path))
        })
        .unwrap_or_else(|| "attachment".to_string());
    let error = clean_text(result.get("error"))
        .unwrap_or_else(|| "Discord attachment upload failed.".to_string());
    format!(
        "Could not upload Discord attachment `{file_name}`. Fallback to text/path only.\n{error}"
    )
}

fn file_name_from_path(path: String) -> String {
    path.rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or("attachment")
        .to_string()
}

fn is_recoverable_attachment_failure(result: &JsonValue) -> bool {
    matches!(
        clean_text(result.get("kind")).as_deref(),
        Some("send_channel_attachment" | "send_followup_attachment")
    )
}

fn collect_message_ids(
    operation_results: &[JsonValue],
    post_operation_results: &[JsonValue],
) -> Vec<JsonValue> {
    let mut values = Vec::new();
    for result in operation_results.iter().chain(post_operation_results) {
        if let Some(items) = result.get("message_ids").and_then(JsonValue::as_array) {
            values.extend(items.iter().filter(|value| !value.is_null()).cloned());
        }
        if let Some(value) = result.get("message_id").filter(|value| !value.is_null()) {
            values.push(value.clone());
        }
    }
    values
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = match value? {
        JsonValue::String(value) => value.trim().to_string(),
        JsonValue::Number(value) => value.to_string(),
        _ => return None,
    };
    (!value.is_empty()).then_some(value)
}

fn non_negative_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(value) => value.as_i64().map(|value| value.max(0)),
        JsonValue::String(value) => value.trim().parse::<i64>().ok().map(|value| value.max(0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
