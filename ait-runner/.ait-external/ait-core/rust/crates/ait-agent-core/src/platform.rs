//! Platform-owned socket primitives used by the native agent runtime.
//!
//! Public agent contracts carry a numeric socket identity, but operating-system
//! APIs do not agree on its representation. Unix uses a signed file descriptor;
//! Windows uses an unsigned WinSock `SOCKET`. Keeping the distinction here
//! prevents POSIX assumptions from leaking into transports and worker hosts.

#[cfg(not(any(unix, windows)))]
compile_error!("ait-agent supports Unix and Windows targets only; no native socket backend exists for this target");

use std::io::{self, Read, Write};
use std::mem::ManuallyDrop;
use std::net::{Shutdown, TcpListener, TcpStream};

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, FromRawSocket, IntoRawSocket, RawSocket};

#[cfg(unix)]
pub type NativeSocket = RawFd;
#[cfg(windows)]
pub type NativeSocket = RawSocket;

#[cfg(unix)]
pub const INVALID_NATIVE_SOCKET: NativeSocket = -1;
#[cfg(windows)]
pub const INVALID_NATIVE_SOCKET: NativeSocket = u64::MAX;

pub fn native_socket_is_valid(socket: NativeSocket) -> bool {
    socket != INVALID_NATIVE_SOCKET
}

pub fn native_socket_to_u64(socket: NativeSocket) -> u64 {
    #[cfg(unix)]
    {
        u64::try_from(socket).expect("valid Unix socket must be non-negative")
    }
    #[cfg(windows)]
    {
        socket as u64
    }
}

pub fn native_socket_from_u64(raw: u64) -> Result<NativeSocket, String> {
    #[cfg(unix)]
    {
        i32::try_from(raw)
            .map(|socket| socket as NativeSocket)
            .map_err(|_| "native socket is outside Unix RawFd range".to_string())
    }
    #[cfg(windows)]
    {
        usize::try_from(raw)
            .map(|socket| socket as NativeSocket)
            .map_err(|_| "native socket is outside Windows RawSocket range".to_string())
            .and_then(|socket| {
                native_socket_is_valid(socket)
                    .then_some(socket)
                    .ok_or_else(|| "native socket is the Windows INVALID_SOCKET value".to_string())
            })
    }
}

pub fn native_socket_from_i64(raw: i64) -> Result<NativeSocket, String> {
    let raw = u64::try_from(raw).map_err(|_| "native socket must be non-negative".to_string())?;
    native_socket_from_u64(raw)
}

#[cfg(windows)]
pub(crate) fn native_socket_to_winsock(socket: NativeSocket) -> io::Result<usize> {
    if !native_socket_is_valid(socket) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native socket is the Windows INVALID_SOCKET value",
        ));
    }
    usize::try_from(socket).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "native socket is outside the Windows SOCKET range",
        )
    })
}

pub fn tcp_stream_native_socket(stream: &TcpStream) -> NativeSocket {
    #[cfg(unix)]
    {
        stream.as_raw_fd()
    }
    #[cfg(windows)]
    {
        stream.as_raw_socket()
    }
}

pub fn tcp_listener_native_socket(listener: &TcpListener) -> NativeSocket {
    #[cfg(unix)]
    {
        listener.as_raw_fd()
    }
    #[cfg(windows)]
    {
        listener.as_raw_socket()
    }
}

pub fn tcp_stream_into_native_socket(stream: TcpStream) -> NativeSocket {
    #[cfg(unix)]
    {
        stream.into_raw_fd()
    }
    #[cfg(windows)]
    {
        stream.into_raw_socket()
    }
}

/// Takes ownership of a native socket as a `TcpStream`.
///
/// # Safety
///
/// `socket` must be a live, uniquely owned stream socket. The returned stream
/// becomes responsible for closing it.
pub unsafe fn tcp_stream_from_native_socket(socket: NativeSocket) -> TcpStream {
    #[cfg(unix)]
    {
        unsafe { TcpStream::from_raw_fd(socket) }
    }
    #[cfg(windows)]
    {
        unsafe { TcpStream::from_raw_socket(socket) }
    }
}

fn borrowed_tcp_stream(socket: NativeSocket) -> ManuallyDrop<TcpStream> {
    // SAFETY: The wrapper is placed in `ManuallyDrop`, so the caller retains
    // ownership of the socket after the borrowed operation completes.
    ManuallyDrop::new(unsafe { tcp_stream_from_native_socket(socket) })
}

pub fn set_native_socket_nonblocking(socket: NativeSocket, enabled: bool) -> io::Result<()> {
    borrowed_tcp_stream(socket).set_nonblocking(enabled)
}

pub fn native_socket_take_error(socket: NativeSocket) -> io::Result<Option<io::Error>> {
    borrowed_tcp_stream(socket).take_error()
}

pub fn set_native_socket_close_on_exec(socket: NativeSocket) -> io::Result<()> {
    #[cfg(unix)]
    {
        let flags = unsafe { libc::fcntl(socket, libc::F_GETFD) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let result = unsafe { libc::fcntl(socket, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};

        let handle = native_socket_to_winsock(socket)? as HANDLE;
        let result = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

pub fn read_native_socket(socket: NativeSocket, buffer: &mut [u8]) -> io::Result<usize> {
    let stream = borrowed_tcp_stream(socket);
    let mut stream_ref = &*stream;
    stream_ref.read(buffer)
}

pub fn write_native_socket(socket: NativeSocket, buffer: &[u8]) -> io::Result<usize> {
    let stream = borrowed_tcp_stream(socket);
    let mut stream_ref = &*stream;
    stream_ref.write(buffer)
}

pub fn shutdown_native_socket(socket: NativeSocket) -> io::Result<()> {
    borrowed_tcp_stream(socket).shutdown(Shutdown::Both)
}

pub fn close_native_socket(socket: NativeSocket) -> io::Result<()> {
    if !native_socket_is_valid(socket) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "native socket is invalid",
        ));
    }
    #[cfg(unix)]
    {
        let result = unsafe { libc::close(socket) };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Networking::WinSock::{closesocket, WSAGetLastError};

        let result = unsafe { closesocket(native_socket_to_winsock(socket)?) };
        if result != 0 {
            Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
pub(crate) fn connected_tcp_pair() -> (TcpStream, TcpStream) {
    use std::thread;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback TCP fixture");
    let address = listener.local_addr().expect("loopback fixture address");
    let accept = thread::spawn(move || listener.accept().expect("accept TCP fixture").0);
    let client = TcpStream::connect(address).expect("connect loopback TCP fixture");
    (client, accept.join().expect("join TCP fixture accept"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_socket_round_trips_and_supports_nonblocking_io() {
        use std::time::{Duration, Instant};

        let (client, mut peer) = connected_tcp_pair();
        let socket = tcp_stream_native_socket(&client);
        assert!(native_socket_is_valid(socket));
        assert_eq!(
            native_socket_from_u64(native_socket_to_u64(socket)).unwrap(),
            socket
        );

        set_native_socket_nonblocking(socket, true).unwrap();
        peer.write_all(b"ok").unwrap();
        let mut buffer = [0_u8; 2];
        let deadline = Instant::now() + Duration::from_secs(1);
        let read = loop {
            match read_native_socket(socket, &mut buffer) {
                Ok(read) => break read,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::yield_now();
                }
                Err(error) => panic!("native socket read failed: {error}"),
            }
        };
        assert_eq!(read, 2);
        assert_eq!(&buffer, b"ok");
    }

    #[test]
    fn transferred_native_socket_closes_portably() {
        let (client, mut peer) = connected_tcp_pair();
        let socket = tcp_stream_into_native_socket(client);
        shutdown_native_socket(socket).unwrap();
        close_native_socket(socket).unwrap();

        let mut buffer = [0_u8; 1];
        assert_eq!(peer.read(&mut buffer).unwrap(), 0);
    }
}
