use crate::external::bindings::{
    CommandExternalBindingToolProbe, ExternalBindingCheckProvider, ExternalBindingCommand,
    ExternalBindingCommandOutput, ExternalBindingCommandRunner, ExternalBindingTool,
    ExternalBindingToolProbe, ExternalBindingToolProbeRequest, ExternalBindingToolProbeResult,
    ExternalBindingValidationRequest, ExternalBindingValidator, FilesystemExternalBindingValidator,
    NoopExternalBindingToolProbe,
};
use crate::external::doctor::{build_external_doctor_report, ExternalDoctorOptions};
use crate::external::link::{
    parse_external_local_link_overrides, remove_external_local_link_override,
    render_external_local_link_overrides, upsert_external_local_link_override, ExternalLinkStore,
    FsExternalLinkStore, EXTERNAL_LINKS_FILE,
};
use crate::external::lockfile::{
    ExternalLockBindingSummary, ExternalLockCodec, ExternalLockDriftKind, ExternalLockNode,
    ExternalLockfile, TomlExternalLockCodec,
};
use crate::external::manifest::{
    ExternalDeclaration, ExternalManifest, ExternalManifestCodec, TomlExternalManifestCodec,
};
use crate::external::materializer::{
    ExternalLocalLinkOverride, ExternalMaterializationEntry, ExternalMaterializationOptions,
    ExternalMaterializationReport, ExternalMaterializationState, ExternalMaterializer,
    ExternalMaterializerMarkerFileEntry, ExternalMaterializerMarkerJson,
    ExternalMaterializerMarkerRecord, FilesystemExternalMaterializer, FixtureExternalContentSource,
    EXTERNAL_MATERIALIZER_MARKER, EXTERNAL_MATERIALIZER_MARKER_FORMAT,
    EXTERNAL_MATERIALIZER_MARKER_VERSION,
};
use crate::external::readiness::build_external_readiness_report;
use crate::external::release::external_release_closure_metadata_from_lockfile_bytes;
use crate::external::resolver::{
    resolve_external_lockfile, ExternalResolutionOptions, MemoryExternalResolverCall,
    MemoryExternalSnapshotResolver,
};
use crate::external::status::{
    build_external_status_report, inspect_external_filesystem_status_report,
    inspect_external_materialization, inspect_external_status_report,
    inspect_operational_external_projection_roots, ExternalBindingCheckFact,
    ExternalCurrentSourceArtifactRole, ExternalCurrentSourceArtifactState,
    ExternalCurrentSourceArtifactStatus, ExternalCurrentSourceCoreStatus, ExternalDuplicatePolicy,
    ExternalMaterializationObservation, ExternalObservedMaterializationState, ExternalStatusInput,
    ExternalStatusState,
};
use crate::external::update::{
    run_external_update, ExternalPreparedUpdate, ExternalUpdateOptions, ExternalUpdateStore,
};
use crate::external::ExternalError;
use crate::file_io::{FileIoResult, FileIoStore};
use crate::workspace_test_support;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn external_fixture_path(relative_path: &str) -> PathBuf {
    workspace_test_support::crate_root("ait-core")
        .join("src/external/tests/fixtures")
        .join(relative_path)
}

fn parse_manifest_fixture(relative_path: &str) -> ExternalManifest {
    let bytes = std::fs::read(external_fixture_path(relative_path))
        .unwrap_or_else(|err| panic!("failed to read external fixture {relative_path}: {err}"));
    TomlExternalManifestCodec
        .parse_manifest(&bytes)
        .unwrap_or_else(|err| panic!("failed to parse external fixture {relative_path}: {err}"))
}

fn read_fixture_text(relative_path: &str) -> String {
    std::fs::read_to_string(external_fixture_path(relative_path))
        .unwrap_or_else(|err| panic!("failed to read external fixture {relative_path}: {err}"))
}

fn read_materialized_snapshot_fixture(repo_root: &Path, materialize_to: &str) -> String {
    std::fs::read_to_string(repo_root.join(materialize_to).join("AIT_EXTERNAL_SNAPSHOT"))
        .unwrap_or_else(|err| {
            panic!("failed to read materialized external snapshot {materialize_to}: {err}")
        })
}

fn parse_lock_fixture(relative_path: &str) -> ExternalLockfile {
    let bytes = std::fs::read(external_fixture_path(relative_path))
        .unwrap_or_else(|err| panic!("failed to read external fixture {relative_path}: {err}"));
    TomlExternalLockCodec
        .parse_lockfile(&bytes)
        .unwrap_or_else(|err| {
            panic!("failed to parse external lock fixture {relative_path}: {err}")
        })
}

fn test_manifest(externals: Vec<ExternalDeclaration>) -> ExternalManifest {
    ExternalManifest { externals }
}

fn test_external(
    name: &str,
    repo_name: &str,
    snapshot: &str,
    materialize_to: &str,
) -> ExternalDeclaration {
    ExternalDeclaration {
        name: name.to_string(),
        repo_name: repo_name.to_string(),
        repository_index: 0,
        remote: "origin".to_string(),
        line: "main".to_string(),
        snapshot: snapshot.to_string(),
        materialize_to: materialize_to.to_string(),
        license: "Apache-2.0".to_string(),
        version: None,
        bindings: Default::default(),
    }
}

fn write_external_status_authority(
    root: &Path,
    manifest: &ExternalManifest,
    lockfile: &ExternalLockfile,
) {
    std::fs::write(
        root.join("ait-external.toml"),
        TomlExternalManifestCodec.render_manifest(manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("ait-external.lock"),
        TomlExternalLockCodec.render_lockfile(lockfile).unwrap(),
    )
    .unwrap();
}

fn materialize_external_status_fixture(
    root: &Path,
    snapshot: &str,
) -> (ExternalManifest, ExternalLockfile) {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        snapshot,
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    write_external_status_authority(root, &manifest, &lockfile);
    FilesystemExternalMaterializer::new(root, FixtureExternalContentSource)
        .unwrap()
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();
    (manifest, lockfile)
}

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

#[derive(Clone)]
struct RecordingExternalBindingToolProbe {
    calls: Rc<RefCell<Vec<String>>>,
    result: ExternalBindingToolProbeResult,
}

impl RecordingExternalBindingToolProbe {
    fn new(result: ExternalBindingToolProbeResult) -> Self {
        Self {
            calls: Rc::new(RefCell::new(Vec::new())),
            result,
        }
    }
}

impl ExternalBindingToolProbe for RecordingExternalBindingToolProbe {
    fn probe_binding_tool(
        &self,
        request: ExternalBindingToolProbeRequest<'_>,
    ) -> crate::external::ExternalResult<ExternalBindingToolProbeResult> {
        self.calls.borrow_mut().push(format!(
            "{}:{}",
            request.tool.as_str(),
            request.binding.path
        ));
        Ok(self.result.clone())
    }
}

#[derive(Default)]
struct FakeExternalMaterializerMarkerFileIoStore {
    files: RefCell<BTreeMap<PathBuf, String>>,
    reads: RefCell<Vec<PathBuf>>,
    writes: RefCell<Vec<(PathBuf, String)>>,
}

impl FakeExternalMaterializerMarkerFileIoStore {
    fn insert_file(&self, path: impl Into<PathBuf>, text: impl Into<String>) {
        self.files.borrow_mut().insert(path.into(), text.into());
    }
}

impl FileIoStore for FakeExternalMaterializerMarkerFileIoStore {
    fn home_dir(&self) -> Option<PathBuf> {
        None
    }

    fn path_exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path)
    }

    fn read_to_string(&self, path: &Path) -> FileIoResult<String> {
        self.reads.borrow_mut().push(path.to_path_buf());
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing marker {}", path.display()).into())
    }

    fn write_string(&self, path: &Path, text: &str) -> FileIoResult<()> {
        self.writes
            .borrow_mut()
            .push((path.to_path_buf(), text.to_string()));
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), text.to_string());
        Ok(())
    }

    fn write_string_atomically(
        &self,
        _path: &Path,
        _text: &str,
        _publish_label: &str,
    ) -> FileIoResult<()> {
        unreachable!("external marker JSON uses the legacy direct-write policy")
    }
}

#[derive(Clone)]
struct RecordingExternalBindingCommandRunner {
    calls: Rc<RefCell<Vec<ExternalBindingCommand>>>,
    outputs: Rc<RefCell<VecDeque<ExternalBindingCommandOutput>>>,
}

impl RecordingExternalBindingCommandRunner {
    fn new(outputs: impl IntoIterator<Item = ExternalBindingCommandOutput>) -> Self {
        Self {
            calls: Rc::new(RefCell::new(Vec::new())),
            outputs: Rc::new(RefCell::new(outputs.into_iter().collect())),
        }
    }
}

impl ExternalBindingCommandRunner for RecordingExternalBindingCommandRunner {
    fn run_binding_command(
        &self,
        command: ExternalBindingCommand,
    ) -> crate::external::ExternalResult<ExternalBindingCommandOutput> {
        self.calls.borrow_mut().push(command);
        self.outputs.borrow_mut().pop_front().ok_or_else(|| {
            ExternalError::with_code(
                "test_external_binding_command",
                "missing fake command output",
            )
        })
    }
}

#[derive(Clone)]
struct MemoryExternalUpdateStore {
    state: Rc<RefCell<MemoryExternalUpdateStoreState>>,
}

#[derive(Clone)]
struct MemoryExternalUpdateStoreState {
    manifest: ExternalManifest,
    lockfile: Option<ExternalLockfile>,
    prepared_manifest: Option<ExternalManifest>,
    prepared_lockfile: Option<ExternalLockfile>,
    prepare_count: usize,
    commit_count: usize,
}

impl MemoryExternalUpdateStore {
    fn new(manifest: ExternalManifest, lockfile: Option<ExternalLockfile>) -> Self {
        Self {
            state: Rc::new(RefCell::new(MemoryExternalUpdateStoreState {
                manifest,
                lockfile,
                prepared_manifest: None,
                prepared_lockfile: None,
                prepare_count: 0,
                commit_count: 0,
            })),
        }
    }

    fn manifest_snapshot(&self, name: &str) -> Option<String> {
        self.state
            .borrow()
            .manifest
            .externals
            .iter()
            .find(|external| external.name == name)
            .map(|external| external.snapshot.clone())
    }

    fn lock_snapshot(&self, name: &str) -> Option<String> {
        self.lock_snapshot_at("", name)
    }

    fn lock_snapshot_at(&self, parent_path: &str, name: &str) -> Option<String> {
        self.state
            .borrow()
            .lockfile
            .as_ref()?
            .nodes
            .iter()
            .find(|node| node.parent_path == parent_path && node.name == name)
            .map(|node| node.snapshot.clone())
    }

    fn prepare_count(&self) -> usize {
        self.state.borrow().prepare_count
    }

    fn commit_count(&self) -> usize {
        self.state.borrow().commit_count
    }
}

struct MemoryPreparedExternalUpdate {
    state: Rc<RefCell<MemoryExternalUpdateStoreState>>,
    manifest: ExternalManifest,
    lockfile: ExternalLockfile,
}

impl ExternalUpdateStore for MemoryExternalUpdateStore {
    type Prepared = MemoryPreparedExternalUpdate;

    fn read_manifest(&self) -> crate::external::ExternalResult<ExternalManifest> {
        Ok(self.state.borrow().manifest.clone())
    }

    fn read_lockfile(&self) -> crate::external::ExternalResult<Option<ExternalLockfile>> {
        Ok(self.state.borrow().lockfile.clone())
    }

    fn prepare_update(
        &self,
        manifest: &ExternalManifest,
        lockfile: &ExternalLockfile,
    ) -> crate::external::ExternalResult<Self::Prepared> {
        let mut state = self.state.borrow_mut();
        state.prepare_count += 1;
        state.prepared_manifest = Some(manifest.clone());
        state.prepared_lockfile = Some(lockfile.clone());
        Ok(MemoryPreparedExternalUpdate {
            state: Rc::clone(&self.state),
            manifest: manifest.clone(),
            lockfile: lockfile.clone(),
        })
    }
}

impl ExternalPreparedUpdate for MemoryPreparedExternalUpdate {
    fn commit(self) -> crate::external::ExternalResult<()> {
        let mut state = self.state.borrow_mut();
        state.manifest = self.manifest;
        state.lockfile = Some(self.lockfile);
        state.commit_count += 1;
        Ok(())
    }
}

#[derive(Default)]
struct RecordingExternalMaterializer {
    calls: RefCell<Vec<ExternalLockfile>>,
    fail: Option<ExternalError>,
}

impl RecordingExternalMaterializer {
    fn failing(code: &str) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fail: Some(ExternalError::with_code(code, "materialization failed")),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.borrow().len()
    }
}

impl ExternalMaterializer for RecordingExternalMaterializer {
    fn materialize_lockfile(
        &self,
        lockfile: &ExternalLockfile,
        options: &ExternalMaterializationOptions,
    ) -> crate::external::ExternalResult<ExternalMaterializationReport> {
        self.calls.borrow_mut().push(lockfile.clone());
        if let Some(fail) = &self.fail {
            return Err(fail.clone());
        }
        options.reject_forbidden_local_links()?;
        let entries = lockfile
            .sorted_nodes()
            .iter()
            .map(|node| {
                if options.no_recursive && !node.parent_path.is_empty() {
                    ExternalMaterializationEntry::from_node(
                        node,
                        ExternalMaterializationState::SkippedNoRecursive,
                    )
                } else {
                    ExternalMaterializationEntry::from_node(
                        node,
                        ExternalMaterializationState::Materialized,
                    )
                }
            })
            .collect();
        Ok(ExternalMaterializationReport { entries })
    }
}

#[test]
fn toml_manifest_codec_parses_direct_external_with_rust_and_python_bindings() {
    let manifest = parse_manifest_fixture("rust-python/ait-external.toml");

    assert_eq!(manifest.externals.len(), 1);
    let external = &manifest.externals[0];
    assert_eq!(external.name, "ait-db");
    assert_eq!(external.repo_name, "ait-db");
    assert_eq!(external.repository_index, 11);
    assert_eq!(external.snapshot, "SNP-DB-RUST-PYTHON");
    assert_eq!(external.version.as_deref(), Some("0.2.0"));
    assert_eq!(
        external.bindings.rust.as_ref().unwrap().package.as_deref(),
        Some("ait-db")
    );
    assert_eq!(
        external.bindings.python.as_ref().unwrap().module.as_deref(),
        Some("ait_db")
    );
}

#[test]
fn toml_manifest_codec_parses_recursive_external_fixture_declarations() {
    let root = parse_manifest_fixture("recursive/ait-core/ait-external.toml");
    let nested = parse_manifest_fixture("recursive/ait-db/ait-external.toml");

    assert_eq!(root.externals.len(), 1);
    assert_eq!(root.externals[0].name, "ait-db");
    assert_eq!(root.externals[0].repository_index, 11);
    assert_eq!(root.externals[0].snapshot, "SNP-DB-RECURSIVE");
    assert_eq!(nested.externals.len(), 1);
    assert_eq!(nested.externals[0].name, "ait-codec");
    assert_eq!(nested.externals[0].repository_index, 12);
    assert_eq!(
        nested.externals[0].materialize_to,
        ".ait-external/ait-codec"
    );
}

#[test]
fn toml_manifest_codec_renders_round_trip_manifest() {
    let codec = TomlExternalManifestCodec;
    let original = parse_manifest_fixture("node-go/ait-external.toml");

    let rendered = codec.render_manifest(&original).unwrap();
    let reparsed = codec.parse_manifest(&rendered).unwrap();

    assert_eq!(reparsed, original);
    assert!(String::from_utf8(rendered)
        .unwrap()
        .contains("[[external]]"));
}

#[test]
fn sprint0_direct_manifest_fixture_is_offline_and_reproducible() {
    let manifest = parse_manifest_fixture("direct/ait-external.toml");
    let external = &manifest.externals[0];

    assert_eq!(external.name, "ait-db");
    assert_eq!(external.materialize_to, ".ait-external/ait-db");
    assert_eq!(external.snapshot, "SNP-DB-DIRECT");
    assert_eq!(external.version.as_deref(), Some("0.1.0"));
}

#[test]
fn external_lock_codec_parses_direct_lockfile_with_binding_summary() {
    let lockfile = parse_lock_fixture("direct/ait-external.lock");
    let node = &lockfile.nodes[0];

    assert_eq!(lockfile.format, "ait.external.lock");
    assert_eq!(node.name, "ait-db");
    assert_eq!(node.repository_index, 11);
    assert_eq!(node.snapshot, "SNP-DB-DIRECT");
    assert_eq!(node.version.as_deref(), Some("0.1.0"));
    assert_eq!(
        node.bindings,
        vec![
            ExternalLockBindingSummary::new("rust", "cargo-path", "rust/crates/ait-db")
                .with_package(Some("ait-db".to_string()))
        ]
    );
}

#[test]
fn external_lock_codec_renders_deterministic_order_without_language_lockfiles() {
    let lockfile = ExternalLockfile::new(vec![
        ExternalLockNode {
            name: "ait-tools".to_string(),
            repo_name: "ait-tools".to_string(),
            repository_index: 13,
            remote: "origin".to_string(),
            line: "main".to_string(),
            snapshot: "SNP-TOOLS".to_string(),
            parent_path: String::new(),
            materialize_to: ".ait-external/ait-tools".to_string(),
            license: "Apache-2.0".to_string(),
            version: None,
            bindings: vec![],
        },
        ExternalLockNode {
            name: "ait-db".to_string(),
            repo_name: "ait-db".to_string(),
            repository_index: 11,
            remote: "origin".to_string(),
            line: "main".to_string(),
            snapshot: "SNP-DB".to_string(),
            parent_path: String::new(),
            materialize_to: ".ait-external/ait-db".to_string(),
            license: "Apache-2.0".to_string(),
            version: None,
            bindings: vec![ExternalLockBindingSummary::new(
                "rust",
                "cargo-path",
                "rust/crates/ait-db",
            )],
        },
    ]);

    let rendered =
        String::from_utf8(TomlExternalLockCodec.render_lockfile(&lockfile).unwrap()).unwrap();

    assert!(
        rendered.find("name = \"ait-db\"").unwrap()
            < rendered.find("name = \"ait-tools\"").unwrap()
    );
    assert!(rendered.contains("[[node.binding]]"));
    assert!(!rendered.contains("Cargo.lock"));
    assert!(!rendered.contains("package-lock"));
    assert!(!rendered.contains("poetry.lock"));
}

#[test]
fn external_lockfile_direct_manifest_builder_matches_direct_fixture() {
    let manifest = parse_manifest_fixture("direct/ait-external.toml");
    let expected = parse_lock_fixture("direct/ait-external.lock");

    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();

    assert_eq!(lockfile, expected);
    assert!(lockfile.is_locked_against_manifest(&manifest));
}

#[test]
fn external_binding_metadata_rejects_empty_manifest_and_lock_values() {
    let mut external = test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db");
    external.bindings.rust = Some(crate::external::manifest::ExternalRustBinding {
        kind: "cargo-path".to_string(),
        path: "rust/crates/ait-db".to_string(),
        package: Some(" ".to_string()),
    });
    let manifest_err = test_manifest(vec![external]).validate().unwrap_err();

    assert!(manifest_err.to_string().contains("rust binding package"));

    let mut node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    node.bindings = vec![ExternalLockBindingSummary::new("go", "replace-path", "go")
        .with_module(Some(" ".to_string()))];
    let lock_err = ExternalLockfile::new(vec![node]).validate().unwrap_err();

    assert!(lock_err.to_string().contains("binding module"));
}

#[test]
fn external_lockfile_parses_recursive_dag_and_keeps_parent_path_identity() {
    let lockfile = parse_lock_fixture("recursive/ait-external.lock");

    assert_eq!(lockfile.nodes.len(), 2);
    assert_eq!(lockfile.nodes[0].name, "ait-db");
    assert_eq!(lockfile.nodes[0].parent_path, "");
    assert_eq!(lockfile.nodes[1].name, "ait-codec");
    assert_eq!(lockfile.nodes[1].parent_path, ".ait-external/ait-db");
    assert_eq!(
        lockfile.nodes[1].materialize_to,
        ".ait-external/ait-db/.ait-external/ait-codec"
    );
}

#[test]
fn external_lockfile_parses_duplicate_names_under_different_parents() {
    let lockfile = parse_lock_fixture("duplicate-names/ait-external.lock");
    let codec_nodes = lockfile
        .nodes
        .iter()
        .filter(|node| node.name == "ait-codec")
        .collect::<Vec<_>>();

    assert_eq!(codec_nodes.len(), 2);
    assert_ne!(codec_nodes[0].parent_path, codec_nodes[1].parent_path);
    assert_ne!(codec_nodes[0].snapshot, codec_nodes[1].snapshot);
}

#[test]
fn external_lockfile_locked_check_reports_mismatched_source_index_and_snapshot() {
    let manifest = parse_manifest_fixture("drift/ait-external.toml");
    let lockfile = parse_lock_fixture("drift/ait-external.lock");

    let drifts = lockfile.locked_drift_against_manifest(&manifest);

    assert_eq!(drifts.len(), 2);
    let repository_index = drifts
        .iter()
        .find(|drift| drift.field.as_deref() == Some("repository_index"))
        .expect("repository index drift");
    assert_eq!(repository_index.kind, ExternalLockDriftKind::Mismatch);
    assert_eq!(repository_index.to_json_value()["manifest_value"], "11");
    assert_eq!(repository_index.to_json_value()["lock_value"], "99");
    let snapshot = drifts
        .iter()
        .find(|drift| drift.field.as_deref() == Some("snapshot"))
        .expect("snapshot drift");
    assert_eq!(snapshot.kind, ExternalLockDriftKind::Mismatch);
    assert_eq!(
        snapshot.to_json_value()["manifest_value"],
        "SNP-DB-MANIFEST"
    );
    assert_eq!(snapshot.to_json_value()["lock_value"], "SNP-DB-LOCK");
}

#[test]
fn external_release_closure_metadata_decodes_lockfile_in_core() {
    let bytes = read_fixture_text("recursive/ait-external.lock");

    let metadata = external_release_closure_metadata_from_lockfile_bytes(bytes.as_bytes()).unwrap();

    assert_eq!(metadata["source"], "ait-external.lock");
    assert_eq!(metadata["summary"]["root_count"], 1);
    assert_eq!(metadata["summary"]["node_count"], 2);
    assert_eq!(metadata["canonical_snapshots"][0]["identity"], "ait-db");
    assert_eq!(metadata["canonical_snapshots"][0]["repository_index"], 11);
    assert_eq!(
        metadata["canonical_snapshots"][1]["identity"],
        ".ait-external/ait-db:ait-codec"
    );
}

#[test]
fn external_release_closure_metadata_treats_version_labels_as_metadata_only() {
    let original = read_fixture_text("direct/ait-external.lock");
    let relabeled = original.replace("version = \"0.1.0\"", "version = \"9.9.9\"");

    let original_metadata =
        external_release_closure_metadata_from_lockfile_bytes(original.as_bytes()).unwrap();
    let relabeled_metadata =
        external_release_closure_metadata_from_lockfile_bytes(relabeled.as_bytes()).unwrap();

    assert_eq!(
        original_metadata["canonical_snapshots"],
        relabeled_metadata["canonical_snapshots"]
    );
    assert_eq!(
        original_metadata["version_labels"][0]["snapshot"],
        relabeled_metadata["version_labels"][0]["snapshot"]
    );
    assert_ne!(
        original_metadata["version_labels"][0]["version"],
        relabeled_metadata["version_labels"][0]["version"]
    );
}

#[test]
fn external_lockfile_locked_check_reports_missing_and_extra_direct_nodes() {
    let manifest = ExternalManifest {
        externals: vec![ExternalDeclaration {
            name: "ait-db".to_string(),
            repo_name: "ait-db".to_string(),
            repository_index: 11,
            remote: "origin".to_string(),
            line: "main".to_string(),
            snapshot: "SNP-DB".to_string(),
            materialize_to: ".ait-external/ait-db".to_string(),
            license: "Apache-2.0".to_string(),
            version: None,
            bindings: Default::default(),
        }],
    };
    let lockfile = ExternalLockfile::new(vec![ExternalLockNode {
        name: "ait-tools".to_string(),
        repo_name: "ait-tools".to_string(),
        repository_index: 13,
        remote: "origin".to_string(),
        line: "main".to_string(),
        snapshot: "SNP-TOOLS".to_string(),
        parent_path: String::new(),
        materialize_to: ".ait-external/ait-tools".to_string(),
        license: "Apache-2.0".to_string(),
        version: None,
        bindings: vec![],
    }]);

    let drifts = lockfile.locked_drift_against_manifest(&manifest);
    let kinds = drifts.iter().map(|drift| drift.kind).collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![ExternalLockDriftKind::Missing, ExternalLockDriftKind::Extra]
    );
}

#[test]
fn external_lockfile_json_facts_are_stable_for_status_and_doctor_consumers() {
    let lockfile = parse_lock_fixture("recursive/ait-external.lock");
    let json = lockfile.to_json_value();

    assert_eq!(json["format"], "ait.external.lock");
    assert_eq!(json["summary"]["node_count"], 2);
    assert_eq!(json["summary"]["root_count"], 1);
    assert_eq!(json["nodes"][1]["name"], "ait-codec");
    assert_eq!(json["nodes"][1]["bindings"][0]["language"], "rust");
    assert_eq!(json["nodes"][1]["bindings"][0]["kind"], "cargo-path");
    assert_eq!(json["nodes"][1]["bindings"][0]["package"], "ait-codec");
}

#[test]
fn external_resolver_exact_snapshot_does_not_read_line_heads() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_without_manifest("ait-db", "SNP-DB-EXACT");

    let lockfile = resolve_external_lockfile(
        &resolver,
        &manifest,
        &ExternalResolutionOptions::exact("ait-db", "SNP-DB-EXACT"),
    )
    .unwrap();

    assert_eq!(lockfile.nodes.len(), 1);
    assert_eq!(lockfile.nodes[0].snapshot, "SNP-DB-EXACT");
    assert!(!resolver
        .calls()
        .iter()
        .any(|call| matches!(call, MemoryExternalResolverCall::LineHeadSnapshot { .. })));
}

#[test]
fn external_resolver_latest_uses_manifest_remote_and_line() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_line_head("ait-db", "origin", "main", "SNP-DB-LATEST")
        .with_snapshot_without_manifest("ait-db", "SNP-DB-LATEST");

    let lockfile = resolve_external_lockfile(
        &resolver,
        &manifest,
        &ExternalResolutionOptions::latest("ait-db"),
    )
    .unwrap();

    assert_eq!(lockfile.nodes[0].snapshot, "SNP-DB-LATEST");
    assert!(resolver
        .calls()
        .contains(&MemoryExternalResolverCall::LineHeadSnapshot {
            repository_index: 0,
            repo_name: "ait-db".to_string(),
            remote: "origin".to_string(),
            line: "main".to_string(),
        }));
}

#[test]
fn external_resolver_missing_snapshot_is_not_materialization_error() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let resolver = MemoryExternalSnapshotResolver::default();

    let err = resolve_external_lockfile(
        &resolver,
        &manifest,
        &ExternalResolutionOptions::manifest_pins(),
    )
    .unwrap_err();

    assert_eq!(err.code(), "external_snapshot_missing");
    assert!(!err.code().contains("materializ"));
}

#[test]
fn external_resolver_reads_nested_manifests_into_lockfile_closure() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-RECURSIVE",
        ".ait-external/ait-db",
    )]);
    let nested_manifest = test_manifest(vec![test_external(
        "ait-codec",
        "ait-codec",
        "SNP-CODEC-RECURSIVE",
        ".ait-external/ait-codec",
    )]);
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_manifest("ait-db", "SNP-DB-RECURSIVE", nested_manifest)
        .with_snapshot_without_manifest("ait-codec", "SNP-CODEC-RECURSIVE");

    let lockfile = resolve_external_lockfile(
        &resolver,
        &manifest,
        &ExternalResolutionOptions::manifest_pins(),
    )
    .unwrap();

    assert_eq!(lockfile.nodes.len(), 2);
    assert_eq!(lockfile.nodes[0].name, "ait-db");
    assert_eq!(lockfile.nodes[0].parent_path, "");
    assert_eq!(lockfile.nodes[1].name, "ait-codec");
    assert_eq!(lockfile.nodes[1].parent_path, ".ait-external/ait-db");
    assert_eq!(
        lockfile.nodes[1].materialize_to,
        ".ait-external/ait-db/.ait-external/ait-codec"
    );
    assert!(resolver
        .calls()
        .contains(&MemoryExternalResolverCall::SnapshotManifest {
            repository_index: 0,
            repo_name: "ait-db".to_string(),
            snapshot: "SNP-DB-RECURSIVE".to_string(),
        }));
}

#[test]
fn external_resolver_remote_ready_requires_snapshot_on_required_remote() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_without_manifest("ait-db", "SNP-DB-DIRECT");

    let err = resolve_external_lockfile(
        &resolver,
        &manifest,
        &ExternalResolutionOptions::manifest_pins().with_remote_ready(true),
    )
    .unwrap_err();

    assert_eq!(err.code(), "external_snapshot_remote_unavailable");
    assert!(err.message().contains("exists locally"));
    assert!(resolver
        .calls()
        .contains(&MemoryExternalResolverCall::SnapshotAvailableFromRemote {
            repository_index: 0,
            repo_name: "ait-db".to_string(),
            remote: "origin".to_string(),
            snapshot: "SNP-DB-DIRECT".to_string(),
        }));
}

#[test]
fn external_resolver_duplicate_names_keep_parent_specific_nested_paths() {
    let manifest = parse_manifest_fixture("duplicate-names/ait-core/ait-external.toml");
    let nested_db = parse_manifest_fixture("duplicate-names/ait-db/ait-external.toml");
    let nested_tools = parse_manifest_fixture("duplicate-names/ait-tools/ait-external.toml");
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_manifest_at(11, "ait-db", "SNP-DB-DUPLICATE-PARENT", nested_db)
        .with_snapshot_manifest_at(13, "ait-tools", "SNP-TOOLS-DUPLICATE-PARENT", nested_tools)
        .with_snapshot_without_manifest_at(12, "ait-codec", "SNP-CODEC-FOR-DB")
        .with_snapshot_without_manifest_at(15, "ait-codec", "SNP-CODEC-FOR-TOOLS");

    let lockfile = resolve_external_lockfile(
        &resolver,
        &manifest,
        &ExternalResolutionOptions::manifest_pins(),
    )
    .unwrap();
    let codec_nodes = lockfile
        .nodes
        .iter()
        .filter(|node| node.name == "ait-codec")
        .collect::<Vec<_>>();

    assert_eq!(codec_nodes.len(), 2);
    assert!(codec_nodes.iter().any(|node| {
        node.parent_path == ".ait-external/ait-db"
            && node.materialize_to == ".ait-external/ait-db/.ait-external/ait-codec"
            && node.snapshot == "SNP-CODEC-FOR-DB"
            && node.repository_index == 12
    }));
    assert!(codec_nodes.iter().any(|node| {
        node.parent_path == ".ait-external/ait-tools"
            && node.materialize_to == ".ait-external/ait-tools/.ait-external/ait-codec"
            && node.snapshot == "SNP-CODEC-FOR-TOOLS"
            && node.repository_index == 15
    }));
}

#[test]
fn external_resolver_remote_ready_accepts_exact_snapshot_on_required_remote() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_without_manifest("ait-db", "SNP-DB-REMOTE")
        .with_remote_snapshot("ait-db", "origin", "SNP-DB-REMOTE");

    let lockfile = resolve_external_lockfile(
        &resolver,
        &manifest,
        &ExternalResolutionOptions::exact("ait-db", "SNP-DB-REMOTE").with_remote_ready(true),
    )
    .unwrap();

    assert_eq!(lockfile.nodes[0].snapshot, "SNP-DB-REMOTE");
}

#[test]
fn external_materializer_writes_marker_and_is_idempotent_for_direct_node() {
    let temp = tempfile::tempdir().unwrap();
    let lockfile = ExternalLockfile::new(vec![ExternalLockNode::from_direct_declaration(
        &test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db"),
    )]);
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();

    let first = materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();
    let second = materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();

    let target = temp.path().join(".ait-external/ait-db");
    assert!(target.join(EXTERNAL_MATERIALIZER_MARKER).is_file());
    assert!(target.join("AIT_EXTERNAL_SNAPSHOT").is_file());
    let marker = ExternalMaterializerMarkerJson::filesystem()
        .read_marker(&target.join(EXTERNAL_MATERIALIZER_MARKER))
        .unwrap();
    let ExternalMaterializerMarkerRecord::V3(marker) = marker else {
        panic!("materialized external marker should use v3 format");
    };
    assert_eq!(marker.repository_index, 0);
    assert_eq!(marker.files.len(), 1);
    assert_eq!(marker.files[0].path, "AIT_EXTERNAL_SNAPSHOT");
    assert_eq!(
        first.entries[0].state,
        ExternalMaterializationState::Materialized
    );
    assert_eq!(
        second.entries[0].state,
        ExternalMaterializationState::Materialized
    );
}

#[test]
fn external_materializer_marker_json_writes_pretty_shape_without_trailing_newline() {
    let store = FakeExternalMaterializerMarkerFileIoStore::default();
    let marker_json = ExternalMaterializerMarkerJson::new(&store);
    let path = PathBuf::from("/repo/.ait-external/ait-db/.ait-external-marker.json");
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    let file = ExternalMaterializerMarkerFileEntry::new(
        "AIT_EXTERNAL_SNAPSHOT",
        sha256_hex(b"name=ait-db\nrepo_name=ait-db\nsnapshot=SNP-DB-DIRECT\n"),
    );

    marker_json.write_marker(&path, &node, &[file]).unwrap();

    let writes = store.writes.borrow();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, path);
    assert!(!writes[0].1.ends_with('\n'));
    let payload: crate::json_support::JsonValue =
        crate::json_support::JsonCodec::parse_value_with_error_prefix(
            &writes[0].1,
            "Failed to parse marker JSON",
        )
        .unwrap();
    assert_eq!(payload["format"], EXTERNAL_MATERIALIZER_MARKER_FORMAT);
    assert_eq!(payload["version"], EXTERNAL_MATERIALIZER_MARKER_VERSION);
    assert_eq!(payload["repository_index"], 0);
    assert_eq!(payload["snapshot"], "SNP-DB-DIRECT");
    assert_eq!(payload["files"][0]["path"], "AIT_EXTERNAL_SNAPSHOT");
    let marker = marker_json.read_marker(&writes[0].0).unwrap();
    let ExternalMaterializerMarkerRecord::V3(marker) = marker else {
        panic!("marker JSON should round-trip through v3");
    };
    assert_eq!(marker.snapshot, "SNP-DB-DIRECT");
    assert_eq!(marker.files.len(), 1);
    assert_eq!(marker.files[0].path, "AIT_EXTERNAL_SNAPSHOT");
}

#[test]
fn external_materializer_marker_json_preserves_status_parse_error_prefix() {
    let store = FakeExternalMaterializerMarkerFileIoStore::default();
    let marker_path = PathBuf::from("/repo/.ait-external/ait-db/.ait-external-marker.json");
    store.insert_file(marker_path.clone(), "{");

    let err = ExternalMaterializerMarkerJson::new(&store)
        .read_marker(&marker_path)
        .unwrap_err();

    assert_eq!(err.code(), "external_status_marker");
    assert!(err
        .message()
        .starts_with("failed to parse external materialization marker: "));
}

#[test]
fn external_materializer_preserves_recursive_parent_paths_and_no_recursive_skip() {
    let temp = tempfile::tempdir().unwrap();
    let lockfile = ExternalLockfile::new(vec![
        ExternalLockNode::from_direct_declaration(&test_external(
            "ait-db",
            "ait-db",
            "SNP-DB-RECURSIVE",
            ".ait-external/ait-db",
        )),
        {
            let mut node = ExternalLockNode::from_direct_declaration(&test_external(
                "ait-codec",
                "ait-codec",
                "SNP-CODEC-RECURSIVE",
                ".ait-external/ait-codec",
            ));
            node.parent_path = ".ait-external/ait-db".to_string();
            node.materialize_to = ".ait-external/ait-db/.ait-external/ait-codec".to_string();
            node
        },
    ]);
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();

    let no_recursive = materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::no_recursive())
        .unwrap();
    assert_eq!(no_recursive.entries.len(), 2);
    assert_eq!(
        no_recursive.entries[1].state,
        ExternalMaterializationState::SkippedNoRecursive
    );
    assert!(!temp
        .path()
        .join(".ait-external/ait-db/.ait-external/ait-codec")
        .exists());

    let recursive = materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();
    assert!(recursive
        .entries
        .iter()
        .all(|entry| entry.state == ExternalMaterializationState::Materialized));
    assert!(temp
        .path()
        .join(".ait-external/ait-db/.ait-external/ait-codec")
        .join(EXTERNAL_MATERIALIZER_MARKER)
        .is_file());
}

#[test]
fn external_materializer_rejects_path_traversal() {
    let temp = tempfile::tempdir().unwrap();
    let lockfile = ExternalLockfile::new(vec![ExternalLockNode::from_direct_declaration(
        &test_external("ait-db", "ait-db", "SNP-DB-DIRECT", "../ait-db"),
    )]);
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();

    let err = materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap_err();

    assert_eq!(err.code(), "external_error");
    assert!(err.message().contains("must not escape"));
}

#[test]
fn external_materializer_duplicate_parent_paths_materialize_independently_without_dedupe() {
    let temp = tempfile::tempdir().unwrap();
    let lockfile = parse_lock_fixture("duplicate-names/ait-external.lock");
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();

    let report = materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();

    assert_eq!(report.entries.len(), 4);
    assert!(report
        .entries
        .iter()
        .all(|entry| entry.state == ExternalMaterializationState::Materialized));
    assert_eq!(
        read_materialized_snapshot_fixture(
            temp.path(),
            ".ait-external/ait-db/.ait-external/ait-codec"
        ),
        "name=ait-codec\nrepo_name=ait-codec\nsnapshot=SNP-CODEC-FOR-DB\n"
    );
    assert_eq!(
        read_materialized_snapshot_fixture(
            temp.path(),
            ".ait-external/ait-tools/.ait-external/ait-codec"
        ),
        "name=ait-codec\nrepo_name=ait-codec\nsnapshot=SNP-CODEC-FOR-TOOLS\n"
    );
    assert!(!temp.path().join(".ait-external/ait-codec").exists());
}

#[cfg(unix)]
#[test]
fn external_materializer_rejects_symlink_traversal() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join(".ait-external")).unwrap();
    let lockfile = ExternalLockfile::new(vec![ExternalLockNode::from_direct_declaration(
        &test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db"),
    )]);
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();

    let err = materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap_err();

    assert_eq!(err.code(), "external_materializer_symlink");
}

#[cfg(unix)]
#[test]
fn external_materializer_refuses_generated_directory_with_nested_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = temp.path().join(".ait-external/ait-db");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join(EXTERNAL_MATERIALIZER_MARKER), "{}").unwrap();
    std::os::unix::fs::symlink(outside.path(), target.join("linked-outside")).unwrap();
    let lockfile = ExternalLockfile::new(vec![ExternalLockNode::from_direct_declaration(
        &test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db"),
    )]);
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();

    let err = materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap_err();

    assert_eq!(err.code(), "external_materializer_symlink");
    assert!(target.join("linked-outside").exists());
}

#[test]
fn external_materializer_refuses_dirty_hand_maintained_directory() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join(".ait-external/ait-db")).unwrap();
    std::fs::write(
        temp.path().join(".ait-external/ait-db/README.md"),
        "manual\n",
    )
    .unwrap();
    let lockfile = ExternalLockfile::new(vec![ExternalLockNode::from_direct_declaration(
        &test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db"),
    )]);
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();

    let err = materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap_err();

    assert_eq!(err.code(), "external_materializer_dirty_directory");
    assert!(temp.path().join(".ait-external/ait-db/README.md").is_file());
}

#[test]
fn external_materializer_rejects_local_links_for_locked_or_release_ready_modes() {
    let temp = tempfile::tempdir().unwrap();
    let lockfile = ExternalLockfile::new(vec![ExternalLockNode::from_direct_declaration(
        &test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db"),
    )]);
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();

    let locked_err = materializer
        .materialize_lockfile(
            &lockfile,
            &ExternalMaterializationOptions::recursive()
                .with_locked(true)
                .with_local_link_override("ait-db", "../ait-db"),
        )
        .unwrap_err();
    let release_err = materializer
        .materialize_lockfile(
            &lockfile,
            &ExternalMaterializationOptions::recursive()
                .with_release_ready(true)
                .with_local_link_override("ait-db", "../ait-db"),
        )
        .unwrap_err();

    assert_eq!(locked_err.code(), "external_local_link_forbidden");
    assert_eq!(release_err.code(), "external_local_link_forbidden");
}

#[test]
fn external_materializer_skips_local_links_for_plain_update() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-LINKED",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();

    let report = materializer
        .materialize_lockfile(
            &lockfile,
            &ExternalMaterializationOptions::recursive()
                .with_local_link_override("ait-db", "../ait-db"),
        )
        .unwrap();

    assert_eq!(report.entries.len(), 1);
    assert_eq!(
        report.entries[0].state,
        ExternalMaterializationState::SkippedLocalLink
    );
    assert!(!temp.path().join(".ait-external/ait-db").exists());
    let json = report.to_json_value();
    assert_eq!(json["entries"][0]["state"], "skipped_local_link");
    assert_eq!(json["summary"]["skipped_count"], 1);
}

#[test]
fn external_update_orchestrator_plain_update_reconciles_without_floating_to_latest() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-PINNED",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let store = MemoryExternalUpdateStore::new(manifest, Some(lockfile));
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_without_manifest("ait-db", "SNP-DB-PINNED")
        .with_snapshot_without_manifest("ait-db", "SNP-DB-LATEST")
        .with_line_head("ait-db", "origin", "main", "SNP-DB-LATEST");
    let materializer = RecordingExternalMaterializer::default();

    let report = run_external_update(
        &store,
        &resolver,
        &materializer,
        &ExternalUpdateOptions::manifest_pins().with_locked(true),
    )
    .unwrap();

    assert!(report.changed_pins.is_empty());
    assert!(!report.manifest_changed);
    assert!(!report.lockfile_changed);
    assert_eq!(
        store.manifest_snapshot("ait-db").as_deref(),
        Some("SNP-DB-PINNED")
    );
    assert_eq!(store.prepare_count(), 0);
    assert_eq!(store.commit_count(), 0);
    assert_eq!(materializer.call_count(), 1);
    assert!(!resolver
        .calls()
        .iter()
        .any(|call| matches!(call, MemoryExternalResolverCall::LineHeadSnapshot { .. })));

    let states = report.states();
    assert!(states.unchanged);
    assert!(states.materialized);
    assert!(states.validation_required);
    let json = report.to_json_value();
    assert_eq!(json["states"]["unchanged"], true);
    assert_eq!(json["states"]["materialized"], true);
}

#[test]
fn external_update_orchestrator_locked_rejects_manifest_lock_drift() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-MANIFEST",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-LOCK",
        ".ait-external/ait-db",
    )]))
    .unwrap();
    let store = MemoryExternalUpdateStore::new(manifest, Some(lockfile));
    let resolver = MemoryExternalSnapshotResolver::default();
    let materializer = RecordingExternalMaterializer::default();

    let err = run_external_update(
        &store,
        &resolver,
        &materializer,
        &ExternalUpdateOptions::manifest_pins().with_locked(true),
    )
    .unwrap_err();

    assert_eq!(err.code(), "external_lock_drift");
    assert_eq!(store.prepare_count(), 0);
    assert_eq!(store.commit_count(), 0);
    assert_eq!(materializer.call_count(), 0);
}

#[test]
fn external_update_orchestrator_to_snapshot_changes_exactly_one_root_pin() {
    let manifest = test_manifest(vec![
        test_external("ait-db", "ait-db", "SNP-DB-OLD", ".ait-external/ait-db"),
        test_external(
            "ait-codec",
            "ait-codec",
            "SNP-CODEC-OLD",
            ".ait-external/ait-codec",
        ),
    ]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let store = MemoryExternalUpdateStore::new(manifest, Some(lockfile));
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_without_manifest("ait-db", "SNP-DB-NEW")
        .with_snapshot_without_manifest("ait-codec", "SNP-CODEC-OLD");
    let materializer = RecordingExternalMaterializer::default();

    let report = run_external_update(
        &store,
        &resolver,
        &materializer,
        &ExternalUpdateOptions::exact("ait-db", "SNP-DB-NEW"),
    )
    .unwrap();

    assert_eq!(report.changed_pins.len(), 1);
    assert_eq!(report.changed_pins[0].name, "ait-db");
    assert_eq!(report.changed_pins[0].previous_snapshot, "SNP-DB-OLD");
    assert_eq!(report.changed_pins[0].new_snapshot, "SNP-DB-NEW");
    assert_eq!(
        store.manifest_snapshot("ait-db").as_deref(),
        Some("SNP-DB-NEW")
    );
    assert_eq!(
        store.manifest_snapshot("ait-codec").as_deref(),
        Some("SNP-CODEC-OLD")
    );
    assert_eq!(store.lock_snapshot("ait-db").as_deref(), Some("SNP-DB-NEW"));
    assert_eq!(
        store.lock_snapshot("ait-codec").as_deref(),
        Some("SNP-CODEC-OLD")
    );
    assert!(report.manifest_changed);
    assert!(report.lockfile_changed);
    assert!(report.states().updated);
    assert_eq!(materializer.call_count(), 1);
    assert_eq!(store.commit_count(), 1);
}

#[test]
fn external_update_orchestrator_recomputes_recursive_closure_after_pin_change() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-OLD",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let store = MemoryExternalUpdateStore::new(manifest, Some(lockfile));
    let nested_manifest = test_manifest(vec![test_external(
        "ait-codec",
        "ait-codec",
        "SNP-CODEC-NEW",
        ".ait-external/ait-codec",
    )]);
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_manifest("ait-db", "SNP-DB-NEW", nested_manifest)
        .with_snapshot_without_manifest("ait-codec", "SNP-CODEC-NEW");
    let materializer = RecordingExternalMaterializer::default();

    let report = run_external_update(
        &store,
        &resolver,
        &materializer,
        &ExternalUpdateOptions::exact("ait-db", "SNP-DB-NEW"),
    )
    .unwrap();

    assert_eq!(report.changed_pins.len(), 1);
    assert_eq!(store.lock_snapshot("ait-db").as_deref(), Some("SNP-DB-NEW"));
    assert_eq!(
        store
            .lock_snapshot_at(".ait-external/ait-db", "ait-codec")
            .as_deref(),
        Some("SNP-CODEC-NEW")
    );
    assert_eq!(report.materialization.entries.len(), 2);
}

#[test]
fn external_update_orchestrator_latest_changes_pin_only_for_newer_line_head() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-OLD",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let unchanged_store = MemoryExternalUpdateStore::new(manifest.clone(), Some(lockfile.clone()));
    let unchanged_resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_without_manifest("ait-db", "SNP-DB-OLD")
        .with_line_head("ait-db", "origin", "main", "SNP-DB-OLD");
    let unchanged_materializer = RecordingExternalMaterializer::default();

    let unchanged_report = run_external_update(
        &unchanged_store,
        &unchanged_resolver,
        &unchanged_materializer,
        &ExternalUpdateOptions::latest("ait-db"),
    )
    .unwrap();

    assert!(unchanged_report.changed_pins.is_empty());
    assert!(unchanged_report.states().unchanged);
    assert_eq!(
        unchanged_store.manifest_snapshot("ait-db").as_deref(),
        Some("SNP-DB-OLD")
    );
    assert!(unchanged_resolver.calls().iter().any(|call| matches!(
        call,
        MemoryExternalResolverCall::LineHeadSnapshot {
            repository_index,
            repo_name,
            remote,
            line
        } if *repository_index == 0 && repo_name == "ait-db" && remote == "origin" && line == "main"
    )));

    let updated_store = MemoryExternalUpdateStore::new(manifest, Some(lockfile));
    let updated_resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_without_manifest("ait-db", "SNP-DB-NEW")
        .with_line_head("ait-db", "origin", "main", "SNP-DB-NEW");
    let updated_materializer = RecordingExternalMaterializer::default();

    let updated_report = run_external_update(
        &updated_store,
        &updated_resolver,
        &updated_materializer,
        &ExternalUpdateOptions::latest("ait-db"),
    )
    .unwrap();

    assert_eq!(updated_report.changed_pins.len(), 1);
    assert_eq!(
        updated_report.changed_pins[0].previous_snapshot,
        "SNP-DB-OLD"
    );
    assert_eq!(updated_report.changed_pins[0].new_snapshot, "SNP-DB-NEW");
    assert_eq!(
        updated_store.manifest_snapshot("ait-db").as_deref(),
        Some("SNP-DB-NEW")
    );
    assert!(updated_report.states().updated);
}

#[test]
fn external_update_orchestrator_latest_refreshes_lockfile_and_materialization() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-OLD",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let nested_manifest = test_manifest(vec![test_external(
        "ait-codec",
        "ait-codec",
        "SNP-CODEC-NEW",
        ".ait-external/ait-codec",
    )]);
    let store = MemoryExternalUpdateStore::new(manifest, Some(lockfile));
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_manifest("ait-db", "SNP-DB-NEW", nested_manifest)
        .with_snapshot_without_manifest("ait-codec", "SNP-CODEC-NEW")
        .with_line_head("ait-db", "origin", "main", "SNP-DB-NEW");
    let materializer = RecordingExternalMaterializer::default();

    let report = run_external_update(
        &store,
        &resolver,
        &materializer,
        &ExternalUpdateOptions::latest("ait-db"),
    )
    .unwrap();

    assert_eq!(report.changed_pins.len(), 1);
    assert_eq!(
        store.manifest_snapshot("ait-db").as_deref(),
        Some("SNP-DB-NEW")
    );
    assert_eq!(store.lock_snapshot("ait-db").as_deref(), Some("SNP-DB-NEW"));
    assert_eq!(
        store
            .lock_snapshot_at(".ait-external/ait-db", "ait-codec")
            .as_deref(),
        Some("SNP-CODEC-NEW")
    );
    assert_eq!(materializer.call_count(), 1);
    assert_eq!(report.materialization.entries.len(), 2);
    assert_eq!(report.materialization.entries[0].snapshot, "SNP-DB-NEW");
    assert_eq!(report.materialization.entries[1].snapshot, "SNP-CODEC-NEW");
    assert!(resolver.calls().iter().any(|call| matches!(
        call,
        MemoryExternalResolverCall::LineHeadSnapshot {
            repository_index,
            repo_name,
            remote,
            line
        } if *repository_index == 0 && repo_name == "ait-db" && remote == "origin" && line == "main"
    )));
}

#[test]
fn external_update_orchestrator_failed_resolution_leaves_store_and_materialization_untouched() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-OLD",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let store = MemoryExternalUpdateStore::new(manifest, Some(lockfile));
    let resolver = MemoryExternalSnapshotResolver::default();
    let materializer = RecordingExternalMaterializer::default();

    let err = run_external_update(
        &store,
        &resolver,
        &materializer,
        &ExternalUpdateOptions::exact("ait-db", "SNP-DB-MISSING"),
    )
    .unwrap_err();

    assert_eq!(err.code(), "external_snapshot_missing");
    assert_eq!(
        store.manifest_snapshot("ait-db").as_deref(),
        Some("SNP-DB-OLD")
    );
    assert_eq!(store.lock_snapshot("ait-db").as_deref(), Some("SNP-DB-OLD"));
    assert_eq!(store.prepare_count(), 0);
    assert_eq!(store.commit_count(), 0);
    assert_eq!(materializer.call_count(), 0);
}

#[test]
fn external_update_orchestrator_materialization_failure_does_not_commit_staged_writes() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-OLD",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let store = MemoryExternalUpdateStore::new(manifest, Some(lockfile));
    let resolver = MemoryExternalSnapshotResolver::default()
        .with_snapshot_without_manifest("ait-db", "SNP-DB-NEW");
    let materializer = RecordingExternalMaterializer::failing("external_materializer_failed");

    let err = run_external_update(
        &store,
        &resolver,
        &materializer,
        &ExternalUpdateOptions::exact("ait-db", "SNP-DB-NEW"),
    )
    .unwrap_err();

    assert_eq!(err.code(), "external_materializer_failed");
    assert_eq!(
        store.manifest_snapshot("ait-db").as_deref(),
        Some("SNP-DB-OLD")
    );
    assert_eq!(store.lock_snapshot("ait-db").as_deref(), Some("SNP-DB-OLD"));
    assert_eq!(store.prepare_count(), 1);
    assert_eq!(store.commit_count(), 0);
    assert_eq!(materializer.call_count(), 1);
}

#[test]
fn external_status_facts_report_linked_missing_dirty_outdated_and_lock_drift() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-MANIFEST",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-LOCK",
        ".ait-external/ait-db",
    )]))
    .unwrap();
    let linked = build_external_status_report(
        ExternalStatusInput::new("ait-core", manifest, Some(lockfile))
            .with_local_link("ait-db", "../ait-db")
            .with_materialization(ExternalMaterializationObservation::dirty(
                "ait-db",
                "",
                ".ait-external/ait-db",
                "generated marker is missing",
            )),
    )
    .unwrap();

    assert_eq!(linked.externals.len(), 1);
    assert_eq!(linked.externals[0].state, ExternalStatusState::Linked);
    assert!(linked.externals[0].linked);
    assert!(linked.externals[0].lock_drift);
    assert_eq!(linked.summary.linked, 1);
    assert_eq!(linked.summary.lock_drift, 1);

    let missing = build_external_status_report(ExternalStatusInput::new(
        "ait-core",
        test_manifest(vec![test_external(
            "ait-db",
            "ait-db",
            "SNP-DB-DIRECT",
            ".ait-external/ait-db",
        )]),
        Some(ExternalLockfile::new(vec![
            ExternalLockNode::from_direct_declaration(&test_external(
                "ait-db",
                "ait-db",
                "SNP-DB-DIRECT",
                ".ait-external/ait-db",
            )),
        ])),
    ))
    .unwrap();
    assert_eq!(missing.externals[0].state, ExternalStatusState::Missing);
    assert_eq!(missing.summary.missing, 1);

    let outdated = build_external_status_report(
        ExternalStatusInput::new(
            "ait-core",
            test_manifest(vec![test_external(
                "ait-db",
                "ait-db",
                "SNP-DB-DIRECT",
                ".ait-external/ait-db",
            )]),
            Some(ExternalLockfile::new(vec![
                ExternalLockNode::from_direct_declaration(&test_external(
                    "ait-db",
                    "ait-db",
                    "SNP-DB-DIRECT",
                    ".ait-external/ait-db",
                )),
            ])),
        )
        .with_materialization(ExternalMaterializationObservation::generated(
            "ait-db",
            "",
            ".ait-external/ait-db",
            "SNP-DB-OLD",
        )),
    )
    .unwrap();
    assert_eq!(outdated.externals[0].state, ExternalStatusState::Outdated);
    assert_eq!(outdated.summary.outdated, 1);
}

#[test]
fn external_status_filesystem_inspection_reads_marker_and_binding_paths_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let mut external = test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db");
    external.bindings.rust = Some(crate::external::manifest::ExternalRustBinding {
        kind: "cargo-path".to_string(),
        path: "rust/crates/ait-db".to_string(),
        package: Some("ait-db".to_string()),
    });
    let manifest = test_manifest(vec![external]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();
    materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();
    std::fs::create_dir_all(temp.path().join(".ait-external/ait-db/rust/crates/ait-db")).unwrap();

    let report = inspect_external_status_report(
        temp.path(),
        "ait-core",
        manifest,
        Some(lockfile),
        Vec::new(),
    )
    .unwrap();

    assert_eq!(report.externals[0].state, ExternalStatusState::Materialized);
    assert_eq!(report.summary.missing, 0);
    assert_eq!(report.binding_checks.len(), 1);
    assert!(report.binding_checks[0].exists);
    assert!(report.binding_checks[0].supported);
}

#[test]
fn external_status_json_includes_gate_fields_for_ci_release_and_remote_ready() {
    let mut external = test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db");
    external.bindings.rust = Some(crate::external::manifest::ExternalRustBinding {
        kind: "cargo-path".to_string(),
        path: "rust/crates/ait-db".to_string(),
        package: Some("ait-db".to_string()),
    });
    let manifest = test_manifest(vec![external]);
    let binding = ExternalLockBindingSummary::new("rust", "cargo-path", "rust/crates/ait-db")
        .with_package(Some("ait-db".to_string()));

    let status = build_external_status_report(
        ExternalStatusInput::new("ait-core", manifest, None)
            .with_materialization(ExternalMaterializationObservation::generated(
                "ait-db",
                "",
                ".ait-external/ait-db",
                "SNP-DB-DIRECT",
            ))
            .with_binding_check(ExternalBindingCheckFact::new(
                "ait-db",
                "",
                ".ait-external/ait-db",
                &binding,
                ".ait-external/ait-db/rust/crates/ait-db",
                false,
            )),
    )
    .unwrap();
    let payload = status.to_json_value();

    assert_eq!(payload["externals"][0]["license"], "Apache-2.0");
    assert_eq!(payload["externals"][0]["bindings"][0]["language"], "rust");
    assert_eq!(payload["externals"][0]["bindings"][0]["package"], "ait-db");
    assert_eq!(payload["lock_drifts"][0]["kind"], "missing");
    assert_eq!(payload["lock_drifts"][0]["name"], "ait-db");
    assert_eq!(payload["binding_checks"][0]["language"], "rust");
    assert_eq!(payload["binding_checks"][0]["exists"], false);
    assert_eq!(payload["binding_checks"][0]["supported"], true);
    assert_eq!(payload["binding_checks"][0]["tool"], "cargo");
    assert_eq!(
        payload["binding_checks"][0]["toolchain"]["status"],
        "not_requested"
    );
}

#[test]
fn external_status_filesystem_loader_reads_core_manifest_lock_and_links() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    std::fs::write(
        temp.path().join("ait-external.toml"),
        TomlExternalManifestCodec
            .render_manifest(&manifest)
            .unwrap(),
    )
    .unwrap();
    std::fs::write(
        temp.path().join("ait-external.lock"),
        TomlExternalLockCodec.render_lockfile(&lockfile).unwrap(),
    )
    .unwrap();
    std::fs::write(
        temp.path().join(EXTERNAL_LINKS_FILE),
        render_external_local_link_overrides(&[ExternalLocalLinkOverride {
            name: "ait-db".to_string(),
            path: "../ait-db".to_string(),
        }])
        .unwrap(),
    )
    .unwrap();

    let loaded = inspect_external_filesystem_status_report(temp.path(), "ait-core").unwrap();

    assert!(loaded.manifest_present);
    assert_eq!(
        loaded.report.externals[0].state,
        ExternalStatusState::Linked
    );
    assert_eq!(loaded.report.summary.linked, 1);
}

#[test]
fn external_status_filesystem_loader_marks_missing_manifest_without_error() {
    let temp = tempfile::tempdir().unwrap();

    let loaded = inspect_external_filesystem_status_report(temp.path(), "ait-core").unwrap();

    assert!(!loaded.manifest_present);
    assert!(loaded.report.externals.is_empty());
    assert_eq!(loaded.report.summary.missing, 0);
}

#[test]
fn external_projection_roots_preserve_all_status_state_gates() {
    let clean = tempfile::tempdir().unwrap();
    materialize_external_status_fixture(clean.path(), "SNP-DB-DIRECT");
    assert_eq!(
        inspect_operational_external_projection_roots(clean.path(), "ait-core").unwrap(),
        vec![".ait-external/ait-db"]
    );
    let clean_file = clean
        .path()
        .join(".ait-external/ait-db/AIT_EXTERNAL_SNAPSHOT");
    let clean_bytes = std::fs::read(&clean_file).unwrap();
    let changed_same_size = vec![b'X'; clean_bytes.len()];
    assert_ne!(changed_same_size, clean_bytes);
    std::fs::write(&clean_file, changed_same_size).unwrap();
    assert!(
        inspect_operational_external_projection_roots(clean.path(), "ait-core")
            .unwrap()
            .is_empty()
    );

    let missing = tempfile::tempdir().unwrap();
    let missing_manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let missing_lock = ExternalLockfile::direct_manifest_lock(&missing_manifest).unwrap();
    write_external_status_authority(missing.path(), &missing_manifest, &missing_lock);
    assert!(
        inspect_operational_external_projection_roots(missing.path(), "ait-core")
            .unwrap()
            .is_empty()
    );

    let linked = tempfile::tempdir().unwrap();
    write_external_status_authority(linked.path(), &missing_manifest, &missing_lock);
    std::fs::write(
        linked.path().join(EXTERNAL_LINKS_FILE),
        render_external_local_link_overrides(&[ExternalLocalLinkOverride {
            name: "ait-db".to_string(),
            path: "../ait-db".to_string(),
        }])
        .unwrap(),
    )
    .unwrap();
    assert!(
        inspect_operational_external_projection_roots(linked.path(), "ait-core")
            .unwrap()
            .is_empty()
    );

    let outdated = tempfile::tempdir().unwrap();
    materialize_external_status_fixture(outdated.path(), "SNP-DB-OLD");
    let outdated_manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-NEW",
        ".ait-external/ait-db",
    )]);
    let outdated_lock = ExternalLockfile::direct_manifest_lock(&outdated_manifest).unwrap();
    write_external_status_authority(outdated.path(), &outdated_manifest, &outdated_lock);
    assert!(
        inspect_operational_external_projection_roots(outdated.path(), "ait-core")
            .unwrap()
            .is_empty()
    );

    let lock_drift = tempfile::tempdir().unwrap();
    let (_locked_manifest, locked_lock) =
        materialize_external_status_fixture(lock_drift.path(), "SNP-DB-LOCKED");
    let drifted_manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-MANIFEST",
        ".ait-external/ait-db",
    )]);
    write_external_status_authority(lock_drift.path(), &drifted_manifest, &locked_lock);
    assert!(
        inspect_operational_external_projection_roots(lock_drift.path(), "ait-core")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn external_status_filesystem_inspection_reports_missing_marker_as_dirty() {
    let temp = tempfile::tempdir().unwrap();
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    std::fs::create_dir_all(temp.path().join(".ait-external/ait-db")).unwrap();

    let observation = inspect_external_materialization(temp.path(), &node).unwrap();

    assert_eq!(
        observation.state,
        ExternalObservedMaterializationState::Dirty
    );
    assert_eq!(
        observation.reason.as_deref(),
        Some("generated marker is missing")
    );
}

#[test]
fn external_status_filesystem_inspection_errors_on_malformed_marker_json() {
    let temp = tempfile::tempdir().unwrap();
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    let target = temp.path().join(".ait-external/ait-db");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join(EXTERNAL_MATERIALIZER_MARKER), "{").unwrap();

    let err = inspect_external_materialization(temp.path(), &node).unwrap_err();

    assert_eq!(err.code(), "external_status_marker");
    assert!(err
        .message()
        .starts_with("failed to parse external materialization marker: "));
}

#[test]
fn external_status_filesystem_inspection_marks_legacy_marker_formats_dirty_until_refresh() {
    let temp = tempfile::tempdir().unwrap();
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    let marker_path = temp
        .path()
        .join(".ait-external/ait-db")
        .join(EXTERNAL_MATERIALIZER_MARKER);
    std::fs::create_dir_all(marker_path.parent().unwrap()).unwrap();

    std::fs::write(
        &marker_path,
        r#"{"format":"unknown","snapshot":"SNP-DB-DIRECT"}"#,
    )
    .unwrap();
    let unknown_format = inspect_external_materialization(temp.path(), &node).unwrap();
    assert_eq!(
        unknown_format.state,
        ExternalObservedMaterializationState::Dirty
    );
    assert_eq!(
        unknown_format.reason.as_deref(),
        Some("generated marker format requires refresh")
    );

    std::fs::write(
        &marker_path,
        r#"{"format":"ait.external.materialized","version":2,"snapshot":"SNP-DB-DIRECT"}"#,
    )
    .unwrap();
    let previous_identity_marker = inspect_external_materialization(temp.path(), &node).unwrap();
    assert_eq!(
        previous_identity_marker.state,
        ExternalObservedMaterializationState::Dirty
    );
    assert_eq!(
        previous_identity_marker.reason.as_deref(),
        Some("generated marker format requires refresh")
    );

    std::fs::write(&marker_path, r#"{"snapshot":"SNP-DB-DIRECT"}"#).unwrap();
    let old_marker = inspect_external_materialization(temp.path(), &node).unwrap();
    assert_eq!(
        old_marker.state,
        ExternalObservedMaterializationState::Dirty
    );
    assert_eq!(
        old_marker.reason.as_deref(),
        Some("generated marker format requires refresh")
    );
}

#[test]
fn external_status_filesystem_inspection_marks_modified_materialized_file_dirty() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();
    materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();
    std::fs::write(
        temp.path()
            .join(".ait-external/ait-db/AIT_EXTERNAL_SNAPSHOT"),
        "changed\n",
    )
    .unwrap();

    let report = inspect_external_status_report(
        temp.path(),
        "ait-core",
        manifest,
        Some(lockfile),
        Vec::new(),
    )
    .unwrap();

    assert_eq!(report.externals[0].state, ExternalStatusState::Dirty);
    assert_eq!(report.summary.dirty, 1);
}

#[test]
fn external_status_filesystem_inspection_marks_removed_materialized_file_dirty() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();
    materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();
    std::fs::remove_file(
        temp.path()
            .join(".ait-external/ait-db/AIT_EXTERNAL_SNAPSHOT"),
    )
    .unwrap();

    let report = inspect_external_status_report(
        temp.path(),
        "ait-core",
        manifest,
        Some(lockfile),
        Vec::new(),
    )
    .unwrap();

    assert_eq!(report.externals[0].state, ExternalStatusState::Dirty);
    assert_eq!(report.summary.dirty, 1);
}

#[test]
fn external_status_filesystem_inspection_marks_added_materialized_file_dirty() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let materializer =
        FilesystemExternalMaterializer::new(temp.path(), FixtureExternalContentSource).unwrap();
    materializer
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();
    std::fs::write(
        temp.path().join(".ait-external/ait-db/README.md"),
        "extra\n",
    )
    .unwrap();

    let report = inspect_external_status_report(
        temp.path(),
        "ait-core",
        manifest,
        Some(lockfile),
        Vec::new(),
    )
    .unwrap();

    assert_eq!(report.externals[0].state, ExternalStatusState::Dirty);
    assert_eq!(report.summary.dirty, 1);
}

#[test]
fn external_binding_validator_path_only_does_not_probe_toolchains() {
    let temp = tempfile::tempdir().unwrap();
    let mut external = test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db");
    external.bindings.rust = Some(crate::external::manifest::ExternalRustBinding {
        kind: "cargo-path".to_string(),
        path: "rust/crates/ait-db".to_string(),
        package: Some("ait-db".to_string()),
    });
    let manifest = test_manifest(vec![external]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    std::fs::create_dir_all(temp.path().join(".ait-external/ait-db/rust/crates/ait-db")).unwrap();
    let probe = RecordingExternalBindingToolProbe::new(ExternalBindingToolProbeResult::passed());
    let calls = probe.calls.clone();
    let checks = FilesystemExternalBindingValidator::new(probe)
        .check_bindings(ExternalBindingValidationRequest::path_only(
            temp.path(),
            &lockfile.nodes,
        ))
        .unwrap();

    assert!(checks[0].exists);
    assert_eq!(checks[0].toolchain.as_str(), "not_requested");
    assert!(calls.borrow().is_empty());
}

#[test]
fn external_binding_validator_trait_returns_doctor_findings() {
    let temp = tempfile::tempdir().unwrap();
    let mut external = test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db");
    external.bindings.python = Some(crate::external::manifest::ExternalPythonBinding {
        kind: "python-path".to_string(),
        path: "python".to_string(),
        package: Some("ait-db".to_string()),
        module: Some("ait_db".to_string()),
    });
    let manifest = test_manifest(vec![external]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let findings = FilesystemExternalBindingValidator::default()
        .validate_bindings(ExternalBindingValidationRequest::path_only(
            temp.path(),
            &lockfile.nodes,
        ))
        .unwrap();

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "external_binding_path_missing");
    assert!(!findings[0].release_blocking);
}

#[test]
fn external_binding_validator_toolchain_mode_reports_skipped_when_probe_missing() {
    let temp = tempfile::tempdir().unwrap();
    let mut external = test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db");
    external.bindings.rust = Some(crate::external::manifest::ExternalRustBinding {
        kind: "cargo-path".to_string(),
        path: "rust/crates/ait-db".to_string(),
        package: Some("ait-db".to_string()),
    });
    let manifest = test_manifest(vec![external]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    std::fs::create_dir_all(temp.path().join(".ait-external/ait-db/rust/crates/ait-db")).unwrap();

    let checks = FilesystemExternalBindingValidator::new(NoopExternalBindingToolProbe)
        .check_bindings(ExternalBindingValidationRequest::toolchain_probes(
            temp.path(),
            &lockfile.nodes,
        ))
        .unwrap();
    let status = build_external_status_report(
        ExternalStatusInput::new("ait-core", manifest, Some(lockfile))
            .with_materialization(ExternalMaterializationObservation::generated(
                "ait-db",
                "",
                ".ait-external/ait-db",
                "SNP-DB-DIRECT",
            ))
            .with_binding_check(checks[0].clone()),
    )
    .unwrap();
    let doctor = build_external_doctor_report(&status, &ExternalDoctorOptions::default());
    let codes = doctor
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<Vec<_>>();

    assert!(checks[0].toolchain_skipped());
    assert!(doctor.release_ready);
    assert!(codes.contains(&"external_binding_toolchain_skipped"));
    assert_eq!(doctor.warning_findings().len(), 1);
}

#[test]
fn external_binding_validator_probe_can_simulate_all_supported_language_tools() {
    let temp = tempfile::tempdir().unwrap();
    let mut external = test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db");
    external.bindings.rust = Some(crate::external::manifest::ExternalRustBinding {
        kind: "cargo-path".to_string(),
        path: "rust/crates/ait-db".to_string(),
        package: Some("ait-db".to_string()),
    });
    external.bindings.python = Some(crate::external::manifest::ExternalPythonBinding {
        kind: "python-path".to_string(),
        path: "python".to_string(),
        package: Some("ait-db".to_string()),
        module: Some("ait_db".to_string()),
    });
    external.bindings.node = Some(crate::external::manifest::ExternalNodeBinding {
        kind: "file-package".to_string(),
        path: "node".to_string(),
        package: Some("@ait/db".to_string()),
    });
    external.bindings.go = Some(crate::external::manifest::ExternalGoBinding {
        kind: "replace-path".to_string(),
        path: "go".to_string(),
        module: Some("ait.dev/db".to_string()),
    });
    let manifest = test_manifest(vec![external]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    for relative in [
        ".ait-external/ait-db/rust/crates/ait-db",
        ".ait-external/ait-db/python",
        ".ait-external/ait-db/node",
        ".ait-external/ait-db/go",
    ] {
        std::fs::create_dir_all(temp.path().join(relative)).unwrap();
    }
    let probe = RecordingExternalBindingToolProbe::new(ExternalBindingToolProbeResult::passed());
    let calls = probe.calls.clone();
    let checks = FilesystemExternalBindingValidator::new(probe)
        .check_bindings(ExternalBindingValidationRequest::toolchain_probes(
            temp.path(),
            &lockfile.nodes,
        ))
        .unwrap();
    let calls = calls.borrow().clone();

    assert_eq!(checks.len(), 4);
    assert!(checks.iter().all(|check| check.exists));
    assert!(checks
        .iter()
        .all(|check| check.toolchain.as_str() == "passed"));
    assert!(calls.contains(&"cargo:rust/crates/ait-db".to_string()));
    assert!(calls.contains(&"python:python".to_string()));
    assert!(calls.contains(&"node:node".to_string()));
    assert!(calls.contains(&"go:go".to_string()));
}

#[test]
fn command_external_binding_tool_probe_builds_rust_python_node_and_go_metadata_commands() {
    let temp = tempfile::tempdir().unwrap();
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    let cases = [
        (
            ExternalBindingTool::Cargo,
            ExternalLockBindingSummary::new("rust", "cargo-path", "rust/crates/ait-db")
                .with_package(Some("ait-db".to_string())),
            ExternalBindingCommandOutput::success(r#"{"packages":[{"name":"ait-db"}]}"#),
            "cargo",
        ),
        (
            ExternalBindingTool::Python,
            ExternalLockBindingSummary::new("python", "python-path", "python")
                .with_module(Some("ait_db".to_string())),
            ExternalBindingCommandOutput::success(""),
            "python3",
        ),
        (
            ExternalBindingTool::Node,
            ExternalLockBindingSummary::new("node", "file-package", "node")
                .with_package(Some("@ait/db".to_string())),
            ExternalBindingCommandOutput::success(""),
            "node",
        ),
        (
            ExternalBindingTool::Go,
            ExternalLockBindingSummary::new("go", "replace-path", "go")
                .with_module(Some("ait.dev/db".to_string())),
            ExternalBindingCommandOutput::success(r#"{"Path":"ait.dev/db"}"#),
            "go",
        ),
    ];

    for (tool, binding, output, expected_program) in cases {
        let runner = RecordingExternalBindingCommandRunner::new([output]);
        let calls = runner.calls.clone();
        let result = CommandExternalBindingToolProbe::new(runner)
            .probe_binding_tool(ExternalBindingToolProbeRequest {
                tool,
                node: &node,
                binding: &binding,
                binding_path: temp.path(),
            })
            .unwrap();
        let calls = calls.borrow();
        let command = calls.first().unwrap();

        assert_eq!(result.outcome.as_str(), "passed");
        assert_eq!(command.program, expected_program);
        match tool {
            ExternalBindingTool::Cargo => {
                assert!(command.args.contains(&"metadata".to_string()));
                assert!(command.args.contains(&"--manifest-path".to_string()));
                assert!(command.args.iter().any(|arg| arg.ends_with("Cargo.toml")));
            }
            ExternalBindingTool::Python => {
                assert_eq!(command.args.first().map(String::as_str), Some("-c"));
                assert_eq!(
                    command.args.get(2).map(String::as_str),
                    Some(temp.path().to_string_lossy().as_ref())
                );
                assert_eq!(command.args.get(3).map(String::as_str), Some("ait_db"));
            }
            ExternalBindingTool::Node => {
                assert_eq!(command.args.first().map(String::as_str), Some("-e"));
                assert_eq!(
                    command.args.get(2).map(String::as_str),
                    Some(temp.path().to_string_lossy().as_ref())
                );
                assert_eq!(command.args.get(3).map(String::as_str), Some("@ait/db"));
            }
            ExternalBindingTool::Go => {
                assert_eq!(command.args, vec!["list", "-m", "-json"]);
                assert_eq!(command.cwd.as_deref(), Some(temp.path()));
            }
        }
    }
}

#[test]
fn command_external_binding_tool_probe_skips_when_tool_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    let binding = ExternalLockBindingSummary::new("rust", "cargo-path", "rust/crates/ait-db");
    let result =
        CommandExternalBindingToolProbe::new(RecordingExternalBindingCommandRunner::new([
            ExternalBindingCommandOutput::not_found(),
        ]))
        .probe_binding_tool(ExternalBindingToolProbeRequest {
            tool: ExternalBindingTool::Cargo,
            node: &node,
            binding: &binding,
            binding_path: temp.path(),
        })
        .unwrap();

    assert_eq!(result.outcome.as_str(), "skipped");
    assert!(result
        .outcome
        .message()
        .unwrap()
        .contains("cargo metadata validation tool is not available"));
}

#[test]
fn command_external_binding_tool_probe_falls_back_from_python3_to_python() {
    let temp = tempfile::tempdir().unwrap();
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    let binding = ExternalLockBindingSummary::new("python", "python-path", "python");
    let runner = RecordingExternalBindingCommandRunner::new([
        ExternalBindingCommandOutput::not_found(),
        ExternalBindingCommandOutput::success(""),
    ]);
    let calls = runner.calls.clone();
    let result = CommandExternalBindingToolProbe::new(runner)
        .probe_binding_tool(ExternalBindingToolProbeRequest {
            tool: ExternalBindingTool::Python,
            node: &node,
            binding: &binding,
            binding_path: temp.path(),
        })
        .unwrap();
    let calls = calls.borrow();

    assert_eq!(result.outcome.as_str(), "passed");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].program, "python3");
    assert_eq!(calls[1].program, "python");
}

#[test]
fn command_external_binding_tool_probe_fails_invalid_metadata_output() {
    let temp = tempfile::tempdir().unwrap();
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    let binding = ExternalLockBindingSummary::new("rust", "cargo-path", "rust/crates/ait-db");
    let result =
        CommandExternalBindingToolProbe::new(RecordingExternalBindingCommandRunner::new([
            ExternalBindingCommandOutput::success("{}"),
        ]))
        .probe_binding_tool(ExternalBindingToolProbeRequest {
            tool: ExternalBindingTool::Cargo,
            node: &node,
            binding: &binding,
            binding_path: temp.path(),
        })
        .unwrap();

    assert_eq!(result.outcome.as_str(), "failed");
    assert!(result.outcome.message().unwrap().contains("packages"));
}

#[test]
fn command_external_binding_tool_probe_fails_mismatched_declared_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    let rust_binding = ExternalLockBindingSummary::new("rust", "cargo-path", "rust/crates/ait-db")
        .with_package(Some("ait-db".to_string()));
    let go_binding = ExternalLockBindingSummary::new("go", "replace-path", "go")
        .with_module(Some("ait.dev/db".to_string()));

    let cargo_result =
        CommandExternalBindingToolProbe::new(RecordingExternalBindingCommandRunner::new([
            ExternalBindingCommandOutput::success(r#"{"packages":[{"name":"other"}]}"#),
        ]))
        .probe_binding_tool(ExternalBindingToolProbeRequest {
            tool: ExternalBindingTool::Cargo,
            node: &node,
            binding: &rust_binding,
            binding_path: temp.path(),
        })
        .unwrap();
    let go_result =
        CommandExternalBindingToolProbe::new(RecordingExternalBindingCommandRunner::new([
            ExternalBindingCommandOutput::success(r#"{"Path":"ait.dev/other"}"#),
        ]))
        .probe_binding_tool(ExternalBindingToolProbeRequest {
            tool: ExternalBindingTool::Go,
            node: &node,
            binding: &go_binding,
            binding_path: temp.path(),
        })
        .unwrap();

    assert_eq!(cargo_result.outcome.as_str(), "failed");
    assert!(cargo_result.outcome.message().unwrap().contains("ait-db"));
    assert_eq!(go_result.outcome.as_str(), "failed");
    assert!(go_result.outcome.message().unwrap().contains("ait.dev/db"));
}

#[test]
fn command_external_binding_tool_probe_reports_tool_failures() {
    let temp = tempfile::tempdir().unwrap();
    let node = ExternalLockNode::from_direct_declaration(&test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    ));
    let binding = ExternalLockBindingSummary::new("node", "file-package", "node");
    let result =
        CommandExternalBindingToolProbe::new(RecordingExternalBindingCommandRunner::new([
            ExternalBindingCommandOutput::failure(1, "package.json is missing"),
        ]))
        .probe_binding_tool(ExternalBindingToolProbeRequest {
            tool: ExternalBindingTool::Node,
            node: &node,
            binding: &binding,
            binding_path: temp.path(),
        })
        .unwrap();

    assert_eq!(result.outcome.as_str(), "failed");
    assert!(result
        .outcome
        .message()
        .unwrap()
        .contains("package.json is missing"));
}

#[test]
fn external_binding_validator_toolchain_failure_is_release_blocking() {
    let temp = tempfile::tempdir().unwrap();
    let mut external = test_external("ait-db", "ait-db", "SNP-DB-DIRECT", ".ait-external/ait-db");
    external.bindings.rust = Some(crate::external::manifest::ExternalRustBinding {
        kind: "cargo-path".to_string(),
        path: "rust/crates/ait-db".to_string(),
        package: Some("ait-db".to_string()),
    });
    let manifest = test_manifest(vec![external]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    std::fs::create_dir_all(temp.path().join(".ait-external/ait-db/rust/crates/ait-db")).unwrap();
    let checks = FilesystemExternalBindingValidator::new(RecordingExternalBindingToolProbe::new(
        ExternalBindingToolProbeResult::failed("cargo metadata failed"),
    ))
    .check_bindings(ExternalBindingValidationRequest::toolchain_probes(
        temp.path(),
        &lockfile.nodes,
    ))
    .unwrap();
    let status = build_external_status_report(
        ExternalStatusInput::new("ait-core", manifest, Some(lockfile))
            .with_materialization(ExternalMaterializationObservation::generated(
                "ait-db",
                "",
                ".ait-external/ait-db",
                "SNP-DB-DIRECT",
            ))
            .with_binding_check(checks[0].clone()),
    )
    .unwrap();
    let doctor = build_external_doctor_report(&status, &ExternalDoctorOptions::default());
    let failure = doctor
        .findings
        .iter()
        .find(|finding| finding.code == "external_binding_toolchain_failed")
        .unwrap();

    assert!(checks[0].toolchain_failed());
    assert!(!doctor.release_ready);
    assert!(failure.release_blocking);
    assert!(failure.message.contains("cargo metadata failed"));
}

#[test]
fn external_status_filesystem_inspection_marks_unmarked_directory_dirty() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join(".ait-external/ait-db")).unwrap();
    std::fs::write(
        temp.path().join(".ait-external/ait-db/README.md"),
        "manual\n",
    )
    .unwrap();
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);

    let report =
        inspect_external_status_report(temp.path(), "ait-core", manifest, None, Vec::new())
            .unwrap();

    assert_eq!(report.externals[0].state, ExternalStatusState::Dirty);
    assert_eq!(report.summary.dirty, 1);
}

#[test]
fn external_readiness_allows_repositories_without_externals() {
    let status = build_external_status_report(ExternalStatusInput::new(
        "ait-core",
        ExternalManifest {
            externals: Vec::new(),
        },
        None,
    ))
    .unwrap();

    let readiness = build_external_readiness_report(&status);

    assert!(readiness.ready);
    assert!(readiness.blockers.is_empty());
    assert_eq!(readiness.to_json_value()["summary"]["blockers"], 0);
}

#[test]
fn external_readiness_reports_exact_blockers_for_remote_and_ci_gates() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let binding = ExternalLockBindingSummary::new("python", "python-path", "python");
    let missing_binding = ExternalBindingCheckFact::new(
        "ait-db",
        "",
        ".ait-external/ait-db",
        &binding,
        ".ait-external/ait-db/python",
        false,
    );
    let status = build_external_status_report(
        ExternalStatusInput::new("ait-core", manifest, Some(lockfile))
            .with_local_link("ait-db", "../ait-db")
            .with_binding_check(missing_binding),
    )
    .unwrap();

    let readiness = build_external_readiness_report(&status);
    let codes = readiness
        .blockers
        .iter()
        .map(|blocker| blocker.code.as_str())
        .collect::<Vec<_>>();

    assert!(!readiness.ready);
    assert!(codes.contains(&"external_local_link_active"));
    assert!(codes.contains(&"external_binding_path_missing"));
    assert_eq!(readiness.blockers[0].name.as_deref(), Some("ait-db"));
    assert!(readiness
        .to_json_value()
        .to_string()
        .contains(".ait-external/ait-db"));
}

#[test]
fn external_readiness_classifies_lock_and_materialization_blockers() {
    let manifest = test_manifest(vec![test_external(
        "ait-db",
        "ait-db",
        "SNP-DB-DIRECT",
        ".ait-external/ait-db",
    )]);
    let missing_status =
        build_external_status_report(ExternalStatusInput::new("ait-core", manifest.clone(), None))
            .unwrap();
    let missing_codes = readiness_codes(&missing_status);
    assert!(missing_codes.contains(&"external_lock_missing"));
    assert!(missing_codes.contains(&"external_materialization_missing"));

    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let dirty_status = build_external_status_report(
        ExternalStatusInput::new("ait-core", manifest.clone(), Some(lockfile.clone()))
            .with_materialization(ExternalMaterializationObservation {
                name: "ait-db".to_string(),
                parent_path: String::new(),
                materialize_to: ".ait-external/ait-db".to_string(),
                state: crate::external::status::ExternalObservedMaterializationState::Dirty,
                snapshot: None,
                reason: Some("not generated by AIT".to_string()),
            }),
    )
    .unwrap();
    let dirty_codes = readiness_codes(&dirty_status);
    assert!(dirty_codes.contains(&"external_materialization_dirty"));

    let outdated_status = build_external_status_report(
        ExternalStatusInput::new("ait-core", manifest, Some(lockfile)).with_materialization(
            ExternalMaterializationObservation::generated(
                "ait-db",
                "",
                ".ait-external/ait-db",
                "SNP-DB-OLD",
            ),
        ),
    )
    .unwrap();
    let outdated_codes = readiness_codes(&outdated_status);
    assert!(outdated_codes.contains(&"external_materialization_outdated"));
}

fn readiness_codes(status: &crate::external::status::ExternalStatusReport) -> Vec<&'static str> {
    build_external_readiness_report(status)
        .blockers
        .iter()
        .map(|blocker| match blocker.code.as_str() {
            "external_lock_drift" => "external_lock_drift",
            "external_lock_missing" => "external_lock_missing",
            "external_materialization_missing" => "external_materialization_missing",
            "external_materialization_dirty" => "external_materialization_dirty",
            "external_materialization_outdated" => "external_materialization_outdated",
            "external_local_link_active" => "external_local_link_active",
            "external_binding_path_missing" => "external_binding_path_missing",
            _ => "unknown",
        })
        .collect()
}

#[test]
fn external_doctor_facts_report_release_blocking_and_warning_findings() {
    let mut external = test_external(
        "ait-server-plugin",
        "ait-server-plugin",
        "SNP-SERVER-PLUGIN-AGPL",
        ".ait-external/ait-server-plugin",
    );
    external.license = "AGPL-3.0-only".to_string();
    external.bindings.python = Some(crate::external::manifest::ExternalPythonBinding {
        kind: "python-path".to_string(),
        path: "python".to_string(),
        package: Some("ait-db".to_string()),
        module: None,
    });
    let manifest = test_manifest(vec![external]);
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    let binding = lockfile.nodes[0].bindings[0].clone();
    let status = build_external_status_report(
        ExternalStatusInput::new("ait-core", manifest, Some(lockfile))
            .with_local_link("ait-server-plugin", "../ait-server-plugin")
            .with_binding_check(ExternalBindingCheckFact::new(
                "ait-server-plugin",
                "",
                ".ait-external/ait-server-plugin",
                &binding,
                ".ait-external/ait-server-plugin/python",
                false,
            )),
    )
    .unwrap();

    let doctor = build_external_doctor_report(&status, &ExternalDoctorOptions::default());
    let codes = doctor
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<Vec<_>>();

    assert!(!doctor.release_ready);
    assert!(codes.contains(&"external_local_link_active"));
    assert!(codes.contains(&"external_license_boundary"));
    assert!(codes.contains(&"external_binding_path_missing"));
    assert_eq!(doctor.release_blocking_findings().len(), 2);
    assert_eq!(doctor.warning_findings().len(), 1);
    let json = doctor.to_json_value();
    assert_eq!(json["checked"]["bindings"], true);
    assert_eq!(json["summary"]["release_blocking"], 2);
}

#[test]
fn external_update_contract_fixture_plain_update_does_not_float_to_latest() {
    let payload: crate::json_support::JsonValue =
        crate::json_support::JsonCodec::parse_value_with_error_prefix(
            &read_fixture_text("expected/update.json"),
            "Failed to parse update fixture",
        )
        .unwrap();

    assert_eq!(payload["command"], "external update");
    assert_eq!(payload["mode"], "locked");
    assert_eq!(payload["locked"], true);
    assert_eq!(payload["changed_pins"].as_array().unwrap().len(), 0);
    assert_eq!(payload["materialized"][0]["snapshot"], "SNP-DB-RECURSIVE");
    assert_eq!(payload["materialized"][0]["repository_index"], 11);
}

#[test]
fn external_update_locked_contract_fixture_reports_manifest_lock_drift() {
    let manifest = parse_manifest_fixture("drift/ait-external.toml");
    let lockfile = parse_lock_fixture("drift/ait-external.lock");

    assert_eq!(manifest.externals[0].snapshot, "SNP-DB-MANIFEST");
    assert_eq!(lockfile.nodes[0].snapshot, "SNP-DB-LOCK");
    assert_ne!(manifest.externals[0].snapshot, lockfile.nodes[0].snapshot);
}

#[test]
fn external_update_contract_fixture_to_snapshot_changes_one_external() {
    let manifest = parse_manifest_fixture("direct/ait-external.toml");

    assert_eq!(manifest.externals.len(), 1);
    assert_eq!(manifest.externals[0].name, "ait-db");
    assert_eq!(manifest.externals[0].snapshot, "SNP-DB-DIRECT");
}

#[test]
fn external_update_latest_contract_fixture_uses_remote_and_line_from_manifest() {
    let manifest = parse_manifest_fixture("direct/ait-external.toml");
    let external = &manifest.externals[0];

    assert_eq!(external.remote, "origin");
    assert_eq!(external.line, "main");
}

#[test]
fn sprint0_duplicate_name_fixture_preserves_parent_identity() {
    let root = parse_manifest_fixture("duplicate-names/ait-core/ait-external.toml");
    let ait_db = parse_manifest_fixture("duplicate-names/ait-db/ait-external.toml");
    let ait_tools = parse_manifest_fixture("duplicate-names/ait-tools/ait-external.toml");
    let lockfile = parse_lock_fixture("duplicate-names/ait-external.lock");
    let nodes = &lockfile.nodes;

    assert_eq!(root.externals.len(), 2);
    assert_eq!(ait_db.externals[0].name, "ait-codec");
    assert_eq!(ait_tools.externals[0].name, "ait-codec");
    assert_eq!(ait_db.externals[0].repository_index, 12);
    assert_eq!(ait_tools.externals[0].repository_index, 15);
    assert_ne!(
        ait_db.externals[0].snapshot,
        ait_tools.externals[0].snapshot
    );
    assert_eq!(nodes.len(), 4);
    assert_eq!(nodes[1].name, "ait-codec");
    assert_eq!(nodes[1].parent_path, ".ait-external/ait-db");
    assert_eq!(nodes[3].name, "ait-codec");
    assert_eq!(nodes[3].parent_path, ".ait-external/ait-tools");
}

#[test]
fn external_status_and_doctor_report_duplicate_names_as_allowed_warnings() {
    let manifest = parse_manifest_fixture("duplicate-names/ait-core/ait-external.toml");
    let lockfile = parse_lock_fixture("duplicate-names/ait-external.lock");
    let mut input = ExternalStatusInput::new("ait-core", manifest, Some(lockfile.clone()));
    for node in &lockfile.nodes {
        input = input.with_materialization(ExternalMaterializationObservation::generated(
            node.name.clone(),
            node.parent_path.clone(),
            node.materialize_to.clone(),
            node.snapshot.clone(),
        ));
    }

    let status = build_external_status_report(input).unwrap();
    let duplicate = status.duplicates.first().expect("duplicate group");

    assert_eq!(status.summary.duplicate_names, 1);
    assert_eq!(duplicate.name, "ait-codec");
    assert_eq!(duplicate.policy, ExternalDuplicatePolicy::Allow);
    assert_eq!(duplicate.entries.len(), 2);
    assert!(duplicate
        .entries
        .iter()
        .any(|entry| entry.parent_path == ".ait-external/ait-db"
            && entry.snapshot == "SNP-CODEC-FOR-DB"));
    assert!(duplicate
        .entries
        .iter()
        .any(|entry| entry.parent_path == ".ait-external/ait-tools"
            && entry.snapshot == "SNP-CODEC-FOR-TOOLS"));

    let status_json = status.to_json_value();
    assert_eq!(status_json["summary"]["duplicate_names"], 1);
    assert_eq!(status_json["duplicates"][0]["policy"], "allow");

    let doctor = build_external_doctor_report(&status, &ExternalDoctorOptions::default());
    let duplicate_findings = doctor
        .findings
        .iter()
        .filter(|finding| finding.code == "external_duplicate_name")
        .collect::<Vec<_>>();

    assert!(doctor.release_ready);
    assert_eq!(duplicate_findings.len(), 1);
    assert!(!duplicate_findings[0].release_blocking);
    assert_eq!(duplicate_findings[0].severity.as_str(), "warning");
}

#[test]
fn external_doctor_blocks_empty_manifest_when_current_source_core_is_stale() {
    let status = build_external_status_report(
        ExternalStatusInput::new(
            "ait",
            ExternalManifest {
                externals: Vec::new(),
            },
            None,
        )
        .with_current_source_core(ExternalCurrentSourceCoreStatus {
            repo_root: "/repo/ait".to_string(),
            metadata_path: "/repo/ait/.ait/runtime-extensions/ait_py/.current-source-build.json"
                .to_string(),
            metadata_present: true,
            core_repo_root: Some("/repo/ait-core".to_string()),
            core_source_fingerprint: Some("core-a".to_string()),
            core_source_mtime_ns: Some(100),
            active_binary_path: Some("/repo/ait/.ait/cargo-target/debug/ait-cli".to_string()),
            active_binary_role: ExternalCurrentSourceArtifactRole::ActiveBinary,
            artifacts: vec![
                ExternalCurrentSourceArtifactStatus {
                    name: "ait_py_metadata".to_string(),
                    role: ExternalCurrentSourceArtifactRole::Metadata,
                    path: Some(
                        "/repo/ait/.ait/runtime-extensions/ait_py/.current-source-build.json"
                            .to_string(),
                    ),
                    state: ExternalCurrentSourceArtifactState::Ready,
                    reason: None,
                    expected_profile: None,
                    metadata_sha256: None,
                    actual_sha256: None,
                    metadata_mtime_ns: None,
                    actual_mtime_ns: Some(100),
                },
                ExternalCurrentSourceArtifactStatus {
                    name: "active_ait_cli".to_string(),
                    role: ExternalCurrentSourceArtifactRole::ActiveBinary,
                    path: Some("/repo/ait/.ait/cargo-target/debug/ait-cli".to_string()),
                    state: ExternalCurrentSourceArtifactState::WrongBinary,
                    reason: Some(
                        "active ait-cli is not the canonical current-source release binary"
                            .to_string(),
                    ),
                    expected_profile: Some("release".to_string()),
                    metadata_sha256: Some("expected".to_string()),
                    actual_sha256: Some("actual".to_string()),
                    metadata_mtime_ns: Some(100),
                    actual_mtime_ns: Some(99),
                },
            ],
        }),
    )
    .unwrap();

    let status_json = status.to_json_value();
    assert_eq!(status_json["summary"]["missing"], 0);
    assert_eq!(
        status_json["current_source_core"]["summary"]["wrong_binary"],
        1
    );

    let doctor = build_external_doctor_report(&status, &ExternalDoctorOptions::default());
    assert!(!doctor.release_ready);
    assert!(doctor.checked.current_source_core);
    assert!(doctor.findings.iter().any(|finding| {
        finding.code == "external_current_source_core_artifact"
            && finding.name.as_deref() == Some("active_ait_cli")
            && finding.release_blocking
    }));
}

#[test]
fn external_status_contract_fixture_reports_linked_and_missing_states() {
    let payload: crate::json_support::JsonValue =
        crate::json_support::JsonCodec::parse_value_with_error_prefix(
            &read_fixture_text("expected/status.json"),
            "Failed to parse status fixture",
        )
        .unwrap();
    let links: toml::Value = read_fixture_text("local-link/ait-external.links.toml")
        .parse()
        .unwrap();

    assert_eq!(payload["command"], "external status");
    assert_eq!(payload["summary"]["linked"], 1);
    assert_eq!(payload["summary"]["missing"], 1);
    assert_eq!(payload["externals"][0]["state"], "linked");
    assert_eq!(payload["externals"][0]["repository_index"], 11);
    assert_eq!(links["link"][0]["path"].as_str(), Some("../ait-db"));
}

#[test]
fn external_link_store_parses_renders_upserts_and_removes_links() {
    let parsed = parse_external_local_link_overrides(
        read_fixture_text("local-link/ait-external.links.toml").as_bytes(),
    )
    .unwrap();

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "ait-db");
    assert_eq!(parsed[0].path, "../ait-db");

    let added = upsert_external_local_link_override(&parsed, "ait-codec", "../ait-codec").unwrap();
    assert!(added.changed);
    assert_eq!(added.links[0].name, "ait-codec");
    assert_eq!(added.links[1].name, "ait-db");

    let rendered = render_external_local_link_overrides(&added.links).unwrap();
    let reparsed = parse_external_local_link_overrides(&rendered).unwrap();
    assert_eq!(reparsed, added.links);

    let unchanged =
        upsert_external_local_link_override(&reparsed, "ait-codec", "../ait-codec").unwrap();
    assert!(!unchanged.changed);

    let removed = remove_external_local_link_override(&reparsed, "ait-db").unwrap();
    assert!(removed.changed);
    assert_eq!(removed.links.len(), 1);
    assert_eq!(removed.links[0].name, "ait-codec");

    let missing = remove_external_local_link_override(&removed.links, "ait-missing").unwrap();
    assert!(!missing.changed);
    assert_eq!(missing.links, removed.links);
}

#[test]
fn filesystem_external_link_store_writes_and_removes_local_metadata_file() {
    let temp = tempfile::tempdir().unwrap();
    let store = FsExternalLinkStore::for_repo_root(temp.path());
    let links_path = temp.path().join(EXTERNAL_LINKS_FILE);

    store
        .save_links(&[ExternalLocalLinkOverride {
            name: "ait-db".to_string(),
            path: "../ait-db".to_string(),
        }])
        .unwrap();

    let loaded = store.load_links().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "ait-db");
    assert!(links_path.exists());

    store.save_links(&[]).unwrap();
    assert!(!links_path.exists());
}

#[test]
fn external_doctor_contract_fixture_reports_license_and_binding_findings() {
    let manifest = parse_manifest_fixture("invalid-license/ait-external.toml");
    let payload: crate::json_support::JsonValue =
        crate::json_support::JsonCodec::parse_value_with_error_prefix(
            &read_fixture_text("expected/doctor.json"),
            "Failed to parse doctor fixture",
        )
        .unwrap();
    let findings = payload["findings"].as_array().unwrap();

    assert_eq!(payload["command"], "external doctor");
    assert_eq!(payload["release_ready"], false);
    assert_eq!(manifest.externals[0].license, "AGPL-3.0-only");
    assert!(findings
        .iter()
        .any(|finding| finding["code"] == "external_license_boundary"));
    assert!(findings
        .iter()
        .any(|finding| finding["code"] == "external_binding_path_missing"));
}

#[test]
fn sprint0_fixtures_stay_separate_from_binary_db_docs_and_tests() {
    let update_contract = read_fixture_text("expected/update.json");
    let direct_manifest = read_fixture_text("direct/ait-external.toml");

    assert!(!update_contract.contains("Binary DB"));
    assert!(!direct_manifest.contains("Binary DB"));
}

#[test]
fn toml_manifest_codec_rejects_parent_materialize_path() {
    let err = TomlExternalManifestCodec
        .parse_manifest(
            br#"
[[external]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 11
remote = "origin"
line = "main"
snapshot = "SNP-123"
materialize_to = "../ait-db"
license = "Apache-2.0"
"#,
        )
        .unwrap_err();

    assert!(err.message().contains("must not escape the repository"));
}

#[test]
fn toml_manifest_codec_rejects_absolute_binding_path() {
    let err = TomlExternalManifestCodec
        .parse_manifest(
            br#"
[[external]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 11
remote = "origin"
line = "main"
snapshot = "SNP-123"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"

[external.bindings.rust]
kind = "cargo-path"
path = "/tmp/ait-db"
"#,
        )
        .unwrap_err();

    assert!(err.message().contains("must be repository-relative"));
}

#[test]
fn toml_manifest_codec_rejects_unknown_binding_kind() {
    let err = TomlExternalManifestCodec
        .parse_manifest(
            br#"
[[external]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 11
remote = "origin"
line = "main"
snapshot = "SNP-123"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"

[external.bindings.python]
kind = "editable"
path = "python"
"#,
        )
        .unwrap_err();

    assert!(err.message().contains("python"));
    assert!(err.message().contains("python-path"));
}

#[test]
fn toml_manifest_codec_rejects_missing_required_snapshot() {
    let fixture = read_fixture_text("missing-snapshot/ait-external.toml");
    let err = TomlExternalManifestCodec
        .parse_manifest(fixture.as_bytes())
        .unwrap_err();

    assert!(err.message().contains("snapshot"));
}

#[test]
fn toml_manifest_codec_rejects_missing_source_repository_index() {
    let err = TomlExternalManifestCodec
        .parse_manifest(
            br#"
[[external]]
name = "ait-core"
repo_name = "ait-core"
remote = "origin"
line = "main"
snapshot = "SNP-CORE"
materialize_to = ".ait-external/ait-core"
license = "Apache-2.0"
"#,
        )
        .unwrap_err();

    assert!(err.message().contains("repository_index"));
}

#[test]
fn external_manifest_validation_rejects_empty_required_values_before_filesystem_work() {
    let manifest = ExternalManifest {
        externals: vec![crate::external::manifest::ExternalDeclaration {
            name: "ait-db".to_string(),
            repo_name: " ".to_string(),
            repository_index: 11,
            remote: "origin".to_string(),
            line: "main".to_string(),
            snapshot: "SNP-123".to_string(),
            materialize_to: ".ait-external/ait-db".to_string(),
            license: "Apache-2.0".to_string(),
            version: None,
            bindings: Default::default(),
        }],
    };

    let err = manifest.validate().unwrap_err();

    assert!(err.message().contains("repo_name"));
}

#[test]
fn external_error_exposes_structured_json_payload() {
    let err = ExternalError::with_code("external_manifest_parse", "invalid manifest");
    let json = err.to_json_value();

    assert_eq!(err.code(), "external_manifest_parse");
    assert_eq!(json["code"], "external_manifest_parse");
    assert_eq!(json["message"], "invalid manifest");
}
