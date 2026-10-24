//! Standalone Plan sync opens its own unchanged Local Plan/content formats.
//! Workflow admission, conversion and workflow file access are not capabilities
//! of this adapter. Composite Task and history publication keep full admission.
use super::PlanSyncStorageRequest;
use crate::binary_db::{
    AuthorityId, BinaryDbReadLockSet, BinaryDbReadScope, LocalBinaryDbFs, StorePath,
};
use crate::binary_db_generation::{
    binary_db_activation_lock_root, configured_repo_name, require_direct_authority_directory,
};
use crate::content_binary_db::{
    BLOB_RECORD_SIZE, OBJECT_PACK_MEMBER_RECORD_SIZE, OBJECT_PACK_RECORD_SIZE,
    TREE_PACK_RECORD_SIZE, TREE_RECORD_SIZE,
};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const BIN_PATHS: &[&str] = &[
    "plan.bin",
    "plan_payload.bin",
    "plan_revision.bin",
    "plan_revision_payload.bin",
    "plan_item.bin",
    "plan_item_payload.bin",
    "blob.bin",
    "object_pack.bin",
    "object_pack_member.bin",
    "tree_pack.bin",
    "tree.bin",
];
const INDEX_PATHS: &[&str] = &[
    "blob_id.idx",
    "object_pack_id.idx",
    "tree_id.idx",
    "tree_pack_id.idx",
];

pub(super) fn open(
    request: &PlanSyncStorageRequest,
    repo_name: &str,
) -> Result<(LocalBinaryDbFs, PathBuf), String> {
    let root = Path::new(
        request
            .repo_root
            .as_deref()
            .ok_or("Missing Plan repo_root")?,
    )
    .canonicalize()
    .map_err(|error| error.to_string())?;
    let authority = root.join(".ait/binary-db");
    if request.authority_root.as_deref() != authority.to_str()
        || request.activation_pointer.as_deref() != authority.to_str()
        || request.pack_root.as_deref() != root.to_str()
        || request.local_authority_id.as_deref() != Some(&format!("local:{repo_name}"))
    {
        return Err(
            "Standalone Plan storage must select its canonical repository authority and pack root"
                .into(),
        );
    }
    let guard = BinaryDbReadLockSet::try_acquire(&binary_db_activation_lock_root(&root))
        .map_err(|error| format!("Binary DB generation activation is active: {error}"))?;
    require_direct_authority_directory(&authority)?;
    if authority
        .canonicalize()
        .map_err(|error| error.to_string())?
        != authority
    {
        return Err("Standalone Plan storage requires a direct canonical .ait directory".into());
    }
    if configured_repo_name(&root)? != repo_name {
        return Err("Standalone Plan storage repository mismatch".into());
    }
    let packs =
        fs::symlink_metadata(root.join(".ait/objects")).map_err(|error| error.to_string())?;
    if !packs.file_type().is_dir() {
        return Err("Repository content authority must be a direct directory".into());
    }
    {
        let _read = BinaryDbReadLockSet::try_acquire_for_scope(
            &StorePath::from(authority.clone()),
            BinaryDbReadScope::ContentAndPlan,
        )
        .map_err(|error| error.to_string())?;
        for leaf in BIN_PATHS.iter().chain(INDEX_PATHS) {
            // Existing Plan recovery owns incomplete, uncommitted dependency
            // tails. Do not reject those before it can preserve committed roots.
            validate_file(
                &authority.join(leaf),
                record_size(leaf),
                leaf.starts_with("plan"),
            )?;
        }
    }
    crate::plan_binary_db::repair_plan_binary_db_authority_if_needed(&authority)?;
    let db = LocalBinaryDbFs::new(
        StorePath::from(authority),
        StorePath::from(root.clone()),
        AuthorityId::new(format!("local:{repo_name}")),
        request
            .current_line_state_scope
            .ok_or("Missing Plan local state scope")?,
    )
    .with_declared_bin_paths(BIN_PATHS)
    .with_declared_index_paths(INDEX_PATHS)
    .with_generation_guard(Some(Arc::new(Mutex::new(guard))));
    Ok((db, root))
}

fn record_size(leaf: &str) -> Option<u32> {
    match leaf {
        "blob.bin" => Some(BLOB_RECORD_SIZE),
        "object_pack.bin" => Some(OBJECT_PACK_RECORD_SIZE),
        "object_pack_member.bin" => Some(OBJECT_PACK_MEMBER_RECORD_SIZE),
        "tree_pack.bin" => Some(TREE_PACK_RECORD_SIZE),
        "tree.bin" => Some(TREE_RECORD_SIZE),
        _ => None,
    }
}

fn validate_file(path: &Path, width: Option<u32>, plan_recovery: bool) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.file_type().is_file() {
        return Err(format!(
            "Plan/content file {} must be a direct file",
            path.display()
        ));
    }
    if plan_recovery && metadata.len() < 4 {
        // Recovery removes an incomplete uncommitted dependency, and rejects
        // any missing bytes still referenced by a committed Plan root.
        return Ok(());
    }
    let mut header = [0; 4];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| {
            format!(
                "Cannot read Plan/content header {}: {error}",
                path.display()
            )
        })?;
    if u32::from_le_bytes(header) != 1 {
        return Err(format!(
            "Unsupported Plan/content layout in {}: expected 1",
            path.display()
        ));
    }
    if width.is_some_and(|width| (metadata.len() - 4) % u64::from(width) != 0) {
        return Err(format!(
            "Malformed Plan/content record length in {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "scoped_storage_tests.rs"]
mod tests;
