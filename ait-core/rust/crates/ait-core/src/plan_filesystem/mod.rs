use crate::json_support::{JsonMap as Map, JsonValue};
use regex::Regex;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use zip::ZipArchive;

use crate::external::status::inspect_operational_external_projection_roots;
use crate::file_io::{FileIoError, FileIoErrorKind, FileIoStore, FilesystemFileIoStore};
use crate::json_support::{expand_home_path_with_file_io_store, JsonCodec, JsonEncodeOptions};
use crate::shared_foundation::ArtifactResolver;

const IGNORED_DIRS: &[&str] = &[
    ".ait",
    ".ait-runtime",
    ".ait-worktree",
    ".ait-worktree-links",
    ".git",
    "__pycache__",
    ".pytest_cache",
    ".venv",
    "venv",
    ".mypy_cache",
];
const IGNORED_FILES: &[&str] = &[".DS_Store", ".ait-worktree.json"];
const WORKSPACE_IGNORE_FILE: &str = ".aitignore";
const WORKTREE_CONFIG_NAME: &str = ".ait-worktree.json";
const WORKTREE_CARGO_CONFIG_RELATIVE_PATH: &str = ".cargo/config.toml";
const SHARED_CARGO_TARGET_DIRNAME: &str = "cargo-target";
const SHARED_CARGO_BUILD_DIRNAME: &str = "cargo-build";
const CARGO_WORKSPACE_PATH_HASH_TEMPLATE: &str = "{workspace-path-hash}";
const MANAGED_WORKTREE_CARGO_BUILD_DIRNAME: &str = "task-workspaces";
const GENERATED_CARGO_CONFIG_HEADER: &str =
    "# Managed by ait: stable final artifacts, workspace-isolated intermediates.";
const REPOSITORY_SHARED_GENERATED_CARGO_CONFIG_HEADER: &str =
    "# Managed by ait: stable final artifacts, repository-shared intermediates.";
const WORKTREE_LOCAL_GENERATED_CARGO_CONFIG_HEADER: &str =
    "# Managed by ait: stable final artifacts, worktree-local intermediates.";
const LEGACY_GENERATED_CARGO_CONFIG_HEADER: &str =
    "# Managed by ait to share Rust build artifacts across task worktrees.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanFilesystemError {
    Invalid(String),
    NotFound(String),
    MissingEntry(String),
    Io(String),
}

#[derive(Debug, Clone)]
struct WorkspaceIgnoreRule {
    pattern: String,
    regex: Option<Regex>,
    negated: bool,
    directory_only: bool,
    anchored: bool,
    basename_only: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceIgnoreMatcher {
    rules: Vec<WorkspaceIgnoreRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VisibleWorkspaceEntries {
    pub files: Vec<String>,
    pub file_metadata: BTreeMap<String, VisibleWorkspaceFileMetadata>,
    pub directories: Vec<String>,
    pub operational_external_roots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleWorkspaceFileMetadata {
    pub file_kind: String,
    pub size_bytes: u64,
    pub mode_bits: u32,
    pub modified_ns: u64,
    pub changed_ns: u64,
    pub device_id: u64,
    pub file_id: u64,
}

mod archive_io;
mod artifact_resolver;
mod ignore_policy;
mod path_helpers;
mod path_policy;
mod text_json_io;
mod workspace_discovery;

pub use self::archive_io::*;
pub use self::artifact_resolver::*;
use self::ignore_policy::*;
use self::path_helpers::*;
pub use self::path_policy::*;
pub use self::text_json_io::*;
pub use self::workspace_discovery::*;

#[cfg(test)]
mod tests;
