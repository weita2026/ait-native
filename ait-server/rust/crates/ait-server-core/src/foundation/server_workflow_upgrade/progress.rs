/// In-process observations of completed upgrade work, independent of HTTP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowUpgradePhase {
    Inspect,
    ReadMetadata,
    Convert,
    Stage,
    Validate,
    AwaitPublication,
    Publish,
    Verify,
    Complete,
}

/// Counts apply to the current phase, not to the whole startup lifetime.
/// An unknown total is never presented as zero completed work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowUpgradeProgress {
    pub phase: WorkflowUpgradePhase,
    pub processed: u64,
    pub total: Option<u64>,
}

pub(super) fn report_progress(
    observer: &mut dyn FnMut(WorkflowUpgradeProgress),
    phase: WorkflowUpgradePhase,
    processed: usize,
    total: Option<usize>,
) {
    observer(WorkflowUpgradeProgress {
        phase,
        processed: processed as u64,
        total: total.map(|total| total as u64),
    });
}
