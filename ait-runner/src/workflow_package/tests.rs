use super::*;
use crate::executor::{ExecutorConfig, NativeExecutor};
use crate::protocol::{
    CommandSpec, NATIVE_JOB_CONTRACT, NativeJobRequest, SourceSpec, TerminalStatus,
};
use ait_core::pack_substrate::{
    PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1, build_tree_pack_members,
    read_tree_pack_index_with_format, tree_pack_checksums_by_tree_id,
    write_pack_archive_with_format, write_tree_pack_archive_with_format,
};
use ait_core::server_operational::{RepositoryIndex, WorkerJobIndex};
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

const OBJECT: &str = "PCK-000000000001";
const TREE: &str = "TPK-000000000002";
const BASE: &str = "SNP-000000000001";
const ORIGINAL: &str = "SNP-000000000002";
const FINAL: &str = "SNP-000000000003";

struct Package {
    bytes: Vec<u8>,
    reference: WorkflowPackageReference,
    downloads: AtomicUsize,
}

impl RemoteSnapshotProvider for Package {
    fn fetch_import_manifest(&self, _: &RemoteSnapshotReference) -> Result<Vec<u8>, RunnerError> {
        panic!("package root must not fall back to a committed Snapshot")
    }
    fn download_pack(
        &self,
        _: &RemoteSnapshotReference,
        _: RemotePackKind,
        _: &str,
        _: &Path,
        _: u64,
    ) -> Result<u64, RunnerError> {
        panic!("missing package content must not fall back to canonical packs")
    }
    fn download_workflow_package(
        &self,
        source: &WorkflowPackageReference,
        path: &Path,
        maximum: u64,
    ) -> Result<u64, RunnerError> {
        assert_eq!(source, &self.reference);
        assert!(self.bytes.len() as u64 <= maximum);
        self.downloads.fetch_add(1, Ordering::SeqCst);
        fs::write(path, &self.bytes).unwrap();
        Ok(self.bytes.len() as u64)
    }
}

fn encode(manifest: &Value, members: &[(&str, &[u8])]) -> Vec<u8> {
    let manifest = serde_json::to_vec(&canonical(manifest)).unwrap();
    encode_raw(&manifest, members)
}

fn encode_raw(manifest: &[u8], members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = tar::Builder::new(Vec::new());
    for (name, bytes) in std::iter::once(("manifest.json", manifest)).chain(members.iter().copied())
    {
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(bytes.len() as u64);
        header.set_cksum();
        archive
            .append_data(&mut header, name, Cursor::new(bytes))
            .unwrap();
    }
    let archive = archive.into_inner().unwrap();
    compress(&archive)
}

fn compress(archive: &[u8]) -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    encoder.include_contentsize(true).unwrap();
    encoder.include_checksum(false).unwrap();
    encoder
        .set_pledged_src_size(Some(archive.len() as u64))
        .unwrap();
    encoder.write_all(archive).unwrap();
    encoder.finish().unwrap()
}

fn package(bytes: Vec<u8>) -> Package {
    Package {
        reference: WorkflowPackageReference {
            key: WorkerJobKey::new(RepositoryIndex::new(4), WorkerJobIndex::new(0)),
            package_hash: format!("{:x}", Sha256::digest(&bytes)),
            base_snapshot_id: BASE.into(),
            revision_snapshot_id: FINAL.into(),
        },
        bytes,
        downloads: AtomicUsize::new(0),
    }
}

fn fixture() -> (Value, Vec<u8>, Vec<u8>) {
    let temp = TempDir::new().unwrap();
    let script = b"#!/bin/sh\nset -eu\ntest ! -e deleted.txt\ntest ! -e original-only.txt\ntest -L alias\ntest \"$(readlink alias)\" = secret\ntest \"$(cat alias)\" = private\ncase \"$(uname -s)\" in Darwin) mode=$(stat -f %Lp secret);; *) mode=$(stat -c %a secret);; esac\ntest \"$mode\" = 600\nprintf package-ok\n";
    let data = [
        script.as_slice(),
        b"private".as_slice(),
        b"secret".as_slice(),
        b"old".as_slice(),
        b"original".as_slice(),
    ];
    let ids: Vec<_> = data
        .iter()
        .map(|bytes| format!("BLB-{}", &format!("{:x}", Sha256::digest(bytes))[..20]))
        .collect();
    let object_members: Vec<_> = data
        .iter()
        .zip(&ids)
        .map(|(bytes, id)| {
            json!({
                "entry_name":format!("blobs/{id}"),"blob_id":id,"data":bytes,
                "entry_type":"full","chain_depth":0
            })
        })
        .collect();
    let object_path = temp.path().join("object.pack");
    let object_stats = write_pack_archive_with_format(
        object_path.to_str().unwrap(),
        OBJECT,
        "1970-01-01T00:00:00Z",
        &json!(object_members),
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let tree_members = build_tree_pack_members(&json!([
        {"tree_id":"TRE-BASE","entry_count":1},{"tree_id":"TRE-ORIGINAL","entry_count":1},
        {"tree_id":"TRE-FINAL","entry_count":3},{"tree_id":"TRE-CI","entry_count":1}
    ]),&json!([
        {"tree_id":"TRE-BASE","entry_name":"deleted.txt","entry_type":"blob","target_id":ids[3],"size_bytes":data[3].len(),"mode":"0o644"},
        {"tree_id":"TRE-ORIGINAL","entry_name":"original-only.txt","entry_type":"blob","target_id":ids[4],"size_bytes":data[4].len(),"mode":"0o644"},
        {"tree_id":"TRE-FINAL","entry_name":"ci","entry_type":"tree","target_id":"TRE-CI","size_bytes":null,"mode":"tree"},
        {"tree_id":"TRE-FINAL","entry_name":"secret","entry_type":"blob","target_id":ids[1],"size_bytes":data[1].len(),"mode":"0o100600"},
        {"tree_id":"TRE-FINAL","entry_name":"alias","entry_type":"blob","target_id":ids[2],"size_bytes":data[2].len(),"mode":"0o120777"},
        {"tree_id":"TRE-CI","entry_name":"run.sh","entry_type":"blob","target_id":ids[0],"size_bytes":data[0].len(),"mode":"0o100755"}
    ])).unwrap();
    let tree_path = temp.path().join("tree.pack");
    let tree_stats = write_tree_pack_archive_with_format(
        tree_path.to_str().unwrap(),
        TREE,
        "1970-01-01T00:00:00Z",
        &tree_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let index = read_tree_pack_index_with_format(
        tree_path.to_str().unwrap(),
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .unwrap();
    let checksums = tree_pack_checksums_by_tree_id(&tree_stats).unwrap();
    let ordinal = |tree: &str| {
        index["trees"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["tree_id"] == tree)
            .unwrap()["entry_ordinal"]
            .as_u64()
            .unwrap()
    };
    let snapshots = [(BASE,"TRE-BASE",1,data[3].len()),(ORIGINAL,"TRE-ORIGINAL",1,data[4].len()),(FINAL,"TRE-FINAL",3,data[0].len()+data[1].len()+data[2].len())]
        .into_iter().map(|(id,tree,count,bytes)|json!({"snapshot_id":id,"root_tree_pack_id":TREE,"root_entry_ordinal":ordinal(tree),"file_count":count,"total_bytes":bytes})).collect::<Vec<_>>();
    let blobs = ids.iter().zip(data).map(|(id,bytes)|json!({"blob_id":id,"sha256":format!("{:x}",Sha256::digest(bytes)),"size_bytes":bytes.len(),"pack_id":OBJECT,"pack_entry_name":format!("blobs/{id}"),"pack_entry_type":"full","pack_base_blob_id":null,"pack_chain_depth":0})).collect::<Vec<_>>();
    let trees = ["TRE-BASE","TRE-ORIGINAL","TRE-FINAL","TRE-CI"].into_iter().map(|id| json!({"tree_id":id,"entry_count":if id == "TRE-FINAL" {3} else {1},"tree_pack_id":TREE,"tree_pack_checksum":checksums[id]})).collect::<Vec<_>>();
    let manifest = json!({"contract":"workflow-package/v1","repository_index":4,"target_line":"main","base_snapshot_id":BASE,"revision_snapshot_id":FINAL,"author_mode":"ai_with_human_review","history_entries":[],"content":{
        "object_packs":[{"pack_id":OBJECT,"pack_format":PACK_FORMAT_ZSTD_CHUNKED_V1,"member_count":object_stats["member_count"],"total_bytes":object_stats["total_bytes"],"pack_index_checksum":object_stats["pack_index_checksum"]}],
        "tree_packs":[{"pack_id":TREE,"pack_format":TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,"tree_count":tree_stats["tree_count"],"total_bytes":tree_stats["total_bytes"],"pack_index_checksum":tree_stats["pack_index_checksum"]}],
        "blob_locators":blobs,"tree_locators":trees,"snapshots":snapshots,"line_update":null,"capture_binding":null
    }});
    (
        manifest,
        fs::read(object_path).unwrap(),
        fs::read(tree_path).unwrap(),
    )
}

fn execute(provider: &Package) -> (Result<crate::NativeResult, RunnerError>, TempDir) {
    let attempts = TempDir::new().unwrap();
    let source_root = TempDir::new().unwrap();
    let executor = NativeExecutor::new(ExecutorConfig {
        source_root: source_root.path().into(),
        attempt_root: attempts.path().into(),
    });
    let request = NativeJobRequest {
        contract: NATIVE_JOB_CONTRACT.into(),
        label: None,
        suite_id: None,
        timeout_ms: 10_000,
        source: SourceSpec::WorkflowPackage {
            repository_index: provider.reference.key.repository_index,
            package_hash: provider.reference.package_hash.clone(),
            base_snapshot_id: BASE.into(),
            revision_snapshot_id: FINAL.into(),
        },
        command: CommandSpec {
            argv: vec!["./ci/run".into()],
            working_directory: ".".into(),
            environment: Default::default(),
        },
    };
    let result = executor.execute_claimed(
        &request,
        provider,
        provider.reference.key,
        &ExecutionControl::default(),
    );
    (result, attempts)
}

#[cfg(unix)]
#[test]
fn package_materializes_only_final_tree_with_deletions_modes_symlinks_and_cleans_everything() {
    let (manifest, objects, trees) = fixture();
    let provider = package(encode(
        &manifest,
        &[
            ("objects/PCK-000000000001", &objects),
            ("trees/TPK-000000000002", &trees),
        ],
    ));
    let (result, attempts) = execute(&provider);
    let result = result.unwrap();
    assert_eq!(result.status, TerminalStatus::Succeeded);
    assert_eq!(
        result.suite_results[0]
            .execution
            .materialization
            .source_kind,
        "workflow_package"
    );
    assert!(result.cleanup.attempt_root_removed);
    assert_eq!(fs::read_dir(attempts.path()).unwrap().count(), 0);
    assert_eq!(provider.downloads.load(Ordering::SeqCst), 1);
}

#[test]
fn corrupt_hash_missing_content_and_wrong_package_identity_fail_without_fallback_and_clean_attempts()
 {
    let (manifest, objects, trees) = fixture();
    let bytes = encode(
        &manifest,
        &[
            ("objects/PCK-000000000001", &objects),
            ("trees/TPK-000000000002", &trees),
        ],
    );
    let mut corrupt = package(bytes);
    corrupt.bytes[8] ^= 1;
    let (result, attempts) = execute(&corrupt);
    assert!(result.unwrap_err().to_string().contains("hash"));
    assert_eq!(fs::read_dir(attempts.path()).unwrap().count(), 0);
    let missing = package(encode(&manifest, &[("trees/TPK-000000000002", &trees)]));
    let (result, attempts) = execute(&missing);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("missing declared packs")
    );
    assert_eq!(fs::read_dir(attempts.path()).unwrap().count(), 0);
    let mut wrong = manifest;
    wrong["repository_index"] = json!(5);
    let provider = package(encode(
        &wrong,
        &[
            ("objects/PCK-000000000001", &objects),
            ("trees/TPK-000000000002", &trees),
        ],
    ));
    let (result, attempts) = execute(&provider);
    assert!(result.unwrap_err().to_string().contains("identity"));
    assert_eq!(fs::read_dir(attempts.path()).unwrap().count(), 0);
}

#[test]
fn unsafe_archive_members_noncanonical_manifests_and_extra_frames_are_rejected() {
    let (manifest, objects, trees) = fixture();
    let members = [
        ("objects/PCK-000000000001", objects.as_slice()),
        ("trees/TPK-000000000002", trees.as_slice()),
    ];
    let valid = encode(&manifest, &members);
    let mut raw = zstd::stream::decode_all(valid.as_slice()).unwrap();
    let manifest_length = octal(&raw[124..136]).unwrap() as usize;
    let offset = 512 + manifest_length.div_ceil(512) * 512;
    let mut header = tar::Header::new_ustar();
    header
        .as_mut_bytes()
        .copy_from_slice(&raw[offset..offset + 512]);
    header.as_mut_bytes()[..100].fill(0);
    header.as_mut_bytes()[..9].copy_from_slice(b"../escape");
    header.set_cksum();
    raw[offset..offset + 512].copy_from_slice(header.as_bytes());
    let bytes = compress(&raw);
    let source = package(bytes.clone());
    let root = TempDir::new().unwrap();
    let archive = root.path().join("source.zst");
    fs::write(&archive, bytes).unwrap();
    let extraction = root.path().join("extract");
    fs::create_dir(&extraction).unwrap();
    assert!(
        unpack(
            &archive,
            &extraction,
            &source.reference,
            &ExecutionControl::default()
        )
        .unwrap_err()
        .to_string()
        .contains("pack path")
    );
    assert!(!root.path().join("escape").exists());

    let mut raw = zstd::stream::decode_all(valid.as_slice()).unwrap();
    header
        .as_mut_bytes()
        .copy_from_slice(&raw[offset..offset + 512]);
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_cksum();
    raw[offset..offset + 512].copy_from_slice(header.as_bytes());
    let mut newline = serde_json::to_vec(&canonical(&manifest)).unwrap();
    newline.push(b'\n');
    let duplicate = encode(&manifest, &[members[0], members[0], members[1]]);
    let mut frames = valid.clone();
    frames.extend_from_slice(&valid);
    for bytes in [
        compress(&raw),
        encode_raw(&newline, &members),
        duplicate,
        frames,
        valid[..valid.len() - 1].to_vec(),
    ] {
        let provider = package(bytes);
        let (result, attempts) = execute(&provider);
        assert!(result.is_err());
        assert_eq!(fs::read_dir(attempts.path()).unwrap().count(), 0);
    }
}
