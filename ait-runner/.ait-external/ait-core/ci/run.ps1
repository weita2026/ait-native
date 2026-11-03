[CmdletBinding()]
param(
    [ValidateSet("patchset", "repo", "all")]
    [string]$Mode = "patchset"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:ciFailed = $false
$script:stageExitCode = 0

function Write-Blocked {
    param([string]$Stage)
    $script:ciFailed = $true
    Write-Output "CI BLOCKED $Stage"
}

function Invoke-Collect {
    param(
        [string]$Stage,
        [string]$Program,
        [string[]]$Arguments
    )

    # Native nonzero exits are collected even when the caller enables this setting.
    $PSNativeCommandUseErrorActionPreference = $false
    Write-Output "CI RUN $Stage"
    try {
        & $Program @Arguments
        $script:stageExitCode = $LASTEXITCODE
    } catch {
        Write-Output $_
        $script:stageExitCode = 1
    }
    if ($script:stageExitCode -eq 0) {
        Write-Output "CI PASS $Stage"
    } else {
        $script:ciFailed = $true
        Write-Output "CI FAIL $Stage (exit $script:stageExitCode)"
    }
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$cleanupOwnedRoot = $false
$ownedRoot = $null
try {
    if ([string]::IsNullOrWhiteSpace($env:AIT_RUNNER_ATTEMPT_ROOT)) {
        $ownedRoot = Join-Path ([IO.Path]::GetTempPath()) ("ait-core-ci-" + [guid]::NewGuid().ToString("N"))
        New-Item -ItemType Directory -Path $ownedRoot | Out-Null
        $cleanupOwnedRoot = $true
    } else {
        $ownedRoot = $env:AIT_RUNNER_ATTEMPT_ROOT
    }

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
    # CI links executable binding tests, even if its caller built an extension.
    Remove-Item Env:\PYO3_BUILD_EXTENSION_MODULE -ErrorAction SilentlyContinue

    Set-Location -LiteralPath $repoRoot
    Invoke-Collect "format" "cargo" @("fmt", "--manifest-path", "rust/Cargo.toml", "--all", "--", "--check")

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
        Invoke-Collect "patchset-build" "cargo" @(
            "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
            "--locked", "--all-features",
            "-p", "ait-core", "-p", "ait-cli", "-p", "ait-agent-core",
            "-p", "ait-agent-worker", "-p", "ait-benchmark", "-p", "ait-napi",
            "-p", "ait-py",
            "--lib",
            "--test", "server_source_ownership",
            "--test", "patchset_ci_runner", "--test", "workflow_recovery_cli",
            "--test", "snapshot_task_binding_cli", "--test", "workflow_friction_cli",
            "--test", "plan_cli", "--test", "workflow_package_frozen_candidate_cli",
            "--test", "workflow_package_accepted_retry_cli", "--test", "team_session_cli", "--no-run"
        )
        if ($script:stageExitCode -eq 0) {
            Invoke-Collect "patchset-libraries" "cargo" @(
                "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
                "--locked", "--all-features",
                "-p", "ait-core", "-p", "ait-cli", "-p", "ait-agent-core",
                "-p", "ait-agent-worker", "-p", "ait-benchmark", "-p", "ait-napi",
                "-p", "ait-py", "--lib", "--no-fail-fast", "--", "--show-output"
            )
            Invoke-Collect "patchset-integrations" "cargo" @(
                "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
                "--locked", "--all-features",
                "-p", "ait-core", "-p", "ait-cli",
                "--test", "server_source_ownership",
                "--test", "patchset_ci_runner", "--test", "workflow_recovery_cli",
                "--test", "snapshot_task_binding_cli", "--test", "workflow_friction_cli",
                "--test", "plan_cli", "--test", "workflow_package_frozen_candidate_cli",
                "--test", "workflow_package_accepted_retry_cli", "--no-fail-fast"
            )
            Invoke-Collect "team-create" "cargo" @(
                "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
                "--locked", "--all-features", "-p", "ait-cli", "--test", "team_session_cli",
                "concurrent_creation_then_retry_does_not_create_duplicate", "--", "--exact"
            )
            Invoke-Collect "team-recovery" "cargo" @(
                "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
                "--locked", "--all-features", "-p", "ait-cli", "--test", "team_session_cli",
                "concurrent_missing_recovery_then_retry_does_not_create_duplicate", "--", "--exact"
            )
        } else {
            Write-Blocked "patchset-libraries, patchset-integrations, team-create, team-recovery: patchset-build failed"
        }

        # Markdown is Plan lineage and is intentionally absent from remote Snapshot
        # materialization. Canonical source still carries the sole protected authority.
        if (Test-Path -LiteralPath (Join-Path $repoRoot "docs/binary_db_v0.md") -PathType Leaf) {
            Invoke-Collect "schema-build" "cargo" @(
                "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
                "--locked", "-p", "ait-core", "--test", "binary_db_schema_authority", "--no-run"
            )
            if ($script:stageExitCode -eq 0) {
                Invoke-Collect "schema-tests" "cargo" @(
                    "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
                    "--locked", "-p", "ait-core", "--test", "binary_db_schema_authority"
                )
            } else {
                Write-Blocked "schema-tests: schema-build failed"
            }
        } else {
            Write-Output "skipping binary_db_schema_authority: lineage-only Markdown is unavailable in this Snapshot"
        }
    }

    function Invoke-RepoTests {
        Invoke-Collect "repo-build" "cargo" @(
            "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
            "--workspace", "--all-targets", "--all-features", "--locked", "--no-run"
        )
        if ($script:stageExitCode -eq 0) {
            Invoke-Collect "repo-tests" "cargo" @(
                "test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci",
                "--workspace", "--all-targets", "--all-features", "--locked", "--no-fail-fast"
            )
        } else {
            Write-Blocked "repo-tests: repo-build failed"
        }
    }

    function Invoke-Clippy {
        Invoke-Collect "clippy" "cargo" @(
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
} catch {
    Write-Output $_
    Write-Blocked "remaining Cargo stages: unsafe setup or boundary check failed"
} finally {
    if ($cleanupOwnedRoot -and (Test-Path -LiteralPath $ownedRoot)) {
        try {
            Remove-Item -LiteralPath $ownedRoot -Recurse -Force
        } catch {
            Write-Output $_
            $script:ciFailed = $true
        }
    }
}

if ($script:ciFailed) { exit 1 }
exit 0
