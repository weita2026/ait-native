use super::*;
use ait_core::binary_db::BinaryDbCommandScope;
use ait_core::content_binary_db::{
    snapshot_id_from_hash48, BinarySnapshotPayload, BinarySnapshotRecord,
};
use ait_core::json_support::json;
use ait_core::line_store::LineStore;
use tempfile::TempDir;

const TEST_SEED_SNAPSHOT_HASH48: u64 = 1;
const TEST_SEED_SNAPSHOT_ID: &str = "SNP-000000000001";

struct FakeTaskWorktreeOps {
    platform: TaskWorktreePlatform,
    linux_roots: Vec<PathBuf>,
    windows_roots: Vec<PathBuf>,
    macos_specs: RefCell<Vec<TaskWorktreeMemoryRoot>>,
    default_macos_spec: TaskWorktreeMemoryRoot,
    provision_roots: BTreeSet<String>,
    denied_roots: BTreeSet<String>,
}

struct FakeSnapshotStore {
    total_bytes: Option<i64>,
}

impl SnapshotStore for FakeSnapshotStore {
    fn snapshot_exists(&self, _snapshot_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    fn snapshot_parent_link(
        &self,
        _snapshot_id: &str,
    ) -> Result<Option<ait_core::snapshot_store::SnapshotParentLink>, String> {
        Ok(None)
    }

    fn snapshot_by_id(
        &self,
        _snapshot_id: &str,
    ) -> Result<Option<ait_core::snapshot_store::SnapshotRecord>, String> {
        Ok(None)
    }

    fn list_line_snapshots(&self) -> Result<Vec<ait_core::snapshot_store::SnapshotRecord>, String> {
        Ok(Vec::new())
    }

    fn snapshot_total_bytes(&self, _snapshot_id: &str) -> Result<Option<i64>, String> {
        Ok(self.total_bytes)
    }

    fn snapshot_root_tree_pack_id(&self, _snapshot_id: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn snapshot_kind(&self, _snapshot_id: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn snapshot_chain(&self, snapshot_id: &str) -> Result<Vec<String>, String> {
        Ok(vec![snapshot_id.to_string()])
    }

    fn set_snapshot_kind(&self, _snapshot_id: &str, _snapshot_kind: &str) -> Result<usize, String> {
        Ok(0)
    }
}

impl FakeTaskWorktreeOps {
    fn linux(root: PathBuf) -> Self {
        Self {
            platform: TaskWorktreePlatform::Linux,
            linux_roots: vec![root],
            windows_roots: Vec::new(),
            macos_specs: RefCell::new(Vec::new()),
            default_macos_spec: default_macos_ram_volume_spec(),
            provision_roots: BTreeSet::new(),
            denied_roots: BTreeSet::new(),
        }
    }

    fn windows(root: PathBuf) -> Self {
        Self {
            platform: TaskWorktreePlatform::Windows,
            linux_roots: Vec::new(),
            windows_roots: vec![root],
            macos_specs: RefCell::new(Vec::new()),
            default_macos_spec: default_macos_ram_volume_spec(),
            provision_roots: BTreeSet::new(),
            denied_roots: BTreeSet::new(),
        }
    }

    fn macos(default_root: PathBuf) -> Self {
        let default_spec = TaskWorktreeMemoryRoot {
            kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
            root: default_root.clone(),
            volume_name: Some(
                default_root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(DEFAULT_MACOS_RAM_VOLUME_NAME)
                    .to_string(),
            ),
            sector_count: Some(DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT),
        };
        Self {
            platform: TaskWorktreePlatform::Macos,
            linux_roots: Vec::new(),
            windows_roots: Vec::new(),
            macos_specs: RefCell::new(Vec::new()),
            default_macos_spec: default_spec,
            provision_roots: BTreeSet::new(),
            denied_roots: BTreeSet::new(),
        }
    }
}

impl TaskWorktreeOps for FakeTaskWorktreeOps {
    fn platform(&self) -> TaskWorktreePlatform {
        self.platform
    }

    fn linux_detected_memory_roots(&self) -> Vec<PathBuf> {
        self.linux_roots.clone()
    }

    fn windows_ramdisk_roots(&self) -> Vec<PathBuf> {
        self.windows_roots.clone()
    }

    fn macos_ram_volume_specs(&self) -> Vec<TaskWorktreeMemoryRoot> {
        self.macos_specs.borrow().clone()
    }

    fn macos_default_ram_volume_spec(&self) -> TaskWorktreeMemoryRoot {
        self.default_macos_spec.clone()
    }

    fn ensure_memory_root_available(&self, spec: &TaskWorktreeMemoryRoot) -> bool {
        match spec.kind {
            TaskWorktreeMemoryRootKind::LinuxMemoryRoot => self.linux_roots.contains(&spec.root),
            TaskWorktreeMemoryRootKind::WindowsRamdisk => self.windows_roots.contains(&spec.root),
            TaskWorktreeMemoryRootKind::MacosRamVolume => {
                if self
                    .macos_specs
                    .borrow()
                    .iter()
                    .any(|candidate| candidate.root == spec.root)
                {
                    return true;
                }
                let key = spec.root.to_string_lossy().to_string();
                if !self.provision_roots.contains(&key) {
                    return false;
                }
                self.macos_specs.borrow_mut().push(spec.clone());
                true
            }
        }
    }

    fn ensure_root_candidate(&self, path: &Path) -> Option<PathBuf> {
        let key = path.to_string_lossy().to_string();
        if self.denied_roots.contains(&key) {
            return None;
        }
        Some(resolve_path_strict_false(path))
    }
}

fn init_test_repo_with_snapshot_total_bytes(total_bytes: u64) -> (TempDir, RepoRuntime) {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait/binary-db")).unwrap();
    fs::create_dir_all(root.join(".ait/objects/packs")).unwrap();
    fs::create_dir_all(root.join(".ait/objects/tree-packs")).unwrap();
    fs::write(
        root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "task_worktree": {}
}
"#,
    )
    .unwrap();
    let runtime = RepoRuntime::discover_from_path(root).unwrap();
    let stores = runtime.binary_db_stores::<1>();
    let snapshots = stores.content().snapshots().clone();
    let mut write = snapshots
        .begin_write_txn(BinaryDbCommandScope::ContentWrite)
        .unwrap();
    let (_, snapshot_id, _) = snapshots
        .append_snapshot_with_id_index(
            &mut write,
            BinarySnapshotRecord {
                snapshot_meta: 0,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: TEST_SEED_SNAPSHOT_HASH48,
                parent_snapshot_index_plus1: 0,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                line_index_plus1: 0,
                manifest_hash: [0; 32],
                file_count: 0,
                total_bytes,
                created_at_s: 1,
            },
            &BinarySnapshotPayload {
                line_name: "main".to_string(),
                message: None,
                additional_parent_snapshot_indices: Vec::new(),
            },
        )
        .unwrap();
    write.commit().unwrap();
    assert_eq!(
        snapshot_id,
        snapshot_id_from_hash48(TEST_SEED_SNAPSHOT_HASH48)
    );
    assert_eq!(snapshot_id, TEST_SEED_SNAPSHOT_ID);
    stores
        .lines()
        .create_line("main", Some(&snapshot_id), "2026-06-11T00:00:00Z")
        .unwrap();
    fs::create_dir_all(root.join(".ait/refs/lines")).unwrap();
    fs::write(
        root.join(".ait/refs/lines/main"),
        format!("{snapshot_id}\n"),
    )
    .unwrap();
    (temp, runtime)
}

fn write_config(repo: &RepoRuntime, value: JsonValue) {
    fs::write(
        repo.authoritative_repo_root().join(".ait/config.json"),
        crate::json_support::encode_value_pretty_with_newline_error_string(&value).unwrap(),
    )
    .unwrap();
}

fn repo_with_task_worktree(task_worktree: JsonValue) -> (TempDir, RepoRuntime) {
    repo_with_task_worktree_and_snapshot_total_bytes(task_worktree, 0)
}

fn repo_with_task_worktree_and_snapshot_total_bytes(
    task_worktree: JsonValue,
    snapshot_total_bytes: u64,
) -> (TempDir, RepoRuntime) {
    let (temp, repo) = init_test_repo_with_snapshot_total_bytes(snapshot_total_bytes);
    write_config(
        &repo,
        json!({
            "repo_name": "fixture-ait",
            "default_line": "main",
            "task_worktree": task_worktree,
        }),
    );
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    (temp, repo)
}

fn repo_with_registered_task_worktree(
    task_worktree: JsonValue,
    repository_index: u32,
) -> (TempDir, RepoRuntime) {
    let (temp, repo) = init_test_repo_with_snapshot_total_bytes(0);
    write_config(
        &repo,
        json!({
            "repo_name": "fixture-ait",
            "repository_index": repository_index,
            "default_line": "main",
            "task_worktree": task_worktree,
        }),
    );
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    (temp, repo)
}

#[test]
fn configured_shared_root_isolates_same_name_unregistered_repositories() {
    let shared_temp = TempDir::new().unwrap();
    let shared_root = shared_temp.path().join("shared-worktrees");
    let task_worktree = json!({
        "ephemeral_root": shared_root.to_string_lossy().to_string(),
    });
    let (_first_temp, first_repo) = repo_with_task_worktree(task_worktree.clone());
    let (_second_temp, second_repo) = repo_with_task_worktree(task_worktree);
    let ops = FakeTaskWorktreeOps::linux(shared_root.clone());
    let resolved_shared_root = resolve_path_strict_false(&shared_root);

    let first = resolve_task_worktree_location_with_ops(&first_repo, "t-0001", &ops);
    let second = resolve_task_worktree_location_with_ops(&second_repo, "t-0001", &ops);

    assert_eq!(first.root_source, "configured_ephemeral_root");
    assert_eq!(second.root_source, "configured_ephemeral_root");
    assert_eq!(
        first.target_path,
        configured_repository_worktree_root(&first_repo, &resolved_shared_root).join("t-0001")
    );
    assert_eq!(
        second.target_path,
        configured_repository_worktree_root(&second_repo, &resolved_shared_root).join("t-0001")
    );
    assert_ne!(
        configured_repository_scope_segment(&first_repo),
        configured_repository_scope_segment(&second_repo)
    );
    assert_ne!(first.target_path, second.target_path);
}

#[test]
fn configured_shared_root_includes_repository_index_and_root_hash() {
    let shared_temp = TempDir::new().unwrap();
    let shared_root = shared_temp.path().join("shared-worktrees");
    let (_repo_temp, repo) = repo_with_registered_task_worktree(
        json!({
            "ephemeral_root": shared_root.to_string_lossy().to_string(),
        }),
        27,
    );
    let ops = FakeTaskWorktreeOps::linux(shared_root.clone());
    let resolved_shared_root = resolve_path_strict_false(&shared_root);

    let location = resolve_task_worktree_location_with_ops(&repo, "t-0002", &ops);

    assert_eq!(
        configured_repository_scope_segment(&repo),
        format!("r27-{}", authoritative_repo_root_hash12(&repo))
    );
    assert_eq!(
        location.target_path,
        resolved_shared_root
            .join(configured_repository_scope_segment(&repo))
            .join("fixture-ait")
            .join("t-0002")
    );
}

#[test]
fn linux_candidate_chain_skips_stale_configured_root() {
    let temp = TempDir::new().unwrap();
    let current_root = temp.path().join("runtime-root");
    let missing_root = temp.path().join("missing-runtime-root");
    let (_repo_temp, repo) = repo_with_task_worktree(json!({
        "ephemeral_root": auto_detected_ephemeral_root_from_paths(temp.path(), &missing_root),
        "memory_root": {
            "kind": "linux_memory_root",
            "root": missing_root.to_string_lossy().to_string(),
        },
    }));
    let ops = FakeTaskWorktreeOps::linux(current_root.clone());

    let location = resolve_task_worktree_location_with_ops(&repo, "rt-123", &ops);

    assert_eq!(location.root_source, "linux_memory_root");
    assert_eq!(
        location.target_path,
        auto_detected_ephemeral_root(&repo, &current_root)
            .join(repo_path_segment(&repo))
            .join("rt-123")
    );
}

#[test]
fn main_seed_ram_budget_status_reads_snapshot_total_bytes_from_store() {
    let (_temp, repo) = repo_with_task_worktree(json!({
        "main_seed_ram_max_bytes": 10,
    }));
    let store = FakeSnapshotStore {
        total_bytes: Some(5),
    };

    let budget = main_seed_ram_budget_status_with_snapshot_store(&repo, &store);

    assert_eq!(
        budget,
        Some(MainSeedRamBudgetStatus {
            default_line: "main".to_string(),
            seed_snapshot_id: TEST_SEED_SNAPSHOT_ID.to_string(),
            seed_snapshot_total_bytes: 5,
            main_seed_ram_max_bytes: 10,
            exceeded: false,
        })
    );
}

#[test]
fn linux_budget_guard_falls_back_and_disables_main_seed_mirror() {
    let (_temp, repo) =
        repo_with_task_worktree_and_snapshot_total_bytes(json!({"main_seed_ram_max_bytes": 1}), 5);
    let ops = FakeTaskWorktreeOps::linux(PathBuf::from("/tmp/runtime-root"));
    let budget = main_seed_ram_budget_status(&repo);
    assert_eq!(
        budget,
        Some(MainSeedRamBudgetStatus {
            default_line: "main".to_string(),
            seed_snapshot_id: TEST_SEED_SNAPSHOT_ID.to_string(),
            seed_snapshot_total_bytes: 5,
            main_seed_ram_max_bytes: 1,
            exceeded: true,
        })
    );

    let location = resolve_task_worktree_location_with_ops(&repo, "rt-123", &ops);
    let seed_location = resolve_main_seed_mirror_location_with_ops(&repo, "main-seed", &ops);

    assert_eq!(location.root_source, "repo_internal_fallback");
    assert_eq!(
        location.fallback_reason.as_deref(),
        Some("main_seed_ram_budget_exceeded")
    );
    assert_eq!(location.seed_snapshot_total_bytes, Some(5));
    assert_eq!(seed_location, None);
}

#[test]
fn windows_candidate_chain_uses_current_ramdisk() {
    let temp = TempDir::new().unwrap();
    let current_root = temp.path().join("CurrentRamDisk");
    let missing_root = temp.path().join("MissingRamDisk");
    let (_repo_temp, repo) = repo_with_task_worktree(json!({
        "ephemeral_root": auto_detected_ephemeral_root_from_paths(temp.path(), &missing_root),
        "memory_root": {
            "kind": "windows_ramdisk",
            "root": missing_root.to_string_lossy().to_string(),
        },
    }));
    let ops = FakeTaskWorktreeOps::windows(current_root.clone());

    let location = resolve_task_worktree_location_with_ops(&repo, "rt-456", &ops);

    assert_eq!(location.root_source, "windows_ramdisk");
    assert_eq!(
        location.target_path,
        auto_detected_ephemeral_root(&repo, &current_root)
            .join(repo_path_segment(&repo))
            .join("rt-456")
    );
}

#[test]
fn macos_reprovisions_saved_spec() {
    let temp = TempDir::new().unwrap();
    let ram_root = temp.path().join("Volumes").join("AIT_RAM");
    let (_repo_temp, repo) = repo_with_task_worktree(json!({
        "memory_root": {
            "kind": "macos_ram_volume",
            "root": ram_root.to_string_lossy().to_string(),
            "volume_name": "AIT_RAM",
            "sector_count": 4194304,
        },
    }));
    let ops = FakeTaskWorktreeOps {
        provision_roots: BTreeSet::from([ram_root.to_string_lossy().to_string()]),
        ..FakeTaskWorktreeOps::macos(ram_root.clone())
    };
    let normalized_memory_root = task_worktree_config_value(&repo, "memory_root")
        .and_then(normalize_task_worktree_memory_root);
    assert_eq!(
        effective_task_worktree_ephemeral_root_base(&repo, normalized_memory_root.as_ref()),
        Some(auto_detected_ephemeral_root(&repo, &ram_root))
    );

    let location = resolve_task_worktree_location_with_ops(&repo, "rt-789", &ops);

    assert_eq!(location.root_source, "macos_ram_volume");
    assert_eq!(
        location.target_path,
        auto_detected_ephemeral_root(&repo, &ram_root)
            .join(repo_path_segment(&repo))
            .join("rt-789")
    );
}

#[test]
fn macos_bootstraps_default_spec_when_unmounted() {
    let temp = TempDir::new().unwrap();
    let ram_root = temp.path().join("Volumes").join("AIT_RAM");
    let (_repo_temp, repo) = repo_with_task_worktree(json!({}));
    let ops = FakeTaskWorktreeOps {
        provision_roots: BTreeSet::from([ram_root.to_string_lossy().to_string()]),
        ..FakeTaskWorktreeOps::macos(ram_root.clone())
    };

    let location = resolve_task_worktree_location_with_ops(&repo, "rt-790", &ops);

    assert_eq!(location.root_source, "macos_ram_volume");
    assert_eq!(
        location.target_path,
        auto_detected_ephemeral_root(&repo, &ram_root)
            .join(repo_path_segment(&repo))
            .join("rt-790")
    );
}

#[test]
fn macos_legacy_configured_root_reprovisions_from_path() {
    let (_repo_temp, repo) = repo_with_task_worktree(json!({
        "ephemeral_root": "/Volumes/AIT_RAM/.ait-repos/123456789abc",
    }));
    let expected_root = resolve_path_strict_false(Path::new("/Volumes/AIT_RAM"));
    let ops = FakeTaskWorktreeOps {
        provision_roots: BTreeSet::from([expected_root.to_string_lossy().to_string()]),
        default_macos_spec: TaskWorktreeMemoryRoot {
            kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
            root: expected_root.clone(),
            volume_name: Some("AIT_RAM".to_string()),
            sector_count: Some(DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT),
        },
        ..FakeTaskWorktreeOps::macos(expected_root.clone())
    };

    let location = resolve_task_worktree_location_with_ops(&repo, "rt-791", &ops);
    let configured_base =
        resolve_path_strict_false(Path::new("/Volumes/AIT_RAM/.ait-repos/123456789abc"));

    assert_eq!(location.root_source, "configured_ephemeral_root");
    assert_eq!(
        location.target_path,
        configured_repository_worktree_root(&repo, &configured_base).join("rt-791")
    );
}

#[test]
fn debug_override_payload_resolves_linux_location() {
    let temp = TempDir::new().unwrap();
    let runtime_root = temp.path().join("runtime-root");
    let (_repo_temp, repo) = repo_with_task_worktree(json!({}));

    let location = resolve_task_worktree_location_with_debug(
        &repo,
        "rt-900",
        Some(&json!({
            "platform": "linux",
            "linux_detected_memory_roots": [runtime_root.to_string_lossy().to_string()],
        })),
    )
    .unwrap();

    assert_eq!(location.root_source, "linux_memory_root");
    assert_eq!(
        location.target_path,
        auto_detected_ephemeral_root(&repo, &runtime_root)
            .join(repo_path_segment(&repo))
            .join("rt-900")
    );
}

#[test]
fn memory_root_json_round_trip_preserves_macos_fields() {
    let value = json!({
        "kind": "macos_ram_volume",
        "root": "/Volumes/AIT_RAM",
        "volume_name": "AIT_RAM",
        "sector_count": 4194304,
    });

    let spec = normalize_task_worktree_memory_root(&value).unwrap();

    assert_eq!(spec.to_json()["kind"], "macos_ram_volume");
    assert_eq!(spec.to_json()["volume_name"], "AIT_RAM");
}

#[test]
fn macos_hdiutil_parser_requires_explicit_writable_ram_image_proof() {
    fn image(writeable: Option<bool>) -> PlistValue {
        let mut image = plist::Dictionary::new();
        image.insert(
            "image-path".to_string(),
            PlistValue::String("ram://16777216".to_string()),
        );
        if let Some(writeable) = writeable {
            image.insert("writeable".to_string(), PlistValue::Boolean(writeable));
        }
        let mut entity = plist::Dictionary::new();
        entity.insert(
            "mount-point".to_string(),
            PlistValue::String("/Volumes/AIT_RAM".to_string()),
        );
        image.insert(
            "system-entities".to_string(),
            PlistValue::Array(vec![PlistValue::Dictionary(entity)]),
        );
        PlistValue::Dictionary(image)
    }

    assert!(macos_ram_volume_specs_from_plist(&[image(None)]).is_empty());
    assert!(macos_ram_volume_specs_from_plist(&[image(Some(false))]).is_empty());
    let specs = macos_ram_volume_specs_from_plist(&[image(Some(true))]);
    assert_eq!(specs.len(), 1);
    assert_eq!(
        specs[0].sector_count,
        Some(DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT)
    );
}

fn auto_detected_ephemeral_root_from_paths(repo_root: &Path, root: &Path) -> String {
    let hash = sha256_hex_bytes(repo_root.to_string_lossy().as_bytes())[..12].to_string();
    resolve_path_strict_false(root)
        .join(AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME)
        .join(hash)
        .to_string_lossy()
        .to_string()
}
