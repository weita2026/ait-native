[CmdletBinding()]
param(
    [ValidateSet("fmt", "clippy", "test", "patchset", "repo", "all")]
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

function Invoke-Format {
    Invoke-Checked "cargo" @("fmt", "--all", "--", "--check")
}

function Invoke-Clippy {
    Invoke-Checked "cargo" @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings")
}

function Invoke-Tests {
    Invoke-Checked "cargo" @("test", "--workspace", "--all-targets")
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($env:AIT_RUNNER_ATTEMPT_ROOT)) {
    $ownedRoot = Join-Path ([IO.Path]::GetTempPath()) ("ait-runner-ci-" + [guid]::NewGuid().ToString("N"))
} else {
    $ownedRoot = Join-Path $env:AIT_RUNNER_ATTEMPT_ROOT "repository-ci"
}
$ownedLeaf = Split-Path -Leaf $ownedRoot
if ($ownedLeaf -ne "repository-ci" -and -not $ownedLeaf.StartsWith("ait-runner-ci-")) {
    throw "refusing unsafe repository CI root: $ownedRoot"
}

try {
    $tmpRoot = Join-Path $ownedRoot "tmp"
    $cargoHome = Join-Path $ownedRoot "cache/cargo"
    $cargoTargetRoot = Join-Path $ownedRoot "build/cargo-target"
    $cargoBuildRoot = Join-Path $ownedRoot "build/cargo-build"
    foreach ($path in @($tmpRoot, $cargoHome, $cargoTargetRoot, $cargoBuildRoot)) {
        New-Item -ItemType Directory -Force -Path $path | Out-Null
    }

    $env:TMPDIR = $tmpRoot
    $env:TMP = $tmpRoot
    $env:TEMP = $tmpRoot
    $env:CARGO_HOME = $cargoHome
    $env:CARGO_TARGET_DIR = $cargoTargetRoot
    $env:CARGO_BUILD_BUILD_DIR = $cargoBuildRoot
    $env:CARGO_INCREMENTAL = "0"

    Set-Location -LiteralPath $repoRoot
    switch ($Mode) {
        "fmt" { Invoke-Format }
        "clippy" { Invoke-Clippy }
        "test" { Invoke-Tests }
        default {
            Invoke-Format
            Invoke-Clippy
            Invoke-Tests
        }
    }
} finally {
    if (Test-Path -LiteralPath $ownedRoot) {
        Remove-Item -LiteralPath $ownedRoot -Recurse -Force
    }
}
