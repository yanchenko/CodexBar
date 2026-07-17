<#
build-portable.ps1 — AgentBar Windows portable zip (self-contained).

Dual-mode (DontSpeak-style):
  - Default: full cargo release-ffi + dotnet publish self-contained + zip.
  - -SkipPublish: re-zip an existing stage dir (mechanics check only).

Output: apps/windows/installer/Output/agentbar-<ver>-windows-<x86_64|aarch64>.zip

Usage:
  pwsh apps/windows/installer/build-portable.ps1 [-Arch x64|arm64] [-SkipPublish]
#>
param(
    [ValidateSet('x64', 'arm64')][string]$Arch = 'x64',
    [switch]$SkipPublish
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot\..\..\..").Path
$rust = Join-Path $repo 'rust'
$stage = Join-Path $PSScriptRoot "portable\agentbar-portable-$Arch"
$outDir = Join-Path $PSScriptRoot 'Output'

# Version from version.env (MARKETING_VERSION) or cargo package version fallback.
$ver = '0.1.0'
$versionEnv = Join-Path $repo 'version.env'
if (Test-Path $versionEnv) {
    Get-Content $versionEnv | ForEach-Object {
        if ($_ -match '^\s*MARKETING_VERSION\s*=\s*(.+)\s*$') { $ver = $Matches[1].Trim() }
    }
}
$fileVer = ($ver -split '-')[0]
if (($fileVer -split '\.').Count -eq 3) { $fileVer = "$fileVer.0" }

$archToken = if ($Arch -eq 'arm64') { 'aarch64' } else { 'x86_64' }
$zipName = "agentbar-$ver-windows-$archToken.zip"
$dotnetPlatform = if ($Arch -eq 'arm64') { 'ARM64' } else { 'x64' }
$cargoTarget = if ($Arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { $null }
$ffiDir = if ($cargoTarget) {
    Join-Path $rust "target\$cargoTarget\release-ffi"
} else {
    Join-Path $rust 'target\release-ffi'
}

if (-not $SkipPublish) {
    Write-Host "==> 1/3  cargo build --profile release-ffi -p ab-core (+ agentbar CLI)" -ForegroundColor Cyan
    Push-Location $rust
    try {
        $cargoArgs = @('build', '--profile', 'release-ffi', '-p', 'ab-core')
        if ($cargoTarget) { $cargoArgs += @('--target', $cargoTarget) }
        & cargo @cargoArgs
        if ($LASTEXITCODE) { throw "cargo release-ffi ab-core failed ($LASTEXITCODE)" }

        # CLI is useful next to the portable host (debug/ops); build release binary.
        $cliArgs = @('build', '--release', '-p', 'ab-cli')
        if ($cargoTarget) { $cliArgs += @('--target', $cargoTarget) }
        & cargo @cliArgs
        if ($LASTEXITCODE) { throw "cargo release ab-cli failed ($LASTEXITCODE)" }
    }
    finally { Pop-Location }

    $dll = Join-Path $ffiDir 'ab_core.dll'
    if (-not (Test-Path $dll)) { throw "ab_core.dll missing at $dll" }

    Write-Host "==> 2/3  dotnet publish WinUI (SELF-CONTAINED)" -ForegroundColor Cyan
    if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
    $csproj = Join-Path $repo 'apps\windows\winui\AgentBar.WinUI.csproj'
    $publishOut = & dotnet publish $csproj -c Release `
        -p:Platform=$dotnetPlatform -r "win-$Arch" --self-contained true `
        -p:WindowsAppSDKSelfContained=true `
        -p:Version=$ver -p:AssemblyVersion=$fileVer -p:FileVersion=$fileVer `
        -o $stage 2>&1
    if ($LASTEXITCODE) { $publishOut | Write-Host; throw "dotnet publish failed" }

    # Ensure ab_core.dll is present (csproj copies when found; re-copy for safety).
    Copy-Item $dll (Join-Path $stage 'ab_core.dll') -Force

    $cliRel = if ($cargoTarget) {
        Join-Path $rust "target\$cargoTarget\release\agentbar.exe"
    } else {
        Join-Path $rust 'target\release\agentbar.exe'
    }
    if (Test-Path $cliRel) {
        Copy-Item $cliRel (Join-Path $stage 'agentbar.exe') -Force
    }

    $license = Join-Path $repo 'LICENSE'
    if (Test-Path $license) { Copy-Item $license (Join-Path $stage 'LICENSE') -Force }
}
else {
    Write-Host "==> SkipPublish: reusing stage at $stage" -ForegroundColor DarkYellow
    if (-not (Test-Path $stage)) { throw "stage missing: $stage" }
}

Write-Host "==> 3/3  zip → Output\$zipName" -ForegroundColor Cyan
New-Item -ItemType Directory -Force $outDir | Out-Null
$zip = Join-Path $outDir $zipName
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -CompressionLevel Optimal
$mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host ("DONE → {0} ({1} MB)" -f $zip, $mb) -ForegroundColor Green
