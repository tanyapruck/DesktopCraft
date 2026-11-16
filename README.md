# DesktopCraft

**A customizable desktop organizer for Windows.**

[![Release](https://img.shields.io/github/v/release/tanyapruck/DesktopCraft?label=release)](https://github.com/tanyapruck/DesktopCraft/releases/latest)
[![Windows x64](https://img.shields.io/badge/platform-Windows%20x64-0078D4)](https://github.com/tanyapruck/DesktopCraft/releases/latest)
[![Windows installer](https://github.com/tanyapruck/DesktopCraft/actions/workflows/windows-installer.yml/badge.svg)](https://github.com/tanyapruck/DesktopCraft/actions/workflows/windows-installer.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

DesktopCraft keeps your Windows desktop organized with movable boxes, tabs, and pages. Group your shortcuts and files, customize how items appear, and switch between workspaces without turning every group into a physical folder.

**Current release: v0.1.0**

[**Download DesktopCraft v0.1.0 for Windows (x64)**](https://github.com/tanyapruck/DesktopCraft/releases/download/v0.1.0/DesktopCraft-v0.1.0-Setup-x64.exe) · [All releases](https://github.com/tanyapruck/DesktopCraft/releases)

## Features

- **Desktop boxes** — Create, move, resize, and customize groups directly on your desktop.
- **Pages and tabs** — Keep separate layouts for different activities and switch between them from the page selector.
- **Virtual organization** — Pin files and shortcuts into boxes without moving the original files on disk.
- **Linked folder views** — Show the contents of a real folder inside a box, with filtering and sorting.
- **Drag and drop** — Organize items across boxes, desktop areas, and linked folders.
- **Personalization** — Adjust icon sizes, box appearance, roll-up behavior, Peek shortcuts, and sorting rules.
- **Windows integration** — Open files, rename items, view Properties, and use Windows Shell file operations.
- **Update notifications** — Check GitHub Releases for new versions and choose when to install an update.

## Getting started

1. Download **DesktopCraft-v0.1.0-Setup-x64.exe** from the [latest release](https://github.com/tanyapruck/DesktopCraft/releases/latest).
2. Run the installer. DesktopCraft installs for your Windows account without requiring administrator privileges.
3. Create boxes and organize desktop items into the groups and pages you prefer.
4. Use each box's menu and **Settings** to customize layouts, tabs, linked folders, and shortcuts.

DesktopCraft saves your workspace settings so your arrangement is available when you open it again.

> **Windows SmartScreen:** The installer is currently unsigned, so Windows may display a publisher or reputation warning. Always download it from this repository's official GitHub Releases page.

## How DesktopCraft handles files

DesktopCraft supports two distinct kinds of boxes:

| Box type | What it does | Effect on files |
| --- | --- | --- |
| **Regular box** | Organizes files and shortcuts as virtual pins | Keeps the original files in their existing locations |
| **Linked folder view** | Displays the contents of a selected Windows folder | Dragging files into or out of the folder can **move the actual files** |

Moving an item between regular boxes changes its organization, not the file's location. A linked folder view performs real filesystem operations, so take care when dragging items into it.

Deleting an item through Windows' Recycle Bin is also a real filesystem action; it is not the same as removing a virtual pin.

## Windows desktop integration

DesktopCraft provides a Windows 11-style file-actions menu for items it manages, with common actions such as Open, Copy, Rename, Delete, and Properties. **Show more options** opens the native classic Windows Shell menu.

The DesktopCraft-drawn file menu is not the exact Explorer-owned Windows 11 modern context menu. Desktop icons outside DesktopCraft's managed regions continue to use Windows' own context menu.

## Updates

Installed copies of DesktopCraft check the [latest GitHub Release](https://github.com/tanyapruck/DesktopCraft/releases/latest) for a newer version.

If an update is available, DesktopCraft asks before downloading anything. After you approve it, the updater downloads the installer, verifies its size and published SHA-256 checksum, and starts the in-place update process. Your boxes, pages, rules, and settings are preserved by the existing installer.

Updates are **not installed automatically**. To disable update checks, set the `DESKTOPCRAFT_DISABLE_UPDATE_CHECK` environment variable.

## Build from source

**Requirements:** Windows x64 and the stable Rust toolchain.

```powershell
cargo test --locked --release
cargo build --locked --release
./scripts/package-release.ps1
```

The packaging script creates the Windows setup executable and `SHA256SUMS.txt` in `dist/`.

GitHub Actions also builds and tests the Windows installer. Release tags matching the version in `Cargo.toml` publish the generated installer to [GitHub Releases](https://github.com/tanyapruck/DesktopCraft/releases).

For the packaging and updater implementation, see [Release build documentation](RELEASE_BUILD.md).

## License and acknowledgements

DesktopCraft source is provided under the [MIT License](LICENSE). Third-party components, including locally adapted Macroquad and Miniquad code, retain their respective licenses and notices; see [NOTICE.md](NOTICE.md).

---

**DesktopCraft v0.1.0** · [Download](https://github.com/tanyapruck/DesktopCraft/releases/tag/v0.1.0) · [Report an issue](https://github.com/tanyapruck/DesktopCraft/issues)
