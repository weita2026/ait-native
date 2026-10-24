use super::{GenerationResult, Path};
use serde::Serialize;
use std::fs;
use std::io::Write;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GenerationFileManifest {
    pub relative_path: String,
    pub byte_size: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_count: Option<u64>,
}

pub(super) fn write_json<T: Serialize>(path: &Path, value: &T) -> GenerationResult<()> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to encode generation manifest: {error}"))?;
    bytes.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("failed to create manifest {}: {error}", path.display()))?;
    file.write_all(&bytes)
        .map_err(|error| format!("failed to write manifest {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync manifest {}: {error}", path.display()))
}
