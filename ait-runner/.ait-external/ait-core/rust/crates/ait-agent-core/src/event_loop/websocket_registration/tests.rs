use super::{
    agent_websocket_registration_action_plan_json, execute_agent_websocket_registration_actions,
};
use crate::event_loop::{
    poll_agent_event_loop, AgentEvent, AgentEventLoop, AgentEventLoopBackend,
    AgentEventLoopBackendPort, AgentEventLoopDriver, AgentEventLoopPollPort,
    AgentEventLoopReadWriteRegistrationPort, AgentEventLoopReadableRegistrationPort,
    AgentEventLoopUnregistrationPort,
};
use ait_core::json_support::json;
use std::io;

use crate::platform::{connected_tcp_pair, tcp_stream_native_socket, NativeSocket};
use std::time::Duration;

#[derive(Default)]
struct SubstituteEventLoop {
    calls: Vec<String>,
}

impl AgentEventLoopBackendPort for SubstituteEventLoop {
    fn backend(&self) -> AgentEventLoopBackend {
        AgentEventLoopBackend::PortablePoll
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
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct RegistrationOnlyEventLoop {
    calls: Vec<String>,
}

impl AgentEventLoopBackendPort for RegistrationOnlyEventLoop {
    fn backend(&self) -> AgentEventLoopBackend {
        AgentEventLoopBackend::PortablePoll
    }
}

impl AgentEventLoopReadableRegistrationPort for RegistrationOnlyEventLoop {
    fn register_readable(&mut self, token: u64, fd: NativeSocket) -> io::Result<()> {
        self.calls.push(format!("register_readable:{token}:{fd}"));
        Ok(())
    }
}

impl AgentEventLoopReadWriteRegistrationPort for RegistrationOnlyEventLoop {
    fn register_read_write(&mut self, token: u64, fd: NativeSocket) -> io::Result<()> {
        self.calls.push(format!("register_read_write:{token}:{fd}"));
        Ok(())
    }
}

impl AgentEventLoopUnregistrationPort for RegistrationOnlyEventLoop {
    fn unregister(&mut self, token: u64) -> io::Result<()> {
        self.calls.push(format!("unregister:{token}"));
        Ok(())
    }
}

#[test]
fn websocket_registration_executes_wrapped_shard_actions_in_order() {
    let mut event_loop = SubstituteEventLoop::default();
    let planned = execute_agent_websocket_registration_actions(
        &mut event_loop,
        &json!({
            "backend": "linux_epoll",
            "actions": [
                {
                    "kind": "websocket_shard_worker_action",
                    "worker_key": "slack/team-a",
                    "shard_index": 1,
                    "action": {
                        "kind": "keep_websocket_read_write_registered",
                        "registration": {
                            "token": 7,
                            "fd": 42,
                            "interest": "read_write",
                            "shard_index": 1
                        }
                    }
                },
                {
                    "kind": "dispatch_slack_socket_mode_payload"
                },
                {
                    "kind": "unregister_websocket_readable",
                    "event_loop_token": 8
                },
                {
                    "kind": "keep_websocket_readable_registered",
                    "registration": {
                        "token": 9,
                        "fd": 44
                    }
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_websocket_registration_action_execution"
    );
    assert_eq!(
        planned["websocket_registration_contract"],
        "ait_agent_core.event_loop.WebSocketRegistrationActions.v1"
    );
    assert_eq!(
        planned["websocket_registration_state"],
        "operations_applied"
    );
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["operation_count"], 3);
    assert_eq!(planned["skipped_action_count"], 1);
    assert_eq!(planned["python_websocket_registration_allowed"], false);
    assert_eq!(
        planned["operation_results"][0]["worker_key"],
        "slack/team-a"
    );
    assert_eq!(planned["operation_results"][0]["shard_index"], 1);
    assert_eq!(
        planned["operation_results"][0]["operation"],
        "register_read_write"
    );
    assert_eq!(planned["operation_results"][1]["operation"], "unregister");
    assert_eq!(
        planned["operation_results"][2]["operation"],
        "register_readable"
    );
    assert_eq!(
        event_loop.calls,
        vec![
            "register_read_write:7:42",
            "unregister:8",
            "register_readable:9:44"
        ]
    );
}

#[test]
fn websocket_registration_validates_all_operations_before_mutating_driver() {
    let mut event_loop = SubstituteEventLoop::default();
    let planned = execute_agent_websocket_registration_actions(
        &mut event_loop,
        &json!({
            "actions": [
                {
                    "kind": "keep_websocket_read_write_registered",
                    "registration": {
                        "token": 7,
                        "fd": 42
                    }
                },
                {
                    "kind": "keep_websocket_readable_registered",
                    "registration": {
                        "fd": 44
                    }
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(
        planned["websocket_registration_state"],
        "configuration_error"
    );
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["executed"], false);
    assert!(planned["diagnostics"][0]
        .as_str()
        .unwrap()
        .contains("missing event_loop_token"));
    assert!(event_loop.calls.is_empty());
}

#[test]
fn websocket_registration_plan_parses_connect_registration_without_execution() {
    let planned = agent_websocket_registration_action_plan_json(&json!({
        "actions": [{
            "kind": "register_websocket_readable",
            "registration": {
                "token": 11,
                "fd": 55
            }
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_registration_state"],
        "operations_planned"
    );
    assert_eq!(planned["executed"], false);
    assert_eq!(planned["operation_count"], 1);
    assert_eq!(planned["operations"][0]["operation"], "register_readable");
    assert_eq!(planned["operations"][0]["event_loop_token"], 11);
}

#[test]
fn websocket_registration_executes_against_portable_poll_driver() {
    let (_reader, writer) = connected_tcp_pair();
    let writer_socket = tcp_stream_native_socket(&writer);
    let mut driver =
        AgentEventLoopDriver::new_for_backend(AgentEventLoopBackend::PortablePoll).unwrap();

    let planned = execute_agent_websocket_registration_actions(
        &mut driver,
        &json!({
            "actions": [{
                "kind": "keep_websocket_read_write_registered",
                "registration": {
                    "token": 12,
                    "fd": writer_socket
                }
            }]
        }),
    )
    .unwrap();

    assert_eq!(planned["ok"], true);
    assert_eq!(planned["operation_count"], 1);

    let events = poll_agent_event_loop(&mut driver, Duration::from_millis(250)).unwrap();
    assert!(events
        .iter()
        .any(|event| event.token == 12 && event.writable));
}

#[test]
fn websocket_registration_execution_accepts_registration_only_port() {
    let mut event_loop = RegistrationOnlyEventLoop::default();

    let planned = execute_agent_websocket_registration_actions(
        &mut event_loop,
        &json!({
            "actions": [
                {
                    "kind": "keep_websocket_readable_registered",
                    "registration": {
                        "token": 13,
                        "fd": 43
                    }
                },
                {
                    "kind": "unregister_websocket",
                    "event_loop_token": 13
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(planned["ok"], true);
    assert_eq!(planned["operation_count"], 2);
    assert_eq!(
        event_loop.calls,
        vec!["register_readable:13:43", "unregister:13"]
    );
}

#[test]
fn websocket_registration_bound_helper_accepts_trait_object() {
    let mut event_loop = SubstituteEventLoop::default();
    let event_loop_port: &mut dyn AgentEventLoop = &mut event_loop;

    let planned = execute_agent_websocket_registration_actions(
        event_loop_port,
        &json!({
            "actions": [{
                "kind": "unregister_websocket",
                "event_loop_token": 19
            }]
        }),
    )
    .unwrap();

    assert_eq!(planned["ok"], true);
    assert_eq!(planned["operations"][0]["operation"], "unregister");
}
