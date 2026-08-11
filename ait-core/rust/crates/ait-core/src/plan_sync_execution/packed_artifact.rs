use super::*;
use crate::pack_substrate::CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT;

#[expect(
    clippy::too_many_arguments,
    reason = "packed publication keeps storage and remote boundary inputs explicit"
)]
pub(super) fn publish_plan_revision_packed_artifact<F, L, B, C>(
    file_io_store: &F,
    local_revision_store: &L,
    local_blob_store: &B,
    request: &SyncRequest,
    client: &mut C,
    remote_repo_name: &str,
    revision: &JsonValue,
    artifact_body: &str,
) -> Result<JsonValue, String>
where
    F: FileIoStore + ?Sized,
    L: PlanSyncLocalRevisionStore + ?Sized,
    B: PlanSyncLocalBlobStore + PlanSyncZstdPackStore + ?Sized,
    C: PlanSyncRemotePublisher + ?Sized,
{
    let mut bundle = prepare_plan_revision_packed_artifact(
        local_revision_store,
        local_blob_store,
        request,
        revision,
        artifact_body,
    )?;
    let object_pack_bytes = file_io_store
        .read_bytes(&bundle.object_pack.pack_path)
        .map_err(|err| {
            format!(
                "Failed to read zstd object pack {} for plan sync: {err}",
                bundle.object_pack.pack_path.display()
            )
        })?;
    bundle.object_pack.pack_index_checksum = reuse_or_publish_remote_object_pack(
        client,
        remote_repo_name,
        &bundle.object_pack.pack_id,
        &bundle.object_pack.pack_format,
        &bundle.object_pack.pack_index_checksum,
        &object_pack_bytes,
    )?;
    let tree_pack_bytes = file_io_store
        .read_bytes(&bundle.tree_pack.pack_path)
        .map_err(|err| {
            format!(
                "Failed to read zstd tree pack {} for plan sync: {err}",
                bundle.tree_pack.pack_path.display()
            )
        })?;
    bundle.tree_pack.pack_index_checksum = reuse_or_publish_remote_tree_pack(
        client,
        remote_repo_name,
        &bundle.tree_pack.pack_id,
        &bundle.tree_pack.pack_format,
        &bundle.tree_pack.pack_index_checksum,
        &tree_pack_bytes,
    )?;
    let artifact_path = required_string_field(revision, "artifact_path")?;
    let generation_key = plan_revision_pack_generation_key(revision)?;
    refresh_plan_revision_packed_artifact_bundle(&mut bundle, &generation_key, &artifact_path);
    commit_remote_zstd_bulk_with_plan_sync_remote_client(
        client,
        remote_repo_name,
        &bundle.commit_payload,
    )?;
    Ok(bundle.artifact_payload)
}

fn reuse_or_publish_remote_object_pack<C>(
    client: &mut C,
    remote_repo_name: &str,
    pack_id: &str,
    pack_format: &str,
    local_index_checksum: &str,
    local_pack_bytes: &[u8],
) -> Result<String, String>
where
    C: PlanSyncRemotePackedArtifactUploader + ?Sized,
{
    match get_remote_zstd_object_pack_if_present_with_plan_sync_remote_client(
        client,
        remote_repo_name,
        pack_id,
    )? {
        Some(remote_pack_bytes) => validate_content_addressed_zstd_pack_reuse(
            local_pack_bytes,
            &remote_pack_bytes,
            pack_id,
            pack_format,
        ),
        None => {
            put_remote_zstd_object_pack_with_plan_sync_remote_client(
                client,
                remote_repo_name,
                pack_id,
                local_pack_bytes,
            )?;
            Ok(local_index_checksum.to_string())
        }
    }
}

fn reuse_or_publish_remote_tree_pack<C>(
    client: &mut C,
    remote_repo_name: &str,
    pack_id: &str,
    pack_format: &str,
    local_index_checksum: &str,
    local_pack_bytes: &[u8],
) -> Result<String, String>
where
    C: PlanSyncRemotePackedArtifactUploader + ?Sized,
{
    match get_remote_zstd_tree_pack_if_present_with_plan_sync_remote_client(
        client,
        remote_repo_name,
        pack_id,
    )? {
        Some(remote_pack_bytes) => validate_content_addressed_zstd_pack_reuse(
            local_pack_bytes,
            &remote_pack_bytes,
            pack_id,
            pack_format,
        ),
        None => {
            put_remote_zstd_tree_pack_with_plan_sync_remote_client(
                client,
                remote_repo_name,
                pack_id,
                local_pack_bytes,
            )?;
            Ok(local_index_checksum.to_string())
        }
    }
}

pub(super) fn prepare_plan_revision_packed_artifact<L, B>(
    local_revision_store: &L,
    local_blob_store: &B,
    request: &SyncRequest,
    revision: &JsonValue,
    artifact_body: &str,
) -> Result<PlanSyncPackedArtifactBundle, String>
where
    L: PlanSyncLocalRevisionStore + ?Sized,
    B: PlanSyncLocalBlobStore + PlanSyncZstdPackStore + ?Sized,
{
    let artifact_path = required_string_field(revision, "artifact_path")?;
    if let Some(expected_blob_id) = text_field(revision, "artifact_blob_id") {
        let actual_blob_id = artifact_blob_id(artifact_body);
        if actual_blob_id != expected_blob_id {
            return Err(format!(
                "Plan revision {:?} declares artifact_blob_id {expected_blob_id}, but its local Markdown bytes resolve to {actual_blob_id}.",
                text_field(revision, "plan_revision_id")
            ));
        }
    }
    let created_at = current_timestamp();
    let generation_key = plan_revision_pack_generation_key(revision)?;
    let object_pack = ensure_plan_revision_zstd_object_pack(
        local_revision_store,
        local_blob_store,
        Path::new(&request.root_path),
        revision,
        &generation_key,
        artifact_body.as_bytes(),
        Some(artifact_path.as_str()),
        &created_at,
    )?;
    let tree_pack = write_plan_revision_zstd_tree_pack(
        local_blob_store,
        Path::new(&request.root_path),
        &generation_key,
        artifact_path.as_str(),
        &object_pack.blob_id,
        object_pack.byte_count,
        &created_at,
    )?;
    let artifact_payload = plan_revision_packed_artifact_payload(
        &generation_key,
        &artifact_path,
        &object_pack,
        &tree_pack,
    );
    let commit_payload = RemoteSyncCommitJson::stateless().plan_revision_zstd_bulk_commit_request(
        &generation_key,
        &object_pack,
        &tree_pack,
    );
    Ok(PlanSyncPackedArtifactBundle {
        object_pack,
        tree_pack,
        artifact_payload,
        commit_payload,
    })
}

fn refresh_plan_revision_packed_artifact_bundle(
    bundle: &mut PlanSyncPackedArtifactBundle,
    generation_key: &str,
    artifact_path: &str,
) {
    let object_pack = &bundle.object_pack;
    let tree_pack = &bundle.tree_pack;
    bundle.artifact_payload = plan_revision_packed_artifact_payload(
        generation_key,
        artifact_path,
        object_pack,
        tree_pack,
    );
    bundle.commit_payload = RemoteSyncCommitJson::stateless()
        .plan_revision_zstd_bulk_commit_request(generation_key, object_pack, tree_pack);
}

fn plan_revision_packed_artifact_payload(
    generation_key: &str,
    artifact_path: &str,
    object_pack: &PlanSyncZstdObjectPackBundle,
    tree_pack: &PlanSyncZstdTreePackBundle,
) -> JsonValue {
    json!({
        "storage_authority": "remote_zstd_pack",
        "generation_key": generation_key,
        "artifact_blob_id": object_pack.blob_id,
        "artifact_path": artifact_path,
        "media_type": "text/markdown; charset=utf-8",
        "encoding": "utf-8",
        "byte_count": object_pack.byte_count,
        "object_pack": {
            "generation_key": generation_key,
            "pack_id": object_pack.pack_id,
            "pack_format": object_pack.pack_format,
            "pack_index_entry_name": object_pack.pack_index_entry_name,
            "pack_index_checksum": object_pack.pack_index_checksum,
        },
        "blob_locator": {
            "generation_key": generation_key,
            "blob_id": object_pack.blob_id,
            "sha256": object_pack.sha256,
            "size_bytes": object_pack.byte_count,
            "pack_id": object_pack.pack_id,
            "pack_entry_name": object_pack.pack_entry_name,
            "pack_entry_type": object_pack.pack_entry_type,
            "pack_base_blob_id": object_pack.pack_base_blob_id,
            "pack_chain_depth": object_pack.pack_chain_depth,
        },
        "tree_pack": {
            "generation_key": generation_key,
            "pack_id": tree_pack.pack_id,
            "pack_format": tree_pack.pack_format,
            "pack_index_entry_name": tree_pack.pack_index_entry_name,
            "pack_index_checksum": tree_pack.pack_index_checksum,
        },
        "root_tree": {
            "tree_id": tree_pack.root_tree_id,
            "entry_count": tree_pack.root_entry_count,
            "tree_pack_id": tree_pack.pack_id,
            "tree_pack_checksum": tree_pack.root_tree_checksum,
            "entry_ordinal": tree_pack.root_entry_ordinal,
        },
    })
}

pub(super) fn plan_sync_blob_pack_entry_name(blob_id: &str) -> String {
    format!("blobs/{blob_id}")
}

#[expect(
    clippy::too_many_arguments,
    reason = "pack creation inputs map directly to persisted revision metadata"
)]
pub(super) fn ensure_plan_revision_zstd_object_pack<L, B>(
    local_revision_store: &L,
    local_blob_store: &B,
    repo_root: &Path,
    revision: &JsonValue,
    generation_key: &str,
    data: &[u8],
    path_hint: Option<&str>,
    created_at: &str,
) -> Result<PlanSyncZstdObjectPackBundle, String>
where
    L: PlanSyncLocalRevisionStore + ?Sized,
    B: PlanSyncLocalBlobStore + PlanSyncZstdPackStore + ?Sized,
{
    let sha256 = sha256_hex(data);
    let blob_id = format!("BLB-{}", &sha256[..20]);
    if let Some(existing) = existing_zstd_object_pack_bundle_with_plan_sync_zstd_pack_store(
        local_blob_store,
        &blob_id,
        &sha256,
        data.len() as i64,
    )? {
        return Ok(existing);
    }
    let expected_pack_entry_name = plan_sync_blob_pack_entry_name(&blob_id);
    let blob_items = vec![json!({
        "entry_name": expected_pack_entry_name,
        "blob_id": blob_id.clone(),
        "data": data,
        "path_hint": path_hint.unwrap_or(""),
    })];
    let pack_id = build_plan_revision_object_pack_id(generation_key, &blob_items)?;
    let pack_rel_path = default_object_pack_relative_path(&pack_id);
    let pack_path = repo_root.join(&pack_rel_path);
    if let Some(parent) = pack_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let initial_by_path = match path_hint {
        Some(path) => plan_revision_parent_delta_candidates(
            local_revision_store,
            local_blob_store,
            revision,
            path,
        )?,
        None => json!({}),
    };
    let members = build_pack_members(
        &JsonValue::Array(blob_items.clone()),
        MAX_PACK_CHAIN_DEPTH,
        Some(&initial_by_path),
    )?;
    let member_obj = members
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(JsonValue::as_object)
        .cloned()
        .ok_or_else(|| "Failed to build zstd plan-sync object pack member.".to_string())?;
    let archive_stats = write_pack_archive_with_format(
        pack_path.to_string_lossy().as_ref(),
        &pack_id,
        CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
        &members,
        zstd_only_object_pack_write_format(),
    )?;
    let pack_format = json_string_field(&archive_stats, "pack_format")?;
    let member_count = json_i64_field(&archive_stats, "member_count")?;
    let total_bytes = json_i64_field(&archive_stats, "total_bytes")?;
    let pack_index_entry_name = json_string_field(&archive_stats, "pack_index_entry_name")?;
    let pack_index_checksum = json_string_field(&archive_stats, "pack_index_checksum")?;
    let pack_entry_name = member_obj
        .get("entry_name")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Object pack member entry_name must be a non-empty string.".to_string())?
        .to_string();
    if pack_entry_name != expected_pack_entry_name {
        return Err(format!(
            "Object pack member entry_name {pack_entry_name:?} does not match canonical Plan-sync entry {expected_pack_entry_name:?}."
        ));
    }
    let pack_entry_type = member_obj
        .get("entry_type")
        .and_then(JsonValue::as_str)
        .unwrap_or("full")
        .to_string();
    let pack_base_blob_id = member_obj
        .get("base_blob_id")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    let pack_chain_depth = member_obj
        .get("chain_depth")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    upsert_zstd_object_pack_metadata_with_plan_sync_zstd_pack_store(
        local_blob_store,
        PlanSyncZstdObjectPackMetadata {
            blob_id: &blob_id,
            sha256: &sha256,
            size_bytes: data.len() as i64,
            pack_id: &pack_id,
            pack_rel_path: &pack_rel_path,
            pack_format: &pack_format,
            member_count,
            total_bytes,
            pack_index_entry_name: &pack_index_entry_name,
            pack_index_checksum: &pack_index_checksum,
            pack_entry_type: &pack_entry_type,
            pack_base_blob_id: pack_base_blob_id.as_deref(),
            pack_chain_depth,
            created_at,
        },
    )?;
    Ok(PlanSyncZstdObjectPackBundle {
        blob_id,
        sha256,
        byte_count: data.len() as i64,
        pack_id,
        pack_path,
        pack_format,
        member_count,
        total_bytes,
        pack_index_entry_name,
        pack_index_checksum,
        pack_entry_name,
        pack_entry_type,
        pack_base_blob_id,
        pack_chain_depth,
        created_at: created_at.to_string(),
    })
}

pub(super) fn plan_revision_pack_generation_key(revision: &JsonValue) -> Result<String, String> {
    require_plan_revision_id(revision)
}

pub(super) fn build_plan_revision_object_pack_id(
    generation_key: &str,
    blob_items: &[JsonValue],
) -> Result<String, String> {
    let mut blob_ids = Vec::new();
    for row in blob_items {
        let blob_id = row
            .as_object()
            .and_then(|obj| obj.get("blob_id"))
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                "Plan-sync object pack candidate payload is missing blob_id.".to_string()
            })?;
        blob_ids.push(blob_id.to_string());
    }
    blob_ids.sort();
    let seed = format!("{generation_key}|{}", blob_ids.join("|"));
    Ok(format!(
        "PCK-{}",
        sha256_hex(seed.as_bytes())[..12].to_ascii_uppercase()
    ))
}

pub(super) fn plan_revision_parent_delta_candidates<L, B>(
    local_revision_store: &L,
    local_blob_store: &B,
    revision: &JsonValue,
    artifact_path: &str,
) -> Result<JsonValue, String>
where
    L: PlanSyncLocalRevisionStore + ?Sized,
    B: PlanSyncLocalBlobStore + ?Sized,
{
    let Some(parent_revision_id) = text_field(revision, "parent_plan_revision_id") else {
        return Ok(json!({}));
    };
    let Some(parent_artifact) = get_plan_revision_artifact_with_plan_sync_local_store(
        local_revision_store,
        &parent_revision_id,
    )?
    else {
        return Ok(json!({}));
    };
    if !parent_artifact.remote_published {
        return Ok(json!({}));
    }
    if parent_artifact.artifact_path != artifact_path {
        return Ok(json!({}));
    }
    let Some(parent_blob_id) = parent_artifact.artifact_blob_id else {
        return Ok(json!({}));
    };
    let bytes =
        match read_blob_bytes_with_plan_sync_local_blob_store(local_blob_store, &parent_blob_id) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(json!({})),
        };
    let chain_depth =
        match blob_chain_depth_with_plan_sync_local_blob_store(local_blob_store, &parent_blob_id) {
            Ok(depth) => depth.unwrap_or(0),
            Err(_) => return Ok(json!({})),
        };
    let mut candidates = JsonMap::new();
    candidates.insert(
        artifact_path.to_string(),
        json!({
            "blob_id": parent_blob_id,
            "data": bytes,
            "chain_depth": chain_depth,
        }),
    );
    Ok(JsonValue::Object(candidates))
}

pub(super) fn write_plan_revision_zstd_tree_pack<B>(
    local_blob_store: &B,
    repo_root: &Path,
    generation_key: &str,
    artifact_path: &str,
    blob_id: &str,
    byte_count: i64,
    created_at: &str,
) -> Result<PlanSyncZstdTreePackBundle, String>
where
    B: PlanSyncLocalBlobStore + PlanSyncZstdPackStore + ?Sized,
{
    let (root_tree_id, tree_rows, tree_entry_rows) =
        plan_revision_tree_rows(artifact_path, blob_id, byte_count)?;
    if let Some(existing) = existing_zstd_tree_pack_bundle_with_plan_sync_zstd_pack_store(
        local_blob_store,
        generation_key,
        &root_tree_id,
        &tree_rows,
    )? {
        return Ok(existing);
    }
    let all_tree_ids = tree_rows
        .iter()
        .filter_map(|row| row.get("tree_id").and_then(JsonValue::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let unrecorded_tree_ids = local_blob_store.unrecorded_tree_ids(&all_tree_ids)?;
    if !unrecorded_tree_ids.contains(&root_tree_id) {
        return Err(format!(
            "Plan sync root tree {root_tree_id} exists without a reusable packed locator."
        ));
    }
    let mut new_tree_rows = Vec::with_capacity(unrecorded_tree_ids.len());
    for row in tree_rows {
        let tree_id = required_string_field(&row, "tree_id")?;
        if unrecorded_tree_ids.contains(&tree_id) {
            new_tree_rows.push(row);
        }
    }
    let mut new_tree_entry_rows = Vec::new();
    for row in tree_entry_rows {
        let tree_id = required_string_field(&row, "tree_id")?;
        if unrecorded_tree_ids.contains(&tree_id) {
            new_tree_entry_rows.push(row);
        }
    }
    let tree_ids = unrecorded_tree_ids.into_iter().collect::<Vec<_>>();
    let pack_seed = format!("{generation_key}|{root_tree_id}|{}", tree_ids.join("|"));
    let pack_id = format!(
        "TPK-{}",
        &sha256_hex(pack_seed.as_bytes())[..12].to_ascii_uppercase()
    );
    let pack_rel_path = default_tree_pack_relative_path(&pack_id);
    let pack_path = repo_root.join(&pack_rel_path);
    if let Some(parent) = pack_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let tree_rows_json = JsonValue::Array(new_tree_rows);
    let tree_entry_rows_json = JsonValue::Array(new_tree_entry_rows);
    let members = build_tree_pack_members(&tree_rows_json, &tree_entry_rows_json)?;
    let archive_stats = write_tree_pack_archive_with_format(
        pack_path.to_string_lossy().as_ref(),
        &pack_id,
        CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
        &members,
        zstd_only_tree_pack_write_format(),
    )?;
    let pack_format = json_string_field(&archive_stats, "pack_format")?;
    let tree_count = json_i64_field(&archive_stats, "tree_count")?;
    let total_bytes = json_i64_field(&archive_stats, "total_bytes")?;
    let pack_index_entry_name = json_string_field(&archive_stats, "pack_index_entry_name")?;
    let pack_index_checksum = json_string_field(&archive_stats, "pack_index_checksum")?;
    let checksums_by_tree_id = tree_pack_checksums_by_tree_id(&archive_stats)?;
    let pack_index = archive_stats
        .get("pack_index")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "zstd tree-pack archive stats are missing pack_index.".to_string())?;
    let index_trees = pack_index
        .get("trees")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "zstd tree-pack index is missing trees.".to_string())?;
    let mut tree_locators = Vec::new();
    let mut root_entry_count = 0_i64;
    let mut root_entry_ordinal = 0_i64;
    let mut root_tree_checksum = String::new();
    for entry in index_trees {
        let tree_id = required_string_field(entry, "tree_id")?;
        let entry_count = json_i64_field(entry, "entry_count")?;
        let entry_ordinal = json_i64_field(entry, "entry_ordinal")?;
        let checksum = checksums_by_tree_id
            .get(&tree_id)
            .cloned()
            .ok_or_else(|| format!("zstd tree-pack index is missing checksum for {tree_id}."))?;
        if tree_id == root_tree_id {
            root_entry_count = entry_count;
            root_entry_ordinal = entry_ordinal;
            root_tree_checksum = checksum.clone();
        }
        tree_locators.push(ZstdBulkTreeLocatorRow {
            generation_key: Some(generation_key.to_string()),
            tree_id,
            entry_count: Some(entry_count),
            tree_pack_id: Some(pack_id.clone()),
            tree_pack_checksum: Some(checksum),
            created_at: Some(created_at.to_string()),
        });
    }
    if root_tree_checksum.is_empty() {
        return Err(format!(
            "zstd tree-pack index did not include root tree {root_tree_id}."
        ));
    }
    upsert_zstd_tree_pack_metadata_with_plan_sync_zstd_pack_store(
        local_blob_store,
        PlanSyncZstdTreePackMetadata {
            pack_id: &pack_id,
            pack_rel_path: &pack_rel_path,
            pack_format: &pack_format,
            tree_count,
            total_bytes,
            pack_index_entry_name: &pack_index_entry_name,
            pack_index_checksum: &pack_index_checksum,
            tree_locators: &tree_locators,
            created_at,
        },
    )?;
    Ok(PlanSyncZstdTreePackBundle {
        root_tree_id,
        root_entry_count,
        root_entry_ordinal,
        root_tree_checksum,
        pack_id,
        pack_path,
        pack_format,
        tree_count,
        total_bytes,
        pack_index_entry_name,
        pack_index_checksum,
        tree_locators,
        created_at: created_at.to_string(),
    })
}

pub(super) fn plan_revision_tree_rows(
    artifact_path: &str,
    blob_id: &str,
    byte_count: i64,
) -> Result<(String, Vec<JsonValue>, Vec<JsonValue>), String> {
    let parts = artifact_path
        .split('/')
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err("Plan revision artifact path is empty.".to_string());
    }
    let mut root = BTreeMap::new();
    insert_plan_sync_tree_blob(
        &mut root,
        &parts,
        PlanSyncTreeNode::Blob {
            blob_id: blob_id.to_string(),
            size_bytes: byte_count,
            mode: "0o644".to_string(),
        },
    )?;
    let mut tree_rows = BTreeMap::new();
    let mut tree_entry_rows = BTreeMap::new();
    let root_tree_id = materialize_plan_sync_tree(&root, &mut tree_rows, &mut tree_entry_rows)?;
    Ok((
        root_tree_id,
        tree_rows.into_values().collect(),
        tree_entry_rows.into_values().collect(),
    ))
}

pub(super) fn insert_plan_sync_tree_blob(
    children: &mut BTreeMap<String, PlanSyncTreeNode>,
    parts: &[&str],
    node: PlanSyncTreeNode,
) -> Result<(), String> {
    if parts.len() == 1 {
        children.insert(parts[0].to_string(), node);
        return Ok(());
    }
    let entry = children
        .entry(parts[0].to_string())
        .or_insert_with(|| PlanSyncTreeNode::Tree {
            children: BTreeMap::new(),
        });
    let PlanSyncTreeNode::Tree { children } = entry else {
        return Err(format!(
            "Path collision while building plan-sync tree at {}",
            parts[0]
        ));
    };
    insert_plan_sync_tree_blob(children, &parts[1..], node)
}

pub(super) fn materialize_plan_sync_tree(
    children: &BTreeMap<String, PlanSyncTreeNode>,
    tree_rows: &mut BTreeMap<String, JsonValue>,
    tree_entry_rows: &mut BTreeMap<(String, String), JsonValue>,
) -> Result<String, String> {
    let mut serialized_entries = Vec::new();
    let mut pending_entries = Vec::new();
    for (name, node) in children {
        match node {
            PlanSyncTreeNode::Tree { children } => {
                let child_tree_id =
                    materialize_plan_sync_tree(children, tree_rows, tree_entry_rows)?;
                serialized_entries.push(json!({
                    "name": name,
                    "type": "tree",
                    "target_id": child_tree_id,
                }));
                pending_entries.push(json!({
                    "entry_name": name,
                    "entry_type": "tree",
                    "target_id": child_tree_id,
                    "size_bytes": JsonValue::Null,
                    "mode": "tree",
                }));
            }
            PlanSyncTreeNode::Blob {
                blob_id,
                size_bytes,
                mode,
            } => {
                serialized_entries.push(json!({
                    "name": name,
                    "type": "blob",
                    "target_id": blob_id,
                    "size_bytes": size_bytes,
                    "mode": mode,
                }));
                pending_entries.push(json!({
                    "entry_name": name,
                    "entry_type": "blob",
                    "target_id": blob_id,
                    "size_bytes": size_bytes,
                    "mode": mode,
                }));
            }
        }
    }
    let serialized_entries_json = JsonCodec::encode_value(
        &JsonValue::Array(serialized_entries.clone()),
        JsonEncodeOptions::compact(),
    )
    .map_err(String::from)?;
    let digest = sha256_hex(serialized_entries_json.as_bytes());
    let tree_id = format!("TRE-{}", &digest[..20].to_ascii_uppercase());
    tree_rows
        .entry(tree_id.clone())
        .or_insert_with(|| json!({"tree_id": tree_id, "entry_count": serialized_entries.len()}));
    for entry in pending_entries {
        let entry_name = entry
            .get("entry_name")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string();
        let mut row = entry
            .as_object()
            .cloned()
            .ok_or_else(|| "Invalid plan-sync tree entry row.".to_string())?;
        row.insert("tree_id".to_string(), JsonValue::String(tree_id.clone()));
        tree_entry_rows.insert((tree_id.clone(), entry_name), JsonValue::Object(row));
    }
    Ok(tree_id)
}

pub(super) fn resolve_public_package_targets_contract_artifact(
    request: &SyncRequest,
    path: &Path,
) -> Result<JsonValue, String> {
    let payload =
        resolve_repo_artifact_path(&request.root_path, path.to_string_lossy().as_ref(), false)
            .map_err(plan_fs_error)?;
    let artifact_path = required_string_field(&payload, "artifact_path")?;
    let resolved_path = required_string_field(&payload, "resolved_path")?;
    let resolved = PathBuf::from(resolved_path);
    if !resolved.is_file() {
        return Err(format!(
            "Plan sync artifact must be a file: {}",
            path.display()
        ));
    }
    if artifact_path != PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH {
        return Err(format!(
            "Plan sync paired artifacts currently support `{}` / `{}` only: {}",
            PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH,
            PUBLIC_FUTURE_REPO_PREP_CONTRACT_PATH,
            path.display()
        ));
    }
    let contract = read_json_file(resolved.to_string_lossy().as_ref()).map_err(plan_fs_error)?;
    let body = read_utf8_text_file(resolved.to_string_lossy().as_ref()).map_err(plan_fs_error)?;
    let byte_count = body.len();
    let sha256 = Sha256::digest(body.as_bytes())
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>();
    if !contract.is_object() {
        return Err(format!(
            "Public package targets contract {artifact_path} must contain a JSON object."
        ));
    }
    if contract
        .get("schema_version")
        .and_then(|value| value.as_i64())
        .is_none()
    {
        return Err(format!(
            "Public package targets contract {artifact_path} must declare an integer schema_version."
        ));
    }
    let guide_path = optional_text(value_get(&contract, "guide_path"))?.ok_or_else(|| {
        format!(
            "Public package targets contract {artifact_path} must declare guide_path `{}`.",
            PUBLIC_PACKAGE_TARGETS_GUIDE_PATH
        )
    })?;
    if guide_path != PUBLIC_PACKAGE_TARGETS_GUIDE_PATH {
        return Err(format!(
            "Public package targets contract {artifact_path} must declare guide_path `{}`, not {:?}.",
            PUBLIC_PACKAGE_TARGETS_GUIDE_PATH,
            guide_path
        ));
    }
    let distribution = value_get(&contract, "distribution")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "artifact_label".to_string(),
            JsonValue::String("Public package targets contract".to_string()),
        ),
        (
            "artifact_path".to_string(),
            JsonValue::String(artifact_path.clone()),
        ),
        (
            "source_artifact_path".to_string(),
            JsonValue::String(guide_path.clone()),
        ),
        (
            "role".to_string(),
            JsonValue::String("public_package_targets_contract_json".to_string()),
        ),
        (
            "media_type".to_string(),
            JsonValue::String("application/vnd.ait.public-package-targets+json".to_string()),
        ),
        (
            "encoding".to_string(),
            JsonValue::String("utf-8".to_string()),
        ),
        ("body".to_string(), JsonValue::String(body)),
        (
            "byte_count".to_string(),
            JsonValue::Number(Number::from(byte_count as u64)),
        ),
        ("sha256".to_string(), JsonValue::String(sha256)),
        (
            "metadata".to_string(),
            JsonValue::Object(JsonMap::from_iter([
                (
                    "artifact_kind".to_string(),
                    JsonValue::String("public_package_targets_contract_json".to_string()),
                ),
                ("guide_path".to_string(), JsonValue::String(guide_path)),
                (
                    "distribution".to_string(),
                    JsonValue::Object(JsonMap::from_iter([
                        ("name".to_string(), source_text(distribution.get("name"))),
                        (
                            "version".to_string(),
                            source_text(distribution.get("version")),
                        ),
                        (
                            "artifact_model".to_string(),
                            source_text(distribution.get("artifact_model")),
                        ),
                    ])),
                ),
                ("loaded_from".to_string(), JsonValue::String(artifact_path)),
            ])),
        ),
    ])))
}

pub(super) fn resolve_public_future_repo_extraction_prep_contract_artifact(
    request: &SyncRequest,
    path: &Path,
) -> Result<JsonValue, String> {
    let payload =
        resolve_repo_artifact_path(&request.root_path, path.to_string_lossy().as_ref(), false)
            .map_err(plan_fs_error)?;
    let artifact_path = required_string_field(&payload, "artifact_path")?;
    let resolved_path = required_string_field(&payload, "resolved_path")?;
    let resolved = PathBuf::from(resolved_path);
    if !resolved.is_file() {
        return Err(format!(
            "Plan sync artifact must be a file: {}",
            path.display()
        ));
    }
    if artifact_path != PUBLIC_FUTURE_REPO_PREP_CONTRACT_PATH {
        return Err(format!(
            "Plan sync paired artifacts currently support `{}` / `{}` only: {}",
            PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH,
            PUBLIC_FUTURE_REPO_PREP_CONTRACT_PATH,
            path.display()
        ));
    }
    let contract = read_json_file(resolved.to_string_lossy().as_ref()).map_err(plan_fs_error)?;
    let body = read_utf8_text_file(resolved.to_string_lossy().as_ref()).map_err(plan_fs_error)?;
    let byte_count = body.len();
    let sha256 = Sha256::digest(body.as_bytes())
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>();
    if !contract.is_object() {
        return Err(format!(
            "Future repo extraction prep contract {artifact_path} must contain a JSON object."
        ));
    }
    if contract
        .get("schema_version")
        .and_then(|value| value.as_i64())
        .is_none()
    {
        return Err(format!(
            "Future repo extraction prep contract {artifact_path} must declare an integer schema_version."
        ));
    }
    let guide_path = optional_text(value_get(&contract, "guide_path"))?.ok_or_else(|| {
        format!(
            "Future repo extraction prep contract {artifact_path} must declare guide_path `{}`.",
            PUBLIC_FUTURE_REPO_PREP_GUIDE_PATH
        )
    })?;
    if guide_path != PUBLIC_FUTURE_REPO_PREP_GUIDE_PATH {
        return Err(format!(
            "Future repo extraction prep contract {artifact_path} must declare guide_path `{}`, not {:?}.",
            PUBLIC_FUTURE_REPO_PREP_GUIDE_PATH,
            guide_path
        ));
    }
    let distribution = value_get(&contract, "distribution")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let future_repositories = value_get(&contract, "future_repositories")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let future_repo_ids = future_repositories
        .into_iter()
        .filter_map(|value| value.as_object().cloned())
        .filter_map(|row| {
            row.get("repo_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .map(str::to_string)
        })
        .filter(|value| !value.is_empty())
        .map(JsonValue::String)
        .collect::<Vec<_>>();
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "artifact_label".to_string(),
            JsonValue::String("Future repo extraction prep contract".to_string()),
        ),
        (
            "artifact_path".to_string(),
            JsonValue::String(artifact_path.clone()),
        ),
        (
            "source_artifact_path".to_string(),
            JsonValue::String(guide_path.clone()),
        ),
        (
            "role".to_string(),
            JsonValue::String("public_future_repo_extraction_prep_contract_json".to_string()),
        ),
        (
            "media_type".to_string(),
            JsonValue::String(
                "application/vnd.ait.public-future-repo-extraction-prep+json".to_string(),
            ),
        ),
        (
            "encoding".to_string(),
            JsonValue::String("utf-8".to_string()),
        ),
        ("body".to_string(), JsonValue::String(body)),
        (
            "byte_count".to_string(),
            JsonValue::Number(Number::from(byte_count as u64)),
        ),
        ("sha256".to_string(), JsonValue::String(sha256)),
        (
            "metadata".to_string(),
            JsonValue::Object(JsonMap::from_iter([
                (
                    "artifact_kind".to_string(),
                    JsonValue::String(
                        "public_future_repo_extraction_prep_contract_json".to_string(),
                    ),
                ),
                ("guide_path".to_string(), JsonValue::String(guide_path)),
                (
                    "package_targets_contract_path".to_string(),
                    value_get(&contract, "package_targets_contract_path")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "distribution".to_string(),
                    JsonValue::Object(JsonMap::from_iter([
                        ("name".to_string(), source_text(distribution.get("name"))),
                        (
                            "version".to_string(),
                            source_text(distribution.get("version")),
                        ),
                        (
                            "artifact_model".to_string(),
                            source_text(distribution.get("artifact_model")),
                        ),
                    ])),
                ),
                (
                    "future_repo_ids".to_string(),
                    JsonValue::Array(future_repo_ids),
                ),
                ("loaded_from".to_string(), JsonValue::String(artifact_path)),
            ])),
        ),
    ])))
}

pub(super) fn resolve_public_future_repo_split_dry_run_contract_artifact(
    request: &SyncRequest,
    path: &Path,
) -> Result<JsonValue, String> {
    let payload =
        resolve_repo_artifact_path(&request.root_path, path.to_string_lossy().as_ref(), false)
            .map_err(plan_fs_error)?;
    let artifact_path = required_string_field(&payload, "artifact_path")?;
    let resolved_path = required_string_field(&payload, "resolved_path")?;
    let resolved = PathBuf::from(resolved_path);
    if !resolved.is_file() {
        return Err(format!(
            "Plan sync artifact must be a file: {}",
            path.display()
        ));
    }
    if artifact_path != PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_CONTRACT_PATH {
        return Err(format!(
            "Plan sync paired artifacts currently support `{}` / `{}` / `{}` only: {}",
            PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH,
            PUBLIC_FUTURE_REPO_PREP_CONTRACT_PATH,
            PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_CONTRACT_PATH,
            path.display()
        ));
    }
    let contract = read_json_file(resolved.to_string_lossy().as_ref()).map_err(plan_fs_error)?;
    let body = read_utf8_text_file(resolved.to_string_lossy().as_ref()).map_err(plan_fs_error)?;
    let byte_count = body.len();
    let sha256 = Sha256::digest(body.as_bytes())
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>();
    if !contract.is_object() {
        return Err(format!(
            "Future repo split dry-run contract {artifact_path} must contain a JSON object."
        ));
    }
    if contract
        .get("schema_version")
        .and_then(|value| value.as_i64())
        .is_none()
    {
        return Err(format!(
            "Future repo split dry-run contract {artifact_path} must declare an integer schema_version."
        ));
    }
    let guide_path = optional_text(value_get(&contract, "guide_path"))?.ok_or_else(|| {
        format!(
            "Future repo split dry-run contract {artifact_path} must declare guide_path `{}`.",
            PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_GUIDE_PATH
        )
    })?;
    if guide_path != PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_GUIDE_PATH {
        return Err(format!(
            "Future repo split dry-run contract {artifact_path} must declare guide_path `{}`, not {:?}.",
            PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_GUIDE_PATH,
            guide_path
        ));
    }
    let distribution = value_get(&contract, "distribution")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let future_repositories = value_get(&contract, "future_repositories")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let future_repo_ids = future_repositories
        .into_iter()
        .filter_map(|value| value.as_object().cloned())
        .filter_map(|row| {
            row.get("repo_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .map(str::to_string)
        })
        .filter(|value| !value.is_empty())
        .map(JsonValue::String)
        .collect::<Vec<_>>();
    let dry_run_profiles = value_get(&contract, "dry_run_profiles")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let dry_run_profile_ids = dry_run_profiles
        .into_iter()
        .filter_map(|value| value.as_object().cloned())
        .filter_map(|row| {
            row.get("profile_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .map(str::to_string)
        })
        .filter(|value| !value.is_empty())
        .map(JsonValue::String)
        .collect::<Vec<_>>();
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "artifact_label".to_string(),
            JsonValue::String("Future repo split dry-run contract".to_string()),
        ),
        (
            "artifact_path".to_string(),
            JsonValue::String(artifact_path.clone()),
        ),
        (
            "source_artifact_path".to_string(),
            JsonValue::String(guide_path.clone()),
        ),
        (
            "role".to_string(),
            JsonValue::String("public_future_repo_split_dry_run_contract_json".to_string()),
        ),
        (
            "media_type".to_string(),
            JsonValue::String(
                "application/vnd.ait.public-future-repo-split-dry-run+json".to_string(),
            ),
        ),
        (
            "encoding".to_string(),
            JsonValue::String("utf-8".to_string()),
        ),
        ("body".to_string(), JsonValue::String(body)),
        (
            "byte_count".to_string(),
            JsonValue::Number(Number::from(byte_count as u64)),
        ),
        ("sha256".to_string(), JsonValue::String(sha256)),
        (
            "metadata".to_string(),
            JsonValue::Object(JsonMap::from_iter([
                (
                    "artifact_kind".to_string(),
                    JsonValue::String("public_future_repo_split_dry_run_contract_json".to_string()),
                ),
                ("guide_path".to_string(), JsonValue::String(guide_path)),
                (
                    "package_targets_contract_path".to_string(),
                    value_get(&contract, "package_targets_contract_path")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "future_repo_prep_contract_path".to_string(),
                    value_get(&contract, "future_repo_prep_contract_path")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "distribution".to_string(),
                    JsonValue::Object(JsonMap::from_iter([
                        ("name".to_string(), source_text(distribution.get("name"))),
                        (
                            "version".to_string(),
                            source_text(distribution.get("version")),
                        ),
                        (
                            "artifact_model".to_string(),
                            source_text(distribution.get("artifact_model")),
                        ),
                    ])),
                ),
                (
                    "future_repo_ids".to_string(),
                    JsonValue::Array(future_repo_ids),
                ),
                (
                    "dry_run_profile_ids".to_string(),
                    JsonValue::Array(dry_run_profile_ids),
                ),
                ("loaded_from".to_string(), JsonValue::String(artifact_path)),
            ])),
        ),
    ])))
}

pub(super) fn validate_public_package_targets_contract_artifact_for_revision(
    artifact: &JsonValue,
    markdown_artifact_path: &str,
) -> Result<(), String> {
    let guide_path = optional_text(value_get(artifact, "source_artifact_path"))?;
    let artifact_path = optional_text(value_get(artifact, "artifact_path"))?.unwrap_or_default();
    if guide_path.as_deref() != Some(markdown_artifact_path) {
        return Err(format!(
            "Public package targets contract {artifact_path} points at {:?}, not synced Markdown artifact {:?}.",
            guide_path, markdown_artifact_path
        ));
    }
    Ok(())
}

pub(super) fn validate_public_future_repo_extraction_prep_contract_artifact_for_revision(
    artifact: &JsonValue,
    markdown_artifact_path: &str,
) -> Result<(), String> {
    let guide_path = optional_text(value_get(artifact, "source_artifact_path"))?;
    let artifact_path = optional_text(value_get(artifact, "artifact_path"))?.unwrap_or_default();
    if guide_path.as_deref() != Some(markdown_artifact_path) {
        return Err(format!(
            "Future repo extraction prep contract {artifact_path} points at {:?}, not synced Markdown artifact {:?}.",
            guide_path, markdown_artifact_path
        ));
    }
    Ok(())
}

pub(super) fn validate_public_future_repo_split_dry_run_contract_artifact_for_revision(
    artifact: &JsonValue,
    markdown_artifact_path: &str,
) -> Result<(), String> {
    let guide_path = optional_text(value_get(artifact, "source_artifact_path"))?;
    let artifact_path = optional_text(value_get(artifact, "artifact_path"))?.unwrap_or_default();
    if guide_path.as_deref() != Some(markdown_artifact_path) {
        return Err(format!(
            "Future repo split dry-run contract {artifact_path} points at {:?}, not synced Markdown artifact {:?}.",
            guide_path, markdown_artifact_path
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack_substrate::{
        pack_index_checksum_with_format, tree_pack_index_checksum_with_format,
        CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    };
    use crate::plan_sync_execution::local_ports::PlanSyncLocalRevisionArtifact;
    use crate::repository_pack_json::{ZstdBulkCommitResponse, ZstdPackUploadResponse};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_PACK_ID: AtomicU64 = AtomicU64::new(0);

    struct EmptyRevisionStore;

    impl PlanSyncLocalRevisionStore for EmptyRevisionStore {
        fn list_plan_revisions(&self, _plan_id: &str) -> Result<Vec<JsonValue>, String> {
            Ok(Vec::new())
        }

        fn get_plan_revision_artifact(
            &self,
            _plan_revision_id: &str,
        ) -> Result<Option<PlanSyncLocalRevisionArtifact>, String> {
            Ok(None)
        }
    }

    struct ObjectPackStore {
        existing: Option<PlanSyncZstdObjectPackBundle>,
    }

    impl PlanSyncLocalBlobStore for ObjectPackStore {
        fn ensure_blob_bytes(
            &self,
            _data: &[u8],
            _path_hint: Option<&str>,
        ) -> Result<String, String> {
            unreachable!("object-pack preparation does not call ensure_blob_bytes")
        }

        fn read_blob_bytes(&self, _blob_id: &str) -> Result<Vec<u8>, String> {
            unreachable!("object-pack preparation without a path hint does not read parent blobs")
        }

        fn blob_chain_depth(&self, _blob_id: &str) -> Result<Option<i64>, String> {
            unreachable!("object-pack preparation without a path hint does not inspect parents")
        }
    }

    impl PlanSyncZstdPackStore for ObjectPackStore {
        fn existing_zstd_object_pack_bundle(
            &self,
            _blob_id: &str,
            _expected_sha256: &str,
            _expected_size_bytes: i64,
        ) -> Result<Option<PlanSyncZstdObjectPackBundle>, String> {
            Ok(self.existing.clone())
        }

        fn upsert_zstd_object_pack_metadata(
            &self,
            _metadata: PlanSyncZstdObjectPackMetadata<'_>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn existing_zstd_tree_pack_bundle(
            &self,
            _generation_key: &str,
            _root_tree_id: &str,
            _tree_rows: &[JsonValue],
        ) -> Result<Option<PlanSyncZstdTreePackBundle>, String> {
            Ok(None)
        }

        fn upsert_zstd_tree_pack_metadata(
            &self,
            _metadata: PlanSyncZstdTreePackMetadata<'_>,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct PackRemote {
        object_pack: Option<Vec<u8>>,
        tree_pack: Option<Vec<u8>>,
        object_puts: usize,
        tree_puts: usize,
    }

    impl PlanSyncRemotePackedArtifactUploader for PackRemote {
        fn get_remote_zstd_object_pack_if_present(
            &mut self,
            _repo_name: &str,
            _pack_id: &str,
        ) -> Result<Option<Vec<u8>>, String> {
            Ok(self.object_pack.clone())
        }

        fn get_remote_zstd_tree_pack_if_present(
            &mut self,
            _repo_name: &str,
            _pack_id: &str,
        ) -> Result<Option<Vec<u8>>, String> {
            Ok(self.tree_pack.clone())
        }

        fn put_remote_zstd_object_pack(
            &mut self,
            _repo_name: &str,
            pack_id: &str,
            _pack_bytes: &[u8],
        ) -> Result<ZstdPackUploadResponse, String> {
            self.object_puts += 1;
            Ok(upload_response(pack_id))
        }

        fn put_remote_zstd_tree_pack(
            &mut self,
            _repo_name: &str,
            pack_id: &str,
            _pack_bytes: &[u8],
        ) -> Result<ZstdPackUploadResponse, String> {
            self.tree_puts += 1;
            Ok(upload_response(pack_id))
        }

        fn commit_remote_zstd_bulk(
            &mut self,
            _repo_name: &str,
            _request: &ZstdBulkCommitRequest,
        ) -> Result<ZstdBulkCommitResponse, String> {
            Ok(ZstdBulkCommitResponse {
                repo_name: None,
                committed_snapshot_ids: Vec::new(),
                committed_object_pack_ids: Vec::new(),
                committed_tree_pack_ids: Vec::new(),
                upserted_snapshots: None,
                remote_line: None,
                line_update: None,
            })
        }
    }

    fn upload_response(pack_id: &str) -> ZstdPackUploadResponse {
        ZstdPackUploadResponse {
            repo_name: None,
            pack_id: pack_id.to_string(),
            stored: Some(true),
            pack_format: None,
            checksum: None,
            pack_bytes: None,
            raw_binary_upload: Some(true),
        }
    }

    fn temp_pack_path(label: &str) -> String {
        let unique = NEXT_TEMP_PACK_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!(
                "ait-plan-sync-{label}-{}-{unique}.zstpack",
                std::process::id()
            ))
            .to_string_lossy()
            .into_owned()
    }

    fn object_pack_bytes(pack_id: &str, created_at: &str, body: &[u8]) -> (Vec<u8>, String) {
        let path = temp_pack_path("object");
        let members = build_pack_members(
            &json!([{
                "entry_name": "blobs/BLB-PLAN",
                "blob_id": "BLB-PLAN",
                "data": body,
                "path_hint": "docs/plan.md"
            }]),
            MAX_PACK_CHAIN_DEPTH,
            None,
        )
        .unwrap();
        write_pack_archive_with_format(
            &path,
            pack_id,
            created_at,
            &members,
            PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        )
        .unwrap();
        let checksum = pack_index_checksum_with_format(&path, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1)
            .unwrap()
            .unwrap();
        let bytes = fs::read(&path).unwrap();
        let _ = fs::remove_file(path);
        (bytes, checksum)
    }

    fn tree_pack_bytes(pack_id: &str, created_at: &str) -> (Vec<u8>, String) {
        let path = temp_pack_path("tree");
        let members = build_tree_pack_members(
            &json!([{"tree_id": "TRE-ROOT", "entry_count": 1}]),
            &json!([{
                "tree_id": "TRE-ROOT",
                "entry_name": "plan.md",
                "entry_type": "blob",
                "target_id": "BLB-PLAN",
                "size_bytes": 7,
                "mode": "0o644"
            }]),
        )
        .unwrap();
        write_tree_pack_archive_with_format(
            &path,
            pack_id,
            created_at,
            &members,
            TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        )
        .unwrap();
        let checksum =
            tree_pack_index_checksum_with_format(&path, TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1)
                .unwrap()
                .unwrap();
        let bytes = fs::read(&path).unwrap();
        let _ = fs::remove_file(path);
        (bytes, checksum)
    }

    #[test]
    fn plan_sync_blob_locator_preserves_new_and_reused_object_pack_entry_name() {
        let root = tempfile::tempdir().unwrap();
        let revision = json!({"plan_revision_id": "PRV-PACK-ENTRY"});
        let artifact_body = b"# Plan\n";
        let new_pack = ensure_plan_revision_zstd_object_pack(
            &EmptyRevisionStore,
            &ObjectPackStore { existing: None },
            root.path(),
            &revision,
            "PRV-PACK-ENTRY",
            artifact_body,
            None,
            CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
        )
        .unwrap();
        let expected_entry_name = plan_sync_blob_pack_entry_name(&new_pack.blob_id);
        assert_eq!(new_pack.pack_entry_name, expected_entry_name);

        let reused_pack = ensure_plan_revision_zstd_object_pack(
            &EmptyRevisionStore,
            &ObjectPackStore {
                existing: Some(new_pack.clone()),
            },
            root.path(),
            &revision,
            "PRV-PACK-ENTRY",
            artifact_body,
            None,
            CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
        )
        .unwrap();
        assert_eq!(reused_pack.pack_entry_name, expected_entry_name);

        let tree_pack = PlanSyncZstdTreePackBundle {
            root_tree_id: "TRE-PACK-ENTRY".to_string(),
            root_entry_count: 1,
            root_entry_ordinal: 0,
            root_tree_checksum: "tree-checksum".to_string(),
            pack_id: "TPK-PACK-ENTRY".to_string(),
            pack_path: root.path().join("tree.zstpack"),
            pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
            tree_count: 1,
            total_bytes: 1,
            pack_index_entry_name: "tree-pack-index.json".to_string(),
            pack_index_checksum: "tree-pack-checksum".to_string(),
            tree_locators: Vec::new(),
            created_at: CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT.to_string(),
        };
        let artifact_payload = plan_revision_packed_artifact_payload(
            "PRV-PACK-ENTRY",
            "docs/plan.md",
            &reused_pack,
            &tree_pack,
        );
        let commit_payload = RemoteSyncCommitJson::stateless()
            .plan_revision_zstd_bulk_commit_request("PRV-PACK-ENTRY", &reused_pack, &tree_pack);
        let artifact_entry_name = artifact_payload["blob_locator"]["pack_entry_name"].as_str();
        let commit_entry_name = commit_payload.blob_locators[0].pack_entry_name.as_deref();
        assert_eq!(artifact_entry_name, Some(expected_entry_name.as_str()));
        assert_eq!(commit_entry_name, Some(expected_entry_name.as_str()));
        assert_eq!(artifact_entry_name, commit_entry_name);
    }

    #[test]
    fn plan_sync_reuses_equivalent_historical_object_pack_checksum_without_put() {
        let pack_id = "PCK-HISTORICAL-REUSE";
        let (local, local_checksum) = object_pack_bytes(
            pack_id,
            CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
            b"# Plan\n",
        );
        let (remote, remote_checksum) =
            object_pack_bytes(pack_id, "2026-07-11T07:13:38Z", b"# Plan\n");
        assert_ne!(local_checksum, remote_checksum);
        let mut client = PackRemote {
            object_pack: Some(remote),
            ..PackRemote::default()
        };
        assert_eq!(
            reuse_or_publish_remote_object_pack(
                &mut client,
                "repo",
                pack_id,
                PACK_FORMAT_ZSTD_CHUNKED_V1,
                &local_checksum,
                &local,
            )
            .unwrap(),
            remote_checksum
        );
        assert_eq!(client.object_puts, 0);
    }

    #[test]
    fn plan_sync_uploads_absent_pack_and_rejects_non_equivalent_remote_pack() {
        let pack_id = "PCK-ABSENT-OR-DRIFT";
        let (local, local_checksum) = object_pack_bytes(
            pack_id,
            CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
            b"# Plan\n",
        );
        let mut absent = PackRemote::default();
        assert_eq!(
            reuse_or_publish_remote_object_pack(
                &mut absent,
                "repo",
                pack_id,
                PACK_FORMAT_ZSTD_CHUNKED_V1,
                &local_checksum,
                &local,
            )
            .unwrap(),
            local_checksum
        );
        assert_eq!(absent.object_puts, 1);

        let (drifted, _) = object_pack_bytes(pack_id, "2026-07-11T07:13:38Z", b"# Other\n");
        let mut conflict = PackRemote {
            object_pack: Some(drifted),
            ..PackRemote::default()
        };
        let error = reuse_or_publish_remote_object_pack(
            &mut conflict,
            "repo",
            pack_id,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
            &local_checksum,
            &local,
        )
        .unwrap_err();
        assert!(error.contains("beyond index created_at"), "{error}");
        assert_eq!(conflict.object_puts, 0);
    }

    #[test]
    fn plan_sync_reuses_equivalent_historical_tree_pack_checksum_without_put() {
        let pack_id = "TPK-HISTORICAL-REUSE";
        let (local, local_checksum) =
            tree_pack_bytes(pack_id, CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT);
        let (remote, remote_checksum) = tree_pack_bytes(pack_id, "2026-07-11T07:13:38Z");
        let mut client = PackRemote {
            tree_pack: Some(remote),
            ..PackRemote::default()
        };
        assert_eq!(
            reuse_or_publish_remote_tree_pack(
                &mut client,
                "repo",
                pack_id,
                TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                &local_checksum,
                &local,
            )
            .unwrap(),
            remote_checksum
        );
        assert_eq!(client.tree_puts, 0);
    }
}
