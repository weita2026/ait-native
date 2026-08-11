use super::*;

impl FakeRemote {
    pub(super) fn stop(&mut self) -> Result<(), String> {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| "fake remote worker thread panicked".to_string())?;
        }
        Ok(())
    }
}

impl Drop for FakeRemote {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
