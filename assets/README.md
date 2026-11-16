# DesktopCraft application icon assets

The DesktopCraft monitor-and-folders icon is derived **only from the image the user supplied to this conversation** on 2026-11-13. It is not a stock image or asset taken from a third-party desktop manager.

- `desktopcraft-source.jpg`: 128×128 image, downsampled and compressed from the user's original 1254×1254 PNG for transfer into the local Windows project. The original uploaded PNG remains available in the conversation.
- `desktopcraft.ico`: PNG-backed Windows multi-resolution icon containing 16, 24, 32, 48, 64, 128 and 256 px variants. Generated from the downsampled copy; the 256 px image is upscaled.
- `desktopcraft-16.rgba`, `desktopcraft-32.rgba`, `desktopcraft-64.rgba`: RGBA pixel arrays used by the Macroquad/Miniquad window icon.
- `build.rs`: embeds the ICO into the Windows executable's `RT_ICON` / `RT_GROUP_ICON` resources at build time, without an external resource compiler.

Do not claim that these assets are licensed by the MIT license applying to the code unless the original image's rights have been independently verified. Before making a public GitHub release, obtain or confirm permission to distribute this illustration. Windows system fonts are not bundled.
