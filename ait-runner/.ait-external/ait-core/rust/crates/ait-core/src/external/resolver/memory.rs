use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::external::manifest::ExternalManifest;
use crate::external::resolver::ExternalSnapshotResolver;
use crate::external::ExternalResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryExternalResolverCall {
    SnapshotExists {
        repository_index: u32,
        repo_name: String,
        snapshot: String,
    },
    SnapshotAvailableFromRemote {
        repository_index: u32,
        repo_name: String,
        remote: String,
        snapshot: String,
    },
    LineHeadSnapshot {
        repository_index: u32,
        repo_name: String,
        remote: String,
        line: String,
    },
    SnapshotManifest {
        repository_index: u32,
        repo_name: String,
        snapshot: String,
    },
}

#[derive(Debug, Default)]
pub struct MemoryExternalSnapshotResolver {
    snapshots: BTreeMap<(u32, String, String), Option<ExternalManifest>>,
    remote_snapshots: BTreeSet<(u32, String, String, String)>,
    line_heads: BTreeMap<(u32, String, String, String), String>,
    calls: RefCell<Vec<MemoryExternalResolverCall>>,
}

impl MemoryExternalSnapshotResolver {
    pub fn with_snapshot_without_manifest(
        self,
        repo_name: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.with_snapshot_without_manifest_at(0, repo_name, snapshot)
    }

    pub fn with_snapshot_without_manifest_at(
        mut self,
        repository_index: u32,
        repo_name: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.snapshots
            .insert((repository_index, repo_name.into(), snapshot.into()), None);
        self
    }

    pub fn with_snapshot_manifest(
        self,
        repo_name: impl Into<String>,
        snapshot: impl Into<String>,
        manifest: ExternalManifest,
    ) -> Self {
        self.with_snapshot_manifest_at(0, repo_name, snapshot, manifest)
    }

    pub fn with_snapshot_manifest_at(
        mut self,
        repository_index: u32,
        repo_name: impl Into<String>,
        snapshot: impl Into<String>,
        manifest: ExternalManifest,
    ) -> Self {
        self.snapshots.insert(
            (repository_index, repo_name.into(), snapshot.into()),
            Some(manifest),
        );
        self
    }

    pub fn with_remote_snapshot(
        self,
        repo_name: impl Into<String>,
        remote: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.with_remote_snapshot_at(0, repo_name, remote, snapshot)
    }

    pub fn with_remote_snapshot_at(
        mut self,
        repository_index: u32,
        repo_name: impl Into<String>,
        remote: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.remote_snapshots.insert((
            repository_index,
            repo_name.into(),
            remote.into(),
            snapshot.into(),
        ));
        self
    }

    pub fn with_line_head(
        self,
        repo_name: impl Into<String>,
        remote: impl Into<String>,
        line: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.with_line_head_at(0, repo_name, remote, line, snapshot)
    }

    pub fn with_line_head_at(
        mut self,
        repository_index: u32,
        repo_name: impl Into<String>,
        remote: impl Into<String>,
        line: impl Into<String>,
        snapshot: impl Into<String>,
    ) -> Self {
        self.line_heads.insert(
            (
                repository_index,
                repo_name.into(),
                remote.into(),
                line.into(),
            ),
            snapshot.into(),
        );
        self
    }

    pub fn calls(&self) -> Vec<MemoryExternalResolverCall> {
        self.calls.borrow().clone()
    }

    fn record_call(&self, call: MemoryExternalResolverCall) {
        self.calls.borrow_mut().push(call);
    }
}

impl ExternalSnapshotResolver for MemoryExternalSnapshotResolver {
    fn snapshot_exists(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> ExternalResult<bool> {
        self.record_call(MemoryExternalResolverCall::SnapshotExists {
            repository_index,
            repo_name: repo_name.to_string(),
            snapshot: snapshot.to_string(),
        });
        Ok(self.snapshots.contains_key(&(
            repository_index,
            repo_name.to_string(),
            snapshot.to_string(),
        )))
    }

    fn snapshot_available_from_remote(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        snapshot: &str,
    ) -> ExternalResult<bool> {
        self.record_call(MemoryExternalResolverCall::SnapshotAvailableFromRemote {
            repository_index,
            repo_name: repo_name.to_string(),
            remote: remote.to_string(),
            snapshot: snapshot.to_string(),
        });
        Ok(self.remote_snapshots.contains(&(
            repository_index,
            repo_name.to_string(),
            remote.to_string(),
            snapshot.to_string(),
        )))
    }

    fn line_head_snapshot(
        &self,
        repository_index: u32,
        repo_name: &str,
        remote: &str,
        line: &str,
    ) -> ExternalResult<Option<String>> {
        self.record_call(MemoryExternalResolverCall::LineHeadSnapshot {
            repository_index,
            repo_name: repo_name.to_string(),
            remote: remote.to_string(),
            line: line.to_string(),
        });
        Ok(self
            .line_heads
            .get(&(
                repository_index,
                repo_name.to_string(),
                remote.to_string(),
                line.to_string(),
            ))
            .cloned())
    }

    fn snapshot_manifest(
        &self,
        repository_index: u32,
        repo_name: &str,
        snapshot: &str,
    ) -> ExternalResult<Option<ExternalManifest>> {
        self.record_call(MemoryExternalResolverCall::SnapshotManifest {
            repository_index,
            repo_name: repo_name.to_string(),
            snapshot: snapshot.to_string(),
        });
        Ok(self
            .snapshots
            .get(&(
                repository_index,
                repo_name.to_string(),
                snapshot.to_string(),
            ))
            .cloned()
            .flatten())
    }
}
