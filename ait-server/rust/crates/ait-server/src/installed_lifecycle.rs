use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use ait_server_core::foundation::server_binary_lifecycle::{RUNTIME_DATA_ENV, SERVER_DATA_ENV};

pub const CI_STARTUP_ADMISSION_ENV: &str = "AIT_NATIVE_SERVER_CI_STARTUP_ADMISSION";
pub const CI_STARTUP_ADMISSION_DEFERRED: &str = "deferred";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleCommand {
    Help,
    Init,
    Probe,
    Run,
    Version,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleOptions {
    pub command: LifecycleCommand,
    pub data_root: Option<PathBuf>,
    pub defer_ci_admission: bool,
    pub init_if_missing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedLifecycle {
    pub command: LifecycleCommand,
    pub data_root: PathBuf,
    pub data_root_source: &'static str,
    pub defer_ci_admission: bool,
    pub init_if_missing: bool,
}

pub fn lifecycle_usage() -> &'static str {
    "Usage:\n  ait-server run [--data <absolute-path>] [--init-if-missing] [--defer-ci-admission]\n  ait-server init [--data <absolute-path>]\n  ait-server probe [--data <absolute-path>] [--defer-ci-admission]\n  ait-server --startup-probe\n  ait-server --version\n\nWith no arguments, ait-server prints this help without starting the server.\nWith no --data flag or AIT_NATIVE_SERVER_DATA/AIT_RUNTIME_DATA environment,\nAIT uses the platform user-data root and safely initializes it on first run.\n--defer-ci-admission skips only the startup RAM-workspace probe; managed CI\nallocation still fails closed until a memory-backed root is configured."
}

pub fn parse_lifecycle_args<I, S>(args: I) -> Result<LifecycleOptions, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let values = args
        .into_iter()
        .map(|value| value.as_ref().to_os_string())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(LifecycleOptions {
            command: LifecycleCommand::Help,
            data_root: None,
            defer_ci_admission: false,
            init_if_missing: false,
        });
    }
    if values.len() == 1 && matches!(values[0].to_str(), Some("--version") | Some("-V")) {
        return Ok(LifecycleOptions {
            command: LifecycleCommand::Version,
            data_root: None,
            defer_ci_admission: false,
            init_if_missing: false,
        });
    }
    if values.len() == 1 && matches!(values[0].to_str(), Some("--help") | Some("-h")) {
        return Ok(LifecycleOptions {
            command: LifecycleCommand::Help,
            data_root: None,
            defer_ci_admission: false,
            init_if_missing: false,
        });
    }

    let mut command = None;
    let mut data_root = None;
    let mut defer_ci_admission = false;
    let mut init_if_missing = false;
    let mut index = 0usize;
    while index < values.len() {
        let value = values[index]
            .to_str()
            .ok_or_else(|| "ait-server arguments must be valid UTF-8".to_string())?;
        match value {
            "run" => select_command(&mut command, LifecycleCommand::Run)?,
            "init" => select_command(&mut command, LifecycleCommand::Init)?,
            "probe" | "--startup-probe" => select_command(&mut command, LifecycleCommand::Probe)?,
            "--data" => {
                index += 1;
                let raw = values
                    .get(index)
                    .ok_or_else(|| "--data requires an absolute path".to_string())?;
                if data_root.replace(PathBuf::from(raw)).is_some() {
                    return Err("--data may be supplied only once".to_string());
                }
            }
            "--init-if-missing" => init_if_missing = true,
            "--defer-ci-admission" => defer_ci_admission = true,
            "--version" | "-V" => {
                return Err("--version must be used without other arguments".to_string())
            }
            "--help" | "-h" => {
                return Err("--help must be used without other arguments".to_string())
            }
            _ => return Err(format!("unknown ait-server argument `{value}`")),
        }
        index += 1;
    }
    let command = command.unwrap_or(LifecycleCommand::Run);
    if init_if_missing && command != LifecycleCommand::Run {
        return Err("--init-if-missing is accepted only with `ait-server run`".to_string());
    }
    if defer_ci_admission && command == LifecycleCommand::Init {
        return Err("--defer-ci-admission has no meaning for `ait-server init`".to_string());
    }
    if let Some(root) = data_root.as_deref() {
        require_absolute_data_root(root)?;
    }
    Ok(LifecycleOptions {
        command,
        data_root,
        defer_ci_admission,
        init_if_missing,
    })
}

fn select_command(
    selected: &mut Option<LifecycleCommand>,
    command: LifecycleCommand,
) -> Result<(), String> {
    if let Some(existing) = selected {
        return Err(format!(
            "ait-server lifecycle commands are mutually exclusive: {existing:?} and {command:?}"
        ));
    }
    *selected = Some(command);
    Ok(())
}

fn require_absolute_data_root(root: &Path) -> Result<(), String> {
    if root.as_os_str().is_empty() {
        return Err("--data cannot be empty".to_string());
    }
    if !root.is_absolute() {
        return Err(format!(
            "--data must be an absolute path: {}",
            root.display()
        ));
    }
    Ok(())
}

pub fn prepare_installed_lifecycle(options: LifecycleOptions) -> Result<PreparedLifecycle, String> {
    let (data_root, data_root_source, platform_default) = if let Some(root) = options.data_root {
        require_absolute_data_root(&root)?;
        env::set_var(SERVER_DATA_ENV, &root);
        (root, "cli", false)
    } else if let Some(root) = nonempty_env_path(SERVER_DATA_ENV) {
        (root, "env:AIT_NATIVE_SERVER_DATA", false)
    } else if let Some(root) = nonempty_env_path(RUNTIME_DATA_ENV) {
        (root, "env:AIT_RUNTIME_DATA", false)
    } else {
        let root = platform_user_data_root()?;
        env::set_var(SERVER_DATA_ENV, &root);
        (root, "platform-user-default", true)
    };
    require_absolute_data_root(&data_root)?;

    let configured_ci_admission = env::var_os(CI_STARTUP_ADMISSION_ENV);
    let defer_ci_admission = options.defer_ci_admission
        || configured_ci_admission.as_deref() == Some(OsStr::new(CI_STARTUP_ADMISSION_DEFERRED))
        || (platform_default && configured_ci_admission.is_none());
    if options.defer_ci_admission || (platform_default && configured_ci_admission.is_none()) {
        env::set_var(CI_STARTUP_ADMISSION_ENV, CI_STARTUP_ADMISSION_DEFERRED);
    }
    Ok(PreparedLifecycle {
        command: options.command,
        data_root,
        data_root_source,
        defer_ci_admission,
        init_if_missing: options.init_if_missing || platform_default,
    })
}

fn nonempty_env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn platform_user_data_root() -> Result<PathBuf, String> {
    platform_user_data_root_from(|name| env::var_os(name))
}

fn platform_user_data_root_from<F>(mut value: F) -> Result<PathBuf, String>
where
    F: FnMut(&str) -> Option<OsString>,
{
    if cfg!(target_os = "windows") {
        if let Some(root) = nonempty_value(value("LOCALAPPDATA")) {
            return Ok(PathBuf::from(root).join("AIT").join("server-data"));
        }
        if let Some(root) = nonempty_value(value("USERPROFILE")) {
            return Ok(PathBuf::from(root)
                .join("AppData")
                .join("Local")
                .join("AIT")
                .join("server-data"));
        }
        return Err(
            "Cannot resolve the Windows user-data root: LOCALAPPDATA and USERPROFILE are unset"
                .to_string(),
        );
    }
    if cfg!(target_os = "macos") {
        let home = nonempty_value(value("HOME"))
            .ok_or_else(|| "Cannot resolve the macOS user-data root: HOME is unset".to_string())?;
        return Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("AIT")
            .join("server-data"));
    }
    if let Some(root) = nonempty_value(value("XDG_STATE_HOME")) {
        return Ok(PathBuf::from(root).join("ait").join("server-data"));
    }
    let home = nonempty_value(value("HOME"))
        .ok_or_else(|| "Cannot resolve the Unix user-data root: HOME is unset".to_string())?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("ait")
        .join("server-data"))
}

fn nonempty_value(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_preserves_legacy_forms_and_adds_native_commands() {
        assert_eq!(
            parse_lifecycle_args(Vec::<OsString>::new())
                .unwrap()
                .command,
            LifecycleCommand::Help
        );
        assert_eq!(
            parse_lifecycle_args(["run"]).unwrap().command,
            LifecycleCommand::Run
        );
        assert_eq!(
            parse_lifecycle_args(["--data", "/tmp/ait-server"])
                .unwrap()
                .command,
            LifecycleCommand::Run
        );
        assert_eq!(
            parse_lifecycle_args(["--startup-probe"]).unwrap().command,
            LifecycleCommand::Probe
        );
        assert_eq!(
            parse_lifecycle_args(["probe", "--defer-ci-admission"])
                .unwrap()
                .command,
            LifecycleCommand::Probe
        );
        assert_eq!(
            parse_lifecycle_args(["init", "--data", "/tmp/ait-server"])
                .unwrap()
                .data_root,
            Some(PathBuf::from("/tmp/ait-server"))
        );
        assert!(parse_lifecycle_args(["init", "--init-if-missing"])
            .unwrap_err()
            .contains("only with"));
        assert!(parse_lifecycle_args(["run", "probe"])
            .unwrap_err()
            .contains("mutually exclusive"));
        assert!(parse_lifecycle_args(["--data", "relative"])
            .unwrap_err()
            .contains("absolute"));
    }

    #[test]
    fn unix_platform_default_prefers_xdg_state_then_home() {
        if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
            return;
        }
        let xdg = platform_user_data_root_from(|name| match name {
            "XDG_STATE_HOME" => Some(OsString::from("/state")),
            "HOME" => Some(OsString::from("/home/user")),
            _ => None,
        })
        .unwrap();
        assert_eq!(xdg, PathBuf::from("/state/ait/server-data"));

        let home = platform_user_data_root_from(|name| {
            (name == "HOME").then(|| OsString::from("/home/user"))
        })
        .unwrap();
        assert_eq!(
            home,
            PathBuf::from("/home/user/.local/state/ait/server-data")
        );
    }

    #[test]
    fn current_platform_default_is_user_scoped() {
        let root = platform_user_data_root_from(|name| match name {
            "LOCALAPPDATA" => Some(OsString::from(r"C:\Users\person\AppData\Local")),
            "USERPROFILE" => Some(OsString::from(r"C:\Users\person")),
            "XDG_STATE_HOME" => Some(OsString::from("/state")),
            "HOME" => Some(OsString::from("/home/person")),
            _ => None,
        })
        .unwrap();
        if cfg!(target_os = "windows") {
            assert_eq!(
                root,
                PathBuf::from(r"C:\Users\person\AppData\Local")
                    .join("AIT")
                    .join("server-data")
            );
        } else if cfg!(target_os = "macos") {
            assert_eq!(
                root,
                PathBuf::from("/home/person/Library/Application Support/AIT/server-data")
            );
        } else {
            assert_eq!(root, PathBuf::from("/state/ait/server-data"));
        }
    }
}
