use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::RunnerError;

#[derive(Clone, Debug, Default)]
pub(crate) struct ExecutionControl(Arc<AtomicBool>);

impl ExecutionControl {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn check(&self) -> Result<(), RunnerError> {
        if self.is_cancelled() {
            Err(RunnerError::Cancelled)
        } else {
            Ok(())
        }
    }
}
