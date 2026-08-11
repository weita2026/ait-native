use crate::types::{
    SourceColumn, SourceConstraint, SourceInventory, SourceJobRow, SourceRepositoryRow,
    SourceSnapshot, JOB_TABLE, REPOSITORY_TABLE, SOURCE_DATABASE,
};
use postgres::{Client, IsolationLevel, NoTls, Transaction};
use std::collections::BTreeMap;

const EXPECTED_REPOSITORY_COLUMNS: &[(&str, &str, bool)] = &[
    ("created_at", "timestamp with time zone", true),
    ("default_line", "text", true),
    ("id_namespace_prefix", "text", true),
    ("lifecycle_state", "text", true),
    ("policy_json", "text", true),
    ("repo_id", "text", true),
    ("repo_name", "text", true),
    ("updated_at", "timestamp with time zone", true),
];

const EXPECTED_JOB_COLUMNS: &[(&str, &str, bool)] = &[
    ("attempt_count", "integer", true),
    ("available_at", "timestamp with time zone", true),
    ("created_at", "timestamp with time zone", true),
    ("job_id", "bigint", true),
    ("job_type", "text", true),
    ("last_error", "text", false),
    ("locked_at", "timestamp with time zone", false),
    ("locked_by", "text", false),
    ("max_attempts", "integer", true),
    ("payload_json", "text", true),
    ("repo_id", "text", true),
    ("repo_name", "text", true),
    ("result_json", "text", true),
    ("state", "text", true),
    ("updated_at", "timestamp with time zone", true),
];

const SOURCE_COLUMN_INVENTORY_QUERY: &str = r#"
select n.nspname || '.' || c.relname as table_name,
       a.attname as column_name,
       pg_catalog.format_type(a.atttypid, a.atttypmod) as sql_type,
       a.attnotnull as not_null,
       a.attgenerated <> '' as generated
  from pg_catalog.pg_attribute a
  join pg_catalog.pg_class c on c.oid = a.attrelid
  join pg_catalog.pg_namespace n on n.oid = c.relnamespace
 where ((n.nspname = 'ait_native_content' and c.relname = 'repositories')
     or (n.nspname = 'ait_native_control' and c.relname = 'jobs'))
   and a.attnum > 0 and not a.attisdropped
 order by (n.nspname || '.' || c.relname) collate "C", a.attname collate "C"
"#;

const SOURCE_CONSTRAINT_INVENTORY_QUERY: &str = r#"
select n.nspname || '.' || c.relname as table_name,
       co.conname as constraint_name,
       co.contype::text as constraint_type,
       pg_catalog.pg_get_constraintdef(co.oid, true) as definition
  from pg_catalog.pg_constraint co
  join pg_catalog.pg_class c on c.oid = co.conrelid
  join pg_catalog.pg_namespace n on n.oid = c.relnamespace
 where (n.nspname = 'ait_native_content' and c.relname = 'repositories')
    or (n.nspname = 'ait_native_control' and c.relname = 'jobs')
 order by (n.nspname || '.' || c.relname) collate "C", co.conname collate "C"
"#;

const SOURCE_REPOSITORIES_QUERY: &str = r#"
select repo_id, repo_name, default_line, id_namespace_prefix, policy_json,
       created_at, updated_at, lifecycle_state
  from ait_native_content.repositories
"#;

const SOURCE_JOBS_QUERY: &str = r#"
select job_id, repo_name, repo_id, job_type, state, payload_json, result_json,
       attempt_count, max_attempts, available_at, locked_at, locked_by, last_error,
       created_at, updated_at
  from ait_native_control.jobs
"#;

pub fn read_source_snapshot(dsn: &str) -> Result<SourceSnapshot, String> {
    if dsn.is_empty() || dsn.trim() != dsn {
        return Err("PostgreSQL DSN must be non-empty without surrounding whitespace".to_string());
    }
    let mut client = Client::connect(dsn, NoTls)
        .map_err(|error| format!("PostgreSQL connection failed: {error}"))?;
    let database_name: String = client
        .query_one("select current_database()", &[])
        .map_err(|error| format!("failed to read PostgreSQL database identity: {error}"))?
        .get(0);
    if database_name != SOURCE_DATABASE {
        return Err(format!(
            "PostgreSQL conversion requires database {SOURCE_DATABASE:?}, got {database_name:?}"
        ));
    }

    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .start()
        .map_err(|error| format!("failed to open repeatable-read conversion snapshot: {error}"))?;
    transaction
        .batch_execute(
            "lock table ait_native_content.repositories in share mode;\
             lock table ait_native_control.jobs in share mode;",
        )
        .map_err(|error| format!("failed to quiesce the two admitted source tables: {error}"))?;

    let inventory_before = read_inventory(&mut transaction)?;
    validate_exact_columns(&inventory_before.columns)?;
    let repositories = read_repositories(&mut transaction)?;
    let jobs = read_jobs(&mut transaction)?;
    let inventory_after = read_inventory(&mut transaction)?;
    if inventory_before != inventory_after {
        return Err(
            "PostgreSQL source table inventory or row counts changed during conversion snapshot"
                .to_string(),
        );
    }
    if inventory_before.repository_count
        != u64::try_from(repositories.len())
            .map_err(|_| "Repository row count exceeds u64".to_string())?
        || inventory_before.job_count
            != u64::try_from(jobs.len()).map_err(|_| "Job row count exceeds u64".to_string())?
    {
        return Err("PostgreSQL source row counts disagree with read rows".to_string());
    }
    transaction
        .commit()
        .map_err(|error| format!("failed to close conversion snapshot: {error}"))?;

    Ok(SourceSnapshot {
        database_name,
        inventory_before,
        inventory_after,
        repositories,
        jobs,
    })
}

fn read_inventory(transaction: &mut Transaction<'_>) -> Result<SourceInventory, String> {
    let columns = transaction
        .query(SOURCE_COLUMN_INVENTORY_QUERY, &[])
        .map_err(|error| format!("failed to inventory admitted source columns: {error:?}"))?
        .into_iter()
        .map(|row| SourceColumn {
            table_name: row.get("table_name"),
            column_name: row.get("column_name"),
            sql_type: row.get("sql_type"),
            not_null: row.get("not_null"),
            generated: row.get("generated"),
        })
        .collect::<Vec<_>>();
    let constraints = transaction
        .query(SOURCE_CONSTRAINT_INVENTORY_QUERY, &[])
        .map_err(|error| format!("failed to inventory admitted source constraints: {error:?}"))?
        .into_iter()
        .map(|row| SourceConstraint {
            table_name: row.get("table_name"),
            constraint_name: row.get("constraint_name"),
            constraint_type: row.get("constraint_type"),
            definition: row.get("definition"),
        })
        .collect::<Vec<_>>();
    let repository_count = count_table(transaction, REPOSITORY_TABLE)?;
    let job_count = count_table(transaction, JOB_TABLE)?;
    Ok(SourceInventory {
        columns,
        constraints,
        repository_count,
        job_count,
    })
}

fn count_table(transaction: &mut Transaction<'_>, table: &str) -> Result<u64, String> {
    let query = match table {
        REPOSITORY_TABLE => "select count(*)::bigint from ait_native_content.repositories",
        JOB_TABLE => "select count(*)::bigint from ait_native_control.jobs",
        _ => return Err("converter attempted to count an unadmitted source table".to_string()),
    };
    let count: i64 = transaction
        .query_one(query, &[])
        .map_err(|error| format!("failed to count {table}: {error}"))?
        .get(0);
    u64::try_from(count).map_err(|_| format!("{table} returned a negative row count"))
}

fn validate_exact_columns(columns: &[SourceColumn]) -> Result<(), String> {
    let mut actual = BTreeMap::<String, Vec<(&str, &str, bool)>>::new();
    for column in columns {
        if column.generated {
            return Err(format!(
                "{} column {} is generated; generated columns are not admitted",
                column.table_name, column.column_name
            ));
        }
        actual.entry(column.table_name.clone()).or_default().push((
            column.column_name.as_str(),
            column.sql_type.as_str(),
            column.not_null,
        ));
    }
    for values in actual.values_mut() {
        values.sort_unstable();
    }
    let expected = BTreeMap::from([
        (
            REPOSITORY_TABLE.to_string(),
            EXPECTED_REPOSITORY_COLUMNS.to_vec(),
        ),
        (JOB_TABLE.to_string(), EXPECTED_JOB_COLUMNS.to_vec()),
    ]);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "PostgreSQL admitted source-column inventory mismatch: expected={expected:?}, actual={actual:?}"
        ))
    }
}

fn read_repositories(
    transaction: &mut Transaction<'_>,
) -> Result<Vec<SourceRepositoryRow>, String> {
    transaction
        .query(SOURCE_REPOSITORIES_QUERY, &[])
        .map_err(|error| format!("failed to read {REPOSITORY_TABLE}: {error}"))
        .map(|rows| {
            rows.into_iter()
                .map(|row| SourceRepositoryRow {
                    repo_id: row.get("repo_id"),
                    repo_name: row.get("repo_name"),
                    default_line: row.get("default_line"),
                    id_namespace_prefix: row.get("id_namespace_prefix"),
                    policy_json: row.get("policy_json"),
                    created_at: row.get("created_at"),
                    updated_at: row.get("updated_at"),
                    lifecycle_state: row.get("lifecycle_state"),
                })
                .collect()
        })
}

fn read_jobs(transaction: &mut Transaction<'_>) -> Result<Vec<SourceJobRow>, String> {
    transaction
        .query(SOURCE_JOBS_QUERY, &[])
        .map_err(|error| format!("failed to read {JOB_TABLE}: {error}"))
        .map(|rows| {
            rows.into_iter()
                .map(|row| SourceJobRow {
                    job_id: row.get("job_id"),
                    repo_name: row.get("repo_name"),
                    repo_id: row.get("repo_id"),
                    job_type: row.get("job_type"),
                    state: row.get("state"),
                    payload_json: row.get("payload_json"),
                    result_json: row.get("result_json"),
                    attempt_count: row.get("attempt_count"),
                    max_attempts: row.get("max_attempts"),
                    available_at: row.get("available_at"),
                    locked_at: row.get("locked_at"),
                    locked_by: row.get("locked_by"),
                    last_error: row.get("last_error"),
                    created_at: row.get("created_at"),
                    updated_at: row.get("updated_at"),
                })
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_column_inventory_is_closed() {
        assert_eq!(EXPECTED_REPOSITORY_COLUMNS.len(), 8);
        assert_eq!(EXPECTED_JOB_COLUMNS.len(), 15);
    }

    #[test]
    fn postgres_catalog_inventory_orders_by_source_expressions_not_select_aliases() {
        let columns = SOURCE_COLUMN_INVENTORY_QUERY
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let constraints = SOURCE_CONSTRAINT_INVENTORY_QUERY
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(!columns.contains("order by table_name collate"));
        assert!(!constraints.contains("order by constraint_name collate"));
        assert!(columns.contains("as generated from pg_catalog.pg_attribute"));
        assert!(columns.contains("a.attname collate"));
        assert!(constraints.contains("as definition from pg_catalog.pg_constraint"));
        assert!(constraints.contains("co.conname collate"));
        assert!(SOURCE_REPOSITORIES_QUERY.contains("\n  from ait_native_content.repositories"));
        assert!(SOURCE_JOBS_QUERY.contains("\n  from ait_native_control.jobs"));
    }
}
