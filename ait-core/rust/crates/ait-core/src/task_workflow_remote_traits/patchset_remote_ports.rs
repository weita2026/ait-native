use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;
use super::patchset_ci_remote_ports::TaskWorkflowPatchsetCiStatusReader;

pub trait TaskWorkflowPatchsetLister {
    fn list_patchsets(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<Value>>;
}

pub trait TaskWorkflowPatchsetReader {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowPatchsetPublisher {
    #[allow(clippy::too_many_arguments)]
    fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowPatchsetSelector {
    fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowPatchsetCiRunner {
    fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowPatchsetRemote:
    TaskWorkflowPatchsetLister
    + TaskWorkflowPatchsetReader
    + TaskWorkflowPatchsetPublisher
    + TaskWorkflowPatchsetSelector
    + TaskWorkflowPatchsetCiRunner
    + TaskWorkflowPatchsetCiStatusReader
{
}

impl<R> TaskWorkflowPatchsetRemote for R where
    R: TaskWorkflowPatchsetLister
        + TaskWorkflowPatchsetReader
        + TaskWorkflowPatchsetPublisher
        + TaskWorkflowPatchsetSelector
        + TaskWorkflowPatchsetCiRunner
        + TaskWorkflowPatchsetCiStatusReader
        + ?Sized
{
}
