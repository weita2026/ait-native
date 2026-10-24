use super::*;

pub fn zip_archive_has_member(
    path_value: &str,
    entry_name: &str,
) -> Result<bool, PlanFilesystemError> {
    zip_archive_has_member_with_file_io_store(&FilesystemFileIoStore, path_value, entry_name)
}

pub fn zip_archive_has_member_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    entry_name: &str,
) -> Result<bool, PlanFilesystemError>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_file_io_store(store, path_value);
    let bytes = store
        .read_bytes(&path)
        .map_err(|err| file_io_error_for_path("open zip archive", &path, err))?;
    let file = Cursor::new(bytes);
    let mut archive = ZipArchive::new(file).map_err(|_| {
        PlanFilesystemError::Invalid(format!(
            "Archive is not a valid ZIP file: {}",
            path.display()
        ))
    })?;
    let exists = archive.by_name(entry_name).is_ok();
    Ok(exists)
}

pub fn read_zip_archive_member(
    path_value: &str,
    entry_name: &str,
) -> Result<Vec<u8>, PlanFilesystemError> {
    read_zip_archive_member_with_file_io_store(&FilesystemFileIoStore, path_value, entry_name)
}

pub fn read_zip_archive_member_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    entry_name: &str,
) -> Result<Vec<u8>, PlanFilesystemError>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_file_io_store(store, path_value);
    let bytes = store
        .read_bytes(&path)
        .map_err(|err| file_io_error_for_path("open zip archive", &path, err))?;
    let file = Cursor::new(bytes);
    let mut archive = ZipArchive::new(file).map_err(|_| {
        PlanFilesystemError::Invalid(format!(
            "Archive is not a valid ZIP file: {}",
            path.display()
        ))
    })?;
    let mut member = archive.by_name(entry_name).map_err(|_| {
        PlanFilesystemError::MissingEntry(format!("Archive member not found: {}", entry_name))
    })?;
    let mut output = Vec::new();
    member.read_to_end(&mut output).map_err(|err| {
        PlanFilesystemError::Io(format!(
            "Failed to read archive member {} from {}: {}",
            entry_name,
            path.display(),
            err
        ))
    })?;
    Ok(output)
}
