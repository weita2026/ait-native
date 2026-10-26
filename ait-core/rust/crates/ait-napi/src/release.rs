//! Retained workflow release record and operation SDK exports.
use crate::repository::repository;
use ait_cli::release_surface as releases;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
pub struct CreateWorkflowReleaseInput {
    pub release_id: String,
    pub repo_name: String,
    pub version: String,
    pub line_name: String,
    pub snapshot_id: String,
    pub manifest_hash: String,
    pub profile: String,
    pub package_name: Option<String>,
    pub package_version: Option<String>,
    pub package_requires_python: Option<String>,
    pub status: Option<String>,
    pub checks_json: Option<String>,
    pub artifacts_json: Option<String>,
    pub formula_json: Option<String>,
    pub metadata_json: Option<String>,
}
#[napi]
pub fn create_workflow_release(
    root: String,
    input: CreateWorkflowReleaseInput,
) -> Result<JsonValue> {
    releases::create_workflow_release_explicit(
        &repository(&root)?,
        &input.release_id,
        &input.repo_name,
        &input.version,
        &input.line_name,
        &input.snapshot_id,
        &input.manifest_hash,
        &input.profile,
        input.package_name.as_deref(),
        input.package_version.as_deref(),
        input.package_requires_python.as_deref(),
        input.status.as_deref(),
        input.checks_json.as_deref().unwrap_or("[]"),
        input.artifacts_json.as_deref().unwrap_or("[]"),
        input.formula_json.as_deref().unwrap_or("{}"),
        input.metadata_json.as_deref().unwrap_or("{}"),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn list_workflow_releases(root: String) -> Result<JsonValue> {
    releases::list_workflow_releases(&repository(&root)?).map_err(Error::from_reason)
}
#[napi]
pub fn get_workflow_release(root: String, release_id: String) -> Result<JsonValue> {
    releases::get_workflow_release(&repository(&root)?, &release_id).map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct UpdateWorkflowReleaseOptions {
    pub status: Option<String>,
    pub checks_json: Option<String>,
    pub checks_set: Option<bool>,
    pub artifacts_json: Option<String>,
    pub artifacts_set: Option<bool>,
    pub formula_json: Option<String>,
    pub formula_set: Option<bool>,
    pub metadata_json: Option<String>,
    pub metadata_set: Option<bool>,
}
#[napi]
pub fn update_workflow_release(
    root: String,
    release_id: String,
    options: Option<UpdateWorkflowReleaseOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    releases::update_workflow_release_explicit(
        &repository(&root)?,
        &release_id,
        options.status.as_deref(),
        options
            .checks_set
            .unwrap_or(false)
            .then_some(options.checks_json.as_deref().unwrap_or("null")),
        options
            .artifacts_set
            .unwrap_or(false)
            .then_some(options.artifacts_json.as_deref().unwrap_or("null")),
        options
            .formula_set
            .unwrap_or(false)
            .then_some(options.formula_json.as_deref().unwrap_or("null")),
        options
            .metadata_set
            .unwrap_or(false)
            .then_some(options.metadata_json.as_deref().unwrap_or("null")),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_release_candidate_create(
    root: String,
    version: String,
    line: String,
    profile: String,
) -> Result<JsonValue> {
    releases::release_candidate_create(&repository(&root)?, &version, &line, &profile)
        .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct ReleaseCheckSdkOptions {
    pub tests_command: Option<String>,
    pub skip_tests_reason: Option<String>,
}
#[napi]
pub fn task_workflow_release_check(
    root: String,
    release_id: String,
    options: Option<ReleaseCheckSdkOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    releases::release_check(
        &repository(&root)?,
        &release_id,
        options.tests_command.as_deref(),
        options.skip_tests_reason.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_release_build(root: String, release_id: String) -> Result<JsonValue> {
    releases::release_build(&repository(&root)?, &release_id).map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_release_formula(
    root: String,
    release_id: String,
    name: String,
) -> Result<JsonValue> {
    releases::release_formula(&repository(&root)?, &release_id, &name).map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_release_show(
    root: String,
    release_id: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    releases::release_show(&repository(&root)?, &release_id, remote_name.as_deref())
        .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_release_publish(
    root: String,
    release_id: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    releases::release_publish(&repository(&root)?, &release_id, remote_name.as_deref())
        .map_err(Error::from_reason)
}

// Active language CLI operations. These share native family/legacy selection.
use ait_cli::binding_release as service;
use std::path::PathBuf;

#[napi(object)]
#[derive(Default)]
pub struct ReleaseAdapterOptions {
    pub line_name: Option<String>,
    pub target: Option<String>,
}
#[napi]
pub fn release_adapter_check(
    root: String,
    version: String,
    options: Option<ReleaseAdapterOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::check_release_adapter(
        &repository(&root)?,
        &version,
        options.line_name.as_deref().unwrap_or("main"),
        options.target.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn release_adapter_build(
    root: String,
    version: String,
    options: Option<ReleaseAdapterOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::build_release_adapter(
        &repository(&root)?,
        &version,
        options.line_name.as_deref().unwrap_or("main"),
        options.target.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct ReleaseCandidateOptions {
    pub line_name: Option<String>,
    pub profile: Option<String>,
    pub channel: Option<String>,
    pub public_source_root: Option<String>,
}
#[napi]
pub fn release_candidate_create(
    root: String,
    version: String,
    options: Option<ReleaseCandidateOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::create_release_candidate(
        &repository(&root)?,
        &service::ReleaseCandidateRequest {
            version,
            line_name: options.line_name.unwrap_or_else(|| "main".into()),
            profile: options.profile,
            channel: options.channel,
            public_source_root: options.public_source_root.map(PathBuf::from),
        },
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct ReleaseCheckOptions {
    pub receipts: Option<String>,
    pub public_source_root: Option<String>,
    pub tests_command: Option<String>,
    pub skip_tests_reason: Option<String>,
}
#[napi]
pub fn release_check(
    root: String,
    release_id: String,
    options: Option<ReleaseCheckOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::check_release(
        &repository(&root)?,
        &service::ReleaseCheckRequest {
            release_id,
            receipts: options.receipts.map(PathBuf::from),
            public_source_root: options.public_source_root.map(PathBuf::from),
            tests_command: options.tests_command,
            skip_tests_reason: options.skip_tests_reason,
        },
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct ReleaseBuildOptions {
    pub receipts: Option<String>,
    pub public_source_root: Option<String>,
    pub native_matrix_dir: Option<String>,
    pub native_command_dir: Option<String>,
}
#[napi]
pub fn release_build(
    root: String,
    release_id: String,
    options: Option<ReleaseBuildOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::build_release(
        &repository(&root)?,
        &service::ReleaseBuildRequest {
            release_id,
            receipts: options.receipts.map(PathBuf::from),
            public_source_root: options.public_source_root.map(PathBuf::from),
            native_matrix_dir: options.native_matrix_dir.map(PathBuf::from),
            native_command_dir: options.native_command_dir.map(PathBuf::from),
        },
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
pub struct ReleaseNativeSourceInput {
    pub release_id: String,
    pub target: String,
    pub source_dir: String,
    pub runner: String,
    pub runner_image: String,
    pub rust_toolchain: String,
    pub rustc_path: String,
}
#[napi]
pub fn release_native_source(root: String, request: ReleaseNativeSourceInput) -> Result<JsonValue> {
    service::capture_release_native_source(
        &repository(&root)?,
        &releases::NativeSourceRequest {
            release_id: request.release_id,
            target: request.target,
            source_dir: PathBuf::from(request.source_dir),
            runner: request.runner,
            runner_image: request.runner_image,
            rust_toolchain: request.rust_toolchain,
            rustc_path: PathBuf::from(request.rustc_path),
        },
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn release_native_bundle(
    root: String,
    release_id: String,
    native_matrix_dir: String,
) -> Result<JsonValue> {
    service::bundle_release_native(
        &repository(&root)?,
        &release_id,
        &PathBuf::from(native_matrix_dir),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn release_package(
    root: String,
    release_id: String,
    channel: String,
    public_source_root: Option<String>,
) -> Result<JsonValue> {
    service::package_release(
        &repository(&root)?,
        &release_id,
        &channel,
        public_source_root.map(PathBuf::from).as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn release_formula(
    root: String,
    release_id: String,
    name: String,
    python_formula: Option<String>,
) -> Result<JsonValue> {
    service::generate_release_formula(
        &repository(&root)?,
        &release_id,
        &name,
        python_formula
            .as_deref()
            .unwrap_or(releases::DEFAULT_HOMEBREW_PYTHON_FORMULA),
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct ReleaseShowOptions {
    pub remote_name: Option<String>,
    pub public_source_root: Option<String>,
}
#[napi]
pub fn release_show(
    root: String,
    release_id: String,
    options: Option<ReleaseShowOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::show_release(
        &repository(&root)?,
        &service::ReleaseShowRequest {
            release_id,
            remote: options.remote_name,
            public_source_root: options.public_source_root.map(PathBuf::from),
        },
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn release_publish(
    root: String,
    release_id: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    service::publish_release(&repository(&root)?, &release_id, remote_name.as_deref())
        .map_err(Error::from_reason)
}
#[napi]
pub fn release_promote(
    root: String,
    release_id: String,
    channel: String,
    public_source_root: Option<String>,
) -> Result<JsonValue> {
    service::promote_release(
        &repository(&root)?,
        &release_id,
        &channel,
        public_source_root.map(PathBuf::from).as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn format_release(payload: crate::JsonInput) -> String {
    releases::format_release_text(&payload)
}
