# Package DesktopCraft's self-installing EXE for GitHub Releases.
# The program is a self-installer when its filename contains "Setup".
param([string]$Destination = 'dist')
$ErrorActionPreference = 'Stop'
$project = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$version = (Select-String -LiteralPath (Join-Path $project 'Cargo.toml') -Pattern '^version\s*=\s*"(\d+\.\d+\.\d+)"' | Select-Object -First 1).Matches.Groups[1].Value
if (-not $version) { throw 'Cargo.toml has no semver package version.' }
$release = Join-Path $project 'target\release\desktopcraft.exe'
if (-not (Test-Path -LiteralPath $release)) { throw 'Missing release executable; run cargo build --locked --release first.' }
$info = (Get-Item -LiteralPath $release).VersionInfo
if ($info.FileVersion -ne "$version.0" -or $info.ProductVersion -ne "$version.0") {
    throw "Windows version resources don't match Cargo: $($info.FileVersion) / $($info.ProductVersion) versus $version.0"
}
if (-not [IO.Path]::IsPathRooted($Destination)) { $Destination = Join-Path $project $Destination }
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
$filename = "DesktopCraft-v$version-Setup-x64.exe"
$installer = Join-Path $Destination $filename
Copy-Item -LiteralPath $release -Destination $installer -Force
# When an MSI has already been built, include it in the same checksum list.
$msi = Join-Path $Destination "desktopcraft-$version-windows-x64.msi"
$artifacts = @($installer)
if (Test-Path -LiteralPath $msi) { $artifacts += $msi }
$hashLines = foreach ($path in ($artifacts | Sort-Object)) {
    $sha = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    "$sha  $([IO.Path]::GetFileName($path))"
}
[IO.File]::WriteAllText((Join-Path $Destination 'SHA256SUMS.txt'), (($hashLines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
Write-Output "Release assets ready in: $Destination"
$hashLines | ForEach-Object { Write-Output $_ }
Write-Output 'Note: unsigned self-installer; it may trigger Microsoft SmartScreen.'
