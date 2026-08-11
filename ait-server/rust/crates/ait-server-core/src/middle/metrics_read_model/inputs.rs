use super::helpers::*;
use super::*;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeMetricsInput {
    pub live_turn_metrics: JsonValue,
}

impl RuntimeMetricsInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let obj =
            read_model_payload_object(value, runtime_metrics_read_model_contract().payload_label)?;
        let live_turn_metrics = obj
            .get("live_turn_metrics")
            .cloned()
            .unwrap_or_else(|| JsonValue::Object(obj.clone()));
        if !live_turn_metrics.is_object() && !live_turn_metrics.is_null() {
            return Err("`live_turn_metrics` must be a JSON object when present.".to_string());
        }
        Ok(Self { live_turn_metrics })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperatorMetricsInput {
    pub repo_name: String,
    pub snapshot_at: Option<String>,
    pub recent_jobs_limit: usize,
    pub stale_after_seconds: i64,
    pub cache_state: Option<String>,
    pub cache_age_seconds: f64,
    pub cache_ttl_seconds: Option<f64>,
    pub cached_at: Option<String>,
    pub db_backend: String,
    pub using_postgres: bool,
    pub server_data_root: Option<String>,
    pub live_turn_metrics: JsonValue,
    pub repositories: Vec<JsonMap<String, JsonValue>>,
    pub repository_storage: Vec<JsonMap<String, JsonValue>>,
    pub repository_workers: Vec<JsonMap<String, JsonValue>>,
    pub jobs: Vec<JsonMap<String, JsonValue>>,
    pub job_diagnostics: Vec<JsonMap<String, JsonValue>>,
    pub shared_runtime_policy: Vec<JsonMap<String, JsonValue>>,
    pub rust_server_core_seam: Vec<JsonMap<String, JsonValue>>,
    pub postgres_schema: Vec<JsonMap<String, JsonValue>>,
}

impl OperatorMetricsInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let contract = operator_metrics_read_model_contract();
        let obj = read_model_payload_object(value, contract.payload_label)?;
        let mut rows = ReadModelRows::from_object(obj, contract)?;
        let recent_jobs_limit =
            optional_i64(obj, "recent_jobs_limit")?.unwrap_or(50).max(0) as usize;
        let stale_after_seconds = optional_i64(obj, "stale_after_seconds")?.unwrap_or(300);
        if stale_after_seconds <= 0 {
            return Err("`stale_after_seconds` must be greater than zero.".to_string());
        }
        Ok(Self {
            repo_name: optional_text(obj, "repo_name").unwrap_or_else(|| "ait".to_string()),
            snapshot_at: optional_text(obj, "snapshot_at"),
            recent_jobs_limit,
            stale_after_seconds,
            cache_state: optional_text(obj, "cache_state"),
            cache_age_seconds: optional_f64(obj, "cache_age_seconds")?
                .unwrap_or(0.0)
                .max(0.0),
            cache_ttl_seconds: optional_f64(obj, "cache_ttl_seconds")?.map(|value| value.max(0.0)),
            cached_at: optional_text(obj, "cached_at"),
            db_backend: optional_text(obj, "db_backend").unwrap_or_else(|| "postgres".to_string()),
            using_postgres: optional_bool(obj, "using_postgres").unwrap_or_else(|| {
                optional_text(obj, "db_backend")
                    .map(|backend| backend == "postgres")
                    .unwrap_or(true)
            }),
            server_data_root: optional_text(obj, "server_data_root"),
            live_turn_metrics: obj
                .get("live_turn_metrics")
                .cloned()
                .unwrap_or_else(|| json!({})),
            repositories: rows.take("repositories"),
            repository_storage: rows.take("repository_storage"),
            repository_workers: rows.take("repository_workers"),
            jobs: rows.take("jobs"),
            job_diagnostics: rows.take("job_diagnostics"),
            shared_runtime_policy: rows.take("shared_runtime_policy"),
            rust_server_core_seam: rows.take("rust_server_core_seam"),
            postgres_schema: rows.take("postgres_schema"),
        })
    }
}
