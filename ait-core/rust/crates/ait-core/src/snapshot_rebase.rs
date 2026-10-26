//! Pure tree and ordered history replay. This module neither authors Snapshots
//! nor changes workflow receipts, ownership, or publication mappings.
use std::collections::{BTreeMap, BTreeSet};

use crate::local_snapshot::SnapshotFileRow;
use crate::workflow_binary_db::SnapshotWorkflowBinding;

pub type SnapshotFileMap = BTreeMap<String, SnapshotFileRow>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotRebaseResolution {
    Target,
    Feature,
    SameResult,
    Conflict,
}

impl SnapshotRebaseResolution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Feature => "feature",
            Self::SameResult => "same_result",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotRebaseApply {
    Unchanged,
    Write,
    Remove,
    Conflict,
}

impl SnapshotRebaseApply {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Write => "write",
            Self::Remove => "remove",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotRebasePath {
    pub path: String,
    pub feature_changed: bool,
    pub target_changed: bool,
    pub resolution: SnapshotRebaseResolution,
    pub apply: SnapshotRebaseApply,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotTreeRebasePlan {
    pub feature_delta_count: usize,
    pub paths: Vec<SnapshotRebasePath>,
    pub write_paths: Vec<String>,
    pub remove_paths: Vec<String>,
    pub conflict_paths: Vec<String>,
    /// Never expose a partially merged tree as a successful replay.
    pub merged_files: Option<SnapshotFileMap>,
}

/// The worktree rebase rule: apply an exclusive feature change, keep an
/// exclusive target change, deduplicate equal results, and stop on divergence.
/// Full rows participate in equality, including mode and symlink identity.
pub fn rebase_snapshot_files(
    base: &SnapshotFileMap,
    feature: &SnapshotFileMap,
    target: &SnapshotFileMap,
) -> SnapshotTreeRebasePlan {
    let paths = base
        .keys()
        .chain(feature.keys())
        .chain(target.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut plan = SnapshotTreeRebasePlan {
        feature_delta_count: 0,
        paths: Vec::new(),
        write_paths: Vec::new(),
        remove_paths: Vec::new(),
        conflict_paths: Vec::new(),
        merged_files: None,
    };
    let mut merged = target.clone();
    for path in paths {
        let old = base.get(&path);
        let source = feature.get(&path);
        let destination = target.get(&path);
        let feature_changed = source != old;
        let target_changed = destination != old;
        if !feature_changed && !target_changed {
            continue;
        }
        plan.feature_delta_count += usize::from(feature_changed);
        let (resolution, apply) = if !feature_changed {
            (
                SnapshotRebaseResolution::Target,
                SnapshotRebaseApply::Unchanged,
            )
        } else if !target_changed {
            if let Some(row) = source {
                merged.insert(path.clone(), row.clone());
                plan.write_paths.push(path.clone());
                (
                    SnapshotRebaseResolution::Feature,
                    SnapshotRebaseApply::Write,
                )
            } else {
                merged.remove(&path);
                plan.remove_paths.push(path.clone());
                (
                    SnapshotRebaseResolution::Feature,
                    SnapshotRebaseApply::Remove,
                )
            }
        } else if source == destination {
            (
                SnapshotRebaseResolution::SameResult,
                SnapshotRebaseApply::Unchanged,
            )
        } else {
            plan.conflict_paths.push(path.clone());
            (
                SnapshotRebaseResolution::Conflict,
                SnapshotRebaseApply::Conflict,
            )
        };
        plan.paths.push(SnapshotRebasePath {
            path,
            feature_changed,
            target_changed,
            resolution,
            apply,
        });
    }
    if plan.conflict_paths.is_empty() {
        plan.merged_files = Some(merged);
    }
    plan
}

/// One validated, consecutive source boundary. Metadata is borrowed unchanged
/// by the plan so the caller retains its original receipts and published maps.
#[derive(Clone, Debug)]
pub struct SnapshotHistoryBoundary<M> {
    pub base_snapshot_id: String,
    pub snapshot_id: String,
    pub owner: SnapshotWorkflowBinding,
    pub base_files: SnapshotFileMap,
    pub files: SnapshotFileMap,
    pub metadata: M,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotReplayParent {
    Existing(String),
    /// Index of the preceding step whose newly authored ID is not known yet.
    Replayed(usize),
}

#[derive(Debug)]
pub struct SnapshotHistoryRebaseStep<'a, M> {
    pub source: &'a SnapshotHistoryBoundary<M>,
    pub parent: SnapshotReplayParent,
    pub files: SnapshotFileMap,
    /// Same content is insufficient for reuse: its parent must also match.
    pub reuse_snapshot_id: Option<&'a str>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum SnapshotHistoryRebaseError {
    InvalidHistory(String),
    Conflict {
        task_id: String,
        snapshot_id: String,
        paths: Vec<String>,
    },
}

/// Replay each eligible boundary on the target or the previous replay output.
/// Inputs must exclude any prefix already present in the target. A conflict
/// returns no partial plan; callers must resolve it before capture or CI.
pub fn plan_snapshot_history_rebase<'a, M>(
    target_snapshot_id: &str,
    target_files: &SnapshotFileMap,
    boundaries: &'a [SnapshotHistoryBoundary<M>],
) -> Result<Vec<SnapshotHistoryRebaseStep<'a, M>>, SnapshotHistoryRebaseError> {
    let invalid = |message: &str| SnapshotHistoryRebaseError::InvalidHistory(message.into());
    if target_snapshot_id.trim().is_empty() || boundaries.is_empty() {
        return Err(invalid(
            "History replay requires a target and source boundaries",
        ));
    }
    let mut seen_snapshots = BTreeSet::from([&boundaries[0].base_snapshot_id]);
    let mut seen_tasks = BTreeSet::new();
    let mut previous_task = None;
    let mut previous_source: Option<&SnapshotHistoryBoundary<M>> = None;
    // Validate the complete input before computing any output.
    for boundary in boundaries {
        if boundary.base_snapshot_id.trim().is_empty()
            || boundary.snapshot_id.trim().is_empty()
            || boundary.base_snapshot_id == boundary.snapshot_id
            || boundary.owner.task_id.trim().is_empty()
            || boundary.owner.line_name.trim().is_empty()
            || !seen_snapshots.insert(&boundary.snapshot_id)
        {
            return Err(invalid(
                "History replay has an invalid identity or repeated Snapshot",
            ));
        }
        if previous_task != Some(&boundary.owner.task_id) {
            if !seen_tasks.insert(&boundary.owner.task_id) {
                return Err(invalid("History replay returns to an earlier Task"));
            }
            previous_task = Some(&boundary.owner.task_id);
        }
        if let Some(previous) = previous_source {
            if boundary.base_snapshot_id != previous.snapshot_id
                || boundary.base_files != previous.files
            {
                return Err(invalid(
                    "History replay contains a gap or conflicting immutable tree",
                ));
            }
        }
        if boundary.snapshot_id == target_snapshot_id {
            return Err(invalid(
                "Select only the history suffix not already present in the target",
            ));
        }
        if boundary.base_snapshot_id == target_snapshot_id && boundary.base_files != *target_files {
            return Err(invalid(
                "Target Snapshot has conflicting immutable tree content",
            ));
        }
        previous_source = Some(boundary);
    }
    let mut parent = SnapshotReplayParent::Existing(target_snapshot_id.into());
    let mut files = target_files.clone();
    let mut steps = Vec::with_capacity(boundaries.len());
    for (index, source) in boundaries.iter().enumerate() {
        let merged = rebase_snapshot_files(&source.base_files, &source.files, &files);
        let Some(replayed_files) = merged.merged_files else {
            return Err(SnapshotHistoryRebaseError::Conflict {
                task_id: source.owner.task_id.clone(),
                snapshot_id: source.snapshot_id.clone(),
                paths: merged.conflict_paths,
            });
        };
        let reuse = parent == SnapshotReplayParent::Existing(source.base_snapshot_id.clone())
            && replayed_files == source.files;
        let next_parent = if reuse {
            SnapshotReplayParent::Existing(source.snapshot_id.clone())
        } else {
            SnapshotReplayParent::Replayed(index)
        };
        steps.push(SnapshotHistoryRebaseStep {
            source,
            parent,
            files: replayed_files.clone(),
            reuse_snapshot_id: reuse.then_some(source.snapshot_id.as_str()),
        });
        files = replayed_files;
        parent = next_parent;
    }
    Ok(steps)
}

#[cfg(test)]
mod tests;
