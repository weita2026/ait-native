#[cfg(unix)]
#[test]
fn closed_stdout_pipe_terminates_without_a_rust_panic() {
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};

    let repository = tempfile::tempdir().expect("temporary AIT repository");
    let binary = env!("CARGO_BIN_EXE_ait-cli");
    let init = Command::new(binary)
        .args(["init", "--json"])
        .current_dir(repository.path())
        .output()
        .expect("initialize temporary AIT repository");
    assert!(
        init.status.success(),
        "ait init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let mut pipe_fds = [-1; 2];
    let pipe_result = unsafe { libc::pipe(pipe_fds.as_mut_ptr()) };
    assert_eq!(pipe_result, 0, "create closed-reader stdout pipe");
    let close_result = unsafe { libc::close(pipe_fds[0]) };
    assert_eq!(close_result, 0, "close stdout pipe reader");
    let pipe_writer = unsafe { OwnedFd::from_raw_fd(pipe_fds[1]) };

    let output = Command::new(binary)
        .args(["task", "list", "--all"])
        .current_dir(repository.path())
        .stdout(Stdio::from(pipe_writer))
        .stderr(Stdio::piped())
        .output()
        .expect("run ait with a closed stdout pipe");

    assert_eq!(output.status.signal(), Some(libc::SIGPIPE));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked at"),
        "unexpected panic: {stderr}"
    );
    assert!(
        !stderr.contains("Broken pipe"),
        "unexpected broken-pipe diagnostic: {stderr}"
    );
}
