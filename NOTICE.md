# Third-party notices

The application uses Rust crates from crates.io, including `serde`, `serde_json`, `winapi`, `macroquad` and `miniquad` and their transitive dependencies. Check `Cargo.lock` for the exact dependency versions.

- `vendor/macroquad` is a local adaptation of Macroquad 0.4.16, under MIT OR Apache-2.0. The original MIT/Apache license files and upstream copyright notices are retained; the local changes accumulate high-resolution Windows wheel deltas across frames and report keyboard auto-repeat (so a held Backspace keeps deleting).
- `vendor/miniquad` is a local adaptation of Miniquad 0.4.11, under MIT OR Apache-2.0. The original MIT/Apache license files and upstream copyright notices are retained; it contains Windows desktop host/window-procedure changes.
- The application uses Microsoft Windows API definitions through winapi. Windows system fonts are loaded from the operating system at runtime, and no Windows font files are shipped.

These notices do not assert ownership of third-party dependencies. The separate project source license applies only where the project contributors hold the required rights. Review the source provenance and dependency license list before public distribution.
