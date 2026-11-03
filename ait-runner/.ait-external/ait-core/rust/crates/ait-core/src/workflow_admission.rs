//! Pre-finish admission waiting over normalized progress observations. No wire
//! fields, transport retries, mutation submission, or Server restart policy.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Existing /healthz fields remain extensible. Progress fields are required
/// but nullable: unavailable progress must never become zero completed work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeHealth {
    pub ready: bool,
    pub phase: Option<String>,
    pub processed: Option<u64>,
    pub total: Option<u64>,
    pub last_progress_at: Option<String>,
}

impl UpgradeHealth {
    pub fn from_value(value: &serde_json::Value) -> Result<Self, String> {
        let ready = value["ready"]
            .as_bool()
            .ok_or("Health status requires ready:boolean")?;
        let nullable_text = |field: &str| match value.get(field) {
            Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
            _ => Err(format!("Health status requires nullable {field}:string")),
        };
        let nullable_count = |field: &str| match value.get(field) {
            Some(serde_json::Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| format!("Health status requires nullable {field}:u64")),
            None => Err(format!("Health status is missing {field}")),
        };
        let last_progress_at = nullable_text("last_progress_at")?;
        if let Some(timestamp) = &last_progress_at {
            let parsed = chrono::DateTime::parse_from_rfc3339(timestamp)
                .map_err(|error| format!("Invalid last_progress_at: {error}"))?;
            if parsed.offset().local_minus_utc() != 0 {
                return Err("last_progress_at must be UTC RFC3339".into());
            }
        }
        Ok(Self {
            ready,
            phase: nullable_text("phase")?,
            processed: nullable_count("processed")?,
            total: nullable_count("total")?,
            last_progress_at,
        })
    }

    pub fn progress(&self) -> Option<UpgradeProgress> {
        Some(UpgradeProgress {
            phase: self.phase.clone()?,
            processed: self.processed?,
            total: self.total,
            progress_sequence: self
                .last_progress_at
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .and_then(|value| u64::try_from(value.timestamp_micros()).ok()),
        })
    }
}

/// Facts supplied by a status adapter. A progress sequence must describe real
/// work, never a heartbeat or a poll/request count. Total is display-only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeProgress {
    pub phase: String,
    pub processed: u64,
    pub total: Option<u64>,
    pub progress_sequence: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinishAdmissionObservation {
    Admitted,
    Upgrading(UpgradeProgress),
    ProgressUnavailable,
    TransportDisconnected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeWaitState {
    pub progress: UpgradeProgress,
    pub elapsed: Duration,
    pub idle_for: Duration,
    pub stalled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinishAdmissionWaitState {
    Admitted,
    Upgrading(UpgradeWaitState),
    /// Transport loss establishes neither admission nor an upgrade stall.
    TransportDisconnected {
        elapsed: Duration,
        idle_for: Duration,
    },
    ProgressUnavailable {
        elapsed: Duration,
        idle_for: Duration,
    },
}

/// Measures inactivity, without an absolute wait deadline. Stalled is an
/// observation for the agent; it never restarts the Server or sends finish.
#[derive(Debug)]
pub struct FinishAdmissionTracker {
    started_at: Instant,
    last_observed_at: Instant,
    last_progress_at: Instant,
    stall_after: Duration,
    processed_by_phase: BTreeMap<String, u64>,
    progress_sequence: Option<u64>,
}

impl FinishAdmissionTracker {
    pub fn new(now: Instant, stall_after: Duration) -> Self {
        Self {
            started_at: now,
            last_observed_at: now,
            last_progress_at: now,
            stall_after,
            processed_by_phase: BTreeMap::new(),
            progress_sequence: None,
        }
    }

    pub fn observe(
        &mut self,
        now: Instant,
        observation: FinishAdmissionObservation,
    ) -> FinishAdmissionWaitState {
        let now = now.max(self.last_observed_at);
        self.last_observed_at = now;
        let elapsed = now.saturating_duration_since(self.started_at);
        match observation {
            FinishAdmissionObservation::Admitted => FinishAdmissionWaitState::Admitted,
            FinishAdmissionObservation::TransportDisconnected => {
                FinishAdmissionWaitState::TransportDisconnected {
                    elapsed,
                    idle_for: now.saturating_duration_since(self.last_progress_at),
                }
            }
            FinishAdmissionObservation::ProgressUnavailable => {
                FinishAdmissionWaitState::ProgressUnavailable {
                    elapsed,
                    idle_for: now.saturating_duration_since(self.last_progress_at),
                }
            }
            FinishAdmissionObservation::Upgrading(progress) => {
                let advanced_count_or_phase = match self.processed_by_phase.get_mut(&progress.phase)
                {
                    Some(high_water) if progress.processed > *high_water => {
                        *high_water = progress.processed;
                        true
                    }
                    Some(_) => false,
                    None => {
                        self.processed_by_phase
                            .insert(progress.phase.clone(), progress.processed);
                        true
                    }
                };
                let advanced_sequence = match (self.progress_sequence, progress.progress_sequence) {
                    (Some(before), Some(after)) if after > before => {
                        self.progress_sequence = Some(after);
                        true
                    }
                    (None, Some(sequence)) => {
                        self.progress_sequence = Some(sequence);
                        // A first marker establishes a baseline, not work since
                        // the previous observation of the same phase/count.
                        false
                    }
                    _ => false,
                };
                if advanced_count_or_phase || advanced_sequence {
                    self.last_progress_at = now;
                }
                let idle_for = now.saturating_duration_since(self.last_progress_at);
                FinishAdmissionWaitState::Upgrading(UpgradeWaitState {
                    progress,
                    elapsed,
                    idle_for,
                    stalled: idle_for >= self.stall_after,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        phase: &str,
        processed: u64,
        total: Option<u64>,
        sequence: Option<u64>,
    ) -> FinishAdmissionObservation {
        FinishAdmissionObservation::Upgrading(UpgradeProgress {
            phase: phase.into(),
            processed,
            total,
            progress_sequence: sequence,
        })
    }

    fn upgrading(state: FinishAdmissionWaitState) -> UpgradeWaitState {
        match state {
            FinishAdmissionWaitState::Upgrading(state) => state,
            other => panic!("Expected upgrade state, got {other:?}"),
        }
    }

    #[test]
    fn ongoing_progress_outlives_any_fixed_total_wait_and_eventually_admits() {
        let start = Instant::now();
        let mut tracker = FinishAdmissionTracker::new(start, Duration::from_secs(30));
        for step in 0..=1000 {
            let state = upgrading(tracker.observe(
                start + Duration::from_secs(step * 20),
                sample("convert", step, Some(1000), Some(step)),
            ));
            assert!(!state.stalled);
            assert_eq!(state.idle_for, Duration::ZERO);
        }
        assert_eq!(
            tracker.observe(
                start + Duration::from_secs(20_001),
                FinishAdmissionObservation::Admitted
            ),
            FinishAdmissionWaitState::Admitted
        );
    }

    #[test]
    fn heartbeat_total_changes_and_regressions_do_not_extend_progress() {
        let start = Instant::now();
        let mut tracker = FinishAdmissionTracker::new(start, Duration::from_secs(30));
        tracker.observe(start, sample("convert", 10, Some(20), Some(50)));
        for second in [10, 20, 30, 40] {
            let state = upgrading(tracker.observe(
                start + Duration::from_secs(second),
                sample("convert", 10, Some(20 + second), Some(50)),
            ));
            assert_eq!(state.idle_for, Duration::from_secs(second));
            assert_eq!(state.stalled, second >= 30);
        }
        let regressed = upgrading(tracker.observe(
            start + Duration::from_secs(41),
            sample("convert", 1, None, Some(1)),
        ));
        assert!(regressed.stalled);
        let partial_recovery = upgrading(tracker.observe(
            start + Duration::from_secs(42),
            sample("convert", 2, None, Some(2)),
        ));
        assert!(partial_recovery.stalled);
        let resumed = upgrading(tracker.observe(
            start + Duration::from_secs(43),
            sample("convert", 11, Some(100), Some(51)),
        ));
        assert!(!resumed.stalled);
        assert_eq!(resumed.idle_for, Duration::ZERO);
    }

    #[test]
    fn phase_and_actual_progress_markers_extend_wait_but_phase_oscillation_does_not() {
        let start = Instant::now();
        let mut tracker = FinishAdmissionTracker::new(start, Duration::from_secs(30));
        tracker.observe(start, sample("scan", 100, None, None));
        tracker.observe(
            start + Duration::from_secs(20),
            sample("convert", 0, Some(100), Some(5)),
        );
        let repeated_phase = upgrading(tracker.observe(
            start + Duration::from_secs(40),
            sample("scan", 100, None, Some(5)),
        ));
        assert_eq!(repeated_phase.idle_for, Duration::from_secs(20));
        let actual_progress = upgrading(tracker.observe(
            start + Duration::from_secs(41),
            sample("scan", 100, None, Some(6)),
        ));
        assert_eq!(actual_progress.idle_for, Duration::ZERO);
        let stalled = upgrading(tracker.observe(
            start + Duration::from_secs(71),
            sample("convert", 0, Some(100), Some(6)),
        ));
        assert!(stalled.stalled);
    }

    #[test]
    fn disconnection_is_distinct_and_reconnection_does_not_invent_progress_or_admission() {
        let start = Instant::now();
        let mut tracker = FinishAdmissionTracker::new(start, Duration::from_secs(30));
        tracker.observe(start, sample("convert", 4, Some(10), None));
        assert_eq!(
            tracker.observe(
                start + Duration::from_secs(60),
                FinishAdmissionObservation::TransportDisconnected
            ),
            FinishAdmissionWaitState::TransportDisconnected {
                elapsed: Duration::from_secs(60),
                idle_for: Duration::from_secs(60)
            }
        );
        let unchanged = upgrading(tracker.observe(
            start + Duration::from_secs(61),
            sample("convert", 4, Some(10), None),
        ));
        assert!(unchanged.stalled);
        let recovered = upgrading(tracker.observe(
            start + Duration::from_secs(62),
            sample("convert", 5, Some(10), None),
        ));
        assert!(!recovered.stalled);
        assert_eq!(recovered.elapsed, Duration::from_secs(62));
    }
}
