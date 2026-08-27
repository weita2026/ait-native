use std::env;
use std::ffi::{OsStr, OsString};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use ait_server_core::foundation::server_binary_lifecycle::SERVER_DATA_ENV;

pub const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:8088";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleCommand {
    Help,
    Init,
    Probe,
    Serve,
    Version,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleOptions {
    pub command: LifecycleCommand,
    pub data_root: Option<PathBuf>,
    pub listen_address: Option<SocketAddr>,
    pub defer_ci_admission: bool,
    pub init_if_missing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedLifecycle {
    pub command: LifecycleCommand,
    pub data_root: PathBuf,
    pub data_root_source: &'static str,
    pub listen_address: SocketAddr,
    pub defer_ci_admission: bool,
    pub init_if_missing: bool,
}

pub fn lifecycle_usage() -> &'static str {
    "Usage:\n  ait-server [--data <absolute-path>] [--listen <ip:port>] [--init-if-missing] [--defer-ci-admission]\n  ait-server init [--data <absolute-path>]\n  ait-server probe [--data <absolute-path>] [--defer-ci-admission]\n  ait-server --startup-probe\n  ait-server --help\n  ait-server --version\n\nWith no arguments, ait-server starts with the platform user-data root,\ninitializes it on first use, listens on 127.0.0.1:8088, and defers the startup\nCI RAM-workspace probe. An explicit --data or AIT_NATIVE_SERVER_DATA root does\nnot enable guarded initialization unless --init-if-missing is supplied.\n--listen is accepted only when starting the server.\n--defer-ci-admission skips only the startup RAM-workspace probe; managed CI\nallocation still fails closed until a memory-backed root is configured."
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
    if values.len() == 1 && matches!(values[0].to_str(), Some("--version") | Some("-V")) {
        return Ok(LifecycleOptions {
            command: LifecycleCommand::Version,
            data_root: None,
            listen_address: None,
            defer_ci_admission: false,
            init_if_missing: false,
        });
    }
    if values.len() == 1 && matches!(values[0].to_str(), Some("--help") | Some("-h")) {
        return Ok(LifecycleOptions {
            command: LifecycleCommand::Help,
            data_root: None,
            listen_address: None,
            defer_ci_admission: false,
            init_if_missing: false,
        });
    }

    let mut command = None;
    let mut data_root = None;
    let mut listen_address = None;
    let mut defer_ci_admission = false;
    let mut init_if_missing = false;
    let mut index = 0usize;
    while index < values.len() {
        let value = values[index]
            .to_str()
            .ok_or_else(|| "ait-server arguments must be valid UTF-8".to_string())?;
        match value {
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
            "--listen" => {
                index += 1;
                let raw = values
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "--listen requires an IP address and port".to_string())?;
                let parsed = raw
                    .parse::<SocketAddr>()
                    .map_err(|_| format!("--listen must be an IP address and port: {raw}"))?;
                if listen_address.replace(parsed).is_some() {
                    return Err("--listen may be supplied only once".to_string());
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
    let command = command.unwrap_or(LifecycleCommand::Serve);
    if init_if_missing && command != LifecycleCommand::Serve {
        return Err("--init-if-missing is accepted only when starting ait-server".to_string());
    }
    if defer_ci_admission && command == LifecycleCommand::Init {
        return Err("--defer-ci-admission has no meaning for `ait-server init`".to_string());
    }
    if listen_address.is_some() && command != LifecycleCommand::Serve {
        return Err("--listen is accepted only when starting ait-server".to_string());
    }
    if let Some(root) = data_root.as_deref() {
        require_absolute_data_root(root)?;
    }
    Ok(LifecycleOptions {
        command,
        data_root,
        listen_address,
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
        (root, "cli", false)
    } else if let Some(root) = nonempty_env_path(SERVER_DATA_ENV) {
        (root, "env:AIT_NATIVE_SERVER_DATA", false)
    } else {
        let root = platform_user_data_root()?;
        (root, "platform-user-default", true)
    };
    require_absolute_data_root(&data_root)?;

    let defer_ci_admission = options.defer_ci_admission || platform_default;
    Ok(PreparedLifecycle {
        command: options.command,
        data_root,
        data_root_source,
        listen_address: options.listen_address.unwrap_or_else(|| {
            DEFAULT_LISTEN_ADDRESS
                .parse()
                .expect("default listen address must remain valid")
        }),
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
    fn parser_preserves_direct_start_and_native_commands() {
        assert_eq!(
            parse_lifecycle_args(Vec::<OsString>::new())
                .unwrap()
                .command,
            LifecycleCommand::Serve
        );
        assert_eq!(
            parse_lifecycle_args(["--listen", "127.0.0.1:9090"])
                .unwrap()
                .listen_address,
            Some("127.0.0.1:9090".parse().unwrap())
        );
        assert_eq!(
            parse_lifecycle_args(["--data", "/tmp/ait-server"])
                .unwrap()
                .command,
            LifecycleCommand::Serve
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
            .contains("only when starting"));
        assert!(parse_lifecycle_args(["run"])
            .unwrap_err()
            .contains("unknown ait-server argument `run`"));
        assert!(parse_lifecycle_args(["--data", "relative"])
            .unwrap_err()
            .contains("absolute"));
        assert!(
            parse_lifecycle_args(["probe", "--listen", "127.0.0.1:9090"])
                .unwrap_err()
                .contains("only when starting")
        );
        assert!(parse_lifecycle_args(["--listen", "localhost:9090"])
            .unwrap_err()
            .contains("IP address"));
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
