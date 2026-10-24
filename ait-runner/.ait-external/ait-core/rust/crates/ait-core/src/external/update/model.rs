use crate::json_support::{json, JsonValue};

use crate::external::lockfile::ExternalLockfile;
use crate::external::manifest::ExternalManifest;
use crate::external::materializer::{
    ExternalLocalLinkOverride, ExternalMaterializationOptions, ExternalMaterializationReport,
    ExternalMaterializer,
};
use crate::external::resolver::{
    resolve_external_lockfile, ExternalResolutionOptions, ExternalSnapshotResolver,
};
use crate::external::{ExternalError, ExternalResult};

pub trait ExternalUpdateStore {
    type Prepared: ExternalPreparedUpdate;

    fn read_manifest(&self) -> ExternalResult<ExternalManifest>;
    fn read_lockfile(&self) -> ExternalResult<Option<ExternalLockfile>>;

    fn prepare_update(
        &self,
        manifest: &ExternalManifest,
        lockfile: &ExternalLockfile,
    ) -> ExternalResult<Self::Prepared>;
}

pub trait ExternalPreparedUpdate {
    fn commit(self) -> ExternalResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalUpdateSelection {
    ManifestPins,
    Exact { name: String, snapshot: String },
    Latest { name: String },
}

impl ExternalUpdateSelection {
    pub fn manifest_pins() -> Self {
        Self::ManifestPins
    }

    pub fn exact(name: impl Into<String>, snapshot: impl Into<String>) -> Self {
        Self::Exact {
            name: name.into(),
            snapshot: snapshot.into(),
        }
    }

    pub fn latest(name: impl Into<String>) -> Self {
        Self::Latest { name: name.into() }
    }

    fn target_name(&self) -> Option<&str> {
        match self {
            Self::ManifestPins => None,
            Self::Exact { name, .. } | Self::Latest { name } => Some(name),
        }
    }

    fn to_resolution_options(&self) -> ExternalResolutionOptions {
        match self {
            Self::ManifestPins => ExternalResolutionOptions::manifest_pins(),
            Self::Exact { name, snapshot } => {
                ExternalResolutionOptions::exact(name.clone(), snapshot.clone())
            }
            Self::Latest { name } => ExternalResolutionOptions::latest(name.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalUpdateOptions {
    pub selection: ExternalUpdateSelection,
    pub locked: bool,
    pub release_ready: bool,
    pub no_recursive: bool,
    pub validate: bool,
    pub remote_ready: bool,
    pub local_link_overrides: Vec<ExternalLocalLinkOverride>,
}

impl ExternalUpdateOptions {
    pub fn manifest_pins() -> Self {
        Self {
            selection: ExternalUpdateSelection::ManifestPins,
            locked: false,
            release_ready: false,
            no_recursive: false,
            validate: false,
            remote_ready: false,
            local_link_overrides: Vec::new(),
        }
    }

    pub fn exact(name: impl Into<String>, snapshot: impl Into<String>) -> Self {
        Self {
            selection: ExternalUpdateSelection::exact(name, snapshot),
            ..Self::manifest_pins()
        }
    }

    pub fn latest(name: impl Into<String>) -> Self {
        Self {
            selection: ExternalUpdateSelection::latest(name),
            ..Self::manifest_pins()
        }
    }

    pub fn with_locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    pub fn with_release_ready(mut self, release_ready: bool) -> Self {
        self.release_ready = release_ready;
        self
    }

    pub fn with_no_recursive(mut self, no_recursive: bool) -> Self {
        self.no_recursive = no_recursive;
        self
    }

    pub fn with_validate(mut self, validate: bool) -> Self {
        self.validate = validate;
        self
    }

    pub fn with_remote_ready(mut self, remote_ready: bool) -> Self {
        self.remote_ready = remote_ready;
        self
    }

    pub fn with_local_link_override(
        mut self,
        name: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        self.local_link_overrides.push(ExternalLocalLinkOverride {
            name: name.into(),
            path: path.into(),
        });
        self
    }

    fn materialization_options(&self) -> ExternalMaterializationOptions {
        ExternalMaterializationOptions {
            no_recursive: self.no_recursive,
            locked: self.locked,
            release_ready: self.release_ready,
            local_link_overrides: self.local_link_overrides.clone(),
        }
    }
}

pub fn run_external_update<S, R, M>(
    store: &S,
    resolver: &R,
    materializer: &M,
    options: &ExternalUpdateOptions,
) -> ExternalResult<ExternalUpdateReport>
where
    S: ExternalUpdateStore,
    R: ExternalSnapshotResolver + ?Sized,
    M: ExternalMaterializer + ?Sized,
{
    let current_manifest = store.read_manifest()?;
    let current_lockfile = store.read_lockfile()?;
    if let Some(lockfile) = &current_lockfile {
        lockfile.validate()?;
    }
    if options.locked {
        let lockfile = current_lockfile.as_ref().ok_or_else(|| {
            ExternalError::with_code(
                "external_lock_missing",
                "`ait external update --locked` requires ait-external.lock",
            )
        })?;
        let drift = lockfile.locked_drift_against_manifest(&current_manifest);
        if !drift.is_empty() {
            let messages = drift
                .iter()
                .map(|drift| drift.message.clone())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(ExternalError::with_code(
                "external_lock_drift",
                format!(
                    "`ait external update --locked` requires ait-external.lock to match ait-external.toml: {messages}"
                ),
            ));
        }
        let materialization =
            materializer.materialize_lockfile(lockfile, &options.materialization_options())?;
        return Ok(ExternalUpdateReport {
            changed_pins: Vec::new(),
            manifest_changed: false,
            lockfile_changed: false,
            materialization,
            locked: true,
            recursive: !options.no_recursive,
            validated: options.validate,
        });
    }

    let resolution_options = options
        .selection
        .to_resolution_options()
        .with_remote_ready(options.remote_ready);
    let resolved_lockfile =
        resolve_external_lockfile(resolver, &current_manifest, &resolution_options)?;
    let next_manifest = manifest_with_resolved_selection(
        &current_manifest,
        &resolved_lockfile,
        &options.selection,
    )?;
    let changed_pins = changed_pins(&current_manifest, &next_manifest);

    let manifest_changed = current_manifest != next_manifest;
    let lockfile_changed = current_lockfile.as_ref() != Some(&resolved_lockfile);

    let prepared = store.prepare_update(&next_manifest, &resolved_lockfile)?;
    let materialization = materializer
        .materialize_lockfile(&resolved_lockfile, &options.materialization_options())?;
    prepared.commit()?;

    Ok(ExternalUpdateReport {
        changed_pins,
        manifest_changed,
        lockfile_changed,
        materialization,
        locked: options.locked,
        recursive: !options.no_recursive,
        validated: options.validate,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalUpdatePinChange {
    pub name: String,
    pub repo_name: String,
    pub repository_index: u32,
    pub remote: String,
    pub line: String,
    pub previous_snapshot: String,
    pub new_snapshot: String,
}

impl ExternalUpdatePinChange {
    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "name": self.name,
            "repo_name": self.repo_name,
            "repository_index": self.repository_index,
            "remote": self.remote,
            "line": self.line,
            "previous_snapshot": self.previous_snapshot,
            "new_snapshot": self.new_snapshot,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalUpdateReport {
    pub changed_pins: Vec<ExternalUpdatePinChange>,
    pub manifest_changed: bool,
    pub lockfile_changed: bool,
    pub materialization: ExternalMaterializationReport,
    pub locked: bool,
    pub recursive: bool,
    pub validated: bool,
}

impl ExternalUpdateReport {
    pub fn states(&self) -> ExternalUpdateStates {
        let updated = self.manifest_changed || self.lockfile_changed;
        ExternalUpdateStates {
            updated,
            materialized: !self.materialization.entries.is_empty(),
            unchanged: !updated,
            validation_required: !self.validated,
        }
    }

    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "command": "external update",
            "mode": if self.locked { "locked" } else { "update" },
            "recursive": self.recursive,
            "locked": self.locked,
            "validated": self.validated,
            "changed_pins": self.changed_pins.iter().map(ExternalUpdatePinChange::to_json_value).collect::<Vec<_>>(),
            "manifest_changed": self.manifest_changed,
            "lockfile_changed": self.lockfile_changed,
            "states": self.states().to_json_value(),
            "materialization": self.materialization.to_json_value(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalUpdateStates {
    pub updated: bool,
    pub materialized: bool,
    pub unchanged: bool,
    pub validation_required: bool,
}

impl ExternalUpdateStates {
    pub fn to_json_value(self) -> JsonValue {
        json!({
            "updated": self.updated,
            "materialized": self.materialized,
            "unchanged": self.unchanged,
            "validation_required": self.validation_required,
        })
    }
}

fn manifest_with_resolved_selection(
    manifest: &ExternalManifest,
    lockfile: &ExternalLockfile,
    selection: &ExternalUpdateSelection,
) -> ExternalResult<ExternalManifest> {
    let Some(target_name) = selection.target_name() else {
        return Ok(manifest.clone());
    };

    let root_node = lockfile
        .nodes
        .iter()
        .find(|node| node.parent_path.is_empty() && node.name == target_name)
        .ok_or_else(|| {
            ExternalError::with_code(
                "external_update_target_unresolved",
                format!("external {target_name:?} was not resolved into the lockfile root"),
            )
        })?;

    let mut next_manifest = manifest.clone();
    let mut match_count = 0usize;
    for external in &mut next_manifest.externals {
        if external.name == target_name {
            external.snapshot = root_node.snapshot.clone();
            match_count += 1;
        }
    }
    if match_count != 1 {
        return Err(ExternalError::with_code(
            "external_update_target_unresolved",
            format!("external {target_name:?} must match exactly one root manifest entry"),
        ));
    }
    next_manifest.validate()?;
    Ok(next_manifest)
}

fn changed_pins(
    before: &ExternalManifest,
    after: &ExternalManifest,
) -> Vec<ExternalUpdatePinChange> {
    before
        .externals
        .iter()
        .zip(after.externals.iter())
        .filter(|(before, after)| before.snapshot != after.snapshot)
        .map(|(before, after)| ExternalUpdatePinChange {
            name: after.name.clone(),
            repo_name: after.repo_name.clone(),
            repository_index: after.repository_index,
            remote: after.remote.clone(),
            line: after.line.clone(),
            previous_snapshot: before.snapshot.clone(),
            new_snapshot: after.snapshot.clone(),
        })
        .collect()
}
