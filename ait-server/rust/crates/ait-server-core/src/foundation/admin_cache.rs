use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Map as JsonMap, Value as JsonValue};

pub const ADMIN_CACHE_CONTRACT_VERSION: &str = "ait.server.admin_cache.v1";
pub const ADMIN_CACHE_TTL_ENV: &str = "AIT_SERVER_PRESSURE_METRICS_CACHE_TTL_SECONDS";
pub const DEFAULT_ADMIN_CACHE_TTL_SECONDS: f64 = 5.0;

#[derive(Debug, Clone, PartialEq)]
struct AdminCacheEntry {
    payload: JsonMap<String, JsonValue>,
    stored_at_monotonic: f64,
    cached_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdminResponseCache {
    entries: BTreeMap<(String, i64, i64), AdminCacheEntry>,
}

impl AdminResponseCache {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn cached_payload(
        &mut self,
        name: &str,
        key: (i64, i64),
        computed_payload: JsonMap<String, JsonValue>,
        ttl_seconds: f64,
        now_monotonic: f64,
        cached_at: &str,
    ) -> JsonValue {
        let ttl_seconds = ttl_seconds.max(0.0);
        let cache_key = (name.to_string(), key.0, key.1);
        if ttl_seconds > 0.0 {
            if let Some(entry) = self.entries.get(&cache_key) {
                let age = now_monotonic - entry.stored_at_monotonic;
                if age <= ttl_seconds {
                    return annotated_admin_payload(
                        entry.payload.clone(),
                        "cached",
                        age,
                        ttl_seconds,
                        &entry.cached_at,
                    );
                }
            }
        }

        let annotated = annotated_admin_payload(
            computed_payload.clone(),
            "computed",
            0.0,
            ttl_seconds,
            cached_at,
        );
        if ttl_seconds > 0.0 {
            self.entries.insert(
                cache_key,
                AdminCacheEntry {
                    payload: computed_payload,
                    stored_at_monotonic: now_monotonic,
                    cached_at: cached_at.to_string(),
                },
            );
        }
        annotated
    }
}

impl Default for AdminResponseCache {
    fn default() -> Self {
        Self::new()
    }
}

pub fn admin_cache_contract() -> JsonValue {
    json!({
        "contract": ADMIN_CACHE_CONTRACT_VERSION,
        "reference_modules": [],
        "migration_status": "python_wrapper_removed_rust_owned",
        "environment_inputs": {
            "ttl_seconds": ADMIN_CACHE_TTL_ENV,
        },
        "defaults": {
            "ttl_seconds": DEFAULT_ADMIN_CACHE_TTL_SECONDS,
            "ttl_invalid_fallback_seconds": DEFAULT_ADMIN_CACHE_TTL_SECONDS,
            "negative_ttl_clamps_to_seconds": 0.0,
        },
        "operations": [
            "ttl",
            "annotate",
            "cached-payload",
            "clear",
        ],
        "cache_key": {
            "shape": ["name", "key0", "key1"],
            "key_parts": 2,
        },
        "annotation_fields": [
            "cache_state",
            "cache_age_seconds",
            "cache_ttl_seconds",
            "cached_at",
        ],
        "compatibility_notes": {
            "python_reference": "The former Python admin-cache wrapper has been removed; this Rust contract owns TTL parsing, annotation, hit/miss, and clear behavior.",
            "durability": "Admin response cache is in-memory runtime optimization, not durable database authority.",
            "task_dag": "Task DAG is retired and is not an admin cache surface.",
        },
    })
}

pub fn admin_cache_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    if operation == "contract" {
        return Ok(admin_cache_contract());
    }
    let payload = request
        .as_object()
        .ok_or_else(|| "admin cache payload must be a JSON object.".to_string())?;
    let mut cache = default_admin_response_cache()
        .lock()
        .map_err(|_| "admin response cache lock is poisoned".to_string())?;
    admin_cache_json_with_cache(&mut cache, operation, payload)
}

pub fn admin_cache_json_with_cache(
    cache: &mut AdminResponseCache,
    operation: &str,
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    match operation {
        "ttl" => {
            let raw = value_text(payload.get("raw"));
            Ok(json!({
                "contract": ADMIN_CACHE_CONTRACT_VERSION,
                "cache_ttl_seconds": admin_metrics_cache_ttl_seconds(raw.as_deref()),
            }))
        }
        "annotate" => {
            let payload_object = json_object(payload.get("payload"));
            let cache_state = value_text(payload.get("cache_state"))
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "computed".to_string());
            let cache_age_seconds = value_f64(payload.get("cache_age_seconds")).unwrap_or(0.0);
            let cache_ttl_seconds = value_f64(payload.get("cache_ttl_seconds"))
                .unwrap_or(DEFAULT_ADMIN_CACHE_TTL_SECONDS)
                .max(0.0);
            let cached_at = required_text(payload.get("cached_at"), "cached_at")?;
            Ok(json!({
                "contract": ADMIN_CACHE_CONTRACT_VERSION,
                "payload": annotated_admin_payload(
                    payload_object,
                    &cache_state,
                    cache_age_seconds,
                    cache_ttl_seconds,
                    &cached_at,
                ),
            }))
        }
        "cached-payload" => {
            let name = required_text(payload.get("name"), "name")?;
            let key = cache_key(payload.get("key"))?;
            let computed_payload = json_object(payload.get("payload"));
            let ttl_seconds = value_text(payload.get("ttl_raw"))
                .map(|raw| admin_metrics_cache_ttl_seconds(Some(&raw)))
                .or_else(|| value_f64(payload.get("cache_ttl_seconds")).map(|value| value.max(0.0)))
                .unwrap_or(DEFAULT_ADMIN_CACHE_TTL_SECONDS);
            let now_monotonic = value_f64(payload.get("now_monotonic")).unwrap_or(0.0);
            let cached_at = required_text(payload.get("cached_at"), "cached_at")?;
            Ok(json!({
                "contract": ADMIN_CACHE_CONTRACT_VERSION,
                "payload": cache.cached_payload(
                    &name,
                    key,
                    computed_payload,
                    ttl_seconds,
                    now_monotonic,
                    &cached_at,
                ),
            }))
        }
        "clear" => {
            cache.clear();
            Ok(json!({
                "contract": ADMIN_CACHE_CONTRACT_VERSION,
                "cleared": true,
            }))
        }
        other => Err(format!("Unsupported admin cache operation `{other}`.")),
    }
}

pub fn admin_metrics_cache_ttl_seconds(raw: Option<&str>) -> f64 {
    let Some(raw) = raw else {
        return DEFAULT_ADMIN_CACHE_TTL_SECONDS;
    };
    match raw.trim().parse::<f64>() {
        Ok(value) => value.max(0.0),
        Err(_) => DEFAULT_ADMIN_CACHE_TTL_SECONDS,
    }
}

pub fn annotated_admin_payload(
    payload: JsonMap<String, JsonValue>,
    cache_state: &str,
    cache_age_seconds: f64,
    cache_ttl_seconds: f64,
    cached_at: &str,
) -> JsonValue {
    let mut annotated = payload;
    annotated.insert("cache_state".to_string(), json!(cache_state));
    annotated.insert(
        "cache_age_seconds".to_string(),
        json!(round_cache_age_seconds(cache_age_seconds.max(0.0))),
    );
    annotated.insert("cache_ttl_seconds".to_string(), json!(cache_ttl_seconds));
    annotated.insert("cached_at".to_string(), json!(cached_at));
    JsonValue::Object(annotated)
}

fn default_admin_response_cache() -> &'static Mutex<AdminResponseCache> {
    static CACHE: OnceLock<Mutex<AdminResponseCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(AdminResponseCache::new()))
}

fn round_cache_age_seconds(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn cache_key(value: Option<&JsonValue>) -> Result<(i64, i64), String> {
    let array = value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "key must be an array with exactly two integers.".to_string())?;
    if array.len() != 2 {
        return Err("key must be an array with exactly two integers.".to_string());
    }
    let first = value_i64(array.first(), "key[0]")?;
    let second = value_i64(array.get(1), "key[1]")?;
    Ok((first, second))
}

fn json_object(value: Option<&JsonValue>) -> JsonMap<String, JsonValue> {
    value
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value_text(value)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("Field `{field}` is required."))
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

fn value_i64(value: Option<&JsonValue>, field: &str) -> Result<i64, String> {
    match value {
        Some(JsonValue::Number(number)) => number
            .as_i64()
            .ok_or_else(|| format!("{field} must be an integer.")),
        Some(JsonValue::String(text)) => text
            .trim()
            .parse::<i64>()
            .map_err(|_| format!("{field} must be an integer.")),
        _ => Err(format!("{field} must be an integer.")),
    }
}
