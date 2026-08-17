use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};

pub const SERVER_BINARY_LIFECYCLE_CONTRACT: &str = "ait.server.binary-lifecycle.v1";
pub const SERVER_BINARY_LAYOUT_ID: u32 = 1;
pub const SERVER_DATA_ENV: &str = crate::environment_contract::names::AIT_NATIVE_SERVER_DATA;
pub const SERVER_BINARY_ACTIVATION_SCHEMA: &str = "ait.server.binary_v0.activation.v1";
pub const SERVER_FRESH_COMPLETION_FILE: &str = "generation-complete.json";
pub const SERVER_LEGACY_CONVERSION_COMPLETION_FILE: &str = "conversion-complete.json";

const SERVER_BINARY_DIR: &str = "binary-v0";
const SERVER_GENERATIONS_DIR: &str = "generations";
const SERVER_ACTIVATION_FILE: &str = "active.json";
const SERVER_LIFECYCLE_LOCK_FILE: &str = "lifecycle.lock";
const SERVER_RUNTIME_DIR: &str = "runtime";
const SERVER_RUNTIME_LEASE_FILE: &str = "worker-leases.bin";
const MAX_ACTIVATION_BYTES: u64 = 64 * 1024;
const MAX_COMPLETION_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryLifecycleConfig {
    pub server_data_root: PathBuf,
    pub binary_root: PathBuf,
    pub generations_root: PathBuf,
    pub activation_pointer: PathBuf,
    pub lifecycle_lock: PathBuf,
    pub runtime_lease_replica: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryActivation {
    pub activation_pointer: PathBuf,
    pub generation_root: PathBuf,
    pub completion_file: PathBuf,
    pub completion_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ActivationPointer {
    schema: String,
    layout_id: u32,
    generation: String,
    completion_sha256: String,
}

impl ServerBinaryLifecycleConfig {
    pub fn from_server_data_root(server_data_root: PathBuf) -> Result<Self, String> {
        validate_absolute_path("server data root", &server_data_root)?;

        let binary_root = server_data_root.join(SERVER_BINARY_DIR);
        let generations_root = binary_root.join(SERVER_GENERATIONS_DIR);
        let activation_pointer = binary_root.join(SERVER_ACTIVATION_FILE);
        let runtime_lease_replica = server_data_root
            .join(SERVER_RUNTIME_DIR)
            .join(SERVER_RUNTIME_LEASE_FILE);

        validate_absolute_path("Binary activation pointer", &activation_pointer)?;
        validate_absolute_path("runtime lease replica", &runtime_lease_replica)?;
        if activation_pointer.starts_with(&generations_root) {
            return Err(
                "Binary activation pointer must stay outside activated generation roots"
                    .to_string(),
            );
        }
        if runtime_lease_replica.starts_with(&generations_root) {
            return Err(
                "runtime lease replica must stay outside every activated generation root"
                    .to_string(),
            );
        }

        Ok(Self {
            server_data_root,
            lifecycle_lock: binary_root.join(SERVER_LIFECYCLE_LOCK_FILE),
            binary_root,
            generations_root,
            activation_pointer,
            runtime_lease_replica,
        })
    }

    pub fn ensure_layout(&self) -> Result<(), String> {
        ensure_real_directory(&self.server_data_root)?;
        ensure_real_directory(&self.binary_root)?;
        ensure_real_directory(&self.generations_root)?;
        let activation_parent = self.activation_pointer.parent().ok_or_else(|| {
            format!(
                "activation pointer has no parent: {}",
                self.activation_pointer.display()
            )
        })?;
        ensure_real_directory(activation_parent)?;
        let lease_parent = self.runtime_lease_replica.parent().ok_or_else(|| {
            format!(
                "runtime lease replica has no parent: {}",
                self.runtime_lease_replica.display()
            )
        })?;
        ensure_real_directory(lease_parent)?;
        Ok(())
    }

    pub fn activation(&self) -> Result<Option<ServerBinaryActivation>, String> {
        let pointer_bytes = match read_regular_file(&self.activation_pointer, MAX_ACTIVATION_BYTES)
        {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "failed to read Binary server activation pointer {}: {error}",
                    self.activation_pointer.display()
                ))
            }
        };
        let pointer: ActivationPointer =
            serde_json::from_slice(&pointer_bytes).map_err(|error| {
                format!(
                    "failed to parse Binary server activation pointer {}: {error}",
                    self.activation_pointer.display()
                )
            })?;
        if pointer.schema != SERVER_BINARY_ACTIVATION_SCHEMA
            || pointer.layout_id != SERVER_BINARY_LAYOUT_ID
            || !is_sha256(&pointer.completion_sha256)
        {
            return Err("Binary server activation pointer envelope is invalid".to_string());
        }
        let generation_path = PathBuf::from(&pointer.generation);
        validate_absolute_path("Binary server activated generation", &generation_path)?;
        let generation_root = canonical_real_directory(&generation_path)?;
        if path_is_within(&self.runtime_lease_replica, &generation_root) {
            return Err(
                "Binary runtime lease replica must stay outside the activated generation root"
                    .to_string(),
            );
        }
        let completion_file =
            matching_completion_file(&generation_root, &pointer.completion_sha256)?;
        Ok(Some(ServerBinaryActivation {
            activation_pointer: self.activation_pointer.clone(),
            generation_root,
            completion_file,
            completion_sha256: pointer.completion_sha256,
        }))
    }

    pub fn ensure_fresh_activation<F>(
        &self,
        initialize: F,
    ) -> Result<ServerBinaryActivation, String>
    where
        F: FnOnce(&Path) -> Result<(), String>,
    {
        self.ensure_layout()?;
        reject_non_regular_if_present(&self.lifecycle_lock)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.lifecycle_lock)
            .map_err(|error| {
                format!(
                    "failed to open Binary server lifecycle lock {}: {error}",
                    self.lifecycle_lock.display()
                )
            })?;
        lock.lock_exclusive().map_err(|error| {
            format!(
                "failed to acquire Binary server lifecycle lock {}: {error}",
                self.lifecycle_lock.display()
            )
        })?;
        let result = self.ensure_fresh_activation_locked(initialize);
        FileExt::unlock(&lock).map_err(|error| {
            format!(
                "failed to release Binary server lifecycle lock {}: {error}",
                self.lifecycle_lock.display()
            )
        })?;
        result
    }

    fn ensure_fresh_activation_locked<F>(
        &self,
        initialize: F,
    ) -> Result<ServerBinaryActivation, String>
    where
        F: FnOnce(&Path) -> Result<(), String>,
    {
        if let Some(active) = self.activation()? {
            return Ok(active);
        }
        let generation_path = self.unique_fresh_generation_path()?;
        initialize(&generation_path)?;
        let generation_root = canonical_real_directory(&generation_path)?;
        let canonical_generations = canonical_real_directory(&self.generations_root)?;
        if generation_root.parent() != Some(canonical_generations.as_path()) {
            return Err(
                "fresh Binary server generation must be a direct child of generations root"
                    .to_string(),
            );
        }
        let completion_file = generation_root.join(SERVER_FRESH_COMPLETION_FILE);
        let completion_bytes =
            read_regular_file(&completion_file, MAX_COMPLETION_BYTES).map_err(|error| {
                format!(
                    "failed to read fresh Binary server completion evidence {}: {error}",
                    completion_file.display()
                )
            })?;
        let completion_sha256 = sha256(&completion_bytes);
        self.write_new_activation(&generation_root, &completion_sha256)?;
        self.activation()?
            .ok_or_else(|| "Binary server activation disappeared after creation".to_string())
    }

    fn unique_fresh_generation_path(&self) -> Result<PathBuf, String> {
        for _ in 0..32 {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random).map_err(|error| {
                format!("failed to generate Binary server generation ID: {error}")
            })?;
            let name = format!("fresh-{}", hex(&random));
            let path = self.generations_root.join(name);
            if !path.exists() {
                return Ok(path);
            }
        }
        Err("failed to allocate a unique Binary server generation path".to_string())
    }

    fn write_new_activation(
        &self,
        generation_root: &Path,
        completion_sha256: &str,
    ) -> Result<(), String> {
        if !is_sha256(completion_sha256) {
            return Err("Binary server completion SHA-256 is invalid".to_string());
        }
        reject_non_regular_if_present(&self.activation_pointer)?;
        if self.activation_pointer.exists() {
            return Err(format!(
                "Binary server activation already exists: {}",
                self.activation_pointer.display()
            ));
        }
        let generation = generation_root
            .to_str()
            .ok_or_else(|| "Binary server generation path is not UTF-8".to_string())?;
        let pointer = ActivationPointer {
            schema: SERVER_BINARY_ACTIVATION_SCHEMA.to_string(),
            layout_id: SERVER_BINARY_LAYOUT_ID,
            generation: generation.to_string(),
            completion_sha256: completion_sha256.to_string(),
        };
        let mut bytes = serde_json::to_vec_pretty(&pointer)
            .map_err(|error| format!("failed to encode Binary server activation: {error}"))?;
        bytes.push(b'\n');
        atomic_create(&self.activation_pointer, &bytes)
    }
}

fn validate_absolute_path(label: &str, path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!(
            "{label} must be an absolute path: {}",
            path.display()
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(format!(
            "{label} must not contain dot path segments: {}",
            path.display()
        ));
    }
    Ok(())
}

fn ensure_real_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("failed to create directory {}: {error}", path.display()))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect directory {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("path is not a real directory: {}", path.display()));
    }
    Ok(())
}

fn canonical_real_directory(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect directory {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("path is not a real directory: {}", path.display()));
    }
    path.canonicalize()
        .map_err(|error| format!("failed to resolve directory {}: {error}", path.display()))
}

fn reject_non_regular_if_present(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(format!(
            "path exists but is not a regular file: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

fn read_regular_file(path: &Path, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    let link_metadata = fs::symlink_metadata(path)?;
    if link_metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("path is not a real regular file: {}", path.display()),
        ));
    }
    if metadata.len() > max_bytes {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("file exceeds {max_bytes} byte limit: {}", path.display()),
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn matching_completion_file(
    generation_root: &Path,
    expected_sha256: &str,
) -> Result<PathBuf, String> {
    let mut matches = Vec::new();
    for name in [
        SERVER_FRESH_COMPLETION_FILE,
        SERVER_LEGACY_CONVERSION_COMPLETION_FILE,
    ] {
        let path = generation_root.join(name);
        match read_regular_file(&path, MAX_COMPLETION_BYTES) {
            Ok(bytes) if sha256(&bytes) == expected_sha256 => matches.push(path),
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to read Binary server completion evidence {}: {error}",
                    path.display()
                ))
            }
        }
    }
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(
            "Binary server activation completion hash disagrees with generation evidence"
                .to_string(),
        ),
        _ => Err("Binary server activation completion evidence is ambiguous".to_string()),
    }
}

fn atomic_create(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|error| format!("failed to generate activation temporary name: {error}"))?;
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| format!("activation file name is not UTF-8: {}", path.display()))?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", hex(&random)));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| {
            format!(
                "failed to create activation temporary file {}: {error}",
                temporary.display()
            )
        })?;
    let write_result = (|| -> Result<(), String> {
        file.write_all(bytes).map_err(|error| {
            format!(
                "failed to write activation temporary file {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync activation temporary file {}: {error}",
                temporary.display()
            )
        })?;
        fs::hard_link(&temporary, path).map_err(|error| {
            format!(
                "failed to install new Binary server activation {}: {error}",
                path.display()
            )
        })?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                format!(
                    "failed to sync activation directory {}: {error}",
                    parent.display()
                )
            })?;
        Ok(())
    })();
    let cleanup_result = fs::remove_file(&temporary);
    write_result?;
    cleanup_result.map_err(|error| {
        format!(
            "failed to remove activation temporary file {}: {error}",
            temporary.display()
        )
    })
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    if path.exists() {
        path.canonicalize()
            .is_ok_and(|resolved| resolved.starts_with(root))
    } else {
        path.starts_with(root)
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    fn config(temp: &TempDir) -> ServerBinaryLifecycleConfig {
        let data = temp.path().join("server-data");
        ServerBinaryLifecycleConfig::from_server_data_root(data).unwrap()
    }

    fn write_fresh_generation(path: &Path) -> Result<(), String> {
        fs::create_dir_all(path)
            .map_err(|error| format!("failed to create test generation: {error}"))?;
        fs::write(
            path.join(SERVER_FRESH_COMPLETION_FILE),
            b"{\"schema\":\"ait.server.binary_v0.fresh.complete.v1\",\"layout_id\":1,\"status\":\"validated_inactive\"}\n",
        )
        .map_err(|error| format!("failed to write test completion: {error}"))
    }

    #[test]
    fn config_derives_every_binary_path_from_the_typed_data_root() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join("server-data");
        let config = ServerBinaryLifecycleConfig::from_server_data_root(data.clone()).unwrap();

        assert_eq!(config.server_data_root, data);
        assert_eq!(
            config.activation_pointer,
            data.join("binary-v0/active.json")
        );
        assert_eq!(config.generations_root, data.join("binary-v0/generations"));
        assert_eq!(
            config.runtime_lease_replica,
            data.join("runtime/worker-leases.bin")
        );
    }

    #[test]
    fn data_root_must_be_absolute_and_normalized() {
        let temp = TempDir::new().unwrap();
        assert!(
            ServerBinaryLifecycleConfig::from_server_data_root(PathBuf::from("relative"))
                .unwrap_err()
                .contains("absolute")
        );
        assert!(ServerBinaryLifecycleConfig::from_server_data_root(
            temp.path().join("server-data").join("..")
        )
        .unwrap_err()
        .contains("dot path segments"));
    }

    #[test]
    fn fresh_initialization_is_locked_activated_once_and_ignores_legacy_dsn() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let calls = AtomicUsize::new(0);
        let activated = config
            .ensure_fresh_activation(|path| {
                calls.fetch_add(1, Ordering::SeqCst);
                write_fresh_generation(path)
            })
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            activated.generation_root.parent(),
            Some(config.generations_root.canonicalize().unwrap().as_path())
        );
        assert_eq!(
            activated.completion_file.file_name(),
            Some(OsStr::new(SERVER_FRESH_COMPLETION_FILE))
        );
        assert!(!config
            .runtime_lease_replica
            .starts_with(&activated.generation_root));
        let pointer: ActivationPointer =
            serde_json::from_slice(&fs::read(&config.activation_pointer).unwrap()).unwrap();
        assert_eq!(pointer.schema, SERVER_BINARY_ACTIVATION_SCHEMA);
        assert_eq!(pointer.layout_id, SERVER_BINARY_LAYOUT_ID);
        assert_eq!(pointer.completion_sha256, activated.completion_sha256);

        let second = config
            .ensure_fresh_activation(|_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Err("initializer must not run twice".to_string())
            })
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(second, activated);
    }

    #[test]
    fn failed_initialization_never_publishes_an_activation_pointer() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let error = config
            .ensure_fresh_activation(|path| {
                fs::create_dir_all(path).unwrap();
                Err("injected initializer failure".to_string())
            })
            .unwrap_err();
        assert!(error.contains("injected initializer failure"));
        assert!(!config.activation_pointer.exists());
    }

    #[test]
    fn activation_rejects_tampered_or_ambiguous_completion_evidence() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp);
        let activated = config
            .ensure_fresh_activation(write_fresh_generation)
            .unwrap();
        fs::write(&activated.completion_file, b"tampered\n").unwrap();
        assert!(config
            .activation()
            .unwrap_err()
            .contains("completion hash disagrees"));

        let pointer: ActivationPointer =
            serde_json::from_slice(&fs::read(&config.activation_pointer).unwrap()).unwrap();
        let original = b"same completion\n";
        fs::write(&activated.completion_file, original).unwrap();
        fs::write(
            activated
                .generation_root
                .join(SERVER_LEGACY_CONVERSION_COMPLETION_FILE),
            original,
        )
        .unwrap();
        let replacement = ActivationPointer {
            completion_sha256: sha256(original),
            ..pointer
        };
        fs::write(
            &config.activation_pointer,
            serde_json::to_vec(&replacement).unwrap(),
        )
        .unwrap();
        assert!(config
            .activation()
            .unwrap_err()
            .contains("evidence is ambiguous"));
    }
}
