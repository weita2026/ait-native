use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_CI_PROCESS_NICE: i32 = 10;
const CI_PROCESS_NICE_ENV_NAMES: [&str; 2] = ["AIT_NATIVE_SERVER_CI_NICE", "AIT_SERVER_CI_NICE"];
pub(crate) const DEFAULT_CI_PROCESS_TIMEOUT_SECONDS: u64 = 3_600;
pub(crate) const MAX_CI_PROCESS_TIMEOUT_SECONDS: u64 = 86_400;
const DEFAULT_CI_PROCESS_TERMINATION_GRACE_SECONDS: u64 = 2;
const CI_PROCESS_WAIT_POLL_MILLIS: u64 = 20;
pub(crate) const PROCESS_STDOUT_TAIL_BYTES: u64 = 8_000;
pub(crate) const PROCESS_STDERR_TAIL_BYTES: u64 = 8_000;
pub(crate) const PROCESS_COMBINED_TAIL_CHARS: usize = 12_000;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CiProcessStdoutCapture {
    None,
    Required(u64),
    Optional(u64),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CiProcessExecutionOptions {
    pub(crate) timeout: Duration,
    pub(crate) termination_grace: Duration,
}

impl CiProcessExecutionOptions {
    pub(crate) fn from_timeout_seconds(timeout_seconds: u64) -> Self {
        Self {
            timeout: Duration::from_secs(timeout_seconds),
            termination_grace: Duration::from_secs(DEFAULT_CI_PROCESS_TERMINATION_GRACE_SECONDS),
        }
    }
}

pub(crate) struct StreamedProcessOutput {
    pub(crate) status: ExitStatus,
    pub(crate) timed_out: bool,
    pub(crate) stdout_bytes: usize,
    pub(crate) stderr_bytes: usize,
    pub(crate) stdout_tail: String,
    pub(crate) stderr_tail: String,
    pub(crate) combined_tail: String,
    pub(crate) captured_stdout: Option<String>,
    pub(crate) stdout_capture_truncated: bool,
}

pub(crate) fn validated_ci_process_timeout_seconds(
    configured: Option<i64>,
    field: &str,
) -> Result<u64, String> {
    let Some(value) = configured else {
        return Ok(DEFAULT_CI_PROCESS_TIMEOUT_SECONDS);
    };
    if value < 1 {
        return Err(format!("Field `{field}` must be a positive integer."));
    }
    let value =
        u64::try_from(value).map_err(|_| format!("Field `{field}` must be a positive integer."))?;
    if value > MAX_CI_PROCESS_TIMEOUT_SECONDS {
        return Err(format!(
            "Field `{field}` must not exceed {MAX_CI_PROCESS_TIMEOUT_SECONDS} seconds."
        ));
    }
    Ok(value)
}

pub(crate) fn run_streamed_command(
    command: &mut Command,
    log_path: &Path,
    command_text: &str,
    cwd: &Path,
    stdout_capture: CiProcessStdoutCapture,
    execution: CiProcessExecutionOptions,
) -> Result<StreamedProcessOutput, String> {
    if execution.timeout < Duration::from_secs(1)
        || execution.timeout > Duration::from_secs(MAX_CI_PROCESS_TIMEOUT_SECONDS)
    {
        return Err(format!(
            "CI process timeout must be between 1 second and {MAX_CI_PROCESS_TIMEOUT_SECONDS} seconds."
        ));
    }
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create process log parent `{}`: {exc}",
                path_text(parent)
            )
        })?;
    }
    let stream_files = ProcessStreamFiles::create(log_path)?;
    command
        .stdout(Stdio::from(stream_files.stdout_file()?))
        .stderr(Stdio::from(stream_files.stderr_file()?));
    configure_ci_process_priority(command)?;
    configure_ci_process_group(command);
    let mut child = command
        .spawn()
        .map_err(|exc| format!("Failed to execute streamed CI process: {exc}"))?;
    let wait = wait_for_ci_process(&mut child, execution)?;
    let stdout_bytes = file_len_usize(&stream_files.stdout_path)?;
    let stderr_bytes = file_len_usize(&stream_files.stderr_path)?;
    let stdout_tail = read_file_tail(&stream_files.stdout_path, PROCESS_STDOUT_TAIL_BYTES)?;
    let stderr_tail = read_file_tail(&stream_files.stderr_path, PROCESS_STDERR_TAIL_BYTES)?;
    let combined_tail = tail_chars(
        &format!("stdout:\n{stdout_tail}\n\nstderr:\n{stderr_tail}"),
        PROCESS_COMBINED_TAIL_CHARS,
    );
    write_process_log_from_streams(
        log_path,
        ProcessLogMetadata {
            command: command_text,
            cwd,
            exit_code: wait.status.code().unwrap_or(-1),
            timed_out: wait.timed_out,
            timeout_seconds: execution.timeout.as_secs_f64(),
        },
        &stream_files.stdout_path,
        &stream_files.stderr_path,
    )?;
    let (captured_stdout, stdout_capture_truncated) = match stdout_capture {
        CiProcessStdoutCapture::None => (None, false),
        CiProcessStdoutCapture::Required(limit) => {
            if u64::try_from(stdout_bytes).unwrap_or(u64::MAX) > limit {
                return Err(format!(
                    "CI stdout capture exceeds the bounded {limit}-byte parser limit ({stdout_bytes} bytes); full output remains in the process log `{}`",
                    path_text(log_path)
                ));
            }
            (
                Some(read_bounded_file(&stream_files.stdout_path, limit)?),
                false,
            )
        }
        CiProcessStdoutCapture::Optional(limit) => {
            if u64::try_from(stdout_bytes).unwrap_or(u64::MAX) > limit {
                (None, true)
            } else {
                (
                    Some(read_bounded_file(&stream_files.stdout_path, limit)?),
                    false,
                )
            }
        }
    };
    Ok(StreamedProcessOutput {
        status: wait.status,
        timed_out: wait.timed_out,
        stdout_bytes,
        stderr_bytes,
        stdout_tail,
        stderr_tail,
        combined_tail,
        captured_stdout,
        stdout_capture_truncated,
    })
}

struct CiProcessWait {
    status: ExitStatus,
    timed_out: bool,
}

fn wait_for_ci_process(
    child: &mut Child,
    execution: CiProcessExecutionOptions,
) -> Result<CiProcessWait, String> {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|exc| format!("Failed to wait for streamed CI process: {exc}"))?
        {
            return Ok(CiProcessWait {
                status,
                timed_out: false,
            });
        }
        if started.elapsed() >= execution.timeout {
            let status = terminate_timed_out_process(child, execution.termination_grace)?;
            return Ok(CiProcessWait {
                status,
                timed_out: true,
            });
        }
        let remaining = execution.timeout.saturating_sub(started.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(CI_PROCESS_WAIT_POLL_MILLIS)));
    }
}

#[cfg(unix)]
fn terminate_timed_out_process(
    child: &mut Child,
    termination_grace: Duration,
) -> Result<ExitStatus, String> {
    let process_group = i32::try_from(child.id())
        .map_err(|_| format!("CI child PID {} exceeds the Unix PID range.", child.id()))?;
    let _ = signal_process_group(process_group, libc::SIGTERM);
    let grace_started = Instant::now();
    let mut direct_status = None;
    loop {
        if direct_status.is_none() {
            direct_status = child
                .try_wait()
                .map_err(|exc| format!("Failed to reap timed-out CI process: {exc}"))?;
        }
        if !process_group_exists(process_group) || grace_started.elapsed() >= termination_grace {
            break;
        }
        let remaining = termination_grace.saturating_sub(grace_started.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(CI_PROCESS_WAIT_POLL_MILLIS)));
    }
    if process_group_exists(process_group)
        && signal_process_group(process_group, libc::SIGKILL).is_err()
        && direct_status.is_none()
    {
        let _ = child.kill();
    }
    match direct_status {
        Some(status) => Ok(status),
        None => child
            .wait()
            .map_err(|exc| format!("Failed to reap killed CI process: {exc}")),
    }
}

#[cfg(not(unix))]
fn terminate_timed_out_process(
    child: &mut Child,
    _termination_grace: Duration,
) -> Result<ExitStatus, String> {
    child
        .kill()
        .map_err(|exc| format!("Failed to kill timed-out CI process: {exc}"))?;
    child
        .wait()
        .map_err(|exc| format!("Failed to reap killed CI process: {exc}"))
}

#[cfg(unix)]
fn signal_process_group(process_group: i32, signal: i32) -> io::Result<bool> {
    let status = unsafe { libc::kill(-process_group, signal) };
    if status == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn process_group_exists(process_group: i32) -> bool {
    signal_process_group(process_group, 0).unwrap_or(true)
}

fn configure_ci_process_group(command: &mut Command) {
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

pub(crate) fn configure_ci_process_priority(command: &mut Command) -> Result<(), String> {
    let nice_value = configured_ci_process_nice()?;
    configure_ci_process_priority_with_value(command, nice_value);
    Ok(())
}

fn configured_ci_process_nice() -> Result<i32, String> {
    let configured = CI_PROCESS_NICE_ENV_NAMES.into_iter().find_map(|name| {
        env::var(name)
            .ok()
            .map(|value| (name, value))
            .filter(|(_, value)| !value.trim().is_empty())
    });
    let Some((name, raw_value)) = configured else {
        return Ok(DEFAULT_CI_PROCESS_NICE);
    };
    parse_ci_process_nice(&raw_value).map_err(|error| format!("{name} {error}; got `{raw_value}`"))
}

fn parse_ci_process_nice(raw_value: &str) -> Result<i32, String> {
    let nice_value = raw_value
        .trim()
        .parse::<i32>()
        .map_err(|_| "must be an integer from 0 through 19".to_string())?;
    if !(0..=19).contains(&nice_value) {
        return Err("must be an integer from 0 through 19".to_string());
    }
    Ok(nice_value)
}

fn configure_ci_process_priority_with_value(command: &mut Command, nice_value: i32) {
    #[cfg(unix)]
    unsafe {
        command.pre_exec(move || {
            let status = libc::setpriority(libc::PRIO_PROCESS as _, 0, nice_value);
            if status == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    #[cfg(not(unix))]
    {
        let _ = (command, nice_value);
    }
}

struct ProcessStreamFiles {
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

impl ProcessStreamFiles {
    fn create(log_path: &Path) -> Result<Self, String> {
        let stdout_path = log_path.with_extension("stdout.tmp");
        let stderr_path = log_path.with_extension("stderr.tmp");
        File::create(&stdout_path).map_err(|exc| {
            format!(
                "Failed to create CI process stdout stream `{}`: {exc}",
                path_text(&stdout_path)
            )
        })?;
        File::create(&stderr_path).map_err(|exc| {
            format!(
                "Failed to create CI process stderr stream `{}`: {exc}",
                path_text(&stderr_path)
            )
        })?;
        Ok(Self {
            stdout_path,
            stderr_path,
        })
    }

    fn stdout_file(&self) -> Result<File, String> {
        append_file(&self.stdout_path, "stdout")
    }

    fn stderr_file(&self) -> Result<File, String> {
        append_file(&self.stderr_path, "stderr")
    }
}

impl Drop for ProcessStreamFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.stdout_path);
        let _ = fs::remove_file(&self.stderr_path);
    }
}

fn append_file(path: &Path, stream: &str) -> Result<File, String> {
    File::options().append(true).open(path).map_err(|exc| {
        format!(
            "Failed to open CI process {stream} stream `{}`: {exc}",
            path_text(path)
        )
    })
}

fn file_len_usize(path: &Path) -> Result<usize, String> {
    let len = fs::metadata(path)
        .map_err(|exc| {
            format!(
                "Failed to inspect CI process stream `{}`: {exc}",
                path_text(path)
            )
        })?
        .len();
    usize::try_from(len).map_err(|_| {
        format!(
            "CI process stream `{}` is too large to report on this platform: {len} bytes",
            path_text(path)
        )
    })
}

fn read_file_tail(path: &Path, limit: u64) -> Result<String, String> {
    let mut file = File::open(path).map_err(|exc| {
        format!(
            "Failed to open CI process stream `{}` for tail read: {exc}",
            path_text(path)
        )
    })?;
    let len = file
        .metadata()
        .map_err(|exc| {
            format!(
                "Failed to inspect CI process stream `{}`: {exc}",
                path_text(path)
            )
        })?
        .len();
    file.seek(SeekFrom::Start(len.saturating_sub(limit)))
        .map_err(|exc| {
            format!(
                "Failed to seek CI process stream `{}`: {exc}",
                path_text(path)
            )
        })?;
    let mut bytes = Vec::with_capacity(usize::try_from(len.min(limit)).unwrap_or(0));
    file.read_to_end(&mut bytes).map_err(|exc| {
        format!(
            "Failed to read CI process stream tail `{}`: {exc}",
            path_text(path)
        )
    })?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_bounded_file(path: &Path, limit: u64) -> Result<String, String> {
    let len = fs::metadata(path)
        .map_err(|exc| {
            format!(
                "Failed to inspect CI stdout stream `{}`: {exc}",
                path_text(path)
            )
        })?
        .len();
    if len > limit {
        return Err(format!(
            "CI stdout capture exceeds the bounded {limit}-byte parser limit ({len} bytes); full output remains in the process log"
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|exc| {
            format!(
                "Failed to read bounded CI stdout `{}`: {exc}",
                path_text(path)
            )
        })?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

struct ProcessLogMetadata<'a> {
    command: &'a str,
    cwd: &'a Path,
    exit_code: i32,
    timed_out: bool,
    timeout_seconds: f64,
}

fn write_process_log_from_streams(
    log_path: &Path,
    metadata: ProcessLogMetadata<'_>,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<(), String> {
    let mut log = File::create(log_path).map_err(|exc| {
        format!(
            "Failed to create process log `{}`: {exc}",
            path_text(log_path)
        )
    })?;
    writeln!(
        log,
        "$ {}\ncwd={}\nexit_code={}\ntimed_out={}\ntimeout_seconds={:.3}\n\nstdout:",
        metadata.command,
        path_text(metadata.cwd),
        metadata.exit_code,
        metadata.timed_out,
        metadata.timeout_seconds,
    )
    .map_err(|exc| format!("Failed to write process log header: {exc}"))?;
    copy_stream(stdout_path, &mut log)?;
    log.write_all(b"\n\nstderr:\n")
        .map_err(|exc| format!("Failed to write process log separator: {exc}"))?;
    copy_stream(stderr_path, &mut log)?;
    log.write_all(b"\n")
        .map_err(|exc| format!("Failed to finish process log: {exc}"))?;
    log.flush().map_err(|exc| {
        format!(
            "Failed to flush process log `{}`: {exc}",
            path_text(log_path)
        )
    })
}

fn copy_stream(path: &Path, target: &mut File) -> Result<(), String> {
    let mut source = File::open(path).map_err(|exc| {
        format!(
            "Failed to open CI process stream `{}` for log copy: {exc}",
            path_text(path)
        )
    })?;
    io::copy(&mut source, target).map_err(|exc| {
        format!(
            "Failed to copy CI process stream `{}` into process log: {exc}",
            path_text(path)
        )
    })?;
    Ok(())
}

fn tail_chars(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    text.chars().skip(count - limit).collect()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn test_output_dir(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "ait-ci-process-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn ci_nice_override_is_bounded() {
        assert_eq!(parse_ci_process_nice("0"), Ok(0));
        assert_eq!(parse_ci_process_nice("19"), Ok(19));
        assert!(parse_ci_process_nice("-1").is_err());
        assert!(parse_ci_process_nice("20").is_err());
        assert!(parse_ci_process_nice("not-a-number").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn configured_child_runs_at_requested_lower_priority() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "ps -o nice= -p $$"]);
        configure_ci_process_priority_with_value(&mut command, DEFAULT_CI_PROCESS_NICE);

        let output = command.output().expect("spawn nice child");
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<i32>()
                .expect("child nice value"),
            DEFAULT_CI_PROCESS_NICE
        );
    }

    #[test]
    fn ci_timeout_seconds_are_defaulted_and_bounded() {
        assert_eq!(
            validated_ci_process_timeout_seconds(None, "runner.timeout_seconds"),
            Ok(DEFAULT_CI_PROCESS_TIMEOUT_SECONDS)
        );
        assert_eq!(
            validated_ci_process_timeout_seconds(Some(1), "runner.timeout_seconds"),
            Ok(1)
        );
        assert_eq!(
            validated_ci_process_timeout_seconds(
                Some(MAX_CI_PROCESS_TIMEOUT_SECONDS as i64),
                "runner.timeout_seconds"
            ),
            Ok(MAX_CI_PROCESS_TIMEOUT_SECONDS)
        );
        assert!(validated_ci_process_timeout_seconds(Some(0), "runner.timeout_seconds").is_err());
        assert!(validated_ci_process_timeout_seconds(Some(-1), "runner.timeout_seconds").is_err());
        assert!(validated_ci_process_timeout_seconds(
            Some(MAX_CI_PROCESS_TIMEOUT_SECONDS as i64 + 1),
            "runner.timeout_seconds"
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn streamed_process_timeout_kills_descendants_and_reaps_streams() {
        let output_dir = test_output_dir("timeout");
        fs::create_dir_all(&output_dir).expect("create timeout output");
        let descendant_pid_path = output_dir.join("descendant.pid");
        let log_path = output_dir.join("timeout.log");
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' TERM; /bin/sh -c 'trap \"\" TERM; sleep 30' & echo $! > \"$DESCENDANT_PID_PATH\"; wait",
            ])
            .env("DESCENDANT_PID_PATH", &descendant_pid_path);

        let output = run_streamed_command(
            &mut command,
            &log_path,
            "timeout fixture",
            Path::new("/"),
            CiProcessStdoutCapture::None,
            CiProcessExecutionOptions {
                timeout: Duration::from_secs(1),
                termination_grace: Duration::from_millis(100),
            },
        )
        .expect("timed-out process should return bounded evidence");

        assert!(output.timed_out);
        assert!(!output.status.success());
        assert!(log_path.is_file());
        assert!(fs::read_to_string(&log_path)
            .expect("read timeout log")
            .contains("timed_out=true"));
        assert!(!log_path.with_extension("stdout.tmp").exists());
        assert!(!log_path.with_extension("stderr.tmp").exists());

        let descendant_pid = fs::read_to_string(&descendant_pid_path)
            .expect("descendant PID should be recorded")
            .trim()
            .parse::<i32>()
            .expect("descendant PID should be numeric");
        let deadline = Instant::now() + Duration::from_secs(2);
        while process_exists(descendant_pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !process_exists(descendant_pid),
            "timed-out process group left descendant PID {descendant_pid} alive"
        );

        let _ = fs::remove_dir_all(output_dir);
    }

    #[cfg(unix)]
    fn process_exists(pid: i32) -> bool {
        let status = unsafe { libc::kill(pid, 0) };
        status == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(unix)]
    #[test]
    fn optional_stdout_capture_stays_bounded_while_log_is_complete() {
        let output_dir = test_output_dir("optional-capture");
        fs::create_dir_all(&output_dir).expect("create capture output");
        let log_path = output_dir.join("capture.log");
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 4096 /dev/zero | tr '\\000' x"]);

        let output = run_streamed_command(
            &mut command,
            &log_path,
            "optional capture fixture",
            Path::new("/"),
            CiProcessStdoutCapture::Optional(32),
            CiProcessExecutionOptions::from_timeout_seconds(5),
        )
        .expect("optional capture should not fail on overflow");

        assert!(output.status.success());
        assert!(!output.timed_out);
        assert_eq!(output.stdout_bytes, 4_096);
        assert!(output.captured_stdout.is_none());
        assert!(output.stdout_capture_truncated);
        assert!(fs::metadata(&log_path).expect("inspect complete log").len() > 4_096);

        let _ = fs::remove_dir_all(output_dir);
    }
}
