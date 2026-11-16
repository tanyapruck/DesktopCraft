//! DesktopCraft native Quick Actions menu, hosted on an independent Windows
//! UI thread. TrackPopupMenuEx runs a nested message loop; running that on the
//! Macroquad rendering thread can panic with "Only one active query is allowed
//! by OpenGL". Only a small command ID is returned to the main thread.
#![allow(dead_code)]
use std::{ffi::c_void, ptr};
use winapi::shared::windef::{HMENU, HWND};
use winapi::um::winuser::{CreateWindowExW, DestroyWindow, WS_EX_TOOLWINDOW, WS_POPUP};
const MF_GRAYED: u32 = 1;
const MF_SEPARATOR: u32 = 0x0800;
const TPM_RETURNCMD: u32 = 0x0100;
const TPM_RIGHTBUTTON: u32 = 0x0002;
const TPM_NONOTIFY: u32 = 0x0080;

#[link(name = "user32")]
extern "system" {
    fn CreatePopupMenu() -> HMENU;
    fn AppendMenuW(menu: HMENU, flags: u32, id: usize, label: *const u16) -> i32;
    fn TrackPopupMenuEx(
        menu: HMENU,
        flags: u32,
        x: i32,
        y: i32,
        owner: HWND,
        params: *const c_void,
    ) -> u32;
    fn DestroyMenu(menu: HMENU) -> i32;
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn add(menu: HMENU, id: u32, name: &str, disabled: bool) {
    let label = wide(name);
    unsafe {
        AppendMenuW(
            menu,
            if disabled { MF_GRAYED } else { 0 },
            id as usize,
            label.as_ptr(),
        );
    }
}
pub const PROPERTIES: u32 = 1;
pub const PIN_CLIPBOARD: u32 = 2;
pub const UNPIN_SELECTED: u32 = 3;
pub const COPY_PATHS: u32 = 4;
pub const USE_VIRTUAL: u32 = 5;

pub fn show_detached(x: i32, y: i32, selected: usize, virtual_tab: bool) -> u32 {
    // This function runs only on the worker thread. It owns the HWND/menu
    // and never dispatches a Windows nested popup loop on the GL thread.
    unsafe {
        let class = wide("STATIC");
        let title = wide("DesktopCraft Quick Actions");
        let owner = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            -32000,
            -32000,
            1,
            1,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if owner.is_null() {
            return 0;
        }
        let menu = CreatePopupMenu();
        if menu.is_null() {
            DestroyWindow(owner);
            return 0;
        }
        add(menu, PROPERTIES, "Properties (Alt+Enter)", selected != 1);
        // Pin from clipboard is a virtual membership action, not disk move.
        add(
            menu,
            PIN_CLIPBOARD,
            "Pin files from Clipboard (no Move)",
            !virtual_tab,
        );
        add(
            menu,
            UNPIN_SELECTED,
            "Unpin selection (keep files on disk)",
            selected == 0 || !virtual_tab,
        );
        add(menu, COPY_PATHS, "Copy selected full paths", selected == 0);
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        add(menu, USE_VIRTUAL, "Use Virtual Box renderer", false);
        let action = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            x,
            y,
            owner,
            ptr::null(),
        );
        DestroyMenu(menu);
        DestroyWindow(owner);
        action
    }
}
