use super::*;

pub(super) fn set_repository_lifecycle_state(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    repo_id: &str,
    state: &str,
) -> Result<(), NativeRepositoryError> {
    let repo_name = repo_name.to_string();
    let repo_id = repo_id.to_string();
    let state = state.to_string();
    service.with_write(move |client| {
        let updated = client
            .execute(
                "update repositories set lifecycle_state = $1, updated_at = $2::text::timestamptz where repo_name = $3 and repo_id = $4",
                &[&state, &now_rfc3339(), &repo_name, &repo_id],
            )
            .map_err(db_internal)?;
        if updated == 0 {
            return Err(NativeRepositoryError::not_found(format!(
                "Unknown repository: {repo_name}"
            )));
        }
        Ok(())
    })
}

pub(super) fn insert_retirement_record(
    service: &PostgresNativeRepositoryService,
    retirement_id: &str,
    repo_name: &str,
    repo_id: &str,
    actor_identity: &str,
    actor_type: &str,
    export_path: &Path,
    manifest_path: &Path,
    manifest_sha256: &str,
    summary: &JsonValue,
) -> Result<(), NativeRepositoryError> {
    let summary_json = serde_json::to_string(summary).map_err(|exc| {
        NativeRepositoryError::internal(format!("failed to encode retirement summary: {exc}"))
    })?;
    service.with_control_write(|client| {
        ensure_retirement_schema(client)?;
        let now = now_rfc3339();
        client
            .execute(
                "insert into repository_retirements(retirement_id, repo_name, repo_id, state, actor_identity, actor_type, export_path, manifest_path, manifest_sha256, summary_json, created_at, exported_at, verified_at, purged_at, updated_at, last_error) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::text::timestamptz, $11::text::timestamptz, $11::text::timestamptz, null, $11::text::timestamptz, null)",
                &[&retirement_id, &repo_name, &repo_id, &RETIREMENT_STATE_EXPORTED, &actor_identity, &actor_type, &path_string(export_path), &path_string(manifest_path), &manifest_sha256, &summary_json, &now],
            )
            .map_err(db_internal)?;
        record_event(
            client,
            "repository.retire_exported",
            "repository",
            repo_name,
            &json!({
                "repo_id": repo_id,
                "retirement_id": retirement_id,
                "export_path": path_string(export_path),
                "manifest_path": path_string(manifest_path),
                "manifest_sha256": manifest_sha256
            }),
            actor_identity,
            actor_type,
        )?;
        Ok(())
    })
}

pub(super) fn update_retirement_record(
    service: &PostgresNativeRepositoryService,
    retirement_id: &str,
    state: &str,
    last_error: Option<&str>,
    summary_patch: Option<JsonValue>,
) -> Result<(), NativeRepositoryError> {
    service.with_control_write(|client| {
        ensure_retirement_schema(client)?;
        let row = client
            .query_opt(
                "select repo_name, repo_id, summary_json from repository_retirements where retirement_id = $1",
                &[&retirement_id],
            )
            .map_err(db_internal)?;
        let Some(row) = row else {
            return Ok(());
        };
        let repo_name: String = row.get("repo_name");
        let repo_id: String = row.get("repo_id");
        let summary_json: String = row.get("summary_json");
        let mut summary = serde_json::from_str::<JsonValue>(&summary_json)
            .unwrap_or_else(|_| json!({}));
        if let Some(patch) = summary_patch {
            merge_json_object(&mut summary, patch);
        }
        let summary_text = serde_json::to_string(&summary).map_err(|exc| {
            NativeRepositoryError::internal(format!("failed to encode retirement summary: {exc}"))
        })?;
        let now = now_rfc3339();
        let purged_at: Option<String> = (state == RETIREMENT_STATE_PURGED).then(|| now.clone());
        client
            .execute(
                "update repository_retirements set state = $1, summary_json = $2, purged_at = coalesce($3::text::timestamptz, purged_at), updated_at = $4::text::timestamptz, last_error = $5 where retirement_id = $6",
                &[&state, &summary_text, &purged_at, &now, &last_error, &retirement_id],
            )
            .map_err(db_internal)?;
        let event_type = if state == RETIREMENT_STATE_PURGED {
            "repository.retired"
        } else {
            "repository.retire_failed"
        };
        record_event(
            client,
            event_type,
            "repository",
            &repo_name,
            &json!({
                "repo_id": repo_id,
                "retirement_id": retirement_id,
                "state": state,
                "last_error": last_error
            }),
            "system",
            "system_worker",
        )?;
        Ok(())
    })
}

pub(super) fn ensure_retirement_schema(
    client: &mut pg::Client,
) -> Result<(), NativeRepositoryError> {
    client
        .batch_execute(
            r#"
            create table if not exists repository_retirements(
                retirement_id text primary key,
                repo_name text not null,
                repo_id text not null,
                state text not null,
                actor_identity text not null,
                actor_type text not null,
                export_path text not null,
                manifest_path text not null,
                manifest_sha256 text not null,
                summary_json text not null default '{}',
                created_at timestamptz not null,
                exported_at timestamptz,
                verified_at timestamptz,
                purged_at timestamptz,
                updated_at timestamptz not null,
                last_error text
            );
            create table if not exists events(
                event_id bigserial primary key,
                event_type text not null,
                entity_type text not null,
                entity_id text not null,
                payload_json text not null default '{}',
                actor_identity text not null default 'system',
                actor_type text not null default 'system_worker',
                created_at timestamptz not null
            );
            "#,
        )
        .map_err(db_internal)
}

pub(super) fn record_event(
    client: &mut pg::Client,
    event_type: &str,
    entity_type: &str,
    entity_id: &str,
    payload: &JsonValue,
    actor_identity: &str,
    actor_type: &str,
) -> Result<(), NativeRepositoryError> {
    let payload_json = serde_json::to_string(payload).map_err(|exc| {
        NativeRepositoryError::internal(format!("failed to encode event payload: {exc}"))
    })?;
    client
        .execute(
            "insert into events(event_type, entity_type, entity_id, payload_json, actor_identity, actor_type, created_at) values ($1, $2, $3, $4, $5, $6, $7::text::timestamptz)",
            &[&event_type, &entity_type, &entity_id, &payload_json, &actor_identity, &actor_type, &now_rfc3339()],
        )
        .map_err(db_internal)?;
    Ok(())
}
