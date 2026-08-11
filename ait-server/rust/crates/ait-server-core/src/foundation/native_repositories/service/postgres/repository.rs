use super::*;

pub(super) fn create_or_update_repository(
    client: &mut pg::Client,
    request: RepositoryCreateRequest,
) -> Result<JsonValue, NativeRepositoryError> {
    create_or_update_repository_inner(client, request, true, true)
}

pub(super) fn create_or_update_repository_metadata(
    client: &mut pg::Client,
    request: RepositoryCreateRequest,
) -> Result<JsonValue, NativeRepositoryError> {
    create_or_update_repository_inner(client, request, false, false)
}

fn create_or_update_repository_inner(
    client: &mut pg::Client,
    request: RepositoryCreateRequest,
    create_default_line: bool,
    include_content_pack_inventory: bool,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo_name = normalize_required_text(&request.repo_name, "repo_name")?;
    let default_line = normalize_required_text(&request.default_line, "default_line")?;
    let policy = normalize_policy_json(&request.policy)?;
    let namespace_prefix = normalize_namespace_prefix(request.id_namespace_prefix)?;

    if let Some(existing) = select_repository_row(client, &repo_name)? {
        let mut changed = false;
        let current_policy = parse_policy_json(existing.policy_json.as_str());
        if current_policy != policy {
            changed = true;
            client
                .execute(
                    "update repositories set policy_json = $1, updated_at = $2::text::timestamptz where repo_name = $3",
                    &[&serde_json::to_string(&policy).unwrap_or_else(|_| "{}".to_string()), &now_rfc3339(), &repo_name],
                )
                .map_err(db_internal)?;
        }
        if existing.id_namespace_prefix != namespace_prefix {
            ensure_namespace_prefix_available(client, &repo_name, &namespace_prefix)?;
            changed = true;
            client
                .execute(
                    "update repositories set id_namespace_prefix = $1, updated_at = $2::text::timestamptz where repo_name = $3",
                    &[&namespace_prefix, &now_rfc3339(), &repo_name],
                )
                .map_err(db_internal)?;
        }
        if changed {
            let updated = select_repository_row(client, &repo_name)?.ok_or_else(|| {
                NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
            })?;
            return repository_payload(client, updated, include_content_pack_inventory);
        }
        return repository_payload(client, existing, include_content_pack_inventory);
    }

    ensure_namespace_prefix_available(client, &repo_name, &namespace_prefix)?;
    let repo_id = new_identifier("REPO", format!("{repo_name}|{}", now_rfc3339()).as_str());
    let created_at = now_rfc3339();
    let policy_text = serde_json::to_string(&policy).unwrap_or_else(|_| "{}".to_string());
    client
        .execute(
            "insert into repositories(repo_name, repo_id, default_line, lifecycle_state, id_namespace_prefix, policy_json, created_at, updated_at) values ($1, $2, $3, 'active', $4, $5, $6::text::timestamptz, $7::text::timestamptz)",
            &[&repo_name, &repo_id, &default_line, &namespace_prefix, &policy_text, &created_at, &created_at],
        )
        .map_err(db_internal)?;
    if create_default_line {
        client
            .execute(
                "insert into lines(repo_name, repo_id, line_name, head_snapshot_id, status, archived_at, created_at, updated_at) values ($1, $2, $3, null, 'active', null, $4::text::timestamptz, $5::text::timestamptz)",
                &[&repo_name, &repo_id, &default_line, &created_at, &created_at],
            )
            .map_err(db_internal)?;
    }
    let created = select_repository_row(client, &repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    repository_payload(client, created, include_content_pack_inventory)
}

fn repository_payload(
    client: &mut pg::Client,
    row: RepositoryRow,
    include_content_pack_inventory: bool,
) -> Result<JsonValue, NativeRepositoryError> {
    if include_content_pack_inventory {
        repository_json_with_pack_storage(client, row)
    } else {
        Ok(repository_json(row))
    }
}

pub(super) fn get_repository_json(
    client: &mut pg::Client,
    repo_name: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let row = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    repository_json_with_pack_storage(client, row)
}

pub(super) fn get_repository_metadata_json(
    client: &mut pg::Client,
    repo_name: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let row = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    Ok(repository_json(row))
}

pub(super) fn get_repository_json_by_id(
    client: &mut pg::Client,
    repo_id: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let row = select_repository_row_by_id(client, repo_id)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository id: {repo_id}"))
    })?;
    repository_json_with_pack_storage(client, row)
}

pub(super) fn get_repository_metadata_json_by_id(
    client: &mut pg::Client,
    repo_id: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let row = select_repository_row_by_id(client, repo_id)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository id: {repo_id}"))
    })?;
    Ok(repository_json(row))
}

pub(super) fn list_repositories_json(
    client: &mut pg::Client,
) -> Result<JsonValue, NativeRepositoryError> {
    let rows = client
        .query(
            "select repo_name, repo_id, default_line, lifecycle_state, id_namespace_prefix, policy_json, created_at::text as created_at_text, updated_at::text as updated_at_text from repositories order by repo_name asc",
            &[],
        )
        .map_err(db_internal)?;
    let repositories = rows
        .into_iter()
        .map(repository_row_from_db)
        .map(|row| repository_json_with_pack_storage(client, row))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(JsonValue::Array(repositories))
}

pub(super) fn list_repository_metadata_json(
    client: &mut pg::Client,
) -> Result<JsonValue, NativeRepositoryError> {
    let rows = client
        .query(
            "select repo_name, repo_id, default_line, lifecycle_state, id_namespace_prefix, policy_json, created_at::text as created_at_text, updated_at::text as updated_at_text from repositories order by repo_name asc",
            &[],
        )
        .map_err(db_internal)?;
    Ok(JsonValue::Array(
        rows.into_iter()
            .map(repository_row_from_db)
            .map(repository_json)
            .collect(),
    ))
}

pub(in crate::foundation::native_repositories) fn select_repository_row(
    client: &mut pg::Client,
    repo_name: &str,
) -> Result<Option<RepositoryRow>, NativeRepositoryError> {
    client
        .query_opt(
            "select repo_name, repo_id, default_line, lifecycle_state, id_namespace_prefix, policy_json, created_at::text as created_at_text, updated_at::text as updated_at_text from repositories where repo_name = $1",
            &[&repo_name],
        )
        .map_err(db_internal)
        .map(|row| row.map(repository_row_from_db))
}

pub(in crate::foundation::native_repositories) fn select_repository_row_by_id(
    client: &mut pg::Client,
    repo_id: &str,
) -> Result<Option<RepositoryRow>, NativeRepositoryError> {
    client
        .query_opt(
            "select repo_name, repo_id, default_line, lifecycle_state, id_namespace_prefix, policy_json, created_at::text as created_at_text, updated_at::text as updated_at_text from repositories where repo_id = $1",
            &[&repo_id],
        )
        .map_err(db_internal)
        .map(|row| row.map(repository_row_from_db))
}
