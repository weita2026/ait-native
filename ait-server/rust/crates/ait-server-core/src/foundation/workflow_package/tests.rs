use super::*;
use serde_json::json;

fn manifest() -> Value {
    json!({"contract":PACKAGE_MANIFEST_CONTRACT,"repository_index":0,"target_line":"main",
        "base_snapshot_id":"SNP-000000000001","revision_snapshot_id":"SNP-000000000002","author_mode":"ai_with_human_review",
        "history_entries":[{"local_task_id":"LT-0001","expected_remote_task_id":null,
            "task":{"title":"Source Task","intent":"Publish completed history"},
            "pre_land_target_snapshot_id":"SNP-000000000001","landed_snapshot_id":"SNP-000000000002","landed_at_s":10,
            "changes":[],"snapshots":[{"snapshot_id":"SNP-000000000002","created_at_s":2,"local_change_ref":null}]}],
        "content":{"object_packs":[{"pack_id":"PCK-A"}],"tree_packs":[{"pack_id":"TPK-A"}],
            "blob_locators":[],"tree_locators":[],"snapshots":[{"snapshot_id":"SNP-000000000002","parent_snapshot_id":"SNP-000000000001"}],
            "line_update":null,"capture_binding":null}})
}

fn octal_field(bytes: &mut [u8], number: u64) {
    let text = format!("{number:0width$o}", width = bytes.len() - 1);
    bytes[..text.len()].copy_from_slice(text.as_bytes());
}

fn tar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = Vec::new();
    for (name, body) in files {
        let mut header = [0; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        octal_field(&mut header[100..108], 0o644);
        octal_field(&mut header[108..116], 0);
        octal_field(&mut header[116..124], 0);
        octal_field(&mut header[124..136], body.len() as u64);
        octal_field(&mut header[136..148], 0);
        header[148..156].fill(b' ');
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let checksum = header.iter().map(|byte| *byte as u64).sum();
        octal_field(&mut header[148..156], checksum);
        archive.extend_from_slice(&header);
        archive.extend_from_slice(body);
        archive.resize(archive.len() + (512 - body.len() % 512) % 512, 0);
    }
    archive.resize(archive.len() + 1024, 0);
    archive
}

fn compressed(tar: &[u8]) -> Vec<u8> {
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
    encoder.include_contentsize(true).unwrap();
    encoder.include_checksum(false).unwrap();
    encoder
        .set_pledged_src_size(Some(tar.len() as u64))
        .unwrap();
    encoder.write_all(tar).unwrap();
    encoder.finish().unwrap()
}

fn hash(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}

#[test]
fn optional_summary_is_hashed_verbatim_and_null_is_rejected() {
    let absent = manifest();
    let mut supplied = absent.clone();
    supplied["summary"] = json!("  Review summary\nsecond line  ");
    let bytes = serde_json::to_vec(&supplied).unwrap();
    let parsed = WorkflowPackageManifest::parse(&bytes).unwrap();
    assert_eq!(
        parsed.summary.as_deref(),
        Some("  Review summary\nsecond line  ")
    );
    assert_ne!(
        hash(&compressed(&tar(&[("manifest.json", &bytes)]))),
        hash(&compressed(&tar(&[(
            "manifest.json",
            &serde_json::to_vec(&absent).unwrap()
        )])))
    );
    supplied["summary"] = Value::Null;
    assert!(WorkflowPackageManifest::parse(&serde_json::to_vec(&supplied).unwrap()).is_err());
}

struct Seed;
#[test]
fn original_and_rebased_snapshot_branches_follow_topology_without_time_sorting() {
    let mut value = manifest();
    let ids = [
        "SNP-000000000002",
        "SNP-000000000003",
        "SNP-000000000004",
        "SNP-000000000005",
    ];
    value["revision_snapshot_id"] = json!(ids[3]);
    value["history_entries"][0]["landed_snapshot_id"] = json!(ids[2]);
    value["history_entries"][0]["snapshots"] = json!(ids
        .iter()
        .zip([20, 10, 5, 1])
        .map(|(id, time)| json!({"snapshot_id":id,"created_at_s":time,"local_change_ref":null}))
        .collect::<Vec<_>>());
    value["content"]["snapshots"] = json!(ids
        .iter()
        .zip(["SNP-000000000001", "SNP-000000000001", ids[0], ids[1]])
        .map(|(id, parent)| json!({"snapshot_id":id,"parent_snapshot_id":parent}))
        .collect::<Vec<_>>());
    let parsed = WorkflowPackageManifest::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        parsed.history_entries[0]
            .snapshots
            .iter()
            .map(|snapshot| snapshot.created_at_s)
            .collect::<Vec<_>>(),
        [20, 10, 5, 1]
    );
    for (first, second) in [(0, 1), (0, 2)] {
        let mut invalid = value.clone();
        invalid["history_entries"][0]["snapshots"]
            .as_array_mut()
            .unwrap()
            .swap(first, second);
        let error =
            WorkflowPackageManifest::parse(&serde_json::to_vec(&invalid).unwrap()).unwrap_err();
        assert!(error.contains("parent-first"), "{error}");
    }
}

impl WorkflowPackageBaseContent for Seed {
    fn snapshot(&self, _: &str) -> Result<Value, String> {
        Err("No seed Snapshot expected".into())
    }
    fn root_tree(&self, _: &str, _: u32) -> Result<String, String> {
        Err("No seed root expected".into())
    }
    fn tree(&self, id: &str) -> Result<Value, String> {
        if id != "TRE-SEED" {
            return Err("Unknown seed tree".into());
        }
        Ok(
            json!([{"entry_name":"base.txt","entry_type":"blob","target_id":"BLB-SEED","size_bytes":4,"mode":"0o100644"}]),
        )
    }
    fn blob(&self, id: &str) -> Result<Vec<u8>, String> {
        if id != "BLB-SEED" {
            return Err("Unknown seed blob".into());
        }
        Ok(b"seed".to_vec())
    }
}

#[test]
fn sealed_pack_closure_materializes_seed_deletions_modes_and_real_symlinks() {
    use crate::foundation::pack_substrate::{
        build_tree_pack_members, read_pack_index, read_tree_pack_index, write_pack_archive,
        write_tree_pack_archive,
    };
    let root = tempfile::tempdir().unwrap();
    let object = root.path().join("object");
    let tree = root.path().join("tree");
    write_pack_archive(object.to_str().unwrap(),"PCK-A","2026-10-20T00:00:00Z",&json!([
        {"entry_name":"blobs/BLB-LINK","blob_id":"BLB-LINK","data":b"run".to_vec(),"entry_type":"full","chain_depth":0},
        {"entry_name":"blobs/BLB-RUN","blob_id":"BLB-RUN","data":b"#!/bin/sh\nexit 0\n".to_vec(),"entry_type":"full","chain_depth":0}
    ])).unwrap();
    let tree_members=build_tree_pack_members(&json!([{"tree_id":"TRE-ROOT","entry_count":3}]),&json!([
        {"tree_id":"TRE-ROOT","entry_name":"run","entry_type":"blob","target_id":"BLB-RUN","size_bytes":17,"mode":"0o100755"},
        {"tree_id":"TRE-ROOT","entry_name":"link","entry_type":"blob","target_id":"BLB-LINK","size_bytes":3,"mode":"0o120777"},
        {"tree_id":"TRE-ROOT","entry_name":"seed","entry_type":"tree","target_id":"TRE-SEED","size_bytes":null,"mode":"tree"}
    ])).unwrap();
    write_tree_pack_archive(
        tree.to_str().unwrap(),
        "TPK-A",
        "2026-10-20T00:00:00Z",
        &tree_members,
    )
    .unwrap();
    let mut value = manifest();
    value["content"]["object_packs"] = json!([{"pack_id":"PCK-A","pack_index":read_pack_index(object.to_str().unwrap()).unwrap()}]);
    value["content"]["tree_packs"] = json!([{"pack_id":"TPK-A","pack_index":read_tree_pack_index(tree.to_str().unwrap()).unwrap()}]);
    value["content"]["blob_locators"] = json!([
        {"blob_id":"BLB-LINK","pack_id":"PCK-A","pack_entry_name":"blobs/BLB-LINK","size_bytes":3,"sha256":hash(b"run")},
        {"blob_id":"BLB-RUN","pack_id":"PCK-A","pack_entry_name":"blobs/BLB-RUN","size_bytes":17,"sha256":hash(b"#!/bin/sh\nexit 0\n")}
    ]);
    value["content"]["tree_locators"] = json!([{"tree_id":"TRE-ROOT","tree_pack_id":"TPK-A"}]);
    value["content"]["snapshots"][0]["root_tree_pack_id"] = json!("TPK-A");
    value["content"]["snapshots"][0]["root_entry_ordinal"] = json!(0);
    value["content"]["snapshots"][0]["file_count"] = json!(3);
    value["content"]["snapshots"][0]["total_bytes"] = json!(24);
    let raw = compressed(&tar(&[
        ("manifest.json", &serde_json::to_vec(&value).unwrap()),
        ("objects/PCK-A", &fs::read(object).unwrap()),
        ("trees/TPK-A", &fs::read(tree).unwrap()),
    ]));
    let mut package =
        WorkflowPackage::unpack(&raw, &root.path().join("inputs"), 0, &hash(&raw), 1_000_000)
            .unwrap();
    let source = root.path().join("source");
    fs::create_dir_all(source.join("run/obsolete")).unwrap();
    fs::write(source.join("seed"), b"old file").unwrap();
    fs::write(source.join("deleted"), b"seed leftover").unwrap();
    package.materialize_final_tree(&source, &Seed).unwrap();
    assert!(!source.join("deleted").exists());
    assert_eq!(
        fs::read(source.join("run")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    assert_eq!(fs::read(source.join("seed/base.txt")).unwrap(), b"seed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::read_link(source.join("link")).unwrap(),
            PathBuf::from("run")
        );
        assert_eq!(
            fs::metadata(source.join("run"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    // Existing Core typed bulk rows retain these semantic fields and omit
    // raw-reader creation/index/chunk fields and stored member byte lengths.
    let objects = &value["content"]["object_packs"][0]["pack_index"];
    let trees = &value["content"]["tree_packs"][0]["pack_index"];
    package.manifest.content.object_packs[0]["pack_index"] = json!({
        "pack_id":"PCK-A","pack_format":objects["pack_format"],"member_count":2,"total_bytes":objects["total_bytes"],
        "entries":[
            {"entry_name":"blobs/BLB-LINK","blob_id":"BLB-LINK","entry_type":"full","checksum":hash(b"run"),"chain_depth":0},
            {"entry_name":"blobs/BLB-RUN","blob_id":"BLB-RUN","entry_type":"full","checksum":hash(b"#!/bin/sh\nexit 0\n"),"chain_depth":0}
        ]
    });
    package.manifest.content.tree_packs[0]["pack_index"] = json!({
        "pack_id":"TPK-A","pack_format":trees["pack_format"],"tree_count":1,"total_bytes":trees["total_bytes"],
        "trees":[{"tree_id":"TRE-ROOT","entry_ordinal":0,"entry_count":3,"checksum":trees["trees"][0]["checksum"]}]
    });
    package.materialize_final_tree(&source, &Seed).unwrap();
    assert_eq!(fs::read(source.join("seed/base.txt")).unwrap(), b"seed");
    for (tree, pointer, wrong) in [
        (false, "/pack_id", json!("PCK-OTHER")),
        (false, "/pack_format", json!("unsupported")),
        (false, "/member_count", json!(1)),
        (false, "/total_bytes", json!(0)),
        (false, "/entries/0/entry_name", json!("blobs/OTHER")),
        (false, "/entries/0/blob_id", json!("BLB-OTHER")),
        (false, "/entries/0/entry_type", json!("delta")),
        (false, "/entries/0/checksum", json!("00".repeat(32))),
        (false, "/entries/0/chain_depth", json!(1)),
        (true, "/tree_count", json!(2)),
        (true, "/trees/0/tree_id", json!("TRE-OTHER")),
        (true, "/trees/0/entry_ordinal", json!(1)),
        (true, "/trees/0/entry_count", json!(4)),
        (true, "/trees/0/checksum", json!("00".repeat(32))),
    ] {
        let rows = if tree {
            &mut package.manifest.content.tree_packs
        } else {
            &mut package.manifest.content.object_packs
        };
        let index = &mut rows[0]["pack_index"];
        let previous = index.pointer(pointer).unwrap().clone();
        *index.pointer_mut(pointer).unwrap() = wrong;
        let untouched = root.path().join("rejected");
        let error = package
            .materialize_final_tree(&untouched, &Seed)
            .unwrap_err();
        assert!(error.contains("pack_index differs"), "{error}");
        assert!(!untouched.exists());
        let rows = if tree {
            &mut package.manifest.content.tree_packs
        } else {
            &mut package.manifest.content.object_packs
        };
        *rows[0]["pack_index"].pointer_mut(pointer).unwrap() = previous;
    }
    package.manifest.content.object_packs[0]["pack_index"]["entries"][0]["base_blob_id"] =
        json!("BLB-OTHER");
    assert!(package
        .materialize_final_tree(&root.path().join("wrong-base"), &Seed)
        .unwrap_err()
        .contains("pack_index differs"));
    package.manifest.content.object_packs[0]["pack_index"]["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("base_blob_id");
    package.manifest.content.object_packs[0]["pack_index"]["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("checksum");
    assert!(package
        .materialize_final_tree(&root.path().join("incomplete-index"), &Seed)
        .unwrap_err()
        .contains("pack_index differs"));
}

#[test]
fn package_uses_exact_member_bytes_and_changes_no_authority_files() {
    let root = tempfile::tempdir().unwrap();
    let encoded_manifest = serde_json::to_vec(&manifest()).unwrap();
    let raw = compressed(&tar(&[
        ("manifest.json", &encoded_manifest),
        ("objects/PCK-A", b"object"),
        ("trees/TPK-A", b"tree"),
    ]));
    let first =
        WorkflowPackage::unpack(&raw, &root.path().join("first"), 0, &hash(&raw), 1_000_000)
            .unwrap();
    let second =
        WorkflowPackage::unpack(&raw, &root.path().join("second"), 0, &hash(&raw), 1_000_000)
            .unwrap();
    assert_eq!(first.hash, second.hash);
    assert_eq!(
        fs::read(first.inputs.join("objects/PCK-A")).unwrap(),
        b"object"
    );
    assert_eq!(
        fs::read(first.inputs.join("manifest.json")).unwrap(),
        encoded_manifest
    );
    assert_eq!(
        first.manifest.history_entries[0].snapshots[0].local_change_ref,
        None
    );
    assert!(!first.inputs.join("worker_job.bin").exists());
    assert!(!first.inputs.join("task.bin").exists());
}

#[test]
fn mixed_snapshot_owners_are_explicit_and_missing_owner_is_rejected() {
    let mut value = manifest();
    value["history_entries"][0]["changes"] = json!([
        {"local_change_id":"C-01","local_change_ref":"LT-0001/C-01","expected_remote_change_ref":null,"change":{"title":"First","base_line":"main","fork_snapshot_id":"SNP-000000000001"}},
        {"local_change_id":"C-02","local_change_ref":"LT-0001/C-02","expected_remote_change_ref":null,"change":{"title":"Second","base_line":"main","fork_snapshot_id":"SNP-000000000001"}}]);
    value["history_entries"][0]["snapshots"] = json!([
        {"snapshot_id":"SNP-000000000002","created_at_s":2,"local_change_ref":"LT-0001/C-01"},
        {"snapshot_id":"SNP-000000000003","created_at_s":3,"local_change_ref":null},
        {"snapshot_id":"SNP-000000000004","created_at_s":4,"local_change_ref":"LT-0001/C-02"}]);
    let parsed = WorkflowPackageManifest::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    parsed.require_supported_change_readback().unwrap();
    value["history_entries"][0]["snapshots"][1]
        .as_object_mut()
        .unwrap()
        .remove("local_change_ref");
    assert!(WorkflowPackageManifest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn no_snapshot_change_without_explicit_mapping_is_held_before_publication() {
    let mut value = manifest();
    value["history_entries"][0]["changes"] = json!([
        {"local_change_id":"C-07","local_change_ref":"LT-0001/C-07","expected_remote_change_ref":null,
        "change":{"title":"Empty Change","base_line":"main","fork_snapshot_id":"SNP-000000000001"}}]);
    let parsed = WorkflowPackageManifest::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(parsed.require_supported_change_readback().is_err());
    value["history_entries"][0]["expected_remote_task_id"] = json!("RT-0003");
    value["history_entries"][0]["changes"][0]["expected_remote_change_ref"] = json!("RT-0003/C-02");
    WorkflowPackageManifest::parse(&serde_json::to_vec(&value).unwrap())
        .unwrap()
        .require_supported_change_readback()
        .unwrap();
}

#[test]
fn manifest_rejects_unknown_fields_legacy_change_shape_and_noncanonical_json() {
    let mut unknown = manifest();
    unknown["package_hash"] = json!("00".repeat(32));
    assert!(WorkflowPackageManifest::parse(&serde_json::to_vec(&unknown).unwrap()).is_err());
    let mut legacy = manifest();
    legacy["history_entries"][0]["local_change_ref"] = Value::Null;
    assert!(WorkflowPackageManifest::parse(&serde_json::to_vec(&legacy).unwrap()).is_err());
    let mut missing = manifest();
    missing["history_entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("expected_remote_task_id");
    assert!(WorkflowPackageManifest::parse(&serde_json::to_vec(&missing).unwrap()).is_err());
    let mut newline = serde_json::to_vec(&manifest()).unwrap();
    newline.push(b'\n');
    assert!(WorkflowPackageManifest::parse(&newline).is_err());
    assert!(
        WorkflowPackageManifest::parse(&serde_json::to_vec_pretty(&manifest()).unwrap()).is_err()
    );
}

#[test]
fn archive_rejects_hash_route_order_duplicate_and_traversal_without_partial_inputs() {
    let root = tempfile::tempdir().unwrap();
    let body = serde_json::to_vec(&manifest()).unwrap();
    for (index, files) in [
        vec![
            ("manifest.json", body.as_slice()),
            ("trees/TPK-A", b"tree".as_slice()),
            ("objects/PCK-A", b"object".as_slice()),
        ],
        vec![
            ("manifest.json", body.as_slice()),
            ("manifest.json", body.as_slice()),
        ],
        vec![("../manifest.json", body.as_slice())],
    ]
    .into_iter()
    .enumerate()
    {
        let raw = compressed(&tar(&files));
        let destination = root.path().join(format!("invalid-{index}"));
        assert!(WorkflowPackage::unpack(&raw, &destination, 0, &hash(&raw), 1_000_000).is_err());
        assert!(!destination.exists());
    }
    let raw = compressed(&tar(&[
        ("manifest.json", &body),
        ("objects/PCK-A", b"object"),
        ("trees/TPK-A", b"tree"),
    ]));
    assert!(WorkflowPackage::unpack(
        &raw,
        &root.path().join("wrong-hash"),
        0,
        &"00".repeat(32),
        1_000_000
    )
    .is_err());
    assert!(WorkflowPackage::unpack(
        &raw,
        &root.path().join("wrong-repository"),
        1,
        &hash(&raw),
        1_000_000
    )
    .is_err());
}

#[test]
fn package_rejects_multiple_frames_and_missing_pledged_size() {
    let root = tempfile::tempdir().unwrap();
    let body = serde_json::to_vec(&manifest()).unwrap();
    let archive = tar(&[
        ("manifest.json", &body),
        ("objects/PCK-A", b"object"),
        ("trees/TPK-A", b"tree"),
    ]);
    let mut multiple = compressed(&archive);
    multiple.extend_from_slice(&compressed(&archive));
    assert!(WorkflowPackage::unpack(
        &multiple,
        &root.path().join("multiple"),
        0,
        &hash(&multiple),
        1_000_000
    )
    .is_err());
    let unknown_size = zstd::stream::encode_all(archive.as_slice(), 3).unwrap();
    assert!(WorkflowPackage::unpack(
        &unknown_size,
        &root.path().join("unknown-size"),
        0,
        &hash(&unknown_size),
        1_000_000
    )
    .is_err());
}

#[test]
fn projected_line_encoding_is_task_scoped_injective_and_checks_longest_key_byte_budget() {
    let longest = "RZZT-4294967295";
    let prefix = format!("import/{longest}/base/");
    let limit = (usize::from(u16::MAX) - prefix.len()) / 2;
    let source = "a".repeat(limit);
    let encoded = projected_package_line(longest, &source).unwrap();
    assert!(encoded.len() <= usize::from(u16::MAX));
    assert!(projected_package_line(longest, &(source + "a")).is_err());
    assert_eq!(
        projected_package_line("RT-0001", "  中文/a  ").unwrap(),
        "import/RT-0001/base/e4b8ade696872f61"
    );
    assert_ne!(
        projected_package_line("RT-0001", "a").unwrap(),
        projected_package_line("RT-0002", "a").unwrap()
    );
    assert_ne!(
        projected_package_line("RT-0001", "A").unwrap(),
        projected_package_line("RT-0001", "a").unwrap()
    );
    assert!(require_ordinary_remote_line(" import/RT-0001/base/61 ").is_err());
    assert!(require_ordinary_remote_line("feature/import/a").is_ok());
}
