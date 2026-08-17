use crate::external_readiness_gate::{
    external_readiness_blocker_details, external_readiness_report_for_repo,
};
use crate::json_support::{
    encode_string_or, encode_value_pretty_to_vec, encode_value_pretty_with_newline_error_string,
    parse_slice_value, parse_value, parse_value_or,
};
use crate::runtime::{RemoteRow, RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT};
use ait_core::environment_contract::names;
use ait_core::external::readiness::ExternalReadinessReport;
use ait_core::external::release::{
    external_release_closure_metadata_from_lockfile_bytes, EXTERNAL_RELEASE_LOCKFILE_PATH,
};
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::local_snapshot::{
    export_snapshot_source_manifest_with_store, LocalSnapshotBlobReadStore, LocalSnapshotReadStore,
};
use ait_core::plan_http_client::{
    PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientManager,
};
use ait_core::task_store::{list_completed_tasks_with_landed_changes_with_task_store, TaskStore};
use ait_core::workflow_release_store::{
    create_workflow_release_with_store, get_workflow_release_with_store,
    latest_published_workflow_release_excluding_version_with_store,
    list_workflow_releases_with_store, update_workflow_release_with_store, WorkflowReleaseRecord,
    WorkflowReleaseStore, WorkflowReleaseUpdate,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use flate2::{write::GzEncoder, Compression, GzBuilder};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tar::{Builder as TarBuilder, Header};
use tempfile::{Builder as TempDirBuilder, NamedTempFile, TempDir};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

const PUBLIC_PACKAGE_TARGETS_CONTRACT_PATH: &str =
    "release/contracts/public_package_targets_contract.json";
const PUBLIC_FUTURE_REPO_PREP_CONTRACT_PATH: &str =
    "release/contracts/public_future_repo_extraction_prep_contract.json";
const PUBLIC_FUTURE_REPO_PREP_GUIDE_PATH: &str = "release/guides/FUTURE_REPOSITORY_PREP.md";
const PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_CONTRACT_PATH: &str =
    "release/contracts/public_future_repo_split_dry_run_contract.json";
const PUBLIC_FUTURE_REPO_SPLIT_DRY_RUN_GUIDE_PATH: &str =
    "release/guides/FUTURE_REPOSITORY_SPLIT_DRY_RUN.md";
const EXTERNAL_LOCKFILE_PATH: &str = EXTERNAL_RELEASE_LOCKFILE_PATH;
const NATIVE_RELEASE_SMOKE_COMPILEALL_SKIP_REASON: &str =
    "The Rust-only release artifact smoke does not execute Python; syntax and test coverage belong to the repository test suites.";
const REQUIRED_NATIVE_WORKER_COMMANDS: &[&str] = &["ait-agent-worker"];

#[derive(Clone, Debug)]
struct ReleaseProfile {
    id: &'static str,
    required_scripts: &'static [&'static str],
    forbidden_scripts: &'static [&'static str],
    release_docs: &'static [&'static str],
    license_files: &'static [&'static str],
    contributor_files: &'static [&'static str],
    quickstart_files: &'static [&'static str],
    excluded_paths: &'static [&'static str],
    setuptools_package_excludes: &'static [&'static str],
    description: &'static str,
    license: &'static str,
    readme_file: Option<&'static str>,
    keywords: &'static [&'static str],
    classifiers: &'static [&'static str],
    required_package_urls: &'static [&'static str],
    publish_support: bool,
}

#[derive(Clone, Debug)]
struct BundleEntry {
    path: String,
    data: Vec<u8>,
    mode: String,
}

#[derive(Clone, Debug, Default)]
struct PackageMetadata {
    name: String,
    version: String,
    description: Option<String>,
    readme: Option<JsonValue>,
    requires_python: Option<String>,
    license: Option<String>,
    license_files: Vec<String>,
    dependencies: Vec<String>,
    scripts: BTreeMap<String, String>,
    urls: BTreeMap<String, String>,
    classifiers: Vec<String>,
    keywords: Vec<String>,
}

struct ReleaseBundle {
    raw: JsonValue,
    files: BTreeMap<String, BundleEntry>,
}

fn filesystem_mode(metadata: &fs::Metadata, _non_unix_fallback: u32) -> u32 {
    crate::filesystem_permissions::portable_mode(metadata, _non_unix_fallback)
}

#[cfg(test)]
fn set_filesystem_mode(path: &Path, mode: u32) -> Result<(), String> {
    crate::filesystem_permissions::set_portable_mode(path, mode).map_err(io_error)
}

mod artifact_projection;
mod build_orchestration;
mod commands;
mod family_packages;
mod family_release;
mod generic_adapter;
mod native_distribution;
mod package_generation;
mod rendering;

use self::artifact_projection::*;
pub use self::build_orchestration::*;
pub use self::commands::*;
pub use self::family_packages::family_release_package;
pub use self::family_release::{
    family_candidate_exists, family_manifest_exists, family_release_publish_error,
    FAMILY_RELEASE_PROFILE,
};
pub use self::family_release::{
    family_release_build, family_release_candidate_create,
    family_release_candidate_create_from_public_source, family_release_check,
    family_release_promote, family_release_show,
};
use self::generic_adapter::*;
pub use self::generic_adapter::{
    release_adapter_build, release_adapter_build_for_target, release_adapter_check,
    release_adapter_check_for_target,
};
use self::native_distribution::*;
pub use self::native_distribution::{release_native_source, NativeSourceRequest};
pub use self::package_generation::*;
pub use self::rendering::*;

#[cfg(test)]
mod tests;
