//! Read accepted publication before any source-dependent or mutating work.
//! The small response cache lives only for this invocation and grants no authority.
use super::*;
use ait_core::land_json::LandJson;
use ait_core::server_operational::{RepositoryIndex, WorkerJobIndex, WorkerJobKey};
use ait_core::task_store::TaskStore;
use ait_core::workflow_package::{
    lookup_package_completion, package_response_data, PackageCompletion, PackageCompletionOutcome,
    PackageReadResource,
};

struct LocalAnchor {
    task_id: String,
    target: String,
    original_final: String,
    mapped: Option<String>,
}

struct Candidate {
    identity: PackageWorkflowExpectedIdentity,
    target: String,
    job: Option<JsonValue>,
    snapshot: Option<JsonValue>,
    task: Option<JsonValue>,
    patchset: Option<JsonValue>,
}

fn local_anchor(repo: &RepoRuntime, reference: &str) -> Result<Option<LocalAnchor>, String> {
    let task_id = reference.split('/').next().unwrap_or(reference);
    let tasks = repo.task_store()?;
    let task = TaskStore::get_task(&tasks, task_id).map_err(|error| error.to_string())?;
    if task["status"].as_str() != Some("completed") {
        return Ok(None);
    }
    let receipt = tasks
        .local_task_receipt(task_id)
        .map_err(|error| error.to_string())?
        .ok_or("Completed Local Task has no original successful receipt")?;
    if reference != task_id && receipt["change_ref"].as_str() != Some(reference) {
        return Err(
            "Requested revision is not the original Task receipt's associated Change".into(),
        );
    }
    Ok(Some(LocalAnchor {
        task_id: task_id.into(),
        target: required_string_field(&receipt, "target_line")?,
        original_final: required_string_field(&receipt, "landed_snapshot_id")?,
        mapped: string_field(&task, "published_task_id"),
    }))
}

fn snapshot_owner(
    value: &JsonValue,
    repository: u32,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<String, String> {
    let row = package_response_data(value)?;
    if row["repo_name"].as_str() != Some(repo_name)
        || row["repository_index"].as_u64() != Some(u64::from(repository))
        || row["snapshot_id"].as_str() != Some(snapshot_id)
    {
        return Err("Original Snapshot readback belongs to another Repository or Snapshot".into());
    }
    let owner = row["workflow_owner"]
        .as_object()
        .filter(|owner| owner.len() == 2 && owner.contains_key("change_ref"))
        .ok_or("Original Snapshot has no exact governed owner")?;
    let task = required_string_field(&row["workflow_owner"], "task_id")?;
    match &owner["change_ref"] {
        JsonValue::Null => {}
        JsonValue::String(change)
            if change.rsplit_once('/').is_some_and(|(parent, child)| {
                parent == task
                    && child.strip_prefix("C-").is_some_and(|ordinal| {
                        ordinal.parse::<u8>().is_ok_and(|number| {
                            (1..=64).contains(&number) && child == format!("C-{number:02}")
                        })
                    })
            }) => {}
        _ => return Err("Original Snapshot Change has a foreign owner or ordinal".into()),
    }
    Ok(task)
}

fn receipt(task: &JsonValue, task_id: &str, repository: u32) -> Result<JsonValue, String> {
    LandJson::stateless()
        .task_successful_receipt(package_response_data(task)?, task_id, Some(repository))?
        .ok_or("The mapped Remote Task has no unique successful aggregate receipt; retain the original explicit package identity")
        .map_err(str::to_string)
}

fn verify_known_original_metadata(
    repo: &RepoRuntime,
    value: &JsonValue,
    snapshot_id: &str,
) -> Result<(), String> {
    let Ok(store) = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&repo.workspace_root())
    else {
        return Ok(());
    };
    let Ok(Some(original)) = snapshot_by_id_with_snapshot_store(&store, snapshot_id) else {
        return Ok(());
    };
    let row = package_response_data(value)?;
    for (field, expected) in [
        ("manifest_hash", json!(original.manifest_hash)),
        ("line_name", json!(original.line_name)),
        ("parent_snapshot_ids", json!(original.parent_snapshot_ids)),
    ] {
        if row.get(field).is_some_and(|actual| actual != &expected) {
            return Err(
                "Selected origin conflicts with the original immutable Snapshot metadata".into(),
            );
        }
    }
    Ok(())
}

fn candidate_from_task(
    client: &mut PlanHttpClientManager,
    repo_name: &str,
    repository: u32,
    task_id: &str,
    task: JsonValue,
) -> Result<Candidate, String> {
    let receipt = receipt(&task, task_id, repository)?;
    let patchset_id = required_string_field(&receipt, "patchset_id")?;
    let patchset = client
        .get_patchset(&patchset_id, Some(repo_name), None)
        .map_err(|error| error.to_string())?;
    let row = package_response_data(&patchset)?;
    let summary = required_string_field(row, "summary")?;
    let raw = summary
        .strip_prefix("ait-history-promotion/v1 ")
        .ok_or("Original aggregate manifest is unavailable")?;
    let manifest = crate::json_support::parse_value(raw, "Invalid original aggregate manifest")?;
    let key = required_string_field(&manifest, "idempotency_key")?;
    let key = key
        .strip_prefix("workflow-package:")
        .ok_or("The successful receipt is not an original package execution")?;
    let (hash, job) = key
        .split_once(':')
        .ok_or("Original package key is malformed")?;
    let worker_job_index = job
        .parse::<u32>()
        .map_err(|_| "Original package Job is invalid")?;
    if worker_job_index.to_string() != job {
        return Err("Original package Job is not canonical".into());
    }
    let identity = PackageWorkflowExpectedIdentity {
        repository_index: repository,
        package_hash: hash.into(),
        worker_job_index,
        base_snapshot_id: required_string_field(&receipt, "base_snapshot_id")?,
        revision_snapshot_id: required_string_field(&receipt, "revision_snapshot_id")?,
    };
    identity.validate()?;
    Ok(Candidate {
        identity,
        target: required_string_field(&receipt, "target_line")?,
        job: None,
        snapshot: None,
        task: Some(task),
        patchset: Some(patchset),
    })
}

fn cached(value: &Option<JsonValue>, field: &str, id: &str) -> Option<JsonValue> {
    value.as_ref().and_then(|value| {
        package_response_data(value)
            .ok()
            .filter(|row| row[field].as_str() == Some(id))
            .map(|_| value.clone())
    })
}

fn lookup(
    client: &mut PlanHttpClientManager,
    repo_name: &str,
    candidate: &mut Candidate,
) -> PackageCompletion {
    let identity = candidate.identity.clone();
    lookup_package_completion(
        &identity,
        &candidate.target,
        &[],
        |resource| -> Result<JsonValue, String> {
            match resource {
                PackageReadResource::Job(index) => match &candidate.job {
                    Some(job) => Ok(job.clone()),
                    None => {
                        let job = client
                            .get_worker_job(WorkerJobKey::new(
                                RepositoryIndex::new(identity.repository_index),
                                WorkerJobIndex::new(index),
                            ))
                            .map_err(|error| error.to_string())?;
                        candidate.job = Some(job.clone());
                        Ok(job)
                    }
                },
                PackageReadResource::Snapshot(id) => {
                    if let Some(value) = cached(&candidate.snapshot, "snapshot_id", &id) {
                        return Ok(value);
                    }
                    let value = client
                        .get_remote_snapshot(repo_name, &id, false, None)
                        .map_err(|error| error.to_string())?;
                    snapshot_owner(&value, identity.repository_index, repo_name, &id)?;
                    candidate.snapshot = Some(value.clone());
                    Ok(value)
                }
                PackageReadResource::Task(id) => {
                    if let Some(value) = cached(&candidate.task, "task_id", &id) {
                        return Ok(value);
                    }
                    let value = client
                        .get_task(&id, Some(repo_name))
                        .map_err(|error| error.to_string())?;
                    candidate.task = Some(value.clone());
                    Ok(value)
                }
                PackageReadResource::Land(id) => client
                    .get_land(&id, Some(repo_name))
                    .map_err(|error| error.to_string()),
                PackageReadResource::Patchset(id) => {
                    if let Some(value) = cached(&candidate.patchset, "patchset_id", &id) {
                        return Ok(value);
                    }
                    let value = client
                        .get_patchset(&id, Some(repo_name), None)
                        .map_err(|error| error.to_string())?;
                    candidate.patchset = Some(value.clone());
                    Ok(value)
                }
            }
        },
    )
}

/// None permits only the existing source/admission path. Unknown proof is a hold.
pub(crate) fn workflow_package_accepted_retry(
    repo: &RepoRuntime,
    reference: &str,
    remote_name: Option<&str>,
    expected: Option<&PackageWorkflowExpectedIdentity>,
    explicit_author_mode: Option<&str>,
    allow_ready_reprepare: bool,
) -> Result<Option<JsonValue>, String> {
    let stopped = |identity, detail: &str| {
        Some(apply::state(
            reference,
            identity,
            "completion_unknown",
            detail,
        ))
    };
    let anchor = match local_anchor(repo, reference) {
        Ok(Some(anchor)) => Some(anchor),
        Ok(None) if expected.is_none() => return Ok(None),
        Ok(None) => {
            return Ok(stopped(expected, "The requested Local Task is not completed; it cannot borrow another package's acceptance."))
        }
        Err(error) if expected.is_none() => return Ok(stopped(None, &error)),
        Err(_) => None,
    };
    let (remote, repo_name) = remote_context(repo, remote_name, None)?;
    let repository = repo.require_repository_index()?.get();
    if expected.is_some_and(|identity| identity.repository_index != repository) {
        return Err("Package identity belongs to another Repository".into());
    }
    let mut client = package_client(repo, &remote, &repo_name)?;
    let authority = match client.get_repository_by_index(RepositoryIndex::new(repository)) {
        Ok(value) => value,
        Err(error) => {
            return Ok(stopped(
                expected,
                &format!("Selected origin authority is unavailable: {error}"),
            ))
        }
    };
    let authority = package_response_data(&authority)?;
    let authority = authority.get("repository").unwrap_or(authority);
    if authority["repository_index"].as_u64() != Some(u64::from(repository))
        || authority["repository_name"].as_str() != Some(repo_name.as_str())
        || authority["tombstoned"].as_bool() != Some(false)
    {
        return Ok(stopped(
            expected,
            "Selected origin returned a different or unavailable Repository authority.",
        ));
    }
    let candidate = if let Some(identity) = expected {
        identity.validate()?;
        let mut candidate = Candidate {
            identity: identity.clone(),
            target: anchor
                .as_ref()
                .map(|anchor| anchor.target.clone())
                .unwrap_or_default(),
            job: None,
            snapshot: None,
            task: None,
            patchset: None,
        };
        if anchor.is_none() {
            let job = match client.get_worker_job(WorkerJobKey::new(
                RepositoryIndex::new(repository),
                WorkerJobIndex::new(identity.worker_job_index),
            )) {
                Ok(value) => value,
                Err(error) => {
                    return Ok(stopped(
                        expected,
                        &format!("Original Job readback is unavailable: {error}"),
                    ))
                }
            };
            let job_data = package_response_data(&job)?;
            if job_data["job"]["patchset_index"].is_u64() {
                let snapshot = match client.get_remote_snapshot(
                    &repo_name,
                    &identity.revision_snapshot_id,
                    false,
                    None,
                ) {
                    Ok(value) => value,
                    Err(error) => {
                        return Ok(stopped(
                            expected,
                            &format!("Original formal owner is unavailable: {error}"),
                        ))
                    }
                };
                let owner = snapshot_owner(
                    &snapshot,
                    repository,
                    &repo_name,
                    &identity.revision_snapshot_id,
                )?;
                let task = client
                    .get_task(&owner, Some(&repo_name))
                    .map_err(|error| error.to_string())?;
                candidate.target =
                    required_string_field(&receipt(&task, &owner, repository)?, "target_line")?;
                candidate.snapshot = Some(snapshot);
                candidate.task = Some(task);
            }
            candidate.job = Some(job);
        }
        Ok(candidate)
    } else {
        let anchor = anchor
            .as_ref()
            .expect("Local anchor without explicit identity");
        let owner =
            match client.get_remote_snapshot(&repo_name, &anchor.original_final, false, None) {
                Ok(value) => verify_known_original_metadata(repo, &value, &anchor.original_final)
                    .and_then(|()| snapshot_owner(&value, repository, &repo_name, &anchor.original_final))
                    .and_then(|owner| {
                        if anchor.mapped.as_ref().is_some_and(|mapped| mapped != &owner) {
                            return Err("Selected origin conflicts with the original Snapshot's published Task mapping".into());
                        }
                        Ok((owner, Some(value)))
                    }),
                Err(error)
                    if anchor.mapped.is_none() && authority::missing_snapshot(
                        &error,
                        &repo_name,
                        repository,
                        &anchor.original_final,
                    ) =>
                {
                    return Ok(None)
                }
                Err(error) => Err(format!("Original source authority is unavailable: {error}")),
            }
        ;
        owner.and_then(|(owner, snapshot)| {
            let task = client
                .get_task(&owner, Some(&repo_name))
                .map_err(|error| error.to_string())?;
            let mut candidate =
                candidate_from_task(&mut client, &repo_name, repository, &owner, task)?;
            candidate.snapshot = snapshot;
            Ok(candidate)
        })
    };
    let mut candidate = match candidate {
        Ok(candidate) => candidate,
        Err(error) => return Ok(stopped(expected, &error)),
    };
    if anchor
        .as_ref()
        .is_some_and(|anchor| anchor.target != candidate.target)
    {
        return Ok(stopped(
            Some(&candidate.identity),
            "The original Local receipt and formal candidate target different Lines.",
        ));
    }
    let completion = lookup(&mut client, &repo_name, &mut candidate);
    if !completion.is_accepted() {
        // This permits only the explicit ready handler to ask Server to check
        // reset/source eligibility. Null execution itself grants no allocation.
        let retained_request = allow_ready_reprepare
            && expected.is_some()
            && candidate
                .job
                .as_ref()
                .and_then(|value| package_response_data(value).ok())
                .is_some_and(|row| {
                    row["job"]["repository_index"].as_u64()
                        == Some(u64::from(candidate.identity.repository_index))
                        && row["job"]["worker_job_index"].as_u64()
                            == Some(u64::from(candidate.identity.worker_job_index))
                        && row["job"]["tombstoned"].as_bool() == Some(false)
                        && row["job"].get("patchset_index") == Some(&JsonValue::Null)
                        && ait_core::workflow_package::PackageExecution::from_job_response(
                            row,
                            &candidate.identity,
                        )
                        .is_ok_and(|execution| {
                            execution.is_none_or(|execution| {
                                execution.ci_state
                                    == ait_core::workflow_package::PackageCiState::Interrupted
                                    && !execution.recoverable
                            })
                        })
                });
        return Ok(
            if completion.outcome == PackageCompletionOutcome::Incomplete || retained_request {
                None
            } else {
                stopped(Some(&candidate.identity), "The exact original formal acceptance cannot be established. Retain its identity; no package, Job, review or finish was created.")
            },
        );
    }
    let patchset = package_response_data(
        candidate
            .patchset
            .as_ref()
            .expect("validated formal Patchset"),
    )?;
    let owner = required_string_field(patchset, "task_id")?;
    if let Some(anchor) = &anchor {
        if anchor
            .mapped
            .as_ref()
            .is_some_and(|mapped| mapped != &owner)
        {
            return Ok(stopped(
                Some(&candidate.identity),
                "The requested Local Task maps to a different formal owner.",
            ));
        }
        let tasks = repo.task_store()?;
        if let Ok(rows) = tasks.snapshot_ownership_rows(std::slice::from_ref(
            &candidate.identity.revision_snapshot_id,
        )) {
            if rows.iter().any(|row| {
                row["ownership_source"] == "durable_snapshot_binding"
                    && row["task_id"].as_str() != Some(anchor.task_id.as_str())
            }) {
                return Ok(stopped(
                    Some(&candidate.identity),
                    "The pinned final Snapshot belongs to another Local Task.",
                ));
            }
        }
    }
    if let Some(requested) = explicit_author_mode {
        let sealed = required_string_field(patchset, "author_mode")?;
        if requested != sealed {
            return Err("Explicit author mode conflicts with the original accepted Patchset; keep its sealed mode, package and Job".into());
        }
    }
    let mut output = apply::accepted_mapping_pending(reference, &completion)
        .expect("formal GET proves acceptance without complete source membership");
    output["remote_task_id"] = json!(owner);
    let detail = format!(
        "{} Cleanup completion is not established by formal readback.",
        output["next_action"]["summary"]
            .as_str()
            .unwrap_or_default()
    );
    output["next_action"]["summary"] = json!(detail);
    output["apply_stopped_reason"] = json!(detail);
    Ok(Some(output))
}
