//! Conservative GitHub Releases updater for installed Windows builds.
//! Network I/O and downloads run off the render thread. The user must opt in.
//! GitHub's published SHA256SUMS.txt is checked before invoking our installer.
use serde::Deserialize;
use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

const API: &str = "https://api.github.com/repos/tanyapruck/DesktopCraft/releases/latest";
const RELEASES: &str = "https://github.com/tanyapruck/DesktopCraft/releases/download/";
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub version: String,
    pub installer: String,
    pub checksum: String,
    pub size: u64,
}
impl Release {
    fn valid(&self) -> bool {
        let chunks: Vec<&str> = self.version.split('.').collect();
        if chunks.len() != 3
            || chunks.iter().any(|v| v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()))
            || self.size < 50_000
            || self.size > 60_000_000
        {
            return false;
        }
        let base = format!("{RELEASES}v{}/", self.version);
        self.installer == format!("{base}DesktopCraft-v{}-Setup-x64.exe", self.version)
            && self.checksum == format!("{base}SHA256SUMS.txt")
    }
}

pub enum Event {
    Available(Release),
    Ready(PathBuf),
    Failure(String),
}
pub struct Client {
    tx: Sender<Event>,
    rx: Receiver<Event>,
}
impl Client {
    pub fn start() -> Option<Self> {
        if std::env::var_os("DESKTOPCRAFT_DISABLE_UPDATE_CHECK").is_some() {
            return None;
        }
        // Never contact GitHub from tests, a portable executable, or the setup file.
        let current = std::env::current_exe().ok()?;
        let installed = crate::host::install_dir().join("DesktopCraft.exe");
        if !current.to_string_lossy().eq_ignore_ascii_case(&installed.to_string_lossy()) {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        let checking = tx.clone();
        let _ = thread::Builder::new()
            .name("desktopcraft-release-check".into())
            .spawn(move || {
                if let Ok(Some(release)) = latest_release() {
                    if crate::host::version_newer(&release.version, env!("CARGO_PKG_VERSION")) {
                        let _ = checking.send(Event::Available(release));
                    }
                }
                // No release, network failure or a not-newer release are all nonfatal.
            });
        Some(Self { tx, rx })
    }
    pub fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
    pub fn download(&self, release: Release) {
        let tx = self.tx.clone();
        let _ = thread::Builder::new()
            .name("desktopcraft-update-download".into())
            .spawn(move || {
                let result = download_verified(&release);
                let _ = tx.send(match result {
                    Ok(path) => Event::Ready(path),
                    Err(message) => Event::Failure(message),
                });
            });
    }
}
fn powershell(body: &str, args: &[&str]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    // Fixed scripts with positional arguments: never interpolate server strings
    // into PowerShell source, and validate all URLs before they get here.
    // Windows PowerShell re-tokenizes arguments after -Command, even when
    // CreateProcess quoted them correctly. Hex-encode positional UTF-8 args
    // so paths containing spaces and shell punctuation survive intact.
    let encoded: Vec<String> = args.iter().map(|s| {
        s.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }).collect();
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", body])
        .args(encoded.iter().map(String::as_str))
        .creation_flags(CREATE_NO_WINDOW);
    let out = command.output().map_err(|_| "PowerShell could not start".to_string())?;
    if !out.status.success() {
        return Err("GitHub could not be reached or the download was rejected".into());
    }
    String::from_utf8(out.stdout).map_err(|_| "Invalid response encoding".into())
}
fn latest_release() -> Result<Option<Release>, String> {
    let script = format!(
        r#"& {{
            $ErrorActionPreference = 'Stop'
            $ProgressPreference = 'SilentlyContinue'
            [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
            $r = Invoke-RestMethod -Uri '{}' -Headers @{{'User-Agent'='DesktopCraft-Updater'}} -TimeoutSec 20
            $version = ([string]$r.tag_name).TrimStart('v')
            $filename = "DesktopCraft-v$version-Setup-x64.exe"
            $installer = $r.assets | Where-Object {{ $_.name -eq $filename }} | Select-Object -First 1
            $checksum = $r.assets | Where-Object {{ $_.name -eq 'SHA256SUMS.txt' }} | Select-Object -First 1
            if ($null -ne $installer -and $null -ne $checksum) {{
                @{{version=$version; installer=$installer.browser_download_url; checksum=$checksum.browser_download_url; size=[long]$installer.size}} | ConvertTo-Json -Compress
            }}
        }}"#,
        API
    );
    let stdout = powershell(&script, &[])?;
    if stdout.trim().is_empty() {
        return Ok(None);
    }
    let release: Release =
        serde_json::from_str(stdout.trim()).map_err(|_| "Invalid release metadata".to_string())?;
    Ok(release.valid().then_some(release))
}
fn download_verified(release: &Release) -> Result<PathBuf, String> {
    if !release.valid() {
        return Err("Release metadata failed validation".into());
    }
    let root = std::env::temp_dir().join(format!("DesktopCraftUpdate-{}", std::process::id()));
    std::fs::create_dir_all(&root).map_err(|_| "Could not create update directory".to_string())?;
    let filename = format!("DesktopCraft-v{}-Setup-x64.exe", release.version);
    let exe = root.join(&filename);
    let checksum = root.join("SHA256SUMS.txt");
    let download_script = r#"& {
        param([string]$installerHex, [string]$checksumHex, [string]$outputHex, [string]$sumfileHex, [string]$filenameHex, [string]$sizeHex)
        function DecodeHex([string]$hex) {
            $bytes = New-Object byte[] ($hex.Length / 2)
            for ($i = 0; $i -lt $bytes.Length; $i++) {
                $bytes[$i] = [Convert]::ToByte($hex.Substring($i * 2, 2), 16)
            }
            [Text.Encoding]::UTF8.GetString($bytes)
        }
        $installer = DecodeHex $installerHex
        $checksumUrl = DecodeHex $checksumHex
        $output = DecodeHex $outputHex
        $sumfile = DecodeHex $sumfileHex
        $filename = DecodeHex $filenameHex
        $size = [long](DecodeHex $sizeHex)
        $ErrorActionPreference = 'Stop'
        $ProgressPreference = 'SilentlyContinue'
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $headers = @{'User-Agent'='DesktopCraft-Updater'}
        Invoke-WebRequest -Uri $installer -Headers $headers -UseBasicParsing -TimeoutSec 120 -OutFile $output
        Invoke-WebRequest -Uri $checksumUrl -Headers $headers -UseBasicParsing -TimeoutSec 30 -OutFile $sumfile
        $file = Get-Item -LiteralPath $output
        if ($file.Length -ne $size) { throw 'Installer length mismatch' }
        $matches = @(
            Get-Content -LiteralPath $sumfile | Where-Object {
                $_ -match ('^[0-9a-fA-F]{64}\s+\*?' + [regex]::Escape($filename) + '\s*$')
            }
        )
        if ($matches.Count -ne 1) { throw 'Missing or ambiguous checksum' }
        $expected = ($matches[0] -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { throw 'Installer SHA256 mismatch' }
        'VERIFIED'
    }"#;
    let result = powershell(
        download_script,
        &[
            &release.installer,
            &release.checksum,
            &exe.to_string_lossy(),
            &checksum.to_string_lossy(),
            &filename,
            &release.size.to_string(),
        ],
    );
    let _ = std::fs::remove_file(&checksum);
    if result.as_deref().map(str::trim) == Ok("VERIFIED") {
        Ok(exe)
    } else {
        let _ = std::fs::remove_file(&exe);
        Err("Update download or SHA-256 verification failed. No installer was run.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn powershell_preserves_unicode_and_spaces_in_arguments() {
        let script = r#"& { param([string]$h)
            $bytes = New-Object byte[] ($h.Length / 2)
            for ($i=0; $i -lt $bytes.Length; $i++) { $bytes[$i] = [Convert]::ToByte($h.Substring($i*2,2),16) }
            [Text.Encoding]::UTF8.GetString($bytes)
        }"#;
        let path = r"C:\Users\Test Name\with spaces\Setup.exe";
        assert_eq!(powershell(script, &[path]).unwrap().trim(), path);
    }
    #[test]
    fn validates_release_urls_and_version() {
        let mut r = Release {
            version: "0.1.1".into(),
            installer: format!("{RELEASES}v0.1.1/DesktopCraft-v0.1.1-Setup-x64.exe"),
            checksum: format!("{RELEASES}v0.1.1/SHA256SUMS.txt"),
            size: 3_000_000,
        };
        assert!(r.valid());
        r.installer = "https://example.org/installer.exe".into();
        assert!(!r.valid());
        r.installer = format!("{RELEASES}v0.1.1/DesktopCraft-v0.1.1-Setup-x64.exe");
        r.version = "0.1.1;Invoke-Expression".into();
        assert!(!r.valid());
    }
}
