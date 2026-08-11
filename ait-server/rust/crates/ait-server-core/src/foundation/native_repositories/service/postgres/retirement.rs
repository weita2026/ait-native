use super::*;
use crate::foundation::pack_substrate::tree_pack_contains_blob_ids_with_format;
use crate::foundation::server_protocol::encode_ref_name;
use ::postgres::types::ToSql;
use std::env;
use std::fs;
use std::io::Write;

#[path = "retirement/common.rs"]
mod common;
#[path = "retirement/export.rs"]
mod export;
#[path = "retirement/files.rs"]
mod files;
#[path = "retirement/lifecycle.rs"]
mod lifecycle;
#[path = "retirement/preflight.rs"]
mod preflight;
#[path = "retirement/purge.rs"]
mod purge;

use common::*;
use export::*;
use files::*;
use lifecycle::*;
use preflight::*;
use purge::*;

pub(in crate::foundation::native_repositories) const RETIRE_EXPORT_ROOT_ENV: &str =
    "AIT_SERVER_RETIRE_EXPORT_ROOT";
const RETIREMENT_STATE_EXPORTED: &str = "exported";
const RETIREMENT_STATE_PURGED: &str = "purged";
const RETIREMENT_STATE_FAILED: &str = "failed";
const BLOB_REFERENCE_SAMPLE_LIMIT: usize = 25;

const REPO_SCOPED_CONTROL_TABLES: &[&str] = &[
    "tasks",
    "plans",
    "plan_revision_blobs",
    "plan_revision_artifacts",
    "changes",
    "releases",
    "patchsets",
    "review_requests",
    "reviews",
    "attestations",
    "policy_decisions",
    "waivers",
    "land_requests",
    "role_bindings",
    "jobs",
    "authority_maps",
];

const PURGE_REPO_SCOPED_CONTROL_TABLES: &[&str] = &[
    "review_requests",
    "reviews",
    "attestations",
    "policy_decisions",
    "waivers",
    "land_requests",
    "patchsets",
    "jobs",
    "releases",
    "changes",
    "tasks",
    "plan_revision_artifacts",
    "plan_revision_blobs",
    "plans",
    "role_bindings",
    "authority_maps",
];

#[derive(Debug, Clone)]
struct WrittenFile {
    path: PathBuf,
    sha256: String,
    size_bytes: u64,
}

pub(in crate::foundation::native_repositories) fn retire_repository_json(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    request: RetireRepositoryRequest,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo_name = normalize_required_text(repo_name, "repo_name")?;
    let expected_repo_id = normalize_required_text(&request.expected_repo_id, "expected_repo_id")?;
    let actor_identity = request
        .actor_identity
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("anonymous")
        .to_string();
    let actor_type = request
        .actor_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("human")
        .to_string();
    let repository = service.with_read(|client| get_repository_json(client, &repo_name))?;
    let repo_id = repository
        .get("repo_id")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if repo_id != expected_repo_id {
        return Err(NativeRepositoryError::bad_request(format!(
            "Repository scope mismatch for {repo_name}: repo_id {expected_repo_id} does not match {repo_id}"
        )));
    }

    let blockers = preflight_blockers(service, &repo_name, &repo_id)?;
    if !blockers.is_empty() {
        let details = blockers
            .iter()
            .map(|(key, value)| {
                let count = value.as_array().map(Vec::len).unwrap_or(1);
                format!("{key}={count}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(NativeRepositoryError::conflict(format!(
            "Repository {repo_name} cannot be retired while blockers remain: {details}"
        )));
    }

    set_repository_lifecycle_state(service, &repo_name, &repo_id, "retiring")?;
    let mut retirement_id: Option<String> = None;
    match (|| {
        let export_result = export_bundle(service, &repo_name, &repo_id)?;
        let verification = if request.require_verified_export {
            verify_export(
                &export_result.export_path,
                &export_result.manifest_path,
                &export_result.manifest_sha256,
            )?
        } else {
            json!({
                "verified": false,
                "verified_file_count": 0,
                "verified_total_bytes": 0
            })
        };
        let summary = json!({
            "repo_name": repo_name,
            "repo_id": repo_id,
            "require_verified_export": request.require_verified_export,
            "verification": verification,
            "manifest": {
                "snapshot_count": export_result.manifest["snapshot_count"].as_i64().unwrap_or(0),
                "control_blob_count": export_result.manifest["control_blob_count"].as_i64().unwrap_or(0),
                "file_count": export_result.manifest["files"].as_array().map(Vec::len).unwrap_or(0)
            }
        });
        let new_retirement_id = new_identifier("RTR", &format!("{repo_name}|{repo_id}"));
        retirement_id = Some(new_retirement_id.clone());
        insert_retirement_record(
            service,
            &new_retirement_id,
            &repo_name,
            &repo_id,
            &actor_identity,
            &actor_type,
            &export_result.export_path,
            &export_result.manifest_path,
            &export_result.manifest_sha256,
            &summary,
        )?;
        let purge = purge_repository(service, &repo_name, &repo_id)?;
        update_retirement_record(
            service,
            &new_retirement_id,
            RETIREMENT_STATE_PURGED,
            None,
            Some(json!({ "purge": purge.clone() })),
        )?;
        Ok::<JsonValue, NativeRepositoryError>(json!({
            "retirement_id": new_retirement_id,
            "repo_name": repo_name,
            "repo_id": repo_id,
            "export_path": path_string(&export_result.export_path),
            "manifest_path": path_string(&export_result.manifest_path),
            "manifest_sha256": export_result.manifest_sha256,
            "verification": verification,
            "purge": purge
        }))
    })() {
        Ok(value) => Ok(value),
        Err(error) => {
            if let Some(id) = retirement_id {
                let _ = update_retirement_record(
                    service,
                    &id,
                    RETIREMENT_STATE_FAILED,
                    Some(&error.message),
                    None,
                );
            } else {
                let _ = set_repository_lifecycle_state(service, &repo_name, &repo_id, "active");
            }
            Err(error)
        }
    }
}
