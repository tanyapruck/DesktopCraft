use crate::*;

/// A recovery report is evidence, not permission to reattach by itself.
/// Do not restart after unknown worker failures, partial rollback or timeouts.
#[cfg(windows)]
fn verified_native_rebind_report(report: &serde_json::Value, topology_changed: bool) -> bool {
    report["callback_detached"] == true
        && report["native_in_box_positions_restored"] == true
        && if topology_changed {
            report["native_monitor_extension_auto_recovered"] == true
        } else {
            report["native_policy_lease_rebind_required"] == true
        }
}

#[cfg(all(windows, test))]
#[test]
fn rebind_requires_a_verified_recovery_for_the_actual_trigger() {
    let mut report = serde_json::json!({
        "callback_detached":true, "native_in_box_positions_restored":true,
        "native_monitor_extension_auto_recovered":true,
        "native_policy_lease_rebind_required":false
    });
    assert!(verified_native_rebind_report(&report, true));
    assert!(!verified_native_rebind_report(&report, false));
    report["native_policy_lease_rebind_required"] = true.into();
    assert!(verified_native_rebind_report(&report, false));
    report["callback_detached"] = false.into();
    assert!(!verified_native_rebind_report(&report, true));
    report["callback_detached"] = true.into();
    report["native_in_box_positions_restored"] = false.into();
    assert!(!verified_native_rebind_report(&report, false));
    assert!(!verified_native_rebind_report(&serde_json::json!({}), true));
}

pub(super) async fn run() {
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
    let shot = option("--screenshot")
        .or_else(|| text_check.clone())
        .or_else(|| settings_capture.as_ref().map(|(_, path)| path.clone()));
    let test = option("--self-test");
    let perf_path = option("--perf-report");
    if perf_path.is_some() {
        perf::enable();
    }
    let automation = shot.is_some() || test.is_some();
    let integrated_preview = args.iter().any(|a| a == "--native-integrated-preview");
    let native_main_hwnd_lab = args.iter().any(|a|a=="--native-main-hwnd-lab");
    // A bounded runtime test: actual Miniquad main HWND + OLE registration,
    // but no Explorer icon input, synthetic mouse, or user Workspace.
    let isolated_ole_smoke_report=if native_main_hwnd_lab {
        option("--native-main-ole-smoke-report")
    }else{None};
    let isolated_ole_smoke_drop=native_main_hwnd_lab
        && isolated_ole_smoke_report.is_some()
        && args.iter().any(|a|a=="--native-main-ole-smoke-drop");
    let isolated_ole_smoke_multi=isolated_ole_smoke_drop &&
        args.iter().any(|a|a=="--native-main-ole-smoke-drop-multi");
    if isolated_ole_smoke_drop && !cfg!(debug_assertions) {
        eprintln!("Synthetic OLE smoke is available only in a Debug native-lab build");
        return;
    }
    if native_main_hwnd_lab && (
        host_mode() ||
        !cfg!(all(windows,feature="native-lab")) ||
        !args.iter().any(|a|a=="--unsafe-native-test") ||
        !args.iter().any(|a|a=="--isolated-workspace-only")
    ){
        eprintln!("Main HWND OLE requires native-lab feature and explicit --unsafe-native-test --isolated-workspace-only; no real Desktop is ever used");
        return;
    }
    // Opt-in only: real Explorer icons with DesktopCraft Tab title chrome,
    // never the legacy replica-icon renderer and global icon hiding together.
    let native_desktop_mode=native_desktop_launch_requested(&args);
    // The native desktop path is the user's REAL application surface, not a
    // synthetic Debug/lab overlay or disposable UI. It preserves a durable
    // real-icon recovery journal and leaves original files Explorer-owned.
    let native_chrome_preview=native_desktop_mode ||
        (args.iter().any(|a|a=="--native-explorer-chrome")
        && args.iter().any(|a|a=="--unsafe-native-test"));
    let native_general_workspace=native_desktop_mode ||
        args.iter().any(|a|a=="--native-general-workspace-lab");
    let native_two_box_ui=native_general_workspace
        || args.iter().any(|a|a=="--native-two-box-ui-lab");
    let native_in_box_layout=native_general_workspace
        || args.iter().any(|a|a=="--native-in-box-layout-lab");
    if native_in_box_layout && !native_two_box_ui {
        eprintln!("Native in-Box layout requires the explicitly gated two-Box Debug UI");
        std::process::exit(2);
    }
    let native_second_monitor_lab=args.iter()
        .any(|a|a=="--native-two-box-secondary-monitor-lab");
    if native_second_monitor_lab && !native_two_box_ui {
        eprintln!("Secondary-monitor native UI requires the isolated two-Box Debug lab");
        std::process::exit(2);
    }
    if native_two_box_ui && (!native_chrome_preview
        || (!native_desktop_mode && (!cfg!(all(debug_assertions,feature="native-lab"))
            || !args.iter().any(|a|a=="--acknowledge-explorer-risk")
            || !args.iter().any(|a|a=="--acknowledge-bulk-explorer-hide")))
        || args.iter().any(|a|a=="--native-chrome-ole-lab")
        || args.iter().any(|a|a=="--native-chrome-real-pin-transfer")){
        eprintln!("Native Desktop activation failed preflight; normal shell icon state was not modified");
        std::process::exit(2);
    }
    if native_desktop_mode && !cfg!(all(windows,feature="native-release")) {
        eprintln!("Real Windows desktop needs an embedded native Explorer DLL");
        std::process::exit(2);
    }
    if native_desktop_mode && native_desktop_has_disallowed_lab_args(&args) {
        host::event(
            "WARN",
            "native_desktop.lab_args_refused",
            "Real Native Desktop cannot be combined with diagnostic Lab commands",
        );
        eprintln!("Real Native Desktop cannot be combined with diagnostic Lab commands");
        std::process::exit(2);
    }
    // A one-shot original-UI integration lab on a fresh isolated COPY of
    // the user's real Box layout. Even with real Explorer icons present, it
    // never writes the real Workspace and does NOT claim genuine Explorer OLE.
    let native_chrome_ole_lab=args.iter().any(|a|a=="--native-chrome-ole-lab");
    let native_chrome_ole_report=if native_chrome_ole_lab{
        option("--native-chrome-ole-report")
    }else{None};
    let native_chrome_ole_smoke=native_chrome_ole_lab &&
        args.iter().any(|a|a=="--native-chrome-ole-smoke");
    let native_chrome_gesture_lab=native_chrome_ole_lab &&
        args.iter().any(|a|a=="--native-chrome-gesture-lab");
    let native_chrome_ole_repeat=native_chrome_ole_smoke &&
        args.iter().any(|a|a=="--native-chrome-ole-repeat-smoke");
    let native_chrome_live_transfer=native_chrome_gesture_lab &&
        args.iter().any(|a|a=="--native-chrome-real-pin-transfer");
    #[cfg(all(windows,feature="native-lab",feature="native-unsafe-drag-lab"))]
    let live_transfer_paths=if native_chrome_live_transfer {
        let checkpoint=option("--native-live-checkpoint");
        let expected=option("--native-live-expected-post");
        match (checkpoint,expected){
            (Some(checkpoint),Some(expected))=>
                Some((checkpoint,expected)),
            _=>{
                eprintln!("Real Pin transfer requires checkpoint and NEW expected-post paths");
                std::process::exit(2);
            }
        }
    }else{None};

    if native_chrome_ole_lab && (!native_chrome_preview ||
        !cfg!(all(windows,feature="native-lab",debug_assertions)) ||
        (!args.iter().any(|a|a=="--isolated-workspace-only") &&
            !native_chrome_live_transfer) ||
        (native_chrome_live_transfer &&
            (args.iter().any(|a|a=="--isolated-workspace-only") ||
             !args.iter().any(|a|a=="--confirm-real-user-pin-transfer") ||
             !cfg!(feature="native-unsafe-drag-lab"))) ||
        native_chrome_ole_report.is_none() ||
        native_chrome_ole_smoke==native_chrome_gesture_lab ||
        (native_chrome_gesture_lab &&
            !cfg!(feature="native-unsafe-drag-lab"))) {
        eprintln!("Native Chrome lab requires Debug native-lab, --native-explorer-chrome --unsafe-native-test --isolated-workspace-only, one of --native-chrome-ole-smoke or --native-chrome-gesture-lab, --native-chrome-ole-report PATH; genuine gesture requires --features native-unsafe-drag-lab");
        std::process::exit(2);
    }
    // Development-only generic regular-tab target. Defaults preserve the
    // already verified Craft / Work pair. Other groups must pass every
    // native identity and bounded item-count check before a hook can attach.
    let native_target_group=args.iter().position(|a|a=="--native-chrome-group")
        .map(|i|args.get(i+1).and_then(|s|s.parse::<u64>().ok())
            .unwrap_or_else(||{
                eprintln!("Invalid --native-chrome-group; explicit group ID is required");
                std::process::exit(2);
            })).unwrap_or(144);
    let native_target_tab=args.iter().position(|a|a=="--native-chrome-to-tab")
        .map(|i|args.get(i+1).and_then(|s|s.parse::<usize>().ok())
            .unwrap_or_else(||{
                eprintln!("Invalid --native-chrome-to-tab; explicit zero-based tab index is required");
                std::process::exit(2);
            })).unwrap_or(2);
    let native_manual_index=args.iter().position(|a|a=="--native-chrome-ui-manual");
    let native_manual_seconds=native_manual_index.map(|i|args.get(i+1)
        .and_then(|s|s.parse::<u64>().ok()).unwrap_or(45).clamp(10,120));
    let native_chrome_started=std::time::Instant::now();
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
    let desktop_mode = host_mode() && !automation && !integrated_preview && !native_main_hwnd_lab;
    // Desktop mode uses the existing Desktop Spaces workspace if found, or
    // %APPDATA%\DesktopCraft for new users. Window mode uses sample data.
    let root = if native_main_hwnd_lab {
        std::env::temp_dir().join(format!(
            "DesktopCraftIsolatedMainOle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0,|t|t.as_nanos())
        ))
    } else if integrated_preview {
        std::env::temp_dir().join(format!(
            "DesktopCraftNativeIntegrated-{}",
            std::process::id()
        ))
    } else if native_desktop_mode || (native_chrome_preview && native_chrome_live_transfer) {
        // REAL DesktopCraft UI mode owns the active, persisted Workspace;
        // developer-only OLE transfer is separately gated above.
        // A recovery journal and a temporary worker shadow are used only
        // for native icon transaction safety, not for a fake desktop window.
        host::data_dir()
    } else if native_chrome_preview {
        // An isolated *copy* of the existing persisted desktop layout.
        // Clicks in this real Explorer-owned UI preview cannot overwrite the
        // user's actual Workspace, even on abnormal program termination.
        let copy_root=if native_chrome_ole_lab {
            std::env::temp_dir().join(format!(
                "DesktopCraftExplorerChromeOLE-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0,|t|t.as_nanos())))
        }else{
            std::env::temp_dir().join(format!(
                "DesktopCraftExplorerChrome-{}",std::process::id()))
        };
        let out=copy_root.join("data/workspace.json");
        if std::fs::create_dir_all(out.parent().unwrap()).is_err()
            || std::fs::copy(host::data_dir().join("data/workspace.json"),&out).is_err() {
            eprintln!("Cannot make isolated DesktopCraft Workspace snapshot; native preview refused");
            std::process::exit(2);
        }
        copy_root
    } else if desktop_mode {
        host::data_dir()
    } else {
        app_root()
    };
    #[cfg(all(windows,feature="native-lab"))]
    let _isolated_main_ole_cleanup=if native_main_hwnd_lab ||
        (native_chrome_ole_lab && !native_chrome_live_transfer) {
        Some(IsolatedNativeOleTempRoot(root.clone()))
    } else {None};
    if !desktop_mode {
        if let Err(e) = create_demo(&root) {
            eprintln!("Could not prepare sample files: {e}");
            std::process::exit(1)
        }
    }
    // COM/OLE for Explorer's icons (all modes) and the shell (desktop mode).
    #[cfg(windows)]
    if native_desktop_mode && !enable_native_audit_physical_dpi(){
        host::event("WARN","native_desktop.invalid_dpi_awareness",
            "Physical DPI-aware thread context required before real Explorer icon placement");
        return;
    }
    shell::init();
    let startup_desktop_enabled = !native_desktop_mode
        || automation
        || State::load(&root.join("data/workspace.json"))
            .map_or(true, |st| st.prefs.desktop_enabled);
    #[cfg(windows)]
    if native_desktop_mode && !automation && startup_desktop_enabled {
        if let Some(view) = shell::DesktopView::acquire() {
            if view.icons_visible() == Some(false) {
                if view.set_icons_visible(true) {
                    host::event(
                        "INFO",
                        "native_desktop.restored_hidden_icons",
                        "Restored Explorer desktop icon layer visibility on Native Desktop startup",
                    );
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
            }
        }
        if let Err(reason) = prepare_native_crash_recovery() {
            host::event("WARN", "native_desktop.crash_recovery_blocked", &reason);
            host::message(&format!("DesktopCraft could not complete Explorer crash recovery:\n\n{reason}"), false);
            return;
        }
    }
    let mut app = App::new(root.clone(), automation, desktop_mode);
    let choice_file = root.join("data/onboarding-choice.txt");
    // An old setup choice is not permission to replace an existing workspace.
    let is_first_run = app.host.as_ref().is_some_and(|h| h.first_run)
        && host::workspace_allows_onboarding(&root);
    if !automation && desktop_mode && !is_first_run
        && root.join("data/workspace.json").is_file()
    {
        let _ = std::fs::remove_file(&choice_file);
    }
    if desktop_mode
        && !automation
        && !integrated_preview
        && settings_capture.is_none()
        && is_first_run
    {
        app.prepare_native_monitor_layout();
        let onboard = if let Ok(raw) = std::fs::read_to_string(&choice_file) {
            let _ = std::fs::remove_file(&choice_file);
            !raw.trim().eq_ignore_ascii_case("custom")
        } else {
            host::message(
                "How would you like to set up your desktop boxes?\n\n• Click Yes for Recommended Onboarding layout:\n  - Center Box: General desktop icons + your Windows Downloads folder (List view)\n  - Right Drawer: Documents (Word, Excel, PDF) + your Windows Pictures folder\n\n• Click No to start with Custom boxes.",
                true,
            )
        };
        if onboard {
            app.apply_onboarding_layout(
                shell::downloads_folder(),
                shell::pictures_folder(),
                shell::desktop_entries(),
            );
        } else {
            app.apply_custom_starter_layout();
        }
        if let Some(h) = app.host.as_mut() {
            h.first_run = false;
        }
        app.remember_layout();
        let _ = app.state.save(&app.save_path);
    }
    if native_desktop_mode && startup_desktop_enabled {
        // The Explorer worker snapshots workspace.json and places icons from
        // it. Apply this monitor arrangement's layout and save it FIRST, so
        // the worker, the preflight and the Box chrome share the same bounds.
        let bounds_before=app.state.groups.iter().map(|g|(g.id,g.bounds)).collect::<Vec<_>>();
        app.prepare_native_monitor_layout();
        let bounds_after=app.state.groups.iter().map(|g|(g.id,g.bounds)).collect::<Vec<_>>();
        if bounds_after!=bounds_before {
            app.remember_layout();
            match app.state.save(&app.save_path) {
                Ok(())=>host::event("INFO","native_desktop.monitor_layout_saved",
                    "Boxes moved to this monitor arrangement's layout before Explorer placement"),
                Err(e)=>host::event("WARN","native_desktop.monitor_layout_save_failed",&e.to_string()),
            }
        }
        // Do not install ANY Explorer callback or move an icon until the
        // current real user profile, every active Box and every physical
        // native cell have passed this read-only audit.
        let result=shell::DesktopView::acquire()
            .ok_or_else(||"Actual Windows Explorer Desktop unavailable".to_string())
            .and_then(|view|native_box_live_layout::native_desktop_preflight(
                &app.state,&view));
        match result {
            Ok(report)=>host::event("INFO","native_desktop.preflight_ready",
                &format!("Native Windows Desktop placement ready: {report}")),
            Err(reason)=>{
                host::event("WARN","native_desktop.preflight_blocked",&reason);
                if preflight_should_exit_silently(&reason) {
                    // A second launch must NEVER create a classic overlay on
                    // top of an existing Native Shell icon writer. The
                    // original running instance owns its icon transaction.
                    host::event("WARN","native_desktop.duplicate_refused",
                        "Existing Explorer native session owns the Desktop; no fallback UI started");
                    return;
                }
                host::message(&format!(
                    "Windows Explorer icon mode could not start safely.\n\n{reason}\n\nDesktopCraft did not start desktop management. Resolve the Explorer issue and try again."),false);
                return;
            }
        }
    }
    #[cfg(all(windows,feature="native-lab"))]
    let native_main_hwnd_sample=if native_main_hwnd_lab {
        match prepare_isolated_native_main_hwnd_ole_fixture(&mut app,isolated_ole_smoke_multi) {
            Ok(fixture)=>Some(fixture),
            Err(e)=>{
                eprintln!("Isolated DesktopCraft main OLE fixture refused: {e}");
                return;
            }
        }
    } else {None};
    #[cfg(all(windows,feature="native-lab"))]
    let native_chrome_main_sample=if native_chrome_ole_lab {
        let attempt=if native_chrome_live_transfer{
            #[cfg(all(windows,feature="native-lab",feature="native-unsafe-drag-lab"))]
            {
                let (checkpoint,post)=live_transfer_paths.as_ref()
                    .expect("Real transfer flags were validated before GUI");
                prepare_original_live_native_chrome(
                    &app,native_target_group,checkpoint,post)
            }
            #[cfg(not(all(windows,feature="native-lab",feature="native-unsafe-drag-lab")))]
            {Err("Real native transfer requires Debug dual feature".into())}
        }else{
            prepare_native_chrome_ole_copy(&mut app,native_target_group)
        };
        match attempt {
            Ok(sample)=>{
                // Paint the additional disposable receiver's original-theme
                // header alongside the verified source Box in this lab only.
                std::env::set_var("DESKTOPCRAFT_NATIVE_CHROME_OLE_DEST_ID",
                    sample.1[1].to_string());
                if native_chrome_gesture_lab{None}else{Some(sample)}
            },
            Err(e)=>{
                eprintln!("Native Chrome temp-copy original UI OLE integration refused: {e}");
                return;
            }
        }
    }else{None};
    let native_from_tab=app.state.group(native_target_group)
        .map(|g|g.active).unwrap_or(usize::MAX);
    #[cfg_attr(not(windows), allow(unused_mut, unused_variables))]
    let mut native_save_permitted = false;
    if native_chrome_preview {
        if !desktop_mode {
            eprintln!("Native Explorer chrome requires an active DesktopCraft desktop host");
            std::process::exit(2);
        }
        let enforce_live_native = !native_desktop_mode || startup_desktop_enabled;
        if enforce_live_native
            && shell::DesktopView::acquire().and_then(|v| v.icons_visible()) != Some(true)
        {
            eprintln!("Native Explorer chrome requires existing visible Explorer desktop icons");
            std::process::exit(2);
        }
        native_save_permitted = native_desktop_mode
            && app.save_allowed
            && native_acceptance_test_token(&args).is_none()
            && option("--native-two-box-ui-report").is_none();
        // Keep generic auto-save off so Explorer-visible geometry, Tab switches,
        // and Pin changes only persist after worker/Explorer ACK.
        app.save_allowed = false;
        if !native_chrome_live_transfer {
            app.state.explorer_icons_hidden = false;
        }
        if native_chrome_ole_lab && !native_chrome_live_transfer {
            // Keep the isolated copied Workspace byte-equivalent to the
            // on-screen State *before* OLE DragEnter snapshot verification.
            if let Err(e) = app.state.save(&app.save_path) {
                eprintln!("Could not save disposable native OLE Chrome snapshot: {e}");
                return;
            }
        }
        if enforce_live_native {
            let eligible = if native_general_workspace {
                native_general_workspace_eligible(&app.state)
            } else if native_two_box_ui {
                app.state.group(144).is_some_and(|source| {
                    source.page == app.state.page
                        && !source.rolled
                        && source.tabs.len() >= 3
                        && source.tabs.iter().all(|tab| tab.folder.is_none())
                }) && app.state.group(168).is_some_and(|target| {
                    target.page == app.state.page
                        && !target.rolled
                        && target.tabs.len() >= 2
                        && target.tabs.iter().all(|tab| tab.folder.is_none())
                })
            } else {
                app.state.group(native_target_group).is_some_and(|g| {
                    g.page == app.state.page
                        && !g.rolled
                        && native_from_tab < g.tabs.len()
                        && native_target_tab < g.tabs.len()
                        && native_from_tab != native_target_tab
                        && [native_from_tab, native_target_tab].iter().all(|&index| {
                            let tab = &g.tabs[index];
                            tab.folder.is_none() && !tab.items.is_empty() && tab.items.len() <= 3
                        })
                        && {
                            let (shown, _) = g.header_tabs();
                            shown.iter().any(|(i, _)| *i == native_from_tab)
                                && shown.iter().any(|(i, _)| *i == native_target_tab)
                        }
                })
            };
            if !eligible {
                eprintln!("Native Explorer preview refused: group / Tab selection is not an active, visible, bounded pair of regular Desktop Tabs");
                std::process::exit(2);
            }
        }
        // App.frame reads the opted-in target to paint exactly this group's
        // two Header Tab controls; only the Worker filters actual icons.
        std::env::set_var("DESKTOPCRAFT_NATIVE_CHROME_TARGET_GROUP", native_target_group.to_string());
        std::env::set_var("DESKTOPCRAFT_NATIVE_CHROME_FROM_TAB", native_from_tab.to_string());
        std::env::set_var("DESKTOPCRAFT_NATIVE_CHROME_TO_TAB", native_target_tab.to_string());
        if native_two_box_ui {
            std::env::set_var("DESKTOPCRAFT_NATIVE_TWO_BOX_UI", "1");
            if native_in_box_layout {
                std::env::set_var("DESKTOPCRAFT_NATIVE_INBOX_LAYOUT", "1");
            }
            if native_general_workspace {
                std::env::set_var("DESKTOPCRAFT_NATIVE_GENERAL_WORKSPACE", "1");
            }
            if native_desktop_mode {
                std::env::set_var("DESKTOPCRAFT_NATIVE_REAL_DESKTOP", "1");
            }
        }
    }
    app.native_env = NativeEnvFlags::read();
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
    if desktop_mode {
        app.host_start_native_chrome();
        host::event("INFO","native_chrome.safe_start",
            "Explorer icons preserved; monitor geometry initialized without global hide or tuck");
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
    // Never register this receiver in normal DesktopCraft or on the user's
    // actual Explorer Desktop. Even the experimental main-HWND path requires
    // explicit flags plus a freshly created disposable sample Workspace.
    #[cfg(all(windows,feature="native-lab"))]
    let (_native_main_ole_receiver,native_main_ole_context)=
        if let Some((icons,visible))=native_main_hwnd_sample.or(native_chrome_main_sample) {
            let hwnd=host::window();
            // Match App.frame's scaled Box coordinate space, not the raw
            // Macroquad screen dimensions (which differ after window resize).
            let ui_scale=ui_scale_for_frame(desktop_mode,screen_width(),screen_height());
            let logical=(screen_width()/ui_scale,screen_height()/ui_scale);
            let viewport=match native_drop::measured_main_hwnd_viewport(
                hwnd,logical.0 as f64,logical.1 as f64) {
                Ok(v)=>v,
                Err(e)=>{eprintln!("Main HWND OLE refused: {e}");return;}
            };
            let context=std::sync::Arc::new(std::sync::Mutex::new(
                native_drop::NativeOlePersistedLabContext::new(
                    app.save_path.clone(),app.state.clone(),icons,
                    if native_chrome_ole_lab{host::signature(&host::monitors())}
                    else{"isolated-main-window-lab".to_string()},
                    viewport,visible,if native_chrome_ole_lab{16}else{4},
                    if native_chrome_ole_lab{128}else{8}
                )
            ));
            let receiver=match native_drop::NativeOlePersistedReceiver
                ::register_isolated_main_hwnd(
                    hwnd,logical.0 as f64,logical.1 as f64,
                    context.clone()) {
                Ok(registered)=>registered,
                Err(e)=>{eprintln!("Could not enable isolated OLE main HWND: {e}");return;}
            };
            if isolated_ole_smoke_report.is_some() ||
                native_chrome_ole_report.is_some() {
                // Hide only the freshly created laboratory application HWND.
                // This is not Explorer/Progman, not a shell window or a user
                // program. The event loop still renders for a bounded smoke.
                #[link(name="user32")]
                extern "system" {fn ShowWindow(hwnd:*mut std::ffi::c_void,cmd:i32)->i32;}
                unsafe{ShowWindow(hwnd.cast(),0);}
            }
            (Some(receiver),Some(context))
        } else {(None,None)};
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
    let updater = if desktop_mode && !automation && !native_chrome_preview {
        updates::Client::start()
    }else{None};
    #[cfg(windows)]
    if native_desktop_mode && !automation {
        start_native_crash_watch();
    }
    #[cfg(windows)]
    let mut native_two_box_worker =
        if native_two_box_ui && (!native_desktop_mode || app.state.prefs.desktop_enabled) {
            let dll = match resolve_native_filter_dll() {
                Ok(path) => path,
                Err(error) => {
                    host::event("WARN", "native_bundle.extract_failed", &error);
                    app.host_exit();
                    return;
                }
            };
            Some(native_filter_host::NativeTwoBoxBackgroundService::start(
                root.join("data/workspace.json"),
                dll,
                host::window() as usize,
            ))
        } else {
            None
        };
    #[cfg(windows)]
    if let Some(ref mut worker)=native_two_box_worker {
        if !worker.await_ready(std::time::Duration::from_secs(6)){
            let report=worker.shutdown();
            host::event("WARN","native_two_box.startup_not_ready",
                &format!("Explorer identity/policy refused native UI before any file operation: {report}"));
            if let Some(path)=option("--native-two-box-ui-report"){
                let _=std::fs::write(path,
                    serde_json::to_vec_pretty(&report).unwrap_or_default());
            }
            app.host_exit();
            return;
        }
        host::event("INFO","native_two_box.startup_ready",
            "Both Native Boxes verified in Explorer before accepting UI Tab clicks");
    }
    #[cfg(all(windows, feature = "native-release"))]
    let mut native_window_drops = if native_desktop_mode && app.state.prefs.desktop_enabled {
        match native_drop::NativeWindowDropReceiver::register(host::window()) {
            Ok(receiver) => Some(receiver),
            Err(reason) => {
                host::event("WARN", "native_desktop.tab_drop_receiver_unavailable", &reason);
                None
            }
        }
    } else { None };
    #[cfg(windows)]
    let mut native_header_reply: Option<std::sync::mpsc::Receiver<Result<(), String>>> = None;
    #[cfg(windows)]
    let mut native_tab_worker=if native_chrome_preview && !native_chrome_gesture_lab
        && !native_two_box_ui {
        let dll=std::env::current_exe().ok()
            .and_then(|exe|exe.ancestors().nth(3)
                .map(|root|root.join("native-filter-build/debug/desktopcraft_native_filter.dll")))
            .unwrap_or_default();
        Some(native_filter_host::NativeTabBackgroundService::start(
            root.join("data/workspace.json"),dll,native_target_group,native_target_tab))
    }else{None};
    #[cfg(windows)]
    if let Some(ref mut worker)=native_tab_worker {
        if !worker.await_ready(std::time::Duration::from_secs(6)) {
            host::event("WARN","native_chrome.startup_not_ready",
                "Explorer native filter did not attach before timeout; native UI preview refused and hook detached");
            let report=worker.shutdown();
            if let Some(i)=args.iter().position(|a|a=="--native-chrome-report") {
                if let Some(path)=args.get(i+1) {
                    let _=std::fs::write(path,
                        serde_json::to_vec_pretty(&report).unwrap_or_default());
                }
            }
            app.host_exit();
            return;
        }
        host::event("INFO","native_chrome.startup_ready",
            "Explorer native filter attached and validated before accepting UI tab clicks");
    }
    #[cfg(all(windows,feature="native-unsafe-drag-lab"))]
    let native_gesture_dll=if native_chrome_gesture_lab {
        let relative=option("--native-chrome-gesture-ready-file")
            .ok_or("Genuine original-UI native gesture requires a new relative ready-file path");
        let ready=relative.and_then(|name|{
            let normalized=name.to_string_lossy().replace('\\',"/");
            let path=std::path::Path::new(&normalized);
            if !normalized.starts_with("checks/explorer-owned/native-chrome-gesture-")
                || path.is_absolute() ||
                path.components().any(|component|
                    !matches!(component,std::path::Component::Normal(_))) ||
                path.exists() {
                return Err("Refusing unsafe or pre-existing native gesture ready-file path");
            }
            Ok(normalized)
        });
        let ready=match ready{
            Ok(ready)=>ready,
            Err(reason)=>{eprintln!("{reason}");return;}
        };
        std::env::set_var("DESKTOPCRAFT_NATIVE_DRAG_TEST","1");
        std::env::set_var("DESKTOPCRAFT_NATIVE_DRAG_MULTI_TEST","1");
        std::env::set_var("DESKTOPCRAFT_NATIVE_ORIGINAL_UI_GESTURE","1");
        std::env::set_var("DESKTOPCRAFT_NATIVE_DRAG_READY_FILE",&ready);
        let dll=std::env::current_exe().ok()
            .and_then(|exe|exe.ancestors().nth(3)
                .map(|base|base.join(
                    "native-filter-build/debug/desktopcraft_native_filter.dll")))
            .unwrap_or_default();
        // Do NOT install the hook yet. The original HWND begins as a
        // fullscreen rectangle and becomes Explorer-transparent only after
        // the FIRST real frame calls host_after_frame(SetWindowRgn). Starting
        // now races and can overlay the native icon at first MouseDown.
        Some(dll)
    }else{None};
    #[cfg(all(windows,feature="native-unsafe-drag-lab"))]
    let mut native_gesture_worker:
        Option<native_filter_host::NativeGestureBackgroundService>=None;
    #[cfg(all(windows,feature="native-unsafe-drag-lab"))]
    let mut native_original_gesture_outcome:Option<serde_json::Value>=None;
    #[cfg(all(windows,feature="native-unsafe-drag-lab"))]
    let mut native_original_gesture_worker_report:Option<serde_json::Value>=None;
    let mut native_display_baseline = if native_chrome_preview {
        Some(host::signature(&host::monitors()))
    } else {
        None
    };
    let mut native_display_checked_at=std::time::Instant::now();
    #[cfg(windows)]
    let mut last_native_rebind_attempt: Option<std::time::Instant> = None;
    // Real Native Desktop changes are acknowledged by the Explorer layout
    // worker as one transaction. This is the last accepted in-memory State
    // to restore if the user's Header/Page/Rollup edit is refused.
    // What workspace.json holds. Native Desktop keeps Tab switches and Box
    // moves in memory only, so a Pin drop is checked against the file's
    // State and patches just that Pin (not the unsaved in-memory edits).
    #[cfg(windows)]
    let mut native_disk_state=if native_desktop_mode {
        State::load(&app.save_path).ok()
    }else{None};
    #[cfg(windows)]
    let mut native_last_accepted_state =
        if native_desktop_mode && app.state.prefs.desktop_enabled {
            Some(app.state.clone())
        } else {
            None
        };
    if desktop_mode && !automation && !app.state.prefs.desktop_enabled && app.dialog.is_none() {
        app.dialog = Some(Dialog::Settings {
            page: 4,
            pattern: String::new(),
            mode: 0,
            target: 0,
        });
        app.settings_bring_to_front = true;
    }
    #[cfg(windows)]
    let mut native_last_accepted_revealed: Option<u64> = None;
    #[cfg(windows)]
    let mut native_last_accepted_hidden: bool = false;
    #[cfg(windows)]
    let mut native_pending_ui:Option<(
        native_filter_host::PendingNativeWorkspaceSync,
        Option<(State, Option<u64>, bool)>, // (provisional real Workspace, revealed id, hidden) awaiting worker ACK
    )>=None;
    #[cfg(windows)]
    let mut native_tab_trace: Option<(u64, std::time::Instant)> = None;
    #[cfg(windows)]
    let mut native_pending_drop_commit:Option<
        native_filter_host::NativeVerifiedDrop>=None;
    #[cfg(windows)]
    let mut native_rules_sort_reply: Option<(native_filter_host::PendingNativeWorkspaceSync, desktop::RuleSortScope)> = None;
    #[cfg(windows)]
    let mut native_last_sort_scope = desktop::RuleSortScope::LooseOnly;
    #[cfg(windows)]
    let mut native_layout_restore_reply: Option<(native_filter_host::PendingNativeWorkspaceSync, desktop::PendingNativeLayoutRestore)> = None;
    #[cfg(windows)]
    let mut native_pending_layout_followup: Option<desktop::PendingNativeLayoutRestore> = None;
    #[cfg(windows)]
    let mut native_stage2_layout_followup: Option<desktop::PendingNativeLayoutRestore> = None;
    #[cfg(windows)]
    let mut native_pin_paths_reply: Option<(native_filter_host::PendingNativeWorkspaceSync, String)> = None;
    #[cfg(windows)]
    let mut native_pending_pin_paths_toast: Option<String> = None;
    #[cfg(windows)]
    let mut native_pending_sort_followup = false;
    #[cfg(windows)]
    let native_acceptance_deadline=if native_desktop_mode &&
        native_acceptance_test_token(&args).is_some() {
        Some(std::time::Instant::now()+std::time::Duration::from_secs(
            native_acceptance_test_seconds(&args)))
    }else{None};
    let mut frame = 0u32;
    #[cfg(all(windows,feature="native-lab"))]
    let mut isolated_ole_smoke_drop_error:Option<String>=None;
    #[cfg(all(windows,feature="native-lab"))]
    let mut native_chrome_ole_worker_shutdown:Option<serde_json::Value>=None;
    #[cfg(all(windows,feature="native-lab"))]
    let mut native_chrome_ole_rehook_result:Option<serde_json::Value>=None;
    #[cfg(all(windows,feature="native-lab"))]
    let mut native_chrome_ole_rehook_history:Vec<serde_json::Value>=Vec::new();
    #[cfg(all(windows,feature="native-lab"))]
    let mut native_chrome_handled_drops=0u32;
    // Check whether persisted data changed on a modest cadence rather than
    // serializing the full workspace twice on every rendered frame.
    let mut last_saved_state = if automation {
        Vec::new()
    } else {
        serde_json::to_vec(&app.state).unwrap_or_default()
    };
    let mut last_seen_state = last_saved_state.clone();
    let mut next_state_check_at = 0.0f64;
    let mut next_idle_state_check_at = 4.0f64;
    let mut last_edit_at = 0.0f64;
    let mut previous_frame_start: Option<std::time::Instant> = None;
    // The transparent desktop host does not reliably receive VSync pacing.
    // Limit only the native UI loop; background Explorer workers retain
    // their existing timing and the windowed/capture tests are unchanged.
    let mut pacing_cursor = host::cursor();
    let mut pacing_active_until = std::time::Instant::now();
    loop {
        let perf_frame_start = std::time::Instant::now();
        #[cfg(windows)]
        if native_acceptance_deadline.is_some_and(|deadline|perf_frame_start>=deadline){
            host::event("WARN","native_acceptance.self_deadline",
                "Test-owned native session reached its bounded acceptance deadline; closing UI and recovering original icons");
            if let Some(mut worker)=native_two_box_worker.take(){
                let result=worker.shutdown();
                host::event("INFO","native_acceptance.shutdown",
                    &format!("Native test worker shutdown report: {result}"));
            }
            app.host_exit();
            break;
        }
        // Handle an idle native session's display change or expired lease
        // BEFORE collecting input or accepting any new Explorer commands.
        // On an unsafe/ambiguous transition keep the existing fail-open exit.
        #[cfg(windows)]
        if native_desktop_mode && !automation && native_save_permitted
            && native_acceptance_deadline.is_none()
            && !(cfg!(debug_assertions)
                && std::env::var_os("DESKTOPCRAFT_NATIVE_TEST_MAIN_MONITOR_CHANGED")
                    .is_some_and(|v| v == "1"))
            && app.state.prefs.desktop_enabled
            && !app.host.as_ref().is_some_and(|h| h.quit)
            && !is_quit_requested() {
            let worker_stopped = native_two_box_worker.as_ref().is_some_and(|w| !w.is_ready());
            let due = native_display_checked_at.elapsed() >= std::time::Duration::from_secs(1);
            let observed = if worker_stopped || due {
                Some(host::signature(&host::monitors()))
            } else { None };
            if due { native_display_checked_at = std::time::Instant::now(); }
            let topology_changed = observed.as_ref().is_some_and(|now|
                native_display_baseline.as_ref().is_some_and(|old| now != old));
            if topology_changed || worker_stopped {
                let idle = app.drag.is_none() && app.dialog.is_none()
                    && app.menu.is_none() && app.folder_sort_menu.is_none()
                    && app.minimized_settings.is_none() && !app.peek
                    && !app.native_tx_inflight && app.native_ghost.is_none()
                    && native_pending_ui.is_none() && native_header_reply.is_none()
                    && native_rules_sort_reply.is_none() && native_layout_restore_reply.is_none()
                    && native_pending_layout_followup.is_none() && native_stage2_layout_followup.is_none()
                    && native_pin_paths_reply.is_none() && native_pending_pin_paths_toast.is_none()
                    && !native_pending_sort_followup && native_pending_drop_commit.is_none()
                    && app.native_rules_sort_requested.is_none() && app.native_layout_restore_requested.is_none()
                    && app.native_pin_paths_requested.is_none() && app.native_ai_pending.is_none()
                    && app.native_undo_inflight.is_none() && app.native_record_undo_on_sync.is_none()
                    && app.desktop_toggle_requested.is_none()
                    && native_last_accepted_state.as_ref().is_some_and(|accepted|
                        !native_effective_sync_needed(accepted, native_last_accepted_revealed,
                            native_last_accepted_hidden, &app.state, app.revealed, app.hidden));
                let cooled_down = last_native_rebind_attempt.is_none_or(|at|
                    at.elapsed() >= std::time::Duration::from_secs(10));
                if idle && cooled_down && observed.as_ref().is_some_and(|s| !s.is_empty())
                    && native_two_box_worker.is_some() {
                    last_native_rebind_attempt = Some(std::time::Instant::now());
                    host::set_native_quick_hide(false);
                    let _ = host::take_native_quick_hide();
                    #[cfg(feature = "native-release")]
                    { native_window_drops = None; }
                    let region_cleared = host::set_region(&[]);
                    if let Some(h) = app.host.as_mut() {
                        h.region.clear();
                        if h.front { host::set_front(false); h.front = false; }
                    }
                    app.sync_native_backdrop(false, None, false);
                    let report = native_two_box_worker.take().unwrap().shutdown();
                    host::event("INFO", "native_desktop.rebind_checkpoint", &report.to_string());
                    let journal = host::data_dir().join("native-inbox-recovery.json");
                    let policy = native_filter_host::native_policy_status();
                    if region_cleared && verified_native_rebind_report(&report, topology_changed)
                        && !journal.exists() && policy["mapping_absence_confirmed"] == true
                        && observed.as_ref().is_some_and(|s| s == &host::signature(&host::monitors())) {
                        app.clear_native_scroll_state();
                        app.native_ghost = None;
                        app.native_box_ghosts.clear();
                        app.native_visible_loose_rects = std::sync::Arc::from([]);
                        match try_enable_native_desktop(
                            &mut app, native_save_permitted, &mut native_disk_state,
                            &mut native_last_accepted_state, &mut native_last_accepted_revealed,
                            &mut native_last_accepted_hidden, &mut last_saved_state,
                            &mut last_seen_state, &mut native_display_baseline, get_time(),
                        ) {
                            Ok(worker) => {
                                native_two_box_worker = Some(worker);
                                #[cfg(feature = "native-release")]
                                { native_window_drops = native_drop::NativeWindowDropReceiver::register(host::window()).ok(); }
                                native_display_checked_at = std::time::Instant::now();
                                host::event("INFO", "native_desktop.rebind_ready",
                                    "Fresh Explorer binding verified after recovery");
                            }
                            Err(reason) => {
                                host::event("WARN", "native_desktop.rebind_refused", &reason);
                                if let Some(h) = app.host.as_mut() { h.quit = true; }
                            }
                        }
                    } else {
                        host::event("WARN", "native_desktop.rebind_refused",
                            "Detach, exact recovery or stable display preflight not verified");
                        if let Some(h) = app.host.as_mut() { h.quit = true; }
                    }
                } else if topology_changed {
                    host::event("WARN", "native_chrome.monitor_changed",
                        "Monitor topology changed during an unsafe or repeated transition; exiting safely");
                    if let Some(h) = app.host.as_mut() { h.quit = true; }
                }
            }
        }
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
        #[cfg(all(windows,feature="native-lab",feature="native-unsafe-drag-lab"))]
        if let Some(worker)=native_gesture_worker.as_mut() {
            if native_original_gesture_outcome.is_none() {
                match worker.try_take() {
                    Ok(Some(evidence))=>{
                        let transfer=apply_genuine_native_event_to_ui(
                            &mut app,evidence,
                            if native_chrome_live_transfer{
                                live_transfer_paths.as_ref()
                                    .map(|(checkpoint,expected)|(
                                        checkpoint.as_path(),expected.as_path()))
                            }else{None});
                        if transfer.is_err(){app.save_allowed=false;}
                        native_original_gesture_outcome=Some(
                            transfer.unwrap_or_else(|reason|serde_json::json!({
                                "validated":false,"reason":reason,
                                "normal_saves_suspended":true
                            })));
                        native_original_gesture_worker_report=Some(worker.shutdown());
                        last_saved_state=serde_json::to_vec(&app.state).unwrap_or_default();
                        last_seen_state=last_saved_state.clone();
                    },
                    Ok(None)=>{},
                    Err(reason)=>{
                        app.save_allowed=false;
                        native_original_gesture_outcome=Some(serde_json::json!({
                            "validated":false,"reason":reason,
                            "normal_saves_suspended":true
                        }));
                        native_original_gesture_worker_report=Some(worker.shutdown());
                    }
                }
            }
        }
        #[cfg(all(windows,feature="native-lab"))]
        if let Some(context)=native_main_ole_context.as_ref() {
            // Synchronize any OLE Drop before normal App input, host tick,
            // rendering or autosave. This preserves single-threaded State
            // ownership: the COM callback can only submit a verified result.
            let hwnd=host::window();
            let size=(screen_width()/scale,screen_height()/scale);
            let measured=native_drop::measured_main_hwnd_viewport(
                hwnd,size.0 as f64,size.1 as f64);
            let previous=app.state.clone();
            let outcome=measured.and_then(|geometry| {
                let signature=if native_chrome_ole_lab {
                    host::signature(&host::monitors())
                }else{"isolated-main-window-lab".into()};
                context.lock().map_err(|_|"OLE main-window context poisoned".to_string())?
                    .synchronize_main_thread(&mut app.state,geometry,&signature)
            });
            match outcome {
                Ok(true)=>{
                    app.undo.push(previous.clone());
                    if app.undo.len()>32{app.undo.remove(0);}
                    app.drag=None;
                    app.selected=None;
                    app.multi_selected.clear();
                    app.save_ok=true;
                    last_saved_state=serde_json::to_vec(&app.state).unwrap_or_default();
                    last_seen_state=last_saved_state.clone();
                    app.notify("OLE virtual Pin saved and synchronized to DesktopCraft Box",get_time());
                    if native_chrome_ole_lab {
                        // Native Tab policy was built from the OLD Box
                        // membership. Detach it immediately after a fully
                        // validated persisted UI handoff. Never leave stale
                        // native hide/input policy active while the new
                        // temporary Box membership is displayed.
                        if let Some(mut worker)=native_tab_worker.take(){
                            native_chrome_ole_worker_shutdown=Some(worker.shutdown());
                        }
                        // A detached hook leaves all real icons to Explorer;
                        // never rehook against membership from before Drop.
                        // Revalidate exact source/dest and ALL real native
                        // item identities using the freshly persisted clone.
                        let validated=(||->Result<(u64,u64),String>{
                            let old_shutdown=native_chrome_ole_worker_shutdown.as_ref()
                                .ok_or("Native OLE rehook refused: prior filter was not present")?;
                            if old_shutdown["callback_detached"]!=true {
                                return Err("Native OLE rehook refused: prior callback not detached".into());
                            }
                            let ctx=context.lock()
                                .map_err(|_|"Native OLE rehook context poisoned")?;
                            if ctx.successful_drops!=native_chrome_handled_drops+1
                                || ctx.ui_handoff_pending() {
                                return Err("Native OLE rehook requires the next single acknowledged Drop".into());
                            }
                            let [src,dst]=ctx.visible_box_ids.as_slice() else {
                                return Err("Native OLE rehook requires exactly two verified Boxes".into());
                            };
                            let old_target_count=previous.group(*dst)
                                .ok_or("Native OLE rehook prior target disappeared")?
                                .tab().items.len();
                            let new_target_count=app.state.group(*dst)
                                .ok_or("Native OLE rehook target disappeared")?
                                .tab().items.len();
                            let moved_count=new_target_count.checked_sub(old_target_count)
                                .filter(|n|*n>0 && *n<=8)
                                .ok_or("Native OLE rehook target Pin delta invalid")?;
                            validate_native_ole_rehook_after_move(
                                &previous,&app.state,&ctx.explorer,*src,*dst,moved_count)?;
                            let on_disk=State::load(&app.save_path)?;
                            if serde_json::to_vec(&on_disk).map_err(|e|e.to_string())?
                                !=serde_json::to_vec(&app.state).map_err(|e|e.to_string())? {
                                return Err("Native OLE rehook disk does not match UI State".into());
                            }
                            Ok((*src,*dst))
                        })();
                        native_chrome_ole_rehook_result=Some(match validated {
                            Ok((src,dst))=>{
                                let dll=std::env::current_exe().ok()
                                    .and_then(|exe|exe.ancestors().nth(3)
                                        .map(|base|base.join(
                                            "native-filter-build/debug/desktopcraft_native_filter.dll")))
                                    .unwrap_or_default();
                                let mut new_worker=native_filter_host::NativeTabBackgroundService::start(
                                    app.save_path.clone(),dll,src,native_target_tab);
                                if new_worker.await_ready(std::time::Duration::from_secs(6)){
                                    native_tab_worker=Some(new_worker);
                                    native_chrome_handled_drops+=1;
                                    serde_json::json!({
                                        "validated_post_drop":true,
                                        "fresh_worker_attached":true,
                                        "source_group_id":src,
                                        "destination_group_id":dst,
                                        "old_filter_detached_before_refresh":true,
                                        "native_policy_uses_committed_disk_state":true
                                    })
                                }else{
                                    let failure=new_worker.shutdown();
                                    app.save_allowed=false;
                                    serde_json::json!({
                                        "validated_post_drop":true,
                                        "fresh_worker_attached":false,
                                        "reason":"New native Worker failed to reattach; remained fail-open",
                                        "shutdown":failure,
                                        "normal_saves_suspended":true
                                    })
                                }
                            },
                            Err(reason)=>{
                                app.save_allowed=false;
                                serde_json::json!({
                                    "validated_post_drop":false,
                                    "fresh_worker_attached":false,
                                    "reason":reason,
                                    "normal_saves_suspended":true
                                })
                            }
                        });
                        if let Some(ref status)=native_chrome_ole_rehook_result {
                            native_chrome_ole_rehook_history.push(status.clone());
                        }
                    }
                },
                Ok(false)=>{},
                Err(e)=>{
                    // Never silently write the old UI State over a committed
                    // OLE disk transaction if reconciliation failed.
                    app.save_allowed=false;
                    app.notify(format!("OLE UI reconciliation failed; saves suspended: {e}"),get_time());
                },
            }
        }
        #[cfg(all(windows,feature="native-lab",debug_assertions))]
        if frame==10 && (isolated_ole_smoke_drop || native_chrome_ole_smoke) {
            isolated_ole_smoke_drop_error=_native_main_ole_receiver.as_ref()
                .ok_or("Isolated OLE receiver missing".to_string())
                .and_then(|receiver|receiver.synthetic_disposable_pin_smoke(
                    isolated_ole_smoke_multi || native_chrome_ole_smoke)).err();
        }
        #[cfg(all(windows,feature="native-lab",debug_assertions))]
        if frame==24 && native_chrome_ole_repeat {
            isolated_ole_smoke_drop_error=if native_chrome_handled_drops!=1 {
                Some("Second OLE Drop refused: first refreshed native worker never validated".into())
            }else{
                _native_main_ole_receiver.as_ref()
                    .ok_or("Original Chrome OLE receiver missing".to_string())
                    .and_then(|receiver|receiver.synthetic_disposable_pin_smoke(false)).err()
            };
        }
        p.regions.borrow_mut().clear();
        p.icon_budget.set(10);
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
            )
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
            #[cfg(windows)]
            if app.drag.is_none()
                && app
                    .settings_window
                    .as_ref()
                    .is_some_and(|sw| sw.is_point_over_window(mx, my))
            {
                input.pos = vec2(-10000., -10000.);
            }
        }
        p.mouse = input.pos;
        if native_two_box_ui && (input.pressed || input.released){
            let physical=host::cursor();
            host::event("INFO","native_two_box.mouse_input",
                &format!("frame={frame} pressed={} released={} local=({:.1},{:.1}) screen={physical:?}",
                    input.pressed,input.released,input.pos.x,input.pos.y));
        }
        if !automation && app.color_pick_target.is_some() {
            if let Some(color) = host::finish_color_pick() {
                let index = app.color_pick_target.take().unwrap_or(0);
                if let Some(rgb) = color {
                    app.remember();
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
                app.settings_bring_to_front = true;
            }
        }
        #[cfg(windows)]
        if desktop_mode && !automation {
            host::cleanup_color_pick_hook();
            if let Some(action) = host::sync_tray(app.state.prefs.desktop_enabled) {
                match action {
                    host::TrayAction::ToggleEnabled => {
                        app.desktop_toggle_requested = Some(!app.state.prefs.desktop_enabled);
                    }
                    host::TrayAction::OpenSettings => {
                        app.dialog = Some(Dialog::Settings {
                            page: if app.state.prefs.desktop_enabled { 0 } else { 4 },
                            pattern: String::new(),
                            mode: 0,
                            target: 0,
                        });
                        app.settings_bring_to_front = true;
                    }
                    host::TrayAction::Exit => {
                        if let Some(h) = app.host.as_mut() {
                            h.quit = true;
                        }
                    }
                }
            }
        }
        if !automation {
            // Settings (except during recording) can preview Peek too.
            let available = app.state.prefs.desktop_enabled && !app.has_blocking_dialog();
            let hotkey = available
                && !app.shortcut_recording
                && app.state.prefs.peek_hotkeys
                && platform::hotkey(&app.state.prefs.peek_shortcut);
            if !available || !app.state.prefs.peek_hotkeys {
                app.peek_hotkey_was_down = false;
                if app.peek {
                    app.set_peek(false);
                }
            } else if app.state.prefs.peek_toggle {
                if hotkey && !app.peek_hotkey_was_down {
                    app.set_peek(!app.peek);
                }
                app.peek_hotkey_was_down = hotkey;
            } else {
                app.peek_hotkey_was_down = hotkey;
                // Peek is a temporary reveal while the combination is held.
                // Releasing any key returns the boxes to their desktop layer.
                if hotkey != app.peek {
                    app.set_peek(hotkey);
                }
            }
            let mut dropped: Vec<PathBuf> = get_dropped_files()
                .into_iter()
                .filter_map(|f| f.path)
                .collect();
            let mut drop_at = macroquad::miniquad::take_drop_screen();
            #[cfg(all(windows, feature = "native-release"))]
            if let Some(drop) = native_window_drops.as_ref().and_then(|receiver| receiver.take_drop()) {
                if !app.state.prefs.desktop_enabled {
                    dropped.clear();
                } else {
                    dropped = drop.paths;
                    drop_at = Some(drop.screen);
                    if let native_drop::WindowDropRoute::Tab(group, tab) = drop.route {
                        let current = app.from_screen(drop.screen.0, drop.screen.1);
                        if native_pin_drop::native_tab_header_at(&app.state, (current.x, current.y)) != Some((group, tab)) {
                            dropped.clear();
                            app.notify("The target Tab changed. Drop the icons again.", input.time);
                        }
                    }
                }
            }
            #[cfg(windows)]
            if let Some(reply) = native_header_reply.as_ref() {
                match reply.try_recv() {
                    Ok(result) => {
                        native_header_reply = None;
                        if let Err(reason) = result { app.notify(reason, input.time); }
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        native_header_reply = None;
                        app.notify("Explorer did not confirm the Tab drop.", input.time);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
            }
            if native_main_hwnd_lab {
                // The isolated OLE receiver has exclusive authority over
                // CF_HDROP. Do not also route legacy WM_DROPFILES into the
                // ordinary import/move code: one gesture must not pin twice
                // or accidentally turn virtual Pin into a physical move.
                if !dropped.is_empty() {
                    app.notify("OLE lab: legacy file-drop delivery ignored",input.time);
                }
            } else if desktop_mode {
                if !dropped.is_empty() {
                    #[cfg(windows)]
                    if native_desktop_mode {
                        let screen = drop_at.unwrap_or_else(host::cursor);
                        let local = app.from_screen(screen.0, screen.1);
                        if let Some(destination) = native_pin_drop::native_tab_header_at(&app.state, (local.x, local.y)) {
                            let paths = std::mem::take(&mut dropped).into_iter()
                                .map(|path| path.to_string_lossy().into_owned()).collect();
                            let transfer = if native_header_reply.is_some() {
                                Err("Explorer is handling a previous Tab drop".into())
                            } else {
                                native_two_box_worker.as_ref().ok_or("Explorer Desktop is unavailable".to_string())
                                    .and_then(|worker| worker.drop_onto_tab(destination, screen, paths))
                            };
                            match transfer {
                                Ok(reply) => native_header_reply = Some(reply),
                                Err(reason) => app.notify(reason, input.time),
                            }
                        }
                    }
                    if !dropped.is_empty() {
                        app.drop_files(dropped, input.time, drop_at);
                    }
                }
            } else {
                for path in dropped {
                    app.import_path(&path, path.is_dir(), input.time)
                }
            }
        }
        let mut native_commands = Vec::new();
        if native_chrome_preview {
            // A changed monitor arrangement invalidates native hit targets:
            // exit rather than moving any Desktop icon or saving Box layout.
            if app.minimized_settings.is_some() {
                if !host::settings_has_focus() {
                    app.settings_restore_armed = true;
                } else if app.settings_restore_armed {
                    app.dialog = app.minimized_settings.take();
                    app.settings_restore_armed = false;
                }
            }
            if !automation && app.host.as_ref().is_some_and(|h| input.time - h.last_shown > 1.0) {
                if let Some(h) = app.host.as_mut() {
                    h.last_shown = input.time;
                }
                host::keep_shown();
            }
            #[cfg(windows)]
            if native_desktop_mode && !automation {
                let enable_quick_hide = app.state.prefs.desktop_enabled
                    && app.state.quick_hide
                    && !app.has_blocking_dialog()
                    && app.menu.is_none()
                    && app.folder_sort_menu.is_none()
                    && app.drag.is_none()
                    && !app.peek
                    && !app.native_tx_inflight
                    && !host::color_pick_shielded();
                host::set_native_quick_hide(enable_quick_hide);
                if enable_quick_hide {
                    if let Some((sx, sy)) = host::take_native_quick_hide() {
                        if !app.native_point_hits_box_or_loose_icon(sx, sy) {
                            app.set_hidden(!app.hidden);
                        }
                    }
                } else {
                    let _ = host::take_native_quick_hide();
                }
            }
            if (!native_desktop_mode || app.state.prefs.desktop_enabled)
                && native_display_checked_at.elapsed() >= std::time::Duration::from_secs(1)
            {
                native_display_checked_at=std::time::Instant::now();
                let observed=host::signature(&host::monitors());
                let debug_change=cfg!(debug_assertions)
                    && std::env::var_os("DESKTOPCRAFT_NATIVE_TEST_MAIN_MONITOR_CHANGED")
                        .is_some_and(|v|v=="1");
                if native_display_baseline.as_ref()
                    .is_some_and(|baseline|observed.is_empty() || *baseline!=observed)
                    || debug_change {
                    host::event("WARN","native_chrome.monitor_changed",
                        "Monitor topology changed; exiting safely, detaching native filter; original icon layout untouched");
                    if let Some(h)=app.host.as_mut(){h.quit=true;}
                }
            }
            native_commands = host::take_commands();
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
                        // The verified Setup waits until this exact running
                        // DesktopCraft instance has exited and the native
                        // recovery journal is gone. Request normal GUI exit:
                        // this detaches Explorer hooks and restores icons.
                        match std::process::Command::new(installer)
                            .args(["--install", "--approved-update", "--wait-for-app-rollback"])
                            .spawn()
                        {
                            Ok(_) => {
                                host::event("INFO","update.verified_installer_started",
                                    "requesting graceful app exit before installer replaces executable");
                                if let Some(host)=app.host.as_mut(){
                                    host.quit=true;
                                }else{
                                    app.notify("Close DesktopCraft to complete the verified update.",input.time);
                                }
                            }
                            Err(_) => app.notify("Could not start the verified installer", input.time),
                        }
                    }
                    updates::Event::Failure(error) => app.notify(error, input.time),
                }
            }
        }
        let render_start = std::time::Instant::now();
        let native_desktop_page_before=app.state.page;
        let native_active_before=if native_chrome_preview {
            app.state.group(native_target_group).map(|g|g.active)
        }else{None};
        let native_two_box_before:Vec<(u64,usize)>=if native_two_box_ui {
            if native_general_workspace {
                app.state.groups.iter().filter(|group|
                    group.page==app.state.page && !group.rolled
                    && group.tabs.get(group.active).is_some_and(|t|t.folder.is_none()))
                    .map(|g|(g.id,g.active)).collect()
            }else{
                [144u64,168].iter().filter_map(|&group_id|
                    app.state.group(group_id).map(|group|(group_id,group.active)))
                    .collect()
            }
        }else{Vec::new()};
        #[cfg(windows)]
        if let Some(sync) = native_two_box_worker
            .as_ref()
            .and_then(native_filter_host::NativeTwoBoxBackgroundService::take_layout_sync)
        {
            if let Some(pages) = sync.pages {
                app.native_pages = pages;
                if !native_desktop_mode {
                    app.advance_native_scroll_target();
                }
            }
            if let Some(icons) = sync.ghost_icons {
                app.native_box_ghosts = icons;
            }
            if let Some(rects) = sync.visible_loose_rects {
                app.native_visible_loose_rects = rects;
            }
            let cell_changed = app.native_cell != sync.main_cell;
            app.native_cell = sync.main_cell;
            app.native_icon_size = sync.icon_size;
            app.native_snap_to_grid = sync.snap_to_grid;
            if cell_changed {
                let primary_dpi = (app.host_scale() * 96.0).round().max(96.0) as u32;
                let label_width = ghost_label_width(app.native_cell);
                if let Some(art) = app.ghost_art.as_mut() {
                    for tab in app.state.groups.iter().flat_map(|g| &g.tabs).filter(|t| t.folder.is_none()) {
                        for pin in &tab.items {
                            art.request_label(pin.label.as_deref().unwrap_or(&pin.name), label_width, primary_dpi);
                        }
                    }
                }
            }
        }
        #[cfg(windows)]
        if native_desktop_mode && app.state.prefs.desktop_enabled {
            app.update_native_ghost(native_two_box_worker.as_ref(), native_pending_ui.is_some());
        }
        #[cfg(windows)]
        let mut retired_drag = None;
        #[cfg(windows)]
        let native_acknowledged = native_last_accepted_state.as_ref();
        #[cfg(not(windows))]
        let native_acknowledged = None;
        #[cfg(windows)]
        let drawn_portals = if native_desktop_mode && app.state.prefs.desktop_enabled {
            // App::frame draws first, then handles the tab click. Remember
            // which Portal bodies may still be in this frame's window region.
            app.state.groups.iter().filter(|g| {
                g.page == app.state.page && g.tab().folder.is_some()
                    && !portal_waiting_for_explorer(g, native_acknowledged)
            }).map(|g| g.id).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        #[cfg(windows)]
        let mut native_region_applied = false;
        #[cfg(not(windows))]
        let native_region_applied = false;
        #[cfg(windows)]
        if native_desktop_mode && app.state.prefs.desktop_enabled {
            app.native_tx_inflight = native_pending_ui.is_some()
                || native_rules_sort_reply.is_some()
                || native_layout_restore_reply.is_some()
                || native_pending_layout_followup.is_some()
                || native_stage2_layout_followup.is_some()
                || native_pin_paths_reply.is_some()
                || native_pending_pin_paths_toast.is_some()
                || native_pending_sort_followup
                || native_pending_drop_commit.is_some()
                || app.native_rules_sort_requested.is_some()
                || app.native_layout_restore_requested.is_some()
                || app.native_pin_paths_requested.is_some()
                || app.native_ai_pending.is_some()
                || app.native_undo_inflight.is_some()
                || app.native_record_undo_on_sync.is_some();
        } else {
            #[cfg(windows)]
            {
                app.native_tx_inflight = false;
            }
        }
        for command in native_commands {
            app.run_command(&command, input.time);
        }
        #[cfg(windows)]
        let native_pin_tx_waiting = native_desktop_mode
            && app.state.prefs.desktop_enabled
            && (native_rules_sort_reply.is_some()
                || native_layout_restore_reply.is_some()
                || native_pending_layout_followup.is_some()
                || native_stage2_layout_followup.is_some()
                || native_pin_paths_reply.is_some()
                || native_pending_pin_paths_toast.is_some()
                || native_pending_sort_followup
                || native_pending_drop_commit.is_some()
                || app.native_rules_sort_requested.is_some()
                || app.native_layout_restore_requested.is_some()
                || app.native_pin_paths_requested.is_some()
                || app.native_ai_pending.is_some()
                || app.native_undo_inflight.is_some()
                || app.native_record_undo_on_sync.is_some());
        #[cfg(windows)]
        let native_tx_baseline = if native_pin_tx_waiting {
            native_pending_ui
                .as_ref()
                .and_then(|(_, prop)| prop.as_ref().map(|(st, rev, hid)| (st.clone(), *rev, *hid)))
                .or_else(|| {
                    (app.native_undo_inflight.is_some()
                        || app.native_record_undo_on_sync.is_some()
                        || native_stage2_layout_followup.is_some()
                        || app.native_ai_pending.as_ref().is_some_and(|p| {
                            p.route == ai_tools::NativeAiToolRoute::WorkspaceSync
                        }))
                    .then(|| (app.state.clone(), app.revealed, app.hidden))
                })
                .or_else(|| {
                    native_last_accepted_state
                        .as_ref()
                        .map(|st| (st.clone(), native_last_accepted_revealed, native_last_accepted_hidden))
                })
        } else {
            None
        };
        #[cfg(windows)]
        let native_undo_before_frame = (native_pin_tx_waiting
            || (native_desktop_mode && native_pending_ui.is_some()))
        .then(|| app.undo.clone());
        #[cfg(windows)]
        if !app.settings_in_own_window() {
            if let Some(sw) = app.settings_window.as_mut() {
                sw.hide();
            }
        }
        app.frame(&mut p, &input, native_acknowledged);
        #[cfg(windows)]
        if let (Some((ref base_state, base_rev, base_hidden)), Some(ref before_undo)) =
            (&native_tx_baseline, &native_undo_before_frame)
        {
            clamp_native_tx_frame_mutation(&mut app, (base_state, *base_rev, *base_hidden), before_undo);
        }
        #[cfg(windows)]
        app.render_own_settings_window(&mut p, input.time);
        #[cfg(windows)]
        if app.native_ai_pending.is_some() {
            if let (Some((ref base_state, base_rev, base_hidden)), Some(ref before_undo)) =
                (&native_tx_baseline, &native_undo_before_frame)
            {
                if app.state != *base_state
                    || app.revealed != *base_rev
                    || app.hidden != *base_hidden
                    || app.undo != *before_undo
                {
                    app.state = base_state.clone();
                    app.revealed = *base_rev;
                    app.hidden = *base_hidden;
                    app.undo = before_undo.clone();
                }
            }
        }
        if let Some(enable) = app.desktop_toggle_requested.take() {
            if enable {
                #[cfg(windows)]
                if native_desktop_mode {
                    match try_enable_native_desktop(
                        &mut app,
                        native_save_permitted,
                        &mut native_disk_state,
                        &mut native_last_accepted_state,
                        &mut native_last_accepted_revealed,
                        &mut native_last_accepted_hidden,
                        &mut last_saved_state,
                        &mut last_seen_state,
                        &mut native_display_baseline,
                        input.time,
                    ) {
                        Ok(worker) => {
                            native_two_box_worker = Some(worker);
                            #[cfg(feature = "native-release")]
                            if native_window_drops.is_none() {
                                native_window_drops = native_drop::NativeWindowDropReceiver::register(host::window())
                                    .map_err(|reason| {
                                        host::event(
                                            "WARN",
                                            "native_desktop.tab_drop_receiver_unavailable",
                                            &reason,
                                        );
                                        reason
                                    })
                                    .ok();
                            }
                            app.settings_last_paint = 0.0;
                            app.notify("DesktopCraft enabled", input.time);
                        }
                        Err(reason) => {
                            app.settings_last_paint = 0.0;
                            app.notify(reason, input.time);
                        }
                    }
                } else {
                    app.state.prefs.desktop_enabled = true;
                    app.save(input.time);
                    app.settings_last_paint = 0.0;
                    app.notify("DesktopCraft enabled", input.time);
                }
                #[cfg(not(windows))]
                {
                    app.state.prefs.desktop_enabled = true;
                    app.save(input.time);
                    app.notify("DesktopCraft enabled", input.time);
                }
            } else {
                #[cfg(windows)]
                let busy = native_desktop_mode
                    && (app.native_tx_inflight
                        || native_pending_ui.is_some()
                        || native_pin_tx_waiting
                        || app.drag.is_some());
                #[cfg(not(windows))]
                let busy = false;
                if busy {
                    app.notify(
                        "Wait for the current desktop update to finish before disabling.",
                        input.time,
                    );
                } else {
                    app.state.prefs.desktop_enabled = false;
                    let mut saved_ok = true;
                    #[cfg(windows)]
                    if native_desktop_mode {
                        if native_save_permitted {
                            persist_native_workspace(
                                &mut app,
                                &mut native_disk_state,
                                &mut native_last_accepted_state,
                                &mut last_saved_state,
                                &mut last_seen_state,
                                input.time,
                            );
                            if !app.save_ok {
                                app.state.prefs.desktop_enabled = true;
                                saved_ok = false;
                            }
                        } else if let Err(e) = app.state.save(&app.save_path) {
                            app.state.prefs.desktop_enabled = true;
                            saved_ok = false;
                            app.notify(format!("Could not save disabled state: {e}"), input.time);
                        }
                    } else {
                        app.host_exit();
                        app.save(input.time);
                    }
                    #[cfg(not(windows))]
                    {
                        app.host_exit();
                        app.save(input.time);
                    }
                    if saved_ok {
                        #[cfg(windows)]
                        if native_desktop_mode {
                            if let Some(mut worker) = native_two_box_worker.take() {
                                let report = worker.shutdown();
                                host::event(
                                    "INFO",
                                    "native_desktop.disabled_live",
                                    &format!("DesktopCraft disabled live from Settings: {report}"),
                                );
                            }
                            #[cfg(feature = "native-release")]
                            {
                                native_window_drops = None;
                            }
                            host::set_native_quick_hide(false);
                            let _ = host::take_native_quick_hide();
                            native_last_accepted_state = None;
                            native_last_accepted_revealed = None;
                            native_last_accepted_hidden = false;
                            native_pending_ui = None;
                            native_rules_sort_reply = None;
                            native_layout_restore_reply = None;
                            native_pending_layout_followup = None;
                            native_stage2_layout_followup = None;
                            native_pin_paths_reply = None;
                            native_pending_pin_paths_toast = None;
                            native_pending_sort_followup = false;
                            native_pending_drop_commit = None;
                            let _ = ai_tools::abort_pending(
                                &mut app,
                                "DesktopCraft was disabled before the change completed",
                            );
                        }
                        app.revealed = None;
                        app.native_visual_revealed = None;
                        app.hover_rolled = None;
                        app.hidden = false;
                        app.peek = false;
                        app.menu = None;
                        app.folder_sort_menu = None;
                        app.drag = None;
                        app.native_tx_inflight = false;
                        app.clear_native_scroll_state();
                        #[cfg(windows)]
                        {
                            app.native_ghost = None;
                            app.sync_native_backdrop(false, None, false);
                            app.settings_last_paint = 0.0;
                        }
                        if let Some(h) = app.host.as_mut() {
                            if !h.region.is_empty() {
                                host::set_region(&[]);
                                h.region.clear();
                            }
                            if h.front {
                                host::set_front(false);
                                h.front = false;
                            }
                        }
                        app.notify(
                            "DesktopCraft disabled. Close Settings to exit, or enable anytime.",
                            input.time,
                        );
                    }
                }
            }
        }
        if desktop_mode
            && !automation
            && !app.state.prefs.desktop_enabled
            && app.native_ai_pending.is_none()
            && app.dialog.is_none()
            && app.minimized_settings.is_none()
            && app.color_pick_target.is_none()
            && app.settings_return.is_none()
        {
            #[cfg(windows)]
            if native_desktop_mode && native_save_permitted {
                let saved_matches = native_disk_state.as_ref().is_some_and(|disk| {
                    serde_json::to_vec(disk).ok() == serde_json::to_vec(&app.state).ok()
                });
                if !saved_matches {
                    persist_native_workspace(
                        &mut app,
                        &mut native_disk_state,
                        &mut native_last_accepted_state,
                        &mut last_saved_state,
                        &mut last_seen_state,
                        input.time,
                    );
                }
            } else {
                app.save(input.time);
            }
            #[cfg(not(windows))]
            app.save(input.time);
            if let Some(h) = app.host.as_mut() {
                h.quit = true;
            }
        }
        #[cfg(all(windows, feature = "native-release"))]
        if let Some(receiver) = native_window_drops.as_ref() {
            let mut targets = Vec::new();
            if app.state.prefs.desktop_enabled
                && !app.has_blocking_dialog()
                && app.menu.is_none()
                && app.drag.is_none()
            {
                for group in app.state.groups.iter().rev().filter(|g| g.page == app.state.page) {
                    for (index, bounds) in paint::native_header_tabs(group) {
                        let tab = &group.tabs[index];
                        if tab.folder.is_none() {
                            targets.push((app.to_screen(bounds), native_drop::WindowDropRoute::Tab(group.id, tab.id)));
                        }
                    }
                    // Put the title fallback after named tabs, and leave Portal tabs to their own routes.
                    if group.rolled && group.tabs.iter().all(|tab| tab.folder.is_none()) {
                        targets.push((app.to_screen(paint::native_group_header(group)),
                            native_drop::WindowDropRoute::Tab(group.id, group.tab().id)));
                    }
                    if !group.rolled && group.tab().folder.is_some() {
                        targets.push((app.to_screen(paint::native_group_body(group)), native_drop::WindowDropRoute::Folder));
                    }
                }
            }
            receiver.set_targets(targets);
        }
        if native_two_box_ui && args.iter().any(|a|a=="--native-two-box-ui-smoke"){
            // Exercise the exact App.frame -> two-Box ACK-backed path that
            // handles real header Tab clicks, against an isolated copy.
            let change=match frame{
                60=>Some((144,0)),
                105=>Some((168,1)),
                150=>Some((168,0)),
                195=>Some((144,1)),
                _=>None,
            };
            if let Some((id,tab))=change{
                if let Some(group)=app.state.group_mut(id){group.active=tab;}
            }
        }
        if native_chrome_preview && !native_two_box_ui
            && args.iter().any(|a|a=="--native-chrome-ui-smoke") {
            // Exercises the SAME App-State -> NativeFilterWorker path as
            // actual Tab clicks, but only against an isolated Workspace copy.
            if frame==60 {
                if let Some(g)=app.state.group_mut(native_target_group){g.active=native_target_tab;}
            } else if frame==140 {
                if let Some(g)=app.state.group_mut(native_target_group){g.active=native_from_tab;}
            }
        }
        #[cfg(windows)]
        let mut native_ai_exit_rollback = false;
        #[cfg(windows)]
        if native_desktop_mode && app.state.prefs.desktop_enabled {
            let exiting = app.host.as_ref().is_some_and(|h| h.quit) || is_quit_requested();
            // right-click menus/dragging can temporarily occupy that thread.
            // At most ONE command remains in flight. No late ACK commits
            // after its deadline: the service stops its Explorer hook instead.
            let mut must_stop = exiting
                || native_two_box_worker
                    .as_ref()
                    .is_none_or(|worker| !worker.is_ready());
            if must_stop {
                // Also handle spontaneous Explorer restart/detach while
                // idle: no outstanding ACK is required for a safe exit.
                if !exiting {
                    host::event("WARN","native_desktop.worker_not_ready",
                        "Native Explorer filter detached; exiting UI before any more file interactions");
                }
                let reason = if exiting {
                    "DesktopCraft exited before Explorer confirmed the change"
                } else {
                    "Native Explorer worker disconnected before confirming the change"
                };
                if app.native_ai_pending.is_some() {
                    let _ = ai_tools::abort_pending_checked(&mut app, reason);
                    native_last_accepted_state = Some(app.state.clone());
                    native_ai_exit_rollback = true;
                }
                if let Some(h)=app.host.as_mut(){h.quit=true;}
            }
            // The Explorer worker queues the verified layout immediately.
            // The UI still merges Pin changes and saves only after the ACK.
            // Wait until any in-flight batch reply has been consumed so follow-up
            // markers are always populated before the matching verified drop.
            if !must_stop
                && native_pending_ui.is_none()
                && native_pin_paths_reply.is_none()
                && native_layout_restore_reply.is_none()
                && native_rules_sort_reply.is_none()
            {
                let event=native_two_box_worker.as_mut()
                    .ok_or_else(||"Native Explorer drag receiver is unavailable".to_string())
                    .and_then(|worker|worker.poll_verified_drop());
                match event {
                    Ok(Some((proposal, ticket)))=>{
                        if matches!(&proposal.kind, native_pin_drop::NativeMultiDropKind::SortRules) {
                            native_pending_sort_followup = false;
                        }
                        let is_ai_owned_proposal = matches!(
                            &proposal.kind,
                            native_pin_drop::NativeMultiDropKind::RestoreLayout
                        ) && (native_pending_pin_paths_toast.is_some()
                            || native_pending_layout_followup.is_some());
                        if !is_ai_owned_proposal && app.native_ai_pending.is_some() {
                            app.native_pin_paths_requested = None;
                            app.native_layout_restore_requested = None;
                            native_stage2_layout_followup = None;
                            let explorer_changed = ai_tools::pending_explorer_diverged(
                                &app,
                                native_last_accepted_state.as_ref(),
                            );
                            if ai_tools::abort_pending(
                                &mut app,
                                "Desktop changed by a user drag; please retry the AI request",
                            ) || explorer_changed {
                                native_ai_exit_rollback = true;
                                native_last_accepted_state = Some(app.state.clone());
                                must_stop = true;
                                if let Some(h) = app.host.as_mut() {
                                    h.quit = true;
                                }
                            }
                        }
                        if must_stop {
                            // An earlier AI stage touched disk and rolled back; do not commit a user drag on top.
                        } else {
                        let matches_acknowledged = native_last_accepted_state.as_ref().is_some_and(|accepted| {
                            native_drop_matches_acknowledged_ui(
                                accepted,
                                native_last_accepted_revealed,
                                native_last_accepted_hidden,
                                &app.state,
                                app.hidden,
                                &proposal.before,
                            )
                        });
                        if !matches_acknowledged || native_pending_drop_commit.is_some()
                        {
                            host::event("WARN","native_desktop.drag_state_mismatch",
                                "Explorer drag source differs from last acknowledged UI; exiting without changing Workspace");
                            must_stop=true;
                            if let Some(h)=app.host.as_mut(){h.quit=true;}
                        } else {
                            match native_ole_persist::pins_change_onto(
                                &app.state,
                                &proposal.next,
                                &proposal.source_paths,
                            ) {
                                Ok(patched_real) => {
                                    app.state = patched_real.clone();
                                    app.revealed = native_last_accepted_revealed;
                                    app.hidden = native_last_accepted_hidden;
                                    if app.revealed.is_some() {
                                        app.rollup_interact_until = input.time + 0.50;
                                    }
                                    native_pending_drop_commit = Some(proposal);
                                    native_pending_ui = Some((
                                        ticket,
                                        Some((patched_real, native_last_accepted_revealed, native_last_accepted_hidden)),
                                    ));
                                }
                                Err(reason) => {
                                    host::event("WARN","native_desktop.drag_state_mismatch",&reason);
                                    must_stop=true;
                                    if let Some(h)=app.host.as_mut(){h.quit=true;}
                                }
                            }
                        }
                        }
                    }
                    Ok(None)=>{}
                    Err(reason)=>{
                        host::event("WARN","native_desktop.drag_channel_failed",&reason);
                        must_stop=true;
                        if let Some(h)=app.host.as_mut(){h.quit=true;}
                    }
                }
            }
            if !must_stop {
            if let Some((ticket, scope)) = native_rules_sort_reply.take() {
                use native_filter_host::NativeWorkspaceReply;
                let reply = native_two_box_worker.as_ref()
                    .ok_or_else(|| "Windows Explorer is unavailable".to_string())
                    .and_then(|worker| worker.poll_workspace_sync(&ticket));
                match reply {
                    Ok(NativeWorkspaceReply::Pending) => native_rules_sort_reply = Some((ticket, scope)),
                    Ok(NativeWorkspaceReply::Applied(0)) => {
                        app.clear_native_scroll_state();
                        app.notify(
                            if scope == desktop::RuleSortScope::LooseOnly {
                                "No loose desktop icons need moving under the enabled Rules."
                            } else {
                                "No icons need moving under the enabled Rules."
                            },
                            input.time,
                        );
                    }
                    Ok(NativeWorkspaceReply::Applied(_)) => {
                        if !native_pending_drop_commit.as_ref().is_some_and(|drop| {
                            matches!(&drop.kind, native_pin_drop::NativeMultiDropKind::SortRules)
                        }) {
                            native_pending_sort_followup = true;
                        }
                    }
                    Ok(NativeWorkspaceReply::Rejected(reason)) | Err(reason) => {
                        native_pending_sort_followup = false;
                        app.clear_native_scroll_state();
                        host::event("WARN", "native_desktop.rules_sort_rejected", &reason);
                        app.notify(format!("Could not sort icons: {reason}"), input.time);
                    }
                }
            }
            if let Some((ticket, toast_template)) = native_pin_paths_reply.take() {
                use native_filter_host::NativeWorkspaceReply;
                let reply = native_two_box_worker.as_ref()
                    .ok_or_else(|| "Windows Explorer is unavailable".to_string())
                    .and_then(|worker| worker.poll_workspace_sync(&ticket));
                match reply {
                    Ok(NativeWorkspaceReply::Pending) => {
                        native_pin_paths_reply = Some((ticket, toast_template));
                    }
                    Ok(NativeWorkspaceReply::Applied(0)) => {
                        app.clear_native_scroll_state();
                        if app.native_ai_pending.as_ref().is_some_and(|p| {
                            matches!(p.route, ai_tools::NativeAiToolRoute::PinFile { .. })
                        }) {
                            if ai_tools::abort_pending(
                                &mut app,
                                "File could not be pinned (already pinned or duplicated)",
                            ) {
                                native_ai_exit_rollback = true;
                                native_last_accepted_state = Some(app.state.clone());
                                must_stop = true;
                                if let Some(h) = app.host.as_mut() {
                                    h.quit = true;
                                }
                            }
                        }
                        app.notify("File is already pinned in this Box.", input.time);
                    }
                    Ok(NativeWorkspaceReply::Applied(n)) => {
                        native_pending_pin_paths_toast =
                            Some(toast_template.replace("{count}", &n.to_string()));
                    }
                    Ok(NativeWorkspaceReply::Rejected(reason)) => {
                        app.clear_native_scroll_state();
                        if ai_tools::abort_pending(&mut app, reason.clone()) {
                            native_ai_exit_rollback = true;
                            native_last_accepted_state = Some(app.state.clone());
                            must_stop = true;
                            if let Some(h) = app.host.as_mut() {
                                h.quit = true;
                            }
                        }
                        host::event("WARN", "native_desktop.pin_paths_rejected", &reason);
                        app.notify(reason, input.time);
                    }
                    Err(reason) => {
                        app.clear_native_scroll_state();
                        if app.native_ai_pending.is_some() {
                            let _ = ai_tools::abort_pending_checked(&mut app, reason.clone());
                            native_ai_exit_rollback = true;
                            native_last_accepted_state = Some(app.state.clone());
                        }
                        must_stop = true;
                        if let Some(h) = app.host.as_mut() {
                            h.quit = true;
                        }
                        host::event("WARN", "native_desktop.pin_paths_failed", &reason);
                        app.notify(reason, input.time);
                    }
                }
            }
            if let Some((ticket, req)) = native_layout_restore_reply.take() {
                use native_filter_host::NativeWorkspaceReply;
                let reply = native_two_box_worker.as_ref()
                    .ok_or_else(|| "Windows Explorer is unavailable".to_string())
                    .and_then(|worker| worker.poll_workspace_sync(&ticket));
                match reply {
                    Ok(NativeWorkspaceReply::Pending) => {
                        native_layout_restore_reply = Some((ticket, req));
                    }
                    Ok(NativeWorkspaceReply::Applied(0)) => {
                        if !desktop::native_layout_structure_matches(&app.state, &req.target) {
                            app.state = desktop::stage2_apply_target_structure_preserving_pins(
                                &app.state,
                                &req.target,
                            );
                            app.revealed = None;
                            app.hover_rolled = None;
                            app.selected = None;
                            app.multi_selected.clear();
                            app.clear_native_scroll_state();
                            native_stage2_layout_followup = Some(req);
                        } else {
                            let ai_pin_mismatch = app.native_ai_pending.as_ref().is_some_and(|p| {
                                p.route == ai_tools::NativeAiToolRoute::LayoutRestore
                                    && p.expected_target.as_ref().is_some_and(|expected| {
                                        !ai_tools::native_regular_pin_paths_match(
                                            &app.state, expected,
                                        )
                                    })
                            });
                            if ai_pin_mismatch {
                                if let Some(undo_state) = req.restore_undo_on_fail {
                                    app.push_undo(undo_state);
                                }
                                app.clear_native_scroll_state();
                                if ai_tools::abort_pending(
                                    &mut app,
                                    "Explorer could not apply all requested pin changes; some icons were missing from Desktop or duplicated",
                                ) {
                                    native_ai_exit_rollback = true;
                                    native_last_accepted_state = Some(app.state.clone());
                                    must_stop = true;
                                    if let Some(h) = app.host.as_mut() {
                                        h.quit = true;
                                    }
                                }
                            } else {
                                if req.record_undo {
                                    app.native_record_undo_on_sync =
                                        req.undo_snapshot.or_else(|| native_last_accepted_state.clone());
                                } else {
                                    app.native_undo_inflight = req.restore_undo_on_fail;
                                }
                                desktop::apply_non_pin_layout_from(&mut app.state, &req.target);
                                app.revealed = None;
                                app.hover_rolled = None;
                                app.selected = None;
                                app.multi_selected.clear();
                                app.clear_native_scroll_state();
                                if app.native_ai_pending.as_ref().is_some_and(|p| {
                                    p.route == ai_tools::NativeAiToolRoute::LayoutRestore
                                }) {
                                    let needs_layout_ack = native_last_accepted_state
                                        .as_ref()
                                        .is_some_and(|acc| {
                                            native_effective_sync_needed(
                                                acc,
                                                native_last_accepted_revealed,
                                                native_last_accepted_hidden,
                                                &app.state,
                                                app.revealed,
                                                app.hidden,
                                            )
                                        });
                                    if needs_layout_ack {
                                        if let Some(ref mut pending) = app.native_ai_pending {
                                            pending.route =
                                                ai_tools::NativeAiToolRoute::WorkspaceSync;
                                        }
                                    } else {
                                        if native_save_permitted {
                                            persist_native_workspace(
                                                &mut app,
                                                &mut native_disk_state,
                                                &mut native_last_accepted_state,
                                                &mut last_saved_state,
                                                &mut last_seen_state,
                                                input.time,
                                            );
                                        }
                                        let saved = native_save_permitted && app.save_ok;
                                        if !ai_tools::complete_pending_if_saved(
                                            &mut app,
                                            saved,
                                            "Could not save workspace changes",
                                        ) {
                                            app.save_allowed = false;
                                            native_save_permitted = false;
                                            native_ai_exit_rollback = true;
                                            native_last_accepted_state = Some(app.state.clone());
                                            must_stop = true;
                                            if let Some(h) = app.host.as_mut() {
                                                h.quit = true;
                                            }
                                        }
                                    }
                                }
                                if !req.success_toast.is_empty() {
                                    app.notify(req.success_toast, input.time);
                                }
                            }
                        }
                    }
                    Ok(NativeWorkspaceReply::Applied(_)) => {
                        native_pending_layout_followup = Some(req);
                    }
                    Ok(NativeWorkspaceReply::Rejected(reason)) => {
                        if let Some(undo_state) = req.restore_undo_on_fail {
                            app.push_undo(undo_state);
                        }
                        app.clear_native_scroll_state();
                        let explorer_changed = ai_tools::pending_explorer_diverged(
                            &app,
                            native_last_accepted_state.as_ref(),
                        );
                        if ai_tools::abort_pending(&mut app, reason.clone()) || explorer_changed {
                            native_ai_exit_rollback = true;
                            native_last_accepted_state = Some(app.state.clone());
                            must_stop = true;
                            if let Some(h) = app.host.as_mut() {
                                h.quit = true;
                            }
                        }
                        host::event("WARN", "native_desktop.layout_restore_rejected", &reason);
                        app.notify(format!("Could not restore layout: {reason}"), input.time);
                    }
                    Err(reason) => {
                        if let Some(undo_state) = req.restore_undo_on_fail {
                            app.push_undo(undo_state);
                        }
                        app.clear_native_scroll_state();
                        if app.native_ai_pending.is_some() {
                            let _ = ai_tools::abort_pending_checked(&mut app, reason.clone());
                            native_ai_exit_rollback = true;
                            native_last_accepted_state = Some(app.state.clone());
                        }
                        must_stop = true;
                        if let Some(h) = app.host.as_mut() {
                            h.quit = true;
                        }
                        host::event("WARN", "native_desktop.layout_restore_failed", &reason);
                        app.notify(format!("Could not restore layout: {reason}"), input.time);
                    }
                }
            }
            if let Some((ticket,proposed))=native_pending_ui.take() {
                use native_filter_host::NativeWorkspaceReply;
                let reply=native_two_box_worker.as_ref()
                    .ok_or_else(||"Native Explorer worker disconnected while ACK was pending".to_string())
                    .and_then(|worker|worker.poll_workspace_sync(&ticket));
                if !matches!(&reply, Ok(NativeWorkspaceReply::Pending)) {
                    if let Some((group, started)) = native_tab_trace.take() {
                        let outcome = match &reply {
                            Ok(NativeWorkspaceReply::Applied(_)) => "applied",
                            Ok(NativeWorkspaceReply::Rejected(_)) => "rejected",
                            _ => "failed",
                        };
                        host::event("INFO", "native_desktop.tab_finished", &format!(
                            "group={group} elapsed_ms={} outcome={outcome}", started.elapsed().as_millis()));
                    }
                }
                match reply {
                    Ok(NativeWorkspaceReply::Applied(_))=>{
                        if let Some((proposed_real, proposed_revealed, proposed_hidden))=proposed {
                            // The real Explorer worker ACKed a new native
                            // icon generation. For an actual drag, persist
                            // ONLY the one-path virtual Pin transfer, with
                            // Windows ReplaceFileW predecessor preservation,
                            // BEFORE treating the in-memory UI as committed.
                            if let Some(drop)=native_pending_drop_commit.take(){
                                let sorting = matches!(&drop.kind, native_pin_drop::NativeMultiDropKind::SortRules);
                                let restoring = matches!(&drop.kind, native_pin_drop::NativeMultiDropKind::RestoreLayout);
                                let ai_precommit_check = if restoring {
                                    match app.native_ai_pending.as_ref() {
                                        Some(p)
                                            if native_pending_pin_paths_toast.is_some() =>
                                        {
                                            if let ai_tools::NativeAiToolRoute::PinFile {
                                                group_id,
                                                tab_id,
                                                ref path,
                                            } = p.route
                                            {
                                                ai_tools::validate_pin_file_proposal(
                                                    &drop.before,
                                                    &proposed_real,
                                                    group_id,
                                                    tab_id,
                                                    path,
                                                )
                                            } else {
                                                Ok(())
                                            }
                                        }
                                        Some(p)
                                            if p.route == ai_tools::NativeAiToolRoute::LayoutRestore =>
                                        {
                                            if let Some(f) = native_pending_layout_followup.as_ref() {
                                                if desktop::native_layout_structure_matches(
                                                    &proposed_real,
                                                    &f.target,
                                                ) {
                                                    if p.expected_target.as_ref().is_some_and(|target| {
                                                        !ai_tools::native_regular_pin_paths_match(
                                                            &proposed_real,
                                                            target,
                                                        )
                                                    }) {
                                                        Err("Explorer could not apply all requested pin changes; some icons were missing from Desktop or duplicated".into())
                                                    } else {
                                                        Ok(())
                                                    }
                                                } else if let Some(stage1) =
                                                    desktop::stage1_unpin_incompatible_tabs(
                                                        &drop.before,
                                                        &f.target,
                                                    )
                                                {
                                                    if !ai_tools::native_regular_pin_paths_match(
                                                        &proposed_real,
                                                        &stage1,
                                                    ) {
                                                        Err("Explorer Stage 1 unpin proposal did not match expected target".into())
                                                    } else {
                                                        Ok(())
                                                    }
                                                } else {
                                                    Ok(())
                                                }
                                            } else {
                                                Ok(())
                                            }
                                        }
                                        _ => Ok(()),
                                    }
                                } else {
                                    Ok(())
                                };
                                let ai_owns_commit = restoring
                                    && (native_pending_pin_paths_toast.is_some()
                                        || native_pending_layout_followup.is_some());
                                let result: Result<(Option<(Vec<u8>, Vec<u8>)>, usize), String> =
                                    ai_precommit_check.and_then(|()| {
                                        let persisted =
                                            native_disk_state.as_ref().unwrap_or(&drop.before);
                                        if ai_owns_commit {
                                            native_ole_persist::commit_verified_native_layout_restore_over_with_bytes(
                                                &app.save_path,
                                                persisted,
                                                &drop.before,
                                                &drop.next,
                                                &drop.source_paths,
                                                ai_tools::expected_ai_disk_bytes(&app),
                                            )
                                            .map(|(pred, written)| {
                                                let len = written.len();
                                                (Some((pred, written)), len)
                                            })
                                        } else {
                                            let commit = if sorting {
                                                native_ole_persist::commit_verified_native_rules_sort_over
                                            } else if restoring {
                                                native_ole_persist::commit_verified_native_layout_restore_over
                                            } else {
                                                native_ole_persist::commit_verified_native_pin_drops_over
                                            };
                                            commit(
                                                &app.save_path,
                                                persisted,
                                                &drop.before,
                                                &drop.next,
                                                &drop.source_paths,
                                            )
                                            .map(|len| (None, len))
                                        }
                                    });
                                match result {
                                    Ok((ai_bytes, bytes))=>{
                                        if let Some((pred, written)) = ai_bytes {
                                            ai_tools::record_ai_disk_bytes(
                                                &mut app,
                                                pred,
                                                written,
                                            );
                                        }
                                        native_disk_state=State::load(&app.save_path).ok();
                                        let undo_before = native_last_accepted_state.clone();
                                        app.state=proposed_real.clone();
                                        app.revealed=proposed_revealed;
                                        app.hidden=proposed_hidden;
                                        native_last_accepted_state=Some(proposed_real);
                                        native_last_accepted_revealed=proposed_revealed;
                                        native_last_accepted_hidden=proposed_hidden;
                                        app.native_visual_revealed=native_last_accepted_revealed;
                                        let destination = match &drop.kind {
                                            native_pin_drop::NativeMultiDropKind::DesktopToBox { to, to_tab, .. } |
                                            native_pin_drop::NativeMultiDropKind::BoxToBox { to, to_tab, .. } => Some((*to, *to_tab)),
                                            _ => None,
                                        };
                                        if let Some((group_id, tab_id)) = destination {
                                            if app.state.prefs.switch_tab_after_drop {
                                                if let Some(group) = app.state.group_mut(group_id) {
                                                    if let Some(index) = group.tabs.iter().position(|tab| tab.id == tab_id) {
                                                        group.active = index;
                                                    }
                                                }
                                            }
                                            if app.state.prefs.rollup && app.state.group(group_id).is_some_and(|group| {
                                                group.rolled && group.tab().folder.is_none()
                                                    && paint::native_group_header(group).contains(input.pos.x, input.pos.y)
                                            }) {
                                                app.revealed = Some(group_id);
                                                app.hover_rolled = None;
                                                app.rollup_interact_until = input.time + 0.50;
                                            }
                                        }
                                        let mut restore_toast: Option<String> = None;
                                        if restoring {
                                            if let Some(pin_toast) = native_pending_pin_paths_toast.take() {
                                                if let Some(prev) = undo_before.clone() {
                                                    app.push_undo(prev);
                                                }
                                                if app.native_ai_pending.as_ref().is_some_and(|p| {
                                                    matches!(p.route, ai_tools::NativeAiToolRoute::PinFile { .. })
                                                }) {
                                                    ai_tools::complete_pending_if_saved(
                                                        &mut app,
                                                        true,
                                                        "Could not save workspace changes",
                                                    );
                                                }
                                                restore_toast = Some(pin_toast);
                                            } else if let Some(followup) = native_pending_layout_followup.take() {
                                                if !desktop::native_layout_structure_matches(&app.state, &followup.target) {
                                                    app.state = desktop::stage2_apply_target_structure_preserving_pins(
                                                        &app.state,
                                                        &followup.target,
                                                    );
                                                    app.revealed = None;
                                                    app.hover_rolled = None;
                                                    app.selected = None;
                                                    app.multi_selected.clear();
                                                    native_stage2_layout_followup = Some(followup);
                                                    restore_toast = Some(String::new());
                                                } else {
                                                    if followup.record_undo {
                                                        if let Some(prev) = followup.undo_snapshot.or_else(|| undo_before.clone()) {
                                                            app.push_undo(prev);
                                                        }
                                                    } else {
                                                        app.native_undo_inflight = followup.restore_undo_on_fail;
                                                    }
                                                    desktop::apply_non_pin_layout_from(&mut app.state, &followup.target);
                                                    app.revealed = None;
                                                    app.hover_rolled = None;
                                                    app.selected = None;
                                                    app.multi_selected.clear();
                                                    if app.native_ai_pending.as_ref().is_some_and(|p| {
                                                        p.route == ai_tools::NativeAiToolRoute::LayoutRestore
                                                    }) {
                                                        let needs_layout_ack = native_last_accepted_state
                                                            .as_ref()
                                                            .is_some_and(|acc| {
                                                                native_effective_sync_needed(
                                                                    acc,
                                                                    native_last_accepted_revealed,
                                                                    native_last_accepted_hidden,
                                                                    &app.state,
                                                                    app.revealed,
                                                                    app.hidden,
                                                                )
                                                            });
                                                        if needs_layout_ack {
                                                            if let Some(ref mut pending) =
                                                                app.native_ai_pending
                                                            {
                                                                pending.route = ai_tools::NativeAiToolRoute::WorkspaceSync;
                                                            }
                                                        } else {
                                                            let saved = if native_save_permitted {
                                                                if serde_json::to_vec(&app.state).ok()
                                                                    != native_disk_state.as_ref().and_then(|d| serde_json::to_vec(d).ok())
                                                                {
                                                                    persist_native_workspace(
                                                                        &mut app,
                                                                        &mut native_disk_state,
                                                                        &mut native_last_accepted_state,
                                                                        &mut last_saved_state,
                                                                        &mut last_seen_state,
                                                                        input.time,
                                                                    );
                                                                    app.save_ok
                                                                } else {
                                                                    true
                                                                }
                                                            } else {
                                                                false
                                                            };
                                                            if !ai_tools::complete_pending_if_saved(
                                                                &mut app,
                                                                saved,
                                                                "Could not save workspace changes",
                                                            ) {
                                                                app.save_allowed = false;
                                                                native_save_permitted = false;
                                                                must_stop = true;
                                                                if let Some(h) = app.host.as_mut() {
                                                                    h.quit = true;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    restore_toast = Some(followup.success_toast);
                                                }
                                            }
                                        } else if let Some(prev) = undo_before.clone() {
                                            app.push_undo(prev);
                                        }
                                        if sorting || restoring {
                                            app.clear_native_scroll_state();
                                        }
                                        app.sync_native_backdrop(
                                            app.host.is_some()
                                                && app.native_env.chrome_only
                                                && app.native_env.in_box_layout
                                                && !host::color_pick_shielded(),
                                            if restoring { native_last_accepted_state.as_ref() } else { None },
                                            app.host_lifted_for_popup(),
                                        );
                                        last_saved_state = native_disk_state
                                            .as_ref()
                                            .and_then(|d| serde_json::to_vec(d).ok())
                                            .unwrap_or_default();
                                        last_seen_state = serde_json::to_vec(&app.state)
                                            .unwrap_or_default();
                                        if last_seen_state != last_saved_state {
                                            last_edit_at = input.time;
                                        }
                                        let event = if sorting { "native_desktop.rules_sort_committed" }
                                            else if restoring { "native_desktop.layout_restore_committed" }
                                            else { "native_desktop.pin_drop_committed" };
                                        host::event("INFO",event,
                                            &format!("Native virtual Pin {:?} committed atomically ({} Workspace bytes); source Desktop file unchanged",
                                                drop.kind,bytes));
                                        let note = match drop.kind {
                                            native_pin_drop::NativeMultiDropKind::DesktopToBox { .. } =>
                                                "Explorer Pin added to the selected Box".into(),
                                            native_pin_drop::NativeMultiDropKind::BoxToBox { .. } =>
                                                "Explorer Pin moved to the selected Box".into(),
                                            native_pin_drop::NativeMultiDropKind::BoxToDesktop { .. } =>
                                                "Explorer Pin removed from Box to Desktop".into(),
                                            native_pin_drop::NativeMultiDropKind::SameBox { .. } =>
                                                "Explorer Pins reordered in Box".into(),
                                            native_pin_drop::NativeMultiDropKind::SortRules => {
                                                app.multi_selected.clear();
                                                app.native_rules_sort_requested = None;
                                                let dests = desktop::format_pin_destinations(&drop.before, &drop.next);
                                                desktop::format_rule_sort_toast(
                                                    drop.source_paths.len(),
                                                    native_last_sort_scope,
                                                    &dests,
                                                )
                                            }
                                            native_pin_drop::NativeMultiDropKind::RestoreLayout => {
                                                restore_toast.unwrap_or_else(|| "Layout restored.".into())
                                            }
                                        };
                                        if !note.is_empty() {
                                            app.notify(note,input.time);
                                        }
                                    }
                                    Err(reason)=>{
                                        native_pending_pin_paths_toast = None;
                                        if let Some(followup) = native_pending_layout_followup.take() {
                                            if let Some(undo_state) = followup.restore_undo_on_fail {
                                                app.push_undo(undo_state);
                                            }
                                        }
                                        host::event("WARN",
                                            "native_desktop.pin_drop_persistence_blocked",&reason);
                                        if let Some(ref accepted)=native_last_accepted_state {
                                            app.state=accepted.clone();
                                            app.revealed=native_last_accepted_revealed;
                                            app.hidden=native_last_accepted_hidden;
                                            app.native_visual_revealed=native_last_accepted_revealed;
                                        }
                                        app.save_allowed=false;
                                        native_save_permitted=false;
                                        if ai_tools::abort_pending(&mut app, reason.clone()) {
                                            native_ai_exit_rollback = true;
                                        }
                                        native_last_accepted_state = Some(app.state.clone());
                                        must_stop=true;
                                        if let Some(h)=app.host.as_mut(){h.quit=true;}
                                    }
                                }
                            }else{
                                app.native_undo_inflight = None;
                                if let Some(prev) = app.native_record_undo_on_sync.take() {
                                    app.push_undo(prev);
                                }
                                // Explorer already painted the final positions. Restore the
                                // chrome-only window region before this frame presents. Keep any
                                // queued ghost texture alive through its GPU submission.
                                if app.native_ghost.as_ref().is_some_and(|ghost| {
                                    let (eff_proposed, _) = native_effective_state(&proposed_real, proposed_revealed, proposed_hidden);
                                    ghost.dropped && ghost.restoring.is_none()
                                        && eff_proposed.group(ghost.group).is_some_and(|group| {
                                            !ghost_should_end_preview(Some((group.bounds, group.rolled)), ghost.origin)
                                        })
                                }) {
                                    retired_drag = app.retire_native_ghost();
                                }
                                native_last_accepted_revealed=proposed_revealed;
                                native_last_accepted_hidden=proposed_hidden;
                                app.native_visual_revealed=native_last_accepted_revealed;
                                app.sync_native_backdrop(
                                    app.host.is_some()
                                        && app.native_env.chrome_only
                                        && app.native_env.in_box_layout
                                        && !host::color_pick_shielded(),
                                    native_last_accepted_state.as_ref(),
                                    app.host_lifted_for_popup(),
                                );
                                native_last_accepted_state=Some(proposed_real);
                                let stage3_still_needed = native_stage2_layout_followup
                                    .as_ref()
                                    .is_some_and(|f| !desktop::native_regular_pins_match(&app.state, &f.target));
                                let ai_target_ok = if let Err(reason) = ai_tools::validate_workspace_sync_target(&app) {
                                    if ai_tools::abort_pending(&mut app, reason) {
                                        native_ai_exit_rollback = true;
                                    }
                                    app.save_allowed = false;
                                    native_save_permitted = false;
                                    native_last_accepted_state = Some(app.state.clone());
                                    must_stop = true;
                                    if let Some(h) = app.host.as_mut() {
                                        h.quit = true;
                                    }
                                    false
                                } else {
                                    true
                                };
                                let mut stage2_finished = false;
                                if let Some(followup) = native_stage2_layout_followup.take() {
                                    if desktop::native_regular_pins_match(&app.state, &followup.target) {
                                        app.native_undo_inflight = None;
                                        if followup.record_undo {
                                            if let Some(undo_state) = followup.undo_snapshot {
                                                app.push_undo(undo_state);
                                            }
                                        }
                                        desktop::apply_non_pin_layout_from(&mut app.state, &followup.target);
                                        stage2_finished = true;
                                        if !followup.success_toast.is_empty() {
                                            app.notify(followup.success_toast, input.time);
                                        }
                                    } else {
                                        app.native_layout_restore_requested = Some(followup);
                                    }
                                }
                                let mut ai_verified_write_done = false;
                                if ai_target_ok && native_save_permitted && !stage3_still_needed {
                                    let saved_matches = native_disk_state.as_ref().is_some_and(|disk| {
                                        serde_json::to_vec(disk).ok() == serde_json::to_vec(&app.state).ok()
                                    });
                                    let worker_sync_matches = native_last_accepted_state.as_ref().is_some_and(|acc| {
                                        !native_effective_sync_needed(
                                            acc,
                                            native_last_accepted_revealed,
                                            native_last_accepted_hidden,
                                            &app.state,
                                            app.revealed,
                                            app.hidden,
                                        )
                                    });
                                    let ai_needs_verified_save =
                                        app.native_ai_pending.as_ref().is_some_and(|p| {
                                            p.route == ai_tools::NativeAiToolRoute::WorkspaceSync
                                        });
                                    if stage2_finished
                                        || ((ai_needs_verified_save || !saved_matches)
                                            && worker_sync_matches)
                                    {
                                        persist_native_workspace(
                                            &mut app,
                                            &mut native_disk_state,
                                            &mut native_last_accepted_state,
                                            &mut last_saved_state,
                                            &mut last_seen_state,
                                            input.time,
                                        );
                                        ai_verified_write_done = app.save_ok;
                                    }
                                }
                                if app.native_ai_pending.as_ref().is_some_and(|p| {
                                    p.route == ai_tools::NativeAiToolRoute::WorkspaceSync
                                        || (stage2_finished
                                            && p.route == ai_tools::NativeAiToolRoute::LayoutRestore)
                                }) {
                                    let saved = native_save_permitted && ai_verified_write_done;
                                    if !ai_tools::complete_pending_if_saved(
                                        &mut app,
                                        saved,
                                        "Could not save workspace changes",
                                    ) {
                                        app.save_allowed = false;
                                        native_save_permitted = false;
                                        native_ai_exit_rollback = true;
                                        native_last_accepted_state = Some(app.state.clone());
                                        must_stop = true;
                                        if let Some(h) = app.host.as_mut() {
                                            h.quit = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(NativeWorkspaceReply::Pending)=>{
                        // A new click must not replace a command whose native
                        // Explorer positions have not yet been acknowledged.
                        // Harmless Portal/cosmetic state may continue locally.
                        let baseline = proposed
                            .as_ref()
                            .map(|(st, rev, hid)| (st, *rev, *hid))
                            .or_else(|| {
                                native_last_accepted_state
                                    .as_ref()
                                    .map(|st| (st, native_last_accepted_revealed, native_last_accepted_hidden))
                            });
                        if let Some((base_state, base_rev, base_hidden)) = baseline {
                            if native_effective_sync_needed(
                                base_state,
                                base_rev,
                                base_hidden,
                                &app.state,
                                app.revealed,
                                app.hidden,
                            ) {
                                app.state = base_state.clone();
                                app.clear_orphaned_rename_dialog();
                                app.revealed = base_rev;
                                app.hidden = base_hidden;
                                if let Some(ref before_undo) = native_undo_before_frame {
                                    app.undo = before_undo.clone();
                                }
                            }
                        }
                        native_pending_ui=Some((ticket,proposed));
                    }
                    Ok(NativeWorkspaceReply::Rejected(reason)) => {
                        let rejected_restore = native_pending_drop_commit
                            .take()
                            .is_some_and(|drop| {
                                matches!(
                                    &drop.kind,
                                    native_pin_drop::NativeMultiDropKind::RestoreLayout
                                )
                            });
                        if rejected_restore {
                            native_pending_pin_paths_toast = None;
                            if let Some(followup) = native_pending_layout_followup.take() {
                                if let Some(undo_state) = followup.restore_undo_on_fail {
                                    app.push_undo(undo_state);
                                }
                            }
                        }
                        if let Some(followup) = native_stage2_layout_followup.take() {
                            if let Some(undo_state) = followup.restore_undo_on_fail {
                                app.push_undo(undo_state);
                            }
                        }
                        app.native_record_undo_on_sync = None;
                        if let Some(undo_state) = app.native_undo_inflight.take() {
                            app.push_undo(undo_state);
                        }
                        if let Some(ref accepted) = native_last_accepted_state {
                            app.state = accepted.clone();
                            app.revealed = native_last_accepted_revealed;
                            app.hidden = native_last_accepted_hidden;
                            app.native_visual_revealed = native_last_accepted_revealed;
                            app.clear_orphaned_rename_dialog();
                        }
                        app.clear_native_scroll_state();
                        let explorer_changed = ai_tools::pending_explorer_diverged(
                            &app,
                            native_last_accepted_state.as_ref(),
                        );
                        if ai_tools::abort_pending(&mut app, reason.clone()) || explorer_changed {
                            native_ai_exit_rollback = true;
                            native_last_accepted_state = Some(app.state.clone());
                            must_stop = true;
                            if let Some(h) = app.host.as_mut() {
                                h.quit = true;
                            }
                        }
                        if reason.contains("verified native icon drop is waiting") {
                            if app.revealed.is_some() {
                                app.rollup_interact_until = input.time + 0.60;
                            }
                            host::event(
                                "INFO",
                                "native_desktop.workspace_deferred_for_drop",
                                &reason,
                            );
                        } else {
                            host::event("INFO", "native_desktop.workspace_refused", &reason);
                            let notice = if reason.contains("overlaps") {
                                "Box overlaps a desktop icon — drop on empty space"
                            } else {
                                "Native Box change refused. The previous layout was restored."
                            };
                            app.notify(notice, input.time);
                        }
                    }
                    Err(reason)=>{
                        native_pending_drop_commit=None;
                        native_pending_sort_followup = false;
                        native_pending_pin_paths_toast = None;
                        app.native_record_undo_on_sync = None;
                        if let Some(undo_state) = app.native_undo_inflight.take() {
                            app.push_undo(undo_state);
                        }
                        if let Some(followup) = native_pending_layout_followup.take() {
                            if let Some(undo_state) = followup.restore_undo_on_fail {
                                app.push_undo(undo_state);
                            }
                        }
                        if let Some(followup) = native_stage2_layout_followup.take() {
                            if let Some(undo_state) = followup.restore_undo_on_fail {
                                app.push_undo(undo_state);
                            }
                        }
                        if let Some(ref accepted)=native_last_accepted_state {
                            app.state=accepted.clone();
                            app.revealed=native_last_accepted_revealed;
                            app.hidden=native_last_accepted_hidden;
                            app.native_visual_revealed=native_last_accepted_revealed;
                        }
                        app.clear_native_scroll_state();
                        if app.native_ai_pending.is_some() {
                            let _ = ai_tools::abort_pending_checked(&mut app, reason.clone());
                            native_ai_exit_rollback = true;
                            native_last_accepted_state = Some(app.state.clone());
                        }
                        host::event("WARN","native_desktop.ack_failed",&reason);
                        app.notify("Explorer did not confirm the change; Native mode stopping safely",input.time);
                        // Never keep the real UI running with possibly
                        // divergent icon positions after an expired ACK.
                        must_stop=true;
                        if let Some(h)=app.host.as_mut(){h.quit=true;}
                    }
                }
            }
            } // end if !must_stop (reply polling)
            if !must_stop
                && native_pending_ui.is_none()
                && native_rules_sort_reply.is_none()
                && native_layout_restore_reply.is_none()
                && native_pending_layout_followup.is_none()
                && native_pin_paths_reply.is_none()
                && native_pending_pin_paths_toast.is_none()
                && !native_pending_sort_followup
            {
                if let Some(ref mut accepted)=native_last_accepted_state {
                    let (accepted_eff, accepted_overrides) =
                        native_effective_state(accepted, native_last_accepted_revealed, native_last_accepted_hidden);
                    let (current_eff, rolled_overrides) =
                        native_effective_state(&app.state, app.revealed, app.hidden);
                    let ai_workspace_sync_pending = app.native_ai_pending.as_ref().is_some_and(|p| {
                        p.route == ai_tools::NativeAiToolRoute::WorkspaceSync
                    });
                    let changed = native_desktop_requires_worker_sync(&accepted_eff, &current_eff)
                        || accepted_overrides != rolled_overrides
                        || ai_workspace_sync_pending;
                    let mut leaving_portal = false;
                    let mut outgoing_portal_drawn = false;
                    if changed && app.drag.is_none() && accepted_eff.page == current_eff.page {
                        for group in current_eff.groups.iter().filter(|g| {
                            g.page == current_eff.page && g.tab().folder.is_none()
                        }) {
                            if accepted_eff.group(group.id).is_some_and(|g| {
                                g.page == group.page && g.tab().folder.is_some()
                            }) {
                                leaving_portal = true;
                                outgoing_portal_drawn |= drawn_portals.contains(&group.id);
                            }
                        }
                    }
                    if !must_stop && changed && app.drag.is_none() && !outgoing_portal_drawn {
                        let tab_request = (accepted_eff.page == current_eff.page).then(|| {
                            current_eff.groups.iter().find(|g| {
                                g.page == current_eff.page && !g.rolled
                                    && accepted_eff.group(g.id).is_some_and(|old| old.active != g.active)
                            }).map(|group| (group.id, group.active))
                        }).flatten();
                        let current_eff = current_eff.into_owned();
                        if leaving_portal {
                            // Remove the outgoing Portal's pixels and hit area
                            // before the worker can expose the incoming icons.
                            app.host_after_frame(&p);
                            native_region_applied = true;
                            must_stop = app.host.as_ref().is_some_and(|h| h.quit);
                        }
                        app.native_scroll_target=None;
                        let proposed_real=app.state.clone();
                        let proposed_revealed=app.revealed;
                        let proposed_hidden=app.hidden;
                        let result=if must_stop { Err("The desktop overlay is closing".into()) } else { native_two_box_worker.as_ref()
                            .ok_or_else(||"Native Explorer Worker disconnected".to_string())
                            .and_then(|worker|worker.begin_workspace_sync_with_rollup_overrides(
                                current_eff, rolled_overrides)) };
                        match result {
                            Ok(ticket)=>{
                                if let Some((group, tab)) = tab_request {
                                    native_tab_trace = Some((group, std::time::Instant::now()));
                                    host::event("INFO", "native_desktop.tab_requested", &format!(
                                        "group={group} selected_tab={tab}"));
                                }
                                native_pending_ui=Some((ticket,Some((proposed_real, proposed_revealed, proposed_hidden))));
                            }
                            Err(reason)=>{
                                if let Some(followup) = native_stage2_layout_followup.take() {
                                    if let Some(undo_state) = followup.restore_undo_on_fail {
                                        app.push_undo(undo_state);
                                    }
                                }
                                app.native_record_undo_on_sync = None;
                                if let Some(undo_state) = app.native_undo_inflight.take() {
                                    app.push_undo(undo_state);
                                }
                                app.state=accepted.clone();
                                app.revealed=native_last_accepted_revealed;
                                app.hidden=native_last_accepted_hidden;
                                app.native_visual_revealed=native_last_accepted_revealed;
                                app.clear_orphaned_rename_dialog();
                                if ai_tools::abort_pending(&mut app, reason.clone()) {
                                    native_ai_exit_rollback = true;
                                    *accepted = app.state.clone();
                                    must_stop = true;
                                    if let Some(h) = app.host.as_mut() {
                                        h.quit = true;
                                    }
                                }
                                host::event("WARN","native_desktop.workspace_refused",&reason);
                                app.notify("Native Box edit refused; previous layout preserved",input.time);
                                if native_two_box_worker.as_ref()
                                    .is_none_or(|worker|!worker.is_ready()) {
                                    must_stop=true;
                                    if let Some(h)=app.host.as_mut(){h.quit=true;}
                                }
                            }
                        }
                    }else if !changed{
                        app.native_undo_inflight = None;
                        if let Some(prev) = app.native_record_undo_on_sync.take() {
                            app.push_undo(prev);
                        }
                        // A Portal refresh or cosmetic update must never
                        // trigger expensive IFolderView repositioning.
                        if *accepted != app.state {
                            accepted.clone_from(&app.state);
                        }
                        native_last_accepted_revealed=app.revealed;
                        native_last_accepted_hidden=app.hidden;
                        app.native_visual_revealed=native_last_accepted_revealed;
                        let mut stage2_finished = false;
                        if let Some(followup) = native_stage2_layout_followup.take() {
                            if desktop::native_regular_pins_match(&app.state, &followup.target) {
                                if followup.record_undo {
                                    if let Some(undo_state) = followup.undo_snapshot {
                                        app.push_undo(undo_state);
                                    }
                                }
                                desktop::apply_non_pin_layout_from(&mut app.state, &followup.target);
                                stage2_finished = true;
                                if !followup.success_toast.is_empty() {
                                    app.notify(followup.success_toast, input.time);
                                }
                            } else {
                                app.native_layout_restore_requested = Some(followup);
                            }
                        }
                        if stage2_finished
                            && app.native_ai_pending.as_ref().is_some_and(|p| {
                                p.route == ai_tools::NativeAiToolRoute::LayoutRestore
                            })
                        {
                            let mut saved = false;
                            if native_save_permitted {
                                app.remember_layout();
                                let save_res = ai_tools::expected_ai_disk_bytes(&app)
                                    .map(|b| b.to_vec())
                                    .ok_or_else(|| "Missing baseline workspace bytes for AI save".to_string())
                                    .and_then(|expected| {
                                        app.state.save_returning_bytes(&app.save_path, &expected)
                                    });
                                match save_res {
                                    Ok((predecessor, written_bytes)) => {
                                        app.save_ok = true;
                                        ai_tools::record_ai_disk_bytes(
                                            &mut app,
                                            predecessor,
                                            written_bytes,
                                        );
                                        native_disk_state = State::load(&app.save_path)
                                            .ok()
                                            .or_else(|| Some(app.state.clone()));
                                        last_saved_state =
                                            serde_json::to_vec(&app.state).unwrap_or_default();
                                        last_seen_state = last_saved_state.clone();
                                        *accepted = app.state.clone();
                                        saved = true;
                                        host::event(
                                            "INFO",
                                            "native_desktop.workspace_saved",
                                            "Saved acknowledged Native Desktop workspace layout",
                                        );
                                    }
                                    Err(e) => {
                                        app.save_ok = false;
                                        host::event(
                                            "WARN",
                                            "native_desktop.workspace_save_failed",
                                            &e.to_string(),
                                        );
                                        app.notify(format!("Could not save changes: {e}"), input.time);
                                    }
                                }
                            }
                            if !ai_tools::complete_pending_if_saved(
                                &mut app,
                                saved,
                                "Could not save workspace changes",
                            ) {
                                app.save_allowed = false;
                                native_save_permitted = false;
                                must_stop = true;
                                if let Some(h) = app.host.as_mut() {
                                    h.quit = true;
                                }
                            }
                        }
                    }
                }
                if !must_stop
                    && native_pending_ui.is_none()
                    && app.native_ghost.is_none()
                    && native_stage2_layout_followup.is_none()
                {
                    if native_layout_restore_reply.is_none()
                        && native_pending_layout_followup.is_none()
                        && native_pin_paths_reply.is_none()
                        && native_pending_pin_paths_toast.is_none()
                        && native_rules_sort_reply.is_none()
                        && !native_pending_sort_followup
                        && app.native_rules_sort_requested.is_some_and(|_| {
                            app.drag.is_none()
                                && native_last_accepted_state.as_ref().is_some_and(|accepted| {
                                    !native_effective_sync_needed(accepted, native_last_accepted_revealed, native_last_accepted_hidden,
                                        &app.state, app.revealed, app.hidden)
                                })
                        })
                    {
                        if let Some(scope) = app.native_rules_sort_requested.take() {
                            if ensure_native_disk_matches(
                                native_save_permitted,
                                &mut app,
                                &mut native_disk_state,
                                &mut native_last_accepted_state,
                                &mut last_saved_state,
                                &mut last_seen_state,
                                input.time,
                            ) {
                                let include_existing_pins = scope == desktop::RuleSortScope::AllIncludingPins;
                                let request = native_two_box_worker.as_ref()
                                    .ok_or_else(|| "Windows Explorer is unavailable".to_string())
                                    .and_then(|worker| worker.request_rule_sort(
                                        app.state.rules.clone(), app.state.next_id, include_existing_pins));
                                match request {
                                    Ok(ticket) => {
                                        native_last_sort_scope = scope;
                                        native_rules_sort_reply = Some((ticket, scope));
                                    }
                                    Err(reason) => app.notify(format!("Could not sort icons: {reason}"), input.time),
                                }
                            } else {
                                app.notify("Could not sort icons because the Workspace could not be saved.", input.time);
                            }
                        }
                    }
                    if native_rules_sort_reply.is_none()
                        && !native_pending_sort_followup
                        && native_layout_restore_reply.is_none()
                        && native_pending_layout_followup.is_none()
                        && native_pin_paths_reply.is_none()
                        && native_pending_pin_paths_toast.is_none()
                        && app.native_pin_paths_requested.as_ref().is_some_and(|_| {
                            app.drag.is_none()
                                && native_last_accepted_state.as_ref().is_some_and(|accepted| {
                                    !native_effective_sync_needed(
                                        accepted,
                                        native_last_accepted_revealed,
                                        native_last_accepted_hidden,
                                        &app.state,
                                        app.revealed,
                                        app.hidden,
                                    )
                                })
                        })
                    {
                        if let Some((group_id, tab_id, paths, toast_template)) =
                            app.native_pin_paths_requested.take()
                        {
                            if ensure_native_disk_matches(
                                native_save_permitted,
                                &mut app,
                                &mut native_disk_state,
                                &mut native_last_accepted_state,
                                &mut last_saved_state,
                                &mut last_seen_state,
                                input.time,
                            ) {
                                let request = native_two_box_worker
                                    .as_ref()
                                    .ok_or_else(|| "Windows Explorer is unavailable".to_string())
                                    .and_then(|worker| {
                                        worker.request_pin_paths(
                                            group_id,
                                            tab_id,
                                            paths,
                                            app.state.next_id,
                                        )
                                    });
                                match request {
                                    Ok(ticket) => {
                                        native_pin_paths_reply = Some((ticket, toast_template));
                                    }
                                    Err(reason) => {
                                        if ai_tools::abort_pending(&mut app, reason.clone()) {
                                            native_ai_exit_rollback = true;
                                            native_last_accepted_state = Some(app.state.clone());
                                            must_stop = true;
                                            if let Some(h) = app.host.as_mut() {
                                                h.quit = true;
                                            }
                                        }
                                        app.notify(reason, input.time);
                                    }
                                }
                            } else {
                                if ai_tools::abort_pending(
                                    &mut app,
                                    "Could not pin items because the Workspace could not be saved.",
                                ) {
                                    native_ai_exit_rollback = true;
                                    native_last_accepted_state = Some(app.state.clone());
                                    must_stop = true;
                                    if let Some(h) = app.host.as_mut() {
                                        h.quit = true;
                                    }
                                }
                                app.notify(
                                    "Could not pin items because the Workspace could not be saved.",
                                    input.time,
                                );
                            }
                        }
                    }
                    if native_rules_sort_reply.is_none()
                        && !native_pending_sort_followup
                        && native_pin_paths_reply.is_none()
                        && native_pending_pin_paths_toast.is_none()
                        && app.native_layout_restore_requested.as_ref().is_some_and(|_| {
                            app.drag.is_none()
                                && native_last_accepted_state.as_ref().is_some_and(|accepted| {
                                    !native_effective_sync_needed(accepted, native_last_accepted_revealed, native_last_accepted_hidden,
                                        &app.state, app.revealed, app.hidden)
                                })
                        })
                    {
                        if let Some(req) = app.native_layout_restore_requested.take() {
                            if desktop::native_layout_structure_matches(&app.state, &req.target) {
                                if ensure_native_disk_matches(
                                    native_save_permitted,
                                    &mut app,
                                    &mut native_disk_state,
                                    &mut native_last_accepted_state,
                                    &mut last_saved_state,
                                    &mut last_seen_state,
                                    input.time,
                                ) {
                                    let request = native_two_box_worker.as_ref()
                                        .ok_or_else(|| "Windows Explorer is unavailable".to_string())
                                        .and_then(|worker| worker.request_layout_restore(
                                            req.target.clone(), app.state.next_id));
                                    match request {
                                        Ok(ticket) => native_layout_restore_reply = Some((ticket, req)),
                                        Err(reason) => {
                                            if let Some(undo_state) = req.restore_undo_on_fail {
                                                app.push_undo(undo_state);
                                            }
                                            let explorer_changed = app.native_ai_pending.as_ref().is_some_and(|p| {
                                                native_last_accepted_state.as_ref() != Some(&p.state_before)
                                            });
                                            if ai_tools::abort_pending(&mut app, reason.clone()) || explorer_changed {
                                                native_ai_exit_rollback = true;
                                                native_last_accepted_state = Some(app.state.clone());
                                                must_stop = true;
                                                if let Some(h) = app.host.as_mut() {
                                                    h.quit = true;
                                                }
                                            }
                                            app.notify(format!("Could not restore layout: {reason}"), input.time);
                                        }
                                    }
                                } else {
                                    if let Some(undo_state) = req.restore_undo_on_fail {
                                        app.push_undo(undo_state);
                                    }
                                    let explorer_changed = app.native_ai_pending.as_ref().is_some_and(|p| {
                                        native_last_accepted_state.as_ref() != Some(&p.state_before)
                                    });
                                    if ai_tools::abort_pending(
                                        &mut app,
                                        "Could not restore layout because the Workspace could not be saved.",
                                    ) || explorer_changed {
                                        native_ai_exit_rollback = true;
                                        native_last_accepted_state = Some(app.state.clone());
                                        must_stop = true;
                                        if let Some(h) = app.host.as_mut() {
                                            h.quit = true;
                                        }
                                    }
                                    app.notify("Could not restore layout because the Workspace could not be saved.", input.time);
                                }
                            } else if let Some(stage1) =
                                desktop::stage1_unpin_incompatible_tabs(&app.state, &req.target)
                            {
                                if ensure_native_disk_matches(
                                    native_save_permitted,
                                    &mut app,
                                    &mut native_disk_state,
                                    &mut native_last_accepted_state,
                                    &mut last_saved_state,
                                    &mut last_seen_state,
                                    input.time,
                                ) {
                                    let request = native_two_box_worker
                                        .as_ref()
                                        .ok_or_else(|| "Windows Explorer is unavailable".to_string())
                                        .and_then(|worker| {
                                            worker.request_layout_restore(stage1, app.state.next_id)
                                        });
                                    match request {
                                        Ok(ticket) => {
                                            native_layout_restore_reply = Some((ticket, req));
                                        }
                                        Err(reason) => {
                                            if let Some(undo_state) = req.restore_undo_on_fail {
                                                app.push_undo(undo_state);
                                            }
                                            let explorer_changed = ai_tools::pending_explorer_diverged(
                                                &app,
                                                native_last_accepted_state.as_ref(),
                                            );
                                            if ai_tools::abort_pending(&mut app, reason.clone()) || explorer_changed {
                                                native_ai_exit_rollback = true;
                                                native_last_accepted_state = Some(app.state.clone());
                                                must_stop = true;
                                                if let Some(h) = app.host.as_mut() {
                                                    h.quit = true;
                                                }
                                            }
                                            app.notify(
                                                format!("Could not restore layout: {reason}"),
                                                input.time,
                                            );
                                        }
                                    }
                                } else {
                                    if let Some(undo_state) = req.restore_undo_on_fail {
                                        app.push_undo(undo_state);
                                    }
                                    let explorer_changed = ai_tools::pending_explorer_diverged(
                                        &app,
                                        native_last_accepted_state.as_ref(),
                                    );
                                    if ai_tools::abort_pending(
                                        &mut app,
                                        "Could not restore layout because the Workspace could not be saved.",
                                    ) || explorer_changed {
                                        native_ai_exit_rollback = true;
                                        native_last_accepted_state = Some(app.state.clone());
                                        must_stop = true;
                                        if let Some(h) = app.host.as_mut() {
                                            h.quit = true;
                                        }
                                    }
                                    app.notify("Could not restore layout because the Workspace could not be saved.", input.time);
                                }
                            } else {
                                app.state = desktop::stage2_apply_target_structure_preserving_pins(
                                    &app.state,
                                    &req.target,
                                );
                                app.revealed = None;
                                app.hover_rolled = None;
                                app.selected = None;
                                app.multi_selected.clear();
                                app.clear_native_scroll_state();
                                native_stage2_layout_followup = Some(req);
                            }
                        }
                    }
                    let page_change_allowed = native_page_change_allowed(
                        native_rules_sort_reply.is_some(),
                        native_pending_sort_followup,
                        native_layout_restore_reply.is_some(),
                        native_pending_layout_followup.is_some(),
                        app.native_undo_inflight.is_some(),
                        app.native_record_undo_on_sync.is_some(),
                        app.drag.is_some(),
                        native_last_accepted_state
                            .as_ref()
                            .map(|acc| (acc, native_last_accepted_revealed, native_last_accepted_hidden)),
                        (&app.state, app.revealed, app.hidden),
                    ) && app.native_rules_sort_requested.is_none()
                        && app.native_layout_restore_requested.is_none()
                        && app.native_pin_paths_requested.is_none()
                        && native_pin_paths_reply.is_none()
                        && native_pending_pin_paths_toast.is_none()
                        && native_stage2_layout_followup.is_none();
                    if page_change_allowed {
                        app.advance_native_scroll_target();
                    }
                    if let Some((group_id, delta)) =
                        app.native_page_request.take_if(|_| page_change_allowed)
                    {
                        let result=app.state.group(group_id)
                            .ok_or_else(||"Native Box missing".to_string())
                            .and_then(|group|native_two_box_worker.as_ref()
                                .ok_or_else(||"Native Explorer page Worker disconnected".to_string())
                                .and_then(|worker|worker.begin_page_change(
                                    group_id,group.active,delta)));
                        match result {
                            Ok(ticket)=>native_pending_ui=Some((ticket,None)),
                            Err(reason)=>{
                                app.native_scroll_target=None;
                                host::event("WARN","native_desktop.viewport_refused",&reason);
                                app.notify("Native icon page unavailable; current page preserved",input.time);
                            }
                        }
                    }
                }
            }
            // In-flight commands are serialized. A row-scroll request waits
            // (wheel notches keep accumulating into it) and is sent after the
            // current reflow is acknowledged, never as a concurrent reflow.
        }
        #[cfg(windows)]
        if native_two_box_ui && !native_desktop_mode {
            if !native_general_workspace && app.state.page!=native_desktop_page_before{
                app.state.page=native_desktop_page_before;
                app.notify("Desktop Page changes are unavailable in the two-Box diagnostic",input.time);
            }
            if native_general_workspace && app.state.page!=native_desktop_page_before{
                let result=native_two_box_worker.as_ref()
                    .ok_or_else(||"Native Explorer Workspace worker disconnected".to_string())
                    .and_then(|worker|worker.select_desktop_page(app.state.page));
                match result {
                    Ok(())=>{}
                    Err(reason)=>{
                        app.state.page=native_desktop_page_before;
                        host::event("WARN","native_workspace.page_refused",&reason);
                        app.notify("Native Desktop Page rejected; previous Explorer view preserved",input.time);
                    }
                }
            }
            for (group_id,previous) in native_two_box_before {
                let current=app.state.group(group_id).map(|group|group.active);
                if let Some(current)=current.filter(|current|*current!=previous) {
                    let result=native_two_box_worker.as_ref()
                        .ok_or_else(||"Native Explorer two-Box worker disconnected".to_string())
                        .and_then(|worker|worker.selected(group_id,current));
                    if let Err(reason)=result{
                        if let Some(group)=app.state.group_mut(group_id){group.active=previous;}
                        host::event("WARN","native_two_box.ui_refused",&reason);
                        app.notify("Native Tab rejected; keeping previous Explorer view",input.time);
                    }
                }
            }
            if let Some((group_id,delta))=app.native_page_request.take(){
                let tab=app.state.group(group_id).map(|g|g.active);
                let result=tab.ok_or_else(||"Native Box missing".to_string())
                    .and_then(|active|native_two_box_worker.as_ref()
                        .ok_or_else(||"Native Explorer viewport worker disconnected".to_string())
                        .and_then(|worker|worker.change_page(group_id,active,delta)));
                match result{
                    Ok(_)=>{}
                    Err(reason)=>{
                        host::event("WARN","native_inbox.page_refused",&reason);
                        app.notify("Native page rejected; keeping previous icon view",input.time);
                    }
                }
            }
        }
        #[cfg(windows)]
        if native_chrome_preview && !native_two_box_ui {
            if let (Some(previous),Some(current))=
                (native_active_before,app.state.group(native_target_group).map(|g|g.active)){
                if previous!=current {
                    if current!=native_from_tab && current!=native_target_tab {
                        if let Some(g)=app.state.group_mut(native_target_group){g.active=previous;}
                        app.notify("Native preview accepts only the verified Tab pair",input.time);
                    } else if !native_tab_worker.as_ref()
                        .is_some_and(|worker|worker.selected(current)){
                        app.notify("Native Explorer filter disconnected; icons remain visible",input.time);
                    }
                }
            }
        }
        #[cfg(all(windows, feature = "native-lab"))]
        if !automation && !native_chrome_preview {
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
            if !native_region_applied {
                app.host_after_frame(&p);
            }
            #[cfg(all(windows,feature="native-lab",feature="native-unsafe-drag-lab"))]
            if native_chrome_gesture_lab && native_gesture_worker.is_none()
                && native_original_gesture_outcome.is_none()
                && frame>=3 && frame%4==0 {
                // Paint + SetWindowRgn MUST precede the Explorer hook. A
                // ready-file from the native Worker is published only after
                // WindowFromPoint at a REAL source icon resolves to Explorer.
                // No physical MouseDown is permitted before this barrier.
                match native_original_gui_source_is_explorer_exposed(
                    &app,native_target_group) {
                    Ok(true)=>{
                        let dll=native_gesture_dll.clone().unwrap_or_default();
                        let mut worker=native_filter_host::NativeGestureBackgroundService::start(
                            app.save_path.clone(),dll,native_chrome_live_transfer);
                        if worker.await_ready(std::time::Duration::from_secs(6)){
                            host::event("INFO","native_chrome_gesture.explorer_exposed",
                                "Native source is Explorer-owned after UI clipping; hook attached");
                            native_gesture_worker=Some(worker);
                        }else{
                            let report=worker.shutdown();
                            native_original_gesture_worker_report=Some(report);
                            app.save_allowed=false;
                            native_original_gesture_outcome=Some(serde_json::json!({
                                "validated":false,
                                "reason":"Original Main GUI already clipped but native Explorer hook did not attach",
                                "normal_saves_suspended":true
                            }));
                        }
                    },
                    Ok(false)=>{
                        if native_chrome_started.elapsed()>std::time::Duration::from_secs(12){
                            app.save_allowed=false;
                            native_original_gesture_outcome=Some(serde_json::json!({
                                "validated":false,
                                "reason":"Original Main GUI region did not expose the genuine Explorer source before deadline",
                                "normal_saves_suspended":true
                            }));
                        }
                    },
                    Err(reason)=>{
                        app.save_allowed=false;
                        native_original_gesture_outcome=Some(serde_json::json!({
                            "validated":false,"reason":reason,
                            "normal_saves_suspended":true
                        }));
                    }
                }
            }
            app.run_pending(input.time, &p);
            if app.host.as_ref().is_some_and(|h| h.quit) {
                // Host commands (including --command exit) reach this branch
                // before is_quit_requested(). Explicitly detach the native
                // Explorer hook and capture its report on this path as well.
                #[cfg(windows)]
                if let Some(mut native) = native_two_box_worker.take() {
                    let report = native.shutdown();
                    if let Some(path) = option("--native-two-box-ui-report") {
                        let _ = std::fs::write(
                            path,
                            serde_json::to_vec_pretty(&report).unwrap_or_default(),
                        );
                    }
                }
                #[cfg(windows)]
                if let Some(mut native)=native_tab_worker.take() {
                    let report=native.shutdown();
                    if let Some(i)=args.iter().position(|a|a=="--native-chrome-report") {
                        if let Some(path)=args.get(i+1) {
                            let _=std::fs::write(path,
                                serde_json::to_vec_pretty(&report).unwrap_or_default());
                        }
                    }
                    host::event("INFO","native_chrome.host_exit_detach",
                        if report["callback_detached"]==true{"native filter detached"}
                        else{"native filter failed-open or was not attached"});
                }
                #[cfg(windows)]
                {
                    if app.native_ai_pending.is_some() {
                        let _ = ai_tools::abort_pending_checked(
                            &mut app,
                            "DesktopCraft exited before Explorer confirmed the change",
                        );
                        native_last_accepted_state = Some(app.state.clone());
                        native_ai_exit_rollback = true;
                    } else if native_desktop_mode && !native_ai_exit_rollback {
                        if let Some(ref accepted) = native_last_accepted_state {
                            if native_pending_ui.is_some()
                                || native_effective_sync_needed(
                                    accepted,
                                    native_last_accepted_revealed,
                                    native_last_accepted_hidden,
                                    &app.state,
                                    app.revealed,
                                    app.hidden,
                                )
                            {
                                app.state = accepted.clone();
                            }
                        }
                    }
                    app.settings_window = None;
                    host::shutdown_mouse_hooks();
                }
                app.host_exit();
                #[cfg(windows)]
                if native_desktop_mode && native_save_permitted && !native_ai_exit_rollback {
                    let saved_matches = native_disk_state.as_ref().is_some_and(|disk| {
                        serde_json::to_vec(disk).ok() == serde_json::to_vec(&app.state).ok()
                    });
                    if !saved_matches {
                        persist_native_workspace(
                            &mut app,
                            &mut native_disk_state,
                            &mut native_last_accepted_state,
                            &mut last_saved_state,
                            &mut last_seen_state,
                            input.time,
                        );
                    }
                }
                app.save(input.time);
                break;
            }
        }
        #[cfg(windows)]
        let block_classic_menu = native_desktop_mode
            && (native_pin_tx_waiting || app.native_tx_inflight);
        #[cfg(not(windows))]
        let block_classic_menu = false;
        if block_classic_menu {
            app.classic_file_menu = None;
        } else {
            app.run_classic_file_menu(input.time);
        }
        if !automation {
            let had_interaction = input.pressed
                || input.released
                || input.right_pressed
                || input.right_released
                || input.middle_pressed
                || input.middle_released
                || input.scroll != 0.
                || !input.keys.is_empty()
                || !input.repeats.is_empty()
                || !input.typed.is_empty()
                || app.drag.is_some()
                || app.dialog.is_some()
                || last_seen_state != last_saved_state;
            let due = (had_interaction && input.time >= next_state_check_at)
                || input.released
                || input.right_released
                || input.key(KeyCode::Enter)
                || input.time >= next_idle_state_check_at;
            if due {
                let current = serde_json::to_vec(&app.state).unwrap_or_default();
                if current != last_seen_state {
                    last_seen_state = current;
                    last_edit_at = input.time;
                }
                next_state_check_at = input.time + 0.25;
                next_idle_state_check_at = input.time + 4.0;
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
            #[cfg(windows)]
            if native_desktop_mode
                && native_save_permitted
                && app.native_ai_pending.is_none()
                && native_pending_ui.is_none()
                && native_rules_sort_reply.is_none()
                && native_layout_restore_reply.is_none()
                && native_pending_layout_followup.is_none()
                && native_stage2_layout_followup.is_none()
                && native_pin_paths_reply.is_none()
                && native_pending_pin_paths_toast.is_none()
                && !native_pending_sort_followup
                && native_pending_drop_commit.is_none()
                && app.drag.is_none()
                && last_seen_state != last_saved_state
                && input.time - last_edit_at >= 0.4
                && (!app.state.prefs.desktop_enabled
                    || native_last_accepted_state.as_ref().is_some_and(|acc| {
                        !native_effective_sync_needed(
                            acc,
                            native_last_accepted_revealed,
                            native_last_accepted_hidden,
                            &app.state,
                            app.revealed,
                            app.hidden,
                        )
                    }))
            {
                persist_native_workspace(
                    &mut app,
                    &mut native_disk_state,
                    &mut native_last_accepted_state,
                    &mut last_saved_state,
                    &mut last_seen_state,
                    input.time,
                );
                if !app.save_ok {
                    last_edit_at = input.time;
                }
            }
        }
        if test.is_some() {
            if let Some(h) = harness.as_mut() {
                h.observe(frame, &mut app);
            }
        }
        #[cfg(all(windows,feature="native-lab",feature="native-unsafe-drag-lab"))]
        if native_chrome_gesture_lab &&
            (native_original_gesture_outcome.is_some() && frame>=35
                || native_chrome_started.elapsed()>std::time::Duration::from_secs(30)) {
            if native_original_gesture_worker_report.is_none(){
                if let Some(worker)=native_gesture_worker.as_mut(){
                    native_original_gesture_worker_report=Some(worker.shutdown());
                }
            }
            let gesture=native_original_gesture_outcome.take()
                .unwrap_or_else(||serde_json::json!({
                    "validated":false,
                    "reason":"No real Explorer native physical mouse release was delivered before deadline"
                }));
            let disk=State::load(&app.save_path);
            let same=disk.ok().is_some_and(|saved|
                serde_json::to_vec(&saved).ok()==serde_json::to_vec(&app.state).ok());
            let report=serde_json::json!({
                "mode":"original-main-gui-real-explorer-one-shot-native-drag",
                "real_explorer_mouse_gesture":true,
                "genuine_explorer_ole_data_object_used":false,
                "real_user_workspace_modified":gesture["real_user_workspace_modified"]==true,
                "workspace_is_unique_temp_clone":!native_chrome_live_transfer,
                "original_gui_frame_count":frame,
                "native_gui_to_disk_reconciled":same,
                "native_real_gesture_ui_persist":gesture,
                "native_original_worker":native_original_gesture_worker_report,
                "native_filter_detached_on_exit":
                    native_original_gesture_worker_report.as_ref()
                        .is_some_and(|v|v["callback_detached"]==true),
                "no_normal_workspace_save_mode":!app.save_allowed,
            });
            if let Some(destination)=native_chrome_ole_report.as_ref(){
                if let Some(parent)=destination.parent(){
                    let _=std::fs::create_dir_all(parent);
                }
                let _=std::fs::write(destination,
                    serde_json::to_vec_pretty(&report).unwrap_or_default());
            }
            break;
        }
        #[cfg(all(windows,feature="native-lab"))]
        if let (Some(destination),Some(context))=
            (isolated_ole_smoke_report.as_ref().or(native_chrome_ole_report.as_ref()),
             native_main_ole_context.as_ref()) {
            if frame>=45 {
                let verified_geometry=native_drop::measured_main_hwnd_viewport(
                    host::window(),app.width as f64,app.height as f64);
                let live=context.lock().map_err(|e|e.to_string()).ok();
                let same=live.as_ref().is_some_and(|ctx|
                    serde_json::to_vec(&ctx.state).ok()==serde_json::to_vec(&app.state).ok());
                let source_id=live.as_ref().and_then(|ctx|ctx.visible_box_ids.first()).copied();
                let destination_id=live.as_ref()
                    .and_then(|ctx|ctx.visible_box_ids.get(1)).copied();
                let source_items=source_id.and_then(|id|app.state.group(id))
                    .map(|group|group.tab().items.len());
                let destination_items=destination_id.and_then(|id|app.state.group(id))
                    .map(|group|group.tab().items.len());
                let preserved_sources=destination_id
                    .and_then(|id|app.state.group(id))
                    .is_some_and(|group|group.tab().items.iter()
                        .all(|pin|Path::new(&pin.path).exists()));
                let refreshed_worker_final_shutdown=if native_chrome_ole_lab {
                    native_tab_worker.take().map(|mut worker|worker.shutdown())
                }else{None};
                let report=serde_json::json!({
                    "mode":if native_chrome_ole_lab {
                        "isolated-real-layout-original-native-chrome-main-hwnd"
                    }else{"isolated-main-hwnd-ole-registration"},
                    "frame_count":frame,
                    "real_main_hwnd_verified":verified_geometry.is_ok(),
                    "receiver_registered":_native_main_ole_receiver.is_some(),
                    "original_ui_box_count":app.state.groups.len(),
                    "original_ui_page_count":app.state.pages.len(),
                    "disposable_workspace_loaded":State::load(&app.save_path).is_ok(),
                    "lab_state_matches_app_state":same,
                    "native_ole_drop_count":live.as_ref().map(|ctx|ctx.successful_drops).unwrap_or(0),
                    "synthetic_com_drop_tested":isolated_ole_smoke_drop || native_chrome_ole_smoke,
                    "synthetic_com_multi_item":isolated_ole_smoke_multi || native_chrome_ole_smoke,
                    "synthetic_com_selected_items":
                        if isolated_ole_smoke_multi || native_chrome_ole_smoke {2}else{1},
                    "synthetic_com_drop_error":isolated_ole_smoke_drop_error,
                    "pending_ui_handoff":live.as_ref().is_some_and(|ctx|ctx.ui_handoff_pending()),
                    "source_group_id":source_id,
                    "destination_group_id":destination_id,
                    "source_tab_items":source_items,
                    "destination_tab_items":destination_items,
                    "sample_source_still_exists":preserved_sources,
                    "user_desktop_access":native_chrome_ole_lab,
                    "user_workspace_written":false,
                    "workspace_is_temp_copy":true,
                    "native_worker_detached_after_ole_handoff":
                        if native_chrome_ole_lab {
                            native_chrome_ole_worker_shutdown.as_ref()
                                .is_some_and(|value|value["callback_detached"]==true)
                        }else{true},
                    "native_worker_shutdown":native_chrome_ole_worker_shutdown,
                    "native_post_drop_rehook":native_chrome_ole_rehook_result,
                    "native_rehook_history":native_chrome_ole_rehook_history,
                    "native_rehook_count":native_chrome_handled_drops,
                    "native_repeat_smoke":native_chrome_ole_repeat,
                    "native_rehook_verified":if native_chrome_ole_lab {
                        native_chrome_ole_rehook_result.as_ref()
                            .is_some_and(|value|value["validated_post_drop"]==true
                                && value["fresh_worker_attached"]==true)
                    }else{true},
                    "native_refreshed_worker_detached_on_exit":
                        if native_chrome_ole_lab {
                            refreshed_worker_final_shutdown.as_ref()
                                .is_some_and(|value|value["callback_detached"]==true
                                    && value["abort_reason"].is_null())
                        }else{true},
                    "native_refreshed_worker_final_shutdown":
                        refreshed_worker_final_shutdown,
                    "genuine_explorer_ole_data_object_used":false,
                    "real_explorer_drag_test":false
                });
                if let Some(parent)=destination.parent() {
                    let _=std::fs::create_dir_all(parent);
                }
                let _=std::fs::write(destination,
                    serde_json::to_vec_pretty(&report).unwrap_or_default());
                break;
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
        if native_chrome_preview && native_manual_seconds
            .is_some_and(|seconds|native_chrome_started.elapsed()
                >=std::time::Duration::from_secs(seconds)){
            #[cfg(windows)]
            if let Some(mut native)=native_two_box_worker.take(){
                let mut report=native.shutdown();
                report["ui_smoke_final_tab_144"]=app.state.group(144)
                    .map(|g|g.active).map_or(serde_json::Value::Null,|n|serde_json::json!(n));
                report["ui_smoke_final_tab_168"]=app.state.group(168)
                    .map(|g|g.active).map_or(serde_json::Value::Null,|n|serde_json::json!(n));
                if let Some(path)=option("--native-two-box-ui-report"){
                    let _=std::fs::write(path,
                        serde_json::to_vec_pretty(&report).unwrap_or_default());
                }
            }
            #[cfg(windows)]
            if let Some(mut native)=native_tab_worker.take(){
                let report=native.shutdown();
                if let Some(i)=native_manual_index{
                    if let Some(path)=args.get(i+2){
                        let _=std::fs::write(path,
                            serde_json::to_vec_pretty(&report).unwrap_or_default());
                    }
                }
            }
            app.host_exit();
            break;
        }
        if native_two_box_ui && args.iter().any(|a|a=="--native-two-box-ui-smoke")
            && frame>=250 {
            #[cfg(windows)]
            if let Some(mut native)=native_two_box_worker.take(){
                let mut report=native.shutdown();
                report["ui_smoke_final_tab_144"]=app.state.group(144)
                    .map(|g|g.active).map_or(serde_json::Value::Null,|n|serde_json::json!(n));
                report["ui_smoke_final_tab_168"]=app.state.group(168)
                    .map(|g|g.active).map_or(serde_json::Value::Null,|n|serde_json::json!(n));
                if let Some(path)=option("--native-two-box-ui-report"){
                    let _=std::fs::write(path,
                        serde_json::to_vec_pretty(&report).unwrap_or_default());
                }
                host::event("INFO","native_two_box.ui_smoke_done",
                    if report["callback_detached"]==true{
                        "Two-Box native Explorer hook detached"}
                    else{"Two-Box native filter stopped without detach acknowledgment"});
            }
            app.host_exit();
            break;
        }
        if native_chrome_preview && !native_two_box_ui
            && args.iter().any(|a|a=="--native-chrome-ui-smoke")
            && frame>=240 {
            #[cfg(windows)]
            if let Some(mut native)=native_tab_worker.take() {
                let report=native.shutdown();
                if let Some(i)=args.iter().position(|a|a=="--native-chrome-ui-smoke") {
                    if let Some(dest)=args.get(i+1) {
                        let _=std::fs::write(dest,
                            serde_json::to_vec_pretty(&report).unwrap_or_default());
                    }
                }
                host::event("INFO","native_chrome.smoke_closed",
                    if report["callback_detached"]==true{"Native Explorer hooks detached"}
                    else{"No active hook; native policy fails open"});
            }
            app.host_exit();
            break;
        }
        if is_quit_requested() {
            #[cfg(windows)]
            if let Some(mut native)=native_two_box_worker.take(){
                let report=native.shutdown();
                if let Some(path)=option("--native-two-box-ui-report"){
                    let _=std::fs::write(path,
                        serde_json::to_vec_pretty(&report).unwrap_or_default());
                }
            }
            #[cfg(windows)]
            if let Some(mut native)=native_tab_worker.take() {
                let report=native.shutdown();
                if let Some(i)=args.iter().position(|a|a=="--native-chrome-report") {
                    if let Some(path)=args.get(i+1) {
                        let _=std::fs::write(path,
                            serde_json::to_vec_pretty(&report).unwrap_or_default());
                    }
                }
                host::event("INFO","native_chrome.filter_detach",
                    if report["callback_detached"]==true{"native filter detached"}
                    else{"native filter failed-open or was not attached"});
            }
            if desktop_mode {
                #[cfg(windows)]
                {
                    if app.native_ai_pending.is_some() {
                        let _ = ai_tools::abort_pending_checked(
                            &mut app,
                            "DesktopCraft exited before Explorer confirmed the change",
                        );
                        native_last_accepted_state = Some(app.state.clone());
                        native_ai_exit_rollback = true;
                    } else if native_desktop_mode && native_save_permitted && !native_ai_exit_rollback {
                        if let Some(ref accepted) = native_last_accepted_state {
                            if native_pending_ui.is_some()
                                || native_effective_sync_needed(
                                    accepted,
                                    native_last_accepted_revealed,
                                    native_last_accepted_hidden,
                                    &app.state,
                                    app.revealed,
                                    app.hidden,
                                )
                            {
                                app.state = accepted.clone();
                            }
                        }
                    }
                }
                #[cfg(windows)]
                host::shutdown_mouse_hooks();
                app.host_exit();
            }
            #[cfg(windows)]
            if native_desktop_mode && native_save_permitted && !native_ai_exit_rollback {
                let saved_matches = native_disk_state.as_ref().is_some_and(|disk| {
                    serde_json::to_vec(disk).ok() == serde_json::to_vec(&app.state).ok()
                });
                if !saved_matches {
                    persist_native_workspace(
                        &mut app,
                        &mut native_disk_state,
                        &mut native_last_accepted_state,
                        &mut last_saved_state,
                        &mut last_seen_state,
                        input.time,
                    );
                }
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
        if native_desktop_mode && !automation {
            let cursor = host::cursor();
            let active = cursor != pacing_cursor || input.down || input.right_down
                || input.middle_down || input.pressed || input.released
                || input.right_pressed || input.right_released
                || input.scroll != 0.0 || !input.keys.is_empty()
                || !input.repeats.is_empty() || !input.typed.is_empty()
                || app.drag.is_some() || app.peek;
            pacing_cursor = cursor;
            if active {
                pacing_active_until = std::time::Instant::now()
                    + std::time::Duration::from_millis(250);
            }
            let budget = std::time::Duration::from_secs_f64(
                if std::time::Instant::now() < pacing_active_until { 1.0 / 120.0 }
                else { 1.0 / 60.0 }
            );
            // No busy wait. Time already spent updating/rendering counts
            // against the budget, so slow frames incur no extra sleep.
            let remaining = budget.saturating_sub(perf_frame_start.elapsed());
            if !remaining.is_zero() {
                std::thread::sleep(remaining);
            }
        }
        frame += 1;
        next_frame().await;
        #[cfg(windows)]
        drop(retired_drag);
    }
    #[cfg(windows)]
    {
        app.settings_window = None;
        host::remove_tray();
        host::shutdown_mouse_hooks();
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
