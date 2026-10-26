use super::*;
use ait_server_core::foundation::server_content_binary_db::{
    ServerBinaryDbSnapshotStore, ServerBinaryLineCodec,
};
use ait_server_core::foundation::workflow_package::{projected_package_line, WorkflowPackage};

const FORK: &str = "SNP-000000000008";
const ORIGINAL_WORK: &str = "SNP-000000000009";
const RECEIPT: &str = "SNP-00000000000A";
const NEXT_ORIGINAL: &str = "SNP-00000000000B";
const NEXT_TESTED: &str = "SNP-00000000000C";

async fn rebased_fixture(root: &FsPath) -> CompleteFixture {
    let mut f = complete_fixture(root, None).await;
    let package = WorkflowPackage::unpack(
        &f.delta.raw,
        &root.join("rebase-case-inputs"),
        1,
        &f.delta.hash,
        usize::MAX,
    )
    .unwrap();
    let members = retained_members(&package);
    let entry = &mut f.delta.manifest["history_entries"][1];
    entry["changes"][0]["change"]["base_line"] = json!("feature/source");
    entry["changes"][0]["change"]["fork_snapshot_id"] = json!(FORK);
    entry["changes"].as_array_mut().unwrap().push(json!({
        "local_change_id":"C-03","local_change_ref":"LSET-0002/C-03",
        "expected_remote_change_ref":null,
        "change":{"title":"Ordinary later revision","base_line":"feature/source","fork_snapshot_id":ORIGINAL_CHANGE}
    }));
    entry["landed_snapshot_id"] = json!(RECEIPT);
    entry["landed_at_s"] = json!(560);
    // The original fork/work branch and the prior Local rebase branch retain
    // the same Change owner. A later Change has an ordinary descendant result.
    entry["snapshots"] = json!([
        {"snapshot_id":ORIGINAL_TASK,"created_at_s":400,"local_change_ref":"LSET-0002/C-01"},
        {"snapshot_id":ORIGINAL_CHANGE,"created_at_s":500,"local_change_ref":"LSET-0002/C-01"},
        {"snapshot_id":TESTED_TASK,"created_at_s":600,"local_change_ref":"LSET-0002/C-01"},
        {"snapshot_id":TESTED_CHANGE,"created_at_s":700,"local_change_ref":"LSET-0002/C-03"},
        {"snapshot_id":ORIGINAL_WORK,"created_at_s":350,"local_change_ref":"LSET-0002/C-01"},
        {"snapshot_id":RECEIPT,"created_at_s":550,"local_change_ref":"LSET-0002/C-03"}
    ]);
    let template = f.delta.manifest["content"]["snapshots"][0].clone();
    for (id, parent, time) in [
        (FORK, REVISION, 320),
        (ORIGINAL_WORK, FORK, 350),
        (RECEIPT, ORIGINAL_CHANGE, 550),
    ] {
        let mut row = template.clone();
        row["snapshot_id"] = json!(id);
        row["parent_snapshot_id"] = json!(parent);
        row["created_at"] = json!(chrono::DateTime::from_timestamp(time, 0)
            .unwrap()
            .to_rfc3339());
        f.delta.manifest["content"]["snapshots"]
            .as_array_mut()
            .unwrap()
            .push(row);
    }
    (f.delta.raw, f.delta.hash) = archive(&f.delta.manifest, &members);
    f
}

async fn publish(app: &Router, raw: &[u8], hash: &str) -> JsonValue {
    let (status, ready) = upload(app, raw, hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{ready}");
    let identity = &ready["expected_identity"];
    complete(app, identity["worker_job_index"].as_u64().unwrap(), "pass").await;
    attest(app, identity, "ai_only_experimental").await;
    review(app, identity, "summary", "code_review_summary").await;
    review(app, identity, "task", "task_approve").await;
    let (status, result) = finish(app, identity).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    result
}

fn extend_with_task_only_suffix(manifest: &mut JsonValue, published: &JsonValue) {
    for (entry, mapping) in manifest["history_entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(published["entries"].as_array().unwrap())
    {
        entry["expected_remote_task_id"] = mapping["remote_task_id"].clone();
        for (change, mapped) in entry["changes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(mapping["changes"].as_array().unwrap())
        {
            change["expected_remote_change_ref"] = mapped["remote_change_ref"].clone();
        }
    }
    manifest["base_snapshot_id"] = json!(TESTED_CHANGE);
    manifest["revision_snapshot_id"] = json!(NEXT_TESTED);
    manifest["history_entries"].as_array_mut().unwrap().push(json!({
        "local_task_id":"LSET-0003","expected_remote_task_id":null,
        "task":{"title":"Task-only suffix","intent":"Reuse exact historical Changes"},
        "pre_land_target_snapshot_id":RECEIPT,"landed_snapshot_id":NEXT_ORIGINAL,"landed_at_s":1150,
        "changes":[],"snapshots":[
            {"snapshot_id":NEXT_ORIGINAL,"created_at_s":1000,"local_change_ref":null},
            {"snapshot_id":NEXT_TESTED,"created_at_s":1100,"local_change_ref":null}
        ]
    }));
    let template = manifest["content"]["snapshots"][0].clone();
    for (id, parent, time) in [
        (NEXT_ORIGINAL, RECEIPT, 1000),
        (NEXT_TESTED, TESTED_CHANGE, 1100),
    ] {
        let mut row = template.clone();
        row["snapshot_id"] = json!(id);
        row["parent_snapshot_id"] = json!(parent);
        row["created_at"] = json!(chrono::DateTime::from_timestamp(time, 0)
            .unwrap()
            .to_rfc3339());
        manifest["content"]["snapshots"]
            .as_array_mut()
            .unwrap()
            .push(row);
    }
}

fn authority_counts(state: &ServerState) -> Vec<u32> {
    let mut result = counts(state);
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    for file in [
        WorkflowBinaryV0Codec::snapshot_link_file(),
        ServerBinaryLineCodec::<1>::record_file(),
        ait_server_core::foundation::operational_binary_v0::ServerOperationalBinaryV0Codec::worker_job_file(),
    ] {
        result.push(read.record_count(file).unwrap());
    }
    result
}

fn snapshot_mut<'a>(manifest: &'a mut JsonValue, id: &str) -> &'a mut JsonValue {
    manifest["content"]["snapshots"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["snapshot_id"] == id)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_accepts_prior_local_rebase_owners_and_preserves_exact_mapped_reuse() {
    let root = TestDirectory::new("package-rebased-change-owner");
    let mut f = rebased_fixture(root.path()).await;
    let before = authority_counts(&f.delta.state);
    let (status, ready) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{ready}");
    let mut only_job_added = before.clone();
    *only_job_added.last_mut().unwrap() += 1;
    assert_eq!(authority_counts(&f.delta.state), only_job_added);
    let published = publish(&f.delta.app, &f.delta.raw, &f.delta.hash).await;
    assert_eq!(published["entries"][0]["changes"], json!([]));
    assert_eq!(
        published["entries"][1]["changes"].as_array().unwrap().len(),
        2
    );
    let (_, db) = f.delta.state.operational_binary.repository_db(1).unwrap();
    let snapshots = ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone());
    let lines = ServerBinaryDbLineStore::<_, 1>::new(db.clone());
    let read = BinaryDbReadTxn::new(&db);
    let fork_index = snapshots.snapshot_by_id(&read, FORK).unwrap().unwrap().0;
    let last_fork = snapshots
        .snapshot_by_id(&read, ORIGINAL_CHANGE)
        .unwrap()
        .unwrap()
        .0;
    let change_records = (0..2)
        .map(|index| {
            read.read_record(WorkflowBinaryV0Codec::change_file(), index)
                .unwrap()
        })
        .collect::<Vec<_>>();
    for (bytes, expected) in change_records.iter().zip([fork_index, last_fork]) {
        let record = WorkflowBinaryV0Codec::decode_change(bytes).unwrap();
        assert_eq!(record.fork_snapshot_index_plus1, expected + 1);
    }
    let name = projected_package_line(
        published["entries"][1]["remote_task_id"].as_str().unwrap(),
        "feature/source",
    )
    .unwrap();
    let projected = lines.line_by_name(&read, &name).unwrap().unwrap().1;
    assert!(projected.is_archived());
    assert_eq!(projected.head_snapshot_index(), Some(last_fork));
    let links = (0..read
        .record_count(WorkflowBinaryV0Codec::snapshot_link_file())
        .unwrap())
        .map(|index| {
            WorkflowBinaryV0Codec::decode_snapshot_link(
                &read
                    .read_record(WorkflowBinaryV0Codec::snapshot_link_file(), index)
                    .unwrap(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(!links
        .iter()
        .any(|link| link.content_snapshot_index == fork_index));
    for (id, change) in [
        (ORIGINAL_TASK, 1),
        (ORIGINAL_CHANGE, 1),
        (ORIGINAL_WORK, 1),
        (TESTED_TASK, 1),
        (RECEIPT, 2),
        (TESTED_CHANGE, 2),
    ] {
        let index = snapshots.snapshot_by_id(&read, id).unwrap().unwrap().0;
        let link = links
            .iter()
            .find(|link| link.content_snapshot_index == index)
            .unwrap();
        assert_eq!(link.change_index_plus1, change);
    }
    let immutable = f.delta.manifest["content"]["snapshots"]
        .as_array()
        .unwrap()
        .clone();
    drop(read);
    for row in immutable {
        let (_, stored) = call(
            &f.delta.app,
            "GET",
            &format!("{ROUTE}/snapshots/{}", row["snapshot_id"].as_str().unwrap()),
            None,
        )
        .await;
        assert_eq!(stored["parent_snapshot_id"], row["parent_snapshot_id"]);
        assert_eq!(stored["created_at"], row["created_at"]);
    }
    let package = WorkflowPackage::unpack(
        &f.delta.raw,
        &root.path().join("reuse-inputs"),
        1,
        &f.delta.hash,
        usize::MAX,
    )
    .unwrap();
    let members = retained_members(&package);
    extend_with_task_only_suffix(&mut f.delta.manifest, &published);
    let before_invalid = authority_counts(&f.delta.state);
    for (kind, expected) in [
        ("missing-fork", "Required Snapshot"),
        ("mapped-fork", "title/base/fork differs"),
        ("mapped-base", "projected base"),
        ("foreign-task", "owners conflict"),
        ("foreign-change", "owners conflict"),
        ("task-only-owner", "exact Task/Change owner differs"),
        ("immutable-parent", "immutable"),
        ("immutable-message", "immutable canonical content"),
        ("missing-parent", "Required Snapshot"),
        ("cyclic-parent", "cycle"),
        ("receipt-chain", "discontinuous"),
        ("tested-chain", "Tested revision does not descend"),
    ] {
        let mut manifest = f.delta.manifest.clone();
        match kind {
            "missing-fork" => {
                manifest["history_entries"][1]["changes"][0]["change"]["fork_snapshot_id"] =
                    json!("SNP-00000000FFFF")
            }
            "mapped-fork" => {
                manifest["history_entries"][1]["changes"][0]["change"]["fork_snapshot_id"] =
                    json!(ORIGINAL_CHANGE)
            }
            "mapped-base" => {
                manifest["history_entries"][1]["changes"][0]["change"]["base_line"] =
                    json!("feature/foreign")
            }
            "foreign-task" => {
                manifest["history_entries"][1]["expected_remote_task_id"] =
                    published["entries"][0]["remote_task_id"].clone()
            }
            "foreign-change" => {
                manifest["history_entries"][1]["snapshots"][0]["local_change_ref"] =
                    json!("LSET-0002/C-03")
            }
            "task-only-owner" => {
                manifest["history_entries"][1]["snapshots"][0]["local_change_ref"] = JsonValue::Null
            }
            "immutable-parent" => {
                snapshot_mut(&mut manifest, ORIGINAL_TASK)["parent_snapshot_id"] = json!(PEER)
            }
            "immutable-message" => {
                snapshot_mut(&mut manifest, ORIGINAL_TASK)["message"] =
                    json!("Changed immutable message")
            }
            "missing-parent" => {
                snapshot_mut(&mut manifest, NEXT_ORIGINAL)["parent_snapshot_id"] =
                    json!("SNP-00000000FFFF")
            }
            "cyclic-parent" => {
                snapshot_mut(&mut manifest, NEXT_ORIGINAL)["parent_snapshot_id"] =
                    json!(NEXT_TESTED);
                snapshot_mut(&mut manifest, NEXT_TESTED)["parent_snapshot_id"] =
                    json!(NEXT_ORIGINAL);
            }
            "receipt-chain" => {
                manifest["history_entries"][2]["pre_land_target_snapshot_id"] = json!(REVISION)
            }
            "tested-chain" => {
                snapshot_mut(&mut manifest, NEXT_TESTED)["parent_snapshot_id"] = json!(BASE)
            }
            _ => unreachable!(),
        }
        let (raw, hash) = archive(&manifest, &members);
        let (status, result) = upload(&f.delta.app, &raw, &hash, None).await;
        assert!(!status.is_success(), "{kind}: {result}");
        assert!(result.to_string().contains(expected), "{kind}: {result}");
        assert_eq!(authority_counts(&f.delta.state), before_invalid, "{kind}");
    }
    let (raw, hash) = archive(&f.delta.manifest, &members);
    let reused = publish(&f.delta.app, &raw, &hash).await;
    assert_eq!(reused["entries"][0], published["entries"][0]);
    assert_eq!(reused["entries"][1], published["entries"][1]);
    assert_eq!(reused["entries"][2]["changes"], json!([]));
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        lines.line_by_name(&read, &name).unwrap().unwrap().1,
        projected
    );
    for (index, expected) in change_records.iter().enumerate() {
        assert_eq!(
            read.read_record(WorkflowBinaryV0Codec::change_file(), index as u32)
                .unwrap(),
            *expected
        );
    }
}
