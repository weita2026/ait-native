use super::*;

pub fn read_utf8_text_file(path_value: &str) -> Result<String, PlanFilesystemError> {
    read_utf8_text_file_with_file_io_store(&FilesystemFileIoStore, path_value)
}

pub fn read_utf8_text_file_with_file_io_store<S>(
    store: &S,
    path_value: &str,
) -> Result<String, PlanFilesystemError>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_file_io_store(store, path_value);
    let bytes = store
        .read_bytes(&path)
        .map_err(|err| file_io_error_for_path("read text file", &path, err))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        PlanFilesystemError::Invalid(format!("File is not valid UTF-8: {}", path.display()))
    })?;
    Ok(normalize_text_newlines(&text))
}

pub(super) fn normalize_text_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn read_json_file(path_value: &str) -> Result<JsonValue, PlanFilesystemError> {
    read_json_file_with_file_io_store(&FilesystemFileIoStore, path_value)
}

pub fn read_json_file_with_file_io_store<S>(
    store: &S,
    path_value: &str,
) -> Result<JsonValue, PlanFilesystemError>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_file_io_store(store, path_value);
    let text = read_utf8_text_file_with_file_io_store(store, path_value)?;
    JsonCodec::parse_value(&text, "artifact file").map_err(|_| {
        PlanFilesystemError::Invalid(format!("File is not valid JSON: {}", path.display()))
    })
}

pub fn read_binary_file(path_value: &str) -> Result<Vec<u8>, PlanFilesystemError> {
    read_binary_file_with_file_io_store(&FilesystemFileIoStore, path_value)
}

pub fn read_binary_file_with_file_io_store<S>(
    store: &S,
    path_value: &str,
) -> Result<Vec<u8>, PlanFilesystemError>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_file_io_store(store, path_value);
    store
        .read_bytes(&path)
        .map_err(|err| file_io_error_for_path("read binary file", &path, err))
}
