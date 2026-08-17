use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ait_server_core::foundation::ci_runtime_temp::validated_ci_ram_runtime_root_with_source;
use ait_server_core::foundation::ci_workspace_cleanup::{
    prune_runtime_temp_namespace_json, RuntimeTempPruneRequest,
};
const PROBE_DIR_NAME: &str = ".startup-probe";
const PROBE_FILE_PREFIX: &str = "ait-server-runtime-probe";
const PROBE_PAYLOAD: &[u8] = b"ait-server startup runtime probe\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupProbeReport {
    pub runtime_root: PathBuf,
    pub ci_ram_runtime_root: PathBuf,
    pub ci_ram_runtime_root_source: String,
    pub ci_startup_admission_deferred: bool,
    pub ci_runtime_pruned_run_base_count: usize,
    pub launch_hint: Option<String>,
    pub object_probe: String,
}

pub fn ensure_durable_runtime_access(root: &Path) -> Result<StartupProbeReport, String> {
    probe_runtime_root(root)
}

pub fn ensure_startup_runtime_access(
    root: &Path,
    defer_ci_admission: bool,
) -> Result<StartupProbeReport, String> {
    let mut report = ensure_durable_runtime_access(root)?;
    if defer_ci_admission {
        report.ci_ram_runtime_root_source = "deferred".to_string();
        report.ci_startup_admission_deferred = true;
        return Ok(report);
    }
    let (ci_ram_runtime_root, ci_ram_runtime_root_source) =
        validated_ci_ram_runtime_root_with_source()?;
    probe_runtime_root(&ci_ram_runtime_root).map_err(|error| {
        format!(
            "CI RAM runtime root `{}` is not usable; CI is configured to fail closed instead of spilling into durable storage or /tmp: {error}",
            ci_ram_runtime_root.display()
        )
    })?;
    let ci_runtime_pruned_run_base_count =
        prune_ci_runtime_namespaces(&ci_ram_runtime_root, &report.runtime_root)?;
    report.ci_ram_runtime_root = ci_ram_runtime_root;
    report.ci_ram_runtime_root_source = ci_ram_runtime_root_source;
    report.ci_runtime_pruned_run_base_count = ci_runtime_pruned_run_base_count;
    Ok(report)
}

fn prune_ci_runtime_namespaces(
    ci_ram_root: &Path,
    server_data_root: &Path,
) -> Result<usize, String> {
    const NAMESPACES: [&str; 4] = [
        "patchset-ci",
        "repo-ci",
        "land-main-seed",
        "snapshot-materialize",
    ];
    let mut roots = BTreeSet::<PathBuf>::new();
    for namespace in NAMESPACES {
        roots.insert(ci_ram_root.join("ci-runs").join(namespace));
        roots.insert(server_data_root.join("tmp").join(namespace));
        roots.insert(env::temp_dir().join("ait-server").join(namespace));
    }
    let mut removed = 0usize;
    for namespace_root in roots {
        let value = prune_runtime_temp_namespace_json(
            &RuntimeTempPruneRequest::default_for_namespace(namespace_root),
        )?;
        for key in ["removed_completed", "removed_abandoned"] {
            removed += value
                .get(key)
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
        }
    }
    Ok(removed)
}

fn probe_runtime_root(root: &Path) -> Result<StartupProbeReport, String> {
    create_dir_all_with_context(root, "runtime root")?;

    let probe_dir = root.join(PROBE_DIR_NAME);
    create_dir_all_with_context(&probe_dir, "runtime probe directory")?;

    let probe_path = probe_dir.join(format!(
        "{}-{}-{}.txt",
        PROBE_FILE_PREFIX,
        std::process::id(),
        timestamp_millis()
    ));
    write_probe_file(&probe_path)?;
    read_probe_file(&probe_path)?;
    remove_probe_file(&probe_path)?;
    let _ = fs::remove_dir(&probe_dir);

    let object_probe = probe_existing_object(root)?;
    Ok(StartupProbeReport {
        runtime_root: root.to_path_buf(),
        ci_ram_runtime_root: PathBuf::new(),
        ci_ram_runtime_root_source: String::new(),
        ci_startup_admission_deferred: false,
        ci_runtime_pruned_run_base_count: 0,
        launch_hint: launch_hint(root),
        object_probe,
    })
}

fn create_dir_all_with_context(path: &Path, context: &str) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| {
        startup_access_error(
            format!("failed to create {context} `{}`: {err}", path.display()),
            Some(&err),
            path,
        )
    })
}

fn write_probe_file(path: &Path) -> Result<(), String> {
    let mut file = File::create(path).map_err(|err| {
        startup_access_error(
            format!("failed to write startup probe `{}`: {err}", path.display()),
            Some(&err),
            path,
        )
    })?;
    file.write_all(PROBE_PAYLOAD).map_err(|err| {
        startup_access_error(
            format!(
                "failed to write startup probe payload `{}`: {err}",
                path.display()
            ),
            Some(&err),
            path,
        )
    })?;
    file.sync_all().map_err(|err| {
        startup_access_error(
            format!("failed to sync startup probe `{}`: {err}", path.display()),
            Some(&err),
            path,
        )
    })
}

fn read_probe_file(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|err| {
        startup_access_error(
            format!("failed to read startup probe `{}`: {err}", path.display()),
            Some(&err),
            path,
        )
    })?;
    if bytes == PROBE_PAYLOAD {
        Ok(())
    } else {
        Err(format!(
            "startup probe readback mismatch for `{}`; refusing to start with an unreliable runtime root.",
            path.display()
        ))
    }
}

fn remove_probe_file(path: &Path) -> Result<(), String> {
    fs::remove_file(path).map_err(|err| {
        startup_access_error(
            format!("failed to remove startup probe `{}`: {err}", path.display()),
            Some(&err),
            path,
        )
    })
}

fn probe_existing_object(root: &Path) -> Result<String, String> {
    let object_roots = [
        root.join("objects").join("packs"),
        root.join("objects").join("tree-packs"),
    ];
    for object_root in object_roots {
        let Ok(entries) = fs::read_dir(&object_root) else {
            continue;
        };
        for entry in entries {
            let entry = entry.map_err(|err| {
                startup_access_error(
                    format!(
                        "failed to inspect runtime object directory `{}`: {err}",
                        object_root.display()
                    ),
                    Some(&err),
                    &object_root,
                )
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("zip") {
                continue;
            }
            read_object_prefix(&path)?;
            return Ok(format!("read:{}", path.display()));
        }
    }
    Ok("skipped:no-existing-pack".to_string())
}

fn read_object_prefix(path: &Path) -> Result<(), String> {
    let mut file = File::open(path).map_err(|err| {
        startup_access_error(
            format!(
                "failed to open existing runtime object `{}`: {err}",
                path.display()
            ),
            Some(&err),
            path,
        )
    })?;
    let mut buffer = [0u8; 64];
    let _ = file.read(&mut buffer).map_err(|err| {
        startup_access_error(
            format!(
                "failed to read existing runtime object `{}`: {err}",
                path.display()
            ),
            Some(&err),
            path,
        )
    })?;
    Ok(())
}

fn startup_access_error(message: String, error: Option<&std::io::Error>, path: &Path) -> String {
    let mut out = message;
    let permission_denied = error
        .map(|err| err.kind() == ErrorKind::PermissionDenied)
        .unwrap_or(false);
    if permission_denied {
        out.push_str(
            "\n\nThe current ait-server process cannot access its configured durable runtime root.",
        );
        if cfg!(target_os = "macos") && path.starts_with("/Volumes") {
            out.push_str("\nThe path is under /Volumes; on macOS, launchd/daemon jobs can be denied external-volume content access even when an interactive shell can read the same files.");
        }
        out.push_str("\nStart ait-server with a supervisor, service account, container mount, or session that has read/write access to AIT_NATIVE_SERVER_DATA, or choose a durable runtime root that this process can access.");
    }
    out
}

fn launch_hint(root: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        let _ = root;
        return None;
    }
    if !root.starts_with("/Volumes") {
        return None;
    }
    if env::var("STY").is_ok() || env::var("TMUX").is_ok() {
        return None;
    }
    let xpc_service_name = env::var("XPC_SERVICE_NAME").unwrap_or_default();
    if !xpc_service_name.is_empty() && xpc_service_name != "0" {
        return Some(
            "runtime root is under /Volumes while this process has an XPC_SERVICE_NAME; macOS launchd/TCC may deny external-volume content access unless the launched service has the required permissions.".to_string(),
        );
    }
    Some(
        "runtime root is under /Volumes; verify that the selected macOS launcher can read and write that external-volume path from its own process context.".to_string(),
    )
}

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn probe_runtime_root_writes_reads_and_removes_probe_file() {
        let root = test_root("startup-probe-ok");
        let report = probe_runtime_root(&root).expect("probe should pass");

        assert_eq!(report.runtime_root, root);
        assert_eq!(report.object_probe, "skipped:no-existing-pack");
        assert!(!report.runtime_root.join(PROBE_DIR_NAME).exists());

        fs::remove_dir_all(report.runtime_root).expect("test root cleanup should pass");
    }

    #[test]
    fn startup_access_error_mentions_user_session_for_permission_denied_volumes() {
        let error = std::io::Error::new(ErrorKind::PermissionDenied, "operation not permitted");
        let message = startup_access_error(
            "failed".to_string(),
            Some(&error),
            Path::new("/Volumes/external/ait-runtime/server-data"),
        );

        if cfg!(target_os = "macos") {
            assert!(message.contains("launchd/daemon jobs"));
        }
        assert!(message.contains("AIT_NATIVE_SERVER_DATA"));
    }

    fn test_root(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "ait-server-{}-{}-{}",
            name,
            std::process::id(),
            TEST_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
