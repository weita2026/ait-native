use super::*;

pub(super) fn preflight_blockers(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    repo_id: &str,
) -> Result<JsonMap<String, JsonValue>, NativeRepositoryError> {
    let repo_name = repo_name.to_string();
    let repo_id = repo_id.to_string();
    let control_repo_name = repo_name.clone();
    let control_repo_id = repo_id.clone();
    let mut blockers = service.with_control_read(move |client| {
        let mut blockers = JsonMap::new();
        let active_jobs = query_json_object_rows(
            client,
            "select json_build_object('job_id', job_id, 'job_type', job_type, 'state', state)::text as row_json from jobs where (repo_id = $1 or (repo_id is null and repo_name = $2)) and state in ('queued', 'running') order by job_id asc limit 25",
            &[&control_repo_id, &control_repo_name],
        )?;
        insert_non_empty(&mut blockers, "active_jobs", active_jobs);
        let active_tasks = query_json_object_rows(
            client,
            "select json_build_object('task_id', task_id, 'status', status)::text as row_json from tasks where (repo_id = $1 or (repo_id is null and repo_name = $2)) and status = 'active' order by created_at asc, task_id asc limit 25",
            &[&control_repo_id, &control_repo_name],
        )?;
        insert_non_empty(&mut blockers, "active_tasks", active_tasks);
        let open_changes = query_json_object_rows(
            client,
            "select json_build_object('change_id', change_id, 'status', status)::text as row_json from changes where (repo_id = $1 or (repo_id is null and repo_name = $2)) and status not in ('archived', 'landed', 'superseded') order by updated_at asc, change_id asc limit 25",
            &[&control_repo_id, &control_repo_name],
        )?;
        insert_non_empty(&mut blockers, "open_changes", open_changes);
        let pending_lands = query_json_object_rows(
            client,
            "select json_build_object('submission_id', submission_id, 'status', status, 'target_line', target_line)::text as row_json from land_requests where repo_id = $1 and status in ('queued', 'running') order by created_at asc, submission_id asc limit 25",
            &[&control_repo_id],
        )?;
        insert_non_empty(&mut blockers, "pending_lands", pending_lands);
        Ok(blockers)
    })?;
    let shared_pack_refs = cross_repo_pack_refs(service, &repo_name, &repo_id)?;
    insert_non_empty(&mut blockers, "shared_pack_refs", shared_pack_refs);
    Ok(blockers)
}

pub(super) fn insert_non_empty(
    map: &mut JsonMap<String, JsonValue>,
    key: &str,
    rows: Vec<JsonValue>,
) {
    if !rows.is_empty() {
        map.insert(key.to_string(), JsonValue::Array(rows));
    }
}

pub(super) fn cross_repo_pack_refs(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    repo_id: &str,
) -> Result<Vec<JsonValue>, NativeRepositoryError> {
    let target_repo_name = repo_name.to_string();
    let target_repo_id = repo_id.to_string();
    let query_repo_name = target_repo_name.clone();
    let query_repo_id = target_repo_id.clone();
    let paths = service.paths.clone();
    let (mut content_collisions, target_blob_ids) = service.with_read(move |client| {
        let pack_ids = text_column(
            client,
            "select pack_id from packs where repo_id = $1 or (repo_id is null and repo_name = $2) order by pack_id asc",
            &[&query_repo_id, &query_repo_name],
            "pack_id",
        )?;
        if pack_ids.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let target_blob_ids = repo_blob_ids(client, &query_repo_name, &query_repo_id, &pack_ids)?;
        if target_blob_ids.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut collisions = Vec::<JsonValue>::new();
        append_other_blob_locator_owners(
            client,
            &mut collisions,
            &target_blob_ids,
            &query_repo_name,
            &query_repo_id,
        )?;
        append_snapshot_tree_pack_owners(
            client,
            &paths,
            &mut collisions,
            &target_blob_ids,
            &query_repo_name,
            &query_repo_id,
        )?;
        append_delta_base_refs(client, &mut collisions, &target_blob_ids, &pack_ids)?;
        Ok((collisions, target_blob_ids))
    })?;
    let control_collisions = control_blob_owners(service, &target_repo_name, &target_blob_ids)?;
    content_collisions.extend(control_collisions);
    Ok(content_collisions
        .into_iter()
        .take(BLOB_REFERENCE_SAMPLE_LIMIT)
        .collect())
}

pub(super) fn repo_blob_ids(
    client: &mut pg::Client,
    repo_name: &str,
    repo_id: &str,
    pack_ids: &[String],
) -> Result<Vec<String>, NativeRepositoryError> {
    let mut ids = text_column(
        client,
        "select blob_id from blob_locators where repo_id = $1 or (repo_id is null and repo_name = $2) order by blob_id asc",
        &[&repo_id, &repo_name],
        "blob_id",
    )?;
    let mut packed = text_column(
        client,
        "select blob_id from blobs where pack_id = any($1) order by blob_id asc",
        &[&pack_ids],
        "blob_id",
    )?;
    ids.append(&mut packed);
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub(super) fn append_other_blob_locator_owners(
    client: &mut pg::Client,
    collisions: &mut Vec<JsonValue>,
    target_blob_ids: &[String],
    repo_name: &str,
    repo_id: &str,
) -> Result<(), NativeRepositoryError> {
    for row in client
        .query(
            "select blob_id, array_agg(distinct repo_name order by repo_name) as owner_repo_names from blob_locators where blob_id = any($1) and not (repo_id = $2 or (repo_id is null and repo_name = $3)) group by blob_id order by blob_id asc limit 25",
            &[&target_blob_ids, &repo_id, &repo_name],
        )
        .map_err(db_internal)?
    {
        let blob_id: String = row.get("blob_id");
        let owners: Vec<String> = row.get("owner_repo_names");
        collisions.push(json!({
            "blob_id": blob_id,
            "other_repo_names": owners,
            "source": "blob_locators"
        }));
    }
    Ok(())
}

pub(super) fn append_snapshot_tree_pack_owners(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    collisions: &mut Vec<JsonValue>,
    target_blob_ids: &[String],
    repo_name: &str,
    repo_id: &str,
) -> Result<(), NativeRepositoryError> {
    let target_value = JsonValue::Array(
        target_blob_ids
            .iter()
            .cloned()
            .map(JsonValue::String)
            .collect(),
    );
    let rows = client
        .query(
            "select distinct s.repo_name, tp.pack_path, tp.pack_format from snapshots s join tree_packs tp on tp.pack_id = s.root_tree_pack_id where not (s.repo_id = $1 or (s.repo_id is null and s.repo_name = $2)) and coalesce(tp.pack_path, '') <> '' order by s.repo_name asc, tp.pack_path asc limit 100",
            &[&repo_id, &repo_name],
        )
        .map_err(db_internal)?;
    for row in rows {
        let owner_repo_name: String = row.get("repo_name");
        let pack_path: String = row.get("pack_path");
        let pack_format: String = row.get("pack_format");
        let path = runtime_storage_path(paths, &pack_path);
        let hits = tree_pack_contains_blob_ids_with_format(
            path_to_string(&path)?.as_str(),
            &target_value,
            &pack_format,
        )
        .map_err(NativeRepositoryError::internal)?;
        let Some(object) = hits.as_object() else {
            continue;
        };
        for blob_id in target_blob_ids {
            if object
                .get(blob_id)
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
            {
                collisions.push(json!({
                    "blob_id": blob_id,
                    "other_repo_names": [owner_repo_name.clone()],
                    "source": "snapshot_tree_pack"
                }));
            }
        }
    }
    Ok(())
}

pub(super) fn append_delta_base_refs(
    client: &mut pg::Client,
    collisions: &mut Vec<JsonValue>,
    target_blob_ids: &[String],
    pack_ids: &[String],
) -> Result<(), NativeRepositoryError> {
    for row in client
        .query(
            "select pack_base_blob_id, array_agg(blob_id order by blob_id) as dependent_blob_ids from blobs where pack_base_blob_id = any($1) and not (pack_id = any($2)) group by pack_base_blob_id order by pack_base_blob_id asc limit 25",
            &[&target_blob_ids, &pack_ids],
        )
        .map_err(db_internal)?
    {
        let blob_id: String = row.get("pack_base_blob_id");
        let dependent_blob_ids: Vec<String> = row.get("dependent_blob_ids");
        collisions.push(json!({
            "blob_id": blob_id,
            "dependent_blob_ids": dependent_blob_ids,
            "source": "delta_base"
        }));
    }
    Ok(())
}

pub(super) fn control_blob_owners(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    target_blob_ids: &[String],
) -> Result<Vec<JsonValue>, NativeRepositoryError> {
    if target_blob_ids.is_empty() {
        return Ok(Vec::new());
    }
    let target_blob_ids = target_blob_ids.to_vec();
    let target_repo_name = repo_name.to_string();
    service.with_control_read(move |client| {
        let mut collisions = Vec::new();
        for table in ["plan_revision_blobs", "plan_revision_artifacts"] {
            for row in client
                .query(
                    format!("select blob_id, array_agg(distinct repo_name order by repo_name) as owner_repo_names from {table} where blob_id = any($1) and repo_name <> $2 group by blob_id order by blob_id asc limit 25").as_str(),
                    &[&target_blob_ids, &target_repo_name],
                )
                .map_err(db_internal)?
            {
                let blob_id: String = row.get("blob_id");
                let owners: Vec<String> = row.get("owner_repo_names");
                if owners.is_empty() {
                    continue;
                }
                collisions.push(json!({
                    "blob_id": blob_id,
                    "other_repo_names": owners,
                    "source": table
                }));
            }
        }
        for row in client
            .query(
                "select repo_name, artifacts_json::text as artifacts_json from releases where repo_name <> $1 order by repo_name asc",
                &[&target_repo_name],
            )
            .map_err(db_internal)?
        {
            let repo_name: String = row.get("repo_name");
            let artifacts_json: String = row.get("artifacts_json");
            let artifacts = serde_json::from_str::<JsonValue>(&artifacts_json)
                .unwrap_or_else(|_| JsonValue::Array(Vec::new()));
            let Some(items) = artifacts.as_array() else {
                continue;
            };
            for item in items {
                let Some(blob_id) = item.get("blob_id").and_then(JsonValue::as_str) else {
                    continue;
                };
                if target_blob_ids.iter().any(|target| target == blob_id) {
                    collisions.push(json!({
                        "blob_id": blob_id,
                        "other_repo_names": [repo_name.clone()],
                        "source": "releases"
                    }));
                }
            }
        }
        Ok(collisions)
    })
}
