use ait_core::json_support::JsonValue;
use std::path::{Path, PathBuf};

pub(super) const DEFAULT_MACOS_RAM_VOLUME_NAME: &str = "AIT_RAM";
pub(super) const DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT: i64 = 16_777_216;
pub(super) const MACOS_RAM_VOLUME_KIND: &str = "macos_ram_volume";
pub(super) const LINUX_MEMORY_ROOT_KIND: &str = "linux_memory_root";
pub(super) const WINDOWS_RAMDISK_KIND: &str = "windows_ramdisk";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TaskWorktreePlatform {
    Linux,
    Macos,
    Windows,
    Other,
}

impl TaskWorktreePlatform {
    pub(super) fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }

    pub(super) fn from_text(value: Option<&str>) -> Self {
        let Some(raw) = normalized_text(value) else {
            return Self::current();
        };
        if raw.starts_with("linux") {
            Self::Linux
        } else if raw.starts_with("darwin") || raw.starts_with("macos") {
            Self::Macos
        } else if raw.starts_with("win") {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TaskWorktreeMemoryRootKind {
    MacosRamVolume,
    LinuxMemoryRoot,
    WindowsRamdisk,
}

impl TaskWorktreeMemoryRootKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::MacosRamVolume => MACOS_RAM_VOLUME_KIND,
            Self::LinuxMemoryRoot => LINUX_MEMORY_ROOT_KIND,
            Self::WindowsRamdisk => WINDOWS_RAMDISK_KIND,
        }
    }

    pub(super) fn from_text(value: Option<&str>) -> Option<Self> {
        match normalized_text(value)?.as_str() {
            MACOS_RAM_VOLUME_KIND => Some(Self::MacosRamVolume),
            LINUX_MEMORY_ROOT_KIND => Some(Self::LinuxMemoryRoot),
            WINDOWS_RAMDISK_KIND => Some(Self::WindowsRamdisk),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TaskWorktreeMemoryRoot {
    pub(super) kind: TaskWorktreeMemoryRootKind,
    pub(super) root: PathBuf,
    pub(super) volume_name: Option<String>,
    pub(super) sector_count: Option<i64>,
}

impl TaskWorktreeMemoryRoot {
    pub(super) fn to_json(&self) -> JsonValue {
        let mut payload = ait_core::json_support::JsonMap::new();
        payload.insert(
            "kind".to_string(),
            JsonValue::String(self.kind.as_str().to_string()),
        );
        payload.insert(
            "root".to_string(),
            JsonValue::String(self.root.to_string_lossy().to_string()),
        );
        if let Some(volume_name) = &self.volume_name {
            payload.insert(
                "volume_name".to_string(),
                JsonValue::String(volume_name.clone()),
            );
        }
        if let Some(sector_count) = self.sector_count {
            payload.insert(
                "sector_count".to_string(),
                JsonValue::Number(sector_count.into()),
            );
        }
        JsonValue::Object(payload)
    }
}

pub(super) trait TaskWorktreeOps {
    fn platform(&self) -> TaskWorktreePlatform;
    fn linux_detected_memory_roots(&self) -> Vec<PathBuf>;
    fn windows_ramdisk_roots(&self) -> Vec<PathBuf>;
    fn macos_ram_volume_specs(&self) -> Vec<TaskWorktreeMemoryRoot>;
    fn macos_default_ram_volume_spec(&self) -> TaskWorktreeMemoryRoot;
    fn ensure_memory_root_available(&self, spec: &TaskWorktreeMemoryRoot) -> bool;
    fn ensure_root_candidate(&self, path: &Path) -> Option<PathBuf>;
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
