# DesktopCraft v0.1.0 — Windows build and distribution

DesktopCraft builds native Windows x64 executables from Rust. The current stable version is defined in `Cargo.toml`.

## Build locally

On Windows x64 with the stable Rust toolchain:

```powershell
cargo test --locked --release
cargo build --locked --release
./scripts/package-release.ps1
```

For an MSI, install the WiX Toolset command-line tool (version 7.0.0 with the .NET 8 SDK) and run:

```powershell
./scripts/package-msi.ps1
./scripts/package-release.ps1  # refreshes SHA256SUMS.txt to include the MSI
```

## Release files

- `DesktopCraft-v<version>-Setup-x64.exe`: standalone self-installer for the current Windows user.
- `desktopcraft-<version>-windows-x64.msi`: Windows Installer format prepared for ArtCraft compatibility testing. It uses per-user installation and is intended for a clean installation, not on top of the standalone EXE.
- `SHA256SUMS.txt`: SHA-256 integrity manifest for the installer assets.

The signed-in user's workspace data is stored outside the installed program directory and is retained when an MSI is uninstalled. The MSI refuses to overwrite an existing standalone installation to prevent conflicting installer ownership.

## GitHub Actions

`.github/workflows/windows-installer.yml` runs Rust tests, builds the executable, packages an EXE and MSI, and smoke-tests MSI install/uninstall on a disposable Windows runner.

Successful runs upload the assets to GitHub Actions artifacts. Only pushing a matching `vX.Y.Z` tag publishes a GitHub Release; normal pushes and Pull Requests do not publish release packages.

## Update behavior

The installed standalone app checks the latest GitHub Release asynchronously and prompts before installing any newer version. It verifies the asset size and published SHA-256 checksum before launching the approved update. It does not silently install updates or downgrade.

**Private beta limitation:** The current updater uses unauthenticated GitHub requests and therefore cannot read releases in a private repository. Invited beta testers must download newer installers manually. No GitHub access tokens are embedded in DesktopCraft.

The Windows MSI is a separate install mechanism; it must be updated with a newer compatible MSI and is not intended to be overwritten by the EXE updater.

## Readiness

Before inviting additional testers or requesting official ArtCraft Launcher inclusion, review executable signing, code and asset provenance, real Windows install/uninstall/upgrade behavior, antivirus alerts, Explorer integration and the differing EXE/MSI migration paths.

The Windows installer files are unsigned, and SmartScreen may display a warning. The product does not claim official ArtCraft branding or endorsement.

See [Private beta guide](docs/PRIVATE_BETA.md) and [ArtCraft submission preparation](docs/ARTCRAFT_SUBMISSION.md).
