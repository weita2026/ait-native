use super::*;

pub(super) fn publish_artifact(
    source: &Path,
    target: &Path,
    repair_extension_install_name: bool,
) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");
    let temp_path = target.with_file_name(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        monotonic_nanos()
    ));
    let copy_result = (|| -> Result<(), String> {
        fs::copy(source, &temp_path).map_err(|err| {
            format!(
                "Failed to copy current-source artifact {} -> {}: {err}",
                source.display(),
                temp_path.display()
            )
        })?;
        let permissions = fs::metadata(source)
            .map_err(|err| format!("Failed to inspect {}: {err}", source.display()))?
            .permissions();
        fs::set_permissions(&temp_path, permissions).map_err(|err| {
            format!(
                "Failed to set permissions on {}: {err}",
                temp_path.display()
            )
        })?;
        if repair_extension_install_name {
            repair_local_extension_install_name(&temp_path);
        }
        fs::rename(&temp_path, target).map_err(|err| {
            format!(
                "Failed to publish current-source artifact {} -> {}: {err}",
                temp_path.display(),
                target.display()
            )
        })?;
        Ok(())
    })();
    if copy_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    copy_result
}

pub(super) fn ensure_local_extension_init(init_path: &Path) -> Result<(), String> {
    if fs::read_to_string(init_path).ok().as_deref() == Some(LOCAL_EXTENSION_INIT) {
        return Ok(());
    }
    if let Some(parent) = init_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    fs::write(init_path, LOCAL_EXTENSION_INIT)
        .map_err(|err| format!("Failed to write {}: {err}", init_path.display()))
}

pub(super) fn built_ait_cli_binary_path_with_artifact_store<S>(
    store: &S,
    target_dir: &Path,
) -> Option<PathBuf>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    first_existing_artifact_with_current_source_native_cache_artifact_store(
        store,
        ["release/ait-cli", "release/ait-cli.exe"]
            .iter()
            .map(|relative| target_dir.join(relative)),
    )
}

pub(super) fn built_ait_server_core_seam_binary_path_with_artifact_store<S>(
    store: &S,
    target_dir: &Path,
) -> Option<PathBuf>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    first_existing_artifact_with_current_source_native_cache_artifact_store(
        store,
        [
            "release/ait-server-core-seam",
            "release/ait-server-core-seam.exe",
        ]
        .iter()
        .map(|relative| target_dir.join(relative)),
    )
}

pub(super) fn repair_local_extension_install_name(extension_path: &Path) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(tool) = find_executable("install_name_tool") else {
        return;
    };
    let Some(name) = extension_path.file_name().and_then(|value| value.to_str()) else {
        return;
    };
    let _ = Command::new(tool)
        .arg("-id")
        .arg(format!("@rpath/{name}"))
        .arg(extension_path)
        .output();
}

pub(super) fn find_executable(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file() && is_executable(path))
}
