use super::*;
use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex};

const BINARY_DB_WRITE_ADMISSION_LANES: [&str; 6] = [
    "global.write.lock",
    "server-content.write.lock",
    "server-plan.write.lock",
    "server-queue.write.lock",
    "server-repository-pack.write.lock",
    "server-workflow.write.lock",
];

#[derive(Debug, Default)]
struct BinaryDbWriteAdmissionState {
    next_ticket: u64,
    serving_ticket: u64,
    cancelled_tickets: BTreeSet<u64>,
}

#[derive(Debug, Default)]
struct BinaryDbWriteAdmissionLane {
    state: Mutex<BinaryDbWriteAdmissionState>,
    wake: Condvar,
}

impl BinaryDbWriteAdmissionLane {
    fn acquire(&self, max_wait: Option<Duration>) -> StoreResult<u64> {
        let started = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.next_ticket == u64::MAX {
            if state.next_ticket == state.serving_ticket && state.cancelled_tickets.is_empty() {
                state.next_ticket = 0;
                state.serving_ticket = 0;
            } else {
                return Err(BinaryDbError::invalid_domain_data(
                    "Binary DB in-process write-admission ticket space is exhausted",
                ));
            }
        }
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        loop {
            if state.serving_ticket == ticket {
                return Ok(ticket);
            }
            let Some(max_wait) = max_wait else {
                state = self
                    .wake
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
                continue;
            };
            let remaining = max_wait.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                Self::cancel_locked(&mut state, ticket);
                self.wake.notify_all();
                return Err(BinaryDbError::retryable_busy(format!(
                    "timed out waiting for ordered in-process Binary DB write admission; waited_ms={} max_wait_ms={}",
                    started.elapsed().as_millis(),
                    max_wait.as_millis(),
                )));
            }
            let (next_state, timeout) = self
                .wake
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner());
            state = next_state;
            if timeout.timed_out() && state.serving_ticket != ticket {
                Self::cancel_locked(&mut state, ticket);
                self.wake.notify_all();
                return Err(BinaryDbError::retryable_busy(format!(
                    "timed out waiting for ordered in-process Binary DB write admission; waited_ms={} max_wait_ms={}",
                    started.elapsed().as_millis(),
                    max_wait.as_millis(),
                )));
            }
        }
    }

    fn release(&self, ticket: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.serving_ticket != ticket {
            return;
        }
        state.serving_ticket = state.serving_ticket.saturating_add(1);
        Self::skip_cancelled_locked(&mut state);
        self.wake.notify_all();
    }

    fn cancel_locked(state: &mut BinaryDbWriteAdmissionState, ticket: u64) {
        state.cancelled_tickets.insert(ticket);
        Self::skip_cancelled_locked(state);
    }

    fn skip_cancelled_locked(state: &mut BinaryDbWriteAdmissionState) {
        while state.cancelled_tickets.remove(&state.serving_ticket) {
            state.serving_ticket = state.serving_ticket.saturating_add(1);
        }
    }

    #[cfg(test)]
    fn queued_count(&self) -> usize {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let issued_not_served = state
            .next_ticket
            .saturating_sub(state.serving_ticket)
            .saturating_sub(1);
        usize::try_from(issued_not_served)
            .unwrap_or(usize::MAX)
            .saturating_sub(state.cancelled_tickets.len())
    }
}

/// Fixed-size, repository-owned admission lanes for serving Binary DB writers.
///
/// The filesystem process locks remain the cross-process exclusion authority.
/// These lanes only order writers inside one server process so a newer request
/// cannot repeatedly steal a just-released family lock from an older request.
#[derive(Debug)]
pub struct BinaryDbWriterAdmission {
    lanes: [BinaryDbWriteAdmissionLane; BINARY_DB_WRITE_ADMISSION_LANES.len()],
}

impl Default for BinaryDbWriterAdmission {
    fn default() -> Self {
        Self {
            lanes: std::array::from_fn(|_| BinaryDbWriteAdmissionLane::default()),
        }
    }
}

impl BinaryDbWriterAdmission {
    pub fn acquire(
        self: &Arc<Self>,
        command_scope: BinaryDbCommandScope,
        max_wait: Option<Duration>,
    ) -> StoreResult<BinaryDbWriterAdmissionGuard> {
        let started = Instant::now();
        let mut guard = BinaryDbWriterAdmissionGuard {
            admission: Arc::clone(self),
            tickets: Vec::new(),
            released: false,
        };
        for lock_name in command_scope.lock_file_names() {
            let lane_index = BINARY_DB_WRITE_ADMISSION_LANES
                .iter()
                .position(|candidate| candidate == lock_name)
                .ok_or_else(|| {
                    BinaryDbError::invalid_domain_data(format!(
                        "Binary DB command scope {command_scope:?} has unknown write-admission lane {lock_name}"
                    ))
                })?;
            let remaining = max_wait.map(|limit| limit.saturating_sub(started.elapsed()));
            let ticket = self.lanes[lane_index].acquire(remaining).map_err(|error| {
                if error.is_retryable_busy() {
                    BinaryDbError::retryable_busy(format!(
                        "Binary DB {command_scope:?} ordered admission failed at {lock_name}: {error}"
                    ))
                } else {
                    error
                }
            })?;
            guard.tickets.push((lane_index, ticket));
        }
        Ok(guard)
    }

    #[cfg(test)]
    pub(crate) fn queued_count_for_scope(&self, command_scope: BinaryDbCommandScope) -> usize {
        command_scope
            .lock_file_names()
            .iter()
            .filter_map(|lock_name| {
                BINARY_DB_WRITE_ADMISSION_LANES
                    .iter()
                    .position(|candidate| candidate == lock_name)
            })
            .map(|lane_index| self.lanes[lane_index].queued_count())
            .max()
            .unwrap_or(0)
    }
}

#[derive(Debug)]
pub struct BinaryDbWriterAdmissionGuard {
    admission: Arc<BinaryDbWriterAdmission>,
    tickets: Vec<(usize, u64)>,
    released: bool,
}

impl BinaryDbWriterAdmissionGuard {
    pub fn release(&mut self) {
        if self.released {
            return;
        }
        for (lane_index, ticket) in self.tickets.drain(..).rev() {
            self.admission.lanes[lane_index].release(ticket);
        }
        self.released = true;
    }
}

impl Drop for BinaryDbWriterAdmissionGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    fn wait_for_queued(
        admission: &BinaryDbWriterAdmission,
        scope: BinaryDbCommandScope,
        expected: usize,
    ) {
        let started = Instant::now();
        while admission.queued_count_for_scope(scope) != expected {
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "timed out waiting for {expected} queued {scope:?} writers"
            );
            thread::yield_now();
        }
    }

    #[test]
    fn same_family_waiters_acquire_in_ticket_order() {
        let admission = Arc::new(BinaryDbWriterAdmission::default());
        let first = admission
            .acquire(BinaryDbCommandScope::ServerWorkflow, None)
            .expect("first workflow admission");

        let (acquired_tx, acquired_rx) = mpsc::channel();
        let (release_one_tx, release_one_rx) = mpsc::channel();
        let waiter_one_admission = Arc::clone(&admission);
        let waiter_one_acquired = acquired_tx.clone();
        let waiter_one = thread::spawn(move || {
            let guard = waiter_one_admission
                .acquire(BinaryDbCommandScope::ServerWorkflow, None)
                .expect("first queued workflow admission");
            waiter_one_acquired.send(1).expect("report first waiter");
            release_one_rx.recv().expect("release first waiter");
            drop(guard);
        });
        wait_for_queued(&admission, BinaryDbCommandScope::ServerWorkflow, 1);

        let (release_two_tx, release_two_rx) = mpsc::channel();
        let waiter_two_admission = Arc::clone(&admission);
        let waiter_two_acquired = acquired_tx;
        let waiter_two = thread::spawn(move || {
            let guard = waiter_two_admission
                .acquire(BinaryDbCommandScope::ServerWorkflow, None)
                .expect("second queued workflow admission");
            waiter_two_acquired.send(2).expect("report second waiter");
            release_two_rx.recv().expect("release second waiter");
            drop(guard);
        });
        wait_for_queued(&admission, BinaryDbCommandScope::ServerWorkflow, 2);

        drop(first);
        assert_eq!(
            acquired_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("first queued writer should acquire"),
            1
        );
        release_one_tx
            .send(())
            .expect("release first queued writer");
        assert_eq!(
            acquired_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("second queued writer should acquire"),
            2
        );
        release_two_tx
            .send(())
            .expect("release second queued writer");
        waiter_one.join().expect("first waiter thread");
        waiter_two.join().expect("second waiter thread");
    }

    #[test]
    fn timed_out_waiter_is_cancelled_without_late_acquisition() {
        let admission = Arc::new(BinaryDbWriterAdmission::default());
        let active = admission
            .acquire(BinaryDbCommandScope::ServerWorkflow, None)
            .expect("active workflow admission");
        let error = admission
            .acquire(
                BinaryDbCommandScope::ServerWorkflow,
                Some(Duration::from_millis(10)),
            )
            .expect_err("queued workflow admission should time out");
        assert!(error.is_retryable_busy());
        assert_eq!(
            admission.queued_count_for_scope(BinaryDbCommandScope::ServerWorkflow),
            0
        );
        drop(active);
        admission
            .acquire(BinaryDbCommandScope::ServerWorkflow, Some(Duration::ZERO))
            .expect("the cancelled ticket must not block the next writer");
    }

    #[test]
    fn independent_family_admission_does_not_serialize() {
        let admission = Arc::new(BinaryDbWriterAdmission::default());
        let workflow = admission
            .acquire(BinaryDbCommandScope::ServerWorkflow, None)
            .expect("workflow admission");
        let plan = admission
            .acquire(BinaryDbCommandScope::ServerPlan, Some(Duration::ZERO))
            .expect("Plan has an independent admission lane");
        let land_error = admission
            .acquire(BinaryDbCommandScope::ServerLand, Some(Duration::ZERO))
            .expect_err("Land must share Workflow admission");
        assert!(land_error.is_retryable_busy());
        drop(plan);
        drop(workflow);
    }
}
