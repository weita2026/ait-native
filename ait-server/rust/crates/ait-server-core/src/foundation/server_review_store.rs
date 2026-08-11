use crate::foundation::db::ensure_postgres_schema_name;
use crate::foundation::server_context::{DEFAULT_CONTENT_SCHEMA, DEFAULT_CONTROL_SCHEMA};
use crate::foundation::workflow_artifacts::{
    effective_policy_status, is_structured_code_review_summary_text, review_summary_from_rows,
    CODE_REVIEW_SUMMARY_ACTION, TASK_REVIEW_APPROVE_ACTION,
};
use chrono::Utc;
use postgres::{Client, NoTls, Row};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

pub const SERVER_REVIEW_STORE_CONTRACT: &str = "ait.server.review_store.v1";

const REQUIRED_APPROVALS: i64 = 1;
const FAKE_POSTGRES_PREFIX: &str = "fake-postgres://";

pub fn server_review_store_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    if operation == "contract" {
        return Ok(json!({
            "contract": SERVER_REVIEW_STORE_CONTRACT,
            "backend": "postgres",
            "migration_status": "rust_owned_no_python_reference",
            "mutates_state": true,
            "operations": [
                "request-review",
                "record-review",
                "list-reviews",
            ],
        }));
    }
    let payload = request
        .as_object()
        .ok_or_else(|| "review-store payload must be a JSON object.".to_string())?;
    let runtime = ReviewStoreRuntime::from_payload(payload)?;
    let mut store = PostgresReviewStore::connect(runtime)?;
    match operation {
        "request-review" => {
            let change_id = required_text(payload.get("change_id"), "change_id")?;
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            let reviewer_groups = string_list(payload.get("reviewer_groups"), "reviewer_groups")?;
            let note = optional_text(payload.get("note"));
            Ok(json!({
                "contract": SERVER_REVIEW_STORE_CONTRACT,
                "request": store.request_review(&change_id, &patchset_id, &reviewer_groups, note.as_deref())?,
            }))
        }
        "record-review" => {
            let change_id = required_text(payload.get("change_id"), "change_id")?;
            let patchset_id = required_text(payload.get("patchset_id"), "patchset_id")?;
            let reviewer = required_text(payload.get("reviewer"), "reviewer")?;
            let action = required_text(payload.get("action"), "action")?;
            let comment = optional_text(payload.get("comment"));
            let blocking = payload.get("blocking").is_some_and(truthy);
            Ok(json!({
                "contract": SERVER_REVIEW_STORE_CONTRACT,
                "review": store.record_review(&change_id, &patchset_id, &reviewer, &action, comment.as_deref(), blocking)?,
            }))
        }
        "list-reviews" => {
            let change_id = required_text(payload.get("change_id"), "change_id")?;
            Ok(json!({
                "contract": SERVER_REVIEW_STORE_CONTRACT,
                "reviews": store.list_reviews(&change_id)?,
            }))
        }
        other => Err(format!("Unsupported review-store operation `{other}`.")),
    }
}

#[derive(Debug, Clone)]
struct ReviewStoreRuntime {
    dsn: String,
    content_schema: String,
    control_schema: String,
}

impl ReviewStoreRuntime {
    fn from_payload(payload: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let backend =
            optional_text(payload.get("backend")).unwrap_or_else(|| "postgres".to_string());
        if backend != "postgres" {
            return Err(format!(
                "Unsupported ait-server review-store backend `{backend}`. Only PostgreSQL is supported."
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
                "fake-postgres is not supported for ait-server review-store runtime.".to_string(),
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

struct PostgresReviewStore {
    client: Client,
    content_schema: String,
    control_schema: String,
}

impl PostgresReviewStore {
    fn connect(runtime: ReviewStoreRuntime) -> Result<Self, String> {
        let client = Client::connect(&runtime.dsn, NoTls).map_err(|exc| exc.to_string())?;
        Ok(Self {
            client,
            content_schema: runtime.content_schema,
            control_schema: runtime.control_schema,
        })
    }

    fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.transaction(|store| {
            let change = store.change_row(change_id)?;
            ensure_change_mutable(&change, "request reviews")?;
            store.patchset_for_change(patchset_id, change_id)?;
            let now = utc_now();
            let repo_name = required_text(change.get("repo_name"), "change.repo_name")?;
            let repo_id = optional_text(change.get("repo_id"))
                .filter(|value| !value.trim().is_empty())
                .map(Ok)
                .unwrap_or_else(|| store.repo_id_for_repo_name(&repo_name))?;
            let review_requests = store.control_table("review_requests");
            for group in reviewer_groups {
                store
                    .client
                    .execute(
                        &format!("insert into {review_requests}(repo_id, change_id, patchset_id, reviewer_group, note, created_at) values ($1, $2, $3, $4, $5, $6::text::timestamptz)"),
                        &[&repo_id, &change_id, &patchset_id, &group, &note, &now],
                    )
                    .map_err(|exc| exc.to_string())?;
            }
            store.record_event(
                "review.requested",
                "change",
                change_id,
                &json!({"patchset_id": patchset_id, "reviewer_groups": reviewer_groups}),
                &now,
            )?;
            store.refresh_change_state(change_id, &now)?;
            Ok(JsonMap::from_iter([
                ("change_id".to_string(), json!(change_id)),
                ("patchset_id".to_string(), json!(patchset_id)),
                ("requested_groups".to_string(), json!(reviewer_groups)),
                ("status".to_string(), json!("requested")),
            ]))
        })
    }

    fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.transaction(|store| {
            let change = store.change_row(change_id)?;
            ensure_change_mutable(&change, "record reviews")?;
            store.patchset_for_change(patchset_id, change_id)?;
            if action == CODE_REVIEW_SUMMARY_ACTION
                && !is_structured_code_review_summary_text(comment.map(JsonValue::from).as_ref())
            {
                return Err(code_review_summary_requirement_text(comment));
            }
            let now = utc_now();
            let repo_name = required_text(change.get("repo_name"), "change.repo_name")?;
            let repo_id = optional_text(change.get("repo_id"))
                .filter(|value| !value.trim().is_empty())
                .map(Ok)
                .unwrap_or_else(|| store.repo_id_for_repo_name(&repo_name))?;
            let reviews = store.control_table("reviews");
            let row = store
                .client
                .query_one(
                    &format!("insert into {reviews}(repo_id, change_id, patchset_id, reviewer, action, comment, blocking, created_at) values ($1, $2, $3, $4, $5, $6, $7, $8::text::timestamptz) returning review_id::bigint as review_id, change_id, patchset_id, reviewer, action, comment, blocking, created_at::text as created_at"),
                    &[&repo_id, &change_id, &patchset_id, &reviewer, &action, &comment, &blocking, &now],
                )
                .map_err(|exc| exc.to_string())?;
            if action == TASK_REVIEW_APPROVE_ACTION
                || action == CODE_REVIEW_SUMMARY_ACTION
                || action == "approve"
                || blocking
            {
                store.invalidate_patchset_policy(patchset_id)?;
            }
            store.record_event(
                "review.recorded",
                "change",
                change_id,
                &json!({"patchset_id": patchset_id, "reviewer": reviewer, "action": action}),
                &now,
            )?;
            store.refresh_change_state(change_id, &now)?;
            Ok(review_row_json(&row, true))
        })
    }

    fn list_reviews(&mut self, change_id: &str) -> Result<JsonMap<String, JsonValue>, String> {
        self.change_row(change_id)?;
        let patchset_id = self.current_patchset_id(change_id)?;
        let reviews_table = self.control_table("reviews");
        let review_rows = self
            .client
            .query(
                &format!("select review_id::bigint as review_id, change_id, patchset_id, reviewer, action, comment, blocking, created_at::text as created_at from {reviews_table} where change_id = $1 order by review_id asc"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?;
        let reviews = review_rows
            .iter()
            .map(|row| JsonValue::Object(review_row_json(row, true)))
            .collect::<Vec<_>>();
        let requests_table = self.control_table("review_requests");
        let request_rows = self
            .client
            .query(
                &format!("select review_request_id::bigint as review_request_id, patchset_id, reviewer_group, note, created_at::text as created_at from {requests_table} where change_id = $1 order by review_request_id asc"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?;
        let requests = request_rows
            .iter()
            .map(|row| JsonValue::Object(review_request_row_json(row)))
            .collect::<Vec<_>>();
        let summary = if let Some(patchset_id) = patchset_id.as_deref() {
            self.review_summary(change_id, patchset_id)?
        } else {
            empty_review_summary()
        };
        Ok(JsonMap::from_iter([
            ("change_id".to_string(), json!(change_id)),
            (
                "current_patchset_id".to_string(),
                patchset_id
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "approvals".to_string(),
                json!(int_value(summary.get("approval_count")).unwrap_or(0)),
            ),
            (
                "task_approvals".to_string(),
                json!(int_value(summary.get("task_approval_count")).unwrap_or(0)),
            ),
            (
                "team_approvals".to_string(),
                json!(int_value(summary.get("team_approval_count")).unwrap_or(0)),
            ),
            (
                "human_approvals".to_string(),
                json!(int_value(summary.get("human_approval_count")).unwrap_or(0)),
            ),
            (
                "independent_human_approvals".to_string(),
                json!(int_value(summary.get("independent_human_approval_count")).unwrap_or(0)),
            ),
            (
                "human_task_approvals".to_string(),
                json!(int_value(summary.get("human_task_approval_count")).unwrap_or(0)),
            ),
            (
                "independent_task_approvals".to_string(),
                json!(int_value(summary.get("independent_task_approval_count")).unwrap_or(0)),
            ),
            (
                "code_review_summary_reviewers".to_string(),
                json!(int_value(summary.get("code_review_summary_reviewer_count")).unwrap_or(0)),
            ),
            (
                "blocking".to_string(),
                json!(int_value(summary.get("blocking_count")).unwrap_or(0)),
            ),
            (
                "comments".to_string(),
                json!(int_value(summary.get("comment_count")).unwrap_or(0)),
            ),
            (
                "code_review_summaries".to_string(),
                json!(int_value(summary.get("code_review_summary_count")).unwrap_or(0)),
            ),
            ("reviews".to_string(), JsonValue::Array(reviews)),
            ("review_requests".to_string(), JsonValue::Array(requests)),
        ]))
    }

    fn transaction<F>(&mut self, f: F) -> Result<JsonMap<String, JsonValue>, String>
    where
        F: FnOnce(&mut Self) -> Result<JsonMap<String, JsonValue>, String>,
    {
        self.client
            .batch_execute("begin")
            .map_err(|exc| exc.to_string())?;
        match f(self) {
            Ok(value) => {
                self.client
                    .batch_execute("commit")
                    .map_err(|exc| exc.to_string())?;
                Ok(value)
            }
            Err(err) => {
                let _ = self.client.batch_execute("rollback");
                Err(err)
            }
        }
    }

    fn change_row(&mut self, change_id: &str) -> Result<JsonMap<String, JsonValue>, String> {
        let changes = self.control_table("changes");
        let row = self
            .client
            .query_opt(
                &format!("select change_id, repo_name, repo_id, task_id, status, base_line, current_patchset_number::bigint as current_patchset_number, selected_patchset_number::bigint as selected_patchset_number, updated_at::text as updated_at from {changes} where change_id = $1"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Unknown change: {change_id}"))?;
        change_row_json(&row)
    }

    fn patchset_for_change(
        &mut self,
        patchset_id: &str,
        change_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let patchsets = self.control_table("patchsets");
        let row = self
            .client
            .query_opt(
                &format!("select patchset_id, repo_id, change_id, patchset_number::bigint as patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at::text as created_at from {patchsets} where patchset_id = $1 and change_id = $2"),
                &[&patchset_id, &change_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Patchset {patchset_id} does not belong to change {change_id}"))?;
        patchset_row_json(&row)
    }

    fn repo_id_for_repo_name(&mut self, repo_name: &str) -> Result<String, String> {
        let repositories = self.content_table("repositories");
        let row = self
            .client
            .query_opt(
                &format!("select repo_id from {repositories} where repo_name = $1"),
                &[&repo_name],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Unknown repository: {repo_name}"))?;
        row_text(&row, "repo_id")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("Repository {repo_name} is missing repo_id"))
    }

    fn current_patchset_id(&mut self, change_id: &str) -> Result<Option<String>, String> {
        let patchsets = self.control_table("patchsets");
        self.client
            .query_opt(
                &format!("select patchset_id from {patchsets} where change_id = $1 order by patchset_number desc limit 1"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())
            .map(|row| row.and_then(|row| row_text(&row, "patchset_id")))
    }

    fn latest_policy_status(
        &mut self,
        patchset_id: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let policy_decisions = self.control_table("policy_decisions");
        self.client
            .query_opt(
                &format!("select decision, checks_json, input_fingerprint, created_at::text as created_at from {policy_decisions} where patchset_id = $1 order by policy_decision_id desc limit 1"),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?
            .map(|row| {
                let checks_json = row_text(&row, "checks_json").unwrap_or_else(|| "[]".to_string());
                Ok(JsonMap::from_iter([
                    ("patchset_id".to_string(), json!(patchset_id)),
                    ("decision".to_string(), row_text(&row, "decision").map_or(JsonValue::Null, JsonValue::String)),
                    (
                        "checks".to_string(),
                        serde_json::from_str::<JsonValue>(&checks_json).unwrap_or_else(|_| json!([])),
                    ),
                    ("input_fingerprint".to_string(), row_text(&row, "input_fingerprint").map_or(JsonValue::Null, JsonValue::String)),
                    ("evaluated_at".to_string(), row_text(&row, "created_at").map_or(JsonValue::Null, JsonValue::String)),
                ]))
            })
            .transpose()
    }

    fn review_summary(
        &mut self,
        change_id: &str,
        patchset_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let reviews = self.control_table("reviews");
        let rows = self
            .client
            .query(
                &format!("select review_id::bigint as review_id, reviewer, action, blocking, comment, created_at::text as created_at, patchset_id from {reviews} where change_id = $1 and patchset_id = $2 order by review_id asc"),
                &[&change_id, &patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        let reviews = rows
            .iter()
            .map(|row| review_row_json(row, false))
            .collect::<Vec<_>>();
        Ok(review_summary_from_rows(&reviews, Some(patchset_id)))
    }

    fn refresh_change_state(&mut self, change_id: &str, now: &str) -> Result<String, String> {
        let change = self.change_row(change_id)?;
        let existing_state = optional_text(change.get("status")).unwrap_or_default();
        if existing_state == "landed" || existing_state == "archived" {
            return Ok(existing_state);
        }
        let current_patchset_number = int_value(change.get("current_patchset_number")).unwrap_or(0);
        let new_state = if current_patchset_number == 0 {
            "draft".to_string()
        } else {
            let patchset = self.current_patchset_for_change(change_id)?;
            let latest = self.latest_policy_status(
                required_text(patchset.get("patchset_id"), "patchset.patchset_id")?.as_str(),
            )?;
            let policy = effective_policy_status(&patchset, latest.as_ref())?;
            let policy = payload_object(Some(&policy), "effective policy status")?;
            let patchset_id = required_text(patchset.get("patchset_id"), "patchset.patchset_id")?;
            let review = self.review_summary(change_id, &patchset_id)?;
            let repo_name = required_text(change.get("repo_name"), "change.repo_name")?;
            let base_line = optional_text(change.get("base_line")).unwrap_or_default();
            let base_line_head = self.content_line_head(&repo_name, &base_line)?;
            let base_snapshot_id =
                optional_text(patchset.get("base_snapshot_id")).unwrap_or_default();
            let stale = base_line_head
                .as_deref()
                .is_some_and(|head| !head.is_empty() && head != base_snapshot_id);
            let blocking_count = int_value(review.get("blocking_count")).unwrap_or(0);
            let approval_count = int_value(review.get("approval_count")).unwrap_or(0);
            let decision =
                optional_text(policy.get("decision")).unwrap_or_else(|| "pending".to_string());
            if blocking_count > 0 || decision == "hard_fail" || stale {
                "blocked".to_string()
            } else if decision == "pass" && approval_count >= REQUIRED_APPROVALS && !stale {
                "landable".to_string()
            } else if approval_count >= REQUIRED_APPROVALS && decision == "pass" {
                "approved".to_string()
            } else if decision == "pending" || decision == "soft_fail" {
                "gated".to_string()
            } else {
                "review".to_string()
            }
        };
        if new_state != existing_state {
            let changes = self.control_table("changes");
            self.client
                .execute(
                    &format!("update {changes} set status = $1, updated_at = $2::text::timestamptz where change_id = $3"),
                    &[&new_state, &now, &change_id],
                )
                .map_err(|exc| exc.to_string())?;
        }
        Ok(new_state)
    }

    fn current_patchset_for_change(
        &mut self,
        change_id: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let patchsets = self.control_table("patchsets");
        let row = self
            .client
            .query_opt(
                &format!("select patchset_id, repo_id, change_id, patchset_number::bigint as patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at::text as created_at from {patchsets} where change_id = $1 order by patchset_number desc limit 1"),
                &[&change_id],
            )
            .map_err(|exc| exc.to_string())?
            .ok_or_else(|| format!("Change {change_id} has no published patchset"))?;
        patchset_row_json(&row)
    }

    fn content_line_head(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> Result<Option<String>, String> {
        if repo_name.trim().is_empty() || line_name.trim().is_empty() {
            return Ok(None);
        }
        let lines = self.content_table("lines");
        self.client
            .query_opt(
                &format!("select head_snapshot_id from {lines} where repo_name = $1 and line_name = $2 limit 1"),
                &[&repo_name, &line_name],
            )
            .map_err(|exc| exc.to_string())
            .map(|row| row.and_then(|row| row_text(&row, "head_snapshot_id")))
    }

    fn invalidate_patchset_policy(&mut self, patchset_id: &str) -> Result<(), String> {
        let patchsets = self.control_table("patchsets");
        self.client
            .execute(
                &format!(
                    "update {patchsets} set evaluation_state = 'pending' where patchset_id = $1"
                ),
                &[&patchset_id],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(())
    }

    fn record_event(
        &mut self,
        event_type: &str,
        entity_type: &str,
        entity_id: &str,
        payload: &JsonValue,
        created_at: &str,
    ) -> Result<(), String> {
        let events = self.control_table("events");
        let payload_json = serde_json::to_string(payload).map_err(|exc| exc.to_string())?;
        self.client
            .execute(
                &format!("insert into {events}(event_type, entity_type, entity_id, payload_json, actor_identity, actor_type, created_at) values ($1, $2, $3, $4, 'system', 'system_worker', $5::text::timestamptz)"),
                &[&event_type, &entity_type, &entity_id, &payload_json, &created_at],
            )
            .map_err(|exc| exc.to_string())?;
        Ok(())
    }

    fn content_table(&self, name: &str) -> String {
        schema_table(&self.content_schema, name)
    }

    fn control_table(&self, name: &str) -> String {
        schema_table(&self.control_schema, name)
    }
}

fn patchset_row_json(row: &Row) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = JsonMap::new();
    insert_text(&mut out, "patchset_id", row_text(row, "patchset_id"));
    insert_text(&mut out, "repo_id", row_text(row, "repo_id"));
    insert_text(&mut out, "change_id", row_text(row, "change_id"));
    insert_i64(&mut out, "patchset_number", row_i64(row, "patchset_number"));
    insert_text(
        &mut out,
        "base_snapshot_id",
        row_text(row, "base_snapshot_id"),
    );
    insert_text(
        &mut out,
        "revision_snapshot_id",
        row_text(row, "revision_snapshot_id"),
    );
    insert_text(&mut out, "summary", row_text(row, "summary"));
    insert_text(&mut out, "author_mode", row_text(row, "author_mode"));
    insert_text(&mut out, "publish_state", row_text(row, "publish_state"));
    let diff_stats_json = row_text(row, "diff_stats_json").unwrap_or_else(|| "{}".to_string());
    out.insert("diff_stats_json".to_string(), json!(diff_stats_json));
    out.insert(
        "diff_stats".to_string(),
        serde_json::from_str::<JsonValue>(&diff_stats_json).unwrap_or_else(|_| json!({})),
    );
    insert_text(
        &mut out,
        "evaluation_state",
        row_text(row, "evaluation_state"),
    );
    insert_text(&mut out, "created_at", row_text(row, "created_at"));
    Ok(out)
}

fn change_row_json(row: &Row) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = JsonMap::new();
    insert_text(&mut out, "change_id", row_text(row, "change_id"));
    insert_text(&mut out, "repo_name", row_text(row, "repo_name"));
    insert_text(&mut out, "repo_id", row_text(row, "repo_id"));
    insert_text(&mut out, "task_id", row_text(row, "task_id"));
    insert_text(&mut out, "status", row_text(row, "status"));
    insert_text(&mut out, "base_line", row_text(row, "base_line"));
    insert_i64(
        &mut out,
        "current_patchset_number",
        row_i64(row, "current_patchset_number"),
    );
    insert_i64(
        &mut out,
        "selected_patchset_number",
        row_i64(row, "selected_patchset_number"),
    );
    insert_text(&mut out, "updated_at", row_text(row, "updated_at"));
    Ok(out)
}

fn review_row_json(row: &Row, include_change_id: bool) -> JsonMap<String, JsonValue> {
    let mut out = JsonMap::new();
    insert_i64(&mut out, "review_id", row_i64(row, "review_id"));
    if include_change_id {
        insert_text(&mut out, "change_id", row_text(row, "change_id"));
    }
    insert_text(&mut out, "patchset_id", row_text(row, "patchset_id"));
    insert_text(&mut out, "reviewer", row_text(row, "reviewer"));
    insert_text(&mut out, "action", row_text(row, "action"));
    insert_text(&mut out, "comment", row_text(row, "comment"));
    out.insert(
        "blocking".to_string(),
        row_bool(row, "blocking").map_or(JsonValue::Null, JsonValue::Bool),
    );
    insert_text(&mut out, "created_at", row_text(row, "created_at"));
    out
}

fn review_request_row_json(row: &Row) -> JsonMap<String, JsonValue> {
    let mut out = JsonMap::new();
    insert_i64(
        &mut out,
        "review_request_id",
        row_i64(row, "review_request_id"),
    );
    insert_text(&mut out, "patchset_id", row_text(row, "patchset_id"));
    insert_text(&mut out, "reviewer_group", row_text(row, "reviewer_group"));
    insert_text(&mut out, "note", row_text(row, "note"));
    insert_text(&mut out, "created_at", row_text(row, "created_at"));
    out
}

fn empty_review_summary() -> JsonMap<String, JsonValue> {
    JsonMap::from_iter([
        ("approval_count".to_string(), json!(0)),
        ("task_approval_count".to_string(), json!(0)),
        ("team_approval_count".to_string(), json!(0)),
        ("human_approval_count".to_string(), json!(0)),
        ("independent_human_approval_count".to_string(), json!(0)),
        ("human_task_approval_count".to_string(), json!(0)),
        ("independent_task_approval_count".to_string(), json!(0)),
        ("code_review_summary_reviewer_count".to_string(), json!(0)),
        ("blocking_count".to_string(), json!(0)),
        ("comment_count".to_string(), json!(0)),
        ("code_review_summary_count".to_string(), json!(0)),
        ("review_count".to_string(), json!(0)),
    ])
}

fn schema_table(schema: &str, table: &str) -> String {
    format!("\"{schema}\".\"{table}\"")
}

fn ensure_change_mutable(change: &JsonMap<String, JsonValue>, action: &str) -> Result<(), String> {
    let status = optional_text(change.get("status")).unwrap_or_default();
    let change_id = optional_text(change.get("change_id")).unwrap_or_default();
    if status == "archived" {
        return Err(format!(
            "Change {change_id} is archived and cannot {action}"
        ));
    }
    if status == "landed" {
        return Err(format!("Change {change_id} is landed and cannot {action}"));
    }
    Ok(())
}

fn code_review_summary_requirement_text(value: Option<&str>) -> String {
    let suffix = value
        .filter(|text| !text.trim().is_empty())
        .map(|_| " The supplied message is missing one or more required sections.")
        .unwrap_or(" A structured summary message is required.");
    format!(
        "Code review summary must include Reviewed files, Findings, Risks, Tests, and Recommendation sections. Use `ait review code template --style numbered`.{suffix}"
    )
}

fn row_text(row: &Row, name: &str) -> Option<String> {
    row.try_get::<_, Option<String>>(name).ok().flatten()
}

fn row_i64(row: &Row, name: &str) -> Option<i64> {
    row.try_get::<_, Option<i64>>(name).ok().flatten()
}

fn row_bool(row: &Row, name: &str) -> Option<bool> {
    row.try_get::<_, Option<bool>>(name).ok().flatten()
}

fn insert_text(out: &mut JsonMap<String, JsonValue>, key: &str, value: Option<String>) {
    out.insert(
        key.to_string(),
        value.map(JsonValue::String).unwrap_or(JsonValue::Null),
    );
}

fn insert_i64(out: &mut JsonMap<String, JsonValue>, key: &str, value: Option<i64>) {
    out.insert(
        key.to_string(),
        value.map_or(JsonValue::Null, JsonValue::from),
    );
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text(value)
        .ok_or_else(|| format!("review-store payload requires text field `{field}`."))
}

fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if !truthy(value) {
        return None;
    }
    let text = match value {
        JsonValue::String(text) => text.clone(),
        JsonValue::Bool(true) => "True".to_string(),
        JsonValue::Bool(false) => String::new(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
        JsonValue::Null => String::new(),
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn payload_object(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .cloned()
        .ok_or_else(|| format!("{field} must be a JSON object."))
}

fn string_list(value: Option<&JsonValue>, field: &str) -> Result<Vec<String>, String> {
    let Some(values) = value.and_then(JsonValue::as_array) else {
        return Err(format!(
            "review-store payload requires array field `{field}`."
        ));
    };
    Ok(values
        .iter()
        .filter_map(|item| optional_text(Some(item)))
        .collect())
}

fn truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(value) => *value,
        JsonValue::Number(number) => number.as_f64().map(|value| value != 0.0).unwrap_or(true),
        JsonValue::String(text) => !text.trim().is_empty(),
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
    }
}

fn int_value(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        JsonValue::Bool(value) => Some(if *value { 1 } else { 0 }),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn utc_now() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_declares_postgres_review_store_operations() {
        let value = server_review_store_json("contract", &json!({})).expect("contract");
        assert_eq!(value["contract"], json!(SERVER_REVIEW_STORE_CONTRACT));
        assert_eq!(value["backend"], json!("postgres"));
        assert_eq!(
            value["migration_status"],
            json!("rust_owned_no_python_reference")
        );
        assert!(value.get("previous_reference_module").is_none());
        assert_eq!(value["mutates_state"], json!(true));
        assert!(value["operations"]
            .as_array()
            .unwrap()
            .contains(&json!("request-review")));
        assert!(value["operations"]
            .as_array()
            .unwrap()
            .contains(&json!("record-review")));
        assert!(value["operations"]
            .as_array()
            .unwrap()
            .contains(&json!("list-reviews")));
    }

    #[test]
    fn runtime_rejects_non_postgres_and_fake_postgres() {
        let err = server_review_store_json(
            "list-reviews",
            &json!({"backend": "local-file", "change_id": "C-1", "dsn": "postgresql://demo"}),
        )
        .expect_err("non-postgres backend should be rejected");
        assert!(err.contains("Only PostgreSQL is supported"));

        let err = server_review_store_json(
            "list-reviews",
            &json!({"backend": "postgres", "change_id": "C-1", "dsn": "fake-postgres:///tmp/x"}),
        )
        .expect_err("fake postgres should be rejected");
        assert!(err.contains("fake-postgres is not supported"));
    }

    #[test]
    fn code_review_summary_requirement_mentions_required_sections() {
        let err = code_review_summary_requirement_text(Some("too short"));
        assert!(err.contains("Reviewed files"));
        assert!(err.contains("Recommendation"));
        assert!(err.contains("ait review code template --style numbered"));
    }
}
