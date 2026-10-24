//! Explicit test-only, thread-local IO/decode counters; never persisted authority.
use std::cell::RefCell;
use std::collections::BTreeMap;
thread_local! { static COUNTS: RefCell<BTreeMap<String, (u64, u64)>> = const { RefCell::new(BTreeMap::new()) }; }
pub(crate) fn note(label: impl Into<String>, bytes: u64) {
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        let entry = counts.entry(label.into()).or_default();
        entry.0 += 1;
        entry.1 += bytes;
    });
}
pub(crate) fn reset() {
    COUNTS.with(|counts| counts.borrow_mut().clear());
}
pub(crate) fn snapshot() -> BTreeMap<String, (u64, u64)> {
    COUNTS.with(|counts| counts.borrow().clone())
}

pub(crate) fn note_max(label: &str, bytes: u64) {
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        let entry = counts.entry(label.into()).or_default();
        entry.0 += 1;
        entry.1 = entry.1.max(bytes);
    });
}
