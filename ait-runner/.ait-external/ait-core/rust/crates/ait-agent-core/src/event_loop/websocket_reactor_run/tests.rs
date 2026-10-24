use super::execute_agent_websocket_reactor_run;
use crate::event_loop::{
    AgentEvent, AgentEventLoopBackend, AgentEventLoopBackendPort, AgentEventLoopPollPort,
    AgentEventLoopReadWriteRegistrationPort, AgentEventLoopReadableRegistrationPort,
    AgentEventLoopUnregistrationPort,
};
use ait_core::json_support::json;
use std::collections::VecDeque;
use std::io::{self, Read};

use crate::platform::{connected_tcp_pair, tcp_stream_native_socket, NativeSocket};
use std::time::Duration;

struct SubstituteEventLoop {
    backend: AgentEventLoopBackend,
    events_by_poll: VecDeque<Vec<AgentEvent>>,
    calls: Vec<String>,
    poll_count: usize,
}

impl Default for SubstituteEventLoop {
    fn default() -> Self {
        Self {
            backend: AgentEventLoopBackend::PortablePoll,
            events_by_poll: VecDeque::new(),
            calls: Vec::new(),
            poll_count: 0,
        }
    }
}

impl AgentEventLoopBackendPort for SubstituteEventLoop {
    fn backend(&self) -> AgentEventLoopBackend {
        self.backend
    }
}

impl AgentEventLoopReadableRegistrationPort for SubstituteEventLoop {
    fn register_readable(&mut self, token: u64, fd: NativeSocket) -> io::Result<()> {
        self.calls.push(format!("register_readable:{token}:{fd}"));
        Ok(())
    }
}

impl AgentEventLoopReadWriteRegistrationPort for SubstituteEventLoop {
    fn register_read_write(&mut self, token: u64, fd: NativeSocket) -> io::Result<()> {
        self.calls.push(format!("register_read_write:{token}:{fd}"));
        Ok(())
    }
}

impl AgentEventLoopUnregistrationPort for SubstituteEventLoop {
    fn unregister(&mut self, token: u64) -> io::Result<()> {
        self.calls.push(format!("unregister:{token}"));
        Ok(())
    }
}

impl AgentEventLoopPollPort for SubstituteEventLoop {
    fn poll(&mut self, _timeout: Duration) -> io::Result<Vec<AgentEvent>> {
        self.poll_count += 1;
        Ok(self.events_by_poll.pop_front().unwrap_or_default())
    }
}

#[test]
fn websocket_reactor_run_carries_pending_write_state_between_ticks() {
    let (mut reader, writer) = connected_tcp_pair();
    let writer_fd = tcp_stream_native_socket(&writer);
    let mut event_loop = SubstituteEventLoop {
        events_by_poll: VecDeque::from([
            vec![AgentEvent {
                token: 31,
                readable: false,
                writable: true,
                hangup: false,
            }],
            vec![AgentEvent {
                token: 31,
                readable: false,
                writable: true,
                hangup: false,
            }],
        ]),
        ..SubstituteEventLoop::default()
    };

    let planned = execute_agent_websocket_reactor_run(
        &mut event_loop,
        &json!({
            "backend": "portable_poll",
            "max_ticks": 2,
            "max_idle_ticks": 1,
            "connections": [{
                "worker_key": "discord/ops",
                "transport": "discord",
                "event_loop_token": 31,
                "websocket_fd": writer_fd,
                "pending_write_hex": "01020304",
                "max_write_bytes": 2
            }]
        }),
    )
    .unwrap();

    let mut received = [0u8; 4];
    reader.read_exact(&mut received).unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_websocket_reactor_run_execution"
    );
    assert_eq!(
        planned["websocket_reactor_run_contract"],
        "ait_agent_core.event_loop.WebSocketReactorRun.v1"
    );
    assert_eq!(planned["websocket_reactor_run_state"], "max_ticks_reached");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["tick_count"], 2);
    assert_eq!(planned["eventful_tick_count"], 2);
    assert_eq!(planned["poll_event_count"], 2);
    assert_eq!(planned["known_event_count"], 2);
    assert_eq!(planned["registration_operation_count"], 2);
    assert_eq!(planned["merged_connection_update_count"], 2);
    assert_eq!(planned["final_connections"][0]["pending_write_hex"], "");
    assert_eq!(planned["final_connections"][0]["remaining_write_hex"], "");
    assert_eq!(
        planned["final_connections"][0]["websocket_registration_interest"],
        "readable"
    );
    assert_eq!(
        planned["tick_history"][0]["merged_connection_update_count"],
        1
    );
    assert_eq!(
        planned["tick_history"][1]["merged_connection_update_count"],
        1
    );
    assert_eq!(planned["python_websocket_reactor_run_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(received, [1, 2, 3, 4]);
    assert_eq!(event_loop.poll_count, 2);
    assert_eq!(
        event_loop.calls,
        vec![
            format!("register_read_write:31:{writer_fd}"),
            format!("register_readable:31:{writer_fd}")
        ]
    );
}

#[test]
fn websocket_reactor_run_stops_after_idle_limit() {
    let mut event_loop = SubstituteEventLoop {
        events_by_poll: VecDeque::from([Vec::new()]),
        ..SubstituteEventLoop::default()
    };

    let planned = execute_agent_websocket_reactor_run(
        &mut event_loop,
        &json!({
            "max_ticks": 8,
            "max_idle_ticks": 1,
            "connections": [{
                "worker_key": "slack/team-a",
                "transport": "slack",
                "event_loop_token": 7,
                "websocket_fd": 42
            }]
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_reactor_run_state"], "idle_limit_reached");
    assert_eq!(planned["stop_reason"], "idle_limit_reached");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["tick_count"], 1);
    assert_eq!(planned["idle_tick_count"], 1);
    assert_eq!(planned["poll_event_count"], 0);
    assert_eq!(planned["merged_connection_update_count"], 0);
    assert_eq!(event_loop.poll_count, 1);
    assert!(event_loop.calls.is_empty());
}

#[test]
fn websocket_reactor_run_rejects_backend_mismatch_before_polling() {
    let mut event_loop = SubstituteEventLoop::default();

    let planned = execute_agent_websocket_reactor_run(
        &mut event_loop,
        &json!({
            "backend": "linux_epoll",
            "max_ticks": 4,
            "connections": [{
                "worker_key": "slack/team-a",
                "transport": "slack",
                "event_loop_token": 9,
                "websocket_fd": 99
            }]
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_reactor_run_state"], "failed_closed");
    assert_eq!(planned["stop_reason"], "backend_mismatch");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["tick_count"], 1);
    assert_eq!(planned["poll_event_count"], 0);
    assert_eq!(
        planned["last_tick"]["websocket_reactor_tick_state"],
        "backend_mismatch"
    );
    assert_eq!(planned["python_websocket_reactor_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(event_loop.poll_count, 0);
    assert!(event_loop.calls.is_empty());
}

#[test]
fn websocket_reactor_run_preserves_high_concurrency_epoll_requirement() {
    let mut event_loop = SubstituteEventLoop {
        events_by_poll: VecDeque::from([Vec::new()]),
        ..SubstituteEventLoop::default()
    };

    let planned = execute_agent_websocket_reactor_run(
        &mut event_loop,
        &json!({
            "backend": "portable_poll",
            "max_ticks": 3,
            "expected_concurrent_workers": 128,
            "high_concurrency": true,
            "connections": [{
                "worker_key": "discord/ops",
                "transport": "discord",
                "event_loop_token": 11,
                "websocket_fd": 111
            }]
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_reactor_run_state"], "failed_closed");
    assert_eq!(planned["stop_reason"], "backend_requires_epoll");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["tick_count"], 1);
    assert_eq!(
        planned["last_tick"]["websocket_reactor_tick_state"],
        "backend_requires_epoll"
    );
    assert_eq!(
        planned["last_tick"]["shard_batch_plan"]["websocket_shard_event_batch_state"],
        "backend_requires_epoll"
    );
    assert_eq!(planned["python_websocket_shard_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(event_loop.poll_count, 1);
}

#[test]
fn websocket_reactor_run_returns_no_connections_without_polling() {
    let mut event_loop = SubstituteEventLoop::default();

    let planned = execute_agent_websocket_reactor_run(
        &mut event_loop,
        &json!({
            "max_ticks": 3,
            "connections": []
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_reactor_run_state"], "no_connections");
    assert_eq!(planned["stop_reason"], "no_connections");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["executed"], false);
    assert_eq!(planned["tick_count"], 0);
    assert_eq!(planned["python_websocket_event_loop_allowed"], false);
    assert_eq!(event_loop.poll_count, 0);
}
