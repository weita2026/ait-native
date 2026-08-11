use super::artifact_state_ports::PlanSyncLocalArtifactStateSource;
use super::plan_fs_error;
use std::collections::BTreeSet;
use std::path::Path;

const IGNORED_DIRS: &[&str] = &[
    ".ait",
    ".ait-runtime",
    ".git",
    "__pycache__",
    ".pytest_cache",
    ".venv",
    "venv",
    ".mypy_cache",
];

pub(super) struct FilesystemPlanSyncLocalArtifactStateSource;

impl PlanSyncLocalArtifactStateSource for FilesystemPlanSyncLocalArtifactStateSource {
    fn existing_artifact_paths(
        &self,
        repo_root: &str,
        artifact_paths: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, String> {
        Ok(plan_sync_existing_artifact_paths(repo_root, artifact_paths))
    }

    fn ignored_artifact_paths(
        &self,
        repo_root: &str,
        artifact_paths: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, String> {
        plan_sync_ignored_artifact_paths(repo_root, artifact_paths)
    }
}

pub(super) fn plan_sync_existing_artifact_paths(
    repo_root: &str,
    artifact_paths: &BTreeSet<String>,
) -> BTreeSet<String> {
    artifact_paths
        .iter()
        .filter(|path| Path::new(repo_root).join(path.as_str()).exists())
        .cloned()
        .collect()
}

pub(super) fn plan_sync_ignored_artifact_paths(
    repo_root: &str,
    artifact_paths: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String> {
    let mut ignored = BTreeSet::new();
    for path in artifact_paths {
        if plan_sync_artifact_path_is_ignored(repo_root, path)? {
            ignored.insert(path.clone());
        }
    }
    Ok(ignored)
}

pub(super) fn plan_sync_artifact_path_is_ignored(
    repo_root: &str,
    artifact_path: &str,
) -> Result<bool, String> {
    if artifact_path_has_ignored_dir(artifact_path) {
        return Ok(true);
    }
    crate::plan_filesystem::workspace_path_is_ignored(repo_root, artifact_path, None)
        .map_err(plan_fs_error)
}

fn artifact_path_has_ignored_dir(artifact_path: &str) -> bool {
    Path::new(artifact_path).components().any(|part| {
        let name = part.as_os_str().to_string_lossy();
        IGNORED_DIRS.iter().any(|ignored| name == *ignored)
    })
}
