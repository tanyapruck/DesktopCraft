use serde::{Deserialize, Serialize};
use std::{
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const HEADER: f32 = 42.0;

// ---- Tunable implementation estimates (not verified original values) ----
// Distance the pointer must travel (strictly more than this) before a press
// on a group header or a file becomes a drag. No SM_CXDRAG use was found in
// the original; adjust here.
pub const DRAG_THRESHOLD: f32 = 5.0;
// Scrollbar opacity while the pointer is not over it (original: window alpha =
// Opacity*BaseOpacity/100, values not recovered).
pub const SCROLL_REST_OPACITY: f32 = 0.6;
// Drawn width of the scrollbar, and the wider area that accepts clicks.
pub const SCROLL_BAR_WIDTH: f32 = 5.0;
pub const SCROLL_HIT_WIDTH: f32 = 14.0;
// --------------------------------------------------------------------------

pub const REFRESH: f64 = 3.0;
// Fallback icon cell when Windows' desktop spacing can't be read (estimated).
pub const DEFAULT_CELL: (f32, f32) = (94.0, 90.0);
// Padding around icons and beneath folder-view navigation.
pub const CONTENT_SIDE: f32 = 10.0;
pub const CONTENT_TOP: f32 = 12.0;
pub const FOLDER_CONTENT_TOP: f32 = 38.0;
// Bottom margin below icon rows.
pub const FOOTER: f32 = 10.0;
// Delay global search filtering while the user is still typing.
pub const SEARCH_UPDATE_DELAY: f64 = 0.350;
// Minimum scrollbar thumb ensures it remains clickable.
pub const SCROLL_MIN_THUMB: f32 = 8.0;

/// Icon cell dimensions come from Windows desktop metrics in 96-DPI
/// logical pixels, with fallbacks for systems without Explorer.
pub fn cell() -> (f32, f32) {
    static CELL: std::sync::OnceLock<(f32, f32)> = std::sync::OnceLock::new();
    *CELL.get_or_init(|| {
        crate::platform::desktop_icon_spacing()
            // Clamp extreme system metrics so labels stay readable.
            .map(|(w, h)| (w.clamp(56.0, 200.0), h.clamp(64.0, 200.0)))
            .unwrap_or(DEFAULT_CELL)
    })
}
pub fn cell_w() -> f32 {
    cell().0
}
pub fn cell_h() -> f32 {
    cell().1
}

/// Minimum content is one icon cell with a two-pixel height allowance.
pub fn min_content(cell: (f32, f32)) -> (f32, f32) {
    (cell.0, cell.1 + (2.0f32 * 1.0 + 0.5).trunc())
}
pub fn min_size(folder: bool) -> (f32, f32) {
    let (w, h) = min_content(cell());
    let top = if folder {
        FOLDER_CONTENT_TOP
    } else {
        CONTENT_TOP
    };
    (w + 2.0 * CONTENT_SIDE, HEADER + top + h + FOOTER)
}

/// Implementation decision (2026-11-12): name and extension rules compare text
/// with simple Unicode lowercase folding, the same on every computer and
/// independent of the system or CRT locale. Scripts without case (Thai,
/// Chinese...) compare exactly. No Unicode normalization is applied.
pub fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// The text after the final dot of a file name ("" when there is none or the
/// only dot is the first character, as in ".env").
pub fn extension_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[i + 1..],
        _ => "",
    }
}

/// The name without its final extension.
pub fn stem_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        Some(_) => "",
        None => name,
    }
}

/// Rule modes 1 contains, 2 starts with, 3 stem ends with, 5 does not contain.
pub fn name_rule_matches(name: &str, pattern: &str, mode: u8) -> bool {
    let pattern = fold(pattern);
    match mode {
        1 => fold(name).contains(&pattern),
        2 => fold(name).starts_with(&pattern),
        3 => fold(stem_of(name)).ends_with(&pattern),
        5 => !fold(name).contains(&pattern),
        _ => false,
    }
}

/// Extension rule: comma-separated list, optional leading dots, any case.
pub fn extension_matches(name: &str, list: &str) -> bool {
    let ext = fold(extension_of(name));
    !ext.is_empty()
        && list
            .split(',')
            .map(|p| fold(p.trim().trim_start_matches('.')))
            .any(|p| !p.is_empty() && p == ext)
}

/// Scrollbar drag: the first visible row for a pointer at `y`, where `grab` is
/// where on the thumb it was taken (inverse of `scroll_thumb`).
pub fn scroll_row_at(
    y: f32,
    track_y: f32,
    track_h: f32,
    total: usize,
    page: usize,
    grab: f32,
) -> usize {
    let total = total.max(1);
    let page = page.clamp(1, total);
    let span = total - page;
    if span == 0 {
        return 0;
    }
    let (_, len) = scroll_thumb(total, page, 0, track_h);
    let room = (track_h - len).max(1.0);
    let t = ((y - grab - track_y) / room).clamp(0.0, 1.0);
    (t * span as f32).round() as usize
}

/// Scroll by whole *rows*, never by a pageful of icons for a single wheel tick.
/// The last row is reachable while the first visible item stays row-aligned.
pub fn scroll_rows(
    current: usize,
    item_count: usize,
    visible: usize,
    cols: usize,
    notches: i32,
) -> usize {
    let cols = cols.max(1);
    let max_row = item_count
        .div_ceil(cols)
        .saturating_sub(visible.div_ceil(cols));
    ((current / cols) as i32 - notches).clamp(0, max_row as i32) as usize * cols
}

/// Resize a group by dragging one or more edges (1 left, 2 right, 4 top, 8
/// bottom). The opposite edge stays put, the size never goes below `min`, and
/// the group stays inside `area`.
pub fn resize_bounds(
    before: Bounds,
    delta: glam_vec::Vec2,
    edges: u8,
    min: (f32, f32),
    area: Bounds,
) -> Bounds {
    let (mut l, mut t, mut r, mut b) =
        (before.x, before.y, before.x + before.w, before.y + before.h);
    if edges & 1 != 0 {
        l = (l + delta.x).clamp(area.x, r - min.0);
    }
    if edges & 2 != 0 {
        r = (r + delta.x).clamp(l + min.0, (area.x + area.w).max(l + min.0));
    }
    if edges & 4 != 0 {
        t = (t + delta.y).clamp(area.y, b - min.1);
    }
    if edges & 8 != 0 {
        b = (b + delta.y).clamp(t + min.1, (area.y + area.h).max(t + min.1));
    }
    Bounds {
        x: l,
        y: t,
        w: r - l,
        h: b - t,
    }
}
mod glam_vec {
    pub use macroquad::math::Vec2;
}

/// Scrollbar thumb occupies at least eight pixels, with its position
/// normalized between the first and last visible rows.
pub fn scroll_thumb(max: usize, page: usize, pos: usize, height: f32) -> (f32, f32) {
    let max = max.max(1);
    let page = page.clamp(1, max);
    let pos = pos.min(max - page);
    let len = ((page as f32 / max as f32) * height)
        .round()
        .max(SCROLL_MIN_THUMB);
    let span = (max - page).max(1);
    let start = (pos as f32 / span as f32 * (height - len)).round();
    (start, len)
}
// Visual and layout constants for this independent desktop organizer.
pub const GROUP_CORNER_RADIUS: f32 = 4.0;
pub const MAX_RULES: usize = 50;
// Alignment and overlap tolerances for drag snapping.
pub const SNAP_ALIGNMENT: f32 = 20.0;
pub const SNAP_OVERLAP: f32 = SNAP_ALIGNMENT * 2.0;
pub const SNAP_PADDING: f32 = 0.0;

fn modified_within(modified: u64, now: u64, days: u64) -> bool {
    // Compare absolute file age against an upper bound in seconds.
    modified.abs_diff(now) < days.saturating_mul(86400)
}

// File size range endpoints are inclusive.
pub fn size_range(pattern: &str) -> Option<(u64, u64)> {
    let (lower, upper) = pattern.split_once(',')?;
    let lower = lower.trim().parse::<u64>().ok()?;
    let upper = upper.trim().parse::<u64>().ok()?;
    (lower <= upper).then_some((lower, upper))
}

#[cfg(test)]
mod original_mechanics_tests {
    use super::*;

    #[test]
    fn groups_resize_from_every_edge_and_keep_the_opposite_edge() {
        use macroquad::math::vec2;
        let before = Bounds {
            x: 100.,
            y: 100.,
            w: 300.,
            h: 200.,
        };
        let area = Bounds {
            x: 0.,
            y: 0.,
            w: 1000.,
            h: 800.,
        };
        let min = (100., 80.);
        // Left edge: moves x and width, right edge fixed.
        let r = resize_bounds(before, vec2(-40., 0.), 1, min, area);
        assert_eq!((r.x, r.w, r.x + r.w), (60., 340., 400.));
        // Bottom-left corner.
        let r = resize_bounds(before, vec2(50., 30.), 1 | 8, min, area);
        assert_eq!((r.x, r.y, r.w, r.h), (150., 100., 250., 230.));
        // Top edge can't shrink below the minimum or leave the area.
        let r = resize_bounds(before, vec2(0., 500.), 4, min, area);
        assert_eq!((r.y, r.h), (220., 80.));
        let r = resize_bounds(before, vec2(0., -500.), 4, min, area);
        assert_eq!((r.y, r.h), (0., 300.));
        // Lower-right corner works as before.
        let r = resize_bounds(before, vec2(20., 10.), 2 | 8, min, area);
        assert_eq!((r.w, r.h), (320., 210.));
    }

    #[test]
    fn page_manager_deletes_only_empty_pages_and_reindexes_groups() {
        let mut s = state();
        s.pages = vec!["A".into(), "B".into(), "C".into()];
        s.page = 2;
        let id = s.add_group(
            "Keep",
            Bounds {
                x: 10.,
                y: 10.,
                w: 120.,
                h: 120.,
            },
        );
        assert!(s.delete_empty_page(2).is_err());
        s.delete_empty_page(1).unwrap();
        assert_eq!(s.pages, ["A", "C"]);
        assert_eq!(s.page, 1);
        assert_eq!(s.group(id).unwrap().page, 1);
        assert!(s.delete_empty_page(1).is_err());
        s.delete_empty_page(0).unwrap();
        assert_eq!(s.page, 0);
        assert_eq!(s.group(id).unwrap().page, 0);
        assert!(s.delete_empty_page(0).is_err());
    }

    #[test]
    fn bind_existing_empty_box_to_folder_never_moves_files_or_discards_pins() {
        let root =
            std::env::temp_dir().join(format!("desktopcraft-bind-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let folder = root.join("work");
        std::fs::create_dir_all(&folder).unwrap();
        let original = root.join("note.txt");
        let in_folder = folder.join("inside.txt");
        std::fs::write(&original, "untouched").unwrap();
        std::fs::write(&in_folder, "folder untouched").unwrap();
        let mut s = State::empty();
        let id = s.add_group(
            "Empty box",
            Bounds {
                x: 10.,
                y: 10.,
                w: 320.,
                h: 240.,
            },
        );
        s.import(&original, id).unwrap();
        assert!(s.bind_group_folder(id, &folder).is_err());
        assert!(s.group(id).unwrap().tab().folder.is_none());
        assert_eq!(s.group(id).unwrap().tab().items.len(), 1);
        s.group_mut(id).unwrap().tab_mut().items.clear();
        s.bind_group_folder(id, &folder).unwrap();
        assert_eq!(
            s.group(id).unwrap().tab().folder.as_deref(),
            Some(folder.to_str().unwrap())
        );
        assert!(s
            .group(id)
            .unwrap()
            .tab()
            .items
            .iter()
            .any(|i| i.path == in_folder.to_string_lossy()));
        assert_eq!(std::fs::read_to_string(&original).unwrap(), "untouched");
        assert_eq!(
            std::fs::read_to_string(&in_folder).unwrap(),
            "folder untouched"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn page_rename_preserves_box_assignments_and_rejects_blank_names() {
        let mut s = State::empty();
        let first = s.add_group(
            "Test",
            Bounds {
                x: 0.,
                y: 0.,
                w: 250.,
                h: 190.,
            },
        );
        assert_eq!(s.create_page("Work").unwrap(), 1);
        let second = s.add_group(
            "Secondary",
            Bounds {
                x: 20.,
                y: 20.,
                w: 250.,
                h: 190.,
            },
        );
        assert!(s.rename_page(0, "Home").is_ok());
        assert!(s.rename_page(1, "งาน").is_ok());
        assert_eq!(s.pages, vec!["Home", "งาน"]);
        assert_eq!(s.group(first).unwrap().page, 0);
        assert_eq!(s.group(second).unwrap().page, 1);
        assert!(s.rename_page(1, " ").is_err());
        assert_eq!(s.pages[1], "งาน");
        assert!(s.rename_page(33, "Missing").is_err());
        assert!(s.create_page("").is_err());
        assert_eq!(s.pages.len(), 2);
    }
    #[test]
    fn add_tab_in_box_preserves_existing_contents_and_rule_tab_ids() {
        let mut state = State::empty();
        let b = Bounds {
            x: 10.,
            y: 20.,
            w: 250.,
            h: 240.,
        };
        let group = state.add_group("Documents", b);
        let original = state.group(group).unwrap().tab().id;
        let existing_file = Item {
            id: 1000,
            name: "report.pdf".into(),
            path: "C:/Desktop/report.pdf".into(),
            kind: "pdf".into(),
            size: 42,
            modified: 0,
            label: None,
        };
        state
            .group_mut(group)
            .unwrap()
            .tab_mut()
            .items
            .push(existing_file);
        state.rules.push(Rule {
            mode: 0,
            pattern: "pdf".into(),
            target: group,
            target_tab: Some(original),
            enabled: true,
        });
        let id = state.add_tab(group).unwrap();
        assert_ne!(id, original);
        let g = state.group(group).unwrap();
        assert_eq!(g.tabs.len(), 2);
        assert_eq!(g.active, 1);
        assert_eq!(g.tab().name, "Tab 2");
        assert!(g.tab().items.is_empty());
        assert!(g.tab().folder.is_none());
        assert_eq!(g.tabs[0].items[0].name, "report.pdf");
        assert_eq!(state.rules[0].target_tab, Some(original));
        assert_eq!(
            state.rule_destination(&state.rules[0]),
            Some((group, original))
        );
        let third = state.add_tab(group).unwrap();
        assert_ne!(third, id);
        assert_eq!(state.group(group).unwrap().tab().name, "Tab 3");
        state.group_mut(group).unwrap().tabs[1].name = "Custom".into();
        let fourth = state.add_tab(group).unwrap();
        assert_ne!(fourth, third);
        assert_eq!(state.group(group).unwrap().tab().name, "Tab 2");
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.pages.len(), 1);
        assert!(state.add_tab(999999).is_err());
    }
    #[test]
    fn clipboard_pins_are_virtual_atomic_and_keep_existing_pin_ids() {
        let root =
            std::env::temp_dir().join(format!("desktopcraft-virtual-paste-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.txt");
        let b = root.join("ภาษาไทย.txt");
        std::fs::write(&a, b"hello").unwrap();
        std::fs::write(&b, b"world").unwrap();
        let mut state = State::empty();
        let bounds = Bounds {
            x: 20.,
            y: 20.,
            w: 300.,
            h: 260.,
        };
        let one = state.add_group("One", bounds);
        let two = state.add_group("Two", bounds);
        let first = state.pin_clipboard_paths(one, &[a.clone()]).unwrap();
        assert_eq!(first, 1);
        let preserved = state.group(one).unwrap().tab().items[0].id;
        assert_eq!(
            state
                .pin_clipboard_paths(two, &[a.clone(), b.clone(), b.clone()])
                .unwrap(),
            2
        );
        assert!(state.group(one).unwrap().tab().items.is_empty());
        assert_eq!(state.group(two).unwrap().tab().items.len(), 2);
        assert_eq!(state.group(two).unwrap().tab().items[0].id, preserved);
        assert!(a.exists() && b.exists(), "Virtual paste cannot move files");
        assert_eq!(state.pin_clipboard_paths(two, &[b.clone()]).unwrap(), 0);
        // One missing path rejects the whole batch; existing pins stay put.
        assert!(state
            .pin_clipboard_paths(one, &[a.clone(), root.join("gone.txt")])
            .is_err());
        assert_eq!(state.group(two).unwrap().tab().items.len(), 2);
        assert_eq!(state.group(one).unwrap().tab().items.len(), 0);
        let tab_id = state.add_tab(two).unwrap();
        state.group_mut(two).unwrap().tab_mut().folder = Some(root.to_string_lossy().to_string());
        assert!(state.pin_clipboard_paths(two, &[a.clone()]).is_err());
        assert_eq!(state.group(two).unwrap().tab().id, tab_id);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cached_sort_matches_previous_stable_sort_for_all_modes() {
        let items: Vec<Item> = (0..600u64)
            .map(|id| Item {
                id,
                name: format!(
                    "{}-{}-{:03}.txt",
                    ["กุญแจ", "Äpple", "zeta", "Alpha"][id as usize % 4],
                    id % 19,
                    599 - id
                ),
                path: format!("C:/fixtures/file{id}.txt"),
                kind: ["txt", "folder", "pdf", "jpg"][id as usize % 4].into(),
                size: id % 37,
                modified: id % 11,
                label: None,
            })
            .collect();
        for mode in 0..4 {
            let mut expected = items.clone();
            match mode {
                1 => expected.sort_by_key(|i| (i.kind.clone(), i.name.to_lowercase())),
                2 => expected.sort_by_key(|i| std::cmp::Reverse(i.modified)),
                3 => expected.sort_by_key(|i| std::cmp::Reverse(i.size)),
                _ => expected.sort_by_key(|i| (i.kind != "folder", i.name.to_lowercase())),
            }
            let mut tab = Tab::new(1, "fixture");
            tab.sort = mode;
            tab.items = items.clone();
            tab.sort_items();
            assert_eq!(
                tab.items.iter().map(|i| i.id).collect::<Vec<_>>(),
                expected.iter().map(|i| i.id).collect::<Vec<_>>(),
                "sort mode {mode}"
            );
        }
    }

    #[test]
    #[ignore = "Run explicitly for local sort-performance numbers"]
    fn bench_cached_sort_vs_original_comparator() {
        use std::hint::black_box;
        use std::time::Instant;
        let items: Vec<Item> = (0..6000u64)
            .rev()
            .map(|id| Item {
                id,
                name: format!(
                    "{}-{:07}-{}",
                    ["งาน", "Alpha", "Äpple", "File"][id as usize % 4],
                    id,
                    id % 17
                ),
                path: format!("C:/fixtures/{id}.lnk"),
                kind: ["lnk", "exe", "url"][id as usize % 3].into(),
                size: id * 10,
                modified: id % 51,
                label: None,
            })
            .collect();
        let mut baseline_us = 0u128;
        let mut optimized_us = 0u128;
        for _ in 0..5 {
            let mut old = items.clone();
            let start = Instant::now();
            old.sort_by_key(|i| (i.kind.clone(), i.name.to_lowercase()));
            baseline_us += start.elapsed().as_micros();
            let mut tab = Tab::new(1, "benchmark");
            tab.sort = 1;
            tab.items = items.clone();
            let start = Instant::now();
            tab.sort_items();
            optimized_us += start.elapsed().as_micros();
            assert_eq!(
                black_box(old.iter().map(|i| i.id).collect::<Vec<_>>()),
                black_box(tab.items.iter().map(|i| i.id).collect::<Vec<_>>())
            );
        }
        println!(
            "6000 icon sort / 5 passes: comparator={}us cached={}us speedup={:.2}x",
            baseline_us,
            optimized_us,
            baseline_us as f64 / optimized_us.max(1) as f64
        );
    }

    #[test]
    fn a_single_wheel_notch_advances_only_one_row() {
        assert_eq!(scroll_rows(0, 40, 9, 3, -1), 3);
        assert_eq!(scroll_rows(3, 40, 9, 3, -1), 6);
        assert_eq!(scroll_rows(0, 100, 8, 1, -1), 1);
        assert_eq!(scroll_rows(0, 40, 9, 3, -100), 33);
        assert_eq!(scroll_rows(33, 40, 9, 3, -1), 33);
        assert_eq!(scroll_rows(0, 3, 9, 3, -1), 0);
    }

    #[test]
    fn minimum_content_is_one_icon_cell_plus_two_pixels() {
        assert_eq!(min_content((75.0, 86.0)), (75.0, 88.0));
    }

    #[test]
    fn scroll_thumb_follows_page_ratio_with_an_eight_pixel_minimum() {
        assert_eq!(scroll_thumb(10, 5, 0, 200.0), (0.0, 100.0));
        assert_eq!(scroll_thumb(10, 5, 5, 200.0), (100.0, 100.0));
        assert_eq!(scroll_thumb(1000, 1, 0, 100.0).1, 8.0);
        assert_eq!(scroll_thumb(1000, 1, 999, 100.0), (92.0, 8.0));
    }

    fn state() -> State {
        State {
            version: 1,
            groups: vec![],
            pages: vec!["One".into(), "Two".into()],
            page_birth: vec![0, 1],
            next_page_birth: 2,
            page: 0,
            next_id: 1,
            rules: vec![],
            snapshots: vec![],
            blend: false,
            tint: false,
            snap: true,
            quick_hide: true,
            auto_rules: false,
            wallpaper: 0,
            layouts: Default::default(),
            placements: Default::default(),
            explorer_icons_hidden: false,
            prefs: Default::default(),
        }
    }

    #[test]
    fn sorting_pages_keeps_boxes_attached_and_newest_order_stable() {
        let mut state = State::empty();
        let area = Bounds {
            x: 10.,
            y: 20.,
            w: 220.,
            h: 180.,
        };
        let home = state.add_group("Home", area);
        state.create_page("Zed").unwrap();
        let zed = state.add_group("Zed items", area);
        state.create_page("Alpha").unwrap();
        let alpha = state.add_group("Alpha items", area);
        state.sort_pages(false);
        assert_eq!(state.pages, ["Alpha", "Desktop", "Zed"]);
        assert_eq!(state.page, 0); // Alpha remains active
        assert_eq!(state.group(home).unwrap().page, 1);
        assert_eq!(state.group(zed).unwrap().page, 2);
        assert_eq!(state.group(alpha).unwrap().page, 0);
        state.sort_pages(true);
        assert_eq!(state.pages, ["Alpha", "Zed", "Desktop"]);
        assert_eq!(state.group(home).unwrap().page, 2);
        assert_eq!(state.group(zed).unwrap().page, 1);
        assert_eq!(state.group(alpha).unwrap().page, 0);
        state.sort_pages(true);
        assert_eq!(state.pages, ["Alpha", "Zed", "Desktop"]);
        state.create_page("New arrival").unwrap();
        state.sort_pages(true);
        assert_eq!(state.pages[0], "New arrival");
        state.snapshot("Sort view");
        let json = serde_json::to_string(&state).unwrap();
        let loaded: State = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.page_birth, state.page_birth);
    }

    #[test]
    fn alignment_uses_inclusive_twenty_pixel_boundary_without_a_grid() {
        let mut s = state();
        let id = s.add_group(
            "Move",
            Bounds {
                x: 80.,
                y: 180.,
                w: 100.,
                h: 100.,
            },
        );
        let peer = s.add_group(
            "Peer",
            Bounds {
                x: 100.,
                y: 500.,
                w: 100.,
                h: 100.,
            },
        );
        let b = s.group(id).unwrap().bounds;
        assert_eq!(s.snap_position(id, b).x, 100.);
        assert_eq!(s.snap_position(id, Bounds { x: 79., ..b }).x, 79.);
        s.group_mut(peer).unwrap().page = 1;
        assert_eq!(s.snap_position(id, Bounds { x: 83., ..b }).x, 83.);
    }

    #[test]
    fn matches_existing_gap_when_third_box_joins_a_row() {
        let mut s = state();
        s.add_group(
            "First",
            Bounds {
                x: 100.,
                y: 100.,
                w: 100.,
                h: 100.,
            },
        );
        s.add_group(
            "Second",
            Bounds {
                x: 220.,
                y: 100.,
                w: 100.,
                h: 100.,
            },
        );
        let third = s.add_group(
            "Third",
            Bounds {
                x: 345.,
                y: 100.,
                w: 100.,
                h: 100.,
            },
        );
        assert_eq!(
            s.snap_position(third, s.group(third).unwrap().bounds).x,
            340.
        );
    }

    #[test]
    fn centers_a_box_between_neighbors_with_equal_clearance() {
        let mut s = state();
        s.add_group(
            "Left",
            Bounds {
                x: 100.,
                y: 100.,
                w: 100.,
                h: 100.,
            },
        );
        s.add_group(
            "Right",
            Bounds {
                x: 400.,
                y: 100.,
                w: 100.,
                h: 100.,
            },
        );
        let id = s.add_group(
            "Middle",
            Bounds {
                x: 256.,
                y: 100.,
                w: 100.,
                h: 100.,
            },
        );
        let snapped = s.snap_position(id, s.group(id).unwrap().bounds);
        assert_eq!(snapped.x, 250.);
        assert_eq!(snapped.x - 200., 400. - (snapped.x + 100.));
    }

    #[test]
    fn resize_snaps_only_dragged_edge_to_neighbor_and_shift_can_bypass() {
        use macroquad::math::vec2;
        let mut s = state();
        s.add_group(
            "Above",
            Bounds {
                x: 100.,
                y: 50.,
                w: 200.,
                h: 100.,
            },
        );
        let id = s.add_group(
            "Resizing",
            Bounds {
                x: 100.,
                y: 161.,
                w: 190.,
                h: 140.,
            },
        );
        let min = (90., 80.);
        let area = Bounds {
            x: 0.,
            y: 0.,
            w: 900.,
            h: 800.,
        };
        let before = s.group(id).unwrap().bounds;
        let proposed = resize_bounds(before, vec2(0., 0.), 2, min, area);
        let snapped = s.snap_resize(id, proposed, 2, min, area);
        assert_eq!(snapped.x, before.x);
        assert_eq!(snapped.w, 200.);
        assert_eq!(snapped.y, before.y);
        let peer = s.add_group(
            "Beside",
            Bounds {
                x: 300.,
                y: 101.,
                w: 120.,
                h: 200.,
            },
        );
        let proposed = resize_bounds(before, vec2(0., -10.), 8, min, area);
        let snapped = s.snap_resize(id, proposed, 8, min, area);
        assert_eq!(snapped.y, before.y);
        assert_eq!(
            snapped.y + snapped.h,
            s.group(peer).unwrap().bounds.y + 200.
        );
    }

    #[test]
    fn separated_neighbor_edges_snap_flush_without_forcing_unrelated_axis() {
        let mut s = state();
        s.add_group(
            "Left",
            Bounds {
                x: 100.,
                y: 160.,
                w: 100.,
                h: 100.,
            },
        );
        let id = s.add_group(
            "Right",
            Bounds {
                x: 215.,
                y: 185.,
                w: 100.,
                h: 100.,
            },
        );
        let proposed = s.group(id).unwrap().bounds;
        let snapped = s.snap_position(id, proposed);
        assert_eq!(snapped.x, 200.);
        assert_eq!(snapped.y, 185.); // beyond 20 px from top AND bottom align
    }

    #[test]
    fn overlapping_edges_separate_only_inside_forty_pixel_boundary() {
        let mut s = state();
        let id = s.add_group(
            "Move",
            Bounds {
                x: 139.,
                y: 160.,
                w: 100.,
                h: 100.,
            },
        );
        s.add_group(
            "Peer",
            Bounds {
                x: 200.,
                y: 150.,
                w: 100.,
                h: 100.,
            },
        );
        let b = s.group(id).unwrap().bounds;
        assert_eq!(s.snap_position(id, b).x, 100.);
        assert_eq!(s.snap_position(id, Bounds { x: 140., ..b }).x, 140.);
    }

    fn item() -> Item {
        Item {
            id: 7,
            name: "Brief.TXT".into(),
            path: "Brief.TXT".into(),
            kind: "txt".into(),
            size: 100,
            modified: 0,
            label: None,
        }
    }

    #[test]
    fn top_name_rule_beats_lower_type_rule() {
        // The person's case: 'Name contains craft' to Craft is at the top,
        // an Applications type rule below it.
        let mut state = State::empty();
        let b = Bounds {
            x: 0.,
            y: 0.,
            w: 250.,
            h: 230.,
        };
        let craft_box = state.add_group("Craft", b);
        let apps = state.add_group("Applications", b);
        state.rules.clear();
        state.rules.push(Rule {
            mode: 1,
            pattern: "craft".into(),
            target: craft_box,
            target_tab: None,
            enabled: true,
        });
        state.rules.push(Rule {
            mode: 0,
            pattern: "exe,lnk,url".into(),
            target: apps,
            target_tab: None,
            enabled: true,
        });
        let shortcut = |id: u64, name: &str| Item {
            id,
            name: name.into(),
            path: format!("C:/Desktop/{name}"),
            kind: "lnk".into(),
            size: 10,
            modified: 0,
            label: None,
        };
        let craft_app = shortcut(60, "DesktopCraft.lnk");
        let wallcraft = shortcut(61, "WallCraft.exe");
        let other = shortcut(62, "Notepad.lnk");
        assert_eq!(
            state.best_rule_destination(&craft_app).unwrap().0,
            craft_box
        );
        assert_eq!(
            state.best_rule_destination(&wallcraft).unwrap().0,
            craft_box
        );
        assert_eq!(state.best_rule_destination(&other).unwrap().0, apps);
        // Pins already sitting in Applications are moved to Craft by Apply.
        state
            .group_mut(apps)
            .unwrap()
            .tab_mut()
            .items
            .extend([craft_app, wallcraft, other]);
        assert_eq!(state.apply_rules(), 2);
        assert_eq!(state.group(craft_box).unwrap().tab().items.len(), 2);
        assert_eq!(state.group(apps).unwrap().tab().items.len(), 1);
        assert_eq!(state.apply_rules(), 0);
        // Disabling the top rule lets the next matching rule take over.
        state.rules[0].enabled = false;
        assert_eq!(state.apply_rules(), 2);
        assert_eq!(state.group(apps).unwrap().tab().items.len(), 3);
    }

    /// Manual check against a saved layout copy (never the live file):
    /// DESKTOPCRAFT_RULES_PREVIEW=<copy>.json cargo test preview_rules -- --ignored --nocapture
    #[test]
    #[ignore]
    fn preview_rules_on_saved_workspace_copy() {
        let Some(path) = std::env::var_os("DESKTOPCRAFT_RULES_PREVIEW") else {
            return;
        };
        let state = State::load(Path::new(&path)).expect("readable workspace copy");
        let tab_name = |gid: u64, tid: u64| {
            state
                .group(gid)
                .and_then(|g| g.tabs.iter().find(|t| t.id == tid))
                .map_or("?".to_string(), |t| t.name.clone())
        };
        for g in &state.groups {
            for t in g.tabs.iter().filter(|t| t.folder.is_none()) {
                for item in &t.items {
                    if let Some((gid, tid)) = state.best_rule_destination(item) {
                        if (gid, tid) != (g.id, t.id) {
                            println!(
                                "MOVE {}  from {}  to {}",
                                item.name,
                                t.name,
                                tab_name(gid, tid)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rule_order_alone_decides_and_missing_boxes_fall_through() {
        let mut state = State::empty();
        let b = Bounds {
            x: 0.,
            y: 0.,
            w: 250.,
            h: 230.,
        };
        let apps = state.add_group("Applications", b);
        let archive = state.add_group("Archives", b);
        let folders = state.add_group("Folders", b);
        state.rules.clear();
        state.rules.push(Rule {
            mode: 1,
            pattern: "craft".into(),
            target: apps,
            target_tab: None,
            enabled: true,
        });
        state.rules.push(Rule {
            mode: 0,
            pattern: "zip,rar,7z".into(),
            target: archive,
            target_tab: None,
            enabled: true,
        });
        state.rules.push(Rule {
            mode: 10,
            pattern: "folder".into(),
            target: folders,
            target_tab: None,
            enabled: true,
        });
        let zip = Item {
            id: 55,
            name: "DesktopCraft.ZIP".into(),
            path: "C:/Desktop/DesktopCraft.ZIP".into(),
            kind: "zip".into(),
            size: 10,
            modified: 0,
            label: None,
        };
        let folder = Item {
            id: 56,
            name: "Craft project".into(),
            path: "C:/Desktop/Craft project".into(),
            kind: "folder".into(),
            size: 0,
            modified: 0,
            label: None,
        };
        let plain_zip = Item {
            id: 58,
            name: "Photos.zip".into(),
            path: "C:/Desktop/Photos.zip".into(),
            kind: "zip".into(),
            size: 10,
            modified: 0,
            label: None,
        };
        // The craft rule is on top, so it takes every craft-named item.
        assert_eq!(state.best_rule_destination(&zip).unwrap().0, apps);
        assert_eq!(state.best_rule_destination(&folder).unwrap().0, apps);
        assert_eq!(state.best_rule_destination(&plain_zip).unwrap().0, archive);
        // Raising the ZIP rule above it (Settings' Up button) changes the winner.
        state.rules.swap(0, 1);
        assert_eq!(state.best_rule_destination(&zip).unwrap().0, archive);
        assert_eq!(state.best_rule_destination(&folder).unwrap().0, apps);
        // A rule pointing at a deleted box is skipped, not a dead end.
        state.rules[0].target = 9999;
        assert_eq!(state.best_rule_destination(&zip).unwrap().0, apps);
    }

    #[test]
    fn multi_pin_transfer_is_ordered_atomic_and_never_touches_disk() {
        let mut state = State::empty();
        let b = Bounds {
            x: 0.,
            y: 0.,
            w: 250.,
            h: 230.,
        };
        let src = state.add_group("A", b);
        let dst = state.add_group("B", b);
        let src_tab = state.group(src).unwrap().tab().id;
        let dst_tab = state.group(dst).unwrap().tab().id;
        for (id, name) in [
            (71, "Craft A.txt"),
            (72, "Craft B.zip"),
            (73, "Craft C.png"),
        ] {
            state.group_mut(src).unwrap().tab_mut().items.push(Item {
                id,
                name: name.into(),
                path: format!("C:/Desktop/{name}"),
                kind: "txt".into(),
                size: 1,
                modified: 0,
                label: None,
            });
        }
        let before = state.group(src).unwrap().tab().items.clone();
        assert!(state
            .transfer_many_between_tabs(src, src_tab, &[71, 99], dst, dst_tab)
            .is_err());
        assert_eq!(state.group(src).unwrap().tab().items.len(), 3);
        assert!(state.group(dst).unwrap().tab().items.is_empty());
        assert!(state
            .transfer_many_between_tabs(src, src_tab, &[71, 71], dst, dst_tab)
            .is_err());
        assert_eq!(
            state
                .transfer_many_between_tabs(src, src_tab, &[73, 71], dst, dst_tab)
                .unwrap(),
            2
        );
        assert_eq!(state.group(src).unwrap().tab().items[0].id, 72);
        assert_eq!(
            state
                .group(dst)
                .unwrap()
                .tab()
                .items
                .iter()
                .map(|i| i.id)
                .collect::<Vec<_>>(),
            vec![71, 73]
        );
        assert_eq!(
            state.group(dst).unwrap().tab().items[0].path,
            before[0].path
        );
        assert_eq!(
            state.group(dst).unwrap().tab().items[1].path,
            before[2].path
        );
        assert_eq!(
            state
                .transfer_many_between_tabs(src, src_tab, &[72], src, src_tab)
                .unwrap(),
            0
        );
        assert!(state
            .transfer_many_between_tabs(src, src_tab, &[72], dst, 999)
            .is_err());
        assert_eq!(state.group(src).unwrap().tab().items.len(), 1);
    }

    #[test]
    fn zip_and_folder_rules_match_their_real_file_kinds() {
        let zip = Item {
            id: 1,
            name: "Backup.ZIP".into(),
            path: "C:/Desktop/Backup.ZIP".into(),
            kind: "zip".into(),
            size: 100,
            modified: 0,
            label: None,
        };
        let folder = Item {
            id: 2,
            name: "Project".into(),
            path: "C:/Desktop/Project".into(),
            kind: "folder".into(),
            size: 0,
            modified: 0,
            label: None,
        };
        let mut rule = Rule {
            mode: 0,
            pattern: "zip,rar,7z".into(),
            target: 1,
            target_tab: None,
            enabled: true,
        };
        assert!(rule.matches(&zip));
        assert!(!rule.matches(&folder));
        rule.mode = 10;
        rule.pattern = "folder".into();
        assert!(rule.matches(&folder));
        assert!(!rule.matches(&zip));
        rule.mode = 0;
        assert!(rule.matches(&folder)); // legacy extension-list 'folder'
        rule.enabled = false;
        assert!(!rule.matches(&folder));
    }

    #[test]
    fn default_folder_and_archive_rules_are_migrated_once_without_overwriting_custom_rules() {
        let mut state = State::empty();
        let b = Bounds {
            x: 0.,
            y: 0.,
            w: 250.,
            h: 220.,
        };
        let folders = state.add_group("Folders", b);
        let archives = state.add_group("Archives & installers", b);
        assert_eq!(state.ensure_builtin_sort_rules(), 2);
        assert_eq!(state.ensure_builtin_sort_rules(), 0);
        assert!(state
            .rules
            .iter()
            .any(|r| r.mode == 10 && r.target == folders));
        assert!(state.rules.iter().any(|r| r.target == archives
            && r.mode == 0
            && extension_matches("example.zip", &r.pattern)));
        let mut custom = State::empty();
        let folders = custom.add_group("Folders", b);
        let tab_id = custom.group(folders).unwrap().tab().id;
        custom.rules.push(Rule {
            mode: 10,
            pattern: "folder".into(),
            target: folders,
            target_tab: Some(tab_id),
            enabled: false,
        });
        assert_eq!(custom.ensure_builtin_sort_rules(), 0);
        assert_eq!(custom.rules.len(), 1);
        assert!(!custom.rules[0].enabled);
    }

    #[test]
    fn size_rules_keep_the_original_strict_upper_and_inclusive_lower_bounds() {
        let mut file = item();
        file.path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml").into();
        let mut r = Rule {
            mode: 6,
            pattern: "100".into(),
            target: 1,
            target_tab: None,
            enabled: true,
        };
        assert!(r.matches(&file));
        r.mode = 7;
        assert!(!r.matches(&file));
        r.pattern = "101".into();
        assert!(r.matches(&file));
        r.mode = 1;
        r.pattern = "BRIEF".into();
        assert!(r.matches(&item()));
    }

    #[test]
    fn name_suffix_ignores_only_the_final_extension() {
        let mut file = item();
        file.name = "Quarterly.Brief.TXT".into();
        let mut r = Rule {
            mode: 3,
            pattern: "BRIEF".into(),
            target: 1,
            target_tab: None,
            enabled: true,
        };
        assert!(r.matches(&file));
        r.pattern = "txt".into();
        assert!(!r.matches(&file));
        r.pattern = "quarterly.brief".into();
        assert!(r.matches(&file));
        file.name = "Brief".into();
        r.pattern = "brief".into();
        assert!(r.matches(&file));
        file.name = ".env".into();
        r.pattern = "env".into();
        assert!(!r.matches(&file));
    }

    #[test]
    fn name_and_extension_rules_fold_case_the_same_everywhere() {
        // Implementation decision: simple Unicode lowercase folding, independent
        // of the system or CRT locale (the original's runtime locale is unknown).
        let mut file = item();
        file.name = "MÉMO.TXT".into();
        let mut r = Rule {
            mode: 1,
            pattern: "mémo".into(),
            target: 1,
            target_tab: None,
            enabled: true,
        };
        assert!(r.matches(&file));
        r.mode = 2;
        assert!(r.matches(&file));
        r.mode = 5;
        assert!(!r.matches(&file));
        r.mode = 3;
        assert!(r.matches(&file));
        r.pattern = "MÉmo".into();
        assert!(r.matches(&file));
        file.name = "📁 ReadMe.TXT".into();
        r.pattern = "README".into();
        assert!(r.matches(&file));
        r.mode = 2;
        assert!(!r.matches(&file));
        r.mode = 1;
        assert!(r.matches(&file));
        r.pattern = "ReadMe.TXT.extra".into();
        r.mode = 3;
        assert!(!r.matches(&file));
    }

    #[test]
    fn thai_greek_cyrillic_and_ascii_names_and_extensions_match() {
        let ext = |name: &str, list: &str| {
            let mut f = item();
            f.name = name.into();
            Rule {
                mode: 0,
                pattern: list.into(),
                target: 1,
                target_tab: None,
                enabled: true,
            }
            .matches(&f)
        };
        let name = |name: &str, pattern: &str, mode: u8| {
            let mut f = item();
            f.name = name.into();
            Rule {
                mode,
                pattern: pattern.into(),
                target: 1,
                target_tab: None,
                enabled: true,
            }
            .matches(&f)
        };
        // ASCII extensions in any case, with or without the dot.
        assert!(ext("Report.PDF", "pdf"));
        assert!(ext("report.pdf", ".PDF, docx"));
        assert!(!ext("report.pdf.bak", "pdf"));
        assert!(!ext("README", "readme"));
        assert!(!ext(".env", "env"));
        // Thai has no case: exact matches only, extensions and names alike.
        assert!(ext("รายงานประจำเดือน.เอกสาร", "เอกสาร"));
        assert!(!ext("รายงาน.เอกสาร", "เอกสารใหม่"));
        assert!(ext("รายงาน.PDF", "pdf"));
        assert!(name("รายงานประจำเดือน.pdf", "ประจำ", 1));
        assert!(name("รายงานประจำเดือน.pdf", "รายงาน", 2));
        assert!(name("รายงานประจำเดือน.pdf", "เดือน", 3));
        assert!(!name("รายงานประจำเดือน.pdf", "pdf", 3));
        assert!(name("รายงาน.pdf", "ภาพ", 5));
        // Greek and Cyrillic fold case.
        assert!(ext("ΣΗΜΕΙΩΣΕΙΣ.ΚΕΙΜ", "κειμ"));
        assert!(name("ΣΗΜΕΙΩΣΕΙΣ.txt", "σημειωσεισ", 2));
        assert!(ext("ОТЧЁТ.ДОК", "док"));
        assert!(name("ОТЧЁТ.docx", "отчёт", 3));
        // German sharp s and accents stay distinct from their base letters.
        assert!(name("Straße.txt", "STRASSE", 1) == false);
        assert!(!name("resume.txt", "résumé", 1));
        // The same answers on every computer: folding never uses the locale.
        assert_eq!(fold("İSTANBUL"), "i̇stanbul");
        assert_eq!(fold("ΆΈΉ"), "άέή");
    }

    #[test]
    fn size_and_age_rules_require_an_existing_file() {
        let mut file = item();
        file.path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml").into();
        file.modified = now_seconds();
        for (mode, pattern) in [(6, "0"), (7, "101"), (8, "1"), (9, "100,100")] {
            let r = Rule {
                mode,
                pattern: pattern.into(),
                target: 1,
                target_tab: None,
                enabled: true,
            };
            assert!(r.matches(&file), "file should match mode {mode}");
            let mut folder = file.clone();
            folder.path = env!("CARGO_MANIFEST_DIR").into();
            folder.kind = "folder".into();
            assert!(!r.matches(&folder), "folder must not match mode {mode}");
            let mut missing = file.clone();
            missing.path = concat!(env!("CARGO_MANIFEST_DIR"), "/__missing-rule-fixture__").into();
            assert!(
                !r.matches(&missing),
                "missing file must not match mode {mode}"
            );
        }
    }

    #[test]
    fn size_between_includes_both_endpoints_and_rejects_invalid_ranges() {
        let mut file = item();
        file.path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml").into();
        let mut r = Rule {
            mode: 9,
            pattern: "100,200".into(),
            target: 1,
            target_tab: None,
            enabled: true,
        };
        for (size, expected) in [
            (99, false),
            (100, true),
            (150, true),
            (200, true),
            (201, false),
        ] {
            file.size = size;
            assert_eq!(r.matches(&file), expected, "size {size}");
        }
        for invalid in ["200,100", "100", "100,", ",200", "a,200", "1,2,3", "-1,200"] {
            r.pattern = invalid.into();
            assert!(!r.matches(&file), "invalid range {invalid}");
        }
        r.pattern = format!("0,{}", u64::MAX);
        file.size = u64::MAX;
        assert!(r.matches(&file));
        assert_eq!(size_range(" 100 , 200 "), Some((100, 200)));
    }

    #[test]
    fn file_age_uses_absolute_seconds_and_a_strict_limit() {
        assert!(modified_within(100_000 - 86399, 100_000, 1));
        assert!(!modified_within(100_000 - 86400, 100_000, 1));
        assert!(modified_within(100_000 + 86399, 100_000, 1));
        assert!(!modified_within(100_000 + 86400, 100_000, 1));
        assert!(!modified_within(100_000, 100_000, 0));
    }

    #[test]
    fn rule_slot_fifty_is_used_but_fifty_one_is_not() {
        let mut s = state();
        let b = Bounds {
            x: 0.,
            y: 0.,
            w: 100.,
            h: 100.,
        };
        let from = s.add_group("From", b);
        let to = s.add_group("To", b);
        s.group_mut(from).unwrap().tab_mut().items.push(item());
        let r = Rule {
            mode: 0,
            pattern: "txt".into(),
            target: to,
            target_tab: None,
            enabled: false,
        };
        s.rules = vec![r; 51];
        s.rules[50].enabled = true;
        assert_eq!(s.apply_rules(), 0);
        s.rules[49].enabled = true;
        assert_eq!(s.apply_rules(), 1);
    }

    #[test]
    fn rules_can_target_a_non_active_tab_inside_a_combined_box() {
        let mut state = state();
        let b = Bounds {
            x: 0.,
            y: 0.,
            w: 250.,
            h: 250.,
        };
        let a = state.add_group("Documents", b);
        let b_id = state.add_group("Pictures", b);
        let source_tab = state.group(a).unwrap().tab().id;
        let destination_tab = state.group(b_id).unwrap().tab().id;
        state.group_mut(a).unwrap().tab_mut().items.push(item());
        assert!(state.merge(b_id, a));
        state.group_mut(a).unwrap().active = 0;
        state.rules.push(Rule {
            mode: 0,
            pattern: "txt".into(),
            target: a,
            target_tab: Some(destination_tab),
            enabled: true,
        });
        assert_eq!(state.apply_rules(), 1);
        let g = state.group(a).unwrap();
        assert!(g
            .tabs
            .iter()
            .find(|t| t.id == source_tab)
            .unwrap()
            .items
            .is_empty());
        assert_eq!(
            g.tabs
                .iter()
                .find(|t| t.id == destination_tab)
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(state.apply_rules(), 0);
    }

    #[test]
    fn dragging_a_pin_between_tabs_of_the_same_box_does_not_move_disk_files() {
        let mut state = state();
        let b = Bounds {
            x: 0.,
            y: 0.,
            w: 250.,
            h: 250.,
        };
        let a = state.add_group("One", b);
        let other = state.add_group("Two", b);
        let source_tab = state.group(a).unwrap().tab().id;
        let dest_tab = state.group(other).unwrap().tab().id;
        state.group_mut(a).unwrap().tab_mut().items.push(item());
        assert!(state.merge(other, a));
        state
            .transfer_between_tabs(a, source_tab, 7, a, dest_tab, 0)
            .unwrap();
        let g = state.group(a).unwrap();
        assert!(g
            .tabs
            .iter()
            .find(|t| t.id == source_tab)
            .unwrap()
            .items
            .is_empty());
        assert_eq!(
            g.tabs.iter().find(|t| t.id == dest_tab).unwrap().items[0].path,
            "Brief.TXT"
        );
    }

    #[test]
    fn destination_tab_survives_merge_detach_and_old_rule_json() {
        let mut state = state();
        let b = Bounds {
            x: 0.,
            y: 0.,
            w: 250.,
            h: 250.,
        };
        let old = state.add_group("Old", b);
        let new = state.add_group("New", b);
        let tab_id = state.group(old).unwrap().tab().id;
        state.rules.push(Rule {
            mode: 0,
            pattern: "txt".into(),
            target: old,
            target_tab: Some(tab_id),
            enabled: true,
        });
        assert!(state.merge(old, new));
        assert_eq!(state.rules[0].target, new);
        assert_eq!(state.rule_destination(&state.rules[0]), Some((new, tab_id)));
        let idx = state
            .group(new)
            .unwrap()
            .tabs
            .iter()
            .position(|t| t.id == tab_id)
            .unwrap();
        let separate = state.detach(new, idx, b).unwrap();
        assert_eq!(
            state.rule_destination(&state.rules[0]),
            Some((separate, tab_id))
        );
        let legacy: Rule = serde_json::from_str(&format!(
            r#"{{"mode":0,"pattern":"txt","target":{},"enabled":true}}"#,
            separate
        ))
        .unwrap();
        assert_eq!(legacy.target_tab, None);
        assert_eq!(state.rule_destination(&legacy), Some((separate, tab_id)));
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Bounds {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Item {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub modified: u64,
    /// What Explorer shows for it (e.g. shortcuts without ".lnk"); rules still
    /// match the real file name in `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
impl Item {
    pub fn display(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
    pub fn from_path(path: &Path, id: u64) -> std::io::Result<Self> {
        let meta = fs::metadata(path)?;
        let kind = if meta.is_dir() {
            "folder".into()
        } else {
            path.extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
        };
        Ok(Self {
            id,
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            path: path.to_string_lossy().into(),
            kind,
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|x| x.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |x| x.as_secs()),
            label: None,
        })
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Tab {
    pub id: u64,
    pub name: String,
    pub items: Vec<Item>,
    pub folder: Option<String>,
    pub history: Vec<String>,
    pub list: bool,
    pub sort: u8,
    pub scroll: usize,
    pub icon_scale: f32,
    /// Folder View filter typed in its folder bar: only files whose name
    /// contains it (any case) are shown, e.g. "report" or ".exe". Not saved.
    #[serde(skip)]
    pub filter: String,
}
/// Whether a file name passes a folder filter ("" shows everything).
pub fn filter_matches(name: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty() || fold(name).contains(&fold(filter))
}

/// Characters Windows doesn't allow in a file name.
pub const BAD_NAME_CHARS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
/// What's selected in an in-place rename, the way Explorer does it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NameSelection {
    None,
    Stem,
    All,
}
/// Editing an icon's name in place, like Explorer's own rename: the text is
/// what the icon shows; an extension Windows hides (".lnk", or every extension
/// when "File name extensions" is off) is kept unseen and put back on save. A
/// visible extension stays put while the name before it starts selected.
#[derive(Clone, Debug)]
pub struct NameEdit {
    pub text: String,
    pub selection: NameSelection,
    /// Caret position in characters (used when nothing is selected).
    pub caret: usize,
    /// The extension Windows doesn't show for this file (".lnk"), or "".
    pub hidden_ext: String,
    original: String,
}
impl NameEdit {
    pub fn new(shown: &str, file_name: &str, folder: bool) -> Self {
        let hidden_ext = if !folder
            && file_name.len() > shown.len()
            && file_name.starts_with(shown)
            && file_name[shown.len()..].starts_with('.')
        {
            file_name[shown.len()..].to_string()
        } else {
            String::new()
        };
        // Explorer selects the name up to its last dot, unless it's a folder,
        // the extension is hidden, or the name starts with the dot.
        let selection = match shown.rfind('.') {
            Some(i) if i > 0 && !folder && hidden_ext.is_empty() => NameSelection::Stem,
            _ => NameSelection::All,
        };
        Self {
            text: shown.to_string(),
            selection,
            caret: shown.chars().count(),
            hidden_ext,
            original: shown.to_string(),
        }
    }
    fn len(&self) -> usize {
        self.text.chars().count()
    }
    /// How many characters at the start are selected.
    pub fn selected_chars(&self) -> usize {
        match self.selection {
            NameSelection::None => 0,
            NameSelection::All => self.len(),
            NameSelection::Stem => self
                .text
                .rfind('.')
                .map_or(0, |i| self.text[..i].chars().count()),
        }
    }
    /// Replace the selection; the caret ends after what was put in.
    fn replace_selection(&mut self, with: &str) {
        let n = self.selected_chars();
        let rest: String = self.text.chars().skip(n).collect();
        self.text = format!("{with}{rest}");
        self.caret = with.chars().count();
        self.selection = NameSelection::None;
    }
    fn byte_at(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(i, _)| i)
    }
    /// Type text; returns false when it held characters a file name can't.
    pub fn type_text(&mut self, typed: &str) -> bool {
        let clean: String = typed
            .chars()
            .filter(|c| !BAD_NAME_CHARS.contains(c) && !c.is_control())
            .collect();
        if !clean.is_empty() {
            if self.selection == NameSelection::None {
                let at = self.byte_at(self.caret.min(self.len()));
                self.text.insert_str(at, &clean);
                self.caret = self.caret.min(self.len()) + clean.chars().count();
            } else {
                self.replace_selection(&clean);
            }
        }
        clean.chars().count() == typed.chars().filter(|c| !c.is_control()).count()
    }
    pub fn backspace(&mut self) {
        if self.selection != NameSelection::None {
            self.replace_selection("");
        } else if self.caret > 0 {
            let at = self.byte_at(self.caret - 1);
            self.text.remove(at);
            self.caret -= 1;
        }
    }
    pub fn delete(&mut self) {
        if self.selection != NameSelection::None {
            self.replace_selection("");
        } else if self.caret < self.len() {
            let at = self.byte_at(self.caret);
            self.text.remove(at);
        }
    }
    /// Left/Right like a text box: with a selection they jump to its start or end.
    pub fn left(&mut self) {
        if self.selection != NameSelection::None {
            self.caret = 0;
        } else {
            self.caret = self.caret.saturating_sub(1);
        }
        self.selection = NameSelection::None;
    }
    pub fn right(&mut self) {
        if self.selection != NameSelection::None {
            self.caret = self.selected_chars();
        } else {
            self.caret = (self.caret + 1).min(self.len());
        }
        self.selection = NameSelection::None;
    }
    pub fn home(&mut self) {
        self.caret = 0;
        self.selection = NameSelection::None;
    }
    pub fn end(&mut self) {
        self.caret = self.len();
        self.selection = NameSelection::None;
    }
    pub fn select_all(&mut self) {
        self.selection = NameSelection::All;
    }
    /// The new file name to rename to, or None when nothing would change.
    /// Like Windows, trailing spaces and dots are dropped.
    pub fn new_file_name(&self) -> Option<String> {
        let shown = self.text.trim().trim_end_matches(['.', ' ']);
        (!shown.is_empty() && shown != self.original).then(|| format!("{shown}{}", self.hidden_ext))
    }
    /// The text the icon should show after the rename.
    pub fn shown(&self) -> &str {
        self.text.trim().trim_end_matches(['.', ' '])
    }
}

#[cfg(test)]
mod name_edit_tests {
    use super::*;
    #[test]
    fn visible_extension_keeps_while_the_name_is_replaced() {
        let mut e = NameEdit::new("Plan.csv", "Plan.csv", false);
        assert_eq!(e.selection, NameSelection::Stem);
        assert_eq!(e.selected_chars(), 4);
        e.type_text("Budget");
        assert_eq!(e.text, "Budget.csv");
        e.type_text(" 2026"); // the caret stays before the extension
        assert_eq!(e.text, "Budget 2026.csv");
        assert_eq!(e.new_file_name().as_deref(), Some("Budget 2026.csv"));
        e.end();
        e.backspace();
        assert_eq!(e.text, "Budget 2026.cs");
        e.home();
        e.delete();
        assert_eq!(e.text, "udget 2026.cs");
    }
    #[test]
    fn hidden_shortcut_extension_is_put_back() {
        let mut e = NameEdit::new("ArtCraft", "ArtCraft.lnk", false);
        assert_eq!(e.hidden_ext, ".lnk");
        assert_eq!(e.selection, NameSelection::All);
        e.type_text("Art tools");
        assert_eq!(e.new_file_name().as_deref(), Some("Art tools.lnk"));
        assert_eq!(e.shown(), "Art tools");
    }
    #[test]
    fn folders_select_everything_and_backspace_clears_selection() {
        let mut e = NameEdit::new("Photos 2024.old", "Photos 2024.old", true);
        assert_eq!(e.selection, NameSelection::All);
        e.backspace();
        assert_eq!(e.text, "");
        assert_eq!(e.new_file_name(), None); // an empty name renames nothing
        let mut f = NameEdit::new("notes.txt", "notes.txt", false);
        f.backspace();
        assert_eq!(f.text, ".txt");
        f.backspace(); // caret is at the start now: nothing before it to delete
        assert_eq!(f.text, ".txt");
        f.type_text("todo");
        assert_eq!(f.text, "todo.txt");
        let mut g = NameEdit::new("Plan.csv", "Plan.csv", false);
        g.right(); // collapse to the end of the selected name, before ".csv"
        g.type_text("s");
        assert_eq!(g.text, "Plans.csv");
    }
    #[test]
    fn refuses_characters_windows_forbids_and_unchanged_names() {
        let mut e = NameEdit::new("a.txt", "a.txt", false);
        assert!(!e.type_text("b:c?"));
        assert_eq!(e.text, "bc.txt");
        let same = NameEdit::new("a.txt", "a.txt", false);
        assert_eq!(same.new_file_name(), None);
        let mut dots = NameEdit::new("x", "x", false);
        dots.type_text("report. ");
        assert_eq!(dots.new_file_name().as_deref(), Some("report"));
        let mut all = NameEdit::new("Plan.csv", "Plan.csv", false);
        all.select_all();
        all.type_text("Q");
        assert_eq!(all.text, "Q");
    }
}
impl Tab {
    pub fn new(id: u64, name: &str) -> Self {
        Self {
            id,
            name: name.into(),
            items: vec![],
            folder: None,
            history: vec![],
            list: false,
            sort: 0,
            scroll: 0,
            icon_scale: 1.0,
            filter: String::new(),
        }
    }
    pub fn refresh(&mut self) -> Result<(), String> {
        if let Some(folder) = &self.folder {
            let read = fs::read_dir(folder).map_err(|e| format!("Cannot read this folder: {e}"))?;
            let mut items = vec![];
            for entry in read.filter_map(Result::ok) {
                let mut key = std::collections::hash_map::DefaultHasher::new();
                self.id.hash(&mut key);
                entry.path().hash(&mut key);
                if let Ok(item) = Item::from_path(&entry.path(), key.finish() | (1u64 << 63)) {
                    if !item.name.starts_with('.') && !item.name.eq_ignore_ascii_case("desktop.ini")
                    {
                        items.push(item)
                    }
                }
            }
            self.items = items;
        }
        self.sort_items();
        Ok(())
    }
    pub fn sort_items(&mut self) {
        match self.sort {
            // Build each expensive Unicode lowercase sort key just once per
            // item, instead of on every comparator invocation (O(n log n)).
            1 => self
                .items
                .sort_by_cached_key(|x| (x.kind.clone(), x.name.to_lowercase())),
            2 => self.items.sort_by_key(|x| std::cmp::Reverse(x.modified)),
            3 => self.items.sort_by_key(|x| std::cmp::Reverse(x.size)),
            _ => self
                .items
                .sort_by_cached_key(|x| (x.kind != "folder", x.name.to_lowercase())),
        }
    }
    pub fn navigate(&mut self, path: &str) -> Result<(), String> {
        if !Path::new(path).is_dir() {
            return Err("Choose an existing folder.".into());
        }
        let old = self.clone();
        if let Some(folder) = self.folder.take() {
            self.history.push(folder)
        }
        self.folder = Some(path.into());
        self.scroll = 0;
        if let Err(e) = self.refresh() {
            *self = old;
            return Err(e);
        }
        Ok(())
    }
    pub fn back(&mut self) -> Result<(), String> {
        if let Some(previous) = self.history.pop() {
            self.folder = Some(previous);
            self.scroll = 0;
            self.refresh()?;
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Group {
    pub id: u64,
    pub bounds: Bounds,
    pub page: usize,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub color: usize,
    pub opacity: f32,
    pub rolled: bool,
    pub locked: bool,
    pub keep_visible: bool,
    /// Background colour chosen with the hue / brightness / saturation sliders
    /// (red, green, blue 0..1); None uses the preset colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_color: Option<[f32; 3]>,
}
impl Group {
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }
    pub fn tab_mut(&mut self) -> &mut Tab {
        let i = self.active.min(self.tabs.len() - 1);
        &mut self.tabs[i]
    }
    /// Reserve the right-hand titlebar controls and a tab-cycling button
    /// when tabs are too numerous to be shown without overlapping controls.
    pub fn header_tabs(&self) -> (Vec<(usize, Bounds)>, Option<Bounds>) {
        if self.tabs.len() <= 1 {
            return (vec![], None);
        }
        let b = self.bounds;
        let total_width = (b.w - 105.).max(40.);
        let capacity = ((total_width / 64.).floor() as usize).max(1);
        let scrolling = self.tabs.len() > capacity;
        let tab_width = if scrolling {
            (total_width - 29.).max(35.)
        } else {
            total_width
        };
        let visible = ((tab_width / 64.).floor() as usize)
            .max(1)
            .min(self.tabs.len());
        let first = if scrolling {
            self.active
                .saturating_sub(visible - 1)
                .min(self.tabs.len() - visible)
        } else {
            0
        };
        let width = (tab_width / visible as f32).min(150.).max(1.);
        let mut tabs = Vec::with_capacity(visible);
        for i in first..first + visible {
            let x = b.x + 7. + (i - first) as f32 * width;
            tabs.push((
                i,
                Bounds {
                    x,
                    y: b.y + 6.,
                    w: (width - 3.).max(1.),
                    h: 30.,
                },
            ));
        }
        let next = scrolling.then_some(Bounds {
            x: b.x + b.w - 104.,
            y: b.y + 7.,
            w: 24.,
            h: 28.,
        });
        (tabs, next)
    }
    pub fn drop_tab_at(&self, x: f32, y: f32) -> Option<usize> {
        self.header_tabs()
            .0
            .into_iter()
            .find_map(|(index, bounds)| bounds.contains(x, y).then_some(index))
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Rule {
    pub mode: u8,
    pub pattern: String,
    pub target: u64,
    /// Stable tab ID, preserving rule destination when boxes are combined.
    /// Older saved rules target the first regular tab in their box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_tab: Option<u64>,
    pub enabled: bool,
}
impl Rule {
    pub fn matches(&self, item: &Item) -> bool {
        let pattern = fold(&self.pattern);
        let path = fold(&item.path);
        // Size/date predicates need a real file, not folder metadata or a stale pin.
        if (6..=9).contains(&self.mode) && !Path::new(&item.path).is_file() {
            return false;
        }
        self.enabled
            && !pattern.is_empty()
            && match self.mode {
                // A 'folder' token in legacy extension lists is also valid.
                // New rules should use explicit mode 10 (Folder), instead.
                0 => {
                    (item.kind == "folder"
                        && self
                            .pattern
                            .split(',')
                            .any(|part| part.trim().eq_ignore_ascii_case("folder")))
                        || (item.kind != "folder" && extension_matches(&item.name, &self.pattern))
                }
                1 | 2 | 3 | 5 => name_rule_matches(&item.name, &self.pattern, self.mode),
                10 => item.kind == "folder",
                4 => path.contains(&pattern),
                6 => pattern.parse::<u64>().is_ok_and(|n| item.size >= n),
                7 => pattern.parse::<u64>().is_ok_and(|n| item.size < n),
                8 => pattern
                    .parse::<u64>()
                    .is_ok_and(|days| modified_within(item.modified, now_seconds(), days)),
                9 => size_range(&pattern)
                    .is_some_and(|(lower, upper)| lower <= item.size && item.size <= upper),
                _ => false,
            }
    }
}
/// The rule list is strictly ordered: the topmost enabled rule that matches
/// (and whose destination still exists) wins, whatever kind of rule it is.
/// No rule kind outranks another, so the order shown in Settings alone decides.
fn best_destination_from_rules(item: &Item, rules: &[(Rule, (u64, u64))]) -> Option<(u64, u64)> {
    rules
        .iter()
        .find(|(r, _)| r.matches(item))
        .map(|(_, dest)| *dest)
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Snapshot {
    pub name: String,
    pub groups: Vec<Group>,
    pub pages: Vec<String>,
    #[serde(default)]
    pub page_birth: Vec<u64>,
    pub time: u64,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct State {
    pub version: u32,
    pub groups: Vec<Group>,
    pub pages: Vec<String>,
    /// Persisted creation order remains stable after alphabetical sorting.
    /// Older workspaces get IDs inferred from their existing page order.
    #[serde(default)]
    pub page_birth: Vec<u64>,
    #[serde(default)]
    pub next_page_birth: u64,
    pub page: usize,
    pub next_id: u64,
    pub rules: Vec<Rule>,
    pub snapshots: Vec<Snapshot>,
    pub blend: bool,
    pub tint: bool,
    pub snap: bool,
    pub quick_hide: bool,
    pub auto_rules: bool,
    pub wallpaper: usize,
    /// Desktop mode: one layout per monitor arrangement (signature -> group -> place).
    #[serde(default)]
    pub layouts: std::collections::BTreeMap<String, std::collections::BTreeMap<u64, Placement>>,
    /// Each group's most recent place, used for arrangements not seen before.
    #[serde(default)]
    pub placements: std::collections::BTreeMap<u64, Placement>,
    /// Set while quick-hide has Explorer's icons hidden, so a crash or shutdown
    /// is recovered on the next start.
    #[serde(default)]
    pub explorer_icons_hidden: bool,
    /// Settings that mirror the original manager's pages (all with defaults so
    /// older saved layouts still load).
    #[serde(default)]
    pub prefs: Prefs,
}

fn yes() -> bool {
    true
}
pub fn default_peek_shortcut() -> String {
    "Ctrl+Alt+F10".to_string()
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Prefs {
    /// Titlebars: 0 always visible, 1 on mouseover, 2 never (manager "Titlebars").
    #[serde(default)]
    pub titlebar: u8,
    /// Personalization "Hide scrollbars until mouseover" (original default: on).
    #[serde(default = "yes")]
    pub hide_scrollbars: bool,
    /// Roll-up: enabled, double-click titlebar toggles, how a rolled group opens
    /// (0 on mouseover immediately, 1 after a delay, 2 on click), and docking
    /// (roll up groups placed at the top/bottom edge of a screen).
    /// Default: rollup on, double-click title toggles, reveal on hover.
    #[serde(default = "yes")]
    pub rollup: bool,
    #[serde(default = "yes")]
    pub rollup_double_click: bool,
    #[serde(default)]
    pub rollup_open: u8,
    #[serde(default = "yes")]
    pub rollup_dock: bool,
    /// Page visibility and old workspace flags. Legacy flags below stay only
    /// for JSON compatibility: only the bottom selector and Ctrl/Alt+arrows
    /// navigate, and only the confirmed + action creates a page.
    #[serde(default = "yes")]
    pub pages: bool,
    #[serde(default = "yes")]
    pub page_edge: bool,
    #[serde(default = "yes")]
    pub page_keys: bool,
    #[serde(default = "yes")]
    pub page_middle: bool,
    /// Empty = follow Windows primary; otherwise use this monitor device key
    /// for the app's tools, dialogs, and new-group fallback placement.
    /// This never changes the Windows operating system's primary display.
    #[serde(default)]
    pub workspace_monitor: String,
    /// Peek can be toggled with a user-chosen Ctrl+Shift or Ctrl+Alt
    /// shortcut; never intercept Windows language switching (Win+Space).
    #[serde(default = "yes")]
    pub peek_hotkeys: bool,
    #[serde(default = "default_peek_shortcut")]
    pub peek_shortcut: String,
    /// After moving a pin to an inactive tab, show destination (true) or stay
    /// on the source tab (false). Stored per workspace for all drag routes.
    #[serde(default)]
    pub switch_tab_after_drop: bool,
}
impl Default for Prefs {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap()
    }
}
/// Recovered roll-up reveal delay when "after a delay" is chosen (DelayTime).
pub const ROLLUP_DELAY: f64 = 0.5;

/// Hue (0..1), saturation (0..1), brightness (0..1) to red, green, blue.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1. - s), v * (1. - s * f), v * (1. - s * (1. - f)));
    match i as i32 % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}
/// Red, green, blue to hue, saturation, brightness (inverse of hsv_to_rgb).
pub fn rgb_to_hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0], c[1], c[2]);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0. {
        0.
    } else if max == r {
        ((g - b) / d).rem_euclid(6.) / 6.
    } else if max == g {
        ((b - r) / d + 2.) / 6.
    } else {
        ((r - g) / d + 4.) / 6.
    };
    (h, if max == 0. { 0. } else { d / max }, max)
}
/// A group's place on one monitor: the monitor's device name and the position
/// relative to that monitor's work area (logical pixels).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Placement {
    pub monitor: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
impl State {
    pub fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn group(&self, id: u64) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }
    pub fn group_mut(&mut self, id: u64) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }
    pub fn snap_position(&self, id: u64, proposed: Bounds) -> Bounds {
        let Some(group) = self.group(id) else {
            return proposed;
        };
        let mut dx = None;
        let mut dy = None;
        let consider = |best: &mut Option<f32>, distance: f32, limit: f32, inclusive: bool| {
            if (distance.abs() < limit || (inclusive && distance.abs() == limit))
                && best.is_none_or(|old| distance.abs() < old.abs())
            {
                *best = Some(distance);
            }
        };
        for other in self
            .groups
            .iter()
            .filter(|g| g.id != id && g.page == group.page)
        {
            let b = other.bounds;
            for distance in [b.x - proposed.x, b.x + b.w - proposed.x - proposed.w] {
                consider(&mut dx, distance, SNAP_ALIGNMENT, true);
            }
            for distance in [b.y - proposed.y, b.y + b.h - proposed.y - proposed.h] {
                consider(&mut dy, distance, SNAP_ALIGNMENT, true);
            }
            // A box beside its neighbor should snap even if there is still a
            // small positive gap. Require overlap on the orthogonal axis so
            // distant unrelated boxes do not attract each other.
            let row_overlap = proposed.y < b.y + b.h + SNAP_PADDING
                && proposed.y + proposed.h > b.y - SNAP_PADDING;
            let column_overlap = proposed.x < b.x + b.w + SNAP_PADDING
                && proposed.x + proposed.w > b.x - SNAP_PADDING;
            if row_overlap {
                let right_gap = b.x - (proposed.x + proposed.w);
                let left_gap = proposed.x - (b.x + b.w);
                // A separated box attracts only within 20 px; a slightly
                // overlapping pair can be resolved within the original 40 px.
                consider(
                    &mut dx,
                    right_gap,
                    if right_gap >= 0. {
                        SNAP_ALIGNMENT
                    } else {
                        SNAP_OVERLAP
                    },
                    false,
                );
                consider(
                    &mut dx,
                    -left_gap,
                    if left_gap >= 0. {
                        SNAP_ALIGNMENT
                    } else {
                        SNAP_OVERLAP
                    },
                    false,
                );
            }
            if column_overlap {
                let bottom_gap = b.y - (proposed.y + proposed.h);
                let top_gap = proposed.y - (b.y + b.h);
                consider(
                    &mut dy,
                    bottom_gap,
                    if bottom_gap >= 0. {
                        SNAP_ALIGNMENT
                    } else {
                        SNAP_OVERLAP
                    },
                    false,
                );
                consider(
                    &mut dy,
                    -top_gap,
                    if top_gap >= 0. {
                        SNAP_ALIGNMENT
                    } else {
                        SNAP_OVERLAP
                    },
                    false,
                );
            }
        }
        // Match the clear spacing between two already-neighboring boxes.
        // This works on rows or columns and only when the proposed box sits
        // along the same band; unrelated boxes never pull it across the page.
        let peers: Vec<&Group> = self
            .groups
            .iter()
            .filter(|g| g.id != id && g.page == group.page)
            .collect();
        for (i, first) in peers.iter().enumerate() {
            for second in peers.iter().skip(i + 1) {
                let (a, b) = (first.bounds, second.bounds);
                let overlap_y = proposed.y < a.y + a.h
                    && proposed.y + proposed.h > a.y
                    && proposed.y < b.y + b.h
                    && proposed.y + proposed.h > b.y;
                let overlap_x = proposed.x < a.x + a.w
                    && proposed.x + proposed.w > a.x
                    && proposed.x < b.x + b.w
                    && proposed.x + proposed.w > b.x;
                let (left, right) = if a.x <= b.x { (a, b) } else { (b, a) };
                let gap_x = right.x - (left.x + left.w);
                if overlap_y {
                    if gap_x >= proposed.w + 8. {
                        // Center a box in the space between two neighbors:
                        // both gaps become equal without any overlap.
                        let middle = (left.x + left.w + right.x - proposed.w) / 2.;
                        consider(&mut dx, middle - proposed.x, SNAP_ALIGNMENT, true);
                    }
                    if (4.0..=100.0).contains(&gap_x) {
                        // Repeat the established gutter on either outside edge.
                        for target_x in [right.x + right.w + gap_x, left.x - gap_x - proposed.w] {
                            consider(&mut dx, target_x - proposed.x, SNAP_ALIGNMENT, true);
                        }
                    }
                }
                let (top, bottom) = if a.y <= b.y { (a, b) } else { (b, a) };
                let gap_y = bottom.y - (top.y + top.h);
                if overlap_x {
                    if gap_y >= proposed.h + 8. {
                        let middle = (top.y + top.h + bottom.y - proposed.h) / 2.;
                        consider(&mut dy, middle - proposed.y, SNAP_ALIGNMENT, true);
                    }
                    if (4.0..=100.0).contains(&gap_y) {
                        for target_y in [bottom.y + bottom.h + gap_y, top.y - gap_y - proposed.h] {
                            consider(&mut dy, target_y - proposed.y, SNAP_ALIGNMENT, true);
                        }
                    }
                }
            }
        }
        Bounds {
            x: proposed.x + dx.unwrap_or(0.0),
            y: proposed.y + dy.unwrap_or(0.0),
            ..proposed
        }
    }
    /// Snap the dragged resize edge (not the opposite fixed edge) to a
    /// neighbor's left/right or top/bottom boundary. No resizing across pages.
    pub fn snap_resize(
        &self,
        id: u64,
        proposed: Bounds,
        edges: u8,
        min: (f32, f32),
        area: Bounds,
    ) -> Bounds {
        let Some(group) = self.group(id) else {
            return proposed;
        };
        let mut result = proposed;
        let (mut l, mut r) = (proposed.x, proposed.x + proposed.w);
        let (mut t, mut b) = (proposed.y, proposed.y + proposed.h);
        let limit = SNAP_ALIGNMENT.min(16.0);
        let near_band = SNAP_OVERLAP; // Neighbor rows may have a 20-40px gutter.
        let mut best_x = limit + 1.0;
        let mut best_y = limit + 1.0;
        for other in self
            .groups
            .iter()
            .filter(|g| g.id != id && g.page == group.page)
        {
            let peer = other.bounds;
            let band_y = t <= peer.y + peer.h + near_band && b >= peer.y - near_band;
            let band_x = l <= peer.x + peer.w + near_band && r >= peer.x - near_band;
            if band_y {
                for target in [peer.x, peer.x + peer.w] {
                    if edges & 1 != 0
                        && (l - target).abs() < best_x
                        && target >= area.x
                        && r - target >= min.0
                    {
                        best_x = (l - target).abs();
                        result.x = target;
                        result.w = r - target;
                    } else if edges & 2 != 0
                        && (r - target).abs() < best_x
                        && target <= area.x + area.w
                        && target - l >= min.0
                    {
                        best_x = (r - target).abs();
                        result.w = target - l;
                    }
                }
            }
            if band_x {
                for target in [peer.y, peer.y + peer.h] {
                    if edges & 4 != 0
                        && (t - target).abs() < best_y
                        && target >= area.y
                        && b - target >= min.1
                    {
                        best_y = (t - target).abs();
                        result.y = target;
                        result.h = b - target;
                    } else if edges & 8 != 0
                        && (b - target).abs() < best_y
                        && target <= area.y + area.h
                        && target - t >= min.1
                    {
                        best_y = (b - target).abs();
                        result.h = target - t;
                    }
                }
            }
        }
        // No coordinates or sizes are committed until after all candidate
        // checks, preserving the fixed far edges and min dimensions.
        l = result.x;
        r = result.x + result.w;
        t = result.y;
        b = result.y + result.h;
        debug_assert!(r - l >= min.0 - 0.01 && b - t >= min.1 - 0.01);
        result
    }
    /// Make legacy page metadata safe without changing any page or group.
    fn sync_page_birth(&mut self) {
        let distinct = self
            .page_birth
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        if self.page_birth.len() != self.pages.len() || distinct != self.pages.len() {
            self.page_birth = (0..self.pages.len() as u64).collect();
        }
        self.next_page_birth = self
            .page_birth
            .iter()
            .copied()
            .max()
            .map_or(0, |v| v.saturating_add(1))
            .max(self.next_page_birth);
    }
    /// Sort page tabs without moving files, and remap every box's page index.
    /// newest_first uses persistent creation IDs so repeated sorting is stable.
    pub fn sort_pages(&mut self, newest_first: bool) {
        self.sync_page_birth();
        let mut order: Vec<usize> = (0..self.pages.len()).collect();
        if newest_first {
            order.sort_by_key(|&i| std::cmp::Reverse(self.page_birth[i]));
        } else {
            order.sort_by(|&a, &b| {
                self.pages[a]
                    .to_lowercase()
                    .cmp(&self.pages[b].to_lowercase())
                    .then(a.cmp(&b))
            });
        }
        let mut destination = vec![0; order.len()];
        for (new_index, old_index) in order.iter().enumerate() {
            destination[*old_index] = new_index;
        }
        let current = self.page;
        self.pages = order.iter().map(|&i| self.pages[i].clone()).collect();
        self.page_birth = order.iter().map(|&i| self.page_birth[i]).collect();
        for group in &mut self.groups {
            group.page = destination[group.page];
        }
        self.page = destination[current];
    }
    /// Explicit, confirmed page creation. Navigation must never call this.
    pub fn create_page(&mut self, name: &str) -> Result<usize, &'static str> {
        let clean = name.trim();
        if clean.is_empty() {
            return Err("Enter a page name");
        }
        if clean.chars().count() > 60 {
            return Err("Page name is too long (max 60 characters)");
        }
        self.sync_page_birth();
        let index = self.pages.len();
        self.pages.push(clean.to_owned());
        self.page_birth.push(self.next_page_birth);
        self.next_page_birth = self.next_page_birth.saturating_add(1);
        self.page = index;
        Ok(index)
    }
    /// Rename only the label; keep every group on its original page.
    pub fn rename_page(&mut self, index: usize, name: &str) -> Result<(), &'static str> {
        let clean = name.trim();
        if clean.is_empty() {
            return Err("Enter a page name");
        }
        if clean.chars().count() > 60 {
            return Err("Page name is too long (max 60 characters)");
        }
        let entry = self.pages.get_mut(index).ok_or("Page not found")?;
        *entry = clean.to_owned();
        Ok(())
    }
    /// Delete only an empty page; never discard its groups or disk files.
    /// Renumber remaining groups and keep the active page in bounds.
    pub fn delete_empty_page(&mut self, index: usize) -> Result<(), &'static str> {
        if self.pages.len() <= 1 {
            return Err("At least one page must remain");
        }
        if index >= self.pages.len() {
            return Err("Page not found");
        }
        if self.groups.iter().any(|g| g.page == index) {
            return Err("Move all groups off this page before deleting it");
        }
        self.sync_page_birth();
        self.pages.remove(index);
        self.page_birth.remove(index);
        for g in &mut self.groups {
            if g.page > index {
                g.page -= 1;
            }
        }
        self.page = if self.page > index {
            self.page - 1
        } else if self.page == index {
            index.min(self.pages.len() - 1)
        } else {
            self.page
        };
        Ok(())
    }
    /// Add an empty virtual tab directly to an existing Box. Does not create
    /// a directory, move any file, or change stable IDs of existing tabs.
    pub fn add_tab(&mut self, group: u64) -> Result<u64, String> {
        let g = self.group(group).ok_or("Box no longer exists")?;
        if g.tabs.len() >= 128 {
            return Err("Too many tabs in this Box".into());
        }
        let mut number = 2usize;
        let name = loop {
            let candidate = format!("Tab {number}");
            if !g
                .tabs
                .iter()
                .any(|tab| tab.name.eq_ignore_ascii_case(&candidate))
            {
                break candidate;
            }
            number += 1;
        };
        let tab_id = self.id();
        let g = self.group_mut(group).unwrap();
        g.tabs.push(Tab::new(tab_id, &name));
        g.active = g.tabs.len() - 1;
        Ok(tab_id)
    }
    pub fn add_group(&mut self, name: &str, bounds: Bounds) -> u64 {
        let id = self.id();
        let tab_id = self.id();
        self.groups.push(Group {
            id,
            bounds,
            page: self.page,
            tabs: vec![Tab::new(tab_id, name)],
            active: 0,
            color: (id as usize) % 6,
            opacity: 0.78,
            rolled: false,
            locked: false,
            keep_visible: false,
            custom_color: None,
        });
        id
    }
    /// Link an existing box to a real folder, without moving disk files.
    /// Refuse to discard a regular box's virtual pins implicitly.
    pub fn bind_group_folder(&mut self, group: u64, path: &Path) -> Result<(), String> {
        let box_ref = self.group_mut(group).ok_or("Box does not exist")?;
        let tab = box_ref.tab_mut();
        if tab.folder.is_none() && !tab.items.is_empty() {
            return Err("Move or unpin the box's existing icons before binding a folder.".into());
        }
        tab.navigate(&path.to_string_lossy())
    }
    pub fn add_portal(&mut self, path: &Path, bounds: Bounds) -> Result<u64, String> {
        if !path.is_dir() {
            return Err("Choose an existing folder.".into());
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let id = self.add_group(&name, bounds);
        if let Err(e) = self
            .group_mut(id)
            .unwrap()
            .tab_mut()
            .navigate(&path.to_string_lossy())
        {
            self.groups.retain(|g| g.id != id);
            return Err(e);
        }
        Ok(id)
    }
    /// Atomic virtual pin paste into a Box. Never moves/copies disk objects.
    /// A real file may belong to one regular Tab at a time; preserve existing
    /// pin IDs when reassigning, and prepare all items before mutating state.
    pub fn pin_clipboard_paths(&mut self, group: u64, paths: &[PathBuf]) -> Result<usize, String> {
        let destination = self.group(group).ok_or("Box not found")?;
        if destination.tab().folder.is_some() {
            return Err("Use a folder paste for a linked Folder View".into());
        }
        if paths.is_empty() {
            return Err("No files to pin".into());
        }
        if paths.len() > 4096 {
            return Err("Too many files selected".into());
        }
        let dest_id = destination.tab().id;
        let mut unique = std::collections::HashSet::new();
        let mut staged = Vec::new();
        for path in paths {
            if !path.exists() {
                return Err(format!("File no longer exists: {}", path.display()));
            }
            let key = path.to_string_lossy().to_lowercase();
            if !unique.insert(key) {
                continue;
            }
            let old = self
                .groups
                .iter()
                .flat_map(|g| &g.tabs)
                .filter(|tab| tab.folder.is_none())
                .flat_map(|tab| &tab.items)
                .find(|item| item.path.eq_ignore_ascii_case(&path.to_string_lossy()))
                .cloned();
            let fresh = Item::from_path(path, 0).map_err(|e| e.to_string())?;
            // Do not overwrite an explicit manually assigned display label,
            // and retain a stable pin id for files previously in another Box.
            let item = old.unwrap_or(fresh);
            staged.push(item);
        }
        if staged.is_empty() {
            return Ok(0);
        }
        let dst_keys: std::collections::HashSet<_> = self
            .group(group)
            .unwrap()
            .tab()
            .items
            .iter()
            .map(|i| i.path.to_lowercase())
            .collect();
        let mut moves = Vec::new();
        for mut item in staged {
            if dst_keys.contains(&item.path.to_lowercase()) {
                continue;
            }
            if item.id == 0 {
                item.id = self.id();
            }
            moves.push(item);
        }
        if moves.is_empty() {
            return Ok(0);
        }
        let keys: std::collections::HashSet<String> =
            moves.iter().map(|i| i.path.to_lowercase()).collect();
        for g in &mut self.groups {
            for tab in &mut g.tabs {
                if tab.folder.is_none() {
                    tab.items.retain(|i| !keys.contains(&i.path.to_lowercase()));
                }
            }
        }
        let total = moves.len();
        let dest = self
            .group_mut(group)
            .and_then(|g| g.tabs.iter_mut().find(|t| t.id == dest_id))
            .ok_or("Destination Tab disappeared")?;
        dest.items.extend(moves);
        Ok(total)
    }
    pub fn import(&mut self, path: &Path, target: u64) -> Result<(), String> {
        let id = self.id();
        let item = Item::from_path(path, id).map_err(|e| e.to_string())?;
        let g = self.group_mut(target).ok_or("Choose a group first.")?;
        if g.tab().folder.is_some() {
            return Err(
                "Folder views mirror their folder. Add this item to a regular group instead."
                    .into(),
            );
        }
        if !g.tab().items.iter().any(|x| x.path == item.path) {
            g.tab_mut().items.push(item)
        }
        Ok(())
    }
    pub fn transfer(
        &mut self,
        source: u64,
        item_id: u64,
        target: u64,
        index: usize,
    ) -> Result<(), String> {
        let src_tab = self.group(source).ok_or("Source box missing")?.tab().id;
        let dst_tab = self
            .group(target)
            .ok_or("Destination box missing")?
            .tab()
            .id;
        self.transfer_between_tabs(source, src_tab, item_id, target, dst_tab, index)
    }
    /// Move a virtual pin between identified tabs, including two tabs in one box.
    /// No filesystem rename or move is performed.
    /// Move selected virtual pins as one validated operation. Linked Folder
    /// Views cannot be batch targets because they represent disk folders.
    pub fn transfer_many_between_tabs(
        &mut self,
        source: u64,
        src_tab: u64,
        ids: &[u64],
        target: u64,
        dst_tab: u64,
    ) -> Result<usize, String> {
        if ids.is_empty() {
            return Err("No pins selected".into());
        }
        if source == target && src_tab == dst_tab {
            return Ok(0);
        }
        let source_items = self
            .group(source)
            .and_then(|g| {
                g.tabs
                    .iter()
                    .find(|t| t.id == src_tab && t.folder.is_none())
            })
            .ok_or("Source must be a regular tab")?;
        let destination = self
            .group(target)
            .and_then(|g| {
                g.tabs
                    .iter()
                    .find(|t| t.id == dst_tab && t.folder.is_none())
            })
            .ok_or("Destination must be a regular tab")?;
        let distinct: std::collections::HashSet<_> = ids.iter().copied().collect();
        if distinct.len() != ids.len() {
            return Err("Duplicate selected pin IDs".into());
        }
        let ordered: Vec<_> = source_items
            .items
            .iter()
            .filter(|item| distinct.contains(&item.id))
            .cloned()
            .collect();
        if ordered.len() != ids.len() {
            return Err("Some pins are no longer in the source tab".into());
        }
        if ordered.iter().any(|item| {
            destination
                .items
                .iter()
                .any(|existing| existing.path.eq_ignore_ascii_case(&item.path))
        }) {
            return Err("A selected pin is already in the destination tab".into());
        }
        self.group_mut(source)
            .unwrap()
            .tabs
            .iter_mut()
            .find(|t| t.id == src_tab)
            .unwrap()
            .items
            .retain(|item| !distinct.contains(&item.id));
        let paths: std::collections::HashSet<_> = ordered
            .iter()
            .map(|item| item.path.to_lowercase())
            .collect();
        for group in &mut self.groups {
            for tab in &mut group.tabs {
                if tab.folder.is_none() && !(group.id == target && tab.id == dst_tab) {
                    tab.items
                        .retain(|pin| !paths.contains(&pin.path.to_lowercase()));
                }
            }
        }
        let count = ordered.len();
        self.group_mut(target)
            .unwrap()
            .tabs
            .iter_mut()
            .find(|t| t.id == dst_tab)
            .unwrap()
            .items
            .extend(ordered);
        Ok(count)
    }
    pub fn transfer_between_tabs(
        &mut self,
        source: u64,
        src_tab: u64,
        item_id: u64,
        target: u64,
        dst_tab: u64,
        index: usize,
    ) -> Result<(), String> {
        if source == target && src_tab == dst_tab {
            return Ok(());
        }
        let find_tab = |gid, tid| {
            self.group(gid)
                .and_then(|g| g.tabs.iter().find(|t| t.id == tid))
        };
        let src = find_tab(source, src_tab).ok_or("Source tab missing")?;
        if src.folder.is_some() {
            return Err("Linked folders retain their on-disk files".into());
        }
        let position = src
            .items
            .iter()
            .position(|item| item.id == item_id)
            .ok_or("Item missing")?;
        let dest = find_tab(target, dst_tab).ok_or("Destination tab missing")?;
        if dest.folder.is_some() {
            return Err("Drop into a linked folder to move its file on disk".into());
        }
        let item = self
            .group_mut(source)
            .unwrap()
            .tabs
            .iter_mut()
            .find(|t| t.id == src_tab)
            .unwrap()
            .items
            .remove(position);
        for group in &mut self.groups {
            for tab in &mut group.tabs {
                if tab.folder.is_none() {
                    tab.items
                        .retain(|pin| !pin.path.eq_ignore_ascii_case(&item.path));
                }
            }
        }
        let dest = self
            .group_mut(target)
            .unwrap()
            .tabs
            .iter_mut()
            .find(|t| t.id == dst_tab)
            .unwrap();
        dest.items.insert(index.min(dest.items.len()), item);
        Ok(())
    }
    /// Reorder tab headers without changing tab IDs or sorting-rule targets.
    pub fn reorder_tab(&mut self, group: u64, tab_id: u64, to: usize) -> bool {
        let Some(g) = self.group_mut(group) else {
            return false;
        };
        let Some(from) = g.tabs.iter().position(|tab| tab.id == tab_id) else {
            return false;
        };
        if to >= g.tabs.len() {
            return false;
        }
        if from == to {
            return true;
        }
        let active_id = g.tab().id;
        let tab = g.tabs.remove(from);
        g.tabs.insert(to, tab);
        g.active = g
            .tabs
            .iter()
            .position(|tab| tab.id == active_id)
            .unwrap_or(0);
        true
    }
    pub fn merge(&mut self, source: u64, target: u64) -> bool {
        if source == target || self.group(source).is_none() || self.group(target).is_none() {
            return false;
        }
        if self.group(source).unwrap().locked || self.group(target).unwrap().locked {
            return false;
        }
        let i = self.groups.iter().position(|g| g.id == source).unwrap();
        let g = self.groups.remove(i);
        // Rules keep following the same stable tab after groups are combined.
        let default_tab = g.tabs.iter().find(|t| t.folder.is_none()).map(|t| t.id);
        for rule in &mut self.rules {
            if rule.target == source {
                rule.target = target;
                if rule.target_tab.is_none() {
                    rule.target_tab = default_tab;
                }
            }
        }
        let dst = self.group_mut(target).unwrap();
        dst.tabs.extend(g.tabs);
        dst.active = dst.tabs.len() - 1;
        true
    }
    pub fn detach(&mut self, group: u64, tab: usize, bounds: Bounds) -> Option<u64> {
        if self.group(group)?.tabs.len() < 2 {
            return None;
        }
        let color = self.group(group)?.color;
        let g = self.group_mut(group)?;
        let tab = g.tabs.remove(tab);
        g.active = g.active.min(g.tabs.len() - 1);
        let id = self.id();
        for rule in &mut self.rules {
            if rule.target == group && rule.target_tab == Some(tab.id) {
                rule.target = id;
            }
        }
        self.groups.push(Group {
            id,
            bounds,
            page: self.page,
            tabs: vec![tab],
            active: 0,
            color,
            opacity: 0.78,
            rolled: false,
            locked: false,
            keep_visible: false,
            custom_color: None,
        });
        Some(id)
    }
    /// Repair omitted starter rules without overwriting a user's chosen
    /// destination, disabled rule, or linked Folder View.
    pub fn ensure_builtin_sort_rules(&mut self) -> usize {
        let mut additions = Vec::new();
        for group in &self.groups {
            for tab in &group.tabs {
                if tab.folder.is_some() {
                    continue;
                }
                let name = tab.name.trim().to_ascii_lowercase();
                let template = if name == "folders" || name == "folder" {
                    Some((10, "folder"))
                } else if ["archives", "archive", "archives & installers"].contains(&name.as_str())
                {
                    Some((0, "zip,rar,7z,exe,msi,iso"))
                } else {
                    None
                };
                let Some((mode, pattern)) = template else {
                    continue;
                };
                let has_rule_for_tab = self.rules.iter().any(|rule| {
                    rule.target == group.id
                        && (rule.target_tab == Some(tab.id)
                            || rule.target_tab.is_none()
                                && group
                                    .tabs
                                    .iter()
                                    .find(|t| t.folder.is_none())
                                    .is_some_and(|t| t.id == tab.id))
                });
                let has_other_matching_rule = self.rules.iter().any(|rule| {
                    if mode == 10 {
                        rule.mode == 10
                            || (rule.mode == 0
                                && rule
                                    .pattern
                                    .split(',')
                                    .any(|s| s.trim().eq_ignore_ascii_case("folder")))
                    } else {
                        rule.mode == 0 && extension_matches("example.zip", &rule.pattern)
                    }
                });
                if !has_rule_for_tab && !has_other_matching_rule {
                    additions.push(Rule {
                        mode,
                        pattern: pattern.into(),
                        target: group.id,
                        target_tab: Some(tab.id),
                        enabled: true,
                    });
                }
            }
        }
        let available = MAX_RULES.saturating_sub(self.rules.len());
        let count = additions.len().min(available);
        self.rules.extend(additions.into_iter().take(available));
        count
    }
    /// The topmost enabled matching rule wins, exactly in the order shown in
    /// Settings (use Up to raise a rule). A rule whose box or tab no longer
    /// exists is skipped so the next matching rule can still apply.
    pub fn best_rule_destination(&self, item: &Item) -> Option<(u64, u64)> {
        self.rules
            .iter()
            .take(MAX_RULES)
            .filter(|rule| rule.matches(item))
            .find_map(|rule| self.rule_destination(rule))
    }
    /// Resolve a rule to a real regular tab (not merely the active box tab).
    pub fn rule_destination(&self, rule: &Rule) -> Option<(u64, u64)> {
        let group = self.group(rule.target)?;
        let tab = match rule.target_tab {
            Some(id) => group
                .tabs
                .iter()
                .find(|t| t.id == id && t.folder.is_none())?,
            None => group.tabs.iter().find(|t| t.folder.is_none())?,
        };
        Some((group.id, tab.id))
    }
    pub fn apply_rules(&mut self) -> usize {
        let rules: Vec<_> = self
            .rules
            .iter()
            .take(MAX_RULES)
            .filter_map(|r| self.rule_destination(r).map(|dest| (r.clone(), dest)))
            .collect();
        let mut transfers = vec![];
        for group in &mut self.groups {
            for tab in &mut group.tabs {
                if tab.folder.is_none() {
                    tab.items.retain(|item| {
                        if let Some((dest_group, dest_tab)) =
                            best_destination_from_rules(item, &rules)
                        {
                            if dest_group != group.id || dest_tab != tab.id {
                                transfers.push((dest_group, dest_tab, item.clone()));
                                return false;
                            }
                        }
                        true
                    });
                }
            }
        }
        let count = transfers.len();
        for (gid, tid, item) in transfers {
            if let Some(group) = self.group_mut(gid) {
                if let Some(tab) = group
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == tid && t.folder.is_none())
                {
                    tab.items.push(item);
                }
            }
        }
        count
    }
    pub fn snapshot(&mut self, name: &str) {
        self.snapshots.push(Snapshot {
            name: name.into(),
            groups: self.groups.clone(),
            pages: self.pages.clone(),
            page_birth: self.page_birth.clone(),
            time: now_seconds(),
        });
    }
    pub fn restore(&mut self, i: usize) -> bool {
        if let Some(s) = self.snapshots.get(i).cloned() {
            self.groups = s.groups;
            self.pages = s.pages;
            self.page_birth = s.page_birth;
            self.sync_page_birth();
            self.page = self.page.min(self.pages.len() - 1);
            true
        } else {
            false
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let temp = path.with_extension("tmp");
        fs::write(&temp, text).map_err(|e| e.to_string())?;
        fs::rename(temp, path).map_err(|e| e.to_string())
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read(path).map_err(|e| e.to_string())?;
        let mut state: Self = serde_json::from_slice(&text).map_err(|e| e.to_string())?;
        state.sync_page_birth();
        if state.version != 1
            || state.pages.is_empty()
            || state.page >= state.pages.len()
            || state.groups.iter().any(|g| {
                g.tabs.is_empty()
                    || g.active >= g.tabs.len()
                    || g.page >= state.pages.len()
                    || !g.bounds.w.is_finite()
                    || !g.bounds.x.is_finite()
                    || !g.bounds.y.is_finite()
                    || !g.bounds.h.is_finite()
            })
        {
            return Err("The saved layout is invalid; the original has been kept.".into());
        }
        Ok(state)
    }
    /// Desktop mode's first start: no groups yet (the first-run prompt offers to
    /// organize the desktop).
    pub fn empty() -> Self {
        Self {
            version: 1,
            groups: vec![],
            pages: vec!["Desktop".into()],
            page_birth: vec![0],
            next_page_birth: 1,
            page: 0,
            next_id: 1,
            rules: vec![],
            snapshots: vec![],
            blend: false,
            tint: false,
            snap: true,
            quick_hide: true,
            auto_rules: false,
            wallpaper: 0,
            layouts: Default::default(),
            placements: Default::default(),
            explorer_icons_hidden: false,
            prefs: Default::default(),
        }
    }
    /// Take an item (by path) out of every regular group.
    pub fn unassign(&mut self, path: &str) {
        for g in &mut self.groups {
            for tab in &mut g.tabs {
                if tab.folder.is_none() {
                    tab.items.retain(|i| !i.path.eq_ignore_ascii_case(path));
                }
            }
        }
    }
    pub fn demo(root: &Path) -> Self {
        let mut s = Self {
            version: 1,
            groups: vec![],
            pages: vec!["Everyday".into(), "Focus".into()],
            page_birth: vec![0, 1],
            next_page_birth: 2,
            page: 0,
            next_id: 1,
            rules: vec![],
            snapshots: vec![],
            blend: false,
            tint: false,
            snap: true,
            quick_hide: true,
            auto_rules: false,
            wallpaper: 0,
            layouts: Default::default(),
            placements: Default::default(),
            explorer_icons_hidden: false,
            prefs: Default::default(),
        };
        let projects = s.add_group(
            "Projects",
            Bounds {
                x: 52.,
                y: 125.,
                w: 376.,
                h: 294.,
            },
        );
        let daily = s.add_group(
            "Quick access",
            Bounds {
                x: 456.,
                y: 125.,
                w: 332.,
                h: 294.,
            },
        );
        let _ = s.add_portal(
            &root.join("demo/Library"),
            Bounds {
                x: 816.,
                y: 125.,
                w: 410.,
                h: 410.,
            },
        );
        let notes = s.add_group(
            "Notes & ideas",
            Bounds {
                x: 52.,
                y: 451.,
                w: 376.,
                h: 220.,
            },
        );
        for (target, paths) in [
            (
                projects,
                vec![
                    "demo/Projects/Studio",
                    "demo/Projects/Website",
                    "demo/Projects/Research",
                    "demo/Projects/Project brief.txt",
                    "demo/Projects/Timeline.csv",
                    "demo/Projects/Checklist.txt",
                ],
            ),
            (
                daily,
                vec![
                    "demo/Library",
                    "demo/Projects",
                    "demo/Notes",
                    "demo/Welcome.html",
                ],
            ),
            (
                notes,
                vec![
                    "demo/Notes/Ideas.txt",
                    "demo/Notes/Reading list.txt",
                    "demo/Notes/This week.txt",
                ],
            ),
        ] {
            for path in paths {
                let _ = s.import(&root.join(path), target);
            }
        }
        for (i, g) in s.groups.iter_mut().enumerate() {
            g.color = [0, 1, 2, 3][i];
        }
        s.snapshot("Starting layout");
        s
    }
}
pub fn create_demo(root: &Path) -> std::io::Result<()> {
    for folder in [
        "Projects/Studio",
        "Projects/Website",
        "Projects/Research",
        "Notes",
        "Library/Design",
        "Library/Documents",
        "Library/Photos",
    ] {
        fs::create_dir_all(root.join("demo").join(folder))?;
    }
    let files=[("Projects/Project brief.txt","PROJECT BRIEF\n\nA clear place for the things you are working on.\n"),("Projects/Timeline.csv","Task,Status\nPlan,Complete\nBuild,In progress\nReview,Next\n"),("Projects/Checklist.txt","[x] Create a workspace\n[ ] Connect your own folders\n[ ] Save a layout\n"),("Notes/Ideas.txt","IDEAS\n\nStart with one small thing.\n"),("Notes/Reading list.txt","READING LIST\n\nAdd a few things you want to explore.\n"),("Notes/This week.txt","THIS WEEK\n\nMake room for focused work.\n"),("Library/Documents/Notes.txt","Your folder view can browse subfolders.\n"),("Library/Welcome.txt","This is a real sample folder. Choose Folder view to connect your own.\n"),("Library/Resources.txt","Keep your most useful files close by.\n"),("Library/Plan.csv","Day,Focus\nMonday,Planning\nTuesday,Creating\n"),("Welcome.html","<!doctype html><meta charset='utf-8'><title>Welcome</title><body style='font:20px system-ui;margin:10%;background:#eff4f8;color:#203545'><h1>Your space, organized.</h1><p>These are real sample files. Add your files or connect a folder to make this workspace yours.</p><p>Drag a header to move a group. Double-click a header to roll it up. Draw a rectangle on empty space to create a group.</p></body>")];
    for (name, contents) in files {
        let path = root.join("demo").join(name);
        if !path.exists() {
            fs::write(path, contents)?;
        }
    }
    Ok(())
}
/// Where the window-mode sample workspace lives: the project folder during
/// development and checks, otherwise the user's own data folder (the installed
/// program never depends on a development path).
pub fn app_root() -> PathBuf {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if dev.join("Cargo.toml").exists() {
        dev
    } else {
        crate::host::data_dir().join("sample")
    }
}
