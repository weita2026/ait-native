use std::env;
use std::path::{Path, PathBuf};

use crate::diagnostic::{WorkerDiagnostic, EXIT_INVALID_CONFIGURATION, EXIT_INVALID_REQUEST};

const REPO_DISCOVERY_ENV_VARS: &[&str] = &[
    "AIT_REPO_ROOT",
    "AIT_NATIVE_WORKSPACE_ROOT",
    "AIT_WORKSPACE_ROOT",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerPathInputs {
    pub current_dir: PathBuf,
    pub repo_root_override: Option<PathBuf>,
    pub manifest_path_override: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkerPaths {
    pub repo_root: PathBuf,
    pub manifest_path: PathBuf,
}

pub fn process_worker_path_inputs() -> Result<WorkerPathInputs, WorkerDiagnostic> {
    let current_dir = env::current_dir().map_err(|error| {
        WorkerDiagnostic::new(
            "current_directory_unavailable",
            format!("Cannot resolve the current directory: {error}"),
            EXIT_INVALID_REQUEST,
        )
    })?;
    let repo_root_override = REPO_DISCOVERY_ENV_VARS
        .iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .find(|path| !path.as_os_str().is_empty());
    let manifest_path_override = env::var_os("AIT_AGENT_CONFIG_PATH")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    Ok(WorkerPathInputs {
        current_dir,
        repo_root_override,
        manifest_path_override,
    })
}

pub fn resolve_worker_paths(
    inputs: &WorkerPathInputs,
) -> Result<ResolvedWorkerPaths, WorkerDiagnostic> {
    let current_dir = absolutize(&inputs.current_dir, &inputs.current_dir);
    let repo_root = match inputs.repo_root_override.as_ref() {
        Some(path) => require_repo_root(&absolutize(path, &current_dir), "environment override")?,
        None => discover_repo_root(&current_dir)?,
    };
    let manifest_path = inputs
        .manifest_path_override
        .as_ref()
        .map(|path| absolutize(path, &current_dir))
        .unwrap_or_else(|| repo_root.join(".ait").join("agent-workers.json"));
    Ok(ResolvedWorkerPaths {
        repo_root,
        manifest_path,
    })
}

fn discover_repo_root(start: &Path) -> Result<PathBuf, WorkerDiagnostic> {
    let mut current = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    loop {
        if current.join(".ait").is_dir() {
            return Ok(current);
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    Err(WorkerDiagnostic::new(
        "repository_root_not_found",
        "No AIT repository root was found in the current path or its parents.",
        EXIT_INVALID_CONFIGURATION,
    )
    .with_detail("start_path", start.display().to_string()))
}

fn require_repo_root(path: &Path, source: &str) -> Result<PathBuf, WorkerDiagnostic> {
    if !path.is_dir() || !path.join(".ait").is_dir() {
        return Err(WorkerDiagnostic::new(
            "repository_root_invalid",
            format!("The repository root from {source} does not contain an AIT repository."),
            EXIT_INVALID_CONFIGURATION,
        )
        .with_detail("repo_root", path.display().to_string()));
    }
    Ok(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
}

fn absolutize(path: &Path, current_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn discovers_repo_root_from_nested_directory() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".ait")).expect("ait dir");
        let nested = temp.path().join("nested/work");
        fs::create_dir_all(&nested).expect("nested dir");

        let paths = resolve_worker_paths(&WorkerPathInputs {
            current_dir: nested,
            repo_root_override: None,
            manifest_path_override: None,
        })
        .expect("paths");

        assert_eq!(paths.repo_root, temp.path().canonicalize().unwrap());
        assert_eq!(
            paths.manifest_path,
            paths.repo_root.join(".ait/agent-workers.json")
        );
    }

    #[test]
    fn explicit_manifest_path_is_resolved_from_current_directory() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".ait")).expect("ait dir");

        let paths = resolve_worker_paths(&WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: Some(temp.path().to_path_buf()),
            manifest_path_override: Some(PathBuf::from("config/workers.json")),
        })
        .expect("paths");

        assert_eq!(paths.manifest_path, temp.path().join("config/workers.json"));
    }

    #[test]
    fn invalid_explicit_root_fails_closed() {
        let temp = tempdir().expect("tempdir");

        let error = resolve_worker_paths(&WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: Some(temp.path().to_path_buf()),
            manifest_path_override: None,
        })
        .expect_err("invalid root");

        assert_eq!(error.code, "repository_root_invalid");
    }
}
