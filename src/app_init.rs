use crate::*;

impl App {
    pub(super) fn new(root: PathBuf, automation: bool, host_mode: bool) -> Self {
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
        // A missing Workspace with an OLE backup or staged recovery is NOT
        // a first run. Never show starter Boxes or overwrite a recovery
        // situation with a freshly initialized desktop layout.
        let recovery_backup=save_path.with_extension("ole-backup").exists();
        let recovery_stages=native_ole_persist::unresolved_workspace_ole_stages(&save_path);
        let native_icon_recovery=root.join("native-inbox-recovery.json").exists();
        let recovery_pending=recovery_backup || native_icon_recovery ||
            recovery_stages.as_ref().map_or(true,|entries|!entries.is_empty());
        let first_run=host_mode && !save_path.exists() && !recovery_pending;
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
        // A previous interrupted OLE transaction may hold a recoverable
        // predecessor or staging candidate. Surface it on startup and keep
        // this UI instance read-only instead of waiting for a later autosave
        // to fail. No original files are restored or removed automatically.
        if !automation && recovery_pending {
            save_allowed=false;
            if message.is_empty() {
                message=if native_icon_recovery {
                    "Native Explorer icon layout recovery pending. Do not update or save until the exact icon positions are reviewed.".into()
                }else{
                    "Workspace recovery needed. Changes cannot be saved until the pending OLE backup or stage is reviewed.".into()
                };
            } else {
                message.push_str(" Pending OLE recovery; saving is disabled.");
            }
            host::event("WARN","workspace.ole_recovery_pending",
                "Workspace locked read-only until user reviews pending OLE recovery data");
        }
        let orphan_entries=if host_mode && !automation && !first_run
            && std::env::var_os("DESKTOPCRAFT_NATIVE_CHROME_ONLY").is_none() {
            state.groups.iter().flat_map(|g|g.tabs.iter().flat_map(move |t|
                t.items.iter().filter_map(move |pin|{
                    let path=Path::new(&pin.path);
                    if path.is_absolute() && !path.exists() {
                        Some((g.id,t.id,pin.id,pin.path.clone()))
                    }else{None}
                })))
                .take(20).collect::<Vec<_>>()
        }else{Vec::new()};
        let orphan_dialog=if !orphan_entries.is_empty(){
            Some(Dialog::OrphanPins{entries:orphan_entries})
        }else{None};
        Self {
            state,
            root,
            selected: None,
            multi_selected: Vec::new(),
            dialog: orphan_dialog,
            minimized_settings: None,
            settings_restore_armed: false,
            #[cfg(windows)]
            settings_window: None,
            settings_return: None,
            settings_bring_to_front: false,
            rendering_settings_window: false,
            settings_minimize_requested: false,
            desktop_toggle_requested: None, 
            #[cfg(windows)]
            update_state: updates::UiState::Disabled,
            #[cfg(windows)]
            update_action: None,
            #[cfg(windows)]
            settings_last_paint: 0.,
            #[cfg(windows)]
            settings_cursor_was_over: false,
            menu: None,
            classic_file_menu: None,
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
            peek_hotkey_was_down: false,
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
            last_refresh: -REFRESH - 1.,
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
            native_pages: Default::default(),
            native_page_request: None,
            native_rules_sort_requested: None,
            native_layout_restore_requested: None,
            native_pin_paths_requested: None,
            native_ai_pending: None,
            native_tx_inflight: false,
            native_undo_inflight: None,
            native_record_undo_on_sync: None,
            native_scroll_target: None,
            native_visible_loose_rects: std::sync::Arc::from([]),
            #[cfg(windows)]
            native_cell: (75, 107),
            #[cfg(windows)]
            native_icon_size: 48,
            #[cfg(windows)]
            native_snap_to_grid: false,
            #[cfg(windows)]
            native_box_ghosts: Default::default(),
            #[cfg(windows)]
            native_backdrop: None,
            native_wheel_remainder: (0, 0),
            #[cfg(windows)]
            native_ghost: None,
            #[cfg(windows)]
            ghost_art: None,
            native_env: NativeEnvFlags::read(),
            portal_refresh: None,
            portal_refresh_inflight: std::collections::HashSet::new(),
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
            native_visual_revealed: None,
            hover_rolled: None,
            dock_reveal_block: None,
            top_dock_preview: None,
            rollup_interact_until: 0.,
            rollup_reveal_hold_until: 0.,
            dropdown_open: None,
            dropdown_list: None,
            dropdown_hit: None,
            dropdown_pick: None,
            dropdown_suppress: None,
            dropdown_scroll: 0,
        }
    }
    pub(super) fn notify(&mut self, s: impl Into<String>, time: f64) {
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
    pub(crate) fn push_undo(&mut self, snapshot: State) {
        self.undo.push(snapshot);
        if self.undo.len() > 32 {
            self.undo.remove(0);
        }
    }
    pub(crate) fn remember(&mut self) {
        let current = self.state.clone();
        self.push_undo(current);
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
    pub(super) fn group_id(&self) -> Option<u64> {
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
    pub(super) fn fresh_bounds(&self) -> Bounds {
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
}
