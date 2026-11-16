use std::path::PathBuf;

/// A chord contains ANY 2-3 distinct supported physical keys; it need not
/// contain modifiers. Key order does not affect matching. This is a global
/// key-state check, not a keyboard hook that records user typing.
fn peek_key_codes() -> impl Iterator<Item = i32> {
    [0x11, 0x12, 0x10, 0x09, 0x20, 0x25, 0x26, 0x27, 0x28]
        .into_iter()
        .chain(0x30..=0x39)
        .chain(0x41..=0x5A)
        .chain(0x70..=0x87)
}
fn peek_key_label(key: i32) -> Option<String> {
    match key {
        0x11 => Some("Ctrl".into()),
        0x12 => Some("Alt".into()),
        0x10 => Some("Shift".into()),
        0x09 => Some("Tab".into()),
        0x20 => Some("Space".into()),
        0x25 => Some("Left".into()),
        0x26 => Some("Up".into()),
        0x27 => Some("Right".into()),
        0x28 => Some("Down".into()),
        0x30..=0x39 | 0x41..=0x5A => Some((key as u8 as char).to_string()),
        0x70..=0x87 => Some(format!("F{}", key - 0x70 + 1)),
        _ => None,
    }
}
fn peek_key_code(label: &str) -> Option<i32> {
    let upper = label.trim().to_ascii_uppercase();
    match upper.as_str() {
        "CTRL" | "CONTROL" => Some(0x11),
        "ALT" => Some(0x12),
        "SHIFT" => Some(0x10),
        "TAB" => Some(0x09),
        "SPACE" => Some(0x20),
        "LEFT" => Some(0x25),
        "UP" => Some(0x26),
        "RIGHT" => Some(0x27),
        "DOWN" => Some(0x28),
        _ if upper.len() == 1 && upper.as_bytes()[0].is_ascii_alphanumeric() => {
            Some(upper.as_bytes()[0] as i32)
        }
        _ if upper.starts_with('F')
            && upper[1..]
                .parse::<u8>()
                .is_ok_and(|n| (1..=24).contains(&n)) =>
        {
            Some(0x70 + upper[1..].parse::<i32>().ok()? - 1)
        }
        _ => None,
    }
}
fn normalized_keys(mut keys: Vec<i32>) -> Option<Vec<i32>> {
    let original_len = keys.len();
    if !(2..=3).contains(&original_len) {
        return None;
    }
    if keys.iter().any(|key| peek_key_label(*key).is_none()) {
        return None;
    }
    keys.sort_by_key(|key| {
        peek_key_codes()
            .position(|code| code == *key)
            .unwrap_or(usize::MAX)
    });
    keys.dedup();
    (keys.len() == original_len).then_some(keys)
}
pub fn parse_peek_shortcut(value: &str) -> Option<Vec<i32>> {
    let parts: Vec<_> = value.split('+').collect();
    normalized_keys(
        parts
            .iter()
            .map(|part| peek_key_code(part))
            .collect::<Option<Vec<_>>>()?,
    )
}
pub fn describe_peek_keys(keys: Vec<i32>) -> Option<String> {
    let codes = normalized_keys(keys)?;
    Some(
        codes
            .iter()
            .filter_map(|key| peek_key_label(*key))
            .collect::<Vec<_>>()
            .join("+"),
    )
}
/// Pure chord check used by the Windows poller and tests. Only an exact
/// simultaneous two- or three-key match can keep Peek active.
pub fn peek_chord_held(shortcut: &str, pressed: &[i32]) -> bool {
    match (
        parse_peek_shortcut(shortcut),
        normalized_keys(pressed.to_vec()),
    ) {
        (Some(expected), Some(held)) => expected == held,
        _ => false,
    }
}

#[cfg(test)]
mod shortcut_parse_tests {
    use super::*;
    #[test]
    fn peek_supports_any_two_or_three_distinct_keys() {
        assert_eq!(parse_peek_shortcut("Z+X+C"), Some(vec![0x43, 0x58, 0x5A]));
        assert_eq!(
            parse_peek_shortcut("Ctrl+Shift+Tab"),
            Some(vec![0x11, 0x10, 0x09])
        );
        assert_eq!(
            parse_peek_shortcut("F1+F2+F3"),
            Some(vec![0x70, 0x71, 0x72])
        );
        assert_eq!(
            parse_peek_shortcut("Ctrl+Alt+F10"),
            Some(vec![0x11, 0x12, 0x79])
        );
        assert_eq!(parse_peek_shortcut("Ctrl+Tab"), Some(vec![0x11, 0x09]));
        assert_eq!(parse_peek_shortcut("Shift+P"), Some(vec![0x10, 0x50]));
        assert_eq!(
            describe_peek_keys(vec![0x5A, 0x58, 0x43]).as_deref(),
            Some("C+X+Z")
        );
        assert!(peek_chord_held("Z+X+C", &[0x43, 0x58, 0x5A]));
        assert!(!peek_chord_held("Z+X+C", &[0x43, 0x58])); // key released
        assert!(!peek_chord_held("Z+X+C", &[0x43, 0x58, 0x5A, 0x11]));
        assert!(peek_chord_held("Ctrl+Shift+Tab", &[0x09, 0x10, 0x11]));
        assert!(peek_chord_held("F1+F2+F3", &[0x70, 0x71, 0x72]));
    }
    #[test]
    fn peek_rejects_missing_duplicate_or_oversized_chords() {
        for bad in [
            "P",
            "",
            "Z+Z",
            "F1+F1+F2",
            "A+B+C+D",
            "Ctrl+Shift+",
            "Ctrl+Alt+Shift+Tab",
            "Win+Space",
            "F25+F1",
            "A+?",
        ] {
            assert!(parse_peek_shortcut(bad).is_none(), "accepted {bad}");
        }
        assert!(describe_peek_keys(vec![0x5A, 0x58, 0x43, 0x11]).is_none());
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::{mem, ptr};
    use winapi::{
        shared::{
            minwindef::{BOOL, LPARAM},
            windef::{HWND, RECT},
        },
        um::{
            combaseapi::{CoInitializeEx, CoTaskMemFree, CoUninitialize},
            commdlg::{
                GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST,
                OPENFILENAMEW,
            },
            processthreadsapi::GetCurrentProcessId,
            shellapi::ShellExecuteW,
            shlobj::SHGetPathFromIDListW,
            winuser::*,
        },
    };
    #[repr(C)]
    struct BrowseInfo {
        owner: HWND,
        root: *const std::ffi::c_void,
        display: *mut u16,
        title: *const u16,
        flags: u32,
        callback: Option<unsafe extern "system" fn(HWND, u32, LPARAM, LPARAM) -> i32>,
        data: LPARAM,
        image: i32,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHBrowseForFolderW(info: *mut BrowseInfo) -> *mut std::ffi::c_void;
    }
    // Name/extension rules now use model::fold (locale-independent Unicode
    // folding) on every platform; see MECHANICS.md "Implementation decisions".
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    unsafe extern "system" fn find(hwnd: HWND, result: LPARAM) -> BOOL {
        let mut process = 0;
        GetWindowThreadProcessId(hwnd, &mut process);
        if process == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
            *(result as *mut HWND) = hwnd;
            0
        } else {
            1
        }
    }
    fn window() -> HWND {
        unsafe {
            let mut hwnd = ptr::null_mut();
            EnumWindows(Some(find), &mut hwnd as *mut _ as LPARAM);
            hwnd
        }
    }
    pub fn open(path: &str) -> Result<(), String> {
        unsafe {
            let result = ShellExecuteW(
                window(),
                wide("open").as_ptr(),
                wide(path).as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            ) as isize;
            if result > 32 {
                Ok(())
            } else {
                Err(format!(
                    "Windows could not open this item (error {result})."
                ))
            }
        }
    }
    pub fn choose_file() -> Option<PathBuf> {
        unsafe {
            let mut buffer = [0u16; 32768];
            let mut ofn: OPENFILENAMEW = mem::zeroed();
            let title = wide("Add a file or shortcut");
            let filter = wide("All files\0*.*\0");
            ofn.lStructSize = mem::size_of::<OPENFILENAMEW>() as u32;
            ofn.hwndOwner = window();
            ofn.lpstrFile = buffer.as_mut_ptr();
            ofn.nMaxFile = buffer.len() as u32;
            ofn.lpstrTitle = title.as_ptr();
            ofn.lpstrFilter = filter.as_ptr();
            ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
            if GetOpenFileNameW(&mut ofn) != 0 {
                Some(PathBuf::from(String::from_utf16_lossy(
                    &buffer[..buffer.iter().position(|&x| x == 0).unwrap_or(0)],
                )))
            } else {
                None
            }
        }
    }
    pub fn choose_folder() -> Option<PathBuf> {
        unsafe {
            let hr = CoInitializeEx(ptr::null_mut(), 2);
            let mut info: BrowseInfo = mem::zeroed();
            let title = wide("Choose a folder to show in your workspace");
            info.owner = window();
            info.title = title.as_ptr();
            info.flags = 0x41;
            let item = SHBrowseForFolderW(&mut info);
            let mut output = None;
            if !item.is_null() {
                let mut path = [0u16; 260];
                if SHGetPathFromIDListW(item as _, path.as_mut_ptr()) != 0 {
                    output = Some(PathBuf::from(String::from_utf16_lossy(
                        &path[..path.iter().position(|&x| x == 0).unwrap_or(0)],
                    )))
                }
                CoTaskMemFree(item as _);
            }
            if hr >= 0 {
                CoUninitialize()
            }
            output
        }
    }
    /// Read a small allowlist of physical keys only while recording or
    /// checking the chosen Peek chord; no key history is retained.
    pub fn peek_pressed_keys() -> Vec<i32> {
        unsafe {
            peek_key_codes()
                .filter(|key| GetAsyncKeyState(*key) as u16 & 0x8000 != 0)
                .collect()
        }
    }
    pub fn hotkey(shortcut: &str) -> bool {
        // Any held chord can be seen even when another application has focus.
        // This does not intercept keys from that application.
        peek_chord_held(shortcut, &peek_pressed_keys())
    }

    /// Global polling for Ctrl/Alt + arrow, limited to DesktopCraft or the
    /// Windows desktop foreground. Never hijack keys inside other programs.
    /// Return one transition per keypress; holding a key does not create pages.
    pub fn page_hotkey() -> i32 {
        use std::sync::atomic::{AtomicI32, Ordering};
        static HELD: AtomicI32 = AtomicI32::new(0);
        unsafe {
            let active = GetForegroundWindow();
            let mut pid = 0;
            GetWindowThreadProcessId(active, &mut pid);
            let allowed = if pid == std::process::id() {
                true
            } else {
                let mut class = [0u16; 64];
                let n = GetClassNameW(active, class.as_mut_ptr(), class.len() as i32);
                matches!(
                    String::from_utf16_lossy(&class[..n as usize]).as_str(),
                    "Progman" | "WorkerW"
                )
            };
            let down = |k: i32| (GetAsyncKeyState(k) as u16 & 0x8000) != 0;
            let state = if allowed && (down(0x11) || down(0x12)) && !down(0x5B) && !down(0x5C) {
                if down(0x27) && !down(0x25) {
                    1
                } else if down(0x25) && !down(0x27) {
                    -1
                } else {
                    0
                }
            } else {
                0
            };
            let previous = HELD.swap(state, Ordering::Relaxed);
            if state != previous {
                state
            } else {
                0
            }
        }
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetDpiForWindow(hwnd: HWND) -> u32;
        fn GetDpiForSystem() -> u32;
    }
    unsafe extern "system" fn find_desktop_view(hwnd: HWND, out: LPARAM) -> BOOL {
        let view = FindWindowExW(
            hwnd,
            ptr::null_mut(),
            wide("SHELLDLL_DefView").as_ptr(),
            ptr::null(),
        );
        if !view.is_null() {
            let list = FindWindowExW(
                view,
                ptr::null_mut(),
                wide("SysListView32").as_ptr(),
                ptr::null(),
            );
            if !list.is_null() {
                *(out as *mut HWND) = list;
                return 0;
            }
        }
        1
    }
    /// Use Windows' documented desktop list-view icon spacing when available,
    /// otherwise SPI_GETICONMETRICS, returned in 96-DPI logical pixels.
    pub fn desktop_icon_spacing() -> Option<(f32, f32)> {
        unsafe {
            let mut list: HWND = ptr::null_mut();
            EnumWindows(Some(find_desktop_view), &mut list as *mut _ as LPARAM);
            if !list.is_null() {
                let mut result = 0usize;
                let ok =
                    SendMessageTimeoutW(list, 0x1033, 0, 0, SMTO_ABORTIFHUNG, 300, &mut result);
                let (cx, cy) = ((result & 0xffff) as f32, ((result >> 16) & 0xffff) as f32);
                let dpi = GetDpiForWindow(list).max(1) as f32;
                if ok != 0 && cx > 0.0 && cy > 0.0 {
                    return Some((cx * 96.0 / dpi, cy * 96.0 / dpi));
                }
            }
            #[repr(C)]
            #[allow(non_snake_case)]
            struct ICONMETRICSW {
                cbSize: u32,
                iHorzSpacing: i32,
                iVertSpacing: i32,
                iTitleWrap: i32,
                lfFont: [u8; 92],
            }
            let mut metrics: ICONMETRICSW = mem::zeroed();
            metrics.cbSize = mem::size_of::<ICONMETRICSW>() as u32;
            if SystemParametersInfoW(
                SPI_GETICONMETRICS,
                metrics.cbSize,
                &mut metrics as *mut _ as _,
                0,
            ) != 0
                && metrics.iHorzSpacing > 0
                && metrics.iVertSpacing > 0
            {
                let dpi = GetDpiForSystem().max(1) as f32;
                return Some((
                    metrics.iHorzSpacing as f32 * 96.0 / dpi,
                    metrics.iVertSpacing as f32 * 96.0 / dpi,
                ));
            }
            None
        }
    }
    pub fn double_click_interval() -> f64 {
        // Use the system double-click interval so the UI matches Windows input.
        unsafe { GetDoubleClickTime() as f64 / 1000.0 }
    }
    pub fn peek(front: bool, desktop: bool) {
        unsafe {
            let hwnd = window();
            if !hwnd.is_null() {
                ShowWindow(hwnd, SW_RESTORE);
                SetWindowPos(
                    hwnd,
                    if front {
                        HWND_TOPMOST
                    } else if desktop {
                        HWND_BOTTOM
                    } else {
                        HWND_NOTOPMOST
                    },
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                if front {
                    SetForegroundWindow(hwnd);
                }
            }
        }
    }
    pub fn desktop(enabled: bool, previous: &mut [i32; 4]) {
        unsafe {
            let hwnd = window();
            if hwnd.is_null() {
                return;
            }
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            if enabled {
                let mut rect: RECT = mem::zeroed();
                GetWindowRect(hwnd, &mut rect);
                *previous = [
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                ];
                SetWindowLongPtrW(
                    hwnd,
                    GWL_STYLE,
                    (style & !(WS_OVERLAPPEDWINDOW as isize)) | WS_POPUP as isize,
                );
                SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rect as *mut _ as _, 0);
                SetWindowPos(
                    hwnd,
                    HWND_BOTTOM,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_FRAMECHANGED | SWP_NOACTIVATE,
                );
            } else {
                SetWindowLongPtrW(
                    hwnd,
                    GWL_STYLE,
                    (style & !(WS_POPUP as isize)) | WS_OVERLAPPEDWINDOW as isize,
                );
                SetWindowPos(
                    hwnd,
                    HWND_NOTOPMOST,
                    previous[0],
                    previous[1],
                    previous[2],
                    previous[3],
                    SWP_FRAMECHANGED | SWP_NOACTIVATE,
                );
            }
        }
    }
}
#[cfg(windows)]
pub use win::*;
#[cfg(not(windows))]
pub fn open(path: &str) -> Result<(), String> {
    std::process::Command::new(if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    })
    .arg(path)
    .spawn()
    .map(|_| ())
    .map_err(|e| e.to_string())
}
#[cfg(not(windows))]
pub fn choose_file() -> Option<PathBuf> {
    None
}
#[cfg(not(windows))]
pub fn choose_folder() -> Option<PathBuf> {
    None
}
#[cfg(not(windows))]
pub fn peek_pressed_keys() -> Vec<i32> {
    vec![]
}
#[cfg(not(windows))]
pub fn hotkey(_: &str) -> bool {
    false
}
#[cfg(not(windows))]
pub fn page_hotkey() -> i32 {
    0
}
#[cfg(not(windows))]
pub fn desktop_icon_spacing() -> Option<(f32, f32)> {
    None
}
#[cfg(not(windows))]
pub fn double_click_interval() -> f64 {
    0.5 // Compatibility estimate; original target is Windows-only.
}
#[cfg(not(windows))]
pub fn peek(_: bool, _: bool) {}
#[cfg(not(windows))]
pub fn desktop(_: bool, _: &mut [i32; 4]) {}
