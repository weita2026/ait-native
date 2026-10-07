$ErrorActionPreference = 'Stop'
Start-Transcript -Path winget-bootstrap.log -Force
try {
  Write-Host "PowerShell: $($PSVersionTable.PSVersion); edition: $($PSVersionTable.PSEdition)"
  if ($PSVersionTable.PSEdition -ne 'Desktop') { throw 'Appx bootstrap requires Windows PowerShell' }
  if (-not (Get-Command winget.exe -ErrorAction SilentlyContinue)) {
    $appInstaller = Get-AppxPackage -AllUsers -Name Microsoft.DesktopAppInstaller | Sort-Object Version -Descending | Select-Object -First 1
    if ($appInstaller -and (Test-Path (Join-Path $appInstaller.InstallLocation 'AppxManifest.xml'))) {
      Add-AppxPackage -Register (Join-Path $appInstaller.InstallLocation 'AppxManifest.xml') -DisableDevelopmentMode
      $env:PATH = "$($appInstaller.InstallLocation);$env:PATH"
      $appInstaller.InstallLocation | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
    }
  }
  if (-not (Get-Command winget.exe -ErrorAction SilentlyContinue)) {
    # Official bootstrap also installs App Installer when the runner lacks it.
    Install-PackageProvider -Name NuGet -Force -Scope CurrentUser | Out-Null
    Install-Module -Name Microsoft.WinGet.Client -RequiredVersion 1.29.380 -Force -Repository PSGallery -Scope CurrentUser | Out-Null
    Import-Module Microsoft.WinGet.Client -RequiredVersion 1.29.380
    Repair-WinGetPackageManager -AllUsers
  }
  $windowsApps = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps'
  $env:PATH = "$windowsApps;$env:PATH"
  $windowsApps | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
  Get-Command winget.exe -ErrorAction Stop | Format-List Name,Source
  winget --info
  if ($LASTEXITCODE -ne 0) { throw 'WinGet bootstrap health check failed' }
} catch {
  $_ | Format-List * -Force | Out-File -Encoding utf8 winget-bootstrap-error.txt
  throw
} finally {
  Stop-Transcript
}
