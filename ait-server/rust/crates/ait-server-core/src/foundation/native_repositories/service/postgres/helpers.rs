use super::*;

pub(in crate::foundation::native_repositories) fn runtime_storage_path(
    paths: &ServerRuntimePaths,
    storage_path: &str,
) -> PathBuf {
    let path = PathBuf::from(storage_path);
    if path.is_absolute() {
        path
    } else {
        paths.root.join(path)
    }
}

pub(super) fn ensure_namespace_prefix_available(
    client: &mut pg::Client,
    repo_name: &str,
    namespace_prefix: &str,
) -> Result<(), NativeRepositoryError> {
    if namespace_prefix.is_empty() {
        return Ok(());
    }
    let row = client
        .query_opt(
            "select repo_name from repositories where id_namespace_prefix = $1 and repo_name <> $2 limit 1",
            &[&namespace_prefix, &repo_name],
        )
        .map_err(db_internal)?;
    if let Some(row) = row {
        let conflicting_repo_name: String = row.get("repo_name");
        return Err(NativeRepositoryError::conflict(format!(
            "Repository namespace prefix {namespace_prefix:?} is already in use by repository {conflicting_repo_name:?}."
        )));
    }
    Ok(())
}
