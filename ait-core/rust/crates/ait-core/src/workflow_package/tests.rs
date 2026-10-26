use super::*;
use std::io::Read;

#[test]
fn package_encoder_accepts_64_tasks_and_rejects_65_without_repository_key_limits() {
    for count in [64, 65] {
        let mut package = manifest();
        package.history_entries = (0..count).map(|index| json!({
            "local_task_id":format!("LCT-{}",10000+index),"expected_remote_task_id":null,
            "task":{"title":"History","intent":"Keep all owners","plan_id":null,"origin_plan_revision_id":null,"plan_item_ref":null},"changes":[],
            "pre_land_target_snapshot_id":format!("SNP-{:012X}",index+1),"landed_snapshot_id":format!("SNP-{:012X}",index+2),"landed_at_s":index+1,
            "snapshots":[{"snapshot_id":format!("SNP-{:012X}",index+2),"created_at_s":index+1,"local_change_ref":null}]
        })).collect();
        package.revision_snapshot_id = format!("SNP-{:012X}", count + 1);
        package.content = ZstdBulkCommitRequest::from_json_rows(None,None,vec![],vec![],vec![],vec![],
            (0..=count).map(|index|json!({"snapshot_id":format!("SNP-{:012X}",index+1),"parent_snapshot_ids":if index==0 {vec![]}else{vec![format!("SNP-{index:012X}")]}})).collect(),None).unwrap();
        let encoded = package.encode(&BTreeMap::new(), &BTreeMap::new());
        if count == 64 {
            assert_eq!(
                inspect_workflow_package_manifest(&encoded.unwrap().bytes).unwrap()
                    ["history_entries"]
                    .as_array()
                    .unwrap()
                    .len(),
                64
            );
        } else {
            assert!(encoded.unwrap_err().contains("1..64 source Tasks"));
        }
    }
}

fn manifest() -> WorkflowPackageManifest {
    WorkflowPackageManifest {
        repository_index: 0, target_line: "main".into(),
        base_snapshot_id: "SNP-000000000001".into(), revision_snapshot_id: "SNP-000000000003".into(),
        author_mode: "ai_with_human_review".into(),
        summary: None,
        history_entries: vec![json!({"local_task_id":"LCT-1","changes":[],
            "expected_remote_task_id":null,
            "task":{"title":"Work","intent":"Change content","plan_id":null,"origin_plan_revision_id":null,"plan_item_ref":null},
            "pre_land_target_snapshot_id":"SNP-000000000001","landed_snapshot_id":"SNP-000000000003","landed_at_s":42,
            "snapshots":[{"snapshot_id":"SNP-000000000002","created_at_s":40,"local_change_ref":null},{"snapshot_id":"SNP-000000000003","created_at_s":41,"local_change_ref":null}]})],
        content: ZstdBulkCommitRequest::from_json_rows(None,None,
            vec![json!({"pack_id":"PCK-000000000002"}),json!({"pack_id":"PCK-000000000001"})],
            vec![json!({"pack_id":"TPK-000000000001"})],vec![],vec![],
            vec![json!({"snapshot_id":"SNP-000000000003","parent_snapshot_ids":["SNP-000000000002"]}),
                json!({"snapshot_id":"SNP-000000000002","parent_snapshot_ids":["SNP-000000000001"]}),
                json!({"snapshot_id":"SNP-000000000001","parent_snapshot_ids":[]})],None).unwrap(),
    }
}

fn packs() -> (BTreeMap<String, Vec<u8>>, BTreeMap<String, Vec<u8>>) {
    (
        BTreeMap::from([
            ("PCK-000000000002".into(), b"second pack".to_vec()),
            ("PCK-000000000001".into(), b"first pack".to_vec()),
        ]),
        BTreeMap::from([("TPK-000000000001".into(), b"tree pack".to_vec())]),
    )
}

#[test]
fn manifest_inspection_preserves_source_identity_without_certifying_pack_bodies() {
    let (objects, trees) = packs();
    let original = manifest();
    let encoded = original.encode(&objects, &trees).unwrap();
    assert_eq!(
        inspect_workflow_package_manifest(&encoded.bytes).unwrap(),
        original.to_value().unwrap()
    );
}

#[test]
fn manifest_inspection_rejects_malformed_leading_data_and_oversized_allocation() {
    let member = |name: &str, body: &[u8]| {
        let mut archive = tar::Builder::new(Vec::new());
        append_regular(&mut archive, name, body).unwrap();
        zstd::stream::encode_all(archive.into_inner().unwrap().as_slice(), 3).unwrap()
    };
    for bytes in [
        b"invalid zstd".to_vec(),
        member("other.json", b"{}"),
        member("manifest.json", b"{"),
        member("manifest.json", br#"{"contract":"other"}"#),
    ] {
        assert!(inspect_workflow_package_manifest(&bytes).is_err());
    }
    let mut header = tar::Header::new_ustar();
    header.set_path("manifest.json").unwrap();
    header.set_size(16 * 1024 * 1024 + 1);
    header.set_cksum();
    let oversized = zstd::stream::encode_all(header.as_bytes().as_slice(), 3).unwrap();
    assert!(inspect_workflow_package_manifest(&oversized)
        .unwrap_err()
        .contains("size"));
    let valid = member("manifest.json", br#"{"contract":"workflow-package/v1"}"#);
    assert!(inspect_workflow_package_manifest(&valid[..valid.len() / 2]).is_err());
}

#[test]
fn deterministic_archive_keeps_exact_pack_bytes_and_domain_times_without_archive_times() {
    let (objects, trees) = packs();
    let first = manifest().encode(&objects, &trees).unwrap();
    let mut reordered = manifest();
    reordered.content.object_packs.reverse();
    reordered.content.snapshots.reverse();
    assert_eq!(first, reordered.encode(&objects, &trees).unwrap());
    assert_eq!(
        first.package_hash,
        format!("{:x}", Sha256::digest(&first.bytes))
    );
    let tar_bytes = zstd::stream::decode_all(first.bytes.as_slice()).unwrap();
    assert_eq!(
        zstd::zstd_safe::get_frame_content_size(&first.bytes).unwrap(),
        Some(tar_bytes.len() as u64)
    );
    assert_eq!(first.bytes[4] & 0b111, 0, "No dictionary or checksum flag");
    let mut archive = tar::Archive::new(tar_bytes.as_slice());
    let mut members = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry.path().unwrap().to_string_lossy().into_owned();
        let header = entry.header();
        assert_eq!(header.mode().unwrap(), 0o644);
        assert_eq!(
            (
                header.uid().unwrap(),
                header.gid().unwrap(),
                header.mtime().unwrap()
            ),
            (0, 0, 0)
        );
        assert_eq!(header.username().unwrap(), Some(""));
        assert_eq!(header.groupname().unwrap(), Some(""));
        assert!(header.entry_type().is_file());
        assert_eq!(&header.as_bytes()[257..265], b"ustar\x0000");
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        members.push((name, bytes));
    }
    assert_eq!(
        members
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "manifest.json",
            "objects/PCK-000000000001",
            "objects/PCK-000000000002",
            "trees/TPK-000000000001"
        ]
    );
    let value: Value = serde_json::from_slice(&members[0].1).unwrap();
    assert_eq!(value["history_entries"][0]["landed_at_s"], 42);
    assert_eq!(
        value["content"]["snapshots"][0]["snapshot_id"],
        "SNP-000000000001"
    );
    assert!(value["content"]["capture_binding"].is_null());
    assert!(value["content"].get("generation_key").is_none());
    assert_eq!(members[1].1, objects["PCK-000000000001"]);
    assert!(!members[0].1.ends_with(b"\n"));
    let mut changed = manifest();
    changed.history_entries[0]["landed_at_s"] = json!(43);
    assert_ne!(
        changed.encode(&objects, &trees).unwrap().package_hash,
        first.package_hash
    );
}

#[test]
fn archive_rejects_extra_missing_traversing_and_conflicting_pack_content() {
    let (mut objects, trees) = packs();
    objects.remove("PCK-000000000001");
    assert!(manifest().encode(&objects, &trees).is_err());
    objects.insert("../PCK-000000000001".into(), vec![]);
    assert!(manifest().encode(&objects, &trees).is_err());
    let mut conflicting = manifest();
    let mut duplicate = conflicting.content.object_packs[0].clone();
    duplicate.total_bytes = Some(20);
    conflicting.content.object_packs.push(duplicate);
    assert!(conflicting.to_value().is_err());
    let mut publishing = manifest();
    publishing.content.generation_key = Some("local-generation".into());
    assert!(publishing.to_value().is_err());
    let mut cyclic = manifest();
    cyclic.content.snapshots[1].parent_snapshot_ids = vec!["SNP-000000000003".into()];
    assert!(cyclic.to_value().is_err());
}

#[test]
fn original_receipts_remain_distinct_from_tested_chain_and_missing_endpoints_are_rejected() {
    let mut rebased = manifest();
    rebased.base_snapshot_id = "SNP-000000000004".into();
    rebased.revision_snapshot_id = "SNP-000000000005".into();
    let mut rows = ZstdBulkCommitRequest::from_json_rows(
        None,
        None,
        vec![],
        vec![],
        vec![],
        vec![],
        vec![
            json!({"snapshot_id":"SNP-000000000004","parent_snapshot_ids":["SNP-000000000001"]}),
            json!({"snapshot_id":"SNP-000000000005","parent_snapshot_ids":["SNP-000000000004"]}),
        ],
        None,
    )
    .unwrap();
    rebased.content.snapshots.append(&mut rows.snapshots);
    rebased.history_entries[0]["snapshots"]
        .as_array_mut()
        .unwrap()
        .push(json!({"snapshot_id":"SNP-000000000005","created_at_s":44,"local_change_ref":null}));
    let value = rebased.to_value().unwrap();
    assert_eq!(
        value["history_entries"][0]["landed_snapshot_id"],
        "SNP-000000000003"
    );
    assert_eq!(value["revision_snapshot_id"], "SNP-000000000005");
    for endpoint in [
        "SNP-000000000001",
        "SNP-000000000003",
        "SNP-000000000004",
        "SNP-000000000005",
    ] {
        let mut missing = rebased.clone();
        missing
            .content
            .snapshots
            .retain(|row| row.snapshot_id != endpoint);
        assert!(missing.to_value().is_err(), "{endpoint}");
    }
    let (objects, trees) = packs();
    let original = rebased.encode(&objects, &trees).unwrap();
    rebased.summary = Some(" Original summary ".into());
    assert_eq!(rebased.to_value().unwrap()["summary"], " Original summary ");
    assert_ne!(
        rebased.encode(&objects, &trees).unwrap().package_hash,
        original.package_hash
    );
}

#[test]
fn owned_snapshots_use_their_own_canonical_order_when_outside_parents_delay_global_rows() {
    let mut package = manifest();
    let base = "SNP-000000000001";
    let replay = "SNP-000000000002";
    let child = "SNP-000000000003";
    let original = "SNP-000000000008";
    let target = "SNP-000000000009";
    package.base_snapshot_id = target.into();
    package.revision_snapshot_id = child.into();
    package.content.snapshots = ZstdBulkCommitRequest::from_json_rows(
        None,
        None,
        vec![],
        vec![],
        vec![],
        vec![],
        vec![
            json!({"snapshot_id":base,"parent_snapshot_ids":[]}),
            json!({"snapshot_id":original,"parent_snapshot_ids":[base]}),
            json!({"snapshot_id":target,"parent_snapshot_ids":[base]}),
            json!({"snapshot_id":replay,"parent_snapshot_ids":[target]}),
            json!({"snapshot_id":child,"parent_snapshot_ids":[replay]}),
        ],
        None,
    )
    .unwrap()
    .snapshots;
    let entry = &mut package.history_entries[0];
    entry["landed_snapshot_id"] = json!(original);
    entry["changes"] = json!([{
        "local_change_id":"C-01","local_change_ref":"LCT-1/C-01",
        "expected_remote_change_ref":null,
        "change":{"title":"Revision","base_line":"feature","fork_snapshot_id":base}
    }]);
    let original_row =
        json!({"snapshot_id":original,"created_at_s":40,"local_change_ref":"LCT-1/C-01"});
    let replay_row =
        json!({"snapshot_id":replay,"created_at_s":44,"local_change_ref":"LCT-1/C-01"});
    let child_row = json!({"snapshot_id":child,"created_at_s":45,"local_change_ref":null});
    entry["snapshots"] = json!([original_row, child_row, replay_row]);
    let input_entry = entry.clone();
    let value = package.to_value().unwrap();
    let mut expected_entry = input_entry.clone();
    expected_entry["snapshots"] = json!([replay_row, child_row, original_row]);
    assert_eq!(value["history_entries"][0], expected_entry);
    assert_eq!(package.history_entries[0], input_entry);
    assert_eq!(
        value["content"]["snapshots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["snapshot_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [base, original, target, replay, child]
    );
    // Both unowned ancestors remain content dependencies, without becoming
    // governed owners. A newly ready child beats the other independent root.
    assert_eq!(
        value["history_entries"][0]["snapshots"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let (objects, trees) = packs();
    let first = package.encode(&objects, &trees).unwrap();
    package.content.snapshots.reverse();
    package.history_entries[0]["snapshots"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert_eq!(package.encode(&objects, &trees).unwrap(), first);
    package
        .content
        .snapshots
        .retain(|row| row.snapshot_id != target);
    assert!(package.to_value().is_err());
}
