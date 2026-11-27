#![cfg_attr(windows, windows_subsystem = "windows")]
#![recursion_limit = "256"]
mod ai_tools;
#[cfg(feature = "profile-alloc")]
mod alloc_profile;
mod controls;
mod desktop;
mod dialogs;
mod explorer_bridge;
mod explorer_native;
mod native_ole_gate;
mod native_ole_persist;
mod native_filter_protocol;
#[cfg(windows)]
mod native_drag_channel;
#[cfg(windows)]
mod native_target;
#[cfg(windows)]
mod native_live_membership;
#[cfg(windows)]
mod native_box_placement;
#[cfg(windows)]
mod native_box_live_layout;
#[cfg(windows)]
mod native_backdrop;
#[cfg(windows)]
mod native_drag_surface;
#[cfg(windows)]
mod explorer_art;
#[cfg(windows)]
mod native_explorer_guard;
#[cfg(windows)]
mod native_bundle;
#[cfg(all(windows,feature="native-lab",feature="native-unsafe-drag-lab"))]
mod native_transfer_recovery;
#[cfg(windows)]
mod native_filter_host;
#[cfg(windows)]
mod native_tab_panel;
mod explorer_owned;
mod explorer_owned_box;
mod host;
mod model;
mod native_box;
#[cfg(all(windows, any(feature = "native-lab", feature = "native-release")))]
mod native_drop;
#[cfg(windows)]
mod native_pin_drop;
#[cfg(all(windows, feature = "native-lab"))]
mod native_integrated;
#[cfg(all(windows, feature = "native-lab"))]
mod native_quick;
#[cfg(all(windows, feature = "native-lab"))]
#[path = "bin/native_box_sandbox.rs"]
mod native_shell;
mod paint;
mod perf;
mod platform;
mod shell;
#[cfg(windows)]
mod settings_window;
mod verification;

mod app_startup_guards;
use app_startup_guards::*;
mod app_startup;
use app_startup::*;
mod app_configuration;
use app_configuration::*;
mod app_init;
mod app_settings;
mod app_actions;
mod app_chrome;
mod app_rollup;
mod app_frame;
mod app_portals;
mod app_native_workspace;
use app_native_workspace::*;
mod app_native_visual;
mod app_ghost_geometry;
use app_ghost_geometry::*;
mod app_ghost_rendering;
use app_ghost_rendering::*;
mod app_layout_view;
use app_layout_view::*;
mod app_native_labs;
use app_native_labs::*;
mod app_runtime;
#[cfg(test)]
mod page_control_tests;
#[cfg(test)]
mod native_ghost_tests;
#[cfg(test)]
mod settings_multimonitor_tests;
#[cfg(test)]
mod drag_placement_tests;
#[cfg(test)]
mod camera_tests;
#[cfg(test)]
mod rollup_reveal_tests;
pub(crate) use app_native_workspace::rollup_header_bounds;
pub(crate) use app_native_workspace::rollup_revealed_bounds;
pub(crate) use app_ghost_geometry::native_box_min_size;
#[cfg(windows)]
mod updates;
use macroquad::prelude::*;
use model::*;
use paint::*;
use std::path::{Path, PathBuf};

pub const THIRD_PARTY_NOTICES: &str = concat!(
    include_str!("../THIRD-PARTY-NOTICES.md"),
    "\n\n## Project notice\n", include_str!("../NOTICE.md"),
    "\n\n## Artwork provenance\n", include_str!("../assets/README.md"),
);


#[derive(Clone, Default)]
pub struct Input {
    pub pos: Vec2,
    pub pressed: bool,
    pub released: bool,
    pub down: bool,
    pub right_pressed: bool,
    pub right_released: bool,
    pub right_down: bool,
    pub middle_pressed: bool,
    pub middle_down: bool,
    pub middle_released: bool,
    pub scroll: f32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub keys: Vec<KeyCode>,
    /// Keys Windows auto-repeated this frame because they're held down. Only
    /// editing keys use these (holding Backspace deletes like anywhere else);
    /// actions such as Enter or Escape still fire once per press.
    pub repeats: Vec<KeyCode>,
    pub typed: String,
    pub time: f64,
}
impl Input {
    fn collect(scale: f32) -> Self {
        let (x, y) = mouse_position();
        let mut typed = String::new();
        while let Some(c) = get_char_pressed() {
            if !c.is_control() {
                typed.push(c)
            }
        }
        Self {
            pos: vec2(x / scale, y / scale),
            pressed: is_mouse_button_pressed(MouseButton::Left),
            released: is_mouse_button_released(MouseButton::Left),
            down: is_mouse_button_down(MouseButton::Left),
            right_pressed: is_mouse_button_pressed(MouseButton::Right),
            right_down: is_mouse_button_down(MouseButton::Right),
            right_released: is_mouse_button_released(MouseButton::Right),
            middle_pressed: is_mouse_button_pressed(MouseButton::Middle),
            middle_down: is_mouse_button_down(MouseButton::Middle),
            middle_released: is_mouse_button_released(MouseButton::Middle),
            scroll: mouse_wheel().1,
            ctrl: is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl),
            shift: is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift),
            alt: is_key_down(KeyCode::LeftAlt) || is_key_down(KeyCode::RightAlt),
            keys: get_keys_pressed().into_iter().collect(),
            repeats: get_keys_repeated().into_iter().collect(),
            typed,
            time: get_time(),
        }
    }
    fn key(&self, key: KeyCode) -> bool {
        self.keys.contains(&key)
    }
    /// Pressed now, or repeating because it's held (for editing keys).
    fn key_or_repeat(&self, key: KeyCode) -> bool {
        self.keys.contains(&key) || self.repeats.contains(&key)
    }
}
#[derive(Clone)]
enum Drag {
    Group {
        id: u64,
        start: Vec2,
        before: Bounds,
        before_dock: Option<DockEdge>,
    },
    Resize {
        id: u64,
        start: Vec2,
        before: Bounds,
        /// Edges being moved: 1 left, 2 right, 4 top, 8 bottom.
        edges: u8,
    },
    Item {
        group: u64,
        item: u64,
        start: Vec2,
    },
    /// Marquee selecting pins in a normal Box; never moves disk files.
    BoxSelect {
        group: u64,
        tab_id: u64,
        start: Vec2,
    },
    /// Reorder a stable tab ID within its box (not a physical file drag).
    Tab {
        group: u64,
        tab_id: u64,
        start: Vec2,
    },
    Create {
        start: Vec2,
    },
    Scroll {
        group: u64,
        track_y: f32,
        track_h: f32,
        total: usize,
        page: usize,
        cols: usize,
        grab: f32,
    },
    NativeScroll {
        group: u64,
        track_y: f32,
        track_h: f32,
        thumb_h: f32,
        positions: usize,
        grab: f32,
        /// Preview first visible row while dragging the native scrollbar thumb.
        row: usize,
    },
}
enum Dialog {
    NewGroup {
        bounds: Bounds,
        value: String,
    },
    Rename {
        group: u64,
        value: String,
    },
    Path {
        portal: bool,
        value: String,
    },
    /// Bind an existing box to a user-selected on-disk folder.
    BindFolder {
        group: u64,
        value: String,
    },
    Remove {
        group: u64,
    },
    /// Explicit, reviewable orphan-Pin cleanup. Only virtual Workspace
    /// references are removed, NEVER the source files on disk.
    OrphanPins {
        entries: Vec<(u64,u64,u64,String)>,
    },
    Snapshot {
        value: String,
    },
    /// Page creation must always go through the explicit + confirmation.
    NewPage {
        value: String,
    },
    RenamePage {
        index: usize,
        value: String,
    },
    /// Pop-out editor for one sorting rule, opened from Settings > Organize &
    /// rules. `target` indexes App::rule_destinations(). Closing it returns
    /// to that Settings page.
    EditRule {
        index: usize,
        mode: u8,
        pattern: String,
        target: usize,
    },
    Settings {
        page: usize,
        pattern: String,
        mode: u8,
        target: usize,
    },
}
pub struct App {
    pub state: State,
    pub root: PathBuf,
    pub selected: Option<(u64, u64)>,
    /// Transient multi-pin selection: (box ID, stable tab ID, pin ID).
    pub multi_selected: Vec<(u64, u64, u64)>,
    dialog: Option<Dialog>,
    minimized_settings: Option<Dialog>,
    settings_restore_armed: bool,
    #[cfg(windows)]
    settings_window: Option<settings_window::SettingsWindow>,
    settings_return: Option<Dialog>,
    settings_bring_to_front: bool,
    rendering_settings_window: bool,
    settings_minimize_requested: bool,
    desktop_toggle_requested: Option<bool>,
    #[cfg(windows)]
    update_state: updates::UiState,
    #[cfg(windows)]
    update_action: Option<updates::Action>,
    #[cfg(windows)]
    settings_last_paint: f64,
    #[cfg(windows)]
    settings_cursor_was_over: bool,
    menu: Option<(u64, Vec2)>,
    classic_file_menu: Option<desktop::ClassicFileMenu>,
    menu_page_open: bool,
    menu_page_scroll: usize,
    menu_sort_open: bool,
    /// A press on a box's "…" button: (box, where, undo entry added). It
    /// becomes a box move if dragged, or opens the menu when released.
    menu_press: Option<(u64, Vec2, bool)>,
    /// The Folder View sort list opened from its folder bar: (box, where).
    folder_sort_menu: Option<(u64, Vec2)>,
    /// The Folder View whose filter field is taking typed text.
    filter_focus: Option<u64>,
    /// An icon being renamed in place, like Explorer: (box, pin, file path, edit).
    inline_rename: Option<(u64, u64, String, model::NameEdit)>,
    /// Where the in-place rename box was drawn last frame.
    inline_rename_rect: Option<Bounds>,
    /// Where open pop-ups were drawn last frame, so a click elsewhere in
    /// Windows (outside DesktopCraft) can close them.
    popup_rects: Vec<Bounds>,
    typed_prefix: String,
    typed_at: f64,
    type_group: Option<u64>,
    drag: Option<Drag>,
    hidden: bool,
    pub peek: bool,
    peek_hotkey_was_down: bool,
    pub desktop: bool,
    previous: [i32; 4],
    pub search: String,
    pub applied_search: String,
    search_seen: String,
    search_changed: f64,
    search_focus: bool,
    edit_focus: usize,
    select_all: bool,
    toast: String,
    toast_until: f64,
    last_click: Option<(Target, f64, Vec2)>,
    last_refresh: f64,
    settings_offset: usize,
    /// Floating Settings panel position within the primary display.
    settings_origin: Option<Vec2>,
    settings_drag_offset: Option<Vec2>,
    settings_mouse_was_down: bool,
    shortcut_recording: bool,
    shortcut_candidate: Option<String>,
    shortcut_recording_rejected: bool,
    /// Appearance target index while sampling one pixel from another window.
    color_pick_target: Option<usize>,
    wheel_remainder: f32,
    wheel_target: Option<u64>,
    /// Native Explorer pages are transient; never persisted in user Workspace.
    /// (Box id, first visible row, scroll positions, visible rows).
    native_pages: std::sync::Arc<[(u64,usize,usize,usize)]>,
    native_page_request: Option<(u64,i8)>,
    native_rules_sort_requested: Option<desktop::RuleSortScope>,
    native_layout_restore_requested: Option<desktop::PendingNativeLayoutRestore>,
    native_pin_paths_requested: Option<(u64, u64, Vec<String>, String)>,
    native_ai_pending: Option<ai_tools::PendingNativeAiTool>,
    native_tx_inflight: bool,
    native_undo_inflight: Option<State>,
    native_record_undo_on_sync: Option<State>,
    /// Target first visible row queued by dragging a native scrollbar thumb.
    native_scroll_target: Option<(u64, usize)>,
    /// Visible un-grouped Explorer desktop item screen rects `[x, y, w, h]`.
    native_visible_loose_rects: std::sync::Arc<[[i32; 4]]>,
    #[cfg(windows)]
    native_cell: (i32, i32),
    #[cfg(windows)]
    native_icon_size: i32,
    #[cfg(windows)]
    native_snap_to_grid: bool,
    #[cfg(windows)]
    native_box_ghosts: native_filter_host::NativeBoxGhosts,
    /// Box bodies drawn behind Explorer's real icons (native in-Box layout).
    #[cfg(windows)]
    native_backdrop: Option<native_backdrop::Backdrop>,
    /// Wheel delta over a native Box body not yet worth a whole row.
    native_wheel_remainder: (u64, i32),
    /// Copies of a dragged native Box's Explorer icons, drawn while it moves.
    #[cfg(windows)]
    native_ghost: Option<NativeGhost>,
    /// Explorer-identical icon/label pixels for those copies.
    #[cfg(windows)]
    ghost_art: Option<GhostArt>,
    native_env: NativeEnvFlags,
    portal_refresh: Option<PortalRefreshWorker>,
    portal_refresh_inflight: std::collections::HashSet<(u64, u64)>,
    undo: Vec<State>,
    pub automation: bool,
    pub opened: Vec<String>,
    /// Files whose Windows Properties sheet was requested (Alt+Enter); kept
    /// so automated runs can confirm the shortcut without opening a window.
    pub properties_requested: Vec<String>,
    pub stats: DrawStats,
    /// Reuse hit-testing storage across frames; capacity tracks the largest
    /// visible icon count instead of reallocating a new Vec every frame.
    hit_buffer: Vec<Hit>,
    pub width: f32,
    pub height: f32,
    save_path: PathBuf,
    save_allowed: bool,
    pub save_ok: bool,
    /// Present in desktop mode: groups on the real Windows desktop.
    pub host: Option<desktop::HostState>,
    cursor_edges: Option<u8>,
    /// Roll-up: the rolled group opened (after the delay or by a click), and
    /// which rolled group the pointer is over since when.
    pub revealed: Option<u64>,
    /// Regular Native Box currently shown expanded (synced to Explorer ACKs).
    native_visual_revealed: Option<u64>,
    hover_rolled: Option<(u64, f64)>,
    dock_reveal_block: Option<u64>,
    top_dock_preview: Option<u64>,
    rollup_interact_until: f64,
    rollup_reveal_hold_until: f64,
    /// Settings dropdowns: which is open, the list to draw, last list rect,
    /// a picked option, and a button click that only closes its list.
    dropdown_open: Option<u32>,
    dropdown_list: Option<(u32, Bounds, Vec<String>, usize)>,
    dropdown_hit: Option<(u32, Bounds, usize, usize)>,
    dropdown_pick: Option<(u32, usize)>,
    dropdown_suppress: Option<u32>,
    dropdown_scroll: usize,
}


fn target_group(target: &Target) -> u64 {
    match *target {
        Target::Header(id)
        | Target::Tab(id, _)
        | Target::TabNext(id)
        | Target::NativePagePrev(id)
        | Target::NativePageNext(id)
        | Target::Roll(id)
        | Target::Menu(id)
        | Target::Resize(id, _)
        | Target::Item(id, _)
        | Target::Back(id)
        | Target::Open(id)
        | Target::FolderFilter(id)
        | Target::FilterField(id)
        | Target::FolderSort(id)
        | Target::Body(id) => id,
        Target::Scroll { group, .. } | Target::NativeScroll { group, .. } => group,
        Target::Loose(_) => 0,
    }
}



/// A native Box drag or resize preview shown smoothly without moving
/// Explorer icons every frame (docs/NATIVE_DRAG_ICONS_OPTIONS_20261119.md,
/// Option 1): Explorer hides the Box's icons and DesktopCraft draws copies
/// that follow/reflow with the Box until the drop's single reflow places the
/// real icons.
#[cfg(windows)]
struct NativeGhost {
    group: u64,
    /// Box bounds when the drag started (workspace units).
    origin: Bounds,
    origin_body: Bounds,
    /// True when started from a Resize drag rather than a Move drag.
    resizing: bool,
    first_row: usize,
    /// Monitor DPI (e.g. 96, 120) where the Box sat when the drag started.
    origin_dpi: u32,
    icons: Vec<native_filter_host::NativeGhostIcon>,
    /// Main-monitor Explorer icon cell reported by the worker.
    main_cell: (i32, i32),
    /// Explorer logical icon bitmap size (px) reported by the worker.
    base_icon_size: i32,
    /// Whether Explorer's global Align-to-Grid is active (false in two-grid mode).
    snap_to_grid: bool,
    reply: Option<std::sync::mpsc::Receiver<native_filter_host::NativeGhostIcons>>,
    cached_positions: bool,
    dropped: bool,
    /// Cancelled or refused drops wait for their own Explorer repaint.
    restoring: Option<native_filter_host::PendingNativeWorkspaceSync>,
    snapshot: Option<NativeGhostSnapshot>,
    snapshot_attempted: bool,
    trace_started: std::time::Instant,
    trace_last_move: std::time::Instant,
    trace_bounds: Bounds,
    trace_reported_pause: bool,
}

#[cfg(windows)]
struct NativeGhostSnapshot {
    target: RenderTarget,
    at: Vec2,
    size: Vec2,
}

#[cfg(windows)]
fn build_instant_ghost_icons(
    group: &Group,
    box_rect: native_box_placement::Rect,
    cell: (i32, i32),
    first_row: usize,
    snap_to_grid: bool,
) -> Vec<native_filter_host::NativeGhostIcon> {
    if cell.0 <= 0 || cell.1 <= 0 || box_rect.w <= 14 || box_rect.h <= 14 {
        return Vec::new();
    }
    let anchor = if snap_to_grid {
        (box_rect.x, box_rect.y)
    } else {
        let (pad_x, pad_y) = native_box_placement::box_grid_padding(box_rect, cell);
        (box_rect.x.saturating_add(pad_x), box_rect.y.saturating_add(pad_y))
    };
    let cols = ((i64::from(box_rect.x) + i64::from(box_rect.w) - 14 - i64::from(anchor.0))
        / i64::from(cell.0))
        .max(0) as usize;
    let rows = ((i64::from(box_rect.y) + i64::from(box_rect.h) - 14 - i64::from(anchor.1))
        / i64::from(cell.1))
        .max(0) as usize;
    let cap = cols.saturating_mul(rows);
    if cap == 0 {
        return Vec::new();
    }
    let items = &group.tab().items;
    let start = first_row.saturating_mul(cols).min(items.len());
    let stop = start.saturating_add(cap).min(items.len());
    items[start..stop]
        .iter()
        .enumerate()
        .map(|(i, item)| native_filter_host::NativeGhostIcon {
            path: item.path.clone(),
            label: item.label.as_deref().unwrap_or(&item.name).to_owned(),
            pos: (
                anchor.0 + (i % cols) as i32 * cell.0,
                anchor.1 + (i / cols) as i32 * cell.1,
            ),
        })
        .collect()
}

/// Cached native launch flags read once at initialization and refreshed on
/// explicit test/mode transitions instead of hitting `GetEnvironmentVariableW` per frame.
#[derive(Default, Clone)]
struct NativeEnvFlags {
    chrome_only: bool,
    general_workspace: bool,
    two_box_ui: bool,
    in_box_layout: bool,
    backdrop_disabled: bool,
    target_group: Option<u64>,
    tab_pair: Option<(usize, usize)>,
    ole_dest: Option<u64>,
}

impl NativeEnvFlags {
    fn read() -> Self {
        let is_one = |k: &str| std::env::var(k).ok().as_deref() == Some("1");
        let from_tab = std::env::var("DESKTOPCRAFT_NATIVE_CHROME_FROM_TAB")
            .ok()
            .and_then(|v| v.parse::<usize>().ok());
        let to_tab = std::env::var("DESKTOPCRAFT_NATIVE_CHROME_TO_TAB")
            .ok()
            .and_then(|v| v.parse::<usize>().ok());
        Self {
            chrome_only: std::env::var_os("DESKTOPCRAFT_NATIVE_CHROME_ONLY").is_some(),
            general_workspace: is_one("DESKTOPCRAFT_NATIVE_GENERAL_WORKSPACE"),
            two_box_ui: is_one("DESKTOPCRAFT_NATIVE_TWO_BOX_UI"),
            in_box_layout: is_one("DESKTOPCRAFT_NATIVE_INBOX_LAYOUT"),
            backdrop_disabled: std::env::var("DESKTOPCRAFT_NATIVE_BACKDROP").ok().as_deref()
                == Some("0"),
            target_group: std::env::var("DESKTOPCRAFT_NATIVE_CHROME_TARGET_GROUP")
                .ok()
                .and_then(|v| v.parse::<u64>().ok()),
            tab_pair: from_tab.zip(to_tab),
            ole_dest: std::env::var("DESKTOPCRAFT_NATIVE_CHROME_OLE_DEST_ID")
                .ok()
                .and_then(|v| v.parse::<u64>().ok()),
        }
    }
}

struct PortalRefreshWorker {
    requests: std::sync::mpsc::Sender<(u64, u64, String, u8)>,
    results: std::sync::mpsc::Receiver<(u64, u64, String, u8, Result<Vec<Item>, String>)>,
}

impl PortalRefreshWorker {
    fn start() -> Option<Self> {
        let (requests, inbox) = std::sync::mpsc::channel::<(u64, u64, String, u8)>();
        let (outbox, results) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("desktopcraft-portal-refresh".into())
            .spawn(move || {
                for (group_id, tab_id, folder, sort) in inbox {
                    let res = model::read_folder_items(tab_id, &folder, sort);
                    if outbox.send((group_id, tab_id, folder, sort, res)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { requests, results })
    }
}

/// Textures of desktop icons/labels rendered the way Explorer renders them,
/// so a dragged Box's copies cannot be told apart from the real icons.
#[cfg(windows)]
struct GhostArt {
    worker: explorer_art::ArtWorker,
    icons: std::collections::HashMap<String, Option<Texture2D>>,
    labels: std::collections::HashMap<(String, i32, u32), Option<Texture2D>>,
    pending_icons: std::collections::HashSet<String>,
    pending_labels: std::collections::HashSet<(String, i32, u32)>,
    snapshot_renderer: Option<native_drag_surface::Renderer>,
}

#[cfg(windows)]
fn rgba_texture((w, h, rgba): (u32, u32, Vec<u8>)) -> Option<Texture2D> {
    if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 { return None; }
    let texture = Texture2D::from_rgba8(w as u16, h as u16, &rgba);
    texture.set_filter(FilterMode::Linear);
    Some(texture)
}

#[cfg(windows)]
impl GhostArt {
    fn request_icon(&mut self, path: &str) {
        if !self.icons.contains_key(path) && self.pending_icons.insert(path.to_owned()) {
            self.worker.request(path);
        }
    }
    fn request_label(&mut self, text: &str, width: i32, dpi: u32) {
        let key = (text.to_owned(), width, dpi);
        if !self.labels.contains_key(&key) && self.pending_labels.insert(key) {
            self.worker.request_label(text, width, dpi);
        }
    }
    fn collect(&mut self) {
        for _ in 0..8 {
            let Ok(result) = self.worker.results.try_recv() else { break; };
            match result {
                explorer_art::ArtResult::Icon(path, art) => {
                    self.pending_icons.remove(&path);
                    self.icons.insert(path, art.and_then(rgba_texture));
                }
                explorer_art::ArtResult::Label { text, width, dpi, art } => {
                    let key = (text, width, dpi);
                    self.pending_labels.remove(&key);
                    self.labels.insert(key, art.and_then(rgba_texture));
                }
            }
        }
    }
    #[allow(dead_code)]
    fn label(&mut self, text: &str, width: i32, dpi: u32) -> Option<Texture2D> {
        self.request_label(text, width, dpi);
        self.labels.get(&(text.to_owned(), width, dpi)).and_then(Clone::clone)
    }
}


trait GhostCell { fn position(&self) -> (i32, i32); }
#[cfg(windows)]
impl GhostCell for native_filter_host::NativeGhostIcon { fn position(&self) -> (i32, i32) { self.pos } }
impl GhostCell for (i32, i32) { fn position(&self) -> (i32, i32) { *self } }

#[macroquad::main(configuration)]
async fn main() {
    app_runtime::run().await;
}
