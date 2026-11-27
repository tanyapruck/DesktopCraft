use crate::*;

pub(super) fn capture_mode() -> bool {
    capture_mode_requested(&std::env::args().collect::<Vec<_>>())
}
pub(super) fn capture_mode_requested(args: &[String]) -> bool {
    args.iter().any(|x| {
        x == "--screenshot"
            || x == "--self-test"
            || x == "--text-check"
            || x == "--settings-screenshot"
    })
}
pub(super) fn host_mode() -> bool {
    macroquad::miniquad::desktop_host()
}

/// The actual replacement of an installed executable is permitted only
/// when the previous DesktopCraft desktop process has quit, its Native
/// Explorer rollback journal is gone, and no Explorer policy mapping is
/// still attached. A stale orphaned mapping is a blocker, not permission
/// to overwrite an executable.
#[cfg(windows)]
pub(super) fn update_install_safe_state(
    running_desktopcraft: bool,
    native_recovery_pending: bool,
    explorer_policy_mapping_present: bool,
) -> bool {
    !running_desktopcraft && !native_recovery_pending && !explorer_policy_mapping_present
}

#[cfg(windows)]
pub(super) fn update_install_quiescent() -> bool {
    update_install_safe_state(
        host::instance_running(),
        !matches!(host::data_dir().join("native-inbox-recovery.json").try_exists(), Ok(false)),
        native_filter_host::native_policy_status()["mapping_absence_confirmed"]!=true,
    )
}

#[cfg(all(test,windows))]
mod update_install_readiness_tests {
    use super::update_install_safe_state;

    #[test]
    fn never_replace_installed_exe_while_any_native_owner_or_recovery_is_active() {
        assert!(update_install_safe_state(false,false,false));
        for app in [false,true] {
            for journal in [false,true] {
                for mapping in [false,true] {
                    if app || journal || mapping {
                        assert!(!update_install_safe_state(app,journal,mapping));
                    }
                }
            }
        }
    }
}

/// Route desktop UI entry points to Explorer-owned icons in native builds.
#[cfg(not(windows))]
pub(super) fn native_desktop_launch_requested(_args:&[String])->bool{false}
#[cfg(windows)]
pub(super) fn native_desktop_launch_requested(args:&[String])->bool {
    cfg!(feature = "native-release") && explorer_desktop_entry_point(&args[1..])
}

#[cfg(windows)]
pub(super) fn explorer_desktop_entry_point(args: &[String]) -> bool {
    if capture_mode_requested(args) || explicitly_windowed_launch(args)
        || args.iter().any(|arg| arg == "--legacy-desktop") {
        return false;
    }
    // Profiling is an option on the same desktop entry point, not another
    // renderer. A trailing report path is required.
    let args = match args {
        [rest @ .., option, path] if option == "--perf-report"
            && !path.is_empty() && !path.starts_with("--") => rest,
        _ => args,
    };
    args.iter().any(|arg| arg == "--native-desktop") || match args {
        [] => true,
        [option] => option == "--at-sign-in",
        [option, _] => matches!(option.as_str(), "--command" | "--pin-file"),
        _ => false,
    }
}

#[cfg(all(windows, test))]
#[test]
pub(super) fn explorer_desktop_covers_all_installed_ui_entry_points() {
    let requested = |args: &[&str]| {
        explorer_desktop_entry_point(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    };
    assert!(requested(&[]));
    assert!(requested(&["--at-sign-in"]));
    assert!(requested(&["--command", "updated"]));
    assert!(requested(&["--pin-file", "C:/Test.txt"]));
    assert!(requested(&["--perf-report", "report.json"]));
    assert!(requested(&["--at-sign-in", "--perf-report", "report.json"]));
    assert!(!requested(&["--perf-report"]));
    assert!(!requested(&["--unknown-option"]));
    assert!(!requested(&["--host-smoke"]));
    assert!(!requested(&["--legacy-desktop", "--native-desktop"]));
    assert!(!requested(&["--windowed", "--perf-report", "report.json"]));
    assert!(requested(&["--native-desktop", "--native-acceptance-seconds", "30"]));
    assert!(!requested(&["--legacy-desktop"]));
    assert!(!requested(&["--native-desktop", "--windowed"]));
    assert!(!requested(&["--install"]));
    assert!(!requested(&["--native-desktop-check"]));
    assert!(!requested(&["--screenshot", "sample.png"]));
}

#[cfg(windows)]
pub(super) fn preflight_should_exit_silently(reason: &str) -> bool {
    reason.contains("native policy is active")
}

#[cfg(all(windows, test))]
#[test]
pub(super) fn preflight_never_exits_silently_on_stale_recovery_journal() {
    assert!(preflight_should_exit_silently(
        "Another real Explorer native policy is active. Close that DesktopCraft instance before starting a second one"
    ));
    assert!(!preflight_should_exit_silently(
        "An unresolved Explorer icon recovery journal exists from an interrupted Native Desktop session."
    ));
    assert!(!preflight_should_exit_silently(
        "Explorer retains a stale Native Desktop policy mapping."
    ));
}

pub(super) fn external_native_probe_requested(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| matches!(a.as_str(),
        "--native-multibox-fixture-test" | "--native-input-rightclick-probe" |
        "--native-input-hover-probe" | "--native-input-observe" |
        "--native-filter-hook-observe" | "--native-filter-hide-probe" |
        "--native-filter-tab-switch-probe" | "--native-filter-tab-full-probe" |
        "--native-filter-tab-click-probe" | "--native-two-box-tab-cycle-probe" |
        "--native-two-box-input-probe" | "--native-staged-hide-probe"
    ))
}

#[cfg(test)]
#[test]
fn external_probe_gate_distinguishes_release_entry_points() {
    let args = |items: &[&str]| items.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    assert!(external_native_probe_requested(&args(&["app.exe", "--native-filter-hook-observe", "3"])));
    assert!(external_native_probe_requested(&args(&["app.exe", "--native-desktop", "--native-input-observe"])));
    assert!(!external_native_probe_requested(&args(&["app.exe", "--native-filter-emergency-off"])));
    assert!(!external_native_probe_requested(&args(&["app.exe", "--command", "updated"])));
}

/// Explicit developer experiments are unavailable in production, even if
/// the caller supplies an acknowledgement or the corresponding environment flag.
pub(super) fn developer_experiment_requested(args: &[String]) -> bool {
    external_native_probe_requested(args) || args.iter().skip(1).any(|a| {
        (a.starts_with("--") && a.contains("-lab"))
            || matches!(a.as_str(),
                "--unsafe-native-test" | "--native-explorer-chrome" |
                "--native-chrome-ole-smoke" | "--native-chrome-real-pin-transfer" |
                "--native-two-pin-box-position-probe" | "--native-one-pin-box-position-probe" |
                "--native-position-one-item" | "--position-test" |
                "--native-filter-policy-preview" | "--explorer-underlay-probe" |
                "--explorer-owned-box" | "--native-target-create" |
                "--native-box-preview" | "--native-box-preview-smoke" |
                "--native-box-rust-preview" | "--native-box-rust-smoke" |
                "--native-box-macroquad-preview" | "--native-box-macroquad-smoke")
    })
}

#[cfg(test)]
#[test]
fn developer_experiment_gate_preserves_recovery_and_user_commands() {
    for flag in ["--native-explorer-chrome", "--native-box-rust-preview",
        "--position-test", "--native-filter-policy-preview", "--unsafe-native-test"] {
        assert!(developer_experiment_requested(&["app.exe".into(), flag.into()]));
    }
    for flag in ["--native-desktop", "--native-inbox-recover", "--recover-desktop",
        "--native-filter-emergency-off", "--install", "--uninstall", "--notices"] {
        assert!(!developer_experiment_requested(&["app.exe".into(), flag.into()]));
    }
}

pub(super) fn native_desktop_has_disallowed_lab_args(args: &[String]) -> bool {
    args.get(1..).unwrap_or(&[]).iter().any(|a| {
        (a.starts_with("--") && a.contains("-lab"))
            || a == "--isolated-workspace-only"
            || a == "--native-chrome-ole-smoke"
    })
}

pub(super) fn native_general_workspace_eligible(state: &State) -> bool {
    state
        .groups
        .iter()
        .any(|group| group.tabs.iter().any(|tab| tab.folder.is_none()))
}

#[cfg(test)]
#[test]
pub(super) fn native_general_workspace_eligible_accepts_all_rolled_up_boxes_and_rejects_portal_only() {
    let mut s = State::empty();
    assert!(!native_general_workspace_eligible(&s));

    let portal = s.add_group("Cloud", Bounds { x: 100., y: 80., w: 300., h: 240. });
    s.group_mut(portal).unwrap().tab_mut().folder = Some("D:/Cloud".into());
    assert!(!native_general_workspace_eligible(&s));

    let reg = s.add_group("Games", Bounds { x: 460., y: 80., w: 300., h: 240. });
    s.group_mut(reg).unwrap().rolled = true;
    s.group_mut(reg).unwrap().page = 1;
    assert!(native_general_workspace_eligible(&s));
}

#[cfg(test)]
#[test]
pub(super) fn native_desktop_lab_guard_ignores_exe_and_file_paths_but_blocks_lab_flags() {
    let mk = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(!native_desktop_has_disallowed_lab_args(&mk(&[
        r"C:\test-lab\preview\desktopcraft.exe",
    ])));
    assert!(!native_desktop_has_disallowed_lab_args(&mk(&[
        r"C:\test-lab\desktopcraft.exe",
        "--at-sign-in",
    ])));
    assert!(!native_desktop_has_disallowed_lab_args(&mk(&[
        r"C:\Apps\desktopcraft.exe",
        "--pin-file",
        r"C:\Users\demo\Desktop\my-lab-label.txt",
    ])));
    assert!(!native_desktop_has_disallowed_lab_args(&mk(&[])));
    assert!(native_desktop_has_disallowed_lab_args(&mk(&[
        r"C:\Apps\desktopcraft.exe",
        "--native-desktop",
        "--native-two-box-ui-lab",
    ])));
    assert!(native_desktop_has_disallowed_lab_args(&mk(&[
        r"C:\Apps\desktopcraft.exe",
        "--native-desktop",
        "--native-chrome-ole-lab",
    ])));
    assert!(native_desktop_has_disallowed_lab_args(&mk(&[
        r"C:\Apps\desktopcraft.exe",
        "--native-desktop",
        "--isolated-workspace-only",
    ])));
    assert!(native_desktop_has_disallowed_lab_args(&mk(&[
        r"C:\Apps\desktopcraft.exe",
        "--native-desktop",
        "--native-chrome-ole-smoke",
    ])));
}


/// Only synchronize changes that alter native Explorer icon ownership,
/// visible Box geometry, selected Tab, or native Desktop Page. OneDrive
/// Folder Portal refreshes (history, items, scroll), tooltip selection,
/// visual effects and other UI metadata must NOT cause an Explorer hook
/// generation/IFolderView reflow on every frame.
#[cfg(windows)]
pub(super) fn native_desktop_requires_worker_sync(before:&State,now:&State)->bool{
    if before.page!=now.page || before.pages!=now.pages || before.groups.len()!=now.groups.len(){
        return true;
    }
    for (a,b) in before.groups.iter().zip(&now.groups){
        if a.id!=b.id || a.page!=b.page || a.bounds!=b.bounds ||
            a.rolled!=b.rolled || a.dock!=b.dock || a.active!=b.active ||
            a.tabs.len()!=b.tabs.len() {
            return true;
        }
        for (left,right) in a.tabs.iter().zip(&b.tabs){
            if left.id!=right.id || left.folder.is_some()!=right.folder.is_some(){
                return true;
            }
            // The Portal is an independent Shell namespace, not Desktop
            // ListView. It can refresh in the UI without a native move.
            if left.folder.is_some(){continue;}
            if left.items.len()!=right.items.len() ||
                left.items.iter().zip(&right.items).any(|(x,y)|
                    x.id!=y.id || !x.path.eq_ignore_ascii_case(&y.path)){
                return true;
            }
        }
    }
    false
}

pub(super) fn portal_waiting_for_explorer(group: &Group, acknowledged: Option<&State>) -> bool {
    group.tab().folder.is_some() && acknowledged
        .filter(|state| state.page == group.page)
        .and_then(|state| state.group(group.id))
        .is_some_and(|previous| previous.page == group.page && previous.tab().folder.is_none())
}

#[cfg(all(test,windows))]
mod native_desktop_sync_safety_tests {
    use super::*;
    fn file(id:u64,path:&str)->Item {
        Item {
            id,name:path.into(),path:path.into(),
            kind:"lnk".into(),size:0,modified:0,label:None
        }
    }
    fn boxes()->(State,u64,u64) {
        let mut s=State::empty();
        let n=s.add_group("Native",Bounds{x:400.,y:100.,w:450.,h:300.});
        s.group_mut(n).unwrap().tab_mut().items.push(
            file(11,"C:/Users/Test/Desktop/A.lnk"));
        s.add_tab(n).unwrap();
        s.group_mut(n).unwrap().active=0;
        let p=s.add_group("OneDrive Portal",Bounds{x:900.,y:100.,w:480.,h:320.});
        s.group_mut(p).unwrap().tabs[0].folder=Some(
            "C:/Users/Test/OneDrive - Company".into());
        s.group_mut(p).unwrap().tabs[0].items.push(
            file(14,"C:/Users/Test/OneDrive - Company/old.txt"));
        (s,n,p)
    }
    #[test]
    fn portal_refresh_and_cosmetic_state_never_trigger_native_reflow(){
        let (before,n,p)=boxes();
        let mut now=before.clone();
        now.group_mut(p).unwrap().tabs[0].items.push(
            file(15,"C:/Users/Test/OneDrive - Company/new.txt"));
        now.group_mut(p).unwrap().tabs[0].scroll=12;
        now.group_mut(n).unwrap().tabs[0].items[0].name="renamed label".into();
        now.blend=!now.blend;
        assert!(!native_desktop_requires_worker_sync(&before,&now));
    }
    #[test]
    fn portal_navigation_does_not_require_an_explorer_layout_ack() {
        let (before, _, portal) = boxes();
        let mut after = before.clone();
        let tab = after.group_mut(portal).unwrap().tab_mut();
        tab.history.push(tab.folder.replace("D:/Folder/Subfolder".into()).unwrap());
        tab.items.clear();
        assert!(!native_desktop_requires_worker_sync(&before, &after));
        assert!(native_filter_host::native_explorer_layout_matches(&before, &after));
    }
    #[test]
    fn portal_contents_wait_until_the_regular_tab_has_been_hidden() {
        let (mut regular, id, _) = boxes();
        regular.group_mut(id).unwrap().tabs[1].folder = Some("D:/Folder".into());
        let mut portal = regular.clone();
        portal.group_mut(id).unwrap().active = 1;
        let group = portal.group(id).unwrap();
        assert!(portal_waiting_for_explorer(group, Some(&regular)));
        assert!(!portal_waiting_for_explorer(group, Some(&portal)));
        assert!(!portal_waiting_for_explorer(group, None));
        assert!(!portal_waiting_for_explorer(regular.group(id).unwrap(), Some(&portal)));
    }
    #[test]
    fn native_tab_rollup_geometry_page_and_pin_changes_need_ack(){
        let (prior,n,p)=boxes();
        let mut next=prior.clone();
        next.group_mut(n).unwrap().active=1;
        assert!(native_desktop_requires_worker_sync(&prior,&next));
        next=prior.clone();
        next.group_mut(n).unwrap().rolled=true;
        assert!(native_desktop_requires_worker_sync(&prior,&next));
        next=prior.clone();
        next.group_mut(n).unwrap().bounds.x+=48.;
        assert!(native_desktop_requires_worker_sync(&prior,&next));
        next=prior.clone();
        next.group_mut(n).unwrap().tabs[0].items[0].path="C:/Other.lnk".into();
        assert!(native_desktop_requires_worker_sync(&prior,&next));
        next=prior.clone();
        next.group_mut(p).unwrap().tabs[0].folder=None;
        assert!(native_desktop_requires_worker_sync(&prior,&next));
        next=prior.clone();
        next.pages.push("Space 2".into());
        assert!(native_desktop_requires_worker_sync(&prior,&next));
    }
    #[test]
    fn clamp_native_tx_frame_mutation_preserves_acknowledged_ui_and_undo_stack() {
        let root = std::env::temp_dir().join(format!(
            "desktopcraft-clamp-tx-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let (base_state, box_id, _) = boxes();
        let mut app = App::new(root.join("workspace"), false, true);
        app.state = base_state.clone();
        app.revealed = None;
        app.native_visual_revealed = None;
        let undo_before = app.undo.clone();

        // Untouched frame does not report a mutation.
        assert!(!clamp_native_tx_frame_mutation(
            &mut app,
            (&base_state, None, false),
            &undo_before
        ));

        // Accidental Tab switch + push_undo during active pin tx is rolled back cleanly.
        app.remember();
        app.state.group_mut(box_id).unwrap().active = 1;
        assert!(clamp_native_tx_frame_mutation(
            &mut app,
            (&base_state, None, false),
            &undo_before
        ));
        assert_eq!(app.state.group(box_id).unwrap().active, 0);
        assert_eq!(app.undo, undo_before);
        assert!(native_drop_matches_acknowledged_ui(
            &base_state,
            None,
            false,
            &app.state,
            false,
            &base_state,
        ));

        // Full 32-entry undo stack: "..." press (drag + menu_press + remember evicting undo[0])
        // and stray pop must both be restored identically without leaving menu_press armed.
        for step in 0..32 {
            let mut snap = base_state.clone();
            snap.next_id = 100 + step as u64;
            app.push_undo(snap);
        }
        assert_eq!(app.undo.len(), 32);
        let full_undo = app.undo.clone();

        app.state.blend = !base_state.blend;
        app.drag = Some(Drag::Group {
            id: box_id,
            start: Vec2::new(0., 0.),
            before: base_state.group(box_id).unwrap().bounds,
            before_dock: None,
        });
        app.menu_press = Some((box_id, Vec2::new(10., 10.), true));
        app.remember(); // evicts full_undo[0] while keeping len == 32
        assert_eq!(app.undo.len(), 32);
        assert!(clamp_native_tx_frame_mutation(
            &mut app,
            (&base_state, None, false),
            &full_undo
        ));
        assert_eq!(app.state.blend, base_state.blend);
        assert!(app.drag.is_none());
        assert!(app.menu_press.is_none());
        assert_eq!(app.undo, full_undo);
        assert_eq!(app.undo[0].next_id, 100);

        // Stray pop during a clamped frame must also restore the popped entry.
        app.undo.pop();
        assert!(clamp_native_tx_frame_mutation(
            &mut app,
            (&base_state, None, false),
            &full_undo
        ));
        assert_eq!(app.undo, full_undo);

        // Scrollbar / wheel state queued during a clamped frame must also clear cleanly.
        app.native_scroll_target = Some((box_id, 3));
        app.native_page_request = Some((box_id, 2));
        app.native_wheel_remainder = (box_id, 60);
        assert!(clamp_native_tx_frame_mutation(
            &mut app,
            (&base_state, None, false),
            &full_undo
        ));
        assert!(app.native_scroll_target.is_none());
        assert!(app.native_page_request.is_none());
        assert_eq!(app.native_wheel_remainder, (0, 0));

        // Queued classic file menu during a clamped frame must be cancelled safely.
        app.classic_file_menu = Some(desktop::ClassicFileMenu {
            path: "C:/Desktop/A.lnk".into(),
            anchor: Some((100, 100)),
            pin: None,
        });
        assert!(clamp_native_tx_frame_mutation(
            &mut app,
            (&base_state, None, false),
            &full_undo
        ));
        assert!(app.classic_file_menu.is_none());

        // Creating a group during a clamped frame must rollback both the group and its Rename dialog.
        app.create_group(Bounds { x: 120., y: 120., w: 300., h: 240. });
        assert!(matches!(app.dialog, Some(Dialog::Rename { .. })));
        assert!(clamp_native_tx_frame_mutation(
            &mut app,
            (&base_state, None, false),
            &full_undo
        ));
        assert!(app.dialog.is_none());
        assert_eq!(app.state.groups.len(), base_state.groups.len());

        // Toggling Quick-Hide during a clamped frame rolls back cleanly while leaving `peek` intact.
        app.hidden = true;
        app.peek = true;
        assert!(clamp_native_tx_frame_mutation(
            &mut app,
            (&base_state, None, false),
            &full_undo
        ));
        assert!(!app.hidden);
        assert!(app.peek);
        app.peek = false;

        drop(app);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn native_page_change_blocked_while_undo_restore_or_sync_is_pending() {
        let (accepted, box_id, _) = boxes();
        let mut altered = accepted.clone();
        altered.group_mut(box_id).unwrap().active = 1;

        assert!(native_page_change_allowed(
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some((&accepted, None, false)),
            (&accepted, None, false),
        ));
        assert!(!native_page_change_allowed(
            true, false, false, false, false, false, false,
            Some((&accepted, None, false)), (&accepted, None, false),
        ));
        assert!(!native_page_change_allowed(
            false, true, false, false, false, false, false,
            Some((&accepted, None, false)), (&accepted, None, false),
        ));
        assert!(!native_page_change_allowed(
            false, false, true, false, false, false, false,
            Some((&accepted, None, false)), (&accepted, None, false),
        ));
        assert!(!native_page_change_allowed(
            false, false, false, true, false, false, false,
            Some((&accepted, None, false)), (&accepted, None, false),
        ));
        assert!(!native_page_change_allowed(
            false, false, false, false, true, false, false,
            Some((&accepted, None, false)), (&accepted, None, false),
        ));
        assert!(!native_page_change_allowed(
            false, false, false, false, false, true, false,
            Some((&accepted, None, false)), (&accepted, None, false),
        ));
        assert!(!native_page_change_allowed(
            false, false, false, false, false, false, true,
            Some((&accepted, None, false)), (&accepted, None, false),
        ));
        assert!(!native_page_change_allowed(
            false, false, false, false, false, false, false,
            Some((&accepted, None, false)), (&altered, None, false),
        ));
    }
}

/// Test-owned real DesktopCraft sessions are deliberately self-limiting.
/// The CLI token is not a user setting and never enables native mode on its
/// own; it only lets a short physical acceptance run terminate gracefully
/// even if the external test controller/PowerShell process disappears.
#[cfg(windows)]
pub(super) fn native_acceptance_test_token(args:&[String])->Option<&str>{
    args.iter().find_map(|arg|arg.strip_prefix("--native-acceptance-token=")
        .filter(|token|token.len()==32 &&
            token.as_bytes().iter().all(u8::is_ascii_hexdigit)))
}
#[cfg(windows)]
pub(super) fn native_acceptance_test_seconds(args:&[String])->u64{
    args.iter().filter_map(|arg|arg.strip_prefix("--native-acceptance-soak-seconds="))
        .find_map(|value|value.parse::<u64>().ok())
        .filter(|secs|(20..=180).contains(secs))
        .unwrap_or(20)
}
#[cfg(all(windows,test))]
mod native_acceptance_token_tests{
    use super::*;
    #[test]
    fn only_explicit_guid_tag_is_recognized(){
        assert!(native_acceptance_test_token(&[
            "--native-desktop".into(),
            "--native-acceptance-token=1234567890abcdef1234567890ABCDEF".into()
        ]).is_some());
        assert!(native_acceptance_test_token(&[
            "--native-acceptance-token=bad".into()]).is_none());
        assert!(native_acceptance_test_token(&[
            "--native-desktop".into()]).is_none());
    }
}

#[cfg(test)]
mod startup_mode_guards_tests {
    use super::explicitly_windowed_launch;
    fn args(flags:&[&str])->Vec<String>{
        std::iter::once("desktopcraft.exe".to_string())
            .chain(flags.iter().map(|arg|arg.to_string())).collect()
    }
    #[test]
    fn explicit_ole_lab_is_guaranteed_window_mode_before_startup() {
        assert!(explicitly_windowed_launch(&args(&["--native-main-hwnd-lab"])));
        assert!(explicitly_windowed_launch(&args(&["--windowed"])));
        assert!(explicitly_windowed_launch(&args(&["--native-integrated-preview"])));
        assert!(!explicitly_windowed_launch(&args(&[])));
        assert!(!explicitly_windowed_launch(&args(&["--native-explorer-chrome"])));
        assert!(!explicitly_windowed_launch(&args(&["--native-ole-recovery-inspect"])));
    }
}

#[cfg(test)]
mod ole_recovery_startup_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static TEST_ID:AtomicU64=AtomicU64::new(1);
    #[test]
    fn desktopcraft_startup_pauses_writes_until_pending_ole_recovery_is_reviewed() {
        let root=std::env::temp_dir().join(format!(
            "DesktopCraft-OLE-StartupTest-{}-{}",
            std::process::id(), TEST_ID.fetch_add(1,Ordering::Relaxed)));
        let data=root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let workspace=data.join("workspace.json");
        State::empty().save(&workspace).unwrap();
        let original=std::fs::read(&workspace).unwrap();
        let stage=data.join(".workspace.json.desktopcraft-ole-test.tmp");
        std::fs::write(&stage,&original).unwrap();
        let mut app=App::new(root.clone(),false,false);
        assert!(!app.save_allowed);
        assert!(app.toast.contains("Workspace recovery needed"));
        app.state.tint=!app.state.tint;
        app.save(1.);
        assert_eq!(std::fs::read(&workspace).unwrap(),original);
        assert_eq!(std::fs::read(&stage).unwrap(),original);
        drop(app);
        std::fs::remove_file(&stage).unwrap();
        let backup=workspace.with_extension("ole-backup");
        std::fs::write(&backup,&original).unwrap();
        let app=App::new(root.clone(),false,false);
        assert!(!app.save_allowed);
        assert_eq!(std::fs::read(&backup).unwrap(),original);
        drop(app);
        std::fs::remove_file(&backup).unwrap();
        assert!(App::new(root.clone(),false,false).save_allowed);
        assert_eq!(std::fs::read(&workspace).unwrap(),original);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_workspace_with_pending_ole_artifact_is_not_first_run() {
        let root=std::env::temp_dir().join(format!(
            "DesktopCraft-OLE-MissingWorkspace-{}-{}",
            std::process::id(),TEST_ID.fetch_add(1,Ordering::Relaxed)));
        let data=root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let workspace=data.join("workspace.json");
        let pristine=serde_json::to_vec_pretty(&State::empty()).unwrap();
        // Simulated interruption after Windows has preserved original data
        // but before the active Workspace can be opened.
        let backup=workspace.with_extension("ole-backup");
        std::fs::write(&backup,&pristine).unwrap();
        let mut app=App::new(root.clone(),false,true);
        assert!(!app.save_allowed);
        assert!(!app.host.as_ref().unwrap().first_run);
        assert!(app.state.groups.is_empty(),"Recovery must not create starter Boxes");
        assert!(app.toast.contains("recovery needed"));
        app.save(0.5);
        assert!(!workspace.exists());
        assert_eq!(std::fs::read(&backup).unwrap(),pristine);
        drop(app);
        std::fs::remove_file(&backup).unwrap();

        let stage=data.join(".workspace.json.desktopcraft-ole-crash.tmp");
        std::fs::write(&stage,&pristine).unwrap();
        let app=App::new(root.clone(),false,true);
        assert!(!app.save_allowed);
        assert!(!app.host.as_ref().unwrap().first_run);
        assert!(!workspace.exists());
        assert_eq!(std::fs::read(&stage).unwrap(),pristine);
        drop(app);
        std::fs::remove_file(&stage).unwrap();
        assert!(App::new(root.clone(),false,true).host.as_ref().unwrap().first_run);
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Side-effect-free classification: developer sample OLE mode MUST return
/// early from startup before any Desktop/Explorer ownership path is reached.
pub(super) fn explicitly_windowed_launch(args:&[String])->bool {
    args.iter().any(|arg|matches!(arg.as_str(),
        "--windowed"|"--native-integrated-preview"|"--native-main-hwnd-lab"))
}
/// Runs before any window opens: setup, removal, passing a command to the
/// running copy, and choosing desktop mode (the normal way to run on Windows).
#[cfg(windows)]
pub(super) fn enable_native_audit_physical_dpi() -> bool {
    // startup() runs before Macroquad's Windows DPI initialization. Without
    // a per-monitor-aware *thread*, EnumDisplayMonitors reports the 125%
    // logical dimensions while the real Desktop icon layout uses physical
    // pixels. Set only this short-lived audit thread; never alter user
    // Display Settings.
    #[link(name="user32")]
    extern "system" {
        fn SetThreadDpiAwarenessContext(context:isize)->isize;
    }
    unsafe {SetThreadDpiAwarenessContext(-4)!=0}
}


/// The watcher waits on the exact owner process, not on a timer. A PID plus
/// creation time protects against accidentally following a reused PID.
#[cfg(windows)]
pub(crate) fn process_creation_stamp(handle: winapi::um::winnt::HANDLE) -> Option<u64> {
    use winapi::{shared::minwindef::FILETIME, um::processthreadsapi::GetProcessTimes};
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exited = created;
    let mut kernel = created;
    let mut user = created;
    (unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } != 0)
        .then_some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

#[cfg(windows)]
pub(super) fn start_native_crash_watch() {
    use winapi::um::processthreadsapi::GetCurrentProcess;
    let result = (|| -> Result<(), String> {
        let stamp = process_creation_stamp(unsafe { GetCurrentProcess() })
            .ok_or("Owner creation time unavailable")?;
        std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args(["--native-crash-watch", &std::process::id().to_string(), &stamp.to_string()])
            .spawn().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(reason) = result {
        host::event("WARN", "native_desktop.crash_watch_unavailable", &reason);
    }
}

#[cfg(windows)]
fn crash_arrange_required(exit_code: u32, pending_checkpoint: bool) -> bool {
    exit_code != 0 && pending_checkpoint
}

#[cfg(windows)]
pub(super) fn run_native_crash_watch(args: &[String]) -> i32 {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use winapi::um::processthreadsapi::{GetExitCodeProcess, OpenProcess};
    #[link(name = "kernel32")]
    extern "system" {
        fn WaitForSingleObject(handle: winapi::um::winnt::HANDLE, milliseconds: u32) -> u32;
    }
    if !cfg!(feature = "native-release") {
        return 2;
    }
    let [pid, stamp] = args else { return 2; };
    let (Ok(pid), Ok(stamp)) = (pid.parse::<u32>(), stamp.parse::<u64>()) else { return 2; };
    if pid == 0 || pid == std::process::id() { return 2; }
    // SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION; no write/terminate rights.
    let handle = unsafe { OpenProcess(0x0010_1000, 0, pid) };
    if handle.is_null() { return 2; }
    let owner = unsafe { OwnedHandle::from_raw_handle(handle.cast()) };
    let handle = owner.as_raw_handle().cast();
    if process_creation_stamp(handle) != Some(stamp) { return 2; }
    if unsafe { WaitForSingleObject(handle, u32::MAX) } != 0 { return 2; }
    let mut code = 0;
    if unsafe { GetExitCodeProcess(handle, &mut code) } == 0 { return 2; }
    let journal = host::data_dir().join("native-inbox-recovery.json");
    if !crash_arrange_required(code, journal.is_file()) { return 0; }
    // Claim the same single-instance guard as the GUI. If the user already
    // restarted it, that instance will handle the interrupted journal.
    if !host::first_instance() { return 0; }
    if !enable_native_audit_physical_dpi() { return 2; }
    shell::init();
    match wait_for_native_crash_detach().and_then(|()| arrange_interrupted_native_desktop()) {
        Ok(()) => 0,
        Err(reason) => {
            host::event("WARN", "native_desktop.crash_arrange_failed", &reason);
            2
        }
    }
}

// Request a normal Explorer repaint only for a verified expired native
// lease. The existing DLL handles its own detach on WM_PAINT. This is not
// permission to change any Explorer setting until mapping absence is proven.
#[cfg(windows)]
fn wait_for_native_crash_detach() -> Result<(), String> {
    for _ in 0..32 {
        let policy = native_filter_host::native_policy_status();
        if policy["mapping_absence_confirmed"] == true {
            return Ok(());
        }
        if policy["mapping_matches_live_Explorer"] == true
            && policy["protocol_magic_valid"] == true
            && policy["protocol_abi_valid"] == true
            && policy["lease_expired"] == true
        {
            if let Some(view) = shell::DesktopView::acquire() {
                if native_explorer_guard::desktop_owner(&view)
                    .is_ok_and(|owner| policy["explorer_pid"].as_u64() == Some(u64::from(owner.pid)))
                {
                    unsafe {
                        winapi::um::winuser::InvalidateRect(view.list_window(), std::ptr::null(), 0);
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err("Explorer callback mapping not confirmed absent; recovery checkpoint retained".into())
}

/// Both callers own the single-instance guard; this never runs on normal
/// shutdown. Preserve the old journal rather than discard its evidence.
#[cfg(windows)]
fn arrange_interrupted_native_desktop() -> Result<(), String> {
    let root = host::data_dir();
    let journal = root.join("native-inbox-recovery.json");
    if !journal.is_file() { return Ok(()); }
    if native_filter_host::native_policy_status()["mapping_absence_confirmed"] != true {
        return Err("Explorer native callback absence not confirmed".into());
    }
    let view = shell::DesktopView::acquire().ok_or("Explorer Desktop unavailable")?;
    native_explorer_guard::desktop_owner(&view)?;
    let marker = root.join("native-crash-arranged");
    // Marker is written before changing the preference, so even interruption
    // between the Explorer change and journal archive is recoverable.
    if !marker.exists() {
        std::fs::OpenOptions::new().write(true).create_new(true).open(&marker)
            .and_then(|file| file.sync_all()).map_err(|e| e.to_string())?;
    }
    if view.icons_visible() != Some(true) && !view.set_icons_visible(true) {
        return Err("Explorer refused to show desktop icons".into());
    }
    if view.icons_visible() != Some(true) || !view.enable_auto_arrange() {
        return Err("Explorer refused Auto arrange icons".into());
    }
    // Native Box placement temporarily disables Align icons to grid.
    // Restore the recorded preference when available; do not infer it from
    // a malformed journal or manually write any icon coordinates.
    if let Ok(bytes) = std::fs::read(&journal) {
        if let Ok(data) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if data["format"] == 1 {
                if let Some(original) = data["original_snap_to_grid"].as_bool() {
                    if view.snap_to_grid() != Some(original) && !view.set_snap_to_grid(original) {
                        return Err("Explorer refused to restore Align icons to grid".into());
                    }
                }
            }
        }
    }
    let archived = native_box_live_layout::archive_stale_recovery_journal(&journal)?;
    host::event("INFO", "native_desktop.crash_arranged",
        &format!("Explorer Auto arrange enabled; recovery checkpoint archived at {}", archived.display()));
    Ok(())
}

/// The next native launch turns off ONLY Auto arrange that our recovery
/// enabled. Ordinary user settings and normal exit are not changed.
#[cfg(windows)]
pub(super) fn prepare_native_crash_recovery() -> Result<(), String> {
    let root = host::data_dir();
    let journal = root.join("native-inbox-recovery.json");
    let marker = root.join("native-crash-arranged");
    if !journal.exists() && !marker.exists() { return Ok(()); }
    wait_for_native_crash_detach()?;
    if journal.exists() { arrange_interrupted_native_desktop()?; }
    if marker.exists() {
        let view = shell::DesktopView::acquire().ok_or("Explorer Desktop unavailable")?;
        native_explorer_guard::desktop_owner(&view)?;
        if view.auto_arrange() == Some(true) { let _ = view.disable_auto_arrange(); }
        if view.auto_arrange() != Some(false) {
            return Err("Explorer refused to disable recovery Auto arrange for Native Desktop".into());
        }
        std::fs::remove_file(marker).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(all(windows, test))]
#[test]
fn crash_arrange_requires_abnormal_exit_and_pending_checkpoint() {
    assert!(!crash_arrange_required(0, true));
    assert!(!crash_arrange_required(101, false));
    assert!(crash_arrange_required(101, true));
    assert!(crash_arrange_required(0xc0000005, true));
}
