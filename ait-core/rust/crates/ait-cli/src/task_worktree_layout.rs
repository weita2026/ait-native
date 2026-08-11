use crate::runtime::{RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT};
use ait_core::json_support::{json, JsonValue};
use ait_core::snapshot_store::SnapshotStore;
use plist::Value as PlistValue;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

#[path = "task_worktree_layout_ports.rs"]
mod task_worktree_layout_ports;

#[path = "task_worktree_layout/memory_lifecycle.rs"]
mod memory_lifecycle;

pub(crate) use self::memory_lifecycle::doctor_memory_root_payload;

use self::task_worktree_layout_ports::{
    TaskWorktreeMemoryRoot, TaskWorktreeMemoryRootKind, TaskWorktreeOps, TaskWorktreePlatform,
    DEFAULT_MACOS_RAM_VOLUME_NAME, DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT, LINUX_MEMORY_ROOT_KIND,
    MACOS_RAM_VOLUME_KIND, WINDOWS_RAMDISK_KIND,
};

const DEFAULT_TASK_WORKTREE_ALIAS_ROOT: &str = ".ait-worktree-links";
const TASK_WORKTREE_ROOT_DIRNAME: &str = ".ait-worktree";
const INTERNAL_WORKTREE_ROOT_DIRNAME: &str = ".ait-internal";
const AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME: &str = ".ait-repos";
const LINUX_MEMORY_BACKED_FSTYPES: &[&str] = &["tmpfs", "ramfs"];
const WINDOWS_DRIVE_RAMDISK: u32 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MainSeedRamBudgetStatus {
    pub(crate) default_line: String,
    pub(crate) seed_snapshot_id: String,
    pub(crate) seed_snapshot_total_bytes: i64,
    pub(crate) main_seed_ram_max_bytes: i64,
    pub(crate) exceeded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManagedWorktreeRoot {
    target_root: PathBuf,
    root_source: String,
    ephemeral_enabled: bool,
    fallback_reason: Option<String>,
    default_line: Option<String>,
    seed_snapshot_id: Option<String>,
    seed_snapshot_total_bytes: Option<i64>,
    main_seed_ram_max_bytes: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManagedWorktreeLocation {
    pub(crate) target_path: PathBuf,
    pub(crate) alias_path: Option<PathBuf>,
    pub(crate) preferred_path: PathBuf,
    pub(crate) root_source: String,
    pub(crate) ephemeral_enabled: bool,
    pub(crate) fallback_reason: Option<String>,
    pub(crate) default_line: Option<String>,
    pub(crate) seed_snapshot_id: Option<String>,
    pub(crate) seed_snapshot_total_bytes: Option<i64>,
    pub(crate) main_seed_ram_max_bytes: Option<i64>,
}

impl ManagedWorktreeLocation {
    pub(crate) fn to_json(&self) -> JsonValue {
        json!({
            "target_path": self.target_path.to_string_lossy().to_string(),
            "alias_path": self.alias_path.as_ref().map(|value| value.to_string_lossy().to_string()),
            "preferred_path": self.preferred_path.to_string_lossy().to_string(),
            "root_source": self.root_source,
            "ephemeral_enabled": self.ephemeral_enabled,
            "fallback_reason": self.fallback_reason,
            "default_line": self.default_line,
            "seed_snapshot_id": self.seed_snapshot_id,
            "seed_snapshot_total_bytes": self.seed_snapshot_total_bytes,
            "main_seed_ram_max_bytes": self.main_seed_ram_max_bytes,
        })
    }
}

pub(crate) fn config_task_worktree_summary(repo: &RepoRuntime) -> JsonValue {
    let stored_memory_root = task_worktree_config_value(repo, "memory_root")
        .and_then(normalize_task_worktree_memory_root);
    let stored_ephemeral_root = task_worktree_config_value(repo, "ephemeral_root")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .map(|raw| resolve_configured_path(&repo.authoritative_repo_root(), &raw));
    let derived_ephemeral_root = if stored_ephemeral_root.is_none() {
        stored_memory_root
            .as_ref()
            .map(|spec| auto_detected_ephemeral_root(repo, &spec.root))
    } else {
        None
    };
    let alias_root = task_worktree_config_value(repo, "alias_root")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .unwrap_or_else(|| DEFAULT_TASK_WORKTREE_ALIAS_ROOT.to_string());
    let alias_root_source = if task_worktree_config_value(repo, "alias_root").is_some() {
        "repo_config"
    } else {
        "built_in"
    };
    let main_seed_ram_max_bytes = task_worktree_config_value(repo, "main_seed_ram_max_bytes")
        .and_then(normalize_main_seed_ram_max_bytes_value);
    json!({
        "ephemeral_root": {
            "value": stored_ephemeral_root
                .as_ref()
                .or(derived_ephemeral_root.as_ref())
                .map(|value| value.to_string_lossy().to_string()),
            "source": if stored_ephemeral_root.is_some() {
                "repo_config"
            } else if derived_ephemeral_root.is_some() {
                "derived_from_memory_root"
            } else {
                "built_in"
            },
        },
        "alias_root": {
            "value": alias_root,
            "source": alias_root_source,
        },
        "memory_root": {
            "value": stored_memory_root.as_ref().map(TaskWorktreeMemoryRoot::to_json).unwrap_or(JsonValue::Null),
            "source": if stored_memory_root.is_some() {
                "repo_config"
            } else {
                "built_in"
            },
        },
        "main_seed_ram_max_bytes": {
            "value": main_seed_ram_max_bytes,
            "source": if main_seed_ram_max_bytes.is_some() {
                "repo_config"
            } else {
                "built_in"
            },
        },
    })
}

pub(crate) fn detect_init_task_worktree_defaults(repo: &RepoRuntime) -> Option<JsonValue> {
    let memory_root = match TaskWorktreePlatform::current() {
        TaskWorktreePlatform::Linux => {
            linux_detected_memory_roots()
                .into_iter()
                .next()
                .map(|root| {
                    TaskWorktreeMemoryRoot {
                        kind: TaskWorktreeMemoryRootKind::LinuxMemoryRoot,
                        root,
                        volume_name: None,
                        sector_count: None,
                    }
                    .to_json()
                })
        }
        TaskWorktreePlatform::Windows => windows_ramdisk_roots().into_iter().next().map(|root| {
            TaskWorktreeMemoryRoot {
                kind: TaskWorktreeMemoryRootKind::WindowsRamdisk,
                root,
                volume_name: None,
                sector_count: None,
            }
            .to_json()
        }),
        TaskWorktreePlatform::Macos => macos_ram_volume_specs()
            .into_iter()
            .next()
            .map(|spec| spec.to_json()),
        TaskWorktreePlatform::Other => None,
    }?;
    let _ = repo;
    Some(json!({ "memory_root": memory_root }))
}

struct SystemTaskWorktreeOps;

impl TaskWorktreeOps for SystemTaskWorktreeOps {
    fn platform(&self) -> TaskWorktreePlatform {
        TaskWorktreePlatform::current()
    }

    fn linux_detected_memory_roots(&self) -> Vec<PathBuf> {
        linux_detected_memory_roots()
    }

    fn windows_ramdisk_roots(&self) -> Vec<PathBuf> {
        windows_ramdisk_roots()
    }

    fn macos_ram_volume_specs(&self) -> Vec<TaskWorktreeMemoryRoot> {
        macos_ram_volume_specs()
    }

    fn macos_default_ram_volume_spec(&self) -> TaskWorktreeMemoryRoot {
        default_macos_ram_volume_spec()
    }

    fn ensure_memory_root_available(&self, spec: &TaskWorktreeMemoryRoot) -> bool {
        ensure_memory_root_available_system(spec)
    }

    fn ensure_root_candidate(&self, path: &Path) -> Option<PathBuf> {
        ensure_root_candidate_system(path)
    }
}

struct DebugTaskWorktreeOps {
    platform: TaskWorktreePlatform,
    linux_detected_roots: Vec<PathBuf>,
    windows_ramdisk_roots: Vec<PathBuf>,
    macos_specs: RefCell<Vec<TaskWorktreeMemoryRoot>>,
    macos_default_spec: TaskWorktreeMemoryRoot,
    macos_provision_success_roots: BTreeSet<String>,
}

impl DebugTaskWorktreeOps {
    fn from_json(value: &JsonValue) -> Result<Self, String> {
        let payload = value.as_object().ok_or_else(|| {
            "task-worktree debug probe override must decode to an object.".to_string()
        })?;
        let linux_detected_roots = path_list_field(payload.get("linux_detected_memory_roots"))?;
        let windows_ramdisk_roots = path_list_field(payload.get("windows_ramdisk_roots"))?;
        let macos_specs = memory_root_list_field(payload.get("macos_ram_volume_specs"))?;
        let macos_default_spec = payload
            .get("macos_default_ram_volume_spec")
            .and_then(normalize_task_worktree_memory_root)
            .unwrap_or_else(default_macos_ram_volume_spec);
        let macos_provision_success_roots =
            path_list_field(payload.get("macos_provision_success_roots"))?
                .into_iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect();
        Ok(Self {
            platform: TaskWorktreePlatform::from_text(
                payload.get("platform").and_then(JsonValue::as_str),
            ),
            linux_detected_roots,
            windows_ramdisk_roots,
            macos_specs: RefCell::new(macos_specs),
            macos_default_spec,
            macos_provision_success_roots,
        })
    }
}

impl TaskWorktreeOps for DebugTaskWorktreeOps {
    fn platform(&self) -> TaskWorktreePlatform {
        self.platform
    }

    fn linux_detected_memory_roots(&self) -> Vec<PathBuf> {
        self.linux_detected_roots.clone()
    }

    fn windows_ramdisk_roots(&self) -> Vec<PathBuf> {
        self.windows_ramdisk_roots.clone()
    }

    fn macos_ram_volume_specs(&self) -> Vec<TaskWorktreeMemoryRoot> {
        self.macos_specs.borrow().clone()
    }

    fn macos_default_ram_volume_spec(&self) -> TaskWorktreeMemoryRoot {
        self.macos_default_spec.clone()
    }

    fn ensure_memory_root_available(&self, spec: &TaskWorktreeMemoryRoot) -> bool {
        match spec.kind {
            TaskWorktreeMemoryRootKind::LinuxMemoryRoot => {
                self.linux_detected_roots.contains(&spec.root)
            }
            TaskWorktreeMemoryRootKind::WindowsRamdisk => {
                self.windows_ramdisk_roots.contains(&spec.root)
            }
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
                if !self.macos_provision_success_roots.contains(&key) {
                    return false;
                }
                self.macos_specs.borrow_mut().push(spec.clone());
                true
            }
        }
    }

    fn ensure_root_candidate(&self, path: &Path) -> Option<PathBuf> {
        ensure_root_candidate_system(path)
    }
}

pub(crate) fn resolve_task_worktree_location_with_debug(
    repo: &RepoRuntime,
    worktree_name: &str,
    debug_probe_override: Option<&JsonValue>,
) -> Result<ManagedWorktreeLocation, String> {
    match debug_probe_override {
        Some(override_value) => {
            let debug_ops = DebugTaskWorktreeOps::from_json(override_value)?;
            Ok(resolve_task_worktree_location_with_ops(
                repo,
                worktree_name,
                &debug_ops,
            ))
        }
        None => Ok(resolve_task_worktree_location_with_ops(
            repo,
            worktree_name,
            &SystemTaskWorktreeOps,
        )),
    }
}

pub(crate) fn resolve_main_seed_mirror_location(
    repo: &RepoRuntime,
    seed_name: &str,
) -> Option<ManagedWorktreeLocation> {
    resolve_main_seed_mirror_location_with_ops(repo, seed_name, &SystemTaskWorktreeOps)
}

fn resolve_task_worktree_location_with_ops(
    repo: &RepoRuntime,
    worktree_name: &str,
    ops: &impl TaskWorktreeOps,
) -> ManagedWorktreeLocation {
    let alias_base = resolve_task_worktree_alias_base(repo);
    let root_info = resolve_managed_worktree_root(repo, ops);
    let target_path = root_info.target_root.join(worktree_name);
    let alias_path = root_info
        .ephemeral_enabled
        .then(|| alias_base.join(worktree_name));
    let preferred_path = alias_path.clone().unwrap_or_else(|| target_path.clone());
    ManagedWorktreeLocation {
        target_path,
        alias_path,
        preferred_path,
        root_source: root_info.root_source,
        ephemeral_enabled: root_info.ephemeral_enabled,
        fallback_reason: root_info.fallback_reason,
        default_line: root_info.default_line,
        seed_snapshot_id: root_info.seed_snapshot_id,
        seed_snapshot_total_bytes: root_info.seed_snapshot_total_bytes,
        main_seed_ram_max_bytes: root_info.main_seed_ram_max_bytes,
    }
}

fn resolve_main_seed_mirror_location_with_ops(
    repo: &RepoRuntime,
    seed_name: &str,
    ops: &impl TaskWorktreeOps,
) -> Option<ManagedWorktreeLocation> {
    let root_info = resolve_managed_worktree_root(repo, ops);
    if !root_info.ephemeral_enabled {
        return None;
    }
    let target_path = root_info
        .target_root
        .join(INTERNAL_WORKTREE_ROOT_DIRNAME)
        .join(seed_name);
    Some(ManagedWorktreeLocation {
        preferred_path: target_path.clone(),
        target_path,
        alias_path: None,
        root_source: root_info.root_source,
        ephemeral_enabled: true,
        fallback_reason: root_info.fallback_reason,
        default_line: root_info.default_line,
        seed_snapshot_id: root_info.seed_snapshot_id,
        seed_snapshot_total_bytes: root_info.seed_snapshot_total_bytes,
        main_seed_ram_max_bytes: root_info.main_seed_ram_max_bytes,
    })
}

fn resolve_managed_worktree_root(
    repo: &RepoRuntime,
    ops: &impl TaskWorktreeOps,
) -> ManagedWorktreeRoot {
    let default_root = repo
        .authoritative_repo_root()
        .join(TASK_WORKTREE_ROOT_DIRNAME);
    let normalized_memory_root = task_worktree_config_value(repo, "memory_root")
        .and_then(normalize_task_worktree_memory_root);
    let configured_ephemeral_root = task_worktree_config_value(repo, "ephemeral_root");
    let effective_ephemeral_root =
        effective_task_worktree_ephemeral_root_base(repo, normalized_memory_root.as_ref());
    let budget_status = main_seed_ram_budget_status(repo);
    let memory_roots_allowed = budget_status
        .as_ref()
        .map(|status| !status.exceeded)
        .unwrap_or(true);

    if configured_ephemeral_root.is_some() {
        if let Some(target_root) = ensure_configured_ephemeral_root(
            repo,
            ops,
            effective_ephemeral_root.as_deref(),
            normalized_memory_root.as_ref(),
            memory_roots_allowed,
        ) {
            return ManagedWorktreeRoot {
                target_root,
                root_source: "configured_ephemeral_root".to_string(),
                ephemeral_enabled: true,
                fallback_reason: None,
                default_line: None,
                seed_snapshot_id: None,
                seed_snapshot_total_bytes: None,
                main_seed_ram_max_bytes: None,
            };
        }
    }

    if memory_roots_allowed && configured_ephemeral_root.is_none() {
        if let Some(memory_root) = normalized_memory_root.as_ref() {
            let _ = ops.ensure_memory_root_available(memory_root);
        }
    }

    if memory_roots_allowed && ops.platform() == TaskWorktreePlatform::Linux {
        for (candidate, source) in linux_ephemeral_root_candidates(repo, ops) {
            if let Some(target_root) = ops.ensure_root_candidate(&candidate) {
                return ManagedWorktreeRoot {
                    target_root,
                    root_source: source,
                    ephemeral_enabled: true,
                    fallback_reason: None,
                    default_line: None,
                    seed_snapshot_id: None,
                    seed_snapshot_total_bytes: None,
                    main_seed_ram_max_bytes: None,
                };
            }
        }
    }

    if memory_roots_allowed && ops.platform() == TaskWorktreePlatform::Windows {
        for candidate in windows_ephemeral_root_candidates(repo, ops) {
            if let Some(target_root) = ops.ensure_root_candidate(&candidate) {
                return ManagedWorktreeRoot {
                    target_root,
                    root_source: WINDOWS_RAMDISK_KIND.to_string(),
                    ephemeral_enabled: true,
                    fallback_reason: None,
                    default_line: None,
                    seed_snapshot_id: None,
                    seed_snapshot_total_bytes: None,
                    main_seed_ram_max_bytes: None,
                };
            }
        }
    }

    if memory_roots_allowed && ops.platform() == TaskWorktreePlatform::Macos {
        for spec in collect_macos_resolution_specs(
            repo,
            ops,
            effective_ephemeral_root.as_deref(),
            normalized_memory_root.as_ref(),
        ) {
            if !ops.ensure_memory_root_available(&spec) {
                continue;
            }
            let candidate =
                auto_detected_ephemeral_root(repo, &spec.root).join(repo_path_segment(repo));
            if let Some(target_root) = ops.ensure_root_candidate(&candidate) {
                return ManagedWorktreeRoot {
                    target_root,
                    root_source: MACOS_RAM_VOLUME_KIND.to_string(),
                    ephemeral_enabled: true,
                    fallback_reason: None,
                    default_line: None,
                    seed_snapshot_id: None,
                    seed_snapshot_total_bytes: None,
                    main_seed_ram_max_bytes: None,
                };
            }
        }
    }

    ManagedWorktreeRoot {
        target_root: default_root,
        root_source: "repo_internal_fallback".to_string(),
        ephemeral_enabled: false,
        fallback_reason: budget_status
            .as_ref()
            .filter(|status| status.exceeded)
            .map(|_| "main_seed_ram_budget_exceeded".to_string()),
        default_line: budget_status
            .as_ref()
            .map(|status| status.default_line.clone()),
        seed_snapshot_id: budget_status
            .as_ref()
            .map(|status| status.seed_snapshot_id.clone()),
        seed_snapshot_total_bytes: budget_status
            .as_ref()
            .map(|status| status.seed_snapshot_total_bytes),
        main_seed_ram_max_bytes: budget_status
            .as_ref()
            .map(|status| status.main_seed_ram_max_bytes),
    }
}

fn resolve_task_worktree_alias_base(repo: &RepoRuntime) -> PathBuf {
    let raw = task_worktree_config_value(repo, "alias_root")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)));
    let base = raw
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_TASK_WORKTREE_ALIAS_ROOT));
    let repo_root = repo.authoritative_repo_root();
    let resolved = if base.is_absolute() {
        base
    } else {
        repo_root.join(base)
    };
    resolve_path_strict_false(&resolved)
}

fn effective_task_worktree_ephemeral_root_base(
    repo: &RepoRuntime,
    memory_root: Option<&TaskWorktreeMemoryRoot>,
) -> Option<PathBuf> {
    if let Some(raw) = task_worktree_config_value(repo, "ephemeral_root")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
    {
        return Some(resolve_configured_path(
            &repo.authoritative_repo_root(),
            &raw,
        ));
    }
    memory_root.map(|spec| auto_detected_ephemeral_root(repo, &spec.root))
}

fn ensure_configured_ephemeral_root(
    repo: &RepoRuntime,
    ops: &impl TaskWorktreeOps,
    effective_ephemeral_root_base: Option<&Path>,
    memory_root: Option<&TaskWorktreeMemoryRoot>,
    allow_memory_roots: bool,
) -> Option<PathBuf> {
    let configured_root = configured_repository_worktree_root(repo, effective_ephemeral_root_base?);
    let effective_memory_root = memory_root
        .cloned()
        .or_else(|| infer_task_worktree_memory_root(repo, ops, effective_ephemeral_root_base));
    if let Some(spec) = effective_memory_root {
        if !allow_memory_roots {
            return None;
        }
        if !path_is_relative_to(&configured_root, &spec.root) {
            return if memory_root.is_some() {
                None
            } else {
                ops.ensure_root_candidate(&configured_root)
            };
        }
        if !ops.ensure_memory_root_available(&spec) {
            return None;
        }
    } else if looks_like_missing_macos_auto_detected_root(&configured_root) {
        return None;
    }
    ops.ensure_root_candidate(&configured_root)
}

fn infer_task_worktree_memory_root(
    repo: &RepoRuntime,
    ops: &impl TaskWorktreeOps,
    effective_ephemeral_root_base: Option<&Path>,
) -> Option<TaskWorktreeMemoryRoot> {
    let configured_root = effective_ephemeral_root_base?.join(repo_path_segment(repo));
    match ops.platform() {
        TaskWorktreePlatform::Linux => {
            for root in ops.linux_detected_memory_roots() {
                if path_is_relative_to(&configured_root, &root) {
                    return Some(TaskWorktreeMemoryRoot {
                        kind: TaskWorktreeMemoryRootKind::LinuxMemoryRoot,
                        root,
                        volume_name: None,
                        sector_count: None,
                    });
                }
            }
            None
        }
        TaskWorktreePlatform::Windows => {
            for root in ops.windows_ramdisk_roots() {
                if path_is_relative_to(&configured_root, &root) {
                    return Some(TaskWorktreeMemoryRoot {
                        kind: TaskWorktreeMemoryRootKind::WindowsRamdisk,
                        root,
                        volume_name: None,
                        sector_count: None,
                    });
                }
            }
            None
        }
        TaskWorktreePlatform::Macos => {
            for spec in ops.macos_ram_volume_specs() {
                if path_is_relative_to(&configured_root, &spec.root) {
                    return Some(spec);
                }
            }
            infer_macos_auto_detected_memory_root(&configured_root)
        }
        TaskWorktreePlatform::Other => None,
    }
}

fn looks_like_missing_macos_auto_detected_root(path: &Path) -> bool {
    let parts = path.components().collect::<Vec<_>>();
    if parts.len() < 4 {
        return false;
    }
    if parts[0] != Component::RootDir
        || component_text(parts[1]) != Some("Volumes")
        || component_text(parts[3]) != Some(AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME)
    {
        return false;
    }
    let volume_root = Path::new("/Volumes").join(component_text(parts[2]).unwrap_or_default());
    if volume_root.exists() {
        return false;
    }
    parts.len() >= 5
}

fn collect_macos_resolution_specs(
    repo: &RepoRuntime,
    ops: &impl TaskWorktreeOps,
    effective_ephemeral_root_base: Option<&Path>,
    memory_root: Option<&TaskWorktreeMemoryRoot>,
) -> Vec<TaskWorktreeMemoryRoot> {
    let mut specs = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |spec: Option<TaskWorktreeMemoryRoot>| {
        let Some(spec) = spec else {
            return;
        };
        if spec.kind != TaskWorktreeMemoryRootKind::MacosRamVolume {
            return;
        }
        let key = spec.root.to_string_lossy().to_string();
        if seen.insert(key) {
            specs.push(spec);
        }
    };
    add(memory_root.cloned());
    add(infer_task_worktree_memory_root(
        repo,
        ops,
        effective_ephemeral_root_base,
    ));
    for spec in ops.macos_ram_volume_specs() {
        add(Some(spec));
    }
    add(Some(ops.macos_default_ram_volume_spec()));
    specs
}

fn linux_ephemeral_root_candidates(
    repo: &RepoRuntime,
    ops: &impl TaskWorktreeOps,
) -> Vec<(PathBuf, String)> {
    let xdg_runtime_root = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .and_then(|value| normalized_text(Some(&value)))
        .map(|value| resolve_path_strict_false(&expanduser_path(&value)));
    let dev_shm = resolve_path_strict_false(Path::new("/dev/shm"));
    let tmp_root = resolve_path_strict_false(Path::new("/tmp"));
    ops.linux_detected_memory_roots()
        .into_iter()
        .map(|root| {
            let source = if xdg_runtime_root
                .as_ref()
                .map(|candidate| *candidate == root)
                .unwrap_or(false)
            {
                "linux_xdg_runtime_dir".to_string()
            } else if root == dev_shm {
                "linux_dev_shm".to_string()
            } else if root == tmp_root {
                "linux_tmpfs".to_string()
            } else {
                LINUX_MEMORY_ROOT_KIND.to_string()
            };
            (
                auto_detected_ephemeral_root(repo, &root).join(repo_path_segment(repo)),
                source,
            )
        })
        .collect()
}

fn windows_ephemeral_root_candidates(
    repo: &RepoRuntime,
    ops: &impl TaskWorktreeOps,
) -> Vec<PathBuf> {
    ops.windows_ramdisk_roots()
        .into_iter()
        .map(|root| auto_detected_ephemeral_root(repo, &root).join(repo_path_segment(repo)))
        .collect()
}

pub(crate) fn main_seed_ram_budget_status(repo: &RepoRuntime) -> Option<MainSeedRamBudgetStatus> {
    let workspace_root = repo.workspace_root();
    let store = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)
        .ok()?;
    main_seed_ram_budget_status_with_snapshot_store(repo, &store)
}

fn main_seed_ram_budget_status_with_snapshot_store<S>(
    repo: &RepoRuntime,
    snapshot_store: &S,
) -> Option<MainSeedRamBudgetStatus>
where
    S: SnapshotStore + ?Sized,
{
    let budget_bytes = normalize_task_worktree_main_seed_ram_max_bytes(
        task_worktree_config_value(repo, "main_seed_ram_max_bytes"),
    )?;
    let effective_line_name = repo.default_line_name();
    let seed_snapshot_id = read_line_ref(&repo.authoritative_repo_root(), &effective_line_name)?;
    let seed_snapshot_total_bytes = snapshot_store
        .snapshot_total_bytes(&seed_snapshot_id)
        .ok()??;
    Some(MainSeedRamBudgetStatus {
        default_line: effective_line_name,
        seed_snapshot_id,
        seed_snapshot_total_bytes,
        main_seed_ram_max_bytes: budget_bytes,
        exceeded: seed_snapshot_total_bytes > budget_bytes,
    })
}

fn task_worktree_config_value<'a>(repo: &'a RepoRuntime, key: &str) -> Option<&'a JsonValue> {
    repo.config
        .get("task_worktree")
        .and_then(JsonValue::as_object)
        .and_then(|raw| raw.get(key))
}

fn normalize_task_worktree_memory_root(value: &JsonValue) -> Option<TaskWorktreeMemoryRoot> {
    let payload = value.as_object()?;
    let kind =
        TaskWorktreeMemoryRootKind::from_text(payload.get("kind").and_then(JsonValue::as_str))?;
    let root = payload
        .get("root")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))?;
    let volume_name = payload
        .get("volume_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)));
    let sector_count = payload
        .get("sector_count")
        .and_then(json_i64_like)
        .filter(|value| *value > 0);
    match kind {
        TaskWorktreeMemoryRootKind::MacosRamVolume => Some(TaskWorktreeMemoryRoot {
            kind,
            root: resolve_path_strict_false(&expanduser_path(&root)),
            volume_name,
            sector_count,
        }),
        TaskWorktreeMemoryRootKind::LinuxMemoryRoot
        | TaskWorktreeMemoryRootKind::WindowsRamdisk => Some(TaskWorktreeMemoryRoot {
            kind,
            root: resolve_path_strict_false(&expanduser_path(&root)),
            volume_name: None,
            sector_count: None,
        }),
    }
}

fn normalize_main_seed_ram_max_bytes_value(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => number.as_i64().filter(|candidate| *candidate >= 0),
        JsonValue::String(text) => normalized_text(Some(text))
            .and_then(|candidate| candidate.parse::<i64>().ok())
            .filter(|candidate| *candidate >= 0),
        _ => None,
    }
}

fn normalize_task_worktree_main_seed_ram_max_bytes(value: Option<&JsonValue>) -> Option<i64> {
    let raw = value?;
    if raw.is_boolean() || raw.is_null() {
        return None;
    }
    json_i64_like(raw).filter(|value| *value >= 0)
}

fn path_list_field(value: Option<&JsonValue>) -> Result<Vec<PathBuf>, String> {
    let Some(JsonValue::Array(values)) = value else {
        return Ok(Vec::new());
    };
    values
        .iter()
        .map(|value| {
            let text = value
                .as_str()
                .and_then(|raw| normalized_text(Some(raw)))
                .ok_or_else(|| {
                    "task-worktree debug probe paths must be non-empty strings.".to_string()
                })?;
            Ok(resolve_path_strict_false(&expanduser_path(&text)))
        })
        .collect()
}

fn memory_root_list_field(
    value: Option<&JsonValue>,
) -> Result<Vec<TaskWorktreeMemoryRoot>, String> {
    let Some(JsonValue::Array(values)) = value else {
        return Ok(Vec::new());
    };
    values
        .iter()
        .map(|value| {
            normalize_task_worktree_memory_root(value).ok_or_else(|| {
                "task-worktree debug probe memory_root entries must decode to valid specs."
                    .to_string()
            })
        })
        .collect()
}

fn json_i64_like(value: &JsonValue) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    let text = value.as_str().and_then(|raw| normalized_text(Some(raw)))?;
    text.parse::<i64>().ok()
}

fn repo_path_segment(repo: &RepoRuntime) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in repo.repo_name().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let normalized = out.trim_matches('-').to_string();
    if normalized.is_empty() {
        "repo".to_string()
    } else {
        normalized
    }
}

fn configured_repository_worktree_root(repo: &RepoRuntime, base: &Path) -> PathBuf {
    base.join(configured_repository_scope_segment(repo))
        .join(repo_path_segment(repo))
}

fn configured_repository_scope_segment(repo: &RepoRuntime) -> String {
    let root_hash = authoritative_repo_root_hash12(repo);
    match repo.repository_index() {
        Some(repository_index) => format!("r{}-{root_hash}", repository_index.get()),
        None => format!("h{root_hash}"),
    }
}

fn encode_ref_name(name: &str) -> String {
    let mut out = String::new();
    for byte in name.as_bytes() {
        let ch = *byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

fn read_line_ref(repo_root: &Path, line_name: &str) -> Option<String> {
    let path = repo_root
        .join(".ait")
        .join("refs")
        .join("lines")
        .join(encode_ref_name(line_name));
    let content = fs::read_to_string(path).ok()?;
    normalized_text(Some(content.trim()))
}

pub(crate) fn auto_detected_ephemeral_root(repo: &RepoRuntime, root: &Path) -> PathBuf {
    resolve_path_strict_false(root)
        .join(AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME)
        .join(authoritative_repo_root_hash12(repo))
}

fn authoritative_repo_root_hash12(repo: &RepoRuntime) -> String {
    sha256_hex_bytes(repo.authoritative_repo_root().to_string_lossy().as_bytes())[..12].to_string()
}

fn sha256_hex_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    format!("{digest:x}")
}

fn resolve_configured_path(repo_root: &Path, value: &str) -> PathBuf {
    let base = expanduser_path(value);
    let resolved = if base.is_absolute() {
        base
    } else {
        repo_root.join(base)
    };
    resolve_path_strict_false(&resolved)
}

fn normalized_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn component_text(component: Component<'_>) -> Option<&str> {
    match component {
        Component::Normal(value) => value.to_str(),
        _ => None,
    }
}

fn path_is_relative_to(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).is_ok()
}

fn expanduser_path(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn resolve_path_strict_false(path: &Path) -> PathBuf {
    let normalized = lexical_normalize(path);
    if let Ok(canonical) = normalized.canonicalize() {
        return canonical;
    }
    let mut cursor = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        if cursor.exists() {
            if let Ok(canonical_parent) = cursor.canonicalize() {
                let mut resolved = canonical_parent;
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return lexical_normalize(&resolved);
            }
        }
        let Some(file_name) = cursor.file_name() else {
            return normalized;
        };
        missing.push(file_name.to_os_string());
        let Some(parent) = cursor.parent() else {
            return normalized;
        };
        cursor = parent;
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => output.push(prefix.as_os_str()),
            Component::RootDir => output.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            Component::Normal(part) => output.push(part),
        }
    }
    output
}

fn ensure_root_candidate_system(path: &Path) -> Option<PathBuf> {
    fs::create_dir_all(path).ok()?;
    Some(resolve_path_strict_false(path))
}

fn ensure_memory_root_available_system(spec: &TaskWorktreeMemoryRoot) -> bool {
    match spec.kind {
        TaskWorktreeMemoryRootKind::LinuxMemoryRoot => {
            linux_detected_memory_roots().contains(&spec.root)
        }
        TaskWorktreeMemoryRootKind::WindowsRamdisk => windows_ramdisk_roots().contains(&spec.root),
        TaskWorktreeMemoryRootKind::MacosRamVolume => {
            if macos_ram_volume_specs()
                .iter()
                .any(|candidate| candidate.root == spec.root)
            {
                return true;
            }
            provision_macos_ram_volume(spec)
        }
    }
}

fn decode_mountinfo_path(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let digits = [chars.next(), chars.next(), chars.next()];
        if digits.iter().all(Option::is_some) {
            let raw = digits.into_iter().flatten().collect::<String>();
            if raw
                .chars()
                .all(|digit| digit.is_ascii_digit() && digit < '8')
            {
                if let Ok(value) = u8::from_str_radix(&raw, 8) {
                    out.push(value as char);
                    continue;
                }
            }
            out.push('\\');
            out.push_str(&raw);
            continue;
        }
        out.push('\\');
        for digit in digits.into_iter().flatten() {
            out.push(digit);
        }
    }
    out
}

fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let current = path;
    for candidate in current.ancestors() {
        if candidate.exists() {
            return Some(
                candidate
                    .canonicalize()
                    .unwrap_or_else(|_| resolve_path_strict_false(candidate)),
            );
        }
    }
    None
}

fn linux_mount_fstype_for_path(path: &Path) -> Option<String> {
    let anchor = nearest_existing_ancestor(path)?;
    let anchor_text = anchor.to_string_lossy();
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").ok()?;
    let mut best_mount_point = None::<String>;
    let mut best_fstype = None::<String>;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let left_fields = left.split_whitespace().collect::<Vec<_>>();
        let right_fields = right.split_whitespace().collect::<Vec<_>>();
        if left_fields.len() < 5 || right_fields.is_empty() {
            continue;
        }
        let mount_point = decode_mountinfo_path(left_fields[4]);
        if anchor_text.as_ref() != mount_point
            && !anchor_text.starts_with(&format!("{}/", mount_point.trim_end_matches('/')))
        {
            continue;
        }
        if best_mount_point
            .as_ref()
            .map(|current| mount_point.len() > current.len())
            .unwrap_or(true)
        {
            best_mount_point = Some(mount_point);
            best_fstype = normalized_text(Some(right_fields[0]));
        }
    }
    best_fstype
}

fn linux_detected_memory_roots() -> Vec<PathBuf> {
    if TaskWorktreePlatform::current() != TaskWorktreePlatform::Linux {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    if let Some(runtime_dir) = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .and_then(|value| normalized_text(Some(&value)))
    {
        candidates.push(expanduser_path(&runtime_dir));
    }
    candidates.push(PathBuf::from("/dev/shm"));
    candidates.push(PathBuf::from("/tmp"));

    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();
    for candidate in candidates {
        let Some(fstype) = linux_mount_fstype_for_path(&candidate) else {
            continue;
        };
        if !LINUX_MEMORY_BACKED_FSTYPES.contains(&fstype.as_str()) {
            continue;
        }
        let resolved = resolve_path_strict_false(&candidate);
        let key = resolved.to_string_lossy().to_string();
        if seen.insert(key) {
            roots.push(resolved);
        }
    }
    roots
}

fn windows_ramdisk_roots() -> Vec<PathBuf> {
    if TaskWorktreePlatform::current() != TaskWorktreePlatform::Windows {
        return Vec::new();
    }
    let mut raw_candidates = Vec::new();
    for env_name in ["LOCALAPPDATA", "TEMP", "TMP"] {
        if let Some(value) = std::env::var(env_name)
            .ok()
            .and_then(|raw| normalized_text(Some(&raw)))
        {
            if let Some(root) = windows_drive_root(&expanduser_path(&value)) {
                raw_candidates.push(root);
            }
        }
    }
    if let Some(root) = windows_drive_root(&std::env::temp_dir()) {
        raw_candidates.push(root);
    }
    raw_candidates.extend(windows_list_drive_roots());

    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();
    for root in raw_candidates {
        if windows_get_drive_type(&root) != Some(WINDOWS_DRIVE_RAMDISK) {
            continue;
        }
        let resolved = resolve_path_strict_false(&root);
        let key = resolved.to_string_lossy().to_string();
        if seen.insert(key) {
            roots.push(resolved);
        }
    }
    roots
}

fn windows_drive_root(path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use std::path::Prefix;
        for component in path.components() {
            if let Component::Prefix(prefix_component) = component {
                if let Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) = prefix_component.kind()
                {
                    return Some(PathBuf::from(format!("{}:\\", letter as char)));
                }
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let text = path.to_string_lossy();
        let bytes = text.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic() {
            return Some(PathBuf::from(format!("{}:\\", bytes[0] as char)));
        }
        None
    }
}

#[cfg(windows)]
fn windows_list_drive_roots() -> Vec<PathBuf> {
    let mask = windows_get_logical_drives();
    let mut roots = Vec::new();
    for index in 0..26 {
        if mask & (1 << index) == 0 {
            continue;
        }
        roots.push(PathBuf::from(format!(
            "{}:\\",
            (b'A' + index as u8) as char
        )));
    }
    roots
}

#[cfg(not(windows))]
fn windows_list_drive_roots() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn windows_get_drive_type(root: &Path) -> Option<u32> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide = root.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.push(0);
    Some(windows_get_drive_type_wide(&wide))
}

#[cfg(not(windows))]
fn windows_get_drive_type(_root: &Path) -> Option<u32> {
    None
}

#[cfg(windows)]
fn windows_get_logical_drives() -> u32 {
    unsafe { GetLogicalDrives() }
}

#[cfg(windows)]
fn windows_get_drive_type_wide(root: &[u16]) -> u32 {
    unsafe { GetDriveTypeW(root.as_ptr()) }
}

#[cfg(windows)]
#[link(name = "Kernel32")]
extern "system" {
    fn GetDriveTypeW(root_path_name: *const u16) -> u32;
    fn GetLogicalDrives() -> u32;
}

fn macos_ram_volume_specs() -> Vec<TaskWorktreeMemoryRoot> {
    if TaskWorktreePlatform::current() != TaskWorktreePlatform::Macos {
        return Vec::new();
    }
    let raw = Command::new("hdiutil")
        .args(["info", "-plist"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
        .unwrap_or_default();
    if raw.is_empty() {
        return Vec::new();
    }
    let payload = PlistValue::from_reader_xml(raw.as_slice()).ok();
    let Some(PlistValue::Dictionary(dict)) = payload else {
        return Vec::new();
    };
    let Some(PlistValue::Array(images)) = dict.get("images") else {
        return Vec::new();
    };
    macos_ram_volume_specs_from_plist(images)
}

fn macos_ram_volume_specs_from_plist(images: &[PlistValue]) -> Vec<TaskWorktreeMemoryRoot> {
    let mut specs = Vec::new();
    let mut seen = BTreeSet::new();
    for image in images {
        let PlistValue::Dictionary(image_dict) = image else {
            continue;
        };
        let Some(image_path) = image_dict
            .get("image-path")
            .and_then(plist_string)
            .and_then(|value| normalized_text(Some(&value)))
        else {
            continue;
        };
        let Some(sector_count_text) = image_path.strip_prefix("ram://") else {
            continue;
        };
        if image_dict.get("writeable").and_then(plist_bool) != Some(true) {
            continue;
        }
        let sector_count = sector_count_text.trim().parse::<i64>().ok();
        let Some(PlistValue::Array(entities)) = image_dict.get("system-entities") else {
            continue;
        };
        for entity in entities {
            let PlistValue::Dictionary(entity_dict) = entity else {
                continue;
            };
            let Some(mount_point) = entity_dict
                .get("mount-point")
                .and_then(plist_string)
                .and_then(|value| normalized_text(Some(&value)))
            else {
                continue;
            };
            let root = resolve_path_strict_false(&expanduser_path(&mount_point));
            let key = root.to_string_lossy().to_string();
            if !seen.insert(key) {
                continue;
            }
            specs.push(TaskWorktreeMemoryRoot {
                kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
                volume_name: root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .and_then(|value| normalized_text(Some(value))),
                sector_count: sector_count.filter(|value| *value > 0),
                root,
            });
        }
    }
    specs.sort_by(|left, right| left.root.cmp(&right.root));
    specs
}

fn plist_string(value: &PlistValue) -> Option<String> {
    match value {
        PlistValue::String(text) => Some(text.clone()),
        _ => None,
    }
}

fn plist_bool(value: &PlistValue) -> Option<bool> {
    match value {
        PlistValue::Boolean(flag) => Some(*flag),
        _ => None,
    }
}

fn default_macos_ram_volume_spec() -> TaskWorktreeMemoryRoot {
    TaskWorktreeMemoryRoot {
        kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
        root: resolve_path_strict_false(
            Path::new("/Volumes")
                .join(DEFAULT_MACOS_RAM_VOLUME_NAME)
                .as_path(),
        ),
        volume_name: Some(DEFAULT_MACOS_RAM_VOLUME_NAME.to_string()),
        sector_count: Some(DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT),
    }
}

fn infer_macos_auto_detected_memory_root(path: &Path) -> Option<TaskWorktreeMemoryRoot> {
    let parts = path.components().collect::<Vec<_>>();
    if parts.len() < 5
        || parts[0] != Component::RootDir
        || component_text(parts[1]) != Some("Volumes")
        || component_text(parts[3]) != Some(AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME)
    {
        return None;
    }
    let volume_name = normalized_text(component_text(parts[2]))?;
    let root = resolve_path_strict_false(&Path::new("/Volumes").join(&volume_name));
    Some(TaskWorktreeMemoryRoot {
        kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
        root,
        volume_name: Some(volume_name),
        sector_count: Some(DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT),
    })
}

fn parse_macos_attached_device(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|token| token.starts_with("/dev/"))
        .map(|value| value.to_string())
}

fn provision_macos_ram_volume(spec: &TaskWorktreeMemoryRoot) -> bool {
    if spec.kind != TaskWorktreeMemoryRootKind::MacosRamVolume {
        return false;
    }
    let Some(volume_name) = spec.volume_name.as_ref() else {
        return false;
    };
    let Some(sector_count) = spec.sector_count.filter(|value| *value > 0) else {
        return false;
    };
    if macos_ram_volume_specs()
        .iter()
        .any(|candidate| candidate.root == spec.root)
    {
        return true;
    }
    if spec.root.exists() {
        return false;
    }
    let attach_output = Command::new("hdiutil")
        .args(["attach", "-nomount", &format!("ram://{sector_count}")])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());
    let Some(attached_device) = attach_output
        .as_deref()
        .and_then(parse_macos_attached_device)
    else {
        return false;
    };
    let erase_status = Command::new("diskutil")
        .args(["erasevolume", "APFS", volume_name, &attached_device])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if erase_status
        .as_ref()
        .map(|status| !status.success())
        .unwrap_or(true)
    {
        let _ = Command::new("hdiutil")
            .args(["detach", &attached_device, "-force"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return false;
    }
    if macos_ram_volume_specs()
        .iter()
        .any(|candidate| candidate.root == spec.root)
    {
        return true;
    }
    let _ = Command::new("hdiutil")
        .args(["detach", &attached_device, "-force"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    false
}

#[cfg(test)]
mod tests;
