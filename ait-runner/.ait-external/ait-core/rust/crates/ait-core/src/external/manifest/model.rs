use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::external::{ExternalError, ExternalResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalManifest {
    #[serde(rename = "external", default)]
    pub externals: Vec<ExternalDeclaration>,
}

impl ExternalManifest {
    pub fn validate(&self) -> ExternalResult<()> {
        for external in &self.externals {
            external.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalDeclaration {
    pub name: String,
    pub repo_name: String,
    pub repository_index: u32,
    pub remote: String,
    pub line: String,
    pub snapshot: String,
    pub materialize_to: String,
    pub license: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    #[serde(default, skip_serializing_if = "ExternalBindingSet::is_empty")]
    pub bindings: ExternalBindingSet,
}

impl ExternalDeclaration {
    fn validate(&self) -> ExternalResult<()> {
        require_non_empty(&self.name, "external.name")?;
        require_non_empty(
            &self.repo_name,
            external_field(&self.name, "repo_name").as_str(),
        )?;
        require_non_empty(&self.remote, external_field(&self.name, "remote").as_str())?;
        require_non_empty(&self.line, external_field(&self.name, "line").as_str())?;
        require_non_empty(
            &self.snapshot,
            external_field(&self.name, "snapshot").as_str(),
        )?;
        require_non_empty(
            &self.materialize_to,
            external_field(&self.name, "materialize_to").as_str(),
        )?;
        require_non_empty(
            &self.license,
            external_field(&self.name, "license").as_str(),
        )?;
        validate_repo_relative_path(
            &self.materialize_to,
            external_field(&self.name, "materialize_to").as_str(),
        )?;
        self.bindings.validate(&self.name)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalBindingSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust: Option<ExternalRustBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<ExternalPythonBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<ExternalNodeBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub go: Option<ExternalGoBinding>,
}

impl ExternalBindingSet {
    pub fn is_empty(&self) -> bool {
        self.rust.is_none() && self.python.is_none() && self.node.is_none() && self.go.is_none()
    }

    fn validate(&self, external_name: &str) -> ExternalResult<()> {
        if let Some(binding) = &self.rust {
            binding.validate(external_name)?;
        }
        if let Some(binding) = &self.python {
            binding.validate(external_name)?;
        }
        if let Some(binding) = &self.node {
            binding.validate(external_name)?;
        }
        if let Some(binding) = &self.go {
            binding.validate(external_name)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRustBinding {
    pub kind: String,
    pub path: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
}

impl ExternalRustBinding {
    fn validate(&self, external_name: &str) -> ExternalResult<()> {
        validate_binding_kind(external_name, "rust", &self.kind, "cargo-path")?;
        validate_repo_relative_path(
            &self.path,
            binding_field(external_name, "rust", "path").as_str(),
        )?;
        validate_optional_metadata(
            self.package.as_deref(),
            binding_field(external_name, "rust", "package").as_str(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalPythonBinding {
    pub kind: String,
    pub path: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

impl ExternalPythonBinding {
    fn validate(&self, external_name: &str) -> ExternalResult<()> {
        validate_binding_kind(external_name, "python", &self.kind, "python-path")?;
        validate_repo_relative_path(
            &self.path,
            binding_field(external_name, "python", "path").as_str(),
        )?;
        validate_optional_metadata(
            self.package.as_deref(),
            binding_field(external_name, "python", "package").as_str(),
        )?;
        validate_optional_metadata(
            self.module.as_deref(),
            binding_field(external_name, "python", "module").as_str(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalNodeBinding {
    pub kind: String,
    pub path: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
}

impl ExternalNodeBinding {
    fn validate(&self, external_name: &str) -> ExternalResult<()> {
        validate_binding_kind(external_name, "node", &self.kind, "file-package")?;
        validate_repo_relative_path(
            &self.path,
            binding_field(external_name, "node", "path").as_str(),
        )?;
        validate_optional_metadata(
            self.package.as_deref(),
            binding_field(external_name, "node", "package").as_str(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalGoBinding {
    pub kind: String,
    pub path: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

impl ExternalGoBinding {
    fn validate(&self, external_name: &str) -> ExternalResult<()> {
        validate_binding_kind(external_name, "go", &self.kind, "replace-path")?;
        validate_repo_relative_path(
            &self.path,
            binding_field(external_name, "go", "path").as_str(),
        )?;
        validate_optional_metadata(
            self.module.as_deref(),
            binding_field(external_name, "go", "module").as_str(),
        )
    }
}

fn require_non_empty(value: &str, field: &str) -> ExternalResult<()> {
    if value.trim().is_empty() {
        return Err(ExternalError::new(format!("{field} must not be empty")));
    }
    Ok(())
}

fn validate_optional_metadata(value: Option<&str>, field: &str) -> ExternalResult<()> {
    if let Some(value) = value {
        require_non_empty(value, field)?;
    }
    Ok(())
}

fn validate_binding_kind(
    external_name: &str,
    language: &str,
    actual: &str,
    expected: &str,
) -> ExternalResult<()> {
    if actual != expected {
        return Err(ExternalError::new(format!(
            "external {external_name:?} {language} binding kind must be {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}

fn validate_repo_relative_path(path: &str, field: &str) -> ExternalResult<()> {
    let path = path.trim();
    require_non_empty(path, field)?;
    let parsed = Path::new(path);
    if parsed.is_absolute() {
        return Err(ExternalError::new(format!(
            "{field} must be repository-relative, got absolute path {path:?}"
        )));
    }

    let mut has_normal = false;
    for component in parsed.components() {
        match component {
            Component::Normal(_) => has_normal = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ExternalError::new(format!(
                    "{field} must not escape the repository, got {path:?}"
                )));
            }
        }
    }

    if !has_normal {
        return Err(ExternalError::new(format!(
            "{field} must contain a repository-relative path component"
        )));
    }

    Ok(())
}

fn external_field(external_name: &str, field: &str) -> String {
    format!("external {external_name:?} {field}")
}

fn binding_field(external_name: &str, language: &str, field: &str) -> String {
    format!("external {external_name:?} {language} binding {field}")
}
