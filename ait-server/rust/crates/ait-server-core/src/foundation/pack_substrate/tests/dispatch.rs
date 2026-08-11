use super::super::*;
use super::helpers::*;

#[test]
fn object_pack_dispatch_accepts_only_current_zstd_format() {
    let pack_path = temp_path_with_suffix("pack-dispatch", ".blobpack");
    let members = json!([
        {"entry_name": "blobs/BLB-1", "blob_id": "BLB-1", "data": [104,101,108,108,111,10], "entry_type": "full", "chain_depth": 0}
    ]);
    let archive = write_pack_archive_with_format(
        &pack_path,
        "PCK-DISPATCH",
        "2026-07-03T00:00:00+00:00",
        &members,
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let index = read_pack_index_with_format(&pack_path, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1).unwrap();
    assert_eq!(
        object_pack_backend_from_persisted_format(PACK_FORMAT_ZSTD_CHUNKED_V1)
            .unwrap()
            .format_kind(),
        PackFormatKind::ZstdChunkedV1
    );
    assert_eq!(archive["pack_format"], PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(index["pack_id"], "PCK-DISPATCH");
    assert_eq!(
        read_pack_entry_with_format(
            &pack_path,
            "blobs/BLB-1",
            None,
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        )
        .unwrap(),
        b"hello\n"
    );
    let unsupported =
        read_pack_index_with_format(&pack_path, "unsupported-object-pack").unwrap_err();
    assert!(unsupported.contains("Unsupported object pack format"));
    let _ = std::fs::remove_file(&pack_path);
}

#[test]
fn tree_pack_dispatch_accepts_only_current_zstd_format() {
    let pack_path = temp_path_with_suffix("treepack-dispatch", ".treepack");
    let members = build_tree_pack_members(
        &json!([
            {"tree_id": "TRE-ROOT", "entry_count": 1}
        ]),
        &json!([
            {"tree_id": "TRE-ROOT", "entry_name": "README.md", "entry_type": "blob", "target_id": "BLB-README", "size_bytes": 5, "mode": "0o644"}
        ])
    ).unwrap();
    let stats = write_tree_pack_archive_with_format(
        &pack_path,
        "TPK-DISPATCH",
        "2026-07-03T00:00:00+00:00",
        &members,
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let index = read_tree_pack_index_with_format(&pack_path, TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1)
        .unwrap();
    assert_eq!(
        tree_pack_backend_from_persisted_format(TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
            .unwrap()
            .format_kind(),
        TreePackFormatKind::ZstdChunkedTreeV1
    );
    assert_eq!(stats["pack_format"], TREE_PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(index["pack_id"], "TPK-DISPATCH");
    let rows = read_tree_pack_tree_with_format(
        &pack_path,
        "TRE-ROOT",
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    let unsupported =
        read_tree_pack_index_with_format(&pack_path, "unsupported-tree-pack").unwrap_err();
    assert!(unsupported.contains("Unsupported tree-pack format"));
    let _ = std::fs::remove_file(&pack_path);
}
