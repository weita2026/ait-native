use std::path::Path;

use crate::external::lockfile::ExternalLockBindingSummary;
use crate::json_support::{json, JsonValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalBindingValidationMode {
    PathOnly,
    ToolchainProbes,
}

impl ExternalBindingValidationMode {
    pub fn toolchain_probes_enabled(self) -> bool {
        matches!(self, Self::ToolchainProbes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalBindingTool {
    Cargo,
    Python,
    Node,
    Go,
}

impl ExternalBindingTool {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Python => "python",
            Self::Node => "node",
            Self::Go => "go",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalBindingToolOutcome {
    NotRequested,
    Skipped { reason: String },
    Passed,
    Failed { message: String },
}

impl ExternalBindingToolOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotRequested => "not_requested",
            Self::Skipped { .. } => "skipped",
            Self::Passed => "passed",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Skipped { reason } => Some(reason.as_str()),
            Self::Failed { message } => Some(message.as_str()),
            Self::NotRequested | Self::Passed => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalBindingValidationRequest<'a> {
    pub repo_root: &'a Path,
    pub nodes: &'a [crate::external::lockfile::ExternalLockNode],
    pub mode: ExternalBindingValidationMode,
}

impl<'a> ExternalBindingValidationRequest<'a> {
    pub fn path_only(
        repo_root: &'a Path,
        nodes: &'a [crate::external::lockfile::ExternalLockNode],
    ) -> Self {
        Self {
            repo_root,
            nodes,
            mode: ExternalBindingValidationMode::PathOnly,
        }
    }

    pub fn toolchain_probes(
        repo_root: &'a Path,
        nodes: &'a [crate::external::lockfile::ExternalLockNode],
    ) -> Self {
        Self {
            repo_root,
            nodes,
            mode: ExternalBindingValidationMode::ToolchainProbes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalBindingCheckFact {
    pub name: String,
    pub parent_path: String,
    pub materialize_to: String,
    pub language: String,
    pub kind: String,
    pub path: String,
    pub full_path: String,
    pub exists: bool,
    pub supported: bool,
    pub tool: Option<ExternalBindingTool>,
    pub toolchain: ExternalBindingToolOutcome,
}

impl ExternalBindingCheckFact {
    pub fn new(
        name: impl Into<String>,
        parent_path: impl Into<String>,
        materialize_to: impl Into<String>,
        binding: &ExternalLockBindingSummary,
        full_path: impl Into<String>,
        exists: bool,
    ) -> Self {
        let language = binding.language.clone();
        let kind = binding.kind.clone();
        Self {
            name: name.into(),
            parent_path: parent_path.into(),
            materialize_to: materialize_to.into(),
            language: language.clone(),
            kind: kind.clone(),
            path: binding.path.clone(),
            full_path: full_path.into(),
            exists,
            supported: binding_kind_is_supported(&language, &kind),
            tool: binding_tool_for(&language, &kind),
            toolchain: ExternalBindingToolOutcome::NotRequested,
        }
    }

    pub fn with_toolchain(mut self, toolchain: ExternalBindingToolOutcome) -> Self {
        self.toolchain = toolchain;
        self
    }

    pub fn toolchain_skipped(&self) -> bool {
        matches!(self.toolchain, ExternalBindingToolOutcome::Skipped { .. })
    }

    pub fn toolchain_failed(&self) -> bool {
        matches!(self.toolchain, ExternalBindingToolOutcome::Failed { .. })
    }

    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "name": self.name,
            "parent_path": self.parent_path,
            "materialize_to": self.materialize_to,
            "language": self.language,
            "kind": self.kind,
            "path": self.path,
            "full_path": self.full_path,
            "exists": self.exists,
            "supported": self.supported,
            "tool": self.tool.map(ExternalBindingTool::as_str),
            "toolchain": {
                "status": self.toolchain.as_str(),
                "message": self.toolchain.message(),
            },
        })
    }
}

pub fn binding_kind_is_supported(language: &str, kind: &str) -> bool {
    binding_tool_for(language, kind).is_some()
}

pub fn binding_tool_for(language: &str, kind: &str) -> Option<ExternalBindingTool> {
    match (language, kind) {
        ("rust", "cargo-path") => Some(ExternalBindingTool::Cargo),
        ("python", "python-path") => Some(ExternalBindingTool::Python),
        ("node", "file-package") => Some(ExternalBindingTool::Node),
        ("go", "replace-path") => Some(ExternalBindingTool::Go),
        _ => None,
    }
}
