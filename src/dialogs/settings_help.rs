use super::*;

impl App {
    pub(super) fn settings_page_help(&mut self, p: &mut Painter, input: &Input, x: f32, y: f32, w: f32) {
        p.text("Your files, within reach", x, y - 4., 23., TEXT);
        for (i, (key, desc)) in [
            (
                "Drag title / box edge",
                "Move or resize; snaps to neighbor edges",
            ),
            ("Hold Shift while dragging", "Temporarily disable snapping"),
            ("Double-click box title", "Roll up or expand the box"),
            (
                "Hover rolled titlebar",
                "Reveal body; move into it to interact",
            ),
            ("Box menu (...)", "Tabs, folders, appearance & box options"),
            (
                "Drag a header onto another",
                "Combine as tabs; Shift-click or menu detaches",
            ),
            (
                "Ctrl / Alt + Left / Right",
                "Switch pages; create pages with + in pager",
            ),
            ("Ctrl + Z", "Undo last layout change"),
            (
                "2x click blank desktop",
                "Quick-hide or show all boxes (Behavior)",
            ),
            (
                "Pick screen pixel",
                "Sample any visible window colour; right-click cancels",
            ),
        ]
        .iter()
        .enumerate()
        {
            let yy = y + 40. + i as f32 * 26.;
            p.text(key, x, yy, 12., ACCENT);
            p.text(&p.fit(desc, 12., w - 230.), x + 225., yy, 12., TEXT);
        }
        let ver = format!("DesktopCraft v{}", env!("CARGO_PKG_VERSION"));
        p.text(&ver, x, y + 324., 14., MUTED);
        let (status_label, status_color) = if self.state.prefs.desktop_enabled {
            ("Status: Enabled", ACCENT)
        } else {
            ("Status: Disabled", Color::new(0.95, 0.65, 0.45, 1.))
        };
        p.text(status_label, x + 225., y + 324., 14., status_color);
        p.text(
            if self.state.prefs.desktop_enabled {
                "Disable turns off DesktopCraft desktop overlays while keeping Settings accessible."
            } else {
                "DesktopCraft is disabled. Enable anytime below, or close Settings to exit."
            },
            x,
            y + 356.,
            12.,
            MUTED,
        );
        p.text("Diagnostics & control", x, y + 394., 15., TEXT);

        if p.button(
            "Open diagnostic logs",
            Bounds {
                x,
                y: y + 432.,
                w: 215.,
                h: 38.,
            },
            false,
        ) {
            host::event(
                "INFO",
                "diagnostics.open_logs",
                "log folder requested from settings",
            );
            if let Err(error) = platform::open(&host::log_dir().to_string_lossy()) {
                self.notify(error, input.time);
            }
        }

        let enabled_now = self.state.prefs.desktop_enabled;
        if p.button(
            if enabled_now {
                "Disable DesktopCraft"
            } else {
                "Enable DesktopCraft"
            },
            Bounds {
                x: x + 232.,
                y: y + 432.,
                w: 235.,
                h: 38.,
            },
            !enabled_now,
        ) {
            self.desktop_toggle_requested = Some(!enabled_now);
        }
    }
}

#[cfg(windows)]
impl App {
    pub(super) fn settings_page_updates(&mut self, p: &mut Painter, x: f32, y: f32, w: f32) {
        use updates::{Action, UiState};
        p.text("Software updates", x, y, 24., TEXT);
        p.text(&format!("Installed version: v{}", env!("CARGO_PKG_VERSION")), x, y + 48., 15., MUTED);
        let (heading, detail) = match &self.update_state {
            UiState::Disabled => ("Managed outside DesktopCraft".to_string(), "Use Microsoft Store or download the installer from GitHub.".to_string()),
            UiState::Idle => ("Check for updates".to_string(), "Check the official DesktopCraft GitHub release.".to_string()),
            UiState::Checking => ("Checking for updates...".to_string(), "Contacting GitHub.".to_string()),
            UiState::Current => ("You're up to date".to_string(), "No newer verified release was found.".to_string()),
            UiState::Available(r) => (format!("DesktopCraft v{} is available", r.version), format!("Download size: {:.1} MB", r.size as f64 / 1_000_000.0)),
            UiState::Downloading => ("Downloading and verifying...".to_string(), "Checking installer size and SHA-256 checksum.".to_string()),
            UiState::Ready(_) => ("Update ready to install".to_string(), "DesktopCraft will close safely before installing.".to_string()),
            UiState::Failed(reason) => ("Update check or download failed".to_string(), reason.clone()),
        };
        let panel = Bounds { x, y: y + 95., w, h: 190. };
        round(panel, 8., alpha(WHITE, 0.06));
        p.text(&p.fit(&heading, 20., w-36.), x+18., y+115., 20., TEXT);
        p.text(&p.fit(&detail, 13., w-36.), x+18., y+155., 13., MUTED);
        let action = match &self.update_state {
            UiState::Available(_) => Some(("Download update", Action::Download)),
            UiState::Ready(_) => Some(("Install & restart", Action::Install)),
            state if state.can_check() => Some(("Check for update", Action::Check)),
            _ => None,
        };
        if let Some((label, action)) = action {
            if p.button(label, Bounds { x: x+18., y: y+216., w: 210., h: 38. }, true) {
                self.update_action = Some(action);
            }
        }
        p.text("Automatic checks: every hour for installed GitHub builds.", x, y+316., 14., MUTED);
        p.text("Updates require your approval. Your workspace is retained.", x, y+345., 14., MUTED);
        p.text("GitHub installers are unsigned; Windows may show a warning.", x, y+374., 14., MUTED);
    }
}
