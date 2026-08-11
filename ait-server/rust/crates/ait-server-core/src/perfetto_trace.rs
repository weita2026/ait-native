use serde_json::json;
#[cfg(test)]
use serde_json::Value as JsonValue;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Instant;

const PERFETTO_TRACE_PATH_ENV: &str = "AIT_PERFETTO_TRACE";
const PERFETTO_TRACE_MAX_EVENTS_ENV: &str = "AIT_PERFETTO_TRACE_MAX_EVENTS";
const DEFAULT_TRACE_MAX_EVENTS: usize = 100_000;

#[derive(Debug)]
struct TraceEvent {
    name: &'static str,
    started_ns: u64,
    duration_ns: u64,
    process_id: u32,
    thread_id: u64,
}

#[derive(Debug)]
struct TraceSession {
    started: Instant,
    active_ranges: usize,
    max_events: usize,
    dropped_events: u64,
    events: VecDeque<TraceEvent>,
}

static TRACE_SESSIONS: OnceLock<Mutex<BTreeMap<PathBuf, TraceSession>>> = OnceLock::new();

pub struct PerfettoRange {
    path: Option<PathBuf>,
    name: &'static str,
    session_started: Option<Instant>,
    started_ns: u64,
    process_id: u32,
    thread_id: u64,
}

impl PerfettoRange {
    pub fn new(name: &'static str) -> Self {
        let path = env::var_os(PERFETTO_TRACE_PATH_ENV)
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty());
        Self::with_path(name, path)
    }

    pub fn new_lane(name: &'static str) -> Self {
        let mut range = Self::new(name);
        // Admission ends while the admitted transaction continues. Put that
        // deliberately overlapping phase on a stable named lane so Perfetto
        // does not have to reinterpret it as malformed thread-stack nesting.
        range.thread_id = named_lane_id(name);
        range
    }

    fn with_path(name: &'static str, path: Option<PathBuf>) -> Self {
        let mut session_started = None;
        let started_ns = path
            .as_ref()
            .and_then(|path| {
                let sessions = TRACE_SESSIONS.get_or_init(|| Mutex::new(BTreeMap::new()));
                let mut sessions = sessions.lock().ok()?;
                let session = sessions
                    .entry(path.clone())
                    .or_insert_with(|| TraceSession {
                        started: Instant::now(),
                        active_ranges: 0,
                        max_events: trace_event_capacity(),
                        dropped_events: 0,
                        events: VecDeque::new(),
                    });
                session.active_ranges = session.active_ranges.saturating_add(1);
                session_started = Some(session.started);
                Some(nanos(session.started.elapsed()))
            })
            .unwrap_or_default();
        Self {
            path,
            name,
            session_started,
            started_ns,
            process_id: process::id(),
            thread_id: current_thread_id(),
        }
    }
}

impl Drop for PerfettoRange {
    fn drop(&mut self) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let duration_ns = self
            .session_started
            .map(|started| nanos(started.elapsed()).saturating_sub(self.started_ns))
            .unwrap_or_default();
        let Some(sessions) = TRACE_SESSIONS.get() else {
            return;
        };
        let Ok(mut sessions) = sessions.lock() else {
            return;
        };
        let Some(session) = sessions.get_mut(path) else {
            return;
        };
        push_event(
            session,
            TraceEvent {
                name: self.name,
                started_ns: self.started_ns,
                duration_ns,
                process_id: self.process_id,
                thread_id: self.thread_id,
            },
        );
        session.active_ranges = session.active_ranges.saturating_sub(1);
        if session.active_ranges == 0 {
            write_trace(path, session);
        }
    }
}

fn write_trace(path: &PathBuf, session: &TraceSession) {
    let mut events = session.events.iter().collect::<Vec<_>>();
    events.sort_by_key(|event| (event.started_ns, std::cmp::Reverse(event.duration_ns)));
    let mut rows = vec![json!({
        "name": "process_name",
        "ph": "M",
        "pid": process::id(),
        "tid": 0,
        "args": {"name": "ait-server"},
    })];
    if session.dropped_events > 0 {
        rows.push(json!({
            "name": "ait.server.perfetto.dropped_events",
            "cat": "ait",
            "ph": "C",
            "ts": 0,
            "pid": process::id(),
            "tid": 0,
            "args": {"count": session.dropped_events},
        }));
    }
    rows.extend(events.into_iter().map(|event| {
        json!({
            "name": event.name,
            "cat": "ait",
            "ph": "X",
            "ts": event.started_ns as f64 / 1_000.0,
            "dur": event.duration_ns as f64 / 1_000.0,
            "pid": event.process_id,
            "tid": event.thread_id,
        })
    }));
    let payload = json!({
        "traceEvents": rows,
        "displayTimeUnit": "ms",
    });
    if let Ok(bytes) = serde_json::to_vec_pretty(&payload) {
        let _ = fs::write(path, bytes);
    }
}

fn push_event(session: &mut TraceSession, event: TraceEvent) {
    if session.events.len() >= session.max_events {
        session.events.pop_front();
        session.dropped_events = session.dropped_events.saturating_add(1);
    }
    session.events.push_back(event);
}

fn trace_event_capacity() -> usize {
    env::var(PERFETTO_TRACE_MAX_EVENTS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TRACE_MAX_EVENTS)
}

fn current_thread_id() -> u64 {
    let mut hasher = DefaultHasher::new();
    thread::current().id().hash(&mut hasher);
    hasher.finish()
}

fn named_lane_id(name: &'static str) -> u64 {
    let mut hasher = DefaultHasher::new();
    "ait-perfetto-lane".hash(&mut hasher);
    name.hash(&mut hasher);
    // Chrome JSON importers consistently preserve 32-bit thread identifiers;
    // larger hashed u64 values may be rounded and collapse distinct lanes.
    u64::from((hasher.finish() as u32) | 0x8000_0000)
}

fn nanos(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    fn temporary_trace_path(label: &str) -> PathBuf {
        static NEXT_PATH: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "ait-server-perfetto-{label}-{}-{sequence}.json",
            process::id()
        ))
    }

    #[test]
    fn nested_ranges_write_perfetto_complete_events() {
        let path = temporary_trace_path("nested-ranges");
        let outer = PerfettoRange::with_path("ait.server.request", Some(path.clone()));
        {
            let _inner = PerfettoRange::with_path("ait.server.read", Some(path.clone()));
            thread::sleep(Duration::from_millis(1));
            assert!(!path.exists());
        }
        drop(outer);

        let payload: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let events = payload["traceEvents"].as_array().unwrap();
        assert!(events.iter().any(|event| {
            event["name"] == JsonValue::String("ait.server.request".to_string())
                && event["ph"] == JsonValue::String("X".to_string())
        }));
        assert!(events.iter().any(|event| {
            event["name"] == JsonValue::String("ait.server.read".to_string())
                && event["dur"].as_f64().unwrap_or_default() > 0.0
        }));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn named_lane_uses_an_independent_perfetto_track() {
        let path = temporary_trace_path("named-lane");
        let outer = PerfettoRange::with_path("ait.server.request", Some(path.clone()));
        let mut lane = PerfettoRange::with_path("ait.server.writer_admission", Some(path.clone()));
        lane.thread_id = named_lane_id("ait.server.writer_admission");
        drop(outer);
        drop(lane);

        let payload: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let events = payload["traceEvents"].as_array().unwrap();
        let request_tid = events
            .iter()
            .find(|event| event["name"] == "ait.server.request")
            .and_then(|event| event["tid"].as_u64())
            .expect("request track");
        let lane_tid = events
            .iter()
            .find(|event| event["name"] == "ait.server.writer_admission")
            .and_then(|event| event["tid"].as_u64())
            .expect("named lane track");
        assert_ne!(request_tid, lane_tid);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn trace_session_evicts_oldest_events_at_the_configured_bound() {
        let started = Instant::now();
        let mut session = TraceSession {
            started,
            active_ranges: 0,
            max_events: 2,
            dropped_events: 0,
            events: VecDeque::new(),
        };
        for name in ["first", "second", "third"] {
            push_event(
                &mut session,
                TraceEvent {
                    name,
                    started_ns: 0,
                    duration_ns: 1,
                    process_id: 1,
                    thread_id: 1,
                },
            );
        }
        assert_eq!(session.events.len(), 2);
        assert_eq!(session.events.front().unwrap().name, "second");
        assert_eq!(session.dropped_events, 1);
    }
}
