//! Optional UI-thread allocation instrumentation, compiled out of normal builds.
//! `--features profile-alloc --perf-report FILE` enables per-frame samples.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};

static ENABLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Default)]
pub struct Snapshot {
    pub calls: u64,
    pub bytes: u64,
}
thread_local! { static COUNTERS: Cell<Snapshot> = const { Cell::new(Snapshot { calls: 0, bytes: 0 }) }; }

struct TrackingAllocator;
#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

#[inline]
fn count(bytes: usize) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // Only the thread doing the allocation is counted; icon worker allocations
    // never pollute the UI thread's frame deltas. TLS is fallible during exit.
    let _ = COUNTERS.try_with(|c| {
        let previous = c.get();
        c.set(Snapshot {
            calls: previous.calls.saturating_add(1),
            bytes: previous.bytes.saturating_add(bytes as u64),
        });
    });
}

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let result = System.alloc(layout);
        if !result.is_null() {
            count(layout.size());
        }
        result
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let result = System.alloc_zeroed(layout);
        if !result.is_null() {
            count(layout.size());
        }
        result
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let result = System.realloc(ptr, layout, new_size);
        if !result.is_null() {
            count(new_size);
        }
        result
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}
pub fn snapshot() -> Snapshot {
    COUNTERS.try_with(Cell::get).unwrap_or_default()
}
