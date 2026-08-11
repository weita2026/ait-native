use super::super::*;
use super::helpers::*;

#[test]
fn object_pack_round_trip_with_zstd_chunked_format() {
    let pack_path = temp_path_with_suffix("pack-zstd", ".zstpack");
    let items = json!([
        {
            "entry_name": "blobs/BLB-BASE",
            "blob_id": "BLB-BASE",
            "data": [97,108,112,104,97,10,98,101,116,97,10,103,97,109,109,97,10,100,101,108,116,97,10,101,112,115,105,108,111,110,10,122,101,116,97,10],
            "path_hint": "docs/demo.md"
        },
        {
            "entry_name": "blobs/BLB-TARGET",
            "blob_id": "BLB-TARGET",
            "data": [97,108,112,104,97,10,98,101,116,97,32,99,104,97,110,103,101,100,10,103,97,109,109,97,10,100,101,108,116,97,10,101,112,115,105,108,111,110,10,122,101,116,97,10],
            "path_hint": "docs/demo.md"
        }
    ]);
    let members = build_pack_members(&items, DEFAULT_MAX_DELTA_CHAIN_DEPTH, None).unwrap();
    let archive = write_pack_archive_with_format(
        &pack_path,
        "PCK-ZSTD",
        "2026-07-03T00:00:00+00:00",
        &members,
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let index = read_pack_index_with_format(&pack_path, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1).unwrap();
    assert_eq!(archive["pack_format"], PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(
        archive["pack_index_entry_name"],
        "zstd-chunked-object-index"
    );
    assert_eq!(index["pack_id"], "PCK-ZSTD");
    assert_eq!(index["pack_format"], PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert!(pack_has_entry_with_format(
        &pack_path,
        "blobs/BLB-TARGET",
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap());
    assert_eq!(
        read_pack_entry_with_format(
            &pack_path,
            "blobs/BLB-TARGET",
            None,
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        )
        .unwrap(),
        b"alpha\nbeta changed\ngamma\ndelta\nepsilon\nzeta\n"
    );
    let mut reader =
        PackEntryArchive::open_with_format(&pack_path, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1).unwrap();
    assert!(reader.has_entry("blobs/BLB-BASE"));
    assert_eq!(
        reader
            .read_entry("blobs/BLB-BASE", None, DEFAULT_MAX_DELTA_CHAIN_DEPTH)
            .unwrap(),
        b"alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n"
    );
    let archive_bytes = std::fs::read(&pack_path).unwrap();
    let retired_index_entry_name = ["pack", "-index.json"].concat();
    assert!(!bytes_contain(
        &archive_bytes,
        retired_index_entry_name.as_bytes()
    ));
    let _ = std::fs::remove_file(&pack_path);
}

#[test]
fn object_pack_zstd_random_read_across_chunks() {
    let pack_path = temp_path_with_suffix("pack-zstd-random-read", ".zstpack");
    let first = "a".repeat(900 * 1024);
    let second = "b".repeat(900 * 1024);
    let third = "c".repeat(32 * 1024);
    let members = json!([
        {"entry_name": "blobs/BLB-FIRST", "blob_id": "BLB-FIRST", "data": first, "entry_type": "full", "chain_depth": 0},
        {"entry_name": "blobs/BLB-SECOND", "blob_id": "BLB-SECOND", "data": second, "entry_type": "full", "chain_depth": 0},
        {"entry_name": "blobs/BLB-THIRD", "blob_id": "BLB-THIRD", "data": third, "entry_type": "full", "chain_depth": 0}
    ]);
    unsafe {
        std::env::set_var("AIT_OBJECT_PACK_CHUNK_MIB", "1");
    }
    let archive = write_pack_archive_with_format(
        &pack_path,
        "PCK-ZSTD-RANDOM",
        "2026-07-03T00:00:00+00:00",
        &members,
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    unsafe {
        std::env::remove_var("AIT_OBJECT_PACK_CHUNK_MIB");
    }
    assert_eq!(archive["pack_format"], PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(archive["pack_index"]["chunk_count"], 2);

    let mut reader =
        PackEntryArchive::open_with_format(&pack_path, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1).unwrap();
    assert_eq!(
        reader
            .read_entry("blobs/BLB-SECOND", None, DEFAULT_MAX_DELTA_CHAIN_DEPTH)
            .unwrap(),
        vec![b'b'; 900 * 1024]
    );
    assert_eq!(
        reader
            .read_entry("blobs/BLB-THIRD", None, DEFAULT_MAX_DELTA_CHAIN_DEPTH)
            .unwrap(),
        vec![b'c'; 32 * 1024]
    );
    assert_eq!(reader.cached_zstd_chunk_count(), 1);
    assert_eq!(
        read_pack_entry_with_format(
            &pack_path,
            "blobs/BLB-FIRST",
            None,
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        )
        .unwrap(),
        vec![b'a'; 900 * 1024]
    );
    let _ = std::fs::remove_file(&pack_path);
}

#[test]
fn tree_pack_round_trip_with_zstd_chunked_format() {
    let pack_path = temp_path_with_suffix("treepack-zstd", ".zstpack");
    let members = build_tree_pack_members(
        &json!([
            {"tree_id": "TRE-CHILD", "entry_count": 1},
            {"tree_id": "TRE-ROOT", "entry_count": 2}
        ]),
        &json!([
            {"tree_id": "TRE-ROOT", "entry_name": "README.md", "entry_type": "blob", "target_id": "BLB-README", "size_bytes": 5, "mode": "0o644"},
            {"tree_id": "TRE-ROOT", "entry_name": "nested", "entry_type": "tree", "target_id": "TRE-CHILD", "size_bytes": null, "mode": "tree"},
            {"tree_id": "TRE-CHILD", "entry_name": "main.py", "entry_type": "blob", "target_id": "BLB-MAIN", "size_bytes": 11, "mode": "0o755"}
        ])
    ).unwrap();
    let stats = write_tree_pack_archive_with_format(
        &pack_path,
        "TPK-ZSTD",
        "2026-07-03T00:00:00+00:00",
        &members,
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let index = read_tree_pack_index_with_format(&pack_path, TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1)
        .unwrap();
    let trees = index["trees"].as_array().unwrap();
    let root_rows = read_tree_pack_tree_with_format(
        &pack_path,
        "TRE-ROOT",
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let child_by_ordinal = read_tree_pack_tree_by_ordinal_with_format(
        &pack_path,
        0,
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let root_by_ordinal = read_tree_pack_tree_by_ordinal_with_format(
        &pack_path,
        1,
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let child_entry = trees[0].as_object().unwrap();
    let root_entry = trees[1].as_object().unwrap();
    let root_by_entry_name = read_tree_pack_tree_by_entry_name_with_format(
        &pack_path,
        "TRE-ROOT",
        root_entry["entry_name"].as_str().unwrap(),
        root_entry["entry_count"].as_u64().unwrap() as usize,
        root_entry["checksum"].as_str().unwrap(),
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let mut cached_reader =
        TreePackEntryArchive::open_with_format(&pack_path, TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1)
            .unwrap();
    let child_cached = cached_reader
        .read_tree_by_entry_name(
            "TRE-CHILD",
            child_entry["entry_name"].as_str().unwrap(),
            child_entry["entry_count"].as_u64().unwrap() as usize,
            child_entry["checksum"].as_str().unwrap(),
        )
        .unwrap();
    let root_cached = cached_reader
        .read_tree_by_entry_name(
            "TRE-ROOT",
            root_entry["entry_name"].as_str().unwrap(),
            root_entry["entry_count"].as_u64().unwrap() as usize,
            root_entry["checksum"].as_str().unwrap(),
        )
        .unwrap();
    let child_cached_by_ordinal = cached_reader.read_tree_by_ordinal(0).unwrap();
    let root_cached_by_ordinal = cached_reader.read_tree_by_ordinal(1).unwrap();
    assert_eq!(stats["pack_format"], TREE_PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(stats["pack_index_entry_name"], "zstd-chunked-tree-index");
    assert_eq!(index["pack_id"], "TPK-ZSTD");
    assert_eq!(index["pack_format"], TREE_PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(root_rows.as_array().unwrap().len(), 2);
    assert_eq!(root_by_entry_name, root_rows);
    assert_eq!(child_cached.as_array().unwrap().len(), 1);
    assert_eq!(root_cached, root_rows);
    assert_eq!(child_cached_by_ordinal["tree_id"], "TRE-CHILD");
    assert_eq!(root_cached_by_ordinal["tree_id"], "TRE-ROOT");
    assert_eq!(root_cached_by_ordinal["rows"], root_rows);
    assert_eq!(cached_reader.cached_zstd_chunk_count(), 1);
    assert_eq!(child_by_ordinal["tree_id"], "TRE-CHILD");
    assert_eq!(child_by_ordinal["entry_ordinal"], 0);
    assert_eq!(root_by_ordinal["tree_id"], "TRE-ROOT");
    assert_eq!(root_by_ordinal["entry_ordinal"], 1);
    assert_eq!(
        tree_pack_contains_blob_ids_with_format(
            &pack_path,
            &json!(["BLB-MAIN", "BLB-MISSING"]),
            TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        )
        .unwrap(),
        json!({"matching_blob_ids": ["BLB-MAIN"]})
    );
    let bad_checksum = read_tree_pack_tree_by_entry_name_with_format(
        &pack_path,
        "TRE-ROOT",
        root_entry["entry_name"].as_str().unwrap(),
        root_entry["entry_count"].as_u64().unwrap() as usize,
        "not-the-checksum",
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap_err();
    assert!(bad_checksum.contains("Tree pack entry checksum mismatch"));
    let archive_bytes = std::fs::read(&pack_path).unwrap();
    let retired_index_entry_name = ["tree-pack", "-index.json"].concat();
    assert!(!bytes_contain(
        &archive_bytes,
        retired_index_entry_name.as_bytes()
    ));
    let pack_index = read_zstd_chunked_container_index(
        &pack_path,
        ZSTD_CHUNKED_INDEX_KIND_TREE,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let root_member = pack_index
        .members
        .iter()
        .find(|member| member.content_id == "TRE-ROOT")
        .unwrap();
    let root_payload =
        read_zstd_chunked_member_stored_bytes(&pack_path, &pack_index, root_member).unwrap();
    assert!(root_payload.starts_with(ZSTD_CHUNKED_TREE_MEMBER_MAGIC));
    assert!(!bytes_contain(&root_payload, br#""entries""#));
    assert!(!bytes_contain(&root_payload, br#""tree_id""#));
    let _ = std::fs::remove_file(&pack_path);
}
