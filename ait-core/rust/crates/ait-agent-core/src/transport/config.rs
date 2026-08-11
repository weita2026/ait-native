use ait_core::json_support::{json, JsonValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTransportConfigIntMode {
    PositiveOrMinimum,
    Minimum,
}

impl AgentTransportConfigIntMode {
    pub fn from_name(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "positive_or_minimum" | "positive" => Ok(Self::PositiveOrMinimum),
            "minimum" | "strict_minimum" => Ok(Self::Minimum),
            other => Err(format!(
                "unsupported ait-agent transport config int mode `{other}`"
            )),
        }
    }
}

pub fn agent_transport_config_clean_optional_text(value: Option<&str>) -> Option<String> {
    let text = value?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub fn agent_transport_config_normalize_base_url(value: Option<&str>, fallback: &str) -> String {
    let raw = value.unwrap_or_default().trim();
    let selected = if raw.is_empty() { fallback } else { raw };
    selected.trim_end_matches('/').to_string()
}

pub fn agent_transport_config_parse_timeout_seconds(
    value: Option<&str>,
    fallback: Option<f64>,
    minimum: f64,
) -> Option<f64> {
    let raw = value.unwrap_or_default().trim().to_ascii_lowercase();
    if raw.is_empty() {
        return fallback;
    }
    if matches!(raw.as_str(), "inf" | "infinite" | "none") {
        return None;
    }
    let Ok(parsed) = raw.parse::<f64>() else {
        return fallback;
    };
    if parsed <= 0.0 {
        return fallback;
    }
    Some(parsed.max(minimum))
}

pub fn agent_transport_config_parse_int(
    value: Option<&str>,
    fallback: i64,
    minimum: i64,
    mode: AgentTransportConfigIntMode,
) -> i64 {
    let raw = value.unwrap_or_default().trim();
    if raw.is_empty() {
        return fallback;
    }
    let Ok(parsed) = raw.parse::<i64>() else {
        return fallback;
    };
    match mode {
        AgentTransportConfigIntMode::PositiveOrMinimum => {
            if parsed <= 0 {
                fallback
            } else {
                parsed.max(minimum)
            }
        }
        AgentTransportConfigIntMode::Minimum => {
            if parsed < minimum {
                fallback
            } else {
                parsed
            }
        }
    }
}

pub fn agent_transport_config_split_message_chunks_json(text: &str, limit: usize) -> JsonValue {
    json!(agent_transport_config_split_message_chunks(text, limit))
}

pub fn agent_transport_config_split_message_chunks(text: &str, limit: usize) -> Vec<String> {
    let content = text.trim().to_string();
    if content.is_empty() {
        return vec!["(empty)".to_string()];
    }

    let effective_limit = limit.max(1);
    let threshold = effective_limit / 2;
    let mut chunks = Vec::new();
    let mut remaining = content;

    while remaining.chars().count() > effective_limit {
        let split_at = rfind_char_position_before_limit(&remaining, '\n', effective_limit)
            .filter(|index| *index >= threshold)
            .or_else(|| {
                rfind_char_position_before_limit(&remaining, ' ', effective_limit)
                    .filter(|index| *index >= threshold)
            })
            .unwrap_or(effective_limit);

        let chunk: String = remaining.chars().take(split_at).collect();
        let rest: String = remaining.chars().skip(split_at).collect();
        let chunk = chunk.trim_end().to_string();
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        remaining = rest.trim_start().to_string();
    }

    if !remaining.is_empty() {
        chunks.push(remaining);
    }
    chunks
}

fn rfind_char_position_before_limit(text: &str, needle: char, limit: usize) -> Option<usize> {
    text.chars()
        .take(limit)
        .enumerate()
        .filter_map(|(index, value)| (value == needle).then_some(index))
        .last()
}

#[cfg(test)]
mod tests;
