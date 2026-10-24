use crate::json_support::JsonValue;

pub(super) trait PlanSyncLocalArtifactBodySource {
    fn read_plan_revision_artifact_body(
        &self,
        repo_root: &str,
        revision: &JsonValue,
    ) -> Option<String>;
}

pub(super) fn read_plan_revision_artifact_body_with_plan_sync_local_artifact_body_source<S>(
    source: &S,
    repo_root: &str,
    revision: &JsonValue,
) -> Option<String>
where
    S: PlanSyncLocalArtifactBodySource + ?Sized,
{
    source.read_plan_revision_artifact_body(repo_root, revision)
}
