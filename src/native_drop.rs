//! Windows Shell Drag & Drop -> DesktopCraft virtual Pin membership.
//!
//! This target implements the documented OLE IDropTarget interface. It
//! requests DROPEFFECT_LINK only; it does not perform any disk file operation.
//! The actual move between DesktopCraft Boxes is a model-only Pin update,
//! performed later by the main UI thread after Drop returns.
#![allow(dead_code)]
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use winapi::shared::windef::HWND;
use winapi::um::winuser::{
    CreateWindowExW, DestroyWindow, MoveWindow, ShowWindow, SW_HIDE, SW_SHOW, WS_BORDER, WS_CHILD,
    WS_VISIBLE,
};

const EFFECT_LINK: u32 = 4;
const E_NOINTERFACE: i32 = 0x80004002u32 as i32;
const E_POINTER: i32 = 0x80004003u32 as i32;
const FORMAT_HDROP: u16 = 15;

#[repr(C)]
#[derive(PartialEq, Eq)]
struct Guid {
    a: u32,
    b: u16,
    c: u16,
    d: [u8; 8],
}
const IID_UNKNOWN: Guid = Guid {
    a: 0,
    b: 0,
    c: 0,
    d: [0xc0, 0, 0, 0, 0, 0, 0, 0x46],
};
const IID_DROP_TARGET: Guid = Guid {
    a: 0x00000122,
    b: 0,
    c: 0,
    d: [0xc0, 0, 0, 0, 0, 0, 0, 0x46],
};
#[repr(C)]
struct PointL {
    x: i32,
    y: i32,
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
    tymed: u32,
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
    release: usize,
    get_data: unsafe extern "system" fn(*mut DataObject, *const FormatEtc, *mut StgMedium) -> i32,
}
#[repr(C)]
struct DropTarget {
    vtbl: *const DropTargetVtbl,
    refs: AtomicU32,
    accepted: AtomicBool,
    group: u64,
    queue: Arc<Mutex<Vec<PinDrop>>>,
}
#[repr(C)]
struct DropTargetVtbl {
    query_interface:
        unsafe extern "system" fn(*mut DropTarget, *const Guid, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut DropTarget) -> u32,
    release: unsafe extern "system" fn(*mut DropTarget) -> u32,
    drag_enter:
        unsafe extern "system" fn(*mut DropTarget, *mut DataObject, u32, PointL, *mut u32) -> i32,
    drag_over: unsafe extern "system" fn(*mut DropTarget, u32, PointL, *mut u32) -> i32,
    drag_leave: unsafe extern "system" fn(*mut DropTarget) -> i32,
    drop: unsafe extern "system" fn(*mut DropTarget, *mut DataObject, u32, PointL, *mut u32) -> i32,
}
#[derive(Debug)]
pub struct PinDrop {
    pub group: u64,
    pub paths: Vec<PathBuf>,
}
pub type Queue = Arc<Mutex<Vec<PinDrop>>>;

#[link(name = "ole32")]
extern "system" {
    fn RegisterDragDrop(hwnd: HWND, target: *mut c_void) -> i32;
    fn RevokeDragDrop(hwnd: HWND) -> i32;
    fn ReleaseStgMedium(medium: *mut StgMedium);
}
#[link(name = "shell32")]
extern "system" {
    fn DragQueryFileW(drop: *mut c_void, index: u32, buffer: *mut u16, len: u32) -> u32;
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn paths(data: *mut DataObject) -> Option<Vec<PathBuf>> {
    if data.is_null() {
        return None;
    }
    let fmt = FormatEtc {
        format: FORMAT_HDROP,
        ptd: ptr::null_mut(),
        aspect: 1,
        index: -1,
        medium: 1,
    };
    let mut medium: StgMedium = std::mem::zeroed();
    let hr = ((*(*data).vtbl).get_data)(data, &fmt, &mut medium);
    if hr < 0 {
        return None;
    }
    let mut found = Vec::new();
    if !medium.handle.is_null() {
        let count = DragQueryFileW(medium.handle, u32::MAX, ptr::null_mut(), 0);
        if (1..=64).contains(&count) {
            for i in 0..count {
                let size = DragQueryFileW(medium.handle, i, ptr::null_mut(), 0);
                if size == 0 || size > 32767 {
                    break;
                }
                let mut w = vec![0u16; size as usize + 1];
                let n = DragQueryFileW(medium.handle, i, w.as_mut_ptr(), w.len() as u32);
                let path = PathBuf::from(String::from_utf16_lossy(&w[..n as usize]));
                if !path.exists() {
                    break;
                }
                found.push(path);
            }
        }
    }
    ReleaseStgMedium(&mut medium);
    (!found.is_empty() && found.len() <= 64).then_some(found)
}
unsafe extern "system" fn qi(
    this: *mut DropTarget,
    iid: *const Guid,
    result: *mut *mut c_void,
) -> i32 {
    if iid.is_null() || result.is_null() {
        return E_POINTER;
    }
    *result = ptr::null_mut();
    if *iid == IID_UNKNOWN || *iid == IID_DROP_TARGET {
        *result = this.cast();
        add_ref(this);
        return 0;
    }
    E_NOINTERFACE
}
unsafe extern "system" fn add_ref(this: *mut DropTarget) -> u32 {
    (*this).refs.fetch_add(1, Ordering::AcqRel) + 1
}
unsafe extern "system" fn release(this: *mut DropTarget) -> u32 {
    let refs = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if refs == 0 {
        drop(Box::from_raw(this));
    }
    refs
}
unsafe extern "system" fn drag_enter(
    this: *mut DropTarget,
    data: *mut DataObject,
    _keys: u32,
    _point: PointL,
    effect: *mut u32,
) -> i32 {
    if effect.is_null() {
        return E_POINTER;
    }
    let allows_link = (*effect & EFFECT_LINK) != 0;
    let accept = allows_link && paths(data).is_some();
    (*this).accepted.store(accept, Ordering::Release);
    *effect = if accept { EFFECT_LINK } else { 0 };
    0
}
unsafe extern "system" fn drag_over(
    this: *mut DropTarget,
    _keys: u32,
    _point: PointL,
    effect: *mut u32,
) -> i32 {
    if effect.is_null() {
        return E_POINTER;
    }
    *effect = if (*this).accepted.load(Ordering::Acquire) {
        EFFECT_LINK
    } else {
        0
    };
    0
}
unsafe extern "system" fn drag_leave(this: *mut DropTarget) -> i32 {
    (*this).accepted.store(false, Ordering::Release);
    0
}
unsafe extern "system" fn drop_items(
    this: *mut DropTarget,
    data: *mut DataObject,
    _keys: u32,
    _point: PointL,
    effect: *mut u32,
) -> i32 {
    if effect.is_null() {
        return E_POINTER;
    }
    let accept = (*this).accepted.swap(false, Ordering::AcqRel);
    if !accept {
        *effect = 0;
        return 0;
    }
    if let Some(paths) = paths(data) {
        if let Ok(mut queue) = (*this).queue.lock() {
            queue.push(PinDrop {
                group: (*this).group,
                paths,
            });
            *effect = EFFECT_LINK;
            return 0;
        }
    }
    *effect = 0;
    0
}
static VTABLE: DropTargetVtbl = DropTargetVtbl {
    query_interface: qi,
    add_ref,
    release,
    drag_enter,
    drag_over,
    drag_leave,
    drop: drop_items,
};

pub struct DropZone {
    hwnd: HWND,
    target: *mut DropTarget,
}
impl DropZone {
    pub fn new(parent: HWND, group: u64, queue: Queue) -> Result<Self, String> {
        let class = wide("BUTTON");
        let title = wide("DROP TO PIN (no file move)");
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_BORDER,
                0,
                0,
                200,
                25,
                parent,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if hwnd.is_null() {
                return Err("Could not create Pin Drop Target HWND".into());
            }
            let target = Box::into_raw(Box::new(DropTarget {
                vtbl: &VTABLE,
                refs: AtomicU32::new(1),
                accepted: AtomicBool::new(false),
                group,
                queue,
            }));
            let hr = RegisterDragDrop(hwnd, target.cast());
            if hr < 0 {
                release(target);
                DestroyWindow(hwnd);
                return Err(format!("RegisterDragDrop 0x{:08X}", hr as u32));
            }
            Ok(Self { hwnd, target })
        }
    }
    pub fn set_rect(&self, rect: [i32; 4]) {
        let [x, y, w, h] = rect;
        unsafe {
            MoveWindow(self.hwnd, x, y, w.max(90), h.max(22), 1);
        }
    }
    pub fn set_visible(&self, yes: bool) {
        unsafe {
            ShowWindow(self.hwnd, if yes { SW_SHOW } else { SW_HIDE });
        }
    }
}
impl Drop for DropZone {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.is_null() {
                RevokeDragDrop(self.hwnd);
                DestroyWindow(self.hwnd);
                self.hwnd = ptr::null_mut();
            }
            if !self.target.is_null() {
                release(self.target);
                self.target = ptr::null_mut();
            }
        }
    }
}
