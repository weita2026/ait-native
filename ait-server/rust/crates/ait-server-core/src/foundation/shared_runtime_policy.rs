use serde_json::{json, Value as JsonValue};

pub const SHARED_RUNTIME_POLICY_CONTRACT_VERSION: &str = "ait.server.shared_runtime_policy.v1";
pub const SHARED_RUNTIME_POLICY_REFERENCE_MODULE: &str =
    "../ait/src/ait_web/shared_runtime_policy.py";

const DEFAULT_RUNTIME_HOST: &str = "127.0.0.1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRuntimePolicy {
    pub component: String,
    pub db_backend: String,
    pub deployment_scope: String,
    pub state: String,
    pub ok: bool,
    pub override_active: bool,
    pub override_supported: bool,
    pub reason: String,
    pub server_host: String,
    pub web_host: String,
}

impl SharedRuntimePolicy {
    pub fn to_json(&self) -> JsonValue {
        json!({
            "component": self.component,
            "db_backend": self.db_backend,
            "deployment_scope": self.deployment_scope,
            "state": self.state,
            "ok": self.ok,
            "override_active": self.override_active,
            "override_supported": self.override_supported,
            "reason": self.reason,
            "server_host": self.server_host,
            "web_host": self.web_host,
        })
    }
}

pub fn shared_runtime_policy_contract() -> JsonValue {
    json!({
        "contract": SHARED_RUNTIME_POLICY_CONTRACT_VERSION,
        "reference_modules": [SHARED_RUNTIME_POLICY_REFERENCE_MODULE],
        "environment_inputs": {
            "shared_deployment_flag": "AIT_NATIVE_SHARED_DEPLOYMENT",
            "server_host": "AIT_NATIVE_SERVER_HOST",
            "web_host": "AIT_NATIVE_WEB_HOST",
            "db_backend": "AIT_NATIVE_SERVER_DB_BACKEND",
        },
        "operations": [
            "evaluate",
            "normalize-host",
            "is-loopback-host",
            "detect-shared-deployment",
        ],
        "policy": {
            "required_backend": "postgres",
            "allowed_backends": ["postgres"],
            "legacy_override_supported": false,
            "default_host": DEFAULT_RUNTIME_HOST,
        },
        "compatibility_notes": {
            "python_reference": "Web caller glue lives in ait_web.shared_runtime_policy; Rust owns the shared runtime policy contract.",
            "legacy_override": "allow_legacy_override is accepted only for compatibility and does not enable overrides.",
            "task_dag": "Task DAG is retired and is not a shared runtime policy surface.",
        },
    })
}

pub fn shared_runtime_policy_json(
    operation: &str,
    request: &JsonValue,
) -> Result<JsonValue, String> {
    if operation == "contract" {
        return Ok(shared_runtime_policy_contract());
    }
    let payload = request
        .as_object()
        .ok_or_else(|| "shared runtime policy payload must be a JSON object.".to_string())?;
    match operation {
        "evaluate" => {
            let component = required_text(payload.get("component"), "component")?;
            let db_backend = value_text(payload.get("db_backend"));
            let shared_deployment_flag = value_text(payload.get("shared_deployment_flag"))
                .or_else(|| value_text(payload.get("AIT_NATIVE_SHARED_DEPLOYMENT")));
            let server_host = value_text(payload.get("server_host"))
                .or_else(|| value_text(payload.get("AIT_NATIVE_SERVER_HOST")));
            let web_host = value_text(payload.get("web_host"))
                .or_else(|| value_text(payload.get("AIT_NATIVE_WEB_HOST")));
            let policy = evaluate_shared_runtime_policy(
                &component,
                db_backend.as_deref(),
                shared_deployment_flag.as_deref(),
                server_host.as_deref(),
                web_host.as_deref(),
            )?;
            Ok(json!({
                "contract": SHARED_RUNTIME_POLICY_CONTRACT_VERSION,
                "policy": policy.to_json(),
            }))
        }
        "normalize-host" => {
            let default = value_text(payload.get("default"))
                .unwrap_or_else(|| DEFAULT_RUNTIME_HOST.to_string());
            let host = value_text(payload.get("host"));
            Ok(json!({
                "contract": SHARED_RUNTIME_POLICY_CONTRACT_VERSION,
                "host": normalize_host(host.as_deref(), &default),
            }))
        }
        "is-loopback-host" => {
            let host = value_text(payload.get("host"));
            Ok(json!({
                "contract": SHARED_RUNTIME_POLICY_CONTRACT_VERSION,
                "is_loopback": is_loopback_host(host.as_deref()),
            }))
        }
        "detect-shared-deployment" => {
            let explicit = value_text(payload.get("shared_deployment_flag"))
                .or_else(|| value_text(payload.get("AIT_NATIVE_SHARED_DEPLOYMENT")));
            let server_host = value_text(payload.get("server_host"))
                .or_else(|| value_text(payload.get("AIT_NATIVE_SERVER_HOST")));
            let web_host = value_text(payload.get("web_host"))
                .or_else(|| value_text(payload.get("AIT_NATIVE_WEB_HOST")));
            let server_host = normalize_host(server_host.as_deref(), DEFAULT_RUNTIME_HOST);
            let web_host = normalize_host(web_host.as_deref(), DEFAULT_RUNTIME_HOST);
            let (shared, reason) = detect_shared_deployment(
                explicit.as_deref(),
                server_host.as_str(),
                web_host.as_str(),
            );
            Ok(json!({
                "contract": SHARED_RUNTIME_POLICY_CONTRACT_VERSION,
                "shared_deployment": shared,
                "reason": reason,
                "server_host": server_host,
                "web_host": web_host,
            }))
        }
        other => Err(format!(
            "Unsupported shared runtime policy operation `{other}`."
        )),
    }
}

pub fn evaluate_shared_runtime_policy(
    component: &str,
    db_backend: Option<&str>,
    shared_deployment_flag: Option<&str>,
    server_host: Option<&str>,
    web_host: Option<&str>,
) -> Result<SharedRuntimePolicy, String> {
    let component = component.to_string();
    let db_backend = normalize_backend(db_backend)?;
    let server_host = normalize_host(server_host, DEFAULT_RUNTIME_HOST);
    let web_host = normalize_host(web_host, DEFAULT_RUNTIME_HOST);
    let (shared_deployment, _detection_reason) =
        detect_shared_deployment(shared_deployment_flag, &server_host, &web_host);
    let deployment_scope = if shared_deployment { "shared" } else { "local" }.to_string();

    Ok(SharedRuntimePolicy {
        component,
        db_backend,
        deployment_scope,
        state: "postgres_compliant".to_string(),
        ok: true,
        override_active: false,
        override_supported: false,
        reason: "PostgreSQL-backed runtime satisfies the shared deployment policy.".to_string(),
        server_host,
        web_host,
    })
}

pub fn parse_env_flag(raw: Option<&str>) -> Option<bool> {
    let value = raw?.trim().to_lowercase();
    if value.is_empty() {
        return None;
    }
    if matches!(value.as_str(), "0" | "false" | "no" | "off") {
        return Some(false);
    }
    Some(true)
}

pub fn normalize_host(value: Option<&str>, default: &str) -> String {
    let mut host = value.unwrap_or(default).trim().to_string();
    if host.starts_with('[') && host.ends_with(']') {
        host = host[1..host.len() - 1].to_string();
    }
    if host.is_empty() {
        default.to_string()
    } else {
        host
    }
}

pub fn is_loopback_host(value: Option<&str>) -> bool {
    let host = normalize_host(value, DEFAULT_RUNTIME_HOST).to_lowercase();
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") || host.starts_with("127.")
}

pub fn detect_shared_deployment(
    explicit_flag: Option<&str>,
    server_host: &str,
    _web_host: &str,
) -> (bool, String) {
    match parse_env_flag(explicit_flag) {
        Some(true) => return (true, "AIT_NATIVE_SHARED_DEPLOYMENT=1".to_string()),
        Some(false) => return (false, "AIT_NATIVE_SHARED_DEPLOYMENT=0".to_string()),
        None => {}
    }
    if !is_loopback_host(Some(server_host)) {
        return (true, format!("server_host={server_host}"));
    }
    (false, "loopback_hosts".to_string())
}

fn normalize_backend(value: Option<&str>) -> Result<String, String> {
    let backend = value.unwrap_or("postgres").trim().to_lowercase();
    let backend = if backend.is_empty() {
        "postgres".to_string()
    } else {
        backend
    };
    if backend == "postgres" {
        Ok(backend)
    } else {
        Err(format!(
            "Unsupported AIT native server database backend: '{backend}'"
        ))
    }
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value_text(value)
        .filter(|text| !text.trim().is_empty())
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
