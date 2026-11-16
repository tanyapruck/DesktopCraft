//! Round 18 - real DesktopCraft Box model -> in-process native Shell views.
//! Gated behind Cargo native-lab + explicit environment opt-in.
//!
//! This is not a simulated three-tab fixture. Each visible DesktopCraft Group
//! supplies its active Tab's real folder or pinned filesystem paths. The
//! existing Macroquad renderer remains in place underneath each native child:
//! any failed child creation/readiness can fall back to that renderer.
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::model::{Bounds, Group, HEADER};
use crate::native_drop::{DropZone, PinDrop, Queue};
use crate::native_shell::windows::EmbeddedLab;
use crate::{App, Input};
use macroquad::prelude::*;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

const OPT_IN: &str = "DESKTOPCRAFT_ENABLE_NATIVE_BOX_LAB";
const NATIVE_MODE: &str = "DESKTOPCRAFT_NATIVE_BOXES";
/// Classic keeps the existing Macroquad Box visuals. The Explorer visual
/// remains opt-in, since its white Shell surface cannot match the old design.
const APPEARANCE_MODE: &str = "DESKTOPCRAFT_BOX_APPEARANCE";
const MAX_NATIVE_BOXES: usize = 16;
const TOOLBAR_HEIGHT: f32 = 31.;

static DPI_AWARENESS_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
pub fn configure_dpi_early() {
    if !NativeWorkspaceController::configured() {
        return;
    }
    #[cfg(windows)]
    unsafe {
        use std::ffi::c_void;
        #[link(name = "user32")]
        extern "system" {
            fn SetProcessDpiAwarenessContext(value: *mut c_void) -> i32;
        }
        let result = SetProcessDpiAwarenessContext((-4isize) as *mut c_void) != 0;
        let _ = DPI_AWARENESS_ENABLED.set(result);
    }
}

struct Slot {
    signature: u64,
    // RevokeDragDrop must run before the native Shell COM apartment uninitializes.
    drop_zone: DropZone,
    view: EmbeddedLab,
    started: Instant,
}
pub(crate) struct NativeWorkspaceController {
    enabled: bool,
    all_virtual: bool,
    disabled: HashSet<u64>,
    slots: HashMap<u64, Slot>,
    failed: HashMap<u64, String>,
    hotkey_down: bool,
    drop_queue: Queue,
    drop_count: usize,
    drop_failures: usize,
    last_drop_group: Option<u64>,
    last_drop_delta: usize,
    session_marker: Option<PathBuf>,
    crash_disable: Option<PathBuf>,
    pending_quick: Option<Receiver<(u64, u32)>>,
}

impl NativeWorkspaceController {
    pub fn configured() -> bool {
        // Only binaries compiled with native-lab contain this module.
        cfg!(windows) && std::env::var(NATIVE_MODE).ok().as_deref() != Some("0")
    }
    pub fn new(data_root: &Path, preview: bool) -> Self {
        let enabled = Self::configured();
        let session_marker = if enabled && !preview {
            Some(data_root.join(".native-v018-running"))
        } else {
            None
        };
        let crash_disable = if enabled && !preview {
            Some(data_root.join(".native-v018-disabled"))
        } else {
            None
        };
        let stale_session = session_marker.as_ref().is_some_and(|m| m.exists());
        if stale_session {
            if let Some(marker) = &session_marker {
                let _ = std::fs::remove_file(marker);
            }
            if let Some(disabled) = &crash_disable {
                let _ = std::fs::write(
                    disabled,
                    b"Native Box disabled after an abnormal exit. Ctrl+Alt+N retries Native.\n",
                );
            }
        }
        let fallback = crash_disable.as_ref().is_some_and(|p| p.exists());
        // Preview remains Explorer-first to preserve the existing native GUI
        // regression suite. An explicit appearance setting always wins.
        // On installed DesktopCraft, the original theme is the default.
        let classic_appearance = match std::env::var(APPEARANCE_MODE).ok().as_deref() {
            Some("native") | Some("explorer") => false,
            Some("classic") | Some("original") => true,
            _ => !preview,
        };
        let initial_virtual = fallback || classic_appearance;
        if enabled && !initial_virtual {
            if let Some(marker) = &session_marker {
                let _ = std::fs::write(marker, format!("process={}\n", std::process::id()));
            }
        }
        Self {
            enabled,
            all_virtual: initial_virtual,
            disabled: HashSet::new(),
            slots: HashMap::new(),
            failed: HashMap::new(),
            hotkey_down: false,
            drop_queue: Arc::new(Mutex::new(Vec::new())),
            drop_count: 0,
            drop_failures: 0,
            last_drop_group: None,
            last_drop_delta: 0,
            session_marker,
            crash_disable,
            pending_quick: None,
        }
    }
    fn toolbar(g: &Group) -> [Bounds; 4] {
        let b = g.bounds;
        let y = b.y + HEADER + 4.;
        let button = |offset: f32, width: f32| Bounds {
            x: b.x + offset,
            y,
            w: width,
            h: TOOLBAR_HEIGHT - 7.,
        };
        [
            button(8., 66.),
            button(79., 74.),
            button(158., 85.),
            button(b.w - 40., 32.),
        ]
    }
    fn visible_group(app: &App, g: &Group) -> bool {
        g.page == app.state.page
            && (!app.hidden || g.keep_visible || app.peek)
            && !(g.rolled && app.state.prefs.rollup && app.revealed != Some(g.id))
            && !app.dialog.is_some()
            && g.bounds.w > 195.
            && g.bounds.h > HEADER + 92.
    }
    fn signature(g: &Group) -> u64 {
        let tab = g.tab();
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        tab.id.hash(&mut hash);
        tab.folder.hash(&mut hash);
        tab.list.hash(&mut hash);
        tab.sort.hash(&mut hash);
        for item in tab.items.iter() {
            item.path.hash(&mut hash);
        }
        hash.finish()
    }
    fn create(g: &Group) -> Result<EmbeddedLab, String> {
        let parent = crate::host::window();
        if parent.is_null() {
            return Err("DesktopCraft Macroquad HWND unavailable".into());
        }
        let tab = g.tab();
        let folder = tab.folder.as_ref().map(PathBuf::from);
        if let Some(path) = folder.as_ref() {
            if !path.is_dir() {
                return Err("Linked folder does not exist".into());
            }
        }
        let pins = tab
            .items
            .iter()
            .map(|i| PathBuf::from(&i.path))
            .filter(|path| path.exists())
            .collect();
        EmbeddedLab::from_workspace(parent, folder, pins)
    }
    fn fail(&mut self, id: u64, error: String) {
        self.slots.remove(&id);
        self.failed.insert(id, error);
        self.disabled.insert(id);
    }
    fn host_window_dpi() -> u32 {
        #[cfg(windows)]
        unsafe {
            #[link(name = "user32")]
            extern "system" {
                fn GetDpiForWindow(hwnd: winapi::shared::windef::HWND) -> u32;
            }
            let hwnd = crate::host::window();
            if hwnd.is_null() {
                0
            } else {
                GetDpiForWindow(hwnd)
            }
        }
        #[cfg(not(windows))]
        {
            0
        }
    }
    fn global_hotkey() -> bool {
        #[cfg(windows)]
        unsafe {
            use winapi::um::winuser::GetAsyncKeyState;
            let pressed = |k: i32| (GetAsyncKeyState(k) as u16 & 0x8000) != 0;
            pressed(0x11) && pressed(0x12) && pressed(0x4E) // Ctrl+Alt+N
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    /// Runs before App::frame, so toolbar clicks never become a group drag or
    /// native file double-click and both paths preserve the underlying model.
    pub fn consume_input(&mut self, app: &mut App, input: &mut Input) {
        if !self.enabled {
            return;
        }
        let incoming: Vec<PinDrop> = self
            .drop_queue
            .lock()
            .ok()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default();
        for dropped in incoming {
            match app.state.pin_clipboard_paths(dropped.group, &dropped.paths) {
                Ok(added) => {
                    self.drop_count += 1;
                    self.last_drop_group = Some(dropped.group);
                    self.last_drop_delta = added;
                }
                Err(_) => self.drop_failures += 1,
            }
        }
        // TrackPopupMenuEx runs on its own Windows UI thread. Never spin a
        // nested native menu message loop inside Macroquad's active GL frame.
        let menu_result = self.pending_quick.as_ref().map(|rx| rx.try_recv());
        if let Some(outcome) = menu_result {
            match outcome {
                Ok((id, command)) => {
                    self.pending_quick = None;
                    self.apply_quick_command(app, id, command);
                }
                Err(TryRecvError::Disconnected) => self.pending_quick = None,
                Err(TryRecvError::Empty) => {
                    input.pressed = false;
                    input.down = false;
                    input.released = false;
                    return;
                }
            }
            input.pressed = false;
            input.down = false;
            input.released = false;
            return;
        }
        let down = Self::global_hotkey();
        if down && !self.hotkey_down {
            self.all_virtual = !self.all_virtual;
            if self.all_virtual {
                self.slots.clear();
                self.suspend_native_marker();
            } else {
                self.reenable_native();
                self.disabled.clear();
                self.failed.clear();
            }
        }
        self.hotkey_down = down;
        // The original Box appearance must not intercept clicks with hidden
        // native toolbar hitboxes. Its existing drag, context, tabs and grid
        // remain owned entirely by the original DesktopCraft renderer.
        if self.all_virtual {
            return;
        }
        if !input.pressed {
            return;
        }
        let action = app
            .state
            .groups
            .iter()
            .filter(|g| Self::visible_group(app, g))
            .find_map(|g| {
                Self::toolbar(g)
                    .iter()
                    .enumerate()
                    .find(|(index, b)| {
                        (*index != 2 || g.bounds.w >= 285.) && b.contains(input.pos.x, input.pos.y)
                    })
                    .map(|(index, _)| (g.id, index))
            });
        let Some((id, index)) = action else {
            return;
        };
        match index {
            0 => {
                if self.all_virtual {
                    self.all_virtual = false;
                    self.reenable_native();
                    self.disabled.clear();
                    self.failed.clear();
                } else if !self.disabled.insert(id) {
                    self.disabled.remove(&id);
                    self.failed.remove(&id);
                } else {
                    self.slots.remove(&id);
                }
            }
            1 => {
                if let Some(slot) = self.slots.get_mut(&id) {
                    let _ = slot.view.properties();
                }
            }
            2 => {
                // A Pin operation is explicitly named. Cut clipboard data is
                // treated as a PIN only: never issue file Move/Delete.
                if let Ok(Some(files)) = crate::shell::read_file_clipboard() {
                    let _ = app.state.pin_clipboard_paths(id, &files.paths);
                    // Recreate that view next frame using the updated model.
                }
            }
            3 => self.quick_actions(app, id),
            _ => {}
        }
        // Macroquad paint still runs; only the consumed pointer click is muted.
        input.pressed = false;
        input.down = false;
        input.released = false;
    }
    fn quick_actions(&mut self, app: &mut App, id: u64) {
        if self.pending_quick.is_some() {
            return;
        }
        let selected = self
            .slots
            .get(&id)
            .map(|slot| slot.view.selection())
            .unwrap_or_default();
        let virtual_tab = app
            .state
            .group(id)
            .is_some_and(|g| g.tab().folder.is_none());
        let (x, y) = crate::host::cursor();
        let (sender, receiver) = std::sync::mpsc::channel::<(u64, u32)>();
        self.pending_quick = Some(receiver);
        std::thread::spawn(move || {
            let action = crate::native_quick::show_detached(x, y, selected.len(), virtual_tab);
            let _ = sender.send((id, action));
        });
    }

    fn apply_quick_command(&mut self, app: &mut App, id: u64, chosen: u32) {
        let selected = self
            .slots
            .get(&id)
            .map(|slot| slot.view.selection())
            .unwrap_or_default();
        let virtual_tab = app
            .state
            .group(id)
            .is_some_and(|g| g.tab().folder.is_none());
        match chosen {
            crate::native_quick::PROPERTIES => {
                if let Some(slot) = self.slots.get_mut(&id) {
                    let _ = slot.view.properties();
                }
            }
            crate::native_quick::PIN_CLIPBOARD if virtual_tab => {
                if let Ok(Some(files)) = crate::shell::read_file_clipboard() {
                    let _ = app.state.pin_clipboard_paths(id, &files.paths);
                }
            }
            crate::native_quick::UNPIN_SELECTED if virtual_tab => {
                let keys: HashSet<String> = selected
                    .iter()
                    .map(|p| p.to_string_lossy().replace('/', "\\").to_lowercase())
                    .collect();
                if let Some(group) = app.state.group_mut(id) {
                    group.tab_mut().items.retain(|item| {
                        !keys.contains(&item.path.replace('/', "\\").to_lowercase())
                    });
                }
            }
            crate::native_quick::COPY_PATHS if !selected.is_empty() => {
                let value = selected
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join("\r\n");
                macroquad::miniquad::window::clipboard_set(&value);
            }
            crate::native_quick::USE_VIRTUAL => {
                self.disabled.insert(id);
                self.slots.remove(&id);
            }
            _ => {}
        }
    }
    /// Render toolbar and synchronize per-Box native HWNDs. Works in both
    /// standalone sample Window mode and the real DesktopCraft desktop host.
    pub fn render(&mut self, app: &App, scale: f32) {
        if !self.enabled || self.all_virtual {
            // Do not even create native Shell HWNDs while using the original
            // look. No white Explorer panes or Drop-to-Pin buttons may render.
            self.slots.clear();
            return;
        }
        let visible: Vec<&Group> = app
            .state
            .groups
            .iter()
            .filter(|g| Self::visible_group(app, g))
            .collect();
        let ids: HashSet<u64> = visible.iter().map(|g| g.id).collect();
        self.slots
            .retain(|id, _| ids.contains(id) && !self.disabled.contains(id) && !self.all_virtual);

        // Macroquad coordinates are logical. Win32 child HWNDs use physical
        // client pixels. Desktop host has its own monitor scale; ordinary
        // windows use Macroquad UI scale * Miniquad's per-window DPI.
        let physical = if app.host.is_some() {
            app.host_scale()
        } else {
            (scale * macroquad::window::screen_dpi_scale()).max(0.5)
        };
        for g in visible {
            let [toggle, properties, pin, actions] = Self::toolbar(g);
            for (index, button) in [toggle, properties, pin, actions].iter().enumerate() {
                if index == 2 && g.bounds.w < 285. {
                    continue;
                }
                let native_active = !self.all_virtual && !self.disabled.contains(&g.id);
                let label = match index {
                    0 => {
                        if native_active {
                            "Virtual"
                        } else {
                            "Native"
                        }
                    }
                    1 => "Properties",
                    2 => "Pin Clipboard",
                    _ => "⋯",
                };
                let fg = if native_active && index == 0 {
                    BLACK
                } else {
                    WHITE
                };
                let bg = if native_active && index == 0 {
                    Color::from_rgba(189, 163, 99, 255)
                } else {
                    Color::from_rgba(52, 61, 78, 255)
                };
                draw_rectangle(button.x, button.y, button.w, button.h, bg);
                draw_text(label, button.x + 5., button.y + 17., 14., fg);
            }
            let active = !self.all_virtual && !self.disabled.contains(&g.id);
            if !active {
                continue;
            }
            let target_signature = Self::signature(g);
            let needs_new = self
                .slots
                .get(&g.id)
                .is_none_or(|slot| slot.signature != target_signature);
            if needs_new {
                self.slots.remove(&g.id);
                if self.slots.len() >= MAX_NATIVE_BOXES {
                    continue;
                }
                match Self::create(g) {
                    Ok(view) => {
                        let parent = crate::host::window();
                        match DropZone::new(parent, g.id, self.drop_queue.clone()) {
                            Ok(drop_zone) => {
                                self.slots.insert(
                                    g.id,
                                    Slot {
                                        signature: target_signature,
                                        drop_zone,
                                        view,
                                        started: Instant::now(),
                                    },
                                );
                                self.failed.remove(&g.id);
                            }
                            Err(error) => {
                                self.fail(g.id, error);
                                continue;
                            }
                        }
                    }
                    Err(error) => {
                        self.fail(g.id, error);
                        continue;
                    }
                }
            }
            if let Some(slot) = self.slots.get_mut(&g.id) {
                let b = g.bounds;
                let left = (b.x + 8.) * physical;
                let top = (b.y + HEADER + TOOLBAR_HEIGHT + 8.) * physical;
                let w = (b.w - 16.) * physical;
                let h = (b.h - HEADER - TOOLBAR_HEIGHT - 49.) * physical;
                if w < 180. || h < 80. {
                    slot.view.set_visible(false);
                    slot.drop_zone.set_visible(false);
                    continue;
                }
                slot.view.set_rect([
                    left.round() as i32,
                    top.round() as i32,
                    w.round() as i32,
                    h.round() as i32,
                ]);
                slot.view.set_visible(true);
                slot.drop_zone.set_rect([
                    ((b.x + 8.) * physical).round() as i32,
                    ((b.y + b.h - 31.) * physical).round() as i32,
                    ((b.w - 16.) * physical).round() as i32,
                    (25. * physical).round() as i32,
                ]);
                slot.drop_zone.set_visible(true);
                slot.view.tick();
                let error = slot.view.error().to_owned();
                if !error.is_empty() {
                    self.fail(g.id, error);
                    continue;
                }
                if !slot.view.ready() && slot.started.elapsed() > Duration::from_secs(8) {
                    self.fail(g.id, "Shell View did not become ready".into());
                }
            }
        }
    }

    pub fn report(&self, app: &App) -> serde_json::Value {
        let slots = self
            .slots
            .iter()
            .map(|(id, slot)| {
                serde_json::json!({
                    "group_id":id,
                    "ready":slot.view.ready(),
                    "view_items":slot.view.items(),
                    "native_parent":slot.view.parent_is_macroquad(),
                    "source_tab":app.state.group(*id).map(|g|g.tab().id),
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "enabled":self.enabled,
            "per_monitor_v2_requested":DPI_AWARENESS_ENABLED.get().copied(),
            "host_hwnd_dpi":Self::host_window_dpi(),
            "virtual_override":self.all_virtual,
            "appearance":if self.all_virtual {"classic"} else {"native"},
            "visible_groups":app.state.groups.iter()
                .filter(|g|Self::visible_group(app,g)).count(),
            "native_count":self.slots.len(),
            "drag_to_pin_accepted":self.drop_count,
            "last_drop_group":self.last_drop_group,
            "last_drop_delta":self.last_drop_delta,
            "group_pin_counts":app.state.groups.iter().map(|g|serde_json::json!({
                "group_id":g.id,"items":g.tab().items.len(),"active_tab":g.tab().id
            })).collect::<Vec<_>>(),
            "drag_to_pin_rejected":self.drop_failures,
            "native_boxes":slots,
            "disabled_count":self.disabled.len(),
            "errors":self.failed.iter().map(|(id,reason)|serde_json::json!({
                "group_id":id,"error":reason
            })).collect::<Vec<_>>(),
            "production_installer_changed":false
        })
    }

    pub fn preview_exit_hotkey() -> bool {
        #[cfg(windows)]
        unsafe {
            use winapi::um::winuser::GetAsyncKeyState;
            let held = |key: i32| (GetAsyncKeyState(key) as u16 & 0x8000) != 0;
            held(0x11) && held(0x12) && held(0x58) // Ctrl+Alt+X (preview only)
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    fn suspend_native_marker(&self) {
        if let Some(marker) = &self.session_marker {
            let _ = std::fs::remove_file(marker);
        }
    }
    fn reenable_native(&mut self) {
        if let Some(path) = &self.crash_disable {
            let _ = std::fs::remove_file(path);
        }
        if let Some(path) = &self.session_marker {
            let _ = std::fs::write(path, format!("process={}\n", std::process::id()));
        }
    }
    pub fn shutdown(&mut self) {
        self.slots.clear();
        if let Some(path) = &self.session_marker {
            let _ = std::fs::remove_file(path);
        }
    }
}
