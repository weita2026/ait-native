fn main() -> std::process::ExitCode {
    restore_default_sigpipe();
    ait_cli::entry()
}

#[cfg(unix)]
fn restore_default_sigpipe() {
    // Rust ignores SIGPIPE and turns a closed stdout pipe into an I/O error.
    // Many CLI renderers use infallible print macros, which would then panic.
    // Restoring the Unix default makes short readers such as `head` terminate
    // the producer through the conventional SIGPIPE process boundary.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_default_sigpipe() {}
