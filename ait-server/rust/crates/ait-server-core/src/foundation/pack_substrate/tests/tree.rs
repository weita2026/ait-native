use super::super::*;
use super::helpers::*;

#[test]
fn default_tree_pack_archive_is_current_zstd() {
    let pack_path = temp_path("treepack");
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
    let stats = write_tree_pack_archive(
        &pack_path,
        "TPK-TEST",
        "2026-04-16T00:00:00+00:00",
        &members,
    )
    .unwrap();
    let index = read_tree_pack_index(&pack_path).unwrap();
    let trees = index["trees"].as_array().unwrap();
    let root_rows = read_tree_pack_tree(&pack_path, "TRE-ROOT").unwrap();
    let child_by_ordinal = read_tree_pack_tree_by_ordinal(&pack_path, 0).unwrap();
    let root_by_ordinal = read_tree_pack_tree_by_ordinal(&pack_path, 1).unwrap();
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
    assert_eq!(stats["pack_format"], TREE_PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(index["pack_id"], "TPK-TEST");
    assert_eq!(trees[0]["entry_ordinal"], 0);
    assert_eq!(trees[1]["entry_ordinal"], 1);
    assert_eq!(root_rows.as_array().unwrap().len(), 2);
    assert_eq!(root_by_entry_name, root_rows);
    assert_eq!(child_cached.as_array().unwrap().len(), 1);
    assert_eq!(root_cached, root_rows);
    assert_eq!(child_by_ordinal["tree_id"], "TRE-CHILD");
    assert_eq!(root_by_ordinal["tree_id"], "TRE-ROOT");
    assert_eq!(
        tree_pack_contains_blob_ids(&pack_path, &json!(["BLB-MAIN", "BLB-MISSING"])).unwrap(),
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
    let _ = std::fs::remove_file(&pack_path);
}

#[test]
fn tree_pack_index_requires_complete_current_zstd_metadata() {
    let contract = TreePackIndexJson::stateless();
    let index = json!({
        "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        "index_entry_name": ZSTD_CHUNKED_TREE_INDEX_ENTRY_NAME,
        "trees": [{
            "tree_id": "TRE-ROOT",
            "entry_ordinal": 0,
            "entry_name": "trees/TRE-ROOT.json",
            "entry_count": 2,
            "byte_length": 12,
            "checksum": "tree-checksum",
        }],
    });
    assert_eq!(contract.entries_by_id(&index).unwrap().len(), 1);

    let mut missing_format = index.clone();
    missing_format
        .as_object_mut()
        .unwrap()
        .remove("pack_format");
    assert_eq!(
        contract.entries_by_id(&missing_format).unwrap_err(),
        "missing required field: pack_format"
    );

    let mut missing_ordinal = index;
    missing_ordinal["trees"][0]
        .as_object_mut()
        .unwrap()
        .remove("entry_ordinal");
    assert_eq!(
        contract.entries_by_id(&missing_ordinal).unwrap_err(),
        "missing required integer field: entry_ordinal"
    );
}

#[test]
fn tree_pack_index_json_converts_zstd_fixture() {
    let contract = TreePackIndexJson::stateless();
    let pack_index = ZstdChunkedPackIndex {
        pack_format: TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
        pack_id: "TPK-ZSTD-FIXTURE".to_string(),
        created_at: "2026-07-05T00:00:00Z".to_string(),
        index_entry_name: ZSTD_CHUNKED_TREE_INDEX_ENTRY_NAME.to_string(),
        chunks: vec![ZstdChunkedChunkIndex {
            chunk_ordinal: 0,
            compressed_offset: 0,
            compressed_len: 12,
            raw_len: 12,
            checksum: "chunk-checksum".to_string(),
        }],
        members: vec![ZstdChunkedMemberIndex {
            member_ordinal: 0,
            entry_name: "trees/TRE-ROOT.json".to_string(),
            content_id: "TRE-ROOT".to_string(),
            entry_type: "tree".to_string(),
            entry_count: Some(2),
            base_content_id: None,
            delta_algorithm: None,
            chain_depth: 0,
            chunk_ordinal: 0,
            in_chunk_offset: 0,
            stored_len: 12,
            logical_len: 12,
            checksum: "tree-checksum".to_string(),
        }],
    };

    let pack_index_json = contract.zstd_chunked_index_json(&pack_index).unwrap();
    assert_eq!(
        pack_index_json["pack_format"],
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1
    );
    assert_eq!(pack_index_json["trees"][0]["tree_id"], "TRE-ROOT");
}
