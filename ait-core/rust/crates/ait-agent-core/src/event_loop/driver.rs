use std::collections::HashMap;
use std::io;
use std::time::Duration;

use crate::platform::NativeSocket;

use super::AgentEventLoopBackend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEvent {
    pub token: u64,
    pub readable: bool,
    pub writable: bool,
    pub hangup: bool,
}

pub trait AgentEventLoopBackendPort {
    fn backend(&self) -> AgentEventLoopBackend;
}

pub trait AgentEventLoopReadableRegistrationPort {
    fn register_readable(&mut self, token: u64, socket: NativeSocket) -> io::Result<()>;
}

pub trait AgentEventLoopReadWriteRegistrationPort {
    fn register_read_write(&mut self, token: u64, socket: NativeSocket) -> io::Result<()>;
}

pub trait AgentEventLoopUnregistrationPort {
    fn unregister(&mut self, token: u64) -> io::Result<()>;
}

pub trait AgentEventLoopPollPort {
    fn poll(&mut self, timeout: Duration) -> io::Result<Vec<AgentEvent>>;
}

pub trait AgentEventLoopRegistrationPort:
    AgentEventLoopReadableRegistrationPort
    + AgentEventLoopReadWriteRegistrationPort
    + AgentEventLoopUnregistrationPort
{
}

impl<E> AgentEventLoopRegistrationPort for E where
    E: AgentEventLoopReadableRegistrationPort
        + AgentEventLoopReadWriteRegistrationPort
        + AgentEventLoopUnregistrationPort
        + ?Sized
{
}

pub trait AgentEventLoop:
    AgentEventLoopBackendPort + AgentEventLoopRegistrationPort + AgentEventLoopPollPort
{
}

impl<E> AgentEventLoop for E where
    E: AgentEventLoopBackendPort + AgentEventLoopRegistrationPort + AgentEventLoopPollPort + ?Sized
{
}

pub fn agent_event_loop_backend<E>(event_loop: &E) -> AgentEventLoopBackend
where
    E: AgentEventLoopBackendPort + ?Sized,
{
    event_loop.backend()
}

pub fn register_agent_event_loop_readable<E>(
    event_loop: &mut E,
    token: u64,
    socket: NativeSocket,
) -> io::Result<()>
where
    E: AgentEventLoopReadableRegistrationPort + ?Sized,
{
    event_loop.register_readable(token, socket)
}

pub fn register_agent_event_loop_read_write<E>(
    event_loop: &mut E,
    token: u64,
    socket: NativeSocket,
) -> io::Result<()>
where
    E: AgentEventLoopReadWriteRegistrationPort + ?Sized,
{
    event_loop.register_read_write(token, socket)
}

pub fn unregister_agent_event_loop<E>(event_loop: &mut E, token: u64) -> io::Result<()>
where
    E: AgentEventLoopUnregistrationPort + ?Sized,
{
    event_loop.unregister(token)
}

pub fn poll_agent_event_loop<E>(
    event_loop: &mut E,
    timeout: Duration,
) -> io::Result<Vec<AgentEvent>>
where
    E: AgentEventLoopPollPort + ?Sized,
{
    event_loop.poll(timeout)
}

pub struct AgentEventLoopDriver {
    inner: AgentEventLoopDriverInner,
}

enum AgentEventLoopDriverInner {
    #[cfg(target_os = "linux")]
    LinuxEpoll(linux::LinuxEpollEventLoop),
    PortablePoll(PortablePollEventLoop),
}

impl AgentEventLoopDriver {
    pub fn new_default() -> io::Result<Self> {
        match AgentEventLoopBackend::current_platform_default() {
            AgentEventLoopBackend::LinuxEpoll => {
                Self::new_for_backend(AgentEventLoopBackend::LinuxEpoll)
            }
            AgentEventLoopBackend::PortablePoll => {
                Self::new_for_backend(AgentEventLoopBackend::PortablePoll)
            }
        }
    }

    pub fn new_for_backend(backend: AgentEventLoopBackend) -> io::Result<Self> {
        match backend {
            AgentEventLoopBackend::LinuxEpoll => {
                #[cfg(target_os = "linux")]
                {
                    linux::LinuxEpollEventLoop::new().map(|inner| Self {
                        inner: AgentEventLoopDriverInner::LinuxEpoll(inner),
                    })
                }
                #[cfg(not(target_os = "linux"))]
                {
                    Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "linux epoll backend is only available on Linux",
                    ))
                }
            }
            AgentEventLoopBackend::PortablePoll => Ok(Self {
                inner: AgentEventLoopDriverInner::PortablePoll(PortablePollEventLoop::new()),
            }),
        }
    }
}

impl AgentEventLoopBackendPort for AgentEventLoopDriver {
    fn backend(&self) -> AgentEventLoopBackend {
        match &self.inner {
            #[cfg(target_os = "linux")]
            AgentEventLoopDriverInner::LinuxEpoll(inner) => inner.backend(),
            AgentEventLoopDriverInner::PortablePoll(inner) => inner.backend(),
        }
    }
}

impl AgentEventLoopReadableRegistrationPort for AgentEventLoopDriver {
    fn register_readable(&mut self, token: u64, socket: NativeSocket) -> io::Result<()> {
        match &mut self.inner {
            #[cfg(target_os = "linux")]
            AgentEventLoopDriverInner::LinuxEpoll(inner) => inner.register_readable(token, socket),
            AgentEventLoopDriverInner::PortablePoll(inner) => {
                inner.register_readable(token, socket)
            }
        }
    }
}

impl AgentEventLoopReadWriteRegistrationPort for AgentEventLoopDriver {
    fn register_read_write(&mut self, token: u64, socket: NativeSocket) -> io::Result<()> {
        match &mut self.inner {
            #[cfg(target_os = "linux")]
            AgentEventLoopDriverInner::LinuxEpoll(inner) => {
                inner.register_read_write(token, socket)
            }
            AgentEventLoopDriverInner::PortablePoll(inner) => {
                inner.register_read_write(token, socket)
            }
        }
    }
}

impl AgentEventLoopUnregistrationPort for AgentEventLoopDriver {
    fn unregister(&mut self, token: u64) -> io::Result<()> {
        match &mut self.inner {
            #[cfg(target_os = "linux")]
            AgentEventLoopDriverInner::LinuxEpoll(inner) => inner.unregister(token),
            AgentEventLoopDriverInner::PortablePoll(inner) => inner.unregister(token),
        }
    }
}

impl AgentEventLoopPollPort for AgentEventLoopDriver {
    fn poll(&mut self, timeout: Duration) -> io::Result<Vec<AgentEvent>> {
        match &mut self.inner {
            #[cfg(target_os = "linux")]
            AgentEventLoopDriverInner::LinuxEpoll(inner) => inner.poll(timeout),
            AgentEventLoopDriverInner::PortablePoll(inner) => inner.poll(timeout),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AgentFdInterest {
    readable: bool,
    writable: bool,
}

impl AgentFdInterest {
    fn readable() -> Self {
        Self {
            readable: true,
            writable: false,
        }
    }

    fn read_write() -> Self {
        Self {
            readable: true,
            writable: true,
        }
    }

    #[cfg(unix)]
    fn poll_events(self) -> i16 {
        let mut events = 0;
        if self.readable {
            events |= libc::POLLIN;
        }
        if self.writable {
            events |= libc::POLLOUT;
        }
        events
    }

    #[cfg(windows)]
    fn poll_events(self) -> i16 {
        use windows_sys::Win32::Networking::WinSock::{POLLIN, POLLOUT};

        let mut events = 0;
        if self.readable {
            events |= POLLIN;
        }
        if self.writable {
            events |= POLLOUT;
        }
        events
    }

    #[cfg(target_os = "linux")]
    fn epoll_events(self) -> u32 {
        let mut events = (libc::EPOLLET | libc::EPOLLRDHUP) as u32;
        if self.readable {
            events |= libc::EPOLLIN as u32;
        }
        if self.writable {
            events |= libc::EPOLLOUT as u32;
        }
        events
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AgentFdRegistration {
    socket: NativeSocket,
    interest: AgentFdInterest,
}

struct PortablePollEventLoop {
    registrations: HashMap<u64, AgentFdRegistration>,
}

impl PortablePollEventLoop {
    fn new() -> Self {
        Self {
            registrations: HashMap::new(),
        }
    }
}

impl AgentEventLoopBackendPort for PortablePollEventLoop {
    fn backend(&self) -> AgentEventLoopBackend {
        AgentEventLoopBackend::PortablePoll
    }
}

impl AgentEventLoopReadableRegistrationPort for PortablePollEventLoop {
    fn register_readable(&mut self, token: u64, socket: NativeSocket) -> io::Result<()> {
        self.register_interest(token, socket, AgentFdInterest::readable())
    }
}

impl AgentEventLoopReadWriteRegistrationPort for PortablePollEventLoop {
    fn register_read_write(&mut self, token: u64, socket: NativeSocket) -> io::Result<()> {
        self.register_interest(token, socket, AgentFdInterest::read_write())
    }
}

impl AgentEventLoopUnregistrationPort for PortablePollEventLoop {
    fn unregister(&mut self, token: u64) -> io::Result<()> {
        self.registrations.remove(&token);
        Ok(())
    }
}

impl AgentEventLoopPollPort for PortablePollEventLoop {
    fn poll(&mut self, timeout: Duration) -> io::Result<Vec<AgentEvent>> {
        let registrations: Vec<(u64, AgentFdRegistration)> = self
            .registrations
            .iter()
            .map(|(token, registration)| (*token, *registration))
            .collect();
        portable::poll(&registrations, timeout)
    }
}

impl PortablePollEventLoop {
    fn register_interest(
        &mut self,
        token: u64,
        socket: NativeSocket,
        interest: AgentFdInterest,
    ) -> io::Result<()> {
        self.registrations
            .insert(token, AgentFdRegistration { socket, interest });
        Ok(())
    }
}

fn duration_to_poll_timeout_ms(timeout: Duration) -> i32 {
    i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX)
}

#[cfg(unix)]
mod portable {
    use super::*;

    pub(super) fn poll(
        registrations: &[(u64, AgentFdRegistration)],
        timeout: Duration,
    ) -> io::Result<Vec<AgentEvent>> {
        let mut pollfds: Vec<libc::pollfd> = registrations
            .iter()
            .map(|(_, registration)| libc::pollfd {
                fd: registration.socket,
                events: registration.interest.poll_events(),
                revents: 0,
            })
            .collect();
        let timeout_ms = duration_to_poll_timeout_ms(timeout);
        loop {
            let result = unsafe {
                libc::poll(
                    pollfds.as_mut_ptr(),
                    pollfds.len() as libc::nfds_t,
                    timeout_ms,
                )
            };
            if result < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            break;
        }
        Ok(pollfds
            .iter()
            .zip(registrations)
            .filter_map(|(pollfd, (token, _registration))| {
                event_from_revents(*token, pollfd.revents)
            })
            .collect())
    }

    fn event_from_revents(token: u64, revents: i16) -> Option<AgentEvent> {
        if revents == 0 {
            return None;
        }
        Some(AgentEvent {
            token,
            readable: revents & libc::POLLIN != 0,
            writable: revents & libc::POLLOUT != 0,
            hangup: revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0,
        })
    }
}

#[cfg(windows)]
mod portable {
    use super::*;
    use crate::platform::native_socket_to_winsock;
    use windows_sys::Win32::Networking::WinSock::{
        WSAGetLastError, WSAPoll, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT, WSAEINTR, WSAPOLLFD,
    };

    pub(super) fn poll(
        registrations: &[(u64, AgentFdRegistration)],
        timeout: Duration,
    ) -> io::Result<Vec<AgentEvent>> {
        if registrations.is_empty() {
            std::thread::sleep(timeout);
            return Ok(Vec::new());
        }
        let mut pollfds: Vec<WSAPOLLFD> = registrations
            .iter()
            .map(|(_, registration)| {
                Ok(WSAPOLLFD {
                    fd: native_socket_to_winsock(registration.socket)?,
                    events: registration.interest.poll_events(),
                    revents: 0,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let timeout_ms = duration_to_poll_timeout_ms(timeout);
        loop {
            let result = unsafe {
                WSAPoll(
                    pollfds.as_mut_ptr(),
                    u32::try_from(pollfds.len()).unwrap_or(u32::MAX),
                    timeout_ms,
                )
            };
            if result < 0 {
                let code = unsafe { WSAGetLastError() };
                if code == WSAEINTR {
                    continue;
                }
                return Err(io::Error::from_raw_os_error(code));
            }
            break;
        }
        Ok(pollfds
            .iter()
            .zip(registrations)
            .filter_map(|(pollfd, (token, _registration))| {
                event_from_revents(*token, pollfd.revents)
            })
            .collect())
    }

    fn event_from_revents(token: u64, revents: i16) -> Option<AgentEvent> {
        if revents == 0 {
            return None;
        }
        Some(AgentEvent {
            token,
            readable: revents & POLLIN != 0,
            writable: revents & POLLOUT != 0,
            hangup: revents & (POLLHUP | POLLERR | POLLNVAL) != 0,
        })
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    pub struct LinuxEpollEventLoop {
        epoll_fd: NativeSocket,
        registrations: HashMap<u64, AgentFdRegistration>,
    }

    impl LinuxEpollEventLoop {
        pub fn new() -> io::Result<Self> {
            let epoll_fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
            if epoll_fd < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                epoll_fd,
                registrations: HashMap::new(),
            })
        }
    }

    impl Drop for LinuxEpollEventLoop {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.epoll_fd);
            }
        }
    }

    impl AgentEventLoopBackendPort for LinuxEpollEventLoop {
        fn backend(&self) -> AgentEventLoopBackend {
            AgentEventLoopBackend::LinuxEpoll
        }
    }

    impl AgentEventLoopReadableRegistrationPort for LinuxEpollEventLoop {
        fn register_readable(&mut self, token: u64, socket: NativeSocket) -> io::Result<()> {
            self.register_interest(token, socket, AgentFdInterest::readable())
        }
    }

    impl AgentEventLoopReadWriteRegistrationPort for LinuxEpollEventLoop {
        fn register_read_write(&mut self, token: u64, socket: NativeSocket) -> io::Result<()> {
            self.register_interest(token, socket, AgentFdInterest::read_write())
        }
    }

    impl AgentEventLoopUnregistrationPort for LinuxEpollEventLoop {
        fn unregister(&mut self, token: u64) -> io::Result<()> {
            if let Some(registration) = self.registrations.remove(&token) {
                let result = unsafe {
                    libc::epoll_ctl(
                        self.epoll_fd,
                        libc::EPOLL_CTL_DEL,
                        registration.socket,
                        std::ptr::null_mut(),
                    )
                };
                if result < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }

    impl AgentEventLoopPollPort for LinuxEpollEventLoop {
        fn poll(&mut self, timeout: Duration) -> io::Result<Vec<AgentEvent>> {
            let capacity = self.registrations.len().max(1);
            let mut events = vec![libc::epoll_event { events: 0, u64: 0 }; capacity];
            let timeout_ms = duration_to_poll_timeout_ms(timeout);
            let ready = loop {
                let result = unsafe {
                    libc::epoll_wait(
                        self.epoll_fd,
                        events.as_mut_ptr(),
                        events.len() as i32,
                        timeout_ms,
                    )
                };
                if result < 0 {
                    let err = io::Error::last_os_error();
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(err);
                }
                break result as usize;
            };
            Ok(events
                .into_iter()
                .take(ready)
                .map(|event| AgentEvent {
                    token: event.u64,
                    readable: event.events & libc::EPOLLIN as u32 != 0,
                    writable: event.events & libc::EPOLLOUT as u32 != 0,
                    hangup: event.events
                        & (libc::EPOLLHUP | libc::EPOLLERR | libc::EPOLLRDHUP) as u32
                        != 0,
                })
                .collect())
        }
    }

    impl LinuxEpollEventLoop {
        fn register_interest(
            &mut self,
            token: u64,
            socket: NativeSocket,
            interest: AgentFdInterest,
        ) -> io::Result<()> {
            let mut event = libc::epoll_event {
                events: interest.epoll_events(),
                u64: token,
            };
            let previous = self.registrations.get(&token).copied();
            if let Some(previous) = previous {
                if previous.socket != socket {
                    let result = unsafe {
                        libc::epoll_ctl(
                            self.epoll_fd,
                            libc::EPOLL_CTL_DEL,
                            previous.socket,
                            std::ptr::null_mut(),
                        )
                    };
                    if result < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
            }
            let op = if previous.is_some_and(|previous| previous.socket == socket) {
                libc::EPOLL_CTL_MOD
            } else {
                libc::EPOLL_CTL_ADD
            };
            let result = unsafe { libc::epoll_ctl(self.epoll_fd, op, socket, &mut event) };
            if result < 0 {
                return Err(io::Error::last_os_error());
            }
            self.registrations
                .insert(token, AgentFdRegistration { socket, interest });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
