$ErrorActionPreference = 'Stop'
$releaseRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
& node (Join-Path $releaseRoot 'build-release.mjs') @args
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
