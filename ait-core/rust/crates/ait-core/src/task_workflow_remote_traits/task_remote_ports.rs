use super::change_remote_ports::TaskWorkflowChangeRemote;
use super::http_client_remote_ports::TaskWorkflowHttpClientRemote;
use super::line_remote_ports::TaskWorkflowLineRemote;
use super::queue_remote_ports::TaskWorkflowQueueRemote;
use super::repository_remote_ports::TaskWorkflowRepositoryRemote;
use super::snapshot_remote_ports::TaskWorkflowSnapshotRemote;
use super::task_record_remote_ports::TaskWorkflowTaskRecordRemote;

pub trait TaskWorkflowTaskRemote:
    TaskWorkflowHttpClientRemote
    + TaskWorkflowRepositoryRemote
    + TaskWorkflowLineRemote
    + TaskWorkflowTaskRecordRemote
    + TaskWorkflowQueueRemote
    + TaskWorkflowChangeRemote
    + TaskWorkflowSnapshotRemote
{
}

impl<R> TaskWorkflowTaskRemote for R where
    R: TaskWorkflowHttpClientRemote
        + TaskWorkflowRepositoryRemote
        + TaskWorkflowLineRemote
        + TaskWorkflowTaskRecordRemote
        + TaskWorkflowQueueRemote
        + TaskWorkflowChangeRemote
        + TaskWorkflowSnapshotRemote
        + ?Sized
{
}
