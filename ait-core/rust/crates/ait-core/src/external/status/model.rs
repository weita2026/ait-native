use std::collections::{BTreeMap, BTreeSet};

use crate::json_support::{json, JsonValue};

use crate::external::bindings::ExternalBindingCheckFact;
use crate::external::lockfile::{
    ExternalLockBindingSummary, ExternalLockDrift, ExternalLockDriftKind, ExternalLockNode,
    ExternalLockfile,
};
use crate::external::manifest::ExternalManifest;
use crate::external::materializer::ExternalLocalLinkOverride;
use crate::external::{ExternalError, ExternalResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalStatusInput {
    pub repo_name: String,
    pub manifest_path: String,
    pub lockfile_path: String,
    pub manifest: ExternalManifest,
    pub lockfile: Option<ExternalLockfile>,
    pub local_links: Vec<ExternalLocalLinkOverride>,
    pub materializations: Vec<ExternalMaterializationObservation>,
    pub binding_checks: Vec<ExternalBindingCheckFact>,
    pub current_source_core: Option<ExternalCurrentSourceCoreStatus>,
}

impl ExternalStatusInput {
    pub fn new(
        repo_name: impl Into<String>,
        manifest: ExternalManifest,
        lockfile: Option<ExternalLockfile>,
    ) -> Self {
        Self {
            repo_name: repo_name.into(),
            manifest_path: "ait-external.toml".to_string(),
            lockfile_path: "ait-external.lock".to_string(),
            manifest,
            lockfile,
            local_links: Vec::new(),
            materializations: Vec::new(),
            binding_checks: Vec::new(),
            current_source_core: None,
        }
    }

    pub fn with_local_link(mut self, name: impl Into<String>, path: impl Into<String>) -> Self {
        self.local_links.push(ExternalLocalLinkOverride {
            name: name.into(),
            path: path.into(),
        });
        self
    }

    pub fn with_materialization(mut self, fact: ExternalMaterializationObservation) -> Self {
        self.materializations.push(fact);
        self
    }

    pub fn with_binding_check(mut self, fact: ExternalBindingCheckFact) -> Self {
        self.binding_checks.push(fact);
        self
    }

    pub fn with_current_source_core(mut self, fact: ExternalCurrentSourceCoreStatus) -> Self {
        self.current_source_core = Some(fact);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalMaterializationObservation {
    pub name: String,
    pub parent_path: String,
    pub materialize_to: String,
    pub state: ExternalObservedMaterializationState,
    pub snapshot: Option<String>,
    pub reason: Option<String>,
}

impl ExternalMaterializationObservation {
    pub fn missing(node: &ExternalLockNode) -> Self {
        Self {
            name: node.name.clone(),
            parent_path: node.parent_path.clone(),
            materialize_to: node.materialize_to.clone(),
            state: ExternalObservedMaterializationState::Missing,
            snapshot: None,
            reason: None,
        }
    }

    pub fn generated(
        name: impl Into<String>,
        parent_path: impl Into<String>,
        materialize_to: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            parent_path: parent_path.into(),
            materialize_to: materialize_to.into(),
            state: ExternalObservedMaterializationState::Generated,
            snapshot: Some(snapshot.into()),
            reason: None,
        }
    }

    pub fn dirty(
        name: impl Into<String>,
        parent_path: impl Into<String>,
        materialize_to: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            parent_path: parent_path.into(),
            materialize_to: materialize_to.into(),
            state: ExternalObservedMaterializationState::Dirty,
            snapshot: None,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalObservedMaterializationState {
    Missing,
    Generated,
    Dirty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalStatusReport {
    pub command: String,
    pub repo_name: String,
    pub manifest_path: String,
    pub lockfile_path: String,
    pub externals: Vec<ExternalStatusEntry>,
    pub duplicates: Vec<ExternalDuplicateGroup>,
    pub summary: ExternalStatusSummary,
    pub lock_drifts: Vec<ExternalLockDrift>,
    pub binding_checks: Vec<ExternalBindingCheckFact>,
    pub current_source_core: Option<ExternalCurrentSourceCoreStatus>,
}

impl ExternalStatusReport {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "command": self.command,
            "repo_name": self.repo_name,
            "manifest_path": self.manifest_path,
            "lockfile_path": self.lockfile_path,
            "externals": self.externals.iter().map(ExternalStatusEntry::to_json_value).collect::<Vec<_>>(),
            "duplicates": self.duplicates.iter().map(ExternalDuplicateGroup::to_json_value).collect::<Vec<_>>(),
            "lock_drifts": self.lock_drifts.iter().map(ExternalLockDrift::to_json_value).collect::<Vec<_>>(),
            "binding_checks": self.binding_checks.iter().map(ExternalBindingCheckFact::to_json_value).collect::<Vec<_>>(),
            "current_source_core": self.current_source_core.as_ref().map(ExternalCurrentSourceCoreStatus::to_json_value),
            "summary": self.summary.to_json_value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCurrentSourceCoreStatus {
    pub repo_root: String,
    pub metadata_path: String,
    pub metadata_present: bool,
    pub core_repo_root: Option<String>,
    pub core_source_fingerprint: Option<String>,
    pub core_source_mtime_ns: Option<u64>,
    pub active_binary_path: Option<String>,
    pub active_binary_role: ExternalCurrentSourceArtifactRole,
    pub artifacts: Vec<ExternalCurrentSourceArtifactStatus>,
}

impl ExternalCurrentSourceCoreStatus {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "repo_root": self.repo_root,
            "metadata_path": self.metadata_path,
            "metadata_present": self.metadata_present,
            "core_repo_root": self.core_repo_root,
            "core_source_fingerprint": self.core_source_fingerprint,
            "core_source_mtime_ns": self.core_source_mtime_ns,
            "active_binary_path": self.active_binary_path,
            "active_binary_role": self.active_binary_role.as_str(),
            "artifacts": self.artifacts.iter().map(ExternalCurrentSourceArtifactStatus::to_json_value).collect::<Vec<_>>(),
            "summary": {
                "checked": self.artifacts.len(),
                "ready": self.artifacts.iter().filter(|artifact| artifact.state == ExternalCurrentSourceArtifactState::Ready).count(),
                "missing": self.artifacts.iter().filter(|artifact| artifact.state == ExternalCurrentSourceArtifactState::Missing).count(),
                "stale": self.artifacts.iter().filter(|artifact| artifact.state == ExternalCurrentSourceArtifactState::Stale).count(),
                "wrong_binary": self.artifacts.iter().filter(|artifact| artifact.state == ExternalCurrentSourceArtifactState::WrongBinary).count(),
            },
        })
    }

    pub fn blocking_artifacts(&self) -> Vec<&ExternalCurrentSourceArtifactStatus> {
        self.artifacts
            .iter()
            .filter(|artifact| artifact.state != ExternalCurrentSourceArtifactState::Ready)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCurrentSourceArtifactStatus {
    pub name: String,
    pub role: ExternalCurrentSourceArtifactRole,
    pub path: Option<String>,
    pub state: ExternalCurrentSourceArtifactState,
    pub reason: Option<String>,
    pub expected_profile: Option<String>,
    pub metadata_sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub metadata_mtime_ns: Option<u64>,
    pub actual_mtime_ns: Option<u64>,
}

impl ExternalCurrentSourceArtifactStatus {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "name": self.name,
            "role": self.role.as_str(),
            "path": self.path,
            "state": self.state.as_str(),
            "reason": self.reason,
            "expected_profile": self.expected_profile,
            "metadata_sha256": self.metadata_sha256,
            "actual_sha256": self.actual_sha256,
            "metadata_mtime_ns": self.metadata_mtime_ns,
            "actual_mtime_ns": self.actual_mtime_ns,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalCurrentSourceArtifactRole {
    ActiveBinary,
    CanonicalBinary,
    PythonExtension,
    Metadata,
    Unknown,
}

impl ExternalCurrentSourceArtifactRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ActiveBinary => "active_binary",
            Self::CanonicalBinary => "canonical_binary",
            Self::PythonExtension => "python_extension",
            Self::Metadata => "metadata",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalCurrentSourceArtifactState {
    Ready,
    Missing,
    Stale,
    WrongBinary,
}

impl ExternalCurrentSourceArtifactState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::Stale => "stale",
            Self::WrongBinary => "wrong_binary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDuplicateGroup {
    pub name: String,
    pub policy: ExternalDuplicatePolicy,
    pub entries: Vec<ExternalDuplicateEntry>,
}

impl ExternalDuplicateGroup {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "name": self.name,
            "policy": self.policy.as_str(),
            "entries": self.entries.iter().map(ExternalDuplicateEntry::to_json_value).collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDuplicateEntry {
    pub parent_path: String,
    pub materialize_to: String,
    pub repo_name: String,
    pub repository_index: u32,
    pub snapshot: String,
}

impl ExternalDuplicateEntry {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "parent_path": self.parent_path,
            "materialize_to": self.materialize_to,
            "repo_name": self.repo_name,
            "repository_index": self.repository_index,
            "snapshot": self.snapshot,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalDuplicatePolicy {
    Allow,
}

impl ExternalDuplicatePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalStatusEntry {
    pub name: String,
    pub repo_name: String,
    pub repository_index: u32,
    pub snapshot: String,
    pub parent_path: String,
    pub materialize_to: String,
    pub state: ExternalStatusState,
    pub linked: bool,
    pub dirty: bool,
    pub outdated: bool,
    pub lock_drift: bool,
    pub link_path: Option<String>,
    pub license: String,
    pub bindings: Vec<ExternalLockBindingSummary>,
}

impl ExternalStatusEntry {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "name": self.name,
            "repo_name": self.repo_name,
            "repository_index": self.repository_index,
            "snapshot": self.snapshot,
            "parent_path": self.parent_path,
            "materialize_to": self.materialize_to,
            "state": self.state.as_str(),
            "linked": self.linked,
            "dirty": self.dirty,
            "outdated": self.outdated,
            "lock_drift": self.lock_drift,
            "link_path": self.link_path,
            "license": self.license,
            "bindings": self.bindings.iter().map(ExternalLockBindingSummary::to_json_value).collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalStatusState {
    Materialized,
    Missing,
    Linked,
    Dirty,
    Outdated,
}

impl ExternalStatusState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Materialized => "materialized",
            Self::Missing => "missing",
            Self::Linked => "linked",
            Self::Dirty => "dirty",
            Self::Outdated => "outdated",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalStatusSummary {
    pub missing: usize,
    pub linked: usize,
    pub dirty: usize,
    pub outdated: usize,
    pub lock_drift: usize,
    pub duplicate_names: usize,
}

impl ExternalStatusSummary {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "missing": self.missing,
            "linked": self.linked,
            "dirty": self.dirty,
            "outdated": self.outdated,
            "lock_drift": self.lock_drift,
            "duplicate_names": self.duplicate_names,
        })
    }
}

pub fn build_external_status_report(
    input: ExternalStatusInput,
) -> ExternalResult<ExternalStatusReport> {
    input.manifest.validate()?;
    if let Some(lockfile) = &input.lockfile {
        lockfile.validate()?;
    }
    let nodes = status_nodes(&input.manifest, input.lockfile.as_ref())?;
    let lock_drifts = input
        .lockfile
        .as_ref()
        .map(|lockfile| lockfile.locked_drift_against_manifest(&input.manifest))
        .unwrap_or_else(|| {
            nodes
                .iter()
                .filter(|node| node.parent_path.is_empty())
                .map(missing_lock_drift)
                .collect::<Vec<_>>()
        });
    let drift_keys = lock_drifts
        .iter()
        .map(|drift| (drift.parent_path.clone(), drift.name.clone()))
        .collect::<BTreeSet<_>>();
    let links = input
        .local_links
        .iter()
        .map(|link| (link.name.as_str(), link.path.as_str()))
        .collect::<BTreeMap<_, _>>();
    let observations = input
        .materializations
        .iter()
        .map(|fact| {
            (
                (
                    fact.parent_path.as_str(),
                    fact.name.as_str(),
                    fact.materialize_to.as_str(),
                ),
                fact,
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut entries = Vec::new();
    for node in nodes {
        let link_path = links
            .get(node.name.as_str())
            .map(|value| (*value).to_string());
        let observation = observations
            .get(&(
                node.parent_path.as_str(),
                node.name.as_str(),
                node.materialize_to.as_str(),
            ))
            .copied();
        let linked = link_path.is_some();
        let dirty = !linked
            && observation
                .map(|fact| fact.state == ExternalObservedMaterializationState::Dirty)
                .unwrap_or(false);
        let missing = !linked
            && observation
                .map(|fact| fact.state == ExternalObservedMaterializationState::Missing)
                .unwrap_or(true);
        let outdated = !linked
            && !dirty
            && !missing
            && observation
                .and_then(|fact| fact.snapshot.as_ref())
                .map(|snapshot| snapshot != &node.snapshot)
                .unwrap_or(false);
        let state = if linked {
            ExternalStatusState::Linked
        } else if dirty {
            ExternalStatusState::Dirty
        } else if missing {
            ExternalStatusState::Missing
        } else if outdated {
            ExternalStatusState::Outdated
        } else {
            ExternalStatusState::Materialized
        };
        entries.push(ExternalStatusEntry {
            name: node.name,
            repo_name: node.repo_name,
            repository_index: node.repository_index,
            snapshot: node.snapshot,
            parent_path: node.parent_path,
            materialize_to: node.materialize_to,
            state,
            linked,
            dirty,
            outdated,
            lock_drift: false,
            link_path,
            license: node.license,
            bindings: node.bindings,
        });
    }

    for entry in &mut entries {
        entry.lock_drift = drift_keys.contains(&(entry.parent_path.clone(), entry.name.clone()));
    }

    let duplicates = duplicate_groups(&entries);
    let summary = ExternalStatusSummary {
        missing: entries
            .iter()
            .filter(|entry| entry.state == ExternalStatusState::Missing)
            .count(),
        linked: entries.iter().filter(|entry| entry.linked).count(),
        dirty: entries.iter().filter(|entry| entry.dirty).count(),
        outdated: entries.iter().filter(|entry| entry.outdated).count(),
        lock_drift: entries.iter().filter(|entry| entry.lock_drift).count(),
        duplicate_names: duplicates.len(),
    };

    Ok(ExternalStatusReport {
        command: "external status".to_string(),
        repo_name: input.repo_name,
        manifest_path: input.manifest_path,
        lockfile_path: input.lockfile_path,
        externals: entries,
        duplicates,
        summary,
        lock_drifts,
        binding_checks: input.binding_checks,
        current_source_core: input.current_source_core,
    })
}

fn duplicate_groups(entries: &[ExternalStatusEntry]) -> Vec<ExternalDuplicateGroup> {
    let mut by_name: BTreeMap<&str, Vec<&ExternalStatusEntry>> = BTreeMap::new();
    for entry in entries {
        by_name.entry(entry.name.as_str()).or_default().push(entry);
    }
    by_name
        .into_iter()
        .filter_map(|(name, entries)| {
            if entries.len() < 2 {
                return None;
            }
            Some(ExternalDuplicateGroup {
                name: name.to_string(),
                policy: ExternalDuplicatePolicy::Allow,
                entries: entries
                    .into_iter()
                    .map(|entry| ExternalDuplicateEntry {
                        parent_path: entry.parent_path.clone(),
                        materialize_to: entry.materialize_to.clone(),
                        repo_name: entry.repo_name.clone(),
                        repository_index: entry.repository_index,
                        snapshot: entry.snapshot.clone(),
                    })
                    .collect(),
            })
        })
        .collect()
}

fn status_nodes(
    manifest: &ExternalManifest,
    lockfile: Option<&ExternalLockfile>,
) -> ExternalResult<Vec<ExternalLockNode>> {
    let mut nodes = lockfile
        .map(ExternalLockfile::sorted_nodes)
        .unwrap_or_else(|| {
            manifest
                .externals
                .iter()
                .map(ExternalLockNode::from_direct_declaration)
                .collect::<Vec<_>>()
        });
    let mut identities = nodes
        .iter()
        .map(|node| (node.parent_path.clone(), node.name.clone()))
        .collect::<BTreeSet<_>>();
    for external in &manifest.externals {
        let node = ExternalLockNode::from_direct_declaration(external);
        let identity = (node.parent_path.clone(), node.name.clone());
        if identities.insert(identity) {
            nodes.push(node);
        }
    }
    nodes.sort_by(|left, right| {
        (
            left.parent_path.as_str(),
            left.materialize_to.as_str(),
            left.name.as_str(),
            left.repository_index,
            left.repo_name.as_str(),
            left.snapshot.as_str(),
        )
            .cmp(&(
                right.parent_path.as_str(),
                right.materialize_to.as_str(),
                right.name.as_str(),
                right.repository_index,
                right.repo_name.as_str(),
                right.snapshot.as_str(),
            ))
    });
    if nodes.is_empty() && !manifest.externals.is_empty() {
        return Err(ExternalError::with_code(
            "external_status_nodes",
            "external status could not derive status nodes",
        ));
    }
    Ok(nodes)
}

fn missing_lock_drift(node: &ExternalLockNode) -> ExternalLockDrift {
    ExternalLockDrift {
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
