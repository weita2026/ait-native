use super::*;

pub(super) const NATIVE_DISTRIBUTION_CONTRACT: &str = "ait-native-distribution/v1";
pub(super) const NATIVE_BUNDLE_CONTRACT: &str = "ait-native-bundle/v1";
pub(super) const NATIVE_SOURCE_CONTRACT: &str = "ait-native-source/v1";
pub(super) const NATIVE_MATRIX_REVISION: &str = "six-target-2026-07-19.1";
pub(super) const NATIVE_BUNDLE_MANIFEST_PATH: &str = "ait-native-bundle.json";
const NATIVE_SMOKE_CONTRACT: &str = "ait-native-smoke-evidence/v1";

const REQUIRED_TARGET_TRIPLES: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
];
const REQUIRED_CONSUMER_ECOSYSTEMS: &[&str] = &["pip", "npm", "homebrew", "apt"];

#[derive(Clone, Debug)]
pub struct NativeSourceRequest {
    pub release_id: String,
    pub target: String,
    pub source_dir: PathBuf,
    pub runner: String,
    pub runner_image: String,
    pub rust_toolchain: String,
    pub rustc_path: PathBuf,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct NativeTarget {
    pub(super) triple: String,
    pub(super) os: String,
    pub(super) architecture: String,
    pub(super) libc: Option<String>,
    pub(super) executable_suffix: String,
}

impl NativeTarget {
    pub(super) fn parse(triple: &str) -> Result<Self, String> {
        let target = match triple.trim() {
            "aarch64-apple-darwin" => Self {
                triple: "aarch64-apple-darwin".to_string(),
                os: "macos".to_string(),
                architecture: "arm64".to_string(),
                libc: None,
                executable_suffix: String::new(),
            },
            "x86_64-apple-darwin" => Self {
                triple: "x86_64-apple-darwin".to_string(),
                os: "macos".to_string(),
                architecture: "x86_64".to_string(),
                libc: None,
                executable_suffix: String::new(),
            },
            "aarch64-unknown-linux-gnu" => Self {
                triple: "aarch64-unknown-linux-gnu".to_string(),
                os: "linux".to_string(),
                architecture: "arm64".to_string(),
                libc: Some("gnu".to_string()),
                executable_suffix: String::new(),
            },
            "x86_64-unknown-linux-gnu" => Self {
                triple: "x86_64-unknown-linux-gnu".to_string(),
                os: "linux".to_string(),
                architecture: "x86_64".to_string(),
                libc: Some("gnu".to_string()),
                executable_suffix: String::new(),
            },
            "aarch64-pc-windows-msvc" => Self {
                triple: "aarch64-pc-windows-msvc".to_string(),
                os: "windows".to_string(),
                architecture: "arm64".to_string(),
                libc: None,
                executable_suffix: ".exe".to_string(),
            },
            "x86_64-pc-windows-msvc" => Self {
                triple: "x86_64-pc-windows-msvc".to_string(),
                os: "windows".to_string(),
                architecture: "x86_64".to_string(),
                libc: None,
                executable_suffix: ".exe".to_string(),
            },
            other => {
                return Err(format!(
                    "Unsupported native release target {other:?}. Supported targets: {}.",
                    REQUIRED_TARGET_TRIPLES.join(", ")
                ));
            }
        };
        Ok(target)
    }

    pub(super) fn to_json(&self) -> JsonValue {
        json!({
            "triple": self.triple,
            "os": self.os,
            "architecture": self.architecture,
            "libc": self.libc,
            "executable_suffix": self.executable_suffix,
        })
    }

    fn command_archive_path(&self, public_identity: &str) -> String {
        format!("bin/{public_identity}{}", self.executable_suffix)
    }

    fn ecosystems(&self) -> Vec<&'static str> {
        match self.os.as_str() {
            "macos" => vec!["pip", "npm", "homebrew"],
            "linux" => vec!["pip", "npm", "homebrew", "apt"],
            "windows" => vec!["pip", "npm"],
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeCommandProfile {
    Cli,
    CliWithAgent,
}

impl NativeCommandProfile {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "cli" => Ok(Self::Cli),
            "cli-with-agent" => Ok(Self::CliWithAgent),
            other => Err(format!(
                "Unsupported native command profile {other:?}; expected cli or cli-with-agent."
            )),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::CliWithAgent => "cli-with-agent",
        }
    }

    fn commands(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Cli => &[("ait", "ait-cli")],
            Self::CliWithAgent => &[
                ("ait", "ait-cli"),
                ("ait-agent", "ait-agent"),
                ("ait-agent-worker", "ait-agent-worker"),
            ],
        }
    }
}

#[derive(Clone, Debug)]
struct NativeCommandInput {
    public_identity: String,
    source_binary_identity: String,
    archive_path: String,
    data: Vec<u8>,
    mode: u32,
}

impl NativeCommandInput {
    fn to_manifest_json(&self) -> JsonValue {
        let paired_with = match self.public_identity.as_str() {
            "ait-agent" => json!(["ait-agent-worker"]),
            "ait-agent-worker" => json!(["ait-agent"]),
            _ => json!([]),
        };
        json!({
            "public_identity": self.public_identity,
            "source_binary_identity": self.source_binary_identity,
            "archive_path": self.archive_path,
            "runtime_authority": "rust",
            "sha256": sha256_hex(&self.data),
            "size_bytes": self.data.len(),
            "executable_mode": format!("{:04o}", self.mode),
            "paired_with": paired_with,
        })
    }
}

#[derive(Clone, Debug)]
struct NativeBundleBuild {
    artifact: JsonValue,
    target: NativeTarget,
    ait_sha256: String,
}

pub(super) fn supported_native_targets() -> Vec<NativeTarget> {
    REQUIRED_TARGET_TRIPLES
        .iter()
        .map(|triple| NativeTarget::parse(triple).expect("required target is valid"))
        .collect()
}

pub(super) fn native_launcher_conformance_contract() -> JsonValue {
    json!({
        "contract": "ait-thin-launcher-conformance/v1",
        "allowed": [
            "normalize_supported_target",
            "resolve_package_local_binary",
            "validate_version_and_artifact_identity",
            "forward_argv_cwd_environment_and_stdio",
            "preserve_exit_status_and_signal_behavior"
        ],
        "forbidden": [
            "first_invocation_download",
            "path_substitution",
            "subcommand_parsing",
            "repository_mutation",
            "json_interpretation",
            "retry_or_error_translation",
            "python_or_node_workflow_fallback",
            "per_invocation_full_binary_rehash"
        ],
        "failure_policy": "fail_closed",
        "native_runtime_authority": "rust",
        "public_command": "ait",
        "source_binary": "ait-cli",
    })
}

pub(super) fn native_distribution_candidate_contract(
    version: &str,
    release_id: &str,
    snapshot_id: &str,
) -> JsonValue {
    json!({
        "contract": NATIVE_DISTRIBUTION_CONTRACT,
        "matrix_revision": NATIVE_MATRIX_REVISION,
        "state": "configured_unbuilt",
        "version": version,
        "release_id": release_id,
        "source_snapshot_id": snapshot_id,
        "command_profile": "cli",
        "configured_targets": supported_native_targets().iter().map(NativeTarget::to_json).collect::<Vec<_>>(),
        "required_consumer_ecosystems": REQUIRED_CONSUMER_ECOSYSTEMS,
        "launcher_conformance": native_launcher_conformance_contract(),
        "source_layout": "<matrix-dir>/<rust-target-triple>/release/{ait-cli[.exe],ait-native-source.json}",
        "built_targets": [],
        "missing_targets": REQUIRED_TARGET_TRIPLES,
        "rejected_targets": [],
        "consumer_projections": [],
        "multi_ecosystem_ready": false,
    })
}

fn current_host_target() -> Option<NativeTarget> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    };
    NativeTarget::parse(triple).ok()
}

fn native_matrix_root(explicit: Option<&Path>) -> Option<PathBuf> {
    explicit
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

fn host_release_source_dirs(
    repo: &RepoRuntime,
    explicit_source_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = explicit_source_dir.filter(|path| !path.as_os_str().is_empty()) {
        candidates.push(path.to_path_buf());
    }
    if let Ok(current_executable) = std::env::current_exe() {
        if let Some(parent) = current_executable.parent() {
            candidates.push(parent.to_path_buf());
        }
    }
    let authoritative = repo.authoritative_repo_root();
    candidates.push(authoritative.join(".ait/cargo-target/release"));
    if let Some(parent) = authoritative.parent() {
        candidates.push(parent.join("ait-core/.ait/cargo-target/release"));
    }
    let mut seen = BTreeSet::new();
    candidates.retain(|candidate| seen.insert(candidate.clone()));
    candidates
}

fn target_source_dir(
    repo: &RepoRuntime,
    target: &NativeTarget,
    matrix_root: Option<&Path>,
    host_source_dir: Option<&Path>,
) -> Option<(PathBuf, bool)> {
    if let Some(root) = matrix_root {
        return Some((root.join(&target.triple).join("release"), true));
    }
    if current_host_target().as_ref() != Some(target) {
        return None;
    }
    host_release_source_dirs(repo, host_source_dir)
        .into_iter()
        .find(|directory| {
            is_release_source_path(directory)
                && directory
                    .join(format!("ait-cli{}", target.executable_suffix))
                    .is_file()
        })
        .map(|directory| (directory, false))
}

fn is_release_source_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    !normalized.ends_with("/debug")
        && !normalized.contains("/target/debug/")
        && !normalized.contains("/cargo-target/debug/")
        && (normalized.ends_with("/release")
            || normalized.contains("/target/release/")
            || normalized.contains("/cargo-target/release/"))
}

fn validate_archive_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!(
            "Native bundle archive path {path:?} must be a normalized relative path without traversal."
        ));
    }
    Ok(())
}

fn read_native_source_descriptor(source_dir: &Path) -> Result<JsonValue, String> {
    let path = source_dir.join("ait-native-source.json");
    let bytes = fs::read(&path).map_err(|err| {
        format!(
            "Target source {} is missing ait-native-source.json ({err}).",
            source_dir.display()
        )
    })?;
    parse_slice_value(&bytes, "ait-native-source.json must contain valid JSON")
}

fn descriptor_command<'a>(descriptor: &'a JsonValue, identity: &str) -> Option<&'a JsonValue> {
    descriptor
        .get("commands")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .find(|row| string_field(row, "public_identity").as_deref() == Some(identity))
}

fn required_observed_fact(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(format!(
            "Native source evidence {label} must be a non-empty single-line value."
        ));
    }
    Ok(value.to_string())
}

fn rustc_evidence(request: &NativeSourceRequest) -> Result<JsonValue, String> {
    let toolchain = required_observed_fact(&request.rust_toolchain, "rust_toolchain")?;
    let rustc_path = fs::canonicalize(&request.rustc_path).map_err(|err| {
        format!(
            "Cannot resolve the selected rustc executable {} ({err}).",
            request.rustc_path.display()
        )
    })?;
    if !fs::metadata(&rustc_path).map_err(io_error)?.is_file() {
        return Err(format!(
            "Selected rustc path {} is not a regular file.",
            rustc_path.display()
        ));
    }
    let output = Command::new(&rustc_path)
        .arg("--version")
        .output()
        .map_err(|err| format!("Failed to execute {} ({err}).", rustc_path.display()))?;
    let stdout = String::from_utf8(output.stdout.clone())
        .map_err(|_| "rustc --version emitted non-UTF-8 stdout.".to_string())?;
    let observed = stdout.trim();
    let mut observed_parts = observed.split_whitespace();
    let observed_command = observed_parts.next().unwrap_or_default();
    let observed_version = observed_parts.next().unwrap_or_default();
    if !output.status.success() || observed_command != "rustc" || observed_version != toolchain {
        return Err(format!(
            "Selected rustc version mismatch: expected {toolchain}, observed {observed:?}."
        ));
    }
    Ok(json!({
        "path": rustc_path.to_string_lossy(),
        "requested_toolchain": toolchain,
        "observed_version": observed,
        "exit_code": output.status.code(),
        "stdout_sha256": sha256_hex(&output.stdout),
        "stderr_sha256": sha256_hex(&output.stderr),
    }))
}

fn run_native_smoke_command(
    executable: &Path,
    cwd: &Path,
    check_id: &str,
    args: &[&str],
    expected_stdout: Option<&str>,
    require_json_stdout: bool,
) -> Result<JsonValue, String> {
    let mut command = Command::new(executable);
    command.current_dir(cwd).args(args);
    for name in [names::AIT_REPO_ROOT, "PYTHONPATH"] {
        command.env_remove(name);
    }
    let output = command.output().map_err(|err| {
        format!(
            "Native smoke {check_id} could not execute {} ({err}).",
            executable.display()
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "Native smoke {check_id} failed for {} with exit code {:?}: {}",
            executable.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if require_json_stdout {
        parse_slice_value(
            &output.stdout,
            &format!("Native smoke {check_id} must emit valid JSON"),
        )?;
    }
    if let Some(expected) = expected_stdout {
        let observed = String::from_utf8(output.stdout.clone())
            .map_err(|_| format!("Native smoke {check_id} emitted non-UTF-8 stdout."))?;
        if observed.trim() != expected {
            return Err(format!(
                "Native smoke {check_id} stdout mismatch: expected {expected:?}, observed {:?}.",
                observed.trim()
            ));
        }
    }
    Ok(json!({
        "check_id": check_id,
        "argv": args,
        "exit_code": output.status.code(),
        "stdout_channel_captured": true,
        "stderr_channel_captured": true,
        "stdout_nonempty": !output.stdout.is_empty(),
        "stderr_nonempty": !output.stderr.is_empty(),
        "stdout_json_valid": require_json_stdout,
        "stdout_identity_valid": expected_stdout.is_some(),
        "passed": true,
    }))
}

fn run_native_source_smoke(
    source_dir: &Path,
    target: &NativeTarget,
    version: &str,
    profile: NativeCommandProfile,
) -> Result<JsonValue, String> {
    let smoke_root = TempDirBuilder::new()
        .prefix("ait-native-release-smoke-")
        .tempdir()
        .map_err(io_error)?;
    let mut checks = Vec::new();
    for (public_identity, source_identity) in profile.commands() {
        let executable = source_dir.join(format!("{source_identity}{}", target.executable_suffix));
        let expected_version = format!("{public_identity} {version}");
        checks.push(run_native_smoke_command(
            &executable,
            smoke_root.path(),
            &format!("{public_identity}.version"),
            &["--version"],
            Some(&expected_version),
            false,
        )?);
        checks.push(run_native_smoke_command(
            &executable,
            smoke_root.path(),
            &format!("{public_identity}.help"),
            &["--help"],
            None,
            false,
        )?);
    }

    let ait = source_dir.join(format!("ait-cli{}", target.executable_suffix));
    checks.push(run_native_smoke_command(
        &ait,
        smoke_root.path(),
        "ait.init",
        &["init", "--json"],
        None,
        true,
    )?);
    checks.push(run_native_smoke_command(
        &ait,
        smoke_root.path(),
        "ait.status",
        &["status", "--json"],
        None,
        true,
    )?);
    checks.push(run_native_smoke_command(
        &ait,
        smoke_root.path(),
        "ait.plan-list",
        &["plan", "list", "--json"],
        None,
        true,
    )?);
    let check_count = checks.len();
    Ok(json!({
        "contract": NATIVE_SMOKE_CONTRACT,
        "target": target.triple,
        "native_execution": true,
        "passed": true,
        "checks": checks,
        "summary": {
            "check_count": check_count,
            "failed_count": 0,
        },
    }))
}

fn reject_unexpected_source_commands(
    source_dir: &Path,
    target: &NativeTarget,
    profile: NativeCommandProfile,
) -> Result<(), String> {
    let expected = profile
        .commands()
        .iter()
        .map(|(_, source)| format!("{source}{}", target.executable_suffix))
        .collect::<BTreeSet<_>>();
    for entry in fs::read_dir(source_dir).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Native release source directory contains a symlink: {}.",
                path.display()
            ));
        }
        if !metadata.file_type().is_file()
            || expected.contains(&file_name)
            || file_name == "ait-native-source.json"
        {
            continue;
        }
        let looks_executable = if target.os == "windows" {
            file_name.ends_with(".exe")
        } else {
            filesystem_mode(&metadata, 0) & 0o111 != 0
        };
        if looks_executable {
            return Err(format!(
                "Native release source directory contains unexpected executable {file_name:?} for profile {}.",
                profile.id()
            ));
        }
    }
    Ok(())
}

fn external_closure_digest(record: &JsonValue) -> Result<JsonValue, String> {
    let Some(closure) = record
        .get("metadata")
        .and_then(|metadata| metadata.get("external_closure"))
    else {
        return Ok(JsonValue::Null);
    };
    let bytes = encode_value_pretty_to_vec(
        closure,
        "failed to encode native source external closure evidence",
    )?;
    Ok(JsonValue::String(sha256_hex(&bytes)))
}

pub fn release_native_source(
    repo: &RepoRuntime,
    request: &NativeSourceRequest,
) -> Result<JsonValue, String> {
    let record = get_release_candidate(repo, request.release_id.trim())?;
    let release_id = required_string_field(&record, "release_id")?;
    let version = required_string_field(&record, "version")?;
    let source_snapshot_id = required_string_field(&record, "snapshot_id")?;
    let source_manifest_hash = required_string_field(&record, "manifest_hash")?;
    let target = NativeTarget::parse(request.target.trim())?;
    if current_host_target().as_ref() != Some(&target) {
        return Err(format!(
            "Native source evidence for {} must execute on the matching target architecture; current host is {:?}.",
            target.triple,
            current_host_target().map(|host| host.triple)
        ));
    }
    let profile = record
        .get("metadata")
        .and_then(|metadata| metadata.get("native_distribution"))
        .and_then(|distribution| distribution.get("command_profile"))
        .and_then(JsonValue::as_str)
        .map(NativeCommandProfile::parse)
        .transpose()?
        .unwrap_or(NativeCommandProfile::Cli);
    let source_dir = fs::canonicalize(&request.source_dir).map_err(|err| {
        format!(
            "Cannot resolve native release source directory {} ({err}).",
            request.source_dir.display()
        )
    })?;
    if !source_dir.is_dir() || !is_release_source_path(&source_dir) {
        return Err(format!(
            "Native source directory {} must be an existing release Cargo output directory.",
            source_dir.display()
        ));
    }
    reject_unexpected_source_commands(&source_dir, &target, profile)?;
    let commands = source_command_inputs(
        &source_dir,
        false,
        &target,
        &version,
        &release_id,
        &source_snapshot_id,
        &source_manifest_hash,
        profile,
    )?;
    let smoke = run_native_source_smoke(&source_dir, &target, &version, profile)?;
    let runner = required_observed_fact(&request.runner, "runner")?;
    let runner_image = required_observed_fact(&request.runner_image, "runner_image")?;
    let rustc = rustc_evidence(request)?;
    let command_rows = commands
        .iter()
        .map(NativeCommandInput::to_manifest_json)
        .collect::<Vec<_>>();
    let descriptor = json!({
        "contract": NATIVE_SOURCE_CONTRACT,
        "schema_version": 1,
        "version": version,
        "release_id": release_id,
        "source_snapshot_id": source_snapshot_id,
        "source_manifest_hash": source_manifest_hash,
        "external_closure_sha256": external_closure_digest(&record)?,
        "target": target.triple,
        "cargo_profile": "release",
        "command_profile": profile.id(),
        "commands": command_rows,
        "runner": {
            "label": runner,
            "image": runner_image,
            "os": target.os,
            "architecture": target.architecture,
            "rustc": rustc,
        },
        "smoke": smoke,
    });
    let bytes = encode_value_pretty_with_newline_error_string(&descriptor)?;
    let output_path = source_dir.join("ait-native-source.json");
    let created = if output_path.exists() {
        let metadata = fs::symlink_metadata(&output_path).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(format!(
                "Native source descriptor {} must be a regular non-symlink file.",
                output_path.display()
            ));
        }
        let existing = fs::read(&output_path).map_err(io_error)?;
        if existing != bytes.as_bytes() {
            return Err(format!(
                "Refusing to replace immutable native source evidence at {}; use a clean release output directory.",
                output_path.display()
            ));
        }
        false
    } else {
        let mut temporary = NamedTempFile::new_in(&source_dir).map_err(io_error)?;
        temporary.write_all(bytes.as_bytes()).map_err(io_error)?;
        temporary.as_file().sync_all().map_err(io_error)?;
        temporary.persist(&output_path).map_err(|err| {
            format!(
                "Failed to persist native source descriptor {} ({}).",
                output_path.display(),
                err.error
            )
        })?;
        true
    };
    Ok(json!({
        "command": "release native-source",
        "status": "ready",
        "release_id": descriptor["release_id"],
        "version": descriptor["version"],
        "snapshot_id": descriptor["source_snapshot_id"],
        "profile": descriptor["command_profile"],
        "target": descriptor["target"],
        "descriptor_path": output_path.to_string_lossy(),
        "descriptor_sha256": sha256_hex(bytes.as_bytes()),
        "created": created,
        "native_source": descriptor,
    }))
}

#[allow(clippy::too_many_arguments)]
fn source_command_inputs(
    source_dir: &Path,
    descriptor_required: bool,
    target: &NativeTarget,
    version: &str,
    release_id: &str,
    source_snapshot_id: &str,
    source_manifest_hash: &str,
    profile: NativeCommandProfile,
) -> Result<Vec<NativeCommandInput>, String> {
    if !is_release_source_path(source_dir) {
        return Err(format!(
            "Refusing non-release or debug native source directory {}.",
            source_dir.display()
        ));
    }
    let descriptor = if descriptor_required {
        let descriptor = read_native_source_descriptor(source_dir)?;
        if string_field(&descriptor, "contract").as_deref() != Some(NATIVE_SOURCE_CONTRACT)
            || descriptor.get("schema_version").and_then(JsonValue::as_u64) != Some(1)
        {
            return Err(format!(
                "Target {} source descriptor has an unsupported contract.",
                target.triple
            ));
        }
        if string_field(&descriptor, "target").as_deref() != Some(target.triple.as_str()) {
            return Err(format!(
                "Target {} source descriptor declares target {:?}.",
                target.triple,
                string_field(&descriptor, "target").unwrap_or_default()
            ));
        }
        if string_field(&descriptor, "version").as_deref() != Some(version) {
            return Err(format!(
                "Target {} source version does not match release version {version}.",
                target.triple
            ));
        }
        if string_field(&descriptor, "release_id").as_deref() != Some(release_id)
            || string_field(&descriptor, "source_snapshot_id").as_deref()
                != Some(source_snapshot_id)
            || string_field(&descriptor, "source_manifest_hash").as_deref()
                != Some(source_manifest_hash)
        {
            return Err(format!(
                "Target {} source descriptor release or source Snapshot identity disagrees with the release candidate.",
                target.triple
            ));
        }
        if string_field(&descriptor, "cargo_profile").as_deref() != Some("release") {
            return Err(format!(
                "Target {} source descriptor must declare cargo_profile release.",
                target.triple
            ));
        }
        if string_field(&descriptor, "command_profile").as_deref() != Some(profile.id()) {
            return Err(format!(
                "Target {} source command profile does not match {}.",
                target.triple,
                profile.id()
            ));
        }
        validate_native_source_runner(&descriptor, target)?;
        validate_native_source_smoke(&descriptor, target, profile)?;
        Some(descriptor)
    } else {
        None
    };

    let mut commands = Vec::new();
    for (public_identity, source_binary_identity) in profile.commands() {
        let source = source_dir.join(format!(
            "{source_binary_identity}{}",
            target.executable_suffix
        ));
        let metadata = fs::symlink_metadata(&source).map_err(|err| {
            format!(
                "Target {} is missing required source binary {} ({err}).",
                target.triple,
                source.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(format!(
                "Target {} source binary {} must be a regular non-symlink file.",
                target.triple,
                source.display()
            ));
        }
        // Windows executable identity is carried by the required `.exe` suffix;
        // POSIX execute bits are neither portable nor meaningful for those bytes.
        // The archive still normalizes every public command to mode 0755.
        let mode = if target.os == "windows" {
            0o755
        } else {
            filesystem_mode(&metadata, 0o755)
        };
        if target.os != "windows" && mode & 0o111 == 0 {
            return Err(format!(
                "Target {} source binary {} is not executable.",
                target.triple,
                source.display()
            ));
        }
        let data = fs::read(&source).map_err(io_error)?;
        if let Some(descriptor) = descriptor.as_ref() {
            let row = descriptor_command(descriptor, public_identity).ok_or_else(|| {
                format!(
                    "Target {} source descriptor is missing command {public_identity}.",
                    target.triple
                )
            })?;
            if string_field(row, "source_binary_identity").as_deref()
                != Some(*source_binary_identity)
                || string_field(row, "sha256").as_deref() != Some(sha256_hex(&data).as_str())
                || row.get("size_bytes").and_then(JsonValue::as_u64) != Some(data.len() as u64)
                || string_field(row, "executable_mode").as_deref()
                    != Some(format!("{:04o}", mode).as_str())
            {
                return Err(format!(
                    "Target {} source descriptor digest, size, mode, or binary identity disagrees for {public_identity}.",
                    target.triple
                ));
            }
        }
        let archive_path = target.command_archive_path(public_identity);
        validate_archive_path(&archive_path)?;
        commands.push(NativeCommandInput {
            public_identity: (*public_identity).to_string(),
            source_binary_identity: (*source_binary_identity).to_string(),
            archive_path,
            data,
            mode: 0o755,
        });
    }
    validate_command_membership(&commands, profile)?;
    Ok(commands)
}

fn validate_native_source_runner(
    descriptor: &JsonValue,
    target: &NativeTarget,
) -> Result<(), String> {
    let runner = descriptor.get("runner").ok_or_else(|| {
        format!(
            "Target {} source descriptor is missing runner evidence.",
            target.triple
        )
    })?;
    if string_field(runner, "label").unwrap_or_default().is_empty()
        || string_field(runner, "image").unwrap_or_default().is_empty()
        || string_field(runner, "os").as_deref() != Some(target.os.as_str())
        || string_field(runner, "architecture").as_deref() != Some(target.architecture.as_str())
    {
        return Err(format!(
            "Target {} source runner evidence is incomplete or host-mismatched.",
            target.triple
        ));
    }
    let rustc = runner.get("rustc").ok_or_else(|| {
        format!(
            "Target {} source descriptor is missing rustc evidence.",
            target.triple
        )
    })?;
    let requested = string_field(rustc, "requested_toolchain").unwrap_or_default();
    let observed = string_field(rustc, "observed_version").unwrap_or_default();
    if string_field(rustc, "path").unwrap_or_default().is_empty()
        || requested.is_empty()
        || observed.split_whitespace().next() != Some("rustc")
        || observed.split_whitespace().nth(1) != Some(requested.as_str())
        || rustc.get("exit_code").and_then(JsonValue::as_i64) != Some(0)
        || !string_field(rustc, "stdout_sha256")
            .as_deref()
            .is_some_and(valid_sha256)
        || !string_field(rustc, "stderr_sha256")
            .as_deref()
            .is_some_and(valid_sha256)
    {
        return Err(format!(
            "Target {} source rustc evidence is incomplete or disagrees with the selected toolchain.",
            target.triple
        ));
    }
    if let Some(digest) = descriptor
        .get("external_closure_sha256")
        .and_then(JsonValue::as_str)
    {
        if !valid_sha256(digest) {
            return Err(format!(
                "Target {} source external closure digest is invalid.",
                target.triple
            ));
        }
    }
    Ok(())
}

fn validate_native_source_smoke(
    descriptor: &JsonValue,
    target: &NativeTarget,
    profile: NativeCommandProfile,
) -> Result<(), String> {
    let smoke = descriptor.get("smoke").ok_or_else(|| {
        format!(
            "Target {} source descriptor is missing native smoke evidence.",
            target.triple
        )
    })?;
    if string_field(smoke, "contract").as_deref() != Some(NATIVE_SMOKE_CONTRACT)
        || string_field(smoke, "target").as_deref() != Some(target.triple.as_str())
        || !bool_field(smoke, "native_execution")
        || !bool_field(smoke, "passed")
        || smoke
            .get("summary")
            .and_then(|summary| summary.get("failed_count"))
            .and_then(JsonValue::as_u64)
            != Some(0)
    {
        return Err(format!(
            "Target {} source native smoke evidence is failed or identity-mismatched.",
            target.triple
        ));
    }
    let mut expected = BTreeSet::from([
        "ait.init".to_string(),
        "ait.plan-list".to_string(),
        "ait.status".to_string(),
    ]);
    for (public_identity, _) in profile.commands() {
        expected.insert(format!("{public_identity}.help"));
        expected.insert(format!("{public_identity}.version"));
    }
    let checks = smoke
        .get("checks")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("Target {} source smoke checks are missing.", target.triple))?;
    let mut observed = BTreeSet::new();
    for check in checks {
        let check_id = required_string_field(check, "check_id")?;
        if !observed.insert(check_id.clone())
            || !bool_field(check, "passed")
            || !bool_field(check, "stdout_channel_captured")
            || !bool_field(check, "stderr_channel_captured")
            || check.get("exit_code").and_then(JsonValue::as_i64) != Some(0)
        {
            return Err(format!(
                "Target {} source smoke check {check_id} is duplicated, failed, or did not preserve process channels.",
                target.triple
            ));
        }
        if matches!(
            check_id.as_str(),
            "ait.init" | "ait.status" | "ait.plan-list"
        ) && !bool_field(check, "stdout_json_valid")
        {
            return Err(format!(
                "Target {} source smoke check {check_id} did not return valid JSON.",
                target.triple
            ));
        }
        if check_id.ends_with(".version") && !bool_field(check, "stdout_identity_valid") {
            return Err(format!(
                "Target {} source smoke check {check_id} did not validate public version identity.",
                target.triple
            ));
        }
    }
    if observed != expected
        || smoke
            .get("summary")
            .and_then(|summary| summary.get("check_count"))
            .and_then(JsonValue::as_u64)
            != Some(expected.len() as u64)
    {
        return Err(format!(
            "Target {} source smoke membership does not match profile {}.",
            target.triple,
            profile.id()
        ));
    }
    Ok(())
}

fn validate_command_membership(
    commands: &[NativeCommandInput],
    profile: NativeCommandProfile,
) -> Result<(), String> {
    let mut identities = BTreeSet::new();
    for command in commands {
        if !identities.insert(command.public_identity.as_str()) {
            return Err(format!(
                "Native bundle contains duplicate command identity {}.",
                command.public_identity
            ));
        }
    }
    let expected = profile
        .commands()
        .iter()
        .map(|(identity, _)| *identity)
        .collect::<BTreeSet<_>>();
    if identities != expected {
        return Err(format!(
            "Native bundle command membership does not match profile {}.",
            profile.id()
        ));
    }
    let has_agent = identities.contains("ait-agent");
    let has_worker = identities.contains("ait-agent-worker");
    if has_agent != has_worker {
        return Err(
            "Native bundle must include ait-agent and ait-agent-worker as an inseparable pair."
                .to_string(),
        );
    }
    Ok(())
}

fn native_license_entries(
    bundle: &ReleaseBundle,
    profile: &ReleaseProfile,
) -> Result<BTreeMap<String, (Vec<u8>, u32)>, String> {
    let mut entries = BTreeMap::new();
    for path in profile.license_files {
        validate_archive_path(path)?;
        let entry = bundle.files.get(*path).ok_or_else(|| {
            format!("Native bundle source is missing required license or notice file {path}.")
        })?;
        entries.insert((*path).to_string(), (entry.data.clone(), 0o644));
    }
    Ok(entries)
}

fn canonical_bundle_content_digest(
    commands: &[NativeCommandInput],
    licenses: &BTreeMap<String, (Vec<u8>, u32)>,
) -> Result<String, String> {
    let command_rows = commands
        .iter()
        .map(NativeCommandInput::to_manifest_json)
        .collect::<Vec<_>>();
    let license_rows = licenses
        .iter()
        .map(|(path, (data, mode))| {
            json!({
                "path": path,
                "sha256": sha256_hex(data),
                "size_bytes": data.len(),
                "mode": format!("{:04o}", mode),
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({
        "commands": command_rows,
        "licenses_and_notices": license_rows,
    });
    let bytes = encode_value_pretty_to_vec(
        &payload,
        "failed to encode native bundle content digest projection",
    )?;
    Ok(sha256_hex(&bytes))
}

#[allow(clippy::too_many_arguments)]
fn build_native_bundle_archive(
    repo: &RepoRuntime,
    record: &JsonValue,
    target: &NativeTarget,
    profile: NativeCommandProfile,
    commands: &[NativeCommandInput],
    licenses: &BTreeMap<String, (Vec<u8>, u32)>,
    dist_dir: &Path,
    epoch: i64,
) -> Result<NativeBundleBuild, String> {
    let version = required_string_field(record, "version")?;
    let release_id = required_string_field(record, "release_id")?;
    let snapshot_id = required_string_field(record, "snapshot_id")?;
    let filename = format!("ait-{version}-{}-{}.tar.gz", profile.id(), target.triple);
    validate_archive_path(&filename)?;
    let content_sha256 = canonical_bundle_content_digest(commands, licenses)?;
    let command_rows = commands
        .iter()
        .map(NativeCommandInput::to_manifest_json)
        .collect::<Vec<_>>();
    let manifest = json!({
        "contract": NATIVE_BUNDLE_CONTRACT,
        "schema_version": 1,
        "product": "ait",
        "version": version,
        "release_id": release_id,
        "source_snapshot_id": snapshot_id,
        "profile": profile.id(),
        "target": target.to_json(),
        "commands": command_rows,
        "agent_pair": {
            "members": ["ait-agent", "ait-agent-worker"],
            "policy": "both_or_neither"
        },
        "bundle": {
            "filename": filename,
            "digest_algorithm": "sha256",
            "content_sha256": content_sha256,
            "content_digest_scope": "canonical ordered command, license, and notice projection",
            "source_date_epoch": epoch.max(0),
            "timestamp_source": "source_snapshot_created_at"
        },
        "provenance": record.get("metadata").and_then(|value| value.get("provenance")).cloned().unwrap_or_else(|| json!([])),
        "launcher_conformance": native_launcher_conformance_contract(),
    });
    let manifest_bytes =
        encode_value_pretty_to_vec(&manifest, "failed to encode native bundle manifest")?;

    let target_path = dist_dir.join(&filename);
    let file = File::create(&target_path).map_err(io_error)?;
    let encoder: GzEncoder<File> = GzBuilder::new()
        .mtime(epoch.max(0) as u32)
        .operating_system(255)
        .write(file, Compression::default());
    let mut tar = TarBuilder::new(encoder);
    let mut archive_entries = BTreeMap::<String, (Vec<u8>, u32)>::new();
    for command in commands {
        archive_entries.insert(
            command.archive_path.clone(),
            (command.data.clone(), command.mode),
        );
    }
    archive_entries.extend(licenses.clone());
    archive_entries.insert(
        NATIVE_BUNDLE_MANIFEST_PATH.to_string(),
        (manifest_bytes, 0o644),
    );
    for (path, (data, mode)) in archive_entries {
        validate_archive_path(&path)?;
        append_tar_bytes(&mut tar, &path, &data, mode, epoch)?;
    }
    let encoder = tar.into_inner().map_err(io_error)?;
    encoder.finish().map_err(io_error)?;

    let mut artifact = artifact_info(repo, &target_path)?;
    let artifact_object = artifact
        .as_object_mut()
        .ok_or_else(|| "native bundle artifact projection must be an object".to_string())?;
    artifact_object.insert("kind".to_string(), json!("native-bundle"));
    artifact_object.insert("target".to_string(), json!(target.triple));
    artifact_object.insert("profile".to_string(), json!(profile.id()));
    artifact_object.insert("version".to_string(), json!(version));
    artifact_object.insert("runtime_authority".to_string(), json!("rust"));
    artifact_object.insert("python_fallback".to_string(), json!(false));
    artifact_object.insert("native_manifest".to_string(), manifest.clone());
    let ait_sha256 = commands
        .iter()
        .find(|command| command.public_identity == "ait")
        .map(|command| sha256_hex(&command.data))
        .ok_or_else(|| "native bundle is missing public command ait".to_string())?;
    artifact_object.insert("ait_sha256".to_string(), json!(ait_sha256));

    Ok(NativeBundleBuild {
        artifact,
        target: target.clone(),
        ait_sha256,
    })
}

fn consumer_projections(version: &str, bundles: &[NativeBundleBuild]) -> Vec<JsonValue> {
    let mut rows = Vec::new();
    for bundle in bundles {
        let bundle_sha256 = string_field(&bundle.artifact, "sha256").unwrap_or_default();
        let bundle_filename = bundle
            .artifact
            .get("native_manifest")
            .and_then(|manifest| manifest.get("bundle"))
            .and_then(|bundle| bundle.get("filename"))
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        for ecosystem in bundle.target.ecosystems() {
            rows.push(json!({
                "ecosystem": ecosystem,
                "target": bundle.target.triple,
                "version": version,
                "public_command": "ait",
                "runtime_authority": "rust",
                "bundle_filename": bundle_filename,
                "bundle_sha256": bundle_sha256,
                "native_command_sha256": bundle.ait_sha256,
                "native_command_path": bundle.target.command_archive_path("ait"),
                "adapter_policy": "thin-launcher-only",
            }));
        }
    }
    rows.sort_by_key(|row| {
        format!(
            "{}/{}",
            string_field(row, "target").unwrap_or_default(),
            string_field(row, "ecosystem").unwrap_or_default()
        )
    });
    rows
}

pub(super) fn build_native_distribution(
    repo: &RepoRuntime,
    record: &JsonValue,
    source_bundle: &ReleaseBundle,
    release_profile: &ReleaseProfile,
    dist_dir: &Path,
    epoch: i64,
    explicit_matrix_root: Option<&Path>,
    explicit_host_source_dir: Option<&Path>,
) -> Result<(Vec<JsonValue>, JsonValue), String> {
    let version = required_string_field(record, "version")?;
    let profile = record
        .get("metadata")
        .and_then(|metadata| metadata.get("native_distribution"))
        .and_then(|distribution| distribution.get("command_profile"))
        .and_then(JsonValue::as_str)
        .map(NativeCommandProfile::parse)
        .transpose()?
        .unwrap_or(NativeCommandProfile::Cli);
    let matrix_root = native_matrix_root(explicit_matrix_root);
    let licenses = native_license_entries(source_bundle, release_profile)?;
    let mut built = Vec::new();
    let mut missing = Vec::new();
    let mut rejected = Vec::new();

    for target in supported_native_targets() {
        let Some((source_dir, descriptor_required)) = target_source_dir(
            repo,
            &target,
            matrix_root.as_deref(),
            explicit_host_source_dir,
        ) else {
            missing.push(target.triple.clone());
            continue;
        };
        if !source_dir.is_dir() {
            missing.push(target.triple.clone());
            continue;
        }
        match source_command_inputs(
            &source_dir,
            descriptor_required,
            &target,
            &version,
            &required_string_field(record, "release_id")?,
            &required_string_field(record, "snapshot_id")?,
            &required_string_field(record, "manifest_hash")?,
            profile,
        )
        .and_then(|commands| {
            build_native_bundle_archive(
                repo, record, &target, profile, &commands, &licenses, dist_dir, epoch,
            )
        }) {
            Ok(bundle) => built.push(bundle),
            Err(reason) => rejected.push(json!({
                "target": target.triple,
                "reason": reason,
            })),
        }
    }
    built.sort_by_key(|bundle| bundle.target.triple.clone());
    missing.sort();
    rejected.sort_by_key(|row| string_field(row, "target").unwrap_or_default());
    let projections = consumer_projections(&version, &built);
    let built_targets = built
        .iter()
        .map(|bundle| bundle.target.triple.clone())
        .collect::<Vec<_>>();
    let ready = built_targets.len() == REQUIRED_TARGET_TRIPLES.len()
        && missing.is_empty()
        && rejected.is_empty();
    let state = if ready {
        "complete"
    } else if built_targets.is_empty() {
        "unbuilt"
    } else {
        "partial"
    };
    let projection = json!({
        "contract": NATIVE_DISTRIBUTION_CONTRACT,
        "matrix_revision": NATIVE_MATRIX_REVISION,
        "state": state,
        "version": version,
        "release_id": required_string_field(record, "release_id")?,
        "source_snapshot_id": required_string_field(record, "snapshot_id")?,
        "command_profile": profile.id(),
        "configured_targets": supported_native_targets().iter().map(NativeTarget::to_json).collect::<Vec<_>>(),
        "required_consumer_ecosystems": REQUIRED_CONSUMER_ECOSYSTEMS,
        "launcher_conformance": native_launcher_conformance_contract(),
        "source_layout": "<matrix-dir>/<rust-target-triple>/release/{ait-cli[.exe],ait-native-source.json}",
        "matrix_source": matrix_root.as_ref().map(|path| path.to_string_lossy().to_string()).unwrap_or_else(|| "host-release-fallback".to_string()),
        "built_targets": built_targets,
        "missing_targets": missing,
        "rejected_targets": rejected,
        "consumer_projections": projections,
        "multi_ecosystem_ready": ready,
    });
    Ok((
        built.into_iter().map(|bundle| bundle.artifact).collect(),
        projection,
    ))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn configured_target_triples(distribution: &JsonValue) -> Result<BTreeSet<String>, String> {
    let rows = distribution
        .get("configured_targets")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "native distribution is missing configured_targets".to_string())?;
    let mut triples = BTreeSet::new();
    for row in rows {
        let triple = required_string_field(row, "triple")?;
        let target = NativeTarget::parse(&triple)?;
        if row != &target.to_json() {
            return Err(format!(
                "configured target {triple} does not match canonical target normalization"
            ));
        }
        if !triples.insert(triple.clone()) {
            return Err(format!("configured target {triple} is duplicated"));
        }
    }
    Ok(triples)
}

fn validate_native_bundle_artifact(
    record: &JsonValue,
    artifact: &JsonValue,
) -> Result<(NativeTarget, String), String> {
    let release_id = required_string_field(record, "release_id")?;
    let version = required_string_field(record, "version")?;
    let snapshot_id = required_string_field(record, "snapshot_id")?;
    let target = NativeTarget::parse(&required_string_field(artifact, "target")?)?;
    let profile = NativeCommandProfile::parse(&required_string_field(artifact, "profile")?)?;
    if string_field(artifact, "version").as_deref() != Some(version.as_str()) {
        return Err(format!(
            "{} bundle version disagrees with release",
            target.triple
        ));
    }
    if string_field(artifact, "runtime_authority").as_deref() != Some("rust")
        || bool_field(artifact, "python_fallback")
    {
        return Err(format!(
            "{} bundle must declare Rust authority without fallback",
            target.triple
        ));
    }
    let path = required_string_field(artifact, "path")?;
    validate_archive_path(&path)?;
    if path_references_debug_cargo_target(&path) {
        return Err(format!(
            "{} bundle path references a debug artifact",
            target.triple
        ));
    }
    let archive_sha256 = required_string_field(artifact, "sha256")?;
    if !valid_sha256(&archive_sha256)
        || artifact
            .get("size_bytes")
            .and_then(JsonValue::as_u64)
            .unwrap_or_default()
            == 0
    {
        return Err(format!(
            "{} bundle archive digest or size is invalid",
            target.triple
        ));
    }
    let manifest = artifact
        .get("native_manifest")
        .ok_or_else(|| format!("{} bundle is missing native_manifest", target.triple))?;
    if string_field(manifest, "contract").as_deref() != Some(NATIVE_BUNDLE_CONTRACT)
        || manifest.get("schema_version").and_then(JsonValue::as_u64) != Some(1)
        || string_field(manifest, "product").as_deref() != Some("ait")
        || string_field(manifest, "version").as_deref() != Some(version.as_str())
        || string_field(manifest, "release_id").as_deref() != Some(release_id.as_str())
        || string_field(manifest, "source_snapshot_id").as_deref() != Some(snapshot_id.as_str())
        || string_field(manifest, "profile").as_deref() != Some(profile.id())
        || manifest
            .get("target")
            .and_then(|value| value.get("triple"))
            .and_then(JsonValue::as_str)
            != Some(target.triple.as_str())
        || manifest.get("target") != Some(&target.to_json())
    {
        return Err(format!(
            "{} bundle manifest identity, target, version, or profile disagrees",
            target.triple
        ));
    }
    let filename = manifest
        .get("bundle")
        .and_then(|value| value.get("filename"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    if Path::new(&path).file_name().and_then(OsStr::to_str) != Some(filename)
        || !manifest
            .get("bundle")
            .and_then(|value| value.get("content_sha256"))
            .and_then(JsonValue::as_str)
            .is_some_and(valid_sha256)
        || manifest
            .get("bundle")
            .and_then(|value| value.get("timestamp_source"))
            .and_then(JsonValue::as_str)
            != Some("source_snapshot_created_at")
    {
        return Err(format!(
            "{} bundle filename, content digest, or timestamp source is invalid",
            target.triple
        ));
    }
    if manifest.get("launcher_conformance") != Some(&native_launcher_conformance_contract()) {
        return Err(format!(
            "{} bundle launcher conformance contract drifted",
            target.triple
        ));
    }

    let rows = manifest
        .get("commands")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{} bundle manifest is missing commands", target.triple))?;
    let mut identities = BTreeSet::new();
    let mut ait_sha256 = None;
    for row in rows {
        let identity = required_string_field(row, "public_identity")?;
        if !identities.insert(identity.clone()) {
            return Err(format!(
                "{} bundle repeats command {identity}",
                target.triple
            ));
        }
        let archive_path = required_string_field(row, "archive_path")?;
        validate_archive_path(&archive_path)?;
        if archive_path != target.command_archive_path(&identity) {
            return Err(format!(
                "{} command {identity} must use canonical archive path {}",
                target.triple,
                target.command_archive_path(&identity)
            ));
        }
        let digest = required_string_field(row, "sha256")?;
        if !valid_sha256(&digest)
            || row
                .get("size_bytes")
                .and_then(JsonValue::as_u64)
                .unwrap_or_default()
                == 0
            || string_field(row, "executable_mode").as_deref() != Some("0755")
            || string_field(row, "runtime_authority").as_deref() != Some("rust")
        {
            return Err(format!(
                "{} command {identity} digest, size, mode, or authority is invalid",
                target.triple
            ));
        }
        if identity == "ait" {
            if string_field(row, "source_binary_identity").as_deref() != Some("ait-cli") {
                return Err(format!(
                    "{} public ait command does not identify source binary ait-cli",
                    target.triple
                ));
            }
            ait_sha256 = Some(digest);
        }
    }
    let expected = profile
        .commands()
        .iter()
        .map(|(identity, _)| (*identity).to_string())
        .collect::<BTreeSet<_>>();
    if identities != expected {
        return Err(format!(
            "{} bundle command membership disagrees with profile {}",
            target.triple,
            profile.id()
        ));
    }
    if identities.contains("ait-agent") != identities.contains("ait-agent-worker") {
        return Err(format!("{} bundle has a partial agent pair", target.triple));
    }
    let ait_sha256 =
        ait_sha256.ok_or_else(|| format!("{} bundle is missing ait", target.triple))?;
    if string_field(artifact, "ait_sha256").as_deref() != Some(ait_sha256.as_str()) {
        return Err(format!(
            "{} bundle artifact and manifest disagree on the ait digest",
            target.triple
        ));
    }
    Ok((target, ait_sha256))
}

fn validate_consumer_projections(
    record: &JsonValue,
    distribution: &JsonValue,
    built: &BTreeMap<String, (String, String)>,
) -> Vec<String> {
    let version = string_field(record, "version").unwrap_or_default();
    let rows = distribution
        .get("consumer_projections")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut projected = BTreeSet::new();
    let mut blockers = Vec::new();
    for row in rows {
        let ecosystem = string_field(&row, "ecosystem").unwrap_or_default();
        let target = string_field(&row, "target").unwrap_or_default();
        let key = (target.clone(), ecosystem.clone());
        if !projected.insert(key) {
            blockers.push(format!(
                "consumer projection repeats {ecosystem} for {target}"
            ));
            continue;
        }
        let Some((bundle_sha256, ait_sha256)) = built.get(&target) else {
            blockers.push(format!(
                "consumer projection {ecosystem}/{target} has no built native bundle"
            ));
            continue;
        };
        let target_model = NativeTarget::parse(&target).ok();
        let expected_ecosystem = target_model
            .as_ref()
            .is_some_and(|model| model.ecosystems().contains(&ecosystem.as_str()));
        if !expected_ecosystem
            || string_field(&row, "version").as_deref() != Some(version.as_str())
            || string_field(&row, "public_command").as_deref() != Some("ait")
            || string_field(&row, "runtime_authority").as_deref() != Some("rust")
            || string_field(&row, "bundle_sha256").as_deref() != Some(bundle_sha256.as_str())
            || string_field(&row, "native_command_sha256").as_deref() != Some(ait_sha256.as_str())
            || string_field(&row, "native_command_path").as_deref()
                != target_model
                    .as_ref()
                    .map(|model| model.command_archive_path("ait"))
                    .as_deref()
            || string_field(&row, "adapter_policy").as_deref() != Some("thin-launcher-only")
        {
            blockers.push(format!(
                "consumer projection {ecosystem}/{target} disagrees with the canonical bundle"
            ));
        }
    }
    for target in built.keys() {
        if let Ok(model) = NativeTarget::parse(target) {
            for ecosystem in model.ecosystems() {
                if !projected.contains(&(target.clone(), ecosystem.to_string())) {
                    blockers.push(format!(
                        "consumer projection is missing {ecosystem} for {target}"
                    ));
                }
            }
        }
    }
    blockers
}

pub(super) fn native_distribution_readiness(record: &JsonValue) -> JsonValue {
    let Some(distribution) = record
        .get("metadata")
        .and_then(|metadata| metadata.get("native_distribution"))
    else {
        return json!({
            "contract": NATIVE_DISTRIBUTION_CONTRACT,
            "state": "legacy_unconfigured",
            "migration_state": "historical record remains readable but is not multi-ecosystem ready",
            "configured_count": 0,
            "built_count": 0,
            "missing_targets": REQUIRED_TARGET_TRIPLES,
            "rejected_targets": [],
            "blockers": ["release record predates the target-aware native distribution contract"],
            "multi_ecosystem_ready": false,
        });
    };
    let expected = REQUIRED_TARGET_TRIPLES
        .iter()
        .map(|value| (*value).to_string())
        .collect::<BTreeSet<_>>();
    let mut blockers = Vec::new();
    if string_field(distribution, "contract").as_deref() != Some(NATIVE_DISTRIBUTION_CONTRACT)
        || string_field(distribution, "matrix_revision").as_deref() != Some(NATIVE_MATRIX_REVISION)
        || string_field(distribution, "version").as_deref()
            != string_field(record, "version").as_deref()
        || string_field(distribution, "release_id").as_deref()
            != string_field(record, "release_id").as_deref()
        || string_field(distribution, "source_snapshot_id").as_deref()
            != string_field(record, "snapshot_id").as_deref()
        || distribution.get("launcher_conformance") != Some(&native_launcher_conformance_contract())
    {
        blockers.push("native distribution identity or launcher contract drifted".to_string());
    }
    let configured = match configured_target_triples(distribution) {
        Ok(configured) => configured,
        Err(reason) => {
            blockers.push(reason);
            BTreeSet::new()
        }
    };
    if configured != expected {
        blockers.push(format!(
            "configured native target matrix must be exactly {}",
            REQUIRED_TARGET_TRIPLES.join(", ")
        ));
    }

    let mut built = BTreeMap::<String, (String, String)>::new();
    let artifacts = record
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    for artifact in artifacts
        .iter()
        .filter(|row| string_field(row, "kind").as_deref() == Some("native-bundle"))
    {
        match validate_native_bundle_artifact(record, artifact) {
            Ok((target, ait_sha256)) => {
                let bundle_sha256 = string_field(artifact, "sha256").unwrap_or_default();
                if built
                    .insert(target.triple.clone(), (bundle_sha256, ait_sha256))
                    .is_some()
                {
                    blockers.push(format!(
                        "native bundle target {} is duplicated",
                        target.triple
                    ));
                }
            }
            Err(reason) => blockers.push(reason),
        }
    }
    let built_targets = built.keys().cloned().collect::<BTreeSet<_>>();
    let missing_targets = expected
        .difference(&built_targets)
        .cloned()
        .collect::<Vec<_>>();
    let unexpected_targets = built_targets
        .difference(&expected)
        .cloned()
        .collect::<Vec<_>>();
    if !missing_targets.is_empty() {
        blockers.push(format!(
            "native target matrix is missing {}",
            missing_targets.join(", ")
        ));
    }
    if !unexpected_targets.is_empty() {
        blockers.push(format!(
            "native target matrix contains unexpected targets {}",
            unexpected_targets.join(", ")
        ));
    }
    blockers.extend(validate_consumer_projections(record, distribution, &built));
    let recorded_rejections = distribution
        .get("rejected_targets")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if !recorded_rejections.is_empty() {
        blockers.push(format!(
            "{} configured target builds were rejected",
            recorded_rejections.len()
        ));
    }
    blockers.sort();
    blockers.dedup();
    let ready = blockers.is_empty() && built_targets == expected;
    json!({
        "contract": NATIVE_DISTRIBUTION_CONTRACT,
        "matrix_revision": NATIVE_MATRIX_REVISION,
        "state": if ready { "complete" } else if built_targets.is_empty() { "configured_unbuilt" } else { "partial" },
        "migration_state": "target-aware",
        "configured_count": configured.len(),
        "configured_targets": configured.into_iter().collect::<Vec<_>>(),
        "built_count": built_targets.len(),
        "built_targets": built_targets.into_iter().collect::<Vec<_>>(),
        "missing_targets": missing_targets,
        "rejected_targets": recorded_rejections,
        "blockers": blockers,
        "multi_ecosystem_ready": ready,
    })
}

pub(super) fn assert_native_distribution_publish_ready(record: &JsonValue) -> Result<(), String> {
    let release_id = required_string_field(record, "release_id")?;
    let readiness = native_distribution_readiness(record);
    if bool_field(&readiness, "multi_ecosystem_ready") {
        return Ok(());
    }
    let blockers = readiness
        .get("blockers")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .collect::<Vec<_>>();
    Err(format!(
        "Release {release_id} is not native multi-ecosystem ready: {}. Supply the complete matrix with `ait release native-bundle {release_id} --native-matrix-dir <dir>`.",
        if blockers.is_empty() {
            "target matrix is incomplete".to_string()
        } else {
            blockers.join("; ")
        }
    ))
}

pub(super) fn native_distribution_contract_check(record: &JsonValue) -> JsonValue {
    let readiness = native_distribution_readiness(record);
    let state = string_field(&readiness, "state").unwrap_or_default();
    let ready = bool_field(&readiness, "multi_ecosystem_ready");
    let built_count = readiness
        .get("built_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default();
    let configured_count = readiness
        .get("configured_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default();
    let blockers = readiness
        .get("blockers")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .collect::<Vec<_>>();
    if ready {
        return check_result(
            "native_distribution_matrix",
            "Target-aware native distribution matrix is complete and conformant",
            "pass",
            format!("All {configured_count} configured native targets are ready."),
            false,
        );
    }
    if state == "configured_unbuilt" && configured_count == REQUIRED_TARGET_TRIPLES.len() as u64 {
        return check_result(
            "native_distribution_matrix",
            "Target-aware native distribution matrix is configured",
            "warn",
            format!(
                "The {configured_count}-target matrix is configured but unbuilt. Run `ait release native-bundle {} --native-matrix-dir <dir>`.",
                string_field(record, "release_id").unwrap_or_default()
            ),
            false,
        );
    }
    check_result(
        "native_distribution_matrix",
        "Target-aware native distribution matrix is complete and conformant",
        "fail",
        format!(
            "Built {built_count}/{configured_count} targets; {}",
            blockers.join("; ")
        ),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_repo(root: &Path) -> RepoRuntime {
        RepoRuntime {
            root: root.to_path_buf(),
            ait_dir: root.join(".ait"),
            config: JsonMap::from_iter([("repo_name".to_string(), json!("ait-core"))]),
            worktree_config_path: None,
        }
    }

    fn test_source_bundle(profile: &ReleaseProfile) -> ReleaseBundle {
        let files = profile
            .license_files
            .iter()
            .map(|path| {
                (
                    (*path).to_string(),
                    BundleEntry {
                        path: (*path).to_string(),
                        data: format!("fixture {path}\n").into_bytes(),
                        mode: "0644".to_string(),
                    },
                )
            })
            .collect();
        ReleaseBundle {
            raw: json!({"created_at": "2026-07-19T00:00:00Z"}),
            files,
        }
    }

    fn test_record() -> JsonValue {
        let distribution = native_distribution_candidate_contract("1.2.3", "REL-1", "SNP-1");
        json!({
            "release_id": "REL-1",
            "repo_name": "ait-core",
            "version": "1.2.3",
            "snapshot_id": "SNP-1",
            "manifest_hash": "MANIFEST-1",
            "profile": "local-cli",
            "artifacts": [],
            "metadata": {"native_distribution": distribution},
        })
    }

    fn write_target_source(root: &Path, target: &NativeTarget, version: &str) {
        let source_dir = root.join(&target.triple).join("release");
        fs::create_dir_all(&source_dir).unwrap();
        let data = format!("native ait {} {version}\n", target.triple).into_bytes();
        let command_path = source_dir.join(format!("ait-cli{}", target.executable_suffix));
        fs::write(&command_path, &data).unwrap();
        set_filesystem_mode(&command_path, 0o755).unwrap();
        let descriptor = json!({
            "contract": NATIVE_SOURCE_CONTRACT,
            "schema_version": 1,
            "version": version,
            "release_id": "REL-1",
            "source_snapshot_id": "SNP-1",
            "source_manifest_hash": "MANIFEST-1",
            "external_closure_sha256": JsonValue::Null,
            "target": target.triple,
            "cargo_profile": "release",
            "command_profile": "cli",
            "commands": [{
                "public_identity": "ait",
                "source_binary_identity": "ait-cli",
                "sha256": sha256_hex(&data),
                "size_bytes": data.len(),
                "executable_mode": "0755"
            }],
            "runner": {
                "label": "fixture-runner",
                "image": "fixture-image-v1",
                "os": target.os,
                "architecture": target.architecture,
                "rustc": {
                    "path": "/toolchains/1.96.0/bin/rustc",
                    "requested_toolchain": "1.96.0",
                    "observed_version": "rustc 1.96.0 (fixture)",
                    "exit_code": 0,
                    "stdout_sha256": sha256_hex(b"rustc stdout"),
                    "stderr_sha256": sha256_hex(b""),
                }
            },
            "smoke": {
                "contract": NATIVE_SMOKE_CONTRACT,
                "target": target.triple,
                "native_execution": true,
                "passed": true,
                "checks": [
                    fixture_smoke_check("ait.version", false),
                    fixture_smoke_check("ait.help", false),
                    fixture_smoke_check("ait.init", true),
                    fixture_smoke_check("ait.status", true),
                    fixture_smoke_check("ait.plan-list", true),
                ],
                "summary": {"check_count": 5, "failed_count": 0}
            }
        });
        fs::write(
            source_dir.join("ait-native-source.json"),
            encode_value_pretty_with_newline_error_string(&descriptor).unwrap(),
        )
        .unwrap();
    }

    fn fixture_smoke_check(check_id: &str, json_stdout: bool) -> JsonValue {
        json!({
            "check_id": check_id,
            "argv": [],
            "exit_code": 0,
            "stdout_channel_captured": true,
            "stderr_channel_captured": true,
            "stdout_nonempty": true,
            "stderr_nonempty": false,
            "stdout_json_valid": json_stdout,
            "stdout_identity_valid": check_id.ends_with(".version"),
            "passed": true,
        })
    }

    fn build_complete_fixture(root: &Path) -> (JsonValue, Vec<JsonValue>) {
        let repo = test_repo(root);
        let profile = require_profile("local-cli").unwrap();
        let source_bundle = test_source_bundle(&profile);
        let matrix_root = root.join("matrix");
        let dist = root.join("dist");
        fs::create_dir_all(&dist).unwrap();
        for target in supported_native_targets() {
            write_target_source(&matrix_root, &target, "1.2.3");
        }
        let mut record = test_record();
        let (artifacts, projection) = build_native_distribution(
            &repo,
            &record,
            &source_bundle,
            &profile,
            &dist,
            1_784_438_400,
            Some(&matrix_root),
            None,
        )
        .unwrap();
        record["artifacts"] = JsonValue::Array(artifacts.clone());
        record["metadata"]["native_distribution"] = projection;
        (record, artifacts)
    }

    #[test]
    fn target_normalization_is_closed_over_the_six_target_release_matrix() {
        let targets = supported_native_targets();
        assert_eq!(targets.len(), 6);
        assert_eq!(targets[0].architecture, "arm64");
        assert_eq!(targets[0].os, "macos");
        assert_eq!(targets[2].libc.as_deref(), Some("gnu"));
        assert_eq!(targets[4].triple, "aarch64-pc-windows-msvc");
        assert_eq!(targets[4].os, "windows");
        assert_eq!(targets[4].architecture, "arm64");
        assert_eq!(targets[4].libc, None);
        assert_eq!(targets[4].executable_suffix, ".exe");
        assert_eq!(targets[5].triple, "x86_64-pc-windows-msvc");
        assert_eq!(targets[5].command_archive_path("ait"), "bin/ait.exe");
        assert!(NativeTarget::parse("x86_64-unknown-linux-musl")
            .unwrap_err()
            .contains("Unsupported native release target"));
        assert!(NativeTarget::parse("x86_64-pc-windows-gnu")
            .unwrap_err()
            .contains("Unsupported native release target"));
    }

    #[test]
    fn candidate_contract_configures_matrix_and_fail_closed_launcher_rules() {
        let contract = native_distribution_candidate_contract("1.2.3", "REL-1", "SNP-1");
        assert_eq!(contract["configured_targets"].as_array().unwrap().len(), 6);
        assert_eq!(contract["command_profile"], json!("cli"));
        assert_eq!(contract["multi_ecosystem_ready"], json!(false));
        let forbidden = contract["launcher_conformance"]["forbidden"]
            .as_array()
            .unwrap();
        assert!(forbidden.contains(&json!("first_invocation_download")));
        assert!(forbidden.contains(&json!("path_substitution")));
        assert!(forbidden.contains(&json!("subcommand_parsing")));
        assert!(forbidden.contains(&json!("python_or_node_workflow_fallback")));
    }

    #[cfg(unix)]
    #[test]
    fn windows_source_executability_uses_exe_identity_not_posix_mode_bits() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo = test_repo(temp.path());
        let profile = require_profile("local-cli").unwrap();
        let source_bundle = test_source_bundle(&profile);
        let matrix_root = temp.path().join("matrix");
        let dist = temp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();
        let target = NativeTarget::parse("x86_64-pc-windows-msvc").unwrap();
        write_target_source(&matrix_root, &target, "1.2.3");
        set_filesystem_mode(
            &matrix_root
                .join(&target.triple)
                .join("release")
                .join("ait-cli.exe"),
            0o644,
        )
        .unwrap();

        let (artifacts, projection) = build_native_distribution(
            &repo,
            &test_record(),
            &source_bundle,
            &profile,
            &dist,
            1_784_438_400,
            Some(&matrix_root),
            None,
        )
        .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0]["target"], json!("x86_64-pc-windows-msvc"));
        assert_eq!(
            artifacts[0]["native_manifest"]["commands"][0]["archive_path"],
            json!("bin/ait.exe")
        );
        assert!(projection["rejected_targets"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn complete_matrix_build_is_deterministic_and_consumer_digests_converge() {
        let temp = tempfile::TempDir::new().unwrap();
        let (record, first_artifacts) = build_complete_fixture(temp.path());
        let first_bytes = first_artifacts
            .iter()
            .map(|artifact| {
                let path = PathBuf::from(artifact["absolute_path"].as_str().unwrap());
                (
                    artifact["target"].as_str().unwrap().to_string(),
                    fs::read(path).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let archive_bytes = first_bytes.values().next().unwrap();
        let decoder = flate2::read::GzDecoder::new(archive_bytes.as_slice());
        let mut archive = tar::Archive::new(decoder);
        let archive_entries = archive
            .entries()
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                assert!(entry.header().entry_type().is_file());
                (
                    entry.path().unwrap().to_string_lossy().to_string(),
                    entry.header().mode().unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(archive_entries.len(), 5);
        assert_eq!(archive_entries.get("bin/ait"), Some(&0o755));
        assert_eq!(
            archive_entries.get(NATIVE_BUNDLE_MANIFEST_PATH),
            Some(&0o644)
        );
        let (second_record, second_artifacts) = build_complete_fixture(temp.path());
        let second_bytes = second_artifacts
            .iter()
            .map(|artifact| {
                let path = PathBuf::from(artifact["absolute_path"].as_str().unwrap());
                (
                    artifact["target"].as_str().unwrap().to_string(),
                    fs::read(path).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(first_bytes, second_bytes);
        assert_eq!(record["artifacts"], second_record["artifacts"]);

        let readiness = native_distribution_readiness(&record);
        assert_eq!(readiness["multi_ecosystem_ready"], json!(true));
        assert_eq!(readiness["built_count"], json!(6));
        assert_native_distribution_publish_ready(&record).unwrap();

        let projections = record["metadata"]["native_distribution"]["consumer_projections"]
            .as_array()
            .unwrap();
        let linux_rows = projections
            .iter()
            .filter(|row| row["target"] == json!("x86_64-unknown-linux-gnu"))
            .collect::<Vec<_>>();
        assert_eq!(linux_rows.len(), 4);
        let ecosystems = linux_rows
            .iter()
            .filter_map(|row| row["ecosystem"].as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            ecosystems,
            BTreeSet::from(["apt", "homebrew", "npm", "pip"])
        );
        assert_eq!(
            linux_rows
                .iter()
                .filter_map(|row| row["native_command_sha256"].as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            1
        );

        let windows_artifact = first_artifacts
            .iter()
            .find(|artifact| artifact["target"] == json!("x86_64-pc-windows-msvc"))
            .unwrap();
        assert_eq!(
            windows_artifact["native_manifest"]["commands"][0]["archive_path"],
            json!("bin/ait.exe")
        );
        let windows_bytes = fs::read(PathBuf::from(
            windows_artifact["absolute_path"].as_str().unwrap(),
        ))
        .unwrap();
        let decoder = flate2::read::GzDecoder::new(windows_bytes.as_slice());
        let mut archive = tar::Archive::new(decoder);
        let windows_entries = archive
            .entries()
            .unwrap()
            .map(|entry| entry.unwrap().path().unwrap().to_string_lossy().to_string())
            .collect::<BTreeSet<_>>();
        assert!(windows_entries.contains("bin/ait.exe"));
        assert!(!windows_entries.contains("bin/ait"));

        let windows_rows = projections
            .iter()
            .filter(|row| row["target"] == json!("x86_64-pc-windows-msvc"))
            .collect::<Vec<_>>();
        assert_eq!(windows_rows.len(), 2);
        assert_eq!(
            windows_rows
                .iter()
                .filter_map(|row| row["ecosystem"].as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["npm", "pip"])
        );
        assert!(windows_rows
            .iter()
            .all(|row| row["native_command_path"] == json!("bin/ait.exe")));

        let fixture = parse_value(
            include_str!("../../tests/fixtures/native_distribution_consumer_matrix.json"),
            "native distribution consumer fixture must be valid JSON",
        )
        .unwrap();
        for fixture_target in fixture["targets"].as_array().unwrap() {
            let target = fixture_target["target"].as_str().unwrap();
            let expected_ecosystems = fixture_target["ecosystems"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(JsonValue::as_str)
                .collect::<BTreeSet<_>>();
            let actual_ecosystems = projections
                .iter()
                .filter(|row| row["target"] == json!(target))
                .filter_map(|row| row["ecosystem"].as_str())
                .collect::<BTreeSet<_>>();
            assert_eq!(actual_ecosystems, expected_ecosystems);
            assert!(projections
                .iter()
                .filter(|row| row["target"] == json!(target))
                .all(|row| row["native_command_path"] == fixture_target["native_command_path"]));
        }
    }

    #[test]
    fn complete_native_matrix_satisfies_the_integrated_publish_gate() {
        let temp = tempfile::TempDir::new().unwrap();
        let (mut record, _) = build_complete_fixture(temp.path());
        let mut artifacts = record["artifacts"].as_array().unwrap().clone();
        artifacts.extend([
            json!({"kind": "sdist", "path": "dist/ait-1.2.3.tar.gz"}),
            json!({"kind": "wheel", "path": "dist/ait-1.2.3.whl"}),
            json!({"kind": "manifest", "path": "dist/ait-release-1.2.3.manifest.json"}),
            json!({"kind": "checksum", "path": "dist/ait-release-1.2.3.sha256"}),
            json!({
                "kind": "native-command",
                "command": "ait-agent",
                "runtime_authority": "rust",
                "python_fallback": false,
                "cargo_profile": "release",
                "path": "dist/ait-agent-1.2.3"
            }),
            json!({
                "kind": "native-command",
                "command": "ait-agent-worker",
                "runtime_authority": "rust",
                "python_fallback": false,
                "cargo_profile": "release",
                "path": "dist/ait-agent-worker-1.2.3"
            }),
        ]);
        record["artifacts"] = JsonValue::Array(artifacts);
        record["checks"] = json!([{"check_id": "fixture", "blocking": false}]);
        record["metadata"]["build"] = json!({
            "rust_release_profile": rust_release_profile_contract(),
            "rust_ci_profile": rust_ci_profile_contract()
        });

        assert_publish_ready(&record).unwrap();
    }

    #[test]
    fn matrix_build_rejects_native_source_release_identity_drift() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo = test_repo(temp.path());
        let profile = require_profile("local-cli").unwrap();
        let source_bundle = test_source_bundle(&profile);
        let matrix_root = temp.path().join("matrix");
        let dist = temp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();
        let target = supported_native_targets().remove(0);
        write_target_source(&matrix_root, &target, "1.2.3");
        let descriptor_path = matrix_root
            .join(&target.triple)
            .join("release")
            .join("ait-native-source.json");
        let mut descriptor =
            parse_slice_value(&fs::read(&descriptor_path).unwrap(), "fixture descriptor").unwrap();
        descriptor["release_id"] = json!("REL-WRONG");
        fs::write(
            descriptor_path,
            encode_value_pretty_with_newline_error_string(&descriptor).unwrap(),
        )
        .unwrap();

        let (artifacts, projection) = build_native_distribution(
            &repo,
            &test_record(),
            &source_bundle,
            &profile,
            &dist,
            1_784_438_400,
            Some(&matrix_root),
            None,
        )
        .unwrap();

        assert!(artifacts.is_empty());
        assert_eq!(projection["rejected_targets"].as_array().unwrap().len(), 1);
        assert!(projection["rejected_targets"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("release or source Snapshot identity disagrees"));
    }

    #[test]
    fn publish_payload_rechecks_artifact_bytes_against_recorded_digest_and_size() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo = test_repo(temp.path());
        fs::create_dir_all(temp.path().join("dist")).unwrap();
        fs::write(temp.path().join("dist/native.tar.gz"), b"changed").unwrap();
        let record = json!({
            "artifacts": [{
                "kind": "native-bundle",
                "path": "dist/native.tar.gz",
                "sha256": sha256_hex(b"original"),
                "size_bytes": 8
            }]
        });

        let error = release_publish_artifacts(&repo, &record).unwrap_err();
        assert!(error.contains("digest or size changed after build"));
    }

    #[test]
    fn partial_matrix_is_diagnostic_but_publish_is_blocked() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo = test_repo(temp.path());
        let profile = require_profile("local-cli").unwrap();
        let source_bundle = test_source_bundle(&profile);
        let matrix_root = temp.path().join("matrix");
        let dist = temp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();
        write_target_source(&matrix_root, &supported_native_targets()[0], "1.2.3");
        let mut record = test_record();
        let (artifacts, projection) = build_native_distribution(
            &repo,
            &record,
            &source_bundle,
            &profile,
            &dist,
            1_784_438_400,
            Some(&matrix_root),
            None,
        )
        .unwrap();
        record["artifacts"] = JsonValue::Array(artifacts);
        record["metadata"]["native_distribution"] = projection;

        let readiness = native_distribution_readiness(&record);
        assert_eq!(readiness["state"], json!("partial"));
        assert_eq!(readiness["built_count"], json!(1));
        assert_eq!(readiness["missing_targets"].as_array().unwrap().len(), 5);
        let error = assert_native_distribution_publish_ready(&record).unwrap_err();
        assert!(error.contains("not native multi-ecosystem ready"));
        assert!(error.contains("--native-matrix-dir"));
    }

    #[test]
    fn mismatched_source_descriptor_is_rejected_per_target() {
        let temp = tempfile::TempDir::new().unwrap();
        let repo = test_repo(temp.path());
        let profile = require_profile("local-cli").unwrap();
        let source_bundle = test_source_bundle(&profile);
        let matrix_root = temp.path().join("matrix");
        let dist = temp.path().join("dist");
        fs::create_dir_all(&dist).unwrap();
        let target = supported_native_targets()[0].clone();
        write_target_source(&matrix_root, &target, "9.9.9");

        let (_, projection) = build_native_distribution(
            &repo,
            &test_record(),
            &source_bundle,
            &profile,
            &dist,
            1_784_438_400,
            Some(&matrix_root),
            None,
        )
        .unwrap();
        let rejected = projection["rejected_targets"].as_array().unwrap();
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0]["reason"]
            .as_str()
            .unwrap()
            .contains("source version does not match"));
    }

    #[test]
    fn malformed_paths_partial_agent_pairs_and_manifest_modes_fail_closed() {
        assert!(validate_archive_path("../bin/ait").is_err());
        assert!(validate_archive_path("/bin/ait").is_err());
        assert!(validate_archive_path("bin\\ait").is_err());

        let partial = vec![
            NativeCommandInput {
                public_identity: "ait".to_string(),
                source_binary_identity: "ait-cli".to_string(),
                archive_path: "bin/ait".to_string(),
                data: b"ait".to_vec(),
                mode: 0o755,
            },
            NativeCommandInput {
                public_identity: "ait-agent".to_string(),
                source_binary_identity: "ait-agent".to_string(),
                archive_path: "bin/ait-agent".to_string(),
                data: b"agent".to_vec(),
                mode: 0o755,
            },
        ];
        assert!(
            validate_command_membership(&partial, NativeCommandProfile::CliWithAgent)
                .unwrap_err()
                .contains("command membership")
        );

        let temp = tempfile::TempDir::new().unwrap();
        let (mut record, _) = build_complete_fixture(temp.path());
        record["artifacts"][0]["native_manifest"]["commands"][0]["executable_mode"] = json!("0644");
        let readiness = native_distribution_readiness(&record);
        assert_eq!(readiness["multi_ecosystem_ready"], json!(false));
        assert!(readiness["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row.as_str().unwrap_or_default().contains("mode")));

        let (mut wrong_windows_path, _) = build_complete_fixture(temp.path());
        let windows_index = wrong_windows_path["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .position(|artifact| artifact["target"] == json!("x86_64-pc-windows-msvc"))
            .unwrap();
        wrong_windows_path["artifacts"][windows_index]["native_manifest"]["commands"][0]
            ["archive_path"] = json!("bin/ait");
        let readiness = native_distribution_readiness(&wrong_windows_path);
        assert_eq!(readiness["multi_ecosystem_ready"], json!(false));
        assert!(readiness["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row
                .as_str()
                .unwrap_or_default()
                .contains("canonical archive path bin/ait.exe")));
    }

    #[test]
    fn legacy_release_records_remain_readable_but_never_claim_matrix_readiness() {
        let record = json!({
            "release_id": "REL-old",
            "version": "0.9.0",
            "snapshot_id": "SNP-old",
            "artifacts": []
        });
        let readiness = native_distribution_readiness(&record);
        assert_eq!(readiness["state"], json!("legacy_unconfigured"));
        assert_eq!(readiness["multi_ecosystem_ready"], json!(false));
        assert!(assert_native_distribution_publish_ready(&record)
            .unwrap_err()
            .contains("predates"));
    }

    #[test]
    fn prior_unix_only_records_remain_readable_but_require_windows_rebuild() {
        let temp = tempfile::TempDir::new().unwrap();
        let (mut record, _) = build_complete_fixture(temp.path());
        record["metadata"]["native_distribution"]["matrix_revision"] =
            json!("unix-foundation-2026-07-19.1");
        record["metadata"]["native_distribution"]["configured_targets"] = JsonValue::Array(
            record["metadata"]["native_distribution"]["configured_targets"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|target| target["os"] != json!("windows"))
                .cloned()
                .collect(),
        );
        record["metadata"]["native_distribution"]["consumer_projections"] = JsonValue::Array(
            record["metadata"]["native_distribution"]["consumer_projections"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| !row["target"].as_str().unwrap().contains("windows"))
                .cloned()
                .collect(),
        );
        record["artifacts"] = JsonValue::Array(
            record["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|artifact| !artifact["target"].as_str().unwrap().contains("windows"))
                .cloned()
                .collect(),
        );

        let readiness = native_distribution_readiness(&record);
        assert_eq!(readiness["state"], json!("partial"));
        assert_eq!(readiness["configured_count"], json!(4));
        assert_eq!(readiness["built_count"], json!(4));
        assert_eq!(readiness["multi_ecosystem_ready"], json!(false));
        assert_eq!(
            readiness["missing_targets"],
            json!(["aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc"])
        );
        assert!(readiness["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row
                .as_str()
                .unwrap_or_default()
                .contains("identity or launcher contract drifted")));
    }
}
