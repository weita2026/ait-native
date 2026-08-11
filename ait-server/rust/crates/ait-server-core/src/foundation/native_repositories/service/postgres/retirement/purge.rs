use super::*;

pub(super) fn purge_repository(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    repo_id: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let control_deleted = service.with_control_write(|client| {
        let plan_ids = repo_plan_ids(client, repo_id, repo_name)?;
        let authority_map_ids = repo_authority_map_ids(client, repo_id, repo_name)?;
        let mut counts = JsonMap::new();
        counts.insert(
            "plan_revisions".to_string(),
            json!(delete_rows_by_ids(
                client,
                "plan_revisions",
                "plan_id",
                &plan_ids
            )?),
        );
        counts.insert(
            "authority_nodes".to_string(),
            json!(delete_rows_by_ids(
                client,
                "authority_nodes",
                "authority_map_id",
                &authority_map_ids,
            )?),
        );
        counts.insert(
            "authority_mutations".to_string(),
            json!(delete_rows_by_ids(
                client,
                "authority_mutations",
                "authority_map_id",
                &authority_map_ids,
            )?),
        );
        for table in PURGE_REPO_SCOPED_CONTROL_TABLES {
            counts.insert(
                (*table).to_string(),
                json!(delete_repo_rows(client, table, repo_id, repo_name)?),
            );
        }
        Ok(JsonValue::Object(counts))
    })?;

    let content_cleanup = service.with_write(|client| {
        let object_pack_rows = pack_rows(client, "packs", repo_id, repo_name)?;
        let tree_pack_rows = pack_rows(client, "tree_packs", repo_id, repo_name)?;
        let pack_ids = object_pack_rows
            .iter()
            .filter_map(|row| {
                row.get("pack_id")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        let repo_blob_ids = repo_blob_ids(client, repo_name, repo_id, &pack_ids)?;
        let repo_row = client
            .query_opt(
                "select repo_name from repositories where repo_name = $1 and repo_id = $2",
                &[&repo_name, &repo_id],
            )
            .map_err(db_internal)?;
        if repo_row.is_none() {
            return Err(NativeRepositoryError::not_found(format!(
                "Unknown repository: {repo_name}"
            )));
        }
        client
            .execute(
                "delete from repositories where repo_name = $1 and repo_id = $2",
                &[&repo_name, &repo_id],
            )
            .map_err(db_internal)?;
        let removed_blob_count = delete_unreferenced_blobs(client, &repo_blob_ids)?;
        let removed_pack_count =
            remove_pack_files_if_unreferenced(client, &service.paths, "blobs", &object_pack_rows)?;
        let removed_tree_pack_count =
            remove_tree_pack_files_if_unreferenced(client, &service.paths, &tree_pack_rows)?;
        for token in [repo_id, repo_name] {
            let path = service.paths.ref_root.join(encode_ref_name(token));
            if path.exists() {
                let _ = fs::remove_dir_all(&path);
            }
        }
        Ok(json!({
            "removed_unreferenced_blob_count": removed_blob_count,
            "removed_unreachable_tree_count": 0,
            "removed_unreachable_tree_entry_count": 0,
            "removed_pack_count": removed_pack_count,
            "removed_tree_pack_count": removed_tree_pack_count,
            "deferred_global_gc": true
        }))
    })?;

    Ok(json!({
        "control_deleted": control_deleted,
        "content_cleanup": content_cleanup
    }))
}

pub(super) fn table_columns(
    client: &mut pg::Client,
    table: &str,
) -> Result<BTreeSet<String>, NativeRepositoryError> {
    let rows = client
        .query(
            "select column_name from information_schema.columns where table_schema = current_schema() and table_name = $1",
            &[&table],
        )
        .map_err(db_internal)?;
    Ok(rows
        .into_iter()
        .map(|row| row.get::<_, String>("column_name"))
        .collect())
}

pub(super) fn table_repo_rows(
    client: &mut pg::Client,
    table: &str,
    repo_id: &str,
    repo_name: &str,
) -> Result<Vec<JsonValue>, NativeRepositoryError> {
    let columns = table_columns(client, table)?;
    let (where_sql, params): (&str, Vec<&(dyn ToSql + Sync)>) =
        if columns.contains("repo_id") && columns.contains("repo_name") {
            (
                "(repo_id = $1 or (repo_id is null and repo_name = $2))",
                vec![&repo_id, &repo_name],
            )
        } else if columns.contains("repo_id") {
            ("repo_id = $1", vec![&repo_id])
        } else if columns.contains("repo_name") {
            ("repo_name = $1", vec![&repo_name])
        } else {
            return Err(NativeRepositoryError::internal(format!(
                "Table {table} is not repository scoped"
            )));
        };
    query_json_object_rows(
        client,
        format!(
            "select to_jsonb(t)::text as row_json from {table} t where {where_sql} order by 1 asc"
        )
        .as_str(),
        params.as_slice(),
    )
}

pub(super) fn delete_repo_rows(
    client: &mut pg::Client,
    table: &str,
    repo_id: &str,
    repo_name: &str,
) -> Result<u64, NativeRepositoryError> {
    let columns = table_columns(client, table)?;
    let (where_sql, params): (&str, Vec<&(dyn ToSql + Sync)>) =
        if columns.contains("repo_id") && columns.contains("repo_name") {
            (
                "(repo_id = $1 or (repo_id is null and repo_name = $2))",
                vec![&repo_id, &repo_name],
            )
        } else if columns.contains("repo_id") {
            ("repo_id = $1", vec![&repo_id])
        } else if columns.contains("repo_name") {
            ("repo_name = $1", vec![&repo_name])
        } else {
            return Err(NativeRepositoryError::internal(format!(
                "Table {table} is not repository scoped"
            )));
        };
    client
        .execute(
            format!("delete from {table} where {where_sql}").as_str(),
            params.as_slice(),
        )
        .map_err(db_internal)
}

pub(super) fn rows_by_ids(
    client: &mut pg::Client,
    table: &str,
    column: &str,
    ids: &[String],
    order_by: &str,
) -> Result<Vec<JsonValue>, NativeRepositoryError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    query_json_object_rows(
        client,
        format!(
            "select to_jsonb(t)::text as row_json from {table} t where {column} = any($1) order by {order_by}"
        )
        .as_str(),
        &[&ids],
    )
}

pub(super) fn delete_rows_by_ids(
    client: &mut pg::Client,
    table: &str,
    column: &str,
    ids: &[String],
) -> Result<u64, NativeRepositoryError> {
    if ids.is_empty() {
        return Ok(0);
    }
    client
        .execute(
            format!("delete from {table} where {column} = any($1)").as_str(),
            &[&ids],
        )
        .map_err(db_internal)
}

pub(super) fn repo_plan_ids(
    client: &mut pg::Client,
    repo_id: &str,
    repo_name: &str,
) -> Result<Vec<String>, NativeRepositoryError> {
    text_column(
        client,
        "select plan_id from plans where repo_id = $1 or (repo_id is null and repo_name = $2) order by plan_id asc",
        &[&repo_id, &repo_name],
        "plan_id",
    )
}

pub(super) fn repo_authority_map_ids(
    client: &mut pg::Client,
    repo_id: &str,
    repo_name: &str,
) -> Result<Vec<String>, NativeRepositoryError> {
    text_column(
        client,
        "select authority_map_id from authority_maps where repo_id = $1 or (repo_id is null and repo_name = $2) order by authority_map_id asc",
        &[&repo_id, &repo_name],
        "authority_map_id",
    )
}

pub(super) fn control_blob_ids(exports: &BTreeMap<String, Vec<JsonValue>>) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for table in ["plan_revision_blobs", "plan_revision_artifacts"] {
        for row in exports.get(table).into_iter().flatten() {
            if let Some(blob_id) = row.get("blob_id").and_then(JsonValue::as_str) {
                if !blob_id.trim().is_empty() {
                    ids.insert(blob_id.to_string());
                }
            }
        }
    }
    for row in exports.get("releases").into_iter().flatten() {
        let Some(artifacts) = row.get("artifacts_json") else {
            continue;
        };
        let parsed = match artifacts {
            JsonValue::String(text) => serde_json::from_str::<JsonValue>(text).ok(),
            other => Some(other.clone()),
        }
        .unwrap_or_else(|| JsonValue::Array(Vec::new()));
        for artifact in parsed.as_array().into_iter().flatten() {
            if let Some(blob_id) = artifact.get("blob_id").and_then(JsonValue::as_str) {
                if !blob_id.trim().is_empty() {
                    ids.insert(blob_id.to_string());
                }
            }
        }
    }
    ids.into_iter().collect()
}

pub(super) fn pack_rows(
    client: &mut pg::Client,
    table: &str,
    repo_id: &str,
    repo_name: &str,
) -> Result<Vec<JsonValue>, NativeRepositoryError> {
    query_json_object_rows(
        client,
        format!("select to_jsonb(t)::text as row_json from {table} t where repo_id = $1 or (repo_id is null and repo_name = $2) order by created_at asc, pack_id asc").as_str(),
        &[&repo_id, &repo_name],
    )
}

pub(super) fn delete_unreferenced_blobs(
    client: &mut pg::Client,
    blob_ids: &[String],
) -> Result<u64, NativeRepositoryError> {
    if blob_ids.is_empty() {
        return Ok(0);
    }
    client
        .execute(
            "delete from blobs b where b.blob_id = any($1) and not exists (select 1 from blob_locators l where l.blob_id = b.blob_id)",
            &[&blob_ids],
        )
        .map_err(db_internal)
}

pub(super) fn remove_pack_files_if_unreferenced(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    reference_table: &str,
    rows: &[JsonValue],
) -> Result<u64, NativeRepositoryError> {
    let mut removed = 0;
    for row in rows {
        let Some(pack_id) = row.get("pack_id").and_then(JsonValue::as_str) else {
            continue;
        };
        let Some(pack_path) = row.get("pack_path").and_then(JsonValue::as_str) else {
            continue;
        };
        if pack_path.trim().is_empty() {
            continue;
        }
        let remaining: i64 = client
            .query_one(
                format!(
                    "select count(*)::bigint as count from {reference_table} where pack_id = $1"
                )
                .as_str(),
                &[&pack_id],
            )
            .map_err(db_internal)?
            .get("count");
        if remaining != 0 {
            return Err(NativeRepositoryError::conflict(format!(
                "Repository cannot remove pack {pack_id}; {remaining} row(s) remain referenced outside the retired repository"
            )));
        }
        let path = runtime_storage_path(paths, pack_path);
        if path.exists() {
            fs::remove_file(&path).map_err(|exc| {
                NativeRepositoryError::internal(format!(
                    "failed to remove pack file `{}`: {exc}",
                    path.display()
                ))
            })?;
        }
        removed += 1;
    }
    Ok(removed)
}

pub(super) fn remove_tree_pack_files_if_unreferenced(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    rows: &[JsonValue],
) -> Result<u64, NativeRepositoryError> {
    let mut removed = 0;
    for row in rows {
        let Some(pack_id) = row.get("pack_id").and_then(JsonValue::as_str) else {
            continue;
        };
        let Some(pack_path) = row.get("pack_path").and_then(JsonValue::as_str) else {
            continue;
        };
        if pack_path.trim().is_empty() {
            continue;
        }
        let remaining: i64 = client
            .query_one(
                "select count(*)::bigint as count from snapshots where root_tree_pack_id = $1",
                &[&pack_id],
            )
            .map_err(db_internal)?
            .get("count");
        if remaining != 0 {
            return Err(NativeRepositoryError::conflict(format!(
                "Repository cannot remove tree pack {pack_id}; {remaining} snapshot(s) remain referenced outside the retired repository"
            )));
        }
        let path = runtime_storage_path(paths, pack_path);
        if path.exists() {
            fs::remove_file(&path).map_err(|exc| {
                NativeRepositoryError::internal(format!(
                    "failed to remove tree pack file `{}`: {exc}",
                    path.display()
                ))
            })?;
        }
        removed += 1;
    }
    Ok(removed)
}
