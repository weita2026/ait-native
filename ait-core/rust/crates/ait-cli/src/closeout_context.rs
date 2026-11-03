//! Process context explicitly owned by CLI dispatch, never by library callers.
use std::cell::Cell;

thread_local! {
    static CLI_CLOSEOUT: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn with_cli_closeout<T>(operation: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            CLI_CLOSEOUT.set(self.0);
        }
    }
    let _restore = Restore(CLI_CLOSEOUT.replace(true));
    operation()
}

#[cfg(any(windows, test))]
pub(crate) fn release_owned_worktree_cwd(
    authoritative_root: &std::path::Path,
    worktree: &std::path::Path,
) -> Result<(), String> {
    if !CLI_CLOSEOUT.get() {
        return Ok(());
    }
    let current = std::env::current_dir().map_err(|error| error.to_string())?;
    let current = std::fs::canonicalize(current).map_err(|error| error.to_string())?;
    let worktree = std::fs::canonicalize(worktree).map_err(|error| error.to_string())?;
    if !current.starts_with(&worktree) {
        return Ok(());
    }
    let root = std::fs::canonicalize(authoritative_root).map_err(|error| error.to_string())?;
    if root.starts_with(&worktree) {
        return Err(
            "Cannot release the CLI current directory into the worktree being closed".into(),
        );
    }
    // Call only after authoritative closeout/ownership admission and before
    // removing that exact worktree. Windows holds its own CWD directory open.
    // The CLI has already resolved inputs against its captured RepoRuntime;
    // shared-library callers do not opt into this process-global handoff.
    std::env::set_current_dir(&root).map_err(|error| {
        format!(
            "Failed to release CLI current directory from {} to {} before owned closeout: {error}",
            current.display(),
            root.display()
        )
    })?;
    #[cfg(all(windows, debug_assertions))]
    {
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr().lock(),
            "worktree-closeout-cwd pid={} before={current:?} after={:?} worktree={worktree:?}",
            std::process::id(),
            std::env::current_dir()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn library_context_and_unrelated_cli_directory_are_unchanged() {
        let before = std::env::current_dir().unwrap();
        // Library calls do not inspect or change process context at all.
        release_owned_worktree_cwd(
            std::path::Path::new("absent-root"),
            std::path::Path::new("absent-worktree"),
        )
        .unwrap();
        assert_eq!(std::env::current_dir().unwrap(), before);
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("unrelated-worktree");
        fs::create_dir(&worktree).unwrap();
        with_cli_closeout(|| release_owned_worktree_cwd(temp.path(), &worktree)).unwrap();
        assert_eq!(std::env::current_dir().unwrap(), before);
        assert!(!CLI_CLOSEOUT.get());
    }

    #[test]
    fn cli_context_is_restored_after_an_error_or_unwind() {
        assert!(!CLI_CLOSEOUT.get());
        let failed: Result<(), ()> = with_cli_closeout(|| Err(()));
        assert!(failed.is_err());
        assert!(!CLI_CLOSEOUT.get());
        let panic = std::panic::catch_unwind(|| with_cli_closeout(|| panic!("scope probe")));
        assert!(panic.is_err());
        assert!(!CLI_CLOSEOUT.get());
    }

    #[cfg(windows)]
    #[test]
    fn windows_own_cwd_handle_is_released_only_inside_cli_closeout() {
        const ROOT: &str = "AIT_TEST_CLOSEOUT_CWD_ROOT";
        if let Some(root) = std::env::var_os(ROOT) {
            let root = std::path::PathBuf::from(root);
            let worktree = root.join("worktree");
            let authority = root.join("authority");
            let before = std::env::current_dir().unwrap();
            release_owned_worktree_cwd(&authority, &worktree).unwrap();
            assert_eq!(std::env::current_dir().unwrap(), before);
            let baseline = fs::remove_dir(&worktree)
                .expect_err("Windows must retain its current directory handle");
            eprintln!("windows-closeout-cwd-probe baseline={baseline:?}");
            with_cli_closeout(|| release_owned_worktree_cwd(&authority, &worktree)).unwrap();
            assert_eq!(
                fs::canonicalize(std::env::current_dir().unwrap()).unwrap(),
                fs::canonicalize(&authority).unwrap()
            );
            fs::remove_dir(&worktree).unwrap();
            assert!(authority.is_dir());
            eprintln!("windows-closeout-cwd-probe own-handoff=Ok exact-worktree-remove=Ok");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        fs::create_dir(&worktree).unwrap();
        fs::create_dir(temp.path().join("authority")).unwrap();
        let case = std::thread::current().name().unwrap().to_owned();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &case, "--nocapture"])
            .env(ROOT, temp.path())
            .current_dir(&worktree)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "status={:?}\nstdout={}\nstderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!worktree.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_other_process_cwd_lock_remains_an_error() {
        const ROOT: &str = "AIT_TEST_CLOSEOUT_OTHER_CWD_ROOT";
        if let Some(root) = std::env::var_os(ROOT) {
            let root = std::path::PathBuf::from(root);
            fs::write(root.join("ready"), b"ready").unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !root.join("stop").is_file() {
                assert!(std::time::Instant::now() < deadline, "holder timed out");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("held-worktree");
        fs::create_dir(&worktree).unwrap();
        let case = std::thread::current().name().unwrap().to_owned();
        let mut holder = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &case, "--nocapture"])
            .env(ROOT, temp.path())
            .current_dir(&worktree)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !temp.path().join("ready").is_file() {
            if std::time::Instant::now() >= deadline {
                let _ = holder.kill();
                let _ = holder.wait();
                panic!("holder did not become ready");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let before = std::env::current_dir().unwrap();
        with_cli_closeout(|| release_owned_worktree_cwd(temp.path(), &worktree)).unwrap();
        assert_eq!(std::env::current_dir().unwrap(), before);
        let denied = fs::remove_dir(&worktree);
        fs::write(temp.path().join("stop"), b"stop").unwrap();
        assert!(holder.wait().unwrap().success());
        assert!(
            denied.is_err(),
            "another process's directory handle must remain a real error"
        );
        assert!(worktree.exists());
        fs::remove_dir(&worktree).unwrap();
    }
}
