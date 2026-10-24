use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;
use super::patchset_remote_ports::TaskWorkflowPatchsetReader;

pub trait TaskWorkflowPatchsetCiStatusReader {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;

    fn read_patchset_ci_readiness(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.read_patchset_ci_status(patchset_id, recent_limit, repo_name, exact_id)
    }
}

pub trait TaskWorkflowRepoJobLister {
    fn list_repo_jobs(
        &mut self,
        repo_name: &str,
        state: Option<&str>,
        limit: i64,
        diagnostics: bool,
        stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowPatchsetCiRemote:
    TaskWorkflowPatchsetReader + TaskWorkflowPatchsetCiStatusReader + TaskWorkflowRepoJobLister
{
}

impl<R> TaskWorkflowPatchsetCiRemote for R where
    R: TaskWorkflowPatchsetReader
        + TaskWorkflowPatchsetCiStatusReader
        + TaskWorkflowRepoJobLister
        + ?Sized
{
}
