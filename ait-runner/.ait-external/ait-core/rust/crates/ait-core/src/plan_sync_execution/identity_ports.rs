pub(super) trait PlanSyncWorkflowIdentitySource {
    fn workflow_id(&self, family: &str, namespace_prefix: Option<&str>) -> Result<String, String>;
    fn timestamp(&self) -> Result<String, String>;
}

pub(super) fn workflow_id_with_plan_sync_workflow_identity_source<S>(
    source: &S,
    family: &str,
    namespace_prefix: Option<&str>,
) -> Result<String, String>
where
    S: PlanSyncWorkflowIdentitySource + ?Sized,
{
    source.workflow_id(family, namespace_prefix)
}

pub(super) fn timestamp_with_plan_sync_workflow_identity_source<S>(
    source: &S,
) -> Result<String, String>
where
    S: PlanSyncWorkflowIdentitySource + ?Sized,
{
    source.timestamp()
}
