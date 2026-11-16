//! Desktop host: the window that carries groups on the real Windows desktop,
//! monitors, single instance and commands, start-at-login and per-user install.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "DesktopCraft";
const LEGACY_APP_NAME: &str = "Desktop Spaces";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\DesktopCraft";
const LEGACY_UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\DesktopSpaces";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VERB_NEW: &str = r"Software\Classes\DesktopBackground\Shell\DesktopCraftNewGroup";
const LEGACY_VERB_NEW: &str = r"Software\Classes\DesktopBackground\Shell\DesktopSpacesNewGroup";
const VERB_SETTINGS: &str = r"Software\Classes\DesktopBackground\Shell\DesktopCraftSettings";
/// The classic Explorer file menu (Windows 11: Show more options).
const VERB_PIN_FILE: &str = r"Software\Classes\*\shell\DesktopCraftPinFile";
const LEGACY_VERB_SETTINGS: &str =
    r"Software\Classes\DesktopBackground\Shell\DesktopSpacesSettings";

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// Stable device name, e.g. \\.\DISPLAY1.
    pub key: String,
    /// Physical pixels in virtual-screen coordinates: x, y, w, h.
    pub rect: [i32; 4],
    pub work: [i32; 4],
    pub dpi: u32,
    pub primary: bool,
}

/// Diagnostic log for the active workspace. Keep just 3 bounded files,
/// serialize writes, and never rely on the log to recover user data.
/// Only pass redacted metadata to this function, not file paths/names.
pub fn log(message: &str) {
    write_log_at(&data_dir(), message);
}

fn write_log_at(dir: &Path, message: &str) {
    use std::io::Write;
    static LOG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let Ok(_guard) = LOG_LOCK.lock() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = dir.join("desktopcraft.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() >= 512 * 1024) {
        let one = dir.join("desktopcraft.1.log");
        let two = dir.join("desktopcraft.2.log");
        let _ = std::fs::remove_file(&two);
        let _ = std::fs::rename(&one, &two);
        let _ = std::fs::rename(&path, &one);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let safe: String = message
            .chars()
            .filter(|c| !c.is_control() || *c == '\t')
            .take(240)
            .collect();
        let _ = writeln!(f, "[{secs}] {safe}");
    }
}

/// Stable diagnostic code plus *non-personal* metadata; avoid file paths,
/// file names, rule text and full Windows error messages in a shared report.
pub fn event(level: &str, code: &str, detail: &str) {
    log(&format!("{level} {code}: {detail}"));
}
pub fn log_dir() -> PathBuf {
    data_dir()
}

#[cfg(test)]
mod diagnostic_log_tests {
    use super::*;
    #[test]
    fn diagnostic_logs_rotate_and_keep_recent_events_bounded() {
        let dir =
            std::env::temp_dir().join(format!("desktopcraft-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let active = dir.join("desktopcraft.log");
        std::fs::write(&active, vec![b'x'; 512 * 1024]).unwrap();
        write_log_at(&dir, "INFO session.started: count=1");
        assert!(dir.join("desktopcraft.1.log").exists());
        assert!(!dir.join("desktopcraft.2.log").exists());
        assert!(std::fs::read_to_string(&active)
            .unwrap()
            .contains("session.started"));
        std::fs::write(&active, vec![b'y'; 512 * 1024]).unwrap();
        write_log_at(
            &dir,
            &format!("WARN ui.operation_problem: {}", "z".repeat(800)),
        );
        assert!(dir.join("desktopcraft.2.log").exists());
        assert!(std::fs::metadata(&active).unwrap().len() < 400);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Keep existing users' workspace untouched across the product rename.
/// A new installation uses %APPDATA%\DesktopCraft. Existing Desktop Spaces
/// workspace data remains where it is; no automatic moves or file deletion.
fn choose_workspace_dir(appdata: &Path) -> PathBuf {
    let current = appdata.join(APP_NAME);
    let legacy = appdata.join(LEGACY_APP_NAME);
    if current.join("data/workspace.json").is_file() {
        current
    } else if legacy.join("data/workspace.json").is_file() {
        legacy
    } else {
        current
    }
}
pub fn data_dir() -> PathBuf {
    if let Some(dir) =
        std::env::var_os("DESKTOPCRAFT_DATA").or_else(|| std::env::var_os("DESKTOP_SPACES_DATA"))
    {
        return PathBuf::from(dir);
    }
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    choose_workspace_dir(&appdata)
}
pub fn install_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Programs")
        .join(APP_NAME)
}
pub fn commands_dir() -> PathBuf {
    data_dir().join("commands")
}

/// A stable name for the current monitor arrangement (used to keep one layout
/// per arrangement and switch automatically when monitors change).
pub fn signature(monitors: &[Monitor]) -> String {
    let mut parts: Vec<String> = monitors
        .iter()
        .map(|m| {
            format!(
                "{}@{}x{}+{}+{}",
                m.key, m.rect[2], m.rect[3], m.rect[0], m.rect[1]
            )
        })
        .collect();
    parts.sort();
    parts.join("|")
}

/// Write a command for the running instance (new-group, settings, exit...).
pub fn send_command(command: &str) -> std::io::Result<()> {
    let dir = commands_dir();
    std::fs::create_dir_all(&dir)?;
    let name = format!(
        "{}-{}.cmd",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );
    std::fs::write(dir.join(name), command)
}
/// Commands waiting for this instance, oldest first (each read once).
pub fn take_commands() -> Vec<String> {
    let Ok(read) = std::fs::read_dir(commands_dir()) else {
        return vec![];
    };
    let mut files: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "cmd"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok();
            let _ = std::fs::remove_file(&p);
            text.map(|t| t.trim().to_string())
        })
        .collect()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::{mem, ptr};
    use winapi::shared::{
        minwindef::{BOOL, DWORD, HKEY, LPARAM, TRUE},
        windef::{HDC, HMONITOR, HWND, LPRECT, POINT},
    };
    use winapi::um::{
        wingdi::*,
        winnt::{KEY_SET_VALUE, REG_DWORD, REG_SZ},
        winreg::*,
        winuser::*,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    #[link(name = "shcore")]
    extern "system" {
        fn GetDpiForMonitor(m: HMONITOR, kind: u32, x: *mut u32, y: *mut u32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            attrs: *mut std::ffi::c_void,
            owner: BOOL,
            name: *const u16,
        ) -> *mut std::ffi::c_void;
        fn GetLastError() -> DWORD;
    }

    unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: LPRECT, out: LPARAM) -> BOOL {
        let list = &mut *(out as *mut Vec<Monitor>);
        let mut info: MONITORINFOEXW = mem::zeroed();
        info.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(m, &mut info as *mut _ as *mut MONITORINFO) != 0 {
            let r = info.rcMonitor;
            let w = info.rcWork;
            let (mut dx, mut dy) = (96u32, 96u32);
            GetDpiForMonitor(m, 0, &mut dx, &mut dy);
            let name = String::from_utf16_lossy(
                &info.szDevice[..info.szDevice.iter().position(|&c| c == 0).unwrap_or(0)],
            );
            list.push(Monitor {
                key: name,
                rect: [r.left, r.top, r.right - r.left, r.bottom - r.top],
                work: [w.left, w.top, w.right - w.left, w.bottom - w.top],
                dpi: dx.max(48),
                primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
        TRUE
    }
    pub fn monitors() -> Vec<Monitor> {
        let mut list: Vec<Monitor> = vec![];
        unsafe {
            EnumDisplayMonitors(
                ptr::null_mut(),
                ptr::null(),
                Some(collect),
                &mut list as *mut _ as LPARAM,
            );
        }
        list
    }
    pub fn virtual_rect() -> [i32; 4] {
        unsafe {
            [
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            ]
        }
    }

    unsafe extern "system" fn find(hwnd: HWND, out: LPARAM) -> BOOL {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let mut class = [0u16; 32];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), 32);
        if pid == std::process::id()
            && String::from_utf16_lossy(&class[..n as usize]) == "MINIQUADAPP"
        {
            *(out as *mut HWND) = hwnd;
            0
        } else {
            1
        }
    }
    pub fn window() -> HWND {
        unsafe {
            let mut hwnd: HWND = ptr::null_mut();
            EnumWindows(Some(find), &mut hwnd as *mut _ as LPARAM);
            hwnd
        }
    }
    /// Keep the window covering every monitor.
    pub fn place(rect: [i32; 4]) {
        unsafe {
            let hwnd = window();
            if !hwnd.is_null() {
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    rect[0],
                    rect[1],
                    rect[2],
                    rect[3],
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
    /// Clip the window to the groups (client physical pixels). Everything else is
    /// the real desktop: Explorer keeps its icons, wallpaper, menus and drops.
    /// Each entry: x, y, w, h, corner diameter (client physical pixels).
    pub fn set_region(rects: &[[i32; 5]]) {
        unsafe {
            let hwnd = window();
            if hwnd.is_null() {
                return;
            }
            if color_pick_shielded() && !rects.is_empty() {
                return;
            }
            let region = CreateRectRgn(0, 0, 0, 0);
            for r in rects {
                let part = if r[4] > 0 {
                    CreateRoundRectRgn(r[0], r[1], r[0] + r[2] + 1, r[1] + r[3] + 1, r[4], r[4])
                } else {
                    CreateRectRgn(r[0], r[1], r[0] + r[2], r[1] + r[3])
                };
                CombineRgn(region, region, part, RGN_OR);
                DeleteObject(part as _);
            }
            // The system owns the region after this call.
            SetWindowRgn(hwnd, region, TRUE);
        }
    }
    pub fn settings_has_focus() -> bool {
        unsafe {
            let hwnd = window();
            !hwnd.is_null() && GetForegroundWindow() == hwnd
        }
    }
    /// Minimizing just the floating Settings panel should leave desktop
    /// groups usable. Return foreground focus to the Windows shell.
    pub fn settings_deactivate() {
        unsafe {
            let hwnd = window();
            if hwnd.is_null() {
                return;
            }
            SetWindowPos(
                hwnd,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            let shell = GetShellWindow();
            if !shell.is_null() {
                SetForegroundWindow(shell);
            }
        }
    }
    /// Temporarily expose Settings in the Windows taskbar and Alt+Tab.
    /// The window itself stays the same clipped desktop host; no second
    /// process, window or desktop manager instance is launched.
    pub fn settings_taskbar(visible: bool) {
        unsafe {
            let hwnd = window();
            if hwnd.is_null() {
                return;
            }
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let desired = if visible {
                (style | WS_EX_APPWINDOW as isize) & !(WS_EX_TOOLWINDOW as isize)
            } else {
                (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize)
            };
            if desired == style {
                return;
            }
            // Windows creates/removes the taskbar button after a visibility
            // transition. Keep the current region and restore Z-order later.
            ShowWindow(hwnd, SW_HIDE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired);
            let title = if visible {
                "DesktopCraft Settings"
            } else {
                "DesktopCraft"
            };
            SetWindowTextW(hwnd, wide(title).as_ptr());
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(
                hwnd,
                ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
    }
    /// Front: above other windows (dialogs, menus, Peek). Otherwise the desktop layer.
    pub fn set_front(front: bool) {
        macroquad::miniquad::DESKTOP_HOST_FRONT.store(front, std::sync::atomic::Ordering::Relaxed);
        unsafe {
            let hwnd = window();
            if hwnd.is_null() {
                return;
            }
            if front {
                // Peek must be ABOVE ordinary foreground apps while held;
                // HWND_TOP only raises within the non-topmost tier.
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            } else {
                // Important: remove TOPMOST as soon as the chord is released,
                // then return the clipped host to Explorer's desktop layer.
                SetWindowPos(
                    hwnd,
                    HWND_NOTOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                SetWindowPos(
                    hwnd,
                    HWND_BOTTOM,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
    /// After "Show desktop" (Win+D) or a minimize, show the groups again.
    pub fn keep_shown() {
        if color_pick_shielded() {
            return;
        }
        unsafe {
            let hwnd = window();
            if !hwnd.is_null() && (IsIconic(hwnd) != 0 || IsWindowVisible(hwnd) == 0) {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                SetWindowPos(
                    hwnd,
                    HWND_BOTTOM,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
    /// Read the physical mouse button while moving across region-clipped boxes.
    pub fn mouse_left_down() -> bool {
        unsafe { GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000 != 0 }
    }
    /// The DesktopCraft host is clipped to icon shapes by SetWindowRgn.
    /// Capture the pointer while dragging a loose icon so MouseMove/MouseUp
    /// still reaches us over blank wallpaper, outside the initial icon region.
    pub fn capture_pointer(enable: bool) {
        unsafe {
            let hwnd = window();
            if hwnd.is_null() {
                return;
            }
            if enable {
                SetCapture(hwnd);
            } else if GetCapture() == hwnd {
                ReleaseCapture();
            }
        }
    }
    pub fn cursor() -> (i32, i32) {
        unsafe {
            let mut p = POINT { x: 0, y: 0 };
            GetCursorPos(&mut p);
            (p.x, p.y)
        }
    }
    /// True only for Explorer's actual desktop. Explorer folder windows can
    /// also contain SysListView32/SHELLDLL_DefView, but their root is CabinetWClass.
    pub fn over_explorer_desktop(x: i32, y: i32) -> bool {
        unsafe {
            let mut hwnd = WindowFromPoint(POINT { x, y });
            if hwnd.is_null() {
                return false;
            }
            let mut hit_pid = 0;
            GetWindowThreadProcessId(hwnd, &mut hit_pid);
            if hit_pid == std::process::id() {
                return false;
            }
            while !GetParent(hwnd).is_null() {
                hwnd = GetParent(hwnd);
            }
            let mut class = [0u16; 64];
            let n = GetClassNameW(hwnd, class.as_mut_ptr(), 64);
            matches!(
                String::from_utf16_lossy(&class[..n as usize]).as_str(),
                "Progman" | "WorkerW"
            )
        }
    }
    /// Whether a drag is over a different application (not a group or the real
    /// desktop), so an external Windows OLE drag is actually needed.
    pub fn over_other_app(x: i32, y: i32) -> bool {
        if over_explorer_desktop(x, y) {
            return false;
        }
        unsafe {
            let hwnd = WindowFromPoint(POINT { x, y });
            if hwnd.is_null() {
                return false;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            pid != 0 && pid != std::process::id()
        }
    }
    // Keep the original cross-version mutex name so Desktop Spaces and
    // DesktopCraft never control the Windows desktop simultaneously.
    /// Whether a copy is running (without claiming the instance for this process).
    pub fn instance_running() -> bool {
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenMutexW(access: u32, inherit: BOOL, name: *const u16) -> *mut std::ffi::c_void;
            fn CloseHandle(h: *mut std::ffi::c_void) -> BOOL;
        }
        unsafe {
            let h = OpenMutexW(0x0010_0000, 0, wide("Local\\DesktopSpacesHost").as_ptr());
            if h.is_null() {
                false
            } else {
                CloseHandle(h);
                true
            }
        }
    }
    /// True for the first instance; later launches pass their command on and exit.
    pub fn first_instance() -> bool {
        unsafe {
            let handle = CreateMutexW(
                ptr::null_mut(),
                0,
                wide("Local\\DesktopSpacesHost").as_ptr(),
            );
            !handle.is_null() && GetLastError() != 183 // ERROR_ALREADY_EXISTS
        }
    }
    // ---- quick-hide: double-click on blank desktop (low-level mouse hook) ----
    // A WH_MOUSE_LL hook runs in this process (nothing is injected into Explorer)
    // and only watches for a double-click on Explorer's desktop away from icons
    // and groups. It never blocks or changes the click.
    static HOOK_RECTS: std::sync::Mutex<Vec<[i32; 4]>> = std::sync::Mutex::new(Vec::new());
    static QUICK_HIDE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static LAST_CLICK: std::sync::Mutex<(u32, i32, i32)> = std::sync::Mutex::new((0, 0, 0));
    // Screen-wide color picker: the hook captures exactly one user click;
    // no screenshot or pixel content is written to disk or sent elsewhere.
    static COLOR_PICK_ACTIVE: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static COLOR_PICK_SHIELDED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static COLOR_PICK_LEFT_UP: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static COLOR_PICK_RIGHT_UP: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static COLOR_PICK_RESULT: std::sync::Mutex<Option<Option<(i32, i32)>>> =
        std::sync::Mutex::new(None);

    pub fn begin_color_pick() {
        if let Ok(mut result) = COLOR_PICK_RESULT.lock() {
            *result = None;
        }
        COLOR_PICK_SHIELDED.store(true, std::sync::atomic::Ordering::SeqCst);
        COLOR_PICK_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
        // Make the entire DesktopCraft window click-through temporarily,
        // revealing pixels of other apps, browsers and photographs beneath it.
        set_region(&[]);
    }
    pub fn color_pick_shielded() -> bool {
        COLOR_PICK_SHIELDED.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn finish_color_pick() -> Option<Option<[f32; 3]>> {
        let event = COLOR_PICK_RESULT.lock().ok()?.take()?;
        let color = event.and_then(|(x, y)| unsafe {
            let dc = GetDC(ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let pixel = GetPixel(dc, x, y) as u32;
            ReleaseDC(ptr::null_mut(), dc);
            if pixel == 0xFFFF_FFFF {
                None
            } else {
                Some([
                    (pixel & 0xff) as f32 / 255.,
                    ((pixel >> 8) & 0xff) as f32 / 255.,
                    ((pixel >> 16) & 0xff) as f32 / 255.,
                ])
            }
        });
        COLOR_PICK_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
        COLOR_PICK_SHIELDED.store(false, std::sync::atomic::Ordering::SeqCst);
        Some(color)
    }
    /// A non-blocking marquee over the otherwise transparent desktop.
    static LEFT_MARQUEE: std::sync::Mutex<LeftMarquee> = std::sync::Mutex::new(LeftMarquee {
        down: None,
        current: (0, 0),
        released: None,
    });
    struct LeftMarquee {
        down: Option<(i32, i32)>,
        current: (i32, i32),
        released: Option<((i32, i32), (i32, i32))>,
    }
    pub fn marquee_drag() -> Option<((i32, i32), (i32, i32))> {
        LEFT_MARQUEE
            .lock()
            .ok()
            .and_then(|s| s.down.map(|d| (d, s.current)))
    }
    pub fn take_marquee_release() -> Option<((i32, i32), (i32, i32))> {
        LEFT_MARQUEE.lock().ok().and_then(|mut s| s.released.take())
    }
    // ---- right button on empty desktop: DesktopCraft's desktop menu ----
    // While enabled, a right-click (or right-drag) on empty desktop is taken by
    // DesktopCraft: a click shows Windows' own desktop menu with DesktopCraft's
    // items added; a drag draws a rectangle to create a group (as in the
    // original). Only empty desktop is affected; icons, groups and apps aren't.
    static RIGHT_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static RIGHT_STATE: std::sync::Mutex<RightState> = std::sync::Mutex::new(RightState {
        down: None,
        current: (0, 0),
        released: None,
    });
    #[derive(Clone, Copy)]
    pub struct RightState {
        /// Where the right button went down on empty desktop (screen pixels).
        pub down: Option<(i32, i32)>,
        pub current: (i32, i32),
        /// Start and end of a finished press, waiting for the app.
        pub released: Option<((i32, i32), (i32, i32))>,
    }
    pub fn set_right_click_enabled(on: bool) {
        RIGHT_ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
        if !on {
            if let Ok(mut s) = RIGHT_STATE.lock() {
                s.down = None;
            }
        }
    }
    /// The right-button press in progress (start, current), if any.
    pub fn right_drag() -> Option<((i32, i32), (i32, i32))> {
        RIGHT_STATE
            .lock()
            .ok()
            .and_then(|s| s.down.map(|d| (d, s.current)))
    }
    pub fn take_right_release() -> Option<((i32, i32), (i32, i32))> {
        RIGHT_STATE.lock().ok().and_then(|mut s| s.released.take())
    }
    /// Pass an empty-background click back to Explorer so Windows 11 draws
    /// its native modern menu; the low-level hook ignores injected replay.
    pub fn show_native_desktop_menu(x: i32, y: i32) {
        unsafe {
            SetCursorPos(x, y);
            mouse_event(MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, 0);
            mouse_event(MOUSEEVENTF_RIGHTUP, 0, 0, 0, 0);
        }
    }
    /// The latest real mouse-button press anywhere on screen (screen pixels).
    /// DesktopCraft only receives clicks inside its own boxes, so this is how
    /// an open menu learns it should close when Windows is clicked elsewhere.
    static LAST_PRESS: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);
    pub fn take_press() -> Option<(i32, i32)> {
        LAST_PRESS.lock().ok().and_then(|mut p| p.take())
    }
    unsafe extern "system" fn mouse_hook(code: i32, wparam: usize, lparam: isize) -> isize {
        if code >= 0 && (*(lparam as *const MSLLHOOKSTRUCT)).flags & 1 != 0 {
            return CallNextHookEx(ptr::null_mut(), code, wparam, lparam);
        }
        if code >= 0 {
            let event = wparam as u32;
            if event == WM_LBUTTONDOWN || event == WM_RBUTTONDOWN || event == WM_MBUTTONDOWN {
                let pt = (*(lparam as *const MSLLHOOKSTRUCT)).pt;
                if let Ok(mut last) = LAST_PRESS.lock() {
                    *last = Some((pt.x, pt.y));
                }
            }
            if event == WM_LBUTTONUP
                && COLOR_PICK_LEFT_UP.swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return 1;
            }
            if event == WM_RBUTTONUP
                && COLOR_PICK_RIGHT_UP.swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return 1;
            }
            if COLOR_PICK_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
                if event == WM_LBUTTONDOWN || event == WM_RBUTTONDOWN {
                    let pos = (*(lparam as *const MSLLHOOKSTRUCT)).pt;
                    if let Ok(mut result) = COLOR_PICK_RESULT.lock() {
                        *result = Some(if event == WM_LBUTTONDOWN {
                            Some((pos.x, pos.y))
                        } else {
                            None
                        });
                    }
                    if event == WM_LBUTTONDOWN {
                        COLOR_PICK_LEFT_UP.store(true, std::sync::atomic::Ordering::SeqCst);
                    } else {
                        COLOR_PICK_RIGHT_UP.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    COLOR_PICK_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
                    return 1; // Do not click/activate the sampled app window.
                }
            }
        }
        if code >= 0 && RIGHT_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            let info = &*(lparam as *const MSLLHOOKSTRUCT);
            let (x, y) = (info.pt.x, info.pt.y);
            let msg = wparam as u32;
            if msg == WM_RBUTTONDOWN {
                let on_item = HOOK_RECTS
                    .lock()
                    .map(|r| {
                        r.iter()
                            .any(|r| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3])
                    })
                    .unwrap_or(true);
                if !on_item && over_explorer_desktop(x, y) {
                    if let Ok(mut s) = RIGHT_STATE.lock() {
                        s.down = Some((x, y));
                        s.current = (x, y);
                    }
                    return 1; // Explorer doesn't see this press.
                }
            } else if msg == WM_MOUSEMOVE {
                if let Ok(mut s) = RIGHT_STATE.lock() {
                    if s.down.is_some() {
                        s.current = (x, y);
                    }
                }
            } else if msg == WM_RBUTTONUP {
                if let Ok(mut s) = RIGHT_STATE.lock() {
                    if let Some(d) = s.down.take() {
                        s.released = Some((d, (x, y)));
                        return 1;
                    }
                }
            }
        }
        if code >= 0 {
            let info = &*(lparam as *const MSLLHOOKSTRUCT);
            let (x, y) = (info.pt.x, info.pt.y);
            if wparam == WM_MOUSEMOVE as usize {
                if let Ok(mut m) = LEFT_MARQUEE.lock() {
                    if m.down.is_some() {
                        m.current = (x, y);
                    }
                }
            } else if wparam == WM_LBUTTONUP as usize {
                if let Ok(mut m) = LEFT_MARQUEE.lock() {
                    if let Some(start) = m.down.take() {
                        if (start.0 - x).abs() > 10 || (start.1 - y).abs() > 10 {
                            m.released = Some((start, (x, y)));
                            if let Ok(mut last) = LAST_CLICK.lock() {
                                *last = (0, 0, 0);
                            }
                        }
                    }
                }
            }
        }
        if code >= 0 && wparam == WM_LBUTTONDOWN as usize {
            let info = &*(lparam as *const MSLLHOOKSTRUCT);
            let (x, y) = (info.pt.x, info.pt.y);
            let on_item = HOOK_RECTS
                .lock()
                .map(|r| {
                    r.iter()
                        .any(|r| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3])
                })
                .unwrap_or(true);
            let blank_desktop = !on_item && over_explorer_desktop(x, y);
            if let Ok(mut m) = LEFT_MARQUEE.lock() {
                m.down =
                    if blank_desktop && RIGHT_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
                        Some((x, y))
                    } else {
                        None
                    };
                m.current = (x, y);
            }
            if let Ok(mut last) = LAST_CLICK.lock() {
                if blank_desktop {
                    let (t, lx, ly) = *last;
                    let near = (lx - x).abs() <= GetSystemMetrics(SM_CXDOUBLECLK) / 2 + 1
                        && (ly - y).abs() <= GetSystemMetrics(SM_CYDOUBLECLK) / 2 + 1;
                    if t != 0 && info.time.wrapping_sub(t) <= GetDoubleClickTime() && near {
                        QUICK_HIDE.store(true, std::sync::atomic::Ordering::Relaxed);
                        *last = (0, 0, 0);
                    } else {
                        *last = (info.time, x, y);
                    }
                } else {
                    // Do not pair a click on a folder/window/group with one on
                    // the desktop. Only two consecutive blank desktop clicks count.
                    *last = (0, 0, 0);
                }
            }
        }
        CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
    }
    pub fn install_quick_hide_hook() {
        unsafe {
            SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(mouse_hook),
                winapi::um::libloaderapi::GetModuleHandleW(ptr::null()),
                0,
            );
        }
    }
    pub fn set_hook_rects(rects: Vec<[i32; 4]>) {
        if let Ok(mut r) = HOOK_RECTS.lock() {
            *r = rects;
        }
    }
    pub fn take_quick_hide() -> bool {
        QUICK_HIDE.swap(false, std::sync::atomic::Ordering::Relaxed)
    }
    pub fn allow_foreground() {
        unsafe {
            AllowSetForegroundWindow(u32::MAX);
        }
    }
    /// A Yes/No question whose default (Enter) answer is No.
    pub fn ask_default_no(text: &str) -> bool {
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide(text).as_ptr(),
                wide(APP_NAME).as_ptr(),
                MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND,
            ) == IDYES
        }
    }
    pub fn message(text: &str, question: bool) -> bool {
        unsafe {
            let flags = if question {
                MB_YESNO | MB_ICONQUESTION
            } else {
                MB_OK | MB_ICONINFORMATION
            };
            MessageBoxW(
                ptr::null_mut(),
                wide(text).as_ptr(),
                wide(APP_NAME).as_ptr(),
                flags | MB_SETFOREGROUND,
            ) == IDYES
                || !question
        }
    }

    // ---- per-user registry (HKEY_CURRENT_USER only) ----
    fn reg_set(key: &str, name: Option<&str>, value: &str) -> bool {
        unsafe {
            let mut h: HKEY = ptr::null_mut();
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(key).as_ptr(),
                0,
                ptr::null_mut(),
                0,
                KEY_SET_VALUE,
                ptr::null_mut(),
                &mut h,
                ptr::null_mut(),
            ) != 0
            {
                return false;
            }
            let data = wide(value);
            let name = name.map(wide);
            let ok = RegSetValueExW(
                h,
                name.as_ref().map_or(ptr::null(), |n| n.as_ptr()),
                0,
                REG_SZ,
                data.as_ptr() as _,
                (data.len() * 2) as u32,
            ) == 0;
            RegCloseKey(h);
            ok
        }
    }
    fn reg_set_dword(key: &str, name: &str, value: u32) {
        unsafe {
            let mut h: HKEY = ptr::null_mut();
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(key).as_ptr(),
                0,
                ptr::null_mut(),
                0,
                KEY_SET_VALUE,
                ptr::null_mut(),
                &mut h,
                ptr::null_mut(),
            ) == 0
            {
                RegSetValueExW(
                    h,
                    wide(name).as_ptr(),
                    0,
                    REG_DWORD,
                    &value as *const u32 as _,
                    4,
                );
                RegCloseKey(h);
            }
        }
    }
    fn reg_get(key: &str, name: &str) -> Option<String> {
        unsafe {
            let mut buf = vec![0u16; 2048];
            let mut size = (buf.len() * 2) as u32;
            let r = RegGetValueW(
                HKEY_CURRENT_USER,
                wide(key).as_ptr(),
                wide(name).as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                buf.as_mut_ptr() as _,
                &mut size,
            );
            (r == 0)
                .then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
        }
    }
    fn reg_delete_value(key: &str, name: &str) {
        unsafe {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr());
        }
    }
    fn reg_delete_tree(key: &str) {
        unsafe {
            RegDeleteTreeW(HKEY_CURRENT_USER, wide(key).as_ptr());
        }
    }

    /// Recognize the previous startup entry, but never start both brands.
    /// A user-initiated toggle replaces old startup registration with new.
    pub fn autostart(set: Option<bool>) -> bool {
        if let Some(on) = set {
            if on {
                if let Ok(exe) = std::env::current_exe() {
                    reg_set(
                        RUN_KEY,
                        Some(APP_NAME),
                        &format!("\"{}\" --at-sign-in", exe.display()),
                    );
                    reg_delete_value(RUN_KEY, LEGACY_APP_NAME);
                }
            } else {
                reg_delete_value(RUN_KEY, APP_NAME);
                reg_delete_value(RUN_KEY, LEGACY_APP_NAME);
            }
        }
        reg_get(RUN_KEY, APP_NAME).is_some() || reg_get(RUN_KEY, LEGACY_APP_NAME).is_some()
    }

    fn shortcut(link: &Path, target: &Path, arguments: &str) -> Result<(), String> {
        use winapi::shared::guiddef::GUID;
        type Ptr = *mut std::ffi::c_void;
        const CLSID_SHELL_LINK: GUID = GUID {
            Data1: 0x00021401,
            Data2: 0,
            Data3: 0,
            Data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
        };
        const IID_SHELL_LINK_W: GUID = GUID {
            Data1: 0x000214F9,
            Data2: 0,
            Data3: 0,
            Data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
        };
        const IID_PERSIST_FILE: GUID = GUID {
            Data1: 0x0000010B,
            Data2: 0,
            Data3: 0,
            Data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
        };
        #[link(name = "ole32")]
        extern "system" {
            fn CoInitializeEx(r: Ptr, flags: u32) -> i32;
            fn CoCreateInstance(
                clsid: *const GUID,
                outer: Ptr,
                ctx: u32,
                iid: *const GUID,
                ppv: *mut Ptr,
            ) -> i32;
        }
        unsafe fn vfn(obj: Ptr, i: usize) -> *const std::ffi::c_void {
            *(*(obj as *mut *const *const std::ffi::c_void)).add(i)
        }
        unsafe {
            CoInitializeEx(ptr::null_mut(), 2);
            let mut link_obj = ptr::null_mut();
            if CoCreateInstance(
                &CLSID_SHELL_LINK,
                ptr::null_mut(),
                1,
                &IID_SHELL_LINK_W,
                &mut link_obj,
            ) < 0
            {
                return Err("Could not create the shortcut".into());
            }
            let set_path: extern "system" fn(Ptr, *const u16) -> i32 =
                mem::transmute(vfn(link_obj, 20));
            let set_args: extern "system" fn(Ptr, *const u16) -> i32 =
                mem::transmute(vfn(link_obj, 11));
            let set_dir: extern "system" fn(Ptr, *const u16) -> i32 =
                mem::transmute(vfn(link_obj, 9));
            let set_desc: extern "system" fn(Ptr, *const u16) -> i32 =
                mem::transmute(vfn(link_obj, 7));
            set_path(link_obj, wide(&target.to_string_lossy()).as_ptr());
            set_args(link_obj, wide(arguments).as_ptr());
            if let Some(dir) = target.parent() {
                set_dir(link_obj, wide(&dir.to_string_lossy()).as_ptr());
            }
            set_desc(
                link_obj,
                wide("Organize your Windows desktop into groups").as_ptr(),
            );
            let qi: extern "system" fn(Ptr, *const GUID, *mut Ptr) -> i32 =
                mem::transmute(vfn(link_obj, 0));
            let mut file = ptr::null_mut();
            qi(link_obj, &IID_PERSIST_FILE, &mut file);
            let mut result = Err("Could not save the shortcut".to_string());
            if !file.is_null() {
                let save: extern "system" fn(Ptr, *const u16, i32) -> i32 =
                    mem::transmute(vfn(file, 6));
                if save(file, wide(&link.to_string_lossy()).as_ptr(), 1) >= 0 {
                    result = Ok(());
                }
                let release: extern "system" fn(Ptr) -> u32 = mem::transmute(vfn(file, 2));
                release(file);
            }
            let release: extern "system" fn(Ptr) -> u32 = mem::transmute(vfn(link_obj, 2));
            release(link_obj);
            result
        }
    }
    fn start_menu_link_for(name: &str) -> PathBuf {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join(format!("{name}.lnk"))
    }
    fn start_menu_link() -> PathBuf {
        start_menu_link_for(APP_NAME)
    }

    /// After a successful new per-user install, retire only registrations that
    /// belong to our prior name. Never remove the old data or installed EXE:
    /// keeping it allows manual recovery if a new release misbehaves.
    fn retire_legacy_install() {
        if reg_get(UNINSTALL_KEY, "DisplayName").as_deref() != Some(APP_NAME) {
            return; // Avoid orphaning a legacy install if new registration failed.
        }
        reg_delete_value(RUN_KEY, LEGACY_APP_NAME);
        // Previous builds (and the standalone menu-repair script) may have
        // registered the desktop verbs without an Apps & features entry.
        reg_delete_tree(LEGACY_VERB_NEW);
        reg_delete_tree(LEGACY_VERB_SETTINGS);
        if reg_get(LEGACY_UNINSTALL_KEY, "DisplayName").as_deref() == Some(LEGACY_APP_NAME) {
            reg_delete_tree(LEGACY_UNINSTALL_KEY);
        }
        let _ = std::fs::remove_file(start_menu_link_for(LEGACY_APP_NAME));
    }

    /// Per-user install (no administrator rights): copy the program, add a Start
    /// menu shortcut, an Apps & features entry and two desktop right-click items.
    pub fn install(start_at_login: bool) -> Result<PathBuf, String> {
        let source = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = install_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let exe = dir.join(format!("{APP_NAME}.exe"));
        if source != exe {
            // Ask a running copy to close (it saves on the way out) so its
            // file can be replaced, and give it a few seconds to finish.
            let _ = send_command("exit");
            for _ in 0..25 {
                if !instance_running() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            let mut copied = std::fs::copy(&source, &exe)
                .map(|_| ())
                .map_err(|e| e.to_string());
            if copied.is_err() && exe.exists() {
                // Still in use: Windows lets a running program's file be
                // renamed, so move it aside and write the new one in its place.
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                let aside = dir.join(format!("{APP_NAME}.exe.{stamp}.old"));
                if std::fs::rename(&exe, &aside).is_ok() {
                    copied = std::fs::copy(&source, &exe)
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                }
            }
            for _ in 0..10 {
                if copied.is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
                copied = std::fs::copy(&source, &exe)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
            }
            copied.map_err(|e| format!("Could not copy the program: {e}"))?;
            // Old copies set aside by earlier updates, once nothing uses them.
            if let Ok(entries) = std::fs::read_dir(&dir) {
                let prefix = format!("{APP_NAME}.exe.");
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(&prefix) && name.ends_with(".old") {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
        let quoted = format!("\"{}\"", exe.display());
        shortcut(&start_menu_link(), &exe, "")?;
        reg_set(UNINSTALL_KEY, Some("DisplayName"), APP_NAME);
        reg_set(
            UNINSTALL_KEY,
            Some("DisplayVersion"),
            env!("CARGO_PKG_VERSION"),
        );
        reg_set(UNINSTALL_KEY, Some("Publisher"), APP_NAME);
        reg_set(
            UNINSTALL_KEY,
            Some("InstallLocation"),
            &dir.to_string_lossy(),
        );
        reg_set(UNINSTALL_KEY, Some("DisplayIcon"), &exe.to_string_lossy());
        reg_set(
            UNINSTALL_KEY,
            Some("UninstallString"),
            &format!("{quoted} --uninstall"),
        );
        reg_set_dword(UNINSTALL_KEY, "NoModify", 1);
        reg_set_dword(UNINSTALL_KEY, "NoRepair", 1);
        reg_set(VERB_NEW, Some("MUIVerb"), "New group here");
        reg_set(VERB_NEW, Some("Icon"), &exe.to_string_lossy());
        reg_set(
            &format!(r"{VERB_NEW}\command"),
            None,
            &format!("{quoted} --command new-group"),
        );
        reg_set(VERB_SETTINGS, Some("MUIVerb"), "DesktopCraft settings");
        reg_set(VERB_SETTINGS, Some("Icon"), &exe.to_string_lossy());
        reg_set(
            &format!(r"{VERB_SETTINGS}\command"),
            None,
            &format!("{quoted} --command settings"),
        );
        // Only classic shell-menu registration; do NOT replace Windows 11's
        // normal context menu or globally change Explorer's presentation.
        reg_set(VERB_PIN_FILE, Some("MUIVerb"), "Pin to DesktopCraft");
        reg_set(VERB_PIN_FILE, Some("Icon"), &exe.to_string_lossy());
        reg_set(
            &format!(r"{VERB_PIN_FILE}\command"),
            None,
            &format!("{quoted} --pin-file \"%1\""),
        );
        if start_at_login {
            reg_set(RUN_KEY, Some(APP_NAME), &format!("{quoted} --at-sign-in"));
        } else {
            reg_delete_value(RUN_KEY, APP_NAME);
        }
        retire_legacy_install();
        Ok(exe)
    }
    /// The version installed for this account (from its Settings > Apps
    /// entry), when its program file is really there; None when not installed.
    pub fn installed_version() -> Option<String> {
        if !install_dir().join(format!("{APP_NAME}.exe")).exists() {
            return None;
        }
        Some(reg_get(UNINSTALL_KEY, "DisplayVersion").unwrap_or_default())
    }
    /// Remove everything install() added. Layout data in %APPDATA% is kept.
    pub fn uninstall() {
        let _ = send_command("exit");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        reg_delete_value(RUN_KEY, APP_NAME);
        reg_delete_tree(VERB_NEW);
        reg_delete_tree(VERB_SETTINGS);
        reg_delete_tree(VERB_PIN_FILE);
        reg_delete_tree(UNINSTALL_KEY);
        let _ = std::fs::remove_file(start_menu_link());
        let dir = install_dir();
        if dir.exists() {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("cmd")
                .args(["/C", "ping -n 3 127.0.0.1 >nul & rmdir /S /Q"])
                .arg(&dir)
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .spawn();
        }
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
mod fallback {
    use super::*;
    pub fn monitors() -> Vec<Monitor> {
        vec![]
    }
    pub fn virtual_rect() -> [i32; 4] {
        [0, 0, 1280, 800]
    }
    pub fn place(_: [i32; 4]) {}
    pub fn set_region(_: &[[i32; 5]]) {}
    pub fn set_front(_: bool) {}
    pub fn settings_taskbar(_: bool) {}
    pub fn settings_has_focus() -> bool {
        false
    }
    pub fn settings_deactivate() {}
    pub fn keep_shown() {}
    pub fn begin_color_pick() {}
    pub fn color_pick_shielded() -> bool {
        false
    }
    pub fn finish_color_pick() -> Option<Option<[f32; 3]>> {
        None
    }
    pub fn mouse_left_down() -> bool {
        false
    }
    pub fn capture_pointer(_: bool) {}
    pub fn cursor() -> (i32, i32) {
        (0, 0)
    }
    pub fn over_explorer_desktop(_: i32, _: i32) -> bool {
        false
    }
    pub fn over_other_app(_: i32, _: i32) -> bool {
        false
    }
    pub fn first_instance() -> bool {
        true
    }
    pub fn instance_running() -> bool {
        false
    }
    pub fn allow_foreground() {}
    pub fn install_quick_hide_hook() {}
    pub fn set_hook_rects(_: Vec<[i32; 4]>) {}
    pub fn marquee_drag() -> Option<((i32, i32), (i32, i32))> {
        None
    }
    pub fn take_marquee_release() -> Option<((i32, i32), (i32, i32))> {
        None
    }
    pub fn take_quick_hide() -> bool {
        false
    }
    pub fn take_press() -> Option<(i32, i32)> {
        None
    }
    pub fn set_right_click_enabled(_: bool) {}
    pub fn right_drag() -> Option<((i32, i32), (i32, i32))> {
        None
    }
    pub fn take_right_release() -> Option<((i32, i32), (i32, i32))> {
        None
    }
    pub fn show_native_desktop_menu(_: i32, _: i32) {}
    pub fn window() -> usize {
        0
    }
    pub fn message(_: &str, _: bool) -> bool {
        false
    }
    pub fn ask_default_no(_: &str) -> bool {
        false
    }
    pub fn autostart(_: Option<bool>) -> bool {
        false
    }
    pub fn install(_: bool) -> Result<PathBuf, String> {
        Err("Installing is only available on Windows.".into())
    }
    pub fn installed_version() -> Option<String> {
        None
    }
    pub fn uninstall() {}
}
#[cfg(not(windows))]
pub use fallback::*;

#[cfg(test)]
mod desktopcraft_rebrand_tests {
    use super::*;
    #[test]
    fn existing_workspace_has_priority_without_moving_user_data() {
        let root =
            std::env::temp_dir().join(format!("desktopcraft-rebrand-test-{}", std::process::id()));
        let new = root.join(APP_NAME);
        let old = root.join(LEGACY_APP_NAME);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(choose_workspace_dir(&root), new);
        std::fs::create_dir_all(old.join("data")).unwrap();
        let legacy_file = old.join("data/workspace.json");
        std::fs::write(&legacy_file, r#"{"pages":["Legacy layout"]}"#).unwrap();
        assert_eq!(choose_workspace_dir(&root), old);
        assert!(legacy_file.exists());
        std::fs::create_dir_all(new.join("data")).unwrap();
        let current_file = new.join("data/workspace.json");
        std::fs::write(&current_file, r#"{"pages":["New layout"]}"#).unwrap();
        assert_eq!(choose_workspace_dir(&root), new);
        assert_eq!(
            std::fs::read_to_string(&legacy_file).unwrap(),
            r#"{"pages":["Legacy layout"]}"#
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}

/// Whether version `a` is newer than `b` ("0.1.10" > "0.1.9"); parts that
/// aren't numbers count as 0.
pub fn version_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect()
    };
    let (x, y) = (parts(a), parts(b));
    for i in 0..x.len().max(y.len()) {
        let (p, q) = (
            x.get(i).copied().unwrap_or(0),
            y.get(i).copied().unwrap_or(0),
        );
        if p != q {
            return p > q;
        }
    }
    false
}
#[cfg(test)]
mod version_tests {
    use super::version_newer;
    #[test]
    fn compares_versions_part_by_part() {
        assert!(version_newer("0.1.1", "0.1.0"));
        assert!(version_newer("0.1.10", "0.1.9"));
        assert!(version_newer("1.0", "0.9.9"));
        assert!(!version_newer("0.1.0", "0.1.0"));
        assert!(!version_newer("0.1.0", "0.1.1"));
        assert!(!version_newer("0.1", "0.1.0"));
        assert!(version_newer("v0.2.0", "0.1.5"));
    }
}
pub fn is_setup_name(exe: &Path) -> bool {
    exe.file_stem()
        .is_some_and(|s| s.to_string_lossy().to_lowercase().contains("setup"))
}
