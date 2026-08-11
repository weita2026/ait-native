use std::collections::BTreeSet;

use crate::external::lockfile::{ExternalLockNode, ExternalLockfile};
use crate::external::manifest::{ExternalDeclaration, ExternalManifest};
use crate::external::{ExternalError, ExternalResult};

pub trait ExternalSnapshotResolver {
    fn snapshot_exists(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> ExternalResult<bool>;

    fn snapshot_available_from_remote(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        snapshot: &str,
    ) -> ExternalResult<bool>;

    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> ExternalResult<Option<String>>;

    fn snapshot_manifest(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> ExternalResult<Option<ExternalManifest>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalSnapshotSelection {
    ManifestPins,
    Exact { name: String, snapshot: String },
    Latest { name: String },
}

impl ExternalSnapshotSelection {
    fn target_name(&self) -> Option<&str> {
        match self {
            Self::ManifestPins => None,
            Self::Exact { name, .. } | Self::Latest { name } => Some(name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalResolutionOptions {
    pub selection: ExternalSnapshotSelection,
    pub remote_ready: bool,
}

impl ExternalResolutionOptions {
    pub fn manifest_pins() -> Self {
        Self {
            selection: ExternalSnapshotSelection::ManifestPins,
            remote_ready: false,
        }
    }

    pub fn exact(name: impl Into<String>, snapshot: impl Into<String>) -> Self {
        Self {
            selection: ExternalSnapshotSelection::Exact {
                name: name.into(),
                snapshot: snapshot.into(),
            },
            remote_ready: false,
        }
    }

    pub fn latest(name: impl Into<String>) -> Self {
        Self {
            selection: ExternalSnapshotSelection::Latest { name: name.into() },
            remote_ready: false,
        }
    }

    pub fn with_remote_ready(mut self, remote_ready: bool) -> Self {
        self.remote_ready = remote_ready;
        self
    }
}

pub fn resolve_external_lockfile<R>(
    resolver: &R,
    manifest: &ExternalManifest,
    options: &ExternalResolutionOptions,
) -> ExternalResult<ExternalLockfile>
where
    R: ExternalSnapshotResolver + ?Sized,
{
    manifest.validate()?;
    ensure_selected_external_exists(manifest, &options.selection)?;

    let mut nodes = Vec::new();
    let mut recursion_stack = BTreeSet::new();
    for declaration in &manifest.externals {
        let resolved_declaration = resolve_root_declaration(resolver, declaration, options)?;
        resolve_declaration(
            resolver,
            &resolved_declaration,
            "",
            options.remote_ready,
            &mut nodes,
            &mut recursion_stack,
        )?;
    }

    let lockfile = ExternalLockfile::new(nodes).normalized();
    lockfile.validate()?;
    Ok(lockfile)
}

fn ensure_selected_external_exists(
    manifest: &ExternalManifest,
    selection: &ExternalSnapshotSelection,
) -> ExternalResult<()> {
    let Some(target_name) = selection.target_name() else {
        return Ok(());
    };

    let matches = manifest
        .externals
        .iter()
        .filter(|external| external.name == target_name)
        .count();
    match matches {
        0 => Err(ExternalError::with_code(
            "external_resolver_target_missing",
            format!("external {target_name:?} is not declared in the root manifest"),
        )),
        1 => Ok(()),
        _ => Err(ExternalError::with_code(
            "external_resolver_target_ambiguous",
            format!("external {target_name:?} appears more than once in the root manifest"),
        )),
    }
}

fn resolve_root_declaration<R>(
    resolver: &R,
    declaration: &ExternalDeclaration,
    options: &ExternalResolutionOptions,
) -> ExternalResult<ExternalDeclaration>
where
    R: ExternalSnapshotResolver + ?Sized,
{
    let mut resolved = declaration.clone();
    match &options.selection {
        ExternalSnapshotSelection::ManifestPins => {}
        ExternalSnapshotSelection::Exact { name, snapshot } if name == &declaration.name => {
            resolved.snapshot = snapshot.clone();
        }
        ExternalSnapshotSelection::Latest { name } if name == &declaration.name => {
            resolved.snapshot = resolver
                .line_head_snapshot(
                    declaration.repository_index,
                    &declaration.repo_name,
                    &declaration.remote,
                    &declaration.line,
                )?
                .ok_or_else(|| {
                    ExternalError::with_code(
                        "external_line_head_missing",
                        format!(
                            "external {:?} line {:?} on remote {:?} has no head snapshot",
                            declaration.name, declaration.line, declaration.remote
                        ),
                    )
                })?;
        }
        ExternalSnapshotSelection::Exact { .. } | ExternalSnapshotSelection::Latest { .. } => {}
    }
    Ok(resolved)
}

fn resolve_declaration<R>(
    resolver: &R,
    declaration: &ExternalDeclaration,
    parent_path: &str,
    remote_ready: bool,
    nodes: &mut Vec<ExternalLockNode>,
    recursion_stack: &mut BTreeSet<String>,
) -> ExternalResult<()>
where
    R: ExternalSnapshotResolver + ?Sized,
{
    ensure_snapshot_available(resolver, declaration, remote_ready)?;

    let stack_key = snapshot_stack_key(declaration);
    if !recursion_stack.insert(stack_key.clone()) {
        return Err(ExternalError::with_code(
            "external_resolver_cycle",
            format!(
                "external {:?} snapshot {:?} would create a recursive external graph",
                declaration.name, declaration.snapshot
            ),
        ));
    }

    let node = lock_node_from_declaration(declaration, parent_path);
    let child_parent_path = node.materialize_to.clone();
    nodes.push(node);

    if let Some(nested_manifest) = resolver.snapshot_manifest(
        declaration.repository_index,
        &declaration.repo_name,
        &declaration.snapshot,
    )? {
        nested_manifest.validate().map_err(|err| {
            ExternalError::with_code(
                "external_nested_manifest_invalid",
                format!(
                    "external {:?} snapshot {:?} contains an invalid nested manifest: {}",
                    declaration.name,
                    declaration.snapshot,
                    err.message()
                ),
            )
        })?;
        for nested_declaration in &nested_manifest.externals {
            resolve_declaration(
                resolver,
                nested_declaration,
                &child_parent_path,
                remote_ready,
                nodes,
                recursion_stack,
            )?;
        }
    }

    recursion_stack.remove(&stack_key);
    Ok(())
}

fn ensure_snapshot_available<R>(
    resolver: &R,
    declaration: &ExternalDeclaration,
    remote_ready: bool,
) -> ExternalResult<()>
where
    R: ExternalSnapshotResolver + ?Sized,
{
    if !resolver.snapshot_exists(
        declaration.repository_index,
        &declaration.repo_name,
        &declaration.snapshot,
    )? {
        return Err(ExternalError::with_code(
            "external_snapshot_missing",
            format!(
                "external {:?} snapshot {:?} is not available locally for repo {:?}",
                declaration.name, declaration.snapshot, declaration.repo_name
            ),
        ));
    }

    if remote_ready
        && !resolver.snapshot_available_from_remote(
            declaration.repository_index,
            &declaration.repo_name,
            &declaration.remote,
            &declaration.snapshot,
        )?
    {
        return Err(ExternalError::with_code(
            "external_snapshot_remote_unavailable",
            format!(
                "external {:?} snapshot {:?} exists locally but is not available from remote {:?} for repo {:?}",
                declaration.name, declaration.snapshot, declaration.remote, declaration.repo_name
            ),
        ));
    }

    Ok(())
}

fn lock_node_from_declaration(
    declaration: &ExternalDeclaration,
    parent_path: &str,
) -> ExternalLockNode {
    let mut node = ExternalLockNode::from_direct_declaration(declaration);
    if !parent_path.is_empty() {
        node.parent_path = parent_path.to_string();
        node.materialize_to = join_materialize_path(parent_path, &declaration.materialize_to);
    }
    node
}

fn join_materialize_path(parent_path: &str, child_path: &str) -> String {
    let parent = parent_path.trim_end_matches('/');
    let child = child_path.trim_start_matches("./").trim_start_matches('/');
    format!("{parent}/{child}")
}

fn snapshot_stack_key(declaration: &ExternalDeclaration) -> String {
    format!(
        "{}|{}|{}|{}",
        declaration.repository_index,
        declaration.repo_name,
        declaration.remote,
        declaration.snapshot
    )
}
