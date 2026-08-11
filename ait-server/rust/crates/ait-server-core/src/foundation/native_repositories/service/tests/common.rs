use super::*;

pub(super) fn native_line_binary_db(label: &str) -> FilesystemServerRemoteBinaryDb {
    let root = env::temp_dir().join(format!(
        "ait-server-core-native-line-binary-db-{label}-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("stale binary db root should remove");
    }
    fs::create_dir_all(&root).expect("current Binary DB fixture root should create");
    for path in SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .map(|schema| schema.path)
        .chain(
            SERVER_BINARY_DB_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path),
        )
    {
        fs::write(root.join(path), SERVER_BINARY_DB_LAYOUT_ID.to_le_bytes())
            .expect("current Binary DB fixture file should initialize");
    }
    FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("REPO-BINARY-LINE"),
        RepoName::new("repo-bin"),
        StorePath::from(root),
        StoreGeneration::new(1),
    )
}

pub(super) fn seed_native_binary_snapshot(db: &FilesystemServerRemoteBinaryDb, snapshot_id: &str) {
    let snapshots =
        ServerBinaryDbSnapshotStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    snapshots
        .append_snapshot(
            snapshot_id,
            ServerBinarySnapshotRecord {
                snapshot_meta: 0,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: server_snapshot_hash48_from_id(snapshot_id)
                    .expect("snapshot id should be canonical"),
                parent_snapshot_index_plus1: 0,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                line_index_plus1: 0,
                manifest_hash: [0; 32],
                file_count: 0,
                total_bytes: 0,
                created_at_s: 1_767_225_600,
            },
            &ServerBinarySnapshotPayload {
                line_name: "main".to_string(),
                message: None,
            },
        )
        .expect("snapshot should append");
}

pub(super) fn create_native_binary_repository<D>(service: &BinaryDbNativeRepositoryService<D>)
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    service
        .create_repository(RepositoryCreateRequest {
            repo_name: "repo-bin".to_string(),
            default_line: "main".to_string(),
            policy: json!({}),
            id_namespace_prefix: None,
        })
        .expect("binary repository should be creatable");
}

#[derive(Clone, Debug)]
pub(super) struct SeededBinaryContent {
    pub(super) blob_id: String,
    pub(super) bytes: Vec<u8>,
    pub(super) object_pack_id: String,
    pub(super) tree_id: String,
    pub(super) tree_pack_id: String,
}

pub(super) fn seed_native_binary_content<D>(
    service: &BinaryDbNativeRepositoryService<D>,
    label: &str,
) -> SeededBinaryContent
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    let bytes = format!("schema-defined Binary DB content for {label}\n").into_bytes();
    let sha256 = sha256_hex(&bytes);
    let blob_id = format!("BLB-{}", &sha256[..20]);
    let object_pack_id = "PCK-000000000001".to_string();
    let tree_id = "TRE-00000000000000000001".to_string();
    let tree_pack_id = "TPK-000000000001".to_string();
    let created_at = "2026-01-01T00:00:00Z";
    let root = env::temp_dir().join(format!(
        "ait-server-core-schema-content-{label}-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("stale schema content fixture should remove");
    }
    fs::create_dir_all(&root).expect("schema content fixture root should create");

    let object_pack_path = root.join(format!("{object_pack_id}.zstpack"));
    write_rebuilt_zstd_pack_archive(
        object_pack_path
            .to_str()
            .expect("object pack path should be UTF-8"),
        &object_pack_id,
        created_at,
        vec![ObjectPackRewriteBlob {
            entry_name: format!("blobs/{blob_id}"),
            blob_id: blob_id.clone(),
            data: bytes.clone(),
            path_hint: Some("README.md".to_string()),
        }],
        0,
    )
    .expect("canonical object zstd pack should write");
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                object_pack_id.clone(),
                fs::read(&object_pack_path).expect("object pack bytes should read"),
            )],
            false,
        )
        .expect("canonical object pack bytes should import");
    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "blob_id": blob_id,
                "sha256": sha256,
                "size_bytes": bytes.len(),
                "pack_id": object_pack_id,
                "pack_entry_name": format!("blobs/{blob_id}"),
                "pack_entry_type": "full",
                "pack_base_blob_id": JsonValue::Null,
                "pack_chain_depth": 0,
                "created_at": created_at,
            })],
            false,
        )
        .expect("canonical blob and object-pack records should import");

    let tree_rows = json!([{
        "tree_id": tree_id,
        "entry_count": 1,
    }]);
    let tree_entry_rows = json!([{
        "tree_id": tree_id,
        "entry_name": "README.md",
        "entry_type": "blob",
        "target_id": blob_id,
        "size_bytes": bytes.len(),
        "mode": "100644",
    }]);
    let members = build_tree_pack_members(&tree_rows, &tree_entry_rows)
        .expect("canonical tree pack members should build");
    let tree_pack_path = root.join(format!("{tree_pack_id}.zstpack"));
    let tree_pack_metadata = write_tree_pack_archive_with_format(
        tree_pack_path
            .to_str()
            .expect("tree pack path should be UTF-8"),
        &tree_pack_id,
        created_at,
        &members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("canonical tree zstd pack should write");
    let tree_checksum = tree_pack_metadata["pack_index"]["trees"][0]["checksum"]
        .as_str()
        .expect("tree checksum should exist")
        .to_string();
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                tree_pack_id.clone(),
                fs::read(&tree_pack_path).expect("tree pack bytes should read"),
            )],
            true,
        )
        .expect("canonical tree pack bytes should import");
    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "tree_id": tree_id,
                "entry_count": 1,
                "tree_pack_id": tree_pack_id,
                "tree_pack_checksum": tree_checksum,
                "created_at": created_at,
            })],
            true,
        )
        .expect("canonical tree/tree-entry/tree-name/tree-pack records should import");
    fs::remove_dir_all(&root).expect("temporary source packs should remove");

    SeededBinaryContent {
        blob_id,
        bytes,
        object_pack_id,
        tree_id,
        tree_pack_id,
    }
}
