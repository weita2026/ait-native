//! Real packed raw-document opening guards for reader reuse and final authority.
use super::*;

fn fixture(root: &Path) -> (LocalBinaryDbFs, LocalContentBinaryDb<1>, u32) {
    let workspace = root.join("native");
    let interop = db(&root.join("interop"), &workspace);
    let c = content(&root.join("content"), &workspace);
    let objects = LocalGitObjectStore::open(interop.clone(), c.clone()).unwrap();
    let source = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [6; 12],
            "document-open-reader",
        )
        .unwrap();
    let mut documents = RawGitDocumentStore::open(interop.clone(), c.clone()).unwrap();
    let tree = documents
        .ensure_document_set(
            source,
            [6; 12],
            "document-open-reader",
            &[
                RawGitDocumentInput {
                    path: "docs/Raw.MD",
                    mode: 0o755,
                    body: b"raw\r\n\xff\0\r",
                },
                RawGitDocumentInput {
                    path: "Link.md",
                    mode: 0o120000,
                    body: b"docs/Raw.MD",
                },
            ],
        )
        .unwrap();
    (interop, c, tree)
}

#[test]
fn document_open_reader_preserves_raw_bytes_modes_and_exact_revisions() {
    let t = tempdir().unwrap();
    let (interop, c, tree) = fixture(t.path());
    let before = fs::read(t.path().join("content/plan_revision.bin")).unwrap();
    let opened = RawGitDocumentStore::open(interop, c).unwrap();
    assert_eq!(
        opened.document_set_files(tree).unwrap(),
        vec![
            ("Link.md".into(), 0o120000, b"docs/Raw.MD".to_vec()),
            ("docs/Raw.MD".into(), 0o755, b"raw\r\n\xff\0\r".to_vec()),
        ]
    );
    assert_eq!(
        before,
        fs::read(t.path().join("content/plan_revision.bin")).unwrap()
    );
}

#[test]
fn document_open_reader_rejects_same_length_metadata_mutation_before_success() {
    let t = tempdir().unwrap();
    let (interop, c, _) = fixture(t.path());
    let path = t.path().join("content/blob.bin");
    let changed = path.clone();
    super::super::documents::set_document_open_hook(Box::new(move |reader| {
        assert!(reader.profile_cached_chunk_bytes() <= 32 * 1024 * 1024);
        let mut bytes = fs::read(&changed).unwrap();
        *bytes.last_mut().unwrap() ^= 0x40;
        fs::write(&changed, bytes).unwrap();
    }));
    let before = fs::read(&path).unwrap();
    let err = match RawGitDocumentStore::open(interop, c) {
        Ok(_) => panic!("changed metadata accepted"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("metadata authority changed"), "{err}");
    let after = fs::read(path).unwrap();
    assert_eq!(before.len(), after.len());
    assert_ne!(before, after);
}

#[test]
fn document_open_reader_rejects_evicted_pack_mutation_before_success() {
    let t = tempdir().unwrap();
    let (interop, c, _) = fixture(t.path());
    super::super::documents::set_document_open_hook(Box::new(|reader| {
        let paths = reader.admitted_pack_paths_for_test();
        assert!(!paths.is_empty());
        reader.evict_readers_for_test();
        assert!(reader.logical_values_empty_for_test());
        for p in paths {
            let mut b = fs::read(&p).unwrap();
            b[4] ^= 0x80;
            fs::write(p, b).unwrap();
        }
    }));
    assert!(RawGitDocumentStore::open(interop, c).is_err());
}
