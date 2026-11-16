# ArtCraft Launcher — DesktopCraft submission preparation

**Status: preparation only; NOT submitted.** The repository is private, and no ArtCraft maintainer has approved DesktopCraft. Do not claim official ArtCraft or Craft App Suite affiliation.

## Candidate metadata

| Field | Proposed value |
| --- | --- |
| ID | `desktopcraft` |
| App name | DesktopCraft |
| Category | Community app — Windows desktop organization |
| Tagline | Organize your desktop with customizable boxes, tabs and pages |
| Repository | `tanyapruck/DesktopCraft` (currently private) |
| Maintainer / Publisher | DesktopCraft contributors (must confirm public-facing legal attribution) |
| MSI Publisher | `DesktopCraft contributors` (must exactly match MSI metadata for Launcher detection) |
| Windows support | Windows x64; Windows 11 beta validation required |
| Other platforms | Not supported |
| License | MIT for the project's original source; third-party notices in `NOTICE.md` |
| Windows MSI asset | `desktopcraft-<version>-windows-x64.msi` |
| Standalone EXE | `DesktopCraft-v<version>-Setup-x64.exe` |
| Checksums | `SHA256SUMS.txt`, containing both assets when packaged |
| Icon | `assets/desktopcraft.ico` (redistribution rights must be confirmed) |

## Packaging / verification

- A WiX-built per-user MSI is authored in `packaging/DesktopCraft.wxs`.
- The MSI does not run the standalone EXE installer as an installer; Windows Installer owns the files, shortcuts and menu registrations.
- It blocks installation on top of an existing standalone EXE registration to avoid conflicting uninstall or file ownership.
- The GitHub Windows workflow should build EXE + MSI and smoke-test MSI installation, removal, and preservation of workspace data on a disposable runner.
- Never mark MSI ready for testers until CI passes and clean-account manual installation / upgrade paths are tested.

## Blocks before ArtCraft catalog proposal

1. **Repository privacy:** Public ArtCraft Launcher catalog is built around an approved, pinned repo and unauthenticated GitHub downloads. A private GitHub Release is not a supported invite-only catalog source.
2. **Maintainer approval:** New third-party catalog entries require an ArtCraft maintainer's decision. Do not create a catalog PR while the project is private or before approval.
3. **Source and assets:** Verify the project's code provenance, third-party dependencies and right to redistribute icon assets.
4. **Packaging:** Confirm the MSI is discoverable by the launcher's Windows installed-app detection (DisplayName, Version, Publisher), prevents invalid downgrades, preserves user data, and updates correctly after new versions.
5. **End-user verification:** Windows 10/11 compatibility, clean-account installation, upgrades, uninstall, Explorer integration, anti-malware/code-signing warnings and cross-device tests.
6. **Details page:** After approval, supply product description, verified screenshots and icon under the ArtCraft-maintainer-requested license and asset format.

References:
- https://github.com/storytold/craft-launcher
- https://github.com/storytold/craft-launcher/blob/main/AGENTS.md
- https://github.com/storytold/craft-launcher/issues/1114 (proposed manifest; not assumed available)

No ArtCraft branding or names are included in DesktopCraft's executable branding.
