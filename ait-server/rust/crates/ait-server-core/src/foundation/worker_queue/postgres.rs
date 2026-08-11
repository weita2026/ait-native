use super::rows::{
    compact_worker_queue_completion_row, postgres_int4, postgres_job_row_to_json,
    postgres_timestamptz,
};
use super::*;
use ::postgres::types::Json as PostgresJson;

#[derive(Clone)]
pub struct PostgresWorkerQueuePool {
    registry: Arc<PostgresConnectionPoolRegistry<NativePostgresDriver>>,
    backend: String,
    dsn: Option<String>,
    content_schema: String,
    control_schema: String,
    timeouts: PostgresTimeoutScope,
}

pub struct PostgresWorkerQueueConnection {
    conn: PostgresDbConnection<NativePostgresDriver>,
}

impl PostgresWorkerQueuePool {
    pub fn new(
        registry: Arc<PostgresConnectionPoolRegistry<NativePostgresDriver>>,
        backend: impl Into<String>,
        dsn: Option<String>,
        content_schema: impl Into<String>,
        control_schema: impl Into<String>,
        timeouts: PostgresTimeoutScope,
    ) -> Self {
        Self {
            registry,
            backend: backend.into(),
            dsn,
            content_schema: content_schema.into(),
            control_schema: control_schema.into(),
            timeouts,
        }
    }

    pub fn resolve_repo_id(&self, repo_name: &str) -> Result<Option<String>, String> {
        let mut conn = connect_server_plane(
            self.registry.as_ref(),
            &self.backend,
            self.dsn.as_deref(),
            &self.content_schema,
            &self.control_schema,
            "content",
            &self.timeouts,
        )?;
        let row = conn
            .raw_mut()
            .query_opt(
                "select repo_id from repositories where repo_name = $1 limit 1",
                &[&repo_name],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(row.and_then(|row| row.get::<_, Option<String>>("repo_id")))
    }
}

impl WorkerQueueConnectionPool for PostgresWorkerQueuePool {
    type Connection = PostgresWorkerQueueConnection;

    fn checkout(&self) -> Result<Self::Connection, String> {
        Ok(PostgresWorkerQueueConnection {
            conn: connect_server_plane(
                self.registry.as_ref(),
                &self.backend,
                self.dsn.as_deref(),
                &self.content_schema,
                &self.control_schema,
                "control",
                &self.timeouts,
            )?,
        })
    }
}

const JOB_ROW_COLUMNS: &str = "job_id, repo_name, repo_id, job_type, state, payload_json, result_json, attempt_count, max_attempts, available_at::text as available_at, locked_at::text as locked_at, locked_by, last_error, created_at::text as created_at, updated_at::text as updated_at";

const WORKER_QUEUE_INDEX_ROW_COLUMNS: &str = "job_id, repo_name, repo_id, job_type, state, '{}'::text as payload_json, '{}'::text as result_json, attempt_count, max_attempts, available_at::text as available_at, locked_at::text as locked_at, locked_by, left(last_error, 4096) as last_error, created_at::text as created_at, updated_at::text as updated_at";

const WORKER_QUEUE_READINESS_ROW_COLUMNS: &str = r#"
job_id,
repo_name,
repo_id,
job_type,
state,
jsonb_strip_nulls(jsonb_build_object(
    'patchset_id', left(payload_json::jsonb ->> 'patchset_id', 256),
    'trigger', left(payload_json::jsonb ->> 'trigger', 256),
    'execution_profile', left(payload_json::jsonb ->> 'execution_profile', 256),
    'suite_ids', case
        when jsonb_typeof(payload_json::jsonb -> 'suite_ids') = 'array'
        then jsonb_path_query_array(payload_json::jsonb, '$.suite_ids[0 to 63]')
        else '[]'::jsonb
    end
))::text as payload_json,
jsonb_strip_nulls(jsonb_build_object(
    'status', left(result_json::jsonb ->> 'status', 256),
    'tests_status', left(result_json::jsonb ->> 'tests_status', 256),
    'trigger', left(result_json::jsonb ->> 'trigger', 256),
    'selected_suite_ids', case
        when jsonb_typeof(result_json::jsonb -> 'selected_suite_ids') = 'array'
        then jsonb_path_query_array(result_json::jsonb, '$.selected_suite_ids[0 to 63]')
        else '[]'::jsonb
    end,
    'blocking_failures', case
        when jsonb_typeof(result_json::jsonb -> 'blocking_failures') = 'array'
        then jsonb_path_query_array(result_json::jsonb, '$.blocking_failures[0 to 63]')
        else '[]'::jsonb
    end,
    'suite_result_count', case
        when jsonb_typeof(result_json::jsonb -> 'suite_result_count') = 'number'
        then result_json::jsonb -> 'suite_result_count'
        when jsonb_typeof(result_json::jsonb -> 'suite_results') = 'array'
        then to_jsonb(jsonb_array_length(result_json::jsonb -> 'suite_results'))
        else null
    end,
    'blocking_failure_count', case
        when jsonb_typeof(result_json::jsonb -> 'blocking_failure_count') = 'number'
        then result_json::jsonb -> 'blocking_failure_count'
        when jsonb_typeof(result_json::jsonb -> 'blocking_failures') = 'array'
        then to_jsonb(jsonb_array_length(result_json::jsonb -> 'blocking_failures'))
        else null
    end
))::text as result_json,
attempt_count,
max_attempts,
available_at::text as available_at,
locked_at::text as locked_at,
locked_by,
left(last_error, 4096) as last_error,
created_at::text as created_at,
updated_at::text as updated_at
"#;

const WORKER_QUEUE_SUMMARY_ROW_COLUMNS: &str = r#"
job_id,
repo_name,
repo_id,
job_type,
state,
jsonb_strip_nulls(jsonb_build_object(
    'patchset_id', left(payload_json::jsonb ->> 'patchset_id', 256),
    'change_id', left(payload_json::jsonb ->> 'change_id', 256),
    'repo_name', left(payload_json::jsonb ->> 'repo_name', 256),
    'repo_id', left(payload_json::jsonb ->> 'repo_id', 256),
    'revision_snapshot_id', left(payload_json::jsonb ->> 'revision_snapshot_id', 256),
    'snapshot_id', left(payload_json::jsonb ->> 'snapshot_id', 256),
    'stage', left(payload_json::jsonb ->> 'stage', 256),
    'plane', left(payload_json::jsonb ->> 'plane', 256),
    'target_line', left(payload_json::jsonb ->> 'target_line', 256),
    'trigger', left(payload_json::jsonb ->> 'trigger', 256),
    'execution_profile', left(payload_json::jsonb ->> 'execution_profile', 256),
    'suite_id', left(payload_json::jsonb ->> 'suite_id', 256),
    'previous_snapshot_id', left(payload_json::jsonb ->> 'previous_snapshot_id', 256),
    'selector', left(payload_json::jsonb ->> 'selector', 256),
    'curated_corpus', left(payload_json::jsonb ->> 'curated_corpus', 256),
    'submission_id', left(payload_json::jsonb ->> 'submission_id', 256),
    'idempotency_key', left(payload_json::jsonb ->> 'idempotency_key', 256),
    'transport', left(payload_json::jsonb ->> 'transport', 256),
    'suite_ids', case
        when jsonb_typeof(payload_json::jsonb -> 'suite_ids') = 'array'
        then jsonb_path_query_array(payload_json::jsonb, '$.suite_ids[0 to 63]')
        else '[]'::jsonb
    end,
    'task_ids', case
        when jsonb_typeof(payload_json::jsonb -> 'task_ids') = 'array'
        then jsonb_path_query_array(payload_json::jsonb, '$.task_ids[0 to 63]')
        else '[]'::jsonb
    end,
    'dependency_evidence', case
        when jsonb_typeof(payload_json::jsonb -> 'dependency_evidence') = 'array'
        then jsonb_path_query_array(payload_json::jsonb, '$.dependency_evidence[0 to 63]')
        else '[]'::jsonb
    end,
    'compliance_evidence', case
        when jsonb_typeof(payload_json::jsonb -> 'compliance_evidence') = 'array'
        then jsonb_path_query_array(payload_json::jsonb, '$.compliance_evidence[0 to 63]')
        else '[]'::jsonb
    end,
    'change_seq', payload_json::jsonb -> 'change_seq',
    'patchset_number', payload_json::jsonb -> 'patchset_number',
    'land_seq', payload_json::jsonb -> 'land_seq',
    'count', payload_json::jsonb -> 'count',
    'window_days', payload_json::jsonb -> 'window_days',
    'max_members', payload_json::jsonb -> 'max_members',
    'prune_unreferenced', payload_json::jsonb -> 'prune_unreferenced',
    'prune_orphan_packs', payload_json::jsonb -> 'prune_orphan_packs',
    'repair', payload_json::jsonb -> 'repair',
    'repack', payload_json::jsonb -> 'repack'
))::text as payload_json,
jsonb_strip_nulls(jsonb_build_object(
    'contract', left(result_json::jsonb ->> 'contract', 256),
    'status', left(result_json::jsonb ->> 'status', 256),
    'patchset_id', left(result_json::jsonb ->> 'patchset_id', 256),
    'change_id', left(result_json::jsonb ->> 'change_id', 256),
    'repo_name', left(result_json::jsonb ->> 'repo_name', 256),
    'target_line', left(result_json::jsonb ->> 'target_line', 256),
    'trigger', left(result_json::jsonb ->> 'trigger', 256),
    'execution_profile', left(result_json::jsonb ->> 'execution_profile', 256),
    'tests_status', left(result_json::jsonb ->> 'tests_status', 256),
    'stage', left(result_json::jsonb ->> 'stage', 256),
    'plane', left(result_json::jsonb ->> 'plane', 256),
    'submission_id', left(result_json::jsonb ->> 'submission_id', 256),
    'task_id', left(result_json::jsonb ->> 'task_id', 256),
    'snapshot_id', left(result_json::jsonb ->> 'snapshot_id', 256),
    'revision_snapshot_id', left(result_json::jsonb ->> 'revision_snapshot_id', 256),
    'selected_suite_ids', case
        when jsonb_typeof(result_json::jsonb -> 'selected_suite_ids') = 'array'
        then jsonb_path_query_array(result_json::jsonb, '$.selected_suite_ids[0 to 63]')
        else '[]'::jsonb
    end,
    'all_patchset_suite_ids', case
        when jsonb_typeof(result_json::jsonb -> 'all_patchset_suite_ids') = 'array'
        then jsonb_path_query_array(result_json::jsonb, '$.all_patchset_suite_ids[0 to 63]')
        else '[]'::jsonb
    end,
    'blocking_suite_ids', case
        when jsonb_typeof(result_json::jsonb -> 'blocking_suite_ids') = 'array'
        then jsonb_path_query_array(result_json::jsonb, '$.blocking_suite_ids[0 to 63]')
        else '[]'::jsonb
    end,
    'blocking_failures', case
        when jsonb_typeof(result_json::jsonb -> 'blocking_failures') = 'array'
        then jsonb_path_query_array(result_json::jsonb, '$.blocking_failures[0 to 63]')
        else '[]'::jsonb
    end,
    'suite_failures', case
        when jsonb_typeof(result_json::jsonb -> 'suite_failures') = 'array'
        then jsonb_path_query_array(result_json::jsonb, '$.suite_failures[0 to 63]')
        else '[]'::jsonb
    end,
    'completed_suite_ids', case
        when jsonb_typeof(result_json::jsonb -> 'completed_suite_ids') = 'array'
        then jsonb_path_query_array(result_json::jsonb, '$.completed_suite_ids[0 to 63]')
        else '[]'::jsonb
    end,
    'suite_result_count', case
        when job_type not in ('patchset.ci', 'patchset.ci.aggregate', 'repo.ci') then null
        when jsonb_typeof(result_json::jsonb -> 'suite_results') = 'array'
        then jsonb_array_length(result_json::jsonb -> 'suite_results')
        else 0
    end,
    'blocking_failure_count', case
        when job_type not in ('patchset.ci', 'patchset.ci.aggregate', 'repo.ci') then null
        when jsonb_typeof(result_json::jsonb -> 'blocking_failures') = 'array'
        then jsonb_array_length(result_json::jsonb -> 'blocking_failures')
        else 0
    end,
    'admitted_cpu_tokens', result_json::jsonb -> 'admitted_cpu_tokens',
    'runner_parallelism', result_json::jsonb -> 'runner_parallelism',
    'count', result_json::jsonb -> 'count',
    'processed_count', result_json::jsonb -> 'processed_count',
    'failed_count', result_json::jsonb -> 'failed_count'
))::text as result_json,
attempt_count,
max_attempts,
available_at::text as available_at,
locked_at::text as locked_at,
locked_by,
left(last_error, 4096) as last_error,
created_at::text as created_at,
updated_at::text as updated_at
"#;

const STALE_PATCHSET_SUCCESSOR_SQL: &str = "select s.job_id from jobs s where s.job_id > $1 and (($3::text is not null and (s.repo_id = $3 or (s.repo_id is null and s.repo_name = $2))) or ($3::text is null and s.repo_name = $2)) and s.job_type = 'patchset.ci' and s.state = 'succeeded' and s.payload_json::jsonb - 'runtime_payload' = $4::jsonb and ((s.result_json::jsonb ->> 'tests_status') = 'pass' or ((s.result_json::jsonb ->> 'status') = 'skipped' and (s.result_json::jsonb ->> 'reason') = 'change_already_landed')) order by s.job_id asc limit 1";

fn active_duplicate_job_sql(repo_id_present: bool, semantic_patchset_payload: bool) -> String {
    match (repo_id_present, semantic_patchset_payload) {
        (true, true) => format!(
            "with advisory_lock as (select pg_advisory_xact_lock(hashtextextended(concat('ait.worker_queue.patchset_ci:', $1::text, ':', $3::text, ':', (($4::jsonb - 'runtime_payload')::text)), 0))) select {JOB_ROW_COLUMNS} from jobs cross join advisory_lock where (repo_id = $1 or (repo_id is null and repo_name = $2)) and job_type = $3 and payload_json::jsonb - 'runtime_payload' = $4::jsonb - 'runtime_payload' and state in ('queued', 'running') order by job_id desc limit 1"
        ),
        (true, false) => format!(
            "select {JOB_ROW_COLUMNS} from jobs where (repo_id = $1 or (repo_id is null and repo_name = $2)) and job_type = $3 and payload_json = $4 and state in ('queued', 'running') order by job_id desc limit 1"
        ),
        (false, true) => format!(
            "with advisory_lock as (select pg_advisory_xact_lock(hashtextextended(concat('ait.worker_queue.patchset_ci:', $1::text, ':', $2::text, ':', (($3::jsonb - 'runtime_payload')::text)), 0))) select {JOB_ROW_COLUMNS} from jobs cross join advisory_lock where repo_name = $1 and job_type = $2 and payload_json::jsonb - 'runtime_payload' = $3::jsonb - 'runtime_payload' and state in ('queued', 'running') order by job_id desc limit 1"
        ),
        (false, false) => format!(
            "select {JOB_ROW_COLUMNS} from jobs where repo_name = $1 and job_type = $2 and payload_json = $3 and state in ('queued', 'running') order by job_id desc limit 1"
        ),
    }
}

impl WorkerQueueConnection for PostgresWorkerQueueConnection {
    fn active_duplicate_job_row(
        &mut self,
        repo_name: &str,
        repo_id: Option<&str>,
        job_type: &str,
        payload_json: &str,
        patchset_dedupe_payload: Option<&JsonMap<String, JsonValue>>,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let semantic_patchset_payload =
            job_type == "patchset.ci" && patchset_dedupe_payload.is_some();
        let sql = active_duplicate_job_sql(repo_id.is_some(), semantic_patchset_payload);
        let patchset_dedupe_json = patchset_dedupe_payload.map(PostgresJson);
        let row = match (repo_id, patchset_dedupe_json.as_ref()) {
            (Some(repo_id), Some(payload)) if semantic_patchset_payload => self
                .conn
                .raw_mut()
                .query_opt(&sql, &[&repo_id, &repo_name, &job_type, payload]),
            (Some(repo_id), _) => self
                .conn
                .raw_mut()
                .query_opt(&sql, &[&repo_id, &repo_name, &job_type, &payload_json]),
            (None, Some(payload)) if semantic_patchset_payload => self
                .conn
                .raw_mut()
                .query_opt(&sql, &[&repo_name, &job_type, payload]),
            (None, _) => self
                .conn
                .raw_mut()
                .query_opt(&sql, &[&repo_name, &job_type, &payload_json]),
        }
        .map_err(|exc| exc.to_string())?;
        Ok(row.map(|row| postgres_job_row_to_json(&row)))
    }

    fn insert_job(
        &mut self,
        repo_name: &str,
        repo_id: Option<&str>,
        job_type: &str,
        payload_json: &str,
        _patchset_dedupe_payload: Option<&JsonMap<String, JsonValue>>,
        max_attempts: i64,
        available_at: &str,
        now: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let max_attempts = postgres_int4("max_attempts", max_attempts)?;
        let available_at = postgres_timestamptz("available_at", available_at)?;
        let now = postgres_timestamptz("now", now)?;
        let row = self
            .conn
            .raw_mut()
            .query_one(
                &format!(
                    "insert into jobs(repo_name, repo_id, job_type, state, payload_json, result_json, attempt_count, max_attempts, available_at, locked_at, locked_by, last_error, created_at, updated_at) values ($1, $2, $3, 'queued', $4, '{{}}', 0, $5::int4, $6::timestamptz, null, null, null, $7::timestamptz, $7::timestamptz) returning {JOB_ROW_COLUMNS}"
                ),
                &[&repo_name, &repo_id, &job_type, &payload_json, &max_attempts, &available_at, &now],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(postgres_job_row_to_json(&row))
    }

    fn queued_job_rows(
        &mut self,
        now: &str,
        repo_name: Option<&str>,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let now = postgres_timestamptz("now", now)?;
        let sql = if repo_name.is_some() {
            format!(
                "select {JOB_ROW_COLUMNS} from jobs where state = 'queued' and available_at <= $1::timestamptz and repo_name = $2 order by job_id asc"
            )
        } else {
            format!(
                "select {JOB_ROW_COLUMNS} from jobs where state = 'queued' and available_at <= $1::timestamptz order by job_id asc"
            )
        };
        let rows = if let Some(repo_name) = repo_name {
            self.conn.raw_mut().query(&sql, &[&now, &repo_name])
        } else {
            self.conn.raw_mut().query(&sql, &[&now])
        }
        .map_err(|exc| exc.to_string())?;
        Ok(rows.iter().map(postgres_job_row_to_json).collect())
    }

    fn running_job_rows(
        &mut self,
        repo_name: Option<&str>,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let sql = if repo_name.is_some() {
            format!(
                "select {JOB_ROW_COLUMNS} from jobs where state = 'running' and repo_name = $1 order by job_id asc"
            )
        } else {
            format!(
                "select {JOB_ROW_COLUMNS} from jobs where state = 'running' order by job_id asc"
            )
        };
        let rows = if let Some(repo_name) = repo_name {
            self.conn.raw_mut().query(&sql, &[&repo_name])
        } else {
            self.conn.raw_mut().query(&sql, &[])
        }
        .map_err(|exc| exc.to_string())?;
        Ok(rows.iter().map(postgres_job_row_to_json).collect())
    }

    fn job_row(&mut self, job_id: i64) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let row = self
            .conn
            .raw_mut()
            .query_opt(
                &format!("select {JOB_ROW_COLUMNS} from jobs where job_id = $1"),
                &[&job_id],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(row.map(|row| postgres_job_row_to_json(&row)))
    }

    fn list_job_rows(
        &mut self,
        repo_name: Option<&str>,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let rows = match (repo_name, state) {
            (Some(repo_name), Some(state)) => self.conn.raw_mut().query(
                &format!(
                    "select {JOB_ROW_COLUMNS} from jobs where repo_name = $1 and state = $2 order by job_id desc limit $3"
                ),
                &[&repo_name, &state, &limit],
            ),
            (Some(repo_name), None) => self.conn.raw_mut().query(
                &format!("select {JOB_ROW_COLUMNS} from jobs where repo_name = $1 order by job_id desc limit $2"),
                &[&repo_name, &limit],
            ),
            (None, Some(state)) => self.conn.raw_mut().query(
                &format!("select {JOB_ROW_COLUMNS} from jobs where state = $1 order by job_id desc limit $2"),
                &[&state, &limit],
            ),
            (None, None) => self.conn.raw_mut().query(
                &format!("select {JOB_ROW_COLUMNS} from jobs order by job_id desc limit $1"),
                &[&limit],
            ),
        }
        .map_err(|exc| exc.to_string())?;
        Ok(rows.iter().map(postgres_job_row_to_json).collect())
    }

    fn list_job_summary_rows(
        &mut self,
        repo_name: Option<&str>,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let rows = match (repo_name, state) {
            (Some(repo_name), Some(state)) => self.conn.raw_mut().query(
                &format!(
                    "select {WORKER_QUEUE_INDEX_ROW_COLUMNS} from jobs where repo_name = $1 and state = $2 order by job_id desc limit $3"
                ),
                &[&repo_name, &state, &limit],
            ),
            (Some(repo_name), None) => self.conn.raw_mut().query(
                &format!("select {WORKER_QUEUE_INDEX_ROW_COLUMNS} from jobs where repo_name = $1 order by job_id desc limit $2"),
                &[&repo_name, &limit],
            ),
            (None, Some(state)) => self.conn.raw_mut().query(
                &format!("select {WORKER_QUEUE_INDEX_ROW_COLUMNS} from jobs where state = $1 order by job_id desc limit $2"),
                &[&state, &limit],
            ),
            (None, None) => self.conn.raw_mut().query(
                &format!("select {WORKER_QUEUE_INDEX_ROW_COLUMNS} from jobs order by job_id desc limit $1"),
                &[&limit],
            ),
        }
        .map_err(|exc| exc.to_string())?;
        Ok(rows.iter().map(postgres_job_row_to_json).collect())
    }

    fn list_patchset_ci_job_rows(
        &mut self,
        repo_name: &str,
        patchset_id: &str,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let rows = if let Some(state) = state {
            self.conn.raw_mut().query(
                &format!(
                    "select {JOB_ROW_COLUMNS} from jobs where repo_name = $1 and job_type in ('patchset.ci', 'patchset.ci.aggregate') and payload_json::jsonb ->> 'patchset_id' = $2 and state = $3 order by job_id desc limit $4"
                ),
                &[&repo_name, &patchset_id, &state, &limit],
            )
        } else {
            self.conn.raw_mut().query(
                &format!(
                    "select {JOB_ROW_COLUMNS} from jobs where repo_name = $1 and job_type in ('patchset.ci', 'patchset.ci.aggregate') and payload_json::jsonb ->> 'patchset_id' = $2 order by job_id desc limit $3"
                ),
                &[&repo_name, &patchset_id, &limit],
            )
        }
        .map_err(|exc| exc.to_string())?;
        Ok(rows.iter().map(postgres_job_row_to_json).collect())
    }

    fn list_patchset_ci_status_job_rows(
        &mut self,
        repo_name: &str,
        patchset_id: &str,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let rows = if let Some(state) = state {
            self.conn.raw_mut().query(
                &format!(
                    "select {WORKER_QUEUE_SUMMARY_ROW_COLUMNS} from jobs where repo_name = $1 and job_type in ('patchset.ci', 'patchset.ci.aggregate') and payload_json::jsonb ->> 'patchset_id' = $2 and state = $3 order by job_id desc limit $4"
                ),
                &[&repo_name, &patchset_id, &state, &limit],
            )
        } else {
            self.conn.raw_mut().query(
                &format!(
                    "select {WORKER_QUEUE_SUMMARY_ROW_COLUMNS} from jobs where repo_name = $1 and job_type in ('patchset.ci', 'patchset.ci.aggregate') and payload_json::jsonb ->> 'patchset_id' = $2 order by job_id desc limit $3"
                ),
                &[&repo_name, &patchset_id, &limit],
            )
        }
        .map_err(|exc| exc.to_string())?;
        Ok(rows.iter().map(postgres_job_row_to_json).collect())
    }

    fn list_patchset_ci_readiness_job_rows(
        &mut self,
        repo_name: &str,
        patchset_id: &str,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let rows = if let Some(state) = state {
            self.conn.raw_mut().query(
                &format!(
                    "select {WORKER_QUEUE_READINESS_ROW_COLUMNS} from jobs where repo_name = $1 and job_type in ('patchset.ci', 'patchset.ci.aggregate') and payload_json::jsonb ->> 'patchset_id' = $2 and state = $3 order by job_id desc limit $4"
                ),
                &[&repo_name, &patchset_id, &state, &limit],
            )
        } else {
            self.conn.raw_mut().query(
                &format!(
                    "select {WORKER_QUEUE_READINESS_ROW_COLUMNS} from jobs where repo_name = $1 and job_type in ('patchset.ci', 'patchset.ci.aggregate') and payload_json::jsonb ->> 'patchset_id' = $2 order by job_id desc limit $3"
                ),
                &[&repo_name, &patchset_id, &limit],
            )
        }
        .map_err(|exc| exc.to_string())?;
        Ok(rows.iter().map(postgres_job_row_to_json).collect())
    }

    fn mark_running(
        &mut self,
        job_id: i64,
        worker_id: &str,
        now: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let now = postgres_timestamptz("now", now)?;
        let row = self
            .conn
            .raw_mut()
            .query_opt(
                &format!(
                    "update jobs set state = 'running', attempt_count = attempt_count + 1, locked_at = $2::timestamptz, locked_by = $3, updated_at = $2::timestamptz where job_id = $1 and state = 'queued' returning {JOB_ROW_COLUMNS}"
                ),
                &[&job_id, &now, &worker_id],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(row.map(|row| postgres_job_row_to_json(&row)))
    }

    fn renew_lease(
        &mut self,
        job_id: i64,
        worker_id: &str,
        now: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let now = postgres_timestamptz("now", now)?;
        let row = self
            .conn
            .raw_mut()
            .query_opt(
                &format!(
                    "update jobs set locked_at = $3::timestamptz, updated_at = $3::timestamptz where job_id = $1 and state = 'running' and locked_by = $2 returning {JOB_ROW_COLUMNS}"
                ),
                &[&job_id, &worker_id, &now],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(row.map(|row| postgres_job_row_to_json(&row)))
    }

    fn mark_attached(
        &mut self,
        job_id: i64,
        active_job_id: &str,
        singleflight_key: &str,
        now: &str,
    ) -> Result<bool, String> {
        let result_json = json!({
            "status": "attached",
            "scheduler": {
                "decision": "attach",
                "active_job_id": active_job_id,
                "singleflight_key": singleflight_key,
            }
        })
        .to_string();
        let now = postgres_timestamptz("now", now)?;
        let updated = self
            .conn
            .raw_mut()
            .execute(
                "update jobs set state = 'succeeded', result_json = $2, locked_at = null, locked_by = null, updated_at = $3::timestamptz where job_id = $1 and state = 'queued'",
                &[&job_id, &result_json, &now],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(updated == 1)
    }

    fn mark_succeeded(
        &mut self,
        job_id: i64,
        result: &JsonValue,
        now: &str,
        required_worker_id: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let result_json = result.to_string();
        let now = postgres_timestamptz("now", now)?;
        let required_worker_id = required_worker_id.map(str::to_string);
        let row = self
            .conn
            .raw_mut()
            .query_opt(
                &format!(
                    "update jobs set state = 'succeeded', result_json = $2, locked_at = null, locked_by = null, last_error = null, updated_at = $3::timestamptz where job_id = $1 and ($4::text is null or (state = 'running' and locked_by = $4)) returning {WORKER_QUEUE_INDEX_ROW_COLUMNS}"
                ),
                &[&job_id, &result_json, &now, &required_worker_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| {
                required_worker_id.as_deref().map_or_else(
                    || format!("Unknown job: {job_id}"),
                    |worker_id| {
                        format!(
                            "Cannot finish job {job_id}: running lease is not owned by `{worker_id}`."
                        )
                    },
                )
            })?;
        compact_worker_queue_completion_row(&postgres_job_row_to_json(&row), result)
    }

    fn mark_failed_or_retry(
        &mut self,
        job_id: i64,
        error: &str,
        retryable: bool,
        retry_available_at: Option<&str>,
        now: &str,
        required_worker_id: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let required_worker_id = required_worker_id.map(str::to_string);
        let row = self
            .conn
            .raw_mut()
            .query_opt(
                "select attempt_count, max_attempts from jobs where job_id = $1 and ($2::text is null or (state = 'running' and locked_by = $2))",
                &[&job_id, &required_worker_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| {
                required_worker_id.as_deref().map_or_else(
                    || format!("Unknown job: {job_id}"),
                    |worker_id| {
                        format!(
                            "Cannot finish job {job_id}: running lease is not owned by `{worker_id}`."
                        )
                    },
                )
            })?;
        let attempt_count: i32 = row.get("attempt_count");
        let max_attempts: i32 = row.get("max_attempts");
        let state = if retryable && attempt_count < max_attempts {
            "queued"
        } else {
            "failed"
        };
        let available_at = if state == "queued" {
            retry_available_at.unwrap_or(now)
        } else {
            now
        };
        let available_at = postgres_timestamptz("available_at", available_at)?;
        let now = postgres_timestamptz("now", now)?;
        let row = self
            .conn
            .raw_mut()
            .query_opt(
                &format!(
                    "update jobs set state = $2, available_at = $3::timestamptz, locked_at = null, locked_by = null, last_error = $4, updated_at = $5::timestamptz where job_id = $1 and ($6::text is null or (state = 'running' and locked_by = $6)) returning {JOB_ROW_COLUMNS}"
                ),
                &[
                    &job_id,
                    &state,
                    &available_at,
                    &error,
                    &now,
                    &required_worker_id,
                ],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| {
                required_worker_id.as_deref().map_or_else(
                    || format!("Unknown job: {job_id}"),
                    |worker_id| {
                        format!(
                            "Cannot finish job {job_id}: running lease is not owned by `{worker_id}`."
                        )
                    },
                )
            })?;
        Ok(postgres_job_row_to_json(&row))
    }

    fn reconcile_superseded_patchset_ci(
        &mut self,
        repo_name: Option<&str>,
        patchset_id: Option<&str>,
        now: &str,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let repo_name = repo_name.map(str::to_string);
        let patchset_id = patchset_id.map(str::to_string);
        let now = postgres_timestamptz("now", now)?;
        let rows = self
            .conn
            .raw_mut()
            .query(
                "select q.job_id, lateral_success.successor_job_id from jobs q cross join lateral (select min(s.job_id) as successor_job_id from jobs s where s.job_id > q.job_id and (s.repo_id = q.repo_id or ((s.repo_id is null or q.repo_id is null) and s.repo_name = q.repo_name)) and s.job_type = 'patchset.ci' and s.state = 'succeeded' and s.payload_json::jsonb - 'runtime_payload' = q.payload_json::jsonb - 'runtime_payload' and ((s.result_json::jsonb ->> 'tests_status') = 'pass' or ((s.result_json::jsonb ->> 'status') = 'skipped' and (s.result_json::jsonb ->> 'reason') = 'change_already_landed'))) lateral_success where q.state = 'queued' and q.job_type = 'patchset.ci' and nullif(q.payload_json::jsonb ->> 'patchset_id', '') is not null and lateral_success.successor_job_id is not null and ($1::text is null or q.repo_name = $1) and ($2::text is null or q.payload_json::jsonb ->> 'patchset_id' = $2) order by q.job_id asc",
                &[&repo_name, &patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        let mut reconciled = Vec::new();
        for candidate in rows {
            let job_id: i64 = candidate.get("job_id");
            let successor_job_id: i64 = candidate.get("successor_job_id");
            let result_json = superseded_patchset_ci_result(job_id, successor_job_id).to_string();
            let row = self
                .conn
                .raw_mut()
                .query_opt(
                    &format!(
                        "update jobs set state = 'succeeded', result_json = $2, available_at = $3::timestamptz, locked_at = null, locked_by = null, last_error = null, updated_at = $3::timestamptz where job_id = $1 and state = 'queued' returning {JOB_ROW_COLUMNS}"
                    ),
                    &[&job_id, &result_json, &now],
                )
                .map_err(|exc| exc.to_string())?;
            if let Some(row) = row {
                reconciled.push(postgres_job_row_to_json(&row));
            }
        }
        Ok(reconciled)
    }

    fn reclaim_stale(
        &mut self,
        stale_cutoff: &str,
        now: &str,
        repo_name: Option<&str>,
    ) -> Result<WorkerQueueReclaimSummary, String> {
        let stale_cutoff = postgres_timestamptz("stale_cutoff", stale_cutoff)?;
        let now = postgres_timestamptz("now", now)?;
        let sql = if repo_name.is_some() {
            "select job_id, repo_name, repo_id, job_type, payload_json, attempt_count, max_attempts from jobs where state = 'running' and locked_at is not null and locked_at <= $1::timestamptz and repo_name = $2 order by job_id asc"
        } else {
            "select job_id, repo_name, repo_id, job_type, payload_json, attempt_count, max_attempts from jobs where state = 'running' and locked_at is not null and locked_at <= $1::timestamptz order by job_id asc"
        };
        let rows = if let Some(repo_name) = repo_name {
            self.conn.raw_mut().query(sql, &[&stale_cutoff, &repo_name])
        } else {
            self.conn.raw_mut().query(sql, &[&stale_cutoff])
        }
        .map_err(|exc| exc.to_string())?;
        let mut requeued_job_ids = Vec::new();
        let mut failed_job_ids = Vec::new();
        let mut superseded_job_ids = Vec::new();
        for row in rows {
            let job_id: i64 = row.get("job_id");
            let row_repo_name: String = row.get("repo_name");
            let row_repo_id: Option<String> = row.get("repo_id");
            let job_type: String = row.get("job_type");
            let payload_json: String = row.get("payload_json");
            let attempt_count: i32 = row.get("attempt_count");
            let max_attempts: i32 = row.get("max_attempts");
            let successor_job_id = if job_type == "patchset.ci" {
                encoded_patchset_semantic_payload(&payload_json)
                    .map(|semantic_payload| {
                        self.conn
                            .raw_mut()
                            .query_opt(
                                STALE_PATCHSET_SUCCESSOR_SQL,
                                &[&job_id, &row_repo_name, &row_repo_id, &semantic_payload],
                            )
                            .map_err(|exc| exc.to_string())
                            .map(|row| row.map(|row| row.get::<_, i64>("job_id")))
                    })
                    .transpose()?
                    .flatten()
            } else {
                None
            };
            if let Some(successor_job_id) = successor_job_id {
                let result_json =
                    superseded_patchset_ci_result(job_id, successor_job_id).to_string();
                self.conn
                    .raw_mut()
                    .execute(
                        "update jobs set state = 'succeeded', result_json = $2, available_at = $3::timestamptz, locked_at = null, locked_by = null, last_error = null, updated_at = $3::timestamptz where job_id = $1 and state = 'running'",
                        &[&job_id, &result_json, &now],
                    )
                    .map_err(|exc| exc.to_string())?;
                superseded_job_ids.push(job_id);
                continue;
            }
            let (state, message) = if attempt_count >= max_attempts {
                failed_job_ids.push(job_id);
                ("failed", "Worker lease expired after max attempts")
            } else {
                requeued_job_ids.push(job_id);
                ("queued", "Worker lease expired; job returned to queue")
            };
            self.conn
                .raw_mut()
                .execute(
                    "update jobs set state = $2, available_at = $3::timestamptz, locked_at = null, locked_by = null, last_error = $4, updated_at = $3::timestamptz where job_id = $1",
                    &[&job_id, &state, &now, &message],
                )
                .map_err(|exc| exc.to_string())?;
        }
        Ok(WorkerQueueReclaimSummary {
            stale_count: requeued_job_ids.len() + failed_job_ids.len() + superseded_job_ids.len(),
            requeued_job_ids,
            failed_job_ids,
            superseded_job_ids,
            reconciled_queued_job_ids: Vec::new(),
        })
    }

    fn commit(&mut self) -> Result<(), String> {
        self.conn.commit()
    }
}

fn encoded_patchset_semantic_payload(
    encoded: &str,
) -> Option<PostgresJson<JsonMap<String, JsonValue>>> {
    let parsed = serde_json::from_str::<JsonValue>(encoded).ok()?;
    let payload = parsed.as_object()?;
    payload
        .get("patchset_id")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(PostgresJson(
        payload
            .iter()
            .filter(|(key, _)| key.as_str() != "runtime_payload")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
}

fn superseded_patchset_ci_result(job_id: i64, successor_job_id: i64) -> JsonValue {
    json!({
        "contract": "ait.server.worker.superseded_patchset_ci.v1",
        "status": "skipped",
        "tests_status": "pass",
        "reason": "equivalent_later_patchset_ci_succeeded",
        "superseded_job_id": job_id,
        "successor_job_id": successor_job_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::postgres::types::{ToSql, Type};
    use ::postgres::{Client, NoTls};
    use bytes::BytesMut;

    fn patchset_payload() -> JsonMap<String, JsonValue> {
        JsonMap::from_iter([
            ("patchset_id".to_string(), json!("RSET-0522/C-01/P-01")),
            ("repo_name".to_string(), json!("ait-server")),
            (
                "runtime_payload".to_string(),
                json!({"workspace_path": "/tmp/runtime"}),
            ),
        ])
    }

    #[test]
    fn patchset_dedupe_and_stale_payloads_serialize_as_postgres_jsonb() {
        let payload = patchset_payload();
        let raw_payload = JsonValue::Object(payload.clone()).to_string();
        let mut raw_output = BytesMut::new();
        assert!(raw_payload
            .to_sql_checked(&Type::JSONB, &mut raw_output)
            .is_err());

        let mut dedupe_output = BytesMut::new();
        PostgresJson(&payload)
            .to_sql_checked(&Type::JSONB, &mut dedupe_output)
            .expect("typed patchset dedupe payload should serialize as jsonb");

        let semantic_payload = encoded_patchset_semantic_payload(&raw_payload)
            .expect("patchset payload should produce a semantic PostgreSQL value");
        assert!(!semantic_payload.0.contains_key("runtime_payload"));
        let mut stale_output = BytesMut::new();
        semantic_payload
            .to_sql_checked(&Type::JSONB, &mut stale_output)
            .expect("typed stale-job semantic payload should serialize as jsonb");
    }

    #[test]
    fn readiness_sql_projects_only_bounded_gate_evidence() {
        assert!(WORKER_QUEUE_READINESS_ROW_COLUMNS.contains("'tests_status'"));
        assert!(WORKER_QUEUE_READINESS_ROW_COLUMNS.contains("'selected_suite_ids'"));
        assert!(WORKER_QUEUE_READINESS_ROW_COLUMNS.contains("'blocking_failures'"));
        assert!(WORKER_QUEUE_READINESS_ROW_COLUMNS.contains("'suite_result_count'"));
        assert!(WORKER_QUEUE_READINESS_ROW_COLUMNS.contains("'blocking_failure_count'"));
        assert!(!WORKER_QUEUE_READINESS_ROW_COLUMNS.contains("runtime_payload"));
        assert!(!WORKER_QUEUE_READINESS_ROW_COLUMNS.contains("attestation_update"));
        assert!(!WORKER_QUEUE_READINESS_ROW_COLUMNS.contains("materialization"));
    }

    #[test]
    fn live_postgres_accepts_worker_queue_jsonb_bind_contracts_when_configured() {
        let Some(dsn) = std::env::var("AIT_WORKER_QUEUE_TEST_POSTGRES_DSN")
            .ok()
            .or_else(|| std::env::var("AIT_NATIVE_SERVER_POSTGRES_DSN").ok())
        else {
            eprintln!(
                "skipping live PostgreSQL bind contract; no worker-queue test DSN is configured"
            );
            return;
        };
        let mut client = Client::connect(&dsn, NoTls).expect("test PostgreSQL should connect");
        client
            .batch_execute(
                "begin;
                 create temporary table jobs (
                     job_id bigint,
                     repo_name text,
                     repo_id text,
                     job_type text,
                     state text,
                     payload_json text,
                     result_json text,
                     attempt_count int4,
                     max_attempts int4,
                     available_at timestamptz,
                     locked_at timestamptz,
                     locked_by text,
                     last_error text,
                     created_at timestamptz,
                     updated_at timestamptz
                 ) on commit drop;
                 set local search_path = pg_temp;",
            )
            .expect("temporary worker queue table should be created");

        let repo_name = format!("ait-server-jsonb-bind-test-{}", std::process::id());
        let repo_id = format!("REPO-JSONB-BIND-{}", std::process::id());
        let job_type = "patchset.ci";
        let payload = patchset_payload();
        let dedupe_payload = PostgresJson(&payload);

        let scoped_sql = active_duplicate_job_sql(true, true);
        let scoped_statement = client
            .prepare(&scoped_sql)
            .expect("scoped semantic dedupe SQL should prepare");
        assert_eq!(scoped_statement.params()[3], Type::JSONB);
        client
            .query_opt(
                &scoped_statement,
                &[&repo_id, &repo_name, &job_type, &dedupe_payload],
            )
            .expect("scoped semantic dedupe payload should bind as jsonb");

        let unscoped_sql = active_duplicate_job_sql(false, true);
        let unscoped_statement = client
            .prepare(&unscoped_sql)
            .expect("unscoped semantic dedupe SQL should prepare");
        assert_eq!(unscoped_statement.params()[2], Type::JSONB);
        client
            .query_opt(
                &unscoped_statement,
                &[&repo_name, &job_type, &dedupe_payload],
            )
            .expect("unscoped semantic dedupe payload should bind as jsonb");

        let raw_payload = JsonValue::Object(payload).to_string();
        let semantic_payload = encoded_patchset_semantic_payload(&raw_payload)
            .expect("patchset payload should produce a semantic PostgreSQL value");
        let stale_statement = client
            .prepare(STALE_PATCHSET_SUCCESSOR_SQL)
            .expect("stale successor SQL should prepare");
        assert_eq!(stale_statement.params()[3], Type::JSONB);
        let job_id = i64::MAX - i64::from(std::process::id());
        let row_repo_id = Some(repo_id);
        client
            .query_opt(
                &stale_statement,
                &[&job_id, &repo_name, &row_repo_id, &semantic_payload],
            )
            .expect("stale successor semantic payload should bind as jsonb");

        client
            .batch_execute("rollback")
            .expect("temporary worker queue transaction should roll back");
    }
}
