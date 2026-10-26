use std::collections::BTreeSet;

pub(super) trait PlanSyncLocalArtifactStateSource {
    fn existing_artifact_paths(
        &self,
        repo_root: &str,
        artifact_paths: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, String>;

    fn ignored_artifact_paths(
        &self,
        repo_root: &str,
        artifact_paths: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, String>;
}

pub(super) fn existing_artifact_paths_with_plan_sync_local_artifact_state_source<S>(
    source: &S,
    repo_root: &str,
    artifact_paths: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String>
where
    S: PlanSyncLocalArtifactStateSource + ?Sized,
{
    source.existing_artifact_paths(repo_root, artifact_paths)
}

pub(super) fn ignored_artifact_paths_with_plan_sync_local_artifact_state_source<S>(
    source: &S,
    repo_root: &str,
    artifact_paths: &BTreeSet<String>,
) -> Result<BTreeSet<String>, String>
where
    S: PlanSyncLocalArtifactStateSource + ?Sized,
{
    source.ignored_artifact_paths(repo_root, artifact_paths)
}
