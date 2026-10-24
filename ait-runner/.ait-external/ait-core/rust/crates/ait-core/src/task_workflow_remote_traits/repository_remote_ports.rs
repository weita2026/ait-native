use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowRepositoryEnsurer {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&Value>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowRepositoryReader {
    fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowRepositoryRemote:
    TaskWorkflowRepositoryEnsurer + TaskWorkflowRepositoryReader
{
}

impl<R> TaskWorkflowRepositoryRemote for R where
    R: TaskWorkflowRepositoryEnsurer + TaskWorkflowRepositoryReader + ?Sized
{
}
