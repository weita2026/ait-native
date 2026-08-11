#[cfg(feature = "legacy-postgres-runtime")]
use super::json_helpers::optional_i64;
use super::json_helpers::{
    bytes_map, optional_f64, optional_text, optional_usize, print_json, required_text,
    required_value,
};
use super::*;

#[cfg(feature = "legacy-postgres-runtime")]
pub(super) fn policy_store_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let mut payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    if operation != "contract" {
        let payload = payload_value
            .as_object_mut()
            .ok_or_else(|| "policy-store payload must be a JSON object.".to_string())?;
        if !payload.contains_key("backend") {
            payload.insert(
                "backend".to_string(),
                env::var("AIT_NATIVE_SERVER_DB_BACKEND")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(JsonValue::String)
                    .unwrap_or_else(|| json!("postgres")),
            );
        }
        if !payload.contains_key("dsn") && !payload.contains_key("postgres_dsn") {
            if let Ok(dsn) = env::var("AIT_NATIVE_SERVER_POSTGRES_DSN") {
                if !dsn.trim().is_empty() {
                    payload.insert("dsn".to_string(), JsonValue::String(dsn));
                }
            }
        }
        if !payload.contains_key("content_schema") {
            payload.insert(
                "content_schema".to_string(),
                env::var("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(JsonValue::String)
                    .unwrap_or_else(|| json!("ait_native_content")),
            );
        }
        if !payload.contains_key("control_schema") {
            payload.insert(
                "control_schema".to_string(),
                env::var("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(JsonValue::String)
                    .unwrap_or_else(|| json!("ait_native_control")),
            );
        }
    }
    print_json(&server_policy_store_json(operation, &payload_value)?)
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(super) fn patchset_store_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let mut payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    if operation != "contract" {
        let payload = payload_value
            .as_object_mut()
            .ok_or_else(|| "patchset-store payload must be a JSON object.".to_string())?;
        if !payload.contains_key("backend") {
            payload.insert(
                "backend".to_string(),
                env::var("AIT_NATIVE_SERVER_DB_BACKEND")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(JsonValue::String)
                    .unwrap_or_else(|| json!("postgres")),
            );
        }
        if !payload.contains_key("dsn") && !payload.contains_key("postgres_dsn") {
            if let Ok(dsn) = env::var("AIT_NATIVE_SERVER_POSTGRES_DSN") {
                if !dsn.trim().is_empty() {
                    payload.insert("dsn".to_string(), JsonValue::String(dsn));
                }
            }
        }
        if !payload.contains_key("content_schema") {
            payload.insert(
                "content_schema".to_string(),
                env::var("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(JsonValue::String)
                    .unwrap_or_else(|| json!("ait_native_content")),
            );
        }
        if !payload.contains_key("control_schema") {
            payload.insert(
                "control_schema".to_string(),
                env::var("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(JsonValue::String)
                    .unwrap_or_else(|| json!("ait_native_control")),
            );
        }
        if !payload.contains_key("server_data") && !payload.contains_key("root") {
            if let Ok(root) = env::var("AIT_NATIVE_SERVER_ROOT") {
                if !root.trim().is_empty() {
                    payload.insert("server_data".to_string(), JsonValue::String(root));
                }
            }
        }
    }
    print_json(&server_patchset_store_json(operation, &payload_value)?)
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(super) fn review_store_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let mut payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    if operation != "contract" {
        let payload = payload_value
            .as_object_mut()
            .ok_or_else(|| "review-store payload must be a JSON object.".to_string())?;
        if !payload.contains_key("backend") {
            payload.insert(
                "backend".to_string(),
                env::var("AIT_NATIVE_SERVER_DB_BACKEND")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(JsonValue::String)
                    .unwrap_or_else(|| json!("postgres")),
            );
        }
        if !payload.contains_key("dsn") && !payload.contains_key("postgres_dsn") {
            if let Ok(dsn) = env::var("AIT_NATIVE_SERVER_POSTGRES_DSN") {
                if !dsn.trim().is_empty() {
                    payload.insert("dsn".to_string(), JsonValue::String(dsn));
                }
            }
        }
        if !payload.contains_key("content_schema") {
            payload.insert(
                "content_schema".to_string(),
                env::var("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(JsonValue::String)
                    .unwrap_or_else(|| json!("ait_native_content")),
            );
        }
        if !payload.contains_key("control_schema") {
            payload.insert(
                "control_schema".to_string(),
                env::var("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(JsonValue::String)
                    .unwrap_or_else(|| json!("ait_native_control")),
            );
        }
    }
    print_json(&server_review_store_json(operation, &payload_value)?)
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(super) fn worker_queue_kernel_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&worker_queue_kernel_json(&payload_value)?)
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(super) fn worker_queue_service_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let payload = payload_value
        .as_object()
        .ok_or_else(|| "worker-queue-service payload must be a JSON object.".to_string())?;
    let backend = optional_text(payload, "backend").unwrap_or_else(|| "postgres".to_string());
    let dsn = optional_text(payload, "dsn").or_else(|| {
        env::var("AIT_NATIVE_SERVER_POSTGRES_DSN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    });
    let content_schema = optional_text(payload, "content_schema")
        .or_else(|| env::var("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA").ok())
        .unwrap_or_else(|| "ait_content".to_string());
    let control_schema = optional_text(payload, "control_schema")
        .or_else(|| env::var("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA").ok())
        .unwrap_or_else(|| "ait_control".to_string());
    if dsn
        .as_deref()
        .is_some_and(|value| value.trim().starts_with(FAKE_POSTGRES_PREFIX))
    {
        return Err(
            "fake-postgres is no longer supported; ait-server requires PostgreSQL.".to_string(),
        );
    }
    let pool_max_size = optional_text(payload, "pool_max_size")
        .or_else(|| env::var("AIT_NATIVE_SERVER_POSTGRES_POOL_MAX_SIZE").ok());
    let timeouts = PostgresTimeoutScope {
        lock_timeout_ms: optional_i64(payload, "lock_timeout_ms")?,
        statement_timeout_ms: optional_i64(payload, "statement_timeout_ms")?,
    };
    let registry = Arc::new(PostgresConnectionPoolRegistry::new(
        Arc::new(NativePostgresDriver),
        resolve_postgres_pool_max_size(pool_max_size.as_deref()),
    ));
    let pool = PostgresWorkerQueuePool::new(
        registry.clone(),
        backend,
        dsn,
        content_schema,
        control_schema,
        timeouts,
    );
    let mut request = payload_value;
    if let Some(request_obj) = request.as_object_mut() {
        let needs_repo_id = request_obj
            .get("operation")
            .and_then(JsonValue::as_str)
            .is_some_and(|operation| operation == "enqueue-job");
        let has_repo_id = request_obj
            .get("repo_id")
            .and_then(JsonValue::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        if needs_repo_id && !has_repo_id {
            if let Some(repo_name) = request_obj
                .get("repo_name")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(repo_id) = pool.resolve_repo_id(repo_name)? {
                    request_obj.insert("repo_id".to_string(), JsonValue::String(repo_id));
                }
            }
        }
    }
    let kernel = WorkerQueueKernel::new(pool, SchedulerPolicy::default());
    let result = worker_queue_service_json(&kernel, &request)?;
    registry.close_all();
    print_json(&result)
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(super) fn postgres_runtime_probe_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let payload = payload_value
        .as_object()
        .ok_or_else(|| "postgres-runtime-probe payload must be a JSON object.".to_string())?;
    let backend = optional_text(payload, "backend").unwrap_or_else(|| "postgres".to_string());
    let dsn = optional_text(payload, "dsn").or_else(|| {
        env::var("AIT_NATIVE_SERVER_POSTGRES_DSN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    });
    let content_schema = optional_text(payload, "content_schema")
        .or_else(|| env::var("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA").ok())
        .unwrap_or_else(|| "ait_content".to_string());
    let control_schema = optional_text(payload, "control_schema")
        .or_else(|| env::var("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA").ok())
        .unwrap_or_else(|| "ait_control".to_string());
    let plane = optional_text(payload, "plane").unwrap_or_else(|| "content".to_string());
    let pool_max_size = optional_text(payload, "pool_max_size")
        .or_else(|| env::var("AIT_NATIVE_SERVER_POSTGRES_POOL_MAX_SIZE").ok());
    let timeouts = PostgresTimeoutScope {
        lock_timeout_ms: optional_i64(payload, "lock_timeout_ms")?,
        statement_timeout_ms: optional_i64(payload, "statement_timeout_ms")?,
    };
    let registry = PostgresConnectionPoolRegistry::new(
        Arc::new(NativePostgresDriver),
        resolve_postgres_pool_max_size(pool_max_size.as_deref()),
    );
    let mut conn = connect_server_plane(
        &registry,
        &backend,
        dsn.as_deref(),
        &content_schema,
        &control_schema,
        &plane,
        &timeouts,
    )?;
    let row = conn
        .raw_mut()
        .query_one(
            "select current_schema()::text as current_schema, current_setting('search_path') as search_path, 1::int4 as probe_value",
            &[],
        )
        .map_err(|exc| exc.to_string())?;
    let current_schema: Option<String> = row.get("current_schema");
    let search_path: String = row.get("search_path");
    let probe_value: i32 = row.get("probe_value");
    conn.close();
    registry.close_all();
    print_json(&json!({
        "contract": "ait.server.postgres.runtime_probe.v1",
        "connected": true,
        "backend": backend,
        "plane": plane,
        "schema": if plane == "control" { control_schema } else { content_schema },
        "current_schema": current_schema,
        "search_path": search_path,
        "probe_value": probe_value,
    }))
}

pub(super) fn server_storage_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let obj = payload
        .as_object()
        .ok_or_else(|| "server-storage payload must be a JSON object.".to_string())?;
    let result = match operation {
        "build-pack-members" => {
            let max_depth = optional_usize(obj, "max_delta_chain_depth")?.unwrap_or(4);
            pack_substrate::build_pack_members(
                required_value(obj, "blob_items")?,
                max_depth,
                obj.get("initial_by_path").filter(|value| !value.is_null()),
            )
        }
        "write-pack-archive" => pack_substrate::write_pack_archive(
            &required_text(obj, "pack_path")?,
            &required_text(obj, "pack_id")?,
            &required_text(obj, "created_at")?,
            required_value(obj, "members")?,
        ),
        "read-pack-index" => {
            let pack_path = required_text(obj, "pack_path")?;
            match optional_text(obj, "pack_format") {
                Some(pack_format) => {
                    pack_substrate::read_pack_index_with_format(&pack_path, &pack_format)
                }
                None => pack_substrate::read_pack_index(&pack_path),
            }
        }
        "pack-has-entry" => Ok(json!({
            "exists": pack_substrate::pack_has_entry(
                &required_text(obj, "pack_path")?,
                &required_text(obj, "entry_name")?,
            )
        })),
        "read-pack-entry" => {
            let base_map = bytes_map(obj.get("resolve_base_blob_map"))?;
            let pack_path = required_text(obj, "pack_path")?;
            let entry_name = required_text(obj, "entry_name")?;
            let max_chain_depth = optional_usize(obj, "max_chain_depth")?.unwrap_or(4);
            let bytes = match optional_text(obj, "pack_format") {
                Some(pack_format) => pack_substrate::read_pack_entry_with_format(
                    &pack_path,
                    &entry_name,
                    base_map.as_ref(),
                    max_chain_depth,
                    &pack_format,
                ),
                None => pack_substrate::read_pack_entry(
                    &pack_path,
                    &entry_name,
                    base_map.as_ref(),
                    max_chain_depth,
                ),
            }?;
            Ok(json!({"data": bytes}))
        }
        "summarize-pack-archives" => pack_substrate::summarize_pack_archives(
            &required_text(obj, "root")?,
            required_value(obj, "pack_rows")?,
        ),
        "build-storage-validation-summary" => Ok(pack_substrate::build_storage_validation_summary(
            optional_usize(obj, "packed_blob_count")?.unwrap_or(0),
            optional_usize(obj, "packed_full_blob_count")?.unwrap_or(0),
            optional_usize(obj, "packed_delta_blob_count")?.unwrap_or(0),
            optional_usize(obj, "pack_count")?.unwrap_or(0),
            optional_usize(obj, "pack_index_error_count")?.unwrap_or(0),
            optional_usize(obj, "tree_pack_index_error_count")?.unwrap_or(0),
            optional_f64(obj, "storage_savings_ratio")?.unwrap_or(0.0),
            optional_usize(obj, "unreferenced_blob_count")?.unwrap_or(0),
            optional_usize(obj, "unreferenced_tree_count")?.unwrap_or(0),
            obj.get("signals_summary"),
        )),
        "build-tree-pack-members" => pack_substrate::build_tree_pack_members(
            required_value(obj, "tree_rows")?,
            required_value(obj, "tree_entry_rows")?,
        ),
        "write-tree-pack-archive" => pack_substrate::write_tree_pack_archive(
            &required_text(obj, "pack_path")?,
            &required_text(obj, "pack_id")?,
            &required_text(obj, "created_at")?,
            required_value(obj, "members")?,
        ),
        "read-tree-pack-index" => {
            pack_substrate::read_tree_pack_index(&required_text(obj, "pack_path")?)
        }
        "read-tree-pack-index-without-ordinals" => {
            pack_substrate::read_tree_pack_index_without_ordinals(&required_text(obj, "pack_path")?)
        }
        "read-tree-pack-tree" => pack_substrate::read_tree_pack_tree(
            &required_text(obj, "pack_path")?,
            &required_text(obj, "tree_id")?,
        ),
        "read-tree-pack-tree-by-ordinal" => pack_substrate::read_tree_pack_tree_by_ordinal(
            &required_text(obj, "pack_path")?,
            optional_usize(obj, "entry_ordinal")?
                .ok_or_else(|| "entry_ordinal must be an integer".to_string())?,
        ),
        "tree-pack-contains-blob-ids" => pack_substrate::tree_pack_contains_blob_ids(
            &required_text(obj, "pack_path")?,
            required_value(obj, "blob_ids")?,
        ),
        "summarize-tree-pack-archives" => pack_substrate::summarize_tree_pack_archives(
            &required_text(obj, "root")?,
            required_value(obj, "pack_rows")?,
        ),
        "tree-pack-manifest-path" => Ok(json!({
            "manifest_path": pack_substrate::tree_pack_manifest_path(
                &required_text(obj, "pack_path")?,
                &required_text(obj, "entry_name")?,
            )
        })),
        "build-tree-records" => {
            revision_trees::build_tree_records(required_value(obj, "file_entries")?)
        }
        "build-snapshot-id" => revision_trees::build_snapshot_id(&payload),
        _ => Err(format!("Unsupported server-storage operation: {operation}")),
    }?;
    print_json(&result)
}
