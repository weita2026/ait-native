use super::*;

pub const SERVER_PATCHSET_STORE_CONTRACT: &str = "ait.server.patchset_store.v1";

pub(super) const REQUIRED_APPROVALS: i64 = 1;
pub(super) const FAKE_POSTGRES_PREFIX: &str = "fake-postgres://";

pub fn server_patchset_store_json(
    operation: &str,
    request: &JsonValue,
) -> Result<JsonValue, String> {
    if operation == "contract" {
        return Ok(json!({
            "contract": SERVER_PATCHSET_STORE_CONTRACT,
            "backend": "postgres",
            "migration_status": "rust_owned_no_python_reference",
            "mutates_state": true,
            "operations": [
                "publish-patchset",
                "list-patchsets",
                "list-patchsets-for-repo",
                "get-patchset",
                "get-patchset-for-repo",
                "select-patchset",
                "upsert-attestation",
                "get-attestation",
            ],
        }));
    }
    let payload = request
        .as_object()
        .ok_or_else(|| "patchset-store payload must be a JSON object.".to_string())?;
    let runtime = PatchsetStoreRuntime::from_payload(payload)?;
    let mut store = PostgresPatchsetStore::connect(runtime)?;
    match operation {
        "publish-patchset" => {
            let change_id = required_text(payload.get("change_id"), "change_id")?;
            let base_snapshot_id =
                required_text(payload.get("base_snapshot_id"), "base_snapshot_id")?;
            let revision_snapshot_id =
                required_text(payload.get("revision_snapshot_id"), "revision_snapshot_id")?;
            let summary = optional_text(payload.get("summary")).unwrap_or_default();
            let author_mode =
                normalize_author_mode(optional_text(payload.get("author_mode")).as_deref())?;
            Ok(json!({
                "contract": SERVER_PATCHSET_STORE_CONTRACT,
                "patchset": store.publish_patchset(
                    &change_id,
                    &base_snapshot_id,
                    &revision_snapshot_id,
                    &summary,
                    &author_mode,
                )?,
            }))
        }
        "list-patchsets" => {
            let change_id = required_text(payload.get("change_id"), "change_id")?;
            Ok(json!({
                "contract": SERVER_PATCHSET_STORE_CONTRACT,
                "patchsets": store.list_patchsets(&change_id)?,
            }))
        }
        "list-patchsets-for-repo" => {
            let repo_name = required_text(payload.get("repo_name"), "repo_name")?;
            let change_ref = required_text(payload.get("change_ref"), "change_ref")?;
            Ok(json!({
                "contract": SERVER_PATCHSET_STORE_CONTRACT,
                "patchsets": store.list_patchsets_for_repo(&repo_name, &change_ref)?,
            }))
        }
        "get-patchset" => {
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            Ok(json!({
                "contract": SERVER_PATCHSET_STORE_CONTRACT,
                "patchset": store.get_patchset(&patchset_id)?,
            }))
        }
        "get-patchset-for-repo" => {
            let repo_name = required_text(payload.get("repo_name"), "repo_name")?;
            let patchset_ref = required_text(payload.get("patchset_ref"), "patchset_ref")?;
            let change_ref = optional_text(payload.get("change_ref"));
            Ok(json!({
                "contract": SERVER_PATCHSET_STORE_CONTRACT,
                "patchset": store.get_patchset_for_repo(&repo_name, &patchset_ref, change_ref.as_deref())?,
            }))
        }
        "select-patchset" => {
            let change_id = required_text(payload.get("change_id"), "change_id")?;
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            Ok(json!({
                "contract": SERVER_PATCHSET_STORE_CONTRACT,
                "change": store.select_patchset(&change_id, &patchset_id)?,
            }))
        }
        "upsert-attestation" => {
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            let author_mode =
                normalize_author_mode(optional_text(payload.get("author_mode")).as_deref())?;
            let evaluation_summary =
                payload_object(payload.get("evaluation_summary"), "evaluation_summary")?;
            let provenance_summary =
                payload_object(payload.get("provenance_summary"), "provenance_summary")?;
            let detail = payload
                .get("detail")
                .and_then(JsonValue::as_object)
                .cloned()
                .unwrap_or_default();
            Ok(json!({
                "contract": SERVER_PATCHSET_STORE_CONTRACT,
                "attestation": store.upsert_attestation(
                    &patchset_id,
                    &author_mode,
                    &evaluation_summary,
                    &provenance_summary,
                    &detail,
                )?,
            }))
        }
        "get-attestation" => {
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            Ok(json!({
                "contract": SERVER_PATCHSET_STORE_CONTRACT,
                "attestation": store.get_attestation(&patchset_id)?,
            }))
        }
        other => Err(format!("Unsupported patchset-store operation `{other}`.")),
    }
}

#[derive(Debug, Clone)]
pub(super) struct PatchsetStoreRuntime {
    pub(super) dsn: String,
    pub(super) content_schema: String,
    pub(super) control_schema: String,
    pub(super) root: Option<PathBuf>,
}

impl PatchsetStoreRuntime {
    fn from_payload(payload: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let backend =
            optional_text(payload.get("backend")).unwrap_or_else(|| "postgres".to_string());
        if backend != "postgres" {
            return Err(format!(
                "Unsupported ait-server patchset-store backend `{backend}`. Only PostgreSQL is supported."
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
                "fake-postgres is not supported for ait-server patchset-store runtime.".to_string(),
            );
        }
        let content_schema = optional_text(payload.get("content_schema"))
            .unwrap_or_else(|| DEFAULT_CONTENT_SCHEMA.to_string());
        let control_schema = optional_text(payload.get("control_schema"))
            .unwrap_or_else(|| DEFAULT_CONTROL_SCHEMA.to_string());
        ensure_postgres_schema_name(&content_schema)?;
        ensure_postgres_schema_name(&control_schema)?;
        let root = optional_text(payload.get("server_data"))
            .or_else(|| optional_text(payload.get("root")))
            .map(PathBuf::from);
        Ok(Self {
            dsn,
            content_schema,
            control_schema,
            root,
        })
    }
}
