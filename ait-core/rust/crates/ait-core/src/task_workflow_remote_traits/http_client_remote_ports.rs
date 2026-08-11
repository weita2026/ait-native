use super::http_client_types::TaskWorkflowHttpClientStats;

pub trait TaskWorkflowHttpClientInspector {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats;
}

pub trait TaskWorkflowHttpClientCloser {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats;
}

pub trait TaskWorkflowHttpClientRemote:
    TaskWorkflowHttpClientInspector + TaskWorkflowHttpClientCloser
{
}

impl<R> TaskWorkflowHttpClientRemote for R where
    R: TaskWorkflowHttpClientInspector + TaskWorkflowHttpClientCloser + ?Sized
{
}
