use super::super::*;
use super::helpers::*;

#[test]
fn git_binary_delta_round_trip() {
    let base = b"alpha\nbeta\ngamma\n";
    let target = b"alpha\nbeta changed\ngamma\ndelta\n";
    let delta = build_git_binary_delta(base, target);
    let replayed = apply_git_binary_delta(base, &delta).unwrap();
    assert_eq!(replayed, target);
}

#[test]
fn build_pack_members_emits_delta_when_savings_threshold_is_met() {
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
    let rows = members.as_array().unwrap();
    assert_eq!(rows[0]["entry_type"], "full");
    assert_eq!(rows[1]["entry_type"], "delta");
    assert_eq!(rows[1]["delta_algorithm"], PACK_DELTA_GIT_BINARY_V1);
}

#[test]
fn default_pack_archive_is_current_zstd_and_supports_index_lookup() {
    let pack_path = temp_path("pack");
    let members = json!([
        {"entry_name": "blobs/BLB-1", "blob_id": "BLB-1", "data": [104,101,108,108,111,10], "entry_type": "full", "chain_depth": 0},
        {"entry_name": "blobs/BLB-2", "blob_id": "BLB-2", "data": [119,111,114,108,100,10], "entry_type": "full", "chain_depth": 0}
    ]);
    let archive = write_pack_archive(
        &pack_path,
        "PCK-TEST",
        "2026-04-12T00:00:00+00:00",
        &members,
    )
    .unwrap();
    let index = read_pack_index(&pack_path).unwrap();
    assert_eq!(archive["pack_format"], PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(index["pack_id"], "PCK-TEST");
    assert!(pack_has_entry(&pack_path, "blobs/BLB-1"));
    assert_eq!(
        read_pack_entry(
            &pack_path,
            "blobs/BLB-2",
            None,
            DEFAULT_MAX_DELTA_CHAIN_DEPTH
        )
        .unwrap(),
        b"world\n"
    );
    let _ = std::fs::remove_file(&pack_path);
}

#[test]
fn direct_zstd_rewrite_builds_bounded_delta_without_json_byte_arrays() {
    let pack_path = temp_path("direct-zstd-rewrite");
    let base = vec![b'a'; 512];
    let mut target = base.clone();
    target[240..248].copy_from_slice(b"changed!");
    let metadata = write_rebuilt_zstd_pack_archive(
        &pack_path,
        "PCK-REWRITE",
        "2026-07-12T00:00:00Z",
        vec![
            ObjectPackRewriteBlob {
                entry_name: "blobs/BLB-BASE".to_string(),
                blob_id: "BLB-BASE".to_string(),
                data: base,
                path_hint: Some("chain-a".to_string()),
            },
            ObjectPackRewriteBlob {
                entry_name: "blobs/BLB-TARGET".to_string(),
                blob_id: "BLB-TARGET".to_string(),
                data: target.clone(),
                path_hint: Some("chain-a".to_string()),
            },
        ],
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
    )
    .unwrap();
    assert_eq!(metadata["pack_index"]["entries"][0]["entry_type"], "full");
    assert_eq!(metadata["pack_index"]["entries"][1]["entry_type"], "delta");
    assert_eq!(metadata["pack_index"]["entries"][1]["chain_depth"], 1);
    assert_eq!(
        read_pack_entry_with_format(
            &pack_path,
            "blobs/BLB-TARGET",
            None,
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .unwrap(),
        target
    );
    let _ = std::fs::remove_file(&pack_path);
}

#[test]
fn object_pack_index_requires_complete_current_zstd_metadata() {
    let contract = ObjectPackIndexJson::stateless();
    let index = json!({
        "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
        "index_entry_name": ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME,
        "entries": [{
            "entry_name": "blobs/BLB-1",
            "blob_id": "BLB-1",
            "entry_type": "full",
            "byte_length": 6,
            "uncompressed_byte_length": 6,
            "base_blob_id": null,
            "chain_depth": 0,
            "checksum": sha256_hex(b"hello\n"),
        }],
    });
    assert_eq!(contract.entries_by_name(&index).unwrap().len(), 1);

    let mut missing_format = index.clone();
    missing_format
        .as_object_mut()
        .unwrap()
        .remove("pack_format");
    assert_eq!(
        contract.entries_by_name(&missing_format).unwrap_err(),
        "missing required field: pack_format"
    );

    let mut missing_chain_depth = index;
    missing_chain_depth["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("chain_depth");
    assert_eq!(
        contract.entries_by_name(&missing_chain_depth).unwrap_err(),
        "missing required integer field: chain_depth"
    );
}

#[test]
fn object_pack_index_json_converts_zstd_fixture_and_rejects_invalid_member_ordinal() {
    let contract = ObjectPackIndexJson::stateless();
    let pack_index = ZstdChunkedPackIndex {
        pack_format: PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
        pack_id: "PCK-ZSTD-FIXTURE".to_string(),
        created_at: "2026-07-05T00:00:00Z".to_string(),
        index_entry_name: ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME.to_string(),
        chunks: vec![ZstdChunkedChunkIndex {
            chunk_ordinal: 0,
            compressed_offset: 0,
            compressed_len: 8,
            raw_len: 6,
            checksum: "chunk-checksum".to_string(),
        }],
        members: vec![ZstdChunkedMemberIndex {
            member_ordinal: 0,
            entry_name: "blobs/BLB-1".to_string(),
            content_id: "BLB-1".to_string(),
            entry_type: "full".to_string(),
            entry_count: None,
            base_content_id: None,
            delta_algorithm: None,
            chain_depth: 0,
            chunk_ordinal: 0,
            in_chunk_offset: 0,
            stored_len: 6,
            logical_len: 6,
            checksum: sha256_hex(b"hello\n"),
        }],
    };

    let pack_index_json = contract.zstd_chunked_index_json(&pack_index).unwrap();
    assert_eq!(pack_index_json["pack_format"], PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(pack_index_json["entries"][0]["blob_id"], "BLB-1");

    let mut invalid_pack_index = pack_index;
    invalid_pack_index.members[0].member_ordinal = 1;
    assert_eq!(
        contract
            .zstd_chunked_index_json(&invalid_pack_index)
            .unwrap_err(),
        "Invalid zstd chunked pack: non-sequential member ordinal 1"
    );
}
