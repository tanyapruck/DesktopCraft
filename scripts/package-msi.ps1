# Creates an MSI from the validated Rust release binary (Windows only).
# WiX Toolset 5.0.2 CLI must be on PATH. Building does not install the app.
param([string]$Destination = 'dist')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$version = (Select-String -LiteralPath (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"(\d+\.\d+\.\d+)"' |
    Select-Object -First 1).Matches.Groups[1].Value
if (-not $version) { throw 'Invalid Rust package version.' }
$exe = Join-Path $root 'target\release\desktopcraft.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw 'Build target/release/desktopcraft.exe first.' }
if ((Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -ne "$version.0") {
    throw 'Rust executable and Cargo.toml version differ.'
}
if (-not [IO.Path]::IsPathRooted($Destination)) { $Destination = Join-Path $root $Destination }
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
$msi = Join-Path $Destination "desktopcraft-$version-windows-x64.msi"
Push-Location $root
try {
    & wix build -arch x64 -d "ProductVersion=$version" -d "BuildExe=$exe" -o "$msi" 'packaging\DesktopCraft.wxs'
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $msi)) { throw 'WiX MSI build failed.' }
}
finally { Pop-Location }
Write-Host "MSI prepared: $msi"
Write-Host 'The MSI is for clean per-user installs; it blocks overlap with the standalone EXE installer.'
