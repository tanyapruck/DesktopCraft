#![cfg_attr(windows, windows_subsystem = "windows")]
mod ai_tools;
#[cfg(feature = "profile-alloc")]
mod alloc_profile;
mod controls;
mod desktop;
mod dialogs;
mod explorer_bridge;
mod explorer_owned;
mod host;
mod model;
mod native_box;
#[cfg(all(windows, feature = "native-lab"))]
mod native_drop;
#[cfg(all(windows, feature = "native-lab"))]
mod native_integrated;
#[cfg(all(windows, feature = "native-lab"))]
mod native_quick;
#[cfg(all(windows, feature = "native-lab"))]
#[path = "bin/native_box_sandbox.rs"]
mod native_shell;
mod paint;
mod perf;
mod platform;
mod shell;
mod verification;
#[cfg(windows)]
mod updates;
use macroquad::prelude::*;
use model::*;
use paint::*;
use std::path::{Path, PathBuf};

fn capture_mode() -> bool {
    std::env::args().any(|x| {
        x == "--screenshot"
            || x == "--modern-menu-screenshot"
            || x == "--self-test"
            || x == "--text-check"
            || x == "--settings-screenshot"
    })
}
fn host_mode() -> bool {
    macroquad::miniquad::desktop_host()
}
/// Runs before any window opens: setup, removal, passing a command to the
/// running copy, and choosing desktop mode (the normal way to run on Windows).
fn startup() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    // Experimental Shell View bridge is an explicit pre-host command. Nothing
    // in the normal DesktopCraft launch path enables this automatically.
    if let Some(code) = native_box::handle_cli(&args) {
        std::process::exit(code);
    }
    let exe = std::env::current_exe().unwrap_or_default();
    // Agent interfaces never create a second GUI or desktop owner.
    // Keep stdout exclusively for CLI JSON / MCP JSON-RPC.
    if has("--mcp") {
        ai_tools::serve_mcp();
        std::process::exit(0);
    }
    if has("--tools-list") {
        println!("{}", serde_json::json!({"tools":ai_tools::tools()}));
        std::process::exit(0);
    }
    if let Some(i) = args.iter().position(|a| a == "--tool") {
        let name = args.get(i + 1).map(String::as_str).unwrap_or("");
        let arguments = args.get(i + 2).map(String::as_str).unwrap_or("{}");
        std::process::exit(ai_tools::cli(name, arguments));
    }
    if has("--uninstall") {
        if host::message(
            "Remove DesktopCraft from this Windows account?\n\nYour desktop files stay where they are, and your group layouts are kept in your profile in case you install it again.",
            true,
        ) {
            host::uninstall();
            host::message("DesktopCraft was removed.", false);
        }
        std::process::exit(0);
    }
    // The setup file installs when simply opened; with options (--command, --probe...)
    // it behaves like the program itself.
    if has("--install") || (host::is_setup_name(&exe) && args.len() == 1) {
        let this = env!("CARGO_PKG_VERSION");
        // Already installed: this setup updates that copy in place, keeping
        // the person's layout, rules and start-at-sign-in choice.
        let existing = host::installed_version();
        // A GitHub update was already approved by the user and SHA-256
        // verified. Reuse the in-place updater without a duplicate prompt.
        // Manual setup launches still require confirmation, including downgrades.
        let approved_update = has("--approved-update")
            && host::is_setup_name(&exe)
            && existing.as_ref().is_some_and(|old| host::version_newer(this, old));
        let go = if approved_update {
            Some(host::autostart(None))
        } else {
            match &existing {
            Some(old) => {
                let old_label = if old.is_empty() { "an earlier version".to_string() } else { old.clone() };
                let question = if !old.is_empty() && host::version_newer(old, this) {
                    format!("A newer DesktopCraft ({old}) is already installed.\n\nReplace it with this older version ({this})?")
                } else if old == this {
                    format!("DesktopCraft {this} is already installed.\n\nInstall it again? Your boxes, pages and rules are kept.")
                } else {
                    format!("Update DesktopCraft from {old_label} to {this}?\n\nThe running DesktopCraft closes, is replaced and starts again. Your boxes, pages, rules and settings are kept.")
                };
                host::message(&question, true).then(|| host::autostart(None))
            }
            None => host::message(
                "Install DesktopCraft for this Windows account?\n\nIt organizes icons into groups. No administrator rights are needed; remove it later through Settings > Apps.",
                true,
            )
            // Opt-in, default No: nothing starts with Windows unless asked for.
            .then(|| host::ask_default_no(
                "Start DesktopCraft automatically every time you sign in to Windows?\n\nYou can change this later in DesktopCraft Settings > Behavior.",
            )),
            }
        };
        if let Some(login) = go {
            match host::install(login) {
                Ok(installed) => {
                    host::event(
                        "INFO",
                        if existing.is_some() {
                            "install.updated"
                        } else {
                            "install.completed"
                        },
                        "per-user registration succeeded",
                    );
                    // Let a copy that was running finish closing first, or the new
                    // one would only hand it a command and exit.
                    for _ in 0..50 {
                        if !host::instance_running() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                    let mut start = std::process::Command::new(&installed);
                    if existing.is_some() {
                        // The new copy says which version it now is.
                        start.args(["--command", "updated"]);
                    }
                    if let Err(e) = start.spawn() {
                        host::event(
                            "ERROR",
                            "install.launch_failed",
                            "Windows rejected the installed executable",
                        );
                        host::message(&format!("Installed, but it could not start: {e}"), false);
                    }
                }
                Err(e) => {
                    host::event(
                        "ERROR",
                        "install.failed",
                        "per-user installation did not finish",
                    );
                    host::message(&format!("Installation did not finish: {e}"), false);
                }
            }
        } else {
            host::log("install: declined");
        }
        std::process::exit(0);
    }
    if let Some(i) = args.iter().position(|a| a == "--diagnose") {
        // Read-only report for another PC: no hidden icons, file paths,
        // private names, rules, screenshots, installation or registry writes.
        shell::init();
        let monitors = host::monitors();
        let workspace = host::data_dir().join("data/workspace.json");
        let workspace_valid = workspace.is_file().then(|| State::load(&workspace).is_ok());
        let windows_version = std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_else(|| "unavailable".to_string());
        let report = serde_json::json!({
            "app": "DesktopCraft",
            "version": env!("CARGO_PKG_VERSION"),
            "architecture": std::env::consts::ARCH,
            "windows_version": windows_version,
            "windows_process_architecture": std::env::var("PROCESSOR_ARCHITECTURE").unwrap_or_default(),
            "windows_native_architecture": std::env::var("PROCESSOR_ARCHITEW6432").unwrap_or_default(),
            "monitor_count": monitors.len(),
            "primary_monitor_found": monitors.iter().any(|m| m.primary),
            "explorer_desktop_available": shell::DesktopView::acquire().is_some(),
            "desktop_entries_count": shell::desktop_entries().len(),
            "workspace_present": workspace.exists(),
            "workspace_valid": workspace_valid,
            "report_contains_personal_paths": false,
        });
        let out = args
            .get(i + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| host::data_dir().join("desktopcraft-diagnostics.json"));
        let result = std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap_or_default());
        match result {
            Ok(()) => {
                host::event(
                    "INFO",
                    "diagnostics.generated",
                    "read-only report generated",
                );
                std::process::exit(0);
            }
            Err(_) => {
                host::event(
                    "ERROR",
                    "diagnostics.write_failed",
                    "report could not be saved",
                );
                std::process::exit(2);
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--perf-shell-probe") {
        // Read-only Explorer profiling. Never hides icons or changes positions.
        shell::init();
        perf::enable();
        for _ in 0..8 {
            let start = std::time::Instant::now();
            let files = shell::desktop_entries();
            perf::record("desktop_entries_ms", start.elapsed());
            perf::record_value("desktop_entry_count", files.len() as f64);
            let start = std::time::Instant::now();
            let view = shell::DesktopView::acquire();
            perf::record("desktop_view_acquire_ms", start.elapsed());
            if let Some(view) = view {
                let start = std::time::Instant::now();
                let items = view.items();
                perf::record("explorer_items_ms", start.elapsed());
                perf::record_value("explorer_icon_count", items.len() as f64);
            }
            perf::sample_memory();
        }
        let path = args
            .get(i + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("phase-d-explorer-read-only.json"));
        if let Err(e) = perf::finish(&path, "explorer_read_only", 0) {
            host::log(&format!("perf probe failed: {e}"));
        }
        std::process::exit(0);
    }
    if let Some((i, action)) = args.iter().enumerate().find_map(|(i, a)| match a.as_str() {
        "--explorer-bridge-probe" => Some((i, "probe")),
        "--explorer-bridge-audit" => Some((i, "audit")),
        "--explorer-bridge-select" => Some((i, "select")),
        "--explorer-bridge-menu" => Some((i, "menu")),
        _ => None,
    }) {
        // Explorer-first laboratory commands. Probe is strictly read-only;
        // select changes only Explorer selection; menu is an EXPLICIT opt-in
        // and does not make undocumented hooks or change desktop icon layout.
        shell::init();
        let target = args
            .get(i + 1)
            .map(String::as_str)
            .filter(|p| !p.starts_with("--"));
        let view = shell::DesktopView::acquire();
        let result = match action {
            "probe" => Ok(()),
            "select" => target
                .ok_or("Provide the exact existing Desktop icon path")
                .and_then(|p| {
                    view.as_ref()
                        .ok_or("Explorer Desktop View unavailable")
                        .map_err(|_| "Explorer Desktop View unavailable")
                        .and_then(|v| {
                            v.select_explorer_icon(p).map_err(|_| {
                                "Selection failed or target not in Explorer Desktop View"
                            })
                        })
                }),
            "menu" => target
                .ok_or("Provide the exact existing Desktop icon path")
                .and_then(|p| {
                    view.as_ref()
                        .ok_or("Explorer Desktop View unavailable")
                        .map_err(|_| "Explorer Desktop View unavailable")
                        .and_then(|v| {
                            v.request_explorer_context_menu(p).map_err(|_| {
                                "Explorer menu request refused or Desktop icons hidden"
                            })
                        })
                }),
            _ => Ok(()),
        };
        let probe = view
            .as_ref()
            .map(|v| v.explorer_bridge_probe(target))
            .unwrap_or_else(
                || serde_json::json!({"bridge":"explorer-desktop-view-lab","available":false}),
            );
        let coverage = if action == "audit" {
            let workspace = host::data_dir().join("data/workspace.json");
            State::load(&workspace).ok().map(|state| {
                let items = view.as_ref().map(|v| v.items()).unwrap_or_default();
                explorer_bridge::analyze(&state, &items).to_json()
            })
        } else {
            None
        };
        let success = result.is_ok();
        println!(
            "{}",
            serde_json::json!({
                "mode":action,
                "success":success,
                "diagnostics":probe,
                "workspace_coverage":coverage,
                "message":match action {
                    "probe"=>"Read-only probe complete; no Desktop files, selections or positions were changed",
                    "audit"=>"Read-only Explorer/DesktopCraft workspace coverage audit complete; counts only, no paths disclosed or state changed",
                    "select" if success=>"Explorer item selection requested; no file or icon positions changed",
                    "menu" if success=>"Explorer context-menu message queued; appearance of modern menu NOT verified",
                    _=>"The bridge could not complete the request; no file or icon positions changed",
                }
            })
        );
        std::process::exit(if success { 0 } else { 1 });
    }
    // Explorer-owned icons plan, Phase 0: make the desktop usable again.
    if let Some(i) = args.iter().position(|a| a == "--recover-desktop") {
        shell::init();
        let snapshot = args.get(i + 1).filter(|p| !p.starts_with("--")).map(PathBuf::from);
        let report = explorer_owned::recover_desktop(snapshot.as_deref());
        explorer_owned::emit(&report);
        std::process::exit(if report["recovered"] == true { 0 } else { 1 });
    }
    // Phase 1: one DesktopCraft frame around ONE real Explorer-owned icon.
    if let Some(i) = args.iter().position(|a| a == "--explorer-owned-lab") {
        shell::init();
        let Some(target) = args.get(i + 1).filter(|p| !p.starts_with("--")) else {
            eprintln!("usage: --explorer-owned-lab <desktop item path> [seconds] [report.json] [x y]");
            std::process::exit(2);
        };
        let seconds = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(120);
        let num = |k: usize| args.get(i + k).and_then(|v| v.parse::<i32>().ok());
        let place = num(4).zip(num(5));
        // Optional measured hole: dx dy w h relative to Explorer's position.
        let hole = match (num(6), num(7), num(8), num(9)) {
            (Some(a), Some(b), Some(c), Some(d)) => Some((a, b, c, d)),
            _ => None,
        };
        let report = explorer_owned::run_lab(target, seconds, place, hole);
        if let Some(out) = args.get(i + 3) {
            let _ = std::fs::write(out, serde_json::to_vec_pretty(&report).unwrap_or_default());
        }
        explorer_owned::emit(&report);
        std::process::exit(if report["started"] == true { 0 } else { 1 });
    }
    if let Some(i) = args.iter().position(|a| a == "--probe") {
        // Read-only check of the live Windows integration: nothing is moved,
        // hidden or registered. Writes a JSON report and exits.
        shell::init();
        let report = probe();
        let out = args
            .get(i + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("probe.json"));
        let _ = std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap_or_default());
        std::process::exit(0);
    }
    #[cfg(windows)]
    if let Some(i) = args.iter().position(|a| a == "--position-test") {
        // Moves ONE desktop icon by one cell and straight back, reporting what
        // Explorer answered each time (diagnoses icons not moving under groups).
        shell::init();
        let mut report = serde_json::Map::new();
        if let Some(view) = shell::DesktopView::acquire() {
            let items = view.items();
            let pick = args
                .get(i + 2)
                .cloned()
                .or_else(|| items.first().map(|x| x.path.clone()));
            report.insert("icons".into(), items.len().into());
            if args.iter().any(|a| a == "--hidden-icons") {
                // With Explorer's "Show desktop icons" off: can positions still be
                // read and set? (Restores everything and turns icons back on.)
                if let Some(first) = items.iter().find(|x| !x.path.starts_with("::")).cloned() {
                    let was_visible = view.icons_visible();
                    view.set_icons_visible(false);
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    report.insert(
                        "hidden_flag_now_visible".into(),
                        format!("{:?}", view.icons_visible()).into(),
                    );
                    report.insert(
                        "hidden_read".into(),
                        format!("{:?} was {:?}", view.position_of(&first.path), first.pos).into(),
                    );
                    let (sx, _) = view.spacing().unwrap_or((75, 98));
                    view.position(&[(first.path.clone(), (first.pos.0 + sx, first.pos.1))]);
                    let r = shell::DesktopView::last_position_result();
                    report.insert(
                        "hidden_move_hresult".into(),
                        format!("0x{:08X} matched {}/{}", r.0, r.2, r.1).into(),
                    );
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    report.insert(
                        "hidden_after_move".into(),
                        format!("{:?}", view.position_of(&first.path)).into(),
                    );
                    view.position(&[(first.path.clone(), first.pos)]);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    view.set_icons_visible(was_visible.unwrap_or(true));
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    report.insert(
                        "restored".into(),
                        format!(
                            "{:?} visible {:?}",
                            view.position_of(&first.path),
                            view.icons_visible()
                        )
                        .into(),
                    );
                    report.insert(
                        "labels".into(),
                        items
                            .iter()
                            .take(4)
                            .map(|x| format!("{} -> {}", x.path, x.label))
                            .collect::<Vec<_>>()
                            .into(),
                    );
                }
            } else if args.iter().any(|a| a == "--offscreen") {
                // Park one icon outside every monitor, read it back, then restore it.
                if let Some(first) = items.iter().find(|x| !x.path.starts_with("::")).cloned() {
                    let v = host::virtual_rect();
                    let spot = (v[0] + v[2] + 4000, first.pos.1);
                    view.position(&[(first.path.clone(), spot)]);
                    let r = shell::DesktopView::last_position_result();
                    report.insert(
                        "offscreen_hresult".into(),
                        format!("0x{:08X} matched {}/{}", r.0, r.2, r.1).into(),
                    );
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    report.insert("offscreen_target".into(), format!("{spot:?}").into());
                    report.insert(
                        "offscreen_after".into(),
                        format!("{:?}", view.position_of(&first.path)).into(),
                    );
                    if let Some(again) = shell::DesktopView::acquire() {
                        report.insert(
                            "offscreen_after_reacquire".into(),
                            format!("{:?}", again.position_of(&first.path)).into(),
                        );
                    }
                    view.position(&[(first.path.clone(), first.pos)]);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    report.insert(
                        "offscreen_restored".into(),
                        format!("{:?} was {:?}", view.position_of(&first.path), first.pos).into(),
                    );
                }
            } else if args.iter().any(|a| a == "--stack") {
                // Send three icons to one point in a single call, then put them back.
                let three: Vec<_> = items
                    .iter()
                    .filter(|x| !x.path.starts_with("::"))
                    .take(3)
                    .cloned()
                    .collect();
                if let Some(first) = three.first() {
                    let (sx, _) = view.spacing().unwrap_or((75, 98));
                    let spot = (first.pos.0 + sx, first.pos.1);
                    let step: i32 = args
                        .iter()
                        .position(|a| a == "--step")
                        .and_then(|i| args.get(i + 1))
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let moves: Vec<_> = three
                        .iter()
                        .enumerate()
                        .map(|(k, x)| {
                            (
                                x.path.clone(),
                                (spot.0 + k as i32 * step, spot.1 + k as i32 * step),
                            )
                        })
                        .collect();
                    view.position(&moves);
                    let r = shell::DesktopView::last_position_result();
                    report.insert(
                        "stack_hresult".into(),
                        format!("0x{:08X} matched {}/{}", r.0, r.2, r.1).into(),
                    );
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    let after: Vec<String> = three
                        .iter()
                        .map(|x| format!("{:?}", view.position_of(&x.path)))
                        .collect();
                    report.insert("stack_target".into(), format!("{spot:?}").into());
                    report.insert("stack_after".into(), after.into());
                    let back: Vec<_> = three.iter().map(|x| (x.path.clone(), x.pos)).collect();
                    view.position(&back);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    let restored: Vec<String> = three
                        .iter()
                        .map(|x| format!("{:?} was {:?}", view.position_of(&x.path), x.pos))
                        .collect();
                    report.insert("stack_restored".into(), restored.into());
                }
            } else if let Some(path) = pick {
                let before = view.position_of(&path);
                let (sx, _) = view.spacing().unwrap_or((75, 98));
                report.insert("item".into(), path.clone().into());
                report.insert("before".into(), format!("{before:?}").into());
                if let Some((x, y)) = before {
                    view.position(&[(path.clone(), (x + sx, y))]);
                    let r = shell::DesktopView::last_position_result();
                    report.insert(
                        "move_hresult".into(),
                        format!("0x{:08X} matched {}/{}", r.0, r.2, r.1).into(),
                    );
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    report.insert(
                        "after_move".into(),
                        format!("{:?}", view.position_of(&path)).into(),
                    );
                    view.position(&[(path.clone(), (x, y))]);
                    let r = shell::DesktopView::last_position_result();
                    report.insert("back_hresult".into(), format!("0x{:08X}", r.0).into());
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    report.insert(
                        "after_back".into(),
                        format!("{:?}", view.position_of(&path)).into(),
                    );
                }
            }
        } else {
            report.insert("error".into(), "Explorer desktop view not available".into());
        }
        let out = args
            .get(i + 1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("position-test.json"));
        let _ = std::fs::write(
            out,
            serde_json::to_vec_pretty(&serde_json::Value::Object(report)).unwrap_or_default(),
        );
        std::process::exit(0);
    }
    if !cfg!(windows) || has("--windowed") || has("--native-integrated-preview") {
        host::log("start: window mode");
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--host-smoke") {
        // Desktop mode against a throwaway data folder, with no first-run
        // question and no groups: the window is created on the desktop layer but
        // has nothing to show, so no icon is moved and nothing is registered.
        let dir = std::env::temp_dir().join(format!("DesktopCraftSmoke-{}", std::process::id()));
        std::env::set_var("DESKTOPCRAFT_DATA", &dir);
        std::env::set_var("DESKTOPCRAFT_SMOKE", "1");
        if let Some(report) = args.get(i + 1) {
            std::env::set_var("DESKTOPCRAFT_SMOKE_REPORT", report);
        }
        host::log("start: desktop mode smoke test");
        macroquad::miniquad::DESKTOP_HOST.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }
    let command = args
        .iter()
        .position(|a| a == "--pin-file")
        .and_then(|i| args.get(i + 1))
        .map(|path| format!("pin-file {path}"))
        .or_else(|| {
            args.iter()
                .position(|a| a == "--command")
                .and_then(|i| args.get(i + 1))
                .cloned()
        });
    if !host::first_instance() {
        host::allow_foreground();
        let _ = host::send_command(command.as_deref().unwrap_or("settings"));
        std::process::exit(0);
    }
    // Leftover commands from an earlier session are not replayed.
    let _ = host::take_commands();
    if let Some(c) = command {
        let _ = host::send_command(&c);
    }
    host::event(
        "INFO",
        "startup.desktop",
        if has("--at-sign-in") {
            "sign-in launch"
        } else {
            "interactive launch"
        },
    );
    macroquad::miniquad::DESKTOP_HOST.store(true, std::sync::atomic::Ordering::Relaxed);
}
fn probe() -> serde_json::Value {
    use serde_json::json;
    let monitors = host::monitors();
    let folders = shell::desktop_folders();
    let entries = shell::desktop_entries();
    let view = shell::DesktopView::acquire();
    let items = view.as_ref().map(|v| v.items()).unwrap_or_default();
    let matched = entries
        .iter()
        .filter(|e| {
            items
                .iter()
                .any(|i| i.path.eq_ignore_ascii_case(&e.to_string_lossy()))
        })
        .count();
    let sample = entries.first().map(|p| p.to_string_lossy().to_string());
    let icon = sample
        .as_deref()
        .and_then(|p| shell::image(p, 64))
        .map(|(w, h, _)| format!("{w}x{h}"));
    json!({
        "version": 1,
        "monitors": monitors.iter().map(|m| json!({"key": m.key, "rect": m.rect, "work": m.work, "dpi": m.dpi, "primary": m.primary})).collect::<Vec<_>>(),
        "monitor_signature": host::signature(&monitors),
        "virtual_screen": host::virtual_rect(),
        "desktop_folders": folders.iter().map(|f| f.to_string_lossy().to_string()).collect::<Vec<_>>(),
        "desktop_items": entries.len(),
        "explorer_desktop_view": view.is_some(),
        "explorer_icons_reported": items.len(),
        "desktop_items_with_icon_position": matched,
        "explorer_icon_spacing": view.as_ref().and_then(|v| v.spacing()),
        "explorer_auto_arrange": view.as_ref().and_then(|v| v.auto_arrange()),
        "explorer_align_to_grid": view.as_ref().and_then(|v| v.snap_to_grid()),
        "icon_positions": items.iter().map(|i| json!({"path": i.path, "x": i.pos.0, "y": i.pos.1})).collect::<Vec<_>>(),
        "explorer_icons_visible": view.as_ref().and_then(|v| v.icons_visible()),
        "first_item_icon": icon,
        "start_at_sign_in": host::autostart(None),
        "installed": host::install_dir().join(format!("{}.exe", host::APP_NAME)).exists(),
        "data_folder": host::data_dir().to_string_lossy(),
        "already_running": !host::first_instance(),
    })
}
/// Any unexpected stop is written to the log and, outside automated checks,
/// shown to the person instead of the program silently disappearing.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let message = format!("{info}");
        let location = info
            .location()
            .map(|location| {
                let source = std::path::Path::new(location.file())
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or("unknown");
                format!("{source}:{}", location.line())
            })
            .unwrap_or_else(|| "unknown".into());
        host::event("ERROR", "panic", &format!("source={location}"));
        if host_mode() && !capture_mode() {
            // Never leave the person without desktop icons.
            desktop::restore_explorer_icons();
        }
        if !capture_mode()
            && std::env::var_os("DESKTOPCRAFT_SMOKE").is_none()
            && std::env::var_os("DESKTOP_SPACES_SMOKE").is_none()
        {
            host::message(
                &format!(
                    "DesktopCraft stopped because of an error:\n\n{message}\n\nDetails are saved in {}",
                    host::data_dir().join("desktopcraft.log").display()
                ),
                false,
            );
        }
    }));
}
fn configuration() -> Conf {
    #[cfg(all(windows, feature = "native-lab"))]
    native_integrated::configure_dpi_early();
    // Older builds put these in the environment, so programs opened from the
    // desktop (and anything they start) may have inherited them: ignore them.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("DESKTOP_SPACES_") {
            std::env::remove_var(key);
        }
    }
    install_panic_hook();
    if capture_mode() {
        std::env::set_var("DESKTOPCRAFT_CAPTURE", "1");
    } else {
        startup();
    }
    Conf {
        window_title: "DesktopCraft".into(),
        window_width: 1280,
        window_height: 800,
        window_resizable: true,
        high_dpi: true,
        sample_count: 4,
        icon: Some(macroquad::miniquad::conf::Icon {
            small: *include_bytes!("../assets/desktopcraft-16.rgba"),
            medium: *include_bytes!("../assets/desktopcraft-32.rgba"),
            big: *include_bytes!("../assets/desktopcraft-64.rgba"),
        }),
        platform: macroquad::miniquad::conf::Platform {
            blocking_event_loop: false,
            swap_interval: Some(1),
            ..Default::default()
        },
        ..Default::default()
    }
}
#[derive(Clone, Default)]
pub struct Input {
    pub pos: Vec2,
    pub pressed: bool,
    pub released: bool,
    pub down: bool,
    pub right_pressed: bool,
    pub right_released: bool,
    pub right_down: bool,
    pub middle_pressed: bool,
    pub middle_down: bool,
    pub middle_released: bool,
    pub scroll: f32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub keys: Vec<KeyCode>,
    /// Keys Windows auto-repeated this frame because they're held down. Only
    /// editing keys use these (holding Backspace deletes like anywhere else);
    /// actions such as Enter or Escape still fire once per press.
    pub repeats: Vec<KeyCode>,
    pub typed: String,
    pub time: f64,
}
impl Input {
    fn collect(scale: f32) -> Self {
        let (x, y) = mouse_position();
        let mut typed = String::new();
        while let Some(c) = get_char_pressed() {
            if !c.is_control() {
                typed.push(c)
            }
        }
        Self {
            pos: vec2(x / scale, y / scale),
            pressed: is_mouse_button_pressed(MouseButton::Left),
            released: is_mouse_button_released(MouseButton::Left),
            down: is_mouse_button_down(MouseButton::Left),
            right_pressed: is_mouse_button_pressed(MouseButton::Right),
            right_down: is_mouse_button_down(MouseButton::Right),
            right_released: is_mouse_button_released(MouseButton::Right),
            middle_pressed: is_mouse_button_pressed(MouseButton::Middle),
            middle_down: is_mouse_button_down(MouseButton::Middle),
            middle_released: is_mouse_button_released(MouseButton::Middle),
            scroll: mouse_wheel().1,
            ctrl: is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl),
            shift: is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift),
            alt: is_key_down(KeyCode::LeftAlt) || is_key_down(KeyCode::RightAlt),
            keys: get_keys_pressed().into_iter().collect(),
            repeats: get_keys_repeated().into_iter().collect(),
            typed,
            time: get_time(),
        }
    }
    fn key(&self, key: KeyCode) -> bool {
        self.keys.contains(&key)
    }
    /// Pressed now, or repeating because it's held (for editing keys).
    fn key_or_repeat(&self, key: KeyCode) -> bool {
        self.keys.contains(&key) || self.repeats.contains(&key)
    }
}
#[derive(Clone)]
enum Drag {
    Group {
        id: u64,
        start: Vec2,
        before: Bounds,
    },
    Resize {
        id: u64,
        start: Vec2,
        before: Bounds,
        /// Edges being moved: 1 left, 2 right, 4 top, 8 bottom.
        edges: u8,
    },
    Item {
        group: u64,
        item: u64,
        start: Vec2,
    },
    /// Marquee selecting pins in a normal Box; never moves disk files.
    BoxSelect {
        group: u64,
        tab_id: u64,
        start: Vec2,
    },
    /// Reorder a stable tab ID within its box (not a physical file drag).
    Tab {
        group: u64,
        tab_id: u64,
        start: Vec2,
    },
    Create {
        start: Vec2,
    },
    Scroll {
        group: u64,
        track_y: f32,
        track_h: f32,
        total: usize,
        page: usize,
        cols: usize,
        grab: f32,
    },
}
enum Dialog {
    NewGroup {
        bounds: Bounds,
        value: String,
    },
    Rename {
        group: u64,
        value: String,
    },
    Path {
        portal: bool,
        value: String,
    },
    /// Bind an existing box to a user-selected on-disk folder.
    BindFolder {
        group: u64,
        value: String,
    },
    Remove {
        group: u64,
    },
    Snapshot {
        value: String,
    },
    /// Page creation must always go through the explicit + confirmation.
    NewPage {
        value: String,
    },
    RenamePage {
        index: usize,
        value: String,
    },
    /// Pop-out editor for one sorting rule, opened from Settings > Organize &
    /// rules. `target` indexes App::rule_destinations(). Closing it returns
    /// to that Settings page.
    EditRule {
        index: usize,
        mode: u8,
        pattern: String,
        target: usize,
    },
    Settings {
        page: usize,
        pattern: String,
        mode: u8,
        target: usize,
    },
}
#[derive(Clone)]
struct ModernFilePopup {
    path: String,
    pos: Vec2,
}

pub struct App {
    pub state: State,
    pub root: PathBuf,
    pub selected: Option<(u64, u64)>,
    /// Transient multi-pin selection: (box ID, stable tab ID, pin ID).
    pub multi_selected: Vec<(u64, u64, u64)>,
    dialog: Option<Dialog>,
    minimized_settings: Option<Dialog>,
    settings_restore_armed: bool,
    menu: Option<(u64, Vec2)>,
    /// One DesktopCraft-owned Windows 11-style file menu. Native Shell API
    /// performs operations behind the same custom-drawn UI.
    file_menu: Option<ModernFilePopup>,
    menu_page_open: bool,
    menu_page_scroll: usize,
    menu_sort_open: bool,
    /// A press on a box's "…" button: (box, where, undo entry added). It
    /// becomes a box move if dragged, or opens the menu when released.
    menu_press: Option<(u64, Vec2, bool)>,
    /// The Folder View sort list opened from its folder bar: (box, where).
    folder_sort_menu: Option<(u64, Vec2)>,
    /// The Folder View whose filter field is taking typed text.
    filter_focus: Option<u64>,
    /// An icon being renamed in place, like Explorer: (box, pin, file path, edit).
    inline_rename: Option<(u64, u64, String, model::NameEdit)>,
    /// Where the in-place rename box was drawn last frame.
    inline_rename_rect: Option<Bounds>,
    /// Where open pop-ups were drawn last frame, so a click elsewhere in
    /// Windows (outside DesktopCraft) can close them.
    popup_rects: Vec<Bounds>,
    typed_prefix: String,
    typed_at: f64,
    type_group: Option<u64>,
    drag: Option<Drag>,
    hidden: bool,
    pub peek: bool,
    pub desktop: bool,
    previous: [i32; 4],
    pub search: String,
    pub applied_search: String,
    search_seen: String,
    search_changed: f64,
    search_focus: bool,
    edit_focus: usize,
    select_all: bool,
    toast: String,
    toast_until: f64,
    last_click: Option<(Target, f64, Vec2)>,
    last_refresh: f64,
    settings_offset: usize,
    /// Floating Settings panel position within the primary display.
    settings_origin: Option<Vec2>,
    settings_drag_offset: Option<Vec2>,
    settings_mouse_was_down: bool,
    shortcut_recording: bool,
    shortcut_candidate: Option<String>,
    shortcut_recording_rejected: bool,
    /// Appearance target index while sampling one pixel from another window.
    color_pick_target: Option<usize>,
    wheel_remainder: f32,
    wheel_target: Option<u64>,
    undo: Vec<State>,
    pub automation: bool,
    pub opened: Vec<String>,
    /// Files whose Windows Properties sheet was requested (Alt+Enter); kept
    /// so automated runs can confirm the shortcut without opening a window.
    pub properties_requested: Vec<String>,
    pub stats: DrawStats,
    /// Reuse hit-testing storage across frames; capacity tracks the largest
    /// visible icon count instead of reallocating a new Vec every frame.
    hit_buffer: Vec<Hit>,
    pub width: f32,
    pub height: f32,
    save_path: PathBuf,
    save_allowed: bool,
    pub save_ok: bool,
    /// Present in desktop mode: groups on the real Windows desktop.
    pub host: Option<desktop::HostState>,
    cursor_edges: Option<u8>,
    /// Roll-up: the rolled group opened (after the delay or by a click), and
    /// which rolled group the pointer is over since when.
    pub revealed: Option<u64>,
    hover_rolled: Option<(u64, f64)>,
    /// Settings dropdowns: which is open, the list to draw, last list rect,
    /// a picked option, and a button click that only closes its list.
    dropdown_open: Option<u32>,
    dropdown_list: Option<(u32, Bounds, Vec<String>, usize)>,
    dropdown_hit: Option<(u32, Bounds, usize, usize)>,
    dropdown_pick: Option<(u32, usize)>,
    dropdown_suppress: Option<u32>,
    dropdown_scroll: usize,
}
impl App {
    fn new(root: PathBuf, automation: bool, host_mode: bool) -> Self {
        let _ = std::fs::create_dir_all(root.join("data"));
        let save_path = root.join("data/workspace.json");
        let mut message = String::new();
        let mut save_allowed = true;
        let fresh = |root: &Path| {
            if host_mode {
                State::empty()
            } else {
                State::demo(root)
            }
        };
        let mut state = if !automation && save_path.exists() {
            match State::load(&save_path) {
                Ok(s) => s,
                Err(e) => {
                    let backup =
                        root.join(format!("data/workspace-damaged-{}.json", now_seconds()));
                    save_allowed = std::fs::copy(&save_path, &backup).is_ok();
                    message = format!("{e} Your saved file has been kept.");
                    if !automation {
                        host::event(
                            "ERROR",
                            "workspace.load_failed",
                            if save_allowed {
                                "saved broken workspace backup"
                            } else {
                                "workspace backup failed"
                            },
                        );
                    }
                    fresh(&root)
                }
            }
        } else {
            fresh(&root)
        };
        // Only a genuinely new desktop workspace gets starter boxes. A
        // corrupt/existing workspace is not overwritten by defaults.
        let first_run = host_mode && !save_path.exists();
        if first_run && !automation {
            for (index, name) in ["Shortcuts", "Documents", "Folders"].iter().enumerate() {
                let id = state.add_group(
                    name,
                    Bounds {
                        x: 45. + index as f32 * 352.,
                        y: 64.,
                        w: 330.,
                        h: 260.,
                    },
                );
                if let Some(pattern) = match index {
                    0 => Some("lnk,url,appref-ms"),
                    1 => Some("pdf,doc,docx,txt,rtf,odt,xls,xlsx,csv,ppt,pptx,md"),
                    _ => None,
                } {
                    let tab_id = state.group(id).unwrap().tab().id;
                    state.rules.push(Rule {
                        mode: 0,
                        pattern: pattern.into(),
                        target: id,
                        target_tab: Some(tab_id),
                        enabled: true,
                    });
                }
            }
        }
        // Some older workspaces had canonical Folders / Archives boxes but
        // no corresponding enabled sorting rule. Add only genuinely missing
        // category rules; do not override existing rules or manual placements.
        if host_mode && !automation {
            state.ensure_builtin_sort_rules();
        }
        // Existing Peek shortcuts remain untouched. The newer default is
        // applied only when Prefs is initialized without a saved value.
        // Manual rules never silently override a person's placement.
        state.auto_rules = false;
        Self {
            state,
            root,
            selected: None,
            multi_selected: Vec::new(),
            dialog: None,
            minimized_settings: None,
            settings_restore_armed: false,
            menu: None,
            file_menu: None,
            menu_page_open: false,
            menu_page_scroll: 0,
            menu_sort_open: false,
            menu_press: None,
            folder_sort_menu: None,
            filter_focus: None,
            inline_rename: None,
            inline_rename_rect: None,
            popup_rects: Vec::new(),
            typed_prefix: String::new(),
            typed_at: -10.,
            type_group: None,
            drag: None,
            hidden: false,
            peek: false,
            desktop: false,
            previous: [100, 100, 1280, 800],
            search: String::new(),
            applied_search: String::new(),
            search_seen: String::new(),
            search_changed: 0.,
            search_focus: false,
            edit_focus: 0,
            select_all: true,
            toast: message,
            toast_until: 12.,
            last_click: None,
            last_refresh: 0.,
            settings_offset: 0,
            settings_origin: None,
            settings_drag_offset: None,
            settings_mouse_was_down: false,
            shortcut_recording: false,
            shortcut_candidate: None,
            shortcut_recording_rejected: false,
            color_pick_target: None,
            wheel_remainder: 0.,
            wheel_target: None,
            undo: vec![],
            automation,
            opened: vec![],
            properties_requested: vec![],
            stats: DrawStats::default(),
            hit_buffer: Vec::with_capacity(128),
            width: 1280.,
            height: 800.,
            save_path,
            save_allowed,
            save_ok: true,
            host: host_mode.then(|| desktop::HostState::new(first_run)),
            cursor_edges: None,
            revealed: None,
            hover_rolled: None,
            dropdown_open: None,
            dropdown_list: None,
            dropdown_hit: None,
            dropdown_pick: None,
            dropdown_suppress: None,
            dropdown_scroll: 0,
        }
    }
    fn notify(&mut self, s: impl Into<String>, time: f64) {
        self.toast = s.into();
        // Record problems without leaking filenames, folder paths or the user's
        // rule/question text. Normal UI hints remain out of the log.
        let lower = self.toast.to_ascii_lowercase();
        if [
            "could not",
            "failed",
            "not available",
            "not found",
            "cannot",
            "invalid",
            "no longer exists",
        ]
        .iter()
        .any(|term| lower.contains(term))
        {
            if !self.automation {
                host::event(
                    "WARN",
                    "ui.operation_problem",
                    "see the on-screen message; private details omitted",
                );
            }
        }
        self.toast_until = time + 5.;
    }
    pub(crate) fn remember(&mut self) {
        self.undo.push(self.state.clone());
        if self.undo.len() > 32 {
            self.undo.remove(0);
        }
    }
    pub(crate) fn save(&mut self, time: f64) {
        if self.automation {
            return;
        }
        if self.host.is_some() {
            self.remember_layout();
        }
        if self.save_allowed {
            match self.state.save(&self.save_path) {
                Ok(()) => self.save_ok = true,
                Err(e) => {
                    self.save_ok = false;
                    host::event(
                        "ERROR",
                        "workspace.save_failed",
                        "workspace changes could not be persisted",
                    );
                    self.notify(format!("Could not save changes: {e}"), time)
                }
            }
        }
    }
    fn group_id(&self) -> Option<u64> {
        self.selected
            .map(|x| x.0)
            .filter(|id| self.state.group(*id).is_some())
            .or_else(|| {
                self.state
                    .groups
                    .iter()
                    .find(|g| g.page == self.state.page)
                    .map(|g| g.id)
            })
    }
    fn fresh_bounds(&self) -> Bounds {
        let n = self
            .state
            .groups
            .iter()
            .filter(|g| g.page == self.state.page)
            .count() as f32;
        Bounds {
            x: 70. + (n * 32.) % 230.,
            y: 140. + (n * 36.) % 190.,
            w: 330.,
            h: 260.,
        }
    }
    fn create_group(&mut self, bounds: Bounds) {
        self.remember();
        let id = self.state.add_group("New group", bounds);
        self.selected = Some((id, 0));
        self.dialog = Some(Dialog::Rename {
            group: id,
            value: "New group".into(),
        });
        self.select_all = true;
        self.edit_focus = 0;
    }
    fn open(&mut self, path: &str, time: f64) {
        if !Path::new(path).exists() {
            self.notify("This item is no longer at its saved location.", time);
            return;
        }
        if self.automation {
            self.opened.push(path.into());
            self.notify("Opened sample item", time)
        } else {
            match platform::open(path) {
                Ok(()) => self.notify("Opened in its default application", time),
                Err(e) => self.notify(e, time),
            }
        }
    }
    fn pick_file(&mut self, time: f64) {
        if self.group_id().is_none() {
            let bounds = self.fresh_bounds();
            let id = self.state.add_group("My files", bounds);
            self.selected = Some((id, 0));
        }
        if self.automation {
            self.dialog = Some(Dialog::Path {
                portal: false,
                value: String::new(),
            });
            return;
        }
        if let Some(path) = platform::choose_file() {
            self.import_path(&path, false, time)
        }
    }
    fn pick_folder(&mut self, time: f64) {
        if self.automation {
            return;
        }
        if let Some(path) = platform::choose_folder() {
            self.import_path(&path, true, time)
        }
    }
    fn import_path(&mut self, path: &Path, portal: bool, time: f64) {
        // Explicit file import follows the destination's semantics too:
        // normal box = pin only, linked Folder View = physical move.
        if !portal && self.host.is_some() {
            if let Some(id) = self.group_id() {
                if let Err(e) = self.drop_path_into_box(&path.to_string_lossy(), id, time) {
                    self.notify(e, time);
                }
                return;
            }
        }
        self.remember();
        let result = if portal {
            self.state.add_portal(path, self.fresh_bounds()).map(|id| {
                self.selected = Some((id, 0));
            })
        } else if let Some(id) = self.group_id() {
            self.state.unassign(&path.to_string_lossy());
            self.state.import(path, id)
        } else {
            Err("Create a group first.".into())
        };
        match result {
            Ok(()) => self.notify(
                if portal {
                    "Folder connected. Double-click a subfolder to browse."
                } else {
                    "File pinned. Double-click to open."
                },
                time,
            ),
            Err(e) => self.notify(e, time),
        }
    }
    fn switch_page(&mut self, direction: i32) {
        let previous = self.state.page;
        if direction > 0 && self.state.page + 1 < self.state.pages.len() {
            self.state.page += 1;
        } else if direction < 0 && self.state.page > 0 {
            self.state.page -= 1;
        }
        self.selected = None;
        self.multi_selected.clear();
        self.menu = None;
        if self.host.is_some() && self.state.page != previous {
            host::event(
                "INFO",
                "pages.switched",
                &format!(
                    "from={} to={} total={}",
                    previous + 1,
                    self.state.page + 1,
                    self.state.pages.len()
                ),
            );
        }
        if self.state.page != previous {
            if let Some(h) = self.host.as_mut() {
                h.last_sync = -10.;
            }
        }
    }
    fn request_new_page(&mut self) {
        self.dialog = Some(Dialog::NewPage {
            value: format!("Space {}", self.state.pages.len() + 1),
        });
        self.edit_focus = 0;
        self.select_all = true;
    }
    fn set_peek(&mut self, on: bool) {
        self.peek = on;
        if self.hidden {
            self.set_hidden(false);
        }
        // Desktop mode raises and lowers the window in host_after_frame.
        if !self.automation && self.host.is_none() {
            platform::peek(on, self.desktop)
        }
    }
    fn activate_item(&mut self, group: u64, item: u64, time: f64) {
        let Some(g) = self.state.group(group) else {
            return;
        };
        let Some(item) = g.tab().items.iter().find(|i| i.id == item).cloned() else {
            return;
        };
        if item.kind == "folder" && g.tab().folder.is_some() {
            if let Err(e) = self
                .state
                .group_mut(group)
                .unwrap()
                .tab_mut()
                .navigate(&item.path)
            {
                self.notify(e, time)
            }
        } else {
            self.open(&item.path, time)
        }
    }
    fn topbar(&mut self, p: &Painter, input: &Input) -> bool {
        draw_rectangle(
            0.,
            0.,
            self.width,
            64.,
            Color::new(0.035, 0.07, 0.115, 0.94),
        );
        draw_line(0., 64., self.width, 64., 1., alpha(WHITE, 0.06));
        for (x, y) in [(22., 22.), (33., 22.), (22., 33.), (33., 33.)] {
            round(Bounds { x, y, w: 8., h: 8. }, 2., ACCENT);
        }
        p.text("DesktopCraft", 52., 18., 20., TEXT);
        let search = Bounds {
            x: 258.,
            y: 15.,
            w: 230.,
            h: 34.,
        };
        round(search, 6., Color::new(0.14, 0.20, 0.28, 0.85));
        draw_circle_lines(273., 30., 5., 1.4, MUTED);
        draw_line(277., 34., 281., 38., 1.4, MUTED);
        p.text(
            &p.fit(
                if self.search.is_empty() {
                    "Find a file…"
                } else {
                    &self.search
                },
                14.,
                181.,
            ),
            290.,
            22.,
            14.,
            if self.search.is_empty() { MUTED } else { TEXT },
        );
        if self.search_focus {
            outline(search, ACCENT);
        }
        if p.enabled && input.pressed && search.contains(input.pos.x, input.pos.y) {
            self.search_focus = true;
            return true;
        }
        let w = self.width;
        if p.button(
            "+ New group",
            Bounds {
                x: w - 652.,
                y: 15.,
                w: 120.,
                h: 34.,
            },
            false,
        ) {
            self.create_group(self.fresh_bounds());
            return true;
        }
        if p.button(
            "Folder view",
            Bounds {
                x: w - 522.,
                y: 15.,
                w: 108.,
                h: 34.,
            },
            false,
        ) {
            self.pick_folder(input.time);
            return true;
        }
        if p.button(
            "Add file",
            Bounds {
                x: w - 404.,
                y: 15.,
                w: 84.,
                h: 34.,
            },
            false,
        ) {
            self.pick_file(input.time);
            return true;
        }
        if p.button(
            if self.peek { "End peek" } else { "Peek" },
            Bounds {
                x: w - 310.,
                y: 15.,
                w: 70.,
                h: 34.,
            },
            false,
        ) {
            self.set_peek(!self.peek);
            return true;
        }
        if p.button(
            "Settings",
            Bounds {
                x: w - 230.,
                y: 15.,
                w: 91.,
                h: 34.,
            },
            false,
        ) {
            self.dialog = Some(Dialog::Settings {
                page: 0,
                pattern: String::new(),
                mode: 0,
                target: 0,
            });
            return true;
        }
        if p.button(
            if self.hidden { "Show all" } else { "Hide all" },
            Bounds {
                x: w - 129.,
                y: 15.,
                w: 106.,
                h: 34.,
            },
            false,
        ) {
            self.hidden = !self.hidden;
            return true;
        }
        input.pressed && input.pos.y < 64.
    }
    /// Bottom navigator. "Sort icons" explicitly applies enabled Rules to
    /// loose Desktop icons and already-pinned items, not page ordering.
    fn desktop_pager(&mut self, p: &mut Painter, input: &Input) -> bool {
        let blocking_dialog = self
            .dialog
            .as_ref()
            .is_some_and(|d| !matches!(d, Dialog::Settings { .. }));
        if !self.state.prefs.pages || blocking_dialog || self.menu.is_some() || self.drag.is_some()
        {
            return false;
        }
        let (cx, cy) = host::cursor();
        let point = self.from_screen(cx, cy);
        let view = self.windows_primary_view();
        if point.y < view.y + view.h - 130.
            || point.y > view.y + view.h
            || (point.x - (view.x + view.w / 2.)).abs() > 260.
        {
            return false;
        }
        let mut consumed = false;
        self.in_monitor_view(p, input, view, |app, p, input| {
            let b = Bounds { x: ((app.width - 414.) / 2.).max(5.),
                y: app.height - 66., w: 414., h: 46. };
            p.push_region(b, 10.);
            round(b, 10., Color::new(0.055, 0.09, 0.14, 0.94));
            let left = Bounds { x: b.x + 8., y: b.y + 7., w: 34., h: 32. };
            let right = Bounds { x: b.x + 227., y: b.y + 7., w: 34., h: 32. };
            let add = Bounds { x: b.x + 268., y: b.y + 7., w: 36., h: 32. };
            let sort = Bounds { x: b.x + 313., y: b.y + 7., w: 92., h: 32. };
            let enabled = app.dialog.as_ref().is_none_or(|d|matches!(d,Dialog::Settings{..}))
                && app.menu.is_none();
            if enabled {
                if app.state.page > 0 {
                    if p.button("‹", left, false) { app.switch_page(-1); }
                } else { p.text("‹", left.x + 12., left.y + 7., 16., MUTED); }
                if app.state.page + 1 < app.state.pages.len() {
                    if p.button("›", right, false) { app.switch_page(1); }
                } else { p.text("›", right.x + 12., right.y + 7., 16., MUTED); }
                if p.button("+", add, false) {
                    app.request_new_page();
                }
                if p.button("Sort icons", sort, false) {
                    if !app.state.rules.iter().any(|rule|
                        rule.enabled && app.state.rule_destination(rule).is_some()) {
                        app.notify("No enabled Rules. Add sorting rules in Settings first.", input.time);
                    } else {
                        // Explicit Sort re-applies enabled Rules to both loose
                        // desktop icons AND pins in the wrong Box or Tab.
                        // Ordinary Pins are virtual: files stay on disk.
                        app.remember();
                        app.multi_selected.clear();
                        let count = app.sort_desktop_now();
                        if count == 0 {
                            app.undo.pop();
                            app.notify("Nothing to sort. Check ZIP/Folder Rules and destinations in Settings.", input.time);
                        } else {
                            app.notify(format!("Organized {count} icon(s) by Rules (files unchanged)."), input.time);
                        }
                    }
                }
            } else {
                p.text("‹", left.x + 12., left.y + 7., 16., MUTED);
                p.text("›", right.x + 12., right.y + 7., 16., MUTED);
                p.text("+", add.x + 12., add.y + 7., 16., MUTED);
                p.text("Sort icons", sort.x + 9., sort.y + 7., 14., MUTED);
            }
            let name = &app.state.pages[app.state.page];
            let label = format!("{}  ({}/{})", name, app.state.page + 1, app.state.pages.len());
            p.text(&p.fit(&label, 14., 170.), b.x + 50., b.y + 15., 14., TEXT);
            consumed = input.pressed && b.contains(input.pos.x,input.pos.y);
        });
        consumed
    }
    fn footer(&mut self, p: &Painter, input: &Input) -> bool {
        let y = self.height - 48.;
        draw_rectangle(0., y, self.width, 48., Color::new(0.035, 0.07, 0.115, 0.92));
        p.text(
            "Draw to create  ·  Double-click blank space to hide",
            23.,
            y + 17.,
            12.,
            MUTED,
        );
        let page_w = 112.;
        let visible = self.state.pages.len().min(3);
        let first = self
            .state
            .page
            .saturating_sub(1)
            .min(self.state.pages.len().saturating_sub(visible));
        let total = page_w * visible as f32 + 118.;
        let start = (self.width - total) / 2. + 36.;
        let left = Bounds {
            x: start - 36.,
            y: y + 8.,
            w: 28.,
            h: 32.,
        };
        if self.state.page > 0 {
            if p.button("‹", left, false) {
                self.switch_page(-1);
                return true;
            }
        } else {
            p.text("‹", left.x + 9., left.y + 8., 14., MUTED);
        }
        let mut page = None;
        for (i, name) in self
            .state
            .pages
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
        {
            let b = Bounds {
                x: start + (i - first) as f32 * page_w,
                y: y + 8.,
                w: page_w - 6.,
                h: 32.,
            };
            if p.button(name, b, self.state.page == i) {
                page = Some(i)
            }
        }
        if let Some(page) = page {
            self.state.page = page;
            self.selected = None;
            return true;
        }
        let right = Bounds {
            x: start + visible as f32 * page_w,
            y: y + 8.,
            w: 28.,
            h: 32.,
        };
        if self.state.page + 1 < self.state.pages.len() {
            if p.button("›", right, false) {
                self.switch_page(1);
                return true;
            }
        } else {
            p.text("›", right.x + 9., right.y + 8., 14., MUTED);
        }
        if p.button(
            "+",
            Bounds {
                x: start + visible as f32 * page_w + 36.,
                y: y + 8.,
                w: 30.,
                h: 32.,
            },
            false,
        ) {
            self.request_new_page();
            return true;
        }
        draw_circle(
            self.width - 151.,
            y + 24.,
            3.,
            if self.save_ok { COLORS[1] } else { COLORS[4] },
        );
        p.text(
            if self.save_ok {
                "Changes saved"
            } else {
                "Save failed"
            },
            self.width - 140.,
            y + 16.,
            12.,
            MUTED,
        );
        self.stats.status = true;
        input.pressed && input.pos.y >= y
    }
    /// Debounce typed search by 350 ms; clearing it resets immediately.
    fn update_search(&mut self, time: f64) {
        if self.search != self.search_seen {
            self.search_seen = self.search.clone();
            self.search_changed = time;
        }
        if self.search.is_empty() {
            self.applied_search.clear();
        } else if self.applied_search != self.search
            && time - self.search_changed >= SEARCH_UPDATE_DELAY
        {
            self.applied_search = self.search.clone();
        }
    }
    /// Roll-up "after a delay" and "on click": which rolled group is open. It
    /// closes again when the pointer leaves the group.
    fn update_rollup_reveal(&mut self, input: &Input) {
        let page = self.state.page;
        let under = self
            .state
            .groups
            .iter()
            .rev()
            .filter(|g| g.page == page)
            .find(|g| {
                // A rolled group is revealed ONLY while hovering its title/tabs.
                // Never treat the invisible body underneath as a hover target.
                Bounds {
                    h: HEADER,
                    ..g.bounds
                }
                .contains(input.pos.x, input.pos.y)
            })
            .map(|g| (g.id, g.rolled));
        // A pointer starting inside the invisible rolled-up body NEVER opens
        // it. Once triggered from the title, however, keep the revealed
        // content usable while the pointer moves into that visible body.
        let still_inside_revealed = self.revealed.is_some_and(|id| {
            self.state.group(id).is_some_and(|g| {
                g.page == page && g.rolled && g.bounds.contains(input.pos.x, input.pos.y)
            })
        });
        // The expanded panel owns its full bounds until the pointer exits.
        // A lower stacked box's title may lie underneath this body; hovering
        // it must not steal the reveal (nor its delayed hover timer).
        if still_inside_revealed {
            self.hover_rolled = None;
            return;
        }
        self.revealed = None;
        match self.state.prefs.rollup_open {
            0 => {
                if let Some((id, true)) = under {
                    self.revealed = Some(id);
                }
            }
            1 => match (under, self.hover_rolled) {
                (Some((id, true)), Some((hid, since))) if hid == id => {
                    if input.time - since >= ROLLUP_DELAY {
                        self.revealed = Some(id);
                    }
                }
                (Some((id, true)), _) => {
                    self.hover_rolled = Some((id, input.time));
                    if self.revealed != Some(id) {
                        self.revealed = None;
                    }
                }
                _ => {
                    self.hover_rolled = None;
                    if !still_inside_revealed {
                        self.revealed = None;
                    }
                }
            },
            2 => { /* Title click sets revealed; body hover does not trigger it. */ }
            _ => self.revealed = None,
        }
    }
    /// Draw something centered on the primary monitor's work area (dialogs and
    /// messages in desktop mode, where the workspace spans every monitor).
    fn in_primary_view(
        &mut self,
        p: &mut Painter,
        input: &Input,
        f: impl FnOnce(&mut Self, &mut Painter, &Input),
    ) {
        let view = self.primary_view();
        self.in_monitor_view(p, input, view, f);
    }
    fn in_monitor_view(
        &mut self,
        p: &mut Painter,
        input: &Input,
        v: Bounds,
        f: impl FnOnce(&mut Self, &mut Painter, &Input),
    ) {
        let (w, h) = (self.width, self.height);
        set_camera(&Camera2D {
            target: vec2(w / 2. - v.x, h / 2. - v.y),
            zoom: vec2(2. / w, 2. / h),
            ..Default::default()
        });
        self.width = v.w;
        self.height = v.h;
        let mut local = input.clone();
        local.pos -= vec2(v.x, v.y);
        let mouse = p.mouse;
        p.mouse = local.pos;
        p.offset = vec2(v.x, v.y);
        f(self, p, &local);
        p.mouse = mouse;
        p.offset = Vec2::ZERO;
        self.width = w;
        self.height = h;
        set_camera(&workspace_camera(w, h));
    }
    /// Settings uses virtual-desktop coordinates in desktop mode, not the
    /// selected monitor's local coordinates. A floating Settings window can
    /// therefore cross from the primary display onto another monitor.
    pub(crate) fn settings_bounds_local(&self) -> Bounds {
        if self.host.is_some() {
            let h = self.host.as_ref().unwrap();
            return floating_settings_bounds(
                Bounds {
                    x: 0.,
                    y: 0.,
                    w: self.width.max(h.virtual_rect[2] as f32 / h.scale.max(0.1)),
                    h: self.height.max(h.virtual_rect[3] as f32 / h.scale.max(0.1)),
                },
                self.primary_view(),
                self.settings_origin,
            );
        }
        self.settings_bounds_local_for_view(Bounds {
            x: 0.,
            y: 0.,
            w: self.width,
            h: self.height,
        })
    }
    pub(crate) fn settings_bounds_local_for_view(&self, view: Bounds) -> Bounds {
        let w = 870.0f32;
        let h = 650.0f32;
        let default = vec2(((view.w - w) / 2.).max(0.), ((view.h - h) / 2.).max(0.));
        let origin = self.settings_origin.unwrap_or(default);
        Bounds {
            x: origin.x.clamp(0., (view.w - w).max(0.)),
            y: origin.y.clamp(0., (view.h - h).max(0.)),
            w,
            h,
        }
    }
    pub fn frame(&mut self, p: &mut Painter, input: &Input) {
        // Opening Settings again replaces any previously minimized panel.
        if matches!(self.dialog, Some(Dialog::Settings { .. })) && self.minimized_settings.is_some()
        {
            self.minimized_settings = None;
            self.settings_restore_armed = false;
        }
        if self.host.is_some() {
            clear_background(Color::new(0., 0., 0., 0.));
        } else {
            p.wallpaper(self.state.wallpaper);
        }
        self.shortcuts(input);
        self.update_search(input.time);
        let floating_settings = matches!(self.dialog, Some(Dialog::Settings { .. }));
        // Settings acts like a floating tool panel: boxes outside its bounds
        // remain interactive, while clicks within it belong to Settings.
        let settings_hit = floating_settings
            && self
                .settings_bounds_local()
                .contains(input.pos.x, input.pos.y);
        let popup_open = self.menu.is_some() || self.folder_sort_menu.is_some();
        p.enabled = (self.dialog.is_none() || (floating_settings && !settings_hit)) && !popup_open;
        p.click = input.pressed && !settings_hit;
        self.stats = DrawStats::default();
        // Desktop mode draws only groups, menus, dialogs and messages: the rest is
        // Explorer's real desktop (wallpaper, icons and its own right-click menu).
        let host = self.host.is_some();
        let top_consumed = if host { false } else { self.topbar(p, input) };
        if !host {
            p.text(
                if self.peek {
                    "Your groups are in front"
                } else {
                    &self.state.pages[self.state.page]
                },
                52.,
                80.,
                17.,
                TEXT,
            );
            p.text(
                if self.hidden {
                    "Double-click to bring everything back"
                } else {
                    "A place for everything you’re working on"
                },
                52.,
                103.,
                12.,
                MUTED,
            );
        }
        self.update_rollup_reveal(input);
        let prefs = &self.state.prefs;
        let style = paint::GroupStyle {
            titlebar: prefs.titlebar,
            hide_scrollbars: prefs.hide_scrollbars,
            rollup: prefs.rollup,
            revealed: self.revealed,
            filter_focus: self.filter_focus,
        };
        let mut hits = std::mem::take(&mut self.hit_buffer);
        hits.clear();
        if host {
            // Desktop icons outside groups first, so groups draw above them.
            self.draw_loose(p, &mut hits);
            self.draw_right_drag(p);
            self.draw_marquee(p);
        }
        // Draw the expanded rolled-up box last so it is physically on top
        // of stacked neighbors, and its hits win pointer dispatch too.
        for g in paint_order(&self.state.groups, self.state.page, self.revealed) {
            paint::group(
                p,
                g,
                self.selected,
                &self.multi_selected,
                self.hidden,
                &self.applied_search,
                self.peek,
                self.state.blend,
                self.state.tint,
                &style,
                &mut hits,
                &mut self.stats,
            );
        }
        // An icon being renamed in place gets its edit box over its label.
        self.draw_inline_rename(p, &hits, input.time);
        if let Some(Drag::BoxSelect { group, start, .. }) = &self.drag {
            if start.distance(input.pos) > DRAG_THRESHOLD
                && self
                    .state
                    .group(*group)
                    .is_some_and(|g| g.page == self.state.page)
            {
                let b = Bounds {
                    x: start.x.min(input.pos.x),
                    y: start.y.min(input.pos.y),
                    w: (start.x - input.pos.x).abs(),
                    h: (start.y - input.pos.y).abs(),
                };
                draw_rectangle(b.x, b.y, b.w, b.h, Color::new(0.25, 0.50, 0.95, 0.16));
                draw_rectangle_lines(b.x, b.y, b.w, b.h, 1.5, Color::new(0.50, 0.75, 1.0, 0.85));
            }
        }
        let pager_consumed = if host {
            self.desktop_pager(p, input)
        } else {
            false
        };

        if !host && self.state.groups.iter().all(|g| g.page != self.state.page) && !self.hidden {
            p.label(
                "Room to focus.",
                Bounds {
                    x: self.width / 2. - 200.,
                    y: self.height / 2. - 65.,
                    w: 400.,
                    h: 50.,
                },
                32.,
                TEXT,
            );
            p.label(
                "Draw a group, add a folder, or bring a group from another page.",
                Bounds {
                    x: self.width / 2. - 300.,
                    y: self.height / 2. - 6.,
                    w: 600.,
                    h: 32.,
                },
                15.,
                MUTED,
            );
        }
        if let Some(Drag::Create { start }) = &self.drag {
            let b = Bounds {
                x: start.x.min(input.pos.x),
                y: start.y.min(input.pos.y),
                w: (start.x - input.pos.x).abs(),
                h: (start.y - input.pos.y).abs(),
            };
            draw_rectangle(b.x, b.y, b.w, b.h, alpha(ACCENT, 0.13));
            outline(b, ACCENT);
        }
        if let Some(Drag::Item { group, item, start }) = &self.drag {
            if start.distance(input.pos) > DRAG_THRESHOLD {
                if let Some(item) = self
                    .state
                    .group(*group)
                    .and_then(|g| g.tab().items.iter().find(|x| x.id == *item))
                {
                    p.icon(item, input.pos.x - 20., input.pos.y - 20., 40., None);
                    p.push_region(
                        Bounds {
                            x: input.pos.x - 22.,
                            y: input.pos.y - 22.,
                            w: 44.,
                            h: 44.,
                        },
                        0.,
                    );
                }
            }
        }
        let footer_consumed = if host { false } else { self.footer(p, input) };
        let had_menu = self.menu.is_some();
        let had_file_menu = self.file_menu.is_some();
        let had_sort_list = self.folder_sort_menu.is_some();
        // Rebuilt below by whichever pop-ups are drawn this frame.
        self.popup_rects.clear();
        // Resize cursor over group edges and corners (and while resizing).
        let edges = match &self.drag {
            Some(Drag::Resize { edges, .. }) => Some(*edges),
            _ => hits
                .iter()
                .rev()
                .find(|h| h.bounds.contains(input.pos.x, input.pos.y))
                .and_then(|h| {
                    if let Target::Resize(_, e) = h.target {
                        Some(e)
                    } else {
                        None
                    }
                }),
        };
        if edges != self.cursor_edges {
            self.cursor_edges = edges;
            if !self.automation {
                macroquad::miniquad::window::set_mouse_cursor(
                    edges
                        .map(paint::resize_cursor)
                        .unwrap_or(macroquad::miniquad::CursorIcon::Default),
                );
            }
        }
        if (self.dialog.is_none() || (floating_settings && !settings_hit))
            && self.menu.is_none()
            && self.file_menu.is_none()
            && self.folder_sort_menu.is_none()
            && !top_consumed
            && !footer_consumed
            && !pager_consumed
        {
            self.pointer(input, &hits);
            if host {
                self.loose_drag(input);
            }
        }
        if self.menu.is_some() {
            if had_menu {
                self.context_menu(p, input);
            } else {
                let mut opening = input.clone();
                opening.pressed = false;
                self.context_menu(p, &opening);
            }
        }
        if self.file_menu.is_some() {
            if had_file_menu {
                self.modern_file_menu(p, input);
            } else {
                let mut opening = input.clone();
                opening.pressed = false;
                self.modern_file_menu(p, &opening);
            }
        }
        if self.folder_sort_menu.is_some() {
            if had_sort_list {
                self.folder_sort_list(p, input);
            } else {
                // The press that opened it must not also pick a row.
                let mut opening = input.clone();
                opening.pressed = false;
                self.folder_sort_list(p, &opening);
            }
        }
        if self.dialog.is_some() {
            if host && !floating_settings {
                self.in_primary_view(p, input, |app, p, input| {
                    p.push_region(
                        Bounds {
                            x: 0.,
                            y: 0.,
                            w: app.width,
                            h: app.height,
                        },
                        0.,
                    );
                    app.modal(p, input)
                });
            } else {
                // Draw floating Settings in the virtual-screen camera so both
                // its rendering and its mouse hit regions cross monitor edges.
                if host {
                    p.push_region(self.settings_bounds_local(), 10.);
                }
                self.modal(p, input);
            }
        }
        if !self.toast.is_empty() && input.time < self.toast_until {
            let draw_toast = |app: &mut App, p: &mut Painter, _: &Input| {
                let w = (p.measure(&app.toast, 14.) + 40.).min(app.width - 80.);
                let b = Bounds {
                    x: (app.width - w) / 2.,
                    y: app.height - if app.host.is_some() { 70. } else { 108. },
                    w,
                    h: 43.,
                };
                p.push_region(b, 7.);
                round(b, 7., SURFACE);
                p.label(&app.toast, b, 14., TEXT);
            };
            if host {
                self.in_primary_view(p, input, draw_toast);
            } else {
                draw_toast(self, p, input);
            }
        }
        if input.time - self.last_refresh > REFRESH && self.dialog.is_none() && self.drag.is_none()
        {
            self.last_refresh = input.time;
            let mut error = None;
            for g in &mut self.state.groups {
                for tab in &mut g.tabs {
                    if tab.folder.is_some() {
                        if let Err(e) = tab.refresh() {
                            error = Some(e)
                        }
                    }
                }
            }
            if let Some(e) = error {
                self.notify(e, input.time)
            }
        }
        self.hit_buffer = hits;
    }
}
#[cfg(test)]
mod page_control_tests {
    use super::*;
    #[test]
    fn right_arrow_stops_at_last_page_and_plus_only_requests_confirmation() {
        let dir =
            std::env::temp_dir().join(format!("desktopcraft-pages-test-{}", std::process::id()));
        let mut app = App::new(dir, true, false);
        app.state = State::empty();
        assert_eq!(app.state.pages.len(), 1);
        for _ in 0..5 {
            app.switch_page(1);
        }
        assert_eq!(app.state.page, 0);
        assert_eq!(app.state.pages.len(), 1);
        app.request_new_page();
        assert!(matches!(app.dialog, Some(Dialog::NewPage { .. })));
        assert_eq!(app.state.pages.len(), 1); // request is not creation
        app.dialog = None; // user cancelled
        assert_eq!(app.state.pages.len(), 1);
        app.state.create_page("Work").unwrap(); // only confirmed action
        assert_eq!(app.state.page, 1);
        for _ in 0..5 {
            app.switch_page(1);
        }
        assert_eq!(app.state.page, 1);
        assert_eq!(app.state.pages.len(), 2);
        app.switch_page(-1);
        assert_eq!(app.state.page, 0);
    }
}

fn target_group(target: &Target) -> u64 {
    match *target {
        Target::Header(id)
        | Target::Tab(id, _)
        | Target::TabNext(id)
        | Target::Roll(id)
        | Target::Menu(id)
        | Target::Resize(id, _)
        | Target::Item(id, _)
        | Target::Back(id)
        | Target::Open(id)
        | Target::FolderFilter(id)
        | Target::FilterField(id)
        | Target::FolderSort(id)
        | Target::Body(id) => id,
        Target::Scroll { group, .. } => group,
        Target::Loose(_) => 0,
    }
}

/// Paint/hit-test order follows the current raised roll-up, not creation
/// order. The active expanded box must take precedence over overlapping tabs.
fn paint_order<'a>(
    groups: &'a [Group],
    page: usize,
    revealed: Option<u64>,
) -> impl Iterator<Item = &'a Group> + 'a {
    groups
        .iter()
        .filter(move |g| g.page == page && Some(g.id) != revealed)
        .chain(
            groups
                .iter()
                .filter(move |g| g.page == page && Some(g.id) == revealed),
        )
}

/// A floating tool window is initially centered on the selected primary
/// monitor, but its drag constraints use the ENTIRE virtual desktop. When
/// crossing a monitor edge, no primary-monitor clamp can trap the window.
fn floating_settings_bounds(workspace: Bounds, primary: Bounds, location: Option<Vec2>) -> Bounds {
    let (w, h) = (870.0f32, 650.0f32);
    let default = vec2(
        primary.x + ((primary.w - w) / 2.).max(0.),
        primary.y + ((primary.h - h) / 2.).max(0.),
    );
    let origin = location.unwrap_or(default);
    Bounds {
        x: origin.x.clamp(
            workspace.x,
            (workspace.x + workspace.w - w).max(workspace.x),
        ),
        y: origin.y.clamp(
            workspace.y,
            (workspace.y + workspace.h - h).max(workspace.y),
        ),
        w,
        h,
    }
}

#[cfg(test)]
mod settings_multimonitor_tests {
    use super::*;
    #[test]
    fn floating_settings_can_move_fully_to_secondary_display() {
        let virtual_screen = Bounds {
            x: 0.,
            y: 0.,
            w: 3840.,
            h: 1080.,
        };
        let primary = Bounds {
            x: 0.,
            y: 0.,
            w: 1920.,
            h: 1080.,
        };
        let initial = floating_settings_bounds(virtual_screen, primary, None);
        assert!(initial.x >= 0. && initial.x < 1920.);
        let moved = floating_settings_bounds(virtual_screen, primary, Some(vec2(2330., 200.)));
        assert_eq!(moved.x, 2330.);
        assert!(moved.x + moved.w <= virtual_screen.w);
        assert!(moved.x > primary.x + primary.w);
    }
    #[test]
    fn settings_clamps_to_virtual_desktop_not_primary_monitor() {
        let virtual_screen = Bounds {
            x: 0.,
            y: 0.,
            w: 3840.,
            h: 1080.,
        };
        let primary = Bounds {
            x: 1920.,
            y: 0.,
            w: 1920.,
            h: 1080.,
        };
        let moved = floating_settings_bounds(virtual_screen, primary, Some(vec2(-200., 100.)));
        assert_eq!(moved.x, 0.);
        assert_eq!(
            floating_settings_bounds(virtual_screen, primary, Some(vec2(9999., 100.))).x,
            2970.
        );
    }
}

/// Macroquad exposes logical screen dimensions (already DPI-normalized).
/// Do not divide desktop UI dimensions or mouse input by DPI again.
fn ui_scale_for_frame(desktop_mode: bool, logical_width: f32, logical_height: f32) -> f32 {
    if desktop_mode {
        1.0
    } else {
        (logical_width / 1280.).min(logical_height / 800.).max(0.2)
    }
}

fn workspace_camera(width: f32, height: f32) -> Camera2D {
    // Screen cameras invert Y internally; positive zoom keeps (0, 0) at the top left.
    Camera2D {
        target: vec2(width / 2., height / 2.),
        zoom: vec2(2. / width, 2. / height),
        ..Default::default()
    }
}

#[cfg(test)]
mod drag_placement_tests {
    use super::*;

    #[test]
    fn moving_between_groups_changes_only_pins_not_the_disk_file() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let before = std::fs::metadata(&path).unwrap().len();
        let mut state = State::empty();
        let bounds = Bounds {
            x: 0.,
            y: 0.,
            w: 300.,
            h: 220.,
        };
        let from = state.add_group("From", bounds);
        let to = state.add_group("To", Bounds { x: 310., ..bounds });
        state.import(&path, from).unwrap();
        let duplicate = state.add_group("Old duplicate", Bounds { x: 650., ..bounds });
        state.import(&path, duplicate).unwrap();
        let item_id = state.group(from).unwrap().tab().items[0].id;
        state.transfer(from, item_id, to, 0).unwrap();
        assert!(state.group(from).unwrap().tab().items.is_empty());
        assert!(
            state.group(duplicate).unwrap().tab().items.is_empty(),
            "Old duplicate pin must not reappear"
        );
        assert_eq!(state.group(to).unwrap().tab().items[0].id, item_id);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
        state.unassign(&path.to_string_lossy());
        assert!(state.group(to).unwrap().tab().items.is_empty());
        assert!(
            path.exists(),
            "Ungrouping a desktop icon must not delete its file"
        );
    }
}

#[cfg(test)]
mod camera_tests {
    use super::*;
    use macroquad::camera::Camera;

    #[test]
    fn desktop_ui_respects_macroquad_logical_dpi_at_100_125_150_percent() {
        // screen_width() and mouse_position() are both pre-divided by DPI.
        // Region pixels are computed once as logical*dpi in host_after_frame.
        for (physical_width, dpi) in [(1920., 1.), (1920., 1.25), (1920., 1.5)] {
            let logical_width = physical_width / dpi;
            let scale = ui_scale_for_frame(true, logical_width, 1200. / dpi);
            assert_eq!(scale, 1.);
            let ui_width = logical_width / scale;
            let logical_button_w = 870.;
            let clip_px = logical_button_w * dpi;
            // On both the GPU and Win32 region sides the same number of
            // physical pixels must correspond to one logical UI length.
            let render_px = logical_button_w * physical_width / ui_width;
            assert!(
                (clip_px - render_px).abs() < 0.001,
                "dpi={dpi}: render {render_px}px != region {clip_px}px"
            );
        }
        assert!((ui_scale_for_frame(false, 1600., 900.) - 1.125).abs() < 0.001);
    }

    #[test]
    fn workspace_projection_matches_pointer_coordinates_after_resizing() {
        for (width, height) in [(1280., 800.), (1600., 800.), (1280., 1000.)] {
            let matrix = workspace_camera(width, height).matrix();
            for point in [vec2(0., 0.), vec2(width, height), vec2(110., 145.)] {
                let projected = matrix.transform_point3(vec3(point.x, point.y, 0.));
                let screen = vec2(
                    (projected.x + 1.) * width / 2.,
                    (1. - projected.y) * height / 2.,
                );
                assert!(
                    screen.distance(point) < 0.001,
                    "{point:?} rendered at {screen:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod rollup_reveal_tests {
    use super::*;
    #[test]
    fn ctrl_click_and_box_marquee_support_batch_drag_to_another_box() {
        let mut app = App::new(
            std::env::temp_dir().join("desktopcraft-ui-batch-fixture"),
            true,
            false,
        );
        app.state = State::empty();
        let a = app.state.add_group(
            "Source",
            Bounds {
                x: 100.,
                y: 100.,
                w: 310.,
                h: 300.,
            },
        );
        let b = app.state.add_group(
            "Destination",
            Bounds {
                x: 450.,
                y: 100.,
                w: 310.,
                h: 300.,
            },
        );
        let tid = app.state.group(a).unwrap().tab().id;
        for id in [91, 92, 93] {
            app.state.group_mut(a).unwrap().tab_mut().items.push(Item {
                id,
                name: format!("Craft{id}.zip"),
                path: format!("C:/Desktop/Craft{id}.zip"),
                kind: "zip".into(),
                size: 1,
                modified: 0,
                label: None,
            });
        }
        let cells = [
            Hit {
                bounds: Bounds {
                    x: 120.,
                    y: 170.,
                    w: 80.,
                    h: 80.,
                },
                target: Target::Item(a, 91),
            },
            Hit {
                bounds: Bounds {
                    x: 215.,
                    y: 170.,
                    w: 80.,
                    h: 80.,
                },
                target: Target::Item(a, 92),
            },
            Hit {
                bounds: Bounds {
                    x: 310.,
                    y: 170.,
                    w: 80.,
                    h: 80.,
                },
                target: Target::Item(a, 93),
            },
        ];
        app.pointer(
            &Input {
                pos: vec2(140., 190.),
                pressed: true,
                ctrl: true,
                time: 1.,
                ..Default::default()
            },
            &cells,
        );
        app.pointer(
            &Input {
                pos: vec2(230., 190.),
                pressed: true,
                ctrl: true,
                time: 2.,
                ..Default::default()
            },
            &cells,
        );
        assert_eq!(app.multi_selected.len(), 2);
        app.pointer(
            &Input {
                pos: vec2(140., 190.),
                pressed: true,
                time: 3.,
                ..Default::default()
            },
            &cells,
        );
        app.pointer(
            &Input {
                pos: vec2(470., 180.),
                released: true,
                time: 3.2,
                ..Default::default()
            },
            &cells,
        );
        assert_eq!(app.state.group(b).unwrap().tab().items.len(), 2);
        assert_eq!(app.state.group(a).unwrap().tab().items[0].id, 93);
        // Create a blank-space marquee over the remaining item and verify
        // the selection uses actual item-hit geometry in the active tab.
        let body = Hit {
            bounds: Bounds {
                x: 100.,
                y: 140.,
                w: 310.,
                h: 260.,
            },
            target: Target::Body(a),
        };
        let source_hits = vec![body, cells[2].clone()];
        app.pointer(
            &Input {
                pos: vec2(300., 155.),
                pressed: true,
                time: 4.,
                ..Default::default()
            },
            &source_hits,
        );
        app.pointer(
            &Input {
                pos: vec2(399., 269.),
                released: true,
                time: 4.2,
                ..Default::default()
            },
            &source_hits,
        );
        assert_eq!(app.multi_selected, vec![(a, tid, 93)]);
    }

    #[test]
    fn normal_boxes_change_pins_without_moving_files() {
        let root = std::env::temp_dir().join(format!(
            "desktop-spaces-virtual-box-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let original = root.join("sample.txt");
        std::fs::write(&original, "original bytes unchanged").unwrap();
        let mut app = App::new(root.join("workspace"), true, false);
        app.state = State::empty();
        let a = app.state.add_group(
            "Regular A",
            Bounds {
                x: 10.,
                y: 10.,
                w: 200.,
                h: 200.,
            },
        );
        let b = app.state.add_group(
            "Regular B",
            Bounds {
                x: 220.,
                y: 10.,
                w: 200.,
                h: 200.,
            },
        );
        app.state.import(&original, a).unwrap();
        app.drop_path_into_box(&original.to_string_lossy(), b, 1.)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&original).unwrap(),
            "original bytes unchanged"
        );
        assert!(!app
            .state
            .group(a)
            .unwrap()
            .tab()
            .items
            .iter()
            .any(|i| i.path == original.to_string_lossy()));
        assert!(app
            .state
            .group(b)
            .unwrap()
            .tab()
            .items
            .iter()
            .any(|i| i.path == original.to_string_lossy()));
        // Re-dropping the same pin must not duplicate it or change the file.
        app.drop_path_into_box(&original.to_string_lossy(), b, 2.)
            .unwrap();
        assert_eq!(app.state.group(b).unwrap().tab().items.len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn only_a_linked_folder_view_moves_the_disposable_test_file() {
        crate::shell::init();
        let root = std::env::temp_dir().join(format!(
            "desktop-spaces-linked-folder-test-{}",
            std::process::id()
        ));
        let source_dir = root.join("source");
        let linked = root.join("linked");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::create_dir_all(&linked).unwrap();
        let original = source_dir.join("sample.txt");
        std::fs::write(&original, "only moved when linked").unwrap();
        let mut app = App::new(root.join("workspace"), true, false);
        app.state = State::empty();
        let linked_id = app
            .state
            .add_portal(
                &linked,
                Bounds {
                    x: 10.,
                    y: 10.,
                    w: 200.,
                    h: 200.,
                },
            )
            .unwrap();
        app.drop_path_into_box(&original.to_string_lossy(), linked_id, 1.)
            .unwrap();
        let destination = linked.join("sample.txt");
        assert!(!original.exists());
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "only moved when linked"
        );
        assert!(app
            .state
            .group(linked_id)
            .unwrap()
            .tab()
            .items
            .iter()
            .any(|i| i.path == destination.to_string_lossy()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn classic_file_menu_command_pins_without_moving_the_source_file() {
        let root = std::env::temp_dir().join(format!(
            "desktopcraft-shell-pin-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("original with spaces.txt");
        std::fs::write(&source, "stay here").unwrap();
        let mut app = App::new(root.join("workspace"), true, false);
        app.state = State::empty();
        app.run_command(&format!("pin-file {}", source.display()), 1.);
        let box_ref = app.state.groups.first().unwrap();
        assert!(box_ref.tab().folder.is_none());
        assert_eq!(box_ref.tab().items.len(), 1);
        assert_eq!(box_ref.tab().items[0].path, source.to_string_lossy());
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "stay here");
        app.run_command(&format!("pin-file {}", source.display()), 2.);
        assert_eq!(app.state.groups[0].tab().items.len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn box_type_ahead_selects_latin_and_thai_and_scrolls_into_view() {
        let root = std::env::temp_dir().join("desktop-spaces-typeahead-test");
        let mut app = App::new(root, true, false);
        app.state = State::empty();
        let id = app.state.add_group(
            "Alphabet",
            Bounds {
                x: 50.,
                y: 50.,
                w: 180.,
                h: 180.,
            },
        );
        for i in 0..18 {
            app.state.group_mut(id).unwrap().tab_mut().items.push(Item {
                id: i + 100,
                name: format!("item{i}.txt"),
                path: format!("item{i}.txt"),
                kind: "txt".into(),
                size: 0,
                modified: 0,
                label: None,
            });
        }
        app.state.group_mut(id).unwrap().tab_mut().items.push(Item {
            id: 900,
            name: "Apple.txt".into(),
            path: "Apple.txt".into(),
            kind: "txt".into(),
            size: 0,
            modified: 0,
            label: None,
        });
        app.state.group_mut(id).unwrap().tab_mut().items.push(Item {
            id: 901,
            name: "กุญแจ.txt".into(),
            path: "กุญแจ.txt".into(),
            kind: "txt".into(),
            size: 0,
            modified: 0,
            label: None,
        });
        app.selected = Some((id, 0));
        app.shortcuts(&Input {
            typed: "a".into(),
            time: 2.,
            ..Default::default()
        });
        assert_eq!(app.selected, Some((id, 900)));
        assert!(app.state.group(id).unwrap().tab().scroll > 0);
        app.shortcuts(&Input {
            typed: "ก".into(),
            time: 4.,
            ..Default::default()
        });
        assert_eq!(app.selected, Some((id, 901)));
    }

    #[test]
    fn first_start_initializes_three_boxes_only_for_missing_workspace() {
        let root =
            std::env::temp_dir().join(format!("desktopcraft-new-defaults-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = App::new(root.clone(), false, true);
        assert!(app.host.as_ref().unwrap().first_run);
        assert_eq!(
            app.state
                .groups
                .iter()
                .map(|g| g.tab().name.as_str())
                .collect::<Vec<_>>(),
            vec!["Shortcuts", "Documents", "Folders"]
        );
        assert_eq!(app.state.rules.len(), 3);
        assert!(app
            .state
            .rules
            .iter()
            .any(|rule| rule.mode == 10 && rule.enabled));
        assert!(!app.state.auto_rules);
        drop(app);
        let path = root.join("data/workspace.json");
        let mut old = State::empty();
        old.add_group(
            "Keep my boxes",
            Bounds {
                x: 10.,
                y: 10.,
                w: 300.,
                h: 260.,
            },
        );
        old.prefs.peek_shortcut = "Ctrl+Shift+P".into();
        old.save(&path).unwrap();
        let reopened = App::new(root.clone(), false, true);
        assert_eq!(reopened.state.groups.len(), 1);
        assert_eq!(reopened.state.groups[0].tab().name, "Keep my boxes");
        assert_eq!(reopened.state.prefs.peek_shortcut, "Ctrl+Shift+P");
        assert!(!reopened.host.as_ref().unwrap().first_run);
        drop(reopened);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inactive_tab_drop_and_reorder_preserve_files_rules_and_active_tab_preference() {
        let root =
            std::env::temp_dir().join(format!("desktopcraft-tab-actions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut app = App::new(root.join("workspace"), true, false);
        app.state = State::empty();
        let b = Bounds {
            x: 40.,
            y: 80.,
            w: 340.,
            h: 280.,
        };
        let group = app.state.add_group("Main", b);
        let second = app.state.add_group("Other", b);
        let destination = app.state.group(second).unwrap().tab().id;
        assert!(app.state.merge(second, group));
        let source = app.state.group(group).unwrap().tab().id;
        let path = root.join("first.txt");
        std::fs::write(&path, b"untouched").unwrap();
        app.drop_path_into_tab(&path.to_string_lossy(), group, destination, 1.)
            .unwrap();
        assert_eq!(app.state.group(group).unwrap().tab().id, source);
        assert_eq!(app.state.group(group).unwrap().tabs[1].items.len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), b"untouched");
        app.state.prefs.switch_tab_after_drop = true;
        let next = root.join("second.txt");
        std::fs::write(&next, b"still unchanged").unwrap();
        app.drop_path_into_tab(&next.to_string_lossy(), group, destination, 2.)
            .unwrap();
        assert_eq!(app.state.group(group).unwrap().tab().id, destination);
        let bounds = app.state.group(group).unwrap().header_tabs().0;
        assert!(bounds.iter().all(|(_, r)| r.x + r.w <= b.x + b.w - 82.));
        app.state.rules.push(Rule {
            mode: 0,
            pattern: "txt".into(),
            target: group,
            target_tab: Some(destination),
            enabled: true,
        });
        assert!(app.state.reorder_tab(group, destination, 0));
        assert_eq!(app.state.group(group).unwrap().tabs[0].id, destination);
        assert_eq!(app.state.group(group).unwrap().active, 0);
        assert_eq!(
            app.state.rule_destination(app.state.rules.last().unwrap()),
            Some((group, destination))
        );
        assert_eq!(std::fs::read(&next).unwrap(), b"still unchanged");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn stacked_rollups_keep_first_hovered_box_open_until_pointer_exits() {
        let root = std::env::temp_dir().join("desktopcraft-stacked-rollup-test");
        let mut app = App::new(root, true, false);
        app.state = State::empty();
        let upper = app.state.add_group(
            "Top",
            Bounds {
                x: 100.,
                y: 100.,
                w: 240.,
                h: 260.,
            },
        );
        let lower = app.state.add_group(
            "Below",
            Bounds {
                x: 100.,
                y: 335.,
                w: 240.,
                h: 260.,
            },
        );
        app.state.group_mut(upper).unwrap().rolled = true;
        app.state.group_mut(lower).unwrap().rolled = true;
        let pointer = |y: f32, time| Input {
            pos: vec2(140., y),
            time,
            ..Default::default()
        };
        app.state.prefs.rollup_open = 0;
        app.update_rollup_reveal(&pointer(115., 1.));
        assert_eq!(app.revealed, Some(upper));
        // The lower title is underneath the open upper body's last 25 px.
        app.update_rollup_reveal(&pointer(350., 1.1));
        assert_eq!(app.revealed, Some(upper));
        assert_eq!(
            paint_order(&app.state.groups, 0, app.revealed)
                .map(|g| g.id)
                .collect::<Vec<_>>(),
            vec![lower, upper]
        );
        // Only AFTER leaving the upper bounds may the lower title respond.
        app.update_rollup_reveal(&pointer(365., 1.2));
        assert_eq!(app.revealed, Some(lower));
        app.state.prefs.rollup_open = 1;
        app.revealed = None;
        app.hover_rolled = None;
        app.update_rollup_reveal(&pointer(115., 2.));
        app.update_rollup_reveal(&pointer(115., 2. + ROLLUP_DELAY + 0.01));
        assert_eq!(app.revealed, Some(upper));
        app.update_rollup_reveal(&pointer(350., 3.));
        assert_eq!(app.revealed, Some(upper));
        app.update_rollup_reveal(&pointer(365., 3.1));
        assert_eq!(app.revealed, None);
        app.update_rollup_reveal(&pointer(365., 3.1 + ROLLUP_DELAY + 0.01));
        assert_eq!(app.revealed, Some(lower));
        app.state.prefs.rollup_open = 2;
        app.revealed = Some(upper);
        app.update_rollup_reveal(&pointer(350., 4.));
        assert_eq!(app.revealed, Some(upper));
        app.update_rollup_reveal(&pointer(365., 4.1));
        assert_eq!(app.revealed, None);
    }

    #[test]
    fn reveal_requires_tab_hover_and_keeps_contents_usable() {
        let root = std::env::temp_dir().join("desktop-spaces-rollup-test-data");
        let mut app = App::new(root, true, false);
        app.state = State::empty();
        let id = app.state.add_group(
            "Rolled",
            Bounds {
                x: 100.,
                y: 100.,
                w: 240.,
                h: 230.,
            },
        );
        app.state.group_mut(id).unwrap().rolled = true;
        let pointer = |x, y, time| Input {
            pos: vec2(x, y),
            time,
            ..Default::default()
        };
        // The phantom body must not open the rolled group.
        app.state.prefs.rollup_open = 0;
        app.update_rollup_reveal(&pointer(140., 260., 1.));
        assert_eq!(app.revealed, None);
        // Only touching the title/tabs initiates the reveal.
        app.update_rollup_reveal(&pointer(140., 115., 1.1));
        assert_eq!(app.revealed, Some(id));
        // Once open, moving into the actual content must not instantly close it.
        app.update_rollup_reveal(&pointer(140., 260., 1.2));
        assert_eq!(app.revealed, Some(id));
        app.update_rollup_reveal(&pointer(500., 500., 1.3));
        assert_eq!(app.revealed, None);
        app.state.prefs.rollup_open = 1;
        app.update_rollup_reveal(&pointer(140., 115., 2.));
        assert_eq!(app.revealed, None);
        app.update_rollup_reveal(&pointer(140., 115., 2. + ROLLUP_DELAY + 0.01));
        assert_eq!(app.revealed, Some(id));
        app.update_rollup_reveal(&pointer(140., 260., 3.));
        assert_eq!(app.revealed, Some(id));
        app.update_rollup_reveal(&pointer(500., 500., 3.1));
        assert_eq!(app.revealed, None);
    }
}

#[macroquad::main(configuration)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let option = |name: &str| {
        args.iter()
            .position(|x| x == name)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    let text_check = option("--text-check");
    let settings_capture = args
        .iter()
        .position(|a| a == "--settings-screenshot")
        .and_then(|i| {
            Some((
                args.get(i + 1)?.parse::<usize>().ok()?,
                PathBuf::from(args.get(i + 2)?),
            ))
        });
    let modern_menu_capture = args
        .iter()
        .any(|a| a == "--modern-menu-screenshot" || a == "--modern-menu-preview");
    let shot = option("--screenshot")
        .or_else(|| option("--modern-menu-screenshot"))
        .or_else(|| text_check.clone())
        .or_else(|| settings_capture.as_ref().map(|(_, path)| path.clone()));
    let test = option("--self-test");
    let perf_path = option("--perf-report");
    if perf_path.is_some() {
        perf::enable();
    }
    let automation = shot.is_some() || test.is_some();
    let integrated_preview = args.iter().any(|a| a == "--native-integrated-preview");
    #[cfg(all(windows, feature = "native-lab"))]
    if integrated_preview
        && (std::env::var("DESKTOPCRAFT_ENABLE_NATIVE_BOX_LAB")
            .ok()
            .as_deref()
            != Some("1")
            || std::env::var("DESKTOPCRAFT_NATIVE_BOXES").ok().as_deref() != Some("1"))
    {
        eprintln!("Native integration preview requires both explicit native-lab environment flags");
        std::process::exit(3);
    }
    let desktop_mode = host_mode() && !automation && !integrated_preview;
    // Desktop mode uses the existing Desktop Spaces workspace if found, or
    // %APPDATA%\DesktopCraft for new users. Window mode uses sample data.
    let root = if integrated_preview {
        std::env::temp_dir().join(format!(
            "DesktopCraftNativeIntegrated-{}",
            std::process::id()
        ))
    } else if desktop_mode {
        host::data_dir()
    } else {
        app_root()
    };
    if !desktop_mode {
        if let Err(e) = create_demo(&root) {
            eprintln!("Could not prepare sample files: {e}");
            std::process::exit(1)
        }
    }
    // COM/OLE for Explorer's icons (all modes) and the shell (desktop mode).
    shell::init();
    let mut app = App::new(root.clone(), automation, desktop_mode);
    if modern_menu_capture {
        if let Some((group, item, path)) = app
            .state
            .groups
            .iter()
            .find_map(|g| g.tab().items.first().map(|i| (g.id, i.id, i.path.clone())))
        {
            app.selected = Some((group, item));
            app.file_menu = Some(ModernFilePopup {
                path,
                pos: vec2(154., 169.),
            });
        }
    }
    if integrated_preview {
        // Full integration fixture: the ACTUAL DesktopCraft State/Group/Tab
        // model owns two Tabs, one virtual Pins and one linked Windows folder.
        // Everything points into this preview's unique disposable TEMP root.
        if let Some(id) = app.state.groups.first().map(|g| g.id) {
            if app.state.add_tab(id).is_ok() {
                if let Some(g) = app.state.group_mut(id) {
                    let tab = g.tab_mut();
                    tab.name = "Native Notes".into();
                    tab.folder = Some(root.join("demo/Notes").to_string_lossy().to_string());
                    g.active = 0;
                }
            }
        }
    }
    if let Some((page, _)) = &settings_capture {
        // Page 99 is not a real page: it shows the rule pop-up editor open on
        // rule 2, over the Organize & rules page.
        let editor = *page == 99;
        let page = if editor { 2 } else { *page };
        app.dialog = Some(Dialog::Settings {
            page,
            pattern: String::new(),
            mode: 0,
            target: 0,
        });
        // Populate a representative long-rules view only for the isolated
        // screenshot command. It never touches the user's workspace.
        if page == 2 && app.state.rules.is_empty() {
            if let Some(destination) = app
                .state
                .groups
                .iter()
                .find(|g| g.tab().folder.is_none())
                .map(|g| g.id)
            {
                for (mode, pattern) in [
                    (0, "lnk,url,appref-ms"),
                    // Longer than the match field, to show it scrolling to its end.
                    (0, "pdf,doc,docx,txt,rtf,odt,xls,xlsx,csv,ppt,pptx,md,epub,pages,numbers,key,odp,ods"),
                    (0, "zip,rar,7z,exe,msi,iso"),
                ] {
                    app.state.rules.push(Rule {
                        mode, pattern: pattern.into(), target: destination, target_tab: None, enabled: true,
                    });
                }
            }
        }
        if editor && app.state.rules.len() > 1 {
            app.dialog = Some(app.rule_editor(1));
        }
        // Page 98: no Settings, an icon being renamed in place instead.
        if page == 98 {
            app.dialog = None;
            let target = app.state.groups.iter().find_map(|g| {
                g.tab()
                    .items
                    .iter()
                    .find(|i| i.kind != "folder" && i.name.contains('.'))
                    .map(|i| (g.id, i.id))
            });
            if let Some((group, item)) = target {
                app.start_inline_rename(group, item);
            }
        }
    }
    if desktop_mode && !automation {
        host::event(
            "INFO",
            "ui.initialized",
            &format!(
                "version={} boxes={} pages={}",
                env!("CARGO_PKG_VERSION"),
                app.state.groups.len(),
                app.state.pages.len()
            ),
        );
    }
    let smoke = std::env::var_os("DESKTOPCRAFT_SMOKE").is_some()
        || std::env::var_os("DESKTOP_SPACES_SMOKE").is_some();
    if desktop_mode {
        if smoke {
            if let Some(h) = app.host.as_mut() {
                h.first_run = false;
            }
        }
        app.host_start();
        if let Ok(mut hook) = macroquad::miniquad::ON_END_SESSION.lock() {
            *hook = Some(|| {
                host::log("session ending: Explorer icons shown again");
                desktop::restore_explorer_icons();
            });
        }
        if let Some(h) = &app.host {
            host::log(&format!(
                "desktop: {} monitor(s) [{}], virtual {:?}, scale {}",
                h.monitors.len(),
                h.signature,
                h.virtual_rect,
                h.scale
            ));
        }
    }
    // Windows fonts are not redistributed with DesktopCraft. Prefer a font
    // known to cover Thai, and record which one was actually loaded.
    let mut font = None;
    let mut chosen_font = "macroquad-fallback";
    for name in ["LeelawUI.ttf", "tahoma.ttf", "segoeui.ttf"] {
        if let Ok(loaded) = load_ttf_font(&format!("C:/Windows/Fonts/{name}")).await {
            font = Some(loaded);
            chosen_font = name;
            break;
        }
    }
    if desktop_mode && !automation {
        host::event(
            if font.is_some() { "INFO" } else { "WARN" },
            "render.font_selected",
            &format!(
                "font={} dpi={:.2}",
                chosen_font,
                macroquad::window::screen_dpi_scale()
            ),
        );
    }
    let mut p = Painter::new(font, cfg!(windows));
    #[cfg(all(windows, feature = "native-lab"))]
    let mut native_boxes =
        native_integrated::NativeWorkspaceController::new(&root, integrated_preview);
    if desktop_mode && !automation {
        host::event(
            "INFO",
            "render.font_metrics",
            &format!(
                "settings_title_width={:.1} thai_sample_width={:.1}",
                p.measure("Settings", 26.),
                p.measure("ภาษาไทย", 14.)
            ),
        );
    }
    // Only the self-test needs the scripted harness (it expects the sample groups;
    // a real desktop may have none yet).
    let mut harness = test.as_ref().map(|_| verification::Harness::new(&app));
    #[cfg(windows)]
    let updater = if desktop_mode && !automation { updates::Client::start() } else { None };
    let mut frame = 0u32;
    // Check whether persisted data changed on a modest cadence rather than
    // serializing the full workspace twice on every rendered frame.
    let mut last_saved_state = if automation {
        Vec::new()
    } else {
        serde_json::to_vec(&app.state).unwrap_or_default()
    };
    let mut last_seen_state = last_saved_state.clone();
    let mut next_state_check_at = 0.0f64;
    let mut last_edit_at = 0.0f64;
    let mut previous_frame_start: Option<std::time::Instant> = None;
    loop {
        let perf_frame_start = std::time::Instant::now();
        #[cfg(feature = "profile-alloc")]
        let allocation_start = alloc_profile::snapshot();
        if let Some(previous) = previous_frame_start {
            perf::record(
                "frame_interval_ms",
                perf_frame_start.duration_since(previous),
            );
        }
        previous_frame_start = Some(perf_frame_start);
        // Macroquad already divides its public screen_width()/screen_height()
        // and mouse_position() by Miniquad's device DPI scale. Applying the
        // desktop monitor scale *again* made OpenGL draw all UI at DPI² while
        // SetWindowRgn clipped in physical pixels at DPI, producing rectangular
        // holes in Settings at 125%/150% Windows display scaling.
        // The monitor's physical/DIP conversion still belongs in host_scale(),
        // from_screen() and host_after_frame() (not in the UI camera).
        let scale = ui_scale_for_frame(desktop_mode, screen_width(), screen_height());
        p.regions.borrow_mut().clear();
        p.icon_budget.set(6);
        p.collect_icons();
        app.width = screen_width() / scale;
        app.height = screen_height() / scale;
        if desktop_mode && frame == 2 && !automation {
            let physical = host::virtual_rect();
            host::event(
                "INFO",
                "ui.dpi_geometry",
                &format!(
                    "logical={:.0}x{:.0} miniquad_dpi={:.2} monitor_dpi={:.2} physical={}x{}",
                    app.width,
                    app.height,
                    macroquad::window::screen_dpi_scale(),
                    app.host_scale(),
                    physical[2],
                    physical[3]
                ),
            );
        }
        p.width = app.width;
        p.height = app.height;
        set_camera(&workspace_camera(app.width, app.height));
        let mut input = Input::collect(scale);
        if test.is_some() {
            input = harness.as_ref().unwrap().input(frame, &app);
        } else if shot.is_some() {
            input = Input {
                pos: vec2(-100., -100.),
                time: frame as f64 / 60.,
                ..Default::default()
            };
        }
        #[cfg(all(windows, feature = "native-lab"))]
        if !automation {
            native_boxes.consume_input(&mut app, &mut input);
        }
        // Region-clipped desktop groups can lose Win32 mouse-up/move messages
        // while dragging into another group or across the uncovered desktop.
        // Complete the gesture using the real cursor/button state, never by
        // trusting a stale local mouse position from the last owned region.
        if desktop_mode
            && !automation
            && (matches!(
                app.drag.as_ref(),
                Some(
                    Drag::Item { .. }
                        | Drag::BoxSelect { .. }
                        | Drag::Tab { .. }
                        | Drag::Group { .. }
                        | Drag::Resize { .. }
                        | Drag::Scroll { .. }
                )
            ) || app.host.as_ref().is_some_and(|h| h.press.is_some())
                || app.settings_drag_offset.is_some())
        {
            let down = host::mouse_left_down();
            if !down {
                input.released = true;
            }
            input.down = down;
            let (mx, my) = host::cursor();
            input.pos = app.from_screen(mx, my);
        }
        // A region-clipped desktop host often receives no WM_MOUSEMOVE over
        // transparent areas. Follow the real global cursor every frame so the
        // page selector is drawn as the pointer approaches the primary screen.
        if desktop_mode && !automation {
            let (mx, my) = host::cursor();
            input.pos = app.from_screen(mx, my);
        }
        p.mouse = input.pos;
        if !automation && app.color_pick_target.is_some() {
            if let Some(color) = host::finish_color_pick() {
                let index = app.color_pick_target.take().unwrap_or(0);
                if let Some(rgb) = color {
                    let group = if index == 0 {
                        None
                    } else {
                        app.state.groups.get(index - 1).map(|g| g.id)
                    };
                    for g in &mut app.state.groups {
                        if group.is_none_or(|id| id == g.id) {
                            g.custom_color = Some(rgb);
                        }
                    }
                    app.notify(
                        format!(
                            "Picked screen pixel: #{:02X}{:02X}{:02X}",
                            (rgb[0] * 255.).round() as u8,
                            (rgb[1] * 255.).round() as u8,
                            (rgb[2] * 255.).round() as u8
                        ),
                        input.time,
                    );
                } else {
                    app.notify(
                        "Screen color selection cancelled or unavailable",
                        input.time,
                    );
                }
                app.dialog = Some(Dialog::Settings {
                    page: 0,
                    pattern: String::new(),
                    mode: 0,
                    target: index,
                });
            }
        }
        if !automation {
            // Settings (except during recording) can preview Peek too.
            let available =
                app.dialog.is_none() || matches!(app.dialog, Some(Dialog::Settings { .. }));
            let hotkey = available
                && !app.shortcut_recording
                && app.state.prefs.peek_hotkeys
                && platform::hotkey(&app.state.prefs.peek_shortcut);
            // Peek is a temporary reveal while the combination is held.
            // Releasing any key returns the boxes to their desktop layer.
            if hotkey != app.peek {
                app.set_peek(hotkey);
            }
            let dropped: Vec<PathBuf> = get_dropped_files()
                .into_iter()
                .filter_map(|f| f.path)
                .collect();
            if desktop_mode {
                if !dropped.is_empty() {
                    app.drop_files(dropped, input.time, macroquad::miniquad::take_drop_screen());
                }
            } else {
                for path in dropped {
                    app.import_path(&path, path.is_dir(), input.time)
                }
            }
        }
        if desktop_mode {
            let start = std::time::Instant::now();
            app.host_tick(input.time);
            perf::record("host_tick_ms", start.elapsed());
        }
        #[cfg(windows)]
        if let Some(updater) = &updater {
            while let Some(event) = updater.poll() {
                match event {
                    updates::Event::Available(release) => {
                        if host::ask_default_no(&format!(
                            "DesktopCraft {} is available on GitHub.\n\nCurrent version: {}\n\nDownload, verify, and update now? Your Boxes, Pages and Settings will be kept.",
                            release.version, env!("CARGO_PKG_VERSION")
                        )) {
                            app.notify("Downloading the new DesktopCraft version...", input.time);
                            updater.download(release);
                        }
                    }
                    updates::Event::Ready(installer) => {
                        // The worker validated the exact release asset, length and SHA-256.
                        // The installer does the normal in-place upgrade, saving the live
                        // workspace before it replaces the running executable.
                        match std::process::Command::new(installer)
                            .args(["--install", "--approved-update"])
                            .spawn()
                        {
                            Ok(_) => app.notify("Starting the verified update...", input.time),
                            Err(_) => app.notify("Could not start the verified installer", input.time),
                        }
                    }
                    updates::Event::Failure(error) => app.notify(error, input.time),
                }
            }
        }
        let render_start = std::time::Instant::now();
        app.frame(&mut p, &input);
        #[cfg(all(windows, feature = "native-lab"))]
        if !automation {
            native_boxes.render(&app, scale);
            if integrated_preview && frame % 30 == 0 {
                if let Some(dest) = std::env::var_os("DESKTOPCRAFT_NATIVE_INTEGRATED_REPORT") {
                    let _ = std::fs::write(
                        dest,
                        serde_json::to_vec_pretty(&native_boxes.report(&app)).unwrap_or_default(),
                    );
                }
            }
        }
        if text_check.is_some() {
            // Isolated text rendering preview on the *same* GPU, without
            // hiding desktop icons or launching the desktop host. This helps
            // distinguish missing glyphs/atlas failures from Win32 region DPI.
            let box_x = (app.width - 480.).max(10.);
            let box_y = (app.height - 280.).max(10.);
            draw_rectangle(box_x, box_y, 465., 248., Color::new(0.07, 0.10, 0.16, 0.98));
            draw_rectangle_lines(box_x, box_y, 465., 248., 1., ACCENT);
            p.text("TEXT RENDER CHECK", box_x + 15., box_y + 12., 24., TEXT);
            p.text(
                "ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789",
                box_x + 15.,
                box_y + 49.,
                16.,
                TEXT,
            );
            p.text(
                "Settings  •  Behavior  •  Pages",
                box_x + 15.,
                box_y + 83.,
                18.,
                TEXT,
            );
            p.text("ภาษาไทย ก ข ฃ ค ง", box_x + 15., box_y + 123., 20., TEXT);
            p.text(
                "ทดสอบวรรณยุกต์ เก่ง ใกล้ เปิดโฟลเดอร์",
                box_x + 15.,
                box_y + 161.,
                17.,
                TEXT,
            );
            p.text(
                "small text: 11px 123 abc กขค",
                box_x + 15.,
                box_y + 206.,
                11.,
                MUTED,
            );
        }
        perf::record("ui_render_ms", render_start.elapsed());
        if desktop_mode {
            app.host_after_frame(&p);
            app.run_pending(input.time, &p);
            if smoke && frame == 180 {
                let h = app.host.as_ref().unwrap();
                #[cfg(windows)]
                let window = !host::window().is_null();
                #[cfg(not(windows))]
                let window = false;
                let report = serde_json::json!({
                    "frames": frame,
                    "window_found": window,
                    "screen": [screen_width(), screen_height()],
                    "monitors": h.monitors.len(),
                    "virtual_rect": h.virtual_rect,
                    "scale": h.scale,
                    "desktop_synced": h.synced_once,
                    "region_parts": h.region.len(),
                    "explorer_icons_hidden_while_running": app.state.explorer_icons_hidden,
                    "desktop_icons_drawn": h.loose.len(),
                    "log": std::fs::read_to_string(host::data_dir().join("desktopcraft.log")).unwrap_or_default(),
                });
                if let Some(out) = std::env::var_os("DESKTOPCRAFT_SMOKE_REPORT")
                    .or_else(|| std::env::var_os("DESKTOP_SPACES_SMOKE_REPORT"))
                {
                    let _ =
                        std::fs::write(out, serde_json::to_vec_pretty(&report).unwrap_or_default());
                }
                app.host_exit();
                break;
            }
            if app.host.as_ref().is_some_and(|h| h.quit) {
                app.host_exit();
                app.save(input.time);
                break;
            }
        }
        if !automation {
            let due = input.time >= next_state_check_at
                || input.released
                || input.right_released
                || input.key(KeyCode::Enter);
            if due {
                let current = serde_json::to_vec(&app.state).unwrap_or_default();
                if current != last_seen_state {
                    last_seen_state = current;
                    last_edit_at = input.time;
                }
                next_state_check_at = input.time + 0.25;
            }
            // Coalesce repeated slider, drag and page edits into a single save.
            // Saving remains synchronous/atomic as before, but is never done
            // during a live drag. Normal shutdown has its own unconditional save.
            if app.save_allowed
                && last_seen_state != last_saved_state
                && app.drag.is_none()
                && input.time - last_edit_at >= 0.4
            {
                app.save(input.time);
                if app.save_ok {
                    last_saved_state = serde_json::to_vec(&app.state).unwrap_or_default();
                    last_seen_state = last_saved_state.clone();
                } else {
                    last_edit_at = input.time; // Retry later without a tight I/O loop.
                }
            }
        }
        if test.is_some() {
            if let Some(h) = harness.as_mut() {
                h.observe(frame, &mut app);
            }
        }
        if frame >= 120 {
            if let Some(path) = &shot {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                // export_png already converts the framebuffer's bottom-up rows.
                get_screen_data().export_png(&path.to_string_lossy());
                if text_check.is_some() {
                    let report = serde_json::json!({
                        "app": "DesktopCraft", "mode": "text-render-check",
                        "font_selected": chosen_font,
                        "dpi": macroquad::window::screen_dpi_scale(),
                        "logical_screen": [app.width, app.height],
                        "english_text_width": p.measure("Settings", 26.),
                        "thai_text_width": p.measure("ภาษาไทย", 20.),
                        "note": "Screenshot and metrics generated in isolated sample mode; no desktop icon changes"
                    });
                    let _ = std::fs::write(
                        path.with_extension("json"),
                        serde_json::to_vec_pretty(&report).unwrap_or_default(),
                    );
                }
                break;
            }
        }
        if frame >= 366 {
            if let Some(path) = &test {
                let result = harness.as_ref().unwrap().finish(&app);
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap())
                    .expect("write self-test report");
                if !result["steps"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|x| x["passed"] == true)
                {
                    std::process::exit(1)
                }
                break;
            }
        }
        #[cfg(all(windows, feature = "native-lab"))]
        if integrated_preview
            && args.iter().any(|a| a == "--native-integrated-smoke")
            && frame >= 180
        {
            let report = native_boxes.report(&app);
            if let Some(dest) = std::env::var_os("DESKTOPCRAFT_NATIVE_INTEGRATED_REPORT") {
                let path = PathBuf::from(dest);
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ =
                    std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap_or_default());
            }
            break;
        }
        #[cfg(all(windows, feature = "native-lab"))]
        if integrated_preview && native_integrated::NativeWorkspaceController::preview_exit_hotkey()
        {
            break;
        }
        if is_quit_requested() {
            if desktop_mode {
                app.host_exit();
            }
            app.save(input.time);
            break;
        }
        perf::record("frame_work_ms", perf_frame_start.elapsed());
        #[cfg(feature = "profile-alloc")]
        if perf::enabled() {
            let after = alloc_profile::snapshot();
            perf::record_value(
                "ui_thread_alloc_calls",
                after.calls.saturating_sub(allocation_start.calls) as f64,
            );
            perf::record_value(
                "ui_thread_alloc_kib",
                after.bytes.saturating_sub(allocation_start.bytes) as f64 / 1024.0,
            );
        }
        if frame % 30 == 0 {
            perf::sample_memory();
            p.profile_cache_counts();
            if let Some(h) = &app.host {
                perf::record_value("desktop_loose_icons", h.loose.len() as f64);
            }
        }
        frame += 1;
        next_frame().await;
    }
    #[cfg(all(windows, feature = "native-lab"))]
    native_boxes.shutdown();
    if integrated_preview {
        // Only a uniquely named, PID-scoped preview fixture is removed.
        let _ = std::fs::remove_dir_all(&root);
    }
    if let Some(path) = perf_path.as_deref() {
        let mode = if desktop_mode {
            "desktop"
        } else if test.is_some() {
            "scripted_self_test"
        } else {
            "windowed"
        };
        if let Err(e) = perf::finish(path, mode, frame) {
            host::log(&format!("perf report failed: {e}"));
        }
    }
}
