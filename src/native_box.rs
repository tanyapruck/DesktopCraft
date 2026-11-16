//! Reversible experimental bridge to the isolated Native Box Shell lab.
//!
//! CLI entry points run BEFORE DesktopCraft initializes its desktop host.
//! No Explorer icon visibility, live Workspace, registry keys, or installed
//! desktop surfaces are touched by this module. Never enable by default.
use std::path::{Path, PathBuf};
use std::process::Command;

const FLAG: &str = "DESKTOPCRAFT_ENABLE_NATIVE_BOX_LAB";
const LAB_NAME: &str = "NativeBoxRound14.ps1";
const STATUS: &str = "--native-box-status";
const SMOKE: &str = "--native-box-preview-smoke";
const PREVIEW: &str = "--native-box-preview";
const RUST_SMOKE: &str = "--native-box-rust-smoke";
const RUST_PREVIEW: &str = "--native-box-rust-preview";
const RUST_LAB_NAME: &str = "native-box-sandbox.exe";
const MACRO_SMOKE: &str = "--native-box-macroquad-smoke";
const MACRO_PREVIEW: &str = "--native-box-macroquad-preview";
const MACRO_LAB_NAME: &str = "native-box-macroquad-lab.exe";

fn enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

fn find_script_from(exe: &Path) -> Option<PathBuf> {
    // Only search up to the app project root from the *actual executable*.
    // Never take a script path or command line from untrusted user data.
    // Release installed binaries have no labs directory and fail closed.
    let mut dir = exe.parent()?;
    for _ in 0..4 {
        let candidate = dir.join("labs").join(LAB_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
    None
}

fn script() -> Option<PathBuf> {
    find_script_from(&std::env::current_exe().ok()?)
}

fn rust_lab() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(RUST_LAB_NAME);
    path.is_file().then_some(path)
}

fn macroquad_lab() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(MACRO_LAB_NAME);
    path.is_file().then_some(path)
}

fn status_value(
    feature_enabled: bool,
    available: bool,
    rust_available: bool,
    macroquad_available: bool,
) -> serde_json::Value {
    serde_json::json!({
        "component": "native_box_lab_bridge",
        "preview_available": available,
        "rust_shell_host_available": rust_available,
        "macroquad_embedded_lab_available": macroquad_available,
        "enabled": feature_enabled,
        "mode": "out_of_process_disposable_sample_only",
        "production_box_replaced": false,
        "explorer_injection": false,
        "workspace_modified": false,
        "real_desktop_icons_modified": false,
        "native_windows_11_modern_menu_verified": false,
        "shell_view_preview": "IExplorerBrowser / IResultsFolder",
        "blocked_without_explicit_opt_in": true,
        "required_flag": FLAG
    })
}

pub(crate) fn handle_cli(args: &[String]) -> Option<i32> {
    let wants_status = args.iter().any(|a| a == STATUS);
    let wants_smoke = args.iter().any(|a| a == SMOKE);
    let wants_preview = args.iter().any(|a| a == PREVIEW);
    let wants_rust_smoke = args.iter().any(|a| a == RUST_SMOKE);
    let wants_rust_preview = args.iter().any(|a| a == RUST_PREVIEW);
    let wants_macro_smoke = args.iter().any(|a| a == MACRO_SMOKE);
    let wants_macro_preview = args.iter().any(|a| a == MACRO_PREVIEW);
    let is_macro = wants_macro_smoke || wants_macro_preview;
    let is_rust = wants_rust_smoke || wants_rust_preview;
    let smoke = wants_smoke || wants_rust_smoke || wants_macro_smoke;
    if !(wants_status || wants_smoke || wants_preview || is_rust || is_macro) {
        return None;
    }
    let feature_enabled = enabled(std::env::var(FLAG).ok().as_deref());
    let path = script();
    let available = cfg!(windows) && path.is_some();
    let rust_path = rust_lab();
    let rust_available = cfg!(windows) && rust_path.is_some();
    let macro_path = macroquad_lab();
    let macro_available = cfg!(windows) && macro_path.is_some();

    if wants_status {
        println!(
            "{}",
            status_value(feature_enabled, available, rust_available, macro_available)
        );
        return Some(0);
    }

    if !feature_enabled {
        eprintln!("Native Box Lab is disabled; set {}=1 to opt in.", FLAG);
        return Some(3);
    }
    if !cfg!(windows)
        || (is_rust && !rust_available)
        || (is_macro && !macro_available)
        || (!is_rust && !is_macro && !available)
    {
        eprintln!("Native Box Lab is unavailable: the matching local debug lab executable/script was not found.");
        return Some(4);
    }

    // The only accepted script is the exact project lab file. It cannot be
    // pointed at live DesktopCraft files by flags from the caller.
    let mut command = if is_macro {
        Command::new(macro_path.unwrap())
    } else if is_rust {
        // Native sandbox built separately with cargo --features native-lab.
        // No Explorer injection or real Workspace path is passed.
        Command::new(rust_path.unwrap())
    } else {
        let mut cmd = Command::new("powershell.exe");
        cmd.arg("-STA")
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(path.unwrap())
            .arg("-PerMonitorDpi");
        cmd
    };
    if smoke {
        command.arg(if is_rust || is_macro {
            "--smoke"
        } else {
            "-SmokeTest"
        });
    }
    match command.spawn() {
        Ok(mut child) => {
            if smoke {
                // Never block indefinitely if the Shell automation fails to close.
                // Only the exact child PowerShell process we created can be killed.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(35);
                loop {
                    match child.try_wait() {
                        Ok(Some(result)) => return Some(result.code().unwrap_or(1)),
                        Ok(None) if std::time::Instant::now() < deadline => {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        Ok(None) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            eprintln!("Native Box Lab smoke test exceeded its safety deadline; its isolated process was closed.");
                            return Some(6);
                        }
                        Err(error) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            eprintln!("Native Box Lab smoke monitoring failed: {error}");
                            return Some(7);
                        }
                    }
                }
            }
            match child.wait() {
                Ok(result) => {
                    let code = result.code().unwrap_or(1);
                    if code != 0 {
                        eprintln!("Native Box Lab returned exit code {}.", code);
                    }
                    Some(code)
                }
                Err(error) => {
                    eprintln!("Native Box Lab process could not be joined: {error}");
                    Some(8)
                }
            }
        }
        Err(error) => {
            eprintln!("Native Box Lab did not start: {error}");
            Some(5)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opt_in_requires_exact_one() {
        assert!(enabled(Some("1")));
        for value in [None, Some("0"), Some("yes"), Some("true"), Some(" 1 ")] {
            assert!(!enabled(value));
        }
    }
    #[test]
    fn script_search_never_uses_working_directory() {
        let root = std::env::temp_dir().join(format!(
            "desktopcraft-native-bridge-test-{}",
            std::process::id()
        ));
        let project = root.join("project");
        std::fs::create_dir_all(project.join("labs")).unwrap();
        std::fs::create_dir_all(project.join("target").join("debug")).unwrap();
        let lab = project.join("labs").join(LAB_NAME);
        std::fs::write(&lab, "# disposable-test").unwrap();
        let exe = project
            .join("target")
            .join("debug")
            .join("desktopcraft.exe");
        assert_eq!(find_script_from(&exe), Some(lab));
        assert!(find_script_from(&root.join("separate").join("desktopcraft.exe")).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn diagnostic_is_explicit_about_unverified_production_features() {
        let status = status_value(false, true, true, true);
        assert_eq!(status["enabled"], false);
        assert_eq!(status["preview_available"], true);
        assert_eq!(status["production_box_replaced"], false);
        assert_eq!(status["native_windows_11_modern_menu_verified"], false);
        assert_eq!(status["workspace_modified"], false);
    }
}
