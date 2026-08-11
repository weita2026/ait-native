[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$Mode = "patchset"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$supportedModes = @("patchset", "repo", "all")
if ($Mode -notin $supportedModes) {
    [Console]::Error.WriteLine("usage: ./ci/run.ps1 {patchset|repo|all}")
    exit 64
}

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,
        [Parameter(Mandatory = $true)]
        [string[]]$ArgumentList
    )

    & $FilePath @ArgumentList
    $nativeExitCode = $LASTEXITCODE
    if ($nativeExitCode -ne 0) {
        throw "native command failed with exit code ${nativeExitCode}: $FilePath"
    }
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$runtimeParent = $env:AIT_RUNNER_ATTEMPT_ROOT
if ([string]::IsNullOrWhiteSpace($runtimeParent)) {
    $runtimeParent = [System.IO.Path]::GetTempPath()
}
[void][System.IO.Directory]::CreateDirectory($runtimeParent)

$ciLeaf = "ait-python-ci." + [Guid]::NewGuid().ToString("N")
$ciRoot = Join-Path $runtimeParent $ciLeaf
[void][System.IO.Directory]::CreateDirectory($ciRoot)
$previousLocation = (Get-Location).Path

try {
    foreach ($relativePath in @(
        "tmp",
        "cache/pip",
        "cache/cargo",
        "cache/python",
        "cargo-target",
        "cargo-build"
    )) {
        [void][System.IO.Directory]::CreateDirectory(
            (Join-Path $ciRoot $relativePath)
        )
    }

    $env:TMPDIR = Join-Path $ciRoot "tmp"
    $env:TMP = $env:TMPDIR
    $env:TEMP = $env:TMPDIR
    $env:XDG_CACHE_HOME = Join-Path $ciRoot "cache"
    $env:PIP_CACHE_DIR = Join-Path $ciRoot "cache/pip"
    $env:PIP_NO_CACHE_DIR = "1"
    $env:PIP_DISABLE_PIP_VERSION_CHECK = "1"
    $env:PYTHONPYCACHEPREFIX = Join-Path $ciRoot "cache/python"
    $env:PYTHONDONTWRITEBYTECODE = "1"
    $env:CARGO_HOME = Join-Path $ciRoot "cache/cargo"
    $env:CARGO_TARGET_DIR = Join-Path $ciRoot "cargo-target"
    $env:CARGO_BUILD_BUILD_DIR = Join-Path $ciRoot "cargo-build/{workspace-path-hash}"
    $env:CARGO_INCREMENTAL = "0"

    $externalRoot = Join-Path $repoRoot ".ait-external/ait-core"
    $marker = Join-Path $externalRoot ".ait-external-marker.json"
    $externalManifest = Join-Path $externalRoot "rust/crates/ait-py/Cargo.toml"
    if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) {
        throw "missing materialized ait-core marker: $marker"
    }
    if (-not (Test-Path -LiteralPath $externalManifest -PathType Leaf)) {
        throw "missing materialized ait-py manifest: $externalManifest"
    }

    $declaredExternalRoot = $env:AIT_EXTERNAL_CORE_REPO_ROOT
    if ([string]::IsNullOrWhiteSpace($declaredExternalRoot)) {
        $declaredExternalRoot = $externalRoot
    }
    $declaredExternal = (Resolve-Path -LiteralPath $declaredExternalRoot).Path
    $materializedExternal = (Resolve-Path -LiteralPath $externalRoot).Path
    if (-not [StringComparer]::OrdinalIgnoreCase.Equals(
        $declaredExternal,
        $materializedExternal
    )) {
        throw "AIT_EXTERNAL_CORE_REPO_ROOT does not match .ait-external/ait-core"
    }

    $pythonCommand = Get-Command python -CommandType Application -ErrorAction Stop |
        Select-Object -First 1
    $python = $pythonCommand.Source
    $verifier = Join-Path $ciRoot "verify_external.py"
    $verifierSource = @'
import json
import pathlib
import sys
import tomllib

lock_path = pathlib.Path(sys.argv[1])
marker_path = pathlib.Path(sys.argv[2])
lock = tomllib.loads(lock_path.read_text(encoding="utf-8"))
marker = json.loads(marker_path.read_text(encoding="utf-8"))
nodes = lock.get("node", [])
if len(nodes) != 1:
    raise SystemExit("ait-external.lock must contain exactly one node")
node = nodes[0]
for field in ("name", "repo_name", "repository_index", "snapshot", "materialize_to"):
    if marker.get(field) != node.get(field):
        raise SystemExit(f"external marker field {field!r} does not match lock")
'@
    [System.IO.File]::WriteAllText(
        $verifier,
        $verifierSource,
        [System.Text.UTF8Encoding]::new($false)
    )
    Invoke-NativeCommand -FilePath $python -ArgumentList @(
        $verifier,
        (Join-Path $repoRoot "ait-external.lock"),
        $marker
    )

    Set-Location -LiteralPath $repoRoot
    $venvRoot = Join-Path $ciRoot "venv"
    Invoke-NativeCommand -FilePath $python -ArgumentList @("-m", "venv", $venvRoot)

    $venvPython = Join-Path $venvRoot "Scripts/python.exe"
    if (-not (Test-Path -LiteralPath $venvPython -PathType Leaf)) {
        throw "virtual environment did not create its Python executable"
    }
    Invoke-NativeCommand -FilePath $venvPython -ArgumentList @(
        "-m", "pip", "install", "--no-cache-dir", ".[test]"
    )
    Invoke-NativeCommand -FilePath $venvPython -ArgumentList @(
        "-m", "pytest", "-p", "no:cacheprovider"
    )
    Invoke-NativeCommand -FilePath $venvPython -ArgumentList @(
        "-m", "pip", "check"
    )
}
finally {
    Set-Location -LiteralPath $previousLocation
    if (Test-Path -LiteralPath $ciRoot) {
        Remove-Item -LiteralPath $ciRoot -Recurse -Force
    }
}
