use super::*;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

const CONTRACT_VERSION: &str = "memory-root-v2";
const RAM_SECTOR_BYTES: u64 = 512;
const DEFAULT_MIN_AVAILABLE_BYTES: u64 = 0;
const MEMORY_ROOT_CONFIG_PATH: &str = "task_worktree.memory_root";
const MEMORY_ROOT_PATH_CONFIG_PATH: &str = "task_worktree.memory_root.root";
const MEMORY_ROOT_VOLUME_CONFIG_PATH: &str = "task_worktree.memory_root.volume_name";
const MEMORY_ROOT_CAPACITY_CONFIG_PATH: &str = "task_worktree.memory_root.sector_count";
const TASK_RUNTIME_ROOT_LABEL: &str = "derived Task runtime root";

#[derive(Clone, Debug, PartialEq, Eq)]
struct MemoryRootConfig {
    platform: TaskWorktreePlatform,
    mount_point: PathBuf,
    mount_point_source: String,
    volume_name: Option<String>,
    volume_name_source: String,
    requested_capacity_bytes: u64,
    capacity_source: String,
    minimum_available_bytes: u64,
    minimum_available_source: String,
    runtime_root: PathBuf,
    runtime_root_source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MacosVolumeInfo {
    volume_name: String,
    mount_point: PathBuf,
    filesystem_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ValidatedMemoryRoot {
    mount_point: PathBuf,
    runtime_root: PathBuf,
    platform_proof: String,
    actual_image_sector_count: Option<u64>,
    actual_image_capacity_bytes: Option<u64>,
    filesystem_total_bytes: u64,
    available_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ValidationFailure {
    Missing(String),
    Invalid(String),
}

impl ValidationFailure {
    fn message(self) -> String {
        match self {
            Self::Missing(message) | Self::Invalid(message) => message,
        }
    }
}

trait MemoryRootOps {
    fn platform(&self) -> TaskWorktreePlatform;
    fn linux_detected_memory_roots(&self) -> Vec<PathBuf>;
    fn windows_ramdisk_roots(&self) -> Vec<PathBuf>;
    fn macos_ram_volume_specs(&self) -> Vec<TaskWorktreeMemoryRoot>;
    fn macos_volume_info(&self, root: &Path) -> Result<MacosVolumeInfo, String>;
    fn linux_mount_fstype(&self, root: &Path) -> Option<String>;
    fn windows_drive_type(&self, root: &Path) -> Option<u32>;
    fn path_exists(&self, path: &Path) -> bool;
    fn path_is_dir(&self, path: &Path) -> bool;
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, String>;
    fn filesystem_space(&self, path: &Path) -> Result<(u64, u64), String>;
    fn writable_probe(&self, path: &Path) -> Result<(), String>;
}

struct SystemMemoryRootOps;

impl MemoryRootOps for SystemMemoryRootOps {
    fn platform(&self) -> TaskWorktreePlatform {
        TaskWorktreePlatform::current()
    }

    fn linux_detected_memory_roots(&self) -> Vec<PathBuf> {
        super::linux_detected_memory_roots()
    }

    fn windows_ramdisk_roots(&self) -> Vec<PathBuf> {
        super::windows_ramdisk_roots()
    }

    fn macos_ram_volume_specs(&self) -> Vec<TaskWorktreeMemoryRoot> {
        super::macos_ram_volume_specs()
    }

    fn macos_volume_info(&self, root: &Path) -> Result<MacosVolumeInfo, String> {
        macos_volume_info_system(root)
    }

    fn linux_mount_fstype(&self, root: &Path) -> Option<String> {
        super::linux_mount_fstype_for_path(root)
    }

    fn windows_drive_type(&self, root: &Path) -> Option<u32> {
        super::windows_get_drive_type(root)
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn path_is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, String> {
        path.canonicalize()
            .map_err(|error| format!("Failed to canonicalize '{}': {error}", path.display()))
    }

    fn filesystem_space(&self, path: &Path) -> Result<(u64, u64), String> {
        let total = fs2::total_space(path).map_err(|error| {
            format!(
                "Failed to inspect total capacity for '{}': {error}",
                path.display()
            )
        })?;
        let available = fs2::available_space(path).map_err(|error| {
            format!(
                "Failed to inspect available capacity for '{}': {error}",
                path.display()
            )
        })?;
        Ok((total, available))
    }

    fn writable_probe(&self, path: &Path) -> Result<(), String> {
        writable_probe_system(path)
    }
}

pub(crate) fn doctor_memory_root_payload(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let ops = SystemMemoryRootOps;
    let config = MemoryRootConfig::from_repo(repo, &ops)?;
    inspect_memory_root_with_ops(&config, &ops)
}

impl MemoryRootConfig {
    fn from_repo(repo: &RepoRuntime, ops: &impl MemoryRootOps) -> Result<Self, String> {
        let platform = ops.platform();
        let configured = match task_worktree_config_value(repo, "memory_root") {
            Some(value) => {
                let raw_root = value
                    .get("root")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| {
                        format!("{MEMORY_ROOT_PATH_CONFIG_PATH} must be a non-empty string.")
                    })?;
                let _ = parse_clean_absolute_path(MEMORY_ROOT_PATH_CONFIG_PATH, raw_root)?;
                Some(normalize_task_worktree_memory_root(value).ok_or_else(|| {
                    format!("{MEMORY_ROOT_CONFIG_PATH} is malformed or unsupported.")
                })?)
            }
            None => None,
        };
        let (spec, mount_point_source, detected_macos_metadata) = match configured {
            Some(spec) => {
                let detected = (platform == TaskWorktreePlatform::Macos)
                    .then(|| {
                        ops.macos_ram_volume_specs()
                            .into_iter()
                            .find(|candidate| candidate.root == spec.root)
                    })
                    .flatten();
                match detected {
                    Some(detected) => (detected, "repo_config".to_string(), true),
                    None => (spec, "repo_config".to_string(), false),
                }
            }
            None => {
                let (spec, source) = detected_memory_root_spec(platform, ops)?;
                let detected = source == "detected_macos_ram_volume";
                (spec, source, detected)
            }
        };
        let configured_ephemeral_root =
            task_worktree_config_value(repo, "ephemeral_root").is_some();
        let runtime_base = effective_task_worktree_ephemeral_root_base(repo, Some(&spec))
            .ok_or_else(|| {
                "Could not derive the Task runtime root from repository configuration.".to_string()
            })?;
        let (runtime_root, runtime_root_source) = if configured_ephemeral_root {
            (
                configured_repository_worktree_root(repo, &runtime_base),
                "repo_config.task_worktree.ephemeral_root".to_string(),
            )
        } else {
            (
                runtime_base.join(repo_path_segment(repo)),
                "derived_from_task_worktree.memory_root".to_string(),
            )
        };
        let mut config = Self::from_resolved_spec(
            platform,
            spec,
            mount_point_source,
            runtime_root,
            runtime_root_source,
        )?;
        if detected_macos_metadata {
            config.capacity_source = "detected_macos_ram_volume".to_string();
            config.volume_name_source = "detected_macos_ram_volume".to_string();
        }
        Ok(config)
    }

    fn from_resolved_spec(
        platform: TaskWorktreePlatform,
        spec: TaskWorktreeMemoryRoot,
        mount_point_source: String,
        runtime_root: PathBuf,
        runtime_root_source: String,
    ) -> Result<Self, String> {
        validate_memory_root_kind(platform, &spec.kind)?;
        let mount_point =
            parse_clean_absolute_path(MEMORY_ROOT_PATH_CONFIG_PATH, &spec.root.to_string_lossy())?;
        let (volume_name, volume_name_source) = if platform == TaskWorktreePlatform::Macos {
            let (value, source) = match spec.volume_name {
                Some(value) => (value, format!("{MEMORY_ROOT_CONFIG_PATH}.volume_name")),
                None => (
                    mount_point
                        .file_name()
                        .and_then(|value| value.to_str())
                        .ok_or_else(|| {
                            format!(
                                "Could not derive a macOS volume label from '{}'.",
                                mount_point.display()
                            )
                        })?
                        .to_string(),
                    "mount_point_basename".to_string(),
                ),
            };
            validate_volume_name(&value)?;
            validate_macos_mount_contract(&mount_point, &value)?;
            (Some(value), source)
        } else {
            (None, "not_applicable".to_string())
        };
        let (requested_capacity_bytes, capacity_source) = if platform == TaskWorktreePlatform::Macos
        {
            let (sector_count, source) = match spec.sector_count {
                Some(value) if value > 0 => (value, MEMORY_ROOT_CAPACITY_CONFIG_PATH.to_string()),
                Some(_) => {
                    return Err(format!(
                        "{MEMORY_ROOT_CAPACITY_CONFIG_PATH} must be a positive integer."
                    ));
                }
                None => (
                    DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT,
                    "built_in_default".to_string(),
                ),
            };
            let capacity = u64::try_from(sector_count)
                .ok()
                .and_then(|value| value.checked_mul(RAM_SECTOR_BYTES))
                .ok_or_else(|| {
                    format!("{MEMORY_ROOT_CAPACITY_CONFIG_PATH} overflows byte capacity.")
                })?;
            let _ = capacity_sector_count(capacity)?;
            (capacity, source)
        } else {
            (0, "not_applicable".to_string())
        };
        validate_strict_descendant(TASK_RUNTIME_ROOT_LABEL, &runtime_root, &mount_point)?;

        Ok(Self {
            platform,
            mount_point,
            mount_point_source,
            volume_name,
            volume_name_source,
            requested_capacity_bytes,
            capacity_source,
            minimum_available_bytes: DEFAULT_MIN_AVAILABLE_BYTES,
            minimum_available_source: "built_in".to_string(),
            runtime_root,
            runtime_root_source,
        })
    }
}

fn detected_memory_root_spec(
    platform: TaskWorktreePlatform,
    ops: &impl MemoryRootOps,
) -> Result<(TaskWorktreeMemoryRoot, String), String> {
    match platform {
        TaskWorktreePlatform::Macos => Ok(ops
            .macos_ram_volume_specs()
            .into_iter()
            .next()
            .map(|spec| (spec, "detected_macos_ram_volume".to_string()))
            .unwrap_or_else(|| {
                (
                    TaskWorktreeMemoryRoot {
                        kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
                        root: PathBuf::from("/Volumes").join(DEFAULT_MACOS_RAM_VOLUME_NAME),
                        volume_name: Some(DEFAULT_MACOS_RAM_VOLUME_NAME.to_string()),
                        sector_count: Some(DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT),
                    },
                    "platform_default".to_string(),
                )
            })),
        TaskWorktreePlatform::Linux => ops
            .linux_detected_memory_roots()
            .into_iter()
            .next()
            .map(|root| {
                (
                    TaskWorktreeMemoryRoot {
                        kind: TaskWorktreeMemoryRootKind::LinuxMemoryRoot,
                        root,
                        volume_name: None,
                        sector_count: None,
                    },
                    "detected_linux_memory_root".to_string(),
                )
            })
            .ok_or_else(|| {
                "No verified Linux tmpfs/ramfs root was detected; mount one and rerun `ait init` to record it."
                    .to_string()
            }),
        TaskWorktreePlatform::Windows => ops
            .windows_ramdisk_roots()
            .into_iter()
            .next()
            .map(|root| {
                (
                    TaskWorktreeMemoryRoot {
                        kind: TaskWorktreeMemoryRootKind::WindowsRamdisk,
                        root,
                        volume_name: None,
                        sector_count: None,
                    },
                    "detected_windows_ramdisk".to_string(),
                )
            })
            .ok_or_else(|| {
                "No Windows DRIVE_RAMDISK root was detected; provision one and rerun `ait init` to record it."
                    .to_string()
            }),
        TaskWorktreePlatform::Other => {
            Err("RAM-root lifecycle is unsupported on this operating system.".to_string())
        }
    }
}

fn validate_memory_root_kind(
    platform: TaskWorktreePlatform,
    kind: &TaskWorktreeMemoryRootKind,
) -> Result<(), String> {
    let matches_platform = matches!(
        (platform, kind),
        (
            TaskWorktreePlatform::Macos,
            TaskWorktreeMemoryRootKind::MacosRamVolume
        ) | (
            TaskWorktreePlatform::Linux,
            TaskWorktreeMemoryRootKind::LinuxMemoryRoot
        ) | (
            TaskWorktreePlatform::Windows,
            TaskWorktreeMemoryRootKind::WindowsRamdisk
        )
    );
    if matches_platform {
        Ok(())
    } else {
        Err(format!(
            "{MEMORY_ROOT_CONFIG_PATH} kind does not match the current platform."
        ))
    }
}

fn inspect_memory_root_with_ops(
    config: &MemoryRootConfig,
    ops: &impl MemoryRootOps,
) -> Result<JsonValue, String> {
    let validated = validate_memory_root(config, ops).map_err(ValidationFailure::message)?;
    Ok(success_payload(config, &validated))
}

fn validate_memory_root(
    config: &MemoryRootConfig,
    ops: &impl MemoryRootOps,
) -> Result<ValidatedMemoryRoot, ValidationFailure> {
    let (platform_proof, actual_image_sector_count, actual_image_capacity_bytes) =
        match config.platform {
            TaskWorktreePlatform::Macos => validate_macos_platform(config, ops)?,
            TaskWorktreePlatform::Linux => {
                validate_existing_directory(config, ops)?;
                let fstype = ops.linux_mount_fstype(&config.mount_point).ok_or_else(|| {
                    ValidationFailure::Invalid(format!(
                        "Could not prove a Linux filesystem type for '{}'.",
                        config.mount_point.display()
                    ))
                })?;
                if !LINUX_MEMORY_BACKED_FSTYPES.contains(&fstype.as_str()) {
                    return Err(ValidationFailure::Invalid(format!(
                        "Linux RAM root '{}' is mounted as '{fstype}', not tmpfs or ramfs.",
                        config.mount_point.display()
                    )));
                }
                (format!("linux_mountinfo:{fstype}"), None, None)
            }
            TaskWorktreePlatform::Windows => {
                validate_existing_directory(config, ops)?;
                if ops.windows_drive_type(&config.mount_point) != Some(WINDOWS_DRIVE_RAMDISK) {
                    return Err(ValidationFailure::Invalid(format!(
                        "Windows RAM root '{}' is not a DRIVE_RAMDISK volume.",
                        config.mount_point.display()
                    )));
                }
                ("windows_drive_type:DRIVE_RAMDISK".to_string(), None, None)
            }
            TaskWorktreePlatform::Other => {
                return Err(ValidationFailure::Invalid(
                    "RAM-root validation is unsupported on this operating system.".to_string(),
                ));
            }
        };

    let canonical_mount = ops
        .canonicalize(&config.mount_point)
        .map_err(ValidationFailure::Invalid)?;
    if canonical_mount != config.mount_point {
        return Err(ValidationFailure::Invalid(format!(
            "RAM mount point '{}' resolves through a symlink or alias to '{}'; an exact mount point is required.",
            config.mount_point.display(),
            canonical_mount.display()
        )));
    }

    let (filesystem_total_bytes, available_bytes) = ops
        .filesystem_space(&canonical_mount)
        .map_err(ValidationFailure::Invalid)?;
    if filesystem_total_bytes < config.requested_capacity_bytes {
        return Err(ValidationFailure::Invalid(format!(
            "RAM root '{}' has {filesystem_total_bytes} total bytes, below the configured capacity {}.",
            canonical_mount.display(),
            config.requested_capacity_bytes
        )));
    }
    if available_bytes < config.minimum_available_bytes {
        return Err(ValidationFailure::Invalid(format!(
            "RAM root '{}' has {available_bytes} available bytes, below the required minimum {}.",
            canonical_mount.display(),
            config.minimum_available_bytes
        )));
    }

    validate_runtime_ancestor(config, ops, &canonical_mount)?;
    let runtime_root = if ops.path_exists(&config.runtime_root) {
        let canonical_runtime = ops
            .canonicalize(&config.runtime_root)
            .map_err(ValidationFailure::Invalid)?;
        if canonical_runtime == canonical_mount || !canonical_runtime.starts_with(&canonical_mount)
        {
            return Err(ValidationFailure::Invalid(format!(
                "The {TASK_RUNTIME_ROOT_LABEL} escapes the validated RAM mount after canonicalization: {}",
                canonical_runtime.display()
            )));
        }
        if !ops.path_is_dir(&canonical_runtime) {
            return Err(ValidationFailure::Invalid(format!(
                "The {TASK_RUNTIME_ROOT_LABEL} is not a directory: {}",
                canonical_runtime.display()
            )));
        }
        canonical_runtime
    } else {
        config.runtime_root.clone()
    };

    ops.writable_probe(&canonical_mount)
        .map_err(ValidationFailure::Invalid)?;

    Ok(ValidatedMemoryRoot {
        mount_point: canonical_mount,
        runtime_root,
        platform_proof,
        actual_image_sector_count,
        actual_image_capacity_bytes,
        filesystem_total_bytes,
        available_bytes,
    })
}

fn validate_macos_platform(
    config: &MemoryRootConfig,
    ops: &impl MemoryRootOps,
) -> Result<(String, Option<u64>, Option<u64>), ValidationFailure> {
    let exact = ops
        .macos_ram_volume_specs()
        .into_iter()
        .find(|candidate| candidate.root == config.mount_point);
    let Some(spec) = exact else {
        if ops.path_exists(&config.mount_point) {
            return Err(ValidationFailure::Invalid(format!(
                "Existing path '{}' is not proved by hdiutil as an explicitly writable ram:// image.",
                config.mount_point.display()
            )));
        }
        return Err(ValidationFailure::Missing(format!(
            "macOS RAM volume '{}' is not mounted at '{}'.",
            config
                .volume_name
                .as_deref()
                .unwrap_or(DEFAULT_MACOS_RAM_VOLUME_NAME),
            config.mount_point.display()
        )));
    };
    validate_existing_directory(config, ops)?;

    let actual_sector_count = spec
        .sector_count
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| {
            ValidationFailure::Invalid(format!(
                "hdiutil did not report a valid ram:// sector count for '{}'.",
                config.mount_point.display()
            ))
        })?;
    let required_sector_count = u64::try_from(
        capacity_sector_count(config.requested_capacity_bytes)
            .map_err(ValidationFailure::Invalid)?,
    )
    .map_err(|_| {
        ValidationFailure::Invalid(
            "Requested RAM sector count could not be represented as u64.".to_string(),
        )
    })?;
    if actual_sector_count < required_sector_count {
        return Err(ValidationFailure::Invalid(format!(
            "Mounted ram:// image at '{}' has {actual_sector_count} sectors, below the requested {required_sector_count}.",
            config.mount_point.display()
        )));
    }
    let actual_capacity = actual_sector_count
        .checked_mul(RAM_SECTOR_BYTES)
        .ok_or_else(|| {
            ValidationFailure::Invalid("Mounted ram:// image capacity overflowed u64.".to_string())
        })?;

    let info = ops
        .macos_volume_info(&config.mount_point)
        .map_err(ValidationFailure::Invalid)?;
    if info.mount_point != config.mount_point {
        return Err(ValidationFailure::Invalid(format!(
            "diskutil reports mount point '{}' instead of the requested '{}'.",
            info.mount_point.display(),
            config.mount_point.display()
        )));
    }
    let expected_volume = config.volume_name.as_deref().unwrap_or_default();
    if info.volume_name != expected_volume {
        return Err(ValidationFailure::Invalid(format!(
            "diskutil reports volume label '{}' instead of {MEMORY_ROOT_VOLUME_CONFIG_PATH}='{expected_volume}'.",
            info.volume_name
        )));
    }
    if !info.filesystem_type.eq_ignore_ascii_case("apfs") {
        return Err(ValidationFailure::Invalid(format!(
            "diskutil reports filesystem '{}' for '{}'; APFS is required for managed macOS RAM volumes.",
            info.filesystem_type,
            config.mount_point.display()
        )));
    }
    Ok((
        "macos_hdiutil:writable_ram_image+diskutil:apfs".to_string(),
        Some(actual_sector_count),
        Some(actual_capacity),
    ))
}

fn validate_existing_directory(
    config: &MemoryRootConfig,
    ops: &impl MemoryRootOps,
) -> Result<(), ValidationFailure> {
    if !ops.path_exists(&config.mount_point) {
        return Err(ValidationFailure::Missing(format!(
            "RAM mount point '{}' does not exist.",
            config.mount_point.display()
        )));
    }
    if !ops.path_is_dir(&config.mount_point) {
        return Err(ValidationFailure::Invalid(format!(
            "RAM mount point '{}' is not a directory.",
            config.mount_point.display()
        )));
    }
    Ok(())
}

fn validate_runtime_ancestor(
    config: &MemoryRootConfig,
    ops: &impl MemoryRootOps,
    canonical_mount: &Path,
) -> Result<(), ValidationFailure> {
    let existing_ancestor = config
        .runtime_root
        .ancestors()
        .find(|candidate| ops.path_exists(candidate))
        .ok_or_else(|| {
            ValidationFailure::Invalid(format!(
                "No existing ancestor could be found for the {TASK_RUNTIME_ROOT_LABEL} '{}'.",
                config.runtime_root.display()
            ))
        })?;
    let canonical_ancestor = ops
        .canonicalize(existing_ancestor)
        .map_err(ValidationFailure::Invalid)?;
    if canonical_ancestor != canonical_mount && !canonical_ancestor.starts_with(canonical_mount) {
        return Err(ValidationFailure::Invalid(format!(
            "The {TASK_RUNTIME_ROOT_LABEL} escapes the validated RAM mount through existing ancestor '{}'.",
            canonical_ancestor.display()
        )));
    }
    Ok(())
}

fn success_payload(config: &MemoryRootConfig, validated: &ValidatedMemoryRoot) -> JsonValue {
    json!({
        "contract": CONTRACT_VERSION,
        "state": "pass",
        "platform": platform_name(config.platform),
        "mount_point": validated.mount_point.to_string_lossy().to_string(),
        "mount_point_source": config.mount_point_source,
        "runtime_root": validated.runtime_root.to_string_lossy().to_string(),
        "runtime_root_source": config.runtime_root_source,
        "volume_name": config.volume_name,
        "volume_name_source": config.volume_name_source,
        "requested_capacity_bytes": config.requested_capacity_bytes,
        "capacity_source": config.capacity_source,
        "actual_image_sector_count": validated.actual_image_sector_count,
        "actual_image_capacity_bytes": validated.actual_image_capacity_bytes,
        "filesystem_total_bytes": validated.filesystem_total_bytes,
        "available_bytes": validated.available_bytes,
        "minimum_available_bytes": config.minimum_available_bytes,
        "minimum_available_source": config.minimum_available_source,
        "platform_proof": validated.platform_proof,
    })
}

fn macos_volume_info_system(root: &Path) -> Result<MacosVolumeInfo, String> {
    let output = Command::new("diskutil")
        .arg("info")
        .arg("-plist")
        .arg(root)
        .stderr(Stdio::null())
        .output()
        .map_err(|error| {
            format!(
                "Failed to invoke diskutil for RAM volume '{}': {error}",
                root.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "diskutil could not inspect RAM volume '{}'.",
            root.display()
        ));
    }
    let payload = PlistValue::from_reader_xml(output.stdout.as_slice()).map_err(|error| {
        format!(
            "diskutil returned invalid plist data for '{}': {error}",
            root.display()
        )
    })?;
    macos_volume_info_from_plist(&payload)
}

fn macos_volume_info_from_plist(payload: &PlistValue) -> Result<MacosVolumeInfo, String> {
    let PlistValue::Dictionary(dict) = payload else {
        return Err("diskutil plist root must be a dictionary.".to_string());
    };
    let required = |key: &str| -> Result<String, String> {
        dict.get(key)
            .and_then(plist_string)
            .and_then(|value| normalized_text(Some(&value)))
            .ok_or_else(|| format!("diskutil plist is missing required field '{key}'."))
    };
    Ok(MacosVolumeInfo {
        volume_name: required("VolumeName")?,
        mount_point: PathBuf::from(required("MountPoint")?),
        filesystem_type: required("FilesystemType")?,
    })
}

fn writable_probe_system(root: &Path) -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let path = root.join(format!(
        ".ait-memory-root-probe-{}-{nonce}",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("RAM root '{}' is not writable: {error}", root.display()))?;
    if let Err(error) = file.write_all(b"ait-memory-root-v2\n") {
        let _ = fs::remove_file(&path);
        return Err(format!(
            "Failed to write RAM-root probe '{}': {error}",
            path.display()
        ));
    }
    if let Err(error) = file.sync_data() {
        let _ = fs::remove_file(&path);
        return Err(format!(
            "Failed to sync RAM-root probe '{}': {error}",
            path.display()
        ));
    }
    drop(file);
    fs::remove_file(&path).map_err(|error| {
        format!(
            "Failed to remove RAM-root probe '{}': {error}",
            path.display()
        )
    })
}

fn parse_clean_absolute_path(name: &str, raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(format!("{name} must be an absolute path; got '{raw}'."));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(format!(
            "{name} must not contain '.' or '..' traversal components; got '{raw}'."
        ));
    }
    Ok(path)
}

fn validate_strict_descendant(name: &str, path: &Path, root: &Path) -> Result<(), String> {
    if path == root || !path.starts_with(root) {
        return Err(format!(
            "{name} must be a strict descendant of '{}'; got '{}'.",
            root.display(),
            path.display()
        ));
    }
    Ok(())
}

fn validate_volume_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || Path::new(value).components().count() != 1
    {
        return Err(format!(
            "{MEMORY_ROOT_VOLUME_CONFIG_PATH} must be one non-traversing path component; got '{value}'."
        ));
    }
    Ok(())
}

fn validate_macos_mount_contract(mount_point: &Path, volume_name: &str) -> Result<(), String> {
    let expected = Path::new("/Volumes").join(volume_name);
    if mount_point != expected {
        return Err(format!(
            "On macOS, {MEMORY_ROOT_PATH_CONFIG_PATH} must exactly match /Volumes/{MEMORY_ROOT_VOLUME_CONFIG_PATH}; expected '{}', got '{}'.",
            expected.display(),
            mount_point.display()
        ));
    }
    Ok(())
}

fn capacity_sector_count(capacity_bytes: u64) -> Result<i64, String> {
    let sectors = capacity_bytes
        .checked_add(RAM_SECTOR_BYTES - 1)
        .ok_or_else(|| format!("{MEMORY_ROOT_CAPACITY_CONFIG_PATH} overflows sector rounding."))?
        / RAM_SECTOR_BYTES;
    i64::try_from(sectors).map_err(|_| {
        format!(
            "{MEMORY_ROOT_CAPACITY_CONFIG_PATH} is too large for hdiutil's ram:// sector count."
        )
    })
}

fn platform_name(platform: TaskWorktreePlatform) -> &'static str {
    match platform {
        TaskWorktreePlatform::Linux => "linux",
        TaskWorktreePlatform::Macos => "macos",
        TaskWorktreePlatform::Windows => "windows",
        TaskWorktreePlatform::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const TEST_RUNTIME_DIRNAME: &str = "task-runtime";

    struct FakeOps {
        platform: TaskWorktreePlatform,
        root: PathBuf,
        runtime_root: PathBuf,
        mounted: Cell<bool>,
        existing_non_ram: bool,
        runtime_exists: Cell<bool>,
        image_sector_count: i64,
        volume_name: String,
        volume_mount_point: PathBuf,
        filesystem_type: String,
        filesystem_total: u64,
        filesystem_available: u64,
        canonical_mount: PathBuf,
        canonical_runtime: PathBuf,
        writable_error: Option<String>,
        linux_fstype: Option<String>,
        windows_drive_type: Option<u32>,
    }

    impl FakeOps {
        fn macos(mounted: bool) -> Self {
            let root = PathBuf::from("/Volumes/AIT_RAM");
            let runtime_root = root.join(TEST_RUNTIME_DIRNAME);
            Self {
                platform: TaskWorktreePlatform::Macos,
                root: root.clone(),
                runtime_root: runtime_root.clone(),
                mounted: Cell::new(mounted),
                existing_non_ram: false,
                runtime_exists: Cell::new(mounted),
                image_sector_count: DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT,
                volume_name: DEFAULT_MACOS_RAM_VOLUME_NAME.to_string(),
                volume_mount_point: root.clone(),
                filesystem_type: "apfs".to_string(),
                filesystem_total: 8 * 1024 * 1024 * 1024,
                filesystem_available: 4 * 1024 * 1024 * 1024,
                canonical_mount: root,
                canonical_runtime: runtime_root,
                writable_error: None,
                linux_fstype: None,
                windows_drive_type: None,
            }
        }

        fn rebase(&mut self, platform: TaskWorktreePlatform, root: PathBuf) {
            let runtime_root = root.join(TEST_RUNTIME_DIRNAME);
            self.platform = platform;
            self.root = root.clone();
            self.runtime_root = runtime_root.clone();
            self.volume_mount_point = root.clone();
            self.canonical_mount = root;
            self.canonical_runtime = runtime_root;
        }
    }

    impl MemoryRootOps for FakeOps {
        fn platform(&self) -> TaskWorktreePlatform {
            self.platform
        }

        fn linux_detected_memory_roots(&self) -> Vec<PathBuf> {
            if self.platform == TaskWorktreePlatform::Linux {
                vec![self.root.clone()]
            } else {
                Vec::new()
            }
        }

        fn windows_ramdisk_roots(&self) -> Vec<PathBuf> {
            if self.platform == TaskWorktreePlatform::Windows {
                vec![self.root.clone()]
            } else {
                Vec::new()
            }
        }

        fn macos_ram_volume_specs(&self) -> Vec<TaskWorktreeMemoryRoot> {
            if !self.mounted.get() {
                return Vec::new();
            }
            vec![TaskWorktreeMemoryRoot {
                kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
                root: self.root.clone(),
                volume_name: Some(self.volume_name.clone()),
                sector_count: Some(self.image_sector_count),
            }]
        }

        fn macos_volume_info(&self, _root: &Path) -> Result<MacosVolumeInfo, String> {
            Ok(MacosVolumeInfo {
                volume_name: self.volume_name.clone(),
                mount_point: self.volume_mount_point.clone(),
                filesystem_type: self.filesystem_type.clone(),
            })
        }

        fn linux_mount_fstype(&self, _root: &Path) -> Option<String> {
            self.linux_fstype.clone()
        }

        fn windows_drive_type(&self, _root: &Path) -> Option<u32> {
            self.windows_drive_type
        }

        fn path_exists(&self, path: &Path) -> bool {
            if path == self.root {
                self.mounted.get() || self.existing_non_ram
            } else if path == self.runtime_root {
                self.runtime_exists.get()
            } else {
                path == Path::new("/Volumes") || path == Path::new("/")
            }
        }

        fn path_is_dir(&self, path: &Path) -> bool {
            self.path_exists(path)
        }

        fn canonicalize(&self, path: &Path) -> Result<PathBuf, String> {
            if path == self.root {
                Ok(self.canonical_mount.clone())
            } else if path == self.runtime_root {
                Ok(self.canonical_runtime.clone())
            } else {
                Ok(path.to_path_buf())
            }
        }

        fn filesystem_space(&self, _path: &Path) -> Result<(u64, u64), String> {
            Ok((self.filesystem_total, self.filesystem_available))
        }

        fn writable_probe(&self, _path: &Path) -> Result<(), String> {
            self.writable_error.clone().map_or(Ok(()), Err)
        }
    }

    fn macos_config(ops: &FakeOps) -> MemoryRootConfig {
        MemoryRootConfig::from_resolved_spec(
            TaskWorktreePlatform::Macos,
            TaskWorktreeMemoryRoot {
                kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
                root: ops.root.clone(),
                volume_name: Some(ops.volume_name.clone()),
                sector_count: Some(ops.image_sector_count),
            },
            "test_memory_root".to_string(),
            ops.runtime_root.clone(),
            "test_runtime_root".to_string(),
        )
        .unwrap()
    }

    fn config_for_existing_platform_root(ops: &FakeOps) -> MemoryRootConfig {
        MemoryRootConfig {
            platform: ops.platform,
            mount_point: ops.root.clone(),
            mount_point_source: "test_memory_root".to_string(),
            volume_name: None,
            volume_name_source: "not_applicable".to_string(),
            requested_capacity_bytes: 0,
            capacity_source: "platform_default".to_string(),
            minimum_available_bytes: 0,
            minimum_available_source: "built_in".to_string(),
            runtime_root: ops.runtime_root.clone(),
            runtime_root_source: "test_runtime_root".to_string(),
        }
    }

    #[test]
    fn typed_memory_root_config_and_derived_runtime_are_bounded() {
        let config = MemoryRootConfig::from_resolved_spec(
            TaskWorktreePlatform::Macos,
            TaskWorktreeMemoryRoot {
                kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
                root: PathBuf::from("/Volumes/FAST_RAM"),
                volume_name: Some("FAST_RAM".to_string()),
                sector_count: Some(8_388_608),
            },
            "repo_config".to_string(),
            PathBuf::from("/Volumes/FAST_RAM/.ait-repos/demo"),
            "derived_from_task_worktree.memory_root".to_string(),
        )
        .unwrap();

        assert_eq!(config.mount_point, PathBuf::from("/Volumes/FAST_RAM"));
        assert_eq!(config.mount_point_source, "repo_config");
        assert_eq!(config.volume_name.as_deref(), Some("FAST_RAM"));
        assert_eq!(config.requested_capacity_bytes, 4_294_967_296);
        assert_eq!(config.minimum_available_bytes, 0);
        assert_eq!(
            config.runtime_root,
            PathBuf::from("/Volumes/FAST_RAM/.ait-repos/demo")
        );
    }

    #[test]
    fn malformed_typed_values_and_overflow_fail_closed() {
        for sector_count in [0, -1] {
            let error = MemoryRootConfig::from_resolved_spec(
                TaskWorktreePlatform::Macos,
                TaskWorktreeMemoryRoot {
                    kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
                    root: PathBuf::from("/Volumes/AIT_RAM"),
                    volume_name: Some("AIT_RAM".to_string()),
                    sector_count: Some(sector_count),
                },
                "repo_config".to_string(),
                PathBuf::from("/Volumes/AIT_RAM/task-runtime"),
                "derived".to_string(),
            )
            .unwrap_err();
            assert!(error.contains("positive integer"), "{error}");
        }
        let overflow = MemoryRootConfig::from_resolved_spec(
            TaskWorktreePlatform::Macos,
            TaskWorktreeMemoryRoot {
                kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
                root: PathBuf::from("/Volumes/AIT_RAM"),
                volume_name: Some("AIT_RAM".to_string()),
                sector_count: Some(i64::MAX),
            },
            "repo_config".to_string(),
            PathBuf::from("/Volumes/AIT_RAM/task-runtime"),
            "derived".to_string(),
        )
        .unwrap_err();
        assert!(overflow.contains("overflows byte capacity"), "{overflow}");
    }

    #[test]
    fn mount_label_and_runtime_boundaries_fail_closed() {
        let mismatch = MemoryRootConfig::from_resolved_spec(
            TaskWorktreePlatform::Macos,
            TaskWorktreeMemoryRoot {
                kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
                root: PathBuf::from("/Volumes/AIT_RAM"),
                volume_name: Some("OTHER".to_string()),
                sector_count: Some(DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT),
            },
            "repo_config".to_string(),
            PathBuf::from("/Volumes/AIT_RAM/task-runtime"),
            "derived".to_string(),
        )
        .unwrap_err();
        assert!(mismatch.contains("must exactly match"), "{mismatch}");

        let runtime_escape = MemoryRootConfig::from_resolved_spec(
            TaskWorktreePlatform::Macos,
            TaskWorktreeMemoryRoot {
                kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
                root: PathBuf::from("/Volumes/AIT_RAM"),
                volume_name: Some("AIT_RAM".to_string()),
                sector_count: Some(DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT),
            },
            "repo_config".to_string(),
            PathBuf::from("/tmp/task-runtime"),
            "repo_config.task_worktree.ephemeral_root".to_string(),
        )
        .unwrap_err();
        assert!(
            runtime_escape.contains("strict descendant"),
            "{runtime_escape}"
        );
    }

    #[test]
    fn mounted_root_is_inspected_without_provisioning_fields() {
        let ops = FakeOps::macos(true);
        let payload = inspect_memory_root_with_ops(&macos_config(&ops), &ops).unwrap();

        assert_eq!(payload["contract"], "memory-root-v2");
        assert_eq!(payload["state"], "pass");
        assert!(payload.get("auto_mount_allowed").is_none());
        assert!(payload.get("auto_mounted").is_none());
        assert!(payload.get("mount_lock").is_none());
    }

    #[test]
    fn missing_root_remains_missing_without_creating_runtime_state() {
        let ops = FakeOps::macos(false);
        let error = inspect_memory_root_with_ops(&macos_config(&ops), &ops).unwrap_err();

        assert!(error.contains("is not mounted"));
        assert!(!ops.mounted.get());
        assert!(!ops.runtime_exists.get());
    }

    #[test]
    fn existing_non_ram_path_is_never_reformatted() {
        let mut ops = FakeOps::macos(false);
        ops.existing_non_ram = true;
        let error = inspect_memory_root_with_ops(&macos_config(&ops), &ops).unwrap_err();

        assert!(error.contains("Existing path"));
        assert!(!ops.mounted.get());
    }

    #[test]
    fn wrong_mount_capacity_free_space_and_symlink_escape_are_rejected() {
        let mut wrong_mount = FakeOps::macos(true);
        wrong_mount.volume_mount_point = PathBuf::from("/Volumes/OTHER");
        assert!(
            inspect_memory_root_with_ops(&macos_config(&wrong_mount), &wrong_mount)
                .unwrap_err()
                .contains("diskutil reports mount point")
        );

        let mut undersized = FakeOps::macos(true);
        let config = macos_config(&undersized);
        undersized.image_sector_count = DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT - 1;
        assert!(inspect_memory_root_with_ops(&config, &undersized)
            .unwrap_err()
            .contains("below the requested"));

        let mut low_free = FakeOps::macos(true);
        low_free.filesystem_available = 1;
        let mut config = macos_config(&low_free);
        config.minimum_available_bytes = 2;
        assert!(inspect_memory_root_with_ops(&config, &low_free)
            .unwrap_err()
            .contains("available bytes"));

        let mut escaped = FakeOps::macos(true);
        escaped.canonical_runtime = PathBuf::from("/tmp/escaped");
        assert!(
            inspect_memory_root_with_ops(&macos_config(&escaped), &escaped)
                .unwrap_err()
                .contains("escapes")
        );
    }

    #[test]
    fn linux_requires_tmpfs_or_ramfs_without_persistent_fallback() {
        let mut ops = FakeOps::macos(true);
        ops.rebase(TaskWorktreePlatform::Linux, PathBuf::from("/dev/shm"));
        ops.linux_fstype = Some("apfs".to_string());
        let config = config_for_existing_platform_root(&ops);
        assert!(inspect_memory_root_with_ops(&config, &ops)
            .unwrap_err()
            .contains("not tmpfs or ramfs"));

        ops.linux_fstype = Some("tmpfs".to_string());
        let payload = inspect_memory_root_with_ops(&config, &ops).unwrap();
        assert_eq!(payload["platform_proof"], "linux_mountinfo:tmpfs");
    }

    #[test]
    fn windows_requires_drive_ramdisk_without_persistent_fallback() {
        let mut ops = FakeOps::macos(true);
        ops.rebase(TaskWorktreePlatform::Windows, PathBuf::from("R:\\"));
        ops.windows_drive_type = Some(3);
        let config = config_for_existing_platform_root(&ops);
        assert!(inspect_memory_root_with_ops(&config, &ops)
            .unwrap_err()
            .contains("not a DRIVE_RAMDISK"));

        ops.windows_drive_type = Some(WINDOWS_DRIVE_RAMDISK);
        let payload = inspect_memory_root_with_ops(&config, &ops).unwrap();
        assert_eq!(
            payload["platform_proof"],
            "windows_drive_type:DRIVE_RAMDISK"
        );
    }

    #[test]
    fn diskutil_plist_parser_requires_all_authority_fields() {
        let payload = PlistValue::Dictionary(plist::Dictionary::from_iter([
            (
                "VolumeName".to_string(),
                PlistValue::String("AIT_RAM".to_string()),
            ),
            (
                "MountPoint".to_string(),
                PlistValue::String("/Volumes/AIT_RAM".to_string()),
            ),
            (
                "FilesystemType".to_string(),
                PlistValue::String("apfs".to_string()),
            ),
        ]));
        let info = macos_volume_info_from_plist(&payload).unwrap();
        assert_eq!(info.volume_name, "AIT_RAM");

        let missing = PlistValue::Dictionary(plist::Dictionary::new());
        assert!(macos_volume_info_from_plist(&missing)
            .unwrap_err()
            .contains("VolumeName"));
    }
}
