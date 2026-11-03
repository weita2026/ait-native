Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-CoreProcess {
    param([string]$Program, [string[]]$Arguments, [string]$Directory, [string]$Stem, [int]$TimeoutMs = 1200000)
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Program
    $info.WorkingDirectory = $Directory
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    if (-not $process.Start()) { throw "Unable to start owned diagnostic process" }
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $timedOut = -not $process.WaitForExit($TimeoutMs)
    if ($timedOut) { $process.Kill($true); $process.WaitForExit() }
    $out = $stdout.GetAwaiter().GetResult()
    $err = $stderr.GetAwaiter().GetResult()
    [IO.File]::WriteAllText($Stem + ".stdout", $out)
    [IO.File]::WriteAllText($Stem + ".stderr", $err)
    Write-Host $out
    Write-Host $err
    $code = if ($timedOut) { 124 } else { $process.ExitCode }
    $process.Dispose()
    if ($timedOut) { Write-Host "Diagnostic controller timed out; native completion is unproven. No retry." }
    return [pscustomobject]@{ Code = $code; Stdout = $out; Stderr = $err; TimedOut = $timedOut }
}

function Start-CoreCapture {
    param([string]$Evidence)
    if (@(Get-Process -Name Procmon, Procmon64, Procmon64a -ErrorAction SilentlyContinue).Count) {
        throw "An existing unowned Process Monitor instance prevents exclusive capture"
    }
    $toolRoot = Join-Path $Evidence "procmon"
    New-Item -ItemType Directory -Path $toolRoot | Out-Null
    $archive = Join-Path $toolRoot "ProcessMonitor.zip"
    Invoke-WebRequest -Uri "https://download.sysinternals.com/files/ProcessMonitor.zip" -OutFile $archive
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne "477754bf299c9e5b311c5666f7a4a48dfdb21265eaa8f0cca70ef9d9552e3a10") {
        throw "Process Monitor archive differs from the frozen 4.12 input"
    }
    Expand-Archive -LiteralPath $archive -DestinationPath $toolRoot
    $exe = Join-Path $toolRoot "Procmon64.exe"
    if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant() -ne "28495e642b2a465474a4edc58380eaf8b7ee580f3fbcb4141061deadafe9b885") {
        throw "Process Monitor executable differs from the frozen input"
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $exe
    if ($signature.Status -ne "Valid" -or $signature.SignerCertificate.Subject -notmatch "CN=Microsoft Corporation(?:,|$)" -or
        (Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -notmatch '^4\.12(?:\.0)*$') {
        throw "Process Monitor Microsoft signature/version is unavailable or mismatched"
    }
    $pml = Join-Path $Evidence "cleanup.pml"
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $exe
    $info.UseShellExecute = $false
    foreach ($argument in @("/AcceptEula", "/Quiet", "/Minimized", "/NoFilter", "/BackingFile", $pml)) { $info.ArgumentList.Add($argument) }
    $owner = [Diagnostics.Process]::new()
    $owner.StartInfo = $info
    if (-not $owner.Start()) { throw "Process Monitor did not start" }
    $session = [pscustomobject]@{ Exe = $exe; Owner = $owner; Pml = $pml; Evidence = $Evidence }
    try {
        $idle = Invoke-CoreProcess $exe @("/WaitForIdle") $toolRoot (Join-Path $Evidence "procmon-idle") 30000
        if ($idle.Code -ne 0 -or $owner.HasExited -or -not (Test-Path -LiteralPath $pml)) {
            throw "Process Monitor driver/elevation/capture initialization is unproven"
        }
        Write-Host "Full unfiltered Process Monitor 4.12 capture initialized before fixture creation"
        return $session
    } catch {
        Stop-CoreCapture $session
        throw
    }
}

function Stop-CoreCapture {
    param($Session)
    try {
    $stopped = Invoke-CoreProcess $Session.Exe @("/Terminate", "/Quiet") ([IO.Path]::GetDirectoryName($Session.Exe)) (Join-Path $Session.Evidence "procmon-stop") 30000
    if (-not $Session.Owner.WaitForExit(15000) -or $stopped.Code -ne 0) {
        if (-not $Session.Owner.HasExited) { $Session.Owner.Kill($true); $Session.Owner.WaitForExit() }
        throw "Owned Process Monitor did not flush normally; trace is incomplete"
    }
    if (-not (Test-Path -LiteralPath $Session.Pml) -or (Get-Item -LiteralPath $Session.Pml).Length -eq 0) { throw "Original PML trace is missing" }
    $csv = Join-Path $Session.Evidence "cleanup.csv"
    $exported = Invoke-CoreProcess $Session.Exe @("/OpenLog", $Session.Pml, "/SaveAs", $csv, "/Quiet") ([IO.Path]::GetDirectoryName($Session.Exe)) (Join-Path $Session.Evidence "procmon-export") 60000
    if ($exported.Code -ne 0 -or -not (Test-Path -LiteralPath $csv) -or (Get-Item -LiteralPath $csv).Length -eq 0) { throw "PML CSV export is unavailable; retain the original PML" }
    Write-Host "Original PML and CSV retained; holder and source cause still require interpretation"
    } finally {
        if (-not $Session.Owner.HasExited) { $Session.Owner.Kill($true); $Session.Owner.WaitForExit() }
    }
}

function Invoke-CoreDiagnostics {
    param([string]$Core, [string]$Owned, [string]$Evidence)
    $failed = $false
    foreach ($directory in @($Owned, $Evidence, (Join-Path $Owned "tmp"), (Join-Path $Owned "cargo-target"), (Join-Path $Owned "cargo-build"))) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }
    $env:TMPDIR = Join-Path $Owned "tmp"
    $env:TMP = $env:TMPDIR
    $env:TEMP = $env:TMPDIR
    $env:CARGO_TARGET_DIR = Join-Path $Owned "cargo-target"
    $env:CARGO_BUILD_BUILD_DIR = Join-Path $Owned "cargo-build/{workspace-path-hash}"
    $env:CARGO_INCREMENTAL = "0"
    Remove-Item Env:\PYO3_BUILD_EXTENSION_MODULE -ErrorAction SilentlyContinue
    $cargo = (Get-Command cargo -CommandType Application).Source
    $session = $null
    try {
        Write-Host "CI RUN core-cleanup-prebuild (outside capture)"
        $built = Invoke-CoreProcess $cargo @("test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci", "--locked", "--all-features", "-p", "ait-cli", "--test", "workflow_friction_cli", "--no-run", "--message-format=json") $Core (Join-Path $Evidence "cleanup-build")
        if ($built.Code -ne 0) { throw "Exact Core cleanup artifact build failed" }
        $artifacts = @($built.Stdout -split "`n" | ForEach-Object {
            try { $record = $_ | ConvertFrom-Json; if ($record.reason -eq "compiler-artifact") { $record } } catch {}
        })
        $manifest = (Resolve-Path -LiteralPath (Join-Path $Core "rust/crates/ait-cli/Cargo.toml")).Path
        $testArtifacts = @($artifacts | Where-Object { $_.target.name -eq "workflow_friction_cli" -and $_.target.kind -contains "test" -and $_.profile.test -eq $true -and (Resolve-Path -LiteralPath $_.manifest_path).Path -eq $manifest -and $_.executable })
        if ($testArtifacts.Count -ne 1) { throw "Core cleanup artifact/manifest is missing or ambiguous" }
        $artifact = $testArtifacts[0]
        $exe = (Resolve-Path -LiteralPath $artifact.executable).Path
        if (-not $exe.StartsWith($Owned + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw "Core artifact is outside this attempt" }
        $cli = @($artifacts | Where-Object { $_.target.name -eq "ait-cli" -and $_.target.kind -contains "bin" -and (Resolve-Path -LiteralPath $_.manifest_path).Path -eq $manifest -and $_.executable -and $_.profile.test -eq $false })
        if ($cli.Count -ne 1 -or -not (Test-Path -LiteralPath $cli[0].executable)) { throw "Fresh corresponding CLI artifact is unavailable" }
        $cliPath = (Resolve-Path -LiteralPath $cli[0].executable).Path
        if (-not $cliPath.StartsWith($Owned + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw "CLI artifact is outside this attempt" }
        $package = [IO.Path]::GetDirectoryName($manifest)
        $env:CARGO_MANIFEST_DIR = $package
        Set-Item -LiteralPath "Env:CARGO_BIN_EXE_ait-cli" -Value $cliPath
        $priorPath = $env:PATH
        try {
            $env:PATH = [IO.Path]::GetDirectoryName($exe) + [IO.Path]::PathSeparator + $priorPath
            $session = Start-CoreCapture $Evidence
            Write-Host "CI RUN core-cleanup-case (one fresh artifact invocation)"
            $case = Invoke-CoreProcess $exe @("finish_closes_nested_checkboxes_under_the_bound_item_only", "--exact", "--nocapture", "--test-threads=1") $package (Join-Path $Evidence "cleanup-case") 600000
            if ($case.Code -ne 0) { $failed = $true; Write-Host "CI FAIL core-cleanup-case (exit $($case.Code)); no retry" }
            else { Write-Host "CI PASS core-cleanup-case; diagnostic observation only" }
        } finally { $env:PATH = $priorPath }
    } catch {
        $failed = $true
        Write-Host "CI BLOCKED/FAILED core-cleanup-case: $_"
    } finally {
        if ($null -ne $session) {
            try { Stop-CoreCapture $session } catch { $failed = $true; Write-Host "CI BLOCKED cleanup trace: $_" }
        }
    }
    # These independent cases are always outside the bounded PML capture.
    $targets = @(
        @{ Package = "ait-benchmark"; Name = "benchmark"; Cases = @("runner::tests::timed_external_subject_records_resource_and_exit_contract") },
        @{ Package = "ait-cli"; Name = "cli"; Cases = @(
            "primitives::git_interop::mirror::tests::mirror_resumes_after_typed_object_transfer_interruption",
            "primitives::git_interop::native::migration::tests::legacy_migration_admits_export_ref_aliases_without_old_export_commit_rows",
            "primitives::git_interop::native::migration::tests::legacy_migration_converts_native_export_commit_rows_and_preserves_source_json",
            "primitives::git_interop::native::migration::tests::legacy_migration_converts_tag_only_with_exact_metadata",
            "primitives::git_interop::native::migration::tests::legacy_migration_preserves_raw_history_native_heads_and_offline_evidence",
            "primitives::git_interop::native::migration::tests::legacy_migration_rejects_changed_input_without_canonical_writes",
            "primitives::git_interop::native::migration::tests::legacy_migration_source_export_preserves_frozen_refs_and_symbolic_head",
            "primitives::git_interop::import_object_reader_tests::git_source_arguments_preserve_remote_urls") },
        @{ Package = "ait-core"; Name = "leases"; Cases = @("current_source_cache::tests::register_lease_writes_through_lease_store_trait_object", "current_source_cache::tests::release_lease_prunes_through_lease_store_trait_object") }
    )
    foreach ($target in $targets) {
        try {
            Write-Host "CI RUN core-nonshared-$($target.Name)"
            $arguments = @("test", "--manifest-path", "rust/Cargo.toml", "--profile", "ait-ci", "--locked", "--all-features", "--no-fail-fast", "-p", $target.Package, "--lib", "--") + $target.Cases + @("--exact", "--nocapture")
            $result = Invoke-CoreProcess $cargo $arguments $Core (Join-Path $Evidence ("nonshared-" + $target.Name))
            if ($result.Code -ne 0) { throw "exit $($result.Code)" }
            Write-Host "CI PASS core-nonshared-$($target.Name)"
        } catch { $failed = $true; Write-Host "CI FAIL core-nonshared-$($target.Name): $_" }
    }
    Write-Host "Core fixture scratch retained without retry or manual fixture deletion: $Owned"
    return $failed
}

if ($MyInvocation.InvocationName -ne ".") {
    try {
        if ($env:RUNNER_OS -ne "Windows" -or $env:SOURCE_QUALITY_EVENT -ne "workflow_dispatch" -or
            $env:SOURCE_QUALITY_REPOSITORY -ne "weita2026/ait-native" -or $env:DIAGNOSTIC_REUSE_R4 -ne "false" -or
            -not ([string]$env:SOURCE_QUALITY_REF).StartsWith("refs/heads/diagnostic/release-121-")) { throw "Core diagnostic context is unavailable" }
        $core = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "../ait-core")).Path
        $owned = Join-Path ([IO.Path]::GetTempPath()) ("ait-core-ci-" + [guid]::NewGuid().ToString("N"))
        if (Invoke-CoreDiagnostics $core $owned $env:AIT_WINDOWS_DIAGNOSTIC_EVIDENCE) { exit 1 }
        exit 0
    } catch { Write-Host "CI BLOCKED Core diagnostics: $_"; exit 1 }
}
