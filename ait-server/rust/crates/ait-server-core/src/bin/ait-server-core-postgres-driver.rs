use ait_server_core::foundation::db::{
    configure_postgres_checkout_sql, ensure_postgres_schema_name, PostgresTimeoutScope,
};
use ait_server_core::foundation::postgres_json_codec::{
    execute_json_statement, executemany_json_statement,
};
use postgres::{Client, NoTls};
use serde_json::{json, Map, Value as JsonValue};
use std::io::{self, BufRead, Write};

const DRIVER_CONTRACT: &str = "ait.server.postgres.driver.v1";
const FAKE_POSTGRES_PREFIX: &str = "fake-postgres://";

struct Driver {
    client: Option<Client>,
}

impl Driver {
    fn new() -> Self {
        Self { client: None }
    }

    fn handle(&mut self, request: JsonValue) -> Result<JsonValue, String> {
        let object = request
            .as_object()
            .ok_or_else(|| "driver request must be a JSON object".to_string())?;
        let command = required_text(object, "command")?;
        match command.as_str() {
            "connect" => self.connect(object),
            "configure" => self.configure(object),
            "execute" => self.execute(object),
            "executemany" => self.executemany(object),
            "commit" => self.commit(),
            "rollback" => self.rollback(),
            "close" => self.close(),
            "ping" => Ok(json!({"contract": DRIVER_CONTRACT, "ready": true})),
            _ => Err(format!("unsupported postgres driver command: {command}")),
        }
    }

    fn connect(&mut self, object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
        if self.client.is_some() {
            return Err("postgres driver is already connected".to_string());
        }
        let dsn = required_text(object, "dsn")?;
        let schema = required_text(object, "schema")?;
        ensure_postgres_schema_name(&schema)?;
        let ensure_schema = optional_bool(object, "ensure_schema").unwrap_or(true);
        let timeouts = PostgresTimeoutScope {
            lock_timeout_ms: optional_i64(object, "lock_timeout_ms")?,
            statement_timeout_ms: optional_i64(object, "statement_timeout_ms")?,
        };
        if dsn.starts_with(FAKE_POSTGRES_PREFIX) {
            return Err(
                "fake-postgres is no longer supported; ait-server requires PostgreSQL.".to_string(),
            );
        }
        let mut client = Client::connect(&dsn, NoTls).map_err(|exc| exc.to_string())?;
        configure_client(&mut client, &schema, ensure_schema, &timeouts)?;
        self.client = Some(client);
        Ok(json!({
            "contract": DRIVER_CONTRACT,
            "connected": true,
            "schema": schema,
        }))
    }

    fn configure(&mut self, object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
        let schema = required_text(object, "schema")?;
        ensure_postgres_schema_name(&schema)?;
        let ensure_schema = optional_bool(object, "ensure_schema").unwrap_or(false);
        let timeouts = PostgresTimeoutScope {
            lock_timeout_ms: optional_i64(object, "lock_timeout_ms")?,
            statement_timeout_ms: optional_i64(object, "statement_timeout_ms")?,
        };
        let client = self.client_mut()?;
        configure_client(client, &schema, ensure_schema, &timeouts)?;
        Ok(json!({"configured": true, "schema": schema}))
    }

    fn execute(&mut self, object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
        let sql = required_text(object, "sql")?;
        let client = self.client_mut()?;
        execute_json_statement(client, &sql, object.get("params"))
    }

    fn executemany(&mut self, object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
        let sql = required_text(object, "sql")?;
        object
            .get("params_seq")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| "params_seq must be a JSON array".to_string())?;
        let client = self.client_mut()?;
        executemany_json_statement(client, &sql, object.get("params_seq"))
    }

    fn commit(&mut self) -> Result<JsonValue, String> {
        let client = self.client_mut()?;
        client
            .batch_execute("commit")
            .map_err(|exc| exc.to_string())?;
        client
            .batch_execute("begin")
            .map_err(|exc| exc.to_string())?;
        Ok(json!({"committed": true}))
    }

    fn rollback(&mut self) -> Result<JsonValue, String> {
        let client = self.client_mut()?;
        client
            .batch_execute("rollback")
            .map_err(|exc| exc.to_string())?;
        client
            .batch_execute("begin")
            .map_err(|exc| exc.to_string())?;
        Ok(json!({"rolled_back": true}))
    }

    fn close(&mut self) -> Result<JsonValue, String> {
        if let Some(client) = self.client.take() {
            client.close().map_err(|exc| exc.to_string())?;
        }
        Ok(json!({"closed": true}))
    }

    fn client_mut(&mut self) -> Result<&mut Client, String> {
        self.client
            .as_mut()
            .ok_or_else(|| "postgres driver is not connected".to_string())
    }
}

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut driver = Driver::new();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(value) => value,
            Err(exc) => {
                write_response(&mut stdout, Err(exc.to_string()));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request = serde_json::from_str::<JsonValue>(&line)
            .map_err(|exc| format!("driver request must be valid JSON: {exc}"))
            .and_then(|value| driver.handle(value));
        let should_exit = matches!(
            request,
            Ok(ref payload) if payload.get("closed").and_then(JsonValue::as_bool) == Some(true)
        );
        write_response(&mut stdout, request);
        if should_exit {
            break;
        }
    }
}

fn write_response(stdout: &mut io::Stdout, result: Result<JsonValue, String>) {
    let response = match result {
        Ok(payload) => json!({"ok": true, "payload": payload}),
        Err(error) => json!({"ok": false, "error": error}),
    };
    let _ = writeln!(stdout, "{response}");
    let _ = stdout.flush();
}

fn configure_client(
    client: &mut Client,
    schema: &str,
    ensure_schema: bool,
    timeouts: &PostgresTimeoutScope,
) -> Result<(), String> {
    for statement in configure_postgres_checkout_sql(schema, ensure_schema, timeouts) {
        client
            .batch_execute(&statement)
            .map_err(|exc| exc.to_string())?;
    }
    Ok(())
}

fn required_text(object: &Map<String, JsonValue>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

fn optional_bool(object: &Map<String, JsonValue>, key: &str) -> Option<bool> {
    object.get(key).and_then(JsonValue::as_bool)
}

fn optional_i64(object: &Map<String, JsonValue>, key: &str) -> Result<Option<i64>, String> {
    match object.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .ok_or_else(|| format!("{key} must be an integer or null"))
            .map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_postgres_dsn_is_rejected() {
        let mut driver = Driver::new();
        let err = driver
            .handle(json!({
                "command": "connect",
                "dsn": "fake-postgres:///tmp/ait-server-core-fake-postgres",
                "schema": "ait_content",
                "ensure_schema": true,
                "lock_timeout_ms": null,
                "statement_timeout_ms": null
            }))
            .expect_err("fake postgres should not connect");
        assert!(err.contains("fake-postgres is no longer supported"));
    }
}
