//! Read-only formal acceptance joins. The immutable aggregate key must bind the
//! caller's hash and original Job; this is not a rehash of cleaned-up bytes.
use super::*;
use crate::workflow_expected_identity::PackageWorkflowExpectedIdentity;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageCompletionOutcome {
    Confirmed,
    Incomplete,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageMapping {
    pub local_task_id: String,
    pub remote_task_id: String,
    pub changes: Vec<PackageChangeMapping>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageChangeMapping {
    pub local_change_ref: String,
    pub remote_change_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageChangeMappingExpectation {
    pub local_change_ref: String,
    pub remote_change_ref: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageMappingExpectation {
    pub local_task_id: String,
    pub remote_task_id: Option<String>,
    pub changes: Vec<PackageChangeMappingExpectation>,
    pub snapshots: Vec<PackageSnapshotOwnerExpectation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageSnapshotOwnerExpectation {
    pub snapshot_id: String,
    pub local_change_ref: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PackageCompletion {
    pub outcome: PackageCompletionOutcome,
    pub expected_identity: PackageWorkflowExpectedIdentity,
    pub patchset_id: Option<String>,
    pub submission_id: Option<String>,
    pub entries: Vec<PackageMapping>,
}

impl PackageCompletion {
    /// Formal acceptance is independent of complete Local mapping recovery.
    /// Use this only on a validated completion response or formal read join.
    pub fn is_accepted(&self) -> bool {
        matches!(
            self.outcome,
            PackageCompletionOutcome::Confirmed | PackageCompletionOutcome::Incomplete
        ) && self.patchset_id.as_ref().is_some_and(|id| !id.is_empty())
            && self.submission_id.as_ref().is_some_and(|id| !id.is_empty())
    }

    /// Only confirmed completion authorizes complete mapping writeback.
    pub fn has_complete_mappings(&self) -> bool {
        self.outcome == PackageCompletionOutcome::Confirmed
    }

    pub fn unknown(identity: &PackageWorkflowExpectedIdentity) -> Self {
        Self::unresolved(identity, PackageCompletionOutcome::Unknown)
    }
    fn unresolved(
        identity: &PackageWorkflowExpectedIdentity,
        outcome: PackageCompletionOutcome,
    ) -> Self {
        Self {
            outcome,
            expected_identity: identity.clone(),
            patchset_id: None,
            submission_id: None,
            entries: Vec::new(),
        }
    }

    pub fn from_value(
        value: &Value,
        expected: &PackageWorkflowExpectedIdentity,
        source: &[PackageMappingExpectation],
    ) -> Result<Self, String> {
        let data = package_response_data(value)?;
        for field in ["patchset_id", "submission_id"] {
            if data.get(field).is_none() {
                return Err(format!("Completion readback is missing nullable {field}"));
            }
        }
        let result: Self =
            serde_json::from_value(data.clone()).map_err(|error| error.to_string())?;
        result.expected_identity.validate()?;
        if result.expected_identity != *expected {
            return Err("Completion readback changed the exact package/Job identity".into());
        }
        if result.outcome == PackageCompletionOutcome::Confirmed {
            if result.patchset_id.as_ref().is_none_or(String::is_empty)
                || result.submission_id.as_ref().is_none_or(String::is_empty)
            {
                return Err(
                    "Confirmed completion requires formal Patchset and submission identities"
                        .into(),
                );
            }
            validate_mappings(&result.entries, source)?;
            let final_task = &result
                .entries
                .last()
                .expect("validated nonempty batch")
                .remote_task_id;
            crate::workflow_expected_identity::validate_patchset_reference_owner(
                result.patchset_id.as_deref().expect("required Patchset"),
                final_task,
                None,
            )?;
            let (owner, ordinal) = result
                .submission_id
                .as_deref()
                .expect("required submission")
                .rsplit_once("/L-")
                .ok_or("Completion submission lacks a Task owner")?;
            let number = ordinal
                .parse::<u8>()
                .map_err(|_| "Invalid completion submission ordinal")?;
            if owner != final_task
                || !(1..=64).contains(&number)
                || ordinal != format!("{number:02}")
            {
                return Err("Completion submission has a foreign owner or ordinal".into());
            }
        } else {
            if !result.entries.is_empty() {
                return Err("Incomplete/unknown completion cannot infer mappings".into());
            }
            match (
                result.outcome,
                result.patchset_id.as_deref(),
                result.submission_id.as_deref(),
            ) {
                (_, None, None) => {}
                (PackageCompletionOutcome::Incomplete, Some(patchset), Some(submission)) => {
                    let (task, _) = submission
                        .rsplit_once("/L-")
                        .ok_or("Completion submission lacks a Task owner")?;
                    validate_accepted_references(patchset, submission, task)?;
                }
                _ => {
                    return Err(
                        "Only accepted incomplete completion may carry paired formal identities"
                            .into(),
                    )
                }
            }
        }
        Ok(result)
    }
}

fn validate_mappings(
    entries: &[PackageMapping],
    source: &[PackageMappingExpectation],
) -> Result<(), String> {
    if entries.len() != source.len() || entries.is_empty() {
        return Err("Completion requires the entire original batch mapping".into());
    }
    let mut local = BTreeSet::new();
    let mut remote = BTreeSet::new();
    for (entry, expected) in entries.iter().zip(source) {
        if entry.local_task_id != expected.local_task_id
            || !local.insert(&entry.local_task_id)
            || !remote.insert(&entry.remote_task_id)
            || entry.remote_task_id.is_empty()
            || entry.remote_task_id.trim() != entry.remote_task_id
            || expected
                .remote_task_id
                .as_ref()
                .is_some_and(|id| id != &entry.remote_task_id)
            || entry.changes.len() != expected.changes.len()
        {
            return Err("Completion mapping changed batch order, ownership or an existing publication mapping".into());
        }
        let mut local_changes = BTreeSet::new();
        let mut remote_changes = BTreeSet::new();
        for (change, expected_change) in entry.changes.iter().zip(&expected.changes) {
            let valid_ref = |reference: &str, task: &str| {
                reference
                    .rsplit_once('/')
                    .is_some_and(|(owner, child)| owner == task && is_package_change_id(child))
            };
            if change.local_change_ref != expected_change.local_change_ref
                || !valid_ref(&change.local_change_ref, &entry.local_task_id)
                || !valid_ref(&change.remote_change_ref, &entry.remote_task_id)
                || !local_changes.insert(&change.local_change_ref)
                || !remote_changes.insert(&change.remote_change_ref)
                || expected_change
                    .remote_change_ref
                    .as_ref()
                    .is_some_and(|id| id != &change.remote_change_ref)
            {
                return Err(
                    "Completion Change mappings are incomplete, reordered or differently owned"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub enum PackageReadResource {
    Job(u32),
    Snapshot(String),
    Task(String),
    Land(String),
    Patchset(String),
}

/// Read existing formal evidence without replaying a mutation. Current GET
/// projections can prove acceptance, but not complete sealed source membership.
/// Proven acceptance returns incomplete with formal IDs and no mappings, even
/// when Local source assertions or mapping reads are unavailable. A direct exact
/// full finish response is validated by `PackageCompletion::from_value`.
pub fn lookup_package_completion<E: std::fmt::Display>(
    expected: &PackageWorkflowExpectedIdentity,
    target_line: &str,
    _source: &[PackageMappingExpectation],
    mut read: impl FnMut(PackageReadResource) -> Result<Value, E>,
) -> PackageCompletion {
    let unknown = || PackageCompletion::unresolved(expected, PackageCompletionOutcome::Unknown);
    let join = || -> Result<PackageCompletion, String> {
        expected.validate()?;
        let job_value = read(PackageReadResource::Job(expected.worker_job_index))
            .map_err(|error| error.to_string())?;
        let data = package_response_data(&job_value)?;
        let job = &data["job"];
        if job["repository_index"].as_u64() != Some(u64::from(expected.repository_index))
            || job["worker_job_index"].as_u64() != Some(u64::from(expected.worker_job_index))
            || job["tombstoned"].as_bool() != Some(false)
        {
            return Err("Formal Job identity is unavailable".into());
        }
        if job.get("patchset_index") == Some(&Value::Null) {
            let execution = PackageExecution::from_job_response(&job_value, expected)?;
            if execution.is_some_and(|execution| {
                execution.availability == PackageAvailability::Available
                    && !matches!(
                        execution.ci_state,
                        PackageCiState::Unknown | PackageCiState::Interrupted
                    )
            }) {
                return Ok(PackageCompletion::unresolved(
                    expected,
                    PackageCompletionOutcome::Incomplete,
                ));
            }
            return Err("No authoritative available precommit package execution".into());
        }
        let job_patchset = job["patchset_index"]
            .as_u64()
            .ok_or("Job has no formal Patchset association")?;
        let snapshot = read(PackageReadResource::Snapshot(
            expected.revision_snapshot_id.clone(),
        ))
        .map_err(|error| error.to_string())?;
        let snapshot = package_response_data(&snapshot)?;
        if snapshot["snapshot_id"].as_str() != Some(expected.revision_snapshot_id.as_str()) {
            return Err("Final Snapshot binding differs".into());
        }
        let binding = exact_owner(snapshot)?;
        let task_id = text(binding, "task_id")?;
        let task =
            read(PackageReadResource::Task(task_id.clone())).map_err(|error| error.to_string())?;
        let receipt = crate::land_json::LandJson::stateless()
            .task_successful_receipt(
                package_response_data(&task)?,
                &task_id,
                Some(expected.repository_index),
            )?
            .ok_or("Final Task has no successful finish")?;
        let submission_id = text(&receipt, "submission_id")?;
        let patchset_id = text(&receipt, "patchset_id")?;
        validate_accepted_references(&patchset_id, &submission_id, &task_id)?;
        let land = read(PackageReadResource::Land(submission_id.clone()))
            .map_err(|error| error.to_string())?;
        let patchset = read(PackageReadResource::Patchset(patchset_id.clone()))
            .map_err(|error| error.to_string())?;
        let land = package_response_data(&land)?;
        let patchset = package_response_data(&patchset)?;
        for row in [&receipt, land, patchset] {
            if text(row, "base_snapshot_id")? != expected.base_snapshot_id
                || text(row, "revision_snapshot_id")? != expected.revision_snapshot_id
                || row.get("repository_index").is_some_and(|value| {
                    value.as_u64() != Some(u64::from(expected.repository_index))
                })
            {
                return Err("Formal finish Snapshot tuple differs".into());
            }
        }
        for row in [&receipt, land] {
            if text(row, "target_line")? != target_line
                || text(row, "task_id")? != task_id
                || text(row, "patchset_id")? != patchset_id
                || text(row, "submission_id")? != submission_id
                || row["status"].as_str() != Some("succeeded")
                || text(row, "mode")? != text(&receipt, "mode")?
                || [
                    "change_id",
                    "change_ref",
                    "target_line_index",
                    "landed_snapshot_id",
                    "pre_land_target_snapshot_id",
                ]
                .iter()
                .any(|field| row.get(field) != receipt.get(field))
            {
                return Err("Formal successful finish joins conflict".into());
            }
        }
        if text(patchset, "patchset_id")? != patchset_id
            || text(patchset, "task_id")? != task_id
            || patchset
                .get("patchset_index")
                .is_some_and(|index| index.as_u64() != Some(job_patchset))
            || ["change_id", "change_ref"].iter().any(|field| {
                patchset
                    .get(field)
                    .is_some_and(|value| Some(value) != receipt.get(field))
            })
        {
            return Err("Original Job is bound to a different formal Patchset".into());
        }
        let summary = text(patchset, "summary")?;
        let raw = summary
            .strip_prefix("ait-history-promotion/v1 ")
            .ok_or("Formal aggregate mapping is missing")?;
        let manifest: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
        if text(&manifest, "idempotency_key")? != format!("workflow-package:{}:{}", expected.package_hash, expected.worker_job_index)
            || text(&manifest, "target_line")? != target_line
            || text(&manifest, "base_snapshot_id")? != expected.base_snapshot_id
            || text(&manifest, "revision_snapshot_id")? != expected.revision_snapshot_id
            || text(&manifest["aggregate"], "task_id")? != task_id
            || text(&manifest["aggregate"], "patchset_id")? != patchset_id
            // The existing detail projection omits its physical key. The
            // immutable aggregate retains that key for the original Job join.
            || manifest["aggregate"]["patchset_index"].as_u64() != Some(job_patchset)
        {
            return Err("Aggregate manifest identity differs from successful finish".into());
        }
        // Formal acceptance is already proven. These GET projections expose no
        // immutable complete source inventory. Missing or conflicting Local labels,
        // source anchors, or mapping rows cannot erase acceptance or authorize
        // complete Local publication. Only a validated full response can do that.
        Ok(PackageCompletion {
            outcome: PackageCompletionOutcome::Incomplete,
            expected_identity: expected.clone(),
            patchset_id: Some(patchset_id),
            submission_id: Some(submission_id),
            entries: Vec::new(),
        })
    };
    let mut join = join;
    join().unwrap_or_else(|_| unknown())
}

fn validate_accepted_references(
    patchset: &str,
    submission: &str,
    task: &str,
) -> Result<(), String> {
    if task.is_empty() || task.trim() != task {
        return Err("Completion requires an exact Task owner".into());
    }
    crate::workflow_expected_identity::validate_patchset_reference_owner(patchset, task, None)?;
    let (owner, ordinal) = submission
        .rsplit_once("/L-")
        .ok_or("Completion submission lacks a Task owner")?;
    let number = ordinal
        .parse::<u8>()
        .map_err(|_| "Invalid completion submission ordinal")?;
    if owner != task || !(1..=64).contains(&number) || ordinal != format!("{number:02}") {
        return Err("Completion submission has a foreign owner or ordinal".into());
    }
    Ok(())
}

fn text(row: &Value, field: &str) -> Result<String, String> {
    row[field]
        .as_str()
        .filter(|value| !value.is_empty() && value.trim() == *value)
        .map(str::to_string)
        .ok_or_else(|| format!("Missing exact {field}"))
}
fn exact_owner(snapshot: &Value) -> Result<&Value, String> {
    let owner = snapshot
        .get("workflow_owner")
        .filter(|owner| owner.is_object())
        .ok_or("Snapshot has no exact governed owner")?;
    if owner.as_object().expect("owner object").len() != 2
        || owner.get("task_id").is_none()
        || owner.get("change_ref").is_none()
    {
        return Err("Snapshot workflow_owner has missing or unknown fields".into());
    }
    let task = text(owner, "task_id")?;
    if let Some(change) = nullable_text(owner, "change_ref")? {
        if !change
            .rsplit_once('/')
            .is_some_and(|(owner, child)| owner == task && is_package_change_id(child))
        {
            return Err("Snapshot Change has a foreign owner or ordinal".into());
        }
    }
    Ok(owner)
}
fn nullable_text(row: &Value, field: &str) -> Result<Option<String>, String> {
    match row.get(field) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() && value.trim() == value => {
            Ok(Some(value.clone()))
        }
        _ => Err(format!("Missing nullable {field}")),
    }
}

#[cfg(test)]
mod tests;
