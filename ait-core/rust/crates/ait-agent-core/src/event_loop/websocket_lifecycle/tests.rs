use super::{
    execute_agent_websocket_lifecycle_actions, execute_with_websocket_lifecycle_executor,
    WebSocketLifecycleExecutor,
};
use crate::event_loop::{
    AgentEventLoopBackend, AgentEventLoopBackendPort, AgentEventLoopUnregistrationPort,
};
use ait_core::json_support::json;
use std::io::{self, Read};

use crate::platform::{
    connected_tcp_pair, tcp_stream_into_native_socket, tcp_stream_native_socket, NativeSocket,
};

#[derive(Default)]
struct SubstituteEventLoop {
    calls: Vec<String>,
    unregister_error: Option<io::ErrorKind>,
}

impl AgentEventLoopBackendPort for SubstituteEventLoop {
    fn backend(&self) -> AgentEventLoopBackend {
        AgentEventLoopBackend::PortablePoll
    }
}

impl AgentEventLoopUnregistrationPort for SubstituteEventLoop {
    fn unregister(&mut self, token: u64) -> io::Result<()> {
        self.calls.push(format!("unregister:{token}"));
        if let Some(kind) = self.unregister_error {
            return Err(io::Error::new(kind, "substitute unregister failure"));
        }
        Ok(())
    }
}

#[derive(Default)]
struct SubstituteLifecycleExecutor {
    closed_fds: Vec<NativeSocket>,
    close_error: Option<io::ErrorKind>,
}

impl WebSocketLifecycleExecutor for SubstituteLifecycleExecutor {
    fn close_websocket_fd(&mut self, fd: NativeSocket) -> io::Result<()> {
        self.closed_fds.push(fd);
        if let Some(kind) = self.close_error {
            return Err(io::Error::new(kind, "substitute close failure"));
        }
        Ok(())
    }
}

#[test]
fn websocket_lifecycle_executes_nested_shard_actions_without_python_fallback() {
    let mut event_loop = SubstituteEventLoop::default();
    let mut lifecycle = SubstituteLifecycleExecutor::default();

    let planned = execute_with_websocket_lifecycle_executor(
        &mut event_loop,
        &mut lifecycle,
        &json!({
            "shard_batch_plan": {
                "actions": [
                    {
                        "kind": "websocket_shard_worker_action",
                        "backend": "portable_poll",
                        "shard_index": 2,
                        "worker_key": "slack/team-a",
                        "transport": "slack",
                        "event_loop_token": 7,
                        "websocket_fd": 44,
                        "action": {
                            "kind": "close_websocket",
                            "reason": "websocket_event_loop_hangup"
                        }
                    },
                    {
                        "kind": "websocket_shard_worker_action",
                        "backend": "portable_poll",
                        "shard_index": 2,
                        "worker_key": "slack/team-a",
                        "transport": "slack",
                        "event_loop_token": 7,
                        "websocket_fd": 44,
                        "action": {
                            "kind": "unregister_websocket_readable"
                        }
                    },
                    {
                        "kind": "websocket_shard_worker_action",
                        "backend": "portable_poll",
                        "shard_index": 2,
                        "worker_key": "slack/team-a",
                        "transport": "slack",
                        "event_loop_token": 7,
                        "websocket_fd": 44,
                        "action": {
                            "kind": "reconnect_socket_mode",
                            "reason": "websocket_event_loop_hangup",
                            "delay_seconds": 0.25
                        }
                    }
                ]
            }
        }),
    )
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_websocket_lifecycle_action_execution"
    );
    assert_eq!(
        planned["websocket_lifecycle_contract"],
        "ait_agent_core.event_loop.WebSocketLifecycleActions.v1"
    );
    assert_eq!(planned["websocket_lifecycle_state"], "operations_applied");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["operation_count"], 3);
    assert_eq!(planned["applied_operation_count"], 3);
    assert_eq!(planned["close_operation_count"], 1);
    assert_eq!(planned["unregister_operation_count"], 1);
    assert_eq!(planned["reconnect_operation_count"], 1);
    assert_eq!(planned["reconnect_requests"][0]["transport"], "slack");
    assert_eq!(
        planned["reconnect_requests"][0]["reason"],
        "websocket_event_loop_hangup"
    );
    assert_eq!(planned["reconnect_requests"][0]["execute_connect"], false);
    assert_eq!(planned["python_websocket_lifecycle_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(event_loop.calls, vec!["unregister:7"]);
    assert_eq!(lifecycle.closed_fds, vec![44]);
}

#[test]
fn websocket_lifecycle_projects_final_connection_state() {
    let mut event_loop = SubstituteEventLoop::default();
    let mut lifecycle = SubstituteLifecycleExecutor::default();

    let planned = execute_with_websocket_lifecycle_executor(
        &mut event_loop,
        &mut lifecycle,
        &json!({
            "reactor_run_result": {
                "final_connections": [{
                    "worker_key": "discord/ops",
                    "transport": "discord",
                    "event_loop_token": 9,
                    "websocket_fd": 99,
                    "should_unregister": true,
                    "should_close_websocket": true,
                    "should_reconnect": true,
                    "last_websocket_turn_state": "hangup_reconnect"
                }]
            }
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_lifecycle_state"], "operations_applied");
    assert_eq!(planned["operation_count"], 3);
    assert_eq!(planned["reconnect_requests"][0]["transport"], "discord");
    assert_eq!(
        planned["reconnect_requests"][0]["reason"],
        "hangup_reconnect"
    );
    assert_eq!(event_loop.calls, vec!["unregister:9"]);
    assert_eq!(lifecycle.closed_fds, vec![99]);
}

#[test]
fn websocket_lifecycle_inherits_source_registration_context() {
    let mut event_loop = SubstituteEventLoop::default();
    let mut lifecycle = SubstituteLifecycleExecutor::default();

    let planned = execute_with_websocket_lifecycle_executor(
        &mut event_loop,
        &mut lifecycle,
        &json!({
            "worker_key": "discord/alerts",
            "event_loop_registration": {
                "token": 21,
                "fd": 66,
                "transport": "discord",
                "shard_index": 3
            },
            "actions": [
                {
                    "kind": "close_websocket"
                },
                {
                    "kind": "unregister_websocket_readable"
                },
                {
                    "kind": "reconnect_gateway",
                    "reason": "source_registration_context"
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_lifecycle_state"], "operations_applied");
    assert_eq!(planned["operation_count"], 3);
    assert_eq!(planned["operations"][0]["websocket_fd"], 66);
    assert_eq!(planned["operations"][1]["event_loop_token"], 21);
    assert_eq!(planned["reconnect_requests"][0]["transport"], "discord");
    assert_eq!(
        planned["reconnect_requests"][0]["worker_key"],
        "discord/alerts"
    );
    assert_eq!(event_loop.calls, vec!["unregister:21"]);
    assert_eq!(lifecycle.closed_fds, vec![66]);
}

#[test]
fn websocket_lifecycle_validates_all_actions_before_mutation() {
    let mut event_loop = SubstituteEventLoop::default();
    let mut lifecycle = SubstituteLifecycleExecutor::default();

    let planned = execute_with_websocket_lifecycle_executor(
        &mut event_loop,
        &mut lifecycle,
        &json!({
            "actions": [
                {
                    "kind": "unregister_websocket_readable",
                    "event_loop_token": 3
                },
                {
                    "kind": "close_websocket"
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_lifecycle_state"], "configuration_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["executed"], false);
    assert!(planned["diagnostics"][0]
        .as_str()
        .unwrap()
        .contains("websocket_fd"));
    assert!(event_loop.calls.is_empty());
    assert!(lifecycle.closed_fds.is_empty());
}

#[test]
fn websocket_lifecycle_reports_close_execution_errors_fail_closed() {
    let mut event_loop = SubstituteEventLoop::default();
    let mut lifecycle = SubstituteLifecycleExecutor {
        close_error: Some(io::ErrorKind::PermissionDenied),
        ..SubstituteLifecycleExecutor::default()
    };

    let planned = execute_with_websocket_lifecycle_executor(
        &mut event_loop,
        &mut lifecycle,
        &json!({
            "actions": [{
                "kind": "close_websocket",
                "websocket_fd": 55,
                "reason": "test"
            }]
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_lifecycle_state"], "execution_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["operation_count"], 1);
    assert_eq!(planned["applied_operation_count"], 0);
    assert_eq!(planned["operation_results"][0]["status"], "failed");
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(lifecycle.closed_fds, vec![55]);
}

#[test]
fn websocket_lifecycle_default_executor_closes_transferred_fd() {
    let (client, mut peer) = connected_tcp_pair();
    let client_fd = tcp_stream_into_native_socket(client);
    let mut event_loop = SubstituteEventLoop::default();

    let planned = execute_agent_websocket_lifecycle_actions(
        &mut event_loop,
        &json!({
            "actions": [{
                "kind": "close_websocket",
                "websocket_fd": client_fd,
                "reason": "test_close"
            }]
        }),
    )
    .unwrap();

    let mut buffer = [0_u8; 1];
    let read = peer.read(&mut buffer).unwrap();

    assert_eq!(planned["websocket_lifecycle_state"], "operations_applied");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["close_operation_count"], 1);
    assert_eq!(read, 0);
}

#[test]
fn websocket_lifecycle_uses_registration_payload_aliases() {
    let (_reader, writer) = connected_tcp_pair();
    let writer_fd = tcp_stream_native_socket(&writer);
    let mut event_loop = SubstituteEventLoop::default();
    let mut lifecycle = SubstituteLifecycleExecutor::default();

    let planned = execute_with_websocket_lifecycle_executor(
        &mut event_loop,
        &mut lifecycle,
        &json!({
            "actions": [{
                "kind": "unregister_websocket",
                "registration": {
                    "token": 15,
                    "fd": writer_fd,
                    "transport": "slack",
                    "shard_index": 1
                }
            }]
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_lifecycle_state"], "operations_applied");
    assert_eq!(planned["operation_count"], 1);
    assert_eq!(planned["operations"][0]["event_loop_token"], 15);
    assert_eq!(planned["operations"][0]["websocket_fd"], writer_fd);
    assert_eq!(planned["operations"][0]["transport"], "slack");
    assert_eq!(event_loop.calls, vec!["unregister:15"]);
    assert!(lifecycle.closed_fds.is_empty());
}
