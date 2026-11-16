# DesktopCraft 0.1.0 — GitHub source and Windows installer

This repository is the DesktopCraft **v0.1.11 source snapshot**, refactored and intentionally renumbered as the start of the **0.1.0 release line**. Version **0.2.0** is developed separately. This is **not** the historical v0.1.0 source snapshot.

## Build (Windows x64)

~~~powershell
cargo test --locked --release
cargo build --locked --release
./scripts/package-release.ps1
~~~

The resulting DesktopCraft-v0.1.0-Setup-x64.exe is the compiled executable renamed to activate its built-in self-installer. Installing/updating is per-user, without administrator rights, and preserves Workspace, Boxes, Pages, Rules, Settings, and start-at-sign-in preference.

**Unsigned installer:** Windows may show a SmartScreen warning. No claim is made that the installer is code-signed or trusted by SmartScreen.

## GitHub Actions and Releases

The **Windows installer** workflow runs tests, builds with stable Rust on a GitHub Windows runner, checks version resources, and uploads the installer and SHA256SUMS.txt. A matching version tag (for example v0.1.0) additionally publishes a GitHub Release. A tag differing from the Cargo version is rejected.

## Version check and approved in-place updates

The installed application checks the latest stable release from [GitHub Releases](https://github.com/tanyapruck/DesktopCraft/releases) asynchronously. If a newer release has the correctly named installer and SHA256SUMS.txt, the application prompts the user.

After a user clicks **Yes**, the application downloads both assets in the background, checks the installer size and SHA-256, then launches its existing in-place installer with the approved-update flag. No silent installation or downgrades. Errors do not change the installed program.

The version checker does not run in local development builds or screenshot/self-test mode. Set DESKTOPCRAFT_DISABLE_UPDATE_CHECK=1 to disable checking on installed builds.

## Performance refactor

The icon GPU-upload loop now processes at most two decoded icons per frame, using a short time budget to smooth frame-time spikes. The worker continues extracting icons in the background. This prioritizes UI responsiveness while icons populate; it does not assert faster total startup or change on-disk file operations.

## Source publication cautions

Third-party Macroquad and Miniquad fork notices and license files are retained in vendor/. Verify redistribution/provenance and rights to all assets before broader distribution. No user workspace, logs, personal paths, proprietary investigation artifacts, or archived installer are part of this source repository.
