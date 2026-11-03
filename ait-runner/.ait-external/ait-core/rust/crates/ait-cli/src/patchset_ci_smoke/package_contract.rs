//! Current package HTTP contract for disposable native smoke repositories.
use super::*;
use sha2::{Digest, Sha256};

fn reply(status: u16, value: JsonValue) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(value.to_string()).with_status_code(status)
}

pub(super) fn prepare(
    bytes: &[u8],
    hash: Option<&str>,
    actor: Option<&str>,
    state: &Arc<Mutex<FakeRemoteState>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let digest = format!("{:x}", Sha256::digest(bytes));
    let mut locked = state.lock().unwrap();
    let actor = actor.and_then(|value| value.parse::<u32>().ok());
    if hash != Some(digest.as_str())
        || !actor.is_some_and(|key| locked.actor_keys.values().any(|value| *value == key))
    {
        return reply(400, json!({"detail":"package hash or live Actor mismatch"}));
    }
    let manifest = match ait_core::workflow_package::inspect_workflow_package_manifest(bytes) {
        Ok(manifest) => manifest,
        Err(error) => return reply(400, json!({"detail":error})),
    };
    assert_eq!(manifest["contract"], "workflow-package/v1");
    assert_eq!(manifest["repository_index"], 7);
    assert_eq!(manifest["target_line"], "main");
    if manifest["base_snapshot_id"].as_str() != locked.remote_head_snapshot_id.as_deref() {
        return reply(409, json!({"detail":"package base is not Remote main"}));
    }
    let identity = json!({"repository_index":7,"worker_job_index":0,"package_hash":digest,
        "base_snapshot_id":manifest["base_snapshot_id"],"revision_snapshot_id":manifest["revision_snapshot_id"]});
    locked.package_manifest = Some(manifest);
    locked.package_identity = Some(identity.clone());
    reply(
        200,
        json!({"expected_identity":identity,"ci_state":"passed","reused":false}),
    )
}

pub(super) fn response(
    method: &str,
    url: &str,
    body: &str,
    state: &Arc<Mutex<FakeRemoteState>>,
) -> Option<Response<std::io::Cursor<Vec<u8>>>> {
    let job = "/v1/native/repository-authorities/7/worker-jobs/0";
    let parsed = parse_value_or(body, JsonValue::Null);
    let package_finish = method == "POST"
        && url == "/v1/native/repository-authorities/7/task-land"
        && parsed["contract"] == "task-land-package-guarded/v1";
    if url != job && !package_finish {
        return None;
    }
    let mut locked = state.lock().unwrap();
    let Some(identity) = locked.package_identity.clone() else {
        return Some(reply(404, json!({"detail":"package Job absent"})));
    };
    if method == "GET" && url == job {
        return Some(reply(
            200,
            json!({"job":{"repository_index":7,"worker_job_index":0,"tombstoned":false,
            "patchset_index":if locked.package_completion.is_some() { json!(0) } else { JsonValue::Null }},
            "package_execution":if locked.package_completion.is_some() { JsonValue::Null } else { json!({"expected_identity":identity,"ci_state":"passed","availability":"available","recoverable":true}) }}),
        ));
    }
    if method != "POST"
        || parsed["expected_identity"] != identity
        || !parsed["actor_key_index"]
            .as_u64()
            .and_then(|key| u32::try_from(key).ok())
            .is_some_and(|key| locked.actor_keys.values().any(|value| *value == key))
    {
        return Some(reply(
            409,
            json!({"detail":"package operation identity or Actor mismatch"}),
        ));
    }
    if url == job {
        let operation = parsed["operation"].as_str().expect("evidence operation");
        assert!(matches!(operation, "attestation" | "review"));
        let key = if operation == "review" {
            parsed["payload"]["action"].as_str().expect("review action")
        } else {
            operation
        };
        locked.package_operations.insert(key.into());
        return Some(reply(
            200,
            json!({"expected_identity":identity,"operation":operation,"accepted":true,"recorded_at_s":123}),
        ));
    }
    if let Some(completion) = locked.package_completion.clone() {
        return Some(reply(200, completion));
    }
    if !["attestation", "code_review_summary", "task_approve"]
        .iter()
        .all(|key| locked.package_operations.contains(*key))
    {
        return Some(reply(409, json!({"detail":"package evidence incomplete"})));
    }
    assert_eq!(parsed["target_line"], "main");
    assert_eq!(parsed["mode"], "direct");
    let manifest = locked.package_manifest.clone().unwrap();
    let entries = manifest["history_entries"].as_array().unwrap().iter().enumerate().map(|(index, entry)| {
        assert!(entry["local_change_ref"].is_null());
        json!({"local_task_id":entry["local_task_id"],"remote_task_id":format!("RT-{:04}",index+1),"changes":[]})
    }).collect::<Vec<_>>();
    let final_task = entries.last().unwrap()["remote_task_id"]
        .as_str()
        .unwrap()
        .to_string();
    let patchset = format!("{final_task}/P-01");
    let submission = format!("{final_task}/L-01");
    for entry in &entries {
        let remote_task = entry["remote_task_id"].as_str().unwrap();
        locked.created_task_ids.insert(remote_task.into());
        locked.closed_task_ids.insert(remote_task.into());
    }
    let completion = json!({"outcome":"confirmed","expected_identity":identity,"patchset_id":patchset,"submission_id":submission,"entries":entries});
    locked.remote_head_snapshot_id = identity["revision_snapshot_id"]
        .as_str()
        .map(str::to_string);
    locked.package_completion = Some(completion.clone());
    Some(reply(200, completion))
}
