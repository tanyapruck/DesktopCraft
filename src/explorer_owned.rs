//! Explorer-owned icons — Phase 0 recovery and Phase 1 one-icon laboratory
//! (app/docs/EXPLORER_OWNED_ICONS_ARCHITECTURE_PLAN_20261116.md).
//!
//! Both are explicit command-line modes; the normal DesktopCraft UI never
//! calls them. Neither injects into Explorer, hooks it, or touches files.
//!
//! * `--recover-desktop [positions.json]`: turn Explorer's desktop icons back
//!   on if they are hidden and, only when a snapshot file is given, put the
//!   listed icons back where the snapshot says.
//! * `--explorer-owned-lab <desktop item path> [seconds] [report.json]`:
//!   draw ONE DesktopCraft-style box frame (header + border) around ONE real
//!   Explorer desktop icon. The frame's window region has a hole exactly over
//!   the icon cell, so Explorer's own list view keeps every click, the
//!   selection, F2, drag and its Windows 11 context menu. Dragging the header
//!   moves the frame and asks Explorer (IFolderView) to move the icon into
//!   the hole. On exit the icon goes back to where it started.

use serde_json::{json, Value};

/// Print one JSON line. This is a windowed program: started without an
/// output channel, `println!` would panic, so write errors are ignored.
pub fn emit(value: &Value) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

/// Icons whose position differs from a snapshot, as (path, snapshot x/y).
/// The snapshot is DesktopCraft's `--probe` format: `icon_positions` rows.
pub fn positions_to_restore(snapshot: &Value, current: &[(String, (i32, i32))]) -> Vec<(String, (i32, i32))> {
    let Some(rows) = snapshot["icon_positions"].as_array() else { return vec![] };
    rows.iter()
        .filter_map(|r| Some((r["path"].as_str()?.to_string(), (r["x"].as_i64()? as i32, r["y"].as_i64()? as i32))))
        .filter(|(path, pos)| {
            current.iter().any(|(p, now)| p.eq_ignore_ascii_case(path) && now != pos)
        })
        .collect()
}

/// The lab frame around an icon cell (screen pixels): the window rectangle
/// and the hole, both as [x, y, w, h].
pub fn frame_around(icon: (i32, i32), cell: (i32, i32), margin: i32, header: i32) -> ([i32; 4], [i32; 4]) {
    let hole = [icon.0, icon.1, cell.0, cell.1];
    let outer = [icon.0 - margin, icon.1 - margin - header, cell.0 + 2 * margin, cell.1 + 2 * margin + header];
    (outer, hole)
}

#[cfg(windows)]
pub use win::{recover_desktop, run_lab};

#[cfg(not(windows))]
pub fn recover_desktop(_: Option<&std::path::Path>) -> Value { json!({"available": false, "reason": "Windows only"}) }
#[cfg(not(windows))]
pub fn run_lab(_: &str, _: u32, _: Option<(i32, i32)>, _: Option<(i32, i32, i32, i32)>) -> Value { json!({"available": false, "reason": "Windows only"}) }

#[cfg(windows)]
mod win {
    use super::*;
    use crate::shell::DesktopView;
    use std::cell::RefCell;
    use std::ptr;
    use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
    use winapi::shared::windef::{HWND, POINT, RECT};
    use winapi::um::libloaderapi::GetModuleHandleW;
    use winapi::um::wingdi::*;
    use winapi::um::winuser::*;

    fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() }

    pub fn recover_desktop(snapshot: Option<&std::path::Path>) -> Value {
        let Some(view) = DesktopView::acquire() else {
            return json!({"recovered": false, "reason": "Explorer desktop view unavailable (is Explorer running?)"});
        };
        let before = view.icons_visible();
        let shown = if before == Some(false) { view.set_icons_visible(true) } else { false };
        let mut restored = 0usize;
        let mut asked = 0usize;
        if let Some(path) = snapshot {
            match std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
                Some(snap) => {
                    let current: Vec<_> = view.items().into_iter().map(|i| (i.path, i.pos)).collect();
                    let moves = positions_to_restore(&snap, &current);
                    asked = moves.len();
                    restored = view.position(&moves);
                }
                None => return json!({"recovered": false, "reason": "snapshot file unreadable"}),
            }
        }
        json!({
            "recovered": true,
            "icons_visible_before": before,
            "icons_turned_on": shown,
            "icons_visible_after": view.icons_visible(),
            "positions_restore_requested": snapshot.is_some(),
            "positions_differing": asked,
            "positions_restored": restored,
            "files_touched": 0,
        })
    }

    const MARGIN: i32 = 10;
    const HEADER: i32 = 30;
    struct Lab {
        view: DesktopView,
        target: String,
        hole: [i32; 4],
        moves: usize,
        aligns: usize,
        errors: Vec<String>,
    }
    thread_local! {
        static LAB: RefCell<Option<Lab>> = const { RefCell::new(None) };
        static OFFSET: std::cell::Cell<(i32, i32)> = const { std::cell::Cell::new((0, 0)) };
    }

    fn cell_size(view: &DesktopView) -> (i32, i32) {
        unsafe {
            let mut result = 0usize;
            let ok = SendMessageTimeoutW(view.list_window(), 0x1033, 0, 0, SMTO_ABORTIFHUNG, 300, &mut result);
            let (cx, cy) = ((result & 0xffff) as i32, ((result >> 16) & 0xffff) as i32);
            if ok != 0 && cx > 0 && cy > 0 { (cx, cy) } else { view.spacing().unwrap_or((96, 96)) }
        }
    }

    fn set_hole_region(hwnd: HWND, outer: [i32; 4], hole: [i32; 4]) {
        unsafe {
            let all = CreateRectRgn(0, 0, outer[2], outer[3]);
            let (hx, hy) = (hole[0] - outer[0], hole[1] - outer[1]);
            let gap = CreateRectRgn(hx, hy, hx + hole[2], hy + hole[3]);
            CombineRgn(all, all, gap, RGN_DIFF);
            DeleteObject(gap as _);
            SetWindowRgn(hwnd, all, 1);
        }
    }

    unsafe extern "system" fn proc(hwnd: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_NCHITTEST => {
                let mut pt = POINT { x: (lp & 0xffff) as i16 as i32, y: ((lp >> 16) & 0xffff) as i16 as i32 };
                ScreenToClient(hwnd, &mut pt);
                // Header drags the box; the border absorbs clicks. The hole
                // is not part of the window at all, so Explorer gets it.
                if pt.y < HEADER { HTCAPTION as LRESULT } else { HTCLIENT as LRESULT }
            }
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            // Re-align the frame so its hole sits on the rectangle where
            // Explorer actually drew the icon (Explorer may snap icons to its
            // grid). wParam/lParam = hole x/y in screen pixels. No icon move.
            WM_APP => {
                let (x, y) = (wp as i32, lp as i32);
                SetWindowPos(hwnd, ptr::null_mut(), x - MARGIN, y - MARGIN - HEADER, 0, 0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                LAB.with(|lab| if let Some(lab) = lab.borrow_mut().as_mut() { lab.hole[0] = x; lab.hole[1] = y; lab.aligns += 1; });
                0
            }
            WM_EXITSIZEMOVE => {
                let mut r: RECT = std::mem::zeroed();
                GetWindowRect(hwnd, &mut r);
                LAB.with(|lab| {
                    if let Some(lab) = lab.borrow_mut().as_mut() {
                        let hole = [r.left + MARGIN, r.top + MARGIN + HEADER, lab.hole[2], lab.hole[3]];
                        let (dx, dy) = OFFSET.with(|o| o.get());
                        if lab.view.position(&[(lab.target.clone(), (hole[0] - dx, hole[1] - dy))]) == 1 {
                            lab.moves += 1;
                        } else {
                            let (hr, _, matched) = DesktopView::last_position_result();
                            lab.errors.push(format!("position failed hr=0x{:08X} matched={matched}", hr as u32));
                        }
                        lab.hole = hole;
                    }
                });
                0
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut r: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut r);
                // DesktopCraft group colours (dark translucent panel look, opaque here).
                let body = CreateSolidBrush(RGB(0x24, 0x30, 0x40));
                FillRect(dc, &r, body);
                DeleteObject(body as _);
                let head = RECT { left: 0, top: 0, right: r.right, bottom: HEADER };
                let hb = CreateSolidBrush(RGB(0x3A, 0x4A, 0x60));
                FillRect(dc, &head, hb);
                DeleteObject(hb as _);
                SetBkMode(dc, TRANSPARENT as i32);
                SetTextColor(dc, RGB(0xF0, 0xF4, 0xF8));
                let mut text = wide("DesktopCraft box");
                let mut tr = head;
                DrawTextW(dc, text.as_mut_ptr(), -1, &mut tr, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                EndPaint(hwnd, &ps);
                0
            }
            WM_TIMER => { DestroyWindow(hwnd); 0 }
            WM_CLOSE => { DestroyWindow(hwnd); 0 }
            WM_DESTROY => { PostQuitMessage(0); 0 }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    /// `hole_offset` = (dx, dy, w, h): the icon's drawn rectangle relative to
    /// the position IFolderView reports, measured by the caller through the
    /// accessibility interface (Explorer's position is not the item's corner).
    pub fn run_lab(target: &str, seconds: u32, place: Option<(i32, i32)>, hole_offset: Option<(i32, i32, i32, i32)>) -> Value {
        let Some(view) = DesktopView::acquire() else {
            return json!({"started": false, "reason": "Explorer desktop view unavailable"});
        };
        if view.icons_visible() != Some(true) {
            return json!({"started": false, "reason": "Explorer desktop icons are hidden; run --recover-desktop or close the program hiding them (DesktopCraft)"});
        }
        let Some(start) = view.position_of(target) else {
            return json!({"started": false, "reason": "the path is not an icon in Explorer's desktop view"});
        };
        // Optionally move the icon to a free spot first (put back on exit).
        let mut moves = 0;
        let here = match place {
            Some(p) if view.position(&[(target.to_string(), p)]) == 1 => { moves = 1; view.position_of(target).unwrap_or(p) }
            _ => start,
        };
        let (off, cell) = match hole_offset {
            Some((dx, dy, w, h)) => ((dx, dy), (w, h)),
            None => ((0, 0), cell_size(&view)),
        };
        let (outer, hole) = frame_around((here.0 + off.0, here.1 + off.1), cell, MARGIN, HEADER);
        OFFSET.with(|o| o.set(off));
        let list = view.list_window();
        let mut explorer_pid = 0u32;
        unsafe { GetWindowThreadProcessId(list, &mut explorer_pid) };
        LAB.with(|l| *l.borrow_mut() = Some(Lab { view, target: target.to_string(), hole, moves, aligns: 0, errors: vec![] }));
        let hwnd = unsafe {
            let class = wide("DesktopCraftExplorerOwnedLab");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(proc),
                hInstance: GetModuleHandleW(ptr::null()),
                hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassExW(&wc);
            // Owned by the desktop window (Progman) like DesktopCraft's host:
            // it stays just above the desktop and below ordinary windows.
            let owner = GetShellWindow();
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(), wide("DesktopCraft explorer-owned lab").as_ptr(),
                WS_POPUP, outer[0], outer[1], outer[2], outer[3],
                owner, ptr::null_mut(), wc.hInstance, ptr::null_mut());
            if !hwnd.is_null() {
                set_hole_region(hwnd, outer, hole);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                SetTimer(hwnd, 1, seconds.clamp(5, 3600) * 1000, None);
            }
            hwnd
        };
        if hwnd.is_null() {
            return json!({"started": false, "reason": "could not create the frame window"});
        }
        // Tell a test script where everything is, then run until closed.
        emit(&json!({"started": true, "frame_hwnd": format!("0x{:X}", hwnd as usize),
            "list_hwnd": format!("0x{:X}", list as usize), "explorer_pid": explorer_pid,
            "icon_start": [start.0, start.1], "icon": [here.0, here.1], "cell": [cell.0, cell.1], "frame": outer, "hole": hole}));
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        // Roll back: the icon returns to where it started.
        LAB.with(|l| {
            let lab = l.borrow_mut().take().unwrap();
            let moved_back = lab.moves == 0 || lab.view.position(&[(lab.target.clone(), start)]) == 1;
            let end = lab.view.position_of(&lab.target);
            json!({"started": true, "target_found": true, "explorer_pid": explorer_pid,
                "list_hwnd": format!("0x{:X}", list as usize), "icon_start": [start.0, start.1],
                "icon_end": end.map(|p| [p.0, p.1]), "restored_to_start": moved_back && end == Some(start),
                "frame_moves": lab.moves, "frame_realigned": lab.aligns, "errors": lab.errors, "files_touched": 0,
                "explorer_injection": false, "hooks": false})
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_has_a_hole_exactly_over_the_icon_cell() {
        let (outer, hole) = frame_around((100, 200), (96, 96), 10, 30);
        assert_eq!(hole, [100, 200, 96, 96]);
        assert_eq!(outer, [90, 160, 116, 146]);
        // The hole sits fully inside the frame, below the header.
        assert!(hole[0] > outer[0] && hole[1] >= outer[1] + 30 + 10);
        assert!(hole[0] + hole[2] < outer[0] + outer[2] && hole[1] + hole[3] < outer[1] + outer[3]);
    }
    #[test]
    fn recovery_restores_only_icons_that_moved() {
        let snap = json!({"icon_positions": [
            {"path": "C:\\D\\a.lnk", "x": 10, "y": 20},
            {"path": "C:\\D\\b.lnk", "x": 30, "y": 40},
            {"path": "C:\\D\\gone.lnk", "x": 1, "y": 1}]});
        let now = vec![("c:\\d\\A.lnk".to_string(), (10, 20)), ("C:\\D\\b.lnk".to_string(), (99, 40))];
        assert_eq!(positions_to_restore(&snap, &now), vec![("C:\\D\\b.lnk".to_string(), (30, 40))]);
        assert!(positions_to_restore(&json!({}), &now).is_empty());
    }
}
