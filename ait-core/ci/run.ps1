[CmdletBinding()]
param(
    [ValidateSet("patchset", "repo", "all")]
    [string]$Mode = "patchset"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Program,
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "command failed with exit code ${LASTEXITCODE}: $Program"
    }
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$cleanupOwnedRoot = $false
if ([string]::IsNullOrWhiteSpace($env:AIT_RUNNER_ATTEMPT_ROOT)) {
    $ownedRoot = Join-Path ([IO.Path]::GetTempPath()) ("ait-core-ci-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $ownedRoot | Out-Null
    $cleanupOwnedRoot = $true
} else {
    $ownedRoot = $env:AIT_RUNNER_ATTEMPT_ROOT
}

try {
    $tmpRoot = Join-Path $ownedRoot "tmp"
    $testOutsideRoot = Join-Path $ownedRoot "test-outside"
    $cargoTargetRoot = Join-Path $ownedRoot "cargo-target"
    $cargoBuildRoot = Join-Path $ownedRoot "cargo-build"
    foreach ($path in @($tmpRoot, $testOutsideRoot, $cargoTargetRoot, $cargoBuildRoot)) {
        New-Item -ItemType Directory -Force -Path $path | Out-Null
    }

    $env:TMPDIR = $tmpRoot
    $env:TMP = $tmpRoot
    $env:TEMP = $tmpRoot
    $env:CARGO_TARGET_DIR = $cargoTargetRoot
    $env:CARGO_BUILD_BUILD_DIR = Join-Path $cargoBuildRoot "{workspace-path-hash}"
    $env:CARGO_INCREMENTAL = "0"
    $env:AIT_TEST_DISABLE_GLOBAL_HOST_RAM_ROOT_CLEANUP = "1"
    $env:AIT_TEST_OUTSIDE_REPO_TMP = $testOutsideRoot

    Set-Location -LiteralPath $repoRoot
    Invoke-Checked "cargo" @("fmt", "--manifest-path", "rust/Cargo.toml", "--all", "--", "--check")

    $pythonFile = Get-ChildItem -LiteralPath $repoRoot -Recurse -File -Filter "*.py" |
        Where-Object {
            $relative = $_.FullName.Substring($repoRoot.Length).TrimStart([char[]]@("\", "/")).Replace("\", "/")
            -not (
                $relative.StartsWith(".ait/") -or
                $relative.StartsWith(".git/") -or
                $relative.StartsWith("target/")
            )
        } |
        Select-Object -First 1
    if ($null -ne $pythonFile) {
        throw "zero-Python boundary violation: $($pythonFile.FullName)"
    }

    Invoke-Checked "cargo" @(
        "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
        "-p", "ait-agent-core", "-p", "ait-py", "-p", "ait-cli",
        "--lib", "--bin", "ait-cli", "--test", "patchset_ci_smoke_cli", "--no-run"
    )
    Invoke-Checked "cargo" @(
        "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
        "-p", "ait-agent-core", "--lib"
    )
    Invoke-Checked "cargo" @(
        "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
        "-p", "ait-py", "--lib"
    )
    Invoke-Checked "cargo" @(
        "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
        "-p", "ait-cli", "--test", "patchset_ci_smoke_cli"
    )
} finally {
    if ($cleanupOwnedRoot -and (Test-Path -LiteralPath $ownedRoot)) {
        Remove-Item -LiteralPath $ownedRoot -Recurse -Force
    }
}
