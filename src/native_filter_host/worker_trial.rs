use super::*;

/// Match the DLL's uptime clock, including time spent suspended. Leave a
/// margin before the five-second native policy lease expires.
fn lease_gap_requires_rebind(now: u64, last: u64) -> bool {
    last == 0 || now.checked_sub(last).is_none_or(|age| age >= 4000)
}

#[cfg(test)]
#[test]
fn suspended_worker_never_reuses_expired_lease() {
    assert!(!lease_gap_requires_rebind(4999, 1000));
    assert!(lease_gap_requires_rebind(5000, 1000));
    assert!(lease_gap_requires_rebind(120000000, 1000));
    assert!(lease_gap_requires_rebind(1000, 2000));
    assert!(lease_gap_requires_rebind(1000, 0));
}

pub(super) fn run_native_hook_trial(seconds: u32, workspace: &std::path::Path,
                         dll_path: &std::path::Path,
                         experimental_single_index: Option<usize>,
                         experimental_tab_group: Option<(u64, Option<usize>, bool)>,
                         panel_mode: Option<bool>,
                         external_commands: Option<Receiver<usize>>,
                         input_diagnostic_index: Option<usize>,
                         synthetic_input: u8,
                         ready_signal: Option<Arc<AtomicBool>>,
                         synthetic_multi_box: bool) -> Value {
    run_native_hook_trial_with_event(seconds,workspace,dll_path,
        experimental_single_index,experimental_tab_group,panel_mode,
        external_commands,input_diagnostic_index,synthetic_input,
        ready_signal,synthetic_multi_box,None,None,false,None,None,None)
}

pub(super) fn run_native_hook_trial_with_event(
                         seconds: u32, workspace: &std::path::Path,
                         dll_path: &std::path::Path,
                         experimental_single_index: Option<usize>,
                         experimental_tab_group: Option<(u64, Option<usize>, bool)>,
                         panel_mode: Option<bool>,
                         external_commands: Option<Receiver<usize>>,
                         input_diagnostic_index: Option<usize>,
                         synthetic_input: u8,
                         ready_signal: Option<Arc<AtomicBool>>,
                         synthetic_multi_box: bool,
                         gesture_sender:Option<SyncSender<NativeGestureEvidence>>,
                         stop_signal:Option<Arc<AtomicBool>>,
                         allow_real_workspace:bool,
                         staged_hide_max:Option<usize>,
                         ui_channel:Option<NativeUiChannel>,
                         real_drop_sender:Option<SyncSender<(NativeVerifiedDrop, PendingNativeWorkspaceSync)>>
                         ) -> Value {
    let two_box_ui_commands=ui_channel.as_ref().map(|channel|&channel.commands);
    let reject = |reason: String| json!({"started":false, "reason":reason,
        "paint_filter_enabled":false, "input_filter_enabled":false});
    let Some(view)=DesktopView::acquire() else {return reject("Explorer Desktop unavailable".into())};
    if view.icons_visible()!=Some(true) {
        return reject("Explorer native icons must be visible first".into());
    }
    let owner=match desktop_owner(&view){
        Ok(owner)=>owner,
        Err(reason)=>return reject(reason),
    };
    let (shell, mut drop_identities) = if real_drop_sender.is_some() && cfg!(feature = "native-fast-tabs") {
        view.items_with_drop_identities()
    } else {
        (view.items(), None)
    };
    let list=view.list_window();
    let mut names:Vec<String>=list_items(list).into_iter().map(|item|item.name).collect();
    let index_audit=explorer_native::audit_native_indices(&shell,&names);
    // Shell order and native accessibility order legitimately differ after
    // Explorer restart/reflow. Reconcile unique names to full Shell paths
    // without reordering, moving or changing the desktop itself.
    let mut native_shell=match explorer_native::native_view_in_msaa_order(&shell,&names) {
        Ok(mapped)=>mapped,
        Err(reason)=>return reject(format!("Native index fail-safe: {reason}")),
    };
    if two_box_ui_commands.is_some(){
        let outside=crate::native_box_live_layout::offscreen_icon_count(
            &native_shell,&crate::native_box_live_layout::current_screens());
        if outside>0 {
            return reject(format!(
                "Native UI preflight refused {outside} Explorer icons outside connected monitors; no hook attached"));
        }
    }
    // Real DesktopCraft is a separately selected UI mode, compiled with its
    // embedded/version-matched Native Explorer DLL. The old diagnostic mode
    // keeps all its Debug-only safety acknowledgements and small limits.
    let real_native_desktop=two_box_ui_commands.is_some()
        && cfg!(feature="native-release")
        && std::env::var("DESKTOPCRAFT_NATIVE_REAL_DESKTOP")
            .ok().as_deref()==Some("1");
    let native_mode_authorized=real_native_desktop
        || cfg!(all(debug_assertions,feature="native-lab"));
    let general_workspace_requested=two_box_ui_commands.is_some()
        && std::env::var("DESKTOPCRAFT_NATIVE_GENERAL_WORKSPACE")
            .ok().as_deref()==Some("1")
        && native_mode_authorized;
    let layout_requested=two_box_ui_commands.is_some()
        && std::env::var("DESKTOPCRAFT_NATIVE_INBOX_LAYOUT")
            .ok().as_deref()==Some("1")
        && native_mode_authorized;
    let native_ui_change_cap = (!real_native_desktop).then_some(MAX_NATIVE_TWO_BOX_UI_CHANGES_PER_SESSION);
    let native_hide_cap=if real_native_desktop {
        crate::native_live_membership::MAX_USER_NATIVE_DESKTOP_HIDE
    }else{crate::native_live_membership::MAX_EXPERIMENTAL_STAGED_HIDE};
    let mut native_original_order=native_shell.clone();
    let mut workspace_before=match crate::native_ole_persist::read_existing(workspace) {
        Ok(bytes)=>bytes,
        Err(e)=>return reject(format!("Cannot snapshot original native Workspace: {e}")),
    };
    // The State must be derived from EXACTLY the protected preimage bytes.
    // Reading the State and baseline separately is a race with another writer.
    let mut state=match State::from_saved_bytes(&workspace_before) {
        Ok(s)=>s,Err(_)=>return reject("DesktopCraft workspace unavailable".into()),
    };
    let mut persisted_disk_state=state.clone();
    let mut expected_disk_after_drop:Option<State>=None;
    let mut current_rolled_overrides:Vec<(u64,bool)>=Vec::new();
    let drag_fixture=synthetic_multi_box && cfg!(debug_assertions)
        && cfg!(feature="native-unsafe-drag-lab")
        && std::env::var("DESKTOPCRAFT_NATIVE_DRAG_TEST").ok().as_deref()==Some("1");
    let multi_drag_fixture=drag_fixture &&
        std::env::var("DESKTOPCRAFT_NATIVE_DRAG_MULTI_TEST")
            .ok().as_deref()==Some("1");
    let original_ui_drag_fixture=multi_drag_fixture &&
        gesture_sender.is_some() &&
        std::env::var("DESKTOPCRAFT_NATIVE_ORIGINAL_UI_GESTURE")
            .ok().as_deref()==Some("1");
    if original_ui_drag_fixture {
        let parent=std::env::temp_dir().canonicalize()
            .map_err(|e|e.to_string()).ok();
        let allowed=workspace.canonicalize().ok().is_some_and(|path|
            path.parent().and_then(|p|p.parent())
                .and_then(|root|root.parent())==parent.as_deref()
            && path.file_name().is_some_and(|n|n=="workspace.json")
            && path.parent().is_some_and(|p|p.file_name().is_some_and(|n|n=="data"))
            && path.parent().and_then(|p|p.parent())
                .and_then(|p|p.file_name())
                .is_some_and(|name|name.to_string_lossy()
                    .starts_with("DesktopCraftExplorerChromeOLE-")));
        if !allowed {
            return reject("Original UI native gesture worker requires an isolated temporary workspace".into());
        }
    }
    let tab_set = if let Some((group_id,to_tab,full)) = experimental_tab_group {
        let result=if full {
            explorer_native::choose_native_tab_set(&state,&native_shell,group_id,to_tab,3)
        }else{
            explorer_native::choose_native_tab_pair(&state,&native_shell,group_id,to_tab)
                .map(explorer_native::NativeTabSet::from_pair)
        };
        match result {
            Ok(set)=>Some(set),
            Err(reason)=>return reject(format!("Unsafe Tab trial rejected: {reason}")),
        }
    } else {None};
    let multi_plan=if synthetic_multi_box {
        match explorer_native::native_multi_box_plan(&state,&native_shell,
             if original_ui_drag_fixture{16}else{2},
             if original_ui_drag_fixture{128}else{2}) {
            Ok(plan) if original_ui_drag_fixture && plan.groups.len()==2
                && plan.groups[0].native_indices
                    .get(plan.groups[0].active).is_some_and(|pins|pins.len()>=2 && pins.len()<=8)
                && plan.groups[1].native_indices
                    .get(plan.groups[1].active).is_some_and(|pins|pins.is_empty())
                && plan.groups[0].group_id!=plan.groups[1].group_id
                =>Some(plan),
            Ok(plan) if !original_ui_drag_fixture && plan.groups.len()==2
                && plan.groups.iter().all(|g|g.active==0 && g.native_indices.len()==2)
                && (if multi_drag_fixture {
                    // Verified active multi-source: 2 Craft pins in source
                    // Tab 0, its inactive Tab empty. Destination has one
                    // Work pin in each of its two regular Tabs.
                    plan.groups[0].native_indices[0].len()==2
                        && plan.groups[0].native_indices[1].is_empty()
                        && plan.groups[1].native_indices[0].len()==1
                        && plan.groups[1].native_indices[1].len()==1
                }else{
                    plan.groups.iter().all(|g|
                        g.native_indices.iter().all(|indices|indices.len()==1))
                })
                =>Some(plan),
            Ok(_)=>return reject("Two-Box fixture must match exact scoped single/multi native source Tab constraints".into()),
            Err(reason)=>return reject(format!("Multi-Box fixture rejected: {reason}")),
        }
    }else{None};
    let (mut staged_hide,mut staged_tab_selections,mut ui_membership)=
        if let Some(maximum)=staged_hide_max {
        if (!real_native_desktop &&
                (!cfg!(debug_assertions) || !cfg!(feature="native-lab")))
            || synthetic_multi_box || drag_fixture
            || experimental_single_index.is_some() || input_diagnostic_index.is_some()
            || tab_set.is_some() || gesture_sender.is_some() {
            return reject("Native visibility requires an explicitly authorized matched Release UI or the isolated Debug Lab".into());
        }
        let member=match if general_workspace_requested {
            crate::native_live_membership::NativeTwoBoxMembership::prepare_workspace(
                1,&state,&native_shell)
        }else{
            crate::native_live_membership::NativeTwoBoxMembership::prepare(
                1,&state,&native_shell,144,168)
        } {
            Ok(member)=>member,Err(error)=>return reject(format!("Staged native identity refused: {error}")),
        };
        let member_view=match member.snapshot(&native_shell) {
            Ok(view)=>view,Err(error)=>return reject(format!("Staged live membership refused: {error}")),
        };
        if maximum==usize::MAX-1 && two_box_ui_commands.is_some(){
            if member_view.hidden_indices.len()>native_hide_cap {
                return reject(format!("Real Native Desktop initial hide {} exceeds its verified cap {}",member_view.hidden_indices.len(),native_hide_cap));
            }
            let initial=member.selected_tabs.clone();
            (Some(vec![member_view.hidden_indices]),Some(vec![initial]),Some(member))
        }else if maximum==0 || maximum==usize::MAX {
            let steps=match crate::native_live_membership::native_two_box_tab_cycle(
                &state,&native_shell,&member) {
                Ok(steps)=>steps,
                Err(error)=>return reject(format!("Native two-Box cycle refused: {error}")),
            };
            (Some(steps.iter().map(|(_,view)|view.hidden_indices.clone())
                    .collect::<Vec<_>>()),
             Some(steps.into_iter().map(|(tabs,_)|tabs).collect::<Vec<_>>()),None)
        }else{
            let stages=match crate::native_live_membership::native_hide_stages(
                &member_view.hidden_indices,maximum){
                Ok(stages)=>stages,Err(error)=>return reject(error),
            };
            (Some(stages),None,None)
        }
    }else{(None,None,None)};
    let decision=NativePresentation::decide(&state,&native_shell);
    if decision.conflicting_active_references!=0 {
        return reject("Conflicting native desktop pins".into());
    }
    let lookup:HashMap<String,NativeVisibility>=decision.items.into_iter()
        .map(|entry|(entry.path.to_lowercase(),entry.visibility)).collect();
    let Some(mut records)=proto::verified_records(native_shell.iter().enumerate().map(|(index,item)| {
        let visibility=lookup.get(&item.path.to_lowercase()).copied().unwrap_or(NativeVisibility::Loose);
        (index,item.label.as_str(),matches!(visibility,NativeVisibility::InactiveTab|
          NativeVisibility::InactivePage|NativeVisibility::RolledUp))
    }),native_shell.len()) else {return reject("No fully verified native item policy".into())};
    // Experimental one-item visibility probe is opt-in and temporary.
    // Never enable 57 hidden items on an unverified rendering implementation.
    if input_diagnostic_index.is_some_and(|index|index>=records.len()) {
        return reject("Native input diagnostic index is outside validated desktop view".into());
    }
    let flags=if staged_hide.is_some(){
        // Start with all Explorer icons visible. Hide only a bounded subset
        // after the initial native DLL identity/attach handshake succeeds.
        for record in &mut records {record.state=proto::STATE_SHOW;}
        proto::FLAG_PAINT_HIDE|proto::FLAG_INPUT_HIDE
    } else if let Some(ref plan)=multi_plan {
        let hidden=match plan.hidden_for(&[]) {
            Ok(items)=>items,Err(reason)=>return reject(reason),
        };
        for (index,record) in records.iter_mut().enumerate(){
            // A drag-capture experiment must NOT also hide/reflow real icons.
            record.state=if !drag_fixture && hidden.contains(&index) {
                proto::STATE_HIDE
            }else{proto::STATE_SHOW};
        }
        if drag_fixture { proto::FLAG_DRAG_CAPTURE }
        else { proto::FLAG_PAINT_HIDE | proto::FLAG_INPUT_HIDE }
    } else if synthetic_input!=0 {
        let Some(index)=input_diagnostic_index else {
            return reject("Hover probe requires a validated index".into());
        };
        for (i,record) in records.iter_mut().enumerate() {
            record.state=if i==index {proto::STATE_HIDE}else{proto::STATE_SHOW};
        }
        proto::FLAG_PAINT_HIDE | proto::FLAG_INPUT_HIDE
    } else if let Some(index)=experimental_single_index {
        if index >= records.len() {return reject("Requested native index out of range".into());}
        for (i,record) in records.iter_mut().enumerate() {
            record.state=if i==index {proto::STATE_HIDE}else{proto::STATE_SHOW};
        }
        proto::FLAG_PAINT_HIDE | proto::FLAG_INPUT_HIDE
    } else if let Some(ref set)=tab_set {
        // Initially display selected Tab A's native icons and hide only the
        // chosen icons belonging to B. All non-target desktop icons remain
        // fully visible, selectable, and controlled by Explorer.
        let hidden=set.hidden_indices(0);
        for (i,record) in records.iter_mut().enumerate() {
            record.state=if hidden.contains(&i) {
                proto::STATE_HIDE
            }else{proto::STATE_SHOW};
        }
        proto::FLAG_PAINT_HIDE | proto::FLAG_INPUT_HIDE
    } else { 0 };
    // A verified model plan alone does not make bulk Explorer item
    // suppression safe. Bound any ACTUAL paint/input subclass policy until
    // the older real Desktop icon-reflow incident has been reproduced and
    // a full recovery gate is approved. A native gesture capture flags=DRAG
    // and NEVER hides any item.
    let hide_count=records.iter().filter(|r|
        r.state==proto::STATE_HIDE).count();
    if flags & (proto::FLAG_PAINT_HIDE|proto::FLAG_INPUT_HIDE)!=0
        && hide_count>crate::native_live_membership::MAX_SAFE_NATIVE_HIDE_ON_REAL_DESKTOP {
        return reject(format!(
            "Real Explorer paint filter refused {} hidden icon records above eight-icon safety cap",
            hide_count));
    }
    if desktop_owner(&view).ok()!=Some(owner) {
        return reject("Explorer Desktop ownership changed before native hook attachment".into());
    }
    let (pid,thread)=(owner.pid,owner.thread);
    let dll=dll_path.to_string_lossy().to_lowercase();
    if !dll.ends_with("desktopcraft_native_filter.dll") || !dll_path.is_file() {
        return reject("Expected independently built DesktopCraft native DLL was not found".into());
    }
    // Release builds accept only bytes embedded in this exact application.
    // Reject external debug/lab DLLs before publishing any policy.
    if cfg!(feature="native-release") {
        match (crate::native_bundle::embedded_filter(), std::fs::read(dll_path)) {
            (Ok(expected), Ok(actual)) if actual.as_slice() == expected => {},
            _ => return reject("Native DLL differs from verified embedded resource".into()),
        }
    } else if !cfg!(all(debug_assertions, feature="native-lab")) {
        return reject("External native DLL requires a Debug native-lab build".into());
    }
    let memory=match unsafe{SharedPolicy::create(pid)} {
        Ok(m)=>m,Err(reason)=>return reject(reason),
    };
    let hwnd=list as usize as u64;
    unsafe {
        if drag_fixture {
            let source=&multi_plan.as_ref().unwrap().groups[0];
            (*memory.mapped).drag_source_index=
                source.native_indices[source.active][0] as u32;
        }
        memory.publish(pid,hwnd,shell.len(),&records,2,flags);
    }
    let mut inventory_generation = unsafe { memory.generation().load(Ordering::Acquire) };
    // Never load the Cargo output DLL directly into Explorer. The module
    // intentionally holds its own reference after installing native window
    // subclasses, so a loaded file can stay locked until Explorer exits.
    // A versioned copy is safe; subsequent Cargo builds remain writable.
    let runtime_dir=dll_path.parent().unwrap_or_else(||std::path::Path::new(".")).join("runtime");
    if let Err(e)=std::fs::create_dir_all(&runtime_dir) {
        return reject(format!("Native hook runtime directory: {e}"));
    }
    let build_id=unsafe{GetTickCount64()};
    let runtime_dll=runtime_dir.join(format!("desktopcraft_native_filter_{}_{}.dll",pid,build_id));
    if let Err(e)=std::fs::copy(dll_path,&runtime_dll) {
        return reject(format!("Cannot copy versioned native hook DLL: {e}"));
    }
    // Lock the runtime copy against writes/deletion while validating and
    // attaching the native hook. No unverified copy may reach LoadLibraryW.
    let _verified_runtime = if cfg!(feature="native-release") {
        use std::{io::Read, os::windows::fs::OpenOptionsExt};
        let Ok(mut file) = std::fs::OpenOptions::new().read(true).share_mode(1).open(&runtime_dll) else {
            return reject("Cannot open verified native runtime DLL".into());
        };
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() || !crate::native_bundle::embedded_filter().is_ok_and(|expected| expected == bytes.as_slice()) {
            return reject("Native runtime DLL content verification failed".into());
        }
        Some(file)
    } else { None };
    let mut library=ptr::null_mut();
    let mut hook=ptr::null_mut();
    let mut error=String::new();
    unsafe {
        let filename:Vec<u16>=runtime_dll.as_os_str().to_string_lossy()
            .encode_utf16().chain(Some(0)).collect();
        library=LoadLibraryW(filename.as_ptr());
        if library.is_null() {error=format!("LoadLibraryW failed: {}",GetLastError());}
        else {
            let abi=GetProcAddress(library,b"DesktopCraftFilterAbiVersion\0".as_ptr() as *const i8);
            let entry=GetProcAddress(library,b"DesktopCraftFilterHook\0".as_ptr() as *const i8);
            if abi.is_null() || entry.is_null() {
                error="Required native DLL exports not found".into();
            } else {
                let get_abi:extern "system" fn()->u32=mem::transmute(abi);
                let paint_ack=GetProcAddress(library,b"DesktopCraftFilterPaintAckVersion\0".as_ptr() as *const i8);
                if get_abi()!=proto::ABI_VERSION {
                    error="Native DLL protocol ABI mismatch".into();
                } else if real_native_desktop && (paint_ack.is_null()
                    || mem::transmute::<_, extern "system" fn()->u32>(paint_ack)()!=proto::PAINT_ACK_VERSION) {
                    error="Native DLL does not support Explorer repaint acknowledgments".into();
                } else if desktop_owner(&view).ok()!=Some(owner) {
                    error="Explorer Desktop ownership changed while preparing native hook".into();
                } else {
                    // WH_GETMESSAGE for this exact Explorer desktop UI thread.
                    hook=SetWindowsHookExW(3,entry,library,thread);
                    if hook.is_null() {error=format!("SetWindowsHookExW failed: {}",GetLastError());}
                }
            }
        }
    }
    if !error.is_empty() {
        unsafe {if !hook.is_null(){UnhookWindowsHookEx(hook); }
            if !library.is_null(){FreeLibrary(library); }}
        return reject(error);
    }
    let controller=HookGuard{hook,module:library,list,mapping:memory.mapped};
    let posted=unsafe{PostMessageW(list,proto::APP_HOOK_INSTALL,proto::HOOK_COOKIE,0)!=0};
    // Pin the INITIAL verified generation while Explorer installs its
    // subclass. Publishing every 200 ms before installation could race
    // in-process LVM_GETITEMTEXTW validation: all 61 names matched, but the
    // generation changed mid-validation (hook_error=3, zero callbacks).
    // No transition is allowed until Explorer ACKs HOOK_STATE_ATTACHED.
    let mut attached=false;
    let install_wait_started=std::time::Instant::now();
    for poll in 0..60 {
        let (state,error)=unsafe{
            ((*memory.mapped).hook_state.load(Ordering::Acquire),(*memory.mapped).hook_error)
        };
        if state==proto::HOOK_STATE_ATTACHED {
            attached=true;
            if !layout_requested {
                if let Some(ref ready)=ready_signal {
                    ready.store(true,Ordering::Release);
                }
            }
            break;
        }
        if error!=0 {break;}
        if poll>0 && poll%12==0 {
            unsafe {PostMessageW(list,proto::APP_HOOK_INSTALL,proto::HOOK_COOKIE,0);}
        }
        std::thread::sleep(Duration::from_millis(75));
    }
    let hook_install_wait_ms=install_wait_started.elapsed().as_millis() as u64;
    // The GUI test driver must not send a physical mouse button until the
    // actual Explorer subclass is installed. This one-use notice is written
    // only in the explicitly unsafe DEBUG fixture, not by the normal app.
    let mut drag_source_hit_verified=!drag_fixture;
    if drag_fixture && attached {
        let source=&multi_plan.as_ref().unwrap().groups[0];
        let index=source.native_indices[source.active][0];
        // Verify ListView's ACTUAL ICON RECT from within Explorer; using the
        // Shell icon origin +24 can click the wrong native item or background.
        let mut diag_result=0usize;
        let diag_ack=unsafe{SendMessageTimeoutW(list,proto::APP_HOOK_INPUT_INSPECT,
            proto::HOOK_COOKIE,index as isize,0x0002,750,&mut diag_result)};
        let mut native_center=unsafe{POINT{
            x:(*memory.mapped).hook_diag_hover_client_x,
            y:(*memory.mapped).hook_diag_hover_client_y,
        }};
        drag_source_hit_verified=diag_ack!=0 && diag_result==1 &&
            unsafe{ClientToScreen(list,&mut native_center)!=0};
        if let Ok(path)=std::env::var("DESKTOPCRAFT_NATIVE_DRAG_READY_FILE") {
            let item=&native_shell[index];
            let destination=state.group(multi_plan.as_ref().unwrap().groups[1].group_id)
                .expect("Verified native destination Box exists");
            let notice=json!({
                "ready":drag_source_hit_verified,
                "native_list_hwnd":hwnd,
                "native_source_index":index,
                "native_source_path":item.path,
                "native_source_label":item.label,
                "native_source_hit_verified":drag_source_hit_verified,
                "native_hit_test_ack":diag_ack!=0,
                "native_hit_test_result":diag_result,
                "source_icon_origin_physical":[item.pos.0,item.pos.1],
                "source_icon_center_physical":[native_center.x,native_center.y],
                "target_group_id":destination.id,
                "target_bounds_physical":destination.bounds,
                "target_point_physical":[
                    destination.bounds.x+destination.bounds.w/2.0,
                    destination.bounds.y+destination.bounds.h/2.0],
                "real_desktop_files_will_not_move":true,
            });
            let _=std::fs::write(&path,
                serde_json::to_vec_pretty(&notice).unwrap_or_default());
        }
    }
    let mut selfchecks=0u32;
    let mut diagnostic_queries=0u32;
    let mut diagnostic_passes=0u32;
    let mut diagnostic_timeouts=0u32;
    // A real user's organizer cannot silently expire after 15 minutes.
    // The 5-second Explorer shared-memory lease, monitor/PID checks and
    // bounded Tab generations remain the safety brakes.
    let cycles=if real_native_desktop {u32::MAX}
        else if two_box_ui_commands.is_some(){seconds.clamp(8,3600)*5}
        else if staged_hide.is_some(){seconds.clamp(8,30)*5}
        else if input_diagnostic_index.is_some(){seconds.clamp(3,12)*5}
        else if external_commands.is_some(){seconds.clamp(6,7200)*5}
        else if panel_mode.is_some(){seconds.clamp(6,120)*5}
        else if experimental_tab_group.is_some() || multi_plan.is_some(){seconds.clamp(6,30)*5}
        else if experimental_single_index.is_some(){seconds.clamp(1,30)*5}
        else{seconds.clamp(1,12)*5};
    let mut panel = if panel_mode.is_some() {
        let Some(ref set)=tab_set else {return reject("UI native Tab requires a validated Tab set".into())};
        let group=state.groups.iter().find(|g|g.id==set.pair.group_id).unwrap();
        let from=&group.tabs[set.pair.from_tab].name;
        let to=&group.tabs[set.pair.to_tab].name;
        match crate::native_tab_panel::NativeTabPanel::open(from,to,group.id) {
            Ok(panel)=>Some(panel),
            Err(reason)=>return reject(reason),
        }
    }else{None};
    let before=shell;
    let mut stage_expected=before.clone();
    let mut native_layout:Option<LiveLayoutTransaction>=None;
    let mut native_layout_restore_error:Option<String>=None;
    let mut native_layout_generations=0usize;
    if layout_requested && attached {
        match LiveLayoutTransaction::new(&view,pid,&state){
            Ok(layout)=>{native_layout=Some(layout);}
            Err(error)=>{
                // Installing a hook alone has not changed any icon position.
                // No layout mutation may proceed without a durable journal.
                let mut shown=records.clone();
                for record in &mut shown{record.state=proto::STATE_SHOW;}
                unsafe{memory.publish(pid,hwnd,shown.len(),&shown,4,0);}
                drop(controller);
                drop(memory);
                return reject(format!("Native Box layout cannot start: {error}"));
            }
        }
    }
    let initial_monitors=crate::host::monitors();
    let initial_monitor_signature=crate::host::signature(&initial_monitors);
    if initial_monitor_signature.is_empty() {
        drop(panel);
        drop(controller);
        return reject("Native monitor topology is unavailable; native policy refused".into());
    }
    let mut current_stage:Option<u32>=None;
    let mut ui_next_stage=0usize;
    let mut pending_two_box_ack:Option<Sender<Result<usize,String>>>=None;
    let mut hold_workspace_paint=false;
    let mut native_move_event=None;
    let mut extra_bridge_hide:Vec<usize>=Vec::new();
    let mut pending_return_page=0usize;
    let mut ui_accepted_commands=0usize;
    let mut ui_rejected_commands=0usize;
    let mut ui_last_rejection:Option<String>=None;
    let mut ui_graceful_shutdown=false;
    let staged_input_enabled=staged_hide_max==Some(usize::MAX);
    let mut pending_input_stage:Option<usize>=None;
    let mut input_completed_stages=0usize;
    let mut next_user_stage=0u32;
    let mut user_clicked_b=false;
    let mut ui_click_commands=0usize;
    let mut ui_stopped=false;
    let mut stage_events:Vec<Value>=Vec::new();
    let mut abort_reason:Option<String>=if !attached {
        Some(format!("Native Explorer hook failed to attach before policy transition (hook_error={})",
            unsafe{(*memory.mapped).hook_error}))
    }else if !drag_source_hit_verified {
        Some("Native source ListView icon geometry/HITTEST rejected; no real mouse drag permitted".into())
    }else{None};
    // Debug-only fault injection verifies that an unexpected display change
    // aborts an *already attached* native hook without changing Windows
    // display settings or moving icons. Never compiled into Release.
    let forced_monitor_change=if cfg!(debug_assertions) {
        std::env::var("DESKTOPCRAFT_NATIVE_TEST_MONITOR_CHANGE_STEP").ok()
            .and_then(|s|s.parse::<u32>().ok())
    }else{None};
    let mut index_validation_checks=1u32; // full native audit at entry
    let mut drag_armed_announced=false;
    let mut next_generation=4u64; // initial policy was published as generation 2
    // In the real desktop do NOT republish a generation on every idle poll.
    // Keep Explorer's mouse capture on one stable generation through a real
    // right-click/context menu/drag, and refresh only the bounded lease.
    let mut last_published_native_stage:Option<u32>=None;
    let mut drag_preview_group = None;
    // Observe actual Explorer mouse drag/end positions WITHOUT capturing
    // input or redirecting Windows Shell Drag&Drop. Only a single moved icon,
    // whose source click and native identity are independently verified,
    // can become a virtual Pin proposal to the main UI.
    let mut native_pointer=crate::native_pin_drop::NativePointerObserver::default();
    let mut drag_landing: Option<NativeDragLanding> = None;
    let mut tab_press: Option<NativePointerEdge> = None;
    let mut rollup_drag_selection: Option<Vec<String>> = None;
    let mut native_header_gesture: Option<NativeHeaderDrop> = None;
    let mut verified_reorder_plan: Option<(u64, PreparedSameBoxDrop)> = None;
    let mut pending_drop_placement: Option<((i32, i32), NativePlacementPlan)> = None;
    let mut awaiting_real_drop:Option<(
        crate::native_pin_drop::NativeReleasedPointer,
        std::time::Instant,
    )>=None;
    let mut verified_drop_inflight:Option<NativeVerifiedDrop>=None;
    let mut verified_drop_sequence=0u64;
    let mut queued_commands:VecDeque<NativeTwoBoxCommand>=VecDeque::new();
    let mut last_disk_meta=std::fs::metadata(workspace).ok()
        .map(|m|(m.len(),m.modified().ok()));
    let mut last_disk_check=std::time::Instant::now();
    let pointer_tap = if real_native_desktop && real_drop_sender.is_some() {
        ui_channel.as_ref().and_then(|channel| NativePointerEdgeTap::start(list, channel.pointer_wake.clone()))
    } else {
        None
    };
    let mut external_press_active = false;
    let mut last_external_drop: Option<((i32, i32), std::time::Instant)> = None;
    let mut inventory_retry_since: Option<std::time::Instant> = None;
    let mut pending_inventory_added: Vec<ViewItem> = Vec::new();
    let mut pending_inventory_removed: Vec<String> = Vec::new();
    let mut last_view_metrics: Option<((i32, i32), Option<i32>)> = if real_native_desktop {
        view.spacing().map(|sp| (sp, view.icon_size()))
    } else {
        None
    };
    let mut pending_icon_resize: Option<(((i32, i32), Option<i32>), std::time::Instant)> = None;
    let mut deferred_idle_poll = false;
    let mut lease_rebind_required = false;
    for step in 0..cycles {
        let worker_step_started=std::time::Instant::now();
        let mut page_status=None;
        let mut refresh_ghost_icons=false;
        let mut confirm_stage_paint=false;
        if abort_reason.is_some() {break;}
        if stop_signal.as_ref().is_some_and(|stop|stop.load(Ordering::Acquire)){
            if two_box_ui_commands.is_some(){
                ui_graceful_shutdown=true;
            }else{
                abort_reason=Some("Original UI requested native gesture worker shutdown".into());
            }
            break;
        }
        if real_native_desktop && current_stage.is_some()
            && unsafe { lease_gap_requires_rebind(GetTickCount64(), memory.lease_tick().load(Ordering::Acquire)) } {
            lease_rebind_required = true;
            abort_reason = Some("Native policy lease interrupted by suspend or stalled worker; require fresh binding".into());
            break;
        }
        if let Some(ref receiver) = two_box_ui_commands {
            queued_commands.extend(receiver.try_iter().filter_map(NativeWorkerMessage::command));
        }
        let recent_external_drop = last_external_drop
            .is_some_and(|(_, t)| t.elapsed() < Duration::from_secs(5));
        if !recent_external_drop {
            last_external_drop = None;
        }
        let count_changed_early = real_native_desktop
            && current_stage.is_some()
            && live_item_count(list).is_some_and(|n| n != records.len());
        let defer_idle_poll = should_defer_idle_poll(
            real_native_desktop,
            current_stage.is_some(),
            pending_two_box_ack.is_none(),
            !queued_commands.is_empty(),
            inventory_retry_since.is_some()
                || count_changed_early
                || !pending_inventory_removed.is_empty()
                || !pending_inventory_added.is_empty(),
            recent_external_drop,
        );
        // Display connect/disconnect, resolution and desktop-origin changes
        // invalidate hit targets. Fail OPEN by tearing down this filter;
        // never attempt to move an existing Windows Explorer icon.
        if step%5==0 && (monitor_topology_changed(&initial_monitor_signature,
            &crate::host::signature(&crate::host::monitors()))
            || forced_monitor_change.is_some_and(|at|step>=at)) {
            abort_reason=Some("Monitor topology changed during native filter (or debug fault injected); detach and fail open".into());
            break;
        }
        if !defer_idle_poll && two_box_ui_commands.is_some() && (step % 5 == 0 || deferred_idle_poll) {
            if desktop_owner(&view).ok()!=Some(owner) {
                abort_reason=Some("Native two-Box UI lost its original Explorer ListView; fail-open".into());
                break;
            }
        }
        if real_native_desktop && current_stage.is_some() {
            let live_metrics = view.spacing().map(|sp| (sp, view.icon_size()));
            if live_metrics.is_some() && live_metrics != last_view_metrics {
                last_view_metrics = live_metrics;
                pending_icon_resize = live_metrics.map(|m| (m, std::time::Instant::now()));
                awaiting_real_drop = None;
                drag_landing = None;
                tab_press = None;
                rollup_drag_selection = None;
                native_header_gesture = None;
                if let Some(((sp, sz), _)) = pending_icon_resize {
                    crate::host::event(
                        "INFO",
                        "native_desktop.icon_size_change_detected",
                        &format!("spacing={:?} icon_size={:?}", sp, sz),
                    );
                }
            }
        }
        // Observe only the REAL user-facing Explorer icon area. We never
        // acquire mouse capture, send pointer messages, or invoke Shell COM
        // on the UI thread. The worker owns the DesktopView STA instance.
        let mut snapped_desktop_drop = false;
        let mut handled_header_drop = false;
        if real_native_desktop && current_stage.is_some() && pending_two_box_ack.is_none()
            && pending_icon_resize.is_none() && expected_disk_after_drop.is_none()
            && verified_drop_inflight.is_none() && drag_preview_group.is_none()
        {
            if let Some(request) = ui_channel.as_ref().and_then(|channel| channel.header_drops.try_recv().ok())
                .or_else(|| native_header_gesture.take())
            {
                if request.result.is_some() && native_header_gesture.as_ref().is_some_and(|gesture| {
                    gesture.destination == request.destination && gesture.screen == request.screen
                }) {
                    native_header_gesture = None;
                }
                let validation = (|| {
                    let layout = native_layout.as_mut().ok_or("Native placement is unavailable")?;
                    let grabbed = request.paths.first().ok_or("The Tab drop did not contain a file")?;
                    let snapshot = drop_identities.as_ref()
                        .and_then(|_| memory.read_drop_positions(list, native_shell.len()));
                    let batch = drop_identities.as_ref().zip(snapshot.as_deref())
                        .map(|(cache, positions)| (cache, native_shell.as_slice(), positions));
                    let (observed, _, _) = view.items_for_drop(grabbed, layout.expected_icons(), batch, None);
                    let eligible = native_shell.iter().zip(&records)
                        .filter(|(_, record)| record.state == proto::STATE_SHOW)
                        .map(|(item, _)| item.path.to_lowercase()).collect();
                    let prepared = crate::native_pin_drop::prepare_native_header_drop(
                        &state, &observed, &eligible,
                        DesktopCoordinates::current()?.workspace_point(request.screen),
                        request.destination, &request.paths,
                    )?;
                    if !layout.positions_match(&observed) {
                        let moved = crate::native_pin_drop::physical_multi_drop_identity(
                            layout.expected_icons(), &observed, request.paths.len(),
                        )?;
                        if prepared.is_none() || !moved.iter().all(|path| {
                            request.paths.iter().any(|source| source.eq_ignore_ascii_case(path))
                        }) {
                            return Err("Explorer layout changed before the Tab drop".into());
                        }
                        layout.accept_inflight_drop_positions(&observed, &moved)?;
                    }
                    Ok(prepared)
                })();
                let submitted = validation.and_then(|prepared| {
                    if let Some(prepared) = prepared {
                        verified_drop_sequence += 1;
                        let proposal = NativeVerifiedDrop {
                            sequence: verified_drop_sequence,
                            before: state.clone(), next: prepared.next,
                            kind: prepared.kind, source_paths: prepared.source_paths,
                            desktop_drop_targets: Vec::new(),
                        };
                        let sender = real_drop_sender.as_ref().ok_or("Native drag notifications unavailable")?;
                        if let Err(reason) = queue_verified_drop(sender, &mut queued_commands, &proposal) {
                            if let Some(layout) = native_layout.as_mut() {
                                layout.revert_inflight_drop_positions(&view)?;
                            }
                            return Err(reason);
                        }
                        crate::host::event("INFO", "native_desktop.tab_drop_verified", &format!(
                            "sequence={} group={} tab={} items={} source={}", proposal.sequence,
                            request.destination.0, request.destination.1, proposal.source_paths.len(),
                            if request.result.is_some() { "shell" } else { "native_drag" },
                        ));
                        verified_drop_inflight = Some(proposal);
                    }
                    Ok(())
                });
                if submitted.is_ok() {
                    handled_header_drop = true;
                    awaiting_real_drop = None;
                    drag_landing = None;
                    tab_press = None;
                    rollup_drag_selection = None;
                    native_pointer.reset();
                    if let Some(tap) = &pointer_tap { tap.drain(); }
                }
                if let Some(reply) = request.result {
                    if let Err(ref reason) = submitted {
                        crate::host::event(
                            "INFO",
                            "native_desktop.tab_drag_refused",
                            &format!("source=shell reason={reason}"),
                        );
                    }
                    let _ = reply.send(submitted);
                } else if let Err(reason) = submitted {
                    crate::host::event("INFO", "native_desktop.tab_drag_refused", &reason);
                }
            }
        }
        if real_native_desktop && real_drop_sender.is_some() &&
            !handled_header_drop &&
            current_stage.is_some() && pending_two_box_ack.is_none() &&
            pending_icon_resize.is_none() && expected_disk_after_drop.is_none() {
            if let Some(layout) = native_layout.as_ref() {
                if let Some(ref tap) = pointer_tap {
                    let edges = tap.drain();
                    if !edges.is_empty() {
                        let monitors = crate::host::monitors();
                        for edge in edges {
                            let NativePointerEdge { pressed, point, on_list, .. } = edge;
                            if pressed {
                                tab_press = (on_list && edge.allow_tab_drop).then_some(edge);
                                rollup_drag_selection = None;
                                drag_landing = None;
                                external_press_active = !on_list;
                                if on_list {
                                    last_external_drop = None;
                                }
                            } else {
                                let sampled_selection = rollup_drag_selection.take().filter(|paths| !paths.is_empty());
                                if let Some(press) = tab_press.take().filter(|_| {
                                    edge.allow_tab_drop && drag_preview_group.is_none()
                                        && ui_channel.as_ref().is_some_and(|channel| {
                                            edge.window != 0 && edge.window == channel.overlay_hwnd
                                        })
                                }) {
                                    if let Ok(coordinates) = DesktopCoordinates::from_monitors(&monitors) {
                                        if let Some(destination) = crate::native_pin_drop::native_tab_header_at(
                                            &state, coordinates.workspace_point(point),
                                        ) {
                                            let paths = memory.native_drag_paths(
                                                press.tick,
                                                edge.tick,
                                                inventory_generation,
                                                &native_shell,
                                            )
                                                .or_else(|| {
                                                    // Some Explorer drags supply no begin notification. A real
                                                    // ListView press and release may still prove a roll-up Pin drop.
                                                    if !native_pointer.is_dragging(point) { return None; }
                                                    let grabbed = native_pointer.pressed_path()?;
                                                    rollup_header_drag_selection(&state, &current_rolled_overrides,
                                                        destination.0, grabbed, || {
                                                            sampled_selection.or_else(|| {
                                                                view.selected_drop_paths(drop_identities.as_ref()?)
                                                            })
                                                        })
                                                });
                                            if let Some(paths) = paths {
                                                native_header_gesture = Some(NativeHeaderDrop {
                                                    destination, screen: point, paths, result: None,
                                                });
                                            } else {
                                                let (started, drag_gen, cur_gen, count) = memory.native_drag_diag();
                                                crate::host::event(
                                                    "INFO",
                                                    "native_desktop.tab_drag_waiting_for_shell",
                                                    &format!(
                                                        "Pointer reached a Tab without a matching Explorer drag-start notification (started={started} drag_gen={drag_gen} cur_gen={cur_gen} inv_gen={inventory_generation} count={count} press={} release={} dragging={} pressed_hit={})",
                                                        press.tick,
                                                        edge.tick,
                                                        native_pointer.is_dragging(point),
                                                        native_pointer.pressed_path().is_some(),
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                                if external_press_active && on_list {
                                    last_external_drop = Some((point, std::time::Instant::now()));
                                }
                                external_press_active = false;
                            }
                            if let Some(released) = native_pointer.sample_with_monitors(
                                pressed,
                                point,
                                on_list,
                                layout.expected_icons(),
                                &monitors,
                                || {
                                    let eligible = native_shell.iter().zip(&records)
                                        .filter(|(_, record)| record.state == proto::STATE_SHOW)
                                        .map(|(item, _)| item.path.to_lowercase())
                                        .collect();
                                    (eligible, view.spacing().unwrap_or((0, 0)))
                                },
                            ) {
                                if verified_drop_inflight.is_none() {
                                    awaiting_real_drop = Some((released, std::time::Instant::now()));
                                }
                            }
                        }
                    }
                } else if let Some((pressed, point, on_list)) = physical_left_button_and_owner(list) {
                    if pressed && !native_pointer.is_down() {
                        drag_landing = None;
                    }
                    let monitors = crate::host::monitors();
                    if let Some(released) = native_pointer.sample_with_monitors(
                        pressed,
                        point,
                        on_list,
                        layout.expected_icons(),
                        &monitors,
                        || {
                            let eligible = native_shell.iter().zip(&records)
                                .filter(|(_, record)| record.state == proto::STATE_SHOW)
                                .map(|(item, _)| item.path.to_lowercase())
                                .collect();
                            (eligible, view.spacing().unwrap_or((0, 0)))
                        },
                    ) {
                        if verified_drop_inflight.is_none() {
                            awaiting_real_drop = Some((released, std::time::Instant::now()));
                        }
                    }
                }
                if native_pointer.is_down() && crate::host::mouse_left_down() {
                    let cursor = crate::host::cursor();
                    if native_pointer.is_dragging(cursor) {
                        if tab_press.is_some() && rollup_drag_selection.is_none() {
                            // Keep the actual selected set if Explorer clears selection on release.
                            rollup_drag_selection = Some((|| {
                                let grabbed = native_pointer.pressed_path()?;
                                let (source, _, _) = crate::native_pin_drop::visible_regular_pin_owner(&state, grabbed)?;
                                let has_rolled_group = state.groups.iter().any(|g| g.page == state.page && g.rolled);
                                if !current_rolled_overrides.contains(&(source, true)) && !has_rolled_group {
                                    return None;
                                }
                                view.selected_drop_paths(drop_identities.as_ref()?)
                                    .filter(|paths| paths.iter().any(|path| path.eq_ignore_ascii_case(grabbed)))
                            })().unwrap_or_default());
                        }
                        if drag_landing.as_ref().is_none_or(|landing| landing.generation != next_generation) {
                            let eligible = native_shell.iter().zip(&records)
                                .filter(|(_, record)| record.state == proto::STATE_SHOW)
                                .map(|(icon, _)| icon.path.to_lowercase()).collect();
                            let spacing = last_view_metrics.map(|(spacing, _)| spacing).unwrap_or((0, 0));
                            drag_landing = NativeDragLanding::new(
                                next_generation, &state, layout.expected_icons(), spacing, eligible,
                            ).ok();
                        }
                        if let Some(landing) = drag_landing.as_mut() {
                            landing.sample(&state, cursor);
                            if let (Some(grabbed), Some(identities), Some(member)) = (
                                native_pointer.pressed_path(), drop_identities.as_ref(), ui_membership.as_ref(),
                            ) {
                                landing.prepare_same_box(
                                    &state, grabbed, member.generation + 1, layout, &native_shell,
                                    &native_original_order, || view.selected_drop_paths(identities),
                                );
                            }
                        }
                    }
                }
            }
            if native_header_gesture.is_some() && verified_drop_inflight.is_none() && drag_preview_group.is_none() {
                continue;
            }
            if let Some((released,when))=awaiting_real_drop.clone() {
                if when.elapsed()>Duration::from_secs(2) {
                    awaiting_real_drop=None;
                } else if let Some(layout)=native_layout.as_mut() {
                    // A genuine desktop icon move can be observed only
                    // after Explorer finishes the physical drag; wait for
                    // that one position update, bounded to two seconds.
                    let observation_started = std::time::Instant::now();
                    let spacing = last_view_metrics.map(|(spacing, _)| spacing).unwrap_or((0, 0));
                    let fast_geometry = drag_landing.as_ref().and_then(|landing| {
                        if landing.same_box.is_none()
                            || landing.generation != next_generation
                        {
                            return None;
                        }
                        let slot = landing.slot_at(&state, released.up);
                        if slot.as_ref().ok().copied() != Some(landing.planned_slot) {
                            return None;
                        }
                        let monitors = crate::host::monitors();
                        landing.matches(next_generation, layout.expected_icons(), spacing, &monitors)
                            .then_some((monitors, slot))
                    });
                    let batch_positions = drop_identities.as_ref()
                        .and_then(|_| memory.read_drop_positions(list, native_shell.len()));
                    let batch = drop_identities.as_ref().zip(batch_positions.as_deref())
                        .map(|(cache, positions)| (cache, native_shell.as_slice(), positions));
                    let cached_shell = fast_geometry.as_ref().and(drop_identities.as_ref());
                    let (observed, place_loose_drop, observation_source) = view.items_for_drop(
                        &released.path, layout.expected_icons(), batch, cached_shell,
                    );
                    let observation_us = observation_started.elapsed().as_micros();
                    let no_movement = layout.positions_match(&observed);
                    if !no_movement {
                        let expected = layout.expected_icons().to_vec();
                        let (monitors, release_slot) = match fast_geometry {
                            Some((monitors, slot)) => (monitors, Some(slot)),
                            None => (crate::host::monitors(), None),
                        };
                        let mut landing = drag_landing.take().filter(|landing| {
                            landing.matches(next_generation, &expected, spacing, &monitors)
                        });
                        let fallback_eligible;
                        let eligible = if let Some(landing) = &landing {
                            &landing.eligible
                        } else {
                            fallback_eligible = native_shell.iter().zip(&records)
                                .filter(|(_, record)| record.state == proto::STATE_SHOW)
                                .map(|(icon, _)| icon.path.to_lowercase()).collect();
                            &fallback_eligible
                        };
                        let (displaced_parked, observed_for_drop) =
                            layout.filter_displaced_hidden_park(&observed, eligible);
                        let coordinates = DesktopCoordinates::from_monitors(&monitors);
                        let cached_slot = landing.as_ref().map(|landing| {
                            (&landing.plan, release_slot.as_ref().or_else(|| landing.slot_sample.as_ref()
                                .filter(|(point, _)| *point == released.up)
                                .map(|(_, slot)| slot)))
                        });
                        let checked=(||->Result<_,String>{
                            let planned=coordinates.clone()
                                .and_then(|coordinates|crate::native_pin_drop::prepare_native_multi_drop_with_slot(
                                    &state,&expected,&observed_for_drop,eligible,
                                    released.down,released.up,spacing,coordinates,&monitors,cached_slot))
                                .and_then(|prepared| {
                                    if prepared.source_paths.len() > crate::native_pin_drop::MAX_MULTI_DRAG_ICONS
                                        && !prepared.source_paths.iter().any(|p| p.eq_ignore_ascii_case(&released.path))
                                    {
                                        return Err("Large Explorer drag did not move the icon grabbed by the mouse".into());
                                    }
                                    Ok(prepared)
                                });
                            if let Ok(ref prepared) = planned {
                                if prepared.next == state
                                {
                                    let mut restore_paths = prepared.source_paths.clone();
                                    for p in &displaced_parked {
                                        if !restore_paths.iter().any(|existing| existing.eq_ignore_ascii_case(p)) {
                                            restore_paths.push(p.clone());
                                        }
                                    }
                                    layout.restore_verified_multi_drag(
                                        &view,&expected,&observed,&restore_paths)?;
                                    if !displaced_parked.is_empty() {
                                        crate::host::event(
                                            "INFO",
                                            "native_desktop.hidden_park_reseated",
                                            &format!("count={} during=drop_restore", displaced_parked.len()),
                                        );
                                    }
                                } else {
                                    // Keep the dropped icon(s) where the user
                                    // released them so direct reflow places them
                                    // without bouncing back first.
                                    if !displaced_parked.is_empty() {
                                        layout.repark_displaced_hidden_icons(&view, &displaced_parked)?;
                                        crate::host::event(
                                            "INFO",
                                            "native_desktop.hidden_park_reseated",
                                            &format!("count={} during=drop_accept", displaced_parked.len()),
                                        );
                                    }
                                    layout.accept_inflight_drop_positions(
                                        &observed_for_drop,
                                        &prepared.source_paths,
                                    )?;
                                }
                            } else {
                                // Only a refused drop needs a repair audit.
                                let mut subset = Vec::new();
                                if observed_for_drop.len() == expected.len() {
                                    let expected_map = expected.iter()
                                        .map(|icon| (icon.path.to_lowercase(), icon))
                                        .collect::<HashMap<_, _>>();
                                    if expected_map.len() == expected.len()
                                        && observed_for_drop.iter().all(|item| {
                                            expected_map.get(&item.path.to_lowercase())
                                                .is_some_and(|old| old.label == item.label)
                                        })
                                    {
                                        subset.extend(observed_for_drop.iter().filter(|item| {
                                            expected_map[&item.path.to_lowercase()].pos != item.pos
                                                && layout.placed_by_app(&item.path)
                                        }).map(|item| item.path.clone()));
                                    }
                                }
                                for p in &displaced_parked {
                                    if !subset.iter().any(|existing| existing.eq_ignore_ascii_case(p)) {
                                        subset.push(p.clone());
                                    }
                                }
                                if !subset.is_empty() {
                                    layout.restore_expected_subset(&view, &subset)?;
                                    if !displaced_parked.is_empty() {
                                        crate::host::event(
                                            "INFO",
                                            "native_desktop.hidden_park_reseated",
                                            &format!("count={} during=refused_drop", displaced_parked.len()),
                                        );
                                    }
                                }
                            }
                            planned
                        })();
                        awaiting_real_drop=None;
                        match checked {
                            Ok(prepared)=>{
                                if prepared.next == state {
                                    // Returning to the same slot restores the grid with no write.
                                    crate::host::event("INFO",
                                        "native_desktop.same_box_drag_restored",
                                        "Dropped in the same native grid order");
                                    continue;
                                }
                                if verified_drop_inflight.is_some() {
                                    let _ = layout.revert_inflight_drop_positions(&view);
                                    abort_reason=Some(
                                        "Second Explorer icon drag arrived before first native Pin ACK".into());
                                    break;
                                }
                                let desktop_drop_targets = if matches!(
                                    prepared.kind,
                                    crate::native_pin_drop::NativeMultiDropKind::BoxToDesktop { .. }
                                ) {
                                    prepared.source_paths.iter().filter_map(|p| {
                                        observed.iter()
                                            .find(|item| item.path.eq_ignore_ascii_case(p))
                                            .map(|item| (item.path.clone(), item.pos))
                                    }).collect()
                                } else {
                                    Vec::new()
                                };
                                verified_drop_sequence+=1;
                                if matches!(prepared.kind, crate::native_pin_drop::NativeMultiDropKind::SameBox { .. }) {
                                    let cached = landing.as_mut().and_then(|landing| landing.same_box.take())
                                        .filter(|plan| plan.next == prepared.next);
                                    if let Some(plan) = cached {
                                        verified_reorder_plan = Some((verified_drop_sequence, plan));
                                    }
                                    crate::host::event("INFO", "native_desktop.same_box_drop_prepared",
                                        &format!("sequence={verified_drop_sequence} observation_us={observation_us} source={observation_source} reused_placement={}",
                                            verified_reorder_plan.is_some()));
                                }
                                let proposal=NativeVerifiedDrop{
                                    sequence:verified_drop_sequence,
                                    before:state.clone(),
                                    next:prepared.next,
                                    kind:prepared.kind,
                                    source_paths:prepared.source_paths,
                                    desktop_drop_targets,
                                };
                                if let Err(reason) = queue_verified_drop(
                                    real_drop_sender.as_ref().unwrap(), &mut queued_commands, &proposal,
                                ) {
                                    let _ = layout.revert_inflight_drop_positions(&view);
                                    abort_reason=Some(reason);
                                    break;
                                }
                                crate::host::event("INFO","native_desktop.drag_verified",
                                    &format!("sequence={} action={:?}; physical drop kept in place for direct native reflow",
                                        proposal.sequence,proposal.kind));
                                verified_drop_inflight=Some(proposal);
                            }
                            Err(reason)=>{
                                if reason == crate::native_pin_drop::LOOSE_DESKTOP_DROP
                                    && displaced_parked.is_empty() && !crate::host::mouse_left_down()
                                {
                                    let result = coordinates
                                        .and_then(|coords| {
                                            let fallback_rects;
                                            let active_boxes = if let Some(landing) = &landing {
                                                landing.active_box_rects.as_slice()
                                            } else {
                                                fallback_rects = coords.active_box_rects(&state);
                                                fallback_rects.as_slice()
                                            };
                                            layout.reconcile_loose_drop(
                                                &view, active_boxes, observed, Some((spacing, &monitors)),
                                                place_loose_drop,
                                            )
                                        });
                                    match result {
                                        Ok((adopted, _)) => {
                                            snapped_desktop_drop = adopted;
                                            if adopted {
                                                refresh_ghost_icons = true;
                                            }
                                            crate::host::event("INFO", "native_desktop.desktop_drop_applied",
                                                &format!("elapsed_ms={} observation_us={observation_us} source={observation_source} positions_adopted={adopted}", when.elapsed().as_millis()));
                                        }
                                        Err(reason) => {
                                            abort_reason = Some(reason);
                                            break;
                                        }
                                    }
                                    // No Workspace membership changes for a loose-to-loose drop.
                                } else {
                                // Leave `reconcile_inventory` below to adopt
                                // loose Desktop drags or dynamic inventory deltas
                                // cleanly in one pass without aborting on loose
                                // drags out to external folders or Trash.
                                    crate::host::event("INFO","native_desktop.drop_refused",&reason);
                                }
                            }
                        }
                    }
                }
            }
        }
        // The native GUI writes one verified membership transaction ONLY
        // after Explorer ACK. Refresh our expected on-disk source bytes when
        // they exactly match the worker-approved State; other concurrent
        // Workspace edits remain an immediate fail-open abort.
        if real_native_desktop && pending_two_box_ack.is_none() {
            let meta_now=std::fs::metadata(workspace).ok().map(|m|(m.len(),m.modified().ok()));
            if expected_disk_after_drop.is_some()
                || meta_now!=last_disk_meta
                || last_disk_check.elapsed()>=Duration::from_secs(2)
            {
                last_disk_meta=meta_now;
                last_disk_check=std::time::Instant::now();
                if let Err(reason)=sync_expected_workspace_disk(
                    workspace,
                    &mut workspace_before,
                    &mut persisted_disk_state,
                    &mut expected_disk_after_drop,
                    &state,
                    &current_rolled_overrides,
                ) {
                    abort_reason=Some(reason);
                    break;
                }
            }
        }
        if current_stage.is_some()
            && awaiting_real_drop.is_none()
            && verified_drop_inflight.is_none()
            && pending_icon_resize.is_none()
            && !native_pointer.is_down()
            && !pointer_tap.as_ref().is_some_and(NativePointerEdgeTap::has_pending_or_pressed)
        {
            let mut handled_inventory_change = false;
            let count_changed = count_changed_early
                || (real_native_desktop && live_item_count(list).is_some_and(|n| n != records.len()));
            let has_pending_inventory = inventory_retry_since.is_some()
                || !pending_inventory_removed.is_empty()
                || !pending_inventory_added.is_empty();
            let poll_due = real_native_desktop
                && !snapped_desktop_drop
                && pending_two_box_ack.is_none()
                && (count_changed
                    || step % 2 == 0
                    || step % 5 == 0
                    || recent_external_drop
                    || has_pending_inventory
                    || deferred_idle_poll);
            if poll_due && defer_idle_poll {
                deferred_idle_poll = true;
            } else if poll_due {
                deferred_idle_poll = false;
                if let Some(layout) = native_layout.as_mut() {
                    let probe = view.items();
                    if layout.positions_match(&probe)
                        && probe.len() == records.len()
                        && !has_pending_inventory
                    {
                        inventory_retry_since = None;
                    } else {
                        let exp = layout.expected_icons();
                        let same_inventory = probe.len() == exp.len() && {
                            let probe_keys = probe.iter()
                                .map(|i| i.path.to_lowercase())
                                .collect::<std::collections::BTreeSet<_>>();
                            probe_keys.len() == probe.len()
                                && exp.iter().all(|i| probe_keys.contains(&i.path.to_lowercase()))
                        };
                        if !same_inventory || probe.len() != records.len() || has_pending_inventory {
                            handled_inventory_change = true;
                            let read_inventory = || {
                                if real_drop_sender.is_some() && cfg!(feature = "native-fast-tabs") {
                                    view.items_with_drop_identities()
                                } else {
                                    (view.items(), None)
                                }
                            };
                            let (mut fresh_items, mut fresh_identities) = read_inventory();
                            let mut fresh_names: Vec<String> = list_items(list).into_iter()
                                .map(|i| i.name)
                                .collect();
                            if explorer_native::native_view_in_msaa_order(&fresh_items, &fresh_names).is_err() {
                                if records.iter().any(|r| r.state == proto::STATE_HIDE)
                                    && fresh_names.len() == fresh_items.len()
                                    && fresh_names.iter().any(String::is_empty)
                                {
                                    let mut unmasked = records.clone();
                                    for r in &mut unmasked {
                                        r.state = proto::STATE_SHOW;
                                    }
                                    unsafe {
                                        memory.publish(pid, hwnd, unmasked.len(), &unmasked, next_generation, 0);
                                        next_generation += 2;
                                        InvalidateRect(list, ptr::null(), 0);
                                    }
                                    std::thread::sleep(Duration::from_millis(30));
                                    (fresh_items, fresh_identities) = read_inventory();
                                    fresh_names = list_items(list).into_iter()
                                        .map(|i| i.name)
                                        .collect();
                                }
                                if explorer_native::native_view_in_msaa_order(&fresh_items, &fresh_names).is_err() {
                                    let started = *inventory_retry_since.get_or_insert_with(std::time::Instant::now);
                                    if started.elapsed() < Duration::from_secs(3) {
                                        if lease_gap_requires_rebind(unsafe { GetTickCount64() },
                                        unsafe { memory.lease_tick().load(Ordering::Acquire) }) {
                                        lease_rebind_required = true;
                                        abort_reason = Some("Native policy lease interrupted during inventory refresh; require fresh binding".into());
                                        break;
                                    }
                                    unsafe { memory.renew_lease(); }
                                        std::thread::sleep(Duration::from_millis(50));
                                        continue;
                                    }
                                    abort_reason = Some("Native inventory refresh MSAA names did not match Explorer view".into());
                                    break;
                                }
                            }
                            let active_boxes = crate::native_box_live_layout::DesktopCoordinates::current()
                                .map(|coords| state.groups.iter()
                                    .filter(|g| g.page == state.page && !g.rolled)
                                    .map(|g| {
                                        let b = if g.tabs.get(g.active).is_some_and(|t| t.folder.is_none()) {
                                            crate::paint::native_group_body(g)
                                        } else {
                                            g.bounds
                                        };
                                        coords.screen_rect(b)
                                    })
                                    .collect::<Vec<_>>())
                                .unwrap_or_default();
                            let external_drop_in_box = last_external_drop.as_ref()
                                .filter(|(_, t)| t.elapsed() < Duration::from_secs(5))
                                .and_then(|(pt, _)| crate::native_box_live_layout::DesktopCoordinates::current()
                                    .ok()
                                    .map(|coords| coords.workspace_point(*pt)))
                                .is_some_and(|(wx, wy)| state.groups.iter().any(|g| {
                                    g.page == state.page && !g.rolled && {
                                        let b = g.bounds;
                                        wx >= b.x + 8. && wx < b.x + b.w - 8. && wy >= b.y + 46. && wy < b.y + b.h - 8.
                                    }
                                }));
                            let outcome = match layout.reconcile_inventory(
                                &view,
                                &active_boxes,
                                &state,
                                external_drop_in_box,
                            ) {
                                Ok(o) => o,
                                Err(reason) => {
                                    abort_reason = Some(reason);
                                    break;
                                }
                            };
                            merge_inventory_deltas(
                                &mut pending_inventory_added,
                                &mut pending_inventory_removed,
                                &outcome,
                            );
                            let (settled_items, settled_names, settled_identities) = if layout.positions_match(&fresh_items) {
                                (fresh_items, fresh_names, fresh_identities)
                            } else {
                                let (items, identities) = read_inventory();
                                (
                                    items,
                                    list_items(list).into_iter().map(|i| i.name).collect::<Vec<_>>(),
                                    identities,
                                )
                            };
                            let mapped = match explorer_native::native_view_in_msaa_order(
                                &settled_items, &settled_names,
                            ) {
                                Ok(m) => m,
                                Err(reason) => {
                                    let started = *inventory_retry_since.get_or_insert_with(std::time::Instant::now);
                                    if started.elapsed() < Duration::from_secs(3) {
                                        if lease_gap_requires_rebind(unsafe { GetTickCount64() },
                                        unsafe { memory.lease_tick().load(Ordering::Acquire) }) {
                                        lease_rebind_required = true;
                                        abort_reason = Some("Native policy lease interrupted during inventory refresh; require fresh binding".into());
                                        break;
                                    }
                                    unsafe { memory.renew_lease(); }
                                        std::thread::sleep(Duration::from_millis(50));
                                        continue;
                                    }
                                    abort_reason = Some(format!(
                                        "Native inventory refresh MSAA order mismatch: {reason}"
                                    ));
                                    break;
                                }
                            };
                            let prev_gen = ui_membership.as_ref().map_or(1, |m| m.generation);
                            let new_member = match crate::native_live_membership::NativeTwoBoxMembership
                                ::prepare_workspace(prev_gen + 1, &state, &mapped)
                            {
                                Ok(m) => m,
                                Err(reason) => {
                                    abort_reason = Some(format!(
                                        "Native inventory refresh membership refused: {reason}"
                                    ));
                                    break;
                                }
                            };
                            let Some(cell) = view.spacing() else {
                                abort_reason = Some("Explorer icon spacing unavailable after inventory refresh".into());
                                break;
                            };
                            let new_hidden = match layout.planned_visibility(&new_member, &state, &mapped, cell) {
                                Ok((_, h, _, _)) => h,
                                Err(reason) => {
                                    abort_reason = Some(format!(
                                        "Native inventory refresh visibility refused: {reason}"
                                    ));
                                    break;
                                }
                            };
                            if new_hidden.len() > native_hide_cap {
                                abort_reason = Some("Native inventory refresh exceeded hidden-icon safety cap".into());
                                break;
                            }
                            let publish_hidden = inventory_refresh_publish_hidden(
                                &new_hidden,
                                &mapped,
                                |path| layout.is_parked(path),
                            );
                            if publish_hidden.len() > native_hide_cap {
                                abort_reason = Some("Native inventory refresh exceeded hidden-icon safety cap".into());
                                break;
                            }
                            let (needs_post_remove_reflow, post_remove_bridge) = if !pending_inventory_removed.is_empty() {
                                inventory_refresh_removed_affects_active_box(
                                    &state,
                                    &pending_inventory_removed,
                                    &mapped,
                                    &new_hidden,
                                    |path| layout.is_parked(path),
                                )
                            } else {
                                (false, Vec::new())
                            };
                            let Some(mut new_records) = proto::verified_records(
                                mapped.iter().enumerate().map(|(index, item)| {
                                    (index, item.label.as_str(), publish_hidden.binary_search(&index).is_ok())
                                }),
                                mapped.len(),
                            ) else {
                                abort_reason = Some("Native inventory refresh records invalid".into());
                                break;
                            };
                            if let Some(group) = drag_preview_group {
                                if let Some(&(_,tab)) = new_member.selected_tabs.iter()
                                    .find(|(id,_)|*id==group) {
                                    for &index in new_member.tab_indices(group,tab).unwrap_or_default() {
                                        if let Some(record)=new_records.get_mut(index) {record.state=proto::STATE_HIDE;}
                                    }
                                }
                            }
                            // Wait for transient multi-item add/delete bursts to
                            // settle BEFORE publishing a new generation so the DLL
                            // keeps verified label masking alive throughout.
                            if real_native_desktop && attached && live_item_count(list) != Some(new_records.len()) {
                                let started = *inventory_retry_since.get_or_insert_with(std::time::Instant::now);
                                if started.elapsed() < Duration::from_secs(3) {
                                    if lease_gap_requires_rebind(unsafe { GetTickCount64() },
                                        unsafe { memory.lease_tick().load(Ordering::Acquire) }) {
                                        lease_rebind_required = true;
                                        abort_reason = Some("Native policy lease interrupted during inventory refresh; require fresh binding".into());
                                        break;
                                    }
                                    unsafe { memory.renew_lease(); }
                                    std::thread::sleep(Duration::from_millis(25));
                                    continue;
                                }
                                abort_reason = Some("Explorer icon count kept changing during native inventory refresh".into());
                                break;
                            }
                            let queue_post_remove_reflow = needs_post_remove_reflow
                                && real_native_desktop
                                && staged_hide.is_some()
                                && staged_tab_selections.is_some()
                                && verified_drop_inflight.is_none()
                                && pending_two_box_ack.is_none()
                                && drag_preview_group.is_none();
                            unsafe {
                                memory.publish(pid, hwnd, new_records.len(), &new_records, next_generation, flags);
                                next_generation += 2;
                            }
                            if real_native_desktop && attached {
                                let confirm_msg = if queue_post_remove_reflow {
                                    proto::APP_HOOK_APPLY_POLICY
                                } else {
                                    proto::APP_HOOK_PAINT_POLICY
                                };
                                if let Err(reason) = memory.confirm_policy(list, confirm_msg) {
                                    if live_item_count(list) != Some(new_records.len()) {
                                        let started = *inventory_retry_since.get_or_insert_with(std::time::Instant::now);
                                        if started.elapsed() < Duration::from_secs(3) {
                                            // Restore the last committed records matching `native_shell`
                                            // past the unconfirmed generation so `drag_generation_is_compatible`
                                            // can never accept `mapped`'s indices against stale `native_shell`.
                                            unsafe {
                                                memory.publish(pid, hwnd, records.len(), &records, next_generation, flags);
                                                next_generation += 2;
                                                inventory_generation = memory.generation().load(Ordering::Acquire);
                                            }
                                            std::thread::sleep(Duration::from_millis(25));
                                            continue;
                                        }
                                    }
                                    abort_reason = Some(reason);
                                    break;
                                }
                            } else {
                                unsafe { InvalidateRect(list, ptr::null(), 0); }
                            }
                            names = settled_names;
                            native_shell = mapped.clone();
                            native_original_order = mapped.clone();
                            stage_expected = mapped.clone();
                            drop_identities = settled_identities;
                            inventory_generation = unsafe { memory.generation().load(Ordering::Acquire) };
                            records = new_records;
                            ui_membership = Some(new_member.clone());
                            refresh_ghost_icons=true;
                            if let (Some(stages), Some(st_idx)) = (staged_hide.as_mut(), current_stage) {
                                if let Some(slot) = stages.get_mut(st_idx as usize) {
                                    *slot = publish_hidden;
                                }
                            }
                            last_published_native_stage = current_stage;
                            inventory_retry_since = None;
                            let refresh_added = std::mem::take(&mut pending_inventory_added);
                            let refresh_removed = std::mem::take(&mut pending_inventory_removed);
                            crate::host::event(
                                "INFO",
                                "native_desktop.inventory_refreshed",
                                &format!(
                                    "added={} removed={} total={}",
                                    refresh_added.len(),
                                    refresh_removed.len(),
                                    records.len(),
                                ),
                            );
                            if queue_post_remove_reflow {
                                hold_workspace_paint = true;
                                native_move_event = if cfg!(feature = "native-fast-tabs") {
                                    Some("native_desktop.delete_reflow_move_batch")
                                } else {
                                    None
                                };
                                extra_bridge_hide = post_remove_bridge;
                                staged_hide.as_mut().unwrap().push(new_hidden);
                                staged_tab_selections.as_mut().unwrap().push(new_member.selected_tabs);
                                ui_next_stage = staged_hide.as_ref().unwrap().len() - 1;
                                crate::host::event(
                                    "INFO",
                                    "native_desktop.delete_reflow_queued",
                                    &format!(
                                        "removed={} bridge_hidden={}",
                                        refresh_removed.len(),
                                        extra_bridge_hide.len(),
                                    ),
                                );
                            }
                            if !refresh_added.is_empty()
                                && verified_drop_inflight.is_none()
                                && expected_disk_after_drop.is_none()
                                && real_drop_sender.is_some()
                            {
                                let drop_pt = last_external_drop
                                    .take()
                                    .filter(|(_, t)| t.elapsed() < Duration::from_secs(5))
                                    .map(|(pt, _)| pt);
                                let ext = crate::native_box_live_layout::DesktopCoordinates::current()
                                    .and_then(|coords| crate::native_pin_drop::prepare_external_desktop_add_drop(
                                        &state,
                                        &native_shell,
                                        &refresh_added,
                                        drop_pt,
                                        coords,
                                    ));
                                match ext {
                                    Ok(Some(prepared)) => {
                                        verified_drop_sequence += 1;
                                        let proposal = NativeVerifiedDrop {
                                            sequence: verified_drop_sequence,
                                            before: state.clone(),
                                            next: prepared.next,
                                            kind: prepared.kind,
                                            source_paths: prepared.source_paths,
                                            desktop_drop_targets: Vec::new(),
                                        };
                                        if let Err(reason) = queue_verified_drop(
                                            real_drop_sender.as_ref().unwrap(), &mut queued_commands, &proposal,
                                        ) {
                                            abort_reason = Some(reason);
                                            break;
                                        }
                                        crate::host::event(
                                            "INFO",
                                            "native_desktop.external_drop_verified",
                                            &format!(
                                                "sequence={} action={:?}",
                                                proposal.sequence, proposal.kind
                                            ),
                                        );
                                        verified_drop_inflight = Some(proposal);
                                    }
                                    Ok(None) => {
                                        crate::host::event(
                                            "INFO",
                                            "native_desktop.external_add_kept_loose",
                                            &format!("count={}", refresh_added.len()),
                                        );
                                    }
                                    Err(reason) => {
                                        crate::host::event(
                                            "INFO",
                                            "native_desktop.external_add_kept_loose",
                                            &reason,
                                        );
                                    }
                                }
                            }
                        } else {
                            handled_inventory_change = true;
                            let active_boxes = crate::native_box_live_layout::DesktopCoordinates::current()
                                .map(|coords| state.groups.iter()
                                    .filter(|g| g.page == state.page && !g.rolled)
                                    .map(|g| {
                                        let b = if g.tabs.get(g.active).is_some_and(|t| t.folder.is_none()) {
                                            crate::paint::native_group_body(g)
                                        } else {
                                            g.bounds
                                        };
                                        coords.screen_rect(b)
                                    })
                                    .collect::<Vec<_>>())
                                .unwrap_or_default();
                            if let Err(reason) = layout.reconcile_observed_loose_positions(&view, &active_boxes, probe) {
                                abort_reason = Some(reason);
                                break;
                            }
                        }
                    }
                }
            }
            if !handled_inventory_change {
                if let Some(layout) = native_layout.as_mut() {
                    if layout.scaled_recheck_due_in().is_some_and(|due| due.is_zero()) {
                        if let Err(reason) = layout.verify_delayed_scaled_settle(&view, &state) {
                            abort_reason = Some(reason);
                            break;
                        }
                    }
                    if !real_native_desktop && step % 5 == 0 {
                        let active_boxes = crate::native_box_live_layout::DesktopCoordinates::current()
                            .map(|coords| state.groups.iter()
                                .filter(|g| g.page == state.page && !g.rolled)
                                .map(|g| coords.screen_rect(g.bounds))
                                .collect::<Vec<_>>())
                            .unwrap_or_default();
                        if let Err(reason) = layout.reconcile_loose_positions(&view, &active_boxes) {
                            abort_reason = Some(reason);
                            break;
                        }
                    }
                }
            }
        }
        if real_native_desktop
            && current_stage.is_some()
            && !native_pointer.is_down()
            && !pointer_tap.as_ref().is_some_and(NativePointerEdgeTap::has_pending_or_pressed)
        {
            if let Some(((new_cell, new_icon_size), since)) = pending_icon_resize {
                if since.elapsed() >= Duration::from_millis(280) {
                    let confirm_metrics = view.spacing().map(|sp| (sp, view.icon_size()));
                    if confirm_metrics != Some((new_cell, new_icon_size)) {
                        if confirm_metrics.is_some() {
                            last_view_metrics = confirm_metrics;
                            pending_icon_resize =
                                confirm_metrics.map(|m| (m, std::time::Instant::now()));
                        }
                    } else {
                        pending_icon_resize = None;
                        if let (Some(layout), Some(member)) =
                            (native_layout.as_mut(), ui_membership.as_ref())
                        {
                            match layout.adopt_explorer_spacing_change(&view, new_cell) {
                                Ok(adopted) => {
                                    let pos_map = adopted
                                        .iter()
                                        .map(|item| (item.path.to_lowercase(), item.pos))
                                        .collect::<HashMap<_, _>>();
                                    for item in native_shell.iter_mut().chain(stage_expected.iter_mut()) {
                                        if let Some(&pos) = pos_map.get(&item.path.to_lowercase()) {
                                            item.pos = pos;
                                        }
                                    }
                                    match layout.planned_visibility(
                                        member,
                                        &state,
                                        &native_original_order,
                                        new_cell,
                                    ) {
                                        Ok((_, new_hidden, _, _))
                                            if new_hidden.len() <= native_hide_cap =>
                                        {
                                            if let (Some(stages), Some(tab_sels)) =
                                                (staged_hide.as_mut(), staged_tab_selections.as_mut())
                                            {
                                                if (pending_two_box_ack.is_some()
                                                    || current_stage.is_some_and(|cur| cur as usize != ui_next_stage))
                                                    && ui_next_stage < stages.len()
                                                {
                                                    stages[ui_next_stage] = new_hidden;
                                                } else {
                                                    stages.push(new_hidden);
                                                    tab_sels.push(member.selected_tabs.clone());
                                                    ui_next_stage = stages.len() - 1;
                                                }
                                                crate::host::event(
                                                    "INFO",
                                                    "native_desktop.icon_size_reflow_queued",
                                                    &format!(
                                                        "spacing={:?} icon_size={:?}",
                                                        new_cell, new_icon_size
                                                    ),
                                                );
                                            }
                                        }
                                        Ok((_, new_hidden, _, _)) => {
                                            if pending_two_box_ack.is_some() {
                                                abort_reason = Some("Explorer zoom left no verified layout for the pending command".into());
                                                break;
                                            }
                                            cancel_pending_background_reflow(
                                                &mut staged_hide,
                                                &mut staged_tab_selections,
                                                current_stage,
                                                &mut ui_next_stage,
                                                &mut hold_workspace_paint,
                                                &mut native_move_event,
                                                &mut extra_bridge_hide,
                                            );
                                            crate::host::event(
                                                "WARN",
                                                "native_desktop.icon_size_reflow_deferred",
                                                &format!(
                                                    "hidden count {} exceeds {} cap after icon resize",
                                                    new_hidden.len(),
                                                    native_hide_cap
                                                ),
                                            );
                                            let _ = layout.persist_current_checkpoint(&state);
                                            if let Some(channel) = ui_channel.as_ref() {
                                                let _ = channel.pages.send(NativeLayoutSync {
                                                    pages: None,
                                                    ghost_icons: Some(HashMap::new()),
                                                    visible_loose_rects: Some(compute_visible_loose_rects(
                                                        layout.expected_icons(),
                                                        &state,
                                                        new_cell,
                                                    )),
                                                    main_cell: new_cell,
                                                    icon_size: new_icon_size.unwrap_or(48),
                                                    snap_to_grid: layout.snap_to_grid(),
                                                });
                                            }
                                        }
                                        Err(reason) => {
                                            if pending_two_box_ack.is_some() {
                                                abort_reason = Some(reason);
                                                break;
                                            }
                                            cancel_pending_background_reflow(
                                                &mut staged_hide,
                                                &mut staged_tab_selections,
                                                current_stage,
                                                &mut ui_next_stage,
                                                &mut hold_workspace_paint,
                                                &mut native_move_event,
                                                &mut extra_bridge_hide,
                                            );
                                            crate::host::event(
                                                "WARN",
                                                "native_desktop.icon_size_reflow_deferred",
                                                &reason,
                                            );
                                            let _ = layout.persist_current_checkpoint(&state);
                                            if let Some(channel) = ui_channel.as_ref() {
                                                let _ = channel.pages.send(NativeLayoutSync {
                                                    pages: None,
                                                    ghost_icons: Some(HashMap::new()),
                                                    visible_loose_rects: Some(compute_visible_loose_rects(
                                                        layout.expected_icons(),
                                                        &state,
                                                        new_cell,
                                                    )),
                                                    main_cell: new_cell,
                                                    icon_size: new_icon_size.unwrap_or(48),
                                                    snap_to_grid: layout.snap_to_grid(),
                                                });
                                            }
                                        }
                                    }
                                }
                                Err(reason) => {
                                    let latest = view.spacing().map(|sp| (sp, view.icon_size()));
                                    if latest != Some((new_cell, new_icon_size)) && latest.is_some()
                                    {
                                        last_view_metrics = latest;
                                        pending_icon_resize =
                                            latest.map(|m| (m, std::time::Instant::now()));
                                    } else if pending_two_box_ack.is_none()
                                        && view.items().len() != layout.expected_icons().len() {
                                        cancel_pending_background_reflow(
                                            &mut staged_hide,
                                            &mut staged_tab_selections,
                                            current_stage,
                                            &mut ui_next_stage,
                                            &mut hold_workspace_paint,
                                            &mut native_move_event,
                                            &mut extra_bridge_hide,
                                        );
                                        crate::host::event(
                                            "INFO",
                                            "native_desktop.icon_size_deferred_for_inventory",
                                            &reason,
                                        );
                                    } else {
                                        abort_reason = Some(reason);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(ref receiver)=two_box_ui_commands {
            let mut batch:Vec<NativeTwoBoxCommand>=queued_commands.drain(..).collect();
            batch.extend(receiver.try_iter().filter_map(NativeWorkerMessage::command));
            for command in batch {
                if command.validated_drop_sequence.is_none()
                    && (verified_drop_inflight.is_some() || expected_disk_after_drop.is_some())
                {
                    let _ = command.result.send(Err(
                        "A verified native icon drop is waiting for its own ACK; other Workspace commands refused".into()
                    ));
                    continue;
                }
                if real_native_desktop
                    && (pending_icon_resize.is_some()
                        || pending_two_box_ack.is_some()
                        || inventory_retry_since.is_some()
                        || !pending_inventory_removed.is_empty()
                        || !pending_inventory_added.is_empty()
                        || current_stage.is_some_and(|st| st as usize != ui_next_stage))
                {
                    queued_commands.push_back(command);
                    continue;
                }
                if let Some(preview)=command.drag_preview {
                    // Box drag preview: hide that Box's icons while DesktopCraft
                    // draws copies that follow it. The drop then runs the normal
                    // reflow; a cancelled drag restores the current view.
                    let mut icons=Vec::new();
                    if preview.active {
                        if let Some(member)=ui_membership.as_ref() {
                            if let Some(&(_,tab))=member.selected_tabs.iter()
                                .find(|(group,_)|*group==preview.group_id) {
                                icons = if preview.resizing {
                                    resize_box_ghosts(member, preview.group_id, tab, &native_shell)
                                } else {
                                    visible_box_ghosts(member, preview.group_id, tab, &records, &native_shell)
                                };
                                for &index in member.tab_indices(preview.group_id,tab).unwrap_or_default() {
                                    if let Some(record)=records.get_mut(index) {record.state=proto::STATE_HIDE;}
                                }
                            }
                        }
                    }else if let (Some(stages),Some(stage))=(staged_hide.as_ref(),current_stage) {
                        if let Some(hidden)=stages.get(stage as usize) {
                            for (index,record) in records.iter_mut().enumerate() {
                                record.state=if hidden.contains(&index){proto::STATE_HIDE}
                                    else{proto::STATE_SHOW};
                            }
                        }
                    }
                    unsafe {
                        memory.publish(pid,hwnd,records.len(),&records,next_generation,flags);
                        next_generation+=2;
                    }
                    if let Err(reason)=memory.confirm_policy(list, proto::APP_HOOK_PAINT_POLICY) {
                        abort_reason=Some(reason);
                        break;
                    }
                    drag_preview_group=preview.active.then_some(preview.group_id);
                    if let Some(reply)=preview.reply {
                        let snap = native_layout.as_ref().is_some_and(LiveLayoutTransaction::snap_to_grid);
                        let _=reply.send((
                            icons,
                            view.spacing().unwrap_or((75,107)),
                            view.icon_size().unwrap_or(48),
                            snap,
                        ));
                    } else {
                        refresh_ghost_icons=true;
                        let _=command.result.send(Ok(0));
                    }
                    continue;
                }
                if pending_two_box_ack.is_some() || (!real_native_desktop && stage_events.len()>=128) {
                    let error="Native two-Box UI command queue is busy; state unchanged".to_string();
                    ui_rejected_commands+=1;
                    ui_last_rejection=Some(error.clone());
                    let _=command.result.send(Err(error));
                    continue;
                }
                if let Some(cap) = native_ui_change_cap.filter(|cap| ui_accepted_commands >= *cap) {
                    let error=format!(
                        "Native Desktop safe session limit {} reached; save the current work and restart DesktopCraft so original icon coordinates can be audited",
                        cap);
                    ui_rejected_commands+=1;
                    ui_last_rejection=Some(error.clone());
                    let _=command.result.send(Err(error));
                    continue;
                }
                let Some(ref member)=ui_membership else {
                    let _=command.result.send(Err("Native two-Box membership unavailable".into()));
                    continue;
                };
                if let Some(batch) = command.pin_batch {
                    let submitted = (|| -> Result<usize, String> {
                        if !real_native_desktop || !general_workspace_requested
                            || drag_preview_group.is_some()
                        {
                            return Err("Windows Explorer is busy with a different layout change".into());
                        }
                        let sender = real_drop_sender.as_ref()
                            .ok_or("Explorer Pin notifications unavailable")?;
                        sync_expected_workspace_disk(
                            workspace,
                            &mut workspace_before,
                            &mut persisted_disk_state,
                            &mut expected_disk_after_drop,
                            &state,
                            &current_rolled_overrides,
                        )?;
                        let (next, paths, kind) = match batch {
                            NativePinBatchCommand::RuleSort { rules, next_id, include_existing_pins } => {
                                state.next_id = state.next_id.max(persisted_disk_state.next_id).max(next_id);
                                let (next, paths) = plan_native_rules_sort(
                                    &state, &native_shell, &rules, state.next_id, include_existing_pins);
                                (next, paths, crate::native_pin_drop::NativeMultiDropKind::SortRules)
                            }
                            NativePinBatchCommand::LayoutRestore { target, next_id } => {
                                state.next_id = state.next_id.max(persisted_disk_state.next_id).max(next_id);
                                let (next, paths) = plan_native_layout_restore(
                                    &state, &native_shell, &target, state.next_id)?;
                                (next, paths, crate::native_pin_drop::NativeMultiDropKind::RestoreLayout)
                            }
                            NativePinBatchCommand::PinPaths {
                                group_id,
                                tab_id,
                                paths,
                                next_id,
                            } => {
                                state.next_id = state.next_id.max(persisted_disk_state.next_id).max(next_id);
                                let (next, changed) = plan_native_pin_paths(
                                    &state,
                                    &native_shell,
                                    group_id,
                                    tab_id,
                                    &paths,
                                    state.next_id,
                                )?;
                                (
                                    next,
                                    changed,
                                    crate::native_pin_drop::NativeMultiDropKind::RestoreLayout,
                                )
                            }
                        };
                        let count = paths.len();
                        if count != 0 {
                            verified_drop_sequence += 1;
                            let proposal = NativeVerifiedDrop {
                                sequence: verified_drop_sequence, before: state.clone(), next,
                                kind, source_paths: paths, desktop_drop_targets: Vec::new(),
                            };
                            queue_verified_drop(sender, &mut queued_commands, &proposal)?;
                            verified_drop_inflight = Some(proposal);
                        }
                        Ok(count)
                    })();
                    let _ = command.result.send(submitted);
                    continue;
                }
                if let Some(next_state)=command.workspace_state {
                    if real_native_desktop {
                        crate::host::event("INFO","native_desktop.worker_workspace_received",
                            &format!("native_generation={} page={} visible_groups={}",
                                member.generation+1,next_state.page,next_state.groups.len()));
                    }
                    if !real_native_desktop || !general_workspace_requested {
                        let reason="Workspace synchronization requires the actual user-facing Native Desktop mode".to_string();
                        ui_rejected_commands+=1;
                        ui_last_rejection=Some(reason.clone());
                        let _=command.result.send(Err(reason));
                        continue;
                    }
                    if command.validated_drop_sequence.is_none()
                        && native_explorer_layout_matches(&state, &next_state)
                    {
                        let fast_res = (|| -> Result<(), String> {
                            if desktop_owner(&view).ok() != Some(owner) {
                                return Err("Explorer Desktop ownership changed during workspace sync".into());
                            }
                            let hook_live = unsafe {
                                memory.verify(pid, hwnd, records.len())
                                    && (*memory.mapped).hook_state.load(Ordering::Acquire)
                                        == proto::HOOK_STATE_ATTACHED
                            };
                            if !hook_live {
                                return Err("Explorer native hook is no longer attached".into());
                            }
                            sync_expected_workspace_disk(
                                workspace,
                                &mut workspace_before,
                                &mut persisted_disk_state,
                                &mut expected_disk_after_drop,
                                &state,
                                &current_rolled_overrides,
                            )?;
                            validate_native_workspace_edit(&state, &next_state)?;
                            Ok(())
                        })();
                        match fast_res {
                            Ok(()) => {
                                current_rolled_overrides = command.rolled_overrides;
                                state = next_state;
                                crate::host::event(
                                    "INFO",
                                    "native_desktop.rollup_overrides_synced",
                                    &format!("overrides={}", current_rolled_overrides.len()),
                                );
                                let _ = command.result.send(Ok(0));
                            }
                            Err(reason) => {
                                ui_rejected_commands += 1;
                                ui_last_rejection = Some(reason.clone());
                                let _ = command.result.send(Err(reason));
                            }
                        }
                        continue;
                    }
                    let prior_pages=native_layout.as_ref()
                        .map(|layout|member.selected_tabs.iter()
                            .map(|(id,_)|(*id,layout.current_page(*id)))
                            .collect::<Vec<_>>())
                        .unwrap_or_default();
                    if let Some(ref mut layout)=native_layout {
                        if command.validated_drop_sequence.is_none()
                            && !drag_preview_group.is_some_and(|group| is_regular_box_drop(&state, &next_state, group))
                        {
                            layout.reset_all_pages();
                        }
                    }
                    let mut prior_drop_originals: Vec<(String, (i32, i32))> = Vec::new();
                    let cached_reorder = verified_reorder_plan.take().filter(|(sequence, _)| {
                        command.validated_drop_sequence == Some(*sequence)
                    }).map(|(_, plan)| plan);
                    let candidate=(||->Result<_,String>{
                        sync_expected_workspace_disk(
                            workspace,
                            &mut workspace_before,
                            &mut persisted_disk_state,
                            &mut expected_disk_after_drop,
                            &state,
                            &current_rolled_overrides,
                        )?;
                        let (expected_disk, applied_drop_targets) = if let Some(sequence)=command.validated_drop_sequence {
                            validate_native_drop_permit(
                                verified_drop_inflight.as_ref(),sequence,
                                &state,&next_state)?;
                            let drop_paths = verified_drop_inflight.as_ref()
                                .map(|d| d.source_paths.clone())
                                .unwrap_or_default();
                            let drop_targets = verified_drop_inflight.as_ref()
                                .map(|d| d.desktop_drop_targets.clone())
                                .unwrap_or_default();
                            let expected = if serde_json::to_vec(&persisted_disk_state).ok()
                                == serde_json::to_vec(&state).ok()
                            {
                                next_state.clone()
                            } else {
                                crate::native_ole_persist::pins_change_onto(
                                    &persisted_disk_state,
                                    &next_state,
                                    &drop_paths,
                                )?
                            };
                            (Some(expected), drop_targets)
                        }else{
                            validate_native_workspace_edit(&state,&next_state)?;
                            (None, Vec::new())
                        };
                        let layout=native_layout.as_mut()
                            .ok_or("Real Native Desktop layout transaction not available")?;
                        if !applied_drop_targets.is_empty() {
                            prior_drop_originals = layout.apply_loose_drop_targets(
                                &applied_drop_targets,
                                &next_state,
                                &view,
                            );
                        }
                        let cell=view.spacing()
                            .ok_or("Explorer icon spacing unavailable")?;
                        let (next, hidden, prepared_placement) = match cached_reorder {
                            Some(plan) if plan.spacing == cell && plan.member.generation == member.generation + 1 => {
                                let hidden = plan.placement.1.clone();
                                (plan.member, hidden, Some((cell, plan.placement)))
                            }
                            _ => {
                                let next = crate::native_live_membership::NativeTwoBoxMembership
                                    ::prepare_workspace(member.generation + 1, &next_state, &native_shell)?;
                                let hidden = layout.planned_visibility(&next, &next_state, &native_original_order, cell)?.1;
                                (next, hidden, None)
                            }
                        };
                        if hidden.len()>native_hide_cap {
                            return Err("New active Native Workspace state exceeds its native hidden-icon safety cap".into());
                        }
                        Ok((next,hidden,expected_disk,prepared_placement))
                    })();
                    match candidate {
                        Ok((next,hidden,expected_disk,prepared_placement))=>{
                            let is_pin_drop = command.validated_drop_sequence.is_some();
                            pending_drop_placement = prepared_placement;
                            let drop_paths = if is_pin_drop {
                                verified_drop_inflight
                                    .as_ref()
                                    .map(|d| d.source_paths.clone())
                                    .unwrap_or_default()
                            } else {
                                Vec::new()
                            };
                            let rollup_transition = !is_pin_drop
                                && is_regular_rollup_transition(&state, &next_state);
                            hold_workspace_paint = is_pin_drop
                                || is_native_box_paint_transition(&state,&next_state)
                                || rollup_transition;
                            native_move_event = if cfg!(feature="native-fast-tabs") && hold_workspace_paint {
                                if is_pin_drop {
                                    Some("native_desktop.pin_drop_move_batch")
                                } else {
                                    match drag_preview_group {
                                        Some(group) if is_regular_box_drop(&state, &next_state, group) =>
                                            Some("native_desktop.box_drop_move_batch"),
                                        None if is_native_icon_tab_switch(&state, &next_state) =>
                                            Some("native_desktop.tab_move_batch"),
                                        None if rollup_transition =>
                                            Some("native_desktop.rollup_move_batch"),
                                        _ => None,
                                    }
                                }
                            } else {
                                None
                            };
                            extra_bridge_hide = if cfg!(feature="native-fast-tabs") && is_pin_drop {
                                pin_drop_bridge_hide_indices(&state, &next_state, &drop_paths, &native_original_order)
                            } else {
                                Vec::new()
                            };
                            if !is_pin_drop {
                                current_rolled_overrides = command.rolled_overrides;
                            }
                            state=next_state;
                            if is_pin_drop {
                                // A patch already represented on disk may leave
                                // the bytes identical. It has no disk transition
                                // to wait for after the Explorer ACK.
                                expected_disk_after_drop = expected_disk.filter(|expected| {
                                    !native_explorer_layout_matches(&persisted_disk_state, expected)
                                });
                            }
                            staged_hide.as_mut().unwrap().push(hidden);
                            staged_tab_selections.as_mut().unwrap()
                                .push(next.selected_tabs.clone());
                            ui_next_stage=staged_hide.as_ref().unwrap().len()-1;
                            ui_membership=Some(next);
                            pending_return_page=0;
                            pending_two_box_ack=Some(command.result);
                        }
                        Err(reason)=>{
                            if command.validated_drop_sequence.is_some() {
                                verified_drop_inflight = None;
                            }
                            if let Some(ref mut layout)=native_layout{
                                if command.validated_drop_sequence.is_some() {
                                    if let Err(rollback) = layout.revert_inflight_drop_positions(&view) {
                                        abort_reason = Some(format!("{reason}; {rollback}"));
                                        break;
                                    }
                                }
                                layout.rollback_loose_drop_targets(&prior_drop_originals);
                                layout.reset_all_pages();
                                for (id,page) in prior_pages{
                                    layout.set_page(id,page);
                                }
                            }
                            ui_rejected_commands+=1;
                            ui_last_rejection=Some(reason.clone());
                            let _=command.result.send(Err(reason));
                        }
                    }
                    continue;
                }
                if let Some(new_desktop_page)=command.desktop_page {
                    let candidate=(||->Result<_,String>{
                        if !general_workspace_requested || native_layout.is_none(){
                            return Err("Desktop Page changes need generalized Native in-Box mode".into());
                        }
                        if new_desktop_page>=state.pages.len(){
                            return Err("Native Desktop Page does not exist".into());
                        }
                        if state.page==new_desktop_page{
                            return Ok(None);
                        }
                        let mut candidate_state=state.clone();
                        candidate_state.page=new_desktop_page;
                        let next=crate::native_live_membership::NativeTwoBoxMembership
                            ::prepare_workspace(member.generation+1,&candidate_state,&native_shell)?;
                        let cell=view.spacing().ok_or("Native Explorer icon spacing unavailable")?;
                        let hidden=native_layout.as_ref().unwrap()
                            .planned_visibility(&next,&candidate_state,&native_original_order,cell)?.1;
                        if hidden.len()>native_hide_cap{
                            return Err("Changing desktop page would exceed verified native hide cap".into());
                        }
                        Ok(Some((candidate_state,next,hidden)))
                    })();
                    match candidate {
                        Ok(None)=>{let _=command.result.send(Ok(0));}
                        Ok(Some((candidate_state,next,hidden)))=>{
                            state=candidate_state;
                            if let Some(ref mut layout)=native_layout {
                                layout.reset_all_pages();
                            }
                            staged_hide.as_mut().unwrap().push(hidden);
                            staged_tab_selections.as_mut().unwrap()
                                .push(next.selected_tabs.clone());
                            ui_next_stage=staged_hide.as_ref().unwrap().len()-1;
                            ui_membership=Some(next);
                            pending_return_page=0;
                            pending_two_box_ack=Some(command.result);
                        }
                        Err(error)=>{
                            ui_rejected_commands+=1;
                            ui_last_rejection=Some(error.clone());
                            let _=command.result.send(Err(error));
                        }
                    }
                    continue;
                }
                let same_tab=member.selected_tabs.iter().any(|&(id,tab)|
                    id==command.group_id && tab==command.tab);
                if command.page_delta==0 && same_tab {
                    let current=native_layout.as_ref()
                        .map_or(0,|layout|layout.current_page(command.group_id));
                    let _=command.result.send(Ok(current));
                    continue;
                }
                let mut old_page=None;
                let candidate=(||->Result<_,String>{
                    if command.page_delta!=0 {
                        if !same_tab {
                            return Err("Native page request must target the active Tab".into());
                        }
                        let layout=native_layout.as_mut().ok_or(
                            "Native page controls only work with real in-Box layout")?;
                        let cell=view.spacing().ok_or("Explorer icon spacing unavailable")?;
                        let max=layout.page_limit(member,&state,command.group_id,
                            command.tab,cell)?;
                        // One wheel notch scrolls one icon row.
                        // Several queued notches arrive as one bounded delta.
                        let prior=layout.current_page(command.group_id);
                        let following=(prior as i64+i64::from(command.page_delta))
                            .clamp(0,max.saturating_sub(1) as i64) as usize;
                        if following==prior {
                            return Ok(None);
                        }
                        old_page=Some(prior);
                        layout.set_page(command.group_id,following);
                    } else if let Some(ref mut layout)=native_layout {
                        old_page=Some(layout.current_page(command.group_id));
                        layout.reset_page(command.group_id);
                    }
                    let next=if command.page_delta!=0{member.clone()}else{
                        member.switch_tab(&state,&native_shell,
                            command.group_id,command.tab)?
                    };
                    let snapshot=next.snapshot(&native_shell)?;
                    let hidden=if let Some(ref layout)=native_layout {
                        let cell=view.spacing().ok_or("Explorer icon spacing unavailable")?;
                        layout.planned_visibility(&next,&state,&native_original_order,cell)?.1
                    }else{snapshot.hidden_indices};
                    if hidden.len()>native_hide_cap {
                        return Err(format!(
                            "Native Tab/viewport would hide {} icons, above current {}-icon safety cap",hidden.len(),native_hide_cap));
                    }
                    Ok(Some((next,hidden)))
                })();
                match candidate {
                    Ok(None) => {
                        let page = native_layout.as_ref()
                            .map_or(0, |layout| layout.current_page(command.group_id));
                        let _ = command.result.send(Ok(page));
                    }
                    Ok(Some((next,hidden)))=>{
                        staged_hide.as_mut().unwrap().push(hidden);
                        staged_tab_selections.as_mut().unwrap()
                            .push(next.selected_tabs.clone());
                        ui_next_stage=staged_hide.as_ref().unwrap().len()-1;
                        pending_return_page=native_layout.as_ref()
                            .map_or(0,|layout|layout.current_page(command.group_id));
                        if real_native_desktop && command.page_delta != 0 {
                            hold_workspace_paint = true;
                            if cfg!(feature = "native-fast-tabs") {
                                // A scroll also moves items visible in both
                                // viewports. Keep them covered by the bridge
                                // policy until the new row is ready to paint.
                                extra_bridge_hide = next.tab_indices(command.group_id, command.tab)
                                    .unwrap_or_default().to_vec();
                                extra_bridge_hide.sort_unstable();
                                native_move_event = Some("native_desktop.viewport_move_batch");
                            }
                        }
                        ui_membership=Some(next);
                        pending_two_box_ack=Some(command.result);
                    }
                    Err(error)=>{
                        if let (Some(ref mut layout),Some(prior))=(&mut native_layout,old_page){
                            layout.set_page(command.group_id,prior);
                        }
                        ui_rejected_commands+=1;
                        ui_last_rejection=Some(error.clone());
                        let _=command.result.send(Err(error));
                    }
                }
            }
        }
        if abort_reason.is_some() {break;}
        if let Some(ref receiver)=external_commands {
            for new_selected in receiver.try_iter() {
                if new_selected==usize::MAX {
                    ui_stopped=true;
                    continue;
                }
                if let Some(ref set)=tab_set {
                    if new_selected==set.pair.from_tab {
                        ui_click_commands+=1;
                        next_user_stage=if user_clicked_b{2}else{0};
                    } else if new_selected==set.pair.to_tab {
                        ui_click_commands+=1;
                        user_clicked_b=true;
                        next_user_stage=1;
                    } else {
                        abort_reason=Some(format!("Unsupported Native Tab {}: fail-open",new_selected));
                        ui_stopped=true;
                    }
                }
            }
            if ui_stopped {break;}
        }
        if let Some(ref mut panel)=panel {
            if panel_mode==Some(true) {
                if step==cycles/3 {panel.simulate_button_for_test(1);}
                if step==(cycles*2)/3 {panel.simulate_button_for_test(0);}
            }
            for command in panel.poll() {
                match command {
                    crate::native_tab_panel::Action::SelectA => {
                        ui_click_commands+=1;
                        next_user_stage=if user_clicked_b {2}else{0};
                    }
                    crate::native_tab_panel::Action::SelectB => {
                        ui_click_commands+=1;
                        user_clicked_b=true;
                        next_user_stage=1;
                    }
                    crate::native_tab_panel::Action::Stop => {ui_stopped=true;}
                }
            }
            if ui_stopped || panel.closed(){break;}
        }
        if let Some(ref set)=tab_set {
            let stage=if panel_mode.is_some() || external_commands.is_some() {next_user_stage}
                else if step < cycles/3 {0}
                else if step < (cycles*2)/3 {1} else {2};
            if current_stage!=Some(stage) {
                // Native display-info filtering deliberately blanks a hidden
                // label. Reading back the ListView *while filtered* will then
                // falsely look like its identity/order changed. For B and
                // restored-A, briefly publish all SHOW with zero filter flags,
                // request a normal Explorer repaint, then revalidate actual
                // native names and stable Shell identities before re-hiding.
                if current_stage.is_some() {
                    let mut unsuppressed=records.clone();
                    for record in &mut unsuppressed {
                        record.state=proto::STATE_SHOW;
                    }
                    unsafe {
                        memory.publish(pid,hwnd,unsuppressed.len(),&unsuppressed,
                            next_generation,0);
                        next_generation+=2;
                        InvalidateRect(list,ptr::null(),0);
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                let fresh=if current_stage.is_none() {before.clone()}else{view.items()};
                let index_audit=if current_stage.is_none() {
                    // The initial full 61/61 validation occurred immediately
                    // before the hook was installed.
                    true
                }else{
                    let current_names:Vec<String>=list_items(list).into_iter()
                        .map(|i|i.name).collect();
                    current_names == names
                        && explorer_native::native_view_in_msaa_order(&fresh,&current_names).is_ok()
                };
                let same_identity=before.len()==fresh.len() && before.iter().zip(&fresh)
                    .all(|(old,now)|old.path.eq_ignore_ascii_case(&now.path)
                        && old.label==now.label);
                if !index_audit || !same_identity {
                    abort_reason=Some(format!(
                        "Native tab transition refused: labels_match={}, shell_identity_match={}; filter disconnected safely",
                        index_audit,same_identity));
                    break;
                }
                index_validation_checks+=1;
                let position_changes=before.iter().filter(|old|fresh.iter()
                    .any(|now|now.path.eq_ignore_ascii_case(&old.path)
                        && now.pos != old.pos)).count();
                // The two actual selected items cannot be hidden with a
                // stale rectangle if an external Explorer reflow moved
                // them while this test was running.
                if position_changes > 0 {
                    abort_reason=Some(format!(
                        "Explorer native icon reflow detected: {position_changes} positions changed; aborted"));
                    break;
                }
                let hidden_indices=set.hidden_indices(stage);
                for (index,record) in records.iter_mut().enumerate() {
                    record.state=if hidden_indices.contains(&index) {
                        proto::STATE_HIDE
                    }else{proto::STATE_SHOW};
                }
                stage_events.push(json!({
                    "stage":match stage {0=>"A",1=>"B",_=>"A_restored"},
                    "from_tab":set.pair.from_tab,"to_tab":set.pair.to_tab,
                    "hidden_indices":hidden_indices,
                    "hidden_labels":set.hidden_labels(stage),
                    "hidden_item_count":hidden_indices.len(),
                    "native_item_count":fresh.len(),
                    "native_positions_changed_at_transition":position_changes
                }));
                current_stage=Some(stage);
            }
        }
        if let Some(ref plan)=multi_plan {
            let stage=if drag_fixture {0}
                else {(step as usize * 4 / cycles as usize).min(3) as u32};
            if current_stage!=Some(stage) {
                if current_stage.is_some() {
                    // ListView may redact hidden labels. Temporarily publish
                    // all SHOW, then verify the actual native names and Shell
                    // positions BEFORE enabling the next per-Box policy.
                    let mut visible=records.clone();
                    for record in &mut visible {record.state=proto::STATE_SHOW;}
                    unsafe {
                        memory.publish(pid,hwnd,visible.len(),&visible,next_generation,0);
                        next_generation+=2;
                        InvalidateRect(list,ptr::null(),0);
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                let fresh=if current_stage.is_some(){view.items()}else{before.clone()};
                let current_names=if current_stage.is_some(){
                    list_items(list).into_iter().map(|item|item.name).collect::<Vec<_>>()
                }else{names.clone()};
                let valid_names=current_names==names &&
                    explorer_native::native_view_in_msaa_order(&fresh,&current_names).is_ok();
                let same_identity=before.len()==fresh.len() && before.iter().zip(&fresh)
                    .all(|(old,now)|old.path.eq_ignore_ascii_case(&now.path)
                        && old.label==now.label);
                let moved=before.iter().filter(|old|fresh.iter().any(|now|
                    old.path.eq_ignore_ascii_case(&now.path) && old.pos!=now.pos)).count();
                if !valid_names || !same_identity || moved>0 {
                    abort_reason=Some(format!(
                        "Native multi-Box transition refused: names={valid_names}, identities={same_identity}, moved={moved}; failing open"));
                    break;
                }
                index_validation_checks+=1;
                let box_a=plan.groups[0].group_id;
                let box_b=plan.groups[1].group_id;
                let overrides=match stage {
                    0=>vec![],
                    1=>vec![(box_a,1)],
                    2=>vec![(box_a,1),(box_b,1)],
                    _=>vec![],
                };
                let hidden=match if drag_fixture {Ok(Vec::new())}
                    else {plan.hidden_for(&overrides)} {
                    Ok(items)=>items,
                    Err(reason)=>{
                        abort_reason=Some(format!("Native multi-Box plan invalidated: {reason}"));
                        break;
                    }
                };
                for (index,record) in records.iter_mut().enumerate() {
                    record.state=if hidden.contains(&index){proto::STATE_HIDE}
                        else{proto::STATE_SHOW};
                }
                stage_events.push(json!({
                    "stage":match stage{0=>"A_A",1=>"B_A",2=>"B_B",_=>"A_A_restored"},
                    "group_ids":[box_a,box_b],
                    "selected_tabs":match stage {
                        0=>[0,0],1=>[1,0],2=>[1,1],_=>[0,0],
                    },
                    "hidden_indices":hidden,
                    "hidden_item_count":hidden.len(),
                    "native_positions_changed_at_transition":moved,
                }));
                current_stage=Some(stage);
            }
        }
        if let Some(ref stages)=staged_hide {
            let stage=if two_box_ui_commands.is_some(){ui_next_stage}
                else{((step as usize)*stages.len()/cycles as usize)
                    .min(stages.len()-1)};
            if current_stage!=Some(stage as u32) && !(real_native_desktop && pending_icon_resize.is_some()) {
                // Icons hidden by either the current or the next view stay
                // hidden while Explorer moves them. Publishing all-SHOW here
                // used to flash every other Tab's icon outside its Box; hiding
                // Stage 0's planned hidden set before the initial reflow also
                // prevents parked hidden icons from flashing in the corner.
                let drop_hide = &extra_bridge_hide;
                let mut transition_hidden:Vec<usize>=Vec::new();
                let initial_pre_hide = if current_stage.is_none() {
                    native_layout.as_ref().zip(ui_membership.as_ref())
                        .zip(view.spacing())
                        .and_then(|((layout, member), cell)|
                            layout.planned_visibility(member, &state, &native_original_order, cell).ok())
                        .map(|(_, hidden, _, _)| hidden)
                        .filter(|hidden| !hidden.is_empty())
                } else {
                    None
                };
                if current_stage.is_some() || initial_pre_hide.is_some() {
                    if hold_workspace_paint {
                        // Arm against the policy Explorer is already drawing,
                        // before publishing the temporary hidden items.
                        if let Err(reason)=memory.confirm_policy(list, proto::APP_HOOK_HOLD_PAINT) {
                            abort_reason=Some(reason);
                            break;
                        }
                    }
                    let mut bridge=records.clone();
                    let next_hidden = initial_pre_hide.as_deref().unwrap_or(&stages[stage]);
                    for (index,record) in bridge.iter_mut().enumerate() {
                        let hide=record.state==proto::STATE_HIDE
                            || next_hidden.contains(&index)
                            || drop_hide.binary_search(&index).is_ok();
                        record.state=if hide {proto::STATE_HIDE} else {proto::STATE_SHOW};
                        if hide {transition_hidden.push(index);}
                    }
                    unsafe {
                        memory.publish(pid,hwnd,bridge.len(),&bridge,next_generation,flags);
                        next_generation+=2;
                    }
                    if real_native_desktop {
                        // Activate filtering before moving hidden icons. Do
                        // not force a frame with both tabs' icons hidden.
                        if let Err(reason)=memory.confirm_policy(list, proto::APP_HOOK_APPLY_POLICY) {
                            abort_reason=Some(reason);
                            break;
                        }
                    } else {
                        unsafe { InvalidateRect(list,ptr::null(),0); }
                        std::thread::sleep(Duration::from_millis(30));
                    }
                }
                let mut live_layout_stage_info:Option<Value>=None;
                let mut active_stage_hidden=stages[stage].clone();
                let mut report_observed=None;
                let mut fast_batch_applied=false;
                let reflow_started=std::time::Instant::now();
                if real_native_desktop {
                    let now_metrics = view.spacing().map(|sp| (sp, view.icon_size()));
                    if now_metrics.is_some() && now_metrics != last_view_metrics {
                        last_view_metrics = now_metrics;
                        pending_icon_resize = now_metrics.map(|m| (m, std::time::Instant::now()));
                        unsafe {
                            memory.publish(pid,hwnd,records.len(),&records,next_generation,flags);
                            next_generation+=2;
                        }
                        if let Err(reason)=memory.confirm_policy(list, proto::APP_HOOK_PAINT_POLICY) {
                            abort_reason=Some(reason);
                            break;
                        }
                        continue;
                    }
                    crate::host::event("INFO","native_desktop.reflow_started",
                        &format!("stage={} next_generation={}",stage+1,next_generation));
                }
                if let Some(ref mut layout)=native_layout {
                    let Some(ref member)=ui_membership else {
                        abort_reason=Some("Native layout membership missing at transition".into());
                        break;
                    };
                    let move_event = native_move_event;
                    let move_hidden_items = |batches: &[Vec<(String, (i32, i32))>], before: &[crate::shell::ViewItem]| {
                        let Some(event) = move_event else { return Ok(false) };
                        let moves = batches.iter().flatten().map(|(path, to)| {
                            let index = native_original_order.iter().position(|item|
                                item.path.eq_ignore_ascii_case(path))?;
                            if !transition_hidden.contains(&index) {
                                return None;
                            }
                            let from = before.iter().find(|item| item.path.eq_ignore_ascii_case(path))?.pos;
                            Some(proto::TabMove { index: index as u32, from: [from.0, from.1], to: [to.0, to.1] })
                        }).collect::<Option<Vec<_>>>();
                        match moves {
                            Some(moves) => memory.move_hidden_items(list, &moves, event),
                            None => Ok(false),
                        }
                    };
                    match layout.reflow(
                        &view, member, &state, &native_original_order,
                        pending_drop_placement.take(), move_hidden_items,
                    ){
                        Ok(report)=>{
                            active_stage_hidden=report.hidden_indices;
                            native_shell=report.expected_native_order;
                            report_observed=Some(report.observed_after);
                            fast_batch_applied=report.fast_batch_applied;
                            // reflow() just read and verified these positions.
                            stage_expected=native_shell.clone();
                            if let Some(ref mut member)=ui_membership {
                                member.desktop_order=native_shell.iter()
                                    .map(|item|(item.path.to_lowercase(),item.pos))
                                    .collect();
                            }
                            native_layout_generations+=1;
                            if real_native_desktop {
                                crate::host::event("INFO","native_desktop.reflow_completed",
                                    &format!("stage={} elapsed_ms={} native_icon_count={} steps_ms=[{}]",
                                        stage+1,reflow_started.elapsed().as_millis(),
                                        native_shell.len(),report.timing));
                            }
                            live_layout_stage_info=Some(json!({
                                "generation":native_layout_generations,
                                "visible_paths":report.visible_paths.len(),
                                "page_status":report.page_status,
                            }));
                            page_status=Some(report.page_status);
                        }
                        Err(error) if real_native_desktop && error=="EXPLORER_SPACING_CHANGED" => {
                            let now_metrics = view.spacing().map(|sp| (sp, view.icon_size()));
                            last_view_metrics = now_metrics;
                            pending_icon_resize = now_metrics.map(|m| (m, std::time::Instant::now()));
                        if pending_icon_resize.is_none() {
                            abort_reason = Some("Explorer spacing is unavailable after a layout mutation".into());
                            break;
                        }
                            unsafe {
                                memory.publish(pid,hwnd,records.len(),&records,next_generation,flags);
                                next_generation+=2;
                            }
                            if let Err(reason)=memory.confirm_policy(list, proto::APP_HOOK_PAINT_POLICY) {
                                abort_reason=Some(reason);
                                break;
                            }
                            continue;
                        }
                        Err(error)=>{
                            abort_reason=Some(format!(
                                "Native Box transactional in-Box placement rejected: {error}"));
                            break;
                        }
                    }
                }
                let (verified_count, identity_unchanged, displaced, relabeled, shell_order_diff, names_unchanged, items_ms, msaa_ms) = if fast_batch_applied {
                    // Explorer verified each move in its own thread. The
                    // fast report's two position lists come from that same
                    // acknowledgment, so comparing them proves nothing new.
                    deferred_idle_poll = true;
                    (native_shell.len(), true, 0, 0, 0, true, 0, 0)
                } else {
                    let started = std::time::Instant::now();
                    let observed = report_observed.unwrap_or_else(|| view.items());
                    let items_ms = started.elapsed().as_millis();
                    let current_names = list_item_names(list);
                    let msaa_ms = started.elapsed().as_millis() - items_ms;
                    let stage_baseline = if native_layout.is_some() { &stage_expected } else { &before };
                    let originals: HashMap<_, _> = stage_baseline.iter()
                        .map(|item| (item.path.to_lowercase(), item)).collect();
                    let observed_paths: HashSet<_> = observed.iter()
                        .map(|item| item.path.to_lowercase()).collect();
                    let exact_ids = observed.len() == stage_baseline.len()
                        && originals.len() == stage_baseline.len()
                        && observed_paths.len() == originals.len()
                        && observed_paths.iter().all(|path| originals.contains_key(path));
                    let displaced = observed.iter().filter(|item|
                        originals.get(&item.path.to_lowercase()).is_none_or(|old| old.pos != item.pos)
                    ).count();
                    let relabeled = observed.iter().filter(|item|
                        originals.get(&item.path.to_lowercase()).is_none_or(|old| old.label != item.label)
                    ).count();
                    let order_diff = observed.iter().zip(stage_baseline)
                        .filter(|(now, old)| !now.path.eq_ignore_ascii_case(&old.path)).count();
                    let names_ok = if transition_hidden.is_empty() {
                        current_names == names
                            && explorer_native::native_view_in_msaa_order(&observed, &current_names).is_ok()
                    } else {
                        redacted_names_match(&current_names, &names, &transition_hidden)
                    };
                    (observed.len(), exact_ids && displaced == 0 && relabeled == 0, displaced,
                        relabeled, order_diff, names_ok, items_ms, msaa_ms)
                };
                if real_native_desktop {
                    crate::host::event("INFO","native_desktop.stage_timing",&format!(
                        "stage={} verify_items_ms={items_ms} msaa_names_ms={msaa_ms} cycle_ms={} fast_batch={}",
                        stage+1,worker_step_started.elapsed().as_millis(),u8::from(fast_batch_applied)));
                }
                let drop_commit_pending = real_native_desktop && pending_two_box_ack.is_some()
                    && verified_drop_inflight.as_ref().is_some_and(|drop| drop.next == state);
                let workspace_unchanged = if drop_commit_pending {
                    // The worker checked the predecessor before moving. The
                    // UI checks the same bytes under its lease during commit.
                    true
                } else if real_native_desktop {
                    sync_expected_workspace_disk(
                        workspace,
                        &mut workspace_before,
                        &mut persisted_disk_state,
                        &mut expected_disk_after_drop,
                        &state,
                        &current_rolled_overrides,
                    ).is_ok()
                } else {
                    std::fs::read(workspace).ok().as_deref()==Some(workspace_before.as_slice())
                };
                if !identity_unchanged || !names_unchanged || !workspace_unchanged {
                    abort_reason=Some(format!(
                        "Staged native hide aborted before stage {}: identity={}, displaced={}, relabeled={}, shell_order_diff={}, native_names={}, workspace={}",
                        stage+1,identity_unchanged,displaced,relabeled,shell_order_diff,
                        names_unchanged,workspace_unchanged));
                    break;
                }
                for (index,record) in records.iter_mut().enumerate() {
                    record.state=if active_stage_hidden.binary_search(&index).is_ok(){
                        proto::STATE_HIDE
                    }else{proto::STATE_SHOW};
                }
                if real_native_desktop && stage_events.len() >= 64 {
                    stage_events.remove(0);
                }
                stage_events.push(json!({
                    "stage":stage+1,
                    "native_in_box_layout_reflow":live_layout_stage_info,
                    "hidden_count":active_stage_hidden.len(),
                    "hidden_indices":active_stage_hidden,
                    "selected_tabs":staged_tab_selections.as_ref()
                        .and_then(|selections|selections.get(stage)).cloned(),
                    "two_box_independent_tab_cycle":staged_tab_selections.is_some(),
                    "hidden_sample_physical_icon_origins":active_stage_hidden.iter()
                        .take(4).filter_map(|&i|native_shell.get(i)
                            .map(|item|[item.pos.0,item.pos.1]))
                        .collect::<Vec<_>>(),
                    "verified_native_icons":verified_count,
                    "positions_changed_before_stage":0,
                    "shell_enumeration_order_differences":shell_order_diff,
                    "native_names_revalidated":names_unchanged,
                    "workspace_unmodified":(!drop_commit_pending).then_some(true),
                }));
                current_stage=Some(stage as u32);
                if staged_input_enabled {pending_input_stage=Some(stage);}
                index_validation_checks+=1;
                if stage+1==stages.len(){
                    if let Ok(marker)=std::env::var("DESKTOPCRAFT_NATIVE_STAGE_VISUAL_READY_FILE"){
                        let _=std::fs::write(marker,serde_json::to_vec(&json!({
                            "stage":stage+1,
                            "intended_hidden_count":stages[stage].len(),
                            "positions_verified":true,
                            "hook_attached":attached,
                            "screen_capture_must_wait_for_policy_publish":true,
                        })).unwrap_or_default());
                    }
                }
                // A cancel restores the last actually applied hidden set,
                // including the planner's final packing and overflow rows.
                if let Some(stages)=staged_hide.as_mut() {
                    stages[stage]=active_stage_hidden;
                }
                drag_preview_group=None;
                hold_workspace_paint=false;
                native_move_event = None;
                extra_bridge_hide.clear();
            }
        }
        if real_native_desktop &&
            (worker_step_started.elapsed()>=Duration::from_secs(4)
                || (current_stage.is_some() && unsafe {
                    lease_gap_requires_rebind(GetTickCount64(), memory.lease_tick().load(Ordering::Acquire))
                })) {
            lease_rebind_required = true;
            // The Explorer DLL's painting/input policy has a five-second
            // lease. After a blocked Shell COM call or modal context menu,
            // never silently renew an old generation that may have expired.
            // Stop fail-open instead and keep the exact-position journal if
            // the original icons cannot be verified restored.
            abort_reason=Some(format!(
                "Native Explorer COM worker cycle exceeded safe policy lease budget ({} ms); refusing stale generation renewal",
                worker_step_started.elapsed().as_millis()));
            break;
        }
        unsafe {
            // CRITICAL: the native Explorer subclass pins a verified
            // generation while it holds mouse capture. Re-publishing every
            // 200 ms changes the generation mid-drag and makes the otherwise
            // valid gesture fail its policy_is_live check after ~10 moves.
            // Preserve the exact attached generation during MouseDown ->
            // release. The existing five-second lease still fails closed
            // if the gesture stalls; do not silently extend it in-process.
            let native_stage_unchanged = reuse_native_generation(
                real_native_desktop,current_stage,last_published_native_stage)
                || (real_native_desktop && inventory_retry_since.is_some());
            if drag_fixture || native_stage_unchanged {
                // An actual Explorer gesture retains the same verified
                // generation. Only the lease timestamp advances. This avoids
                // repeated ListView policy changes during modal file menus.
                memory.renew_lease();
            }else{
                memory.publish(pid,hwnd,records.len(),&records,next_generation,flags);
                next_generation+=2;
                if real_native_desktop {
                    last_published_native_stage=current_stage;
                    confirm_stage_paint=current_stage.is_some();
                }
            }
            if memory.verify(pid,hwnd,records.len()){selfchecks+=1;}
            if (*memory.mapped).hook_state.load(Ordering::Acquire)==proto::HOOK_STATE_ATTACHED {
                attached=true;
                if let Some(ref ready)=ready_signal {
                    ready.store(true,Ordering::Release);
                }
                if let Some(index)=input_diagnostic_index {
                    // Only integer LPARAM crosses the process boundary. The
                    // DLL's in-process ListView subclass builds and derefs
                    // all RECT/LVHITTEST pointers in Explorer's own address
                    // space; SendMessageTimeoutW bounds a blocked UI thread.
                    if step==5 || step==10 {
                        let mut result=0usize;
                        diagnostic_queries+=1;
                        let acknowledged=SendMessageTimeoutW(list,
                            match synthetic_input {
                                1=>proto::APP_HOOK_INPUT_HOVER,
                                2=>proto::APP_HOOK_INPUT_RIGHTCLICK,
                                _=>proto::APP_HOOK_INPUT_INSPECT,
                            },
                            proto::HOOK_COOKIE,
                            index as isize,0x0002,750,&mut result);
                        if acknowledged==0 {diagnostic_timeouts+=1;}
                        else if result==1 {diagnostic_passes+=1;}
                    }
                }
                // Refresh native painting after every staged policy update;
                // no actual file operation or icon position call occurs.
                if !real_native_desktop && (current_stage.is_some() || step%2==0) {
                    InvalidateRect(list,ptr::null(),0);
                }
            }
        }
        if confirm_stage_paint {
            if let Err(reason)=memory.confirm_policy(list, proto::APP_HOOK_PAINT_POLICY) {
                abort_reason=Some(reason);
                break;
            }
        }
        if let Some(stage)=pending_input_stage.take() {
            // Developer-only input acceptance. The DLL allocates all native
            // ListView RECT/HITTEST pointers INSIDE Explorer; only integer
            // indices cross this process boundary. Begin with read-only
            // geometry checks and do not send a right-click unless hover
            // suppression is already proven under the current policy.
            let hidden=staged_hide.as_ref().and_then(|levels|levels.get(stage))
                .and_then(|indices|indices.first()).copied();
            let visible=(0..records.len()).find(|index|
                records[*index].state==proto::STATE_SHOW);
            let input_result=(||->Result<Value,String>{
                let hidden=hidden.ok_or("No verified hidden native item in stage")?;
                let visible=visible.ok_or("No visible native control item in stage")?;
                let hidden_indices=staged_hide.as_ref()
                    .and_then(|levels|levels.get(stage))
                    .ok_or("Native stage has no validated hidden member list")?;
                let hidden_selected_before=native_selected_or_focused_indices(
                    list,hidden_indices)?;
                if !hidden_selected_before.is_empty(){
                    return Err(format!(
                        "Refusing native input when inactive hidden icons are already selected/focused: {:?}",
                        hidden_selected_before));
                }
                let diagnostic=|message:u32,index:usize|->Result<(),String>{
                    let mut result=0usize;
                    let ack=unsafe{SendMessageTimeoutW(list,message,
                        proto::HOOK_COOKIE,index as isize,0x0002,750,&mut result)};
                    if ack==0 || result!=1 {
                        return Err(format!("Explorer-owned native hit-test refused index {index} message {message}, ack={ack}, result={result}"));
                    }
                    Ok(())
                };
                diagnostic(proto::APP_HOOK_INPUT_INSPECT,visible)?;
                let before_hidden=unsafe{(*memory.mapped).hook_diag_hidden_targets};
                diagnostic(proto::APP_HOOK_INPUT_INSPECT,hidden)?;
                if unsafe{(*memory.mapped).hook_diag_hidden_targets}<=before_hidden {
                    return Err("Explorer hit-test did not classify selected item as hidden".into());
                }
                let before_hover=unsafe{(*memory.mapped).hook_diag_hover_suppressed};
                diagnostic(proto::APP_HOOK_INPUT_HOVER,hidden)?;
                std::thread::sleep(Duration::from_millis(120));
                let after_hover=unsafe{(*memory.mapped).hook_diag_hover_suppressed};
                if after_hover<=before_hover {
                    return Err("Hidden Explorer hover was not suppressed; no right-click sent".into());
                }
                let before_right=unsafe{(*memory.mapped).hook_diag_right_suppressed};
                let before_menu=unsafe{(*memory.mapped).hook_diag_menu_suppressed};
                let before_menu_received=unsafe{(*memory.mapped).hook_diag_menu_received};
                let before_parent_menu=unsafe{(*memory.mapped).hook_diag_menu_parent_received};
                let before_selection=unsafe{(*memory.mapped).hook_selection_notify};
                diagnostic(proto::APP_HOOK_INPUT_RIGHTCLICK,hidden)?;
                std::thread::sleep(Duration::from_millis(150));
                let after_right=unsafe{(*memory.mapped).hook_diag_right_suppressed};
                let mut after_menu=unsafe{(*memory.mapped).hook_diag_menu_suppressed};
                let mut synchronous_menu_check=false;
                // A context-menu event posted from within Explorer's own
                // input diagnostic is asynchronous. If it has not been
                // observed by the bounded wait, send a single synchronous
                // numeric WM_CONTEXTMENU from the host. This checks the
                // actual ListView subclass before we claim any suppression.
                // The native DLL still owns all pointer-bearing hit-tests.
                if after_menu==before_menu && after_right>=before_right.saturating_add(2) {
                    let mut at=unsafe{POINT{
                        x:(*memory.mapped).hook_diag_hover_client_x,
                        y:(*memory.mapped).hook_diag_hover_client_y,
                    }};
                    if unsafe{ClientToScreen(list,&mut at)}==0 ||
                        !(i16::MIN as i32..=i16::MAX as i32).contains(&at.x) ||
                        !(i16::MIN as i32..=i16::MAX as i32).contains(&at.y) {
                        return Err("Explorer context-menu point is outside supported physical coordinates".into());
                    }
                    let packed=(u32::from(at.x as u16) |
                        (u32::from(at.y as u16)<<16)) as isize;
                    let mut result=0usize;
                    let acknowledged=unsafe{SendMessageTimeoutW(
                        list,0x007B,list as usize,packed,0x0002,750,&mut result)};
                    if acknowledged==0 {
                        return Err("Synchronous native WM_CONTEXTMENU delivery timed out; stopping input acceptance".into());
                    }
                    synchronous_menu_check=true;
                    std::thread::sleep(Duration::from_millis(50));
                    after_menu=unsafe{(*memory.mapped).hook_diag_menu_suppressed};
                }
                let (menu_received,parent_received,selection_after,hit_index,
                     hit_hidden,policy_active,client_match)=unsafe{
                    let p=&*memory.mapped;
                    (p.hook_diag_menu_received,p.hook_diag_menu_parent_received,
                     p.hook_selection_notify,p.hook_diag_menu_hit_index,
                     p.hook_diag_menu_hidden_at_hit,p.hook_diag_menu_policy_active,
                     p.hook_diag_menu_client_x==p.hook_diag_hover_client_x &&
                     p.hook_diag_menu_client_y==p.hook_diag_hover_client_y)
                };
                let hidden_selected_after=native_selected_or_focused_indices(
                    list,hidden_indices)?;
                let hidden_selection_clear=hidden_selected_before.is_empty()
                    && hidden_selected_after.is_empty();
                let strict_suppression=hidden_selection_clear
                    && hidden_input_counters_pass(
                        (before_hover,before_right,before_menu),
                        (after_hover,after_right,after_menu));
                let background_route=hidden_context_routed_to_background(
                    before_menu_received,before_parent_menu,
                    menu_received,parent_received,hit_index,
                    hit_hidden,policy_active,client_match,hidden_selection_clear);
                let input_edges_ok=after_hover>before_hover
                    && before_right.checked_add(2)
                        .is_some_and(|required|after_right>=required);
                if !input_edges_ok || (!strict_suppression && !background_route) {
                    return Err(format!(
                        "Hidden Explorer input/menu routing refused: right {before_right}->{after_right}, menu {before_menu}->{after_menu}, received {before_menu_received}->{menu_received}, parent {before_parent_menu}->{parent_received}, hit={hit_index}, selected {before_selection}->{selection_after}, client_match={client_match}"));
                }
                let context_route=if strict_suppression {
                    "hidden_icon_context_message_intercepted"
                }else{"native_desktop_background_no_hidden_file_target"};
                Ok(json!({"verified":true,"visible_control_index":visible,
                    "hidden_item_index":hidden,"visible_geometry_checked":true,
                    "hidden_geometry_checked":true,"hover_suppressed":after_hover-before_hover,
                    "right_button_suppressed":after_right-before_right,
                    "context_menu_suppressed":after_menu-before_menu,
                    "synchronous_menu_check":synchronous_menu_check,
                    "context_menu_route":context_route,
                    "hidden_file_context_menu_not_targeted":strict_suppression || background_route,
                    "hidden_selected_or_focused_before":hidden_selected_before,
                    "hidden_selected_or_focused_after":hidden_selected_after,
                    "selection_notifications_before":before_selection,
                    "selection_notifications_after":selection_after,
                    "native_menu_visual_appearance_verified":false,
                    "real_file_operations":false,"explorer_owned_input":true}))
            })();
            match input_result {
                Ok(result)=>{
                    if let Some(event)=stage_events.last_mut(){
                        event["native_hidden_input_acceptance"]=result;
                    }
                    input_completed_stages+=1;
                }
                Err(reason)=>{
                    abort_reason=Some(format!("Native hidden input stage {} stopped safely: {reason}",stage+1));
                    break;
                }
            }
        }
        let attached_layout=||unsafe{
            (*memory.mapped).hook_state.load(Ordering::Acquire)==proto::HOOK_STATE_ATTACHED
                && memory.verify(pid,hwnd,records.len())
        };
        if let Some(channel)=ui_channel.as_ref() {
            if (page_status.is_some() || refresh_ghost_icons || snapped_desktop_drop)
                && abort_reason.is_none()
                && attached_layout()
            {
                let main_cell = view.spacing().unwrap_or((75, 107));
                let icon_size = view.icon_size().unwrap_or(48);
                let snap_to_grid = native_layout.as_ref().is_some_and(LiveLayoutTransaction::snap_to_grid);
                let ghost_icons = if drag_preview_group.is_some() {
                    HashMap::new()
                } else {
                    ui_membership.as_ref().map(|member| member.selected_tabs.iter()
                        .map(|&(group,tab)| ((group,tab),visible_box_ghosts(member,group,tab,&records,&native_shell)))
                        .collect()).unwrap_or_default()
                };
                let visible_loose_rects = Some(compute_visible_loose_rects(
                    native_layout
                        .as_ref()
                        .map_or(native_shell.as_slice(), |layout| layout.expected_icons()),
                    &state,
                    main_cell,
                ));
                let _=channel.pages.send(NativeLayoutSync {
                    pages: page_status.map(Into::into),
                    ghost_icons: Some(ghost_icons),
                    visible_loose_rects,
                    main_cell,
                    icon_size,
                    snap_to_grid,
                });
            }
        }
        if layout_requested && current_stage==Some(0) && attached_layout() {
            if let Some(ref ready)=ready_signal {
                ready.store(true,Ordering::Release);
            }
        }
        let stage_applied = pending_icon_resize.is_none() && current_stage == Some(ui_next_stage as u32);
        let completed_ack = if !real_native_desktop || stage_applied { pending_two_box_ack.take() } else { None };
        if let Some(ack)=completed_ack {
            verified_drop_inflight = None;
            let still_live=attached_layout();
            if still_live && abort_reason.is_none(){
                ui_accepted_commands+=1;
                if real_native_desktop {
                    crate::host::event("INFO","native_desktop.generation_ack",
                        &format!("accepted_commands={} generation={} stage={:?}",
                            ui_accepted_commands,next_generation,current_stage));
                }
                let _=ack.send(Ok(pending_return_page));
            }else{
                let reason="Explorer did not confirm two-Box native visibility generation";
                // A generation that started but lost its policy is fatal.
                // Disconnect its ACK rather than sending a preflight rejection.
                drop(ack);
                abort_reason=Some(reason.into());
                break;
            }
        }
        if real_native_desktop && stage_applied && ui_next_stage != 0 {
            if let (Some(stages), Some(selections)) = (&mut staged_hide, &mut staged_tab_selections) {
                stages.drain(..ui_next_stage);
                stages.truncate(1);
                selections.drain(..ui_next_stage);
                selections.truncate(1);
                current_stage = Some(0);
                ui_next_stage = 0;
                last_published_native_stage = Some(0);
            }
        }
        if drag_fixture {
            let phase=unsafe{(*memory.mapped).drag_phase};
            if phase==1 && !drag_armed_announced {
                // A real mouse movement is FORBIDDEN until Explorer's DLL
                // confirms it intercepted MouseDown and owns the gesture.
                // This one-use acknowledgement is written by the host only
                // after observing native process-owned shared-memory state.
                if let Ok(path)=std::env::var("DESKTOPCRAFT_NATIVE_DRAG_READY_FILE") {
                    let marker=std::path::Path::new(&path).with_file_name("active.json");
                    let accepted=unsafe{(*memory.mapped).drag_down_accepted>0};
                    if accepted && std::fs::write(&marker,
                        serde_json::to_vec(&json!({"intercepted":true,
                            "phase":phase,"explorer_pid":pid})).unwrap_or_default()).is_ok(){
                        drag_armed_announced=true;
                    }
                }
            }
            if phase==2 || phase==3 {break;}
        }
        let mut wait=if awaiting_real_drop.as_ref().is_some_and(|(_, t)| t.elapsed() < Duration::from_millis(100)) {
            Duration::from_millis(5)
        } else if awaiting_real_drop.is_some()
            || pointer_tap.as_ref().is_some_and(|tap| tap.has_pending.load(Ordering::Acquire))
            || native_pointer.is_down()
        {
            Duration::from_millis(10)
        } else if last_external_drop.is_some_and(|(_, t)| t.elapsed() < Duration::from_secs(2))
            || inventory_retry_since.is_some()
            || pending_icon_resize.is_some()
        {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(200)
        };
        if let Some(due)=native_layout.as_ref().and_then(LiveLayoutTransaction::scaled_recheck_due_in) {
            wait=wait.min(due.max(Duration::from_millis(10)));
        }
        if let Some(ref receiver)=two_box_ui_commands {
            if let Ok(NativeWorkerMessage::Command(command))=receiver.recv_timeout(wait) {
                queued_commands.push_back(command);
            }
        } else {
            std::thread::sleep(wait);
        }
    }
    drop(pointer_tap);
    if real_native_desktop {
        if let Some(reason) = abort_reason.as_deref() {
            crate::host::event("WARN", "native_desktop.worker_stopping", reason);
        }
    }
    if drag_fixture {
        // Drag trials are invalid if Explorer or an extension reflows ANY
        // icon, including when a native MouseDown is rejected by our DLL.
        let latest=view.items();
        let drift=before.iter().filter(|item|latest.iter().any(|new|
            item.path.eq_ignore_ascii_case(&new.path) && item.pos!=new.pos)).count();
        if drift>0 {
            abort_reason=Some(format!(
                "Explorer native icon reflow during drag trial ({drift} icons); trial refused"));
        }
    }
    let stats=unsafe {
        let p=&*memory.mapped;
        json!({
            "attached_during_probe":attached,
            "hook_state_before_detach":p.hook_state.load(Ordering::Acquire),
            "hook_error":p.hook_error,
            "parent_customdraw_notifications":p.hook_parent_customdraw,
            "item_customdraw_notifications":p.hook_item_customdraw,
            "verified_native_labels_in_explorer":p.hook_label_matches,
            "suppressed_item_draws":p.hook_draw_suppressed,
            "suppressed_mouse_inputs":p.hook_input_suppressed,
            "stale_policy_rejections":p.hook_rejected_stale,
            "listview_wm_paint":p.hook_listview_paint,
            "listview_wm_printclient":p.hook_listview_printclient,
            "parent_wm_notify":p.hook_parent_notify,
            "parent_wm_drawitem":p.hook_parent_drawitem,
            "selection_notifications":p.hook_selection_notify,
            "notify_getdispinfo_w":p.hook_notify_getdispinfo_w,
            "notify_getdispinfo_a":p.hook_notify_getdispinfo_a,
            "notify_cache_hint":p.hook_notify_cachehint,
            "notify_other":p.hook_notify_other,
            "native_diag_requests":p.hook_diag_requests,
            "native_diag_geometry_ok":p.hook_diag_geometry_ok,
            "native_diag_hit_matches":p.hook_diag_hit_matches,
            "native_diag_hidden_targets":p.hook_diag_hidden_targets,
            "native_diag_failures":p.hook_diag_failures,
            "native_diag_last_index":p.hook_diag_last_index,
            "native_diag_policy_rejections":p.hook_diag_policy_rejections,
            "native_diag_rect_failures":p.hook_diag_rect_failures,
            "native_diag_hit_misses":p.hook_diag_hit_misses,
            "native_diag_hover_posted":p.hook_diag_hover_posted,
            "native_diag_hover_suppressed":p.hook_diag_hover_suppressed,
            "native_diag_hover_client_x":p.hook_diag_hover_client_x,
            "native_diag_hover_client_y":p.hook_diag_hover_client_y,
            "native_diag_right_posted":p.hook_diag_right_posted,
            "native_diag_right_suppressed":p.hook_diag_right_suppressed,
            "native_diag_menu_posted":p.hook_diag_menu_posted,
            "native_diag_menu_suppressed":p.hook_diag_menu_suppressed,
            "native_diag_menu_received":p.hook_diag_menu_received,
            "native_diag_menu_parent_received":p.hook_diag_menu_parent_received,
            "native_diag_menu_client_x":p.hook_diag_menu_client_x,
            "native_diag_menu_client_y":p.hook_diag_menu_client_y,
            "native_diag_menu_hit_index":p.hook_diag_menu_hit_index,
            "native_diag_menu_hidden_at_hit":p.hook_diag_menu_hidden_at_hit,
            "native_diag_menu_policy_active":p.hook_diag_menu_policy_active,
        })
    };
    let drag_snapshot=unsafe {
        let p=&*memory.mapped;
        let phase=(*(ptr::addr_of!(p.drag_phase) as *const std::sync::atomic::AtomicU32))
            .load(Ordering::Acquire);
        json!({
            "phase":phase,
            "source_native_index":p.drag_source_index,
            "selected_native_count":p.drag_selection_count,
            "selected_native_indices":p.drag_selection_indices
                [..(p.drag_selection_count as usize).min(proto::MAX_DRAG_SELECTION)].to_vec(),
            "begin_physical":[p.drag_begin_x,p.drag_begin_y],
            "end_physical":[p.drag_end_x,p.drag_end_y],
            "mouse_moves_observed":p.drag_mouse_moves,
            "cancelled_count":p.drag_cancelled_count,
            "mouse_down_seen":p.drag_down_seen,
            "mouse_down_intercepted":p.drag_down_accepted,
            "mouse_up_seen":p.drag_up_seen,
            "mouse_down_rejection_code":p.drag_down_reject_reason,
            "cancel_reason":p.drag_cancel_reason,
            "release_inferred_after_physical_mouse_up":p.drag_phase==2 &&
                p.drag_cancel_reason==9 && p.drag_up_seen==0,
            "mouse_down_client":[p.drag_down_client_x,p.drag_down_client_y],
            "mouse_down_hit_result":p.drag_hit_result,
            "mouse_down_hit_item":p.drag_hit_item,
            "mouse_down_hit_flags":p.drag_hit_flags,
            "armed_acknowledged_by_host":drag_armed_announced,
        })
    };
    let drag_preview=if drag_fixture && drag_snapshot["phase"]==2 {
        let index=drag_snapshot["source_native_index"].as_u64().unwrap_or(u64::MAX) as usize;
        let x=drag_snapshot["end_physical"][0].as_i64().unwrap_or(i64::MAX) as i32;
        let y=drag_snapshot["end_physical"][1].as_i64().unwrap_or(i64::MAX) as i32;
        let selected=drag_snapshot["selected_native_indices"].as_array()
            .map(|items|items.iter().filter_map(|v|v.as_u64())
                .map(|v|v as usize).collect::<Vec<_>>())
            .unwrap_or_default();
        let expected=if multi_drag_fixture{2}else{1};
        if selected.len()!=expected || !selected.contains(&index) {
            json!({"validated":false,
                "reason":"Native physical drag selection does not match explicit fixture",
                "expected_selected_items":expected,
                "actual_selected_items":selected.len()})
        }else{
            match explorer_native::plan_native_selected_drag_release(
                &state,&native_shell,multi_plan.as_ref().unwrap(),
                &selected,x,y,if original_ui_drag_fixture{16}else{2},
                if original_ui_drag_fixture{128}else if multi_drag_fixture{4}else{2}
            ) {
                Ok((next,plan,target_group,target_tab)) =>json!({
                    "validated":true,
                    "native_source_path":native_shell[index].path,
                    "selected_native_paths":selected.iter()
                        .map(|&i|native_shell[i].path.clone()).collect::<Vec<_>>(),
                    "selected_items_count":selected.len(),
                    "target_group_id":target_group,
                    "target_tab_id":target_tab,
                    "source_items_after":next.groups[0].tabs[0].items.len(),
                    "target_items_after":next.groups[1].tabs[0].items.len(),
                    "policy_hidden_count_after":plan.hidden_for(&[]).unwrap_or_default().len(),
                    "no_files_moved":true,
                    "real_workspace_saved":false,
                }),
                Err(reason)=>json!({"validated":false,"reason":reason}),
            }
        }
    }else{json!({
        "validated":false,
        "reason":if drag_fixture {"No complete native drag release observed"}
            else {"Gesture capture mode not enabled"}
    })};
    // Restore physical Explorer icon positions BEFORE publishing SHOW ALL so
    // parked hidden icons return silently to their original Desktop cells
    // without flashing at the corner parking point. SHOW ALL still runs
    // unconditionally afterward (even if rollback fails) before detaching.
    let native_layout_journal=native_layout.as_ref()
        .map(|layout|layout.recovery_journal().display().to_string());
    let mut native_layout_restored=!layout_requested;
    if let Some(ref mut layout)=native_layout {
        match layout.restore(&view){
            Ok(())=>{native_layout_restored=true;}
            Err(error)=>{
                native_layout_restore_error=Some(error.clone());
                if real_native_desktop {
                    crate::host::event("WARN", "native_desktop.rollback_unverified", &error);
                }
                abort_reason=Some(format!("Native in-Box rollback not verified: {error}"));
            }
        }
    }
    let final_expected_baseline = if real_native_desktop && native_layout_restored {
        native_layout.as_ref().map(|l| l.original_icons().to_vec())
    } else {
        None
    };
    drop(native_layout);
    if staged_hide.is_some(){
        // Always restore original Explorer painting BEFORE unloading the
        // subclass. This runs on success AND on a rejected identity/position
        // transition. The user Workspace and icon positions are never edited.
        let mut fully_visible=records.clone();
        for record in &mut fully_visible {record.state=proto::STATE_SHOW;}
        unsafe{
            memory.publish(pid,hwnd,fully_visible.len(),&fully_visible,next_generation,0);
            InvalidateRect(list,ptr::null(),0);
        }
        std::thread::sleep(Duration::from_millis(350));
    }
    drop(panel); // UI windows belong only to this guarded experiment.
    drop(controller); // detach callbacks before unmapping the policy
    let detach=unsafe{(*memory.mapped).hook_state.load(Ordering::Acquire)==proto::HOOK_STATE_DETACHED};
    drop(memory);

    // Explorer may have been terminated while the original IFolderView
    // transaction was active. The old COM view cannot restore icons in a
    // replacement Explorer process. After all native callbacks are detached,
    // reacquire ONLY the new Explorer Desktop view and use the same strict
    // journal/monitor/identity/Box-bounds validation as manual recovery.
    // Any mismatch keeps the journal for explicit user recovery; never guess.
    let mut native_explorer_restart_auto_recovered=false;
    if real_native_desktop && native_layout_restore_error.is_some()
        && crate::host::signature(&crate::host::monitors())==initial_monitor_signature {
        for _ in 0..8 {
            std::thread::sleep(Duration::from_millis(250));
            let Some(fresh)=DesktopView::acquire() else {continue;};
            if !desktop_owner(&fresh).is_ok_and(|fresh_owner|fresh_owner.pid!=pid) {
                continue;
            }
            let journal=crate::host::data_dir().join("native-inbox-recovery.json");
            if !journal.exists(){
                let current=fresh.items();
                if current.len()==before.len() && before.iter().all(|icon|
                    current.iter().any(|other|other.path.eq_ignore_ascii_case(&icon.path)
                        && other.label==icon.label && other.pos==icon.pos)) {
                    native_explorer_restart_auto_recovered=true;
                    break;
                }
                // A missing journal with a positional mismatch must NEVER be
                // treated as authorization to move any icon.
                continue;
            }
            let Ok(check)=crate::native_box_live_layout::automatic_recovery_status_or_restore(
                &fresh,false) else {continue;};
            if check["pending"]!=true
                || check["all_unclaimed_icons_unchanged"]!=true
                || check["monitor_identity_verified"]!=true {continue;}
            let Ok(outcome)=crate::native_box_live_layout::automatic_recovery_status_or_restore(
                &fresh,true) else {continue;};
            if outcome["manual_restoration_performed"]==true
                && outcome["pending"]==false {
                native_explorer_restart_auto_recovered=true;
                break;
            }
        }
        if native_explorer_restart_auto_recovered {
            native_layout_restored=true;
            native_layout_restore_error=None;
            crate::host::event("INFO","native_desktop.explorer_restart_restored",
                "replacement Explorer positions recovered from verified immutable journal");
        }
    }

    // An added display can leave every previously hidden icon at one
    // shared park point. After verified detach, allow a narrowly scoped
    // exact-ACK recovery only when all original monitors are unchanged.
    // Never reuse the old hook or renew a stale generation.
    let mut native_monitor_extension_auto_recovered=false;
    if real_native_desktop && detach && native_layout_restore_error.is_some()
        && crate::host::signature(&crate::host::monitors())!=initial_monitor_signature {
        let observed=crate::host::monitors();
        std::thread::sleep(Duration::from_millis(300));
        let settled=crate::host::monitors();
        if observed.len()==settled.len() && observed.iter().all(|m|settled.contains(m)) {
            if let Some(fresh)=DesktopView::acquire() {
                if desktop_owner(&fresh).ok()==Some(owner) {
                    match crate::native_box_live_layout::automatic_recovery_after_monitor_extension(
                        &fresh,&initial_monitors) {
                        Ok(outcome) if outcome["monitor_extension_recovery"]==true
                            && outcome["manual_restoration_performed"]==true
                            && outcome["pending"]==false => {
                            native_monitor_extension_auto_recovered=true;
                            native_layout_restored=true;
                            native_layout_restore_error=None;
                            crate::host::event("INFO","native_desktop.monitor_extension_restored",
                                "Detached filter; verified icon recovery after pure display extension");
                        }
                        Err(reason)=>crate::host::event("WARN",
                            "native_desktop.monitor_extension_recovery_refused",&reason),
                        _=>{}
                    }
                }
            }
        }
    }

    // Explorer may complete an asynchronous paint/layout pass after the
    // native subclass is removed. Read back the REAL item positions until
    // they settle or the bounded deadline is reached. Never move them here.
    let baseline_after_detach = final_expected_baseline.as_deref().unwrap_or(&before);
    let mut after=if native_explorer_restart_auto_recovered || native_monitor_extension_auto_recovered {
        DesktopView::acquire()
            .map(|fresh|fresh.items()).unwrap_or_default()
    }else{view.items()};
    let mut same_positions=baseline_after_detach.len()==after.len() && baseline_after_detach.iter().all(|a|
        after.iter().any(|b| a.path.eq_ignore_ascii_case(&b.path) && a.pos==b.pos));
    let post_detach_started=std::time::Instant::now();
    let mut post_detach_rechecks=0u32;
    while !same_positions && post_detach_started.elapsed()<Duration::from_secs(3){
        std::thread::sleep(Duration::from_millis(200));
        after=if native_explorer_restart_auto_recovered || native_monitor_extension_auto_recovered {
            DesktopView::acquire()
                .map(|fresh|fresh.items()).unwrap_or_default()
        }else{view.items()};
        post_detach_rechecks+=1;
        same_positions=baseline_after_detach.len()==after.len() && baseline_after_detach.iter().all(|a|
            after.iter().any(|b|a.path.eq_ignore_ascii_case(&b.path) && a.pos==b.pos));
    }
    let tab_sequence_completed=if tab_set.is_some() {
        let stages:Vec<&str>=stage_events.iter()
            .filter_map(|event|event["stage"].as_str()).collect();
        abort_reason.is_none()
            && stages.windows(3).any(|win|win==["A","B","A_restored"])
    }else{false};
    let multi_completed=if multi_plan.is_some(){
        let stages=stage_events.iter().filter_map(|event|event["stage"].as_str())
            .collect::<Vec<_>>();
        abort_reason.is_none()
            && stages.windows(4).any(|seq|
                seq==["A_A","B_A","B_B","A_A_restored"])
    }else{false};
    let moved_after=baseline_after_detach.iter().filter(|a|after.iter().any(|b|
        a.path.eq_ignore_ascii_case(&b.path) && a.pos!=b.pos)).count();
    // Durable trust boundary: only this currently running Explorer-hook
    // WORKER may place a gesture on the in-process one-slot channel.
    // External JSON reports, stale filenames and caller-provided index
    // strings are never an authority to write a Workspace.
    let mut native_gesture_delivered_inprocess=false;
    if let Some(sender)=gesture_sender {
        let source=drag_snapshot["source_native_index"].as_u64()
            .and_then(|i|usize::try_from(i).ok());
        let indices=drag_snapshot["selected_native_indices"].as_array()
            .and_then(|values|values.iter()
                .map(|v|v.as_u64().and_then(|i|usize::try_from(i).ok()))
                .collect::<Option<Vec<_>>>());
        let original_matches=std::fs::read(workspace)
            .ok().is_some_and(|bytes|bytes==workspace_before);
        let exact_shell=before.len()==after.len() &&
            before.iter().zip(&after).all(|(a,b)|
                a.path.eq_ignore_ascii_case(&b.path) && a.pos==b.pos
                && a.label==b.label);
        let evidence_ok=drag_fixture && attached && detach
            && abort_reason.is_none() && drag_source_hit_verified
            && drag_snapshot["phase"]==2
            && drag_preview["validated"]==true && same_positions
            && exact_shell && original_matches
            && drag_snapshot["armed_acknowledged_by_host"]==true
            && drag_snapshot["mouse_moves_observed"].as_u64().is_some_and(|n|n>=3);
        if evidence_ok {
            if let (Some(source),Some(indices))=(source,indices) {
                if !indices.is_empty() && indices.len()<=proto::MAX_DRAG_SELECTION
                    && drag_snapshot["selected_native_count"]==indices.len()
                    && indices.contains(&source)
                    && indices.iter().all(|&i|i<native_shell.len()) {
                    let begin=drag_snapshot["begin_physical"].as_array();
                    let end=drag_snapshot["end_physical"].as_array();
                    let coord=|arr:Option<&Vec<serde_json::Value>>|
                        arr.filter(|a|a.len()==2).and_then(|a|
                            Some((i32::try_from(a[0].as_i64()?).ok()?,
                                  i32::try_from(a[1].as_i64()?).ok()?)));
                    if let (Some(start_physical),Some(released_physical))=
                        (coord(begin),coord(end)){
                        let evidence=NativeGestureEvidence{
                            observed_at:std::time::Instant::now(),
                            explorer_pid:pid,
                            listview_hwnd:hwnd,
                            monitor_signature:initial_monitor_signature.clone(),
                            workspace_before,
                            observed_desktop:native_shell.iter()
                                .map(|item|(item.path.clone(),item.pos))
                                .collect(),
                            selected_indices:indices,
                            source_native_index:source,
                            start_physical,
                            released_physical,
                            observed_moves:drag_snapshot["mouse_moves_observed"]
                                .as_u64().unwrap_or(0) as u32,
                            hook_detached:detach,
                            icon_positions_unchanged:same_positions,
                        };
                        native_gesture_delivered_inprocess=
                            sender.try_send(evidence).is_ok();
                    }
                }
            }
        }
    }
    json!({
        "started":posted,
        "hook_install_wait_ms":hook_install_wait_ms,
        "observer_only":flags==0,
        "native_tab_trial":tab_set.is_some(),
        "synthetic_multi_box_trial":multi_plan.is_some(),
        "staged_native_hide_trial":staged_hide.is_some(),
        "native_two_box_tab_cycle_trial":staged_tab_selections.is_some()
            && two_box_ui_commands.is_none(),
        "native_two_box_live_ui":two_box_ui_commands.is_some(),
        "native_in_box_layout_requested":layout_requested,
        "native_in_box_layout_generations":native_layout_generations,
        "native_in_box_positions_restored":native_layout_restored,
        "native_explorer_restart_auto_recovered":native_explorer_restart_auto_recovered,
        "native_monitor_extension_auto_recovered":native_monitor_extension_auto_recovered,
        "native_policy_lease_rebind_required":lease_rebind_required,
        "native_in_box_rollback_error":native_layout_restore_error,
        "native_in_box_recovery_journal":native_layout_journal,
        "native_two_box_ui_accepted_commands":ui_accepted_commands,
        "native_two_box_ui_rejected_commands":ui_rejected_commands,
        "native_two_box_ui_last_rejection":ui_last_rejection,
        "native_two_box_ui_graceful_shutdown":ui_graceful_shutdown,
        "native_two_box_ui_experimental_max_hidden":if two_box_ui_commands.is_some(){Some(34usize)}else{None},
        "native_two_box_ui_max_changes_per_session":native_ui_change_cap.filter(|_|two_box_ui_commands.is_some()),
        "real_user_desktop_ui":real_native_desktop,
        "native_two_box_ui_final_selected_tabs":ui_membership.as_ref()
            .map(|member|member.selected_tabs.clone()),
        "native_hidden_input_trial":staged_input_enabled,
        "native_hidden_input_completed_stages":input_completed_stages,
        "native_hidden_input_completed":staged_input_enabled
            && staged_hide.as_ref().is_some_and(|steps|
                input_completed_stages==steps.len()) && abort_reason.is_none()
            && same_positions && attached && detach,
        "native_two_box_tab_cycle_completed":staged_tab_selections.is_some()
            && two_box_ui_commands.is_none()
            && staged_hide.as_ref().is_some_and(|steps|
                stage_events.len()==steps.len() && abort_reason.is_none()
                    && same_positions && attached && detach),
        "staged_native_hide_requested_limit":staged_hide_max,
        "staged_native_hide_completed":staged_hide.as_ref().is_some_and(|levels|
            stage_events.len()==levels.len() && abort_reason.is_none()
                && same_positions && attached && detach),
        "staged_native_hide_peak_count":staged_hide.as_ref().and_then(|levels|
            levels.iter().map(Vec::len).max()).unwrap_or(0),
        "staged_native_hide_restored_all_visible":staged_hide.is_some() && detach,
        "real_user_workspace_written_by_stager":false,
        "native_drag_capture_mode":drag_fixture,
        "native_drag_multi_selection_mode":multi_drag_fixture,
        "native_drag_original_ui_clone_mode":original_ui_drag_fixture,
        "native_drag_telemetry":drag_snapshot,
        "native_gesture_delivered_inprocess":native_gesture_delivered_inprocess,
        "native_drag_virtual_drop_preview":drag_preview,
        "native_drag_accepted_without_reposition":drag_fixture
            && drag_preview["validated"]==true && same_positions
            && attached && detach,
        "multi_box_trial_completed":multi_completed,
        "multi_box_group_ids":multi_plan.as_ref().map(|p|
            p.groups.iter().map(|g|g.group_id).collect::<Vec<_>>()),
        "interactive_native_tab_panel":panel_mode.is_some(),
        "native_app_event_bridge":external_commands.is_some(),
        "initial_monitor_signature":initial_monitor_signature,
        "synthetic_ui_test":panel_mode==Some(true),
        "ui_click_commands":ui_click_commands,
        "ui_stop_requested":ui_stopped,
        "native_complete_tab_trial":experimental_tab_group.is_some_and(|(_,_,full)|full),
        "tab_group_id":tab_set.as_ref().map(|set|set.pair.group_id),
        "tab_from_index":tab_set.as_ref().map(|set|set.pair.from_tab),
        "tab_to_index":tab_set.as_ref().map(|set|set.pair.to_tab),
        "tab_events":stage_events,
        "tab_a_b_a_completed":tab_sequence_completed,
        "index_validation_checks":index_validation_checks,
        "stopped_early":abort_reason.is_some(),
        "abort_reason":abort_reason,
        "native_icon_positions_changed_after":moved_after,
        "post_detach_position_rechecks":post_detach_rechecks,
        "single_item_test_index":experimental_single_index,
        "input_diagnostic_mode":input_diagnostic_index.is_some(),
        "synthetic_hover_test":synthetic_input==1,
        "synthetic_rightclick_test":synthetic_input==2,
        "input_diagnostic_index":input_diagnostic_index,
        "input_diagnostic_queries":diagnostic_queries,
        "input_diagnostic_passes":diagnostic_passes,
        "input_diagnostic_timeouts":diagnostic_timeouts,
        "policy_flags":flags,
        "explorer_pid":pid,
        "native_icons_before":before.len(),
        "initial_shell_index_label_matches":index_audit.same_index_labels,
        "initial_unique_names_reconciled":index_audit.reconciled_names,
        "used_stable_name_to_path_reconciliation":!index_audit.matching_order,
        "native_icons_after":after.len(),
        "all_native_icon_positions_unchanged":same_positions,
        "selfchecks_passed":selfchecks,
        "selfchecks_expected":cycles,
        "callback_detached":detach,
        "paint_filter_enabled":flags & proto::FLAG_PAINT_HIDE != 0,
        "input_filter_enabled":flags & proto::FLAG_INPUT_HIDE != 0,
        "desktop_files_touched":0,
        "versioned_runtime_dll_used":true,
        "stats":stats,
        "message":if flags==0 {
            "Observation only; no native icons hidden or input blocked"
        }else{
            "Explicit native test: bounded visibility/input policy installed temporarily; not production-ready"
        }
    })
}
