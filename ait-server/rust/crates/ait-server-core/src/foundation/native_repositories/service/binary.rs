use super::*;

#[path = "binary/line_snapshot.rs"]
mod line_snapshot;
#[path = "binary/repository.rs"]
mod repository;
#[path = "binary/serialization.rs"]
mod serialization;
#[path = "binary/snapshot_export.rs"]
mod snapshot_export;

use line_snapshot::*;
use serialization::*;
use snapshot_export::*;

pub(in crate::foundation::native_repositories) use serialization::{
    binary_created_at_value, binary_json_text, binary_snapshot_id,
};
#[derive(Clone)]
pub struct BinaryDbNativeRepositoryService<D>
where
    D: ServerRemoteBinaryDb,
{
    db: D,
    default_line: String,
    id_namespace_prefix: String,
    created_at: String,
}
