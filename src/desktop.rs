//! Desktop mode: groups that live on the real Windows desktop.
//!
//! How it fits together (all supported Windows interfaces, nothing injected):
//! - One borderless tool window owned by the shell's desktop spans every monitor
//!   and is clipped (SetWindowRgn) to the groups, so the rest is Explorer's own
//!   desktop: its icons, wallpaper, right-click menu and drag-and-drop.
//! - A group holds real items from the Desktop and Public Desktop folders. Each
//!   grouped item's real Explorer icon is moved (IFolderView) under its group, so
//!   the desktop stays organized even when this program is closed.
//! - Items in a group use Explorer's context menu, Windows drag-and-drop and
//!   undoable shell file operations.
//! - Layouts are kept per monitor arrangement and switch automatically.
use crate::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Windows' canonical Recycle Bin shell folder (language-independent).
/// Matching the shell namespace GUID is safer than a localized display name
/// or a user-controlled ordinary file whose name happens to be "Recycle Bin".
fn is_recycle_bin_shell_item(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    (p.starts_with("::") || p.starts_with("shell:"))
        && p.contains("645ff040-5081-101b-9f08-00aa002f954e")
}

/// Snap the top-left of a DesktopCraft-rendered loose icon to the nearest
/// Explorer-sized desktop cell, searching outward for a free, unobscured slot.
/// This is independent of Explorer's FWF_SNAPTOGRID preference (which we never
/// silently disable). All coordinates are physical screen pixels.
#[cfg(test)]
fn snap_loose_icon(
    desired: (i32, i32),
    work: [i32; 4],
    spacing: (i32, i32),
    occupied: &[(i32, i32)],
    blocked: &[[i32; 4]],
) -> (i32, i32) {
    snap_loose_icon_phase(desired, work, spacing, occupied, blocked, (0, 0))
}
/// Explorer's icon grid is not always aligned to the work-area origin,
/// especially at nonstandard scaling on Windows Server. Calibrate the
/// phase from observed Explorer icon coordinates instead of forcing x=0.
fn grid_phase(work: [i32; 4], spacing: (i32, i32), positions: &[(i32, i32)]) -> (i32, i32) {
    let axis = |origin: i32, step: i32, coords: Vec<i32>| -> i32 {
        let step = step.max(1);
        let residues: Vec<i32> = coords
            .iter()
            .map(|v| (v - origin).rem_euclid(step))
            .collect();
        if residues.is_empty() {
            return 0;
        }
        residues
            .iter()
            .copied()
            .max_by_key(|candidate| {
                residues
                    .iter()
                    .filter(|r| {
                        let d = (**r - *candidate).abs();
                        d.min(step - d) <= 5
                    })
                    .count()
            })
            .unwrap_or(0)
    };
    (
        axis(work[0], spacing.0, positions.iter().map(|p| p.0).collect()),
        axis(work[1], spacing.1, positions.iter().map(|p| p.1).collect()),
    )
}
fn snap_loose_icon_phase(
    desired: (i32, i32),
    work: [i32; 4],
    spacing: (i32, i32),
    occupied: &[(i32, i32)],
    blocked: &[[i32; 4]],
    phase: (i32, i32),
) -> (i32, i32) {
    let (sx, sy) = (spacing.0.max(1), spacing.1.max(1));
    let max_col = ((work[2] - sx - phase.0).max(0) / sx).max(0);
    let max_row = ((work[3] - sy - phase.1).max(0) / sy).max(0);
    let base_col = ((desired.0 - work[0] - phase.0) as f64 / sx as f64).round() as i32;
    let base_row = ((desired.1 - work[1] - phase.1) as f64 / sy as f64).round() as i32;
    let base = (base_col.clamp(0, max_col), base_row.clamp(0, max_row));
    let point = |col: i32, row: i32| (work[0] + phase.0 + col * sx, work[1] + phase.1 + row * sy);
    let clear = |at: (i32, i32)| {
        !occupied
            .iter()
            .any(|p| (p.0 - at.0).abs() < sx / 2 && (p.1 - at.1).abs() < sy / 2)
            && !blocked.iter().any(|b| {
                at.0 < b[0] + b[2] && at.0 + sx > b[0] && at.1 < b[1] + b[3] && at.1 + sy > b[1]
            })
    };
    // A bounded ring search is cheap even on an icon-heavy desktop.
    for radius in 0i32..=12 {
        let mut candidate = None;
        let mut score = i64::MAX;
        for row in (base.1 - radius).max(0)..=(base.1 + radius).min(max_row) {
            for col in (base.0 - radius).max(0)..=(base.0 + radius).min(max_col) {
                if (col - base.0).abs().max((row - base.1).abs()) != radius {
                    continue;
                }
                let at = point(col, row);
                if !clear(at) {
                    continue;
                }
                let dx = (at.0 - desired.0) as i64;
                let dy = (at.1 - desired.1) as i64;
                let distance = dx * dx + dy * dy;
                if distance < score {
                    candidate = Some(at);
                    score = distance;
                }
            }
        }
        if let Some(at) = candidate {
            return at;
        }
    }
    point(base.0, base.1)
}

/// A desktop icon outside any group, drawn by DesktopCraft while Explorer's
/// own icon layer is switched off ("Show desktop icons").
#[derive(Clone, Debug)]
pub struct Loose {
    pub path: String,
    /// Lowercased path cached at Explorer refresh, never recomputed each frame.
    pub key: String,
    pub label: String,
    /// Explorer's stored position (screen, physical pixels).
    pub pos: (i32, i32),
}

pub enum Pending {
    /// Right-click on empty desktop: Windows' desktop menu plus ours.
    DesktopMenu {
        x: i32,
        y: i32,
    },
    /// File/folder popup is drawn and populated by Windows Shell, not by DesktopCraft.
    NativeFileMenu {
        path: String,
    },
    Drag {
        path: String,
    },
}

/// Owns a separate COM apartment; no DesktopView or PIDL crosses threads.
/// Only owned UTF-8 paths and screen coordinates are sent to the worker.
pub struct IconMoveWorker {
    requests: std::sync::mpsc::Sender<(String, (i32, i32))>,
    results: std::sync::mpsc::Receiver<(String, bool)>,
}
impl IconMoveWorker {
    fn start() -> Option<Self> {
        let (requests, receive) = std::sync::mpsc::channel::<(String, (i32, i32))>();
        let (send_result, results) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("desktop-icon-placement".into())
            .spawn(move || {
                shell::init();
                for (path, location) in receive {
                    let started = std::time::Instant::now();
                    let done = shell::DesktopView::acquire()
                        .is_some_and(|view| view.position(&[(path.clone(), location)]) > 0);
                    crate::perf::record("icon_position_worker_ms", started.elapsed());
                    if send_result.send((path, done)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { requests, results })
    }
    fn queue(&self, path: String, location: (i32, i32)) -> bool {
        self.requests.send((path, location)).is_ok()
    }
}

pub struct HostState {
    pub monitors: Vec<host::Monitor>,
    pub signature: String,
    pub virtual_rect: [i32; 4],
    pub scale: f32,
    pub folders: Vec<PathBuf>,
    pub known: HashSet<String>,
    pub synced_once: bool,
    pub last_sync: f64,
    /// Last successful full Explorer COM reconciliation. Ordinary file-system
    /// presence checks run more often without enumerating Explorer icons.
    pub last_full_reconcile: f64,
    /// Defer expensive Explorer COM refresh until shortly after a mouse drag finishes.
    pub last_mouse_held: f64,
    pub last_monitors: f64,
    pub last_commands: f64,
    pub last_shown: f64,
    /// Reassert exclusive icon ownership after Fences exits or Explorer restarts.
    pub last_icon_visibility_check: f64,
    /// Health breadcrumb every two minutes (no sensitive filenames or paths).
    pub last_health_log: f64,
    pub region: Vec<[i32; 5]>,
    pub front: bool,
    pub taskbar_visible: bool,
    pub arrange_checked: bool,
    pub view_warned: bool,
    pub first_run: bool,
    pub pending: Option<Pending>,
    pub quit: bool,
    pub moved: usize,
    pub tuck_report: (i32, usize, usize),
    pub loose: Vec<Loose>,
    pub spacing: (i32, i32),
    pub selected_loose: Option<String>,
    pub marquee_selected: HashSet<String>,
    pub press: Option<(String, Vec2, (i32, i32))>,
    icon_mover: Option<IconMoveWorker>,
}

impl HostState {
    pub fn new(first_run: bool) -> Self {
        Self {
            monitors: vec![],
            signature: String::new(),
            virtual_rect: [0, 0, 0, 0],
            scale: 1.0,
            folders: shell::desktop_folders(),
            known: HashSet::new(),
            synced_once: false,
            last_sync: -10.,
            last_full_reconcile: -10.,
            last_mouse_held: -10.,
            last_monitors: -10.,
            last_commands: -10.,
            last_shown: 0.,
            last_icon_visibility_check: -10.,
            last_health_log: 0.,
            region: vec![],
            front: false,
            taskbar_visible: false,
            arrange_checked: false,
            view_warned: false,
            first_run,
            pending: None,
            quit: false,
            moved: 0,
            tuck_report: (0, 0, 0),
            loose: vec![],
            spacing: (75, 98),
            selected_loose: None,
            marquee_selected: HashSet::new(),
            press: None,
            icon_mover: IconMoveWorker::start(),
        }
    }
}

/// Keep Explorer's COM desktop view fresh periodically, but don't block the
/// render thread every 2.5 seconds when neither icons nor layout changed.
const FULL_RECONCILE_INTERVAL: f64 = 20.0;

fn needs_explorer_reconcile(
    icons_changed: bool,
    explicitly_invalidated: bool,
    never_synced: bool,
    previous_failed: bool,
    since_full: f64,
) -> bool {
    icons_changed
        || explicitly_invalidated
        || never_synced
        || previous_failed
        || since_full >= FULL_RECONCILE_INTERVAL
}

const CATEGORIES: [(&str, &str); 6] = [
    ("Shortcuts", "lnk,url,appref-ms"),
    (
        "Documents",
        "pdf,doc,docx,txt,rtf,odt,xls,xlsx,csv,ppt,pptx,md",
    ),
    (
        "Pictures",
        "png,jpg,jpeg,gif,bmp,webp,heic,svg,ico,tif,tiff",
    ),
    ("Media", "mp3,wav,flac,m4a,mp4,mov,mkv,avi,wmv"),
    ("Archives & installers", "zip,rar,7z,exe,msi,iso"),
    ("Other", ""),
];

impl App {
    pub fn host_scale(&self) -> f32 {
        self.host.as_ref().map_or(1.0, |h| h.scale)
    }
    /// Workspace (logical) rectangle -> physical screen rectangle.
    pub fn to_screen(&self, b: Bounds) -> [i32; 4] {
        let Some(h) = &self.host else {
            return [b.x as i32, b.y as i32, b.w as i32, b.h as i32];
        };
        let s = h.scale;
        [
            h.virtual_rect[0] + (b.x * s).round() as i32,
            h.virtual_rect[1] + (b.y * s).round() as i32,
            (b.w * s).round() as i32,
            (b.h * s).round() as i32,
        ]
    }
    pub fn from_screen(&self, x: i32, y: i32) -> Vec2 {
        let Some(h) = &self.host else {
            return vec2(x as f32, y as f32);
        };
        vec2(
            (x - h.virtual_rect[0]) as f32 / h.scale,
            (y - h.virtual_rect[1]) as f32 / h.scale,
        )
    }
    fn monitor_logical(&self, m: &host::Monitor, work: bool) -> Bounds {
        let r = if work { m.work } else { m.rect };
        let p = self.from_screen(r[0], r[1]);
        let s = self.host_scale();
        Bounds {
            x: p.x,
            y: p.y,
            w: r[2] as f32 / s,
            h: r[3] as f32 / s,
        }
    }
    /// The actual Windows primary monitor, regardless of the optional display
    /// chosen for other DesktopCraft tools (used for the page switcher).
    pub fn windows_primary_view(&self) -> Bounds {
        self.host
            .as_ref()
            .and_then(|h| h.monitors.iter().find(|m| m.primary).or(h.monitors.first()))
            .map(|m| self.monitor_logical(m, true))
            .unwrap_or(Bounds {
                x: 0.,
                y: 0.,
                w: self.width,
                h: self.height,
            })
    }
    /// The selected control monitor's work area, or OS primary by default.
    pub fn primary_view(&self) -> Bounds {
        let Some(h) = &self.host else {
            return Bounds {
                x: 0.,
                y: 0.,
                w: self.width,
                h: self.height,
            };
        };
        h.monitors
            .iter()
            .find(|m| {
                !self.state.prefs.workspace_monitor.is_empty()
                    && m.key == self.state.prefs.workspace_monitor
            })
            .or_else(|| h.monitors.iter().find(|m| m.primary))
            .or(h.monitors.first())
            .map(|m| self.monitor_logical(m, true))
            .unwrap_or(Bounds {
                x: 0.,
                y: 0.,
                w: self.width,
                h: self.height,
            })
    }
    pub(crate) fn menu_monitor_bounds(&self, at: Vec2) -> Bounds {
        let screen = self.to_screen(Bounds {
            x: at.x,
            y: at.y,
            w: 0.,
            h: 0.,
        });
        self.monitor_at(screen[0], screen[1])
            .map(|m| self.monitor_logical(&m, false))
            .unwrap_or(Bounds {
                x: 0.,
                y: 0.,
                w: self.width,
                h: self.height,
            })
    }
    fn monitor_at(&self, x: i32, y: i32) -> Option<host::Monitor> {
        let h = self.host.as_ref()?;
        h.monitors
            .iter()
            .find(|m| {
                x >= m.rect[0]
                    && y >= m.rect[1]
                    && x < m.rect[0] + m.rect[2]
                    && y < m.rect[1] + m.rect[3]
            })
            .or_else(|| h.monitors.iter().find(|m| m.primary))
            .cloned()
    }
    fn is_desktop_item(&self, path: &str) -> bool {
        self.host
            .as_ref()
            .is_some_and(|h| shell::on_desktop(path, &h.folders))
    }

    // ---------- per-monitor layouts ----------
    fn placement(&self, g: &Group) -> Option<Placement> {
        let r = self.to_screen(g.bounds);
        let m = self.monitor_at(r[0] + r[2] / 2, r[1] + r[3] / 2)?;
        let s = self.host_scale();
        Some(Placement {
            monitor: m.key.clone(),
            x: (r[0] - m.work[0]) as f32 / s,
            y: (r[1] - m.work[1]) as f32 / s,
            w: g.bounds.w,
            h: g.bounds.h,
        })
    }
    /// Remember where every group is on which monitor, for this arrangement.
    pub fn remember_layout(&mut self) {
        let Some(sig) = self.host.as_ref().map(|h| h.signature.clone()) else {
            return;
        };
        if sig.is_empty() {
            return;
        }
        let mut layout = BTreeMap::new();
        for g in &self.state.groups {
            if let Some(p) = self.placement(g) {
                layout.insert(g.id, p);
            }
        }
        for (id, p) in &layout {
            self.state.placements.insert(*id, p.clone());
        }
        self.state.placements.retain(|id, _| {
            layout.contains_key(id) || self.state.groups.iter().any(|g| g.id == *id)
        });
        self.state.layouts.insert(sig, layout);
    }
    /// Put groups back for the current monitor arrangement: its own saved layout
    /// first, then each group's last place on a monitor that is still connected,
    /// otherwise the primary monitor.
    fn apply_layout(&mut self) {
        let Some(h) = &self.host else { return };
        let sig = h.signature.clone();
        let monitors = h.monitors.clone();
        let saved = self.state.layouts.get(&sig).cloned().unwrap_or_default();
        let s = self.host_scale();
        let primary = monitors
            .iter()
            .find(|m| {
                !self.state.prefs.workspace_monitor.is_empty()
                    && m.key == self.state.prefs.workspace_monitor
            })
            .or_else(|| monitors.iter().find(|m| m.primary))
            .or(monitors.first())
            .cloned();
        let ids: Vec<u64> = self.state.groups.iter().map(|g| g.id).collect();
        for (n, id) in ids.into_iter().enumerate() {
            let place = saved
                .get(&id)
                .cloned()
                .or_else(|| self.state.placements.get(&id).cloned());
            let target = place.as_ref().and_then(|p| {
                monitors
                    .iter()
                    .find(|m| m.key == p.monitor)
                    .map(|m| (m.clone(), p.clone()))
            });
            let bounds = match target {
                Some((m, p)) => {
                    let o = self.from_screen(m.work[0], m.work[1]);
                    Some(Bounds {
                        x: o.x + p.x,
                        y: o.y + p.y,
                        w: p.w,
                        h: p.h,
                    })
                }
                None => {
                    // Monitor gone (or never placed): keep it if it is still on
                    // screen, else move it to the primary monitor.
                    let g = self.state.group(id).unwrap();
                    let r = self.to_screen(g.bounds);
                    let on_screen = monitors.iter().any(|m| {
                        let (cx, cy) = (r[0] + r[2] / 2, r[1] + 20);
                        cx >= m.rect[0]
                            && cy >= m.rect[1]
                            && cx < m.rect[0] + m.rect[2]
                            && cy < m.rect[1] + m.rect[3]
                    });
                    match (&primary, on_screen) {
                        (_, true) | (None, _) => None,
                        (Some(m), false) => {
                            let o = self.from_screen(m.work[0], m.work[1]);
                            Some(Bounds {
                                x: o.x + 24. + n as f32 * 28.,
                                y: o.y + 24. + n as f32 * 28.,
                                ..g.bounds
                            })
                        }
                    }
                }
            };
            if let Some(b) = bounds {
                if let Some(g) = self.state.group_mut(id) {
                    g.bounds = b;
                }
            }
        }
        // Clamp every group inside its monitor's work area.
        for i in 0..self.state.groups.len() {
            let b = self.state.groups[i].bounds;
            let r = self.to_screen(b);
            if let Some(m) = self.monitor_at(r[0] + r[2] / 2, r[1] + 10) {
                let w = self.monitor_logical(&m, true);
                let g = &mut self.state.groups[i];
                g.bounds.w = g.bounds.w.min(w.w);
                g.bounds.h = g.bounds.h.min(w.h);
                g.bounds.x = g.bounds.x.clamp(w.x, w.x + w.w - g.bounds.w);
                g.bounds.y = g.bounds.y.clamp(w.y, w.y + w.h - g.bounds.h);
            }
        }
        let _ = s;
    }
    fn check_monitors(&mut self) {
        let monitors = host::monitors();
        if monitors.is_empty() {
            return;
        }
        let rect = host::virtual_rect();
        let sig = host::signature(&monitors);
        let Some(h) = &self.host else { return };
        let changed = sig != h.signature;
        let moved = rect != h.virtual_rect;
        if !changed && !moved {
            return;
        }
        if changed && !h.signature.is_empty() {
            self.remember_layout();
        }
        host::event(
            "INFO",
            "display.configuration_changed",
            &format!(
                "monitors={} primary_found={} moved={}",
                monitors.len(),
                monitors.iter().any(|m| m.primary),
                moved
            ),
        );
        let primary = monitors
            .iter()
            .find(|m| m.primary)
            .or(monitors.first())
            .unwrap();
        let h = self.host.as_mut().unwrap();
        h.scale = primary.dpi as f32 / 96.0;
        h.monitors = monitors.clone();
        h.virtual_rect = rect;
        h.signature = sig;
        if moved {
            host::place(rect);
        }
        self.apply_layout();
        if let Some(h) = self.host.as_mut() {
            h.last_sync = -10.;
        }
    }

    // ---------- the real desktop ----------
    fn regular_tab_paths(&self) -> HashSet<String> {
        self.state
            .groups
            .iter()
            .flat_map(|g| {
                g.tabs
                    .iter()
                    .filter(|t| t.folder.is_none())
                    .flat_map(|t| t.items.iter().map(|i| i.path.to_lowercase()))
            })
            .collect()
    }
    /// The first rule that sends this item to an existing regular group.
    fn rule_target(&self, item: &Item) -> Option<(u64, u64)> {
        self.state.best_rule_destination(item)
    }
    /// Refresh desktop item presence without changing group assignments.
    /// Sorting rules are intentionally applied only by explicit user actions.
    pub fn sync_desktop(&mut self, time: f64, explicitly_invalidated: bool) {
        let Some(h) = &self.host else { return };
        // Filesystem enumeration is substantially cheaper than a synchronous
        // IFolderView::Items COM call for an icon-heavy Explorer desktop.
        let scan_started = std::time::Instant::now();
        let present: HashSet<String> = shell::desktop_entries()
            .into_iter()
            .map(|p| p.to_string_lossy().to_lowercase())
            .collect();
        let changed = !h.synced_once || h.known != present;
        let reconcile = needs_explorer_reconcile(
            changed,
            explicitly_invalidated,
            !h.synced_once,
            h.view_warned,
            time - h.last_full_reconcile,
        );
        crate::perf::record("desktop_fs_scan_ms", scan_started.elapsed());
        if changed {
            let folders = &h.folders;
            for g in &mut self.state.groups {
                for tab in &mut g.tabs {
                    if tab.folder.is_none() {
                        tab.items.retain(|i| {
                            !shell::on_desktop(&i.path, folders)
                                || present.contains(&i.path.to_lowercase())
                        });
                    }
                }
            }
        }
        let h = self.host.as_mut().unwrap();
        h.known = present;
        h.synced_once = true;
        if reconcile {
            self.tuck();
            // Don't defer recovery when Explorer was temporarily unavailable.
            if let Some(h) = self.host.as_mut() {
                if !h.view_warned {
                    h.last_full_reconcile = time;
                }
            }
            crate::perf::record_value("explorer_full_reconcile", 1.0);
        } else {
            crate::perf::record_value("explorer_full_reconcile", 0.0);
        }
    }
    /// Assign only currently loose desktop files to matching regular tabs.
    /// Manually placed pins stay where they are, and no disk file moves.
    pub fn sort_loose_desktop_now(&mut self) -> usize {
        let count = self.sort_paths_by_rules(shell::desktop_entries());
        if count > 0 {
            // Update the overlay immediately while Explorer reconciliation
            // happens on its normal background cadence.
            let grouped = self.regular_tab_paths();
            if let Some(host) = self.host.as_mut() {
                host.loose.retain(|icon| !grouped.contains(&icon.key));
                host.last_sync = -10.;
            }
        }
        count
    }
    fn sort_paths_by_rules(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> usize {
        let mut grouped = self.regular_tab_paths();
        let mut count = 0;
        for path in paths {
            let key = path.to_string_lossy().to_lowercase();
            if grouped.contains(&key) {
                continue;
            }
            if let Ok(mut item) = Item::from_path(&path, 0) {
                if let Some((target, tab_id)) = self.rule_target(&item) {
                    item.id = self.state.id();
                    if let Some(tab) = self.state.group_mut(target).and_then(|g| {
                        g.tabs
                            .iter_mut()
                            .find(|t| t.id == tab_id && t.folder.is_none())
                    }) {
                        tab.items.push(item);
                        grouped.insert(key);
                        count += 1;
                    }
                }
            }
        }
        count
    }
    /// Settings' explicit Apply rules reassigns existing pins too.
    pub fn sort_desktop_now(&mut self) -> usize {
        self.sort_loose_desktop_now() + self.state.apply_rules()
    }
    /// Move each grouped item's real Explorer icon under its group, and move any
    /// loose icon that ended up under a group to a free spot on its monitor.
    /// While Explorer's icon layer is off, grouped icons don't need to be moved
    /// (DesktopCraft draws everything); they are arranged under their groups
    /// only when Explorer's icons are showing (on exit, or if hiding failed).
    pub fn tuck(&mut self) {
        let place_grouped = !self.state.explorer_icons_hidden;
        self.tuck_with(place_grouped);
    }
    pub fn tuck_with(&mut self, place_grouped: bool) {
        let tuck_started = std::time::Instant::now();
        let Some(view) = shell::DesktopView::acquire() else {
            if let Some(h) = self.host.as_mut() {
                if !h.view_warned {
                    h.view_warned = true;
                    host::event(
                        "WARN",
                        "explorer.desktop_unavailable",
                        "COM desktop view acquire failed",
                    );
                    self.notify("Windows Explorer's desktop is not available yet; icons will be arranged when it is.", 0.);
                }
            }
            return;
        };
        if let Some(h) = self.host.as_mut() {
            if h.view_warned {
                host::event(
                    "INFO",
                    "explorer.desktop_recovered",
                    "COM desktop view is available again",
                );
            }
            h.view_warned = false;
        }
        let holds_desktop_items = self.state.groups.iter().any(|g| {
            g.tabs.iter().any(|t| {
                t.folder.is_none() && t.items.iter().any(|i| self.is_desktop_item(&i.path))
            })
        });
        // Checked every pass (cheap): the person may turn the options back on.
        if holds_desktop_items && view.disable_auto_arrange() {
            host::event(
                "INFO",
                "explorer.auto_arrange_disabled",
                "Align icons to grid setting preserved",
            );
            if !self.host.as_ref().unwrap().arrange_checked {
                self.notify(
                    "Explorer Auto arrange was disabled; Align to grid was preserved.",
                    get_time(),
                );
            }
            self.host.as_mut().unwrap().arrange_checked = true;
        }
        let s = self.host_scale();
        let (sx, sy) = view
            .spacing()
            .unwrap_or(((cell_w() * s) as i32, (cell_h() * s) as i32));
        let enum_started = std::time::Instant::now();
        let view_items = view.items();
        crate::perf::record("explorer_items_ms", enum_started.elapsed());
        crate::perf::record_value("explorer_icon_count", view_items.len() as f64);
        // Single borrowed Explorer index reused by layout, labels, marquee
        // exclusions and icon lookup; avoids three independent keyed maps and
        // repeated O(icon_count²) case-folded path searches.
        let icons: HashMap<String, &shell::ViewItem> = view_items
            .iter()
            .map(|i| (i.path.to_lowercase(), i))
            .collect();
        let visible =
            |g: &Group, hidden: bool| g.page == self.state.page && (!hidden || g.keep_visible);
        let shown: Vec<[i32; 4]> = self
            .state
            .groups
            .iter()
            .filter(|g| visible(g, self.hidden))
            .map(|g| {
                self.to_screen(Bounds {
                    h: if g.rolled { HEADER } else { g.bounds.h },
                    ..g.bounds
                })
            })
            .collect();
        let mut moves: Vec<(String, (i32, i32))> = vec![];
        let tolerance = sx.max(sy);
        let far = |a: (i32, i32), b: (i32, i32)| {
            (a.0 - b.0).abs() > tolerance || (a.1 - b.1).abs() > tolerance
        };
        let mut grouped = HashSet::new();
        for g in &self.state.groups {
            let r = self.to_screen(g.bounds);
            let header_spot = (r[0] + (r[2] - sx).max(0) / 2, r[1]);
            // Where icons wait when their group or tab isn't showing: behind this
            // group's header, or behind a visible group's header on the page.
            let fallback = shown
                .first()
                .map(|v| (v[0] + (v[2] - sx).max(0) / 2, v[1]))
                .unwrap_or(header_spot);
            let on_page = visible(g, self.hidden);
            let cw = (cell_w() * s) as i32;
            let ch = (cell_h() * s) as i32;
            let x0 = r[0] + (CONTENT_SIDE * s) as i32;
            let y0 = r[1] + ((HEADER + CONTENT_TOP) * s) as i32;
            let cols = ((r[2] - (2. * CONTENT_SIDE * s) as i32) / cw.max(1)).max(1) as usize;
            let rows = (((r[3] - ((HEADER + CONTENT_TOP + FOOTER) * s) as i32) / ch.max(1)).max(1))
                as usize;
            for (ti, tab) in g.tabs.iter().enumerate() {
                if tab.folder.is_some() {
                    continue;
                }
                let active = ti == g.active.min(g.tabs.len() - 1);
                for (i, item) in tab.items.iter().enumerate() {
                    let key = item.path.to_lowercase();
                    grouped.insert(key.clone());
                    if !self.is_desktop_item(&item.path) {
                        continue;
                    }
                    let target = if !on_page {
                        fallback
                    } else if g.rolled || !active {
                        header_spot
                    } else {
                        let i = i.saturating_sub(tab.scroll).min(rows * cols - 1);
                        (
                            x0 + (i % cols) as i32 * cw + (cw - sx).max(0) / 2,
                            y0 + (i / cols) as i32 * ch,
                        )
                    };
                    if let Some(icon) = icons.get(&key) {
                        if place_grouped && far(icon.pos, target) {
                            moves.push((item.path.clone(), target));
                        }
                    }
                }
            }
        }
        // Loose icons hidden under a group are moved to the first free spot.
        let inflate = |r: &[i32; 4]| [r[0] - 6, r[1] - 6, r[2] + 12, r[3] + 12];
        let blocked: Vec<[i32; 4]> = shown.iter().map(inflate).collect();
        let hits = |r: &[i32; 4], x: i32, y: i32, w: i32, h: i32| {
            x < r[0] + r[2] && x + w > r[0] && y < r[1] + r[3] && y + h > r[1]
        };
        let mut taken: Vec<(i32, i32)> = icons
            .iter()
            .filter(|(k, _)| !grouped.contains(*k))
            .map(|(_, icon)| icon.pos)
            .collect();
        let loose_under: Vec<(String, (i32, i32))> = icons
            .iter()
            .filter(|(k, _)| !grouped.contains(*k))
            .filter(|(_, icon)| {
                blocked
                    .iter()
                    .any(|r| hits(r, icon.pos.0 + sx / 4, icon.pos.1 + sy / 4, sx / 2, sy / 2))
            })
            .map(|(k, icon)| (k.clone(), icon.pos))
            .collect();
        for (path, pos) in loose_under {
            let Some(m) = self.monitor_at(pos.0, pos.1) else {
                continue;
            };
            let mut spot = None;
            'search: for c in 0..(m.work[2] / sx.max(1)) {
                for r in 0..(m.work[3] / sy.max(1)) {
                    let (x, y) = (m.work[0] + c * sx, m.work[1] + r * sy);
                    if blocked.iter().any(|b| hits(b, x, y, sx, sy)) {
                        continue;
                    }
                    if taken
                        .iter()
                        .any(|t| (t.0 - x).abs() < sx / 2 && (t.1 - y).abs() < sy / 2)
                    {
                        continue;
                    }
                    spot = Some((x, y));
                    break 'search;
                }
            }
            if let Some(spot) = spot {
                taken.push(spot);
                // Explorer paths keep their case; find the original key.
                let original = icons.get(&path).map(|i| i.path.clone()).unwrap_or(path);
                moves.push((original, spot));
            }
        }
        // Desktop icons outside groups, drawn by DesktopCraft (moved ones at
        // their new spot).
        let moved: HashMap<String, (i32, i32)> = moves
            .iter()
            .map(|(p, at)| (p.to_lowercase(), *at))
            .collect();
        let loose: Vec<Loose> = view_items
            .iter()
            .filter_map(|i| {
                let key = i.path.to_lowercase();
                if grouped.contains(&key) {
                    return None;
                }
                Some(Loose {
                    path: i.path.clone(),
                    pos: moved.get(&key).copied().unwrap_or(i.pos),
                    key,
                    label: i.label.clone(),
                })
            })
            .collect();
        if let Some(h) = self.host.as_mut() {
            h.loose = loose;
            h.spacing = (sx, sy);
        }
        // Grouped items show Explorer's label too (no ".lnk"/".url", and file
        // extensions per the person's Explorer setting).
        for g in &mut self.state.groups {
            for tab in &mut g.tabs {
                for item in &mut tab.items {
                    if let Some(icon) = icons.get(&item.path.to_lowercase()) {
                        if item.label.as_deref() != Some(icon.label.as_str()) {
                            item.label = Some(icon.label.clone());
                        }
                    }
                }
            }
        }
        if !moves.is_empty() {
            let n = view.position(&moves);
            let result = shell::DesktopView::last_position_result();
            if let Some(h) = self.host.as_mut() {
                h.moved += n;
                // Log each new outcome once (moves asked / matched / HRESULT).
                if result != h.tuck_report {
                    h.tuck_report = result;
                    host::log(&format!(
                        "tuck: {} move(s), {} matched, result 0x{:08X}",
                        result.1, result.2, result.0
                    ));
                }
            }
        }
        // Blank-desktop double-clicks (quick-hide) must not count clicks on icons.
        // Hook exclusions should describe the *visible* icon and label, not
        // every full 75x98 desktop grid cell. Otherwise a column of desktop
        // icons makes all blank space at the left unable to quick-hide.
        let art = 48.min((sx - 8).max(16));
        let text_h = (sy - art - 10).clamp(16, 42);
        let mut icon_rects: Vec<[i32; 4]> = Vec::with_capacity(icons.len() * 2 + shown.len());
        for icon in icons.values() {
            let (x, y) = icon.pos;
            icon_rects.push([x + (sx - art) / 2 - 3, y + 1, art + 6, art + 8]);
            icon_rects.push([x + 6, y + art + 8, (sx - 12).max(15), text_h]);
        }
        icon_rects.extend(shown.iter().copied());
        host::set_hook_rects(icon_rects);
        crate::perf::record("desktop_tuck_ms", tuck_started.elapsed());
    }

    /// Regular Box -> Desktop unpins without changing disk files.
    /// Linked Folder View -> Desktop physically moves the file out of its folder.
    pub fn ungroup_to_desktop(&mut self, group: u64, item: u64, cursor: (i32, i32), time: f64) {
        let Some(source) = self
            .state
            .group(group)
            .and_then(|g| g.tab().items.iter().find(|i| i.id == item))
            .map(|i| i.path.clone())
        else {
            return;
        };
        let from_folder_view = self
            .state
            .group(group)
            .is_some_and(|g| g.tab().folder.is_some());
        let mut path = source.clone();
        if from_folder_view && !self.is_desktop_item(&source) {
            // Leaving a linked Folder View is a real move, just as leaving
            // that folder in Explorer. Virtual boxes never enter this branch.
            let Some(desktop) = self.host.as_ref().and_then(|h| h.folders.first()) else {
                self.notify("Windows Desktop folder is unavailable", time);
                return;
            };
            match shell::move_into_folder(Path::new(&source), desktop) {
                Ok(destination) => path = destination.to_string_lossy().to_string(),
                Err(e) => {
                    self.notify(e, time);
                    return;
                }
            }
            self.undo.clear(); // Disk I/O cannot be undone by layout undo.
            let original_parent = Path::new(&source).parent();
            for g in &mut self.state.groups {
                for tab in &mut g.tabs {
                    if tab
                        .folder
                        .as_deref()
                        .is_some_and(|f| Some(Path::new(f)) == original_parent)
                    {
                        let _ = tab.refresh();
                    }
                }
            }
            self.notify("File moved from linked folder to Desktop", time);
        } else {
            self.remember();
        }
        self.state.unassign(&source);
        if self.is_desktop_item(&path) {
            let (sx, sy) = self.host.as_ref().map_or((75, 98), |h| h.spacing);
            if let Some(view) = shell::DesktopView::acquire() {
                view.position(&[(path, (cursor.0 - sx / 2, cursor.1 - sy / 4))]);
            }
            if !from_folder_view {
                self.notify("Icon placed on Desktop (file unchanged)", time);
            }
        } else {
            self.notify("Pin removed; original file remains in its folder", time);
        }
        if let Some(h) = self.host.as_mut() {
            h.last_sync = -10.;
        }
        self.selected = None;
    }

    /// Ordinary boxes are virtual icon groups. A Folder View is the *only*
    /// destination that performs a physical Windows Shell move.
    /// Normal destinations change the pin assignment without touching disks.
    pub fn drop_path_into_box(&mut self, path: &str, target: u64, time: f64) -> Result<(), String> {
        let tab_id = self
            .state
            .group(target)
            .ok_or("Target box not found")?
            .tab()
            .id;
        self.drop_path_into_tab(path, target, tab_id, time)
    }
    /// A drop destination is an explicit tab ID, whether active or not.
    /// Regular tabs only change virtual pins. Linked folder views retain
    /// their existing deliberate Windows Shell move semantics.
    pub fn drop_path_into_tab(
        &mut self,
        path: &str,
        target: u64,
        tab_id: u64,
        time: f64,
    ) -> Result<(), String> {
        let tab = self
            .state
            .group(target)
            .and_then(|g| g.tabs.iter().find(|t| t.id == tab_id))
            .ok_or("Target tab not found")?;
        if let Some(folder) = tab.folder.clone() {
            let folder = PathBuf::from(folder);
            let source = Path::new(path);
            if source.parent() == Some(folder.as_path()) {
                return Ok(());
            }
            let destination = shell::move_into_folder(source, &folder).map_err(|e| {
                host::event(
                    "ERROR",
                    "drag.linked_folder_move_failed",
                    "disk file not reassigned",
                );
                e
            })?;
            self.undo.clear();
            self.state.unassign(path);
            for group in &mut self.state.groups {
                for tab in &mut group.tabs {
                    if tab.folder.as_deref().is_some_and(|f| {
                        Path::new(f) == folder || source.parent() == Some(Path::new(f))
                    }) {
                        let _ = tab.refresh();
                    }
                }
            }
            if let Some(h) = self.host.as_mut() {
                h.last_sync = -10.;
            }
            self.selected = None;
            self.notify(
                format!("Moved to linked folder: {}", destination.display()),
                time,
            );
            return Ok(());
        }
        // Refuse stale paths before modifying any pin; deduplicate across
        // all normal tabs, including hidden tabs in combined boxes.
        let already = tab.items.iter().any(|i| i.path.eq_ignore_ascii_case(path));
        if already {
            return Ok(());
        }
        let id = self.state.id();
        let item = Item::from_path(Path::new(path), id).map_err(|e| e.to_string())?;
        self.remember();
        self.state.unassign(path);
        let show = self.state.prefs.switch_tab_after_drop;
        let group = self.state.group_mut(target).ok_or("Target box missing")?;
        let index = group
            .tabs
            .iter()
            .position(|t| t.id == tab_id)
            .ok_or("Target tab missing")?;
        group.tabs[index].items.push(item);
        if show {
            group.active = index;
        }
        self.selected = Some((target, if show { id } else { 0 }));
        if let Some(h) = self.host.as_mut() {
            h.last_sync = -10.;
        }
        self.notify("File assigned to tab (file unchanged)", time);
        Ok(())
    }

    /// Explorer drag/drop: regular boxes pin; linked Folder Views move files.
    /// No virtual box ever creates a Documents directory or moves a file.
    pub fn drop_files(&mut self, paths: Vec<PathBuf>, time: f64, drop_at: Option<(i32, i32)>) {
        let (cx, cy) = drop_at.unwrap_or_else(host::cursor);
        let pos = self.from_screen(cx, cy);
        let target = self
            .state
            .groups
            .iter()
            .rev()
            .find(|g| g.page == self.state.page && g.bounds.contains(pos.x, pos.y))
            .map(|g| g.id);
        host::log(&format!(
            "explorer-drop: files={} target={target:?} screen=({cx},{cy})",
            paths.len()
        ));
        let Some(target) = target else {
            self.notify(
                "Drop onto a box to add a pin or into a Folder View to move the file",
                time,
            );
            return;
        };
        let target_tab = self
            .state
            .group(target)
            .and_then(|g| {
                g.drop_tab_at(pos.x, pos.y)
                    .and_then(|index| g.tabs.get(index))
                    .or_else(|| Some(g.tab()))
                    .map(|t| t.id)
            })
            .unwrap();
        let mut accepted = 0;
        let total = paths.len();
        let linked = self.state.group(target).is_some_and(|g| {
            g.tabs
                .iter()
                .any(|t| t.id == target_tab && t.folder.is_some())
        });
        for path in paths {
            match self.drop_path_into_tab(&path.to_string_lossy(), target, target_tab, time) {
                Ok(()) => accepted += 1,
                Err(e) => self.notify(e, time),
            }
        }
        host::event(
            if accepted == total { "INFO" } else { "WARN" },
            "drag.explorer_drop_completed",
            &format!("target={target} linked={linked} accepted={accepted}"),
        );
        if accepted > 1 {
            self.notify(
                format!(
                    "{} {accepted} item(s)",
                    if linked {
                        "Moved into linked folder:"
                    } else {
                        "Pinned without moving:"
                    }
                ),
                time,
            );
        }
        if let Some(h) = self.host.as_mut() {
            h.last_sync = -10.;
        }
    }

    /// First run: offer to group the desktop by kind (the person can say no).
    pub fn auto_organize(&mut self) {
        let view = self.primary_view();
        let mut buckets: Vec<(String, String, Vec<PathBuf>)> =
            vec![("Folders".into(), String::new(), vec![])];
        buckets.extend(
            CATEGORIES
                .iter()
                .map(|(n, e)| (n.to_string(), e.to_string(), vec![])),
        );
        let grouped = self.regular_tab_paths();
        for path in shell::desktop_entries() {
            if grouped.contains(&path.to_string_lossy().to_lowercase()) {
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let i = if path.is_dir() {
                0
            } else {
                buckets
                    .iter()
                    .position(|(_, ext, _)| !ext.is_empty() && extension_matches(&name, ext))
                    .unwrap_or(buckets.len() - 1)
            };
            buckets[i].2.push(path);
        }
        self.remember();
        let width = (3. * cell_w() + 2. * CONTENT_SIDE).max(260.);
        let mut x = view.x + view.w - width - 24.;
        let mut y = view.y + 24.;
        for (name, ext, paths) in buckets {
            if paths.is_empty() {
                continue;
            }
            let rows = paths.len().div_ceil(3).clamp(1, 4) as f32;
            let height = HEADER + CONTENT_TOP + rows * cell_h() + FOOTER;
            if y + height > view.y + view.h - 12. {
                y = view.y + 24.;
                x -= width + 20.;
            }
            let id = self.state.add_group(
                &name,
                Bounds {
                    x,
                    y,
                    w: width,
                    h: height,
                },
            );
            for p in &paths {
                let _ = self.state.import(p, id);
            }
            if !ext.is_empty() && self.state.rules.len() < MAX_RULES {
                self.state.rules.push(Rule {
                    mode: 0,
                    pattern: ext,
                    target: id,
                    target_tab: None,
                    enabled: true,
                });
            }
            y += height + 20.;
        }
        self.state.auto_rules = false; // Rules only run during setup or explicit Apply/Organize.
                                       // Organize Desktop is an explicit sorting action: apply custom rules
                                       // once after the category groups have been built, never on idle ticks.
        self.state.apply_rules();
        if let Some(h) = self.host.as_mut() {
            h.last_sync = -10.;
        }
    }

    pub(crate) fn run_command(&mut self, command: &str, time: f64) {
        match command {
            "new-group" => {
                let (cx, cy) = host::cursor();
                let p = self.from_screen(cx, cy);
                self.create_group(Bounds {
                    x: p.x,
                    y: p.y,
                    w: 330.,
                    h: 260.,
                });
                if let Some(id) = self.selected.map(|s| s.0) {
                    let b = self.state.group(id).unwrap().bounds;
                    let r = self.to_screen(b);
                    if let Some(m) = self.monitor_at(cx, cy) {
                        let w = self.monitor_logical(&m, true);
                        let g = self.state.group_mut(id).unwrap();
                        g.bounds.x = g.bounds.x.min(w.x + w.w - b.w);
                        g.bounds.y = g.bounds.y.min(w.y + w.h - b.h);
                    }
                    let _ = r;
                }
            }
            c if c.starts_with("ai-tool ") => {
                crate::ai_tools::process_request(
                    self,
                    c.trim_start_matches("ai-tool ").trim(),
                    time,
                );
            }
            "settings" | "show" => {
                self.dialog = Some(Dialog::Settings {
                    page: 1,
                    pattern: String::new(),
                    mode: 0,
                    target: 0,
                });
            }
            "organize" => self.auto_organize(),
            c if c.starts_with("pin-file ") => {
                let path = c["pin-file ".len()..].trim();
                let path = Path::new(path);
                if !path.is_file() {
                    self.notify("Cannot pin: the selected file is not available.", time);
                    return;
                }
                // Context-menu pinning is *always virtual*. Never target a
                // linked Folder View, which would physically move the file.
                let group = self
                    .selected
                    .and_then(|(id, _)| self.state.group(id))
                    .filter(|g| g.tab().folder.is_none())
                    .map(|g| g.id)
                    .or_else(|| {
                        self.state
                            .groups
                            .iter()
                            .find(|g| g.page == self.state.page && g.tab().folder.is_none())
                            .map(|g| g.id)
                    })
                    .unwrap_or_else(|| self.state.add_group("Pinned files", self.fresh_bounds()));
                if let Err(error) = self.drop_path_into_box(&path.to_string_lossy(), group, time) {
                    self.notify(error, time);
                } else {
                    self.selected = Some((group, 0));
                }
            }
            c if c.starts_with("remove-group ") => {
                // Remove a group by id (its files stay on the desktop).
                if let Ok(id) = c["remove-group ".len()..].trim().parse::<u64>() {
                    self.remember();
                    self.state.groups.retain(|g| g.id != id);
                    if let Some(h) = self.host.as_mut() {
                        h.last_sync = -10.;
                    }
                }
            }
            "exit" => {
                if let Some(h) = self.host.as_mut() {
                    h.quit = true;
                }
            }
            // Sent by the setup program after it replaced an installed copy.
            "updated" => self.notify(
                format!("DesktopCraft updated to {}", env!("CARGO_PKG_VERSION")),
                time,
            ),
            _ => self.notify(format!("Unknown command: {command}"), time),
        }
    }

    /// Once per frame before drawing.
    pub fn host_tick(&mut self, time: f64) {
        // Minimized Settings leaves ordinary desktop boxes usable and keeps
        // its taskbar entry. Activating that taskbar entry restores the panel.
        if self.minimized_settings.is_some() {
            if !host::settings_has_focus() {
                self.settings_restore_armed = true;
            } else if self.settings_restore_armed {
                self.dialog = self.minimized_settings.take();
                self.settings_restore_armed = false;
            }
        }
        // Place loose desktop icons on a separate COM-initialized thread so
        // dragging one icon never stalls the render loop for Explorer enumeration.
        let finished: Vec<_> = self
            .host
            .as_ref()
            .and_then(|h| h.icon_mover.as_ref())
            .map(|worker| worker.results.try_iter().collect())
            .unwrap_or_default();
        for (_path, success) in finished {
            if let Some(h) = self.host.as_mut() {
                h.last_sync = -10.;
            }
            if !success {
                host::log(
                    "desktop icon placement failed; Explorer did not accept the new position",
                );
            }
        }
        // `DesktopView::items` can occupy the UI thread for tens of ms. Never
        // initiate routine COM enumeration in the middle of a physical drag.
        // The 350 ms quiet period also avoids a hitch immediately on release;
        // explicit invalidation is preserved for the next safe refresh.
        if host::mouse_left_down() {
            if let Some(h) = self.host.as_mut() {
                h.last_mouse_held = time;
            }
        }
        let Some(h) = &self.host else { return };
        let (monitors_due, commands_due, sync_due, shown_due) = (
            time - h.last_monitors > 2.0,
            time - h.last_commands > 0.3,
            // Explorer COM refresh is expensive on icon-heavy desktops.
            // All explicit layout/drag/drop invalidations still reset last_sync.
            time - h.last_sync > 2.5
                && time - h.last_mouse_held > 0.35
                && self.drag.is_none()
                && self.dialog.is_none(),
            time - h.last_shown > 1.0,
        );
        let first_run = h.first_run && h.synced_once;
        if time - h.last_health_log >= 120.0 {
            if let Some(h) = self.host.as_mut() {
                h.last_health_log = time;
            }
            host::event(
                "INFO",
                "session.heartbeat",
                &format!(
                    "page={} boxes={} monitors={} explorer_warning={}",
                    self.state.page + 1,
                    self.state.groups.len(),
                    self.host.as_ref().map_or(0, |h| h.monitors.len()),
                    self.host.as_ref().is_some_and(|h| h.view_warned)
                ),
            );
        }
        if monitors_due {
            self.host.as_mut().unwrap().last_monitors = time;
            self.check_monitors();
        }
        if commands_due {
            self.host.as_mut().unwrap().last_commands = time;
            for c in host::take_commands() {
                self.run_command(&c, time);
            }
        }
        if sync_due {
            let explicitly_invalidated = self.host.as_ref().is_some_and(|h| h.last_sync < 0.0);
            self.host.as_mut().unwrap().last_sync = time;
            let started = std::time::Instant::now();
            self.sync_desktop(time, explicitly_invalidated);
            crate::perf::record("desktop_sync_ms", started.elapsed());
        }
        if shown_due {
            self.host.as_mut().unwrap().last_shown = time;
            host::keep_shown();
        }
        // Fences and an Explorer restart can turn Windows' icon layer back on
        // *after* DesktopCraft hid it at startup. Do not duplicate the icons
        // we already draw inside DesktopCraft's own Boxes/loose-icon layer.
        // Probe only periodically, outside drag/dialog interactions.
        let icons_due = self
            .host
            .as_ref()
            .is_some_and(|h| time - h.last_icon_visibility_check > 3.0)
            && self.drag.is_none()
            && self.dialog.is_none();
        if icons_due {
            self.host.as_mut().unwrap().last_icon_visibility_check = time;
            if let Some(view) = shell::DesktopView::acquire() {
                match view.icons_visible() {
                    Some(true) => {
                        if view.set_icons_visible(false) && view.icons_visible() == Some(false) {
                            self.state.explorer_icons_hidden = true;
                            host::event(
                                "INFO",
                                "explorer.icons_rehidden",
                                "Explorer icons reappeared externally and were hidden again",
                            );
                        } else {
                            self.state.explorer_icons_hidden = false;
                            host::event(
                                "WARN",
                                "explorer.icons_rehide_failed",
                                "Explorer desktop refused icon visibility correction",
                            );
                        }
                    }
                    Some(false) => self.state.explorer_icons_hidden = true,
                    None => {}
                }
            }
        }
        // Preserve Windows' own desktop context menu on empty background.
        host::set_right_click_enabled(self.dialog.is_none());
        if let Some((start, end)) = host::take_marquee_release() {
            if !self.hidden && self.state.explorer_icons_hidden {
                let (x0, x1) = (start.0.min(end.0), start.0.max(end.0));
                let (y0, y1) = (start.1.min(end.1), start.1.max(end.1));
                let mut count = 0;
                if let Some(h) = self.host.as_mut() {
                    let (sx, sy) = h.spacing;
                    h.marquee_selected = h
                        .loose
                        .iter()
                        .filter(|l| {
                            l.pos.0 < x1 && l.pos.0 + sx > x0 && l.pos.1 < y1 && l.pos.1 + sy > y0
                        })
                        .map(|l| l.key.clone())
                        .collect();
                    h.selected_loose = h
                        .loose
                        .iter()
                        .find(|l| h.marquee_selected.contains(&l.key))
                        .map(|l| l.path.clone());
                    count = h.marquee_selected.len();
                }
                if count > 0 {
                    self.notify(
                        format!("Selected {count} desktop item(s). Drag one into a group."),
                        time,
                    );
                }
            }
        }
        if let Some((start, end)) = host::take_right_release() {
            let moved = (start.0 - end.0).abs().max((start.1 - end.1).abs());
            host::log(&format!(
                "right-click on desktop at {:?} (moved {moved} px)",
                end
            ));
            // A real rectangle (at least 40 px each way) creates a group; anything
            // smaller is a click (a slightly shaky hand still gets the menu).
            let drawn = (start.0 - end.0).abs() >= 40 && (start.1 - end.1).abs() >= 40;
            if drawn {
                // Drew a rectangle: create a group there (as in the original).
                let a = self.from_screen(start.0.min(end.0), start.1.min(end.1));
                let b = self.from_screen(start.0.max(end.0), start.1.max(end.1));
                let (mw, mh) = min_size(false);
                self.dialog = Some(Dialog::NewGroup {
                    bounds: Bounds {
                        x: a.x,
                        y: a.y,
                        w: (b.x - a.x).max(mw),
                        h: (b.y - a.y).max(mh),
                    },
                    value: "New group".into(),
                });
                self.select_all = true;
                self.edit_focus = 0;
            } else if let Some(h) = self.host.as_mut() {
                h.pending = Some(Pending::DesktopMenu { x: end.0, y: end.1 });
            }
        }
        if host::take_quick_hide() && self.state.quick_hide && self.dialog.is_none() {
            self.set_hidden(!self.hidden);
        }
        // A click anywhere in Windows outside DesktopCraft's own boxes and
        // menus (another app, the taskbar, empty desktop) closes an open "…"
        // menu or sort list and ends filter typing, like any Windows menu.
        if let Some((sx, sy)) = host::take_press() {
            if self.menu.is_some()
                || self.folder_sort_menu.is_some()
                || self.filter_focus.is_some()
                || self.inline_rename.is_some()
            {
                let local = self.from_screen(sx, sy);
                let s = self.host_scale();
                let (px, py) = (local.x * s, local.y * s);
                let ours = self.host.as_ref().is_some_and(|h| {
                    h.region.iter().any(|r| {
                        px >= r[0] as f32
                            && py >= r[1] as f32
                            && px < (r[0] + r[2]) as f32
                            && py < (r[1] + r[3]) as f32
                    })
                });
                let on_popup = self
                    .popup_rects
                    .iter()
                    .any(|b| b.contains(local.x, local.y));
                if !ours && !on_popup {
                    // Like Explorer, clicking away saves an in-place rename.
                    self.commit_inline_rename(time);
                    self.close_popups();
                }
            }
        }
        if first_run {
            self.host.as_mut().unwrap().first_run = false;
            self.seed_first_run_boxes(time);
        }
    }

    /// Populate only the three starter boxes on the very first run. This
    /// never moves real files; anything outside those types remains loose.
    fn seed_first_run_boxes(&mut self, time: f64) {
        let view = self.primary_view();
        let width = 330f32.min((view.w - 28.).max(180.));
        let height = 260f32.min((view.h - 32.).max(120.));
        let cols = ((view.w - 25.) / (width + 14.)).floor().max(1.) as usize;
        let targets: Vec<_> = ["Shortcuts", "Documents", "Folders"]
            .iter()
            .filter_map(|name| {
                self.state
                    .groups
                    .iter()
                    .find(|g| g.tab().name == *name)
                    .map(|g| g.id)
            })
            .collect();
        for (i, id) in targets.iter().enumerate() {
            if let Some(g) = self.state.group_mut(*id) {
                g.bounds = Bounds {
                    x: view.x + 14. + (i % cols) as f32 * (width + 14.),
                    y: view.y + 14. + (i / cols) as f32 * (height + 14.),
                    w: width,
                    h: height,
                };
            }
        }
        let assigned = self.regular_tab_paths();
        let mut count = 0usize;
        for path in shell::desktop_entries() {
            if assigned.contains(&path.to_string_lossy().to_lowercase()) {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let group = if path.is_dir() {
                targets.get(2)
            } else if extension_matches(&name, "lnk,url,appref-ms") {
                targets.first()
            } else if extension_matches(&name, "pdf,doc,docx,txt,rtf,odt,xls,xlsx,csv,ppt,pptx,md")
            {
                targets.get(1)
            } else {
                None
            };
            if let Some(&id) = group {
                if self.state.import(&path, id).is_ok() {
                    count += 1;
                }
            }
        }
        self.state.auto_rules = false;
        if let Some(h) = self.host.as_mut() {
            h.last_sync = -10.;
        }
        self.notify(
            format!("Created Shortcuts, Documents and Folders ({count} items assigned)"),
            time,
        );
    }

    /// Quick-hide hides every group and desktop icon (DesktopCraft draws them all
    /// while Explorer's icon layer is off), like the original.
    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
        if self.host.is_some() && !self.state.explorer_icons_hidden {
            // Fallback when Explorer's icons are still showing.
            if let Some(view) = shell::DesktopView::acquire() {
                view.set_icons_visible(!hidden);
            }
        }
    }

    /// Roll-up docking (manager "Automatically roll up groups placed near the
    /// edge of the screen"): a group dropped against the top or bottom of its
    /// monitor's work area rolls up.
    pub fn dock_if_at_edge(&mut self, id: u64) {
        if self.host.is_none() || !self.state.prefs.rollup || !self.state.prefs.rollup_dock {
            return;
        }
        let Some(b) = self.state.group(id).map(|g| g.bounds) else {
            return;
        };
        let r = self.to_screen(b);
        let Some(m) = self.monitor_at(r[0] + r[2] / 2, r[1] + 10) else {
            return;
        };
        let work = self.monitor_logical(&m, true);
        let near = 6.;
        // Top edge only: a group rolled up at the bottom would open off-screen
        // (the original opens it upwards; not built yet).
        if b.y <= work.y + near {
            if let Some(g) = self.state.group_mut(id) {
                g.bounds.y = work.y;
                g.rolled = true;
            }
            self.notify("Docked: the group rolled up. Point at it to open; double-click its title to unroll.", get_time());
        }
    }

    /// While the right button is dragged on empty desktop: the rectangle that
    /// becomes a new group on release.
    pub fn draw_right_drag(&self, p: &Painter) {
        let Some((start, now)) = host::right_drag() else {
            return;
        };
        if (start.0 - now.0).abs() < 40 || (start.1 - now.1).abs() < 40 {
            return;
        }
        let a = self.from_screen(start.0.min(now.0), start.1.min(now.1));
        let b = self.from_screen(start.0.max(now.0), start.1.max(now.1));
        let r = Bounds {
            x: a.x,
            y: a.y,
            w: b.x - a.x,
            h: b.y - a.y,
        };
        round(r, GROUP_CORNER_RADIUS, Color::new(0.35, 0.6, 0.95, 0.22));
        outline(r, Color::new(0.6, 0.8, 1.0, 0.9));
        p.push_region(r, 0.);
    }

    /// Selection marquee drawn while left-dragging blank desktop background.
    pub fn draw_marquee(&self, p: &Painter) {
        let Some((start, now)) = host::marquee_drag() else {
            return;
        };
        if (start.0 - now.0).abs() < 7 && (start.1 - now.1).abs() < 7 {
            return;
        }
        let a = self.from_screen(start.0.min(now.0), start.1.min(now.1));
        let b = self.from_screen(start.0.max(now.0), start.1.max(now.1));
        let r = Bounds {
            x: a.x,
            y: a.y,
            w: b.x - a.x,
            h: b.y - a.y,
        };
        round(r, 2., Color::new(0.25, 0.48, 0.9, 0.16));
        outline(r, Color::new(0.46, 0.70, 1.0, 0.80));
        p.push_region(r, 0.);
    }

    /// Draw the desktop icons that are not in a group, at Explorer's own positions,
    /// with Explorer's labels and icons.
    pub fn draw_loose(&mut self, p: &Painter, hits: &mut Vec<Hit>) {
        let loose_started = std::time::Instant::now();
        let Some(h) = &self.host else { return };
        if self.hidden || !self.state.explorer_icons_hidden {
            return;
        }
        let s = h.scale;
        let (sx, sy) = (h.spacing.0 as f32 / s, h.spacing.1 as f32 / s);
        let selected = h.selected_loose.as_deref();
        let marquee_selected = &h.marquee_selected;
        let loose = &h.loose;
        let icon = (48. / s.max(1.0)).min(sx - 16.).max(24.);
        for (i, l) in loose.iter().enumerate() {
            let at = self.from_screen(l.pos.0, l.pos.1);
            let cell = Bounds {
                x: at.x,
                y: at.y,
                w: sx,
                h: sy,
            };
            let lines = p.wrap_label_cached(&l.label, 12., sx - 6.);
            let label_h = lines.len() as f32 * 15. + 4.;
            let shape = Bounds {
                h: (icon + 10. + label_h).min(sy),
                ..cell
            };
            let is_selected = marquee_selected.contains(&l.key)
                || selected.is_some_and(|x| x.eq_ignore_ascii_case(&l.path));
            if is_selected {
                round(shape, 3., Color::new(0.35, 0.6, 0.95, 0.35));
                outline(shape, Color::new(0.6, 0.8, 1.0, 0.7));
            } else if p.hover(shape) {
                round(shape, 3., Color::new(1., 1., 1., 0.12));
            }
            // The icon renderer consumes borrowed path/type strings. Building
            // three owned strings and a temporary Item for every visible icon
            // on every frame used to dominate allocation cost on busy desktops.
            p.icon_path(
                &l.path,
                extension_of(&l.path),
                cell.x + (sx - icon) / 2.,
                cell.y + 4.,
                icon,
                None,
            );
            for (n, line) in lines.iter().enumerate() {
                let w = p.measure(line, 12.);
                let (x, y) = (cell.x + (sx - w) / 2., cell.y + icon + 8. + n as f32 * 15.);
                // Explorer-style readable label on any wallpaper.
                p.text(line, x + 1., y + 1., 12., Color::new(0., 0., 0., 0.85));
                p.text(line, x, y, 12., WHITE);
            }
            p.push_region(shape, 0.);
            hits.push(Hit {
                bounds: shape,
                target: Target::Loose(i),
            });
        }
        crate::perf::record("desktop_loose_render_ms", loose_started.elapsed());
    }

    /// Pointer on a loose desktop icon: select, open, Explorer's menu, or a
    /// Windows drag (to move it on the desktop, into a group, a folder or an app).
    pub fn loose_pointer(&mut self, index: usize, input: &Input) {
        let Some(h) = self.host.as_mut() else { return };
        let Some(l) = h.loose.get(index).cloned() else {
            return;
        };
        if input.right_pressed {
            h.selected_loose = Some(l.path.clone());
            self.selected = None;
            self.close_popups();
            self.file_menu = Some(ModernFilePopup {
                path: l.path,
                pos: input.pos,
            });
            return;
        }
        if input.pressed {
            let double = h
                .selected_loose
                .as_deref()
                .is_some_and(|x| x.eq_ignore_ascii_case(&l.path))
                && self.last_click.as_ref().is_some_and(|(t, time, pos)| {
                    *t == Target::Loose(index)
                        && input.time - time <= platform::double_click_interval()
                        && pos.distance(input.pos) < DRAG_THRESHOLD
                });
            let h = self.host.as_mut().unwrap();
            if !h.marquee_selected.contains(&l.key) {
                h.marquee_selected.clear();
            }
            h.selected_loose = Some(l.path.clone());
            self.selected = None;
            if double {
                self.last_click = None;
                if let Err(e) = platform::open(&l.path) {
                    self.notify(e, input.time);
                }
            } else {
                self.last_click = Some((Target::Loose(index), input.time, input.pos));
                self.host.as_mut().unwrap().press = Some((l.path.clone(), input.pos, l.pos));
                host::capture_pointer(true);
            }
        }
    }
    /// User released a file over DesktopCraft's own drawn Recycle Bin.
    /// Shell delete is the only intentional physical file operation here;
    /// a cancelled or failed deletion leaves both the file and its pin intact.
    pub fn try_recycle_drop(&mut self, source: &str, screen: (i32, i32), time: f64) -> bool {
        let on_bin = self.host.as_ref().is_some_and(|h| {
            let (sx, sy) = h.spacing;
            h.loose.iter().any(|l| {
                is_recycle_bin_shell_item(&l.path)
                    && !l.path.eq_ignore_ascii_case(source)
                    && screen.0 >= l.pos.0
                    && screen.0 < l.pos.0 + sx
                    && screen.1 >= l.pos.1
                    && screen.1 < l.pos.1 + sy
            })
        });
        if !on_bin {
            return false;
        }
        if !Path::new(source).exists() {
            self.notify(
                "Only existing files or folders can be sent to Recycle Bin.",
                time,
            );
            return true;
        }
        match shell::recycle(source) {
            Ok(()) => {
                // Windows owns restore/undo for disk files; layout undo cannot
                // undo a Shell recycle operation.
                self.undo.clear();
                self.state.unassign(source);
                // A linked Folder View mirrors the disk, so refresh affected
                // folders after Windows moves an item into the Recycle Bin.
                for group in &mut self.state.groups {
                    for tab in &mut group.tabs {
                        if tab.folder.as_deref().is_some_and(|folder| {
                            Path::new(source).parent() == Some(Path::new(folder))
                        }) {
                            let _ = tab.refresh();
                        }
                    }
                }
                if let Some(h) = self.host.as_mut() {
                    h.loose
                        .retain(|icon| !icon.path.eq_ignore_ascii_case(source));
                    h.selected_loose = None;
                    h.marquee_selected.clear();
                    h.last_sync = -10.;
                }
                host::event(
                    "INFO",
                    "drag.recycled",
                    "Windows Shell recycled dropped item",
                );
                self.notify(
                    "Sent to Recycle Bin (restore using Windows if needed)",
                    time,
                );
            }
            Err(error) => {
                host::event(
                    "WARN",
                    "drag.recycle_failed",
                    "Windows Shell cancelled or rejected operation",
                );
                self.notify(error, time);
            }
        }
        true
    }

    /// Handle a loose desktop icon with the same internal placement semantics
    /// as a grouped icon. Do not invoke Explorer file moves for Desktop -> Box.
    pub fn loose_drag(&mut self, input: &Input) {
        let Some((path, start, original)) = self.host.as_ref().and_then(|h| h.press.clone()) else {
            return;
        };
        // Prefer the actual screen cursor; clipped DesktopCraft windows can
        // have an outdated logical mouse position at the edge of their region.
        let (px, py) = host::cursor();
        let cursor_logical = self.from_screen(px, py);
        let moved = start.distance(cursor_logical) > DRAG_THRESHOLD;
        if input.released || !host::mouse_left_down() {
            host::capture_pointer(false);
            self.host.as_mut().unwrap().press = None;
            if !moved {
                return;
            }
            let target = self
                .state
                .groups
                .iter()
                .rev()
                .find(|g| {
                    g.page == self.state.page
                        && g.bounds.contains(cursor_logical.x, cursor_logical.y)
                })
                .map(|g| g.id);
            if let Some(target) = target {
                let target_tab = self
                    .state
                    .group(target)
                    .and_then(|g| {
                        g.drop_tab_at(cursor_logical.x, cursor_logical.y)
                            .and_then(|index| g.tabs.get(index))
                            .or_else(|| Some(g.tab()))
                            .map(|tab| tab.id)
                    })
                    .unwrap();
                let paths: Vec<String> = if let Some(h) = &self.host {
                    if h.marquee_selected.contains(&path.to_lowercase()) {
                        h.loose
                            .iter()
                            .filter(|l| h.marquee_selected.contains(&l.key))
                            .map(|l| l.path.clone())
                            .collect()
                    } else {
                        vec![path.clone()]
                    }
                } else {
                    vec![path.clone()]
                };
                let mut count = 0;
                for file in &paths {
                    match self.drop_path_into_tab(file, target, target_tab, input.time) {
                        Ok(()) => count += 1,
                        Err(e) => self.notify(e, input.time),
                    }
                }
                self.notify(
                    format!(
                        "{} {count} desktop item(s)",
                        if self.state.group(target).is_some_and(|g| g
                            .tabs
                            .iter()
                            .any(|t| t.id == target_tab && t.folder.is_some()))
                        {
                            "Moved into linked folder:"
                        } else {
                            "Assigned to box (file unchanged):"
                        }
                    ),
                    input.time,
                );
                if let Some(h) = self.host.as_mut() {
                    h.last_sync = -10.;
                    h.marquee_selected.clear();
                }
            } else {
                let (x, y) = (px, py);
                if self.try_recycle_drop(&path, (x, y), input.time) {
                    return;
                }
                if !host::over_other_app(x, y) {
                    let (sx, sy) = self.host.as_ref().map_or((75, 98), |h| h.spacing);
                    let desired = (x - sx / 2, y - sy / 4);
                    let at = if let Some(monitor) = self.monitor_at(x, y) {
                        let occupied = self.host.as_ref().map_or_else(Vec::new, |h| {
                            h.loose
                                .iter()
                                .filter(|l| !l.path.eq_ignore_ascii_case(&path))
                                .map(|l| l.pos)
                                .collect()
                        });
                        let blocked: Vec<[i32; 4]> = self
                            .state
                            .groups
                            .iter()
                            .filter(|g| g.page == self.state.page && !self.hidden)
                            .map(|g| self.to_screen(g.bounds))
                            .collect();
                        let mut samples: Vec<_> = occupied
                            .iter()
                            .copied()
                            .filter(|(ix, iy)| {
                                *ix >= monitor.rect[0]
                                    && *ix < monitor.rect[0] + monitor.rect[2]
                                    && *iy >= monitor.rect[1]
                                    && *iy < monitor.rect[1] + monitor.rect[3]
                            })
                            .collect();
                        samples.push(original); // observed pre-drag Explorer grid coordinate
                        let phase = grid_phase(monitor.work, (sx, sy), &samples);
                        snap_loose_icon_phase(
                            desired,
                            monitor.work,
                            (sx, sy),
                            &occupied,
                            &blocked,
                            phase,
                        )
                    } else {
                        desired
                    };
                    let queued = self
                        .host
                        .as_ref()
                        .and_then(|h| h.icon_mover.as_ref())
                        .is_some_and(|worker| worker.queue(path.clone(), at));
                    if let Some(h) = self.host.as_mut() {
                        if let Some(l) = h
                            .loose
                            .iter_mut()
                            .find(|l| l.path.eq_ignore_ascii_case(&path))
                        {
                            l.pos = at; // Show the dropped position immediately.
                        }
                        // Wait for the COM worker to finish before reconciling.
                        h.last_sync = if queued { input.time } else { -10. };
                    }
                    if !queued {
                        // Preserve functionality on systems where the worker
                        // could not start; the preferred path never blocks UI.
                        if let Some(view) = shell::DesktopView::acquire() {
                            view.position(&[(path, at)]);
                        }
                    }
                }
            }
        } else if input.down && moved {
            let (x, y) = host::cursor();
            if host::over_other_app(x, y) {
                host::capture_pointer(false);
                let h = self.host.as_mut().unwrap();
                h.press = None;
                h.pending = Some(Pending::Drag { path });
            } else if let Some(h) = self.host.as_mut() {
                // Optimistic visual position follows the cursor at frame rate,
                // without a synchronous 65+-icon Explorer COM call per move.
                let (sx, sy) = h.spacing;
                if let Some(icon) = h
                    .loose
                    .iter_mut()
                    .find(|l| l.path.eq_ignore_ascii_case(&path))
                {
                    icon.pos = (x - sx / 2, y - sy / 4);
                }
            }
        } else if !input.down {
            host::capture_pointer(false);
            self.host.as_mut().unwrap().press = None;
        }
    }

    /// Once per frame after drawing: clip the window to what was drawn and keep it
    /// on the desktop layer unless a dialog, menu or Peek needs it in front.
    pub fn host_after_frame(&mut self, p: &Painter) {
        if self.host.is_none() {
            return;
        }
        if host::color_pick_shielded() {
            let h = self.host.as_mut().unwrap();
            if !h.region.is_empty() {
                host::set_region(&[]);
                h.region.clear();
            }
            if h.front {
                host::set_front(false);
                h.front = false;
            }
            return;
        }
        let s = self.host_scale();
        let mut region: Vec<[i32; 5]> = p
            .regions
            .borrow()
            .iter()
            .map(|(b, r)| {
                [
                    (b.x * s).floor() as i32,
                    (b.y * s).floor() as i32,
                    (b.w * s).ceil() as i32,
                    (b.h * s).ceil() as i32,
                    (r * s * 2.).round() as i32,
                ]
            })
            .collect();
        region.sort();
        let front = self.dialog.is_some()
            || self.menu.is_some()
            || self.folder_sort_menu.is_some()
            || self.peek;
        let h = self.host.as_mut().unwrap();
        if region != h.region {
            host::set_region(&region);
            h.region = region;
        }
        // The normal desktop overlay does not need a taskbar button, but
        // Settings should be discoverable and focusable from the taskbar.
        let settings = matches!(self.dialog, Some(Dialog::Settings { .. }))
            || self.minimized_settings.is_some();
        if settings != h.taskbar_visible {
            h.taskbar_visible = settings;
            host::settings_taskbar(settings);
        }
        if front != h.front {
            h.front = front;
            host::set_front(front);
        }
    }

    /// Shell actions that run their own modal loops (Explorer's menu, a Windows
    /// drag), started after the frame so drawing state is consistent.
    pub fn run_pending(&mut self, time: f64, _p: &Painter) {
        let Some(pending) = self.host.as_mut().and_then(|h| h.pending.take()) else {
            return;
        };
        match pending {
            Pending::DesktopMenu { x, y } => {
                // Let Explorer show the genuine Windows 11 context menu.
                // Our classic installed verbs remain under Show more options;
                // tier-one extension requires a separate signed COM package.
                host::show_native_desktop_menu(x, y);
            }
            Pending::NativeFileMenu { path } => {
                // Explicit experiment only: hand a Desktop icon's selection
                // back to Explorer, then ask Explorer's own view to handle a
                // keyboard context menu. Never inject code or reposition icons.
                // The normal DesktopCraft overlay hides native Explorer icons,
                // so the bridge will safely refuse and use classic Shell menu.
                if std::env::var("DESKTOPCRAFT_EXPERIMENTAL_EXPLORER_MENU").as_deref() == Ok("1") {
                    let request = shell::DesktopView::acquire()
                        .ok_or_else(|| "Explorer desktop view unavailable".to_string())
                        .and_then(|view| view.request_explorer_context_menu(&path));
                    if request.is_ok() {
                        host::event(
                            "INFO",
                            "explorer_bridge.context_queued",
                            "experimental request queued; menu appearance not verified",
                        );
                        if let Some(h) = self.host.as_mut() {
                            h.last_sync = -10.;
                        }
                        return;
                    }
                    host::event(
                        "INFO",
                        "explorer_bridge.classic_fallback",
                        "Explorer handoff unavailable; native classic Shell menu used",
                    );
                }
                // Stable Windows Shell IContextMenu (native classic popup).
                // Modern Windows 11 rendering is owned by Explorer's view.
                let (x, y) = host::cursor();
                match shell::context_menu(&path, x, y) {
                    shell::MenuChoice::Rename => {
                        let loose = self.host.as_ref().is_some_and(|h| {
                            h.selected_loose
                                .as_deref()
                                .is_some_and(|selected| selected.eq_ignore_ascii_case(&path))
                        });
                        if loose {
                            self.start_inline_rename_loose(&path);
                        } else {
                            let selected = self.selected.filter(|(gid, iid)| {
                                self.state.group(*gid).is_some_and(|g| {
                                    g.tab().items.iter().any(|item| {
                                        item.id == *iid && item.path.eq_ignore_ascii_case(&path)
                                    })
                                })
                            });
                            if let Some((gid, iid)) = selected {
                                self.start_inline_rename(gid, iid);
                            }
                        }
                    }
                    shell::MenuChoice::Invoked => {
                        if !Path::new(&path).exists() {
                            self.state.unassign(&path);
                            self.selected = None;
                            if let Some(h) = self.host.as_mut() {
                                if h.selected_loose
                                    .as_deref()
                                    .is_some_and(|p| p.eq_ignore_ascii_case(&path))
                                {
                                    h.selected_loose = None;
                                }
                            }
                        }
                    }
                    shell::MenuChoice::Nothing => {}
                }
            }
            Pending::Drag { path } => {
                // External app drops use OLE; internal Desktop/Box placement is
                // handled before reaching here. A cancelled OLE drag must NEVER
                // unassign its icon. A successful move removes a stale pin only
                // when the original path actually disappeared.
                let effect = shell::drag(&path);
                if effect == 0 {
                    self.notify("Drag cancelled", time);
                } else if !Path::new(&path).exists() {
                    self.remember();
                    self.state.unassign(&path);
                    self.selected = None;
                }
            }
        }
        if let Some(h) = self.host.as_mut() {
            h.last_sync = -10.;
        }
    }

    /// Rename a real item through Windows (undoable in Explorer).
    pub fn rename_item(&mut self, path: &str, name: &str, time: f64) -> bool {
        match shell::rename(path, name) {
            Ok(new) if new.exists() => {
                let new = new.to_string_lossy().to_string();
                for g in &mut self.state.groups {
                    for tab in &mut g.tabs {
                        for item in &mut tab.items {
                            if item.path.eq_ignore_ascii_case(path) {
                                item.path = new.clone();
                                item.name = name.to_string();
                            }
                        }
                    }
                }
                // A loose Desktop icon must remain selected after its path
                // changes, and Explorer must reconcile the renamed shell item.
                if let Some(h) = self.host.as_mut() {
                    if h.selected_loose
                        .as_deref()
                        .is_some_and(|p| p.eq_ignore_ascii_case(path))
                    {
                        h.selected_loose = Some(new);
                    }
                    h.last_sync = -10.;
                }
                true
            }
            Ok(_) => {
                self.notify("Windows did not confirm the new file name", time);
                false
            }
            Err(e) => {
                self.notify(e, time);
                false
            }
        }
    }

    /// On exit: arrange grouped icons inside their group areas (as many as fit; the
    /// rest go to the nearest free spots), then switch Explorer's icons back on.
    pub fn host_exit(&mut self) {
        self.tuck_with(true);
        if self.state.explorer_icons_hidden {
            restore_explorer_icons();
            self.state.explorer_icons_hidden = false;
        }
        self.remember_layout();
        host::log("exit: Explorer icons shown again");
    }
    /// While running, DesktopCraft draws every desktop icon itself and switches
    /// off Explorer's icon layer (Windows' own "Show desktop icons" setting).
    /// Without this, Explorer refuses to put more icons under a group than the grid
    /// cells it covers and keeps them on-screen, so grouped icons show twice.
    pub fn host_start(&mut self) {
        host::install_quick_hide_hook();
        self.check_monitors();
        match shell::DesktopView::acquire() {
            Some(view) => {
                if view.set_icons_visible(false) {
                    self.state.explorer_icons_hidden = true;
                    host::log("start: Explorer icon layer off; DesktopCraft draws the icons");
                } else {
                    self.state.explorer_icons_hidden = false;
                    host::event(
                        "WARN",
                        "explorer.hide_icons_failed",
                        "Explorer rejected icon visibility change",
                    );
                }
            }
            None => host::event(
                "ERROR",
                "explorer.desktop_unavailable_at_start",
                "no active Explorer desktop COM view",
            ),
        }
        // Make the recovery flag durable at once (crash or power loss).
        let _ = self.state.save(&self.save_path);
    }
}

/// Switch Explorer's desktop icons back on (exit, crash, Windows sign-out).
pub fn restore_explorer_icons() {
    if let Some(view) = shell::DesktopView::acquire() {
        view.set_icons_visible(true);
    }
}

/// Thai vowel/tone marks must remain attached to their preceding base glyph.
pub fn is_combining_mark(c: char) -> bool {
    matches!(c as u32,
        0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E
        | 0x0300..=0x036F | 0x1AB0..=0x1AFF)
}

/// Up to two lines, wrap long Thai names without requiring spaces.
pub fn wrap_label(p: &Painter, label: &str, size: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::<String>::new();
    let mut current = String::new();
    // On long unspaced Thai names, wrap by codepoint clusters while retaining
    // combining marks; on English names, spaces stay attached naturally.
    for c in label.chars() {
        let trial = format!("{current}{c}");
        if !current.is_empty() && !is_combining_mark(c) && p.measure(&trial, size) > width {
            lines.push(std::mem::take(&mut current));
        }
        current.push(c);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    let mut out: Vec<String> = lines
        .iter()
        .take(2)
        .map(|line| p.fit(line.trim_start(), size, width))
        .collect();
    if lines.len() > 2 {
        if let Some(last) = out.last_mut() {
            *last = p.fit(&format!("{}…", last.trim_end_matches('…')), size, width);
        }
    }
    out
}

impl App {
    fn _desktop_marker(&self) {}
}

#[cfg(test)]
mod desktop_refresh_tests {
    use super::*;
    #[test]
    fn bottom_sort_moves_only_loose_icons_and_keeps_manual_pins_and_pages() {
        let root = std::env::temp_dir().join(format!(
            "desktopcraft-bottom-rule-sort-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut app = App::new(root.join("workspace"), true, false);
        app.state = State::empty();
        let bounds = Bounds {
            x: 30.,
            y: 40.,
            w: 270.,
            h: 220.,
        };
        let pdf_box = app.state.add_group("Documents", bounds);
        let pdf_tab = app.state.group(pdf_box).unwrap().tab().id;
        app.state.create_page("Other page").unwrap();
        let text_box = app.state.add_group("Notes", bounds);
        let text_tab = app.state.group(text_box).unwrap().tab().id;
        app.state.rules.push(Rule {
            mode: 0,
            pattern: "pdf".into(),
            target: pdf_box,
            target_tab: Some(pdf_tab),
            enabled: true,
        });
        app.state.rules.push(Rule {
            mode: 0,
            pattern: "txt".into(),
            target: text_box,
            target_tab: Some(text_tab),
            enabled: true,
        });
        let pdf = root.join("new.pdf");
        let txt = root.join("new.txt");
        let ignored = root.join("unmatched.png");
        let manually_placed = root.join("already-pinned.pdf");
        for file in [&pdf, &txt, &ignored, &manually_placed] {
            std::fs::write(file, b"untouched").unwrap();
        }
        app.state.import(&manually_placed, text_box).unwrap();
        let before_pages = app.state.pages.clone();
        let before_page = app.state.page;
        let count = app.sort_paths_by_rules(vec![
            pdf.clone(),
            txt.clone(),
            ignored.clone(),
            manually_placed.clone(),
        ]);
        assert_eq!(count, 2);
        assert_eq!(app.state.pages, before_pages);
        assert_eq!(app.state.page, before_page);
        assert_eq!(app.state.group(pdf_box).unwrap().page, 0);
        assert_eq!(app.state.group(text_box).unwrap().page, 1);
        let pdf_items = &app.state.group(pdf_box).unwrap().tab().items;
        let txt_items = &app.state.group(text_box).unwrap().tab().items;
        assert_eq!(pdf_items.len(), 1);
        assert_eq!(pdf_items[0].path, pdf.to_string_lossy());
        assert_eq!(txt_items.len(), 2);
        assert!(txt_items
            .iter()
            .any(|i| i.path == manually_placed.to_string_lossy()));
        assert!(txt_items.iter().any(|i| i.path == txt.to_string_lossy()));
        let id = app.state.next_id;
        assert_eq!(
            app.sort_paths_by_rules(vec![pdf.clone(), txt.clone(), ignored, manually_placed]),
            0
        );
        assert_eq!(app.state.next_id, id);
        assert_eq!(std::fs::read(pdf).unwrap(), b"untouched");
        assert_eq!(std::fs::read(txt).unwrap(), b"untouched");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn recycle_bin_target_requires_windows_shell_guid_not_an_ordinary_name() {
        assert!(is_recycle_bin_shell_item(
            "::{645FF040-5081-101B-9F08-00AA002F954E}"
        ));
        assert!(is_recycle_bin_shell_item(
            "shell:::{645ff040-5081-101b-9f08-00aa002f954e}"
        ));
        assert!(!is_recycle_bin_shell_item(
            "C:\\Users\\Test\\Desktop\\Recycle Bin.lnk"
        ));
        assert!(!is_recycle_bin_shell_item(
            "C:\\Temp\\645ff040-5081-101b-9f08-00aa002f954e.txt"
        ));
        assert!(!is_recycle_bin_shell_item("::{some-other-guid}"));
    }
    #[test]
    fn loose_desktop_drop_snaps_to_nearest_free_grid_cell() {
        let work = [0, 0, 360, 320];
        let spacing = (90, 80);
        assert_eq!(
            snap_loose_icon((101, 92), work, spacing, &[], &[]),
            (90, 80)
        );
        assert_eq!(
            snap_loose_icon((177, 155), work, spacing, &[(180, 160)], &[]),
            (180, 80)
        );
        // The closest slot is under a group; choose a free neighbor.
        let blocked = [[90, 80, 95, 85]];
        let found = snap_loose_icon((101, 92), work, spacing, &[], &blocked);
        assert_ne!(found, (90, 80));
        assert_eq!(found.0.rem_euclid(90), 0);
        assert_eq!(found.1.rem_euclid(80), 0);
    }
    #[test]
    fn windows_server_grid_preserves_explorers_observed_horizontal_offset() {
        let work = [0, 0, 450, 400];
        let spacing = (90, 80);
        let phase = grid_phase(work, spacing, &[(22, 15), (112, 95), (202, 175)]);
        assert_eq!(phase, (22, 15));
        assert_eq!(
            snap_loose_icon_phase((110, 98), work, spacing, &[], &[], phase),
            (112, 95)
        );
    }
    #[test]
    fn loose_grid_handles_negative_multi_monitor_coordinates() {
        let work = [-1920, -120, 1920, 1080];
        let at = snap_loose_icon((-1830, -40), work, (90, 80), &[], &[]);
        assert_eq!(at, (-1830, -40));
    }
    #[test]
    fn idle_scans_skip_explorer_com_until_full_reconcile_is_due() {
        // Filesystem presence checks still happen every scheduled scan, but
        // unchanged icons should not force Explorer COM work every time.
        assert!(!needs_explorer_reconcile(false, false, false, false, 2.5));
        assert!(!needs_explorer_reconcile(false, false, false, false, 19.9));
        assert!(needs_explorer_reconcile(false, false, false, false, 20.0));
    }
    #[test]
    fn any_file_change_layout_change_or_shell_failure_reconciles_immediately() {
        assert!(needs_explorer_reconcile(true, false, false, false, 0.0));
        assert!(needs_explorer_reconcile(false, true, false, false, 0.0));
        assert!(needs_explorer_reconcile(false, false, true, false, 0.0));
        assert!(needs_explorer_reconcile(false, false, false, true, 0.0));
    }
}
