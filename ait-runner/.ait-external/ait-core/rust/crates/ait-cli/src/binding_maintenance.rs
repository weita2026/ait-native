//! Existing content maintenance operations used by the native and language CLIs.
use crate::runtime::{RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT};
use ait_core::json_support::JsonValue;
use ait_core::local_content_gc::{
    LocalContentOrphanPackPruneStore, LocalContentStatsStore, LocalContentValidationStore,
};

pub fn gc_stats(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let store = repo.local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()?;
    let _range = crate::perfetto_range!("ait.cli.gc.stats.compute");
    store.storage_stats()
}

pub fn gc_validate(repo: &RepoRuntime) -> Result<JsonValue, String> {
    repo.local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()?
        .validate()
}

pub fn gc_validate_exit_code(payload: &JsonValue) -> Result<u8, String> {
    let needs_attention = payload
        .get("needs_attention")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| {
            "gc validate payload must contain a boolean needs_attention field.".to_string()
        })?;
    Ok(u8::from(needs_attention))
}

pub fn gc_prune(repo: &RepoRuntime, apply: bool) -> Result<JsonValue, String> {
    let store = repo.local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()?;
    if apply {
        store.prune_orphan_packs()
    } else {
        store.preview_orphan_pack_prune()
    }
}
