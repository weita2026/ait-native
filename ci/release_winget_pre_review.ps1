$ErrorActionPreference = 'Stop'
Start-Transcript -Path winget-pre-review.log -Force
try {
  if ($env:RELEASE_VERSION -notmatch '^\d+\.\d+\.\d+$' -or $env:RELEASE_ID -cnotmatch '^REL-FAM-[0-9A-F]{16}$(?![\s\S])') { throw 'Invalid release identity' }
  $expectedArchitecture = @{x64='AMD64';arm64='ARM64'}[$env:INSTALL_ARCH]
  if (-not $expectedArchitecture -or $env:PROCESSOR_ARCHITECTURE -cne $expectedArchitecture) { throw 'Native runner architecture mismatch' }
  $bundle = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($env:MANIFESTS_B64)) | ConvertFrom-Json -AsHashtable
  $names = @('Weita.AitNative.yaml','Weita.AitNative.installer.yaml','Weita.AitNative.locale.en-US.yaml')
  if ($bundle.Count -ne 3) { throw 'Expected exactly three manifests' }
  New-Item -ItemType Directory manifests | Out-Null
  $manifestHashes = @{}
  foreach ($name in $names) {
    if (-not $bundle.ContainsKey($name)) { throw "Missing manifest: $name" }
    $bytes = [Convert]::FromBase64String($bundle[$name])
    $text = [Text.Encoding]::UTF8.GetString($bytes)
    $identities = @($text -split '\r?\n' | Where-Object { $_ -match '^PackageIdentifier:' })
    $versions = @($text -split '\r?\n' | Where-Object { $_ -match '^PackageVersion:' })
    if ($identities.Count -ne 1 -or $versions.Count -ne 1) { throw 'Ambiguous manifest identity' }
    $identifier = ($identities[0] -split ':',2)[1].Trim().Trim('"').Trim("'")
    $version = ($versions[0] -split ':',2)[1].Trim().Trim('"').Trim("'")
    if ($identifier -cne 'Weita.AitNative' -or $version -cne $env:RELEASE_VERSION) { throw 'Manifest identity differs' }
    [IO.File]::WriteAllBytes((Join-Path "$PWD\manifests" $name),$bytes)
    $manifestHashes[$name] = (Get-FileHash "manifests\$name" -Algorithm SHA256).Hash.ToLowerInvariant()
  }
  $validation = winget validate --manifest "$PWD\manifests" 2>&1
  $validationExit = $LASTEXITCODE
  $validation | Out-File -Encoding utf8 winget-manifest-validation.txt
  if ($validationExit -ne 0 -and -not ($validationExit -eq -1978335192 -and ($validation -join "`n").StartsWith('Manifest validation succeeded with warnings.'))) { throw "WinGet validation failed: $validationExit" }
  $links = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Links'
  foreach ($command in @('ait','ait-server','ait-runner')) {
    if (Test-Path (Join-Path $links "$command.exe")) { throw "Fresh host already contains $command" }
  }
  winget settings --enable LocalManifestFiles
  if ($LASTEXITCODE -ne 0) { throw 'Could not enable local manifests on disposable runner' }
  $install = winget install --manifest "$PWD\manifests" --scope user --architecture $env:INSTALL_ARCH --accept-source-agreements --accept-package-agreements --disable-interactivity --silent --log "$PWD\winget-install.log" 2>&1
  $installExit = $LASTEXITCODE
  $install | Out-File -Encoding utf8 winget-install-readback.txt
  if ($installExit -ne 0) { throw "Candidate manifest installation failed: $installExit" }
  $commands = @()
  foreach ($command in @('ait','ait-server','ait-runner')) {
    $executable = Join-Path $links "$command.exe"
    if (-not (Test-Path $executable -PathType Leaf)) { throw "Installed command missing: $command" }
    $reported = & $executable --version 2>&1
    $commandExit = $LASTEXITCODE
    $reportedText = ($reported -join "`n").Trim()
    if ($commandExit -ne 0 -or $reportedText -cne "$command $env:RELEASE_VERSION") { throw "Installed version differs: $command => $reportedText" }
    $commands += @{name=$command;reported_version=$reportedText;sha256=(Get-FileHash $executable -Algorithm SHA256).Hash.ToLowerInvariant()}
  }
  @{contract='ait.release.winget-pre-review/v1';status='installed_verified';version=$env:RELEASE_VERSION;release_id=$env:RELEASE_ID;architecture=$env:INSTALL_ARCH;scope='user';fresh_host=$true;manifests=$manifestHashes;commands=$commands;control_commit=$env:GITHUB_SHA;bootstrap_sha256=(Get-FileHash ci/release_winget_bootstrap.ps1 -Algorithm SHA256).Hash.ToLowerInvariant();checked_at=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 winget-pre-review.json
} catch {
  $_ | Format-List * -Force | Out-File -Encoding utf8 winget-pre-review-error.txt
  throw
} finally {
  Stop-Transcript
}
