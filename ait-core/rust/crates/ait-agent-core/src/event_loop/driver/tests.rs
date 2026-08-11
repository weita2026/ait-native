use super::*;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::thread;

use crate::platform::tcp_stream_native_socket;

fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let accept = thread::spawn(move || listener.accept().unwrap().0);
    let client = TcpStream::connect(address).unwrap();
    (client, accept.join().unwrap())
}

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
    fn poll(&mut self, timeout: Duration) -> io::Result<Vec<AgentEvent>> {
        self.calls.push(format!("poll:{}", timeout.as_millis()));
        Ok(vec![AgentEvent {
            token: 9,
            readable: true,
            writable: false,
            hangup: false,
        }])
    }
}

struct BackendOnlyEventLoop;

impl AgentEventLoopBackendPort for BackendOnlyEventLoop {
    fn backend(&self) -> AgentEventLoopBackend {
        AgentEventLoopBackend::PortablePoll
    }
}

#[derive(Default)]
struct ReadableRegistrationOnlyEventLoop {
    calls: Vec<String>,
}

impl AgentEventLoopReadableRegistrationPort for ReadableRegistrationOnlyEventLoop {
    fn register_readable(&mut self, token: u64, fd: NativeSocket) -> io::Result<()> {
        self.calls.push(format!("register_readable:{token}:{fd}"));
        Ok(())
    }
}

#[derive(Default)]
struct ReadWriteRegistrationOnlyEventLoop {
    calls: Vec<String>,
}

impl AgentEventLoopReadWriteRegistrationPort for ReadWriteRegistrationOnlyEventLoop {
    fn register_read_write(&mut self, token: u64, fd: NativeSocket) -> io::Result<()> {
        self.calls.push(format!("register_read_write:{token}:{fd}"));
        Ok(())
    }
}

#[derive(Default)]
struct UnregistrationOnlyEventLoop {
    calls: Vec<String>,
}

impl AgentEventLoopUnregistrationPort for UnregistrationOnlyEventLoop {
    fn unregister(&mut self, token: u64) -> io::Result<()> {
        self.calls.push(format!("unregister:{token}"));
        Ok(())
    }
}

#[derive(Default)]
struct PollOnlyEventLoop {
    calls: Vec<String>,
}

impl AgentEventLoopPollPort for PollOnlyEventLoop {
    fn poll(&mut self, timeout: Duration) -> io::Result<Vec<AgentEvent>> {
        self.calls.push(format!("poll:{}", timeout.as_millis()));
        Ok(vec![AgentEvent {
            token: 11,
            readable: false,
            writable: true,
            hangup: false,
        }])
    }
}

#[test]
fn agent_event_loop_helpers_accept_single_capability_ports() {
    let backend = BackendOnlyEventLoop;
    assert_eq!(
        agent_event_loop_backend(&backend),
        AgentEventLoopBackend::PortablePoll
    );

    let mut readable = ReadableRegistrationOnlyEventLoop::default();
    register_agent_event_loop_readable(&mut readable, 1, 41).unwrap();
    assert_eq!(readable.calls, vec!["register_readable:1:41"]);

    let mut read_write = ReadWriteRegistrationOnlyEventLoop::default();
    register_agent_event_loop_read_write(&mut read_write, 2, 42).unwrap();
    assert_eq!(read_write.calls, vec!["register_read_write:2:42"]);

    let mut unregister = UnregistrationOnlyEventLoop::default();
    unregister_agent_event_loop(&mut unregister, 3).unwrap();
    assert_eq!(unregister.calls, vec!["unregister:3"]);

    let mut poll = PollOnlyEventLoop::default();
    let events = poll_agent_event_loop(&mut poll, Duration::from_millis(15)).unwrap();
    assert_eq!(events[0].token, 11);
    assert!(events[0].writable);
    assert_eq!(poll.calls, vec!["poll:15"]);
}

#[test]
fn agent_event_loop_bound_helpers_accept_trait_object() {
    let mut event_loop = SubstituteEventLoop::default();
    let event_loop_port: &mut dyn AgentEventLoop = &mut event_loop;

    assert_eq!(
        agent_event_loop_backend(&*event_loop_port),
        AgentEventLoopBackend::PortablePoll
    );
    register_agent_event_loop_readable(event_loop_port, 9, 44).unwrap();
    register_agent_event_loop_read_write(event_loop_port, 9, 44).unwrap();
    unregister_agent_event_loop(event_loop_port, 9).unwrap();
    let events = poll_agent_event_loop(event_loop_port, Duration::from_millis(25)).unwrap();

    assert_eq!(events[0].token, 9);
    assert!(events[0].readable);
    assert_eq!(
        event_loop.calls,
        vec![
            "register_readable:9:44",
            "register_read_write:9:44",
            "unregister:9",
            "poll:25"
        ]
    );
}

#[test]
fn portable_poll_reports_readable_registration() {
    let (reader, mut writer) = connected_pair();
    let mut driver =
        AgentEventLoopDriver::new_for_backend(AgentEventLoopBackend::PortablePoll).unwrap();
    driver
        .register_readable(7, tcp_stream_native_socket(&reader))
        .unwrap();

    writer.write_all(b"x").unwrap();
    let events = driver.poll(Duration::from_millis(250)).unwrap();

    assert!(events
        .iter()
        .any(|event| event.token == 7 && event.readable));
}

#[test]
fn portable_poll_reports_writable_registration() {
    let (_reader, writer) = connected_pair();
    let mut driver =
        AgentEventLoopDriver::new_for_backend(AgentEventLoopBackend::PortablePoll).unwrap();
    driver
        .register_read_write(8, tcp_stream_native_socket(&writer))
        .unwrap();

    let events = driver.poll(Duration::from_millis(250)).unwrap();

    assert!(events
        .iter()
        .any(|event| event.token == 8 && event.writable));
}

#[test]
fn portable_poll_updates_registration_interest_for_existing_token() {
    let (_reader, writer) = connected_pair();
    let mut driver =
        AgentEventLoopDriver::new_for_backend(AgentEventLoopBackend::PortablePoll).unwrap();
    let socket = tcp_stream_native_socket(&writer);
    driver.register_readable(9, socket).unwrap();
    driver.register_read_write(9, socket).unwrap();

    let events = driver.poll(Duration::from_millis(250)).unwrap();

    assert!(events
        .iter()
        .any(|event| event.token == 9 && event.writable));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_epoll_reports_writable_registration() {
    let (_reader, writer) = connected_pair();
    let mut driver =
        AgentEventLoopDriver::new_for_backend(AgentEventLoopBackend::LinuxEpoll).unwrap();
    driver
        .register_read_write(10, tcp_stream_native_socket(&writer))
        .unwrap();

    let events = driver.poll(Duration::from_millis(250)).unwrap();

    assert!(events
        .iter()
        .any(|event| event.token == 10 && event.writable));
}
