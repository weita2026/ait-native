use super::*;

pub(super) fn write_json_file(
    path: &Path,
    payload: &JsonValue,
) -> Result<WrittenFile, NativeRepositoryError> {
    let bytes = serde_json::to_vec_pretty(payload)
        .map_err(|exc| NativeRepositoryError::internal(format!("failed to encode JSON: {exc}")))?;
    let mut bytes_with_newline = bytes;
    bytes_with_newline.push(b'\n');
    write_bytes_file(path, &bytes_with_newline)
}

pub(super) fn write_bytes_file(
    path: &Path,
    bytes: &[u8],
) -> Result<WrittenFile, NativeRepositoryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            NativeRepositoryError::internal(format!(
                "failed to create export directory `{}`: {exc}",
                parent.display()
            ))
        })?;
    }
    let mut file = fs::File::create(path).map_err(|exc| {
        NativeRepositoryError::internal(format!("failed to create `{}`: {exc}", path.display()))
    })?;
    file.write_all(bytes).map_err(|exc| {
        NativeRepositoryError::internal(format!("failed to write `{}`: {exc}", path.display()))
    })?;
    Ok(WrittenFile {
        path: path.to_path_buf(),
        sha256: sha256_hex(bytes),
        size_bytes: bytes.len() as u64,
    })
}

pub(super) fn sha256_path(path: &Path) -> Result<String, NativeRepositoryError> {
    let bytes = fs::read(path).map_err(|exc| {
        NativeRepositoryError::internal(format!("failed to read `{}`: {exc}", path.display()))
    })?;
    Ok(sha256_hex(&bytes))
}

pub(super) fn relative_path(root: &Path, path: &Path) -> Result<String, NativeRepositoryError> {
    path.strip_prefix(root)
        .map_err(|exc| {
            NativeRepositoryError::internal(format!(
                "failed to relativize `{}` against `{}`: {exc}",
                path.display(),
                root.display()
            ))
        })
        .and_then(path_to_string)
}
