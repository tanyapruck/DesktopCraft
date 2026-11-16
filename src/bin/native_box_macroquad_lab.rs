//! Round 17 - isolated, in-process Shell child attached to a Macroquad HWND.
//! DO NOT wire this binary to the real DesktopCraft overlay or saved Workspace.
//! All Shell files are created in a unique %TEMP% directory by the Round16 lab.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
#[path = "native_box_sandbox.rs"]
mod native_shell;

#[cfg(windows)]
mod win {
    use std::ptr;
    use winapi::shared::{
        minwindef::{BOOL, DWORD, LPARAM},
        windef::HWND,
    };
    use winapi::um::winuser::{EnumWindows, GetClassNameW, GetWindowThreadProcessId};

    unsafe extern "system" fn find(hwnd: HWND, result: LPARAM) -> BOOL {
        let mut pid: DWORD = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != std::process::id() {
            return 1;
        }
        let mut class = [0u16; 48];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        if n > 0 && String::from_utf16_lossy(&class[..n as usize]) == "MINIQUADAPP" {
            *(result as *mut HWND) = hwnd;
            0
        } else {
            1
        }
    }
    pub fn window() -> HWND {
        let mut hwnd: HWND = ptr::null_mut();
        unsafe {
            EnumWindows(Some(find), &mut hwnd as *mut HWND as LPARAM);
        }
        hwnd
    }
}

#[cfg(windows)]
use macroquad::prelude::*;

#[cfg(windows)]
fn configuration() -> Conf {
    Conf {
        window_title: "DesktopCraft Round17 - MACROQUAD Native Box Lab (TEMP only)".into(),
        window_width: 850,
        window_height: 700,
        window_resizable: true,
        high_dpi: false,
        sample_count: 1,
        ..Default::default()
    }
}

#[cfg(windows)]
fn tab_button(label: &str, index: usize, selected: bool) -> bool {
    let x = 34. + index as f32 * 236.;
    let y = 100.;
    let w = 225.;
    let h = 34.;
    let (mx, my) = mouse_position();
    let hover = mx >= x && mx <= x + w && my >= y && my <= y + h;
    let fill = if selected {
        Color::from_rgba(195, 157, 74, 255)
    } else if hover {
        Color::from_rgba(65, 75, 91, 255)
    } else {
        Color::from_rgba(47, 55, 72, 255)
    };
    draw_rectangle(x, y, w, h, fill);
    let fg = if selected { BLACK } else { WHITE };
    draw_text(label, x + 14., y + 23., 19., fg);
    hover && is_mouse_button_pressed(MouseButton::Left)
}

#[cfg(windows)]
fn virtual_items_hint(tab: u8) {
    // Explicit rollback preview with the correct disposable tab membership.
    // These are static test labels, NOT production Virtual Box rendering.
    let items: &[&str] = match tab {
        0 => &[
            "sandbox-0.txt",
            "sandbox-1.txt",
            "sandbox-2.txt",
            "sandbox-3.txt",
        ],
        1 => &["alternate-0.txt", "alternate-1.txt"],
        _ => &["sandbox-0.txt", "alternate-0.txt"],
    };
    for (i, item) in items.iter().enumerate() {
        let x = 60. + i as f32 * 142.;
        draw_rectangle(x, 185., 118., 96., Color::from_rgba(58, 67, 86, 255));
        draw_rectangle(
            x + 42.,
            199.,
            30.,
            41.,
            Color::from_rgba(222, 226, 233, 255),
        );
        draw_text(item, x + 7., 264., 16., WHITE);
    }
    draw_text(
        "Virtual fallback: no native shell view is visible.",
        56.,
        325.,
        22.,
        Color::from_rgba(230, 210, 157, 255),
    );
}

#[cfg(windows)]
#[macroquad::main(configuration)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let smoke = args.iter().any(|a| a == "--smoke");
    if std::env::var("DESKTOPCRAFT_ENABLE_NATIVE_BOX_LAB")
        .ok()
        .as_deref()
        != Some("1")
    {
        eprintln!("Round17 lab is disabled without explicit DESKTOPCRAFT_ENABLE_NATIVE_BOX_LAB=1");
        std::process::exit(3);
    }

    let mut window = win::window();
    for _ in 0..60 {
        if !window.is_null() {
            break;
        }
        next_frame().await;
        window = win::window();
    }
    let mut host = match native_shell::windows::EmbeddedLab::new(window) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Round17 native host creation failed: {e}");
            std::process::exit(5);
        }
    };
    let embedded = host.parent_is_macroquad();
    let mut mode_native = true;
    let start = std::time::Instant::now();
    let mut stage = 0usize;
    let mut stage_since = std::time::Instant::now();
    let mut failure = String::new();
    let mut fallback_roundtrip = false;
    loop {
        let w = screen_width().max(480.);
        let h = screen_height().max(380.);
        let box_w = (w - 56.).max(340.);
        let box_h = (h - 84.).max(300.);
        clear_background(Color::from_rgba(23, 27, 37, 255));
        draw_text("DesktopCraft  /  Native Box Lab", 26., 37., 27., WHITE);
        draw_text(
            "Macroquad real parent HWND + in-process Windows Shell child | TEMP files only",
            28.,
            60.,
            16.,
            Color::from_rgba(192, 200, 212, 255),
        );
        draw_rectangle(26., 72., box_w, box_h, Color::from_rgba(39, 46, 60, 255));
        draw_rectangle_lines(
            26.,
            72.,
            box_w,
            box_h,
            2.,
            Color::from_rgba(183, 148, 76, 255),
        );
        draw_text(
            "BOX 1   /   Native Shell Integration Test",
            36.,
            94.,
            18.,
            WHITE,
        );

        let names = ["Tab A - Folder", "Tab B - Folder", "Virtual Pins A+B"];
        for (index, label) in names.iter().enumerate() {
            if tab_button(label, index, host.current_tab() == index as u8) {
                if let Err(e) = host.switch_tab(index as u8) {
                    failure = e;
                    break;
                }
            }
        }

        // A real Macroquad control is the primary fallback path: child Shell
        // HWNDs may own keyboard focus, so V alone is not reliable.
        let toggle_x = w - 192.;
        let toggle_y = 9.;
        let (mx, my) = mouse_position();
        let toggle_hover =
            mx >= toggle_x && mx <= toggle_x + 170. && my >= toggle_y && my <= toggle_y + 38.;
        draw_rectangle(
            toggle_x,
            toggle_y,
            170.,
            38.,
            if toggle_hover {
                Color::from_rgba(97, 102, 118, 255)
            } else {
                Color::from_rgba(57, 71, 92, 255)
            },
        );
        draw_text(
            if mode_native {
                "Use Virtual"
            } else {
                "Restore Native"
            },
            toggle_x + 11.,
            toggle_y + 25.,
            20.,
            WHITE,
        );
        let properties_x = w - 361.;
        let properties_hover = mx >= properties_x
            && mx <= properties_x + 154.
            && my >= toggle_y
            && my <= toggle_y + 38.;
        draw_rectangle(
            properties_x,
            toggle_y,
            154.,
            38.,
            if properties_hover {
                Color::from_rgba(97, 102, 118, 255)
            } else {
                Color::from_rgba(57, 71, 92, 255)
            },
        );
        draw_text("Properties", properties_x + 12., toggle_y + 25., 20., WHITE);
        if properties_hover && is_mouse_button_pressed(MouseButton::Left) && mode_native {
            let _ = host.properties();
        }
        let manual_toggle = toggle_hover && is_mouse_button_pressed(MouseButton::Left);
        if manual_toggle || is_key_pressed(KeyCode::V) {
            mode_native = !mode_native;
            host.set_visible(mode_native);
            if mode_native {
                fallback_roundtrip = true;
            }
        }
        if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Q) {
            break;
        }
        let inner_rect = [
            36,
            144,
            (w - 76.).max(200.) as i32,
            (h - 202.).max(160.) as i32,
        ];
        host.set_rect(inner_rect);
        host.tick();

        if !mode_native {
            draw_rectangle(
                inner_rect[0] as f32,
                inner_rect[1] as f32,
                inner_rect[2] as f32,
                inner_rect[3] as f32,
                Color::from_rgba(33, 39, 54, 255),
            );
            virtual_items_hint(host.current_tab());
        }
        draw_text(
            "V: toggle Native / Virtual fallback     Esc: close      Source files remain unchanged",
            35.,
            h - 23.,
            17.,
            Color::from_rgba(216, 200, 166, 255),
        );

        if smoke {
            // Automated readiness and fallback gates inside the same Macroquad process.
            if start.elapsed().as_secs() > 18 {
                failure = "Macroquad Shell embedding smoke timed out".into();
                break;
            }
            if host.ready() && stage_since.elapsed().as_millis() > 240 {
                let target = match stage {
                    0 => Some(1),
                    1 => Some(2),
                    2 => Some(0),
                    _ => None,
                };
                if let Some(next) = target {
                    if let Err(e) = host.switch_tab(next) {
                        failure = e;
                        break;
                    }
                    stage += 1;
                    stage_since = std::time::Instant::now();
                } else if stage == 3 && !fallback_roundtrip {
                    mode_native = false;
                    host.set_visible(false);
                    stage = 4;
                    stage_since = std::time::Instant::now();
                }
            }
            if stage == 4 && stage_since.elapsed().as_millis() > 350 {
                mode_native = true;
                host.set_visible(true);
                fallback_roundtrip = true;
                stage = 5;
                stage_since = std::time::Instant::now();
            }
            if stage == 5 && stage_since.elapsed().as_millis() > 420 {
                break;
            }
        }
        next_frame().await;
    }

    let visits = host.visited();
    let total = host.items();
    let ready = host.ready();
    let results = host.results_loaded();
    let child_parent = host.parent_is_macroquad();
    let cleaned = host.close();
    let passed = failure.is_empty()
        && embedded
        && child_parent
        && cleaned
        && (!smoke
            || (visits.iter().all(|count| *count > 0)
                && total == 2
                && results
                && ready
                && fallback_roundtrip));
    println!(
        "{}",
        serde_json::json!({
            "test":"macroquad_native_box_in_process",
            "isolated":true,
            "parent_class":"MINIQUADAPP",
            "child_attached_to_parent":embedded && child_parent,
            "tabs_visited":visits,
            "results_folder_items":total,
            "results_loaded":results,
            "virtual_fallback_roundtrip":fallback_roundtrip,
            "temp_cleanup":cleaned,
            "production_overlay_changed":false,
            "workspace_modified":false,
            "smoke":smoke,
            "passed":passed,
            "error":failure
        })
    );
    if !passed {
        std::process::exit(7);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows-only");
    std::process::exit(2);
}
