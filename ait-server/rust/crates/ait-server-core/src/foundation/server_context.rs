use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Map as JsonMap, Value as JsonValue};

use crate::foundation::server_protocol::{
    resolve_server_runtime_root_with_source, SERVER_DATA_ENV,
};

pub const SERVER_CONTEXT_CONTRACT_VERSION: &str = "ait.server.server_context.v1";
pub const SERVER_RUNTIME_PREFLIGHT_REFERENCE_MODULE: &str =
    "../ait/src/ait/server_runtime_preflight.py";

pub const SERVER_DB_BACKEND_ENV: &str = "AIT_NATIVE_SERVER_DB_BACKEND";
pub const SERVER_POSTGRES_DSN_ENV: &str = "AIT_NATIVE_SERVER_POSTGRES_DSN";
pub const SERVER_POSTGRES_CONTENT_SCHEMA_ENV: &str = "AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA";
pub const SERVER_POSTGRES_CONTROL_SCHEMA_ENV: &str = "AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA";
pub const DEFAULT_CONTENT_SCHEMA: &str = "ait_native_content";
pub const DEFAULT_CONTROL_SCHEMA: &str = "ait_native_control";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerContextShape {
    pub root: PathBuf,
    pub manifest_dir: PathBuf,
    pub pack_dir: PathBuf,
    pub tree_pack_dir: PathBuf,
    pub ref_root: PathBuf,
    pub db_backend: String,
    pub postgres_dsn: Option<String>,
    pub content_schema: String,
    pub control_schema: String,
    pub root_source: String,
}

impl ServerContextShape {
    pub fn to_json(&self) -> JsonValue {
        json!({
            "root": path_text(&self.root),
            "manifest_dir": path_text(&self.manifest_dir),
            "pack_dir": path_text(&self.pack_dir),
            "tree_pack_dir": path_text(&self.tree_pack_dir),
            "ref_root": path_text(&self.ref_root),
            "db_backend": self.db_backend,
            "postgres_dsn": self.postgres_dsn,
            "content_schema": self.content_schema,
            "control_schema": self.control_schema,
            "root_source": self.root_source,
            "using_postgres": self.db_backend == "postgres",
        })
    }
}

pub fn server_context_contract() -> JsonValue {
    json!({
        "contract": SERVER_CONTEXT_CONTRACT_VERSION,
        "reference_modules": [
            SERVER_RUNTIME_PREFLIGHT_REFERENCE_MODULE,
        ],
        "environment_inputs": {
            "server_data": SERVER_DATA_ENV,
            "db_backend": SERVER_DB_BACKEND_ENV,
            "postgres_dsn": SERVER_POSTGRES_DSN_ENV,
            "content_schema": SERVER_POSTGRES_CONTENT_SCHEMA_ENV,
            "control_schema": SERVER_POSTGRES_CONTROL_SCHEMA_ENV,
        },
        "defaults": {
            "create_backend": "postgres",
            "content_schema": DEFAULT_CONTENT_SCHEMA,
            "control_schema": DEFAULT_CONTROL_SCHEMA,
        },
        "side_effects": {
            "ensure_directories": "create/from-env only create runtime directories when ensure_directories is true",
        },
        "operations": [
            "create",
            "from-env",
            "resolve-root",
        ],
        "path_fields": [
            "root",
            "manifest_dir",
            "pack_dir",
            "tree_pack_dir",
            "ref_root",
        ],
        "compatibility_notes": {
            "python_reference": "ServerContext caller glue lives outside ait_server in ait.server_runtime_preflight; Rust owns validation and server runtime behavior.",
            "protocol_reference": "server_protocol_seam.py was removed in ../ait LT-1940/LC-1775 after callers imported protocol helpers directly.",
            "postgres_only": "Only PostgreSQL-backed ait-server runtime state is supported.",
            "process_lifecycle": "Process start/stop/restart stays packaging scope outside this contract.",
            "task_dag": "Task DAG is retired and is not a server context/path surface.",
        },
    })
}

pub fn server_context_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    if operation == "contract" {
        return Ok(server_context_contract());
    }
    let payload = request
        .as_object()
        .ok_or_else(|| "server context payload must be a JSON object.".to_string())?;
    match operation {
        "create" => {
            let root = required_text(payload.get("root"), "root")?;
            let backend = value_text(payload.get("backend"));
            let postgres_dsn = normalize_optional_text(value_text(payload.get("postgres_dsn")));
            let content_schema = value_text(payload.get("content_schema"))
                .unwrap_or_else(|| DEFAULT_CONTENT_SCHEMA.to_string());
            let control_schema = value_text(payload.get("control_schema"))
                .unwrap_or_else(|| DEFAULT_CONTROL_SCHEMA.to_string());
            let root_source =
                value_text(payload.get("root_source")).unwrap_or_else(|| "explicit".to_string());
            let context = create_server_context(
                Path::new(&root),
                backend.as_deref(),
                postgres_dsn,
                &content_schema,
                &control_schema,
                &root_source,
            )?;
            maybe_ensure_context_directories(&context, truthy(payload.get("ensure_directories")))?;
            Ok(json!({
                "contract": SERVER_CONTEXT_CONTRACT_VERSION,
                "context": context.to_json(),
            }))
        }
        "from-env" => {
            let env = json_object(payload.get("env"));
            let context = server_context_from_env_map(&env)?;
            maybe_ensure_context_directories(&context, truthy(payload.get("ensure_directories")))?;
            Ok(json!({
                "contract": SERVER_CONTEXT_CONTRACT_VERSION,
                "context": context.to_json(),
            }))
        }
        "resolve-root" => {
            let explicit = value_text(payload.get("root"));
            let root = if let Some(explicit) = explicit.as_deref() {
                resolve_root_from_inputs(Some(explicit), None)?
            } else {
                let env = json_object(payload.get("env"));
                resolve_root_from_inputs(None, Some(&env))?
            };
            Ok(json!({
                "contract": SERVER_CONTEXT_CONTRACT_VERSION,
                "root": path_text(&root.0),
                "root_source": root.1,
            }))
        }
        other => Err(format!("Unsupported server context operation `{other}`.")),
    }
}

pub fn ensure_context_directories(context: &ServerContextShape) -> Result<(), String> {
    for path in [
        context.root.as_path(),
        context.manifest_dir.as_path(),
        context.pack_dir.as_path(),
        context.tree_pack_dir.as_path(),
        context.ref_root.as_path(),
    ] {
        fs::create_dir_all(path).map_err(|exc| {
            format!(
                "Failed to create server runtime directory `{}`: {exc}",
                path_text(path)
            )
        })?;
    }
    Ok(())
}

fn maybe_ensure_context_directories(
    context: &ServerContextShape,
    ensure: bool,
) -> Result<(), String> {
    if ensure {
        ensure_context_directories(context)?;
    }
    Ok(())
}

pub fn create_server_context(
    root: &Path,
    backend: Option<&str>,
    postgres_dsn: Option<String>,
    content_schema: &str,
    control_schema: &str,
    root_source: &str,
) -> Result<ServerContextShape, String> {
    let root = absolutize_path(root);
    let backend = normalize_create_backend(backend)?;
    Ok(context_shape(
        root,
        backend,
        postgres_dsn,
        content_schema.to_string(),
        control_schema.to_string(),
        root_source.to_string(),
    ))
}

pub fn server_context_from_env_map(
    env: &JsonMap<String, JsonValue>,
) -> Result<ServerContextShape, String> {
    let (root, root_source) = resolve_root_from_inputs(None, Some(env))?;
    let backend = value_text(env.get(SERVER_DB_BACKEND_ENV))
        .map(|text| text.trim().to_lowercase())
        .unwrap_or_default();
    if backend.is_empty() {
        return Err(
            "AIT_NATIVE_SERVER_DB_BACKEND is required for server runtime startup; set it explicitly to 'postgres'."
                .to_string(),
        );
    }
    if backend != "postgres" {
        return Err(format!(
            "Unsupported AIT_NATIVE_SERVER_DB_BACKEND value: '{backend}'"
        ));
    }
    let postgres_dsn = normalize_optional_text(value_text(env.get(SERVER_POSTGRES_DSN_ENV)));
    if postgres_dsn.is_none() {
        return Err(
            "AIT_NATIVE_SERVER_POSTGRES_DSN is required when AIT_NATIVE_SERVER_DB_BACKEND=postgres."
                .to_string(),
        );
    }
    let content_schema = value_text(env.get(SERVER_POSTGRES_CONTENT_SCHEMA_ENV))
        .unwrap_or_else(|| DEFAULT_CONTENT_SCHEMA.to_string());
    let control_schema = value_text(env.get(SERVER_POSTGRES_CONTROL_SCHEMA_ENV))
        .unwrap_or_else(|| DEFAULT_CONTROL_SCHEMA.to_string());
    Ok(context_shape(
        root,
        backend,
        postgres_dsn,
        content_schema,
        control_schema,
        root_source,
    ))
}

pub fn resolve_root_from_inputs(
    explicit_root: Option<&str>,
    env: Option<&JsonMap<String, JsonValue>>,
) -> Result<(PathBuf, String), String> {
    if let Some(root) = explicit_root {
        return Ok((absolutize_path(Path::new(root)), "explicit".to_string()));
    }
    let server_data =
        env.and_then(|env| normalize_optional_text(value_text(env.get(SERVER_DATA_ENV))));
    if let Some(root) = server_data {
        return Ok((absolutize_path(Path::new(&root)), "env".to_string()));
    }
    // Fall back to the process environment only for non-test callers that want
    // the same resolver used by the live Rust server.
    if env.is_none() {
        return resolve_server_runtime_root_with_source(None)
            .map(|(root, source)| (root, source.to_string()));
    }
    Err(
        "AIT_NATIVE_SERVER_DATA is required for server runtime access; platform default runtime roots are no longer supported."
            .to_string(),
    )
}

fn context_shape(
    root: PathBuf,
    db_backend: String,
    postgres_dsn: Option<String>,
    content_schema: String,
    control_schema: String,
    root_source: String,
) -> ServerContextShape {
    ServerContextShape {
        manifest_dir: root.join("objects").join("manifests"),
        pack_dir: root.join("objects").join("packs"),
        tree_pack_dir: root.join("objects").join("tree-packs"),
        ref_root: root.join("refs"),
        root,
        db_backend,
        postgres_dsn,
        content_schema,
        control_schema,
        root_source,
    }
}

fn normalize_create_backend(value: Option<&str>) -> Result<String, String> {
    let backend = value.unwrap_or("postgres").trim().to_lowercase();
    let backend = if backend.is_empty() {
        "postgres".to_string()
    } else {
        backend
    };
    if backend != "postgres" {
        return Err(format!(
            "Unsupported AIT native server database backend: '{backend}'"
        ));
    }
    Ok(backend)
}

fn absolutize_path(path: &Path) -> PathBuf {
    let expanded = expand_user(path);
    if expanded.is_absolute() {
        expanded.components().collect::<PathBuf>()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(expanded)
            .components()
            .collect::<PathBuf>()
    }
}

fn expand_user(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
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

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
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

fn truthy(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(value)) => *value,
        Some(JsonValue::Number(number)) => {
            number.as_f64().map(|value| value != 0.0).unwrap_or(true)
        }
        Some(JsonValue::String(text)) => {
            let normalized = text.trim().to_ascii_lowercase();
            !normalized.is_empty() && !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
        }
        Some(JsonValue::Array(values)) => !values.is_empty(),
        Some(JsonValue::Object(values)) => !values.is_empty(),
    }
}
