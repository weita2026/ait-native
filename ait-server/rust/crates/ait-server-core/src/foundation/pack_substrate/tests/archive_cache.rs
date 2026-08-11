use super::super::*;
use super::helpers::*;

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
