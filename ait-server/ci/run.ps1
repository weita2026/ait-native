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
if ([string]::IsNullOrWhiteSpace($env:AIT_RUNNER_ATTEMPT_ROOT)) {
    $ownedRoot = Join-Path ([IO.Path]::GetTempPath()) ("ait-server-ci-" + [guid]::NewGuid().ToString("N"))
} else {
    $ownedRoot = Join-Path $env:AIT_RUNNER_ATTEMPT_ROOT "repository-ci"
}
$ownedLeaf = Split-Path -Leaf $ownedRoot
if ($ownedLeaf -ne "repository-ci" -and -not $ownedLeaf.StartsWith("ait-server-ci-")) {
    throw "refusing unsafe repository CI root: $ownedRoot"
}

try {
    New-Item -ItemType Directory -Force -Path $ownedRoot | Out-Null
    $tmpRoot = Join-Path $ownedRoot "tmp"
    $cargoTargetRoot = Join-Path $ownedRoot "cargo-target"
    $cargoBuildRoot = Join-Path $ownedRoot "cargo-build"
    foreach ($path in @($tmpRoot, $cargoTargetRoot, $cargoBuildRoot)) {
        New-Item -ItemType Directory -Force -Path $path | Out-Null
    }

    $env:TMPDIR = $tmpRoot
    $env:TMP = $tmpRoot
    $env:TEMP = $tmpRoot
    $env:CARGO_TARGET_DIR = $cargoTargetRoot
    $env:CARGO_BUILD_BUILD_DIR = $cargoBuildRoot
    $env:CARGO_INCREMENTAL = "0"
    $env:AIT_TEST_DISABLE_GLOBAL_HOST_RAM_ROOT_CLEANUP = "1"

    Set-Location -LiteralPath $repoRoot
    Invoke-Checked "cargo" @("fmt", "--manifest-path", "rust/Cargo.toml", "--all", "--", "--check")

    $pythonFile = Get-ChildItem -LiteralPath $repoRoot -Recurse -File -Filter "*.py" |
        Where-Object {
            $relative = $_.FullName.Substring($repoRoot.Length).TrimStart([char[]]@("\", "/")).Replace("\", "/")
            -not (
                $relative.StartsWith(".ait/") -or
                $relative.StartsWith(".ait-runtime/") -or
                $relative.StartsWith(".ait-external/") -or
                $relative.StartsWith("rust/target/")
            )
        } |
        Select-Object -First 1
    if ($null -ne $pythonFile) {
        throw "Python source is forbidden in ait-server: $($pythonFile.FullName)"
    }

    Invoke-Checked "cargo" @(
        "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci", "--no-run",
        "-p", "ait-server-core", "-p", "ait-server", "--lib",
        "--test", "seam_contract_direct_tests",
        "--features", "ait-server-core/patch-ci-harness"
    )
    Invoke-Checked "cargo" @(
        "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
        "-p", "ait-server-core", "-p", "ait-server", "--lib",
        "--test", "seam_contract_direct_tests",
        "--features", "ait-server-core/patch-ci-harness", "--no-fail-fast"
    )
} finally {
    if (Test-Path -LiteralPath $ownedRoot) {
        Remove-Item -LiteralPath $ownedRoot -Recurse -Force
    }
}
