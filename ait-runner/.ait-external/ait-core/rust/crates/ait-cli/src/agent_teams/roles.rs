use super::*;
use std::collections::BTreeMap;
mod refresh;

#[derive(Clone)]
pub(super) struct Role {
    pub index: u32,
    pub title: String,
    pub path: String,
    pub revision: u32,
    pub body: String,
}
use crate::role_templates::CURRENT as TEMPLATES;
pub(super) fn paths(root: &Path) -> Result<BTreeMap<u32, (String, String)>, String> {
    let dir = root.join("docs/roles");
    if !dir.exists() {
        return Ok(BTreeMap::new());
    }
    ait_core::plan_sync_execution::role_index_guard::validate_role_directory(&dir)?;
    let mut paths = BTreeMap::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.path().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".md") else {
            continue;
        };
        let Some((number, title)) = stem.split_once('_') else {
            continue;
        };
        let Ok(index) = number.parse::<u32>() else {
            continue;
        };
        if paths
            .insert(index, (format!("docs/roles/{name}"), title.to_string()))
            .is_some()
        {
            return Err("Duplicate role index".into());
        }
    }
    Ok(paths)
}
pub(super) fn initialize(repo: &RepoRuntime) -> Result<JsonValue, String> {
    refresh::synchronize(repo)
}
pub(super) fn load(repo: &RepoRuntime, index: u32) -> Result<Role, String> {
    let (path, title) = paths(&repo.authoritative_repo_root())?
        .remove(&index)
        .ok_or("Role MD is missing")?;
    let stores = repo.binary_db_stores::<1>();
    let plans = stores.plans();
    let read = plans.begin_read_txn();
    let heads = plans
        .list_plans(&read, None, Some(&path))
        .map_err(|e| e.to_string())?;
    if heads.len() != 1 {
        return Err("Role MD must resolve one complete local Plan".into());
    }
    let revision = heads[0]
        .head_revision
        .as_ref()
        .ok_or("Role Plan has no head")?
        .revision_index;
    let (record, payload) = plans
        .read_plan_revision(&read, revision)
        .map_err(|e| e.to_string())?;
    if record.is_tombstone() {
        return Err("Role requires a complete live Markdown revision".into());
    }
    drop(read);
    let blobs = stores.content().blobs().clone();
    let bytes = blobs
        .read_blob_bytes_for_id(
            &blobs.begin_read_txn(),
            &payload.artifact_blob_id_text().map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?
        .ok_or("Role MD content missing")?;
    let body = String::from_utf8(bytes).map_err(|_| "Role MD is not UTF-8")?;
    Ok(Role {
        index,
        title,
        path,
        revision,
        body,
    })
}
