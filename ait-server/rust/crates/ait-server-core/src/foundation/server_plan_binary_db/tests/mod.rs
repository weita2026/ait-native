use super::schema::{PLAN_ITEM_RECORD_SIZE, PLAN_RECORD_SIZE, PLAN_REVISION_RECORD_SIZE};
use super::*;
use crate::foundation::native_repositories::BinaryDbNativeRepositoryService;
use crate::foundation::pack_substrate::{write_rebuilt_zstd_pack_archive, ObjectPackRewriteBlob};
use crate::foundation::remote_binary_db::{
    server_binary_db_plan_golden_checksum, FilesystemServerRemoteBinaryDb, RepoId, RepoName,
    StoreGeneration, StorePath, SERVER_BINARY_DB_PLAN_GOLDEN_CHECKSUM,
    SERVER_BINARY_DB_PLAN_GOLDEN_SOURCE, SERVER_BINARY_DB_PLAN_GOLDEN_VERSION,
};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};

const UNSUPPORTED_TEST_LAYOUT: u32 = PLAN_LAYOUT_ID + 1;

fn make_root(label: &str) -> StorePath {
    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "ait-server-plan-binary-{label}-{}-{sequence}",
        std::process::id()
    ));
    if path.exists() {
        fs::remove_dir_all(&path).expect("stale temp root should remove");
    }
    StorePath::new(path)
}

fn service(label: &str) -> BinaryDbServerPlanService<FilesystemServerRemoteBinaryDb> {
    BinaryDbServerPlanService::new(FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("REPO-PLAN-BIN"),
        RepoName::new("repo-bin"),
        make_root(label),
        StoreGeneration::new(1),
    ))
}

fn create_payload(title: &str, item_ref: &str) -> JsonValue {
    json!({
        "title": title,
        "status": "draft",
        "summary": "Initial summary",
        "artifact_path": "docs/sprints/demo.md",
        "artifact_heading": "Demo",
        "items": [{
            "plan_item_ref": item_ref,
            "text": "Task item",
            "checkbox_state": "open",
            "heading_path": ["Demo"],
            "line_number": 1,
        }],
        "actor_identity": "tester",
        "actor_type": "human",
    })
}

fn seed_packed_plan_content(
    service: &BinaryDbServerPlanServiceV1<FilesystemServerRemoteBinaryDb>,
    label: &str,
    artifact_path: &str,
    body: &str,
) -> JsonValue {
    let native = BinaryDbNativeRepositoryService::new(service.db().clone());
    let bytes = body.as_bytes().to_vec();
    let sha256 = sha256_hex(&bytes);
    let blob_id = format!("BLB-{}", &sha256[..20]);
    let object_pack_seed = sha256_hex(format!("object\0{label}\0{blob_id}").as_bytes());
    let object_pack_id = format!("PCK-{}", &object_pack_seed[..12].to_ascii_uppercase());
    let created_at = "2026-07-13T00:00:00Z";
    let source_root = std::env::temp_dir().join(format!(
        "ait-server-plan-packed-{label}-{}",
        std::process::id()
    ));
    if source_root.exists() {
        fs::remove_dir_all(&source_root).expect("stale packed source should remove");
    }
    fs::create_dir_all(&source_root).expect("packed source should create");

    let object_pack_path = source_root.join(format!("{object_pack_id}.zstpack"));
    let object_stats = write_rebuilt_zstd_pack_archive(
        object_pack_path
            .to_str()
            .expect("object pack path should be UTF-8"),
        &object_pack_id,
        created_at,
        vec![ObjectPackRewriteBlob {
            entry_name: format!("blobs/{blob_id}"),
            blob_id: blob_id.clone(),
            data: bytes.clone(),
            path_hint: Some(artifact_path.to_string()),
        }],
        0,
    )
    .expect("canonical object pack should write");
    native
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                object_pack_id.clone(),
                fs::read(&object_pack_path).expect("object pack should read"),
            )],
            false,
        )
        .expect("object pack should import");
    native
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "blob_id": blob_id,
                "sha256": sha256,
                "size_bytes": bytes.len(),
                "pack_id": object_pack_id,
                "pack_entry_name": format!("blobs/{blob_id}"),
                "pack_entry_type": "full",
                "pack_base_blob_id": JsonValue::Null,
                "pack_chain_depth": 0,
                "created_at": created_at,
            })],
            false,
        )
        .expect("blob locator should import");

    fs::remove_dir_all(&source_root).expect("packed source should remove");

    json!({
        "storage_authority": "remote_zstd_pack",
        "generation_key": label,
        "artifact_blob_id": blob_id,
        "artifact_path": artifact_path,
        "media_type": "text/markdown; charset=utf-8",
        "encoding": "utf-8",
        "byte_count": bytes.len(),
        "object_pack": {
            "generation_key": label,
            "pack_id": object_pack_id,
            "pack_format": object_stats["pack_format"],
            "pack_index_entry_name": object_stats["pack_index_entry_name"],
            "pack_index_checksum": object_stats["pack_index_checksum"],
        },
        "blob_locator": {
            "generation_key": label,
            "blob_id": blob_id,
            "sha256": sha256,
            "size_bytes": bytes.len(),
            "pack_id": object_pack_id,
            "pack_entry_type": "full",
            "pack_base_blob_id": JsonValue::Null,
            "pack_chain_depth": 0,
        },
    })
}

fn overwrite_record_file_layout(
    service: &BinaryDbServerPlanServiceV1<FilesystemServerRemoteBinaryDb>,
    file: BinaryFileId,
    layout: u32,
) {
    let root = service.db().authority_root().as_path().to_path_buf();
    fs::create_dir_all(&root).expect("root should create");
    let path = root.join(file.as_str());
    let mut handle = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap_or_else(|err| panic!("{} should open: {err}", file.as_str()));
    handle
        .write_all(&layout.to_le_bytes())
        .expect("layout should write");
    handle
        .write_all(&vec![0_u8; file.record_size() as usize])
        .expect("record should write");
}

fn authority_data_files(
    service: &BinaryDbServerPlanServiceV1<FilesystemServerRemoteBinaryDb>,
) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(service.db().authority_root().as_path())
        .expect("authority root should read")
        .filter_map(|entry| {
            let entry = entry.expect("authority entry should read");
            let file_type = entry.file_type().expect("authority entry type should read");
            if !file_type.is_file() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = fs::read(entry.path()).expect("authority data file should read");
            Some((name, bytes))
        })
        .collect()
}

mod schema;
mod service;
mod write_txn;
