use super::*;
use crate::content_binary_db::{
    BinaryDbContentWriteCoordinator, BinaryDbObjectPackMemberWriteInput,
    BinaryDbObjectPackWriteInput,
};
use crate::pack_substrate::{
    build_typed_pack_members_with_storage_gate, default_object_pack_relative_path,
    estimated_typed_pack_archive_bytes, write_typed_pack_archive_with_format,
    ObjectPackWriteMember, PackCandidate, CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
    DEFAULT_MAX_DELTA_CHAIN_DEPTH,
};

#[test]
fn hinted_native_batches_store_delta_and_replay_existing_heads() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("native");
    let interop = db(&temp.path().join("interop"), &workspace);
    let content = content(&temp.path().join("content"), &workspace);
    let objects = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    let source = objects
        .ensure_repository(GitRepositoryKind::ImportSource, 1, [5; 12], "hinted-source")
        .unwrap();
    let mut state = 1234567u32;
    let base = (0..32768)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect::<Vec<_>>();
    let mut target = base.clone();
    target[16000] ^= 1;
    let oid = git_object_id(GitObjectKind::Blob, &base);
    let target_oid = git_object_id(GitObjectKind::Blob, &target);
    let base_input = [GitObjectInput {
        kind: GitObjectKind::Blob,
        format: 1,
        object_id: &oid,
        body: &base,
    }];
    objects
        .put_batch_with_delta_hints(source, &base_input, &[Some("source/file")])
        .unwrap();
    objects
        .put_batch_with_delta_hints(
            source,
            &[GitObjectInput {
                kind: GitObjectKind::Blob,
                format: 1,
                object_id: &target_oid,
                body: &target,
            }],
            &[Some("source/file")],
        )
        .unwrap();
    let record = objects.get_record(source, 1, &target_oid).unwrap().unwrap();
    let read = BinaryDbReadTxn::new(content.db());
    let view = content
        .blobs()
        .blob_view_at(&read, record.content_blob_index)
        .unwrap();
    let member = content
        .object_packs()
        .object_pack_member_view_at(&read, view.pack_member_index.unwrap())
        .unwrap();
    assert_eq!(
        member.record.member_kind(),
        crate::content_binary_db::BinaryObjectPackMemberKind::Delta
    );
    assert_eq!(
        objects.get(source, 1, &target_oid).unwrap().unwrap().1,
        target
    );
    drop(read);
    drop(objects);
    let reopened = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    reopened
        .put_batch_with_delta_hints(source, &base_input, &[Some("source/file")])
        .unwrap();
    let mut next = base.clone();
    next[16001] ^= 1;
    let next_id = git_object_id(GitObjectKind::Blob, &next);
    let mut further = next.clone();
    further[16002] ^= 1;
    let further_id = git_object_id(GitObjectKind::Blob, &further);
    reopened
        .put_batch_with_delta_hints(
            source,
            &[
                GitObjectInput {
                    kind: GitObjectKind::Blob,
                    format: 1,
                    object_id: &next_id,
                    body: &next,
                },
                GitObjectInput {
                    kind: GitObjectKind::Blob,
                    format: 1,
                    object_id: &further_id,
                    body: &further,
                },
            ],
            &[Some("source/file"), Some("source/file")],
        )
        .unwrap();
    assert_eq!(reopened.get(source, 1, &next_id).unwrap().unwrap().1, next);
    assert_eq!(
        reopened.get(source, 1, &further_id).unwrap().unwrap().1,
        further
    );
    drop(reopened);
    let fresh = LocalGitObjectStore::open(interop, content).unwrap();
    assert_eq!(
        fresh.get(source, 1, &further_id).unwrap().unwrap().1,
        further
    );
}

fn write_and_check(
    root: &Path,
    kind: GitObjectKind,
    logical: &[(Vec<u8>, String)],
    members: &[ObjectPackWriteMember],
    external_bases: &[(Vec<u8>, String)],
) -> (u64, f64, f64, Vec<u8>, u64) {
    let workspace = root.join("native");
    let interop = db(&workspace.join(".ait/git-interop/v1/binary-v0"), &workspace);
    let content = content(&workspace.join(".ait/binary-db"), &workspace);
    let objects = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    let source = objects
        .ensure_repository(GitRepositoryKind::ImportSource, 1, [4; 12], "delta-sample")
        .unwrap();
    if !external_bases.is_empty() {
        let ids = external_bases
            .iter()
            .map(|(body, _)| git_object_id(kind, body))
            .collect::<Vec<_>>();
        let inputs = external_bases
            .iter()
            .zip(&ids)
            .map(|((body, _), oid)| GitObjectInput {
                kind,
                format: 1,
                object_id: oid,
                body,
            })
            .collect::<Vec<_>>();
        objects.put_batch(source, &inputs).unwrap();
    }
    let mut seed = Vec::new();
    for member in members {
        seed.extend_from_slice(member.blob_id.as_bytes());
        seed.extend_from_slice(&Sha256::digest(&member.data));
    }
    let pack_id = format!(
        "PCK-{}",
        &Sha256::digest(&seed)
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<String>()[..12]
    );
    let relative = default_object_pack_relative_path(&pack_id);
    let path = workspace.join(&relative);
    let started = std::time::Instant::now();
    let stats = write_typed_pack_archive_with_format(
        path.to_str().unwrap(),
        &pack_id,
        CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
        members,
        crate::repository_pack_policy::zstd_only_object_pack_write_format(),
    )
    .unwrap();
    let write_seconds = started.elapsed().as_secs_f64();
    let metadata = members
        .iter()
        .map(|member| {
            let body = member.logical_data.as_deref().unwrap_or(&member.data);
            BinaryDbObjectPackMemberWriteInput {
                blob_id: member.blob_id.clone(),
                sha256: Sha256::digest(body)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                size_bytes: body.len() as i64,
                pack_entry_type: member.entry_type.clone(),
                pack_base_blob_id: member.base_blob_id.clone(),
                pack_chain_depth: member.chain_depth as i64,
                created_at: "2026-10-12T00:00:00Z".into(),
            }
        })
        .collect();
    BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    )
    .record_object_pack_metadata(
        crate::binary_db::BinaryDbCommandScope::ContentWrite,
        &BinaryDbObjectPackWriteInput {
            pack_id,
            pack_rel_path: relative,
            pack_format: stats["pack_format"].as_str().unwrap().to_string(),
            member_count: members.len() as i64,
            total_bytes: stats["total_bytes"].as_i64().unwrap(),
            created_at: "2026-10-12T00:00:00Z".into(),
            members: metadata,
        },
    )
    .unwrap();
    let oids = logical
        .iter()
        .map(|(body, _)| git_object_id(kind, body))
        .collect::<Vec<_>>();
    let inputs = logical
        .iter()
        .zip(&oids)
        .map(|((body, _), oid)| GitObjectInput {
            kind,
            format: 1,
            object_id: oid,
            body,
        })
        .collect::<Vec<_>>();
    objects.put_batch(source, &inputs).unwrap();
    drop(objects);
    let started = std::time::Instant::now();
    let reopened = LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    assert_eq!(
        reopened
            .get_batch(source, 1, &oids)
            .unwrap()
            .iter()
            .map(|(_, body)| body)
            .collect::<Vec<_>>(),
        logical.iter().map(|(body, _)| body).collect::<Vec<_>>()
    );
    let read_seconds = started.elapsed().as_secs_f64();
    let fixed = fs::read(workspace.join(".ait/git-interop/v1/binary-v0/git_object.bin")).unwrap();
    fs::remove_file(workspace.join(".ait/git-interop/v1/binary-v0/git_object.idx")).unwrap();
    LocalGitObjectStore::rebuild_indexes(&interop).unwrap();
    assert_eq!(
        fs::read(workspace.join(".ait/git-interop/v1/binary-v0/git_object.bin")).unwrap(),
        fixed
    );
    LocalGitObjectStore::open(interop.clone(), content.clone()).unwrap();
    if let Some(offset) = members
        .iter()
        .position(|member| member.entry_type == "delta")
    {
        let member_file = workspace.join(".ait/binary-db/object_pack_member.bin");
        let correct = fs::read(&member_file).unwrap();
        let mut corrupt = correct.clone();
        let ordinal = external_bases.len() + offset;
        corrupt[4 + ordinal * 16 + 12..4 + ordinal * 16 + 16]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&member_file, corrupt).unwrap();
        assert!(LocalGitObjectStore::open(interop.clone(), content.clone()).is_err());
        fs::write(member_file, correct).unwrap();
    }
    let correct = fs::read(&path).unwrap();
    let mut corrupt = correct.clone();
    corrupt[0] ^= 0x80;
    fs::write(&path, corrupt).unwrap();
    assert!(LocalGitObjectStore::open(interop, content).is_err());
    fs::write(&path, correct).unwrap();
    (
        fs::read_dir(workspace.join(".ait/objects/packs"))
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum(),
        write_seconds,
        read_seconds,
        fixed,
        path.metadata().unwrap().len(),
    )
}

#[test]
#[ignore = "explicit OpenRig bounded source delta samples"]
fn openrig_bounded_delta_sample_codecs() {
    let root = std::path::PathBuf::from(std::env::var_os("AIT_GIT_DELTA_SAMPLE_ROOT").unwrap());
    let input: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("inputs.json")).unwrap()).unwrap();
    let temp = tempdir().unwrap();
    let mut reports = Vec::new();
    for (name, kind) in [
        ("blob", GitObjectKind::Blob),
        ("tree", GitObjectKind::Tree),
        ("commit", GitObjectKind::Commit),
    ] {
        let mut logical = Vec::new();
        for case in input["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["kind"] == name)
        {
            for role in ["base", "target"] {
                let body =
                    fs::read(root.join(case[format!("{role}_path")].as_str().unwrap())).unwrap();
                assert_eq!(
                    git_object_id(kind, &body)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>(),
                    case[format!("{role}_oid")].as_str().unwrap()
                );
                logical.push((body, case["path_hint"].as_str().unwrap().to_string()));
            }
        }
        assert!(
            logical.len() <= 128
                && logical.iter().map(|(body, _)| body.len()).sum::<usize>() <= 8 * 1024 * 1024
        );
        let candidates = logical
            .iter()
            .map(|(body, hint)| {
                let id =
                    crate::content_binary_db::blob_id_from_sha256(&Sha256::digest(body).into());
                PackCandidate {
                    entry_name: format!("blobs/{id}"),
                    blob_id: id,
                    data: body.clone(),
                    path_hint: Some(hint.clone()),
                    chain_depth: 0,
                }
            })
            .collect::<Vec<_>>();
        let full = candidates
            .iter()
            .map(|c| ObjectPackWriteMember {
                entry_name: c.entry_name.clone(),
                blob_id: c.blob_id.clone(),
                data: c.data.clone(),
                logical_data: None,
                entry_type: "full".into(),
                base_blob_id: None,
                chain_depth: 0,
                delta_algorithm: None,
            })
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let delta = build_typed_pack_members_with_storage_gate(
            candidates,
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            None,
        )
        .unwrap();
        let selection_seconds = started.elapsed().as_secs_f64();
        let full_result = write_and_check(
            &temp.path().join(format!("full-{name}")),
            kind,
            &logical,
            &full,
            &[],
        );
        let delta_result = write_and_check(
            &temp.path().join(format!("delta-{name}")),
            kind,
            &logical,
            &delta,
            &[],
        );
        assert_eq!(
            full_result.3, delta_result.3,
            "Git OIDs, kind, Blob ordinals and record bytes must stay identical"
        );
        assert_eq!(
            estimated_typed_pack_archive_bytes(&full).unwrap() as u64,
            full_result.4
        );
        assert_eq!(
            estimated_typed_pack_archive_bytes(&delta).unwrap() as u64,
            delta_result.4
        );
        assert!(delta_result.0 <= full_result.0);
        reports.push(serde_json::json!({"kind":name,"scenario":"base_and_target_in_one_chunk","objects":logical.len(),"raw_bytes":logical.iter().map(|(body,_)|body.len()).sum::<usize>(),"full_pack_bytes":full_result.0,"delta_pack_bytes":delta_result.0,"delta_members":delta.iter().filter(|m|m.entry_type=="delta").count(),"selection_seconds":selection_seconds,"full_pack_write_seconds":full_result.1,"delta_pack_write_seconds":delta_result.1,"full_reopen_get_seconds":full_result.2,"delta_reopen_get_seconds":delta_result.2,"exact_oids_body_checksums_fk_recovery":true}));
        let bases = logical.iter().step_by(2).cloned().collect::<Vec<_>>();
        let targets = logical
            .iter()
            .skip(1)
            .step_by(2)
            .cloned()
            .collect::<Vec<_>>();
        let initial = bases
            .iter()
            .map(|(body, hint)| {
                let id =
                    crate::content_binary_db::blob_id_from_sha256(&Sha256::digest(body).into());
                (
                    hint.clone(),
                    PackCandidate {
                        entry_name: format!("blobs/{id}"),
                        blob_id: id,
                        data: body.clone(),
                        path_hint: None,
                        chain_depth: 0,
                    },
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let target_candidates = targets
            .iter()
            .map(|(body, hint)| {
                let id =
                    crate::content_binary_db::blob_id_from_sha256(&Sha256::digest(body).into());
                PackCandidate {
                    entry_name: format!("blobs/{id}"),
                    blob_id: id,
                    data: body.clone(),
                    path_hint: Some(hint.clone()),
                    chain_depth: 0,
                }
            })
            .collect::<Vec<_>>();
        let full_targets = target_candidates
            .iter()
            .map(|c| ObjectPackWriteMember {
                entry_name: c.entry_name.clone(),
                blob_id: c.blob_id.clone(),
                data: c.data.clone(),
                logical_data: None,
                entry_type: "full".into(),
                base_blob_id: None,
                chain_depth: 0,
                delta_algorithm: None,
            })
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let delta_targets = build_typed_pack_members_with_storage_gate(
            target_candidates,
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            Some(&initial),
        )
        .unwrap();
        let selection_seconds = started.elapsed().as_secs_f64();
        let full_external = write_and_check(
            &temp.path().join(format!("external-full-{name}")),
            kind,
            &logical,
            &full_targets,
            &bases,
        );
        let delta_external = write_and_check(
            &temp.path().join(format!("external-delta-{name}")),
            kind,
            &logical,
            &delta_targets,
            &bases,
        );
        assert_eq!(full_external.3, delta_external.3);
        assert_eq!(
            estimated_typed_pack_archive_bytes(&full_targets).unwrap() as u64,
            full_external.4
        );
        assert_eq!(
            estimated_typed_pack_archive_bytes(&delta_targets).unwrap() as u64,
            delta_external.4
        );
        assert!(delta_external.0 <= full_external.0);
        reports.push(serde_json::json!({"kind":name,"scenario":"base_in_prior_native_pack","objects":logical.len(),"raw_bytes":logical.iter().map(|(body,_)|body.len()).sum::<usize>(),"full_pack_bytes":full_external.0,"delta_pack_bytes":delta_external.0,"full_new_target_pack_bytes":full_external.4,"delta_new_target_pack_bytes":delta_external.4,"delta_members":delta_targets.iter().filter(|m|m.entry_type=="delta").count(),"selection_seconds":selection_seconds,"full_pack_write_seconds":full_external.1,"delta_pack_write_seconds":delta_external.1,"full_reopen_get_seconds":full_external.2,"delta_reopen_get_seconds":delta_external.2,"exact_oids_body_checksums_fk_recovery":true}));
    }
    fs::write(
        root.join("native-sample-result.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(&reports).unwrap());
}
