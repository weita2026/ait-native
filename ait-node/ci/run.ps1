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

$ciLeaf = "ait-node-ci." + [Guid]::NewGuid().ToString("N")
$ciRoot = Join-Path $runtimeParent $ciLeaf
[void][System.IO.Directory]::CreateDirectory($ciRoot)
$previousLocation = (Get-Location).Path

try {
    foreach ($relativePath in @("tmp", "cache/npm", "project")) {
        [void][System.IO.Directory]::CreateDirectory(
            (Join-Path $ciRoot $relativePath)
        )
    }

    $env:TMPDIR = Join-Path $ciRoot "tmp"
    $env:TMP = $env:TMPDIR
    $env:TEMP = $env:TMPDIR
    $env:XDG_CACHE_HOME = Join-Path $ciRoot "cache"
    $env:npm_config_cache = Join-Path $ciRoot "cache/npm"
    $env:npm_config_audit = "false"
    $env:npm_config_fund = "false"
    $env:npm_config_update_notifier = "false"

    $projectRoot = Join-Path $ciRoot "project"
    foreach ($fileName in @(
        "package.json",
        "ait-release.json",
        "ait-external.toml",
        "ait-external.lock",
        "LICENSE",
        "NOTICE"
    )) {
        Copy-Item -LiteralPath (Join-Path $repoRoot $fileName) `
            -Destination $projectRoot
    }
    foreach ($directoryName in @(
        "bin",
        "lib",
        "release",
        "scripts",
        "src",
        "test",
        "ci"
    )) {
        Copy-Item -LiteralPath (Join-Path $repoRoot $directoryName) `
            -Destination $projectRoot -Recurse
    }

    $npmCommand = Get-Command npm.cmd -CommandType Application -ErrorAction Stop |
        Select-Object -First 1
    $nodeCommand = Get-Command node.exe -CommandType Application -ErrorAction Stop |
        Select-Object -First 1
    $npm = $npmCommand.Source
    $node = $nodeCommand.Source

    Set-Location -LiteralPath $projectRoot
    $externalCore = $env:AIT_EXTERNAL_CORE_REPO_ROOT
    if ([string]::IsNullOrWhiteSpace($externalCore)) {
        $externalCore = Join-Path $repoRoot ".ait-external/ait-core"
    }
    $externalMarker = Join-Path $externalCore ".ait-external-marker.json"
    if (-not (Test-Path -LiteralPath $externalMarker -PathType Leaf)) {
        throw "ait-node CI requires the exact materialized ait-core external"
    }
    $marker = Get-Content -LiteralPath $externalMarker -Raw | ConvertFrom-Json
    if (
        $marker.name -ne "ait-core" -or
        $marker.snapshot -ne "SNP-F136DB9A342B"
    ) {
        throw "ait-core external marker identity drift"
    }
    $externalRoot = Join-Path $projectRoot ".ait-external"
    [void][System.IO.Directory]::CreateDirectory($externalRoot)
    Copy-Item -LiteralPath $externalCore `
        -Destination (Join-Path $externalRoot "ait-core") -Recurse
    Invoke-NativeCommand -FilePath $npm -ArgumentList @("run", "native:build")
    Invoke-NativeCommand -FilePath $npm -ArgumentList @("test")
    Invoke-NativeCommand -FilePath $npm -ArgumentList @("run", "check")

    $releaseAdapter = Join-Path $projectRoot "release/release-adapter.mjs"
    Invoke-NativeCommand -FilePath $node -ArgumentList @(
        $releaseAdapter, "build", "portable", "1.0.0-rc.8"
    )
    Invoke-NativeCommand -FilePath $node -ArgumentList @(
        $releaseAdapter, "smoke", "portable", "1.0.0-rc.8"
    )
}
finally {
    Set-Location -LiteralPath $previousLocation
    if (Test-Path -LiteralPath $ciRoot) {
        Remove-Item -LiteralPath $ciRoot -Recurse -Force
    }
}
