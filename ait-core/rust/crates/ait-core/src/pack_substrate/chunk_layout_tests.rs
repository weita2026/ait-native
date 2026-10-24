use super::*;
fn full(body: &[u8]) -> ObjectPackWriteMember {
    let id = crate::content_binary_db::blob_id_from_sha256(&sha256_bytes(body));
    ObjectPackWriteMember {
        entry_name: format!("blobs/{id}"),
        blob_id: id,
        data: body.into(),
        logical_data: None,
        entry_type: "full".into(),
        base_blob_id: None,
        chain_depth: 0,
        delta_algorithm: None,
    }
}
fn identifier(members: &[ObjectPackWriteMember], layout: &ObjectPackChunkLayout) -> String {
    let mut seed = b"fixture/native-raw-blob-batch/v1".to_vec();
    for m in members {
        seed.extend_from_slice(&sha256_bytes(m.logical_data.as_deref().unwrap_or(&m.data)));
        seed.extend_from_slice(&sha256_bytes(&m.data));
    }
    layout.extend_identity_seed(members, &mut seed);
    format!("PCK-{}", sha256_hex(&seed)[..12].to_ascii_uppercase())
}
#[test]
fn partitioned_chunk_layout_legacy_delta_size_and_damage() {
    let d = tempfile::tempdir().unwrap();
    let base = (0..8192).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    let mut body = base.clone();
    body[4096] ^= 1;
    let b = full(&base);
    let mut target = full(&body);
    target.data = build_git_binary_delta(&base, &body);
    target.logical_data = Some(body.clone());
    target.entry_type = "delta".into();
    target.base_blob_id = Some(b.blob_id.clone());
    target.chain_depth = 1;
    target.delta_algorithm = Some(PACK_DELTA_GIT_BINARY_V1.into());
    let members = vec![b, target, full(&[]), full(b"tag payload")];
    let canonical = ObjectPackChunkLayout::for_members(&members, None);
    let priority = BTreeSet::from([members[1].blob_id.clone(), members[2].blob_id.clone()]);
    let layout = ObjectPackChunkLayout::for_members(&members, Some(&priority));
    let old_id = identifier(&members, &canonical);
    let new_id = identifier(&members, &layout);
    assert_ne!(old_id, new_id);
    let old = d.path().join("legacy.zstpack");
    write_typed_pack_archive_with_format(
        old.to_str().unwrap(),
        &old_id,
        CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
        &members,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let path = d.path().join(format!("{new_id}.zstpack"));
    write_typed_pack_archive_immutable_with_layout(
        path.to_str().unwrap(),
        &new_id,
        &members,
        &layout,
    )
    .unwrap();
    assert_eq!(
        estimated_typed_pack_archive_bytes(&members).unwrap(),
        std::fs::metadata(&old).unwrap().len() as usize
    );
    assert_eq!(
        estimated_typed_pack_archive_bytes_with_layout(&members, &layout).unwrap(),
        std::fs::metadata(&path).unwrap().len() as usize
    );
    let ix = read_zstd_chunked_container_index(
        path.to_str().unwrap(),
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    assert_eq!(ix.chunks.len(), 2);
    assert_eq!(
        ix.members
            .iter()
            .map(|m| m.member_ordinal)
            .collect::<Vec<_>>(),
        (0..members.len()).collect::<Vec<_>>()
    );
    assert!(ix.members[1].chunk_ordinal < ix.members[0].chunk_ordinal);
    for p in [&old, &path] {
        let mut r = PackEntryArchive::open(p.to_str().unwrap()).unwrap();
        for (m, expected) in members.iter().zip([
            base.as_slice(),
            body.as_slice(),
            b"".as_slice(),
            b"tag payload".as_slice(),
        ]) {
            assert_eq!(
                r.read_entry(&m.entry_name, None, MAX_DELTA_CHAIN_READ_DEPTH)
                    .unwrap(),
                expected
            );
        }
    }
    let bytes = std::fs::read(&path).unwrap();
    for (n, c) in ix.chunks.iter().enumerate() {
        let mut damaged = bytes.clone();
        damaged[c.compressed_offset as usize] ^= 0x80;
        let p = d.path().join(format!("damaged-{n}.zstpack"));
        std::fs::write(&p, damaged).unwrap();
        let mut r = PackEntryArchive::open(p.to_str().unwrap()).unwrap();
        assert!(r
            .read_entry(&members[1].entry_name, None, MAX_DELTA_CHAIN_READ_DEPTH)
            .is_err());
    }
    let mut damaged = bytes;
    *damaged.last_mut().unwrap() ^= 1;
    let p = d.path().join("bad-index.zstpack");
    std::fs::write(&p, damaged).unwrap();
    assert!(PackEntryArchive::open(p.to_str().unwrap()).is_err());
    let mut bad = ix;
    bad.members[0].in_chunk_offset = usize::MAX;
    assert!(validate_zstd_chunked_index(
        &bad,
        ZSTD_CHUNKED_INDEX_KIND_OBJECT,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
        None
    )
    .is_err());
    let unchanged = ObjectPackChunkLayout::for_members(
        &members,
        Some(&members.iter().map(|m| m.blob_id.clone()).collect()),
    );
    assert_eq!(unchanged, canonical);
    assert_eq!(identifier(&members, &unchanged), old_id);
    let canonical_copy = d.path().join("canonical-immutable.zstpack");
    write_typed_pack_archive_immutable_with_layout(
        canonical_copy.to_str().unwrap(),
        &old_id,
        &members,
        &unchanged,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&canonical_copy).unwrap(),
        std::fs::read(&old).unwrap()
    );
    assert!(ObjectPackChunkLayout {
        chunks: vec![vec![0, 0]]
    }
    .validate(2)
    .is_err());
    assert!(ObjectPackChunkLayout {
        chunks: vec![vec![1]]
    }
    .validate(2)
    .is_err());
    assert!(ObjectPackChunkLayout {
        chunks: vec![vec![]]
    }
    .validate(0)
    .is_err());
}

#[test]
fn partitioned_chunk_layout_respects_byte_limits_and_single_oversized_members() {
    let sizes = [3, 3, 8, 0, 2, 2];
    let classes = [false, true, true, false, false, true];
    let layout = ObjectPackChunkLayout::new(&sizes, Some(&classes), 4).unwrap();
    layout.validate(sizes.len()).unwrap();
    for chunk in &layout.chunks {
        let bytes: usize = chunk.iter().map(|i| sizes[*i]).sum();
        assert!(bytes <= 4 || chunk.len() == 1);
        assert!(chunk.iter().all(|i| classes[*i] == classes[chunk[0]]));
    }
    assert!(ObjectPackChunkLayout::new(&sizes, Some(&[false]), 4).is_err());
    assert_eq!(
        ObjectPackChunkLayout::new(&[], None, 0)
            .unwrap()
            .chunks
            .len(),
        0
    );
}
#[test]
fn partitioned_immutable_publication_reuse_conflict_and_concurrent_writers() {
    let d = tempfile::tempdir().unwrap();
    let members = vec![full(b"immutable one"), full(b"immutable two")];
    let priority = BTreeSet::from([members[1].blob_id.clone()]);
    let layout = ObjectPackChunkLayout::for_members(&members, Some(&priority));
    let id = identifier(&members, &layout);
    let path = d.path().join(format!("{id}.zstpack"));
    std::thread::scope(|scope| {
        let jobs = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    write_typed_pack_archive_immutable_with_layout(
                        path.to_str().unwrap(),
                        &id,
                        &members,
                        &layout,
                    )
                })
            })
            .collect::<Vec<_>>();
        for j in jobs {
            j.join().unwrap().unwrap();
        }
    });
    let before = std::fs::read(&path).unwrap();
    let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
    write_typed_pack_archive_immutable_with_layout(path.to_str().unwrap(), &id, &members, &layout)
        .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);
    let mut wrong = members.clone();
    wrong[0] = full(b"conflicting immutable body");
    assert!(write_typed_pack_archive_immutable_with_layout(
        path.to_str().unwrap(),
        &id,
        &wrong,
        &layout
    )
    .unwrap_err()
    .contains("different bytes"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let ordinary = d.path().join("normal-create-mode");
        File::create(&ordinary).unwrap();
        assert_eq!(
            std::fs::metadata(&ordinary).unwrap().permissions().mode() & 0o777,
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777
        );
        let link = d.path().join("symlink.zstpack");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(write_typed_pack_archive_immutable_with_layout(
            link.to_str().unwrap(),
            &id,
            &members,
            &layout
        )
        .is_err());
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
