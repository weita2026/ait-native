use super::execute_agent_websocket_reactor_tick;
use crate::event_loop::{
    AgentEvent, AgentEventLoopBackend, AgentEventLoopBackendPort, AgentEventLoopDriver,
    AgentEventLoopPollPort, AgentEventLoopReadWriteRegistrationPort,
    AgentEventLoopReadableRegistrationPort, AgentEventLoopUnregistrationPort,
};
use ait_core::json_support::json;
use std::io::{self, Write};

use crate::platform::{connected_tcp_pair, tcp_stream_native_socket, NativeSocket};
use std::time::Duration;

struct SubstituteEventLoop {
    backend: AgentEventLoopBackend,
    events: Vec<AgentEvent>,
    calls: Vec<String>,
    poll_count: usize,
    poll_error: Option<io::ErrorKind>,
}

impl Default for SubstituteEventLoop {
    fn default() -> Self {
        Self {
            backend: AgentEventLoopBackend::PortablePoll,
            events: Vec::new(),
            calls: Vec::new(),
            poll_count: 0,
            poll_error: None,
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
        if let Some(kind) = self.poll_error {
            return Err(io::Error::new(kind, "substitute poll failure"));
        }
        Ok(self.events.clone())
    }
}

#[test]
fn websocket_reactor_tick_polls_plans_shard_and_applies_registration() {
    let mut event_loop = SubstituteEventLoop {
        events: vec![AgentEvent {
            token: 7,
            readable: true,
            writable: false,
            hangup: false,
        }],
        ..SubstituteEventLoop::default()
    };

    let planned = execute_agent_websocket_reactor_tick(
        &mut event_loop,
        &json!({
            "backend": "portable_poll",
            "timeout_ms": 0,
            "connections": [{
                "worker_key": "slack/team-a",
                "transport": "slack",
                "event_loop_token": 7,
                "websocket_fd": 42,
                "read_bytes": server_text_frame(r#"{"type":"hello","num_connections":1}"#)
            }]
        }),
    )
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_websocket_reactor_tick_execution"
    );
    assert_eq!(
        planned["websocket_reactor_tick_contract"],
        "ait_agent_core.event_loop.WebSocketReactorTick.v1"
    );
    assert_eq!(planned["websocket_reactor_tick_state"], "tick_applied");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["poll_event_count"], 1);
    assert_eq!(
        planned["shard_batch_plan"]["websocket_shard_event_batch_state"],
        "events_planned"
    );
    assert_eq!(
        planned["shard_batch_plan"]["turn_results"][0]["websocket_turn_state"],
        "payloads_dispatched"
    );
    assert_eq!(
        planned["registration_result"]["websocket_registration_state"],
        "operations_applied"
    );
    assert_eq!(planned["registration_operation_count"], 1);
    assert_eq!(planned["python_websocket_reactor_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(event_loop.poll_count, 1);
    assert_eq!(event_loop.calls, vec!["register_readable:7:42"]);
}

#[test]
fn websocket_reactor_tick_reads_fd_through_portable_poll_driver() {
    let (client, mut peer) = connected_tcp_pair();
    let client_socket = tcp_stream_native_socket(&client);
    let mut driver =
        AgentEventLoopDriver::new_for_backend(AgentEventLoopBackend::PortablePoll).unwrap();
    driver.register_readable(21, client_socket).unwrap();
    peer.write_all(&server_text_frame(
        r#"{"type":"hello","num_connections":1}"#,
    ))
    .unwrap();

    let planned = execute_agent_websocket_reactor_tick(
        &mut driver,
        &json!({
            "timeout_ms": 250,
            "connections": [{
                "worker_key": "slack/team-b",
                "transport": "slack",
                "event_loop_token": 21,
                "websocket_fd": client_socket,
                "max_read_bytes": 1024
            }]
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_reactor_tick_state"], "tick_applied");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["poll_event_count"], 1);
    assert_eq!(
        planned["shard_batch_plan"]["turn_results"][0]["turn_plan"]["read_source"],
        "fd_io"
    );
    assert_eq!(
        planned["shard_batch_plan"]["actions"][0]["action"]["kind"],
        "deliver_websocket_fd_read_chunk"
    );
    assert_eq!(
        planned["registration_result"]["websocket_registration_state"],
        "operations_applied"
    );
}

#[test]
fn websocket_reactor_tick_rejects_backend_mismatch_before_polling() {
    let mut event_loop = SubstituteEventLoop::default();
    let planned = execute_agent_websocket_reactor_tick(
        &mut event_loop,
        &json!({
            "backend": "linux_epoll",
            "connections": []
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_reactor_tick_state"], "backend_mismatch");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["executed"], false);
    assert_eq!(planned["poll_event_count"], 0);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(event_loop.poll_count, 0);
    assert!(event_loop.calls.is_empty());
}

#[test]
fn websocket_reactor_tick_preserves_shard_epoll_requirement() {
    let mut event_loop = SubstituteEventLoop::default();
    let planned = execute_agent_websocket_reactor_tick(
        &mut event_loop,
        &json!({
            "backend": "portable_poll",
            "expected_concurrent_workers": 128,
            "high_concurrency": true,
            "connections": []
        }),
    )
    .unwrap();

    assert_eq!(
        planned["websocket_reactor_tick_state"],
        "backend_requires_epoll"
    );
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["shard_batch_plan"]["websocket_shard_event_batch_state"],
        "backend_requires_epoll"
    );
    assert_eq!(
        planned["registration_result"]["websocket_registration_state"],
        "idle"
    );
    assert_eq!(planned["python_websocket_shard_allowed"], false);
    assert_eq!(event_loop.poll_count, 1);
}

#[test]
fn websocket_reactor_tick_poll_error_fails_closed() {
    let mut event_loop = SubstituteEventLoop {
        poll_error: Some(io::ErrorKind::TimedOut),
        ..SubstituteEventLoop::default()
    };

    let planned = execute_agent_websocket_reactor_tick(
        &mut event_loop,
        &json!({
            "timeout_ms": 1,
            "connections": []
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_reactor_tick_state"], "poll_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["executed"], false);
    assert_eq!(planned["poll_timeout_ms"], 1);
    assert_eq!(
        planned["shard_batch_plan"],
        ait_core::json_support::JsonValue::Null
    );
    assert_eq!(
        planned["registration_result"],
        ait_core::json_support::JsonValue::Null
    );
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(event_loop.poll_count, 1);
}

fn server_text_frame(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    assert!(bytes.len() < 126);
    let mut frame = Vec::with_capacity(bytes.len() + 2);
    frame.push(0x81);
    frame.push(bytes.len() as u8);
    frame.extend_from_slice(bytes);
    frame
}
