use super::*;
use crate::file_io::{FileIoResult, FileIoStore};
use crate::json_support::json;
use crate::workspace_test_support;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::artifact_ports::CurrentSourceNativeCacheArtifactStore;
use super::lease_ports::CurrentSourceNativeCacheLeaseStore;
use super::source_ports::{
    CurrentSourceNativeCacheSourceEntry, CurrentSourceNativeCacheSourceStore,
};

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("ait-current-source-cache-{prefix}-{nanos}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn expected_source_fingerprint(root: &Path, inputs: Vec<(PathBuf, Vec<u8>)>) -> String {
    let mut digest = Sha256::new();
    for (path, bytes) in inputs {
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        digest.update(rel.as_bytes());
        digest.update(b"\0");
        digest.update(bytes);
        digest.update(b"\0");
    }
    format!("{:x}", digest.finalize())
}

fn seed_core_repo(root: &Path) {
    write(&root.join("rust/Cargo.toml"), "[workspace]\n");
    write(&root.join("rust/Cargo.lock"), "version = 4\n");
    write(
        &root.join("rust/crates/ait-agent-core/Cargo.toml"),
        "[package]\n",
    );
    write(
        &root.join("rust/crates/ait-agent-worker/Cargo.toml"),
        "[package]\n",
    );
    write(&root.join("rust/crates/ait-cli/Cargo.toml"), "[package]\n");
    write(&root.join("rust/crates/ait-core/Cargo.toml"), "[package]\n");
    write(&root.join("rust/crates/ait-py/Cargo.toml"), "[package]\n");
    write(
        &root.join("rust/crates/ait-agent-core/src/lib.rs"),
        "pub fn agent_core() {}\n",
    );
    write(
        &root.join("rust/crates/ait-agent-worker/src/lib.rs"),
        "pub fn agent_worker() {}\n",
    );
    write(
        &root.join("rust/crates/ait-cli/src/main.rs"),
        "fn main() {}\n",
    );
    write(
        &root.join("rust/crates/ait-core/src/lib.rs"),
        "pub fn core() {}\n",
    );
    write(
        &root.join("rust/crates/ait-py/src/lib.rs"),
        "pub fn py() {}\n",
    );
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

fn test_cache_paths(root: &Path, build_key: &str) -> CurrentSourceNativeCachePaths {
    let namespace_root = root.join("AIT_RAM/.ait-temp/repo-scope");
    let cache_root = namespace_root
        .join(CURRENT_SOURCE_CACHE_NAMESPACE)
        .join(build_key);
    CurrentSourceNativeCachePaths {
        namespace_root,
        build_key: build_key.to_string(),
        cache_root: cache_root.clone(),
        runtime_extensions_root: cache_root.join("runtime-extensions"),
        package_dir: cache_root.join("runtime-extensions/ait_py"),
        target_dir: cache_root.join("cargo-target"),
        lock_path: cache_root.join(".build.lock"),
        manifest_path: cache_root.join("manifest.json"),
        leases_dir: cache_root.join("leases"),
    }
}

#[derive(Default)]
struct FakeCurrentSourceNativeCacheLeaseStore {
    live_by_dir: BTreeMap<String, Vec<PathBuf>>,
    calls: RefCell<Vec<String>>,
    writes: RefCell<Vec<(String, JsonValue)>>,
}

impl CurrentSourceNativeCacheLeaseStore for FakeCurrentSourceNativeCacheLeaseStore {
    fn ensure_leases_dir(&self, leases_dir: &Path) -> Result<(), String> {
        self.calls
            .borrow_mut()
            .push(format!("ensure:{}", path_text(leases_dir)));
        Ok(())
    }

    fn write_lease(&self, lease_path: &Path, payload: &JsonValue) -> Result<(), String> {
        self.calls
            .borrow_mut()
            .push(format!("write:{}", path_text(lease_path)));
        self.writes
            .borrow_mut()
            .push((path_text(lease_path), payload.clone()));
        Ok(())
    }

    fn release_lease(&self, lease_path: &Path) -> Result<(), String> {
        self.calls
            .borrow_mut()
            .push(format!("release:{}", path_text(lease_path)));
        Ok(())
    }

    fn live_lease_paths(&self, leases_dir: &Path) -> Vec<PathBuf> {
        self.calls
            .borrow_mut()
            .push(format!("live:{}", path_text(leases_dir)));
        self.live_by_dir
            .get(&path_text(leases_dir))
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Default)]
struct FakeCurrentSourceNativeCacheArtifactStore {
    exists: BTreeMap<String, bool>,
    executable: BTreeMap<String, bool>,
    mtime_ns: BTreeMap<String, u64>,
    sha256: BTreeMap<String, String>,
    metadata: BTreeMap<String, JsonMap<String, JsonValue>>,
    calls: RefCell<Vec<String>>,
    publications: RefCell<Vec<(String, String, bool)>>,
    init_paths: RefCell<Vec<String>>,
    metadata_writes: RefCell<Vec<(String, JsonValue)>>,
}

#[derive(Default)]
struct FakeCurrentSourceNativeCacheSourceStore {
    resolved_paths: BTreeMap<String, PathBuf>,
    exists: BTreeMap<String, bool>,
    directories: BTreeMap<String, bool>,
    directory_entries: BTreeMap<String, Vec<CurrentSourceNativeCacheSourceEntry>>,
    files: BTreeMap<String, Vec<u8>>,
    mtime_ns: BTreeMap<String, u64>,
    calls: RefCell<Vec<String>>,
}

#[derive(Default)]
struct FakeCurrentSourceCacheFileIoStore {
    files: RefCell<BTreeMap<PathBuf, String>>,
    reads: RefCell<Vec<PathBuf>>,
    atomic_writes: RefCell<Vec<(PathBuf, String, String)>>,
    atomic_error: Option<String>,
}

impl FakeCurrentSourceCacheFileIoStore {
    fn insert_file(&self, path: PathBuf, text: &str) {
        self.files.borrow_mut().insert(path, text.to_string());
    }
}

impl FileIoStore for FakeCurrentSourceCacheFileIoStore {
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
            .ok_or_else(|| format!("missing fake JSON {}", path.display()).into())
    }

    fn write_string(&self, path: &Path, text: &str) -> FileIoResult<()> {
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), text.to_string());
        Ok(())
    }

    fn write_string_atomically(
        &self,
        path: &Path,
        text: &str,
        publish_label: &str,
    ) -> FileIoResult<()> {
        self.atomic_writes.borrow_mut().push((
            path.to_path_buf(),
            text.to_string(),
            publish_label.to_string(),
        ));
        if let Some(err) = &self.atomic_error {
            return Err(err.clone().into());
        }
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), text.to_string());
        Ok(())
    }
}

impl CurrentSourceNativeCacheSourceStore for FakeCurrentSourceNativeCacheSourceStore {
    fn resolve_path_strict_false(&self, path: &Path) -> PathBuf {
        self.calls
            .borrow_mut()
            .push(format!("resolve:{}", path_text(path)));
        self.resolved_paths
            .get(&path_text(path))
            .cloned()
            .unwrap_or_else(|| path.to_path_buf())
    }

    fn path_exists(&self, path: &Path) -> bool {
        self.calls
            .borrow_mut()
            .push(format!("exists:{}", path_text(path)));
        self.exists.get(&path_text(path)).copied().unwrap_or(false)
    }

    fn path_is_dir(&self, path: &Path) -> bool {
        self.calls
            .borrow_mut()
            .push(format!("is-dir:{}", path_text(path)));
        self.directories
            .get(&path_text(path))
            .copied()
            .unwrap_or(false)
    }

    fn read_source_file(&self, path: &Path) -> Result<Vec<u8>, String> {
        self.calls
            .borrow_mut()
            .push(format!("read:{}", path_text(path)));
        self.files
            .get(&path_text(path))
            .cloned()
            .ok_or_else(|| format!("missing fake source file {}", path.display()))
    }

    fn path_mtime_ns(&self, path: &Path) -> Result<u64, String> {
        self.calls
            .borrow_mut()
            .push(format!("mtime:{}", path_text(path)));
        self.mtime_ns
            .get(&path_text(path))
            .copied()
            .ok_or_else(|| format!("missing fake mtime for {}", path.display()))
    }

    fn read_source_dir(
        &self,
        dir: &Path,
    ) -> Result<Vec<CurrentSourceNativeCacheSourceEntry>, String> {
        self.calls
            .borrow_mut()
            .push(format!("read-dir:{}", path_text(dir)));
        self.directory_entries
            .get(&path_text(dir))
            .cloned()
            .ok_or_else(|| format!("missing fake source dir {}", dir.display()))
    }
}

impl CurrentSourceNativeCacheArtifactStore for FakeCurrentSourceNativeCacheArtifactStore {
    fn artifact_exists(&self, path: &Path) -> bool {
        self.calls
            .borrow_mut()
            .push(format!("exists:{}", path_text(path)));
        self.exists.get(&path_text(path)).copied().unwrap_or(false)
    }

    fn artifact_is_executable(&self, path: &Path) -> bool {
        self.calls
            .borrow_mut()
            .push(format!("executable:{}", path_text(path)));
        self.executable
            .get(&path_text(path))
            .copied()
            .unwrap_or(false)
    }

    fn artifact_mtime_ns(&self, path: &Path) -> Result<u64, String> {
        self.calls
            .borrow_mut()
            .push(format!("mtime:{}", path_text(path)));
        self.mtime_ns
            .get(&path_text(path))
            .copied()
            .ok_or_else(|| format!("missing fake mtime for {}", path.display()))
    }

    fn artifact_sha256_hex(&self, path: &Path) -> Result<String, String> {
        self.calls
            .borrow_mut()
            .push(format!("sha:{}", path_text(path)));
        self.sha256
            .get(&path_text(path))
            .cloned()
            .ok_or_else(|| format!("missing fake sha for {}", path.display()))
    }

    fn load_metadata(&self, path: &Path) -> JsonMap<String, JsonValue> {
        self.calls
            .borrow_mut()
            .push(format!("load:{}", path_text(path)));
        self.metadata
            .get(&path_text(path))
            .cloned()
            .unwrap_or_default()
    }

    fn publish_artifact(
        &self,
        source: &Path,
        target: &Path,
        repair_extension_install_name: bool,
    ) -> Result<(), String> {
        self.calls.borrow_mut().push(format!(
            "publish:{}->{}:{repair_extension_install_name}",
            path_text(source),
            path_text(target)
        ));
        self.publications.borrow_mut().push((
            path_text(source),
            path_text(target),
            repair_extension_install_name,
        ));
        Ok(())
    }

    fn ensure_local_extension_init(&self, init_path: &Path) -> Result<(), String> {
        self.calls
            .borrow_mut()
            .push(format!("init:{}", path_text(init_path)));
        self.init_paths.borrow_mut().push(path_text(init_path));
        Ok(())
    }

    fn write_metadata(&self, metadata_path: &Path, payload: &JsonValue) -> Result<(), String> {
        self.calls
            .borrow_mut()
            .push(format!("write-metadata:{}", path_text(metadata_path)));
        self.metadata_writes
            .borrow_mut()
            .push((path_text(metadata_path), payload.clone()));
        Ok(())
    }
}

#[test]
fn build_key_isolates_core_root_fingerprint_server_and_worker() {
    let root = temp_dir("build-key");
    let core = root.join("ait-core");
    let other_core = root.join("ait-core-other");
    fs::create_dir_all(&core).unwrap();
    fs::create_dir_all(&other_core).unwrap();

    let base = current_source_native_cache_build_key(
        &core,
        "core-a",
        Some("server-a"),
        ".cpython-314-darwin.so",
        "-C link-arg=-undefined",
        "shared",
    )
    .unwrap();
    assert_eq!(base.len(), 16);
    assert_ne!(
        base,
        current_source_native_cache_build_key(
            &other_core,
            "core-a",
            Some("server-a"),
            ".cpython-314-darwin.so",
            "-C link-arg=-undefined",
            "shared",
        )
        .unwrap()
    );
    assert_ne!(
        base,
        current_source_native_cache_build_key(
            &core,
            "core-b",
            Some("server-a"),
            ".cpython-314-darwin.so",
            "-C link-arg=-undefined",
            "shared",
        )
        .unwrap()
    );
    assert_ne!(
        base,
        current_source_native_cache_build_key(
            &core,
            "core-a",
            Some("server-b"),
            ".cpython-314-darwin.so",
            "-C link-arg=-undefined",
            "shared",
        )
        .unwrap()
    );
    assert_ne!(
        base,
        current_source_native_cache_build_key(
            &core,
            "core-a",
            Some("server-a"),
            ".cpython-314-darwin.so",
            "-C link-arg=-undefined",
            "gw1",
        )
        .unwrap()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cache_contract_returns_stable_path_shape() {
    let root = temp_dir("contract");
    let namespace_root = root.join("AIT_RAM/.ait-temp/repo-scope");
    let core = root.join("ait-core");
    seed_core_repo(&core);
    let payload = current_source_native_cache_contract_json(&CurrentSourceNativeCacheRequest {
        namespace_root: namespace_root.clone(),
        core_repo_root: core.clone(),
        core_source_fingerprint: Some("core-a".to_string()),
        server_source_fingerprint: None,
        ext_suffix: ".cpython-314-darwin.so".to_string(),
        rustflags: String::new(),
        worker_id: "shared".to_string(),
    })
    .unwrap();
    let build_key = payload["build_key"].as_str().unwrap();
    let cache_root = namespace_root
        .join(CURRENT_SOURCE_CACHE_NAMESPACE)
        .join(build_key)
        .to_string_lossy()
        .to_string();
    assert_eq!(
        payload["cache_schema_version"],
        CURRENT_SOURCE_CACHE_SCHEMA_VERSION
    );
    assert_eq!(payload["namespace"], CURRENT_SOURCE_CACHE_NAMESPACE);
    assert_eq!(payload["cache_root"], cache_root);
    assert_eq!(
        payload["runtime_extensions_root"],
        format!("{cache_root}/runtime-extensions")
    );
    assert_eq!(
        payload["package_dir"],
        format!("{cache_root}/runtime-extensions/ait_py")
    );
    assert_eq!(payload["target_dir"], format!("{cache_root}/cargo-target"));
    assert_eq!(
        payload["binary_profile"],
        CURRENT_SOURCE_CACHE_BINARY_PROFILE
    );
    assert_eq!(payload["lock_path"], format!("{cache_root}/.build.lock"));
    assert_eq!(
        payload["manifest_path"],
        format!("{cache_root}/manifest.json")
    );
    assert_eq!(payload["leases_dir"], format!("{cache_root}/leases"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn register_lease_writes_through_lease_store_trait_object() {
    let root = temp_dir("lease-store-register");
    let paths = test_cache_paths(&root, "lease-build");
    let store = FakeCurrentSourceNativeCacheLeaseStore::default();
    let store_port: &dyn CurrentSourceNativeCacheLeaseStore = &store;

    let payload = register_current_source_native_cache_lease_with_store(
        store_port,
        &paths,
        " gw-test ",
        4242,
    )
    .unwrap();

    let lease_path = PathBuf::from(payload["lease_path"].as_str().unwrap());
    assert_eq!(payload["namespace_root"], path_text(&paths.namespace_root));
    assert_eq!(payload["build_key"], "lease-build");
    assert_eq!(
        store.calls.borrow().as_slice(),
        [
            format!("ensure:{}", path_text(&paths.leases_dir)),
            format!("write:{}", path_text(&lease_path)),
        ]
    );
    let writes = store.writes.borrow();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, path_text(&lease_path));
    assert_eq!(writes[0].1["pid"], json!(4242));
    assert_eq!(writes[0].1["worker_id"], "gw-test");
    assert_eq!(writes[0].1["build_key"], "lease-build");
    assert!(writes[0].1["created_at"].as_f64().is_some());
    fs::remove_dir_all(root).ok();
}

#[test]
fn register_lease_rejects_invalid_external_owner_pid() {
    let root = temp_dir("lease-owner-pid");
    let paths = test_cache_paths(&root, "lease-build");

    let zero =
        register_current_source_native_cache_lease_for_owner_json(&paths, "shared", 0).unwrap_err();
    assert!(zero.contains("positive i32"));
    let too_large = register_current_source_native_cache_lease_for_owner_json(
        &paths,
        "shared",
        i32::MAX as u32 + 1,
    )
    .unwrap_err();
    assert!(too_large.contains("positive i32"));
    assert!(!paths.leases_dir.exists());
    fs::remove_dir_all(root).ok();
}

#[test]
fn release_lease_prunes_through_lease_store_trait_object() {
    let root = temp_dir("lease-store-release");
    let paths = test_cache_paths(&root, "live-lease-build");
    write(&paths.package_dir.join("payload.bin"), "live-cache");
    write_current_source_native_cache_manifest_json(&CurrentSourceNativeCacheManifestRequest {
        paths: paths.clone(),
        state: "ready".to_string(),
        source_mtime_ns: 1,
        last_used_at: Some(10.0),
        size_bytes: None,
        extra: JsonMap::new(),
    })
    .unwrap();
    let lease_path = paths.leases_dir.join("fake-release.json");
    let store = FakeCurrentSourceNativeCacheLeaseStore {
        live_by_dir: BTreeMap::from_iter([(
            path_text(&paths.leases_dir),
            vec![paths.leases_dir.join("fake-live.json")],
        )]),
        ..Default::default()
    };
    let store_port: &dyn CurrentSourceNativeCacheLeaseStore = &store;

    let payload = release_current_source_native_cache_lease_with_store(
        store_port,
        &lease_path,
        &paths.namespace_root,
        true,
    )
    .unwrap();

    assert_eq!(payload["released"], path_text(&lease_path));
    assert_eq!(payload["prune"]["removed_unleased_ready"], json!([]));
    assert!(paths.cache_root.exists());
    assert_eq!(
        store.calls.borrow().as_slice(),
        [
            format!("release:{}", path_text(&lease_path)),
            format!("live:{}", path_text(&paths.leases_dir)),
        ]
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn binary_freshness_reads_through_artifact_store_trait_object() {
    let metadata_path = PathBuf::from("/cache/.current-source-build.json");
    let binary_path = PathBuf::from("/cache/release/ait-cli");
    let store = FakeCurrentSourceNativeCacheArtifactStore {
        exists: BTreeMap::from_iter([(path_text(&binary_path), true)]),
        executable: BTreeMap::from_iter([(path_text(&binary_path), true)]),
        mtime_ns: BTreeMap::from_iter([(path_text(&binary_path), 123)]),
        sha256: BTreeMap::from_iter([(path_text(&binary_path), "sha-a".to_string())]),
        metadata: BTreeMap::from_iter([(
            path_text(&metadata_path),
            JsonMap::from_iter([
                ("core_source_fingerprint".to_string(), json!("core-a")),
                ("core_source_mtime_ns".to_string(), json!(99)),
                ("ait_cli_mtime_ns".to_string(), json!(123)),
                ("ait_cli_sha256".to_string(), json!("sha-a")),
            ]),
        )]),
        ..Default::default()
    };
    let store_port: &dyn CurrentSourceNativeCacheArtifactStore = &store;

    assert!(current_source_binary_is_fresh_with_artifact_store(
        store_port,
        &CurrentSourceBinaryFreshnessRequest {
            metadata_path: metadata_path.clone(),
            binary_path: binary_path.clone(),
            metadata_fingerprint_key: "core_source_fingerprint".to_string(),
            metadata_source_mtime_key: "core_source_mtime_ns".to_string(),
            metadata_mtime_key: "ait_cli_mtime_ns".to_string(),
            metadata_sha_key: "ait_cli_sha256".to_string(),
            source_mtime_ns: 99,
            source_fingerprint: "core-a".to_string(),
        }
    ));

    assert_eq!(
        store.calls.borrow().as_slice(),
        [
            format!("exists:{}", path_text(&binary_path)),
            format!("executable:{}", path_text(&binary_path)),
            format!("load:{}", path_text(&metadata_path)),
            format!("mtime:{}", path_text(&binary_path)),
            format!("sha:{}", path_text(&binary_path)),
        ]
    );
}

#[test]
fn canonical_seed_publishes_artifacts_through_artifact_store_trait_object() {
    let root = temp_dir("artifact-store-seed");
    let repo = root.join("worktree");
    let canonical = root.join("ait");
    let core = root.join("ait-core");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::create_dir_all(&core).unwrap();
    let request = CurrentSourceNativeCacheCanonicalSeedRequest {
        namespace_root: root.join("AIT_RAM/.ait-temp/repo-scope"),
        core_repo_root: core.clone(),
        repo_root: repo,
        canonical_repo_root: canonical.clone(),
        core_source_mtime_ns: 100,
        core_source_fingerprint: "core-a".to_string(),
        server_source_fingerprint: None,
        ext_suffix: ".cpython-314-darwin.so".to_string(),
        rustflags: String::new(),
        worker_id: "shared".to_string(),
    };
    let native_request = CurrentSourceNativeCacheRequest {
        namespace_root: request.namespace_root.clone(),
        core_repo_root: request.core_repo_root.clone(),
        core_source_fingerprint: Some(request.core_source_fingerprint.clone()),
        server_source_fingerprint: request.server_source_fingerprint.clone(),
        ext_suffix: request.ext_suffix.clone(),
        rustflags: request.rustflags.clone(),
        worker_id: request.worker_id.clone(),
    };
    let (paths, _, _, _, _) = current_source_native_cache_paths(&native_request).unwrap();
    let canonical_state_root = canonical.join(".ait");
    let canonical_extension_dir = canonical_state_root.join("runtime-extensions/ait_py");
    let canonical_metadata_path = canonical_extension_dir.join(".current-source-build.json");
    let canonical_extension_path = canonical_extension_dir.join("ait_py.cpython-314-darwin.so");
    let canonical_cli = canonical_state_root.join("cargo-target/release/ait-cli");
    let target_extension = paths.package_dir.join("ait_py.cpython-314-darwin.so");
    let target_cli = paths.target_dir.join("release/ait-cli");
    let target_metadata_path = paths.package_dir.join(".current-source-build.json");
    let store = FakeCurrentSourceNativeCacheArtifactStore {
        exists: BTreeMap::from_iter([
            (path_text(&canonical_extension_path), true),
            (path_text(&canonical_cli), true),
        ]),
        executable: BTreeMap::from_iter([(path_text(&canonical_cli), true)]),
        mtime_ns: BTreeMap::from_iter([
            (path_text(&canonical_extension_path), 100),
            (path_text(&canonical_cli), 125),
            (path_text(&target_cli), 130),
        ]),
        sha256: BTreeMap::from_iter([
            (path_text(&canonical_cli), "cli-sha".to_string()),
            (path_text(&target_cli), "target-cli-sha".to_string()),
        ]),
        metadata: BTreeMap::from_iter([(
            path_text(&canonical_metadata_path),
            JsonMap::from_iter([
                ("core_repo_root".to_string(), json!(path_text(&core))),
                ("core_source_fingerprint".to_string(), json!("core-a")),
                ("core_source_mtime_ns".to_string(), json!(100)),
                ("ait_cli_sha256".to_string(), json!("cli-sha")),
            ]),
        )]),
        ..Default::default()
    };
    let store_port: &dyn CurrentSourceNativeCacheArtifactStore = &store;

    let payload =
        seed_current_source_native_cache_from_canonical_with_artifact_store(store_port, &request)
            .unwrap();

    assert_eq!(payload["seeded"], true);
    assert_eq!(payload["extension_path"], path_text(&target_extension));
    assert_eq!(payload["ait_cli_path"], path_text(&target_cli));
    assert_eq!(
        store.publications.borrow().as_slice(),
        [
            (
                path_text(&canonical_extension_path),
                path_text(&target_extension),
                true,
            ),
            (path_text(&canonical_cli), path_text(&target_cli), false),
        ]
    );
    assert_eq!(
        store.init_paths.borrow().as_slice(),
        [path_text(&paths.package_dir.join("__init__.py"))]
    );
    let metadata_writes = store.metadata_writes.borrow();
    assert_eq!(metadata_writes.len(), 1);
    assert_eq!(metadata_writes[0].0, path_text(&target_metadata_path));
    assert_eq!(metadata_writes[0].1["core_source_fingerprint"], "core-a");
    assert_eq!(metadata_writes[0].1["ait_cli_mtime_ns"], json!(130));
    assert_eq!(metadata_writes[0].1["ait_cli_sha256"], "target-cli-sha");
    assert_eq!(metadata_writes[0].1["ait_cli_profile"], "release");
    fs::remove_dir_all(root).ok();
}

#[test]
fn source_fingerprint_reads_inputs_through_source_store_trait_object() {
    let root = PathBuf::from("/repo");
    let cargo_toml = root.join("rust/Cargo.toml");
    let cli_toml = root.join("rust/crates/ait-cli/Cargo.toml");
    let cli_src = root.join("rust/crates/ait-cli/src");
    let cli_main = cli_src.join("main.rs");
    let cli_ignore = cli_src.join("README.txt");
    let cli_nested = cli_src.join("nested");
    let cli_nested_lib = cli_nested.join("lib.rs");
    let agent_core_src = root.join("rust/crates/ait-agent-core/src");
    let agent_worker_src = root.join("rust/crates/ait-agent-worker/src");
    let core_src = root.join("rust/crates/ait-core/src");
    let py_src = root.join("rust/crates/ait-py/src");
    let store = FakeCurrentSourceNativeCacheSourceStore {
        exists: BTreeMap::from_iter([(path_text(&cargo_toml), true), (path_text(&cli_toml), true)]),
        directories: BTreeMap::from_iter([
            (path_text(&agent_core_src), false),
            (path_text(&agent_worker_src), false),
            (path_text(&cli_src), true),
            (path_text(&core_src), false),
            (path_text(&py_src), false),
        ]),
        directory_entries: BTreeMap::from_iter([
            (
                path_text(&cli_src),
                vec![
                    CurrentSourceNativeCacheSourceEntry::file(cli_ignore.clone()),
                    CurrentSourceNativeCacheSourceEntry::directory(cli_nested.clone()),
                    CurrentSourceNativeCacheSourceEntry::file(cli_main.clone()),
                ],
            ),
            (
                path_text(&cli_nested),
                vec![CurrentSourceNativeCacheSourceEntry::file(
                    cli_nested_lib.clone(),
                )],
            ),
        ]),
        files: BTreeMap::from_iter([
            (path_text(&cargo_toml), b"[workspace]\n".to_vec()),
            (
                path_text(&cli_toml),
                b"[package]\nname = \"ait-cli\"\n".to_vec(),
            ),
            (path_text(&cli_main), b"fn main() {}\n".to_vec()),
            (path_text(&cli_nested_lib), b"pub fn nested() {}\n".to_vec()),
        ]),
        ..Default::default()
    };
    let store_port: &dyn CurrentSourceNativeCacheSourceStore = &store;

    let fingerprint = current_core_source_fingerprint_with_source_store(store_port, &root).unwrap();

    assert_eq!(
        fingerprint,
        expected_source_fingerprint(
            &root,
            vec![
                (cargo_toml.clone(), b"[workspace]\n".to_vec()),
                (
                    cli_toml.clone(),
                    b"[package]\nname = \"ait-cli\"\n".to_vec()
                ),
                (cli_main.clone(), b"fn main() {}\n".to_vec()),
                (cli_nested_lib.clone(), b"pub fn nested() {}\n".to_vec()),
            ],
        )
    );
    assert_eq!(
        store.calls.borrow().as_slice(),
        [
            "resolve:/repo".to_string(),
            format!("exists:{}", path_text(&cargo_toml)),
            format!("exists:{}", path_text(&root.join("rust/Cargo.lock"))),
            format!(
                "exists:{}",
                path_text(&root.join("rust/crates/ait-agent-core/Cargo.toml"))
            ),
            format!(
                "exists:{}",
                path_text(&root.join("rust/crates/ait-agent-worker/Cargo.toml"))
            ),
            format!(
                "exists:{}",
                path_text(&root.join("rust/crates/ait-cli/Cargo.toml"))
            ),
            format!(
                "exists:{}",
                path_text(&root.join("rust/crates/ait-core/Cargo.toml"))
            ),
            format!(
                "exists:{}",
                path_text(&root.join("rust/crates/ait-py/Cargo.toml"))
            ),
            format!("is-dir:{}", path_text(&agent_core_src)),
            format!("is-dir:{}", path_text(&agent_worker_src)),
            format!("is-dir:{}", path_text(&cli_src)),
            format!("read-dir:{}", path_text(&cli_src)),
            format!("read-dir:{}", path_text(&cli_nested)),
            format!("is-dir:{}", path_text(&core_src)),
            format!("is-dir:{}", path_text(&py_src)),
            format!("read:{}", path_text(&cargo_toml)),
            format!("read:{}", path_text(&cli_toml)),
            format!("read:{}", path_text(&cli_main)),
            format!("read:{}", path_text(&cli_nested_lib)),
        ]
    );
}

#[test]
fn source_mtime_reads_inputs_through_source_store_trait_object() {
    let root = PathBuf::from("/server");
    let cargo_toml = root.join("rust/Cargo.toml");
    let server_toml = root.join("rust/crates/ait-server-core/Cargo.toml");
    let server_src = root.join("rust/crates/ait-server-core/src");
    let server_lib = server_src.join("lib.rs");
    let store = FakeCurrentSourceNativeCacheSourceStore {
        exists: BTreeMap::from_iter([
            (path_text(&cargo_toml), true),
            (path_text(&server_toml), true),
        ]),
        directories: BTreeMap::from_iter([(path_text(&server_src), true)]),
        directory_entries: BTreeMap::from_iter([(
            path_text(&server_src),
            vec![CurrentSourceNativeCacheSourceEntry::file(
                server_lib.clone(),
            )],
        )]),
        mtime_ns: BTreeMap::from_iter([
            (path_text(&cargo_toml), 10),
            (path_text(&server_toml), 20),
            (path_text(&server_lib), 30),
        ]),
        ..Default::default()
    };
    let store_port: &dyn CurrentSourceNativeCacheSourceStore = &store;

    let mtime = current_server_source_mtime_ns_with_source_store(store_port, &root).unwrap();

    assert_eq!(mtime, 30);
    assert_eq!(
        store.calls.borrow().as_slice(),
        [
            "resolve:/server".to_string(),
            format!("exists:{}", path_text(&cargo_toml)),
            format!("exists:{}", path_text(&server_toml)),
            format!("is-dir:{}", path_text(&server_src)),
            format!("read-dir:{}", path_text(&server_src)),
            format!("mtime:{}", path_text(&cargo_toml)),
            format!("mtime:{}", path_text(&server_toml)),
            format!("mtime:{}", path_text(&server_lib)),
        ]
    );
}

#[test]
fn core_source_fingerprint_changes_with_rust_inputs_only() {
    let root = temp_dir("fingerprint");
    let core = root.join("ait-core");
    seed_core_repo(&core);
    let first = current_core_source_fingerprint(&core).unwrap();
    write(&core.join("README.md"), "ignored\n");
    assert_eq!(current_core_source_fingerprint(&core).unwrap(), first);
    write(
        &core.join("rust/crates/ait-cli/src/main_app/patchset.rs"),
        "pub fn nested_cli_source() {}\n",
    );
    let nested = current_core_source_fingerprint(&core).unwrap();
    assert_ne!(nested, first);
    write(
        &core.join("rust/crates/ait-core/src/lib.rs"),
        "pub fn changed() {}\n",
    );
    let changed_core = current_core_source_fingerprint(&core).unwrap();
    assert_ne!(changed_core, nested);
    write(
        &core.join("rust/crates/ait-agent-core/src/runtime.rs"),
        "pub fn changed_agent_core() {}\n",
    );
    let changed_agent_core = current_core_source_fingerprint(&core).unwrap();
    assert_ne!(changed_agent_core, changed_core);
    write(
        &core.join("rust/crates/ait-agent-worker/src/runner.rs"),
        "pub fn changed_agent_worker() {}\n",
    );
    assert_ne!(
        current_core_source_fingerprint(&core).unwrap(),
        changed_agent_core
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn server_source_fingerprint_requires_server_inputs() {
    let root = temp_dir("server");
    let missing = root.join("missing-server");
    fs::create_dir_all(&missing).unwrap();
    assert!(current_server_source_fingerprint(&missing).is_err());
    let server = root.join("ait-server");
    write(&server.join("rust/Cargo.toml"), "[workspace]\n");
    write(
        &server.join("rust/crates/ait-server-core/Cargo.toml"),
        "[package]\n",
    );
    write(
        &server.join("rust/crates/ait-server-core/src/lib.rs"),
        "pub fn server() {}\n",
    );
    assert_eq!(
        current_server_source_fingerprint(&server).unwrap().len(),
        64
    );
    fs::remove_dir_all(root).unwrap();
}

fn cli_bootstrap_fixture(
    root: &Path,
) -> (
    CurrentSourceCliBootstrapRequest,
    CurrentSourceIdentity,
    PathBuf,
) {
    let core = root.join("ait-core");
    let metadata_path = root.join("ait/.ait/runtime-extensions/ait_py/.current-source-build.json");
    let executable_path = root.join("ait-core/.ait/cargo-target/release/ait-cli");
    seed_core_repo(&core);
    write(&executable_path, "fixture ait-cli bytes\n");
    make_executable(&executable_path);
    let identity = current_core_source_identity(&core).unwrap();
    let executable_sha256 = artifact_sha256_hex_with_current_source_native_cache_artifact_store(
        &FilesystemCurrentSourceNativeCacheArtifactStore,
        &executable_path,
    )
    .unwrap();
    let executable_mtime_ns = artifact_mtime_ns_with_current_source_native_cache_artifact_store(
        &FilesystemCurrentSourceNativeCacheArtifactStore,
        &executable_path,
    )
    .unwrap();
    CurrentSourceCacheJson::filesystem()
        .write_pretty_json_atomically(
            &metadata_path,
            &json!({
                "ait_cli_mtime_ns": executable_mtime_ns,
                "ait_cli_profile": CURRENT_SOURCE_CACHE_BINARY_PROFILE,
                "ait_cli_sha256": executable_sha256,
                "core_source_fingerprint": identity.source_fingerprint,
                "core_source_mtime_ns": identity.source_mtime_ns,
            }),
        )
        .unwrap();
    (
        CurrentSourceCliBootstrapRequest {
            core_repo_root: core,
            metadata_path,
            executable_path: executable_path.clone(),
        },
        identity,
        executable_path,
    )
}

#[test]
fn cli_bootstrap_validates_source_and_executable_timestamps() {
    let root = temp_dir("cli-bootstrap-fresh");
    let (request, identity, _) = cli_bootstrap_fixture(&root);

    let validated = validate_current_source_cli_bootstrap(&request).unwrap();

    assert_eq!(validated.core_repo_root, request.core_repo_root);
    assert_eq!(validated.source_mtime_ns, identity.source_mtime_ns);
    assert_eq!(validated.source_fingerprint, identity.source_fingerprint);
    assert_eq!(validated.executable_sha256.len(), 64);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_bootstrap_fails_closed_when_core_source_changes() {
    let root = temp_dir("cli-bootstrap-source-stale");
    let (request, _, _) = cli_bootstrap_fixture(&root);
    write(
        &request
            .core_repo_root
            .join("rust/crates/ait-core/src/lib.rs"),
        "pub fn changed_after_build() {}\n",
    );

    let error = validate_current_source_cli_bootstrap(&request).unwrap_err();

    assert!(error.contains("Current-source ait-cli is stale"));
    assert!(error.contains("core source"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_bootstrap_fails_closed_when_executable_timestamp_changes() {
    let root = temp_dir("cli-bootstrap-binary-stale");
    let (request, _, executable_path) = cli_bootstrap_fixture(&root);
    write(&executable_path, "mutated ait-cli bytes\n");
    make_executable(&executable_path);

    let error = validate_current_source_cli_bootstrap(&request).unwrap_err();

    assert!(error.contains("Current-source ait-cli is stale"));
    assert!(error.contains("executable mtime"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_bootstrap_store_fast_path_reads_metadata_and_stats_without_content_hashing() {
    let root = PathBuf::from("/repo");
    let metadata_path = PathBuf::from("/metadata.json");
    let executable_path = PathBuf::from("/ait-cli");
    let cargo_toml = root.join("rust/Cargo.toml");
    let cli_toml = root.join("rust/crates/ait-cli/Cargo.toml");
    let cli_src = root.join("rust/crates/ait-cli/src");
    let cli_main = cli_src.join("main.rs");
    let core_src = root.join("rust/crates/ait-core/src");
    let py_src = root.join("rust/crates/ait-py/src");
    let source_store = FakeCurrentSourceNativeCacheSourceStore {
        exists: BTreeMap::from_iter([(path_text(&cargo_toml), true), (path_text(&cli_toml), true)]),
        directories: BTreeMap::from_iter([
            (path_text(&cli_src), true),
            (path_text(&core_src), false),
            (path_text(&py_src), false),
        ]),
        directory_entries: BTreeMap::from_iter([(
            path_text(&cli_src),
            vec![CurrentSourceNativeCacheSourceEntry::file(cli_main.clone())],
        )]),
        mtime_ns: BTreeMap::from_iter([
            (path_text(&cargo_toml), 10),
            (path_text(&cli_toml), 20),
            (path_text(&cli_main), 30),
        ]),
        ..Default::default()
    };
    let artifact_store = FakeCurrentSourceNativeCacheArtifactStore {
        executable: BTreeMap::from_iter([(path_text(&executable_path), true)]),
        mtime_ns: BTreeMap::from_iter([(path_text(&executable_path), 40)]),
        metadata: BTreeMap::from_iter([(
            path_text(&metadata_path),
            JsonMap::from_iter([
                ("ait_cli_mtime_ns".to_string(), json!(40)),
                (
                    "ait_cli_profile".to_string(),
                    json!(CURRENT_SOURCE_CACHE_BINARY_PROFILE),
                ),
                ("ait_cli_sha256".to_string(), json!("recorded-cli-sha")),
                (
                    "core_source_fingerprint".to_string(),
                    json!("recorded-source-fingerprint"),
                ),
                ("core_source_mtime_ns".to_string(), json!(30)),
            ]),
        )]),
        ..Default::default()
    };
    let request = CurrentSourceCliBootstrapRequest {
        core_repo_root: root,
        metadata_path: metadata_path.clone(),
        executable_path: executable_path.clone(),
    };

    let validated =
        validate_current_source_cli_bootstrap_with_stores(&source_store, &artifact_store, &request)
            .unwrap();

    assert_eq!(validated.source_mtime_ns, 30);
    assert_eq!(validated.source_fingerprint, "recorded-source-fingerprint");
    assert_eq!(validated.executable_sha256, "recorded-cli-sha");
    assert!(
        source_store
            .calls
            .borrow()
            .iter()
            .all(|call| !call.starts_with("read:")),
        "bootstrap fast path must not read source content"
    );
    assert_eq!(
        artifact_store.calls.borrow().as_slice(),
        [
            format!("load:{}", path_text(&metadata_path)),
            format!("executable:{}", path_text(&executable_path)),
            format!("mtime:{}", path_text(&executable_path)),
        ]
    );
}

#[test]
fn cli_bootstrap_store_fast_path_rejects_source_timestamp_drift() {
    let root = temp_dir("cli-bootstrap-store-source-stale");
    let (request, _, executable_path) = cli_bootstrap_fixture(&root);
    let mut metadata =
        CurrentSourceCacheJson::filesystem().load_object_or_empty(&request.metadata_path);
    metadata.insert("core_source_mtime_ns".to_string(), json!(1));
    CurrentSourceCacheJson::filesystem()
        .write_pretty_json_atomically(&request.metadata_path, &JsonValue::Object(metadata))
        .unwrap();

    let error = validate_current_source_cli_bootstrap(&request).unwrap_err();

    assert!(error.contains("core source mtime changed"));
    assert!(executable_path.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_bootstrap_store_fast_path_rejects_executable_timestamp_drift() {
    let root = temp_dir("cli-bootstrap-store-executable-stale");
    let (request, _, _) = cli_bootstrap_fixture(&root);
    let mut metadata =
        CurrentSourceCacheJson::filesystem().load_object_or_empty(&request.metadata_path);
    metadata.insert("ait_cli_mtime_ns".to_string(), json!(1));
    CurrentSourceCacheJson::filesystem()
        .write_pretty_json_atomically(&request.metadata_path, &JsonValue::Object(metadata))
        .unwrap();

    let error = validate_current_source_cli_bootstrap(&request).unwrap_err();

    assert!(error.contains("bootstrap executable mtime changed"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_bootstrap_fails_closed_when_build_metadata_is_missing() {
    let root = temp_dir("cli-bootstrap-missing-metadata");
    let core = root.join("ait-core");
    let executable_path = root.join("ait-core/.ait/cargo-target/release/ait-cli");
    seed_core_repo(&core);
    write(&executable_path, "fixture ait-cli bytes\n");
    make_executable(&executable_path);
    let request = CurrentSourceCliBootstrapRequest {
        core_repo_root: core,
        metadata_path: root.join("missing-build-metadata.json"),
        executable_path,
    };

    let error = validate_current_source_cli_bootstrap(&request).unwrap_err();

    assert!(error.contains("missing or invalid for `core_source_mtime_ns`"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cache_lifecycle_registers_releases_and_prunes_ready_cache() {
    let root = temp_dir("lifecycle");
    let paths = test_cache_paths(&root, "lease-build");
    write(&paths.package_dir.join("payload.bin"), "cache-payload");
    let mut extra = JsonMap::new();
    extra.insert("core_repo_root".to_string(), json!("/native/core"));
    write_current_source_native_cache_manifest_json(&CurrentSourceNativeCacheManifestRequest {
        paths: paths.clone(),
        state: "ready".to_string(),
        source_mtime_ns: 1,
        last_used_at: Some(now_seconds()),
        size_bytes: None,
        extra,
    })
    .unwrap();

    let lease = register_current_source_native_cache_lease_json(&paths, "gw4").unwrap();
    let lease_path = PathBuf::from(lease["lease_path"].as_str().unwrap());

    assert!(lease_path.exists());
    assert!(paths.cache_root.exists());

    release_current_source_native_cache_lease_json(&lease_path, &paths.namespace_root, true)
        .unwrap();

    assert!(!paths.cache_root.exists());
    fs::remove_dir_all(root).ok();
}

#[test]
fn shared_lease_release_reaps_idle_sibling_and_preserves_fresh_and_live_builds() {
    let root = temp_dir("shared-release-prune");
    let current_paths = test_cache_paths(&root, "current-build");
    write(
        &current_paths.package_dir.join("payload.bin"),
        "current-cache",
    );
    write_current_source_native_cache_manifest_json(&CurrentSourceNativeCacheManifestRequest {
        paths: current_paths.clone(),
        state: "ready".to_string(),
        source_mtime_ns: 1,
        last_used_at: Some(now_seconds()),
        size_bytes: None,
        extra: JsonMap::new(),
    })
    .unwrap();

    let idle_paths = test_cache_paths(&root, "idle-sibling");
    write(&idle_paths.package_dir.join("payload.bin"), "idle-cache");
    write_current_source_native_cache_manifest_json(&CurrentSourceNativeCacheManifestRequest {
        paths: idle_paths.clone(),
        state: "ready".to_string(),
        source_mtime_ns: 1,
        last_used_at: Some(1.0),
        size_bytes: None,
        extra: JsonMap::new(),
    })
    .unwrap();

    let leased_paths = test_cache_paths(&root, "live-leased-sibling");
    write(
        &leased_paths.package_dir.join("payload.bin"),
        "leased-cache",
    );
    write_current_source_native_cache_manifest_json(&CurrentSourceNativeCacheManifestRequest {
        paths: leased_paths.clone(),
        state: "ready".to_string(),
        source_mtime_ns: 1,
        last_used_at: Some(1.0),
        size_bytes: None,
        extra: JsonMap::new(),
    })
    .unwrap();
    register_current_source_native_cache_lease_json(&leased_paths, "live-sibling").unwrap();

    let lease = register_current_source_native_cache_lease_json(&current_paths, "shared").unwrap();
    let lease_path = PathBuf::from(lease["lease_path"].as_str().unwrap());
    let payload = release_current_source_native_cache_lease_json(
        &lease_path,
        &current_paths.namespace_root,
        false,
    )
    .unwrap();

    assert_eq!(payload["prune"]["removed_idle"], json!(["idle-sibling"]));
    assert!(current_paths.cache_root.exists());
    assert!(!idle_paths.cache_root.exists());
    assert!(leased_paths.cache_root.exists());
    fs::remove_dir_all(root).ok();
}

#[test]
fn cache_prune_removes_abandoned_builds_and_keeps_live_leases() {
    let root = temp_dir("prune");
    let stale_paths = test_cache_paths(&root, "stale-building");
    write(&stale_paths.package_dir.join("payload.bin"), "stale");
    write_current_source_native_cache_manifest_json(&CurrentSourceNativeCacheManifestRequest {
        paths: stale_paths.clone(),
        state: "building".to_string(),
        source_mtime_ns: 1,
        last_used_at: Some(10.0),
        size_bytes: None,
        extra: JsonMap::new(),
    })
    .unwrap();
    let live_paths = test_cache_paths(&root, "live-ready");
    write(&live_paths.package_dir.join("payload.bin"), "live");
    write_current_source_native_cache_manifest_json(&CurrentSourceNativeCacheManifestRequest {
        paths: live_paths.clone(),
        state: "ready".to_string(),
        source_mtime_ns: 1,
        last_used_at: Some(10.0),
        size_bytes: None,
        extra: JsonMap::new(),
    })
    .unwrap();
    register_current_source_native_cache_lease_json(&live_paths, "gw1").unwrap();

    let summary = prune_current_source_native_caches_json(&CurrentSourceNativeCachePruneRequest {
        namespace_root: stale_paths.namespace_root.clone(),
        now: Some(10_000.0),
        idle_ttl_seconds: CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS,
        build_stale_seconds: CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS,
        max_bytes: CURRENT_SOURCE_CACHE_MAX_BYTES,
        remove_unleased_ready: true,
    })
    .unwrap();

    assert_eq!(
        summary["removed_abandoned_builds"],
        json!(["stale-building"])
    );
    assert!(!stale_paths.cache_root.exists());
    assert!(live_paths.cache_root.exists());
    fs::remove_dir_all(root).ok();
}

#[test]
fn load_json_object_defaults_to_empty_for_missing_malformed_or_non_object() {
    let root = temp_dir("load-json-object-defaults");
    let missing = root.join("missing.json");
    let malformed = root.join("malformed.json");
    let array = root.join("array.json");
    let object = root.join("object.json");
    write(&malformed, "{");
    write(&array, "[]");
    write(&object, r#"{"state":"ready"}"#);

    assert!(load_json_object(&missing).is_empty());
    assert!(load_json_object(&malformed).is_empty());
    assert!(load_json_object(&array).is_empty());
    assert_eq!(
        load_json_object(&object)
            .get("state")
            .and_then(JsonValue::as_str),
        Some("ready")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn current_source_cache_json_loads_objects_and_defaults_to_empty() {
    let store = FakeCurrentSourceCacheFileIoStore::default();
    let missing = PathBuf::from("/cache/missing.json");
    let malformed = PathBuf::from("/cache/malformed.json");
    let array = PathBuf::from("/cache/array.json");
    let object = PathBuf::from("/cache/object.json");
    store.insert_file(malformed.clone(), "{");
    store.insert_file(array.clone(), "[]");
    store.insert_file(object.clone(), r#"{"state":"ready"}"#);
    let cache_json = CurrentSourceCacheJson::new(&store);

    assert!(cache_json.load_object_or_empty(&missing).is_empty());
    assert!(cache_json.load_object_or_empty(&malformed).is_empty());
    assert!(cache_json.load_object_or_empty(&array).is_empty());
    assert_eq!(
        cache_json
            .load_object_or_empty(&object)
            .get("state")
            .and_then(JsonValue::as_str),
        Some("ready")
    );
    assert_eq!(
        store.reads.borrow().as_slice(),
        [missing, malformed, array, object,]
    );
}

#[test]
fn atomic_write_json_writes_pretty_json_with_trailing_newline() {
    let root = temp_dir("atomic-json-newline");
    let path = root.join("nested/manifest.json");

    atomic_write_json(&path, &json!({"z": 1, "a": {"b": true}})).unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"a\": {\n    \"b\": true\n  },\n  \"z\": 1\n}\n"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn current_source_cache_json_atomic_write_uses_pretty_newline_and_label() {
    let store = FakeCurrentSourceCacheFileIoStore::default();
    let path = PathBuf::from("/cache/manifest.json");
    let cache_json = CurrentSourceCacheJson::new(&store);

    cache_json
        .write_pretty_json_atomically(&path, &json!({"z": 1, "a": {"b": true}}))
        .unwrap();

    let expected = "{\n  \"a\": {\n    \"b\": true\n  },\n  \"z\": 1\n}\n";
    assert_eq!(
        store.atomic_writes.borrow().as_slice(),
        [(
            path.clone(),
            expected.to_string(),
            "current-source JSON".to_string(),
        )]
    );
    assert_eq!(
        store.files.borrow().get(&path).map(String::as_str),
        Some(expected)
    );
}

#[test]
fn current_source_cache_json_atomic_write_failure_preserves_target() {
    let store = FakeCurrentSourceCacheFileIoStore {
        atomic_error: Some("rename failed".to_string()),
        ..Default::default()
    };
    let path = PathBuf::from("/cache/manifest.json");
    store.insert_file(path.clone(), "old manifest");
    let cache_json = CurrentSourceCacheJson::new(&store);

    let err = cache_json
        .write_pretty_json_atomically(&path, &json!({"state": "ready"}))
        .unwrap_err();

    assert_eq!(err, "rename failed");
    assert_eq!(
        store.files.borrow().get(&path).map(String::as_str),
        Some("old manifest")
    );
    assert_eq!(store.atomic_writes.borrow().len(), 1);
}

#[test]
fn prune_dead_leases_removes_malformed_missing_pid_and_dead_pid_leases() {
    let root = temp_dir("lease-prune-dead-json");
    let leases_dir = root.join("leases");
    fs::create_dir_all(&leases_dir).unwrap();
    let malformed = leases_dir.join("malformed.json");
    let missing_pid = leases_dir.join("missing-pid.json");
    let dead_pid = leases_dir.join("dead-pid.json");
    let live = leases_dir.join("live.json");
    write(&malformed, "{");
    write(&missing_pid, r#"{"worker_id":"missing"}"#);
    write(&dead_pid, r#"{"pid":-1}"#);
    write(&live, &format!(r#"{{"pid":{}}}"#, std::process::id()));

    let live_paths = prune_dead_leases(&leases_dir);

    assert_eq!(live_paths, vec![live.clone()]);
    assert!(!malformed.exists());
    assert!(!missing_pid.exists());
    assert!(!dead_pid.exists());
    assert!(live.exists());
    fs::remove_dir_all(root).ok();
}

#[test]
fn binary_freshness_accepts_unchanged_sibling_binary_after_source_fingerprint_moves() {
    let root = temp_dir("binary-fresh");
    let metadata_path = root.join(".current-source-build.json");
    let binary_path = root.join("release/ait-server-core-seam");
    write(&binary_path, "#!/bin/sh\nexit 0\n");
    make_executable(&binary_path);
    let binary_mtime_ns = path_mtime_ns(&binary_path).unwrap();
    let source_mtime_ns = binary_mtime_ns + 1_000_000;
    let binary_sha = artifact_sha256_hex(&binary_path).unwrap();
    atomic_write_json(
        &metadata_path,
        &json!({
            "server_source_fingerprint": "server-fingerprint-b",
            "server_source_mtime_ns": source_mtime_ns,
            "ait_server_core_seam_mtime_ns": binary_mtime_ns,
            "ait_server_core_seam_sha256": binary_sha,
        }),
    )
    .unwrap();

    let payload = current_source_binary_is_fresh_json(&CurrentSourceBinaryFreshnessRequest {
        metadata_path,
        binary_path,
        metadata_fingerprint_key: "server_source_fingerprint".to_string(),
        metadata_source_mtime_key: "server_source_mtime_ns".to_string(),
        metadata_mtime_key: "ait_server_core_seam_mtime_ns".to_string(),
        metadata_sha_key: "ait_server_core_seam_sha256".to_string(),
        source_mtime_ns,
        source_fingerprint: "server-fingerprint-b".to_string(),
    });

    assert_eq!(payload["fresh"], true);
    fs::remove_dir_all(root).ok();
}

#[test]
fn canonical_seed_copies_fresh_extension_cli_metadata_and_manifest() {
    let root = temp_dir("seed");
    let repo = root.join("worktree");
    let canonical = root.join("ait");
    let core = root.join("ait-core");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::create_dir_all(&core).unwrap();
    let canonical_extension_dir = canonical.join(".ait/runtime-extensions/ait_py");
    let canonical_extension_path = canonical_extension_dir.join("ait_py.cpython-314-darwin.so");
    write(&canonical_extension_path, "canonical-extension");
    let canonical_cli = canonical.join(".ait/cargo-target/release/ait-cli");
    write(&canonical_cli, "#!/bin/sh\nexit 0\n");
    make_executable(&canonical_cli);
    let source_mtime_ns = path_mtime_ns(&canonical_extension_path).unwrap();
    atomic_write_json(
        &canonical_extension_dir.join(".current-source-build.json"),
        &json!({
            "core_repo_root": path_text(&core),
            "core_source_fingerprint": "core-a",
            "core_source_mtime_ns": source_mtime_ns,
            "ait_cli_sha256": artifact_sha256_hex(&canonical_cli).unwrap(),
        }),
    )
    .unwrap();

    let payload = seed_current_source_native_cache_from_canonical_json(
        &CurrentSourceNativeCacheCanonicalSeedRequest {
            namespace_root: root.join("AIT_RAM/.ait-temp/repo-scope"),
            core_repo_root: core.clone(),
            repo_root: repo,
            canonical_repo_root: canonical.clone(),
            core_source_mtime_ns: source_mtime_ns,
            core_source_fingerprint: "core-a".to_string(),
            server_source_fingerprint: None,
            ext_suffix: ".cpython-314-darwin.so".to_string(),
            rustflags: String::new(),
            worker_id: "shared".to_string(),
        },
    )
    .unwrap();

    assert_eq!(payload["seeded"], true);
    let extension_path = PathBuf::from(payload["extension_path"].as_str().unwrap());
    let cli_path = PathBuf::from(payload["ait_cli_path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(&extension_path).unwrap(),
        "canonical-extension"
    );
    assert_eq!(
        fs::read_to_string(extension_path.parent().unwrap().join("__init__.py")).unwrap(),
        LOCAL_EXTENSION_INIT
    );
    assert_eq!(
        fs::read_to_string(&cli_path).unwrap(),
        "#!/bin/sh\nexit 0\n"
    );
    let metadata = load_json_object(
        &extension_path
            .parent()
            .unwrap()
            .join(".current-source-build.json"),
    );
    assert_eq!(
        metadata_text(&metadata, "core_source_fingerprint").as_deref(),
        Some("core-a")
    );
    assert_eq!(
        metadata_u64(&metadata, "core_source_mtime_ns"),
        Some(source_mtime_ns)
    );
    assert_eq!(
        metadata_text(&metadata, "ait_cli_sha256").as_deref(),
        Some(artifact_sha256_hex(&cli_path).unwrap().as_str())
    );
    assert_eq!(
        metadata_text(&metadata, "ait_cli_profile").as_deref(),
        Some("release")
    );
    let manifest = load_json_object(
        &PathBuf::from(payload["cache_root"].as_str().unwrap()).join("manifest.json"),
    );
    assert_eq!(metadata_text(&manifest, "state").as_deref(), Some("ready"));
    assert_eq!(
        metadata_text(&manifest, "seeded_from_canonical_repo_root").as_deref(),
        Some(path_text(&canonical).as_str())
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn canonical_core_build_refreshes_every_consumer_artifact_after_cargo_succeeds() {
    let repo_root = workspace_test_support::repository_root();
    let script = fs::read_to_string(repo_root.join("ait.sh")).expect("read canonical core helper");
    let function_start = script
        .find("refresh_build_artifact_mtimes() {")
        .expect("artifact refresh function");
    let function_end = script[function_start..]
        .find("\n}\n\ncargo_target_size()")
        .map(|offset| function_start + offset)
        .expect("artifact refresh function end");
    let refresh_function = &script[function_start..function_end];

    for artifact in [
        "ait-cli",
        "ait-agent",
        "ait-agent.exe",
        "ait-agent-worker",
        "ait-agent-worker.exe",
        "libait_py.dylib",
        "libait_py.so",
        "ait_py.dll",
    ] {
        assert!(
            refresh_function.contains(&format!("${{profile_dir}}/{artifact}")),
            "canonical core build does not refresh {artifact}"
        );
    }

    let cargo_build = script
        .rfind("run_cargo build --profile")
        .expect("canonical cargo build command");
    let refresh_call = script
        .rfind("    refresh_build_artifact_mtimes")
        .expect("post-build artifact refresh call");
    assert!(
        refresh_call > cargo_build,
        "artifact mtimes must refresh only after Cargo succeeds"
    );
}
