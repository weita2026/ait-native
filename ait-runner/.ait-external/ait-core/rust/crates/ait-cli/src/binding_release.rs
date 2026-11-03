//! Operation-specific release services shared by native and language frontends.
//! Selection and conflict checks stay with the existing release implementation.
use crate::release_surface::*;
use crate::runtime::RepoRuntime;
use ait_core::json_support::JsonValue;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ReleaseCandidateRequest {
    pub version: String,
    pub line_name: String,
    pub profile: Option<String>,
    pub channel: Option<String>,
    pub public_source_root: Option<PathBuf>,
}
#[derive(Clone, Debug, Default)]
pub struct ReleaseCheckRequest {
    pub release_id: String,
    pub receipts: Option<PathBuf>,
    pub public_source_root: Option<PathBuf>,
    pub tests_command: Option<String>,
    pub skip_tests_reason: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct ReleaseBuildRequest {
    pub release_id: String,
    pub receipts: Option<PathBuf>,
    pub public_source_root: Option<PathBuf>,
    pub native_matrix_dir: Option<PathBuf>,
    pub native_command_dir: Option<PathBuf>,
}
#[derive(Clone, Debug, Default)]
pub struct ReleaseShowRequest {
    pub release_id: String,
    pub remote: Option<String>,
    pub public_source_root: Option<PathBuf>,
}

pub fn check_release_adapter(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    target: Option<&str>,
) -> Result<JsonValue, String> {
    release_adapter_check_for_target(repo, version, line_name, target)
}
pub fn build_release_adapter(
    repo: &RepoRuntime,
    version: &str,
    line_name: &str,
    target: Option<&str>,
) -> Result<JsonValue, String> {
    release_adapter_build_for_target(repo, version, line_name, target)
}

pub fn create_release_candidate(
    repo: &RepoRuntime,
    request: &ReleaseCandidateRequest,
) -> Result<JsonValue, String> {
    if request.public_source_root.is_some()
        && request.profile.as_deref() != Some(FAMILY_RELEASE_PROFILE)
    {
        return Err(
            "--public-source-root is a protected-CI source input and requires --profile family."
                .to_string(),
        );
    }
    let family = request.public_source_root.is_some()
        || request.profile.as_deref() == Some(FAMILY_RELEASE_PROFILE)
        || (request.profile.is_none() && family_manifest_exists(repo, &request.line_name)?);
    let payload = if family {
        if let Some(public_source_root) = request.public_source_root.as_deref() {
            family_release_candidate_create_from_public_source(
                repo,
                &request.version,
                &request.line_name,
                request.channel.as_deref(),
                public_source_root,
            )?
        } else {
            family_release_candidate_create(
                repo,
                &request.version,
                &request.line_name,
                request.channel.as_deref(),
            )?
        }
    } else {
        if request.channel.is_some() {
            return Err(
                            "--channel is supported only by a family release candidate. Add ait-release-family.json to the selected Snapshot or pass --profile family."
                                .to_string(),
                        );
        }
        release_candidate_create(
            repo,
            &request.version,
            &request.line_name,
            request.profile.as_deref().unwrap_or("local-cli"),
        )?
    };
    Ok(payload)
}

pub fn check_release(
    repo: &RepoRuntime,
    request: &ReleaseCheckRequest,
) -> Result<JsonValue, String> {
    if family_candidate_exists(repo, &request.release_id) {
        if request.tests_command.is_some() || request.skip_tests_reason.is_some() {
            return Err("Family release checks consume immutable component receipts; --tests-command and --skip-tests-reason apply only to legacy profiles.".to_string());
        }
        let receipts = request.receipts.as_deref().ok_or_else(|| {
            format!(
                "Family release {} requires --receipts <dir>.",
                request.release_id
            )
        })?;
        family_release_check(
            repo,
            &request.release_id,
            receipts,
            request.public_source_root.as_deref(),
        )
    } else {
        if request.public_source_root.is_some() {
            return Err(
                "--public-source-root applies only to a family release candidate.".to_string(),
            );
        }
        if request.receipts.is_some() {
            return Err("--receipts applies only to a family release candidate.".to_string());
        }
        release_check(
            repo,
            &request.release_id,
            request.tests_command.as_deref(),
            request.skip_tests_reason.as_deref(),
        )
    }
}

pub fn build_release(
    repo: &RepoRuntime,
    request: &ReleaseBuildRequest,
) -> Result<JsonValue, String> {
    let payload = if family_candidate_exists(repo, &request.release_id) {
        if request.native_matrix_dir.is_some() || request.native_command_dir.is_some() {
            return Err(
                        "Family release builds consume component receipts; --native-matrix-dir and --native-command-dir apply only to legacy native profiles."
                            .to_string(),
                    );
        }
        let receipts = request.receipts.as_deref().ok_or_else(|| {
            format!(
                "Family release {} requires --receipts <dir>.",
                request.release_id
            )
        })?;
        family_release_build(
            repo,
            &request.release_id,
            receipts,
            request.public_source_root.as_deref(),
        )?
    } else {
        if request.public_source_root.is_some() {
            return Err(
                "--public-source-root applies only to a family release candidate.".to_string(),
            );
        }
        if request.receipts.is_some() {
            return Err("--receipts applies only to a family release candidate.".to_string());
        }
        release_build_with_native_inputs(
            repo,
            &request.release_id,
            request.native_matrix_dir.as_deref(),
            request.native_command_dir.as_deref(),
        )?
    };
    Ok(payload)
}

pub fn show_release(repo: &RepoRuntime, request: &ReleaseShowRequest) -> Result<JsonValue, String> {
    let payload = if family_candidate_exists(repo, &request.release_id) {
        if request.remote.is_some() {
            return Err(
                "Family release dossiers are local portable evidence; --remote is not supported."
                    .to_string(),
            );
        }
        family_release_show(
            repo,
            &request.release_id,
            request.public_source_root.as_deref(),
        )?
    } else {
        if request.public_source_root.is_some() {
            return Err(
                "--public-source-root applies only to a family release dossier.".to_string(),
            );
        }
        release_show(repo, &request.release_id, request.remote.as_deref())?
    };
    Ok(payload)
}

pub fn capture_release_native_source(
    repo: &RepoRuntime,
    request: &NativeSourceRequest,
) -> Result<JsonValue, String> {
    release_native_source(repo, request)
}
pub fn bundle_release_native(
    repo: &RepoRuntime,
    release_id: &str,
    native_matrix_dir: &Path,
) -> Result<JsonValue, String> {
    release_native_bundle(repo, release_id, native_matrix_dir)
}
pub fn package_release(
    repo: &RepoRuntime,
    release_id: &str,
    channel: &str,
    public_source_root: Option<&Path>,
) -> Result<JsonValue, String> {
    family_release_package(repo, release_id, channel, public_source_root)
}
pub fn generate_release_formula(
    repo: &RepoRuntime,
    release_id: &str,
    name: &str,
    python_formula: &str,
) -> Result<JsonValue, String> {
    release_formula_with_python(repo, release_id, name, python_formula)
}
pub fn publish_release(
    repo: &RepoRuntime,
    release_id: &str,
    remote: Option<&str>,
) -> Result<JsonValue, String> {
    if family_candidate_exists(repo, release_id) {
        return Err(family_release_publish_error(release_id));
    }
    release_publish(repo, release_id, remote)
}
pub fn promote_release(
    repo: &RepoRuntime,
    release_id: &str,
    channel: &str,
    public_source_root: Option<&Path>,
) -> Result<JsonValue, String> {
    family_release_promote(repo, release_id, channel, public_source_root)
}
