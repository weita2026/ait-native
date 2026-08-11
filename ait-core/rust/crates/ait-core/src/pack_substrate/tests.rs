use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_path(name: &str) -> String {
    temp_path_with_suffix(name, ".zstpack")
}

fn temp_path_with_suffix(name: &str, suffix: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!("ait-{name}-{unique}{suffix}"))
        .to_string_lossy()
        .into_owned()
}

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

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
fn pack_archive_round_trip_and_index_lookup() {
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
fn typed_object_pack_writer_preserves_full_and_delta_bytes() {
    let pack_path = temp_path("typed-pack");
    let base = b"alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n".to_vec();
    let target = b"alpha\nbeta changed\ngamma\ndelta\nepsilon\nzeta\n".to_vec();
    let delta = build_git_binary_delta(&base, &target);
    let members = vec![
        ObjectPackWriteMember {
            entry_name: "blobs/BLB-BASE".to_string(),
            blob_id: "BLB-BASE".to_string(),
            data: base.clone(),
            logical_data: None,
            entry_type: "full".to_string(),
            base_blob_id: None,
            chain_depth: 0,
            delta_algorithm: None,
        },
        ObjectPackWriteMember {
            entry_name: "blobs/BLB-TARGET".to_string(),
            blob_id: "BLB-TARGET".to_string(),
            data: delta,
            logical_data: Some(target.clone()),
            entry_type: "delta".to_string(),
            base_blob_id: Some("BLB-BASE".to_string()),
            chain_depth: 1,
            delta_algorithm: Some(PACK_DELTA_GIT_BINARY_V1.to_string()),
        },
    ];
    write_typed_pack_archive_with_format(
        &pack_path,
        "PCK-TYPED",
        "2026-07-12T00:00:00Z",
        &members,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let index = read_pack_index_with_format(&pack_path, PACK_FORMAT_ZSTD_CHUNKED_V1).unwrap();
    assert_eq!(index["entries"][0]["entry_type"], "full");
    assert_eq!(index["entries"][1]["entry_type"], "delta");
    assert_eq!(index["entries"][1]["base_blob_id"], "BLB-BASE");
    let base_map = BTreeMap::from([("BLB-BASE".to_string(), base)]);
    assert_eq!(
        read_pack_entry_with_format(
            &pack_path,
            "blobs/BLB-TARGET",
            Some(&base_map),
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .unwrap(),
        target
    );
    let _ = std::fs::remove_file(&pack_path);
}

#[test]
fn typed_and_json_pack_assembly_write_identical_archives() {
    let json_pack_path = temp_path("json-assembly-pack");
    let typed_pack_path = temp_path("typed-assembly-pack");
    let base = b"alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n".to_vec();
    let target = b"alpha\nbeta changed\ngamma\ndelta\nepsilon\nzeta\n".to_vec();
    let json_items = json!([
        {
            "entry_name": "blobs/BLB-BASE",
            "blob_id": "BLB-BASE",
            "data": base.clone(),
            "path_hint": "docs/demo.md"
        },
        {
            "entry_name": "blobs/BLB-TARGET",
            "blob_id": "BLB-TARGET",
            "data": target.clone(),
            "path_hint": "docs/demo.md"
        }
    ]);
    let json_members =
        build_pack_members(&json_items, DEFAULT_MAX_DELTA_CHAIN_DEPTH, None).unwrap();
    let typed_members = build_typed_pack_members(
        vec![
            PackCandidate {
                entry_name: "blobs/BLB-BASE".to_string(),
                blob_id: "BLB-BASE".to_string(),
                data: base,
                path_hint: Some("docs/demo.md".to_string()),
                chain_depth: 0,
            },
            PackCandidate {
                entry_name: "blobs/BLB-TARGET".to_string(),
                blob_id: "BLB-TARGET".to_string(),
                data: target.clone(),
                path_hint: Some("docs/demo.md".to_string()),
                chain_depth: 0,
            },
        ],
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        None,
    );
    assert_eq!(typed_members.len(), 2);
    assert_eq!(typed_members[0].entry_type, "full");
    assert_eq!(typed_members[0].logical_data, None);
    assert_eq!(typed_members[1].entry_type, "delta");
    assert_eq!(
        typed_members[1].logical_data.as_deref(),
        Some(target.as_slice())
    );
    assert_eq!(
        JsonValue::Array(typed_members.iter().map(member_to_json).collect()),
        json_members
    );

    for (path, write) in [
        (json_pack_path.as_str(), false),
        (typed_pack_path.as_str(), true),
    ] {
        if write {
            write_typed_pack_archive_with_format(
                path,
                "PCK-ASSEMBLY-COMPAT",
                CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
                &typed_members,
                PACK_FORMAT_ZSTD_CHUNKED_V1,
            )
            .unwrap();
        } else {
            write_pack_archive_with_format(
                path,
                "PCK-ASSEMBLY-COMPAT",
                CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
                &json_members,
                PACK_FORMAT_ZSTD_CHUNKED_V1,
            )
            .unwrap();
        }
    }
    assert_eq!(
        std::fs::read(&typed_pack_path).unwrap(),
        std::fs::read(&json_pack_path).unwrap()
    );
    assert_eq!(
        read_pack_entry_with_format(
            &typed_pack_path,
            "blobs/BLB-TARGET",
            None,
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .unwrap(),
        target
    );
    let _ = std::fs::remove_file(&json_pack_path);
    let _ = std::fs::remove_file(&typed_pack_path);
}

#[test]
fn typed_and_json_parent_delta_candidates_select_the_same_member() {
    let base = b"alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n".to_vec();
    let target = b"alpha\nbeta changed\ngamma\ndelta\nepsilon\nzeta\n".to_vec();
    let json_members = build_pack_members(
        &json!([{
            "entry_name": "blobs/BLB-TARGET",
            "blob_id": "BLB-TARGET",
            "data": target.clone(),
            "path_hint": "docs/demo.md"
        }]),
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        Some(&json!({
            "docs/demo.md": {
                "blob_id": "BLB-BASE",
                "data": base.clone(),
                "chain_depth": 0
            }
        })),
    )
    .unwrap();
    let initial_by_path = BTreeMap::from([(
        "docs/demo.md".to_string(),
        PackCandidate {
            entry_name: "blobs/BLB-BASE".to_string(),
            blob_id: "BLB-BASE".to_string(),
            data: base,
            path_hint: Some("docs/demo.md".to_string()),
            chain_depth: 0,
        },
    )]);
    let typed_members = build_typed_pack_members(
        vec![PackCandidate {
            entry_name: "blobs/BLB-TARGET".to_string(),
            blob_id: "BLB-TARGET".to_string(),
            data: target.clone(),
            path_hint: Some("docs/demo.md".to_string()),
            chain_depth: 0,
        }],
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        Some(&initial_by_path),
    );

    assert_eq!(typed_members.len(), 1);
    assert_eq!(typed_members[0].entry_type, "delta");
    assert_eq!(typed_members[0].base_blob_id.as_deref(), Some("BLB-BASE"));
    assert_eq!(typed_members[0].chain_depth, 1);
    assert_eq!(
        typed_members[0].logical_data.as_deref(),
        Some(target.as_slice())
    );
    assert_eq!(
        JsonValue::Array(typed_members.iter().map(member_to_json).collect()),
        json_members
    );
}

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
    let _ = std::fs::remove_file(&pack_path);
}

#[test]
fn content_addressed_object_pack_reuse_accepts_only_historical_index_timestamp_drift() {
    let local_path = temp_path_with_suffix("content-addressed-local-object", ".zstpack");
    let remote_path = temp_path_with_suffix("content-addressed-remote-object", ".zstpack");
    let members = build_pack_members(
        &json!([{
            "entry_name": "blobs/BLB-CONTENT",
            "blob_id": "BLB-CONTENT",
            "data": [35,32,80,108,97,110,10],
            "path_hint": "docs/plan.md"
        }]),
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        None,
    )
    .unwrap();
    for (path, created_at) in [
        (local_path.as_str(), CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT),
        (remote_path.as_str(), "2026-07-11T07:13:38Z"),
    ] {
        write_pack_archive_with_format(
            path,
            "PCK-CONTENT-ADDRESS",
            created_at,
            &members,
            PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        )
        .unwrap();
    }
    let local_bytes = std::fs::read(&local_path).unwrap();
    let remote_bytes = std::fs::read(&remote_path).unwrap();
    assert_ne!(local_bytes, remote_bytes);
    let expected_remote_checksum =
        pack_index_checksum_with_format(&remote_path, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1)
            .unwrap()
            .unwrap();
    assert_eq!(
        validate_content_addressed_zstd_pack_reuse(
            &local_bytes,
            &remote_bytes,
            "PCK-CONTENT-ADDRESS",
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .unwrap(),
        expected_remote_checksum
    );

    let error = validate_content_addressed_zstd_pack_reuse(
        &remote_bytes,
        &local_bytes,
        "PCK-CONTENT-ADDRESS",
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap_err();
    assert!(error.contains("not canonical"), "{error}");
    let _ = std::fs::remove_file(local_path);
    let _ = std::fs::remove_file(remote_path);
}

#[test]
fn content_addressed_object_pack_reuse_rejects_content_drift_and_corruption() {
    let local_path = temp_path_with_suffix("content-addressed-drift-local", ".zstpack");
    let drifted_path = temp_path_with_suffix("content-addressed-drift-remote", ".zstpack");
    let build = |data: Vec<u8>| {
        build_pack_members(
            &json!([{
                "entry_name": "blobs/BLB-CONTENT",
                "blob_id": "BLB-CONTENT",
                "data": data,
                "path_hint": "docs/plan.md"
            }]),
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            None,
        )
        .unwrap()
    };
    write_pack_archive_with_format(
        &local_path,
        "PCK-CONTENT-DRIFT",
        CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
        &build(b"canonical\n".to_vec()),
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    write_pack_archive_with_format(
        &drifted_path,
        "PCK-CONTENT-DRIFT",
        "2026-07-11T07:13:38Z",
        &build(b"different\n".to_vec()),
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let local_bytes = std::fs::read(&local_path).unwrap();
    let drifted_bytes = std::fs::read(&drifted_path).unwrap();
    let error = validate_content_addressed_zstd_pack_reuse(
        &local_bytes,
        &drifted_bytes,
        "PCK-CONTENT-DRIFT",
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap_err();
    assert!(error.contains("beyond index created_at"), "{error}");

    let mut corrupt_bytes = local_bytes.clone();
    corrupt_bytes[0] ^= 0xff;
    let error = validate_content_addressed_zstd_pack_reuse(
        &local_bytes,
        &corrupt_bytes,
        "PCK-CONTENT-DRIFT",
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap_err();
    assert!(error.contains("Remote zstd pack"), "{error}");
    let _ = std::fs::remove_file(local_path);
    let _ = std::fs::remove_file(drifted_path);
}

#[test]
fn content_addressed_tree_pack_reuse_returns_remote_index_checksum() {
    let local_path = temp_path_with_suffix("content-addressed-local-tree", ".zstpack");
    let remote_path = temp_path_with_suffix("content-addressed-remote-tree", ".zstpack");
    let members = build_tree_pack_members(
        &json!([{"tree_id": "TRE-ROOT", "entry_count": 1}]),
        &json!([{
            "tree_id": "TRE-ROOT",
            "entry_name": "README.md",
            "entry_type": "blob",
            "target_id": "BLB-README",
            "size_bytes": 5,
            "mode": "0o644"
        }]),
    )
    .unwrap();
    for (path, created_at) in [
        (local_path.as_str(), CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT),
        (remote_path.as_str(), "2026-07-11T07:13:38Z"),
    ] {
        write_tree_pack_archive_with_format(
            path,
            "TPK-CONTENT-ADDRESS",
            created_at,
            &members,
            TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
        )
        .unwrap();
    }
    let expected_remote_checksum =
        tree_pack_index_checksum_with_format(&remote_path, TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1)
            .unwrap()
            .unwrap();
    assert_eq!(
        validate_content_addressed_zstd_pack_reuse(
            &std::fs::read(&local_path).unwrap(),
            &std::fs::read(&remote_path).unwrap(),
            "TPK-CONTENT-ADDRESS",
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .unwrap(),
        expected_remote_checksum
    );
    let _ = std::fs::remove_file(local_path);
    let _ = std::fs::remove_file(remote_path);
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
    assert_eq!(
        pack_index_json,
        json!({
            "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
            "pack_id": "PCK-ZSTD-FIXTURE",
            "created_at": "2026-07-05T00:00:00Z",
            "index_entry_name": ZSTD_CHUNKED_OBJECT_INDEX_ENTRY_NAME,
            "member_count": 1,
            "total_bytes": 6,
            "chunk_count": 1,
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
        })
    );

    let mut invalid_pack_index = pack_index.clone();
    invalid_pack_index.members[0].member_ordinal = 1;
    let error = contract
        .zstd_chunked_index_json(&invalid_pack_index)
        .unwrap_err();
    assert_eq!(
        error,
        "Invalid zstd chunked pack: non-sequential member ordinal 1"
    );
}

#[test]
fn zstd_object_pack_random_read_preserves_checksum_and_size_mismatch_errors() {
    let pack_path = temp_path_with_suffix("pack-zstd-mismatch", ".zstpack");
    let members = json!([
        {
            "entry_name": "blobs/BLB-1",
            "blob_id": "BLB-1",
            "data": [104,101,108,108,111,10],
            "entry_type": "full",
            "chain_depth": 0
        }
    ]);
    write_pack_archive_with_format(
        &pack_path,
        "PCK-ZSTD-MISMATCH",
        "2026-07-05T00:00:00Z",
        &members,
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();

    let pack_index = read_zstd_chunked_container_index(
        &pack_path,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();

    let mut bad_checksum_index = pack_index.clone();
    bad_checksum_index.members[0].checksum = "not-the-checksum".to_string();
    let checksum_error = read_zstd_chunked_object_entry(
        &pack_path,
        &bad_checksum_index,
        "blobs/BLB-1",
        None,
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        &mut BTreeSet::new(),
        0,
    )
    .unwrap_err();
    assert_eq!(
        checksum_error,
        "Pack entry checksum mismatch for blobs/BLB-1"
    );

    let mut bad_size_index = pack_index.clone();
    bad_size_index.members[0].logical_len = 99;
    let size_error = read_zstd_chunked_object_entry(
        &pack_path,
        &bad_size_index,
        "blobs/BLB-1",
        None,
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        &mut BTreeSet::new(),
        0,
    )
    .unwrap_err();
    assert_eq!(size_error, "Pack entry size mismatch for blobs/BLB-1");

    let _ = std::fs::remove_file(&pack_path);
}

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
fn cached_zstd_pack_entry_archive_reuses_one_decoded_chunk_for_multiple_reads() {
    let pack_path = temp_path("pack-cache");
    let members = json!([
        {"entry_name": "blobs/BLB-1", "blob_id": "BLB-1", "data": [104,101,108,108,111,10], "entry_type": "full", "chain_depth": 0},
        {"entry_name": "blobs/BLB-2", "blob_id": "BLB-2", "data": [119,111,114,108,100,10], "entry_type": "full", "chain_depth": 0}
    ]);
    write_pack_archive_with_format(
        &pack_path,
        "PCK-CACHE",
        "2026-06-08T00:00:00+00:00",
        &members,
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let mut reader =
        PackEntryArchive::open_with_format(&pack_path, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1).unwrap();
    assert!(reader.has_entry("blobs/BLB-1"));
    assert_eq!(
        reader
            .read_entry("blobs/BLB-1", None, DEFAULT_MAX_DELTA_CHAIN_DEPTH)
            .unwrap(),
        b"hello\n"
    );
    std::fs::remove_file(&pack_path).unwrap();
    assert_eq!(
        reader
            .read_entry("blobs/BLB-2", None, DEFAULT_MAX_DELTA_CHAIN_DEPTH)
            .unwrap(),
        b"world\n"
    );
}

#[test]
fn read_pack_entry_rejects_unsupported_algorithm() {
    let pack_path = temp_path("unsupported-algorithm");
    let members = json!([
        {"entry_name": "blobs/BLB-BASE", "blob_id": "BLB-BASE", "data": [104,101,108,108,111,10], "entry_type": "full", "chain_depth": 0},
        {"entry_name": "blobs/BLB-TARGET", "blob_id": "BLB-TARGET", "data": [1,2,3], "logical_data": [104,101,108,108,111,32,119,111,114,108,100,10], "entry_type": "delta", "base_blob_id": "BLB-BASE", "chain_depth": 1, "delta_algorithm": "text-line-v1"}
    ]);
    write_pack_archive(
        &pack_path,
        "PCK-UNSUPPORTED",
        "2026-04-13T00:00:00+00:00",
        &members,
    )
    .unwrap();
    let err = read_pack_entry(
        &pack_path,
        "blobs/BLB-TARGET",
        None,
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
    )
    .unwrap_err();
    assert!(err.contains("Unsupported pack delta algorithm"));
    let _ = std::fs::remove_file(&pack_path);
}

#[test]
fn tree_pack_round_trip() {
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
    assert_eq!(child_by_ordinal["entry_ordinal"], 0);
    assert_eq!(root_by_ordinal["tree_id"], "TRE-ROOT");
    assert_eq!(root_by_ordinal["entry_ordinal"], 1);
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
    assert_eq!(stats["pack_format"], TREE_PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(stats["pack_index_entry_name"], "zstd-chunked-tree-index");
    assert_eq!(index["pack_id"], "TPK-ZSTD");
    assert_eq!(index["pack_format"], TREE_PACK_FORMAT_ZSTD_CHUNKED_V1);
    assert_eq!(root_rows.as_array().unwrap().len(), 2);
    assert_eq!(root_by_entry_name, root_rows);
    assert_eq!(child_cached.as_array().unwrap().len(), 1);
    assert_eq!(root_cached, root_rows);
    assert_eq!(child_by_ordinal["tree_id"], "TRE-CHILD");
    assert_eq!(child_by_ordinal["entry_ordinal"], 0);
    assert_eq!(root_by_ordinal["tree_id"], "TRE-ROOT");
    assert_eq!(root_by_ordinal["entry_ordinal"], 1);
    let bad_count = read_tree_pack_tree_by_entry_name_with_format(
        &pack_path,
        "TRE-ROOT",
        root_entry["entry_name"].as_str().unwrap(),
        root_entry["entry_count"].as_u64().unwrap() as usize + 1,
        root_entry["checksum"].as_str().unwrap(),
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap_err();
    assert!(bad_count.contains("Tree pack entry count mismatch"));
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

#[test]
fn zstd_object_and_tree_pack_behaviors_share_current_substrate_contracts() {
    let zstd_object_path = temp_path_with_suffix("mirror-object-zstd", ".zstpack");
    let items = json!([
        {
            "entry_name": "blobs/BLB-BASE",
            "blob_id": "BLB-BASE",
            "data": [97,108,112,104,97,10,98,101,116,97,10,103,97,109,109,97,10],
            "path_hint": "docs/demo.md"
        },
        {
            "entry_name": "blobs/BLB-TARGET",
            "blob_id": "BLB-TARGET",
            "data": [97,108,112,104,97,10,98,101,116,97,32,99,104,97,110,103,101,100,10,103,97,109,109,97,10],
            "path_hint": "docs/demo.md"
        }
    ]);
    let members = build_pack_members(&items, DEFAULT_MAX_DELTA_CHAIN_DEPTH, None).unwrap();
    write_pack_archive_with_format(
        &zstd_object_path,
        "PCK-MIRROR-ZSTD",
        "2026-07-07T00:00:00Z",
        &members,
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let zstd_index =
        read_pack_index_with_format(&zstd_object_path, PACK_FORMAT_KIND_ZSTD_CHUNKED_V1).unwrap();
    assert_eq!(zstd_index["member_count"], 2);
    assert!(pack_has_entry_with_format(
        &zstd_object_path,
        "blobs/BLB-TARGET",
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap());
    let zstd_target = read_pack_entry_with_format(
        &zstd_object_path,
        "blobs/BLB-TARGET",
        None,
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    assert_eq!(zstd_target, b"alpha\nbeta changed\ngamma\n");

    let zstd_tree_path = temp_path_with_suffix("mirror-tree-zstd", ".zstpack");
    let tree_members = build_tree_pack_members(
        &json!([
            {"tree_id": "TRE-CHILD", "entry_count": 1},
            {"tree_id": "TRE-ROOT", "entry_count": 2}
        ]),
        &json!([
            {"tree_id": "TRE-ROOT", "entry_name": "README.md", "entry_type": "blob", "target_id": "BLB-README", "size_bytes": 5, "mode": "0o644"},
            {"tree_id": "TRE-ROOT", "entry_name": "nested", "entry_type": "tree", "target_id": "TRE-CHILD", "size_bytes": null, "mode": "tree"},
            {"tree_id": "TRE-CHILD", "entry_name": "main.py", "entry_type": "blob", "target_id": "BLB-MAIN", "size_bytes": 11, "mode": "0o755"}
        ]),
    )
    .unwrap();
    write_tree_pack_archive_with_format(
        &zstd_tree_path,
        "TPK-MIRROR-ZSTD",
        "2026-07-07T00:00:00Z",
        &tree_members,
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let zstd_root = read_tree_pack_tree_with_format(
        &zstd_tree_path,
        "TRE-ROOT",
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    assert_eq!(zstd_root.as_array().unwrap().len(), 2);
    let zstd_root_by_ordinal = read_tree_pack_tree_by_ordinal_with_format(
        &zstd_tree_path,
        1,
        TREE_PACK_FORMAT_KIND_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    assert_eq!(zstd_root_by_ordinal["tree_id"], "TRE-ROOT");
    assert_eq!(zstd_root_by_ordinal["rows"].as_array().unwrap().len(), 2);

    let _ = std::fs::remove_file(&zstd_object_path);
    let _ = std::fs::remove_file(&zstd_tree_path);
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
        pack_index_json,
        json!({
            "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
            "pack_id": "TPK-ZSTD-FIXTURE",
            "created_at": "2026-07-05T00:00:00Z",
            "index_entry_name": ZSTD_CHUNKED_TREE_INDEX_ENTRY_NAME,
            "tree_count": 1,
            "total_bytes": 12,
            "chunk_count": 1,
            "trees": [{
                "tree_id": "TRE-ROOT",
                "entry_ordinal": 0,
                "entry_name": "trees/TRE-ROOT.json",
                "entry_count": 2,
                "byte_length": 12,
                "checksum": "tree-checksum",
            }],
        })
    );
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

#[test]
fn storage_validation_summary_matches_packed_full_only_case() {
    let summary = build_storage_validation_summary(4, 4, 0, 1, 0, 0, 0.5, 0, 1, None);
    assert_eq!(summary["state"], "packed_full_only");
    assert_eq!(summary["recommended_action"], "none");
    assert_eq!(summary["next_actions"], json!([]));
}
