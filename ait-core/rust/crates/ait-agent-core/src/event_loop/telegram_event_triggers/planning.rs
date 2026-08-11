use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use regex::RegexBuilder;
use std::cmp::Ordering;
use std::collections::BTreeSet;

const MIGRATION_STAGE: &str = "rust_agent_telegram_event_trigger";
const EVENT_TRIGGER_CONTRACT: &str = "ait_agent_core.event_loop.TelegramEventTrigger.v1";

const DEFAULT_CLEAR_PHRASES: &[&str] = &["換個話題", "換個主題"];
const DEFAULT_TOPIC_LEAD_PHRASES: &[&str] = &["換個話題", "換個主題"];
const DEFAULT_TOPIC_JOINERS: &[&str] = &["跟", "和", "與"];
const DEFAULT_TOPIC_TAIL: &str = "有關";
const DEFAULT_PLANNING_MODE_PHRASES: &[&str] = &["進行計劃", "進行計畫", "進行计划"];

pub trait TelegramEventTriggerPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramEventTriggerPlanner;

impl TelegramEventTriggerPlanner for DefaultTelegramEventTriggerPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_telegram_event_trigger_json(request)
    }
}

pub fn agent_telegram_event_trigger_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_telegram_event_trigger_planner(&DefaultTelegramEventTriggerPlanner, request)
}

pub fn plan_with_telegram_event_trigger_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramEventTriggerPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_telegram_event_trigger_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "Telegram event trigger request must be an object".to_string())?;
    let stage = clean_text(object.get("stage"))
        .or_else(|| clean_text(object.get("kind")))
        .unwrap_or_else(|| "normalize_registry".to_string());

    match stage.as_str() {
        "default_registry" => Ok(base_result(
            &stage,
            json!({
                "registry": default_registry(),
            }),
        )),
        "normalize_registry" => Ok(plan_normalize_registry(object)),
        "normalize_operational_trigger" => Ok(plan_normalize_operational_trigger(object)),
        "fresh_topic_match" => Ok(plan_fresh_topic_match(object)),
        "fresh_topic_confirmation_text" => Ok(plan_fresh_topic_confirmation_text(object)),
        "planning_mode_match" => Ok(plan_planning_mode_match(object)),
        "operational_match" => plan_operational_match(object),
        "operational_dispatch" | "operational_select" => plan_operational_dispatch(object),
        other => Err(format!(
            "unsupported Telegram event trigger plan stage `{other}`"
        )),
    }
}

fn plan_normalize_registry(object: &Map<String, JsonValue>) -> JsonValue {
    let fallback = default_registry();
    let fallback_obj = fallback.as_object().expect("default registry is object");
    let fresh_topic = normalize_fresh_topic_config(
        object
            .get("fresh_topic")
            .and_then(JsonValue::as_object)
            .or_else(|| {
                object
                    .get("fresh_topic_config")
                    .and_then(JsonValue::as_object)
            }),
        fallback_obj
            .get("fresh_topic")
            .and_then(JsonValue::as_object)
            .expect("default fresh_topic is object"),
    );
    let planning_mode = normalize_planning_mode_config(
        object
            .get("planning_mode")
            .and_then(JsonValue::as_object)
            .or_else(|| {
                object
                    .get("planning_mode_config")
                    .and_then(JsonValue::as_object)
            }),
        fallback_obj
            .get("planning_mode")
            .and_then(JsonValue::as_object)
            .expect("default planning_mode is object"),
    );
    let mut telegram_operational = object
        .get("telegram_operational")
        .or_else(|| object.get("operational_triggers"))
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(parse_operational_trigger_from_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    sort_operational_triggers(&mut telegram_operational);

    base_result(
        "normalize_registry",
        json!({
            "registry": {
                "fresh_topic": fresh_topic,
                "planning_mode": planning_mode,
                "telegram_operational": telegram_operational,
            },
        }),
    )
}

fn plan_normalize_operational_trigger(object: &Map<String, JsonValue>) -> JsonValue {
    let config = object
        .get("payload")
        .or_else(|| object.get("config"))
        .or_else(|| object.get("trigger"))
        .and_then(JsonValue::as_object)
        .and_then(|payload| {
            parse_operational_trigger_config(
                payload,
                clean_text(object.get("source_path")).unwrap_or_default(),
            )
        });
    base_result(
        "normalize_operational_trigger",
        json!({
            "ok": config.is_some(),
            "trigger": config,
        }),
    )
}

fn plan_fresh_topic_match(object: &Map<String, JsonValue>) -> JsonValue {
    let fallback = default_registry();
    let fallback_fresh = fallback
        .get("fresh_topic")
        .and_then(JsonValue::as_object)
        .expect("default fresh_topic is object");
    let config = normalize_fresh_topic_config(
        object.get("config").and_then(JsonValue::as_object),
        fallback_fresh,
    );
    let trigger = parse_fresh_topic_trigger(
        clean_text(object.get("text")).as_deref().unwrap_or(""),
        config
            .as_object()
            .expect("normalized fresh topic config is object"),
    );
    base_result(
        "fresh_topic_match",
        json!({
            "matched": trigger.is_some(),
            "trigger": trigger,
        }),
    )
}

fn plan_fresh_topic_confirmation_text(object: &Map<String, JsonValue>) -> JsonValue {
    let fresh_topic = object
        .get("fresh_topic")
        .or_else(|| object.get("trigger"))
        .and_then(JsonValue::as_object);
    let mode = fresh_topic
        .and_then(|value| clean_text(value.get("mode")))
        .unwrap_or_else(|| "clear".to_string())
        .to_ascii_lowercase();
    let topic = fresh_topic.and_then(|value| clean_text(value.get("topic")));
    let default_trigger_label =
        clean_text(object.get("default_trigger_label")).unwrap_or_else(|| "換個話題".to_string());
    let trigger_label = fresh_topic
        .and_then(|value| clean_text(value.get("display_trigger")))
        .unwrap_or(default_trigger_label);

    let mut lines = vec![
        "Started a fresh Telegram conversation.".to_string(),
        format!("Trigger: {trigger_label}."),
    ];
    if mode == "topic" {
        if let Some(topic) = topic {
            lines.push(format!("Topic hint: {topic}"));
        }
    }
    let text = lines.join("\n");
    base_result(
        "fresh_topic_confirmation_text",
        json!({
            "callback_group": "fresh_topic_confirmation",
            "confirmation_text": text,
            "text": text,
        }),
    )
}

fn plan_planning_mode_match(object: &Map<String, JsonValue>) -> JsonValue {
    let fallback = default_registry();
    let fallback_planning = fallback
        .get("planning_mode")
        .and_then(JsonValue::as_object)
        .expect("default planning_mode is object");
    let config = normalize_planning_mode_config(
        object.get("config").and_then(JsonValue::as_object),
        fallback_planning,
    );
    let trigger = parse_planning_mode_trigger(
        clean_text(object.get("text")).as_deref().unwrap_or(""),
        config
            .as_object()
            .expect("normalized planning mode config is object"),
    );
    base_result(
        "planning_mode_match",
        json!({
            "matched": trigger.is_some(),
            "trigger": trigger,
        }),
    )
}

fn plan_operational_match(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let config_value = object
        .get("config")
        .or_else(|| object.get("trigger"))
        .ok_or_else(|| "operational_match requires config".to_string())?;
    let config = config_value
        .as_object()
        .ok_or_else(|| "operational_match config must be an object".to_string())?;
    let trigger = parse_telegram_operational_trigger(
        clean_text(object.get("raw_text")).as_deref().unwrap_or(""),
        clean_text(object.get("normalized_text"))
            .as_deref()
            .unwrap_or(""),
        command_pair(object.get("command")),
        positive_i64(object.get("reply_to_message_id")),
        config,
    )?;
    Ok(base_result(
        "operational_match",
        json!({
            "matched": trigger.is_some(),
            "trigger": trigger,
        }),
    ))
}

fn plan_operational_dispatch(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let triggers = object
        .get("telegram_operational")
        .or_else(|| object.get("operational_triggers"))
        .or_else(|| object.get("triggers"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut normalized_triggers = triggers
        .iter()
        .filter_map(normalize_operational_dispatch_trigger)
        .collect::<Vec<_>>();
    sort_operational_triggers(&mut normalized_triggers);

    let raw_text = clean_text(object.get("raw_text")).unwrap_or_default();
    let normalized_text = clean_text(object.get("normalized_text")).unwrap_or_default();
    let command = command_pair(object.get("command"));
    let reply_to_message_id = positive_i64(object.get("reply_to_message_id"));

    for trigger_value in normalized_triggers {
        let Some(trigger_object) = trigger_value.as_object() else {
            continue;
        };
        let match_payload = parse_telegram_operational_trigger(
            raw_text.as_str(),
            normalized_text.as_str(),
            command.clone(),
            reply_to_message_id,
            trigger_object,
        )?;
        let Some(match_payload) = match_payload else {
            continue;
        };
        return Ok(base_result(
            "operational_dispatch",
            json!({
                "matched": true,
                "handled": true,
                "trigger": trigger_value,
                "match_payload": match_payload,
            }),
        ));
    }

    Ok(base_result(
        "operational_dispatch",
        json!({
            "matched": false,
            "handled": false,
            "trigger": JsonValue::Null,
            "match_payload": JsonValue::Null,
        }),
    ))
}

fn base_result(stage: &str, mut fields: JsonValue) -> JsonValue {
    let mut base = json!({
        "migration_stage": MIGRATION_STAGE,
        "event_trigger_contract": EVENT_TRIGGER_CONTRACT,
        "stage": stage,
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_event_trigger_allowed": false,
    });
    if let (Some(base), Some(fields)) = (base.as_object_mut(), fields.as_object_mut()) {
        for (key, value) in std::mem::take(fields) {
            base.insert(key, value);
        }
    }
    base
}

fn default_registry() -> JsonValue {
    json!({
        "fresh_topic": {
            "clear": {
                "phrases": DEFAULT_CLEAR_PHRASES,
                "display_trigger": "換個話題",
                "allow_trailing_punctuation": true,
            },
            "topic": {
                "lead_phrases": DEFAULT_TOPIC_LEAD_PHRASES,
                "joiners": DEFAULT_TOPIC_JOINERS,
                "tail": DEFAULT_TOPIC_TAIL,
                "display_trigger": "換個話題跟…有關",
                "allow_trailing_punctuation": true,
            },
        },
        "planning_mode": {
            "phrases": DEFAULT_PLANNING_MODE_PHRASES,
            "display_trigger": "進行計劃",
            "allow_trailing_punctuation": true,
        },
        "telegram_operational": [],
    })
}

fn normalize_fresh_topic_config(
    value: Option<&Map<String, JsonValue>>,
    fallback: &Map<String, JsonValue>,
) -> JsonValue {
    let clear_value = value
        .and_then(|value| value.get("clear"))
        .and_then(JsonValue::as_object);
    let topic_value = value
        .and_then(|value| value.get("topic"))
        .and_then(JsonValue::as_object);
    let fallback_clear = fallback
        .get("clear")
        .and_then(JsonValue::as_object)
        .expect("fallback clear is object");
    let fallback_topic = fallback
        .get("topic")
        .and_then(JsonValue::as_object)
        .expect("fallback topic is object");

    json!({
        "clear": {
            "phrases": clean_string_list(
                clear_value.and_then(|value| value.get("phrases")),
                string_array(fallback_clear.get("phrases")).as_slice(),
            ),
            "display_trigger": clean_text(clear_value.and_then(|value| value.get("displayTrigger")))
                .or_else(|| clean_text(clear_value.and_then(|value| value.get("display_trigger"))))
                .or_else(|| clean_text(fallback_clear.get("display_trigger")))
                .unwrap_or_else(|| "換個話題".to_string()),
            "allow_trailing_punctuation": bool_field(
                clear_value.and_then(|value| value.get("allowTrailingPunctuation"))
                    .or_else(|| clear_value.and_then(|value| value.get("allow_trailing_punctuation"))),
                bool_field(fallback_clear.get("allow_trailing_punctuation"), true),
            ),
        },
        "topic": {
            "lead_phrases": clean_string_list(
                topic_value.and_then(|value| value.get("leadPhrases"))
                    .or_else(|| topic_value.and_then(|value| value.get("lead_phrases"))),
                string_array(fallback_topic.get("lead_phrases")).as_slice(),
            ),
            "joiners": clean_string_list(
                topic_value.and_then(|value| value.get("joiners")),
                string_array(fallback_topic.get("joiners")).as_slice(),
            ),
            "tail": clean_text(topic_value.and_then(|value| value.get("tail")))
                .or_else(|| clean_text(fallback_topic.get("tail")))
                .unwrap_or_else(|| DEFAULT_TOPIC_TAIL.to_string()),
            "display_trigger": clean_text(topic_value.and_then(|value| value.get("displayTrigger")))
                .or_else(|| clean_text(topic_value.and_then(|value| value.get("display_trigger"))))
                .or_else(|| clean_text(fallback_topic.get("display_trigger")))
                .unwrap_or_else(|| "換個話題跟…有關".to_string()),
            "allow_trailing_punctuation": bool_field(
                topic_value.and_then(|value| value.get("allowTrailingPunctuation"))
                    .or_else(|| topic_value.and_then(|value| value.get("allow_trailing_punctuation"))),
                bool_field(fallback_topic.get("allow_trailing_punctuation"), true),
            ),
        },
    })
}

fn normalize_planning_mode_config(
    value: Option<&Map<String, JsonValue>>,
    fallback: &Map<String, JsonValue>,
) -> JsonValue {
    json!({
        "phrases": clean_string_list(
            value.and_then(|value| value.get("phrases")),
            string_array(fallback.get("phrases")).as_slice(),
        ),
        "display_trigger": clean_text(value.and_then(|value| value.get("displayTrigger")))
            .or_else(|| clean_text(value.and_then(|value| value.get("display_trigger"))))
            .or_else(|| clean_text(fallback.get("display_trigger")))
            .unwrap_or_else(|| "進行計劃".to_string()),
        "allow_trailing_punctuation": bool_field(
            value.and_then(|value| value.get("allowTrailingPunctuation"))
                .or_else(|| value.and_then(|value| value.get("allow_trailing_punctuation"))),
            bool_field(fallback.get("allow_trailing_punctuation"), true),
        ),
    })
}

fn parse_operational_trigger_from_value(value: &JsonValue) -> Option<JsonValue> {
    let object = value.as_object()?;
    if let Some(payload) = object.get("payload").and_then(JsonValue::as_object) {
        return parse_operational_trigger_config(
            payload,
            clean_text(object.get("source_path")).unwrap_or_default(),
        );
    }
    parse_operational_trigger_config(
        object,
        clean_text(object.get("source_path")).unwrap_or_default(),
    )
}

fn normalize_operational_dispatch_trigger(value: &JsonValue) -> Option<JsonValue> {
    parse_operational_trigger_from_value(value).or_else(|| {
        let object = value.as_object()?;
        let match_payload = object.get("match").and_then(JsonValue::as_object)?;
        let trigger_id = clean_text(object.get("trigger_id"))?;
        let handler_command = command_list(object.get("handler_command"));
        let phrases = clean_string_list(match_payload.get("phrases"), &[]);
        let commands = clean_command_names(match_payload.get("commands"));
        let pattern = clean_text(match_payload.get("pattern"));
        if handler_command.is_empty()
            || (phrases.is_empty() && commands.is_empty() && pattern.is_none())
        {
            return None;
        }
        Some(json!({
            "trigger_id": trigger_id,
            "display_trigger": clean_text(object.get("display_trigger"))
                .unwrap_or_else(|| trigger_id.clone()),
            "handler_command": handler_command,
            "source_path": clean_text(object.get("source_path")).unwrap_or_default(),
            "match": {
                "phrases": phrases,
                "commands": commands,
                "pattern": pattern,
                "allow_trailing_punctuation": bool_field(
                    match_payload.get("allow_trailing_punctuation"),
                    true,
                ),
                "reply_only": bool_field(match_payload.get("reply_only"), false),
                "case_sensitive": bool_field(match_payload.get("case_sensitive"), false),
            },
            "priority": optional_i64(object.get("priority")).unwrap_or(0),
        }))
    })
}

fn parse_operational_trigger_config(
    payload: &Map<String, JsonValue>,
    source_path: String,
) -> Option<JsonValue> {
    let kind = clean_text(payload.get("kind"))
        .unwrap_or_default()
        .to_lowercase();
    if !matches!(
        kind.as_str(),
        "telegram_operational_trigger" | "telegram-operational-trigger"
    ) {
        return None;
    }
    let match_payload = payload
        .get("match")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let handler_command = command_list(payload.get("handlerCommand"));
    let trigger_id = clean_text(payload.get("id"))?;
    let display_trigger =
        clean_text(payload.get("displayTrigger")).unwrap_or_else(|| trigger_id.clone());
    let phrases = clean_string_list(match_payload.get("phrases"), &[]);
    let commands = clean_command_names(match_payload.get("commands"));
    let pattern = clean_text(match_payload.get("pattern"));
    if handler_command.is_empty()
        || (phrases.is_empty() && commands.is_empty() && pattern.is_none())
    {
        return None;
    }
    Some(json!({
        "trigger_id": trigger_id,
        "display_trigger": display_trigger,
        "handler_command": handler_command,
        "source_path": source_path,
        "match": {
            "phrases": phrases,
            "commands": commands,
            "pattern": pattern,
            "allow_trailing_punctuation": bool_field(
                match_payload.get("allowTrailingPunctuation")
                    .or_else(|| match_payload.get("allow_trailing_punctuation")),
                true,
            ),
            "reply_only": bool_field(
                match_payload.get("replyOnly").or_else(|| match_payload.get("reply_only")),
                false,
            ),
            "case_sensitive": bool_field(
                match_payload.get("caseSensitive").or_else(|| match_payload.get("case_sensitive")),
                false,
            ),
        },
        "priority": optional_i64(payload.get("priority")).unwrap_or(0),
    }))
}

fn sort_operational_triggers(values: &mut [JsonValue]) {
    values.sort_by(|left, right| {
        let left_obj = left.as_object();
        let right_obj = right.as_object();
        let left_priority = left_obj
            .and_then(|value| optional_i64(value.get("priority")))
            .unwrap_or(0);
        let right_priority = right_obj
            .and_then(|value| optional_i64(value.get("priority")))
            .unwrap_or(0);
        right_priority
            .cmp(&left_priority)
            .then_with(|| {
                text_field(left_obj, "source_path").cmp(&text_field(right_obj, "source_path"))
            })
            .then_with(|| {
                text_field(left_obj, "trigger_id").cmp(&text_field(right_obj, "trigger_id"))
            })
            .then(Ordering::Equal)
    });
}

fn parse_fresh_topic_trigger(text: &str, config: &Map<String, JsonValue>) -> Option<JsonValue> {
    let raw = text.trim();
    if raw.is_empty() {
        return None;
    }
    let clear = config.get("clear")?.as_object()?;
    let topic = config.get("topic")?.as_object()?;
    let stripped = strip_allowed_trailing_punctuation(
        raw,
        bool_field(clear.get("allow_trailing_punctuation"), true)
            || bool_field(topic.get("allow_trailing_punctuation"), true),
    );
    let lowered = stripped.to_lowercase();
    for phrase in string_array(clear.get("phrases")) {
        if lowered == phrase.to_lowercase() {
            return Some(json!({
                "mode": "clear",
                "topic": JsonValue::Null,
                "display_trigger": clean_text(clear.get("display_trigger")).unwrap_or_default(),
            }));
        }
    }
    let leads = string_array(topic.get("lead_phrases"));
    let joiners = string_array(topic.get("joiners"));
    let tail = clean_text(topic.get("tail")).unwrap_or_default();
    for lead in &leads {
        let Some(after_lead) = strip_prefix_case_insensitive(&stripped, lead) else {
            continue;
        };
        for joiner in &joiners {
            let Some(after_joiner) = strip_prefix_case_insensitive(after_lead, joiner) else {
                continue;
            };
            let Some(candidate) = strip_suffix_case_insensitive(after_joiner, &tail) else {
                continue;
            };
            let topic_text = trim_trigger_punctuation(candidate);
            if !topic_text.is_empty() {
                return Some(json!({
                    "mode": "topic",
                    "topic": topic_text,
                    "display_trigger": clean_text(topic.get("display_trigger")).unwrap_or_default(),
                }));
            }
        }
    }
    None
}

fn parse_planning_mode_trigger(text: &str, config: &Map<String, JsonValue>) -> Option<JsonValue> {
    let raw = text.trim();
    if raw.is_empty() {
        return None;
    }
    let stripped = strip_allowed_trailing_punctuation(
        raw,
        bool_field(config.get("allow_trailing_punctuation"), true),
    );
    let lowered = stripped.to_lowercase();
    for phrase in string_array(config.get("phrases")) {
        if lowered == phrase.to_lowercase() {
            return Some(json!({
                "mode": "planning",
                "display_trigger": clean_text(config.get("display_trigger")).unwrap_or_default(),
            }));
        }
    }
    None
}

fn parse_telegram_operational_trigger(
    raw_text: &str,
    normalized_text: &str,
    command: Option<(String, String)>,
    reply_to_message_id: Option<i64>,
    config: &Map<String, JsonValue>,
) -> Result<Option<JsonValue>, String> {
    let match_config = config.get("match").and_then(JsonValue::as_object);
    let Some(match_config) = match_config else {
        return Ok(None);
    };
    if bool_field(match_config.get("reply_only"), false) && reply_to_message_id.is_none() {
        return Ok(None);
    }
    if let Some((command_name, command_args)) = command {
        let commands = string_array(match_config.get("commands"));
        if !commands.is_empty() {
            let normalized_name = command_name
                .trim()
                .trim_start_matches('/')
                .to_ascii_lowercase();
            if commands.iter().any(|name| name == &normalized_name) {
                return Ok(Some(json!({
                    "mode": "command",
                    "display_trigger": clean_text(config.get("display_trigger")).unwrap_or_default(),
                    "command_name": normalized_name,
                    "command_args": command_args.trim(),
                })));
            }
        }
    }

    let candidate = strip_allowed_trailing_punctuation(
        normalized_text,
        bool_field(match_config.get("allow_trailing_punctuation"), true),
    );
    if candidate.is_empty() {
        return Ok(None);
    }
    let case_sensitive = bool_field(match_config.get("case_sensitive"), false);
    let compare_candidate = if case_sensitive {
        candidate.clone()
    } else {
        candidate.to_lowercase()
    };
    for phrase in string_array(match_config.get("phrases")) {
        let compare_phrase = if case_sensitive {
            phrase
        } else {
            phrase.to_lowercase()
        };
        if compare_candidate == compare_phrase {
            return Ok(Some(json!({
                "mode": "phrase",
                "display_trigger": clean_text(config.get("display_trigger")).unwrap_or_default(),
                "matched_text": candidate,
            })));
        }
    }
    if let Some(pattern_text) = clean_text(match_config.get("pattern")) {
        let pattern = RegexBuilder::new(&pattern_text)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|exc| format!("invalid Telegram operational trigger pattern: {exc}"))?;
        let pattern_input = if raw_text.is_empty() {
            candidate.as_str()
        } else {
            raw_text
        };
        if let Some(captures) = pattern.captures(pattern_input) {
            if captures.get(0).is_none_or(|value| value.start() != 0) {
                return Ok(None);
            }
            let groups = (1..captures.len())
                .map(|index| {
                    captures
                        .get(index)
                        .map(|value| JsonValue::String(value.as_str().to_string()))
                        .unwrap_or(JsonValue::Null)
                })
                .collect::<Vec<_>>();
            let mut groupdict = Map::new();
            for name in pattern.capture_names().flatten() {
                if let Some(value) = captures.name(name) {
                    groupdict.insert(
                        name.to_string(),
                        JsonValue::String(value.as_str().to_string()),
                    );
                }
            }
            return Ok(Some(json!({
                "mode": "pattern",
                "display_trigger": clean_text(config.get("display_trigger")).unwrap_or_default(),
                "matched_text": captures.get(0).map(|value| value.as_str()).unwrap_or(""),
                "groups": groups,
                "groupdict": groupdict,
            })));
        }
    }
    Ok(None)
}

fn strip_allowed_trailing_punctuation(text: &str, enabled: bool) -> String {
    let mut output = text.trim().to_string();
    if !enabled {
        return output;
    }
    while output
        .chars()
        .last()
        .is_some_and(|ch| ch.is_whitespace() || is_trigger_punctuation(ch))
    {
        output.pop();
    }
    output.trim().to_string()
}

fn trim_trigger_punctuation(text: &str) -> String {
    text.trim()
        .trim_matches(|ch| ch == ' ' || is_trigger_punctuation(ch))
        .trim()
        .to_string()
}

fn is_trigger_punctuation(ch: char) -> bool {
    matches!(
        ch,
        ',' | '，' | '.' | '。' | '!' | '！' | '?' | '？' | ':' | '：' | ';' | '；'
    )
}

fn strip_prefix_case_insensitive<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let len = prefix.len();
    let candidate = text.get(..len)?;
    candidate.eq_ignore_ascii_case(prefix).then(|| &text[len..])
}

fn strip_suffix_case_insensitive<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let len = suffix.len();
    if len > text.len() {
        return None;
    }
    let start = text.len() - len;
    let candidate = text.get(start..)?;
    candidate
        .eq_ignore_ascii_case(suffix)
        .then(|| &text[..start])
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    let text = match value {
        JsonValue::String(text) => text.trim().to_string(),
        JsonValue::Null => return None,
        other => other.to_string().trim().to_string(),
    };
    (!text.is_empty()).then_some(text)
}

fn bool_field(value: Option<&JsonValue>, default: bool) -> bool {
    value.and_then(JsonValue::as_bool).unwrap_or(default)
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().map(|value| value as i64)),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn positive_i64(value: Option<&JsonValue>) -> Option<i64> {
    let value = optional_i64(value)?;
    (value > 0).then_some(value)
}

fn string_array(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| clean_text(Some(item)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn clean_string_list(value: Option<&JsonValue>, fallback: &[String]) -> Vec<String> {
    let Some(items) = value.and_then(JsonValue::as_array) else {
        return fallback.to_vec();
    };
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for item in items {
        let Some(cleaned) = clean_text(Some(item)) else {
            continue;
        };
        let key = cleaned.to_lowercase();
        if seen.insert(key) {
            normalized.push(cleaned);
        }
    }
    if normalized.is_empty() {
        fallback.to_vec()
    } else {
        normalized
    }
}

fn clean_command_names(value: Option<&JsonValue>) -> Vec<String> {
    let Some(items) = value.and_then(JsonValue::as_array) else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for item in items {
        let Some(cleaned) = clean_text(Some(item)) else {
            continue;
        };
        let name = cleaned.trim_start_matches('/').to_ascii_lowercase();
        if !name.is_empty() && seen.insert(name.clone()) {
            normalized.push(name);
        }
    }
    normalized
}

fn command_list(value: Option<&JsonValue>) -> Vec<String> {
    match value {
        Some(JsonValue::String(text)) => shell_split(text),
        Some(JsonValue::Array(items)) => items
            .iter()
            .filter_map(|item| clean_text(Some(item)))
            .collect(),
        _ => Vec::new(),
    }
}

fn shell_split(text: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in text.trim().chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                output.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if escaped {
        current.push('\\');
    }
    if !current.is_empty() {
        output.push(current);
    }
    output
}

fn command_pair(value: Option<&JsonValue>) -> Option<(String, String)> {
    match value? {
        JsonValue::Array(items) if items.len() >= 2 => Some((
            clean_text(items.first()).unwrap_or_default(),
            clean_text(items.get(1)).unwrap_or_default(),
        )),
        JsonValue::Object(object) => Some((
            clean_text(object.get("command_name")).unwrap_or_default(),
            clean_text(object.get("command_args")).unwrap_or_default(),
        )),
        _ => None,
    }
}

fn text_field(object: Option<&Map<String, JsonValue>>, key: &str) -> String {
    object
        .and_then(|object| clean_text(object.get(key)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
