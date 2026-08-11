use serde_json::{json, Map as JsonMap, Value as JsonValue};

pub const AGENT_SERVER_PROTOCOL_VERSION: &str = "ait.agent_server_protocol.v2";

pub const AGENT_SERVER_JOB_KINDS: &[&str] = &["agent.turn.submit"];

pub struct AgentProtocolJson<S> {
    store: S,
}

impl<S> AgentProtocolJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl AgentProtocolJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> AgentProtocolJson<S> {
    pub fn protocol_version(&self) -> &'static str {
        let _ = &self.store;
        AGENT_SERVER_PROTOCOL_VERSION
    }

    pub fn supported_job_kinds(&self) -> Vec<String> {
        let _ = &self.store;
        AGENT_SERVER_JOB_KINDS
            .iter()
            .map(|value| (*value).to_string())
            .collect()
    }

    pub fn schema_json(&self) -> JsonValue {
        let _ = &self.store;
        agent_server_protocol_schema_json_value()
    }

    pub fn normalize_job_json(&self, request_json: &str) -> Result<JsonValue, String> {
        let _ = &self.store;
        normalize_agent_server_job_json_impl(request_json)
    }
}

pub fn agent_server_protocol_version() -> &'static str {
    AgentProtocolJson::stateless().protocol_version()
}

pub fn agent_server_supported_job_kinds() -> Vec<String> {
    AgentProtocolJson::stateless().supported_job_kinds()
}

pub fn agent_server_protocol_schema_json() -> JsonValue {
    AgentProtocolJson::stateless().schema_json()
}

pub fn normalize_agent_server_job_json(request_json: &str) -> Result<JsonValue, String> {
    AgentProtocolJson::stateless().normalize_job_json(request_json)
}

fn agent_server_protocol_schema_json_value() -> JsonValue {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://ait.dev/schema/ait.agent_server_protocol.v2.schema.json",
        "title": "AitAgentServerProtocol",
        "type": "object",
        "additionalProperties": false,
        "required": [
            "contract_version",
            "job_kind",
            "repo_name",
            "idempotency_key",
            "payload",
            "singleflight_key",
            "read_keys",
            "write_keys",
            "cpu_tokens",
            "io_tokens",
            "remote_tokens",
            "db_tokens",
            "priority",
            "lease_timeout_seconds",
            "retry_policy"
        ],
        "properties": {
            "contract_version": {"const": AGENT_SERVER_PROTOCOL_VERSION},
            "job_kind": {"enum": AGENT_SERVER_JOB_KINDS},
            "repo_name": {"type": "string", "minLength": 1},
            "idempotency_key": {"type": "string", "minLength": 1},
            "payload": {"type": "object"},
            "singleflight_key": {"type": "string", "minLength": 1},
            "read_keys": {"type": "array", "items": {"type": "string"}},
            "write_keys": {"type": "array", "items": {"type": "string"}},
            "cpu_tokens": {"type": "integer", "minimum": 0},
            "io_tokens": {"type": "integer", "minimum": 0},
            "remote_tokens": {"type": "integer", "minimum": 0},
            "db_tokens": {"type": "integer", "minimum": 0},
            "priority": {"type": "integer"},
            "lease_timeout_seconds": {"type": "integer", "minimum": 1},
            "retry_policy": {
                "type": "object",
                "additionalProperties": false,
                "required": ["max_attempts", "delay_seconds"],
                "properties": {
                    "max_attempts": {"type": "integer", "minimum": 1},
                    "delay_seconds": {"type": "integer", "minimum": 0}
                }
            }
        }
    })
}

fn normalize_agent_server_job_json_impl(request_json: &str) -> Result<JsonValue, String> {
    let request = parse_json_object(request_json, "agent server job request")?;
    reject_unknown_request_fields(&request)?;
    let job_kind = normalize_job_kind(
        request
            .get("job_kind")
            .or_else(|| request.get("operation_kind")),
    )?;
    let repo_name = require_nonempty_text(request.get("repo_name"), "repo_name")?;
    let idempotency_key = require_nonempty_text(request.get("idempotency_key"), "idempotency_key")?;
    let payload = require_payload_object(request.get("payload"))?;
    let transport = optional_nonempty_text(request.get("transport"), "transport")?;
    let singleflight_key = format!("agent:{repo_name}:{job_kind}:{idempotency_key}");
    let resource_keys = resource_keys_for_job(&repo_name, &job_kind, &idempotency_key);

    let mut out = JsonMap::from_iter([
        (
            "contract_version".to_string(),
            JsonValue::String(AGENT_SERVER_PROTOCOL_VERSION.to_string()),
        ),
        ("job_kind".to_string(), JsonValue::String(job_kind.clone())),
        (
            "repo_name".to_string(),
            JsonValue::String(repo_name.clone()),
        ),
        (
            "idempotency_key".to_string(),
            JsonValue::String(idempotency_key.clone()),
        ),
        ("payload".to_string(), JsonValue::Object(payload)),
        (
            "singleflight_key".to_string(),
            JsonValue::String(singleflight_key),
        ),
        (
            "read_keys".to_string(),
            JsonValue::Array(resource_keys.read_keys),
        ),
        (
            "write_keys".to_string(),
            JsonValue::Array(resource_keys.write_keys),
        ),
        ("cpu_tokens".to_string(), JsonValue::from(1)),
        ("io_tokens".to_string(), JsonValue::from(1)),
        ("remote_tokens".to_string(), JsonValue::from(0)),
        ("db_tokens".to_string(), JsonValue::from(1)),
        ("priority".to_string(), JsonValue::from(50)),
        ("lease_timeout_seconds".to_string(), JsonValue::from(120)),
        (
            "retry_policy".to_string(),
            json!({"max_attempts": 8, "delay_seconds": 3}),
        ),
    ]);
    out.insert(
        "transport".to_string(),
        transport.map(JsonValue::String).unwrap_or(JsonValue::Null),
    );
    Ok(JsonValue::Object(out))
}

struct ResourceKeys {
    read_keys: Vec<JsonValue>,
    write_keys: Vec<JsonValue>,
}

fn resource_keys_for_job(repo_name: &str, job_kind: &str, idempotency_key: &str) -> ResourceKeys {
    let repo_key = format!("repo:{repo_name}");
    match job_kind {
        "agent.turn.submit" => ResourceKeys {
            read_keys: Vec::new(),
            write_keys: vec![JsonValue::String(format!(
                "{repo_key}:agent-turn:{idempotency_key}"
            ))],
        },
        _ => ResourceKeys {
            read_keys: Vec::new(),
            write_keys: vec![JsonValue::String(repo_key)],
        },
    }
}

fn normalize_job_kind(value: Option<&JsonValue>) -> Result<String, String> {
    let job_kind = require_nonempty_text(value, "job_kind")?;
    if !AGENT_SERVER_JOB_KINDS.contains(&job_kind.as_str()) {
        return Err(format!(
            "agent server job_kind must be one of: {}.",
            AGENT_SERVER_JOB_KINDS.join(", ")
        ));
    }
    Ok(job_kind)
}

fn reject_unknown_request_fields(request: &JsonMap<String, JsonValue>) -> Result<(), String> {
    const ALLOWED_FIELDS: &[&str] = &[
        "job_kind",
        "operation_kind",
        "repo_name",
        "idempotency_key",
        "payload",
        "transport",
    ];
    let unsupported = request
        .keys()
        .filter(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "agent server job has unsupported field(s): {}.",
            unsupported.join(", ")
        ))
    }
}

fn require_payload_object(value: Option<&JsonValue>) -> Result<JsonMap<String, JsonValue>, String> {
    match value {
        Some(JsonValue::Object(map)) => Ok(map.clone()),
        _ => Err("agent server job payload must be a JSON object.".to_string()),
    }
}

fn parse_json_object(
    payload_json: &str,
    label: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    match serde_json::from_str::<JsonValue>(payload_json)
        .map_err(|err| format!("{label} must be valid JSON: {err}"))?
    {
        JsonValue::Object(map) => Ok(map),
        _ => Err(format!("{label} must be a JSON object.")),
    }
}

fn require_nonempty_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_nonempty_text(value, field)?
        .ok_or_else(|| format!("agent server job field `{field}` is required."))
}

fn optional_nonempty_text(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => {
            let trimmed = text.trim();
            Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
        }
        _ => Err(format!(
            "agent server job field `{field}` must be a string or null."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_protocol_json_wrapper_preserves_schema_and_scheduler_contract() {
        let contract = AgentProtocolJson::stateless();

        assert_eq!(contract.protocol_version(), AGENT_SERVER_PROTOCOL_VERSION);
        assert_eq!(
            contract.supported_job_kinds(),
            vec!["agent.turn.submit".to_string()]
        );
        assert_eq!(contract.schema_json(), agent_server_protocol_schema_json());

        let payload = contract
            .normalize_job_json(
                &json!({
                    "operation_kind": "agent.turn.submit",
                    "repo_name": "ait",
                    "idempotency_key": "idem-wrapper",
                    "transport": "slack",
                    "payload": {"message": "hello"}
                })
                .to_string(),
            )
            .expect("agent job should normalize through wrapper");

        assert_eq!(payload["contract_version"], AGENT_SERVER_PROTOCOL_VERSION);
        assert_eq!(payload["job_kind"], "agent.turn.submit");
        assert_eq!(
            payload["singleflight_key"],
            "agent:ait:agent.turn.submit:idem-wrapper"
        );
        assert_eq!(
            payload["write_keys"],
            json!(["repo:ait:agent-turn:idem-wrapper"])
        );
        assert_eq!(payload["transport"], "slack");
        assert_eq!(
            payload["retry_policy"],
            json!({"max_attempts": 8, "delay_seconds": 3})
        );
    }

    #[test]
    fn agent_protocol_json_wrapper_preserves_stable_error_text() {
        let contract = AgentProtocolJson::stateless();

        assert!(contract
            .normalize_job_json("{bad-json")
            .expect_err("malformed JSON should fail")
            .starts_with("agent server job request must be valid JSON:"));
        assert_eq!(
            contract
                .normalize_job_json("[]")
                .expect_err("non-object JSON should fail"),
            "agent server job request must be a JSON object."
        );
        assert_eq!(
            contract
                .normalize_job_json(
                    &json!({
                        "job_kind": "agent.turn.submit",
                        "idempotency_key": "idem-missing-repo",
                        "payload": {}
                    })
                    .to_string()
                )
                .expect_err("missing repo_name should fail"),
            "agent server job field `repo_name` is required."
        );
        assert_eq!(
            contract
                .normalize_job_json(
                    &json!({
                        "job_kind": "agent.turn.submit",
                        "repo_name": "ait",
                        "idempotency_key": "idem-bad-payload",
                        "payload": []
                    })
                    .to_string()
                )
                .expect_err("non-object payload should fail"),
            "agent server job payload must be a JSON object."
        );
    }

    #[test]
    fn normalizes_turn_submission_into_scheduler_shape() {
        let payload = normalize_agent_server_job_json(
            &json!({
                "job_kind": "agent.turn.submit",
                "repo_name": "ait",
                "idempotency_key": "idem-1",
                "transport": "telegram",
                "payload": {"message": "hello"}
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(payload["contract_version"], AGENT_SERVER_PROTOCOL_VERSION);
        assert_eq!(
            payload["singleflight_key"],
            "agent:ait:agent.turn.submit:idem-1"
        );
        assert_eq!(payload["read_keys"], json!([]));
        assert_eq!(payload["write_keys"], json!(["repo:ait:agent-turn:idem-1"]));
        assert_eq!(payload["retry_policy"]["max_attempts"], 8);
    }

    #[test]
    fn rejects_retired_session_jobs_and_fields() {
        let error = normalize_agent_server_job_json(
            &json!({
                "operation_kind": "agent.session.create",
                "repo_name": "ait",
                "idempotency_key": "idem-2",
                "payload": {"transport": "slack"}
            })
            .to_string(),
        )
        .expect_err("retired session job should fail");
        assert!(error.contains("agent server job_kind must be one of"));

        let error = normalize_agent_server_job_json(
            &json!({
                "job_kind": "agent.turn.submit",
                "repo_name": "ait",
                "session_id": "SES-legacy",
                "idempotency_key": "idem-legacy",
                "payload": {}
            })
            .to_string(),
        )
        .expect_err("retired session field should fail");
        assert_eq!(
            error,
            "agent server job has unsupported field(s): session_id."
        );
    }

    #[test]
    fn rejects_unknown_agent_job_kind() {
        let err = normalize_agent_server_job_json(
            &json!({
                "job_kind": "repo.ci",
                "repo_name": "ait",
                "idempotency_key": "idem-3",
                "payload": {}
            })
            .to_string(),
        )
        .unwrap_err();

        assert!(err.contains("agent server job_kind must be one of"));
    }
}
