use std::collections::BTreeSet;

use crate::json_support::{json, JsonValue};

use crate::external::bindings::doctor_findings_for_binding_checks;
use crate::external::status::{ExternalStatusReport, ExternalStatusState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDoctorOptions {
    pub allowed_licenses: BTreeSet<String>,
}

impl ExternalDoctorOptions {
    pub fn permissive_consumer() -> Self {
        Self {
            allowed_licenses: ["Apache-2.0", "MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }
}

impl Default for ExternalDoctorOptions {
    fn default() -> Self {
        Self::permissive_consumer()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDoctorReport {
    pub command: String,
    pub repo_name: String,
    pub release_ready: bool,
    pub checked: ExternalDoctorChecked,
    pub findings: Vec<ExternalDoctorFinding>,
}

impl ExternalDoctorReport {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "command": self.command,
            "repo_name": self.repo_name,
            "release_ready": self.release_ready,
            "checked": self.checked.to_json_value(),
            "findings": self.findings.iter().map(ExternalDoctorFinding::to_json_value).collect::<Vec<_>>(),
            "summary": {
                "release_blocking": self.release_blocking_findings().len(),
                "warnings": self.warning_findings().len(),
                "errors": self.findings.iter().filter(|finding| finding.severity == ExternalDoctorSeverity::Error).count(),
            },
        })
    }

    pub fn release_blocking_findings(&self) -> Vec<&ExternalDoctorFinding> {
        self.findings
            .iter()
            .filter(|finding| finding.release_blocking)
            .collect()
    }

    pub fn warning_findings(&self) -> Vec<&ExternalDoctorFinding> {
        self.findings
            .iter()
            .filter(|finding| finding.severity == ExternalDoctorSeverity::Warning)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalDoctorChecked {
    pub manifest: bool,
    pub lockfile: bool,
    pub materialization: bool,
    pub bindings: bool,
    pub licenses: bool,
    pub current_source_core: bool,
}

impl Default for ExternalDoctorChecked {
    fn default() -> Self {
        Self {
            manifest: true,
            lockfile: true,
            materialization: true,
            bindings: true,
            licenses: true,
            current_source_core: false,
        }
    }
}

impl ExternalDoctorChecked {
    pub fn to_json_value(self) -> JsonValue {
        json!({
            "manifest": self.manifest,
            "lockfile": self.lockfile,
            "materialization": self.materialization,
            "bindings": self.bindings,
            "licenses": self.licenses,
            "current_source_core": self.current_source_core,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDoctorFinding {
    pub code: String,
    pub severity: ExternalDoctorSeverity,
    pub release_blocking: bool,
    pub name: Option<String>,
    pub path: Option<String>,
    pub message: String,
}

impl ExternalDoctorFinding {
    pub fn error(
        code: impl Into<String>,
        name: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: ExternalDoctorSeverity::Error,
            release_blocking: true,
            name: Some(name.into()),
            path: Some(path.into()),
            message: message.into(),
        }
    }

    pub fn warning(
        code: impl Into<String>,
        name: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: ExternalDoctorSeverity::Warning,
            release_blocking: false,
            name: Some(name.into()),
            path: Some(path.into()),
            message: message.into(),
        }
    }

    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "code": self.code,
            "severity": self.severity.as_str(),
            "release_blocking": self.release_blocking,
            "name": self.name,
            "path": self.path,
            "message": self.message,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalDoctorSeverity {
    Error,
    Warning,
}

impl ExternalDoctorSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

pub fn build_external_doctor_report(
    status: &ExternalStatusReport,
    options: &ExternalDoctorOptions,
) -> ExternalDoctorReport {
    let mut findings = Vec::new();
    for drift in &status.lock_drifts {
        findings.push(ExternalDoctorFinding::error(
            "external_lock_drift",
            drift.name.clone(),
            drift.parent_path.clone(),
            drift.message.clone(),
        ));
    }
    for entry in &status.externals {
        match entry.state {
            ExternalStatusState::Missing => findings.push(ExternalDoctorFinding::error(
                "external_materialization_missing",
                entry.name.clone(),
                entry.materialize_to.clone(),
                "external materialization is missing",
            )),
            ExternalStatusState::Linked => findings.push(ExternalDoctorFinding::error(
                "external_local_link_active",
                entry.name.clone(),
                entry.materialize_to.clone(),
                "local link override is active",
            )),
            ExternalStatusState::Dirty => findings.push(ExternalDoctorFinding::error(
                "external_materialization_dirty",
                entry.name.clone(),
                entry.materialize_to.clone(),
                "external materialization is dirty or not generated by AIT",
            )),
            ExternalStatusState::Outdated => findings.push(ExternalDoctorFinding::error(
                "external_materialization_outdated",
                entry.name.clone(),
                entry.materialize_to.clone(),
                "external materialization does not match the lockfile snapshot",
            )),
            ExternalStatusState::Materialized => {}
        }
        if !options.allowed_licenses.contains(&entry.license) {
            findings.push(ExternalDoctorFinding::error(
                "external_license_boundary",
                entry.name.clone(),
                entry.materialize_to.clone(),
                format!(
                    "declared license {:?} is not allowed for this consumer boundary",
                    entry.license
                ),
            ));
        }
    }
    for duplicate in &status.duplicates {
        findings.push(ExternalDoctorFinding::warning(
            "external_duplicate_name",
            duplicate.name.clone(),
            status.lockfile_path.clone(),
            format!(
                "external name {:?} appears in {} resolved lockfile entries; default policy keeps each parent path isolated",
                duplicate.name,
                duplicate.entries.len()
            ),
        ));
    }
    findings.extend(doctor_findings_for_binding_checks(&status.binding_checks));
    if let Some(current_source_core) = &status.current_source_core {
        for artifact in current_source_core.blocking_artifacts() {
            findings.push(ExternalDoctorFinding::error(
                "external_current_source_core_artifact",
                artifact.name.clone(),
                artifact
                    .path
                    .clone()
                    .unwrap_or_else(|| current_source_core.metadata_path.clone()),
                artifact
                    .reason
                    .clone()
                    .unwrap_or_else(|| "current-source core artifact is not ready".to_string()),
            ));
        }
    }

    let release_ready = !findings.iter().any(|finding| finding.release_blocking);
    ExternalDoctorReport {
        command: "external doctor".to_string(),
        repo_name: status.repo_name.clone(),
        release_ready,
        checked: ExternalDoctorChecked {
            current_source_core: status.current_source_core.is_some(),
            ..ExternalDoctorChecked::default()
        },
        findings,
    }
}
