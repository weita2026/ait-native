use super::*;

pub(super) fn write_downloaded_object_pack(
    ctx: &RemoteSyncLocalStoreContext,
    pack: &ZstdBulkObjectPackRow,
    bytes: &[u8],
) -> Result<(), String> {
    let pack_rel_path = default_object_pack_relative_path(&pack.pack_id);
    let pack_abs_path = repo_stored_path(ctx, &pack_rel_path);
    if pack_abs_path.exists() {
        validate_object_pack_file_matches_manifest(&pack_abs_path, pack)?;
        return Ok(());
    }
    write_pack_bytes_atomically(&pack_abs_path, bytes)?;
    validate_object_pack_file_matches_manifest(&pack_abs_path, pack)
}

pub(super) fn write_downloaded_tree_pack(
    ctx: &RemoteSyncLocalStoreContext,
    pack: &ZstdBulkTreePackRow,
    bytes: &[u8],
) -> Result<(), String> {
    let pack_rel_path = default_tree_pack_relative_path(&pack.pack_id);
    let pack_abs_path = repo_stored_path(ctx, &pack_rel_path);
    if pack_abs_path.exists() {
        validate_tree_pack_file_matches_manifest(&pack_abs_path, pack)?;
        return Ok(());
    }
    write_pack_bytes_atomically(&pack_abs_path, bytes)?;
    validate_tree_pack_file_matches_manifest(&pack_abs_path, pack)
}

pub(super) fn write_pack_bytes_atomically(
    pack_abs_path: &Path,
    bytes: &[u8],
) -> Result<(), String> {
    if let Some(parent) = pack_abs_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create zstd pack parent directory: {err}"))?;
    }
    let tmp_path = pack_abs_path.with_extension("zstd-download.tmp");
    fs::write(&tmp_path, bytes)
        .map_err(|err| format!("failed to write downloaded zstd pack: {err}"))?;
    fs::rename(&tmp_path, pack_abs_path)
        .map_err(|err| format!("failed to install downloaded zstd pack: {err}"))?;
    Ok(())
}
