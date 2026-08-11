use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use bytes::BytesMut;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use postgres::types::{to_sql_checked, IsNull, ToSql, Type};
use postgres::{Client, Row};
use serde_json::{json, Map, Value as JsonValue};
use std::error::Error;

#[derive(Debug)]
enum PgParamValue {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
}

impl ToSql for PgParamValue {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        match self {
            Self::Null => Ok(IsNull::Yes),
            Self::Bool(value) => value.to_sql(ty, out),
            Self::I64(value) => match *ty {
                Type::INT2 => i16::try_from(*value)?.to_sql(ty, out),
                Type::INT4 => i32::try_from(*value)?.to_sql(ty, out),
                Type::INT8 => value.to_sql(ty, out),
                Type::FLOAT4 => (*value as f32).to_sql(ty, out),
                Type::FLOAT8 => (*value as f64).to_sql(ty, out),
                Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
                    value.to_string().to_sql(ty, out)
                }
                _ => value.to_sql(ty, out),
            },
            Self::F64(value) => match *ty {
                Type::FLOAT4 => (*value as f32).to_sql(ty, out),
                Type::FLOAT8 => value.to_sql(ty, out),
                Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
                    value.to_string().to_sql(ty, out)
                }
                _ => value.to_sql(ty, out),
            },
            Self::String(value) => string_param_to_sql(value, ty, out),
            Self::Bytes(value) => value.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    to_sql_checked!();
}

pub fn execute_json_statement(
    client: &mut Client,
    sql: &str,
    params_value: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    let params = parse_params(params_value)?;
    let sql = postgres_query_sql(sql)?;
    let refs = param_refs(&params);
    if statement_returns_rows(&sql) {
        let rows = client.query(&sql, &refs).map_err(|exc| exc.to_string())?;
        rows_payload(rows)
    } else {
        let rowcount = client.execute(&sql, &refs).map_err(|exc| exc.to_string())?;
        Ok(json!({
            "rows": [],
            "rowcount": i64::try_from(rowcount).unwrap_or(i64::MAX),
        }))
    }
}

pub fn executemany_json_statement(
    client: &mut Client,
    sql: &str,
    params_seq_value: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    let sql = postgres_query_sql(sql)?;
    let batches = params_seq_value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "params_seq must be a JSON array".to_string())?;
    let mut total: i64 = 0;
    for batch in batches {
        let params = parse_params(Some(batch))?;
        let refs = param_refs(&params);
        let rowcount = client.execute(&sql, &refs).map_err(|exc| exc.to_string())?;
        total = total.saturating_add(i64::try_from(rowcount).unwrap_or(i64::MAX));
    }
    Ok(json!({"rows": [], "rowcount": total}))
}

pub fn postgres_query_sql(sql: &str) -> Result<String, String> {
    postgres_placeholder_sql(&normalize_postgres_sql(sql))
}

pub fn normalize_postgres_sql(sql: &str) -> String {
    let trimmed = sql.trim();
    let without_semicolon = trimmed
        .trim_end_matches(|ch: char| ch == ';' || ch.is_ascii_whitespace())
        .trim_end();
    let mut out = without_semicolon.to_string();
    let lower = out.to_ascii_lowercase();
    if lower.starts_with("insert or ignore into ") {
        let into_index = lower
            .find("into")
            .expect("insert-or-ignore normalization should keep `into`");
        out = format!("insert {}", &out[into_index..]);
        if !lower.contains(" on conflict ") {
            out.push_str(" on conflict do nothing");
        }
    }
    out
}

pub fn split_sql_script(script: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut previous = '\0';
    for ch in script.chars() {
        if ch == '\'' && !in_double && previous != '\\' {
            in_single = !in_single;
        } else if ch == '"' && !in_single && previous != '\\' {
            in_double = !in_double;
        }
        if ch == ';' && !in_single && !in_double {
            let statement = current.trim();
            if !statement.is_empty() {
                statements.push(statement.to_string());
            }
            current.clear();
        } else {
            current.push(ch);
        }
        previous = ch;
    }
    let tail = current.trim();
    if !tail.is_empty() {
        statements.push(tail.to_string());
    }
    statements
}

fn parse_params(value: Option<&JsonValue>) -> Result<Vec<PgParamValue>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| "params must be a JSON array".to_string())?;
    values.iter().map(parse_param).collect()
}

fn parse_param(value: &JsonValue) -> Result<PgParamValue, String> {
    match value {
        JsonValue::Null => Ok(PgParamValue::Null),
        JsonValue::Bool(value) => Ok(PgParamValue::Bool(*value)),
        JsonValue::Number(value) => {
            if let Some(int_value) = value.as_i64() {
                Ok(PgParamValue::I64(int_value))
            } else if let Some(float_value) = value.as_f64() {
                Ok(PgParamValue::F64(float_value))
            } else {
                Err(format!("unsupported JSON number parameter: {value}"))
            }
        }
        JsonValue::String(value) => Ok(PgParamValue::String(value.to_string())),
        JsonValue::Object(object) => {
            if object
                .get("__ait_pg_type")
                .and_then(JsonValue::as_str)
                .is_some_and(|value| value == "bytes")
            {
                let encoded = object
                    .get("base64")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| "byte parameter requires base64".to_string())?;
                let data = BASE64_STANDARD
                    .decode(encoded)
                    .map_err(|exc| format!("invalid byte parameter base64: {exc}"))?;
                return Ok(PgParamValue::Bytes(data));
            }
            Ok(PgParamValue::String(value.to_string()))
        }
        JsonValue::Array(_) => Ok(PgParamValue::String(value.to_string())),
    }
}

fn param_refs(params: &[PgParamValue]) -> Vec<&(dyn ToSql + Sync)> {
    params
        .iter()
        .map(|value| value as &(dyn ToSql + Sync))
        .collect()
}

fn string_param_to_sql(
    value: &str,
    ty: &Type,
    out: &mut BytesMut,
) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
    match *ty {
        Type::JSON | Type::JSONB => {
            let parsed = serde_json::from_str::<JsonValue>(value)?;
            parsed.to_sql(ty, out)
        }
        Type::TIMESTAMPTZ => {
            let parsed = DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc);
            parsed.to_sql(ty, out)
        }
        Type::TIMESTAMP => {
            let parsed = parse_naive_datetime(value)?;
            parsed.to_sql(ty, out)
        }
        Type::DATE => {
            let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")?;
            parsed.to_sql(ty, out)
        }
        Type::INT2 => value.parse::<i16>()?.to_sql(ty, out),
        Type::INT4 => value.parse::<i32>()?.to_sql(ty, out),
        Type::INT8 => value.parse::<i64>()?.to_sql(ty, out),
        Type::FLOAT4 => value.parse::<f32>()?.to_sql(ty, out),
        Type::FLOAT8 => value.parse::<f64>()?.to_sql(ty, out),
        _ => value.to_sql(ty, out),
    }
}

fn parse_naive_datetime(value: &str) -> Result<NaiveDateTime, chrono::ParseError> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f"))
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f"))
}

fn statement_returns_rows(sql: &str) -> bool {
    let normalized = sql.trim_start().to_ascii_lowercase();
    normalized.starts_with("select")
        || normalized.starts_with("with")
        || normalized.starts_with("show")
        || normalized.starts_with("explain")
        || normalized.contains(" returning ")
}

fn rows_payload(rows: Vec<Row>) -> Result<JsonValue, String> {
    let mut output_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut output = Map::new();
        for (idx, column) in row.columns().iter().enumerate() {
            output.insert(
                column.name().to_string(),
                pg_cell_to_json(row, idx, column.type_())?,
            );
        }
        output_rows.push(JsonValue::Object(output));
    }
    Ok(json!({
        "rows": output_rows,
        "rowcount": i64::try_from(rows.len()).unwrap_or(i64::MAX),
    }))
}

fn pg_cell_to_json(row: &Row, idx: usize, ty: &Type) -> Result<JsonValue, String> {
    if *ty == Type::VOID {
        return Ok(JsonValue::Null);
    }
    if *ty == Type::BOOL {
        return optional_cell::<bool>(row, idx).map(|value| optional_json(value, JsonValue::Bool));
    }
    if *ty == Type::INT2 {
        return optional_cell::<i16>(row, idx)
            .map(|value| optional_json(value, |item| json!(item)));
    }
    if *ty == Type::INT4 {
        return optional_cell::<i32>(row, idx)
            .map(|value| optional_json(value, |item| json!(item)));
    }
    if *ty == Type::INT8 {
        return optional_cell::<i64>(row, idx)
            .map(|value| optional_json(value, |item| json!(item)));
    }
    if *ty == Type::FLOAT4 {
        return optional_cell::<f32>(row, idx)
            .map(|value| optional_json(value, |item| json!(item)));
    }
    if *ty == Type::FLOAT8 {
        return optional_cell::<f64>(row, idx)
            .map(|value| optional_json(value, |item| json!(item)));
    }
    if matches!(
        *ty,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN
    ) {
        return optional_cell::<String>(row, idx)
            .map(|value| optional_json(value, JsonValue::String));
    }
    if *ty == Type::BYTEA {
        return optional_cell::<Vec<u8>>(row, idx).map(|value| {
            optional_json(value, |data| {
                json!({
                    "__ait_pg_type": "bytes",
                    "base64": BASE64_STANDARD.encode(data),
                })
            })
        });
    }
    if *ty == Type::JSON || *ty == Type::JSONB {
        return optional_cell::<JsonValue>(row, idx).map(|value| optional_json(value, |item| item));
    }
    if *ty == Type::TIMESTAMPTZ {
        return optional_cell::<DateTime<Utc>>(row, idx)
            .map(|value| optional_json(value, |item| JsonValue::String(item.to_rfc3339())));
    }
    if *ty == Type::TIMESTAMP {
        return optional_cell::<NaiveDateTime>(row, idx)
            .map(|value| optional_json(value, |item| JsonValue::String(item.to_string())));
    }
    if *ty == Type::DATE {
        return optional_cell::<NaiveDate>(row, idx)
            .map(|value| optional_json(value, |item| JsonValue::String(item.to_string())));
    }
    optional_cell::<String>(row, idx).map(|value| optional_json(value, JsonValue::String))
}

fn optional_cell<T>(row: &Row, idx: usize) -> Result<Option<T>, String>
where
    T: postgres::types::FromSqlOwned,
    Option<T>: postgres::types::FromSqlOwned,
{
    row.try_get::<usize, Option<T>>(idx)
        .map_err(|exc| exc.to_string())
}

fn optional_json<T, F>(value: Option<T>, mapper: F) -> JsonValue
where
    F: FnOnce(T) -> JsonValue,
{
    match value {
        Some(value) => mapper(value),
        None => JsonValue::Null,
    }
}

fn postgres_placeholder_sql(sql: &str) -> Result<String, String> {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut param_index = 1usize;
    let mut previous = '\0';
    while let Some(ch) = chars.next() {
        if ch == '\'' && !in_double && previous != '\\' {
            in_single = !in_single;
            out.push(ch);
        } else if ch == '"' && !in_single {
            in_double = !in_double;
            out.push(ch);
        } else if ch == '?' && !in_single && !in_double {
            out.push('$');
            out.push_str(&param_index.to_string());
            param_index += 1;
        } else if ch == '%' && !in_single && !in_double && chars.peek() == Some(&'s') {
            let _ = chars.next();
            out.push('$');
            out.push_str(&param_index.to_string());
            param_index += 1;
        } else {
            out.push(ch);
        }
        previous = ch;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_question_and_percent_placeholders_outside_literals() {
        assert_eq!(
            postgres_query_sql("select * from t where a = ? and b = '%s' and c = %s and d = \"?\"")
                .unwrap(),
            "select * from t where a = $1 and b = '%s' and c = $2 and d = \"?\""
        );
    }

    #[test]
    fn rewrites_insert_or_ignore_for_postgres() {
        assert_eq!(
            normalize_postgres_sql(
                "insert or ignore into blobs(blob_id, created_at) values (?, ?);"
            ),
            "insert into blobs(blob_id, created_at) values (?, ?) on conflict do nothing"
        );
    }

    #[test]
    fn splits_sql_script_outside_literals() {
        assert_eq!(
            split_sql_script("select 1; insert into t(v) values('a;b'); update t set v = \"x;y\";"),
            vec![
                "select 1".to_string(),
                "insert into t(v) values('a;b')".to_string(),
                "update t set v = \"x;y\"".to_string(),
            ]
        );
    }
}
