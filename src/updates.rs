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
const REPO: &str = "tanyapruck/DesktopCraft";
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


#[derive(Deserialize)]
struct GhAsset {
    name: String,
    url: String,
    size: u64,
}
#[derive(Deserialize)]
struct GhRelease {
    #[serde(rename = "tagName")]
    tag_name: String,
    assets: Vec<GhAsset>,
}

fn parse_github_cli_release(json: &str) -> Option<Release> {
    let response: GhRelease = serde_json::from_str(json).ok()?;
    let version = response.tag_name.strip_prefix('v')?.to_owned();
    let installer_name = format!("DesktopCraft-v{version}-Setup-x64.exe");
    let installer_assets: Vec<_> = response.assets.iter()
        .filter(|asset| asset.name == installer_name)
        .collect();
    let checksum_assets: Vec<_> = response.assets.iter()
        .filter(|asset| asset.name == "SHA256SUMS.txt")
        .collect();
    if installer_assets.len() != 1 || checksum_assets.len() != 1 {
        return None;
    }
    let release = Release {
        version,
        installer: installer_assets[0].url.clone(),
        checksum: checksum_assets[0].url.clone(),
        size: installer_assets[0].size,
    };
    release.valid().then_some(release)
}

/// Use only an existing authenticated GitHub CLI session. Never ask for,
/// inspect, log or embed the user's GitHub access token.
fn trusted_github_cli_executable() -> Result<PathBuf, String> {
    // Do not execute an arbitrary gh.exe from the working directory or PATH
    // of a shortcut. A release updater has a particularly sensitive trust
    // boundary; only use conventional official installation locations.
    let mut roots = Vec::new();
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(key) {
            roots.push(PathBuf::from(root).join("GitHub CLI").join("gh.exe"));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Programs").join("GitHub CLI").join("gh.exe"));
    }
    roots.into_iter().find(|candidate| candidate.is_file())
        .ok_or_else(|| "Official GitHub CLI installation was not found".into())
}

fn github_cli(args: &[&str]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let executable = trusted_github_cli_executable()?;
    let output = Command::new(executable)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_HTTP_TIMEOUT", "20")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|_| "GitHub CLI is not installed or available".to_string())?;
    if !output.status.success() {
        return Err("Authenticated GitHub CLI access to the private release failed".into());
    }
    String::from_utf8(output.stdout)
        .map_err(|_| "GitHub CLI returned invalid output".into())
}

fn github_cli_latest_release() -> Result<Option<Release>, String> {
    let output = github_cli(&["release", "view", "--repo", REPO, "--json", "tagName,assets"])?;
    Ok(parse_github_cli_release(&output))
}

pub const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

pub enum Event {
    Available(Release),
    Current,
    Ready(PathBuf),
    Failure(String),
}

pub enum UiState {
    Disabled,
    Idle,
    Checking,
    Current,
    Available(Release),
    Downloading,
    Ready(PathBuf),
    Failed(String),
}
impl UiState {
    pub fn can_check(&self) -> bool {
        matches!(self, Self::Idle | Self::Current | Self::Available(_) | Self::Failed(_))
    }
    pub fn header(&self) -> &'static str {
        match self {
            Self::Disabled => "Updates",
            Self::Checking => "Checking...",
            Self::Available(_) => "Update available",
            Self::Downloading => "Downloading...",
            Self::Ready(_) => "Install update",
            _ => "Check for update",
        }
    }
}
#[derive(Clone, Copy)]
pub enum Action { Check, Download, Install }

pub struct Client {
    tx: Sender<Event>,
    rx: Receiver<Event>,
}
impl Client {
    pub fn start() -> Option<Self> {
        if std::env::var_os("DESKTOPCRAFT_DISABLE_UPDATE_CHECK").is_some() {
            return None;
        }
        // Store/MSIX, portable and setup builds are not self-updated.
        let current = std::env::current_exe().ok()?;
        let installed = crate::host::install_dir().join("DesktopCraft.exe");
        if !current.to_string_lossy().eq_ignore_ascii_case(&installed.to_string_lossy()) {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        Some(Self { tx, rx })
    }
    pub fn check(&self) {
        let tx = self.tx.clone();
        let failed = self.tx.clone();
        if thread::Builder::new().name("desktopcraft-release-check".into()).spawn(move || {
            let event = match latest_release() {
                Ok(Some(release)) if crate::host::version_newer(&release.version, env!("CARGO_PKG_VERSION")) => Event::Available(release),
                Ok(Some(_)) => Event::Current,
                Ok(None) => Event::Failure("No verified release metadata was found.".into()),
                Err(reason) => Event::Failure(reason),
            };
            let _ = tx.send(event);
        }).is_err() {
            let _ = failed.send(Event::Failure("Could not start update check.".into()));
        }
    }
    pub fn poll(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
    pub fn download(&self, release: Release) {
        let tx = self.tx.clone();
        let failed = self.tx.clone();
        if thread::Builder::new()
            .name("desktopcraft-update-download".into())
            .spawn(move || {
                let result = download_verified(&release);
                let _ = tx.send(match result {
                    Ok(path) => Event::Ready(path),
                    Err(message) => Event::Failure(message),
                });
            }).is_err() {
            let _ = failed.send(Event::Failure("Could not start update download.".into()));
        }
    }
}

#[cfg(test)]
#[test]
fn update_state_prevents_overlapping_operations() {
    assert_eq!(CHECK_INTERVAL.as_secs(), 3600);
    assert!(UiState::Idle.can_check());
    assert!(UiState::Current.can_check());
    assert!(UiState::Failed("network".into()).can_check());
    assert!(!UiState::Disabled.can_check());
    assert!(!UiState::Checking.can_check());
    assert!(!UiState::Downloading.can_check());
    assert!(!UiState::Ready(PathBuf::from("setup.exe")).can_check());
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
    // The unauthenticated Releases API is appropriate for public projects.
    // A private repository returns 404 without authentication. Fall back to
    // the user's own read-only GitHub CLI session, if they have one.
    match powershell(&script, &[]) {
        Ok(stdout) if !stdout.trim().is_empty() => {
            let release: Release = serde_json::from_str(stdout.trim())
                .map_err(|_| "Invalid release metadata".to_string())?;
            Ok(release.valid().then_some(release))
        }
        _ => github_cli_latest_release(),
    }
}

/// For private GitHub Releases, let the authenticated GitHub CLI fetch two
/// exact assets. Never persist, print or pass any access tokens to this app.
fn github_cli_download_verified(
    release: &Release,
    directory: &std::path::Path,
) -> Result<(), String> {
    if !release.valid() {
        return Err("Private release metadata failed validation".into());
    }
    let tag = format!("v{}", release.version);
    let filename = format!("DesktopCraft-v{}-Setup-x64.exe", release.version);
    let dir = directory.to_string_lossy();
    github_cli(&[
        "release", "download", &tag,
        "--repo", REPO, "--dir", &dir,
        "--pattern", &filename,
        "--pattern", "SHA256SUMS.txt",
    ])?;

    // The official SHA256SUMS.txt is required even after an authenticated
    // download; a valid GitHub login alone is NOT sufficient verification.
    let installer = directory.join(&filename);
    let checksum = directory.join("SHA256SUMS.txt");
    let verify_script = r#"& {
        param([string]$exeHex, [string]$sumHex, [string]$fileHex, [string]$sizeHex)
        function D([string]$h) {
            $b = New-Object byte[] ($h.Length / 2)
            for($i=0;$i -lt $b.Length;$i++){
                $b[$i]=[Convert]::ToByte($h.Substring($i*2,2),16)
            }
            [Text.Encoding]::UTF8.GetString($b)
        }
        $exe=D $exeHex; $sum=D $sumHex
        $filename=D $fileHex; $size=[long](D $sizeHex)
        $ErrorActionPreference='Stop'
        if((Get-Item -LiteralPath $exe).Length -ne $size){
            throw 'Private release size mismatch'
        }
        $lines=@(Get-Content -LiteralPath $sum | Where-Object {
            $_ -match ('^[0-9a-fA-F]{64}\s+\*?' + [regex]::Escape($filename) + '\s*$')
        })
        if($lines.Count -ne 1){throw 'Missing or duplicate SHA-256 entry'}
        $expected=($lines[0] -split '\s+')[0].ToLowerInvariant()
        $actual=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        if($expected -ne $actual){throw 'Private release SHA-256 mismatch'}
        'VERIFIED'
    }"#;
    match powershell(
        verify_script,
        &[
            &installer.to_string_lossy(),
            &checksum.to_string_lossy(),
            &filename,
            &release.size.to_string(),
        ],
    ) {
        Ok(result) if result.trim() == "VERIFIED" => Ok(()),
        _ => Err("Authenticated download failed SHA-256 or size validation".into()),
    }
}

fn download_verified(release: &Release) -> Result<PathBuf, String> {
    if !release.valid() {
        return Err("Release metadata failed validation".into());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Update staging clock unavailable".to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "DesktopCraftUpdate-{}-{stamp}", std::process::id()
    ));
    std::fs::create_dir(&root)
        .map_err(|_| "Could not create a fresh update directory".to_string())?;
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
    if result.as_deref().map(str::trim) == Ok("VERIFIED") {
        let _ = std::fs::remove_file(&checksum);
        return Ok(exe);
    }
    // A private release cannot be downloaded via the public API. Remove any
    // partial or untrusted HTTP downloads before trying authenticated GitHub CLI.
    let _ = std::fs::remove_file(&exe);
    let _ = std::fs::remove_file(&checksum);
    let verified = github_cli_download_verified(release, &root);
    let _ = std::fs::remove_file(&checksum);
    if verified.is_ok() {
        Ok(exe)
    } else {
        let _ = std::fs::remove_file(&exe);
        Err(
            "Update could not be verified. For private DesktopCraft releases, sign in with GitHub CLI (gh auth login) and retry; no installer was run."
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_authenticated_private_release_only_with_two_exact_assets() {
        let data = r#"{
          "tagName":"v0.2.0",
          "assets":[
            {"name":"DesktopCraft-v0.2.0-Setup-x64.exe","url":"https://github.com/tanyapruck/DesktopCraft/releases/download/v0.2.0/DesktopCraft-v0.2.0-Setup-x64.exe","size":5000000},
            {"name":"SHA256SUMS.txt","url":"https://github.com/tanyapruck/DesktopCraft/releases/download/v0.2.0/SHA256SUMS.txt","size":101}
          ]
        }"#;
        let rel = parse_github_cli_release(data).unwrap();
        assert_eq!(rel.version, "0.2.0");
        assert_eq!(rel.size, 5_000_000);
        let swapped = data.replace(
            "https://github.com/tanyapruck/DesktopCraft/releases/download/",
            "https://untrusted.example/releases/"
        );
        assert!(parse_github_cli_release(&swapped).is_none());
        let missing = data.replace("SHA256SUMS.txt", "not-the-checksum.txt");
        assert!(parse_github_cli_release(&missing).is_none());
        let duplicated = data.replace(
            r#"{"name":"SHA256SUMS.txt","url":"https://github.com/tanyapruck/DesktopCraft/releases/download/v0.2.0/SHA256SUMS.txt","size":101}"#,
            r#"{"name":"SHA256SUMS.txt","url":"https://github.com/tanyapruck/DesktopCraft/releases/download/v0.2.0/SHA256SUMS.txt","size":101},{"name":"SHA256SUMS.txt","url":"https://github.com/tanyapruck/DesktopCraft/releases/download/v0.2.0/SHA256SUMS.txt","size":101}"#
        );
        assert!(parse_github_cli_release(&duplicated).is_none());
    }
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
    #[ignore = "requires authenticated GitHub CLI and a live private DesktopCraft release"]
    fn private_release_live_download_verifies_sha256_without_installing() {
        let release = github_cli_latest_release().unwrap().unwrap();
        assert!(release.valid());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "desktopcraft-private-download-test-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let result = github_cli_download_verified(&release, &dir);
        if result.is_ok() {
            let filename = format!("DesktopCraft-v{}-Setup-x64.exe", release.version);
            assert_eq!(
                std::fs::metadata(dir.join(filename)).unwrap().len(),
                release.size
            );
        }
        let _ = std::fs::remove_dir_all(dir);
        result.unwrap();
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
