use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use crate::json_support::{json, JsonValue};
use serde::{Deserialize, Serialize};

use crate::external::manifest::{ExternalBindingSet, ExternalDeclaration, ExternalManifest};
use crate::external::{ExternalError, ExternalResult};

pub const EXTERNAL_LOCKFILE_FORMAT: &str = "ait.external.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalLockfile {
    pub format: String,

    #[serde(rename = "node", default)]
    pub nodes: Vec<ExternalLockNode>,
}

impl ExternalLockfile {
    pub fn new(nodes: Vec<ExternalLockNode>) -> Self {
        Self {
            format: EXTERNAL_LOCKFILE_FORMAT.to_string(),
            nodes,
        }
    }

    pub fn validate(&self) -> ExternalResult<()> {
        if self.format != EXTERNAL_LOCKFILE_FORMAT {
            return Err(ExternalError::with_code(
                "external_lock_format",
                format!(
                    "ait-external.lock format must be {:?}, got {:?}",
                    EXTERNAL_LOCKFILE_FORMAT, self.format
                ),
            ));
        }

        let mut identities = BTreeSet::new();
        for node in &self.nodes {
            node.validate()?;
            let identity = node.identity_key();
            if !identities.insert(identity.clone()) {
                return Err(ExternalError::with_code(
                    "external_lock_duplicate_node",
                    format!("ait-external.lock contains duplicate node {identity:?}"),
                ));
            }
        }

        Ok(())
    }

    pub fn sorted_nodes(&self) -> Vec<ExternalLockNode> {
        let mut nodes = self.nodes.clone();
        nodes.sort_by_key(|left| left.sort_key());
        nodes
    }

    pub fn normalized(&self) -> Self {
        Self {
            format: self.format.clone(),
            nodes: self.sorted_nodes(),
        }
    }

    pub fn direct_manifest_lock(manifest: &ExternalManifest) -> ExternalResult<Self> {
        let mut nodes = Vec::new();
        for declaration in &manifest.externals {
            nodes.push(ExternalLockNode::from_direct_declaration(declaration));
        }
        let lockfile = Self::new(nodes).normalized();
        lockfile.validate()?;
        Ok(lockfile)
    }

    pub fn locked_drift_against_manifest(
        &self,
        manifest: &ExternalManifest,
    ) -> Vec<ExternalLockDrift> {
        let manifest_nodes = manifest
            .externals
            .iter()
            .map(|declaration| {
                (
                    declaration.name.as_str(),
                    ExternalLockNode::from_direct_declaration(declaration),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let root_lock_nodes = self
            .nodes
            .iter()
            .filter(|node| node.parent_path.is_empty())
            .map(|node| (node.name.as_str(), node))
            .collect::<BTreeMap<_, _>>();

        let mut drifts = Vec::new();
        for (name, manifest_node) in &manifest_nodes {
            match root_lock_nodes.get(name) {
                Some(lock_node) => {
                    drifts.extend(compare_node_fields(manifest_node, lock_node));
                }
                None => drifts.push(ExternalLockDrift::missing(manifest_node)),
            }
        }
        for (name, lock_node) in &root_lock_nodes {
            if !manifest_nodes.contains_key(name) {
                drifts.push(ExternalLockDrift::extra(lock_node));
            }
        }
        drifts
    }

    pub fn is_locked_against_manifest(&self, manifest: &ExternalManifest) -> bool {
        self.locked_drift_against_manifest(manifest).is_empty()
    }

    pub fn to_json_value(&self) -> JsonValue {
        let nodes = self
            .sorted_nodes()
            .iter()
            .map(ExternalLockNode::to_json_value)
            .collect::<Vec<_>>();
        let root_count = self
            .nodes
            .iter()
            .filter(|node| node.parent_path.is_empty())
            .count();
        json!({
            "format": self.format,
            "nodes": nodes,
            "summary": {
                "node_count": self.nodes.len(),
                "root_count": root_count,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalLockNode {
    pub name: String,
    pub repo_name: String,
    pub repository_index: u32,
    pub remote: String,
    pub line: String,
    pub snapshot: String,
    pub parent_path: String,
    pub materialize_to: String,
    pub license: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    #[serde(rename = "binding", default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<ExternalLockBindingSummary>,
}

impl ExternalLockNode {
    pub fn from_direct_declaration(declaration: &ExternalDeclaration) -> Self {
        Self {
            name: declaration.name.clone(),
            repo_name: declaration.repo_name.clone(),
            repository_index: declaration.repository_index,
            remote: declaration.remote.clone(),
            line: declaration.line.clone(),
            snapshot: declaration.snapshot.clone(),
            parent_path: String::new(),
            materialize_to: declaration.materialize_to.clone(),
            license: declaration.license.clone(),
            version: declaration.version.clone(),
            bindings: binding_summaries(&declaration.bindings),
        }
    }

    fn validate(&self) -> ExternalResult<()> {
        require_non_empty(&self.name, "lock node name")?;
        require_non_empty(
            &self.repo_name,
            lock_node_field(&self.name, "repo_name").as_str(),
        )?;
        require_non_empty(&self.remote, lock_node_field(&self.name, "remote").as_str())?;
        require_non_empty(&self.line, lock_node_field(&self.name, "line").as_str())?;
        require_non_empty(
            &self.snapshot,
            lock_node_field(&self.name, "snapshot").as_str(),
        )?;
        require_non_empty(
            &self.materialize_to,
            lock_node_field(&self.name, "materialize_to").as_str(),
        )?;
        require_non_empty(
            &self.license,
            lock_node_field(&self.name, "license").as_str(),
        )?;
        if !self.parent_path.is_empty() {
            validate_repo_relative_path(
                &self.parent_path,
                lock_node_field(&self.name, "parent_path").as_str(),
            )?;
        }
        validate_repo_relative_path(
            &self.materialize_to,
            lock_node_field(&self.name, "materialize_to").as_str(),
        )?;
        for binding in &self.bindings {
            binding.validate(&self.name)?;
        }
        Ok(())
    }

    fn identity_key(&self) -> String {
        format!("{}|{}", self.parent_path, self.name)
    }

    fn sort_key(&self) -> (String, String, String, u32, String, String) {
        (
            self.parent_path.clone(),
            self.materialize_to.clone(),
            self.name.clone(),
            self.repository_index,
            self.repo_name.clone(),
            self.snapshot.clone(),
        )
    }

    pub fn to_json_value(&self) -> JsonValue {
        let bindings = self
            .bindings
            .iter()
            .map(ExternalLockBindingSummary::to_json_value)
            .collect::<Vec<_>>();
        json!({
            "name": self.name,
            "repo_name": self.repo_name,
            "repository_index": self.repository_index,
            "remote": self.remote,
            "line": self.line,
            "snapshot": self.snapshot,
            "parent_path": self.parent_path,
            "materialize_to": self.materialize_to,
            "license": self.license,
            "version": self.version,
            "bindings": bindings,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalLockBindingSummary {
    pub language: String,
    pub kind: String,
    pub path: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

impl ExternalLockBindingSummary {
    pub fn new(
        language: impl Into<String>,
        kind: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            language: language.into(),
            kind: kind.into(),
            path: path.into(),
            package: None,
            module: None,
        }
    }

    pub fn with_package(mut self, package: Option<String>) -> Self {
        self.package = package;
        self
    }

    pub fn with_module(mut self, module: Option<String>) -> Self {
        self.module = module;
        self
    }

    fn validate(&self, node_name: &str) -> ExternalResult<()> {
        require_non_empty(
            &self.language,
            lock_binding_field(node_name, "language").as_str(),
        )?;
        require_non_empty(&self.kind, lock_binding_field(node_name, "kind").as_str())?;
        require_non_empty(&self.path, lock_binding_field(node_name, "path").as_str())?;
        match self.language.as_str() {
            "rust" | "python" | "node" | "go" => {}
            actual => {
                return Err(ExternalError::with_code(
                    "external_lock_binding_language",
                    format!(
                        "lock node {node_name:?} binding language must be one of rust, python, node, or go, got {actual:?}"
                    ),
                ));
            }
        }
        validate_repo_relative_path(&self.path, lock_binding_field(node_name, "path").as_str())?;
        validate_optional_metadata(
            self.package.as_deref(),
            lock_binding_field(node_name, "package").as_str(),
        )?;
        validate_optional_metadata(
            self.module.as_deref(),
            lock_binding_field(node_name, "module").as_str(),
        )
    }

    pub fn to_json_value(&self) -> JsonValue {
        let mut payload = json!({
            "language": self.language,
            "kind": self.kind,
            "path": self.path,
        });
        if let Some(object) = payload.as_object_mut() {
            if let Some(package) = &self.package {
                object.insert("package".to_string(), json!(package));
            }
            if let Some(module) = &self.module {
                object.insert("module".to_string(), json!(module));
            }
        }
        payload
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalLockDriftKind {
    Missing,
    Extra,
    Mismatch,
}

impl ExternalLockDriftKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Extra => "extra",
            Self::Mismatch => "mismatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLockDrift {
    pub kind: ExternalLockDriftKind,
    pub name: String,
    pub parent_path: String,
    pub field: Option<String>,
    pub manifest_value: Option<String>,
    pub lock_value: Option<String>,
    pub message: String,
}

impl ExternalLockDrift {
    fn missing(node: &ExternalLockNode) -> Self {
        Self {
            kind: ExternalLockDriftKind::Missing,
            name: node.name.clone(),
            parent_path: node.parent_path.clone(),
            field: None,
            manifest_value: Some(node.snapshot.clone()),
            lock_value: None,
            message: format!(
                "ait-external.lock is missing direct external {:?}",
                node.name
            ),
        }
    }

    fn extra(node: &ExternalLockNode) -> Self {
        Self {
            kind: ExternalLockDriftKind::Extra,
            name: node.name.clone(),
            parent_path: node.parent_path.clone(),
            field: None,
            manifest_value: None,
            lock_value: Some(node.snapshot.clone()),
            message: format!(
                "ait-external.lock contains extra direct external {:?}",
                node.name
            ),
        }
    }

    fn mismatch(
        node: &ExternalLockNode,
        field: &str,
        manifest_value: Option<String>,
        lock_value: Option<String>,
    ) -> Self {
        Self {
            kind: ExternalLockDriftKind::Mismatch,
            name: node.name.clone(),
            parent_path: node.parent_path.clone(),
            field: Some(field.to_string()),
            manifest_value,
            lock_value,
            message: format!(
                "ait-external.lock direct external {:?} field {field} does not match the manifest",
                node.name
            ),
        }
    }

    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "kind": self.kind.as_str(),
            "name": self.name,
            "parent_path": self.parent_path,
            "field": self.field,
            "manifest_value": self.manifest_value,
            "lock_value": self.lock_value,
            "message": self.message,
        })
    }
}

fn binding_summaries(bindings: &ExternalBindingSet) -> Vec<ExternalLockBindingSummary> {
    let mut summaries = Vec::new();
    if let Some(binding) = &bindings.rust {
        summaries.push(
            ExternalLockBindingSummary::new("rust", binding.kind.clone(), binding.path.clone())
                .with_package(binding.package.clone()),
        );
    }
    if let Some(binding) = &bindings.python {
        summaries.push(
            ExternalLockBindingSummary::new("python", binding.kind.clone(), binding.path.clone())
                .with_package(binding.package.clone())
                .with_module(binding.module.clone()),
        );
    }
    if let Some(binding) = &bindings.node {
        summaries.push(
            ExternalLockBindingSummary::new("node", binding.kind.clone(), binding.path.clone())
                .with_package(binding.package.clone()),
        );
    }
    if let Some(binding) = &bindings.go {
        summaries.push(
            ExternalLockBindingSummary::new("go", binding.kind.clone(), binding.path.clone())
                .with_module(binding.module.clone()),
        );
    }
    summaries.sort_by(|left, right| {
        (left.language.as_str(), left.path.as_str())
            .cmp(&(right.language.as_str(), right.path.as_str()))
    });
    summaries
}

fn compare_node_fields(
    manifest_node: &ExternalLockNode,
    lock_node: &ExternalLockNode,
) -> Vec<ExternalLockDrift> {
    let mut drifts = Vec::new();
    compare_field(
        &mut drifts,
        manifest_node,
        lock_node,
        "repository_index",
        Some(manifest_node.repository_index.to_string()),
        Some(lock_node.repository_index.to_string()),
    );
    compare_field(
        &mut drifts,
        manifest_node,
        lock_node,
        "repo_name",
        Some(manifest_node.repo_name.clone()),
        Some(lock_node.repo_name.clone()),
    );
    compare_field(
        &mut drifts,
        manifest_node,
        lock_node,
        "remote",
        Some(manifest_node.remote.clone()),
        Some(lock_node.remote.clone()),
    );
    compare_field(
        &mut drifts,
        manifest_node,
        lock_node,
        "line",
        Some(manifest_node.line.clone()),
        Some(lock_node.line.clone()),
    );
    compare_field(
        &mut drifts,
        manifest_node,
        lock_node,
        "snapshot",
        Some(manifest_node.snapshot.clone()),
        Some(lock_node.snapshot.clone()),
    );
    compare_field(
        &mut drifts,
        manifest_node,
        lock_node,
        "materialize_to",
        Some(manifest_node.materialize_to.clone()),
        Some(lock_node.materialize_to.clone()),
    );
    compare_field(
        &mut drifts,
        manifest_node,
        lock_node,
        "license",
        Some(manifest_node.license.clone()),
        Some(lock_node.license.clone()),
    );
    compare_field(
        &mut drifts,
        manifest_node,
        lock_node,
        "version",
        manifest_node.version.clone(),
        lock_node.version.clone(),
    );
    compare_field(
        &mut drifts,
        manifest_node,
        lock_node,
        "bindings",
        Some(binding_key(&manifest_node.bindings)),
        Some(binding_key(&lock_node.bindings)),
    );
    drifts
}

fn compare_field(
    drifts: &mut Vec<ExternalLockDrift>,
    manifest_node: &ExternalLockNode,
    lock_node: &ExternalLockNode,
    field: &str,
    manifest_value: Option<String>,
    lock_value: Option<String>,
) {
    if manifest_value != lock_value {
        drifts.push(ExternalLockDrift::mismatch(
            manifest_node,
            field,
            manifest_value,
            lock_value,
        ));
    } else {
        let _ = lock_node;
    }
}

fn binding_key(bindings: &[ExternalLockBindingSummary]) -> String {
    bindings
        .iter()
        .map(|binding| {
            format!(
                "{}:{}:{}:{}:{}",
                binding.language,
                binding.kind,
                binding.path,
                binding.package.as_deref().unwrap_or(""),
                binding.module.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("|")
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

fn lock_node_field(node_name: &str, field: &str) -> String {
    format!("lock node {node_name:?} {field}")
}

fn lock_binding_field(node_name: &str, field: &str) -> String {
    format!("lock node {node_name:?} binding {field}")
}
