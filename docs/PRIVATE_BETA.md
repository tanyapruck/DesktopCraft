# DesktopCraft — private beta distribution

DesktopCraft is distributed from a **private GitHub repository** during this beta. This document is for the repository owner and invited testers. It is not an ArtCraft Launcher catalog entry.

## Invite a tester

1. Go to **Repository Settings → Collaborators → Add people**.
2. Invite the tester's GitHub username. The invitation must be accepted.
3. Send the tester the private [DesktopCraft release page](https://github.com/tanyapruck/DesktopCraft/releases/tag/v0.1.0).
4. Tell testers not to forward or publicly re-upload installers. GitHub's access control does not revoke downloaded copies.

**Important:** GitHub repository collaborators can also see and clone **source code**. If you need installer-only access with no source access, use a separate private distribution service or private binary-only repository with appropriately scoped access.

## Which installer?

- **Existing beta / recommended:** `DesktopCraft-v0.1.0-Setup-x64.exe` on the private Release page. This is the tested self-installer and keeps existing boxes, pages and settings when upgrading via the same EXE format.
- **ArtCraft preparation / experimental:** `desktopcraft-0.1.0-windows-x64.msi` in **Actions → Windows installer → Artifacts** after the new MSI CI job passes. Use only on a clean test account until separate MSI upgrade compatibility is validated. **Do not install the MSI over an existing self-installed DesktopCraft.** The MSI blocks that overlap.
- `SHA256SUMS.txt` contains the expected SHA-256 digest for available packages. Verify checksums before installation, especially when files are copied outside GitHub.

The MSI is a different Windows Installer ownership mechanism. To move from the EXE installer to MSI, first uninstall the EXE-installed app from Windows Settings, verify workspace data is backed up, and then run the MSI. **The MSI has not been released publicly or approved by ArtCraft.**

## Updating private builds

The current in-app version checker uses anonymous GitHub Releases requests. **Those requests cannot read a private repository**, so invite-only builds do **not** support in-app GitHub update downloads. This is expected, not a licensing or installation error.

For private beta updates, invited collaborators should open the private Releases page, download the newest installer in the **same installer format**, check its checksum and run it. Do not add a GitHub PAT to DesktopCraft's source code, executable or public documentation.

## Tester checklist

- Verify **Windows 11 x64** launch and exit, and test using a non-administrator Windows account.
- Back up any important files before testing drag/drop in linked **Folder Views**, because those operations move files.
- Test virtual box pinning separately; virtual pins should not move files on disk.
- Verify page switching, icon visibility, right-click actions, rename, Settings and persistence after restart.
- Test with Fences and other desktop-organizer software **closed** to prevent simultaneous desktop control.
- Report reproducible issues in the repository's **Issues** tab, omitting personal file paths, private documents, screenshots containing sensitive information and access tokens.

## CI and release workflow

Every push or pull request runs tests, builds the Windows EXE and MSI, and runs an isolated Windows MSI install/uninstall smoke test. Successful runs upload **private GitHub Actions artifacts**. Publishing assets to GitHub Releases only happens on a matching `vX.Y.Z` tag, after the build/tests pass. Normal pushes **do not** create releases.

Before a production rollout, review signing, provenance, the user-supplied app icon's redistribution rights, MSI/EXE migration, Explorer desktop behavior, and Windows compatibility.

## Keeping the repo private

Do not submit a catalog PR to `storytold/craft-launcher` yet. ArtCraft Launcher currently uses an approved, compiled-in catalog and unauthenticated GitHub release URLs; it cannot install from this private source by default. Prepare packaging privately and request maintainer approval when a public release is authorized.
