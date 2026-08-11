use super::*;

pub(super) fn materialize_workspace(config: &PatchsetCiRuntimeConfig) -> Result<(), String> {
    fs::create_dir_all(&config.workspace_path).map_err(|exc| {
        format!(
            "Failed to create patchset CI workspace `{}`: {exc}",
            path_string(&config.workspace_path)
        )
    })?;
    for file in &config.materialized_files {
        if path_has_parent_escape(&file.path) || file.path.is_absolute() {
            return Err(
                "materialized file paths must be relative and stay inside workspace.".to_string(),
            );
        }
        let path = config.workspace_path.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|exc| {
                format!(
                    "Failed to create materialized file parent `{}`: {exc}",
                    path_string(parent)
                )
            })?;
        }
        fs::write(&path, &file.content)
            .map_err(|exc| format!("Failed to materialize `{}`: {exc}", path_string(&path)))?;
        #[cfg(unix)]
        if let Some(mode) = file.mode {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).map_err(|exc| {
                format!(
                    "Failed to set mode on materialized file `{}`: {exc}",
                    path_string(&path)
                )
            })?;
        }
    }
    Ok(())
}
