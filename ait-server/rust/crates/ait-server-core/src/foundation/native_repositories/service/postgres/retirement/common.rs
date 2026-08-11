use super::*;

pub(super) fn query_json_object_rows(
    client: &mut pg::Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<JsonValue>, NativeRepositoryError> {
    client
        .query(sql, params)
        .map_err(db_internal)?
        .into_iter()
        .map(|row| {
            let text: String = row.get("row_json");
            serde_json::from_str::<JsonValue>(&text).map_err(|exc| {
                NativeRepositoryError::internal(format!("database row JSON is invalid: {exc}"))
            })
        })
        .collect()
}

pub(super) fn text_column(
    client: &mut pg::Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
    column: &str,
) -> Result<Vec<String>, NativeRepositoryError> {
    Ok(client
        .query(sql, params)
        .map_err(db_internal)?
        .into_iter()
        .map(|row| row.get::<_, String>(column))
        .collect())
}

pub(super) fn merge_json_object(target: &mut JsonValue, patch: JsonValue) {
    let target_object = target.as_object_mut();
    let patch_object = patch.as_object();
    if let (Some(target_object), Some(patch_object)) = (target_object, patch_object) {
        for (key, value) in patch_object {
            target_object.insert(key.clone(), value.clone());
        }
    }
}
