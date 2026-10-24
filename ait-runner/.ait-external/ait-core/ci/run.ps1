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
    $cargoTargetRoot = Join-Path $ownedRoot "cargo-target"
    $cargoBuildRoot = Join-Path $ownedRoot "cargo-build"
    foreach ($path in @($tmpRoot, $cargoTargetRoot, $cargoBuildRoot)) {
        New-Item -ItemType Directory -Force -Path $path | Out-Null
    }

    $env:TMPDIR = $tmpRoot
    $env:TMP = $tmpRoot
    $env:TEMP = $tmpRoot
    $env:CARGO_TARGET_DIR = $cargoTargetRoot
    $env:CARGO_BUILD_BUILD_DIR = Join-Path $cargoBuildRoot "{workspace-path-hash}"
    $env:CARGO_INCREMENTAL = "0"

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

    function Invoke-PatchsetTests {
        Invoke-Checked "cargo" @(
            "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
            "--locked", "--all-features",
            "-p", "ait-core", "-p", "ait-cli", "-p", "ait-agent-core",
            "-p", "ait-agent-worker", "-p", "ait-benchmark", "-p", "ait-napi",
            "-p", "ait-py",
            "--lib",
            "--test", "server_source_ownership",
            "--test", "patchset_ci_runner", "--no-run"
        )
        Invoke-Checked "cargo" @(
            "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
            "--locked", "--all-features",
            "-p", "ait-core", "-p", "ait-cli", "-p", "ait-agent-core",
            "-p", "ait-agent-worker", "-p", "ait-benchmark", "-p", "ait-napi",
            "-p", "ait-py", "--lib"
        )
        Invoke-Checked "cargo" @(
            "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
            "--locked", "--all-features",
            "-p", "ait-core", "-p", "ait-cli",
            "--test", "server_source_ownership",
            "--test", "patchset_ci_runner"
        )

        # Markdown is Plan lineage and is intentionally absent from remote Snapshot
        # materialization. Canonical source still carries the sole protected authority.
        if (Test-Path -LiteralPath (Join-Path $repoRoot "docs/binary_db_v0.md") -PathType Leaf) {
            Invoke-Checked "cargo" @(
                "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
                "--locked", "-p", "ait-core", "--test", "binary_db_schema_authority"
            )
        } else {
            Write-Output "skipping binary_db_schema_authority: lineage-only Markdown is unavailable in this Snapshot"
        }
    }

    function Invoke-RepoTests {
        Invoke-Checked "cargo" @(
            "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
            "--workspace", "--all-targets", "--all-features", "--locked"
        )
    }

    function Invoke-Clippy {
        Invoke-Checked "cargo" @(
            "clippy", "--manifest-path", "rust/Cargo.toml",
            "--workspace", "--all-targets", "--all-features", "--locked",
            "--", "-D", "warnings"
        )
    }

    switch ($Mode) {
        "patchset" { Invoke-PatchsetTests }
        "repo" { Invoke-RepoTests }
        "all" {
            Invoke-RepoTests
            Invoke-Clippy
        }
    }
} finally {
    if ($cleanupOwnedRoot -and (Test-Path -LiteralPath $ownedRoot)) {
        Remove-Item -LiteralPath $ownedRoot -Recurse -Force
    }
}
