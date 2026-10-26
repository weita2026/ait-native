use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::external::bindings::model::{
    ExternalBindingCheckFact, ExternalBindingTool, ExternalBindingToolOutcome,
    ExternalBindingValidationMode, ExternalBindingValidationRequest,
};
use crate::external::doctor::ExternalDoctorFinding;
use crate::external::lockfile::{ExternalLockBindingSummary, ExternalLockNode};
use crate::external::{ExternalError, ExternalResult};

pub trait ExternalBindingValidator {
    fn validate_bindings(
        &self,
        request: ExternalBindingValidationRequest<'_>,
    ) -> ExternalResult<Vec<ExternalDoctorFinding>>;
}

pub trait ExternalBindingCheckProvider {
    fn check_bindings(
        &self,
        request: ExternalBindingValidationRequest<'_>,
    ) -> ExternalResult<Vec<ExternalBindingCheckFact>>;
}

pub trait ExternalBindingToolProbe {
    fn probe_binding_tool(
        &self,
        request: ExternalBindingToolProbeRequest<'_>,
    ) -> ExternalResult<ExternalBindingToolProbeResult>;
}

#[derive(Debug, Clone, Copy)]
pub struct ExternalBindingToolProbeRequest<'a> {
    pub tool: ExternalBindingTool,
    pub node: &'a ExternalLockNode,
    pub binding: &'a ExternalLockBindingSummary,
    pub binding_path: &'a Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalBindingToolProbeResult {
    pub outcome: ExternalBindingToolOutcome,
}

impl ExternalBindingToolProbeResult {
    pub fn skipped(reason: impl Into<String>) -> Self {
        Self {
            outcome: ExternalBindingToolOutcome::Skipped {
                reason: reason.into(),
            },
        }
    }

    pub fn passed() -> Self {
        Self {
            outcome: ExternalBindingToolOutcome::Passed,
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            outcome: ExternalBindingToolOutcome::Failed {
                message: message.into(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopExternalBindingToolProbe;

impl ExternalBindingToolProbe for NoopExternalBindingToolProbe {
    fn probe_binding_tool(
        &self,
        request: ExternalBindingToolProbeRequest<'_>,
    ) -> ExternalResult<ExternalBindingToolProbeResult> {
        Ok(ExternalBindingToolProbeResult::skipped(format!(
            "{} metadata validation probe is not configured",
            request.tool.as_str()
        )))
    }
}

#[derive(Debug, Clone)]
pub struct FilesystemExternalBindingValidator<P = NoopExternalBindingToolProbe> {
    tool_probe: P,
}

impl Default for FilesystemExternalBindingValidator<NoopExternalBindingToolProbe> {
    fn default() -> Self {
        Self::new(NoopExternalBindingToolProbe)
    }
}

impl<P> FilesystemExternalBindingValidator<P> {
    pub fn new(tool_probe: P) -> Self {
        Self { tool_probe }
    }
}

impl<P> ExternalBindingCheckProvider for FilesystemExternalBindingValidator<P>
where
    P: ExternalBindingToolProbe,
{
    fn check_bindings(
        &self,
        request: ExternalBindingValidationRequest<'_>,
    ) -> ExternalResult<Vec<ExternalBindingCheckFact>> {
        validate_binding_paths_with_probe(
            request.repo_root,
            request.nodes,
            request.mode,
            &self.tool_probe,
        )
    }
}

impl<P> ExternalBindingValidator for FilesystemExternalBindingValidator<P>
where
    P: ExternalBindingToolProbe,
{
    fn validate_bindings(
        &self,
        request: ExternalBindingValidationRequest<'_>,
    ) -> ExternalResult<Vec<ExternalDoctorFinding>> {
        let checks = self.check_bindings(request)?;
        Ok(doctor_findings_for_binding_checks(&checks))
    }
}

pub fn inspect_external_binding_paths(
    repo_root: &Path,
    nodes: &[ExternalLockNode],
) -> ExternalResult<Vec<ExternalBindingCheckFact>> {
    FilesystemExternalBindingValidator::default().check_bindings(
        ExternalBindingValidationRequest::path_only(repo_root, nodes),
    )
}

pub fn doctor_findings_for_binding_checks(
    checks: &[ExternalBindingCheckFact],
) -> Vec<ExternalDoctorFinding> {
    let mut findings = Vec::new();
    for check in checks {
        if !check.supported {
            findings.push(ExternalDoctorFinding::error(
                "external_binding_kind_unsupported",
                check.name.clone(),
                check.full_path.clone(),
                format!(
                    "declared {} binding kind {:?} is unsupported",
                    check.language, check.kind
                ),
            ));
        } else if !check.exists {
            findings.push(ExternalDoctorFinding::warning(
                "external_binding_path_missing",
                check.name.clone(),
                check.full_path.clone(),
                format!("declared {} binding path is missing", check.language),
            ));
        } else if check.toolchain_failed() {
            findings.push(ExternalDoctorFinding::error(
                "external_binding_toolchain_failed",
                check.name.clone(),
                check.full_path.clone(),
                check
                    .toolchain
                    .message()
                    .unwrap_or("declared binding toolchain validation failed"),
            ));
        } else if check.toolchain_skipped() {
            findings.push(ExternalDoctorFinding::warning(
                "external_binding_toolchain_skipped",
                check.name.clone(),
                check.full_path.clone(),
                check
                    .toolchain
                    .message()
                    .unwrap_or("declared binding toolchain validation was skipped"),
            ));
        }
    }
    findings
}

fn validate_binding_paths_with_probe<P>(
    repo_root: &Path,
    nodes: &[ExternalLockNode],
    mode: ExternalBindingValidationMode,
    probe: &P,
) -> ExternalResult<Vec<ExternalBindingCheckFact>>
where
    P: ExternalBindingToolProbe,
{
    let mut checks = Vec::new();
    for node in nodes {
        let node_root = safe_destination(repo_root, &node.materialize_to)?;
        for binding in &node.bindings {
            let full_path = binding_full_path(&node_root, &binding.path)?;
            let exists = match fs::symlink_metadata(&full_path) {
                Ok(_) => true,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                Err(err) => {
                    return Err(ExternalError::with_code(
                        "external_binding_stat",
                        format!(
                            "failed to inspect external binding path {:?}: {err}",
                            full_path
                        ),
                    ));
                }
            };
            let mut check = ExternalBindingCheckFact::new(
                node.name.clone(),
                node.parent_path.clone(),
                node.materialize_to.clone(),
                binding,
                repo_relative_display(repo_root, &full_path),
                exists,
            );
            if mode.toolchain_probes_enabled() && exists && check.supported {
                if let Some(tool) = check.tool {
                    let result = probe.probe_binding_tool(ExternalBindingToolProbeRequest {
                        tool,
                        node,
                        binding,
                        binding_path: &full_path,
                    })?;
                    check = check.with_toolchain(result.outcome);
                }
            }
            checks.push(check);
        }
    }
    Ok(checks)
}

fn binding_full_path(node_root: &Path, binding_path: &str) -> ExternalResult<PathBuf> {
    let relative = validate_repo_relative_path(binding_path, "external binding path")?;
    let mut full_path = node_root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            full_path.push(part);
        }
    }
    Ok(full_path)
}

fn safe_destination(repo_root: &Path, materialize_to: &str) -> ExternalResult<PathBuf> {
    let relative = validate_repo_relative_path(materialize_to, "materialize_to")?;
    let mut destination = repo_root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            destination.push(part);
        }
    }
    ensure_existing_ancestors_are_not_symlinks(repo_root, &destination, materialize_to)?;
    Ok(destination)
}

fn validate_repo_relative_path(path: &str, field: &str) -> ExternalResult<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return Err(ExternalError::with_code(
            "external_binding_path",
            format!("{field} must not be empty"),
        ));
    }
    let parsed = Path::new(path);
    if parsed.is_absolute() {
        return Err(ExternalError::with_code(
            "external_binding_path",
            format!("{field} must be repository-relative, got absolute path {path:?}"),
        ));
    }

    let mut normalized = PathBuf::new();
    let mut has_normal = false;
    for component in parsed.components() {
        match component {
            Component::Normal(part) => {
                has_normal = true;
                normalized.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ExternalError::with_code(
                    "external_binding_path",
                    format!("{field} must not escape the repository, got {path:?}"),
                ));
            }
        }
    }
    if !has_normal {
        return Err(ExternalError::with_code(
            "external_binding_path",
            format!("{field} must contain a repository-relative path component"),
        ));
    }
    Ok(normalized)
}

fn ensure_existing_ancestors_are_not_symlinks(
    repo_root: &Path,
    destination: &Path,
    display_path: &str,
) -> ExternalResult<()> {
    let relative = destination.strip_prefix(repo_root).map_err(|_| {
        ExternalError::with_code(
            "external_binding_path",
            format!("external materialization path {display_path:?} is outside the repository"),
        )
    })?;

    let mut cursor = repo_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        cursor.push(part);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ExternalError::with_code(
                    "external_binding_symlink",
                    format!(
                        "external materialization path {display_path:?} crosses symlink {:?}",
                        cursor
                    ),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(ExternalError::with_code(
                    "external_binding_stat",
                    format!(
                        "failed to inspect external materialization path {display_path:?}: {err}"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn repo_relative_display(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}
