use std::collections::{BTreeMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map as JsonMap, Value as JsonValue};

pub const LIVE_TURNS_CONTRACT_VERSION: &str = "ait.server.live_turns.v1";

const DEFAULT_RECENT_COMPLETED_LIMIT: usize = 20;

#[derive(Debug, Clone)]
struct ActiveTurn {
    token: String,
    repo_name: String,
    surface: Option<String>,
    started_at_epoch_seconds: f64,
    metadata: JsonMap<String, JsonValue>,
}

#[derive(Debug, Clone)]
pub struct LiveTurnRegistry {
    recent_completed_limit: usize,
    active_turns: BTreeMap<String, ActiveTurn>,
    recent_finished_turns: VecDeque<JsonValue>,
}

impl LiveTurnRegistry {
    pub fn new(recent_completed_limit: usize) -> Result<Self, String> {
        if recent_completed_limit == 0 {
            return Err("recent_completed_limit must be greater than zero".to_string());
        }
        Ok(Self {
            recent_completed_limit,
            active_turns: BTreeMap::new(),
            recent_finished_turns: VecDeque::with_capacity(recent_completed_limit),
        })
    }

    pub fn start(
        &mut self,
        repo_name: &str,
        surface: Option<&str>,
        metadata: JsonMap<String, JsonValue>,
        extra_metadata: JsonMap<String, JsonValue>,
        started_at_epoch_seconds: f64,
        requested_token: Option<&str>,
    ) -> Result<String, String> {
        let repo_name = normalize_required_text(repo_name, "repo_name")?;
        let mut merged_metadata = metadata;
        for (key, value) in extra_metadata {
            merged_metadata.insert(key, value);
        }
        let token = match normalize_optional_text(requested_token) {
            Some(token) => {
                if self.active_turns.contains_key(&token) {
                    return Err("turn token already exists".to_string());
                }
                token
            }
            None => self.unique_token()?,
        };
        self.active_turns.insert(
            token.clone(),
            ActiveTurn {
                token: token.clone(),
                repo_name,
                surface: normalize_optional_text(surface),
                started_at_epoch_seconds,
                metadata: merged_metadata,
            },
        );
        Ok(token)
    }

    pub fn finish(
        &mut self,
        token: &str,
        completion_metadata: JsonMap<String, JsonValue>,
        finished_at_epoch_seconds: f64,
    ) -> JsonValue {
        let Some(token) = normalize_optional_text(Some(token)) else {
            return json!({});
        };
        let Some(active_turn) = self.active_turns.remove(&token) else {
            return json!({});
        };
        let outcome = completion_outcome(&completion_metadata);
        let failed = completion_failed(&outcome, &completion_metadata);
        let completion_payload = completion_metadata_payload(&completion_metadata);
        let mut completed_turn = JsonMap::new();
        completed_turn.insert("turn_token".to_string(), json!(active_turn.token));
        completed_turn.insert("repo_name".to_string(), json!(active_turn.repo_name));
        completed_turn.insert("surface".to_string(), json!(active_turn.surface));
        completed_turn.insert(
            "started_at_epoch_seconds".to_string(),
            json!(active_turn.started_at_epoch_seconds),
        );
        completed_turn.insert(
            "finished_at_epoch_seconds".to_string(),
            json!(finished_at_epoch_seconds),
        );
        completed_turn.insert(
            "duration_seconds".to_string(),
            json!((finished_at_epoch_seconds - active_turn.started_at_epoch_seconds).max(0.0)),
        );
        completed_turn.insert("outcome".to_string(), json!(outcome));
        completed_turn.insert("failed".to_string(), json!(failed));
        completed_turn.insert(
            "metadata".to_string(),
            JsonValue::Object(active_turn.metadata),
        );
        completed_turn.insert(
            "completion_metadata".to_string(),
            JsonValue::Object(completion_payload),
        );
        if let Some(error) = normalize_json_optional_text(completion_metadata.get("error")) {
            completed_turn.insert("error".to_string(), json!(error));
        }
        let completed = JsonValue::Object(completed_turn);
        if self.recent_finished_turns.len() == self.recent_completed_limit {
            self.recent_finished_turns.pop_front();
        }
        self.recent_finished_turns.push_back(completed.clone());
        completed
    }

    pub fn snapshot(
        &self,
        snapshot_at_epoch_seconds: f64,
        recent_completed_limit: Option<usize>,
    ) -> Result<JsonValue, String> {
        let active_turn_count = self.active_turns.len();
        let oldest_started_at = self
            .active_turns
            .values()
            .map(|turn| turn.started_at_epoch_seconds)
            .min_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        let mut active_turns_by_repo: BTreeMap<String, usize> = BTreeMap::new();
        for turn in self.active_turns.values() {
            *active_turns_by_repo
                .entry(turn.repo_name.clone())
                .or_insert(0) += 1;
        }

        let limit = recent_completed_limit.unwrap_or(self.recent_completed_limit);
        let recent_finished = self
            .recent_finished_turns
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        let recent_completed = recent_finished
            .iter()
            .filter(|item| !object_bool(item, "failed"))
            .cloned()
            .collect::<Vec<_>>();
        let recent_failed = recent_finished
            .iter()
            .filter(|item| object_bool(item, "failed"))
            .cloned()
            .collect::<Vec<_>>();
        let recent_completed_p95_seconds = p95_seconds(
            recent_completed
                .iter()
                .map(|item| object_f64(item, "duration_seconds").unwrap_or(0.0))
                .collect(),
        );
        let oldest_age =
            oldest_started_at.map(|started| (snapshot_at_epoch_seconds - started).max(0.0));

        Ok(json!({
            "active_turns": active_turn_count,
            "active_repositories": active_turns_by_repo,
            "oldest_active_turn_started_at": oldest_started_at,
            "oldest_active_turn_age_seconds": oldest_age,
            "recent_completed_turns": recent_completed,
            "recent_failed_turns": recent_failed,
            "recent_completed_p95_seconds": recent_completed_p95_seconds,
            "snapshot_at_epoch_seconds": snapshot_at_epoch_seconds,
            "active_turn_count": active_turn_count,
            "oldest_active_turn_started_at_epoch_seconds": oldest_started_at,
            "active_turns_by_repo": active_turns_by_repo,
            "recent_completed_turn_count": recent_completed.len(),
            "recent_failed_turn_count": recent_failed.len(),
        }))
    }

    fn unique_token(&self) -> Result<String, String> {
        loop {
            let mut bytes = [0_u8; 16];
            getrandom::fill(&mut bytes)
                .map_err(|exc| format!("Failed to generate live turn token: {exc}"))?;
            let token = hex_encode(&bytes);
            if !self.active_turns.contains_key(&token) {
                return Ok(token);
            }
        }
    }
}

pub fn live_turns_contract() -> JsonValue {
    json!({
        "contract": LIVE_TURNS_CONTRACT_VERSION,
        "reference_modules": [],
        "migration_status": "python_wrapper_removed_rust_owned",
        "state": {
            "storage": "in_memory",
            "default_recent_completed_limit": DEFAULT_RECENT_COMPLETED_LIMIT,
            "token_shape": "32 lowercase hex characters",
        },
        "operations": ["start", "finish", "snapshot"],
        "snapshot_fields": [
            "active_turns",
            "active_repositories",
            "oldest_active_turn_started_at",
            "oldest_active_turn_age_seconds",
            "recent_completed_turns",
            "recent_failed_turns",
            "recent_completed_p95_seconds",
            "snapshot_at_epoch_seconds",
            "active_turn_count",
            "oldest_active_turn_started_at_epoch_seconds",
            "active_turns_by_repo",
            "recent_completed_turn_count",
            "recent_failed_turn_count",
        ],
        "compatibility_notes": {
            "python_reference": "The former Python live-turns wrapper has been removed; this Rust contract owns token generation, registry state, finish shaping, and snapshots.",
            "durability": "Live-turn registry state remains in-memory runtime observation, not durable database authority.",
            "task_dag": "Task DAG is retired; live turns expose no graph-progress injection.",
        },
    })
}

pub fn live_turns_json_with_registry(
    registry: &mut LiveTurnRegistry,
    operation: &str,
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    match operation {
        "start" => {
            let repo_name = required_json_text(payload.get("repo_name"), "repo_name")?;
            let surface = value_text(payload.get("surface"));
            let metadata = json_object(payload.get("metadata"));
            let extra_metadata = json_object(payload.get("extra_metadata"));
            let started_at = value_f64(payload.get("started_at_epoch_seconds"))
                .unwrap_or_else(current_epoch_seconds);
            let requested_token = value_text(payload.get("turn_token"));
            let token = registry.start(
                &repo_name,
                surface.as_deref(),
                metadata,
                extra_metadata,
                started_at,
                requested_token.as_deref(),
            )?;
            Ok(json!({
                "contract": LIVE_TURNS_CONTRACT_VERSION,
                "turn_token": token,
            }))
        }
        "finish" => {
            let token = value_text(payload.get("turn_token")).unwrap_or_default();
            let completion_metadata = json_object(payload.get("completion_metadata"));
            let finished_at = value_f64(payload.get("finished_at_epoch_seconds"))
                .unwrap_or_else(current_epoch_seconds);
            Ok(json!({
                "contract": LIVE_TURNS_CONTRACT_VERSION,
                "turn": registry.finish(&token, completion_metadata, finished_at),
            }))
        }
        "snapshot" => {
            let now = value_f64(payload.get("now")).unwrap_or_else(current_epoch_seconds);
            let limit = recent_limit(payload.get("recent_completed_limit"))?;
            Ok(json!({
                "contract": LIVE_TURNS_CONTRACT_VERSION,
                "snapshot": registry.snapshot(now, limit)?,
            }))
        }
        other => Err(format!("Unsupported live turns operation `{other}`.")),
    }
}

fn completion_outcome(completion_metadata: &JsonMap<String, JsonValue>) -> String {
    for key in ["outcome", "status", "result"] {
        if let Some(value) = normalize_json_optional_text(completion_metadata.get(key)) {
            return value;
        }
    }
    if let Some(JsonValue::Bool(ok)) = completion_metadata.get("ok") {
        return if *ok { "ok" } else { "failed" }.to_string();
    }
    if normalize_json_optional_text(completion_metadata.get("error")).is_some() {
        return "failed".to_string();
    }
    "completed".to_string()
}

fn completion_failed(outcome: &str, completion_metadata: &JsonMap<String, JsonValue>) -> bool {
    if let Some(JsonValue::Bool(failed)) = completion_metadata.get("failed") {
        return *failed;
    }
    if let Some(JsonValue::Bool(ok)) = completion_metadata.get("ok") {
        return !*ok;
    }
    if normalize_json_optional_text(completion_metadata.get("error")).is_some() {
        return true;
    }
    matches!(
        outcome.trim().to_lowercase().as_str(),
        "error" | "failed" | "failure"
    )
}

fn completion_metadata_payload(
    completion_metadata: &JsonMap<String, JsonValue>,
) -> JsonMap<String, JsonValue> {
    let mut payload = json_object(completion_metadata.get("metadata"));
    for (key, value) in completion_metadata {
        if matches!(
            key.as_str(),
            "outcome" | "status" | "result" | "ok" | "failed" | "error" | "metadata"
        ) {
            continue;
        }
        payload.insert(key.clone(), value.clone());
    }
    payload
}

fn p95_seconds(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let rank = (values.len() * 95).div_ceil(100);
    Some(values[rank.saturating_sub(1)])
}

fn current_epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

fn recent_limit(value: Option<&JsonValue>) -> Result<Option<usize>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let limit = match value {
        JsonValue::Number(number) => number
            .as_i64()
            .ok_or_else(|| "recent_completed_limit must be an integer.".to_string())?,
        JsonValue::String(text) => text
            .trim()
            .parse::<i64>()
            .map_err(|_| "recent_completed_limit must be an integer.".to_string())?,
        JsonValue::Null => return Ok(None),
        _ => return Err("recent_completed_limit must be an integer.".to_string()),
    };
    if limit < 0 {
        return Err("recent_completed_limit must be greater than or equal to zero".to_string());
    }
    Ok(Some(limit as usize))
}

fn object_bool(value: &JsonValue, key: &str) -> bool {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn object_f64(value: &JsonValue, key: &str) -> Option<f64> {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(|value| value_f64(Some(value)))
}

fn json_object(value: Option<&JsonValue>) -> JsonMap<String, JsonValue> {
    value
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

fn required_json_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value_text(value)
        .and_then(|text| normalize_optional_text(Some(&text)))
        .ok_or_else(|| format!("{field} is required"))
}

fn normalize_required_text(value: &str, field: &str) -> Result<String, String> {
    normalize_optional_text(Some(value)).ok_or_else(|| format!("{field} is required"))
}

fn normalize_json_optional_text(value: Option<&JsonValue>) -> Option<String> {
    value_text(value).and_then(|text| normalize_optional_text(Some(&text)))
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    let text = value?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn value_text(value: Option<&JsonValue>) -> Option<String> {
    match value? {
        JsonValue::Null => None,
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Bool(value) => Some(if *value { "true" } else { "false" }.to_string()),
        JsonValue::Number(value) => Some(value.to_string()),
        other => Some(other.to_string()),
    }
}

fn value_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
