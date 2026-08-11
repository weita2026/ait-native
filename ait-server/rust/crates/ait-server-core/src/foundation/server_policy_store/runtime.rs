use super::*;

pub const SERVER_POLICY_STORE_CONTRACT: &str = "ait.server.policy_store.v1";

pub fn server_policy_store_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    if operation == "contract" {
        return Ok(json!({
            "contract": SERVER_POLICY_STORE_CONTRACT,
            "backend": "postgres",
            "migration_status": "rust_owned_no_python_reference",
            "mutates_state": true,
            "operations": [
                "get-policy",
                "evaluate-policy",
                "create-waiver",
            ],
        }));
    }
    let payload = request
        .as_object()
        .ok_or_else(|| "policy-store payload must be a JSON object.".to_string())?;
    let runtime = PolicyStoreRuntime::from_payload(payload)?;
    let mut store = PostgresPolicyStore::connect(runtime)?;
    match operation {
        "get-policy" => {
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            Ok(json!({
                "contract": SERVER_POLICY_STORE_CONTRACT,
                "policy": store.get_policy(&patchset_id)?,
            }))
        }
        "evaluate-policy" => {
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            Ok(json!({
                "contract": SERVER_POLICY_STORE_CONTRACT,
                "policy": store.evaluate_policy(&patchset_id)?,
            }))
        }
        "create-waiver" => {
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            let rule_name = required_text(payload.get("rule_name"), "rule_name")?;
            let reason = optional_text(payload.get("reason")).unwrap_or_default();
            let expires_at = optional_text(payload.get("expires_at"));
            let inline = payload.get("inline").map(truthy).unwrap_or(true);
            Ok(json!({
                "contract": SERVER_POLICY_STORE_CONTRACT,
                "waiver": store.create_waiver(&patchset_id, &rule_name, &reason, expires_at.as_deref(), inline)?,
            }))
        }
        other => Err(format!("Unsupported policy-store operation `{other}`.")),
    }
}

#[derive(Debug, Clone)]
pub(super) struct PolicyStoreRuntime {
    pub(super) dsn: String,
    pub(super) content_schema: String,
    pub(super) control_schema: String,
}

impl PolicyStoreRuntime {
    pub(super) fn from_payload(payload: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let backend =
            optional_text(payload.get("backend")).unwrap_or_else(|| "postgres".to_string());
        if backend != "postgres" {
            return Err(format!(
                "Unsupported ait-server policy-store backend `{backend}`. Only PostgreSQL is supported."
            ));
        }
        let dsn = optional_text(payload.get("dsn"))
            .or_else(|| optional_text(payload.get("postgres_dsn")))
            .ok_or_else(|| {
                "PostgreSQL backend requested but AIT_NATIVE_SERVER_POSTGRES_DSN is not configured"
                    .to_string()
            })?;
        if dsn.starts_with(FAKE_POSTGRES_PREFIX) {
            return Err(
                "fake-postgres is not supported for ait-server policy-store runtime.".to_string(),
            );
        }
        let content_schema = optional_text(payload.get("content_schema"))
            .unwrap_or_else(|| DEFAULT_CONTENT_SCHEMA.to_string());
        let control_schema = optional_text(payload.get("control_schema"))
            .unwrap_or_else(|| DEFAULT_CONTROL_SCHEMA.to_string());
        ensure_postgres_schema_name(&content_schema)?;
        ensure_postgres_schema_name(&control_schema)?;
        Ok(Self {
            dsn,
            content_schema,
            control_schema,
        })
    }
}
