use super::attestation_remote_ports::TaskWorkflowAttestationRemote;
use super::http_client_remote_ports::TaskWorkflowHttpClientRemote;
use super::land_remote_ports::TaskWorkflowLandRemote;
use super::mutation_receipt_remote_ports::TaskWorkflowMutationReceiptRemote;
use super::patchset_ci_remote_ports::TaskWorkflowPatchsetCiRemote;
use super::patchset_remote_ports::TaskWorkflowPatchsetRemote;
use super::policy_remote_ports::TaskWorkflowPolicyRemote;
use super::review_remote_ports::TaskWorkflowReviewRemote;
use super::task_lifecycle_remote_ports::TaskWorkflowRemoteTaskCloser;

pub trait TaskWorkflowCloseoutRemote:
    TaskWorkflowHttpClientRemote
    + TaskWorkflowMutationReceiptRemote
    + TaskWorkflowPatchsetRemote
    + TaskWorkflowPatchsetCiRemote
    + TaskWorkflowReviewRemote
    + TaskWorkflowAttestationRemote
    + TaskWorkflowPolicyRemote
    + TaskWorkflowLandRemote
    + TaskWorkflowRemoteTaskCloser
{
}

impl<R> TaskWorkflowCloseoutRemote for R where
    R: TaskWorkflowHttpClientRemote
        + TaskWorkflowMutationReceiptRemote
        + TaskWorkflowPatchsetRemote
        + TaskWorkflowPatchsetCiRemote
        + TaskWorkflowReviewRemote
        + TaskWorkflowAttestationRemote
        + TaskWorkflowPolicyRemote
        + TaskWorkflowLandRemote
        + TaskWorkflowRemoteTaskCloser
        + ?Sized
{
}
