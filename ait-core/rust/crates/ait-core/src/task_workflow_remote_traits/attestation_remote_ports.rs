use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowAttestationWriter {
    #[allow(clippy::too_many_arguments)]
    fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        statement: &crate::attest_json::RemoteAttestationStatement,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowAttestationReader {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowAttestationRemote:
    TaskWorkflowAttestationWriter + TaskWorkflowAttestationReader
{
}

impl<R> TaskWorkflowAttestationRemote for R where
    R: TaskWorkflowAttestationWriter + TaskWorkflowAttestationReader + ?Sized
{
}
