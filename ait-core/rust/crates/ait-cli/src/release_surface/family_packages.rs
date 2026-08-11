use super::*;
use flate2::read::GzDecoder;
use std::io::{Cursor, Read};
use tar::Archive as TarArchive;
use zip::ZipArchive;

const FAMILY_PACKAGE_CONTRACT: &str = "ait.release.family.package/v1";
const FAMILY_PACKAGE_CONTENT_CONTRACT: &str = "ait.release.family.package-content/v1";
const PACKAGE_RECEIPT_FILENAME: &str = "ait-release.package.json";
const PACKAGE_CHECKSUM_FILENAME: &str = "SHA256SUMS";
const WINGET_MANIFEST_VERSION: &str = "1.12.0";
const AIT_SERVER_SYSTEMD_UNIT_PATH: &str = "usr/lib/systemd/system/ait-server.service";
const AIT_SERVER_SYSTEMD_UNIT: &str = "[Unit]\nDescription=AIT native server\nDocumentation=https://github.com/weita2026/ait-native\nAfter=network.target\n\n[Service]\nType=simple\nDynamicUser=yes\nStateDirectory=ait-native\nRuntimeDirectory=ait-native\nUMask=0077\nExecStart=/usr/bin/ait-server run --data /var/lib/ait-native/server-data --init-if-missing --defer-ci-admission\nRestart=on-failure\nRestartSec=2s\nNoNewPrivileges=yes\nPrivateTmp=yes\nProtectSystem=strict\nProtectHome=yes\nProtectControlGroups=yes\nProtectKernelModules=yes\nProtectKernelTunables=yes\nRestrictAddressFamilies=AF_UNIX AF_INET AF_INET6\nRestrictSUIDSGID=yes\nLockPersonality=yes\nCapabilityBoundingSet=\nAmbientCapabilities=\n\n[Install]\nWantedBy=multi-user.target\n";
const WINGET_SERVER_CONTROLLER_PATH: &str = "ait-server-control.ps1";
const WINGET_SERVER_CONTROLLER: &str = r#"[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('init', 'probe', 'run', 'start', 'status', 'stop')]
    [string]$Command = 'status',
    [string]$DataRoot = '',
    [string]$Listen = '127.0.0.1:8088'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    throw 'LOCALAPPDATA is required for the ait-server user-session controller.'
}
if ([string]::IsNullOrWhiteSpace($DataRoot)) {
    $DataRoot = Join-Path $env:LOCALAPPDATA 'AIT\server-data'
}
if (-not [System.IO.Path]::IsPathRooted($DataRoot)) {
    throw "ait-server data root must be absolute: $DataRoot"
}

$AdjacentServer = Join-Path $PSScriptRoot 'ait-server.exe'
if (Test-Path -LiteralPath $AdjacentServer -PathType Leaf) {
    $Server = (Resolve-Path -LiteralPath $AdjacentServer).Path
} else {
    $ServerCommand = Get-Command 'ait-server.exe' -CommandType Application -ErrorAction Stop
    $Server = $ServerCommand.Source
}

$RuntimeRoot = Join-Path $env:LOCALAPPDATA 'AIT\runtime'
$StatePath = Join-Path $RuntimeRoot 'ait-server-state.json'
$ControllerLockPath = Join-Path $RuntimeRoot 'ait-server-control.lock'
$StdoutPath = Join-Path $RuntimeRoot 'ait-server.stdout.log'
$StderrPath = Join-Path $RuntimeRoot 'ait-server.stderr.log'
$env:AIT_NATIVE_SERVER_DATA = $DataRoot
$env:AITSERVER_LISTEN = $Listen

function Invoke-AitServer {
    param([string[]]$Arguments)
    & $Server @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "ait-server exited with code $LASTEXITCODE"
    }
}

function Get-ManagedAitServer {
    if (-not (Test-Path -LiteralPath $StatePath -PathType Leaf)) {
        return $null
    }
    $State = Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json
    $RawProcessId = [string]$State.pid
    [int]$ManagedProcessId = 0
    if (-not [int]::TryParse($RawProcessId, [ref]$ManagedProcessId) -or $ManagedProcessId -le 0) {
        throw "Invalid ait-server controller state: $StatePath"
    }
    $ManagedProcess = Get-Process -Id $ManagedProcessId -ErrorAction SilentlyContinue
    if ($null -eq $ManagedProcess) {
        Remove-Item -LiteralPath $StatePath -Force
        return $null
    }
    [long]$RecordedStartTicks = $State.started_at_utc_ticks
    $ActualStartTicks = $ManagedProcess.StartTime.ToUniversalTime().Ticks
    if ($RecordedStartTicks -ne $ActualStartTicks) {
        throw "PID $ManagedProcessId was reused; refusing to manage it."
    }
    $ExpectedPath = [System.IO.Path]::GetFullPath([string]$State.executable_path)
    $ActualPath = [System.IO.Path]::GetFullPath($ManagedProcess.Path)
    if (-not [string]::Equals($ExpectedPath, $ActualPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "PID $ManagedProcessId belongs to another executable; refusing to manage it."
    }
    return $ManagedProcess
}

$ControllerExitCode = 0
$ControllerLock = $null
if ($Command -in @('start', 'status', 'stop')) {
    New-Item -ItemType Directory -Path $RuntimeRoot -Force | Out-Null
    try {
        $ControllerLock = [System.IO.File]::Open(
            $ControllerLockPath,
            [System.IO.FileMode]::OpenOrCreate,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
    } catch {
        throw "Another ait-server controller operation is active: $ControllerLockPath"
    }
}

try {
switch ($Command) {
    'init' {
        Invoke-AitServer @('init')
    }
    'probe' {
        Invoke-AitServer @('probe', '--defer-ci-admission')
    }
    'run' {
        Invoke-AitServer @('run', '--init-if-missing', '--defer-ci-admission')
    }
    'start' {
        if ($null -ne (Get-ManagedAitServer)) {
            throw "ait-server is already active; state: $StatePath"
        }
        Invoke-AitServer @('init')
        $Started = Start-Process -FilePath $Server `
            -ArgumentList @('run', '--init-if-missing', '--defer-ci-admission') `
            -RedirectStandardOutput $StdoutPath `
            -RedirectStandardError $StderrPath `
            -WindowStyle Hidden `
            -PassThru
        Start-Sleep -Milliseconds 500
        $Started.Refresh()
        if ($Started.HasExited) {
            throw "ait-server exited during startup; inspect $StderrPath"
        }
        $State = [ordered]@{
            pid = [int]$Started.Id
            started_at_utc_ticks = [long]$Started.StartTime.ToUniversalTime().Ticks
            executable_path = [string]$Started.Path
        } | ConvertTo-Json -Compress
        [System.IO.File]::WriteAllText(
            $StatePath,
            $State,
            [System.Text.UTF8Encoding]::new($false)
        )
        Write-Output "ait-server started: pid=$($Started.Id) listen=$Listen data=$DataRoot"
    }
    'status' {
        $Managed = Get-ManagedAitServer
        if ($null -eq $Managed) {
            Write-Output "ait-server inactive: data=$DataRoot"
            $ControllerExitCode = 3
        } else {
            Write-Output "ait-server active: pid=$($Managed.Id) listen=$Listen data=$DataRoot"
        }
    }
    'stop' {
        $Managed = Get-ManagedAitServer
        if ($null -eq $Managed) {
            Write-Output 'ait-server already inactive'
        } else {
            Stop-Process -Id $Managed.Id
            $Deadline = [DateTime]::UtcNow.AddSeconds(15)
            do {
                Start-Sleep -Milliseconds 100
                $Managed.Refresh()
            } while (-not $Managed.HasExited -and [DateTime]::UtcNow -lt $Deadline)
            if (-not $Managed.HasExited) {
                throw "ait-server PID $($Managed.Id) did not stop within 15 seconds"
            }
            Remove-Item -LiteralPath $StatePath -Force -ErrorAction SilentlyContinue
            Write-Output 'ait-server stopped'
        }
    }
}
} finally {
    if ($null -ne $ControllerLock) {
        $ControllerLock.Dispose()
    }
}
exit $ControllerExitCode
"#;
const MAX_REGISTRY_ARCHIVE_ENTRIES: usize = 4_096;
const MAX_REGISTRY_ARCHIVE_MEMBER_BYTES: u64 = 256 * 1024 * 1024;
const MAX_REGISTRY_ARCHIVE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const REGISTRY_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
];

#[derive(Clone, Debug)]
struct ComponentDefinition {
    id: String,
    source_repository: String,
    source_snapshot: String,
    ecosystem: String,
    license: String,
    version: String,
}

#[derive(Clone, Debug)]
struct DistributionDefinition {
    channel: String,
    role: String,
    identity: String,
    components: Vec<String>,
    targets: Vec<String>,
}

#[derive(Clone, Debug)]
struct FrozenComponentArtifact {
    component: String,
    kind: String,
    target: Option<String>,
    path: String,
    sha256: String,
    size_bytes: u64,
}

#[derive(Clone, Debug)]
struct FrozenLicenseMaterial {
    source_repository: String,
    source_snapshot: String,
    material_role: String,
    declared_path: String,
    path: String,
    sha256: String,
    size_bytes: u64,
}

#[derive(Debug)]
struct FamilyPackageInput {
    release_id: String,
    version: String,
    release_channel: String,
    tag: String,
    snapshot_id: String,
    epoch: u64,
    family_manifest_sha256: String,
    frozen_manifest_sha256: String,
    frozen_checksum_sha256: String,
    components: BTreeMap<String, ComponentDefinition>,
    distributions: Vec<DistributionDefinition>,
    artifacts: Vec<FrozenComponentArtifact>,
    license_material: Vec<FrozenLicenseMaterial>,
}

#[derive(Clone, Debug)]
struct ContentProjection {
    source: FrozenComponentArtifact,
    destination: String,
}

#[derive(Clone, Debug)]
struct MaterialProjection {
    source: FrozenLicenseMaterial,
    destination: String,
}

#[derive(Debug)]
struct GeneratedArtifact {
    relative_path: String,
    bytes: Vec<u8>,
    evidence: JsonValue,
}

type PackageEntries = BTreeMap<String, (Vec<u8>, u32)>;
type NativeDistributionEntries = (
    PackageEntries,
    Vec<ContentProjection>,
    Vec<MaterialProjection>,
);

fn json_array_strings(value: Option<&JsonValue>, context: &str) -> Result<Vec<String>, String> {
    value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{context} must be an array."))?
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str()
                .filter(|text| !text.is_empty())
                .map(ToString::to_string)
                .ok_or_else(|| format!("{context}[{index}] must be a non-empty string."))
        })
        .collect()
}

fn required_u64(value: &JsonValue, field: &str, context: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| format!("{context} is missing unsigned integer {field}."))
}

fn parse_family_package_input(
    repo: &RepoRuntime,
    release_id: &str,
    channel: &str,
) -> Result<FamilyPackageInput, String> {
    if !matches!(channel, "homebrew" | "apt" | "winget" | "pypi" | "npm") {
        return Err(format!(
            "Unsupported family package channel {channel:?}; expected homebrew, apt, winget, pypi, or npm."
        ));
    }
    let build = super::family_release::validate_existing_family_build(repo, release_id)?;
    let family = build
        .get("family")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Frozen family build is missing its family definition.".to_string())?;
    let component_rows = family
        .get("components")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Frozen family definition is missing components.".to_string())?;
    let mut components = BTreeMap::new();
    for row in component_rows {
        let definition = ComponentDefinition {
            id: required_string_field(row, "id")?,
            source_repository: required_string_field(row, "source_repository")?,
            source_snapshot: required_string_field(row, "source_snapshot")?,
            ecosystem: required_string_field(row, "ecosystem")?,
            license: required_string_field(row, "license")?,
            version: required_string_field(row, "version")?,
        };
        if components
            .insert(definition.id.clone(), definition)
            .is_some()
        {
            return Err("Frozen family definition contains duplicate components.".to_string());
        }
    }

    let distribution_rows = family
        .get("distributions")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Frozen family definition is missing distributions.".to_string())?;
    let mut distributions = Vec::new();
    for row in distribution_rows {
        let definition = DistributionDefinition {
            channel: required_string_field(row, "channel")?,
            role: required_string_field(row, "role")?,
            identity: required_string_field(row, "identity")?,
            components: json_array_strings(row.get("components"), "distribution.components")?,
            targets: json_array_strings(row.get("targets"), "distribution.targets")?,
        };
        distributions.push(definition);
    }
    if !distributions
        .iter()
        .any(|distribution| distribution.channel == channel)
    {
        return Err(format!(
            "Frozen family does not declare a {channel} distribution."
        ));
    }

    let build_artifacts = build
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Frozen family build is missing artifacts.".to_string())?;
    let mut artifacts = Vec::new();
    let mut license_material = Vec::new();
    let mut frozen_manifest_sha256 = None;
    let mut frozen_checksum_sha256 = None;
    for row in build_artifacts {
        match string_field(row, "role").as_deref() {
            Some("component-artifact") => artifacts.push(FrozenComponentArtifact {
                component: required_string_field(row, "component")?,
                kind: required_string_field(row, "kind")?,
                target: string_field(row, "target"),
                path: required_string_field(row, "path")?,
                sha256: required_string_field(row, "sha256")?,
                size_bytes: required_u64(row, "size_bytes", "Frozen component artifact")?,
            }),
            Some("license-material") => license_material.push(FrozenLicenseMaterial {
                source_repository: required_string_field(row, "source_repository")?,
                source_snapshot: required_string_field(row, "source_snapshot")?,
                material_role: required_string_field(row, "material_role")?,
                declared_path: required_string_field(row, "declared_path")?,
                path: required_string_field(row, "path")?,
                sha256: required_string_field(row, "sha256")?,
                size_bytes: required_u64(row, "size_bytes", "Frozen license material")?,
            }),
            Some("family-manifest") => {
                frozen_manifest_sha256 = Some(required_string_field(row, "sha256")?)
            }
            Some("family-checksum") => {
                frozen_checksum_sha256 = Some(required_string_field(row, "sha256")?)
            }
            _ => {}
        }
    }
    let created_at = required_string_field(&build, "created_at")?;
    let epoch = created_at.parse::<u64>().map_err(|_| {
        "Frozen family build created_at must be a decimal Unix-second Snapshot time.".to_string()
    })?;
    u32::try_from(epoch).map_err(|_| {
        "Frozen family Snapshot time exceeds deterministic gzip timestamp range.".to_string()
    })?;
    Ok(FamilyPackageInput {
        release_id: release_id.to_string(),
        version: required_string_field(&build, "version")?,
        release_channel: required_string_field(&build, "channel")?,
        tag: required_string_field(&build, "tag")?,
        snapshot_id: required_string_field(&build, "snapshot_id")?,
        epoch,
        family_manifest_sha256: required_string_field(&build, "family_manifest_sha256")?,
        frozen_manifest_sha256: frozen_manifest_sha256
            .ok_or_else(|| "Frozen family build is missing its manifest digest.".to_string())?,
        frozen_checksum_sha256: frozen_checksum_sha256
            .ok_or_else(|| "Frozen family build is missing its checksum digest.".to_string())?,
        components,
        distributions,
        artifacts,
        license_material,
    })
}

fn safe_relative_path(value: &str, context: &str) -> Result<PathBuf, String> {
    if value.is_empty() || value.contains('\\') {
        return Err(format!(
            "{context} must be a non-empty slash-normalized relative path."
        ));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(format!("{context} contains an unsafe path: {value:?}."));
    }
    Ok(path.to_path_buf())
}

fn read_frozen_bytes(
    repo: &RepoRuntime,
    path: &str,
    size_bytes: u64,
    sha256: &str,
    context: &str,
) -> Result<Vec<u8>, String> {
    let relative = safe_relative_path(path, context)?;
    let absolute = repo.workspace_root().join(relative);
    let metadata = fs::symlink_metadata(&absolute).map_err(|error| {
        format!(
            "{context} is unavailable at {}: {error}",
            absolute.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{context} must be a regular non-symlink file."));
    }
    if metadata.len() != size_bytes {
        return Err(format!("{context} size changed before package assembly."));
    }
    let bytes = fs::read(&absolute).map_err(io_error)?;
    if sha256_hex(&bytes) != sha256 {
        return Err(format!(
            "{context} SHA-256 changed before package assembly."
        ));
    }
    Ok(bytes)
}

fn component_artifact(
    input: &FamilyPackageInput,
    component: &str,
    target: &str,
    kind: &str,
) -> Result<FrozenComponentArtifact, String> {
    let matches = input
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.component == component
                && artifact.kind == kind
                && artifact.target.as_deref() == Some(target)
        })
        .cloned()
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "Frozen family must supply exactly one {kind} for {component}/{target}; found {}.",
            matches.len()
        ));
    }
    Ok(matches[0].clone())
}

fn portable_component_artifact(
    input: &FamilyPackageInput,
    component: &str,
    kind: &str,
) -> Result<FrozenComponentArtifact, String> {
    let matches = input
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.component == component && artifact.kind == kind && artifact.target.is_none()
        })
        .cloned()
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "Frozen family must supply exactly one portable {kind} for {component}; found {}.",
            matches.len()
        ));
    }
    Ok(matches[0].clone())
}

fn require_registry_targets(distribution: &DistributionDefinition) -> Result<(), String> {
    let actual = distribution
        .targets
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = REGISTRY_TARGETS.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected || distribution.targets.len() != REGISTRY_TARGETS.len() {
        return Err(format!(
            "{} distribution {:?} must select the exact six-target registry matrix.",
            distribution.channel, distribution.identity
        ));
    }
    Ok(())
}

fn require_distribution_components(
    distribution: &DistributionDefinition,
    expected: &[&str],
) -> Result<(), String> {
    let actual = distribution
        .components
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected || distribution.components.len() != expected.len() {
        return Err(format!(
            "{} distribution {:?} has an invalid product component set.",
            distribution.channel, distribution.identity
        ));
    }
    Ok(())
}

fn component_command(component: &str, target: &str) -> Result<String, String> {
    let command = match component {
        "ait" => "ait",
        "ait-server" => "ait-server",
        "ait-runner" => "ait-runner",
        _ => {
            return Err(format!(
                "Native channel package does not define an installed command for component {component:?}."
            ))
        }
    };
    Ok(if target.ends_with("windows-msvc") {
        format!("{command}.exe")
    } else {
        command.to_string()
    })
}

fn material_for_components(
    input: &FamilyPackageInput,
    component_ids: &[String],
) -> Result<Vec<FrozenLicenseMaterial>, String> {
    let repositories = component_ids
        .iter()
        .map(|component| {
            input
                .components
                .get(component)
                .map(|definition| definition.source_repository.clone())
                .ok_or_else(|| format!("Distribution references unknown component {component:?}."))
        })
        .collect::<Result<BTreeSet<_>, String>>()?;
    let mut selected = input
        .license_material
        .iter()
        .filter(|material| repositories.contains(&material.source_repository))
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        (&left.source_repository, &left.material_role)
            .cmp(&(&right.source_repository, &right.material_role))
    });
    for repository in repositories {
        let roles = selected
            .iter()
            .filter(|material| material.source_repository == repository)
            .map(|material| material.material_role.as_str())
            .collect::<BTreeSet<_>>();
        if roles != BTreeSet::from(["license", "notice"]) {
            return Err(format!(
                "Channel package requires exact license and notice material for repository {repository:?}."
            ));
        }
    }
    Ok(selected)
}

fn github_source_identity(input: &FamilyPackageInput) -> Result<&str, String> {
    let rows = input
        .distributions
        .iter()
        .filter(|distribution| distribution.channel == "github")
        .collect::<Vec<_>>();
    if rows.len() != 1 || rows[0].role != "product" {
        return Err(
            "Frozen family must declare exactly one product GitHub release-monorepo distribution."
                .to_string(),
        );
    }
    let identity = rows[0].identity.as_str();
    if identity.split('/').count() != 2 {
        return Err("GitHub distribution identity must use owner/repository form.".to_string());
    }
    Ok(identity)
}

fn public_source_root(input: &FamilyPackageInput) -> Result<String, String> {
    Ok(format!(
        "https://github.com/{}/tree/{}",
        github_source_identity(input)?,
        input.tag
    ))
}

fn public_source_subtree_url(
    input: &FamilyPackageInput,
    source_repository: &str,
) -> Result<String, String> {
    Ok(format!(
        "{}/{source_repository}",
        public_source_root(input)?
    ))
}

fn content_evidence(
    input: &FamilyPackageInput,
    projections: &[ContentProjection],
) -> Result<Vec<JsonValue>, String> {
    projections
        .iter()
        .map(|projection| {
            let component = input
                .components
                .get(&projection.source.component)
                .ok_or_else(|| "Frozen artifact references an unknown component.".to_string())?;
            Ok(json!({
                "component": component.id,
                "source_repository": component.source_repository,
                "source_snapshot": component.source_snapshot,
                "source_tag": input.tag,
                "public_source_url": public_source_subtree_url(input, &component.source_repository)?,
                "source_kind": projection.source.kind,
                "source_target": projection.source.target,
                "source_path": projection.source.path,
                "source_sha256": projection.source.sha256,
                "source_size_bytes": projection.source.size_bytes,
                "installed_path": projection.destination,
            }))
        })
        .collect()
}

fn material_evidence(
    input: &FamilyPackageInput,
    projections: &[MaterialProjection],
) -> Result<Vec<JsonValue>, String> {
    projections
        .iter()
        .map(|projection| {
            Ok(json!({
                "source_repository": projection.source.source_repository,
                "source_snapshot": projection.source.source_snapshot,
                "source_tag": input.tag,
                "public_source_url": public_source_subtree_url(input, &projection.source.source_repository)?,
                "material_role": projection.source.material_role,
                "declared_path": projection.source.declared_path,
                "source_path": projection.source.path,
                "source_sha256": projection.source.sha256,
                "source_size_bytes": projection.source.size_bytes,
                "installed_path": projection.destination,
            }))
        })
        .collect()
}

fn package_provenance(
    input: &FamilyPackageInput,
    distribution: &DistributionDefinition,
    target: Option<&str>,
    content: &[ContentProjection],
    materials: &[MaterialProjection],
) -> Result<Vec<u8>, String> {
    let payload = json!({
        "contract": FAMILY_PACKAGE_CONTENT_CONTRACT,
        "release_id": input.release_id,
        "version": input.version,
        "release_channel": input.release_channel,
        "distribution_channel": distribution.channel,
        "distribution_role": distribution.role,
        "distribution_identity": distribution.identity,
        "target": target,
        "coordinator_snapshot": input.snapshot_id,
        "family_manifest_sha256": input.family_manifest_sha256,
        "frozen_manifest_sha256": input.frozen_manifest_sha256,
        "frozen_checksum_sha256": input.frozen_checksum_sha256,
        "component_content": content_evidence(input, content)?,
        "license_material": material_evidence(input, materials)?,
        "server_activation": "inactive",
        "component_rebuild": false,
        "registry_write": false,
    });
    encode_value_pretty_with_newline_error_string(&payload).map(String::into_bytes)
}

fn append_tar_entries<W: Write>(
    tar: &mut TarBuilder<W>,
    entries: &PackageEntries,
    epoch: u64,
) -> Result<(), String> {
    for (path, (bytes, mode)) in entries {
        safe_relative_path(path, "Archive member")?;
        append_tar_bytes(tar, path, bytes, *mode, epoch as i64)?;
    }
    Ok(())
}

fn tar_gz_bytes(entries: &PackageEntries, epoch: u64) -> Result<Vec<u8>, String> {
    let encoder = GzBuilder::new()
        .mtime(epoch as u32)
        .operating_system(255)
        .write(Vec::new(), Compression::default());
    let mut tar = TarBuilder::new(encoder);
    append_tar_entries(&mut tar, entries, epoch)?;
    let encoder = tar.into_inner().map_err(io_error)?;
    encoder.finish().map_err(io_error)
}

fn zip_bytes(entries: &PackageEntries) -> Result<Vec<u8>, String> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = ZipWriter::new(cursor);
    let timestamp = zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
        .map_err(|_| "Failed to construct deterministic ZIP timestamp.".to_string())?;
    for (path, (bytes, mode)) in entries {
        safe_relative_path(path, "ZIP member")?;
        let options = FileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(timestamp)
            .unix_permissions(*mode);
        archive.start_file(path, options).map_err(zip_error)?;
        archive.write_all(bytes).map_err(io_error)?;
    }
    archive
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(zip_error)
}

fn wheel_zip_bytes(entries: &PackageEntries, record_path: &str) -> Result<Vec<u8>, String> {
    if !entries.contains_key(record_path) {
        return Err("Repacked wheel is missing its generated RECORD.".to_string());
    }
    let cursor = Cursor::new(Vec::new());
    let mut archive = ZipWriter::new(cursor);
    let timestamp = zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
        .map_err(|_| "Failed to construct deterministic wheel timestamp.".to_string())?;
    let dist_info_prefix = record_path
        .strip_suffix("RECORD")
        .ok_or_else(|| "Wheel RECORD path is malformed.".to_string())?;
    let mut paths = entries
        .keys()
        .filter(|path| path.as_str() != record_path)
        .cloned()
        .collect::<Vec<_>>();
    paths.sort_by_key(|path| (path.starts_with(dist_info_prefix), path.clone()));
    paths.push(record_path.to_string());
    for path in paths {
        safe_relative_path(&path, "Wheel member")?;
        let (bytes, mode) = entries
            .get(&path)
            .ok_or_else(|| format!("Wheel member {path:?} disappeared during assembly."))?;
        let options = FileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(timestamp)
            .unix_permissions(*mode);
        archive.start_file(&path, options).map_err(zip_error)?;
        archive.write_all(bytes).map_err(io_error)?;
    }
    archive
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(zip_error)
}

fn read_wheel_entries(bytes: &[u8]) -> Result<PackageEntries, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Frozen Python wheel is not a valid ZIP archive: {error}"))?;
    if archive.is_empty() || archive.len() > MAX_REGISTRY_ARCHIVE_ENTRIES {
        return Err(format!(
            "Frozen Python wheel must contain between 1 and {MAX_REGISTRY_ARCHIVE_ENTRIES} members."
        ));
    }
    if !archive.comment().is_empty() {
        return Err("Frozen Python wheel contains a ZIP comment.".to_string());
    }
    let mut entries = PackageEntries::new();
    let mut total_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            format!("Frozen Python wheel member {index} cannot be opened: {error}")
        })?;
        if entry.is_dir() {
            return Err(format!(
                "Frozen Python wheel contains directory member {:?}; normalized wheels contain files only.",
                entry.name()
            ));
        }
        if entry.name_raw() != entry.name().as_bytes() {
            return Err("Frozen Python wheel contains a non-UTF-8 member name.".to_string());
        }
        let path = entry.name().to_string();
        safe_relative_path(&path, "Frozen Python wheel member")?;
        if !entry.comment().is_empty() || !entry.extra_data().is_empty() {
            return Err(format!(
                "Frozen Python wheel member {path:?} contains ZIP comment or extra data."
            ));
        }
        if path.contains("/__pycache__/")
            || path.ends_with(".pyc")
            || path.ends_with(".pyo")
            || path.ends_with("/RECORD.jws")
            || path.ends_with("/RECORD.p7s")
        {
            return Err(format!(
                "Frozen Python wheel contains forbidden member {path:?}."
            ));
        }
        if entry.size() > MAX_REGISTRY_ARCHIVE_MEMBER_BYTES {
            return Err(format!(
                "Frozen Python wheel member {path:?} exceeds the per-member size limit."
            ));
        }
        total_bytes = total_bytes
            .checked_add(entry.size())
            .ok_or_else(|| "Frozen Python wheel total size overflowed.".to_string())?;
        if total_bytes > MAX_REGISTRY_ARCHIVE_TOTAL_BYTES {
            return Err("Frozen Python wheel exceeds the uncompressed size limit.".to_string());
        }
        let mode = entry.unix_mode().unwrap_or(0o644);
        let file_type = mode & 0o170000;
        if file_type != 0 && file_type != 0o100000 {
            return Err(format!(
                "Frozen Python wheel member {path:?} is not a regular file."
            ));
        }
        let mut member_bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut member_bytes).map_err(|error| {
            format!("Frozen Python wheel member {path:?} failed CRC/read validation: {error}")
        })?;
        if entries
            .insert(path.clone(), (member_bytes, mode & 0o777))
            .is_some()
        {
            return Err(format!(
                "Frozen Python wheel contains duplicate member {path:?}."
            ));
        }
    }
    Ok(entries)
}

fn validate_wheel_record(entries: &PackageEntries, record_path: &str) -> Result<(), String> {
    let record = entries
        .get(record_path)
        .ok_or_else(|| "Frozen Python wheel is missing RECORD.".to_string())?;
    let text = std::str::from_utf8(&record.0)
        .map_err(|_| "Frozen Python wheel RECORD must be UTF-8.".to_string())?;
    let mut recorded = BTreeSet::new();
    for (line_index, raw_line) in text.lines().enumerate() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let fields = line.split(',').collect::<Vec<_>>();
        if fields.len() != 3 || fields[0].contains('"') {
            return Err(format!(
                "Frozen Python wheel RECORD line {} must use the canonical unquoted three-field form.",
                line_index + 1
            ));
        }
        let path = fields[0];
        safe_relative_path(path, "Frozen Python wheel RECORD path")?;
        if !recorded.insert(path.to_string()) {
            return Err(format!(
                "Frozen Python wheel RECORD contains duplicate path {path:?}."
            ));
        }
        let (member_bytes, _) = entries
            .get(path)
            .ok_or_else(|| format!("Frozen Python wheel RECORD references missing {path:?}."))?;
        if path == record_path {
            if !fields[1].is_empty() || !fields[2].is_empty() {
                return Err(
                    "Frozen Python wheel RECORD must leave its own digest and size empty."
                        .to_string(),
                );
            }
            continue;
        }
        let encoded = fields[1]
            .strip_prefix("sha256=")
            .ok_or_else(|| format!("Frozen Python wheel RECORD {path:?} lacks SHA-256."))?;
        let decoded = BASE64_URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| format!("Frozen Python wheel RECORD {path:?} has invalid SHA-256."))?;
        if decoded.as_slice() != Sha256::digest(member_bytes).as_slice() {
            return Err(format!(
                "Frozen Python wheel RECORD digest differs for {path:?}."
            ));
        }
        let size = fields[2]
            .parse::<usize>()
            .map_err(|_| format!("Frozen Python wheel RECORD {path:?} has invalid size."))?;
        if size != member_bytes.len() {
            return Err(format!(
                "Frozen Python wheel RECORD size differs for {path:?}."
            ));
        }
    }
    if recorded != entries.keys().cloned().collect::<BTreeSet<_>>() {
        return Err("Frozen Python wheel RECORD inventory is not exact.".to_string());
    }
    Ok(())
}

fn generated_wheel_record(entries: &PackageEntries, record_path: &str) -> Result<Vec<u8>, String> {
    let mut rows = Vec::new();
    for (path, (bytes, _)) in entries {
        if path == record_path {
            continue;
        }
        if path.contains(',') || path.contains('"') {
            return Err(format!(
                "Generated wheel path {path:?} cannot use canonical RECORD encoding."
            ));
        }
        rows.push(format!(
            "{path},sha256={},{}",
            BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)),
            bytes.len()
        ));
    }
    rows.push(format!("{record_path},,"));
    Ok((rows.join("\n") + "\n").into_bytes())
}

fn read_npm_envelope_entries(bytes: &[u8]) -> Result<PackageEntries, String> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = TarArchive::new(decoder);
    let mut entries = PackageEntries::new();
    let mut total_bytes = 0_u64;
    let rows = archive
        .entries()
        .map_err(|error| format!("Frozen npm envelope is not a valid tar.gz archive: {error}"))?;
    for row in rows {
        let mut entry = row.map_err(|error| format!("Frozen npm envelope is invalid: {error}"))?;
        if entries.len() >= MAX_REGISTRY_ARCHIVE_ENTRIES {
            return Err("Frozen npm envelope exceeds its member-count limit.".to_string());
        }
        if !entry.header().entry_type().is_file() {
            return Err("Frozen npm envelope may contain regular files only.".to_string());
        }
        let entry_path = entry
            .path()
            .map_err(|error| format!("Frozen npm envelope has an invalid path: {error}"))?;
        let path = entry_path
            .to_str()
            .ok_or_else(|| "Frozen npm envelope has a non-UTF-8 member path.".to_string())?
            .to_string();
        safe_relative_path(&path, "Frozen npm envelope member")?;
        let size = entry.size();
        if size > MAX_REGISTRY_ARCHIVE_MEMBER_BYTES {
            return Err(format!(
                "Frozen npm envelope member {path:?} exceeds the per-member size limit."
            ));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| "Frozen npm envelope total size overflowed.".to_string())?;
        if total_bytes > MAX_REGISTRY_ARCHIVE_TOTAL_BYTES {
            return Err("Frozen npm envelope exceeds the uncompressed size limit.".to_string());
        }
        let mode = entry.header().mode().map_err(io_error)? & 0o777;
        let mut member_bytes = Vec::with_capacity(size as usize);
        entry.read_to_end(&mut member_bytes).map_err(io_error)?;
        if entries.insert(path.clone(), (member_bytes, mode)).is_some() {
            return Err(format!(
                "Frozen npm envelope contains duplicate member {path:?}."
            ));
        }
    }
    if entries.is_empty() {
        return Err("Frozen npm envelope is empty.".to_string());
    }
    Ok(entries)
}

fn github_asset_base(input: &FamilyPackageInput) -> Result<String, String> {
    Ok(format!(
        "https://github.com/{}/releases/download/{}",
        github_source_identity(input)?,
        input.tag
    ))
}

fn json_quoted(value: &str) -> String {
    encode_string_or(value, "\"\"")
}

fn artifact_evidence(
    kind: &str,
    distribution: &DistributionDefinition,
    target: Option<&str>,
    content: &[ContentProjection],
    materials: &[MaterialProjection],
    extra: JsonValue,
    input: &FamilyPackageInput,
) -> Result<JsonValue, String> {
    Ok(json!({
        "role": "channel-package",
        "kind": kind,
        "distribution_channel": distribution.channel,
        "distribution_role": distribution.role,
        "distribution_identity": distribution.identity,
        "target": target,
        "component_content": content_evidence(input, content)?,
        "license_material": material_evidence(input, materials)?,
        "metadata": extra,
    }))
}

fn native_distribution_entries(
    repo: &RepoRuntime,
    input: &FamilyPackageInput,
    distribution: &DistributionDefinition,
    target: &str,
    executable_prefix: &str,
    license_prefix: &str,
    provenance_path: &str,
) -> Result<NativeDistributionEntries, String> {
    let mut entries = BTreeMap::new();
    let mut content = Vec::new();
    for component in &distribution.components {
        let source = component_artifact(input, component, target, "native-executable")?;
        let command = component_command(component, target)?;
        let destination = format!("{executable_prefix}/{command}");
        let bytes = read_frozen_bytes(
            repo,
            &source.path,
            source.size_bytes,
            &source.sha256,
            "Frozen native executable",
        )?;
        if entries
            .insert(destination.clone(), (bytes, 0o755))
            .is_some()
        {
            return Err(format!(
                "Native package destination collides at {destination:?}."
            ));
        }
        content.push(ContentProjection {
            source,
            destination,
        });
    }
    let materials = material_for_components(input, &distribution.components)?;
    let mut material_projections = Vec::new();
    for material in materials {
        let destination = format!(
            "{license_prefix}/{}/{}",
            material.source_repository, material.declared_path
        );
        let bytes = read_frozen_bytes(
            repo,
            &material.path,
            material.size_bytes,
            &material.sha256,
            "Frozen license material",
        )?;
        if entries
            .insert(destination.clone(), (bytes, 0o644))
            .is_some()
        {
            return Err(format!(
                "License-material destination collides at {destination:?}."
            ));
        }
        material_projections.push(MaterialProjection {
            source: material,
            destination,
        });
    }
    let provenance = package_provenance(
        input,
        distribution,
        Some(target),
        &content,
        &material_projections,
    )?;
    entries.insert(provenance_path.to_string(), (provenance, 0o644));
    Ok((entries, content, material_projections))
}

fn homebrew_formula_class(name: &str) -> String {
    let mut class_name = String::new();
    let mut capitalize = true;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if capitalize {
                class_name.push(character.to_ascii_uppercase());
                capitalize = false;
            } else {
                class_name.push(character);
            }
        } else {
            capitalize = true;
        }
    }
    if class_name.is_empty() {
        "AitNative".to_string()
    } else {
        class_name
    }
}

fn homebrew_license_expression(
    input: &FamilyPackageInput,
    distribution: &DistributionDefinition,
) -> Result<String, String> {
    let licenses = distribution
        .components
        .iter()
        .map(|component| {
            input
                .components
                .get(component)
                .map(|definition| definition.license.clone())
                .ok_or_else(|| format!("Unknown distribution component {component:?}."))
        })
        .collect::<Result<BTreeSet<_>, String>>()?;
    if licenses.len() == 1 {
        Ok(json_quoted(licenses.iter().next().unwrap()))
    } else {
        Ok(format!(
            "all_of: [{}]",
            licenses
                .iter()
                .map(|license| json_quoted(license))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

fn homebrew_arch_block(
    os_name: &str,
    archives: &BTreeMap<String, (String, String)>,
    asset_base: &str,
) -> Result<Option<String>, String> {
    let (arm_target, intel_target) = match os_name {
        "macos" => ("aarch64-apple-darwin", "x86_64-apple-darwin"),
        "linux" => ("aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"),
        _ => return Err(format!("Unsupported Homebrew OS block {os_name:?}.")),
    };
    let arm = archives.get(arm_target);
    let intel = archives.get(intel_target);
    if arm.is_none() && intel.is_none() {
        return Ok(None);
    }
    let mut text = format!("  on_{os_name} do\n");
    text.push_str("    if Hardware::CPU.arm?\n");
    if let Some((filename, digest)) = arm {
        text.push_str(&format!(
            "      url {}\n      sha256 {}\n",
            json_quoted(&format!("{asset_base}/{filename}")),
            json_quoted(digest)
        ));
    } else {
        text.push_str("      odie \"ait-native does not publish this ARM target\"\n");
    }
    text.push_str("    elsif Hardware::CPU.intel?\n");
    if let Some((filename, digest)) = intel {
        text.push_str(&format!(
            "      url {}\n      sha256 {}\n",
            json_quoted(&format!("{asset_base}/{filename}")),
            json_quoted(digest)
        ));
    } else {
        text.push_str("      odie \"ait-native does not publish this Intel target\"\n");
    }
    text.push_str("    else\n      odie \"unsupported CPU architecture\"\n    end\n  end\n");
    Ok(Some(text))
}

fn assemble_homebrew(
    repo: &RepoRuntime,
    input: &FamilyPackageInput,
) -> Result<Vec<GeneratedArtifact>, String> {
    let distributions = input
        .distributions
        .iter()
        .filter(|distribution| distribution.channel == "homebrew")
        .collect::<Vec<_>>();
    if distributions.len() != 1 {
        return Err(format!(
            "Homebrew assembly requires exactly one declared distribution; found {}.",
            distributions.len()
        ));
    }
    let distribution = distributions[0];
    if distribution.role != "product" {
        return Err("Homebrew ait-native distribution must have product role.".to_string());
    }
    let asset_base = github_asset_base(input)?;
    let mut generated = Vec::new();
    let mut archive_rows = BTreeMap::new();
    let mut formula_inputs = Vec::new();
    for target in &distribution.targets {
        if target.ends_with("windows-msvc") {
            return Err(format!(
                "Homebrew distribution cannot select Windows target {target:?}."
            ));
        }
        let (entries, content, materials) = native_distribution_entries(
            repo,
            input,
            distribution,
            target,
            "bin",
            "share/licenses",
            "share/ait-native/ait-family-provenance.json",
        )?;
        let bytes = tar_gz_bytes(&entries, input.epoch)?;
        let filename = format!(
            "{}-{}-{target}.tar.gz",
            distribution.identity, input.version
        );
        safe_relative_path(&filename, "Homebrew archive filename")?;
        let digest = sha256_hex(&bytes);
        archive_rows.insert(target.clone(), (filename.clone(), digest.clone()));
        formula_inputs.push(json!({
            "target": target,
            "filename": filename,
            "sha256": digest,
            "size_bytes": bytes.len(),
        }));
        generated.push(GeneratedArtifact {
            relative_path: format!("archives/{filename}"),
            bytes,
            evidence: artifact_evidence(
                "homebrew-archive",
                distribution,
                Some(target),
                &content,
                &materials,
                json!({
                    "asset_url": format!("{asset_base}/{filename}"),
                    "server_activation": "inactive",
                }),
                input,
            )?,
        });
    }
    let route_name = if input.release_channel == "rc" {
        format!("{}-rc", distribution.identity)
    } else {
        distribution.identity.clone()
    };
    let class_name = homebrew_formula_class(&route_name);
    let mut platform_blocks = String::new();
    if let Some(block) = homebrew_arch_block("macos", &archive_rows, &asset_base)? {
        platform_blocks.push_str(&block);
        platform_blocks.push('\n');
    }
    if let Some(block) = homebrew_arch_block("linux", &archive_rows, &asset_base)? {
        platform_blocks.push_str(&block);
        platform_blocks.push('\n');
    }
    let homepage = format!("https://github.com/{}", github_source_identity(input)?);
    let formula = format!(
        "class {class_name} < Formula\n  desc \"Language-neutral native AIT CLI and inactive self-hosted server\"\n  homepage \"{homepage}\"\n  version {}\n  license {}\n\n{}  def install\n    bin.install \"bin/ait\"\n    bin.install \"bin/ait-server\"\n    pkgshare.install \"share/licenses\"\n    pkgshare.install \"share/ait-native/ait-family-provenance.json\"\n  end\n\n  service do\n    run [opt_bin/\"ait-server\", \"run\", \"--data\", var/\"ait-native/server-data\", \"--init-if-missing\", \"--defer-ci-admission\"]\n    keep_alive true\n    log_path var/\"log/ait-server.log\"\n    error_log_path var/\"log/ait-server.error.log\"\n  end\n\n  def caveats\n    <<~EOS\n      ait-server is installed but remains inactive until explicitly started.\n      Foreground: #{{bin}}/ait-server run\n      Managed user service: brew services start {route_name}\n      Service data: #{{var}}/ait-native/server-data\n      Managed CI still requires an admitted memory-backed runtime root.\n    EOS\n  end\n\n  test do\n    assert_match version.to_s, shell_output(\"#{{bin}}/ait --version\")\n    assert_match version.to_s, shell_output(\"#{{bin}}/ait-server --version\")\n  end\nend\n",
        json_quoted(&input.version),
        homebrew_license_expression(input, distribution)?,
        platform_blocks,
    );
    let formula_filename = format!("{route_name}.rb");
    generated.push(GeneratedArtifact {
        relative_path: format!("Formula/{formula_filename}"),
        bytes: formula.into_bytes(),
        evidence: artifact_evidence(
            "homebrew-formula",
            distribution,
            None,
            &[],
            &[],
            json!({
                "class_name": class_name,
                "route": if input.release_channel == "rc" { "rc-tap" } else { "stable" },
                "stable_formula_mutation": input.release_channel == "stable",
                "archives": formula_inputs,
                "server_service_stanza": true,
                "server_activation": "explicit_brew_services_start",
                "server_data_root": "#{var}/ait-native/server-data",
            }),
            input,
        )?,
    });
    Ok(generated)
}

fn debian_version(version: &str) -> Result<String, String> {
    if let Some((base, ordinal)) = version.rsplit_once("-rc.") {
        if base.is_empty()
            || ordinal.is_empty()
            || !ordinal.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(format!(
                "Cannot map family version {version:?} to Debian syntax."
            ));
        }
        Ok(format!("{base}~rc.{ordinal}"))
    } else if version
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'~'))
    {
        Ok(version.to_string())
    } else {
        Err(format!(
            "Cannot map family version {version:?} to Debian syntax."
        ))
    }
}

fn debian_architecture(target: &str) -> Result<&'static str, String> {
    match target {
        "aarch64-unknown-linux-gnu" => Ok("arm64"),
        "x86_64-unknown-linux-gnu" => Ok("amd64"),
        _ => Err(format!("apt does not support family target {target:?}.")),
    }
}

fn debian_package_name(identity: &str) -> Result<&str, String> {
    if identity.is_empty()
        || !identity.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')
        })
    {
        return Err(format!("Invalid Debian package identity {identity:?}."));
    }
    Ok(identity)
}

fn ar_member(bytes: &mut Vec<u8>, name: &str, data: &[u8], epoch: u64) -> Result<(), String> {
    let member_name = format!("{name}/");
    if member_name.len() > 16 {
        return Err(format!("Debian ar member name {name:?} exceeds 16 bytes."));
    }
    let header = format!(
        "{member_name:<16}{epoch:<12}{uid:<6}{gid:<6}{mode:<8}{size:<10}`\n",
        uid = 0,
        gid = 0,
        mode = "100644",
        size = data.len(),
    );
    if header.len() != 60 {
        return Err("Failed to construct a fixed-width Debian ar header.".to_string());
    }
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(data);
    if !data.len().is_multiple_of(2) {
        bytes.push(b'\n');
    }
    Ok(())
}

fn debian_archive_bytes(
    control_tar: &[u8],
    data_tar: &[u8],
    epoch: u64,
) -> Result<Vec<u8>, String> {
    let mut bytes = b"!<arch>\n".to_vec();
    ar_member(&mut bytes, "debian-binary", b"2.0\n", epoch)?;
    ar_member(&mut bytes, "control.tar.gz", control_tar, epoch)?;
    ar_member(&mut bytes, "data.tar.gz", data_tar, epoch)?;
    Ok(bytes)
}

fn debian_copyright(
    input: &FamilyPackageInput,
    distribution: &DistributionDefinition,
    package_name: &str,
) -> Result<Vec<u8>, String> {
    let mut repositories = BTreeMap::<String, (String, String)>::new();
    let source_root = public_source_root(input)?;
    let mut text = format!(
        "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\nUpstream-Name: {}\nSource: {source_root}\nComment: This binary aggregate preserves each installed component's own license.\n\n",
        distribution.identity
    );
    for component_id in &distribution.components {
        let component = input
            .components
            .get(component_id)
            .ok_or_else(|| format!("Unknown apt component {component_id:?}."))?;
        match repositories.get(&component.source_repository) {
            Some((snapshot, license))
                if snapshot != &component.source_snapshot || license != &component.license =>
            {
                return Err(format!(
                    "apt source repository {:?} has conflicting Snapshot or license authority.",
                    component.source_repository
                ));
            }
            Some(_) => {}
            None => {
                repositories.insert(
                    component.source_repository.clone(),
                    (component.source_snapshot.clone(), component.license.clone()),
                );
            }
        }
        let command = component_command(component_id, "x86_64-unknown-linux-gnu")?;
        let component_source_url = public_source_subtree_url(input, &component.source_repository)?;
        text.push_str(&format!(
            "Files: usr/bin/{command}\nCopyright: 2026 Weita and contributors\nLicense: {}\nComment: Source: {component_source_url} ; AIT Snapshot: {}\n\n",
            component.license,
            component.source_snapshot
        ));
    }
    for (repository, (snapshot, license)) in &repositories {
        let repository_source_url = public_source_subtree_url(input, repository)?;
        text.push_str(&format!(
            "Files: usr/share/doc/{package_name}/licenses/{repository}/*\nCopyright: 2026 Weita and contributors\nLicense: {license}\nComment: Exact upstream legal material from {repository_source_url} ; AIT Snapshot: {snapshot}\n\n"
        ));
    }
    let mut package_owned_files = vec![
        format!("usr/share/doc/{package_name}/ait-family-provenance.json"),
        format!("usr/share/doc/{package_name}/copyright"),
    ];
    if distribution.role == "product"
        && distribution
            .components
            .iter()
            .any(|component| component == "ait-server")
    {
        package_owned_files.push(AIT_SERVER_SYSTEMD_UNIT_PATH.to_string());
    }
    text.push_str(&format!(
        "Files: {}\nCopyright: 2026 Weita and contributors\nLicense: Apache-2.0\n\n",
        package_owned_files.join("\n ")
    ));
    let licenses = repositories
        .values()
        .map(|(_, license)| license.clone())
        .chain(std::iter::once("Apache-2.0".to_string()))
        .collect::<BTreeSet<_>>();
    for license in licenses {
        let common_path = match license.as_str() {
            "Apache-2.0" => "/usr/share/common-licenses/Apache-2.0",
            "AGPL-3.0-only" => "/usr/share/common-licenses/AGPL-3",
            _ => {
                return Err(format!(
                    "apt copyright has no admitted full-text mapping for license {license:?}."
                ))
            }
        };
        let exact_paths = repositories
            .iter()
            .filter(|(_, (_, repository_license))| repository_license == &license)
            .map(|(repository, _)| {
                format!("/usr/share/doc/{package_name}/licenses/{repository}/LICENSE")
            })
            .collect::<Vec<_>>();
        text.push_str(&format!(
            "License: {license}\n On Debian systems, the complete license text is available at\n {common_path}.\n"
        ));
        if !exact_paths.is_empty() {
            text.push_str(&format!(
                " .\n The exact upstream text{} installed at {}.\n",
                if exact_paths.len() == 1 {
                    " is"
                } else {
                    "s are"
                },
                exact_paths.join(", ")
            ));
        }
        text.push('\n');
    }
    Ok(text.into_bytes())
}

fn assemble_apt(
    repo: &RepoRuntime,
    input: &FamilyPackageInput,
) -> Result<Vec<GeneratedArtifact>, String> {
    let distributions = input
        .distributions
        .iter()
        .filter(|distribution| distribution.channel == "apt")
        .collect::<Vec<_>>();
    let version = debian_version(&input.version)?;
    let suite = if input.release_channel == "rc" {
        "testing"
    } else {
        "stable"
    };
    let mut generated = Vec::new();
    for distribution in distributions {
        let package_name = debian_package_name(&distribution.identity)?;
        if !matches!(distribution.role.as_str(), "product" | "standalone") {
            return Err(format!(
                "apt distribution {package_name:?} must have product or standalone role."
            ));
        }
        for target in &distribution.targets {
            let architecture = debian_architecture(target)?;
            let documentation_root = format!("usr/share/doc/{package_name}");
            let installs_server_unit = distribution.role == "product"
                && distribution
                    .components
                    .iter()
                    .any(|component| component == "ait-server");
            let (mut data_entries, content, materials) = native_distribution_entries(
                repo,
                input,
                distribution,
                target,
                "usr/bin",
                &format!("{documentation_root}/licenses"),
                &format!("{documentation_root}/ait-family-provenance.json"),
            )?;
            data_entries.insert(
                format!("{documentation_root}/copyright"),
                (debian_copyright(input, distribution, &package_name)?, 0o644),
            );
            if installs_server_unit
                && data_entries
                    .insert(
                        AIT_SERVER_SYSTEMD_UNIT_PATH.to_string(),
                        (AIT_SERVER_SYSTEMD_UNIT.as_bytes().to_vec(), 0o644),
                    )
                    .is_some()
            {
                return Err(format!(
                    "apt systemd unit destination collides at {AIT_SERVER_SYSTEMD_UNIT_PATH:?}."
                ));
            }
            let installed_bytes = data_entries
                .values()
                .map(|(bytes, _)| bytes.len() as u64)
                .sum::<u64>();
            let installed_size = installed_bytes.div_ceil(1024).max(1);
            let description = if distribution.role == "product" {
                "Language-neutral native AIT CLI and inactive self-hosted server"
            } else {
                "Native AIT execution runner"
            };
            let homepage = format!("https://github.com/{}", github_source_identity(input)?);
            let control = format!(
                "Package: {package_name}\nVersion: {version}\nSection: devel\nPriority: optional\nArchitecture: {architecture}\nMaintainer: AIT maintainers <weita2026@users.noreply.github.com>\nInstalled-Size: {installed_size}\nDepends: libc6 (>= 2.28)\nHomepage: {homepage}\nDescription: {description}\n Built from an immutable AIT family dossier without starting services.\n"
            );
            let control_entries =
                BTreeMap::from([("control".to_string(), (control.into_bytes(), 0o644))]);
            let control_tar = tar_gz_bytes(&control_entries, input.epoch)?;
            let data_tar = tar_gz_bytes(&data_entries, input.epoch)?;
            let bytes = debian_archive_bytes(&control_tar, &data_tar, input.epoch)?;
            let filename = format!("{package_name}_{version}_{architecture}.deb");
            generated.push(GeneratedArtifact {
                relative_path: format!("packages/{filename}"),
                bytes,
                evidence: artifact_evidence(
                    "debian-package",
                    distribution,
                    Some(target),
                    &content,
                    &materials,
                    json!({
                        "package": package_name,
                        "debian_version": version,
                        "architecture": architecture,
                        "suite": suite,
                        "server_activation": "inactive",
                        "maintainer_script_count": 0,
                        "systemd_unit": installs_server_unit,
                        "systemd_unit_path": installs_server_unit.then_some(AIT_SERVER_SYSTEMD_UNIT_PATH),
                        "systemd_unit_sha256": installs_server_unit.then(|| sha256_hex(AIT_SERVER_SYSTEMD_UNIT.as_bytes())),
                    }),
                    input,
                )?,
            });
        }
    }
    Ok(generated)
}

fn winget_architecture(target: &str) -> Result<&'static str, String> {
    match target {
        "aarch64-pc-windows-msvc" => Ok("arm64"),
        "x86_64-pc-windows-msvc" => Ok("x64"),
        _ => Err(format!("WinGet does not support family target {target:?}.")),
    }
}

fn winget_license_expression(
    input: &FamilyPackageInput,
    distribution: &DistributionDefinition,
) -> Result<String, String> {
    distribution
        .components
        .iter()
        .map(|component| {
            input
                .components
                .get(component)
                .map(|definition| definition.license.clone())
                .ok_or_else(|| format!("Unknown WinGet component {component:?}."))
        })
        .collect::<Result<BTreeSet<_>, String>>()
        .map(|licenses| licenses.into_iter().collect::<Vec<_>>().join(" AND "))
}

fn assemble_winget(
    repo: &RepoRuntime,
    input: &FamilyPackageInput,
) -> Result<Vec<GeneratedArtifact>, String> {
    let distributions = input
        .distributions
        .iter()
        .filter(|distribution| distribution.channel == "winget")
        .collect::<Vec<_>>();
    if distributions.len() != 1 {
        return Err(format!(
            "WinGet assembly requires exactly one declared distribution; found {}.",
            distributions.len()
        ));
    }
    let distribution = distributions[0];
    if distribution.role != "product" {
        return Err("WinGet ait-native distribution must have product role.".to_string());
    }
    let asset_base = github_asset_base(input)?;
    let route = if input.release_channel == "rc" {
        "validation"
    } else {
        "community"
    };
    let mut generated = Vec::new();
    let mut installer_rows = Vec::new();
    let mut installer_yaml = String::new();
    installer_yaml.push_str(&format!(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.{WINGET_MANIFEST_VERSION}.schema.json\nPackageIdentifier: {}\nPackageVersion: {}\nInstallerType: zip\nInstallers:\n",
        json_quoted(&distribution.identity),
        json_quoted(&input.version),
    ));
    for target in &distribution.targets {
        let architecture = winget_architecture(target)?;
        let (mut entries, mut content, materials) = native_distribution_entries(
            repo,
            input,
            distribution,
            target,
            "bin",
            "licenses",
            "ait-family-provenance.json",
        )?;
        for projection in &mut content {
            projection.destination = projection
                .destination
                .strip_prefix("bin/")
                .ok_or_else(|| {
                    format!(
                        "WinGet executable destination {:?} is not rooted under bin/.",
                        projection.destination
                    )
                })?
                .to_string();
        }
        entries.insert(
            "ait-family-provenance.json".to_string(),
            (
                package_provenance(input, distribution, Some(target), &content, &materials)?,
                0o644,
            ),
        );
        if entries
            .insert(
                WINGET_SERVER_CONTROLLER_PATH.to_string(),
                (WINGET_SERVER_CONTROLLER.as_bytes().to_vec(), 0o644),
            )
            .is_some()
        {
            return Err(format!(
                "WinGet server-controller destination collides at {WINGET_SERVER_CONTROLLER_PATH:?}."
            ));
        }
        let mut zip_entries = BTreeMap::new();
        for (path, value) in entries {
            let destination = path
                .strip_prefix("bin/")
                .map(ToString::to_string)
                .unwrap_or(path);
            if zip_entries.insert(destination.clone(), value).is_some() {
                return Err(format!(
                    "WinGet ZIP destination collides at {destination:?}."
                ));
            }
        }
        let bytes = zip_bytes(&zip_entries)?;
        let filename = format!("ait-native-{}-{target}.zip", input.version);
        let digest = sha256_hex(&bytes);
        let url = format!("{asset_base}/{filename}");
        installer_yaml.push_str(&format!(
            "  - Architecture: {architecture}\n    InstallerUrl: {}\n    InstallerSha256: {}\n    NestedInstallerType: portable\n    Scope: user\n    NestedInstallerFiles:\n    - RelativeFilePath: ait.exe\n      PortableCommandAlias: ait\n    - RelativeFilePath: ait-server.exe\n      PortableCommandAlias: ait-server\n    - RelativeFilePath: ait-server-control.ps1\n      PortableCommandAlias: ait-server-control.ps1\n    ArchiveBinariesDependOnPath: false\n",
            json_quoted(&url),
            digest.to_ascii_uppercase(),
        ));
        installer_rows.push(json!({
            "target": target,
            "architecture": architecture,
            "filename": filename,
            "url": url,
            "sha256": digest,
            "size_bytes": bytes.len(),
        }));
        generated.push(GeneratedArtifact {
            relative_path: format!("installers/{filename}"),
            bytes,
            evidence: artifact_evidence(
                "winget-portable-zip",
                distribution,
                Some(target),
                &content,
                &materials,
                json!({
                    "architecture": architecture,
                    "installer_type": "zip",
                    "nested_installer_type": "portable",
                    "scope": "user",
                    "portable_commands": ["ait", "ait-server", "ait-server-control.ps1"],
                    "asset_url": url,
                    "server_activation": "inactive",
                    "server_controller": "user_session_powershell",
                    "server_controller_path": WINGET_SERVER_CONTROLLER_PATH,
                    "server_controller_sha256": sha256_hex(WINGET_SERVER_CONTROLLER.as_bytes()),
                    "windows_service_registration": false,
                }),
                input,
            )?,
        });
    }
    installer_yaml.push_str(&format!(
        "ManifestType: installer\nManifestVersion: {WINGET_MANIFEST_VERSION}\n"
    ));
    let version_yaml = format!(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.{WINGET_MANIFEST_VERSION}.schema.json\nPackageIdentifier: {}\nPackageVersion: {}\nDefaultLocale: en-US\nManifestType: version\nManifestVersion: {WINGET_MANIFEST_VERSION}\n",
        json_quoted(&distribution.identity),
        json_quoted(&input.version),
    );
    let public_repository_url = format!("https://github.com/{}", github_source_identity(input)?);
    let public_license_url = format!(
        "{public_repository_url}/blob/{}/docs/distribution.md#license-and-source-publication-gate",
        input.tag
    );
    let locale_yaml = format!(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.{WINGET_MANIFEST_VERSION}.schema.json\nPackageIdentifier: {}\nPackageVersion: {}\nPackageLocale: en-US\nPublisher: Weita\nPublisherUrl: https://github.com/weita2026\nPackageName: ait-native\nPackageUrl: {}\nLicense: {}\nLicenseUrl: {}\nShortDescription: Language-neutral native AIT CLI and inactive self-hosted server\nTags:\n  - ai\n  - cli\n  - developer-tools\nManifestType: defaultLocale\nManifestVersion: {WINGET_MANIFEST_VERSION}\n",
        json_quoted(&distribution.identity),
        json_quoted(&input.version),
        json_quoted(&public_repository_url),
        json_quoted(&winget_license_expression(input, distribution)?),
        json_quoted(&public_license_url),
    );
    let manifest_files = [
        (
            format!("{}.yaml", distribution.identity),
            "winget-version-manifest",
            version_yaml.into_bytes(),
        ),
        (
            format!("{}.locale.en-US.yaml", distribution.identity),
            "winget-default-locale-manifest",
            locale_yaml.into_bytes(),
        ),
        (
            format!("{}.installer.yaml", distribution.identity),
            "winget-installer-manifest",
            installer_yaml.into_bytes(),
        ),
    ];
    for (filename, kind, bytes) in manifest_files {
        generated.push(GeneratedArtifact {
            relative_path: format!("manifests/{filename}"),
            bytes,
            evidence: artifact_evidence(
                kind,
                distribution,
                None,
                &[],
                &[],
                json!({
                    "manifest_version": WINGET_MANIFEST_VERSION,
                    "route": route,
                    "community_manifest_submission": input.release_channel == "stable",
                    "installers": installer_rows,
                }),
                input,
            )?,
        });
    }
    Ok(generated)
}

fn single_channel_distribution<'a>(
    input: &'a FamilyPackageInput,
    channel: &str,
) -> Result<&'a DistributionDefinition, String> {
    let distributions = input
        .distributions
        .iter()
        .filter(|distribution| distribution.channel == channel)
        .collect::<Vec<_>>();
    if distributions.len() != 1 {
        return Err(format!(
            "{channel} assembly requires exactly one declared distribution; found {}.",
            distributions.len()
        ));
    }
    Ok(distributions[0])
}

fn required_component<'a>(
    input: &'a FamilyPackageInput,
    component: &str,
    ecosystem: &str,
) -> Result<&'a ComponentDefinition, String> {
    let definition = input
        .components
        .get(component)
        .ok_or_else(|| format!("Frozen family is missing component {component:?}."))?;
    if definition.ecosystem != ecosystem {
        return Err(format!(
            "Frozen family component {component:?} must use ecosystem {ecosystem:?}."
        ));
    }
    Ok(definition)
}

fn metadata_header_blocks(header: &str) -> Result<Vec<(String, String)>, String> {
    let mut blocks = Vec::<(String, String)>::new();
    for line in header.lines() {
        if line.ends_with('\r') {
            return Err("Frozen Python wheel METADATA must use LF line endings.".to_string());
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            let (_, block) = blocks.last_mut().ok_or_else(|| {
                "Frozen Python wheel METADATA begins with a continuation line.".to_string()
            })?;
            block.push('\n');
            block.push_str(line);
            continue;
        }
        let (name, _) = line.split_once(':').ok_or_else(|| {
            format!("Frozen Python wheel METADATA has malformed header line {line:?}.")
        })?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(format!(
                "Frozen Python wheel METADATA has invalid field name {name:?}."
            ));
        }
        blocks.push((name.to_string(), line.to_string()));
    }
    Ok(blocks)
}

fn single_line_metadata_value<'a>(block: &'a str, field: &str) -> Result<&'a str, String> {
    if block.contains('\n') {
        return Err(format!(
            "Frozen Python wheel METADATA field {field} may not be folded."
        ));
    }
    block
        .split_once(':')
        .map(|(_, value)| value.trim())
        .ok_or_else(|| format!("Frozen Python wheel METADATA field {field} is malformed."))
}

fn rewrite_python_metadata(
    source: &[u8],
    python_version: &str,
    license_expression: &str,
    license_files: &[String],
    source_url: &str,
    documentation_url: &str,
) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(source)
        .map_err(|_| "Frozen Python wheel METADATA must be UTF-8.".to_string())?;
    let (header, _) = text
        .split_once("\n\n")
        .ok_or_else(|| "Frozen Python wheel METADATA lacks its header boundary.".to_string())?;
    let blocks = metadata_header_blocks(header)?;
    let mut output = Vec::new();
    let mut metadata_version_count = 0;
    let mut name_count = 0;
    let mut version_count = 0;
    let mut summary_count = 0;
    let mut license_expression_count = 0;
    for (name, block) in blocks {
        if name.eq_ignore_ascii_case("Metadata-Version") {
            metadata_version_count += 1;
            if single_line_metadata_value(&block, "Metadata-Version")? != "2.4" {
                return Err("Frozen Python wheel must use Metadata-Version 2.4.".to_string());
            }
            output.push(block);
        } else if name.eq_ignore_ascii_case("Name") {
            name_count += 1;
            if single_line_metadata_value(&block, "Name")? != "ait-python" {
                return Err("Frozen Python wheel METADATA Name must be ait-python.".to_string());
            }
            output.push("Name: ait-native".to_string());
        } else if name.eq_ignore_ascii_case("Version") {
            version_count += 1;
            if single_line_metadata_value(&block, "Version")? != python_version {
                return Err(
                    "Frozen Python wheel METADATA Version differs from the family.".to_string(),
                );
            }
            output.push(block);
        } else if name.eq_ignore_ascii_case("Summary") {
            summary_count += 1;
            output.push(
                "Summary: Language-neutral native AIT CLI, inactive server, and Python binding"
                    .to_string(),
            );
        } else if name.eq_ignore_ascii_case("License-Expression") {
            license_expression_count += 1;
            output.push(format!("License-Expression: {license_expression}"));
            output.extend(
                license_files
                    .iter()
                    .map(|path| format!("License-File: {path}")),
            );
        } else if !name.eq_ignore_ascii_case("License-File")
            && !name.eq_ignore_ascii_case("Project-URL")
        {
            output.push(block);
        }
    }
    if metadata_version_count != 1
        || name_count != 1
        || version_count != 1
        || summary_count != 1
        || license_expression_count != 1
    {
        return Err(
            "Frozen Python wheel METADATA must contain one metadata version, name, version, summary, and license expression."
                .to_string(),
        );
    }
    if license_files.is_empty() {
        return Err("Repacked Python wheel requires license files.".to_string());
    }
    if !source_url.starts_with("https://github.com/weita2026/ait-native/tree/")
        || !documentation_url.starts_with("https://github.com/weita2026/ait-native/blob/")
    {
        return Err("Repacked Python wheel requires exact tagged monorepo URLs.".to_string());
    }
    output.push(format!("Project-URL: Source, {source_url}"));
    output.push(format!("Project-URL: Documentation, {documentation_url}"));
    Ok(format!(
        "{}\n\nInstalls the language-neutral ait and ait-server commands plus the admitted ait-python binding. The server remains inactive until explicitly started.\n",
        output.join("\n")
    )
    .into_bytes())
}

fn rewrite_wheel_metadata(source: &[u8], tags: &str) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(source)
        .map_err(|_| "Frozen Python wheel WHEEL metadata must be UTF-8.".to_string())?;
    let mut rows = Vec::new();
    let mut wheel_version_count = 0;
    let mut root_count = 0;
    let mut tag_count = 0;
    let mut generator_count = 0;
    for raw_line in text.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(value) = line.strip_prefix("Wheel-Version: ") {
            wheel_version_count += 1;
            if value != "1.0" {
                return Err("Frozen Python wheel uses an unsupported Wheel-Version.".to_string());
            }
            rows.push(line.to_string());
        } else if let Some(value) = line.strip_prefix("Root-Is-Purelib: ") {
            root_count += 1;
            if value != "false" {
                return Err("Frozen Python wheel must be platform-specific.".to_string());
            }
            rows.push(line.to_string());
        } else if let Some(value) = line.strip_prefix("Tag: ") {
            tag_count += 1;
            if value != tags {
                return Err("Frozen Python wheel tag differs from its filename.".to_string());
            }
            rows.push(line.to_string());
        } else if line.starts_with("Generator: ") {
            generator_count += 1;
            rows.push("Generator: ait release package (family/v1)".to_string());
        } else if !line.is_empty() {
            rows.push(line.to_string());
        }
    }
    if wheel_version_count != 1 || root_count != 1 || tag_count != 1 || generator_count != 1 {
        return Err(
            "Frozen Python wheel WHEEL metadata must contain one version, generator, root, and tag row."
                .to_string(),
        );
    }
    Ok((rows.join("\n") + "\n").into_bytes())
}

fn pypi_wheel_identity(
    artifact: &FrozenComponentArtifact,
    python_version: &str,
    target: &str,
) -> Result<(String, String, String, String), String> {
    let source_filename = Path::new(&artifact.path)
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| "Frozen Python wheel has no portable filename.".to_string())?;
    let prefix = format!("ait_python-{python_version}-");
    let tags = source_filename
        .strip_prefix(&prefix)
        .and_then(|suffix| suffix.strip_suffix(".whl"))
        .ok_or_else(|| {
            format!(
                "Frozen Python wheel filename {source_filename:?} does not match ait-python {python_version}."
            )
        })?;
    if tags.split('-').count() != 3
        || !tags
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("Frozen Python wheel tags {tags:?} are invalid."));
    }
    let mut tag_parts = tags.split('-');
    let python_tag = tag_parts.next().unwrap_or_default();
    let abi_tag = tag_parts.next().unwrap_or_default();
    let platform_tag = tag_parts.next().unwrap_or_default();
    if python_tag != "cp311" || abi_tag != "abi3" {
        return Err(format!(
            "Frozen Python wheel {source_filename:?} must use the product cp311-abi3 contract."
        ));
    }
    let every_platform_tag = |prefix: &str, suffix: &str| {
        platform_tag
            .split('.')
            .all(|tag| tag.starts_with(prefix) && tag.ends_with(suffix))
    };
    let platform_matches = match target {
        "aarch64-apple-darwin" => every_platform_tag("macosx_", "_arm64"),
        "x86_64-apple-darwin" => every_platform_tag("macosx_", "_x86_64"),
        "aarch64-unknown-linux-gnu" => every_platform_tag("manylinux", "_aarch64"),
        "x86_64-unknown-linux-gnu" => every_platform_tag("manylinux", "_x86_64"),
        "aarch64-pc-windows-msvc" => platform_tag == "win_arm64",
        "x86_64-pc-windows-msvc" => platform_tag == "win_amd64",
        _ => false,
    };
    if !platform_matches {
        return Err(format!(
            "Frozen Python wheel platform tag {platform_tag:?} does not match family target {target:?}."
        ));
    }
    Ok((
        source_filename.to_string(),
        format!("ait_native-{python_version}-{tags}.whl"),
        format!("ait_python-{python_version}.dist-info"),
        format!("ait_native-{python_version}.dist-info"),
    ))
}

fn assemble_pypi(
    repo: &RepoRuntime,
    input: &FamilyPackageInput,
) -> Result<Vec<GeneratedArtifact>, String> {
    let distribution = single_channel_distribution(input, "pypi")?;
    if distribution.role != "product" || distribution.identity != "ait-native" {
        return Err("PyPI distribution must be the ait-native product.".to_string());
    }
    require_distribution_components(distribution, &["ait", "ait-server", "ait-python"])?;
    require_registry_targets(distribution)?;
    let python_component = required_component(input, "ait-python", "python")?;
    required_component(input, "ait", "native")?;
    required_component(input, "ait-server", "native")?;
    let license_expression = winget_license_expression(input, distribution)?;
    let mut generated = Vec::new();
    for target in &distribution.targets {
        let source_wheel = component_artifact(input, "ait-python", target, "python-wheel")?;
        let source_bytes = read_frozen_bytes(
            repo,
            &source_wheel.path,
            source_wheel.size_bytes,
            &source_wheel.sha256,
            "Frozen ait-python wheel",
        )?;
        let (source_filename, output_filename, source_dist_info, output_dist_info) =
            pypi_wheel_identity(&source_wheel, &python_component.version, target)?;
        let source_record = format!("{source_dist_info}/RECORD");
        let output_record = format!("{output_dist_info}/RECORD");
        let mut source_entries = read_wheel_entries(&source_bytes)?;
        validate_wheel_record(&source_entries, &source_record)?;
        if !source_entries.contains_key(&format!("{source_dist_info}/METADATA"))
            || !source_entries.contains_key(&format!("{source_dist_info}/WHEEL"))
            || !source_entries.keys().any(|path| {
                path.starts_with("ait_py/ait_py")
                    && path.ends_with(if target.ends_with("windows-msvc") {
                        ".pyd"
                    } else {
                        ".so"
                    })
            })
        {
            return Err(format!(
                "Frozen ait-python wheel {source_filename:?} lacks required binding metadata or extension."
            ));
        }
        let foreign_dist_info = source_entries.keys().any(|path| {
            path.split_once('/')
                .map(|(root, _)| root.ends_with(".dist-info") && root != source_dist_info)
                .unwrap_or(false)
        });
        if foreign_dist_info {
            return Err(
                "Frozen ait-python wheel contains more than one dist-info root.".to_string(),
            );
        }
        let source_metadata = source_entries
            .get(&format!("{source_dist_info}/METADATA"))
            .map(|entry| entry.0.clone())
            .ok_or_else(|| "Frozen ait-python wheel is missing METADATA.".to_string())?;
        let source_wheel_metadata = source_entries
            .get(&format!("{source_dist_info}/WHEEL"))
            .map(|entry| entry.0.clone())
            .ok_or_else(|| "Frozen ait-python wheel is missing WHEEL metadata.".to_string())?;
        let tags = output_filename
            .strip_prefix(&format!("ait_native-{}-", python_component.version))
            .and_then(|suffix| suffix.strip_suffix(".whl"))
            .ok_or_else(|| "Generated Python wheel filename is malformed.".to_string())?;

        source_entries.remove(&source_record);
        let mut output_entries = PackageEntries::new();
        for (path, value) in source_entries {
            let output_path =
                if let Some(relative) = path.strip_prefix(&format!("{source_dist_info}/")) {
                    if relative == "METADATA" {
                        continue;
                    }
                    if relative == "WHEEL" {
                        continue;
                    }
                    if relative.starts_with("licenses/") {
                        continue;
                    }
                    format!("{output_dist_info}/{relative}")
                } else {
                    path
                };
            if output_entries.insert(output_path.clone(), value).is_some() {
                return Err(format!(
                    "Repacked Python wheel destination collides at {output_path:?}."
                ));
            }
        }

        let materials = material_for_components(input, &distribution.components)?;
        let mut material_projections = Vec::new();
        let mut license_files = Vec::new();
        for material in materials {
            let license_relative =
                format!("{}/{}", material.source_repository, material.declared_path);
            let destination = format!("{output_dist_info}/licenses/{license_relative}");
            let bytes = read_frozen_bytes(
                repo,
                &material.path,
                material.size_bytes,
                &material.sha256,
                "Frozen PyPI license material",
            )?;
            if output_entries
                .insert(destination.clone(), (bytes, 0o644))
                .is_some()
            {
                return Err(format!(
                    "Repacked Python wheel legal destination collides at {destination:?}."
                ));
            }
            license_files.push(license_relative);
            material_projections.push(MaterialProjection {
                source: material,
                destination,
            });
        }
        output_entries.insert(
            format!("{output_dist_info}/METADATA"),
            (
                rewrite_python_metadata(
                    &source_metadata,
                    &python_component.version,
                    &license_expression,
                    &license_files,
                    &public_source_root(input)?,
                    &format!(
                        "https://github.com/{}/blob/{}/docs/distribution.md",
                        github_source_identity(input)?,
                        input.tag
                    ),
                )?,
                0o644,
            ),
        );
        output_entries.insert(
            format!("{output_dist_info}/WHEEL"),
            (rewrite_wheel_metadata(&source_wheel_metadata, tags)?, 0o644),
        );

        let mut content = vec![ContentProjection {
            source: source_wheel.clone(),
            destination: output_filename.clone(),
        }];
        for component in ["ait", "ait-server"] {
            let source = component_artifact(input, component, target, "native-executable")?;
            let command = component_command(component, target)?;
            let destination = format!(
                "ait_native-{}.data/scripts/{command}",
                python_component.version
            );
            let bytes = read_frozen_bytes(
                repo,
                &source.path,
                source.size_bytes,
                &source.sha256,
                "Frozen PyPI native command",
            )?;
            if output_entries
                .insert(destination.clone(), (bytes, 0o755))
                .is_some()
            {
                return Err(format!(
                    "Repacked Python wheel command destination collides at {destination:?}."
                ));
            }
            content.push(ContentProjection {
                source,
                destination,
            });
        }
        let provenance_path = format!("{output_dist_info}/ait-family-provenance.json");
        output_entries.insert(
            provenance_path,
            (
                package_provenance(
                    input,
                    distribution,
                    Some(target),
                    &content,
                    &material_projections,
                )?,
                0o644,
            ),
        );
        let record_bytes = generated_wheel_record(&output_entries, &output_record)?;
        output_entries.insert(output_record.clone(), (record_bytes, 0o644));
        validate_wheel_record(&output_entries, &output_record)?;
        let binding_member_count = output_entries
            .keys()
            .filter(|path| path.starts_with("ait_py/") || path.starts_with("ait_python/"))
            .count();
        let bytes = wheel_zip_bytes(&output_entries, &output_record)?;
        generated.push(GeneratedArtifact {
            relative_path: format!("wheels/{output_filename}"),
            bytes,
            evidence: artifact_evidence(
                "python-wheel",
                distribution,
                Some(target),
                &content,
                &material_projections,
                json!({
                    "distribution": "ait-native",
                    "python_version": python_component.version,
                    "source_binding_distribution": "ait-python",
                    "source_wheel_filename": source_filename,
                    "source_wheel_sha256": source_wheel.sha256,
                    "source_wheel_size_bytes": source_wheel.size_bytes,
                    "binding_member_count": binding_member_count,
                    "wheel_record_regenerated": true,
                    "native_script_count": 2,
                    "server_activation": "inactive",
                }),
                input,
            )?,
        });
    }
    Ok(generated)
}

#[derive(Clone, Debug)]
struct NpmPayloadDefinition {
    target: String,
    os: String,
    cpu: String,
    component: String,
    package: String,
    source_repository: String,
    license: String,
    executable: String,
}

fn npm_platform(target: &str) -> Result<(&'static str, &'static str), String> {
    match target {
        "aarch64-apple-darwin" => Ok(("darwin", "arm64")),
        "x86_64-apple-darwin" => Ok(("darwin", "x64")),
        "aarch64-unknown-linux-gnu" => Ok(("linux", "arm64")),
        "x86_64-unknown-linux-gnu" => Ok(("linux", "x64")),
        "aarch64-pc-windows-msvc" => Ok(("win32", "arm64")),
        "x86_64-pc-windows-msvc" => Ok(("win32", "x64")),
        _ => Err(format!("npm does not support family target {target:?}.")),
    }
}

fn exact_json_fields(value: &JsonValue, expected: &[&str], context: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object."))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!(
            "{context} fields do not match the frozen npm contract."
        ));
    }
    Ok(())
}

fn parse_npm_payload_contract(
    input: &FamilyPackageInput,
    value: &JsonValue,
) -> Result<Vec<NpmPayloadDefinition>, String> {
    exact_json_fields(
        value,
        &["schema", "family_version", "top_level_package", "payloads"],
        "Frozen npm payload contract",
    )?;
    if string_field(value, "schema").as_deref() != Some("ait.node.npm-platform-packages/v1")
        || string_field(value, "family_version").as_deref() != Some(input.version.as_str())
        || string_field(value, "top_level_package").as_deref() != Some("ait-native")
    {
        return Err("Frozen npm payload contract identity or version drifted.".to_string());
    }
    let rows = value
        .get("payloads")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "Frozen npm payload contract lacks payloads.".to_string())?;
    if rows.len() != 12 {
        return Err(
            "Frozen npm payload contract must declare exactly twelve payloads.".to_string(),
        );
    }
    let mut definitions = Vec::new();
    let mut packages = BTreeSet::new();
    let mut selections = BTreeSet::new();
    for (index, row) in rows.iter().enumerate() {
        exact_json_fields(
            row,
            &[
                "target",
                "os",
                "cpu",
                "component",
                "command",
                "package",
                "version",
                "source_repository",
                "license",
                "executable",
            ],
            &format!("Frozen npm payload row {index}"),
        )?;
        let target = required_string_field(row, "target")?;
        let component = required_string_field(row, "component")?;
        let (os, cpu) = npm_platform(&target)?;
        let (command, package_prefix) = match component.as_str() {
            "ait" => ("ait", "ait"),
            "ait-server" => ("ait-server", "server"),
            _ => {
                return Err(format!(
                    "Frozen npm payload row {index} has invalid component."
                ))
            }
        };
        let component_definition = required_component(input, &component, "native")?;
        let executable = if os == "win32" {
            format!("bin/{command}.exe")
        } else {
            format!("bin/{command}")
        };
        let package = format!("ait-native-{package_prefix}-{os}-{cpu}");
        if required_string_field(row, "os")? != os
            || required_string_field(row, "cpu")? != cpu
            || required_string_field(row, "command")? != command
            || required_string_field(row, "package")? != package
            || required_string_field(row, "version")? != input.version
            || required_string_field(row, "source_repository")?
                != component_definition.source_repository
            || required_string_field(row, "license")? != component_definition.license
            || required_string_field(row, "executable")? != executable
        {
            return Err(format!(
                "Frozen npm payload row {index} differs from its family component/target mapping."
            ));
        }
        if !packages.insert(package.clone())
            || !selections.insert((component.clone(), target.clone()))
        {
            return Err("Frozen npm payload contract contains a duplicate selection.".to_string());
        }
        definitions.push(NpmPayloadDefinition {
            target,
            os: os.to_string(),
            cpu: cpu.to_string(),
            component,
            package,
            source_repository: component_definition.source_repository.clone(),
            license: component_definition.license.clone(),
            executable,
        });
    }
    let expected = REGISTRY_TARGETS
        .iter()
        .flat_map(|target| {
            ["ait", "ait-server"]
                .into_iter()
                .map(move |component| (component.to_string(), (*target).to_string()))
        })
        .collect::<BTreeSet<_>>();
    if selections != expected {
        return Err(
            "Frozen npm payload contract does not cover the exact twelve selections.".to_string(),
        );
    }
    Ok(definitions)
}

fn validate_npm_envelope(
    input: &FamilyPackageInput,
    entries: &PackageEntries,
    node_materials: &[(FrozenLicenseMaterial, Vec<u8>)],
) -> Result<Vec<NpmPayloadDefinition>, String> {
    let expected_paths = BTreeSet::from([
        "package/LICENSE".to_string(),
        "package/NOTICE".to_string(),
        "package/bin/ait-server.mjs".to_string(),
        "package/bin/ait.mjs".to_string(),
        "package/bin/launch.mjs".to_string(),
        "package/lib/npm-payload-contract.json".to_string(),
        "package/package.json".to_string(),
    ]);
    if entries.keys().cloned().collect::<BTreeSet<_>>() != expected_paths {
        return Err(
            "Frozen npm envelope inventory differs from the command-only contract.".to_string(),
        );
    }
    for (material, bytes) in node_materials {
        let path = format!("package/{}", material.declared_path);
        if entries.get(&path).map(|entry| entry.0.as_slice()) != Some(bytes.as_slice()) {
            return Err(format!(
                "Frozen npm envelope does not embed exact ait-node {} bytes.",
                material.material_role
            ));
        }
    }
    let package_json = parse_slice_value(
        &entries["package/package.json"].0,
        "Frozen npm envelope package.json must contain valid JSON",
    )?;
    exact_json_fields(
        &package_json,
        &[
            "name",
            "version",
            "description",
            "license",
            "type",
            "engines",
            "bin",
            "exports",
            "files",
            "optionalDependencies",
            "scripts",
        ],
        "Frozen npm envelope package.json",
    )?;
    let node_component = required_component(input, "ait-node", "node")?;
    if string_field(&package_json, "name").as_deref() != Some("ait-native")
        || string_field(&package_json, "version").as_deref() != Some(input.version.as_str())
        || string_field(&package_json, "description").is_none_or(|value| value.is_empty())
        || string_field(&package_json, "license").as_deref()
            != Some(node_component.license.as_str())
        || string_field(&package_json, "type").as_deref() != Some("module")
        || !package_json
            .get("exports")
            .and_then(JsonValue::as_object)
            .is_some_and(JsonMap::is_empty)
    {
        return Err(
            "Frozen npm envelope package.json is not one portable command-only product."
                .to_string(),
        );
    }
    let engines = package_json
        .get("engines")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Frozen npm envelope package.json lacks engines.".to_string())?;
    if engines.len() != 1 || engines.get("node").and_then(JsonValue::as_str) != Some(">=20") {
        return Err("Frozen npm envelope must require the exact Node.js >=20 runtime.".to_string());
    }
    let files = json_array_strings(package_json.get("files"), "npm package.json.files")?;
    if files.len() != 4
        || files.into_iter().collect::<BTreeSet<_>>()
            != BTreeSet::from([
                "LICENSE".to_string(),
                "NOTICE".to_string(),
                "bin".to_string(),
                "lib".to_string(),
            ])
    {
        return Err(
            "Frozen npm envelope files differ from its command-only inventory.".to_string(),
        );
    }
    let bin = package_json
        .get("bin")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Frozen npm envelope package.json lacks command bins.".to_string())?;
    if bin.len() != 2
        || bin.get("ait").and_then(JsonValue::as_str) != Some("bin/ait.mjs")
        || bin.get("ait-server").and_then(JsonValue::as_str) != Some("bin/ait-server.mjs")
    {
        return Err("Frozen npm envelope must expose only ait and ait-server.".to_string());
    }
    let scripts = package_json
        .get("scripts")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Frozen npm envelope package.json lacks validation scripts.".to_string())?;
    if scripts.keys().map(String::as_str).collect::<BTreeSet<_>>()
        != BTreeSet::from(["check", "test"])
        || scripts
            .values()
            .any(|value| value.as_str().is_none_or(str::is_empty))
    {
        return Err(
            "Frozen npm envelope may contain only non-empty check and test scripts.".to_string(),
        );
    }
    for hook in ["preinstall", "install", "postinstall", "prepack"] {
        if scripts.contains_key(hook) {
            return Err(format!(
                "Frozen npm envelope contains forbidden lifecycle hook {hook:?}."
            ));
        }
    }
    let contract = parse_slice_value(
        &entries["package/lib/npm-payload-contract.json"].0,
        "Frozen npm payload contract must contain valid JSON",
    )?;
    let payloads = parse_npm_payload_contract(input, &contract)?;
    let optional_dependencies = package_json
        .get("optionalDependencies")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Frozen npm envelope lacks optional payload dependencies.".to_string())?;
    if optional_dependencies.len() != payloads.len()
        || payloads.iter().any(|payload| {
            optional_dependencies
                .get(&payload.package)
                .and_then(JsonValue::as_str)
                != Some(input.version.as_str())
        })
    {
        return Err(
            "Frozen npm envelope optionalDependencies differ from its payload contract."
                .to_string(),
        );
    }
    let launcher = std::str::from_utf8(&entries["package/bin/launch.mjs"].0)
        .map_err(|_| "Frozen npm launcher must be UTF-8 JavaScript.".to_string())?
        .to_ascii_lowercase();
    for forbidden in [
        "http://",
        "https://",
        "fetch(",
        "napi",
        ".node",
        "pyproject.toml",
        "composer.json",
        "pom.xml",
        "cmakelists.txt",
        ".csproj",
    ] {
        if launcher.contains(forbidden) {
            return Err(format!(
                "Frozen npm launcher contains forbidden download, addon, or project-detection marker {forbidden:?}."
            ));
        }
    }
    Ok(payloads)
}

fn assemble_npm(
    repo: &RepoRuntime,
    input: &FamilyPackageInput,
) -> Result<Vec<GeneratedArtifact>, String> {
    let distribution = single_channel_distribution(input, "npm")?;
    if distribution.role != "product" || distribution.identity != "ait-native" {
        return Err("npm distribution must be the ait-native product.".to_string());
    }
    require_distribution_components(distribution, &["ait", "ait-server", "ait-node"])?;
    require_registry_targets(distribution)?;
    required_component(input, "ait", "native")?;
    required_component(input, "ait-server", "native")?;
    let node_component = required_component(input, "ait-node", "node")?;
    if node_component.version != input.version {
        return Err("ait-node version must equal the family version.".to_string());
    }

    let envelope = portable_component_artifact(input, "ait-node", "npm-cli-envelope")?;
    let envelope_bytes = read_frozen_bytes(
        repo,
        &envelope.path,
        envelope.size_bytes,
        &envelope.sha256,
        "Frozen ait-node npm envelope",
    )?;
    let envelope_filename = Path::new(&envelope.path)
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| "Frozen npm envelope has no portable filename.".to_string())?;
    if envelope_filename != format!("ait-native-{}.tgz", input.version) {
        return Err("Frozen npm envelope filename differs from the family version.".to_string());
    }
    let node_material_sources = material_for_components(input, &["ait-node".to_string()])?;
    let mut node_materials = Vec::new();
    let mut envelope_material_projections = Vec::new();
    for material in node_material_sources {
        let bytes = read_frozen_bytes(
            repo,
            &material.path,
            material.size_bytes,
            &material.sha256,
            "Frozen ait-node npm legal material",
        )?;
        envelope_material_projections.push(MaterialProjection {
            destination: format!("package/{}", material.declared_path),
            source: material.clone(),
        });
        node_materials.push((material, bytes));
    }
    let envelope_entries = read_npm_envelope_entries(&envelope_bytes)?;
    let payloads = validate_npm_envelope(input, &envelope_entries, &node_materials)?;
    let envelope_content = vec![ContentProjection {
        source: envelope.clone(),
        destination: envelope_filename.to_string(),
    }];
    let mut generated = vec![GeneratedArtifact {
        relative_path: format!("packages/{envelope_filename}"),
        bytes: envelope_bytes,
        evidence: artifact_evidence(
            "npm-cli-envelope",
            distribution,
            None,
            &envelope_content,
            &envelope_material_projections,
            json!({
                "package": "ait-native",
                "version": input.version,
                "preserved_frozen_bytes": true,
                "command_only": true,
                "commands": ["ait", "ait-server"],
                "payload_count": 12,
                "api_surface": false,
                "native_addon": false,
                "install_hook": false,
                "runtime_download": false,
            }),
            input,
        )?,
    }];

    for payload in payloads {
        let source = component_artifact(
            input,
            &payload.component,
            &payload.target,
            "native-executable",
        )?;
        let source_bytes = read_frozen_bytes(
            repo,
            &source.path,
            source.size_bytes,
            &source.sha256,
            "Frozen npm payload executable",
        )?;
        let content = vec![ContentProjection {
            source: source.clone(),
            destination: payload.executable.clone(),
        }];
        let material_sources =
            material_for_components(input, std::slice::from_ref(&payload.component))?;
        let mut material_projections = Vec::new();
        let mut entries = PackageEntries::new();
        entries.insert(
            format!("package/{}", payload.executable),
            (source_bytes, 0o755),
        );
        for material in material_sources {
            let destination = material.declared_path.clone();
            let bytes = read_frozen_bytes(
                repo,
                &material.path,
                material.size_bytes,
                &material.sha256,
                "Frozen npm payload legal material",
            )?;
            if entries
                .insert(format!("package/{destination}"), (bytes, 0o644))
                .is_some()
            {
                return Err(format!(
                    "npm payload legal destination collides at {destination:?}."
                ));
            }
            material_projections.push(MaterialProjection {
                source: material,
                destination,
            });
        }
        let package_json = json!({
            "name": payload.package,
            "version": input.version,
            "description": format!("Implementation-only {} payload for {}", payload.component, payload.target),
            "license": payload.license,
            "homepage": public_source_subtree_url(input, &payload.source_repository)?,
            "repository": {
                "type": "git",
                "url": format!("git+https://github.com/{}.git", github_source_identity(input)?),
                "directory": payload.source_repository,
            },
            "os": [payload.os],
            "cpu": [payload.cpu],
            "files": ["bin", "provenance.json", "LICENSE", "NOTICE"],
            "aitNativePayload": {
                "schema": "ait.node.npm-platform-payload/v1",
                "component": payload.component,
                "target": payload.target,
                "executable": payload.executable,
                "source_repository": payload.source_repository,
                "source_snapshot": input.components[&payload.component].source_snapshot,
            },
        });
        entries.insert(
            "package/package.json".to_string(),
            (
                encode_value_pretty_with_newline_error_string(&package_json)?.into_bytes(),
                0o644,
            ),
        );
        let provenance = json!({
            "schema": "ait.node.npm-platform-payload-provenance/v1",
            "family_release_id": input.release_id,
            "family_version": input.version,
            "family_manifest_sha256": input.family_manifest_sha256,
            "frozen_manifest_sha256": input.frozen_manifest_sha256,
            "frozen_checksum_sha256": input.frozen_checksum_sha256,
            "package": payload.package,
            "target": payload.target,
            "os": payload.os,
            "cpu": payload.cpu,
            "component": payload.component,
            "source_repository": payload.source_repository,
            "source_snapshot": input.components[&payload.component].source_snapshot,
            "license": payload.license,
            "source_artifact": {
                "path": source.path,
                "sha256": source.sha256,
                "size_bytes": source.size_bytes,
            },
            "installed_path": payload.executable,
            "license_material": material_evidence(input, &material_projections)?,
            "component_rebuild": false,
            "registry_write": false,
        });
        entries.insert(
            "package/provenance.json".to_string(),
            (
                encode_value_pretty_with_newline_error_string(&provenance)?.into_bytes(),
                0o644,
            ),
        );
        let bytes = tar_gz_bytes(&entries, input.epoch)?;
        let filename = format!("{}-{}.tgz", payload.package, input.version);
        generated.push(GeneratedArtifact {
            relative_path: format!("packages/{filename}"),
            bytes,
            evidence: artifact_evidence(
                "npm-platform-payload",
                distribution,
                Some(&payload.target),
                &content,
                &material_projections,
                json!({
                    "package": payload.package,
                    "version": input.version,
                    "os": payload.os,
                    "cpu": payload.cpu,
                    "implementation_only": true,
                    "public_command": false,
                    "api_surface": false,
                    "native_addon": false,
                    "install_hook": false,
                    "runtime_download": false,
                }),
                input,
            )?,
        });
    }
    if generated.len() != 13 {
        return Err(
            "npm assembly must produce one envelope and twelve payload tarballs.".to_string(),
        );
    }
    Ok(generated)
}

fn distribution_json(distribution: &DistributionDefinition) -> JsonValue {
    json!({
        "channel": distribution.channel,
        "role": distribution.role,
        "identity": distribution.identity,
        "components": distribution.components,
        "targets": distribution.targets,
    })
}

fn channel_route(input: &FamilyPackageInput, channel: &str) -> JsonValue {
    match channel {
        "homebrew" => json!({
            "channel": if input.release_channel == "rc" { "rc" } else { "stable" },
            "stable_formula_mutation": input.release_channel == "stable",
        }),
        "apt" => json!({
            "suite": if input.release_channel == "rc" { "testing" } else { "stable" },
        }),
        "winget" => json!({
            "route": if input.release_channel == "rc" { "validation" } else { "community" },
            "community_manifest_submission": input.release_channel == "stable",
        }),
        "pypi" => json!({
            "repository": "pypi",
            "prerelease": input.release_channel == "rc",
        }),
        "npm" => json!({
            "dist_tag": if input.release_channel == "rc" { "rc" } else { "latest" },
        }),
        _ => json!({}),
    }
}

fn package_root_relative(release_id: &str, channel: &str) -> String {
    format!("dist/{release_id}/packages/{channel}")
}

fn exact_output_files(root: &Path) -> Result<BTreeSet<String>, String> {
    let root_metadata = fs::symlink_metadata(root).map_err(io_error)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err("Family channel package root must be a real directory.".to_string());
    }
    let mut files = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "Family channel package output contains a symbolic link: {}.",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|error| error.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                safe_relative_path(&relative, "Family channel output path")?;
                files.insert(relative);
            } else {
                return Err(format!(
                    "Family channel package output contains an unsupported filesystem entry: {}.",
                    path.display()
                ));
            }
        }
    }
    Ok(files)
}

fn validate_output_bytes(root: &Path, expected: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let actual_files = exact_output_files(root)?;
    let expected_files = expected.keys().cloned().collect::<BTreeSet<_>>();
    if actual_files != expected_files {
        return Err(format!(
            "Existing family channel package inventory differs (missing: {:?}; extra: {:?}).",
            expected_files.difference(&actual_files).collect::<Vec<_>>(),
            actual_files.difference(&expected_files).collect::<Vec<_>>()
        ));
    }
    for (relative, expected_bytes) in expected {
        let path = root.join(safe_relative_path(relative, "Expected output path")?);
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "Family channel output {relative:?} is not a regular file."
            ));
        }
        let actual = fs::read(&path).map_err(io_error)?;
        if actual != *expected_bytes {
            return Err(format!(
                "Existing family channel output {relative:?} differs from deterministic assembly."
            ));
        }
    }
    Ok(())
}

fn write_family_package_output(
    repo: &RepoRuntime,
    input: &FamilyPackageInput,
    channel: &str,
    mut generated: Vec<GeneratedArtifact>,
) -> Result<JsonValue, String> {
    if generated.is_empty() {
        return Err(format!("{channel} assembly produced no package artifacts."));
    }
    generated.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let mut seen_paths = BTreeSet::new();
    let final_prefix = package_root_relative(&input.release_id, channel);
    let mut artifact_rows = Vec::new();
    let mut package_files = BTreeMap::new();
    for artifact in generated {
        safe_relative_path(&artifact.relative_path, "Generated package path")?;
        if !seen_paths.insert(artifact.relative_path.clone()) {
            return Err(format!(
                "Channel assembly produced duplicate output path {:?}.",
                artifact.relative_path
            ));
        }
        let mut evidence = artifact.evidence;
        let object = evidence
            .as_object_mut()
            .ok_or_else(|| "Generated artifact evidence must be an object.".to_string())?;
        object.insert(
            "path".to_string(),
            json!(format!("{final_prefix}/{}", artifact.relative_path)),
        );
        object.insert("sha256".to_string(), json!(sha256_hex(&artifact.bytes)));
        object.insert("size_bytes".to_string(), json!(artifact.bytes.len()));
        artifact_rows.push(evidence);
        package_files.insert(artifact.relative_path, artifact.bytes);
    }
    let selected_distributions = input
        .distributions
        .iter()
        .filter(|distribution| distribution.channel == channel)
        .map(distribution_json)
        .collect::<Vec<_>>();
    let created_at = input.epoch.to_string();
    let receipt = json!({
        "contract": FAMILY_PACKAGE_CONTRACT,
        "command": "release package",
        "release_id": input.release_id,
        "version": input.version,
        "release_channel": input.release_channel,
        "channel": channel,
        "tag": input.tag,
        "snapshot_id": input.snapshot_id,
        "status": "assembled",
        "source_date_epoch": input.epoch,
        "family_manifest_sha256": input.family_manifest_sha256,
        "frozen_manifest_sha256": input.frozen_manifest_sha256,
        "frozen_checksum_sha256": input.frozen_checksum_sha256,
        "distributions": selected_distributions,
        "route": channel_route(input, channel),
        "artifacts": artifact_rows,
        "artifact_count": package_files.len(),
        "checksum_path": format!("{final_prefix}/{PACKAGE_CHECKSUM_FILENAME}"),
        "checks": [
            {"check_id": "frozen_family", "status": "pass", "blocking": false},
            {"check_id": "component_digest_preservation", "status": "pass", "blocking": false},
            {"check_id": "license_material", "status": "pass", "blocking": false},
            {"check_id": "package_inventory", "status": "pass", "blocking": false},
            {"check_id": "deterministic_assembly", "status": "pass", "blocking": false},
            {"check_id": "public_mutation", "status": "pass", "blocking": false},
        ],
        "check_summary": {
            "total": 6,
            "passed": 6,
            "failed": 0,
            "blocking": 0,
            "decision": "pass",
        },
        "mutation": {
            "component_rebuild": false,
            "credentials_loaded": false,
            "signing": false,
            "tag_write": false,
            "registry_write": false,
            "public_publish": false,
            "service_start": false,
            "service_enable": false,
            "service_registration": false,
            "server_authority_initialization": false,
        },
        "created_at": created_at,
        "updated_at": created_at,
        "next_action": {
            "code": "protected_channel_validation",
            "detail": "Validate and sign these exact bytes in protected CI, then publish without rebuilding only after explicit endpoint approval.",
        },
    });
    let receipt_bytes = encode_value_pretty_with_newline_error_string(&receipt)?.into_bytes();
    package_files.insert(PACKAGE_RECEIPT_FILENAME.to_string(), receipt_bytes);
    let checksum_text = package_files
        .iter()
        .map(|(path, bytes)| format!("{}  {path}", sha256_hex(bytes)))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    package_files.insert(
        PACKAGE_CHECKSUM_FILENAME.to_string(),
        checksum_text.into_bytes(),
    );

    let final_root = repo
        .workspace_root()
        .join("dist")
        .join(&input.release_id)
        .join("packages")
        .join(channel);
    if fs::symlink_metadata(&final_root).is_ok() {
        validate_output_bytes(&final_root, &package_files)?;
        return Ok(receipt);
    }
    let parent = final_root
        .parent()
        .ok_or_else(|| "Family package output has no parent directory.".to_string())?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let staging = TempDirBuilder::new()
        .prefix(".ait-family-package-")
        .tempdir_in(parent)
        .map_err(io_error)?;
    for (relative, bytes) in &package_files {
        let path = staging
            .path()
            .join(safe_relative_path(relative, "Package staging path")?);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        fs::write(&path, bytes).map_err(io_error)?;
    }
    fs::rename(staging.path(), &final_root).map_err(|error| {
        format!("Failed to atomically activate {channel} family package output: {error}")
    })?;
    drop(staging);
    validate_output_bytes(&final_root, &package_files)?;
    Ok(receipt)
}

pub fn family_release_package(
    repo: &RepoRuntime,
    release_id: &str,
    channel: &str,
) -> Result<JsonValue, String> {
    let input = parse_family_package_input(repo, release_id, channel)?;
    let generated = match channel {
        "homebrew" => assemble_homebrew(repo, &input)?,
        "apt" => assemble_apt(repo, &input)?,
        "winget" => assemble_winget(repo, &input)?,
        "pypi" => assemble_pypi(repo, &input)?,
        "npm" => assemble_npm(repo, &input)?,
        _ => return Err(format!("Unsupported family package channel {channel:?}.")),
    };
    write_family_package_output(repo, &input, channel, generated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_archive_readers_reject_unsafe_members() {
        let cursor = Cursor::new(Vec::new());
        let mut wheel = ZipWriter::new(cursor);
        wheel
            .start_file(
                "../escape",
                FileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        wheel.write_all(b"escape").unwrap();
        let wheel = wheel.finish().unwrap().into_inner();
        assert!(read_wheel_entries(&wheel)
            .unwrap_err()
            .contains("unsafe path"));

        let encoder = GzBuilder::new().write(Vec::new(), Compression::default());
        let mut npm = TarBuilder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        npm.append_data(&mut header, "package", Cursor::new([]))
            .unwrap();
        let npm = npm.into_inner().unwrap().finish().unwrap();
        assert!(read_npm_envelope_entries(&npm)
            .unwrap_err()
            .contains("regular files only"));
    }

    #[test]
    fn pypi_wheel_identity_requires_exact_abi_and_target_platform() {
        let artifact = FrozenComponentArtifact {
            component: "ait-python".to_string(),
            kind: "python-wheel".to_string(),
            target: Some("aarch64-apple-darwin".to_string()),
            path: "ait_python-1.0.0rc1-cp311-abi3-macosx_11_0_arm64.whl".to_string(),
            sha256: "a".repeat(64),
            size_bytes: 1,
        };
        assert!(pypi_wheel_identity(&artifact, "1.0.0rc1", "aarch64-apple-darwin").is_ok());
        assert!(
            pypi_wheel_identity(&artifact, "1.0.0rc1", "x86_64-apple-darwin")
                .unwrap_err()
                .contains("does not match family target")
        );

        let wrong_abi = FrozenComponentArtifact {
            path: "ait_python-1.0.0rc1-cp311-cp311-macosx_11_0_arm64.whl".to_string(),
            ..artifact
        };
        assert!(
            pypi_wheel_identity(&wrong_abi, "1.0.0rc1", "aarch64-apple-darwin")
                .unwrap_err()
                .contains("cp311-abi3")
        );
    }
}
