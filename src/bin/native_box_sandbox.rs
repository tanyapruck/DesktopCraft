//! Explicitly gated native Win32 + IExplorerBrowser proof of concept.
//! It creates a separate top-level window with a Shell child view of ONLY
//! new disposable files in a unique folder beneath %TEMP%.
//! No Explorer injection, DesktopCraft Workspace access or desktop icon changes.
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
pub mod windows {
    use std::{
        ffi::{c_void, OsStr, OsString},
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::PathBuf,
        ptr,
    };
    use winapi::{
        shared::{
            minwindef::{LPARAM, LRESULT, UINT, WPARAM},
            windef::{HBRUSH, HMENU, HWND},
        },
        um::{
            libloaderapi::GetModuleHandleW,
            winuser::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
                GetWindowLongPtrW, KillTimer, MoveWindow, PostMessageW, PostQuitMessage,
                RegisterClassExW, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
                ShowWindow, TranslateMessage, UpdateWindow, BS_PUSHBUTTON, COLOR_WINDOW,
                CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, MSG,
                SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SW_SHOW, VK_RETURN, WM_CLOSE, WM_COMMAND,
                WM_DESTROY, WM_NCCREATE, WM_SIZE, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_BORDER,
                WS_CHILD, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
            },
        },
    };

    #[repr(C)]
    struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }
    const CLSID_BROWSER: Guid = Guid {
        a: 0x71F96385,
        b: 0xDDD6,
        c: 0x48D3,
        d: [0xA0, 0xC1, 0xAE, 0x06, 0xE8, 0xB0, 0x55, 0xFB],
    };
    const IID_BROWSER: Guid = Guid {
        a: 0xDFD3B6B5,
        b: 0xC10C,
        c: 0x4BE9,
        d: [0x85, 0xF6, 0xA6, 0x69, 0x69, 0xF4, 0x02, 0xF6],
    };
    const IID_FOLDER_VIEW: Guid = Guid {
        a: 0xCDE725B0,
        b: 0xCCC9,
        c: 0x4519,
        d: [0x91, 0x7E, 0x32, 0x5D, 0x72, 0xFA, 0xB4, 0xCE],
    };

    // IShellView exposes actual selected Shell items through IDataObject, not
    // through guessed filenames or icon screen coordinates.
    const IID_SHELL_VIEW: Guid = Guid {
        a: 0x000214E3,
        b: 0x0000,
        c: 0x0000,
        d: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    const IID_DATA_OBJECT: Guid = Guid {
        a: 0x0000010E,
        b: 0x0000,
        c: 0x0000,
        d: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    #[repr(C)]
    struct ShellView {
        vtbl: *const ShellViewVtbl,
    }
    #[repr(C)]
    struct ShellViewVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut ShellView) -> u32,
        get_window: usize,
        context_sensitive_help: usize,
        translate_accelerator: usize,
        enable_modeless: usize,
        ui_activate: usize,
        refresh: usize,
        create_view_window: usize,
        destroy_view_window: usize,
        get_current_info: usize,
        add_property_sheet_pages: usize,
        save_view_state: usize,
        select_item: usize,
        get_item_object:
            unsafe extern "system" fn(*mut ShellView, u32, *const Guid, *mut *mut c_void) -> i32,
    }
    #[repr(C)]
    struct FormatEtc {
        format: u16,
        ptd: *mut c_void,
        aspect: u32,
        index: i32,
        medium: u32,
    }
    #[repr(C)]
    struct StgMedium {
        medium: u32,
        handle: *mut c_void,
        unknown: *mut c_void,
    }
    #[repr(C)]
    struct DataObject {
        vtbl: *const DataObjectVtbl,
    }
    #[repr(C)]
    struct DataObjectVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut DataObject) -> u32,
        get_data:
            unsafe extern "system" fn(*mut DataObject, *const FormatEtc, *mut StgMedium) -> i32,
    }
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    struct FolderSettings {
        view_mode: u32,
        flags: u32,
    }
    #[repr(C)]
    struct Browser {
        vtbl: *const BrowserVtbl,
    }
    #[repr(C)]
    struct BrowserVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut Browser) -> u32,
        initialize: unsafe extern "system" fn(
            *mut Browser,
            HWND,
            *const Rect,
            *const FolderSettings,
        ) -> i32,
        destroy: unsafe extern "system" fn(*mut Browser) -> i32,
        set_rect: unsafe extern "system" fn(*mut Browser, *mut c_void, *const Rect) -> i32,
        set_property_bag: usize,
        set_empty_text: usize,
        set_folder_settings: usize,
        advise: usize,
        unadvise: usize,
        set_options: usize,
        get_options: usize,
        browse_to_id_list: unsafe extern "system" fn(*mut Browser, *mut c_void, u32) -> i32,
        browse_to_object: usize,
        fill_from_object: unsafe extern "system" fn(*mut Browser, *mut c_void, u32) -> i32,
        remove_all: usize,
        get_current_view:
            unsafe extern "system" fn(*mut Browser, *const Guid, *mut *mut c_void) -> i32,
    }

    // IFolderView and IResultsFolder are standard Shell COM interfaces.
    // The method order below exactly follows the SDK definitions.
    const IID_RESULTS_FOLDER: Guid = Guid {
        a: 0x96E5AE6D,
        b: 0x6AE1,
        c: 0x4B1C,
        d: [0x90, 0x0C, 0xC6, 0x48, 0x0E, 0xAA, 0x88, 0x28],
    };
    const IID_SHELL_ITEM: Guid = Guid {
        a: 0x43826D1E,
        b: 0xE718,
        c: 0x42EE,
        d: [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE],
    };
    #[repr(C)]
    struct FolderView {
        vtbl: *const FolderViewVtbl,
    }
    #[repr(C)]
    struct FolderViewVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut FolderView) -> u32,
        get_current_view_mode: usize,
        set_current_view_mode: usize,
        get_folder:
            unsafe extern "system" fn(*mut FolderView, *const Guid, *mut *mut c_void) -> i32,
    }
    #[repr(C)]
    struct ResultsFolder {
        vtbl: *const ResultsFolderVtbl,
    }
    #[repr(C)]
    struct ResultsFolderVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut ResultsFolder) -> u32,
        add_item: unsafe extern "system" fn(*mut ResultsFolder, *mut c_void) -> i32,
        add_id_list: usize,
    }
    #[repr(C)]
    struct Unknown {
        vtbl: *const UnknownVtbl,
    }
    #[repr(C)]
    struct UnknownVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut Unknown) -> u32,
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
        fn CoUninitialize();
        fn CoCreateInstance(
            clsid: *const Guid,
            outer: *mut c_void,
            context: u32,
            iid: *const Guid,
            result: *mut *mut c_void,
        ) -> i32;
        fn CoTaskMemFree(pointer: *mut c_void);
        fn ReleaseStgMedium(medium: *mut StgMedium);
    }
    #[link(name = "shell32")]
    extern "system" {
        fn DragQueryFileW(drop: *mut c_void, index: u32, buffer: *mut u16, len: u32) -> u32;
        fn SHObjectProperties(
            hwnd: HWND,
            object_type: u32,
            path: *const u16,
            page: *const u16,
        ) -> i32;
        fn SHCreateItemFromParsingName(
            name: *const u16,
            bind: *mut c_void,
            iid: *const Guid,
            result: *mut *mut c_void,
        ) -> i32;
        fn SHParseDisplayName(
            name: *const u16,
            bind: *mut c_void,
            pidl: *mut *mut c_void,
            attributes: u32,
            result_attributes: *mut u32,
        ) -> i32;
    }

    fn shell_path(path: &std::path::Path) -> Vec<u16> {
        // std::fs accepts mixed separators but SHParseDisplayName and
        // SHCreateItemFromParsingName can reject paths containing '/'.
        let normalized = path.to_string_lossy().replace('/', "\\");
        wide(OsStr::new(&normalized))
    }
    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    struct Lab {
        child: HWND,
        browser: *mut Browser,
        temp: PathBuf,
        initialized: bool,
        navigated: bool,
        view_ready: bool,
        resizes: u32,
        ticks: u32,
        smoke: bool,
        stress: bool,
        embedded: bool,
        workspace_folder: Option<PathBuf>,
        workspace_pins: Option<Vec<PathBuf>>,
        error: String,
        properties_invoked: bool,
        current_tab: u8,
        tab_visits: [u32; 3],
        results_items_added: usize,
        results_populated: bool,
        results_ever_loaded: bool,
        smoke_stage: u8,
    }
    impl Lab {
        fn new(smoke: bool, stress: bool) -> Result<Self, String> {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let temp = std::env::temp_dir().join(format!(
                "DesktopCraft-RustNativeBox-{}-{}",
                std::process::id(),
                nanos
            ));
            std::fs::create_dir(&temp).map_err(|e| e.to_string())?;
            for n in 0..4 {
                if let Err(e) = std::fs::write(
                    temp.join(format!("sandbox-{n}.txt")),
                    format!("Rust Native Box disposable Shell item {n}\n"),
                ) {
                    let _ = std::fs::remove_dir_all(&temp);
                    return Err(e.to_string());
                }
            }
            let alt = temp.join("alternate");
            if let Err(e) = std::fs::create_dir(&alt) {
                let _ = std::fs::remove_dir_all(&temp);
                return Err(e.to_string());
            }
            for n in 0..2 {
                if let Err(e) = std::fs::write(
                    alt.join(format!("alternate-{n}.txt")),
                    format!("Second disposable Shell folder {n}\n"),
                ) {
                    let _ = std::fs::remove_dir_all(&temp);
                    return Err(e.to_string());
                }
            }
            if stress {
                // Bounded optional stress fixture: 60 items per disposable source
                // directory. Never reads or modifies existing user files.
                for n in 0..60 {
                    for (dir, prefix) in [(&temp, "stress-a"), (&alt, "stress-b")] {
                        if let Err(e) = std::fs::write(
                            dir.join(format!("{prefix}-{n:03}.txt")),
                            "Round16 disposable ResultsFolder stress sample",
                        ) {
                            let _ = std::fs::remove_dir_all(&temp);
                            return Err(e.to_string());
                        }
                    }
                }
            }
            Ok(Self {
                child: ptr::null_mut(),
                browser: ptr::null_mut(),
                temp,
                initialized: false,
                navigated: false,
                view_ready: false,
                resizes: 0,
                ticks: 0,
                smoke,
                stress,
                embedded: false,
                workspace_folder: None,
                workspace_pins: None,
                error: String::new(),
                properties_invoked: false,
                current_tab: 0,
                tab_visits: [0; 3],
                results_items_added: 0,
                results_populated: false,
                results_ever_loaded: false,
                smoke_stage: 0,
            })
        }
        unsafe fn start(&mut self, hwnd: HWND) -> Result<(), String> {
            let class = wide(OsStr::new("STATIC"));
            let title = wide(OsStr::new("NATIVE SHELL VIEW / TEMP only"));
            let mut bounds: winapi::shared::windef::RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut bounds);
            let (left, top, width, height) = if self.embedded {
                // Real MINIQUADAPP parent in an isolated disposable test process.
                (
                    36,
                    144,
                    (bounds.right - 72).max(240),
                    (bounds.bottom - 188).max(180),
                )
            } else {
                (
                    16,
                    45,
                    (bounds.right - 32).max(100),
                    (bounds.bottom - 65).max(100),
                )
            };
            self.child = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_BORDER,
                left,
                top,
                width,
                height,
                hwnd,
                ptr::null_mut(),
                GetModuleHandleW(ptr::null()),
                ptr::null_mut(),
            );
            if self.child.is_null() {
                return Err("CreateWindowExW shell viewport failed".into());
            }
            if !self.embedded {
                let button_class = wide(OsStr::new("BUTTON"));
                for (index, label) in ["Tab A - Folder", "Tab B - Folder", "Virtual Pins A+B"]
                    .iter()
                    .enumerate()
                {
                    let caption = wide(OsStr::new(label));
                    let button = CreateWindowExW(
                        0,
                        button_class.as_ptr(),
                        caption.as_ptr(),
                        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
                        16 + index as i32 * 178,
                        8,
                        169,
                        29,
                        hwnd,
                        (101 + index) as HMENU,
                        GetModuleHandleW(ptr::null()),
                        ptr::null_mut(),
                    );
                    if button.is_null() {
                        return Err(format!("Could not create native Tab button {}", index));
                    }
                }
            }
            let initial = if self.workspace_pins.is_some() { 2 } else { 0 };
            self.switch_tab(hwnd, initial)?;
            if self.smoke && !self.embedded {
                // Only this sandbox window changes size. Existing Workspace and
                // Windows display DPI settings remain untouched.
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    0,
                    0,
                    930,
                    690,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            Ok(())
        }
        unsafe fn switch_tab(&mut self, hwnd: HWND, tab: u8) -> Result<(), String> {
            if tab > 2 {
                return Err("Unknown native Tab".into());
            }
            self.shutdown();
            self.current_tab = tab;
            self.view_ready = false;
            self.results_populated = false;
            self.results_items_added = if tab == 2 {
                0
            } else {
                self.results_items_added
            };
            self.ticks = 0;
            let mut result = ptr::null_mut();
            let hr = CoCreateInstance(
                &CLSID_BROWSER,
                ptr::null_mut(),
                1,
                &IID_BROWSER,
                &mut result,
            );
            if hr < 0 || result.is_null() {
                return Err(format!("CoCreateInstance 0x{:08X}", hr as u32));
            }
            self.browser = result.cast();
            let mut bounds: winapi::shared::windef::RECT = std::mem::zeroed();
            GetClientRect(self.child, &mut bounds);
            let rect = Rect {
                left: 0,
                top: 0,
                right: bounds.right.max(100),
                bottom: bounds.bottom.max(100),
            };
            let settings = FolderSettings {
                view_mode: 1,
                flags: 0,
            };
            let hr =
                ((*(*self.browser).vtbl).initialize)(self.browser, self.child, &rect, &settings);
            if hr < 0 {
                return Err(format!("IExplorerBrowser.Initialize 0x{:08X}", hr as u32));
            }
            self.initialized = true;
            if tab == 2 {
                // No drop target on virtual results: future Pin/Move/Copy actions
                // must be explicitly implemented, never inferred from a drop.
                let hr = ((*(*self.browser).vtbl).fill_from_object)(
                    self.browser,
                    ptr::null_mut(),
                    0x200,
                );
                if hr < 0 {
                    return Err(format!("FillFromObject 0x{:08X}", hr as u32));
                }
            } else {
                let folder = if let Some(workspace) = &self.workspace_folder {
                    workspace.clone()
                } else if tab == 0 {
                    self.temp.clone()
                } else {
                    self.temp.join("alternate")
                };
                let mut pidl = ptr::null_mut();
                let wide_path = shell_path(&folder);
                let hr = SHParseDisplayName(
                    wide_path.as_ptr(),
                    ptr::null_mut(),
                    &mut pidl,
                    0,
                    ptr::null_mut(),
                );
                if hr < 0 || pidl.is_null() {
                    return Err(format!(
                        "SHParseDisplayName 0x{:08X} abs={} exists={} chars={} preview_path={}",
                        hr as u32,
                        folder.is_absolute(),
                        folder.exists(),
                        wide_path.len(),
                        if std::env::args().any(|s| s == "--native-integrated-preview") {
                            folder.to_string_lossy().to_string()
                        } else {
                            "redacted".into()
                        }
                    ));
                }
                let hr = ((*(*self.browser).vtbl).browse_to_id_list)(self.browser, pidl, 0);
                CoTaskMemFree(pidl);
                if hr < 0 {
                    return Err(format!("BrowseToIDList 0x{:08X}", hr as u32));
                }
            }
            self.navigated = true;
            let label = match tab {
                0 => "DesktopCraft Rust Native Box Lab - Tab A",
                1 => "DesktopCraft Rust Native Box Lab - Tab B",
                _ => "DesktopCraft Rust Native Box Lab - Virtual Pins A+B",
            };
            if !self.embedded {
                let caption = wide(OsStr::new(label));
                SetWindowTextW(hwnd, caption.as_ptr());
            }
            Ok(())
        }
        unsafe fn populate_results(&mut self) -> Result<bool, String> {
            if self.browser.is_null() || !self.initialized {
                return Ok(false);
            }
            let mut view_ptr = ptr::null_mut();
            let hr = ((*(*self.browser).vtbl).get_current_view)(
                self.browser,
                &IID_FOLDER_VIEW,
                &mut view_ptr,
            );
            if hr < 0 || view_ptr.is_null() {
                return Ok(false);
            }
            let view = view_ptr.cast::<FolderView>();
            let mut results_ptr = ptr::null_mut();
            let hr = ((*(*view).vtbl).get_folder)(view, &IID_RESULTS_FOLDER, &mut results_ptr);
            ((*(*view).vtbl).release)(view);
            if hr < 0 || results_ptr.is_null() {
                return Ok(false);
            }
            let results = results_ptr.cast::<ResultsFolder>();
            let mut source_paths = vec![
                self.temp.join("sandbox-0.txt"),
                self.temp.join("alternate").join("alternate-0.txt"),
            ];
            if let Some(pins) = &self.workspace_pins {
                source_paths = pins.iter().filter(|p| p.exists()).cloned().collect();
            } else if self.stress {
                // Replace the two-item set with 120 disposable items, split
                // across the two folders to stress real ResultsFolder AddItem.
                source_paths.clear();
                for n in 0..60 {
                    source_paths.push(self.temp.join(format!("stress-a-{n:03}.txt")));
                    source_paths.push(
                        self.temp
                            .join("alternate")
                            .join(format!("stress-b-{n:03}.txt")),
                    );
                }
            }
            let mut added = 0;
            let result = (|| -> Result<(), String> {
                for path in source_paths.iter() {
                    // Fail closed: the only inputs are freshly created disposable files.
                    if self.workspace_pins.is_none() && !path.starts_with(&self.temp) {
                        return Err("Fixture escaped isolated TEMP".into());
                    }
                    if !path.exists() {
                        continue;
                    }
                    let wide_path = shell_path(path);
                    let mut item = ptr::null_mut();
                    let hr = SHCreateItemFromParsingName(
                        wide_path.as_ptr(),
                        ptr::null_mut(),
                        &IID_SHELL_ITEM,
                        &mut item,
                    );
                    if hr < 0 || item.is_null() {
                        return Err(format!("SHCreateItemFromParsingName 0x{:08X} abs={} exists={} chars={} preview_path={}",
                            hr as u32,path.is_absolute(),path.exists(),wide_path.len(),
                            if std::env::args().any(|s|s=="--native-integrated-preview")
                            {path.to_string_lossy().to_string()}else{"redacted".into()}));
                    }
                    let hr = ((*(*results).vtbl).add_item)(results, item);
                    let unknown = item.cast::<Unknown>();
                    ((*(*unknown).vtbl).release)(unknown);
                    if hr < 0 {
                        return Err(format!("IResultsFolder.AddItem 0x{:08X}", hr as u32));
                    }
                    added += 1;
                }
                Ok(())
            })();
            ((*(*results).vtbl).release)(results);
            result?;
            self.results_items_added = added;
            self.results_populated = added == source_paths.len();
            self.results_ever_loaded |= self.results_populated;
            Ok(self.results_populated)
        }
        unsafe fn resize(&mut self, parent: HWND) {
            if self.child.is_null() {
                return;
            }
            let mut bounds: winapi::shared::windef::RECT = std::mem::zeroed();
            GetClientRect(parent, &mut bounds);
            let w = (bounds.right - 32).max(100);
            let h = (bounds.bottom - 65).max(100);
            MoveWindow(self.child, 16, 45, w, h, 1);
            if self.initialized && !self.browser.is_null() {
                let rect = Rect {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: h,
                };
                let hr = ((*(*self.browser).vtbl).set_rect)(self.browser, ptr::null_mut(), &rect);
                if hr >= 0 {
                    self.resizes += 1;
                }
            }
        }
        unsafe fn poll_view(&mut self) {
            self.ticks += 1;
            if self.browser.is_null() || !self.initialized || self.view_ready {
                return;
            }
            if self.current_tab == 2 {
                match self.populate_results() {
                    Ok(true) => self.view_ready = true,
                    Ok(false) => (),
                    Err(e) => self.error = e,
                }
            } else {
                let mut ptr = ptr::null_mut();
                let hr = ((*(*self.browser).vtbl).get_current_view)(
                    self.browser,
                    &IID_FOLDER_VIEW,
                    &mut ptr,
                );
                if hr >= 0 && !ptr.is_null() {
                    self.view_ready = true;
                    let unknown = ptr.cast::<Unknown>();
                    ((*(*unknown).vtbl).release)(unknown);
                }
            }
            if self.view_ready {
                self.tab_visits[self.current_tab as usize] += 1;
            }
        }
        unsafe fn selected_paths(&self) -> Result<Vec<PathBuf>, String> {
            if self.browser.is_null() || !self.initialized {
                return Ok(Vec::new());
            }
            let mut view_ptr = ptr::null_mut();
            let hr = ((*(*self.browser).vtbl).get_current_view)(
                self.browser,
                &IID_SHELL_VIEW,
                &mut view_ptr,
            );
            if hr < 0 || view_ptr.is_null() {
                return Err(format!("GetCurrentView(IShellView) 0x{:08X}", hr as u32));
            }
            let view = view_ptr.cast::<ShellView>();
            let mut data_ptr = ptr::null_mut();
            let hr = ((*(*view).vtbl).get_item_object)(view, 1, &IID_DATA_OBJECT, &mut data_ptr);
            if hr < 0 || data_ptr.is_null() {
                ((*(*view).vtbl).release)(view);
                return Ok(Vec::new());
            }
            let data = data_ptr.cast::<DataObject>();
            let fmt = FormatEtc {
                format: 15,
                ptd: ptr::null_mut(),
                aspect: 1,
                index: -1,
                medium: 1,
            };
            let mut medium: StgMedium = std::mem::zeroed();
            let hr = ((*(*data).vtbl).get_data)(data, &fmt, &mut medium);
            let mut selected = Vec::new();
            if hr >= 0 && !medium.handle.is_null() {
                let count = DragQueryFileW(medium.handle, u32::MAX, ptr::null_mut(), 0);
                for i in 0..count.min(128) {
                    let len = DragQueryFileW(medium.handle, i, ptr::null_mut(), 0);
                    if len == 0 || len > 32767 {
                        continue;
                    }
                    let mut buffer = vec![0u16; len as usize + 1];
                    let written =
                        DragQueryFileW(medium.handle, i, buffer.as_mut_ptr(), buffer.len() as u32);
                    if written > 0 {
                        selected.push(PathBuf::from(OsString::from_wide(
                            &buffer[..written as usize],
                        )));
                    }
                }
            }
            if hr >= 0 {
                ReleaseStgMedium(&mut medium);
            }
            ((*(*data).vtbl).release)(data);
            ((*(*view).vtbl).release)(view);
            Ok(selected)
        }
        unsafe fn selected_file(&self) -> Result<Option<PathBuf>, String> {
            let selected = self.selected_paths()?;
            if selected.len() == 1 {
                Ok(selected.into_iter().next())
            } else {
                Ok(None)
            }
        }
        unsafe fn open_selected_properties(&mut self, hwnd: HWND) -> bool {
            let selected = match self.selected_file() {
                Ok(Some(path)) => path,
                _ => return false,
            };
            // Only allow the brand-new unique TEMP directory owned by THIS
            // process. This test code must never open a user Workspace item.
            let explicitly_pinned = self
                .workspace_pins
                .as_ref()
                .is_some_and(|paths| paths.iter().any(|p| p == &selected));
            let in_folder = self
                .workspace_folder
                .as_ref()
                .is_some_and(|folder| selected.starts_with(folder));
            if !(selected.starts_with(&self.temp) || explicitly_pinned || in_folder)
                || !selected.exists()
            {
                return false;
            }
            let path = wide(selected.as_os_str());
            let ok = SHObjectProperties(hwnd, 2, path.as_ptr(), ptr::null()) != 0;
            self.properties_invoked |= ok;
            ok
        }
        unsafe fn shutdown(&mut self) {
            if !self.browser.is_null() {
                if self.initialized {
                    let _ = ((*(*self.browser).vtbl).destroy)(self.browser);
                    self.initialized = false;
                }
                ((*(*self.browser).vtbl).release)(self.browser);
                self.browser = ptr::null_mut();
            }
        }
        fn cleanup(&self) -> bool {
            std::fs::remove_dir_all(&self.temp).is_ok() && !self.temp.exists()
        }
    }

    /// In-process Shell child in a disposable MINIQUADAPP host only.
    pub struct EmbeddedLab {
        inner: Lab,
        parent: HWND,
        rect: [i32; 4],
    }
    impl EmbeddedLab {
        pub fn new(parent: HWND) -> Result<Self, String> {
            if parent.is_null() {
                return Err("Macroquad HWND missing".into());
            }
            let hr = unsafe { CoInitializeEx(ptr::null_mut(), 2) };
            if hr < 0 {
                return Err(format!("COM STA 0x{:08X}", hr as u32));
            }
            let mut inner = match Lab::new(false, false) {
                Ok(lab) => lab,
                Err(error) => {
                    unsafe { CoUninitialize() };
                    return Err(error);
                }
            };
            inner.embedded = true;
            if let Err(error) = unsafe { inner.start(parent) } {
                unsafe {
                    inner.shutdown();
                    if !inner.child.is_null() {
                        winapi::um::winuser::DestroyWindow(inner.child);
                    }
                    CoUninitialize();
                }
                let _ = inner.cleanup();
                return Err(error);
            }
            let mut host = Self {
                inner,
                parent,
                rect: [-1; 4],
            };
            host.set_rect([36, 144, 728, 450]);
            Ok(host)
        }
        /// Binds a real DesktopCraft Box to a native Shell view.
        /// The model owns the file identities. This adapter never moves/copies
        /// those paths and only removes its own disposable test fixture on drop.
        pub fn from_workspace(
            parent: HWND,
            folder: Option<PathBuf>,
            pins: Vec<PathBuf>,
        ) -> Result<Self, String> {
            let mut view = Self::new(parent)?;
            view.inner.workspace_pins = if folder.is_none() { Some(pins) } else { None };
            view.inner.workspace_folder = folder;
            let kind = if view.inner.workspace_folder.is_some() {
                0
            } else {
                2
            };
            unsafe {
                view.inner.switch_tab(parent, kind)?;
            }
            Ok(view)
        }
        pub fn set_rect(&mut self, rect: [i32; 4]) {
            if self.rect == rect {
                return;
            }
            let [x, y, w, h] = rect;
            let w = w.max(200);
            let h = h.max(160);
            unsafe {
                if !self.inner.child.is_null() {
                    MoveWindow(self.inner.child, x, y, w, h, 1);
                    if self.inner.initialized && !self.inner.browser.is_null() {
                        let size = Rect {
                            left: 0,
                            top: 0,
                            right: w,
                            bottom: h,
                        };
                        if ((*(*self.inner.browser).vtbl).set_rect)(
                            self.inner.browser,
                            ptr::null_mut(),
                            &size,
                        ) >= 0
                        {
                            self.inner.resizes += 1;
                        }
                    }
                }
            }
            self.rect = rect;
        }
        pub fn tick(&mut self) {
            if !self.inner.view_ready {
                unsafe {
                    self.inner.poll_view();
                }
            }
        }
        pub fn switch_tab(&mut self, tab: u8) -> Result<(), String> {
            unsafe { self.inner.switch_tab(self.parent, tab) }
        }
        pub fn parent_is_macroquad(&self) -> bool {
            unsafe {
                !self.inner.child.is_null()
                    && winapi::um::winuser::GetParent(self.inner.child) == self.parent
            }
        }
        pub fn set_visible(&self, show: bool) {
            if !self.inner.child.is_null() {
                unsafe {
                    winapi::um::winuser::ShowWindow(
                        self.inner.child,
                        if show {
                            winapi::um::winuser::SW_SHOW
                        } else {
                            winapi::um::winuser::SW_HIDE
                        },
                    );
                }
            }
        }
        pub fn current_tab(&self) -> u8 {
            self.inner.current_tab
        }
        pub fn ready(&self) -> bool {
            self.inner.view_ready
        }
        pub fn visited(&self) -> [u32; 3] {
            self.inner.tab_visits
        }
        pub fn items(&self) -> usize {
            self.inner.results_items_added
        }
        pub fn results_loaded(&self) -> bool {
            self.inner.results_ever_loaded
        }
        pub fn error(&self) -> &str {
            &self.inner.error
        }
        pub fn selection(&self) -> Vec<PathBuf> {
            unsafe { self.inner.selected_paths().unwrap_or_default() }
        }
        pub fn properties(&mut self) -> bool {
            unsafe { self.inner.open_selected_properties(self.parent) }
        }
        pub fn close(self) -> bool {
            let path = self.inner.temp.clone();
            drop(self);
            !path.exists()
        }
    }
    impl Drop for EmbeddedLab {
        fn drop(&mut self) {
            unsafe {
                self.inner.shutdown();
                if !self.inner.child.is_null() {
                    winapi::um::winuser::DestroyWindow(self.inner.child);
                    self.inner.child = ptr::null_mut();
                }
                CoUninitialize();
            }
            let _ = self.inner.cleanup();
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: UINT,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_NCCREATE {
            let data = lparam as *const CREATESTRUCTW;
            let state = (*data).lpCreateParams as *mut Lab;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Lab;
        if !state.is_null() {
            match msg {
                WM_SIZE => {
                    (*state).resize(hwnd);
                }
                WM_COMMAND => {
                    // Only three fixed, internal tab buttons. Never accept paths,
                    // shell command strings or external actions via WM_COMMAND.
                    let button_id = (wparam & 0xFFFF) as u16;
                    if (101..=103).contains(&button_id) {
                        if let Err(error) = (*state).switch_tab(hwnd, (button_id - 101) as u8) {
                            (*state).error = error;
                            PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        }
                        return 0;
                    }
                }
                WM_TIMER => {
                    if wparam == 11 {
                        (*state).poll_view();
                        if !(&(*state).error).is_empty()
                            || (!(*state).view_ready && (*state).ticks >= 45)
                        {
                            if (&(*state).error).is_empty() {
                                (*state).error = format!(
                                    "Native Tab {} was not ready after bounded retries",
                                    (*state).current_tab
                                );
                            }
                            KillTimer(hwnd, 11);
                            PostMessageW(hwnd, WM_CLOSE, 0, 0);
                        } else if (*state).smoke && (*state).view_ready {
                            let next = match (*state).smoke_stage {
                                0 => Some(1),
                                1 => Some(2),
                                2 => Some(0),
                                _ => None,
                            };
                            if let Some(tab) = next {
                                (*state).smoke_stage += 1;
                                if let Err(error) = (*state).switch_tab(hwnd, tab) {
                                    (*state).error = error;
                                    KillTimer(hwnd, 11);
                                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                                }
                            } else {
                                KillTimer(hwnd, 11);
                                PostMessageW(hwnd, WM_CLOSE, 0, 0);
                            }
                        }
                    }
                }
                WM_CLOSE => {
                    KillTimer(hwnd, 11);
                    winapi::um::winuser::DestroyWindow(hwnd);
                    return 0;
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    return 0;
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    pub fn run() -> i32 {
        let smoke = std::env::args().any(|arg| arg == "--smoke");
        let stress = std::env::args().any(|arg| arg == "--stress-120");
        let mut lab = match Lab::new(smoke, stress) {
            Ok(lab) => lab,
            Err(error) => {
                eprintln!("sandbox failed: {error}");
                return 3;
            }
        };
        unsafe {
            let hr = CoInitializeEx(ptr::null_mut(), 2);
            if hr < 0 {
                let _ = lab.cleanup();
                eprintln!("COM STA initialization failed 0x{:08X}", hr as u32);
                return 4;
            }
            let klass = wide(OsStr::new("DesktopCraftRustNativeBoxLab"));
            let title = wide(OsStr::new(
                "DesktopCraft Rust Native Box Lab - disposable only",
            ));
            let instance = GetModuleHandleW(ptr::null());
            let mut registration: WNDCLASSEXW = std::mem::zeroed();
            registration.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
            registration.style = CS_HREDRAW | CS_VREDRAW;
            registration.lpfnWndProc = Some(wndproc);
            registration.hInstance = instance;
            registration.hbrBackground = (COLOR_WINDOW + 1) as usize as HBRUSH;
            registration.lpszClassName = klass.as_ptr();
            if RegisterClassExW(&registration) == 0 {
                let _ = lab.cleanup();
                CoUninitialize();
                eprintln!("RegisterClassExW failed");
                return 5;
            }
            let hwnd = CreateWindowExW(
                0,
                klass.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                850,
                630,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                &mut lab as *mut Lab as *mut winapi::ctypes::c_void,
            );
            if hwnd.is_null() {
                let _ = lab.cleanup();
                CoUninitialize();
                eprintln!("CreateWindowExW failed");
                return 6;
            }
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
            if let Err(error) = lab.start(hwnd) {
                lab.error = error;
                winapi::um::winuser::DestroyWindow(hwnd);
            } else {
                // Smoke cycles all three views; interactive sessions poll while
                // asynchronous ResultsFolder creation finishes.
                SetTimer(hwnd, 11, 320, None);
            }
            if lab.error.is_empty() {
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
                    // The native Explorer view is a child HWND; its Alt+Enter
                    // accelerator is intercepted before ordinary dispatch.
                    if message.message == WM_SYSKEYDOWN && message.wParam == VK_RETURN as usize {
                        let _ = lab.open_selected_properties(hwnd);
                        continue;
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            lab.shutdown();
            CoUninitialize();
        }
        let cleaned = lab.cleanup();
        println!(
            "{}",
            serde_json::json!({
                "test":"native_box_rust_shell_host",
                "isolated":true,
                "smoke":lab.smoke,
                "stress_120":lab.stress,
                "initialized":lab.navigated,
                "native_folder_view_ready":lab.view_ready,
                "tab_count":3,
                "tab_visits":lab.tab_visits,
                "all_tabs_visited":lab.tab_visits.iter().all(|v|*v>0),
                "results_folder_items":lab.results_items_added,
                "results_folder_loaded":lab.results_ever_loaded,
                "results_folder_active_at_shutdown":lab.results_populated,
                "active_tab_at_shutdown":lab.current_tab,
                "selected_properties_invoked":lab.properties_invoked,
                "resize_events":lab.resizes,
                "temp_cleanup":cleaned,
                "explorer_injection":false,
                "workspace_modified":false,
                "error":lab.error
            })
        );
        if lab.error.is_empty()
            && lab.navigated
            && (!smoke
                || (lab.view_ready
                    && lab.tab_visits.iter().all(|v| *v > 0)
                    && lab.results_items_added == (if lab.stress { 120 } else { 2 })
                    && lab.results_ever_loaded))
            && cleaned
        {
            0
        } else {
            7
        }
    }
}

#[cfg(windows)]
fn main() {
    std::process::exit(windows::run());
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows-only native Shell lab");
    std::process::exit(2);
}
