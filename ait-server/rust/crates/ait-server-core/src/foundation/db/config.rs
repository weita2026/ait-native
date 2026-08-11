use super::*;

pub const DEFAULT_POSTGRES_POOL_MAX_SIZE: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresTimeoutScope {
    pub lock_timeout_ms: Option<i64>,
    pub statement_timeout_ms: Option<i64>,
}

impl Default for PostgresTimeoutScope {
    fn default() -> Self {
        Self {
            lock_timeout_ms: None,
            statement_timeout_ms: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostgresServerPlane {
    Content,
    Control,
}

impl PostgresServerPlane {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "content" => Ok(Self::Content),
            "control" => Ok(Self::Control),
            _ => Err(format!("Unknown plane: {raw}")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Control => "control",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresServerRuntimeConfig {
    pub dsn: String,
    pub content_schema: String,
    pub control_schema: String,
}

impl PostgresServerRuntimeConfig {
    pub fn new(
        dsn: impl Into<String>,
        content_schema: impl Into<String>,
        control_schema: impl Into<String>,
    ) -> Result<Self, String> {
        let dsn = dsn.into();
        if dsn.trim().is_empty() {
            return Err(
                "PostgreSQL backend requested but AIT_NATIVE_SERVER_POSTGRES_DSN is not configured"
                    .to_string(),
            );
        }
        let content_schema = content_schema.into();
        let control_schema = control_schema.into();
        ensure_postgres_schema_name(&content_schema)?;
        ensure_postgres_schema_name(&control_schema)?;
        Ok(Self {
            dsn,
            content_schema,
            control_schema,
        })
    }

    pub fn schema_for(&self, plane: PostgresServerPlane) -> &str {
        match plane {
            PostgresServerPlane::Content => &self.content_schema,
            PostgresServerPlane::Control => &self.control_schema,
        }
    }
}

pub fn ensure_postgres_schema_name(schema: &str) -> Result<&str, String> {
    let mut chars = schema.chars();
    let Some(first) = chars.next() else {
        return Err(format!("Invalid schema name: {schema}"));
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return Err(format!("Invalid schema name: {schema}"));
    }
    if chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric()) {
        Ok(schema)
    } else {
        Err(format!("Invalid schema name: {schema}"))
    }
}

pub fn resolve_server_plane_runtime(
    backend: &str,
    dsn: Option<&str>,
    content_schema: &str,
    control_schema: &str,
    plane: &str,
) -> Result<(PostgresServerRuntimeConfig, PostgresServerPlane), String> {
    let plane = PostgresServerPlane::parse(plane)?;
    if backend != "postgres" {
        return Err(format!(
            "Unsupported AIT server database backend for {} plane: '{backend}'. Only PostgreSQL is supported for ait-server runtime state.",
            plane.as_str()
        ));
    }
    let config =
        PostgresServerRuntimeConfig::new(dsn.unwrap_or_default(), content_schema, control_schema)?;
    Ok((config, plane))
}

pub fn resolve_postgres_pool_max_size(raw: Option<&str>) -> usize {
    let normalized = raw.unwrap_or_default().trim();
    if normalized.is_empty() {
        return DEFAULT_POSTGRES_POOL_MAX_SIZE;
    }
    match normalized.parse::<i64>() {
        Ok(value) => value.max(1) as usize,
        _ => DEFAULT_POSTGRES_POOL_MAX_SIZE,
    }
}

pub fn postgres_timeout_sql(name: &str, timeout_ms: Option<i64>) -> String {
    match timeout_ms {
        Some(value) => format!("set {name} = '{}ms'", value.max(1)),
        None => format!("reset {name}"),
    }
}

pub fn configure_postgres_session_sql(
    schema: &str,
    ensure_schema: bool,
    timeouts: &PostgresTimeoutScope,
) -> Vec<String> {
    let mut statements = Vec::new();
    if ensure_schema {
        statements.push(format!("create schema if not exists \"{schema}\""));
    }
    statements.push(format!("set search_path to \"{schema}\", public"));
    statements.push(postgres_timeout_sql(
        "lock_timeout",
        timeouts.lock_timeout_ms,
    ));
    statements.push(postgres_timeout_sql(
        "statement_timeout",
        timeouts.statement_timeout_ms,
    ));
    statements
}

pub fn configure_postgres_checkout_sql(
    schema: &str,
    ensure_schema: bool,
    timeouts: &PostgresTimeoutScope,
) -> Vec<String> {
    let mut statements = configure_postgres_session_sql(schema, ensure_schema, timeouts);
    statements.push("begin".to_string());
    statements
}
