use super::*;
use crate::pack_substrate::PackCandidate;

mod manifest_assembly;

pub(crate) use self::manifest_assembly::*;

pub(crate) fn build_snapshot_object_pack_id(
    snapshot_id: &str,
    blob_items: &[PackCandidate],
) -> Result<String, String> {
    let mut blob_ids = blob_items
        .iter()
        .map(|row| row.blob_id.clone())
        .collect::<Vec<_>>();
    blob_ids.sort();
    let seed = format!("{snapshot_id}|{}", blob_ids.join("|"));
    Ok(format!(
        "PCK-{}",
        sha256_hex(seed.as_bytes())[..12].to_ascii_uppercase()
    ))
}
