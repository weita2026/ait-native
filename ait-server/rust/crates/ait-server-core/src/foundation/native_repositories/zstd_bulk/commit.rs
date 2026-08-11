use super::*;
use crate::foundation::native_repositories::require_remote_sync_line_update_authority;

pub(in crate::foundation::native_repositories) fn zstd_bulk_commit_json(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    request: JsonValue,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    ensure_zstd_only_repository_flow_allowed(
        client,
        repo_name,
        &repo,
        ZstdOnlyRepositoryFlow::ZstdBulkCommit,
    )?;
    let contract = RemoteSyncCommitJson::stateless();
    let request_object = contract.zstd_bulk_commit_object(&request)?;
    let object_pack_values = contract.zstd_bulk_commit_values(request_object, "object_packs")?;
    let tree_pack_values = contract.zstd_bulk_commit_values(request_object, "tree_packs")?;
    let blob_locator_values = contract.zstd_bulk_commit_values(request_object, "blob_locators")?;
    let tree_locator_values = contract.zstd_bulk_commit_values(request_object, "tree_locators")?;
    let snapshot_values = contract.zstd_bulk_commit_values(request_object, "snapshots")?;
    let line_update = contract.line_update_request(request_object)?;
    if let Some((line_name, request)) = line_update.as_ref() {
        let current_head = if line_name == &repo.default_line {
            client
                .query_opt(
                    "select head_snapshot_id from lines where repo_id = $1 and line_name = $2",
                    &[&repo.repo_id, line_name],
                )
                .map_err(db_internal)?
                .and_then(|row| row.get::<_, Option<String>>("head_snapshot_id"))
        } else {
            None
        };
        require_remote_sync_line_update_authority(
            &repo.default_line,
            line_name,
            current_head.as_deref(),
            request.head_snapshot_id.as_deref(),
        )?;
    }

    validate_repository_blob_dependency_closure(client, &repo, &blob_locator_values)?;

    let mut object_pack_indexes = BTreeMap::new();
    let mut tree_pack_indexes = BTreeMap::new();
    let mut upserted_object_packs = 0_i64;
    let mut skipped_object_packs = 0_i64;
    for value in object_pack_values {
        let object = json_object(value, "object_packs[]")?;
        let pack_id =
            required_json_text(object, "pack_id").map_err(NativeRepositoryError::bad_request)?;
        validate_pack_id_segment(&pack_id)?;
        let pack_format = required_json_text(object, "pack_format")
            .map_err(NativeRepositoryError::bad_request)?;
        if pack_format != PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {pack_id} has unsupported pack_format {pack_format:?}"
            )));
        }
        let (pack_index, inserted) =
            upsert_zstd_object_pack(client, paths, &repo, object, &pack_id)?;
        if inserted {
            upserted_object_packs += 1;
        } else {
            skipped_object_packs += 1;
        }
        object_pack_indexes.insert(pack_id, pack_index);
    }

    let mut upserted_tree_packs = 0_i64;
    let mut skipped_tree_packs = 0_i64;
    for value in tree_pack_values {
        let object = json_object(value, "tree_packs[]")?;
        let pack_id =
            required_json_text(object, "pack_id").map_err(NativeRepositoryError::bad_request)?;
        validate_pack_id_segment(&pack_id)?;
        let pack_format = required_json_text(object, "pack_format")
            .map_err(NativeRepositoryError::bad_request)?;
        if pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {pack_id} has unsupported pack_format {pack_format:?}"
            )));
        }
        let (pack_index, inserted) = upsert_zstd_tree_pack(client, paths, &repo, object, &pack_id)?;
        if inserted {
            upserted_tree_packs += 1;
        } else {
            skipped_tree_packs += 1;
        }
        tree_pack_indexes.insert(pack_id, pack_index);
    }

    let mut upserted_blobs = 0_i64;
    for value in blob_locator_values {
        let object = json_object(value, "blob_locators[]")?;
        if upsert_zstd_blob_locator(client, paths, &repo, object, &object_pack_indexes)? {
            upserted_blobs += 1;
        }
    }

    let mut upserted_trees = 0_i64;
    for value in tree_locator_values {
        let object = json_object(value, "tree_locators[]")?;
        if upsert_zstd_tree_locator(client, object, &tree_pack_indexes)? {
            upserted_trees += 1;
        }
    }

    let mut incoming_seen = BTreeSet::new();
    let mut upserted_snapshots = 0_i64;
    let mut skipped_snapshots = 0_i64;
    for value in snapshot_values {
        let object = json_object(value, "snapshots[]")?;
        if upsert_zstd_snapshot(
            client,
            paths,
            &repo,
            object,
            &tree_pack_indexes,
            &incoming_seen,
        )? {
            upserted_snapshots += 1;
        } else {
            skipped_snapshots += 1;
        }
        let snapshot_id = required_json_text(object, "snapshot_id")
            .map_err(NativeRepositoryError::bad_request)?;
        incoming_seen.insert(snapshot_id);
    }

    let remote_line = match line_update {
        Some((line_name, line_update)) => Some(update_line_json(
            client,
            repo_name,
            &line_name,
            line_update,
        )?),
        None => None,
    };
    let line_head_updated_after_ingest = remote_line.is_some();

    Ok(
        contract.zstd_bulk_commit_response(RemoteSyncZstdBulkCommitResponse {
            repo_name: repo_name.to_string(),
            repo_id: repo.repo_id,
            upserted_object_packs,
            skipped_object_packs,
            upserted_tree_packs,
            skipped_tree_packs,
            upserted_blobs,
            upserted_trees,
            upserted_snapshots,
            skipped_snapshots,
            remote_line: remote_line.unwrap_or(JsonValue::Null),
            line_head_updated_after_ingest,
        }),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BlobDependencyNode {
    blob_id: String,
    entry_type: String,
    base_blob_id: Option<String>,
    chain_depth: i64,
}

fn validate_repository_blob_dependency_closure(
    client: &mut postgres::Client,
    repo: &RepositoryRow,
    incoming_values: &[&JsonValue],
) -> Result<(), NativeRepositoryError> {
    validate_blob_dependency_closure(&repo.repo_name, incoming_values, |blob_id| {
        let row = client
            .query_opt(
                "select blob_id, pack_entry_type, pack_base_blob_id, pack_chain_depth from blob_locators where repo_id = $1 and blob_id = $2",
                &[&repo.repo_id, &blob_id],
            )
            .map_err(db_internal)?;
        row.map(|row| {
            let blob_id: String = row.get("blob_id");
            let entry_type = row
                .get::<_, Option<String>>("pack_entry_type")
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    NativeRepositoryError::bad_request(format!(
                        "Blob {blob_id} is missing pack entry type metadata"
                    ))
                })?;
            let chain_depth = row
                .get::<_, Option<i32>>("pack_chain_depth")
                .map(i64::from)
                .ok_or_else(|| {
                    NativeRepositoryError::bad_request(format!(
                        "Blob {blob_id} is missing pack chain depth metadata"
                    ))
                })?;
            Ok(BlobDependencyNode {
                blob_id,
                entry_type,
                base_blob_id: row.get("pack_base_blob_id"),
                chain_depth,
            })
        })
        .transpose()
    })
}

fn validate_blob_dependency_closure<F>(
    repo_name: &str,
    incoming_values: &[&JsonValue],
    mut resolve_existing: F,
) -> Result<(), NativeRepositoryError>
where
    F: FnMut(&str) -> Result<Option<BlobDependencyNode>, NativeRepositoryError>,
{
    let mut incoming = BTreeMap::<String, BlobDependencyNode>::new();
    for value in incoming_values {
        let object = json_object(value, "blob_locators[]")?;
        let blob_id =
            required_json_text(object, "blob_id").map_err(NativeRepositoryError::bad_request)?;
        let node = BlobDependencyNode {
            blob_id: blob_id.clone(),
            entry_type: required_json_text(object, "pack_entry_type")
                .map_err(NativeRepositoryError::bad_request)?,
            base_blob_id: optional_json_text(object, "pack_base_blob_id"),
            chain_depth: required_i64_field(object, "pack_chain_depth")?,
        };
        if let Some(existing) = incoming.insert(blob_id.clone(), node.clone()) {
            if existing != node {
                return Err(NativeRepositoryError::bad_request(format!(
                    "Blob {blob_id} has conflicting dependency metadata in one zstd bulk commit"
                )));
            }
        }
    }

    let mut external = BTreeMap::<String, BlobDependencyNode>::new();
    let mut visiting = BTreeSet::<String>::new();
    let mut validated = BTreeSet::<String>::new();
    for blob_id in incoming.keys() {
        validate_blob_dependency_node(
            repo_name,
            blob_id,
            &incoming,
            &mut external,
            &mut visiting,
            &mut validated,
            &mut resolve_existing,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_blob_dependency_node<F>(
    repo_name: &str,
    blob_id: &str,
    incoming: &BTreeMap<String, BlobDependencyNode>,
    external: &mut BTreeMap<String, BlobDependencyNode>,
    visiting: &mut BTreeSet<String>,
    validated: &mut BTreeSet<String>,
    resolve_existing: &mut F,
) -> Result<(), NativeRepositoryError>
where
    F: FnMut(&str) -> Result<Option<BlobDependencyNode>, NativeRepositoryError>,
{
    if validated.contains(blob_id) {
        return Ok(());
    }
    if !visiting.insert(blob_id.to_string()) {
        return Err(NativeRepositoryError::bad_request(format!(
            "Blob {blob_id} has a cyclic delta dependency in repository {repo_name}"
        )));
    }

    if !incoming.contains_key(blob_id) && !external.contains_key(blob_id) {
        if let Some(node) = resolve_existing(blob_id)? {
            external.insert(blob_id.to_string(), node);
        }
    }
    let node = incoming
        .get(blob_id)
        .or_else(|| external.get(blob_id))
        .cloned()
        .ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "Blob {blob_id} is not present in repository {repo_name} or this zstd bulk commit"
            ))
        })?;

    match node.entry_type.as_str() {
        "full" => {
            if node.base_blob_id.is_some() || node.chain_depth != 0 {
                return Err(NativeRepositoryError::bad_request(format!(
                    "Full blob {} must not carry delta base metadata or non-zero chain depth",
                    node.blob_id
                )));
            }
        }
        "delta" => {
            let base_blob_id = node.base_blob_id.as_deref().ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Delta blob {} is missing pack_base_blob_id",
                    node.blob_id
                ))
            })?;
            if node.chain_depth <= 0 {
                return Err(NativeRepositoryError::bad_request(format!(
                    "Delta blob {} must have positive pack_chain_depth",
                    node.blob_id
                )));
            }
            if node.chain_depth > DEFAULT_MAX_DELTA_CHAIN_DEPTH as i64 {
                return Err(NativeRepositoryError::bad_request(format!(
                    "Delta blob {} pack_chain_depth {} exceeds the supported maximum {}",
                    node.blob_id, node.chain_depth, DEFAULT_MAX_DELTA_CHAIN_DEPTH
                )));
            }
            if !incoming.contains_key(base_blob_id) && !external.contains_key(base_blob_id) {
                if let Some(base) = resolve_existing(base_blob_id)? {
                    external.insert(base_blob_id.to_string(), base);
                }
            }
            if !incoming.contains_key(base_blob_id) && !external.contains_key(base_blob_id) {
                return Err(NativeRepositoryError::bad_request(format!(
                    "Blob {} delta base {base_blob_id} is not present in repository {repo_name} or this zstd bulk commit",
                    node.blob_id
                )));
            }
            validate_blob_dependency_node(
                repo_name,
                base_blob_id,
                incoming,
                external,
                visiting,
                validated,
                resolve_existing,
            )?;
            let base_depth = incoming
                .get(base_blob_id)
                .or_else(|| external.get(base_blob_id))
                .map(|base| base.chain_depth)
                .ok_or_else(|| {
                    NativeRepositoryError::internal(format!(
                        "Validated delta base {base_blob_id} disappeared"
                    ))
                })?;
            let expected_depth = base_depth.checked_add(1).ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Blob {} delta chain depth overflowed",
                    node.blob_id
                ))
            })?;
            if node.chain_depth != expected_depth {
                return Err(NativeRepositoryError::bad_request(format!(
                    "Blob {} pack_chain_depth {} does not follow base {base_blob_id} depth {base_depth}",
                    node.blob_id, node.chain_depth
                )));
            }
        }
        other => {
            return Err(NativeRepositoryError::bad_request(format!(
                "Blob {} has unsupported pack_entry_type {other:?}",
                node.blob_id
            )));
        }
    }

    visiting.remove(blob_id);
    validated.insert(blob_id.to_string());
    Ok(())
}

fn upsert_zstd_blob_locator(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo: &RepositoryRow,
    object: &JsonMap<String, JsonValue>,
    object_pack_indexes: &BTreeMap<String, JsonValue>,
) -> Result<bool, NativeRepositoryError> {
    let blob_id =
        required_json_text(object, "blob_id").map_err(NativeRepositoryError::bad_request)?;
    let sha256 =
        required_json_text(object, "sha256").map_err(NativeRepositoryError::bad_request)?;
    let size_bytes = required_i64_field(object, "size_bytes")?;
    let pack_id =
        required_json_text(object, "pack_id").map_err(NativeRepositoryError::bad_request)?;
    let entry_type = required_json_text(object, "pack_entry_type")
        .map_err(NativeRepositoryError::bad_request)?;
    let base_blob_id = optional_json_text(object, "pack_base_blob_id");
    let chain_depth = i32::try_from(required_i64_field(object, "pack_chain_depth")?)
        .map_err(|_| NativeRepositoryError::bad_request("Field `pack_chain_depth` exceeds i32."))?;
    let created_at = optional_json_text(object, "created_at").unwrap_or_else(now_rfc3339);
    validate_object_pack_entry(
        object_pack_indexes,
        &pack_id,
        &blob_id,
        &sha256,
        &entry_type,
    )?;
    if let Some(existing) = select_blob_by_id(client, &blob_id)? {
        if existing.sha256 != sha256 {
            return Err(NativeRepositoryError::conflict(format!(
                "Blob {blob_id} already exists with different sha256"
            )));
        }
    } else {
        client
            .execute(
                "insert into blobs(blob_id, sha256, storage_path, size_bytes, storage_kind, pack_id, pack_entry_type, pack_base_blob_id, pack_chain_depth, pruned_at, created_at) values ($1, $2, $3, $4, 'pack_full', $5, $6, $7, $8, null, $9::text::timestamptz) on conflict (blob_id) do nothing",
                &[
                    &blob_id,
                    &sha256,
                    &stored_path_string(paths, &blob_packref_path(paths, &blob_id))?,
                    &size_bytes,
                    &pack_id,
                    &entry_type,
                    &base_blob_id,
                    &chain_depth,
                    &created_at,
                ],
            )
            .map_err(db_internal)?;
    }
    if let Some(existing) =
        select_blob_locator_for_repo(client, &repo.repo_name, &repo.repo_id, &blob_id)?
    {
        if existing.sha256 != sha256 {
            return Err(NativeRepositoryError::conflict(format!(
                "Blob locator {blob_id} already exists for repository {} with different sha256",
                repo.repo_name
            )));
        }
        if existing.size_bytes == size_bytes
            && existing.pack_id == pack_id
            && existing.pack_entry_type.as_deref() == Some(entry_type.as_str())
            && existing.pack_base_blob_id == base_blob_id
            && existing.pack_chain_depth == Some(i64::from(chain_depth))
        {
            return Ok(false);
        }
        client
            .execute(
                "update blob_locators set sha256 = $1, storage_path = $2, size_bytes = $3, storage_kind = 'pack_full', pack_id = $4, pack_entry_type = $5, pack_base_blob_id = $6, pack_chain_depth = $7, created_at = $8::text::timestamptz where repo_id = $9 and blob_id = $10",
                &[
                    &sha256,
                    &stored_path_string(paths, &blob_packref_path(paths, &blob_id))?,
                    &size_bytes,
                    &pack_id,
                    &entry_type,
                    &base_blob_id,
                    &chain_depth,
                    &created_at,
                    &repo.repo_id,
                    &blob_id,
                ],
            )
            .map_err(db_internal)?;
        return Ok(true);
    }
    client
        .execute(
            "insert into blob_locators(repo_name, repo_id, blob_id, sha256, storage_path, size_bytes, storage_kind, pack_id, pack_entry_type, pack_base_blob_id, pack_chain_depth, created_at) values ($1, $2, $3, $4, $5, $6, 'pack_full', $7, $8, $9, $10, $11::text::timestamptz)",
            &[
                &repo.repo_name,
                &repo.repo_id,
                &blob_id,
                &sha256,
                &stored_path_string(paths, &blob_packref_path(paths, &blob_id))?,
                &size_bytes,
                &pack_id,
                &entry_type,
                &base_blob_id,
                &chain_depth,
                &created_at,
            ],
        )
        .map_err(db_internal)?;
    Ok(true)
}

fn upsert_zstd_tree_locator(
    client: &mut postgres::Client,
    object: &JsonMap<String, JsonValue>,
    tree_pack_indexes: &BTreeMap<String, JsonValue>,
) -> Result<bool, NativeRepositoryError> {
    let tree_id =
        required_json_text(object, "tree_id").map_err(NativeRepositoryError::bad_request)?;
    let entry_count = required_i64_field(object, "entry_count")? as i32;
    let tree_pack_id =
        required_json_text(object, "tree_pack_id").map_err(NativeRepositoryError::bad_request)?;
    let checksum = required_json_text(object, "tree_pack_checksum")
        .map_err(NativeRepositoryError::bad_request)?;
    let created_at = optional_json_text(object, "created_at").unwrap_or_else(now_rfc3339);
    validate_tree_pack_entry(
        tree_pack_indexes,
        &tree_pack_id,
        &tree_id,
        entry_count,
        &checksum,
    )?;
    let inserted = client
        .execute(
            "insert into trees(tree_id, entry_count, tree_pack_id, tree_pack_checksum, created_at) values ($1, $2, $3, $4, $5::text::timestamptz) on conflict (tree_id) do nothing",
            &[&tree_id, &entry_count, &tree_pack_id, &checksum, &created_at],
        )
        .map_err(db_internal)?
        > 0;
    Ok(inserted)
}

fn upsert_zstd_snapshot(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo: &RepositoryRow,
    object: &JsonMap<String, JsonValue>,
    tree_pack_indexes: &BTreeMap<String, JsonValue>,
    incoming_seen: &BTreeSet<String>,
) -> Result<bool, NativeRepositoryError> {
    let snapshot_id =
        required_json_text(object, "snapshot_id").map_err(NativeRepositoryError::bad_request)?;
    let parent_snapshot_id = optional_json_text(object, "parent_snapshot_id");
    let root_tree_pack_id = required_json_text(object, "root_tree_pack_id")
        .map_err(NativeRepositoryError::bad_request)?;
    let root_entry_ordinal = required_i64_field(object, "root_entry_ordinal")?;
    let manifest_hash = optional_json_text(object, "manifest_hash").unwrap_or_default();
    let message = optional_json_text(object, "message");
    let line_name = optional_json_text(object, "line_name").unwrap_or_else(default_main_line);
    let file_count = required_i64_field(object, "file_count")?;
    let total_bytes = required_i64_field(object, "total_bytes")?;
    let created_at = optional_json_text(object, "created_at").unwrap_or_else(now_rfc3339);

    if let Some(parent) = parent_snapshot_id.as_deref() {
        let parent_exists = select_snapshot_row(client, &repo.repo_name, parent)?.is_some();
        if !parent_exists && !incoming_seen.contains(parent) {
            return Err(NativeRepositoryError::bad_request(format!(
                "Snapshot {snapshot_id} parent {parent} is not present in repository {} or earlier in this zstd bulk commit",
                repo.repo_name
            )));
        }
    }

    validate_root_tree_locator(
        client,
        paths,
        &repo.repo_name,
        &repo.repo_id,
        tree_pack_indexes,
        &root_tree_pack_id,
        root_entry_ordinal as usize,
    )?;

    if let Some(existing_repo) = client
        .query_opt(
            "select repo_name from snapshots where snapshot_id = $1",
            &[&snapshot_id],
        )
        .map_err(db_internal)?
        .map(|row| row.get::<_, String>("repo_name"))
    {
        if existing_repo != repo.repo_name {
            return Err(NativeRepositoryError::conflict(format!(
                "Snapshot {snapshot_id} belongs to repository {existing_repo}, not {}",
                repo.repo_name
            )));
        }
        let existing =
            select_snapshot_row(client, &repo.repo_name, &snapshot_id)?.ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Snapshot {snapshot_id} collision disappeared"
                ))
            })?;
        validate_existing_snapshot(
            &existing,
            &repo.repo_name,
            &line_name,
            parent_snapshot_id.as_deref(),
            message.as_deref(),
            file_count,
            total_bytes,
        )?;
        return Ok(false);
    }

    let inserted = client
        .execute(
            "insert into snapshots(snapshot_id, repo_name, repo_id, parent_snapshot_id, root_tree_pack_id, root_entry_ordinal, manifest_hash, message, line_name, file_count, total_bytes, created_at) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::text::timestamptz) on conflict (snapshot_id) do nothing",
            &[
                &snapshot_id,
                &repo.repo_name,
                &repo.repo_id,
                &parent_snapshot_id,
                &root_tree_pack_id,
                &root_entry_ordinal,
                &manifest_hash,
                &message,
                &line_name,
                &(file_count as i32),
                &total_bytes,
                &created_at,
            ],
        )
        .map_err(db_internal)?
        > 0;
    Ok(inserted)
}

fn validate_root_tree_locator(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    repo_id: &str,
    tree_pack_indexes: &BTreeMap<String, JsonValue>,
    pack_id: &str,
    root_entry_ordinal: usize,
) -> Result<(), NativeRepositoryError> {
    let index = if let Some(index) = tree_pack_indexes.get(pack_id) {
        index.clone()
    } else {
        let (pack_path, pack_format) =
            tree_pack_locator_for_id(client, paths, repo_name, repo_id, pack_id)?;
        read_tree_pack_index_with_format(path_to_string(&pack_path)?.as_str(), &pack_format)
            .map_err(NativeRepositoryError::internal)?
    };
    validate_root_tree_locator_index(&index, pack_id, root_entry_ordinal)
}

#[cfg(test)]
mod dependency_tests {
    use super::*;

    fn refs(values: &[JsonValue]) -> Vec<&JsonValue> {
        values.iter().collect()
    }

    #[test]
    fn delta_dependency_preflight_rejects_missing_repository_base() {
        let values = vec![json!({
            "blob_id": "BLB-TARGET",
            "pack_entry_type": "delta",
            "pack_base_blob_id": "BLB-MISSING",
            "pack_chain_depth": 1,
        })];
        let error = validate_blob_dependency_closure("repo-a", &refs(&values), |_| Ok(None))
            .expect_err("missing base must fail before commit metadata mutation");
        assert_eq!(
            error.message,
            "Blob BLB-TARGET delta base BLB-MISSING is not present in repository repo-a or this zstd bulk commit"
        );
    }

    #[test]
    fn delta_dependency_preflight_accepts_full_base_later_in_same_commit() {
        let values = vec![
            json!({
                "blob_id": "BLB-TARGET",
                "pack_entry_type": "delta",
                "pack_base_blob_id": "BLB-BASE",
                "pack_chain_depth": 1,
            }),
            json!({
                "blob_id": "BLB-BASE",
                "pack_entry_type": "full",
                "pack_base_blob_id": null,
                "pack_chain_depth": 0,
            }),
        ];
        validate_blob_dependency_closure("repo-a", &refs(&values), |_| Ok(None))
            .expect("incoming dependency closure should be order-independent");
    }

    #[test]
    fn delta_dependency_preflight_walks_existing_repository_chain() {
        let values = vec![json!({
            "blob_id": "BLB-TARGET",
            "pack_entry_type": "delta",
            "pack_base_blob_id": "BLB-MIDDLE",
            "pack_chain_depth": 2,
        })];
        let existing = BTreeMap::from([
            (
                "BLB-MIDDLE".to_string(),
                BlobDependencyNode {
                    blob_id: "BLB-MIDDLE".to_string(),
                    entry_type: "delta".to_string(),
                    base_blob_id: Some("BLB-BASE".to_string()),
                    chain_depth: 1,
                },
            ),
            (
                "BLB-BASE".to_string(),
                BlobDependencyNode {
                    blob_id: "BLB-BASE".to_string(),
                    entry_type: "full".to_string(),
                    base_blob_id: None,
                    chain_depth: 0,
                },
            ),
        ]);
        validate_blob_dependency_closure("repo-a", &refs(&values), |blob_id| {
            Ok(existing.get(blob_id).cloned())
        })
        .expect("same-repository persisted dependency closure should validate");
    }

    #[test]
    fn delta_dependency_preflight_rejects_cycle_and_depth_drift() {
        let cyclic = vec![
            json!({
                "blob_id": "BLB-A", "pack_entry_type": "delta",
                "pack_base_blob_id": "BLB-B", "pack_chain_depth": 1,
            }),
            json!({
                "blob_id": "BLB-B", "pack_entry_type": "delta",
                "pack_base_blob_id": "BLB-A", "pack_chain_depth": 1,
            }),
        ];
        let error = validate_blob_dependency_closure("repo-a", &refs(&cyclic), |_| Ok(None))
            .expect_err("cycle must fail");
        assert!(error.message.contains("cyclic delta dependency"));

        let drift = vec![
            json!({
                "blob_id": "BLB-TARGET", "pack_entry_type": "delta",
                "pack_base_blob_id": "BLB-BASE", "pack_chain_depth": 2,
            }),
            json!({
                "blob_id": "BLB-BASE", "pack_entry_type": "full",
                "pack_base_blob_id": null, "pack_chain_depth": 0,
            }),
        ];
        let error = validate_blob_dependency_closure("repo-a", &refs(&drift), |_| Ok(None))
            .expect_err("chain depth drift must fail");
        assert!(error
            .message
            .contains("does not follow base BLB-BASE depth 0"));

        let oversized = vec![json!({
            "blob_id": "BLB-TOO-DEEP", "pack_entry_type": "delta",
            "pack_base_blob_id": "BLB-BASE",
            "pack_chain_depth": DEFAULT_MAX_DELTA_CHAIN_DEPTH + 1,
        })];
        let error = validate_blob_dependency_closure("repo-a", &refs(&oversized), |_| Ok(None))
            .expect_err("unsupported chain depth must fail before recursion");
        assert!(error.message.contains("exceeds the supported maximum"));
    }
}
