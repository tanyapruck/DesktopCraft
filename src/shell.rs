//! Windows Shell integration through documented, supported interfaces only:
//! known folders, IShellWindows -> IFolderView (Explorer's desktop icon view),
//! IContextMenu (Explorer's own item menu), OLE DoDragDrop with the shell's data
//! object, SHFileOperation (undoable file operations) and IShellItemImageFactory
//! (Explorer's icons and thumbnails). Nothing is injected into Explorer.
#![allow(non_snake_case, clippy::missing_safety_doc, dead_code)]

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ViewItem {
    pub path: String,
    /// The label Explorer shows (e.g. "Recycle Bin", shortcuts without ".lnk").
    pub label: String,
    /// Screen position (physical pixels) of the icon's top-left corner.
    pub pos: (i32, i32),
}

pub enum MenuChoice {
    Nothing,
    Invoked,
    Rename,
}

/// Real files copied/cut in Explorer (CF_HDROP + Preferred DropEffect).
/// Cut requires a physical move only when pasting into a real directory.
#[derive(Clone, Debug)]
pub struct FileClipboard {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::{ffi::c_void, mem, ptr};
    use winapi::shared::{
        guiddef::GUID,
        windef::{HBITMAP, HMENU, HWND, POINT, SIZE},
    };
    use winapi::um::{shellapi::*, wingdi::*, winuser::*};

    type HRESULT = i32;
    type Ptr = *mut c_void;

    const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> GUID {
        GUID {
            Data1: d1,
            Data2: d2,
            Data3: d3,
            Data4: d4,
        }
    }
    const CLSID_SHELL_WINDOWS: GUID = guid(
        0x9BA05972,
        0xF6A8,
        0x11CF,
        [0xA4, 0x42, 0x00, 0xA0, 0xC9, 0x0A, 0x8F, 0x39],
    );
    const IID_ISHELL_WINDOWS: GUID = guid(
        0x85CB6900,
        0x4D95,
        0x11CF,
        [0x96, 0x0C, 0x00, 0x80, 0xC7, 0xF4, 0xEE, 0x85],
    );
    const IID_ISERVICE_PROVIDER: GUID = guid(
        0x6D5140C1,
        0x7436,
        0x11CE,
        [0x80, 0x34, 0x00, 0xAA, 0x00, 0x60, 0x09, 0xFA],
    );
    const SID_STOP_LEVEL_BROWSER: GUID = guid(
        0x4C96BE40,
        0x915C,
        0x11CF,
        [0x99, 0xD3, 0x00, 0xAA, 0x00, 0x4A, 0xE8, 0x37],
    );
    const IID_ISHELL_BROWSER: GUID = guid(0x000214E2, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_IFOLDER_VIEW: GUID = guid(
        0xCDE725B0,
        0xCCC9,
        0x4519,
        [0x91, 0x7E, 0x32, 0x5D, 0x72, 0xFA, 0xB4, 0xCE],
    );
    const IID_IFOLDER_VIEW2: GUID = guid(
        0x1AF3A467,
        0x214F,
        0x4298,
        [0x90, 0x8E, 0x06, 0xB0, 0x3E, 0x0B, 0x39, 0xF9],
    );
    const IID_ISHELL_FOLDER: GUID = guid(0x000214E6, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_IENUM_IDLIST: GUID = guid(0x000214F2, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_ICONTEXT_MENU: GUID = guid(0x000214E4, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_IDATA_OBJECT: GUID = guid(0x0000010E, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_IDROP_SOURCE: GUID = guid(0x00000121, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_IUNKNOWN: GUID = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_IIMAGE_FACTORY: GUID = guid(
        0xBCC18B79,
        0xBA16,
        0x442F,
        [0x80, 0xC4, 0x8A, 0x59, 0xC3, 0x0C, 0x46, 0x3B],
    );
    const FOLDERID_DESKTOP: GUID = guid(
        0xB4BFCC3A,
        0xDB2C,
        0x424C,
        [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41],
    );
    const FOLDERID_PUBLIC_DESKTOP: GUID = guid(
        0xC4AA340D,
        0xF20F,
        0x4863,
        [0xAF, 0xEF, 0xF8, 0x7E, 0xF2, 0xE6, 0xBA, 0x25],
    );

    #[link(name = "shell32")]
    extern "system" {
        fn SHParseDisplayName(
            name: *const u16,
            bc: Ptr,
            pidl: *mut Ptr,
            sfgao_in: u32,
            sfgao_out: *mut u32,
        ) -> HRESULT;
        fn SHBindToParent(pidl: Ptr, riid: *const GUID, ppv: *mut Ptr, last: *mut Ptr) -> HRESULT;
        fn SHGetKnownFolderPath(
            id: *const GUID,
            flags: u32,
            token: Ptr,
            path: *mut *mut u16,
        ) -> HRESULT;
        fn SHCreateItemFromParsingName(
            path: *const u16,
            bc: Ptr,
            riid: *const GUID,
            ppv: *mut Ptr,
        ) -> HRESULT;
        fn ILFree(pidl: Ptr);
    }
    #[link(name = "shlwapi")]
    extern "system" {
        fn StrRetToStrW(strret: *mut StrRet, pidl: Ptr, out: *mut *mut u16) -> HRESULT;
    }
    #[link(name = "ole32")]
    extern "system" {
        fn OleInitialize(reserved: Ptr) -> HRESULT;
        fn CoCreateInstance(
            clsid: *const GUID,
            outer: Ptr,
            ctx: u32,
            iid: *const GUID,
            ppv: *mut Ptr,
        ) -> HRESULT;
        fn CoTaskMemFree(p: Ptr);
        fn DoDragDrop(data: Ptr, source: Ptr, ok: u32, effect: *mut u32) -> HRESULT;
    }

    #[repr(C, align(8))]
    struct StrRet([u8; 272]);
    #[repr(C)]
    #[derive(Default)]
    struct Variant {
        vt: u16,
        r1: u16,
        r2: u16,
        r3: u16,
        val: [u64; 2],
    }
    #[repr(C)]
    struct InvokeInfo {
        cb: u32,
        mask: u32,
        hwnd: HWND,
        verb: *const i8,
        parameters: *const i8,
        directory: *const i8,
        show: i32,
        hotkey: u32,
        icon: Ptr,
    }

    unsafe fn vfn(obj: Ptr, index: usize) -> *const c_void {
        *(*(obj as *mut *const *const c_void)).add(index)
    }
    /// An owned COM interface pointer, released on drop.
    struct Com(Ptr);
    impl Com {
        fn null() -> Self {
            Com(ptr::null_mut())
        }
        fn ok(&self) -> bool {
            !self.0.is_null()
        }
        unsafe fn query(&self, iid: &GUID) -> Com {
            let mut out = ptr::null_mut();
            if self.ok() {
                let f: extern "system" fn(Ptr, *const GUID, *mut Ptr) -> HRESULT =
                    mem::transmute(vfn(self.0, 0));
                f(self.0, iid, &mut out);
            }
            Com(out)
        }
    }
    impl Drop for Com {
        fn drop(&mut self) {
            if self.ok() {
                unsafe {
                    let f: extern "system" fn(Ptr) -> u32 = mem::transmute(vfn(self.0, 2));
                    f(self.0);
                }
            }
        }
    }
    struct Pidl(Ptr);
    impl Drop for Pidl {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CoTaskMemFree(self.0) }
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    unsafe fn from_wide(p: *const u16) -> String {
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }

    pub fn init() {
        unsafe {
            OleInitialize(ptr::null_mut());
        }
    }

    fn known_folder(id: &GUID) -> Option<PathBuf> {
        unsafe {
            let mut out = ptr::null_mut();
            let hr = SHGetKnownFolderPath(id, 0, ptr::null_mut(), &mut out);
            let result = (hr >= 0 && !out.is_null()).then(|| PathBuf::from(from_wide(out)));
            if !out.is_null() {
                CoTaskMemFree(out as Ptr);
            }
            result
        }
    }
    /// The signed-in user's Desktop folder, then the shared Public Desktop.
    pub fn desktop_folders() -> Vec<PathBuf> {
        [&FOLDERID_DESKTOP, &FOLDERID_PUBLIC_DESKTOP]
            .into_iter()
            .filter_map(known_folder)
            .collect()
    }

    thread_local! {
        static LAST_POSITION: std::cell::Cell<(i32, usize, usize)> = const { std::cell::Cell::new((0, 0, 0)) };
    }
    /// Explorer's desktop icon view, reached through IShellWindows (SWC_DESKTOP).
    pub struct DesktopView {
        view: Com,
        folder: Com,
        list: HWND,
    }
    impl DesktopView {
        pub fn acquire() -> Option<Self> {
            unsafe {
                let mut windows = ptr::null_mut();
                if CoCreateInstance(
                    &CLSID_SHELL_WINDOWS,
                    ptr::null_mut(),
                    0x17,
                    &IID_ISHELL_WINDOWS,
                    &mut windows,
                ) < 0
                {
                    return None;
                }
                let windows = Com(windows);
                let location = Variant {
                    vt: 3,
                    ..Default::default()
                }; // VT_I4 CSIDL_DESKTOP
                let root = Variant::default();
                let mut hwnd = 0i32;
                let mut dispatch = ptr::null_mut();
                let find: extern "system" fn(
                    Ptr,
                    *const Variant,
                    *const Variant,
                    i32,
                    *mut i32,
                    i32,
                    *mut Ptr,
                ) -> HRESULT = mem::transmute(vfn(windows.0, 15));
                find(windows.0, &location, &root, 8, &mut hwnd, 1, &mut dispatch);
                let dispatch = Com(dispatch);
                let provider = dispatch.query(&IID_ISERVICE_PROVIDER);
                if !provider.ok() {
                    return None;
                }
                let mut browser = ptr::null_mut();
                let query_service: extern "system" fn(
                    Ptr,
                    *const GUID,
                    *const GUID,
                    *mut Ptr,
                ) -> HRESULT = mem::transmute(vfn(provider.0, 3));
                query_service(
                    provider.0,
                    &SID_STOP_LEVEL_BROWSER,
                    &IID_ISHELL_BROWSER,
                    &mut browser,
                );
                let browser = Com(browser);
                if !browser.ok() {
                    return None;
                }
                let mut shell_view = ptr::null_mut();
                let active: extern "system" fn(Ptr, *mut Ptr) -> HRESULT =
                    mem::transmute(vfn(browser.0, 15));
                active(browser.0, &mut shell_view);
                let shell_view = Com(shell_view);
                let view = shell_view.query(&IID_IFOLDER_VIEW);
                if !view.ok() {
                    return None;
                }
                let mut folder = ptr::null_mut();
                let get_folder: extern "system" fn(Ptr, *const GUID, *mut Ptr) -> HRESULT =
                    mem::transmute(vfn(view.0, 5));
                get_folder(view.0, &IID_ISHELL_FOLDER, &mut folder);
                let mut defview: HWND = ptr::null_mut();
                let get_window: extern "system" fn(Ptr, *mut HWND) -> HRESULT =
                    mem::transmute(vfn(shell_view.0, 3));
                get_window(shell_view.0, &mut defview);
                let mut list = FindWindowExW(
                    defview,
                    ptr::null_mut(),
                    wide("SysListView32").as_ptr(),
                    ptr::null(),
                );
                if list.is_null() {
                    list = defview;
                }
                Some(DesktopView {
                    view,
                    folder: Com(folder),
                    list,
                })
            }
        }
        unsafe fn name(&self, pidl: Ptr) -> Option<String> {
            self.name_with(pidl, 0x8000) // SHGDN_FORPARSING: full path
        }
        unsafe fn name_with(&self, pidl: Ptr, flags: u32) -> Option<String> {
            if !self.folder.ok() {
                return None;
            }
            let mut ret = StrRet([0; 272]);
            let f: extern "system" fn(Ptr, Ptr, u32, *mut StrRet) -> HRESULT =
                mem::transmute(vfn(self.folder.0, 11));
            if f(self.folder.0, pidl, flags, &mut ret) < 0 {
                return None;
            }
            let mut out = ptr::null_mut();
            if StrRetToStrW(&mut ret, pidl, &mut out) < 0 || out.is_null() {
                return None;
            }
            let s = from_wide(out);
            CoTaskMemFree(out as Ptr);
            Some(s)
        }
        unsafe fn entries(&self) -> Vec<(String, POINT, Pidl)> {
            let mut out = vec![];
            let mut en = ptr::null_mut();
            let items: extern "system" fn(Ptr, u32, *const GUID, *mut Ptr) -> HRESULT =
                mem::transmute(vfn(self.view.0, 8));
            if items(self.view.0, 2, &IID_IENUM_IDLIST, &mut en) < 0 || en.is_null() {
                return out;
            }
            let en = Com(en);
            let next: extern "system" fn(Ptr, u32, *mut Ptr, *mut u32) -> HRESULT =
                mem::transmute(vfn(en.0, 3));
            let position: extern "system" fn(Ptr, Ptr, *mut POINT) -> HRESULT =
                mem::transmute(vfn(self.view.0, 11));
            loop {
                let mut pidl = ptr::null_mut();
                let mut fetched = 0u32;
                if next(en.0, 1, &mut pidl, &mut fetched) != 0 || fetched == 0 {
                    break;
                }
                let pidl = Pidl(pidl);
                let mut pt = POINT { x: 0, y: 0 };
                position(self.view.0, pidl.0, &mut pt);
                ClientToScreen(self.list, &mut pt);
                if let Some(name) = self.name(pidl.0) {
                    out.push((name, pt, pidl));
                }
                if out.len() > 20000 {
                    break;
                }
            }
            out
        }
        /// Every icon on the desktop with its screen position.
        pub fn items(&self) -> Vec<ViewItem> {
            unsafe {
                self.entries()
                    .into_iter()
                    .map(|(path, pt, pidl)| ViewItem {
                        label: self.name_with(pidl.0, 0).unwrap_or_else(|| path.clone()),
                        path,
                        pos: (pt.x, pt.y),
                    })
                    .collect()
            }
        }
        /// Move desktop icons (by full path) to screen positions.
        pub fn position(&self, moves: &[(String, (i32, i32))]) -> usize {
            if moves.is_empty() {
                return 0;
            }
            unsafe {
                let entries = self.entries();
                let mut pidls = vec![];
                let mut points = vec![];
                for (path, (x, y)) in moves {
                    if let Some((_, _, pidl)) = entries
                        .iter()
                        .find(|(p, _, _)| p.eq_ignore_ascii_case(path))
                    {
                        let mut pt = POINT { x: *x, y: *y };
                        ScreenToClient(self.list, &mut pt);
                        pidls.push(pidl.0 as *const c_void);
                        points.push(pt);
                    }
                }
                if pidls.is_empty() {
                    LAST_POSITION.with(|c| c.set((0, moves.len(), 0)));
                    return 0;
                }
                let f: extern "system" fn(
                    Ptr,
                    u32,
                    *const *const c_void,
                    *const POINT,
                    u32,
                ) -> HRESULT = mem::transmute(vfn(self.view.0, 16));
                // SVSI_POSITIONITEM
                let hr = f(
                    self.view.0,
                    pidls.len() as u32,
                    pidls.as_ptr(),
                    points.as_ptr(),
                    0x80,
                );
                LAST_POSITION.with(|c| c.set((hr, moves.len(), pidls.len())));
                if hr >= 0 {
                    pidls.len()
                } else {
                    0
                }
            }
        }
        /// (HRESULT, moves asked, items matched) of the last position() call.
        pub fn last_position_result() -> (i32, usize, usize) {
            LAST_POSITION.with(|c| c.get())
        }
        /// Current screen position of one icon.
        pub fn position_of(&self, path: &str) -> Option<(i32, i32)> {
            unsafe {
                self.entries()
                    .into_iter()
                    .find(|(p, _, _)| p.eq_ignore_ascii_case(path))
                    .map(|(_, pt, _)| (pt.x, pt.y))
            }
        }
        unsafe fn flags(&self) -> Option<(Com, u32)> {
            let v2 = self.view.query(&IID_IFOLDER_VIEW2);
            if !v2.ok() {
                return None;
            }
            let mut flags = 0u32;
            let get: extern "system" fn(Ptr, *mut u32) -> HRESULT = mem::transmute(vfn(v2.0, 25));
            (get(v2.0, &mut flags) >= 0).then_some((v2, flags))
        }
        unsafe fn set_flags(&self, mask: u32, value: u32) -> bool {
            let Some((v2, _)) = self.flags() else {
                return false;
            };
            let set: extern "system" fn(Ptr, u32, u32) -> HRESULT = mem::transmute(vfn(v2.0, 24));
            set(v2.0, mask, value) >= 0
        }
        /// Prevent Explorer from automatically rearranging desktop icons,
        /// but NEVER change the person's Align icons to grid setting.
        /// DesktopCraft snaps its own drawn loose icons during manual drops.
        pub fn disable_auto_arrange(&self) -> bool {
            unsafe {
                match self.flags() {
                    Some((_, flags)) if flags & 0x1 != 0 => self.set_flags(0x1, 0),
                    _ => false,
                }
            }
        }
        pub fn snap_to_grid(&self) -> Option<bool> {
            unsafe { self.flags().map(|(_, f)| f & 0x80000 != 0) }
        }
        pub fn auto_arrange(&self) -> Option<bool> {
            unsafe { self.flags().map(|(_, f)| f & 0x1 != 0) }
        }
        /// Explorer's "Show desktop icons" (FWF_NOICONS), used by quick-hide.
        pub fn set_icons_visible(&self, visible: bool) -> bool {
            unsafe { self.set_flags(0x1000, if visible { 0 } else { 0x1000 }) }
        }
        pub fn icons_visible(&self) -> Option<bool> {
            unsafe { self.flags().map(|(_, f)| f & 0x1000 == 0) }
        }
        /// Explorer's own desktop icon spacing (physical pixels).
        pub fn spacing(&self) -> Option<(i32, i32)> {
            unsafe {
                let mut pt = POINT { x: 0, y: 0 };
                let f: extern "system" fn(Ptr, *mut POINT) -> HRESULT =
                    mem::transmute(vfn(self.view.0, 12));
                (f(self.view.0, &mut pt) >= 0 && pt.x > 0 && pt.y > 0).then_some((pt.x, pt.y))
            }
        }
        pub fn list_window(&self) -> HWND {
            self.list
        }

        /// Bridge phase 1: resolve a real Explorer-owned Desktop icon to its
        /// IFolderView item index. This is read-only: no selection or position
        /// is changed. A Box pin whose file is not on the Windows Desktop is
        /// not represented by this Shell View and must not be faked here.
        fn index_of(&self, path: &str) -> Option<i32> {
            unsafe {
                let mut count = 0i32;
                let item_count: extern "system" fn(Ptr, u32, *mut i32) -> HRESULT =
                    mem::transmute(vfn(self.view.0, 7));
                if item_count(self.view.0, 0, &mut count) < 0 || !(0..=20000).contains(&count) {
                    return None;
                }
                let item: extern "system" fn(Ptr, i32, *mut Ptr) -> HRESULT =
                    mem::transmute(vfn(self.view.0, 6));
                for index in 0..count {
                    let mut pidl = ptr::null_mut();
                    if item(self.view.0, index, &mut pidl) < 0 || pidl.is_null() {
                        continue;
                    }
                    let pidl = Pidl(pidl);
                    if self
                        .name(pidl.0)
                        .is_some_and(|p| p.eq_ignore_ascii_case(path))
                    {
                        return Some(index);
                    }
                }
                None
            }
        }

        /// Opt-in Explorer-first prototype: selection uses Microsoft's
        /// IFolderView::SelectItem, not input injection, undocumented Shell
        /// hooks, Explorer code injection or icon repositioning. The selection
        /// is the only side effect, and only an explicit CLI command calls it.
        pub fn select_explorer_icon(&self, path: &str) -> Result<(), String> {
            let index = self
                .index_of(path)
                .ok_or("This file is not an icon in Explorer's Desktop View")?;
            unsafe {
                let f: extern "system" fn(Ptr, i32, u32) -> HRESULT =
                    mem::transmute(vfn(self.view.0, 15));
                // SELECT | DESELECTOTHERS | FOCUSED | SELECTIONMARK
                let hr = f(self.view.0, index, 0x1 | 0x4 | 0x10 | 0x40);
                if hr < 0 {
                    return Err(format!(
                        "IFolderView::SelectItem failed (0x{:08X})",
                        hr as u32
                    ));
                }
            }
            Ok(())
        }

        /// EXPERIMENTAL: ask Explorer's existing Desktop list view to handle
        /// its own keyboard-style WM_CONTEXTMENU for a selected icon. This is
        /// not a guaranteed supported path to Windows 11's modern menu: some
        /// Explorer builds show classic/background menus or no popup at all.
        /// Never send this if DesktopCraft/Windows has hidden Desktop icons.
        pub fn request_explorer_context_menu(&self, path: &str) -> Result<(), String> {
            if self.icons_visible() != Some(true) || unsafe { IsWindowVisible(self.list) } == 0 {
                return Err(
                    "Explorer Desktop icons are hidden; modern-menu bridge is unavailable".into(),
                );
            }
            self.select_explorer_icon(path)?;
            unsafe {
                if PostMessageW(self.list, WM_CONTEXTMENU, self.list as usize, -1) == 0 {
                    return Err("Explorer Desktop View rejected the context-menu request".into());
                }
            }
            Ok(())
        }

        pub fn explorer_bridge_probe(&self, target: Option<&str>) -> serde_json::Value {
            serde_json::json!({
                "bridge":"explorer-desktop-view-lab",
                "available":true,
                "icons_visible":self.icons_visible(),
                "item_count":self.items().len(),
                "target_resolved":target.map(|p|self.index_of(p).is_some()),
                "select_item_supported":true,
                "menu_dispatch_experimental":true,
                "explorer_code_injection":false,
                "icon_positions_modified":false,
                "limitation":"Selection is documented; modern Windows 11 menu dispatch is experimental and not guaranteed."
            })
        }
    }

    unsafe fn parent_and_child(path: &str) -> Option<(Com, Ptr, Pidl)> {
        let mut pidl = ptr::null_mut();
        if SHParseDisplayName(
            wide(&path.replace('/', "\\")).as_ptr(),
            ptr::null_mut(),
            &mut pidl,
            0,
            ptr::null_mut(),
        ) < 0
            || pidl.is_null()
        {
            return None;
        }
        let pidl = Pidl(pidl);
        let mut folder = ptr::null_mut();
        let mut child = ptr::null_mut();
        if SHBindToParent(pidl.0, &IID_ISHELL_FOLDER, &mut folder, &mut child) < 0 {
            return None;
        }
        Some((Com(folder), child, pidl))
    }

    /// Explorer's own context menu for a file or folder, at a screen position.
    pub fn context_menu(path: &str, x: i32, y: i32) -> MenuChoice {
        let owner = crate::host::window();
        unsafe {
            let Some((folder, child, _pidl)) = parent_and_child(path) else {
                return MenuChoice::Nothing;
            };
            let mut cm = ptr::null_mut();
            let get_ui: extern "system" fn(
                Ptr,
                HWND,
                u32,
                *const Ptr,
                *const GUID,
                *mut u32,
                *mut Ptr,
            ) -> HRESULT = mem::transmute(vfn(folder.0, 10));
            if get_ui(
                folder.0,
                owner,
                1,
                &child,
                &IID_ICONTEXT_MENU,
                ptr::null_mut(),
                &mut cm,
            ) < 0
                || cm.is_null()
            {
                return MenuChoice::Nothing;
            }
            let cm = Com(cm);
            let menu = CreatePopupMenu();
            let query: extern "system" fn(Ptr, HMENU, u32, u32, u32, u32) -> HRESULT =
                mem::transmute(vfn(cm.0, 3));
            query(cm.0, menu, 0, 1, 0x7FFF, 0x10 /* CMF_CANRENAME */);
            let cmd = track(&cm, menu, x, y);
            let owner = menu_owner();
            let mut choice = MenuChoice::Nothing;
            if cmd > 0 {
                let mut verb = [0u16; 64];
                let get_string: extern "system" fn(
                    Ptr,
                    usize,
                    u32,
                    *mut u32,
                    *mut u16,
                    u32,
                ) -> HRESULT = mem::transmute(vfn(cm.0, 5));
                get_string(
                    cm.0,
                    (cmd - 1) as usize,
                    4, /* GCS_VERBW */
                    ptr::null_mut(),
                    verb.as_mut_ptr(),
                    64,
                );
                let verb = String::from_utf16_lossy(
                    &verb[..verb.iter().position(|&c| c == 0).unwrap_or(0)],
                );
                if verb.eq_ignore_ascii_case("rename") {
                    choice = MenuChoice::Rename;
                } else {
                    let info = InvokeInfo {
                        cb: mem::size_of::<InvokeInfo>() as u32,
                        mask: 0,
                        hwnd: owner,
                        verb: (cmd - 1) as usize as *const i8,
                        parameters: ptr::null(),
                        directory: ptr::null(),
                        show: SW_SHOWNORMAL,
                        hotkey: 0,
                        icon: ptr::null_mut(),
                    };
                    let invoke: extern "system" fn(Ptr, *const InvokeInfo) -> HRESULT =
                        mem::transmute(vfn(cm.0, 4));
                    invoke(cm.0, &info);
                    choice = MenuChoice::Invoked;
                }
            }
            DestroyMenu(menu);
            choice
        }
    }

    // ---- menus: a small owner window that forwards owner-drawn/submenu messages
    // to the shell's IContextMenu3/2, so "New", "Send to", "Open with" fill in. ----
    const IID_ICONTEXT_MENU2: GUID = guid(0x000214F4, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_ICONTEXT_MENU3: GUID = guid(
        0xBCFCE0A0,
        0xEC17,
        0x11D0,
        [0x8D, 0x10, 0x00, 0xA0, 0xC9, 0x0F, 0x27, 0x19],
    );
    const IID_ISHELL_VIEW: GUID = guid(0x000214E3, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    static CURRENT_MENU: std::sync::atomic::AtomicPtr<c_void> =
        std::sync::atomic::AtomicPtr::new(ptr::null_mut());
    static CURRENT_VERSION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    unsafe extern "system" fn owner_proc(hwnd: HWND, msg: u32, w: usize, l: isize) -> isize {
        let cm = CURRENT_MENU.load(std::sync::atomic::Ordering::Relaxed);
        if !cm.is_null() && matches!(msg, 0x116 | 0x117 | 0x2B | 0x2C | 0x120) {
            if CURRENT_VERSION.load(std::sync::atomic::Ordering::Relaxed) == 3 {
                let f: extern "system" fn(Ptr, u32, usize, isize, *mut isize) -> HRESULT =
                    mem::transmute(vfn(cm, 7));
                let mut result = 0isize;
                if f(cm, msg, w, l, &mut result) >= 0 {
                    return result;
                }
            } else {
                let f: extern "system" fn(Ptr, u32, usize, isize) -> HRESULT =
                    mem::transmute(vfn(cm, 6));
                if f(cm, msg, w, l) >= 0 {
                    return 0;
                }
            }
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
    fn menu_owner() -> HWND {
        thread_local! {
            static OWNER: std::cell::Cell<HWND> = const { std::cell::Cell::new(ptr::null_mut()) };
        }
        OWNER.with(|o| {
            if o.get().is_null() {
                unsafe {
                    let class = wide("DesktopCraftMenuOwner");
                    let mut wc: WNDCLASSW = mem::zeroed();
                    wc.lpfnWndProc = Some(owner_proc);
                    wc.hInstance = winapi::um::libloaderapi::GetModuleHandleW(ptr::null());
                    wc.lpszClassName = class.as_ptr();
                    RegisterClassW(&wc);
                    let hwnd = CreateWindowExW(
                        WS_EX_TOOLWINDOW,
                        class.as_ptr(),
                        wide("").as_ptr(),
                        WS_POPUP,
                        0,
                        0,
                        0,
                        0,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        wc.hInstance,
                        ptr::null_mut(),
                    );
                    o.set(hwnd);
                }
            }
            o.get()
        })
    }
    /// Show a popup menu built by a shell IContextMenu; returns the chosen id.
    unsafe fn track(cm: &Com, menu: HMENU, x: i32, y: i32) -> u32 {
        let owner = menu_owner();
        let v3 = cm.query(&IID_ICONTEXT_MENU3);
        let v2 = if v3.ok() {
            Com::null()
        } else {
            cm.query(&IID_ICONTEXT_MENU2)
        };
        let (current, version) = if v3.ok() {
            (v3.0, 3)
        } else if v2.ok() {
            (v2.0, 2)
        } else {
            (ptr::null_mut(), 0)
        };
        CURRENT_MENU.store(current, std::sync::atomic::Ordering::Relaxed);
        CURRENT_VERSION.store(version, std::sync::atomic::Ordering::Relaxed);
        SetForegroundWindow(owner);
        let cmd = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            x,
            y,
            owner,
            ptr::null_mut(),
        ) as u32;
        PostMessageW(owner, WM_NULL, 0, 0);
        CURRENT_MENU.store(ptr::null_mut(), std::sync::atomic::Ordering::Relaxed);
        drop(v3);
        drop(v2);
        cmd
    }
    #[repr(C)]
    struct InvokeInfoEx {
        cb: u32,
        mask: u32,
        hwnd: HWND,
        verb: *const i8,
        parameters: *const i8,
        directory: *const i8,
        show: i32,
        hotkey: u32,
        icon: Ptr,
        title: *const i8,
        verb_w: *const u16,
        parameters_w: *const u16,
        directory_w: *const u16,
        title_w: *const u16,
        point: POINT,
    }

    pub enum DesktopChoice {
        Nothing,
        Windows,
        Ours(usize),
    }
    /// Windows' own desktop background menu (View, Sort by, Refresh, New,
    /// Display settings, Personalize and installed extensions), with the given
    /// DesktopCraft items added after it, as the original adds "Configure".
    pub fn desktop_menu(x: i32, y: i32, ours: &[&str]) -> DesktopChoice {
        unsafe {
            let menu = CreatePopupMenu();
            let mut cm = Com::null();
            if let Some(view) = DesktopView::acquire() {
                let sv = view.view.query(&IID_ISHELL_VIEW);
                if sv.ok() {
                    let mut out = ptr::null_mut();
                    let get: extern "system" fn(Ptr, u32, *const GUID, *mut Ptr) -> HRESULT =
                        mem::transmute(vfn(sv.0, 15));
                    get(
                        sv.0,
                        0, /* SVGIO_BACKGROUND */
                        &IID_ICONTEXT_MENU,
                        &mut out,
                    );
                    cm = Com(out);
                }
            }
            if cm.ok() {
                let query: extern "system" fn(Ptr, HMENU, u32, u32, u32, u32) -> HRESULT =
                    mem::transmute(vfn(cm.0, 3));
                query(cm.0, menu, 0, 1, 0x6FFF, 0);
                AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
            }
            for (i, label) in ours.iter().enumerate() {
                AppendMenuW(menu, MF_STRING, 0x7000 + i, wide(label).as_ptr());
            }
            let cmd = if cm.ok() {
                track(&cm, menu, x, y)
            } else {
                let owner = menu_owner();
                SetForegroundWindow(owner);
                TrackPopupMenuEx(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    x,
                    y,
                    owner,
                    ptr::null_mut(),
                ) as u32
            };
            DestroyMenu(menu);
            if cmd >= 0x7000 {
                return DesktopChoice::Ours((cmd - 0x7000) as usize);
            }
            if cmd > 0 && cm.ok() {
                // CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE: "New" items land where
                // the person clicked.
                let info = InvokeInfoEx {
                    cb: mem::size_of::<InvokeInfoEx>() as u32,
                    mask: 0x4000 | 0x2000_0000,
                    hwnd: menu_owner(),
                    verb: (cmd - 1) as usize as *const i8,
                    parameters: ptr::null(),
                    directory: ptr::null(),
                    show: SW_SHOWNORMAL,
                    hotkey: 0,
                    icon: ptr::null_mut(),
                    title: ptr::null(),
                    verb_w: (cmd - 1) as usize as *const u16,
                    parameters_w: ptr::null(),
                    directory_w: ptr::null(),
                    title_w: ptr::null(),
                    point: POINT { x, y },
                };
                let invoke: extern "system" fn(Ptr, *const InvokeInfoEx) -> HRESULT =
                    mem::transmute(vfn(cm.0, 4));
                invoke(cm.0, &info);
                return DesktopChoice::Windows;
            }
            DesktopChoice::Nothing
        }
    }

    // A minimal IDropSource: drop on button release, cancel on Escape.
    #[repr(C)]
    struct DropVtbl {
        query: unsafe extern "system" fn(Ptr, *const GUID, *mut Ptr) -> HRESULT,
        add_ref: unsafe extern "system" fn(Ptr) -> u32,
        release: unsafe extern "system" fn(Ptr) -> u32,
        query_continue: unsafe extern "system" fn(Ptr, i32, u32) -> HRESULT,
        feedback: unsafe extern "system" fn(Ptr, u32) -> HRESULT,
    }
    #[repr(C)]
    struct DropSource {
        vtbl: *const DropVtbl,
    }
    unsafe extern "system" fn ds_query(this: Ptr, iid: *const GUID, out: *mut Ptr) -> HRESULT {
        let iid = &*iid;
        let same = |g: &GUID| {
            g.Data1 == iid.Data1
                && g.Data2 == iid.Data2
                && g.Data3 == iid.Data3
                && g.Data4 == iid.Data4
        };
        if same(&IID_IUNKNOWN) || same(&IID_IDROP_SOURCE) {
            *out = this;
            0
        } else {
            *out = ptr::null_mut();
            0x80004002u32 as i32
        }
    }
    unsafe extern "system" fn ds_ref(_: Ptr) -> u32 {
        1
    }
    unsafe extern "system" fn ds_continue(_: Ptr, escape: i32, keys: u32) -> HRESULT {
        if escape != 0 {
            0x00040101 // DRAGDROP_S_CANCEL
        } else if keys & 0x1 == 0 {
            0x00040100 // DRAGDROP_S_DROP
        } else {
            0
        }
    }
    unsafe extern "system" fn ds_feedback(_: Ptr, _: u32) -> HRESULT {
        0x00040102 // DRAGDROP_S_USEDEFAULTCURSORS
    }
    static DROP_VTBL: DropVtbl = DropVtbl {
        query: ds_query,
        add_ref: ds_ref,
        release: ds_ref,
        query_continue: ds_continue,
        feedback: ds_feedback,
    };

    /// Start a normal Windows drag of one file or folder (blocks until dropped).
    /// Returns the drop effect (0 none, 1 copy, 2 move, 4 link).
    pub fn drag(path: &str) -> u32 {
        unsafe {
            let Some((folder, child, _pidl)) = parent_and_child(path) else {
                return 0;
            };
            let mut data = ptr::null_mut();
            let get_ui: extern "system" fn(
                Ptr,
                HWND,
                u32,
                *const Ptr,
                *const GUID,
                *mut u32,
                *mut Ptr,
            ) -> HRESULT = mem::transmute(vfn(folder.0, 10));
            if get_ui(
                folder.0,
                ptr::null_mut(),
                1,
                &child,
                &IID_IDATA_OBJECT,
                ptr::null_mut(),
                &mut data,
            ) < 0
                || data.is_null()
            {
                return 0;
            }
            let data = Com(data);
            let mut source = DropSource { vtbl: &DROP_VTBL };
            let mut effect = 0u32;
            DoDragDrop(data.0, &mut source as *mut _ as Ptr, 1 | 2 | 4, &mut effect);
            effect
        }
    }

    fn double_null(paths: &[&str]) -> Vec<u16> {
        let mut v = vec![];
        for p in paths {
            v.extend(p.encode_utf16());
            v.push(0);
        }
        v.push(0);
        v
    }
    unsafe fn file_op(owner: HWND, func: u16, from: &str, to: Option<&str>) -> Result<(), String> {
        let from = double_null(&[from]);
        let to = to.map(|t| double_null(&[t]));
        let mut op: SHFILEOPSTRUCTW = mem::zeroed();
        op.hwnd = owner;
        op.wFunc = func as u32;
        op.pFrom = from.as_ptr();
        op.pTo = to.as_ref().map_or(ptr::null(), |t| t.as_ptr());
        // A drop on the drawn Recycle Bin must never silently become a
        // permanent deletion on a volume where recycling is unavailable.
        let warning = if func == FO_DELETE {
            FOF_WANTNUKEWARNING
        } else {
            0
        };
        op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | warning) as u16;
        let r = SHFileOperationW(&mut op);
        if r != 0 {
            Err(format!(
                "Windows could not complete the file operation (code {r})."
            ))
        } else if op.fAnyOperationsAborted != 0 {
            Err("The file operation was cancelled.".into())
        } else {
            Ok(())
        }
    }
    /// Windows shell move (including between drives), with Explorer's undo
    /// support. Refuse to overwrite or auto-rename any existing destination.
    pub fn move_into_folder(path: &Path, folder: &Path) -> Result<PathBuf, String> {
        let name = path.file_name().ok_or("Invalid file or folder name")?;
        let target = folder.join(name);
        if path == target {
            return Ok(target);
        }
        if !path.exists() {
            return Err("The source file no longer exists".into());
        }
        if path.is_dir() && folder.starts_with(path) {
            return Err("Cannot move a folder into itself".into());
        }
        if target.exists() {
            return Err(format!(
                "Destination already contains '{}'; no file was replaced",
                name.to_string_lossy()
            ));
        }
        if !folder.is_dir() {
            return Err("Destination folder no longer exists".into());
        }
        unsafe {
            file_op(
                crate::host::window(),
                FO_MOVE,
                &path.to_string_lossy(),
                Some(&folder.to_string_lossy()),
            )?;
        }
        if !target.exists() {
            return Err("Windows reported a move, but the destination could not be verified; check both folders".into());
        }
        Ok(target)
    }

    #[repr(C)]
    struct OpenAsInfo {
        path: *const u16,
        class: *const u16,
        flags: u32,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHOpenWithDialog(hwnd: HWND, info: *const OpenAsInfo) -> HRESULT;
        fn SHObjectProperties(
            hwnd: HWND,
            object_type: u32,
            object_name: *const u16,
            property_page: *const u16,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Ptr;
        fn GlobalLock(handle: Ptr) -> Ptr;
        fn GlobalUnlock(handle: Ptr) -> i32;
        fn GlobalFree(handle: Ptr) -> Ptr;
    }
    pub fn open_with(path: &str) -> Result<(), String> {
        if !Path::new(path).exists() {
            return Err("File no longer exists".into());
        }
        let path = wide(path);
        let info = OpenAsInfo {
            path: path.as_ptr(),
            class: ptr::null(),
            flags: 0x4, /* OAIF_EXEC */
        };
        let result = unsafe { SHOpenWithDialog(crate::host::window(), &info) };
        if result >= 0 {
            Ok(())
        } else {
            Err(format!(
                "Windows Open with failed (0x{:08X})",
                result as u32
            ))
        }
    }
    pub fn properties(path: &str) -> Result<(), String> {
        if !Path::new(path).exists() {
            return Err("File no longer exists".into());
        }
        // std::fs accepts mixed separators, but SHObjectProperties can reject
        // paths with forward slashes (seen in demo and imported Workspace).
        let normalized = path.replace('/', "\\");
        let path = wide(&normalized);
        // SHOP_FILEPATH, the real Windows file/folder Properties dialog.
        let ok =
            unsafe { SHObjectProperties(crate::host::window(), 2, path.as_ptr(), ptr::null()) };
        if ok != 0 {
            Ok(())
        } else {
            Err("Windows could not show Properties for this item".into())
        }
    }
    pub fn reveal_in_explorer(path: &str) -> Result<(), String> {
        if !Path::new(path).exists() {
            return Err("File no longer exists".into());
        }
        std::process::Command::new("explorer.exe")
            .arg(format!("/select,{path}"))
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Explorer could not open: {e}"))
    }
    /// CF_HDROP uses a DROPFILES header and double-NUL UTF-16 paths.
    fn encode_file_drop(paths: &[String]) -> Vec<u8> {
        let mut payload = vec![0u8; 20];
        payload[0..4].copy_from_slice(&20u32.to_le_bytes());
        payload[16..20].copy_from_slice(&1u32.to_le_bytes()); // fWide TRUE
        for path in paths {
            for c in path
                .replace('/', std::path::MAIN_SEPARATOR_STR)
                .encode_utf16()
            {
                payload.extend_from_slice(&c.to_le_bytes());
            }
            payload.extend_from_slice(&0u16.to_le_bytes());
        }
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }
    #[test]
    fn windows_file_clipboard_payload_is_unicode_and_double_nul_terminated() {
        let files = vec![
            "C:/Users/Test/Desktop/งานไทย.txt".to_string(),
            "C:/Users/Test/Desktop/backup.zip".to_string(),
        ];
        let payload = encode_file_drop(&files);
        assert_eq!(&payload[0..4], &20u32.to_le_bytes());
        assert_eq!(&payload[16..20], &1u32.to_le_bytes());
        let data: Vec<u16> = payload[20..]
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        assert_eq!(data[data.len() - 2..], [0, 0]);
        let joined = String::from_utf16_lossy(&data);
        assert!(joined.contains("งานไทย.txt"));
        assert!(joined.contains("backup.zip"));
    }
    /// Read only file objects from Explorer's current clipboard; never treat
    /// text or a shell PIDL as a path. Own no clipboard storage.
    pub fn read_file_clipboard() -> Result<Option<FileClipboard>, String> {
        unsafe {
            if OpenClipboard(crate::host::window()) == 0 {
                return Err("The Windows clipboard is busy; retry Paste".into());
            }
            let result = (|| -> Result<Option<FileClipboard>, String> {
                let drop = GetClipboardData(15); // CF_HDROP
                if drop.is_null() {
                    return Ok(None);
                }
                let files = drop as HDROP;
                let count = DragQueryFileW(files, u32::MAX, ptr::null_mut(), 0);
                if count == 0 {
                    return Ok(None);
                }
                if count > 4096 {
                    return Err("Too many clipboard files".into());
                }
                let mut paths = Vec::with_capacity(count as usize);
                for i in 0..count {
                    let n = DragQueryFileW(files, i, ptr::null_mut(), 0);
                    if n == 0 || n > 32767 {
                        return Err("Invalid clipboard file path".into());
                    }
                    let mut buf = vec![0u16; n as usize + 1];
                    if DragQueryFileW(files, i, buf.as_mut_ptr(), buf.len() as u32) != n {
                        return Err("Could not read a clipboard file path".into());
                    }
                    paths.push(PathBuf::from(String::from_utf16_lossy(&buf[..n as usize])));
                }
                let mut cut = false;
                let cf = RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr());
                if cf != 0 {
                    let effect = GetClipboardData(cf);
                    if !effect.is_null() {
                        let data = GlobalLock(effect as Ptr);
                        if !data.is_null() {
                            cut = ptr::read_unaligned(data as *const u32) & 2 != 0;
                            GlobalUnlock(effect as Ptr);
                        }
                    }
                }
                Ok(Some(FileClipboard { paths, cut }))
            })();
            CloseClipboard();
            result
        }
    }

    /// Validate every source and destination before Windows Shell starts any
    /// file operation. No overwrite, recursive folder move or partial batch.
    pub fn preflight_folder_paste(
        paths: &[PathBuf],
        folder: &Path,
        cut: bool,
    ) -> Result<(), String> {
        if !folder.is_dir() {
            return Err("Target folder does not exist".into());
        }
        if paths.is_empty() {
            return Err("Clipboard has no files".into());
        }
        let mut names = std::collections::HashSet::new();
        for path in paths {
            if !path.exists() {
                return Err(format!("Clipboard source is missing: {}", path.display()));
            }
            let name = path
                .file_name()
                .ok_or("Clipboard source has no file name")?;
            if !names.insert(name.to_string_lossy().to_lowercase()) {
                return Err("Clipboard contains multiple items with the same name".into());
            }
            let target = folder.join(name);
            if path.as_path() == target.as_path() {
                return Err("The file is already in this folder".into());
            }
            if target.exists() {
                return Err(format!(
                    "Destination already has '{}'; nothing was replaced",
                    name.to_string_lossy()
                ));
            }
            if path.is_dir()
                && (folder.starts_with(path)
                    || folder
                        .canonicalize()
                        .ok()
                        .zip(path.canonicalize().ok())
                        .is_some_and(|(f, p)| f.starts_with(p)))
            {
                return Err("Cannot paste a folder into itself or its subfolder".into());
            }
            if cut && path.parent().is_some_and(|p| p == folder) {
                return Err("Cannot move a file into its current folder".into());
            }
        }
        Ok(())
    }
    pub fn paste_files_into_folder(
        clipboard: &FileClipboard,
        folder: &Path,
    ) -> Result<usize, String> {
        preflight_folder_paste(&clipboard.paths, folder, clipboard.cut)?;
        let names: Vec<String> = clipboard
            .paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let from = double_null(&refs);
        let dst = double_null(&[&folder.to_string_lossy()]);
        unsafe {
            let mut op: SHFILEOPSTRUCTW = mem::zeroed();
            op.hwnd = crate::host::window();
            op.wFunc = if clipboard.cut {
                FO_MOVE as u32
            } else {
                FO_COPY as u32
            };
            op.pFrom = from.as_ptr();
            op.pTo = dst.as_ptr();
            op.fFlags = FOF_ALLOWUNDO as u16; // Explorer handles confirmation
            let result = SHFileOperationW(&mut op);
            if result != 0 {
                return Err(format!(
                    "Windows file {} failed (code {result})",
                    if clipboard.cut { "move" } else { "copy" }
                ));
            }
            if op.fAnyOperationsAborted != 0 {
                return Err("Windows file operation was cancelled".into());
            }
        }
        let success = clipboard
            .paths
            .iter()
            .filter(|p| p.file_name().is_some_and(|name| folder.join(name).exists()))
            .count();
        if success != clipboard.paths.len() {
            return Err(format!(
                "Windows completed {success}/{} file operations; check both folders",
                clipboard.paths.len()
            ));
        }
        Ok(success)
    }

    #[test]
    fn folder_paste_preflight_rejects_conflicts_cycles_and_missing_sources() {
        let root = std::env::temp_dir().join(format!(
            "desktopcraft-paste-validation-{}",
            std::process::id()
        ));
        let from = root.join("from");
        let dst = root.join("to");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        let source = from.join("ภาษาไทย.txt");
        std::fs::write(&source, b"safe").unwrap();
        assert!(preflight_folder_paste(&[source.clone()], &dst, false).is_ok());
        std::fs::write(dst.join("ภาษาไทย.txt"), b"existing").unwrap();
        assert!(preflight_folder_paste(&[source.clone()], &dst, false).is_err());
        assert_eq!(std::fs::read(dst.join("ภาษาไทย.txt")).unwrap(), b"existing");
        assert!(preflight_folder_paste(&[from.join("missing.zip")], &dst, true).is_err());
        let inner = from.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        assert!(preflight_folder_paste(&[from.clone()], &inner, true).is_err());
        assert!(preflight_folder_paste(&[source.clone(), source.clone()], &dst, false).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Actual Explorer-compatible Windows file clipboard. No disk mutation:
    /// Explorer performs moves only when the user pastes a cut file.
    pub fn copy_files_to_clipboard(paths: &[String], cut: bool) -> Result<(), String> {
        if paths.is_empty() || paths.iter().any(|p| !Path::new(p).exists()) {
            return Err("Select existing files or folders first".into());
        }
        let payload = encode_file_drop(paths);
        unsafe {
            if OpenClipboard(crate::host::window()) == 0 {
                return Err("Windows clipboard is currently busy".into());
            }
            let result = (|| -> Result<(), String> {
                if EmptyClipboard() == 0 {
                    return Err("Could not clear clipboard".into());
                }
                let mem = GlobalAlloc(0x2, payload.len()); // GMEM_MOVEABLE
                if mem.is_null() {
                    return Err("Clipboard allocation failed".into());
                }
                let data = GlobalLock(mem);
                if data.is_null() {
                    GlobalFree(mem);
                    return Err("Clipboard could not lock data".into());
                }
                ptr::copy_nonoverlapping(payload.as_ptr(), data as *mut u8, payload.len());
                GlobalUnlock(mem);
                if SetClipboardData(15, mem as _).is_null() {
                    GlobalFree(mem);
                    return Err("Windows rejected the file list".into());
                }
                // Standard Shell Preferred DropEffect: 2=move, 1=copy.
                let cf = RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr());
                if cf != 0 {
                    let mem = GlobalAlloc(0x2, 4);
                    if !mem.is_null() {
                        let data = GlobalLock(mem);
                        if !data.is_null() {
                            *(data as *mut u32) = if cut { 2 } else { 1 };
                            GlobalUnlock(mem);
                            if SetClipboardData(cf, mem as _).is_null() {
                                GlobalFree(mem);
                            }
                        } else {
                            GlobalFree(mem);
                        }
                    }
                }
                Ok(())
            })();
            CloseClipboard();
            result
        }
    }
    pub fn rename(path: &str, new_name: &str) -> Result<PathBuf, String> {
        let source = PathBuf::from(path);
        let target = source.with_file_name(new_name);
        unsafe {
            file_op(
                crate::host::window(),
                FO_RENAME,
                path,
                Some(&target.to_string_lossy()),
            )?
        };
        Ok(target)
    }
    /// Send to the Recycle Bin (Windows asks for confirmation as usual).
    pub fn recycle(path: &str) -> Result<(), String> {
        unsafe { file_op(crate::host::window(), FO_DELETE, path, None) }
    }

    /// Explorer's icon (or thumbnail) for a file, as RGBA pixels.
    pub fn image(path: &str, size: i32) -> Option<(u32, u32, Vec<u8>)> {
        unsafe {
            let mut factory = ptr::null_mut();
            if SHCreateItemFromParsingName(
                wide(&path.replace('/', "\\")).as_ptr(),
                ptr::null_mut(),
                &IID_IIMAGE_FACTORY,
                &mut factory,
            ) < 0
            {
                return None;
            }
            let factory = Com(factory);
            let mut bitmap = ptr::null_mut();
            let get: extern "system" fn(Ptr, SIZE, u32, *mut HBITMAP) -> HRESULT =
                mem::transmute(vfn(factory.0, 3));
            // SIIGBF_BIGGERSIZEOK: icons or thumbnails, as Explorer shows them.
            if get(factory.0, SIZE { cx: size, cy: size }, 0x1, &mut bitmap) < 0 || bitmap.is_null()
            {
                return None;
            }
            let mut bm: BITMAP = mem::zeroed();
            GetObjectW(
                bitmap as _,
                mem::size_of::<BITMAP>() as i32,
                &mut bm as *mut _ as _,
            );
            let (w, h) = (bm.bmWidth.max(1), bm.bmHeight.abs().max(1));
            let mut info: BITMAPINFO = mem::zeroed();
            info.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            let dc = GetDC(ptr::null_mut());
            let lines = GetDIBits(
                dc,
                bitmap,
                0,
                h as u32,
                pixels.as_mut_ptr() as _,
                &mut info,
                DIB_RGB_COLORS,
            );
            ReleaseDC(ptr::null_mut(), dc);
            DeleteObject(bitmap as _);
            if lines == 0 {
                return None;
            }
            let has_alpha = pixels.chunks(4).any(|p| p[3] != 0);
            for p in pixels.chunks_mut(4) {
                p.swap(0, 2);
                if !has_alpha {
                    p[3] = 255;
                } else if p[3] > 0 && p[3] < 255 {
                    // The shell returns premultiplied alpha.
                    for c in 0..3 {
                        p[c] = ((p[c] as u32 * 255) / p[3] as u32).min(255) as u8;
                    }
                }
            }
            Some((w as u32, h as u32, pixels))
        }
    }
}

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
mod fallback {
    use super::*;
    pub fn init() {}
    pub fn desktop_folders() -> Vec<PathBuf> {
        vec![]
    }
    pub struct DesktopView;
    impl DesktopView {
        pub fn acquire() -> Option<Self> {
            None
        }
        pub fn items(&self) -> Vec<ViewItem> {
            vec![]
        }
        pub fn select_explorer_icon(&self, _: &str) -> Result<(), String> {
            Err("Explorer Desktop View is only available on Windows".into())
        }
        pub fn request_explorer_context_menu(&self, _: &str) -> Result<(), String> {
            Err("Explorer Desktop View is only available on Windows".into())
        }
        pub fn explorer_bridge_probe(&self, _: Option<&str>) -> serde_json::Value {
            serde_json::json!({"bridge":"explorer-desktop-view-lab","available":false})
        }
        pub fn position(&self, _: &[(String, (i32, i32))]) -> usize {
            0
        }
        pub fn disable_auto_arrange(&self) -> bool {
            false
        }
        pub fn set_icons_visible(&self, _: bool) -> bool {
            false
        }
        pub fn icons_visible(&self) -> Option<bool> {
            None
        }
        pub fn auto_arrange(&self) -> Option<bool> {
            None
        }
        pub fn snap_to_grid(&self) -> Option<bool> {
            None
        }
        pub fn spacing(&self) -> Option<(i32, i32)> {
            None
        }
    }
    pub fn image(_: &str, _: i32) -> Option<(u32, u32, Vec<u8>)> {
        None
    }
    pub fn context_menu(_: &str, _: i32, _: i32) -> MenuChoice {
        MenuChoice::Nothing
    }
    pub enum DesktopChoice {
        Nothing,
        Windows,
        Ours(usize),
    }
    pub fn desktop_menu(_: i32, _: i32, _: &[&str]) -> DesktopChoice {
        DesktopChoice::Nothing
    }
    pub fn drag(_: &str) -> u32 {
        0
    }
    pub fn move_into_folder(_: &Path, _: &Path) -> Result<PathBuf, String> {
        Err("Only available on Windows".into())
    }
    pub fn open_with(_: &str) -> Result<(), String> {
        Err("Only available on Windows".into())
    }
    pub fn properties(_: &str) -> Result<(), String> {
        Err("Only available on Windows".into())
    }
    pub fn reveal_in_explorer(_: &str) -> Result<(), String> {
        Err("Only available on Windows".into())
    }
    pub fn copy_files_to_clipboard(_: &[String], _: bool) -> Result<(), String> {
        Err("Only available on Windows".into())
    }
    pub fn read_file_clipboard() -> Result<Option<FileClipboard>, String> {
        Err("Only available on Windows".into())
    }
    pub fn preflight_folder_paste(_: &[PathBuf], _: &Path, _: bool) -> Result<(), String> {
        Err("Only available on Windows".into())
    }
    pub fn paste_files_into_folder(_: &FileClipboard, _: &Path) -> Result<usize, String> {
        Err("Only available on Windows".into())
    }
    pub fn rename(_: &str, _: &str) -> Result<PathBuf, String> {
        Err("Only available on Windows".into())
    }
    pub fn recycle(_: &str) -> Result<(), String> {
        Err("Only available on Windows".into())
    }
}
#[cfg(not(windows))]
pub use fallback::*;

/// Every visible item directly on the desktop (user and Public Desktop),
/// skipping hidden/system files such as desktop.ini.
pub fn desktop_entries() -> Vec<PathBuf> {
    let mut out = vec![];
    for folder in desktop_folders() {
        let Ok(read) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.eq_ignore_ascii_case("desktop.ini") || name.starts_with('~') {
                continue;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if let Ok(meta) = entry.metadata() {
                    if meta.file_attributes() & 0x6 != 0 {
                        continue;
                    }
                }
            }
            out.push(path);
        }
    }
    out
}

/// Whether a path sits directly in one of the desktop folders.
pub fn on_desktop(path: &str, folders: &[PathBuf]) -> bool {
    let p = std::path::Path::new(path);
    p.parent().is_some_and(|parent| {
        folders
            .iter()
            .any(|f| f.as_os_str().eq_ignore_ascii_case(parent.as_os_str()))
    })
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn physical_move_changes_disk_location_without_overwriting_collisions() {
        super::init();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "desktop-spaces-safe-move-{}-{nanos}",
            std::process::id()
        ));
        let from = root.join("source");
        let to = root.join("destination");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        let original = from.join("demo.txt");
        std::fs::write(&original, b"test only").unwrap();
        let moved = super::move_into_folder(&original, &to).unwrap();
        assert!(!original.exists());
        assert_eq!(std::fs::read(&moved).unwrap(), b"test only");
        std::fs::write(&original, b"new file").unwrap();
        assert!(super::move_into_folder(&original, &to).is_err());
        assert_eq!(std::fs::read(&original).unwrap(), b"new file");
        assert_eq!(std::fs::read(&moved).unwrap(), b"test only");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn explorer_returns_an_icon_for_a_file() {
        super::init();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), r"\Cargo.toml");
        let image = super::image(path, 64);
        assert!(image.is_some(), "no icon for {path}");
        let (w, h, px) = image.unwrap();
        assert!(w > 0 && h > 0 && px.len() == (w * h * 4) as usize);
        let slashes = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
        assert!(
            super::image(slashes, 64).is_some(),
            "forward slashes rejected"
        );
    }
}
