# DesktopCraft read-only support collector. Does not launch DesktopCraft,
# install drivers, alter Explorer, move files, access file contents, or use network.
# Useful when the app cannot start to write its own logs.
[CmdletBinding()]
param(
  [string]$Executable = '',
  [string]$Output = ''
)
$ErrorActionPreference = 'Stop'
if (-not $Executable) {
  $Executable = Join-Path $PSScriptRoot 'DesktopCraft.exe'
  if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
    $Executable = Join-Path $PSScriptRoot 'DesktopCraft Setup.exe'
  }
}
if (-not $Output) {
  $Output = Join-Path $env:TEMP ('DesktopCraft-diagnostics-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.json')
}

$os = $null
try { $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop } catch { }
$video = @()
try {
  $video = @(Get-CimInstance Win32_VideoController -ErrorAction Stop | ForEach-Object {
    [ordered]@{ name = $_.Name; driver_version = $_.DriverVersion }
  })
} catch { }
$metadata = [ordered]@{
  app = 'DesktopCraft'
  collector_version = 1
  timestamp_utc = (Get-Date).ToUniversalTime().ToString('o')
  operating_system = if ($os) { $os.Caption } else { [Environment]::OSVersion.VersionString }
  os_build = if ($os) { $os.BuildNumber } else { [Environment]::OSVersion.Version.Build.ToString() }
  os_architecture = if ($os) { $os.OSArchitecture } else { $env:PROCESSOR_ARCHITECTURE }
  is_64bit_os = [Environment]::Is64BitOperatingSystem
  gpu = $video
  executable_found = (Test-Path -LiteralPath $Executable -PathType Leaf)
  executable_name = if (Test-Path -LiteralPath $Executable -PathType Leaf) { [IO.Path]::GetFileName($Executable) } else { $null }
  executable_machine = $null
  executable_sha256 = $null
  executable_signature_status = $null
  runtime_dlls_in_system32 = [ordered]@{}
  workspace_found = $false
  logs_found = $false
  recent_problem_codes = @()
  note = 'Read-only report. No desktop file paths, file contents or screenshots are collected.'
}

if ($metadata.executable_found) {
  $stream = [IO.File]::OpenRead($Executable)
  try {
    $reader = New-Object IO.BinaryReader($stream)
    if ($stream.Length -gt 64) {
      $stream.Position = 0x3c
      $offset = $reader.ReadInt32()
      if ($offset -ge 64 -and $offset -lt ($stream.Length - 6)) {
        $stream.Position = $offset
        $signature = $reader.ReadUInt32()
        if ($signature -eq 0x4550) {
          $machine = $reader.ReadUInt16()
          $metadata.executable_machine = switch ($machine) {
            0x8664 { 'x64' }
            0x14c { 'x86' }
            0xAA64 { 'ARM64' }
            default { ('unknown-0x{0:X4}' -f $machine) }
          }
        }
      }
    }
  } finally { $stream.Dispose() }
  try { $metadata.executable_sha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash } catch { }
  try { $metadata.executable_signature_status = [string](Get-AuthenticodeSignature -LiteralPath $Executable).Status } catch { }
}
foreach ($dll in @('vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll','ucrtbase.dll')) {
  $metadata.runtime_dlls_in_system32[$dll] = (Test-Path -LiteralPath (Join-Path $env:WINDIR ('System32\' + $dll)) -PathType Leaf)
}
$profiles = @(
  (Join-Path $env:APPDATA 'DesktopCraft'),
  (Join-Path $env:APPDATA 'Desktop Spaces')
)
$codes = @()
foreach ($profile in $profiles) {
  if (Test-Path -LiteralPath (Join-Path $profile 'data\workspace.json') -PathType Leaf) {
    $metadata.workspace_found = $true
  }
  foreach ($logname in @('desktopcraft.log','desktopcraft.1.log','desktopcraft.2.log')) {
    $logpath = Join-Path $profile $logname
    if (-not (Test-Path -LiteralPath $logpath -PathType Leaf)) { continue }
    $metadata.logs_found = $true
    try {
      Get-Content -LiteralPath $logpath -Tail 150 -ErrorAction Stop | ForEach-Object {
        $m = [regex]::Match($_, '(WARN|ERROR)\s+([a-z0-9_.-]+):', 'IgnoreCase')
        if ($m.Success) { $codes += ($m.Groups[1].Value.ToUpperInvariant() + ' ' + $m.Groups[2].Value) }
      }
    } catch { }
  }
}
$metadata.recent_problem_codes = @($codes | Select-Object -Last 30)
$metadata | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Output -Encoding UTF8
Write-Output 'DesktopCraft diagnostic report created (no automatic uploads):'
Write-Output $Output
Write-Output 'Review the JSON file before sharing it.'
