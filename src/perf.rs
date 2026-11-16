//! Opt-in Phase-D profiler. No file paths, names, or user content in reports.
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::Duration,
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static SAMPLES: OnceLock<Mutex<BTreeMap<&'static str, Vec<f64>>>> = OnceLock::new();

pub fn enable() {
    SAMPLES.get_or_init(|| Mutex::new(BTreeMap::new()));
    ENABLED.store(true, Ordering::Relaxed);
    #[cfg(feature = "profile-alloc")]
    crate::alloc_profile::enable();
}
#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn record(key: &'static str, elapsed: Duration) {
    if enabled() {
        record_value(key, elapsed.as_secs_f64() * 1000.0);
    }
}
pub fn record_value(key: &'static str, value: f64) {
    if !enabled() || !value.is_finite() {
        return;
    }
    if let Some(samples) = SAMPLES.get() {
        if let Ok(mut guard) = samples.lock() {
            let values = guard.entry(key).or_default();
            // Long-running desktop sessions must not grow profiler memory forever.
            // Keep recent samples; the profile remains opt-in.
            if values.len() >= 30_000 {
                values.drain(..15_000);
            }
            values.push(value);
        }
    }
}

fn summarize(mut samples: Vec<f64>) -> Value {
    if samples.is_empty() {
        return json!({"count":0});
    }
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    let q = |fraction: f64| samples[(((n - 1) as f64 * fraction).ceil() as usize).min(n - 1)];
    json!({
        "count": n,
        "min": samples[0],
        "p50": q(0.50),
        "p95": q(0.95),
        "p99": q(0.99),
        "max": samples[n - 1],
        "mean": samples.iter().sum::<f64>() / n as f64,
    })
}

#[cfg(windows)]
fn working_set_bytes() -> Option<usize> {
    use std::{ffi::c_void, mem};
    #[repr(C)]
    struct Counters {
        size: u32,
        faults: u32,
        peak_ws: usize,
        ws: usize,
        peak_page_pool: usize,
        page_pool: usize,
        peak_nonpage_pool: usize,
        nonpage_pool: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(process: *mut c_void, info: *mut Counters, size: u32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
    }
    unsafe {
        let mut counters: Counters = mem::zeroed();
        counters.size = mem::size_of::<Counters>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.size) != 0 {
            Some(counters.ws)
        } else {
            None
        }
    }
}
#[cfg(not(windows))]
fn working_set_bytes() -> Option<usize> {
    None
}

pub fn sample_memory() {
    if let Some(bytes) = working_set_bytes() {
        record_value("working_set_mib", bytes as f64 / (1024.0 * 1024.0));
    }
}

pub fn finish(path: &std::path::Path, mode: &str, frames: u32) -> Result<(), String> {
    if !enabled() {
        return Ok(());
    }
    let mut result = serde_json::Map::new();
    if let Some(samples) = SAMPLES.get() {
        if let Ok(guard) = samples.lock() {
            for (key, data) in guard.iter() {
                result.insert((*key).to_string(), summarize(data.clone()));
            }
        }
    }
    let report = json!({
        "schema": 1,
        "mode": mode,
        "frames": frames,
        "units": "milliseconds for *_ms; MiB for working_set_mib; counts otherwise",
        "definition": "frame_work_ms excludes vsync/present; frame_interval_ms includes next_frame wait. Scripted self-test is not representative of live desktop dragging.",
        "sample_policy": "Up to 30000 most recent samples per metric; UI-thread allocation calls and requested KiB include reallocations when profile-alloc is built, not live allocations or worker allocations",
        "alloc_instrumentation": cfg!(feature = "profile-alloc"),
        "metrics": result
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quantiles_are_sorted_and_include_spikes() {
        let r = summarize(vec![10.0, 1.0, 8.0, 3.0]);
        assert_eq!(r["count"], 4);
        assert_eq!(r["p50"], 8.0);
        assert_eq!(r["p95"], 10.0);
    }
}
