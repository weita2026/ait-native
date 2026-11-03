Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($env:RUNNER_OS -ne "Windows" -or $env:SOURCE_QUALITY_EVENT -ne "workflow_dispatch" -or
    $env:SOURCE_QUALITY_REPOSITORY -ne "weita2026/ait-native" -or
    -not ([string]$env:SOURCE_QUALITY_REF).StartsWith("refs/heads/diagnostic/release-121-") -or
    $env:DIAGNOSTIC_REUSE_R4 -ne "false") {
    throw "The Runner probe requires an exclusive Windows diagnostic dispatch"
}
if ($env:SOURCE_QUALITY_REF -in @("refs/heads/diagnostic/release-121-20261103-r4", "refs/heads/diagnostic/release-121-20261103-r5", "refs/heads/diagnostic/release-121-20261103-r6")) {
    throw "Previously frozen diagnostic branches must remain immutable"
}
$fullBatch = $env:DIAGNOSTIC_WINDOWS_BATCH -eq "true"
$runnerVerify = $env:DIAGNOSTIC_RUNNER_WINDOWS_VERIFY -eq "true"
$affectedVerify = $env:DIAGNOSTIC_AFFECTED_WINDOWS_VERIFY -eq "true"
$inventory = @($env:DIAGNOSTIC_WINDOWS_BATCH, $env:DIAGNOSTIC_RUNNER_WINDOWS_VERIFY, $env:DIAGNOSTIC_AFFECTED_WINDOWS_VERIFY)
if (@($inventory | Where-Object { $_ -notin @("true", "false") }).Count -ne 0 -or @($inventory | Where-Object { $_ -eq "true" }).Count -ne 1) {
    throw "Select exactly one frozen Windows diagnostic inventory"
}
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "../ait-runner")).Path
$probeHash = (Get-FileHash -LiteralPath (Join-Path $repoRoot "src/server.rs") -Algorithm SHA256).Hash.ToLowerInvariant()
$expectedHash = if ($runnerVerify -or $affectedVerify) { "3798355c96d796edeacbaea4e661df9a2f9ab02b645a33edc874328141635e02" } else { "de7e870c2f622c23fb6514d5f88f20d8b71c9d0b6a57e4730605e997be6e8fb5" }
if ($probeHash -ne $expectedHash) {
    throw "Runner source differs from the frozen RRT-0071 diagnostic"
}

$script:diagnosticFailed = $false
function Invoke-DiagnosticCommand {
    param([string]$Stage, [string[]]$Arguments)
    $PSNativeCommandUseErrorActionPreference = $false
    Write-Output "CI RUN $Stage"
    try {
        & cargo @Arguments
        $stageExit = $LASTEXITCODE
    } catch {
        Write-Output $_
        $stageExit = 1
    }
    if ($stageExit -eq 0) {
        Write-Output "CI PASS $Stage"
    } else {
        $script:diagnosticFailed = $true
        Write-Output "CI FAIL $Stage (exit $stageExit)"
    }
}

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
    $evidenceParent = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { [IO.Path]::GetTempPath() } else { $env:RUNNER_TEMP }
    $evidence = Join-Path $evidenceParent ("ait-windows-diagnostic-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $evidence | Out-Null
    $env:AIT_WINDOWS_DIAGNOSTIC_EVIDENCE = $evidence
    if ($env:GITHUB_OUTPUT) { "evidence_root=$evidence" | Out-File -LiteralPath $env:GITHUB_OUTPUT -Append -Encoding utf8 }
    if ($fullBatch -or $affectedVerify) {
    $saved = @{}
    foreach ($name in @("TMPDIR", "TMP", "TEMP", "CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_BUILD_BUILD_DIR", "CARGO_INCREMENTAL", "PYO3_BUILD_EXTENSION_MODULE", "CARGO_MANIFEST_DIR", "CARGO_BIN_EXE_ait-cli", "PATH")) {
        $saved[$name] = [Environment]::GetEnvironmentVariable($name)
    }
    try {
        Write-Output "CI RUN core-selected-diagnostics"
        & (Join-Path $PSScriptRoot "core-windows-probe.ps1")
        if ($LASTEXITCODE -ne 0) { throw "Core diagnostics failed or are blocked (exit $LASTEXITCODE)" }
        Write-Output "CI PASS core-selected-diagnostics"
    } catch {
        Write-Output $_
        Write-Output "CI FAIL/BLOCKED core-selected-diagnostics"
        $script:diagnosticFailed = $true
    } finally {
        foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    }
    } else {
        Write-Output "Core: not selected; original evidence keeps its original source binding"
    }
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
    Write-Output "Windows diagnostic only; no full-suite qualification"
    Invoke-DiagnosticCommand "runner-legacy-transition" @(
        "test", "--locked", "--lib", "server::tests::legacy_transition_executes_and_preserves_attempt_cleanup",
        "--", "--exact", "--nocapture"
    )
    if ($fullBatch) {
    Invoke-DiagnosticCommand "runner-strict-clippy" @(
        "clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"
    )
    } else {
        Write-Output "Runner Clippy: not executed; prior matching results are not a fresh result"
    }
} catch {
    Write-Output $_
    Write-Output "CI BLOCKED Runner diagnostic: unsafe setup or source boundary"
    $script:diagnosticFailed = $true
} finally {
    if (Test-Path -LiteralPath $ownedRoot) {
        try { Remove-Item -LiteralPath $ownedRoot -Recurse -Force }
        catch {
            Write-Output $_
            Write-Output "CI FAIL Runner diagnostic cleanup"
            $script:diagnosticFailed = $true
        }
    }
}
if ($script:diagnosticFailed) { exit 1 }
exit 0
