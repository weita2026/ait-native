use crate::activation::{canonical_real_directory, read_regular_file, sha256};
use crate::types::{SourceJobRow, SourceSnapshot, SOURCE_DATABASE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(crate) const LEGACY_PATCHSET_OMISSION: &str = "legacy_patchset_omission";
pub(crate) const ATTACHED_TERMINAL_EXACT: &str = "attached_terminal_exact";
pub(crate) const UNPROVABLE_ATTACHED_OMISSION: &str = "unprovable_attached_omission";
pub(crate) const REPO_CI_RESULT_SNAPSHOT: &str = "repo_ci_result_snapshot";
pub(crate) const MAIN_SEED_LANDED_SNAPSHOT: &str = "main_seed_landed_snapshot";
pub(crate) const SNAPSHOT_ONLY_NO_PATCHSET_OMISSION: &str = "snapshot_only_no_patchset_omission";
pub(crate) const MISSING_REMOTE_PREFIX: &str = "missing_remote_prefix";
pub(crate) const NON_MAIN_TARGET_OMISSION: &str = "non_main_target_omission";
pub(crate) const DIAGNOSTIC_ATTACHED: &str = "diagnostic_attached";

const POLICY_SCHEMA: &str = "ait.server.postgres_to_binary_v0.job_recovery.v1";

const CATEGORIES: &[(&str, &str)] = &[
    (LEGACY_PATCHSET_OMISSION, "omit"),
    (ATTACHED_TERMINAL_EXACT, "normalize"),
    (UNPROVABLE_ATTACHED_OMISSION, "omit"),
    (REPO_CI_RESULT_SNAPSHOT, "normalize"),
    (MAIN_SEED_LANDED_SNAPSHOT, "normalize"),
    (SNAPSHOT_ONLY_NO_PATCHSET_OMISSION, "omit"),
    (MISSING_REMOTE_PREFIX, "normalize"),
    (NON_MAIN_TARGET_OMISSION, "omit"),
    (DIAGNOSTIC_ATTACHED, "normalize"),
];

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyDocument {
    schema: String,
    status: String,
    source_database: String,
    source_job_count: u64,
    source_jobs_sha256: String,
    categories: BTreeMap<String, PolicyCategory>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyCategory {
    action: String,
    job_count: u64,
    job_ids_sha256: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveryJobPolicy {
    manifest_sha256: String,
    document: PolicyDocument,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RecoveryJobAudit {
    categories: BTreeMap<&'static str, BTreeSet<i64>>,
    omitted: BTreeMap<i64, &'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RecoveryJobClassification {
    pub source_job_id: i64,
    pub action: &'static str,
    pub category: &'static str,
}

impl RecoveryJobPolicy {
    pub(crate) fn load(path: &Path, snapshot: &SourceSnapshot) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Worker Job recovery manifest has no parent".to_string())?;
        let parent = canonical_real_directory(parent)?;
        let name = path
            .file_name()
            .ok_or_else(|| "Worker Job recovery manifest has no name".to_string())?;
        let path = parent.join(name);
        let bytes = read_regular_file(&path)?;
        let document: PolicyDocument = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "failed to parse Worker Job recovery manifest {}: {error}",
                path.display()
            )
        })?;
        if document.schema != POLICY_SCHEMA
            || document.status != "frozen_for_recovery"
            || document.source_database != SOURCE_DATABASE
        {
            return Err("Worker Job recovery manifest envelope is invalid".to_string());
        }
        if document.source_job_count != snapshot.jobs.len() as u64 {
            return Err(format!(
                "Worker Job recovery manifest expects {} source Jobs, found {}",
                document.source_job_count,
                snapshot.jobs.len()
            ));
        }
        validate_sha256(&document.source_jobs_sha256, "source_jobs_sha256")?;
        let actual_source_sha256 = source_jobs_sha256(&snapshot.jobs)?;
        if document.source_jobs_sha256 != actual_source_sha256 {
            return Err(format!(
                "Worker Job recovery manifest source_jobs_sha256 disagrees: expected {}, actual {actual_source_sha256}",
                document.source_jobs_sha256
            ));
        }
        let expected_names = CATEGORIES
            .iter()
            .map(|(name, _)| *name)
            .collect::<BTreeSet<_>>();
        let actual_names = document
            .categories
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual_names != expected_names {
            return Err("Worker Job recovery manifest category inventory is not exact".to_string());
        }
        for (name, action) in CATEGORIES {
            let category = &document.categories[*name];
            if category.action != *action {
                return Err(format!(
                    "Worker Job recovery category {name} must use action {action:?}"
                ));
            }
            validate_sha256(&category.job_ids_sha256, &format!("{name}.job_ids_sha256"))?;
        }
        Ok(Self {
            manifest_sha256: sha256(&bytes),
            document,
        })
    }

    pub(crate) fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub(crate) fn validate_audit(&self, audit: &RecoveryJobAudit) -> Result<(), String> {
        let mut seen = BTreeMap::<i64, &'static str>::new();
        for (name, action) in CATEGORIES {
            let actual = audit.categories.get(name).cloned().unwrap_or_default();
            let expected = &self.document.categories[*name];
            let actual_count = actual.len() as u64;
            let actual_sha256 = job_ids_sha256(&actual);
            if actual_count != expected.job_count || actual_sha256 != expected.job_ids_sha256 {
                return Err(format!(
                    "Worker Job recovery category {name} changed: expected count={} sha256={}, actual count={actual_count} sha256={actual_sha256}",
                    expected.job_count, expected.job_ids_sha256
                ));
            }
            for job_id in actual {
                if let Some(previous) = seen.insert(job_id, name) {
                    return Err(format!(
                        "Worker Job {job_id} matched multiple recovery categories: {previous}, {name}"
                    ));
                }
                if *action == "omit" && audit.omitted.get(&job_id) != Some(name) {
                    return Err(format!(
                        "Worker Job {job_id} category {name} did not produce its exact omission"
                    ));
                }
            }
        }
        if audit
            .omitted
            .iter()
            .any(|(job_id, category)| seen.get(job_id) != Some(category))
        {
            return Err("Worker Job recovery produced an undeclared omission".to_string());
        }
        Ok(())
    }
}

impl RecoveryJobAudit {
    pub(crate) fn normalize(&mut self, category: &'static str, job_id: i64) {
        self.categories.entry(category).or_default().insert(job_id);
    }

    pub(crate) fn omit(&mut self, category: &'static str, job_id: i64) {
        self.categories.entry(category).or_default().insert(job_id);
        self.omitted.insert(job_id, category);
    }

    pub(crate) fn omitted_ids(&self) -> BTreeSet<i64> {
        self.omitted.keys().copied().collect()
    }

    pub(crate) fn classifications(&self) -> Vec<RecoveryJobClassification> {
        let omitted = self.omitted.keys().copied().collect::<BTreeSet<_>>();
        let mut rows = self
            .categories
            .iter()
            .flat_map(|(category, jobs)| {
                jobs.iter().map(|job_id| RecoveryJobClassification {
                    source_job_id: *job_id,
                    action: if omitted.contains(job_id) {
                        "omit"
                    } else {
                        "normalize"
                    },
                    category,
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.source_job_id);
        rows
    }
}

pub(crate) fn source_jobs_sha256(jobs: &[SourceJobRow]) -> Result<String, String> {
    let values = jobs
        .iter()
        .map(|job| {
            json!({
                "job_id": job.job_id,
                "repo_name": job.repo_name,
                "repo_id": job.repo_id,
                "job_type": job.job_type,
                "state": job.state,
                "payload_json": job.payload_json,
                "result_json": job.result_json,
                "attempt_count": job.attempt_count,
                "max_attempts": job.max_attempts,
                "available_at": job.available_at.to_rfc3339(),
                "locked_at": job.locked_at.map(|value| value.to_rfc3339()),
                "locked_by": job.locked_by,
                "last_error": job.last_error,
                "created_at": job.created_at.to_rfc3339(),
                "updated_at": job.updated_at.to_rfc3339(),
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&values)
        .map(|bytes| sha256(&bytes))
        .map_err(|error| format!("failed to fingerprint source Worker Jobs: {error}"))
}

fn job_ids_sha256(ids: &BTreeSet<i64>) -> String {
    let mut bytes = Vec::new();
    for job_id in ids {
        bytes.extend_from_slice(job_id.to_string().as_bytes());
        bytes.push(b'\n');
    }
    sha256(&bytes)
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!(
            "Worker Job recovery {label} is not lowercase SHA-256"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{job_ids_sha256, RecoveryJobAudit, LEGACY_PATCHSET_OMISSION};
    use std::collections::BTreeSet;

    #[test]
    fn job_id_digest_is_sorted_decimal_lines() {
        let ids = BTreeSet::from([12, 3, 101]);
        assert_eq!(
            job_ids_sha256(&ids),
            crate::activation::sha256(b"3\n12\n101\n")
        );
    }

    #[test]
    fn classifications_distinguish_omission_from_normalization() {
        let mut audit = RecoveryJobAudit::default();
        audit.omit(LEGACY_PATCHSET_OMISSION, 9);
        audit.normalize(super::ATTACHED_TERMINAL_EXACT, 10);
        let rows = audit.classifications();
        assert_eq!(rows[0].action, "omit");
        assert_eq!(rows[1].action, "normalize");
    }
}
