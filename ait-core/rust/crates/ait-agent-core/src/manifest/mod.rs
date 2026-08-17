use std::collections::BTreeMap;
use std::str::FromStr;

use ait_core::json_support::{json, JsonValue};
use ait_core::worker_manifest::{
    default_worker_manifest_config_json, normalize_worker_manifest_document_json,
    select_telegram_worker_json, worker_manifest_ir_version, worker_manifest_schema_json,
};
use serde::{Deserialize, Serialize};

use crate::transport::TransportKind;

mod store;

pub use store::{AgentWorkerManifestDocument, AgentWorkerManifestStore};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorkerCount {
    pub transport: TransportKind,
    pub configured_workers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorkerSpec {
    pub key: String,
    pub transport: TransportKind,
    pub name: String,
}

pub fn normalize_agent_worker_manifest(payload: &JsonValue, path: Option<&str>) -> JsonValue {
    normalize_worker_manifest_document_json(payload, path)
}

pub fn agent_worker_manifest_ir_version() -> &'static str {
    worker_manifest_ir_version()
}

pub fn agent_worker_manifest_schema_json() -> JsonValue {
    worker_manifest_schema_json()
}

pub fn agent_default_worker_manifest_config_json() -> JsonValue {
    default_worker_manifest_config_json()
}

pub fn agent_normalize_worker_manifest_document_json(
    payload: &JsonValue,
    path: Option<&str>,
) -> JsonValue {
    normalize_agent_worker_manifest(payload, path)
}

pub fn agent_select_telegram_worker_json(
    config: &JsonValue,
    requested_name: Option<&str>,
) -> JsonValue {
    select_telegram_worker_json(config, requested_name)
}

pub fn count_manifest_workers(payload: &JsonValue) -> Vec<AgentWorkerCount> {
    let mut counts: BTreeMap<TransportKind, usize> = TransportKind::ALL
        .into_iter()
        .map(|kind| (kind, 0))
        .collect();
    for worker in list_manifest_workers(payload) {
        *counts.entry(worker.transport).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(transport, configured_workers)| AgentWorkerCount {
            transport,
            configured_workers,
        })
        .collect()
}

pub fn list_manifest_workers(payload: &JsonValue) -> Vec<AgentWorkerSpec> {
    let normalized = normalize_agent_worker_manifest(payload, None);
    let config = normalized.get("config").unwrap_or(&normalized);
    let workers = config
        .get("workers")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let mut specs = workers
        .into_iter()
        .filter_map(|(key, value)| {
            let kind_text = value
                .get("kind")
                .and_then(JsonValue::as_str)
                .or_else(|| key.split_once('/').map(|(kind, _)| kind))
                .unwrap_or("");
            let transport = TransportKind::from_str(kind_text).ok()?;
            let name = value
                .get("name")
                .and_then(JsonValue::as_str)
                .or_else(|| key.split_once('/').map(|(_, name)| name))
                .unwrap_or(&key)
                .trim()
                .to_string();
            Some(AgentWorkerSpec {
                key,
                transport,
                name: if name.is_empty() {
                    "worker".to_string()
                } else {
                    name
                },
            })
        })
        .collect::<Vec<_>>();
    specs.sort_by(|left, right| left.key.cmp(&right.key));
    specs
}

pub fn default_empty_manifest() -> JsonValue {
    json!({
        "version": 1,
        "workers": {}
    })
}

#[cfg(test)]
mod tests;
