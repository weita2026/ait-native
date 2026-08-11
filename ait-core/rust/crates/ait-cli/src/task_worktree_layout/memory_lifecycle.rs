use super::*;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

const CONTRACT_VERSION: &str = "memory-root-v1";
const RAM_SECTOR_BYTES: u64 = 512;
const DEFAULT_RUNTIME_DIRNAME: &str = "ait-runtime";
const DEFAULT_MIN_AVAILABLE_BYTES: u64 = 0;
const MOUNT_LOCK_BOUNDARY: &str = "missing_mount_recheck_attach_format_validate_cleanup";

const RAM_MOUNT_POINT_ENV: &str = "AIT_RAM_MOUNT_POINT";
const LEGACY_RAM_MOUNT_POINT_ENV: &str = "AIT_RAM";
const RAM_VOLUME_NAME_ENV: &str = "AIT_RAM_VOLUME_NAME";
const RAM_CAPACITY_BYTES_ENV: &str = "AIT_RAM_CAPACITY_BYTES";
const RAM_MIN_AVAILABLE_BYTES_ENV: &str = "AIT_RAM_MIN_AVAILABLE_BYTES";
const RUNTIME_RAM_ROOT_ENV: &str = "AIT_RUNTIME_RAM_ROOT";
const RAM_AUTO_MOUNT_ENV: &str = "AIT_RAM_AUTO_MOUNT";
const RAM_MOUNT_LOCK_PATH_ENV: &str = "AIT_RAM_MOUNT_LOCK_PATH";

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
    auto_mount_allowed: bool,
    auto_mount_source: String,
    mount_lock_path: PathBuf,
    mount_lock_source: String,
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

trait MemoryRootLock {}

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
    fn create_dir_all(&self, path: &Path) -> Result<(), String>;
    fn writable_probe(&self, path: &Path) -> Result<(), String>;
    fn acquire_mount_lock(&self, path: &Path) -> Result<Box<dyn MemoryRootLock>, String>;
    fn provision_macos(&self, spec: &TaskWorktreeMemoryRoot) -> Result<(), String>;
    fn detach_macos(&self, root: &Path) -> Result<(), String>;
}

struct SystemMemoryRootOps;

struct SystemMemoryRootLock(File);

impl MemoryRootLock for SystemMemoryRootLock {}

impl Drop for SystemMemoryRootLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

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

    fn create_dir_all(&self, path: &Path) -> Result<(), String> {
        fs::create_dir_all(path)
            .map_err(|error| format!("Failed to create '{}': {error}", path.display()))
    }

    fn writable_probe(&self, path: &Path) -> Result<(), String> {
        writable_probe_system(path)
    }

    fn acquire_mount_lock(&self, path: &Path) -> Result<Box<dyn MemoryRootLock>, String> {
        let parent = path.parent().ok_or_else(|| {
            format!(
                "{RAM_MOUNT_LOCK_PATH_ENV} must have a parent directory: {}",
                path.display()
            )
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create RAM mount lock directory '{}': {error}",
                parent.display()
            )
        })?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| {
                format!(
                    "Failed to open RAM mount lock '{}': {error}",
                    path.display()
                )
            })?;
        file.lock_exclusive().map_err(|error| {
            format!(
                "Failed to acquire RAM mount lock '{}': {error}",
                path.display()
            )
        })?;
        Ok(Box::new(SystemMemoryRootLock(file)))
    }

    fn provision_macos(&self, spec: &TaskWorktreeMemoryRoot) -> Result<(), String> {
        if super::provision_macos_ram_volume(spec) {
            Ok(())
        } else {
            Err(format!(
                "Failed to attach and format the requested macOS RAM volume at '{}'.",
                spec.root.display()
            ))
        }
    }

    fn detach_macos(&self, root: &Path) -> Result<(), String> {
        let status = Command::new("hdiutil")
            .arg("detach")
            .arg(root)
            .arg("-force")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| {
                format!(
                    "Failed to invoke hdiutil cleanup for '{}': {error}",
                    root.display()
                )
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "hdiutil could not detach newly provisioned invalid RAM volume '{}'.",
                root.display()
            ))
        }
    }
}

pub(crate) fn doctor_memory_root_payload(ensure: bool) -> Result<JsonValue, String> {
    let ops = SystemMemoryRootOps;
    let config = MemoryRootConfig::from_process_environment(ensure, &ops)?;
    ensure_memory_root_with_ops(&config, &ops)
}

impl MemoryRootConfig {
    fn from_process_environment(ensure: bool, ops: &impl MemoryRootOps) -> Result<Self, String> {
        Self::from_lookup(ensure, ops, |name| {
            let Some(value) = std::env::var_os(name) else {
                return Ok(None);
            };
            value.into_string().map(Some).map_err(|_| {
                format!("{name} must contain valid Unicode text so it can be validated.")
            })
        })
    }

    fn from_lookup<F>(ensure: bool, ops: &impl MemoryRootOps, lookup: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Result<Option<String>, String>,
    {
        let get = |name: &str| -> Result<Option<String>, String> {
            Ok(lookup(name)?.and_then(|value| {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }))
        };
        let platform = ops.platform();
        let (mount_point_text, mount_point_source) = if let Some(value) = get(RAM_MOUNT_POINT_ENV)?
        {
            (value, RAM_MOUNT_POINT_ENV.to_string())
        } else if let Some(value) = get(LEGACY_RAM_MOUNT_POINT_ENV)? {
            (value, LEGACY_RAM_MOUNT_POINT_ENV.to_string())
        } else {
            match platform {
                TaskWorktreePlatform::Macos => (
                    format!("/Volumes/{DEFAULT_MACOS_RAM_VOLUME_NAME}"),
                    "platform_default".to_string(),
                ),
                TaskWorktreePlatform::Linux => {
                    let root = ops
                            .linux_detected_memory_roots()
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                "No verified Linux tmpfs/ramfs root was detected; set AIT_RAM_MOUNT_POINT to an existing memory-backed mount."
                                    .to_string()
                            })?;
                    (
                        root.to_string_lossy().to_string(),
                        "detected_linux_memory_root".to_string(),
                    )
                }
                TaskWorktreePlatform::Windows => {
                    let root = ops
                            .windows_ramdisk_roots()
                            .into_iter()
                            .next()
                            .ok_or_else(|| {
                                "No Windows DRIVE_RAMDISK root was detected; set AIT_RAM_MOUNT_POINT to an existing RAM disk."
                                    .to_string()
                            })?;
                    (
                        root.to_string_lossy().to_string(),
                        "detected_windows_ramdisk".to_string(),
                    )
                }
                TaskWorktreePlatform::Other => {
                    return Err(
                        "RAM-root lifecycle is unsupported on this operating system.".to_string(),
                    );
                }
            }
        };
        let mount_point = parse_clean_absolute_path(RAM_MOUNT_POINT_ENV, &mount_point_text)?;

        let (volume_name, volume_name_source) = if platform == TaskWorktreePlatform::Macos {
            let (value, source) = if let Some(value) = get(RAM_VOLUME_NAME_ENV)? {
                (value, RAM_VOLUME_NAME_ENV.to_string())
            } else {
                (
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
                )
            };
            validate_volume_name(&value)?;
            validate_macos_mount_contract(&mount_point, &value)?;
            (Some(value), source)
        } else {
            (None, "not_applicable".to_string())
        };

        let default_capacity = if platform == TaskWorktreePlatform::Macos {
            u64::try_from(DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT)
                .ok()
                .and_then(|value| value.checked_mul(RAM_SECTOR_BYTES))
                .ok_or_else(|| "Built-in macOS RAM capacity overflowed u64.".to_string())?
        } else {
            0
        };
        let (requested_capacity_bytes, capacity_source) =
            if let Some(raw) = get(RAM_CAPACITY_BYTES_ENV)? {
                (
                    parse_byte_count(RAM_CAPACITY_BYTES_ENV, &raw, false)?,
                    RAM_CAPACITY_BYTES_ENV.to_string(),
                )
            } else {
                (default_capacity, "platform_default".to_string())
            };
        if platform == TaskWorktreePlatform::Macos {
            let _ = capacity_sector_count(requested_capacity_bytes)?;
        }

        let (minimum_available_bytes, minimum_available_source) =
            if let Some(raw) = get(RAM_MIN_AVAILABLE_BYTES_ENV)? {
                (
                    parse_byte_count(RAM_MIN_AVAILABLE_BYTES_ENV, &raw, true)?,
                    RAM_MIN_AVAILABLE_BYTES_ENV.to_string(),
                )
            } else {
                (DEFAULT_MIN_AVAILABLE_BYTES, "built_in".to_string())
            };

        let (runtime_root, runtime_root_source) = if let Some(raw) = get(RUNTIME_RAM_ROOT_ENV)? {
            (
                parse_clean_absolute_path(RUNTIME_RAM_ROOT_ENV, &raw)?,
                RUNTIME_RAM_ROOT_ENV.to_string(),
            )
        } else {
            (
                mount_point.join(DEFAULT_RUNTIME_DIRNAME),
                "mount_point/ait-runtime".to_string(),
            )
        };
        validate_strict_descendant(RUNTIME_RAM_ROOT_ENV, &runtime_root, &mount_point)?;

        let env_auto_mount = if let Some(raw) = get(RAM_AUTO_MOUNT_ENV)? {
            Some(parse_bool(RAM_AUTO_MOUNT_ENV, &raw)?)
        } else {
            None
        };
        let (auto_mount_allowed, auto_mount_source) = if ensure {
            (true, "cli_ensure".to_string())
        } else if let Some(value) = env_auto_mount {
            (value, RAM_AUTO_MOUNT_ENV.to_string())
        } else {
            (false, "built_in_false".to_string())
        };

        let (mount_lock_path, mount_lock_source) = if let Some(raw) = get(RAM_MOUNT_LOCK_PATH_ENV)?
        {
            (
                parse_clean_absolute_path(RAM_MOUNT_LOCK_PATH_ENV, &raw)?,
                RAM_MOUNT_LOCK_PATH_ENV.to_string(),
            )
        } else {
            let cache_root = if let Some(raw) = get("XDG_CACHE_HOME")? {
                parse_clean_absolute_path("XDG_CACHE_HOME", &raw)?
            } else {
                let home = match get("HOME")? {
                    Some(value) => Some(value),
                    None => get("USERPROFILE")?,
                }
                .ok_or_else(|| {
                        format!(
                            "{RAM_MOUNT_LOCK_PATH_ENV} is required when HOME, USERPROFILE, and XDG_CACHE_HOME are unavailable."
                        )
                    })?;
                parse_clean_absolute_path("HOME", &home)?.join(".cache")
            };
            (
                cache_root.join("ait/locks/ram-mount.lock"),
                "host_cache_default".to_string(),
            )
        };
        if mount_lock_path == mount_point || mount_lock_path.starts_with(&mount_point) {
            return Err(format!(
                "{RAM_MOUNT_LOCK_PATH_ENV} must remain outside the RAM mount so it exists before provisioning: {}",
                mount_lock_path.display()
            ));
        }

        Ok(Self {
            platform,
            mount_point,
            mount_point_source,
            volume_name,
            volume_name_source,
            requested_capacity_bytes,
            capacity_source,
            minimum_available_bytes,
            minimum_available_source,
            runtime_root,
            runtime_root_source,
            auto_mount_allowed,
            auto_mount_source,
            mount_lock_path,
            mount_lock_source,
        })
    }

    fn macos_spec(&self) -> Result<TaskWorktreeMemoryRoot, String> {
        let volume_name = self
            .volume_name
            .clone()
            .ok_or_else(|| "macOS RAM volume name is unavailable.".to_string())?;
        Ok(TaskWorktreeMemoryRoot {
            kind: TaskWorktreeMemoryRootKind::MacosRamVolume,
            root: self.mount_point.clone(),
            volume_name: Some(volume_name),
            sector_count: Some(capacity_sector_count(self.requested_capacity_bytes)?),
        })
    }
}

fn ensure_memory_root_with_ops(
    config: &MemoryRootConfig,
    ops: &impl MemoryRootOps,
) -> Result<JsonValue, String> {
    let create_runtime = config.auto_mount_allowed;
    match validate_memory_root(config, ops, create_runtime) {
        Ok(validated) => return Ok(success_payload(config, &validated, false, false)),
        Err(ValidationFailure::Invalid(message)) => return Err(message),
        Err(ValidationFailure::Missing(message)) if !config.auto_mount_allowed => {
            return Err(format!(
                "{message} Set {RAM_AUTO_MOUNT_ENV}=true or pass --ensure to provision a missing supported RAM root."
            ));
        }
        Err(ValidationFailure::Missing(_)) => {}
    }

    if config.platform != TaskWorktreePlatform::Macos {
        return Err(format!(
            "Automatic RAM-root provisioning is supported only on macOS; '{}' must already be a verified memory-backed mount on {}.",
            config.mount_point.display(),
            platform_name(config.platform)
        ));
    }

    let _mount_lock = ops.acquire_mount_lock(&config.mount_lock_path)?;
    match validate_memory_root(config, ops, true) {
        Ok(validated) => return Ok(success_payload(config, &validated, false, true)),
        Err(ValidationFailure::Invalid(message)) => return Err(message),
        Err(ValidationFailure::Missing(_)) => {}
    }

    let spec = config.macos_spec()?;
    if let Err(provision_error) = ops.provision_macos(&spec) {
        if let Ok(validated) = validate_memory_root(config, ops, true) {
            return Ok(success_payload(config, &validated, false, true));
        }
        return Err(provision_error);
    }
    match validate_memory_root(config, ops, true) {
        Ok(validated) => Ok(success_payload(config, &validated, true, true)),
        Err(error) => {
            let validation_message = error.message();
            match ops.detach_macos(&config.mount_point) {
                Ok(()) => Err(format!(
                    "Newly provisioned RAM volume failed validation and was detached: {validation_message}"
                )),
                Err(cleanup_error) => Err(format!(
                    "Newly provisioned RAM volume failed validation: {validation_message} Cleanup also failed: {cleanup_error}"
                )),
            }
        }
    }
}

fn validate_memory_root(
    config: &MemoryRootConfig,
    ops: &impl MemoryRootOps,
    create_runtime: bool,
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
            "RAM root '{}' has {filesystem_total_bytes} total bytes, below {RAM_CAPACITY_BYTES_ENV}={}.",
            canonical_mount.display(),
            config.requested_capacity_bytes
        )));
    }
    if available_bytes < config.minimum_available_bytes {
        return Err(ValidationFailure::Invalid(format!(
            "RAM root '{}' has {available_bytes} available bytes, below {RAM_MIN_AVAILABLE_BYTES_ENV}={}.",
            canonical_mount.display(),
            config.minimum_available_bytes
        )));
    }

    validate_runtime_ancestor(config, ops, &canonical_mount)?;
    if create_runtime {
        ops.create_dir_all(&config.runtime_root)
            .map_err(ValidationFailure::Invalid)?;
    }
    let runtime_root = if ops.path_exists(&config.runtime_root) {
        let canonical_runtime = ops
            .canonicalize(&config.runtime_root)
            .map_err(ValidationFailure::Invalid)?;
        if canonical_runtime == canonical_mount || !canonical_runtime.starts_with(&canonical_mount)
        {
            return Err(ValidationFailure::Invalid(format!(
                "{RUNTIME_RAM_ROOT_ENV} escapes the validated RAM mount after canonicalization: {}",
                canonical_runtime.display()
            )));
        }
        if !ops.path_is_dir(&canonical_runtime) {
            return Err(ValidationFailure::Invalid(format!(
                "{RUNTIME_RAM_ROOT_ENV} is not a directory: {}",
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
            "diskutil reports volume label '{}' instead of {RAM_VOLUME_NAME_ENV}='{expected_volume}'.",
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
                "No existing ancestor could be found for {RUNTIME_RAM_ROOT_ENV}='{}'.",
                config.runtime_root.display()
            ))
        })?;
    let canonical_ancestor = ops
        .canonicalize(existing_ancestor)
        .map_err(ValidationFailure::Invalid)?;
    if canonical_ancestor != canonical_mount && !canonical_ancestor.starts_with(canonical_mount) {
        return Err(ValidationFailure::Invalid(format!(
            "{RUNTIME_RAM_ROOT_ENV} escapes the validated RAM mount through existing ancestor '{}'.",
            canonical_ancestor.display()
        )));
    }
    Ok(())
}

fn success_payload(
    config: &MemoryRootConfig,
    validated: &ValidatedMemoryRoot,
    auto_mounted: bool,
    lock_acquired: bool,
) -> JsonValue {
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
        "auto_mount_allowed": config.auto_mount_allowed,
        "auto_mount_source": config.auto_mount_source,
        "auto_mounted": auto_mounted,
        "mount_lock": {
            "path": config.mount_lock_path.to_string_lossy().to_string(),
            "source": config.mount_lock_source,
            "acquired": lock_acquired,
            "boundary": MOUNT_LOCK_BOUNDARY,
        },
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
    if let Err(error) = file.write_all(b"ait-memory-root-v1\n") {
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
            "{RAM_VOLUME_NAME_ENV} must be one non-traversing path component; got '{value}'."
        ));
    }
    Ok(())
}

fn validate_macos_mount_contract(mount_point: &Path, volume_name: &str) -> Result<(), String> {
    let expected = Path::new("/Volumes").join(volume_name);
    if mount_point != expected {
        return Err(format!(
            "On macOS, {RAM_MOUNT_POINT_ENV} must exactly equal /Volumes/{RAM_VOLUME_NAME_ENV}; expected '{}', got '{}'.",
            expected.display(),
            mount_point.display()
        ));
    }
    Ok(())
}

fn parse_byte_count(name: &str, raw: &str, allow_zero: bool) -> Result<u64, String> {
    let value = raw.parse::<u64>().map_err(|_| {
        format!("{name} must be a base-10 non-negative integer byte count; got '{raw}'.")
    })?;
    if !allow_zero && value == 0 {
        return Err(format!("{name} must be greater than zero."));
    }
    Ok(value)
}

fn parse_bool(name: &str, raw: &str) -> Result<bool, String> {
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "{name} must be one of true, false, 1, 0, yes, no, on, or off; got '{raw}'."
        )),
    }
}

fn capacity_sector_count(capacity_bytes: u64) -> Result<i64, String> {
    let sectors = capacity_bytes
        .checked_add(RAM_SECTOR_BYTES - 1)
        .ok_or_else(|| format!("{RAM_CAPACITY_BYTES_ENV} overflows sector rounding."))?
        / RAM_SECTOR_BYTES;
    i64::try_from(sectors).map_err(|_| {
        format!("{RAM_CAPACITY_BYTES_ENV} is too large for hdiutil's ram:// sector count.")
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
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    struct NoopLock;
    impl MemoryRootLock for NoopLock {}

    struct FakeOps {
        platform: TaskWorktreePlatform,
        root: PathBuf,
        runtime_root: PathBuf,
        mounted: Cell<bool>,
        existing_non_ram: bool,
        runtime_exists: Cell<bool>,
        mount_on_specs_call: Option<usize>,
        specs_calls: Cell<usize>,
        lock_count: Cell<usize>,
        provision_count: Cell<usize>,
        detach_count: Cell<usize>,
        provision_error: Option<String>,
        mount_on_provision_error: bool,
        detach_error: Option<String>,
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
        events: RefCell<Vec<String>>,
    }

    impl FakeOps {
        fn macos(mounted: bool) -> Self {
            let root = PathBuf::from("/Volumes/AIT_RAM");
            let runtime_root = root.join(DEFAULT_RUNTIME_DIRNAME);
            Self {
                platform: TaskWorktreePlatform::Macos,
                root: root.clone(),
                runtime_root: runtime_root.clone(),
                mounted: Cell::new(mounted),
                existing_non_ram: false,
                runtime_exists: Cell::new(mounted),
                mount_on_specs_call: None,
                specs_calls: Cell::new(0),
                lock_count: Cell::new(0),
                provision_count: Cell::new(0),
                detach_count: Cell::new(0),
                provision_error: None,
                mount_on_provision_error: false,
                detach_error: None,
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
                events: RefCell::new(Vec::new()),
            }
        }

        fn rebase(&mut self, platform: TaskWorktreePlatform, root: PathBuf) {
            let runtime_root = root.join(DEFAULT_RUNTIME_DIRNAME);
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
            let call = self.specs_calls.get() + 1;
            self.specs_calls.set(call);
            if self.mount_on_specs_call == Some(call) {
                self.mounted.set(true);
                self.runtime_exists.set(true);
            }
            self.events.borrow_mut().push(format!("specs:{call}"));
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

        fn create_dir_all(&self, path: &Path) -> Result<(), String> {
            if path == self.runtime_root {
                self.runtime_exists.set(true);
            }
            Ok(())
        }

        fn writable_probe(&self, _path: &Path) -> Result<(), String> {
            self.writable_error.clone().map_or(Ok(()), Err)
        }

        fn acquire_mount_lock(&self, _path: &Path) -> Result<Box<dyn MemoryRootLock>, String> {
            self.lock_count.set(self.lock_count.get() + 1);
            self.events.borrow_mut().push("lock".to_string());
            Ok(Box::new(NoopLock))
        }

        fn provision_macos(&self, _spec: &TaskWorktreeMemoryRoot) -> Result<(), String> {
            self.provision_count.set(self.provision_count.get() + 1);
            self.events.borrow_mut().push("provision".to_string());
            if let Some(error) = &self.provision_error {
                if self.mount_on_provision_error {
                    self.mounted.set(true);
                    self.runtime_exists.set(true);
                }
                return Err(error.clone());
            }
            self.mounted.set(true);
            Ok(())
        }

        fn detach_macos(&self, _root: &Path) -> Result<(), String> {
            self.detach_count.set(self.detach_count.get() + 1);
            self.events.borrow_mut().push("detach".to_string());
            self.detach_error.clone().map_or(Ok(()), Err)
        }
    }

    fn lookup_from<'a>(
        values: &'a BTreeMap<&'a str, &'a str>,
    ) -> impl Fn(&str) -> Result<Option<String>, String> + 'a {
        move |name| Ok(values.get(name).map(|value| (*value).to_string()))
    }

    fn macos_config(ensure: bool, ops: &FakeOps) -> MemoryRootConfig {
        let values = BTreeMap::from([("HOME", "/Users/test")]);
        MemoryRootConfig::from_lookup(ensure, ops, lookup_from(&values)).unwrap()
    }

    fn config_for_existing_platform_root(ops: &FakeOps) -> MemoryRootConfig {
        MemoryRootConfig {
            platform: ops.platform,
            mount_point: ops.root.clone(),
            mount_point_source: RAM_MOUNT_POINT_ENV.to_string(),
            volume_name: None,
            volume_name_source: "not_applicable".to_string(),
            requested_capacity_bytes: 0,
            capacity_source: "platform_default".to_string(),
            minimum_available_bytes: 0,
            minimum_available_source: "built_in".to_string(),
            runtime_root: ops.runtime_root.clone(),
            runtime_root_source: "mount_point/ait-runtime".to_string(),
            auto_mount_allowed: false,
            auto_mount_source: "built_in_false".to_string(),
            mount_lock_path: PathBuf::from("/host/ait/ram.lock"),
            mount_lock_source: "test".to_string(),
        }
    }

    #[test]
    fn environment_precedence_and_defaults_are_bounded() {
        let ops = FakeOps::macos(true);
        let values = BTreeMap::from([
            ("HOME", "/Users/test"),
            (LEGACY_RAM_MOUNT_POINT_ENV, "/Volumes/LEGACY"),
            (RAM_MOUNT_POINT_ENV, "/Volumes/FAST_RAM"),
            (RAM_VOLUME_NAME_ENV, "FAST_RAM"),
            (RAM_CAPACITY_BYTES_ENV, "4294967296"),
            (RAM_MIN_AVAILABLE_BYTES_ENV, "1073741824"),
            (RUNTIME_RAM_ROOT_ENV, "/Volumes/FAST_RAM/runtime"),
            (RAM_AUTO_MOUNT_ENV, "yes"),
            (
                RAM_MOUNT_LOCK_PATH_ENV,
                "/Users/test/.cache/ait/custom-ram.lock",
            ),
        ]);

        let config = MemoryRootConfig::from_lookup(false, &ops, lookup_from(&values)).unwrap();

        assert_eq!(config.mount_point, PathBuf::from("/Volumes/FAST_RAM"));
        assert_eq!(config.mount_point_source, RAM_MOUNT_POINT_ENV);
        assert_eq!(config.volume_name.as_deref(), Some("FAST_RAM"));
        assert_eq!(config.requested_capacity_bytes, 4_294_967_296);
        assert_eq!(config.minimum_available_bytes, 1_073_741_824);
        assert_eq!(
            config.runtime_root,
            PathBuf::from("/Volumes/FAST_RAM/runtime")
        );
        assert!(config.auto_mount_allowed);
    }

    #[test]
    fn malformed_values_and_overflow_fail_closed() {
        let ops = FakeOps::macos(false);
        for (name, value, expected) in [
            (RAM_CAPACITY_BYTES_ENV, "nope", "integer byte count"),
            (RAM_CAPACITY_BYTES_ENV, "0", "greater than zero"),
            (RAM_MIN_AVAILABLE_BYTES_ENV, "-1", "integer byte count"),
            (RAM_AUTO_MOUNT_ENV, "maybe", "must be one of"),
            (
                RAM_CAPACITY_BYTES_ENV,
                "18446744073709551615",
                "overflows sector rounding",
            ),
        ] {
            let values = BTreeMap::from([("HOME", "/Users/test"), (name, value)]);
            let error =
                MemoryRootConfig::from_lookup(false, &ops, lookup_from(&values)).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn mount_label_and_runtime_boundaries_fail_closed() {
        let ops = FakeOps::macos(false);
        let label_mismatch = BTreeMap::from([
            ("HOME", "/Users/test"),
            (RAM_MOUNT_POINT_ENV, "/Volumes/AIT_RAM"),
            (RAM_VOLUME_NAME_ENV, "OTHER"),
        ]);
        assert!(
            MemoryRootConfig::from_lookup(false, &ops, lookup_from(&label_mismatch))
                .unwrap_err()
                .contains("must exactly equal")
        );

        let runtime_escape = BTreeMap::from([
            ("HOME", "/Users/test"),
            (RUNTIME_RAM_ROOT_ENV, "/tmp/ait-runtime"),
        ]);
        assert!(
            MemoryRootConfig::from_lookup(false, &ops, lookup_from(&runtime_escape))
                .unwrap_err()
                .contains("strict descendant")
        );
    }

    #[test]
    fn already_mounted_root_never_takes_mount_lock() {
        let ops = FakeOps::macos(true);
        let payload = ensure_memory_root_with_ops(&macos_config(true, &ops), &ops).unwrap();

        assert_eq!(payload["state"], "pass");
        assert_eq!(payload["auto_mounted"], false);
        assert_eq!(payload["mount_lock"]["acquired"], false);
        assert_eq!(ops.lock_count.get(), 0);
        assert_eq!(ops.provision_count.get(), 0);
    }

    #[test]
    fn missing_root_requires_explicit_auto_mount_permission() {
        let ops = FakeOps::macos(false);
        let error = ensure_memory_root_with_ops(&macos_config(false, &ops), &ops).unwrap_err();

        assert!(error.contains(RAM_AUTO_MOUNT_ENV));
        assert_eq!(ops.lock_count.get(), 0);
        assert_eq!(ops.provision_count.get(), 0);
    }

    #[test]
    fn existing_non_ram_path_is_never_reformatted() {
        let mut ops = FakeOps::macos(false);
        ops.existing_non_ram = true;
        let error = ensure_memory_root_with_ops(&macos_config(true, &ops), &ops).unwrap_err();

        assert!(error.contains("Existing path"));
        assert_eq!(ops.lock_count.get(), 0);
        assert_eq!(ops.provision_count.get(), 0);
    }

    #[test]
    fn concurrent_mount_is_rechecked_under_short_lock() {
        let mut ops = FakeOps::macos(false);
        ops.mount_on_specs_call = Some(2);
        let payload = ensure_memory_root_with_ops(&macos_config(true, &ops), &ops).unwrap();

        assert_eq!(payload["auto_mounted"], false);
        assert_eq!(payload["mount_lock"]["acquired"], true);
        assert_eq!(ops.lock_count.get(), 1);
        assert_eq!(ops.provision_count.get(), 0);
        assert_eq!(
            ops.events.borrow().as_slice(),
            ["specs:1", "lock", "specs:2"]
        );
    }

    #[test]
    fn missing_root_is_provisioned_and_validated_under_lock() {
        let ops = FakeOps::macos(false);
        let payload = ensure_memory_root_with_ops(&macos_config(true, &ops), &ops).unwrap();

        assert_eq!(payload["auto_mounted"], true);
        assert_eq!(payload["actual_image_capacity_bytes"], 8_589_934_592u64);
        assert_eq!(ops.lock_count.get(), 1);
        assert_eq!(ops.provision_count.get(), 1);
        assert_eq!(ops.detach_count.get(), 0);
        assert_eq!(
            ops.events.borrow().as_slice(),
            ["specs:1", "lock", "specs:2", "provision", "specs:3"]
        );
    }

    #[test]
    fn attach_or_format_failure_is_reported_without_false_success() {
        for failure in ["attach failed", "format failed"] {
            let mut ops = FakeOps::macos(false);
            ops.provision_error = Some(failure.to_string());
            let error = ensure_memory_root_with_ops(&macos_config(true, &ops), &ops).unwrap_err();
            assert_eq!(error, failure);
            assert_eq!(ops.provision_count.get(), 1);
        }
    }

    #[test]
    fn concurrent_mount_after_provision_race_is_revalidated() {
        let mut ops = FakeOps::macos(false);
        ops.provision_error = Some("mount appeared before attach".to_string());
        ops.mount_on_provision_error = true;

        let payload = ensure_memory_root_with_ops(&macos_config(true, &ops), &ops).unwrap();

        assert_eq!(payload["auto_mounted"], false);
        assert_eq!(payload["mount_lock"]["acquired"], true);
        assert_eq!(ops.provision_count.get(), 1);
        assert_eq!(ops.detach_count.get(), 0);
    }

    #[test]
    fn invalid_new_mount_is_detached_and_cleanup_failure_is_fatal() {
        let mut ops = FakeOps::macos(false);
        ops.filesystem_available = 1;
        ops.detach_error = Some("detach failed".to_string());
        let mut config = macos_config(true, &ops);
        config.minimum_available_bytes = 2;

        let error = ensure_memory_root_with_ops(&config, &ops).unwrap_err();

        assert!(error.contains("below AIT_RAM_MIN_AVAILABLE_BYTES=2"));
        assert!(error.contains("Cleanup also failed: detach failed"));
        assert_eq!(ops.detach_count.get(), 1);
    }

    #[test]
    fn wrong_mount_capacity_free_space_and_symlink_escape_are_rejected() {
        let mut wrong_mount = FakeOps::macos(true);
        wrong_mount.volume_mount_point = PathBuf::from("/Volumes/OTHER");
        assert!(
            ensure_memory_root_with_ops(&macos_config(false, &wrong_mount), &wrong_mount)
                .unwrap_err()
                .contains("diskutil reports mount point")
        );

        let mut undersized = FakeOps::macos(true);
        undersized.image_sector_count = DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT - 1;
        assert!(
            ensure_memory_root_with_ops(&macos_config(false, &undersized), &undersized)
                .unwrap_err()
                .contains("below the requested")
        );

        let mut low_free = FakeOps::macos(true);
        low_free.filesystem_available = 1;
        let mut config = macos_config(false, &low_free);
        config.minimum_available_bytes = 2;
        assert!(ensure_memory_root_with_ops(&config, &low_free)
            .unwrap_err()
            .contains("available bytes"));

        let mut escaped = FakeOps::macos(true);
        escaped.canonical_runtime = PathBuf::from("/tmp/escaped");
        assert!(
            ensure_memory_root_with_ops(&macos_config(false, &escaped), &escaped)
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
        assert!(ensure_memory_root_with_ops(&config, &ops)
            .unwrap_err()
            .contains("not tmpfs or ramfs"));

        ops.linux_fstype = Some("tmpfs".to_string());
        let payload = ensure_memory_root_with_ops(&config, &ops).unwrap();
        assert_eq!(payload["platform_proof"], "linux_mountinfo:tmpfs");
        assert_eq!(payload["mount_lock"]["acquired"], false);
    }

    #[test]
    fn windows_requires_drive_ramdisk_without_persistent_fallback() {
        let mut ops = FakeOps::macos(true);
        ops.rebase(TaskWorktreePlatform::Windows, PathBuf::from("R:\\"));
        ops.windows_drive_type = Some(3);
        let config = config_for_existing_platform_root(&ops);
        assert!(ensure_memory_root_with_ops(&config, &ops)
            .unwrap_err()
            .contains("not a DRIVE_RAMDISK"));

        ops.windows_drive_type = Some(WINDOWS_DRIVE_RAMDISK);
        let payload = ensure_memory_root_with_ops(&config, &ops).unwrap();
        assert_eq!(
            payload["platform_proof"],
            "windows_drive_type:DRIVE_RAMDISK"
        );
        assert_eq!(payload["mount_lock"]["acquired"], false);
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
