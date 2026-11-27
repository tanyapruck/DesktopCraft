use crate::*;

mod widgets;
mod settings_appearance;
mod settings_behavior;
mod settings_organize;
mod settings_portal;
mod settings_saved_layouts;
mod settings_rollup;
mod settings_pages;
mod settings_displays;
mod settings_peek;
mod settings_help;
mod rules;
mod preview_common;
mod preview_quick_hide;
mod preview_tab_drop;
mod preview_rollup;
mod preview_peek;
mod preview_displays;

pub(crate) use rules::RULE_MODES;
pub(crate) use widgets::wrap_text;
use rules::rule_field_label;
use widgets::slider;
use preview_quick_hide::draw_quick_hide_preview;
use preview_tab_drop::draw_tab_drop_preview;
use preview_rollup::draw_rollup_preview;
use preview_peek::draw_peek_preview;
use preview_displays::draw_displays_preview;

impl App {
    pub(super) fn modal(&mut self, p: &mut Painter, input: &Input) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        // An open dropdown list takes the click (nothing under it reacts).
        let mut local = input.clone();
        if let Some((id, rect, n, first)) = self.dropdown_hit.take() {
            if rect.contains(input.pos.x, input.pos.y) && input.scroll != 0. {
                let max_scroll = n.saturating_sub(8);
                self.dropdown_scroll = if input.scroll < 0. {
                    self.dropdown_scroll.saturating_add(1).min(max_scroll)
                } else {
                    self.dropdown_scroll.saturating_sub(1)
                };
            }
            if input.pressed {
                if rect.contains(input.pos.x, input.pos.y) {
                    let i = first + ((input.pos.y - rect.y) / 30.) as usize;
                    if i < n {
                        self.dropdown_pick = Some((id, i));
                    }
                    local.pressed = false;
                    p.click = false;
                } else if self.dropdown_open == Some(id) {
                    self.dropdown_suppress = Some(id);
                }
                self.dropdown_open = None;
            }
        }
        let input = &local;
        let settings = matches!(dialog, Dialog::Settings { .. });
        let own_win = settings && self.rendering_settings_window;
        let mut b = if own_win {
            Bounds {
                x: 0.,
                y: 0.,
                w: 870.,
                h: 650.,
            }
        } else if settings {
            self.settings_bounds_local()
        } else if matches!(dialog, Dialog::EditRule { .. }) {
            // Roomier than the name dialogs: two lists and a multi-line field.
            Bounds {
                x: ((self.width - 660.) / 2.).max(0.),
                y: ((self.height - 420.) / 2.).max(0.),
                w: 660.,
                h: 420.,
            }
        } else {
            Bounds {
                x: (self.width - 550.) / 2.,
                y: (self.height - 270.) / 2.,
                w: 550.,
                h: 270.,
            }
        };
        if own_win {
            let header = Bounds {
                x: b.x + 12.,
                y: b.y + 5.,
                w: b.w - if cfg!(windows) { 290. } else { 105. },
                h: 62.,
            };
            if input.pressed && header.contains(input.pos.x, input.pos.y) {
                let (cx, cy) = host::cursor();
                let pointer_ws = self.from_screen(cx, cy);
                let b_ws = self.settings_bounds_local();
                self.settings_drag_offset = Some(pointer_ws - vec2(b_ws.x, b_ws.y));
            }
            if !input.down {
                self.settings_drag_offset = None;
            } else if let Some(offset) = self.settings_drag_offset {
                let (cx, cy) = host::cursor();
                let pointer_ws = self.from_screen(cx, cy);
                self.settings_origin = Some(pointer_ws - offset);
            }
        } else if settings {
            // Native pointer capture crosses the actual Windows monitor edge,
            // even when this desktop-host window is region-clipped.
            let mouse_down = if self.host.is_some() && !self.automation {
                host::mouse_left_down()
            } else {
                input.down
            };
            let pointer = if self.host.is_some() && !self.automation {
                let (x, y) = host::cursor();
                self.from_screen(x, y)
            } else {
                input.pos
            };
            let header = Bounds {
                x: b.x + 12.,
                y: b.y + 5.,
                w: b.w - if cfg!(windows) { 290. } else { 105. },
                h: 62.,
            };
            let start = if self.host.is_some() && !self.automation {
                mouse_down && !self.settings_mouse_was_down
            } else {
                input.pressed
            };
            self.settings_mouse_was_down = mouse_down;
            if start && header.contains(pointer.x, pointer.y) {
                self.settings_drag_offset = Some(pointer - vec2(b.x, b.y));
                if self.host.is_some() {
                    host::capture_pointer(true);
                }
            }
            if !mouse_down {
                self.settings_drag_offset = None;
                if self.host.is_some() {
                    host::capture_pointer(false);
                }
            }
            if let Some(offset) = self.settings_drag_offset {
                if mouse_down {
                    self.settings_origin = Some(pointer - offset);
                    b = self.settings_bounds_local();
                }
            }
        } else {
            draw_rectangle(0., 0., self.width, self.height, alpha(BLACK, 0.47));
        }
        if !own_win {
            round(
                Bounds {
                    x: b.x + 4.,
                    y: b.y + 9.,
                    ..b
                },
                12.,
                alpha(BLACK, 0.22),
            );
            round(b, 10., SURFACE);
        } else {
            draw_rectangle(b.x, b.y, b.w, b.h, SURFACE);
        }
        p.enabled = true;
        p.click = input.pressed;
        let mut close = false;
        let mut minimize = false;
        let mut apply = false;
        if settings
            && p.button(
                "_",
                Bounds {
                    x: b.x + b.w - 81.,
                    y: b.y + 12.,
                    w: 30.,
                    h: 30.,
                },
                false,
            )
        {
            minimize = true;
        }
        if p.button(
            "×",
            Bounds {
                x: b.x + b.w - 44.,
                y: b.y + 12.,
                w: 30.,
                h: 30.,
            },
            false,
        ) {
            close = true
        }
        if input.key(KeyCode::Tab) {
            self.edit_focus = (self.edit_focus + 1) % 2;
        }
        // A dialog may hand over to another one (Settings > Edit opens the
        // rule pop-up); the swap happens after this frame's drawing.
        let mut open_instead: Option<Dialog> = None;
        let mut open_modal_from_settings: Option<Dialog> = None;
        match &mut dialog {
            Dialog::EditRule {
                index,
                mode,
                pattern,
                target,
            } => {
                p.text(
                    &format!("Edit rule {}", *index + 1),
                    b.x + 28.,
                    b.y + 25.,
                    25.,
                    TEXT,
                );
                p.text(
                    "It keeps its place in the list and its on/off state.",
                    b.x + 28.,
                    b.y + 66.,
                    14.,
                    MUTED,
                );
                let modes: Vec<String> = RULE_MODES.iter().map(|m| m.to_string()).collect();
                if let Some(i) = self.dropdown(
                    p,
                    30,
                    Bounds {
                        x: b.x + 28.,
                        y: b.y + 105.,
                        w: 270.,
                        h: 34.,
                    },
                    &modes,
                    (*mode as usize).min(modes.len() - 1),
                ) {
                    *mode = i as u8;
                }
                let choices = self.rule_destinations();
                if !choices.is_empty() {
                    *target = (*target).min(choices.len() - 1);
                    let options: Vec<String> =
                        choices.iter().map(|(_, _, n)| format!("To: {n}")).collect();
                    if let Some(i) = self.dropdown(
                        p,
                        31,
                        Bounds {
                            x: b.x + 312.,
                            y: b.y + 105.,
                            w: b.w - 340.,
                            h: 34.,
                        },
                        &options,
                        *target,
                    ) {
                        *target = i;
                    }
                }
                if *mode == 10 {
                    *pattern = "folder".into();
                    p.text(
                        "Matches every real folder on the Desktop.",
                        b.x + 28.,
                        b.y + 185.,
                        15.,
                        MUTED,
                    );
                } else {
                    let label = rule_field_label(*mode);
                    self.text_area(
                        p,
                        input,
                        label,
                        pattern,
                        Bounds {
                            x: b.x + 28.,
                            y: b.y + 178.,
                            w: b.w - 56.,
                            h: 132.,
                        },
                    );
                }
            }
            Dialog::NewGroup { value, .. } => {
                p.text("Create this group?", b.x + 28., b.y + 25., 25., TEXT);
                p.text(
                    "Save creates the box. Cancel leaves your desktop unchanged.",
                    b.x + 28.,
                    b.y + 68.,
                    14.,
                    MUTED,
                );
                self.text_field(
                    p,
                    input,
                    "Group name",
                    value,
                    Bounds {
                        x: b.x + 28.,
                        y: b.y + 127.,
                        w: b.w - 56.,
                        h: 42.,
                    },
                    0,
                );
            }
            Dialog::Rename { value, .. } => {
                p.text("Name this group", b.x + 28., b.y + 25., 25., TEXT);
                p.text(
                    "Give these files a place that makes sense to you.",
                    b.x + 28.,
                    b.y + 68.,
                    14.,
                    MUTED,
                );
                self.text_field(
                    p,
                    input,
                    "Group name",
                    value,
                    Bounds {
                        x: b.x + 28.,
                        y: b.y + 127.,
                        w: b.w - 56.,
                        h: 42.,
                    },
                    0,
                );
            }
            Dialog::NewPage { value } => {
                p.text("Create a new page?", b.x + 28., b.y + 25., 25., TEXT);
                p.text(
                    "Confirm to add a page. Cancel keeps your pages unchanged.",
                    b.x + 28.,
                    b.y + 68.,
                    14.,
                    MUTED,
                );
                self.text_field(
                    p,
                    input,
                    "Page name",
                    value,
                    Bounds {
                        x: b.x + 28.,
                        y: b.y + 127.,
                        w: b.w - 56.,
                        h: 42.,
                    },
                    0,
                );
            }
            Dialog::RenamePage { value, .. } => {
                p.text("Rename page", b.x + 28., b.y + 25., 25., TEXT);
                p.text(
                    "Only the page name changes. All its boxes remain in place.",
                    b.x + 28.,
                    b.y + 68.,
                    14.,
                    MUTED,
                );
                self.text_field(
                    p,
                    input,
                    "New page name",
                    value,
                    Bounds {
                        x: b.x + 28.,
                        y: b.y + 127.,
                        w: b.w - 56.,
                        h: 42.,
                    },
                    0,
                );
            }
            Dialog::Snapshot { value } => {
                p.text("Save this layout", b.x + 28., b.y + 25., 25., TEXT);
                p.text(
                    "Keep a backup of your groups, tabs and pages.",
                    b.x + 28.,
                    b.y + 68.,
                    14.,
                    MUTED,
                );
                self.text_field(
                    p,
                    input,
                    "Layout name",
                    value,
                    Bounds {
                        x: b.x + 28.,
                        y: b.y + 127.,
                        w: b.w - 56.,
                        h: 42.,
                    },
                    0,
                );
            }
            Dialog::Path { portal, value } => {
                p.text(
                    if *portal {
                        "Connect a folder"
                    } else {
                        "Pin a file"
                    },
                    b.x + 28.,
                    b.y + 25.,
                    25.,
                    TEXT,
                );
                p.text(
                    if *portal {
                        "Connect a Folder View. The folder and its contents stay where they are."
                    } else {
                        "Pin a file in a regular box without moving it. Linked Folder Views move files on drop."
                    },
                    b.x + 28.,
                    b.y + 68.,
                    14.,
                    MUTED,
                );
                self.text_field(
                    p,
                    input,
                    "Full path",
                    value,
                    Bounds {
                        x: b.x + 28.,
                        y: b.y + 127.,
                        w: b.w - 56.,
                        h: 42.,
                    },
                    0,
                );
            }
            Dialog::BindFolder { group, value } => {
                p.text("Bind folder to this box", b.x + 28., b.y + 25., 25., TEXT);
                let title = self
                    .state
                    .group(*group)
                    .map_or("Unknown box", |g| g.tab().name.as_str());
                p.text(
                    &p.fit(
                        &format!("Box: {title} — only the linked folder is shown."),
                        13.,
                        b.w - 56.,
                    ),
                    b.x + 28.,
                    b.y + 68.,
                    13.,
                    MUTED,
                );
                self.text_field(
                    p,
                    input,
                    "Existing folder path",
                    value,
                    Bounds {
                        x: b.x + 28.,
                        y: b.y + 127.,
                        w: b.w - 56.,
                        h: 42.,
                    },
                    0,
                );
            }
            Dialog::OrphanPins { entries } => {
                p.text("Review missing shortcut pins", b.x + 28., b.y + 25., 23., TEXT);
                p.text(&format!("{} saved pin(s) point to files not currently found.",entries.len()),
                    b.x + 28.,b.y + 72.,14.,TEXT);
                let first=entries.first().map(|(_,_,_,path)|path.as_str()).unwrap_or("");
                p.text(&format!("First: {}",first.rsplit(std::path::MAIN_SEPARATOR).next().unwrap_or(first)),
                    b.x + 28.,b.y + 111.,13.,MUTED);
                p.text("Remove virtual pins only? No source files will be deleted.",
                    b.x + 28.,b.y + 147.,13.,MUTED);
                p.text("A backup of the original Workspace is required first.",
                    b.x + 28.,b.y + 178.,13.,MUTED);
            }
            Dialog::Remove { .. } => {
                p.text("Remove this group?", b.x + 28., b.y + 25., 25., TEXT);
                p.text(
                    "Its pins and tabs will be removed from this workspace.",
                    b.x + 28.,
                    b.y + 88.,
                    15.,
                    TEXT,
                );
                p.text(
                    "The files and folders on your computer are kept.",
                    b.x + 28.,
                    b.y + 119.,
                    15.,
                    MUTED,
                );
            }
            Dialog::Settings {
                page,
                pattern,
                mode,
                target,
            } => {
                p.text("Settings", b.x + 26., b.y + 21., 25., TEXT);
                #[cfg(windows)]
                if p.button(
                    self.update_state.header(),
                    Bounds { x: b.x + b.w - 276., y: b.y + 12., w: 182., h: 30. },
                    matches!(&self.update_state, updates::UiState::Available(_) | updates::UiState::Ready(_)),
                ) {
                    *page = 11;
                    self.settings_offset = 0;
                    if matches!(&self.update_state, updates::UiState::Idle | updates::UiState::Current | updates::UiState::Failed(_)) {
                        self.update_action = Some(updates::Action::Check);
                    }
                }
                draw_line(
                    b.x + 194.,
                    b.y + 76.,
                    b.x + 194.,
                    b.y + b.h - 28.,
                    1.,
                    alpha(WHITE, 0.08),
                );
                // Group related controls cleanly: Core UX / Workspace / Maintenance.
                let sidebar_groups: [(&[(&str, usize)], f32, Option<f32>); 3] = [
                    (
                        &[
                            ("Appearance", 0),
                            ("Behavior", 1),
                            ("Roll-up", 6),
                            ("Peek", 8),
                        ],
                        b.y + 82.,
                        Some(b.y + 251.),
                    ),
                    (
                        &[
                            ("Pages", 7),
                            ("Displays", 9),
                            ("Organize & rules", 2),
                            ("Portal", 10),
                        ],
                        b.y + 260.,
                        Some(b.y + 429.),
                    ),
                    (
                        &[("Saved layouts", 3), ("Help", 4)],
                        b.y + 438.,
                        None,
                    ),
                ];
                for (items, start_y, divider_y) in sidebar_groups {
                    for (idx, (label, id)) in items.iter().enumerate() {
                        if p.button(
                            label,
                            Bounds {
                                x: b.x + 18.,
                                y: start_y + idx as f32 * 42.,
                                w: 160.,
                                h: 34.,
                            },
                            *page == *id,
                        ) {
                            *page = *id;
                            self.edit_focus = 0;
                            self.shortcut_recording = false;
                            self.settings_offset = 0;
                        }
                    }
                    if let Some(div_y) = divider_y {
                        draw_line(
                            b.x + 22.,
                            div_y,
                            b.x + 174.,
                            div_y,
                            1.,
                            alpha(WHITE, 0.09),
                        );
                    }
                }
                let x = b.x + 222.;
                let y = b.y + 90.;
                let w = b.w - 250.;
                match *page {
                    0 | 5 => self.settings_page_appearance(p, input, x, y, w, target, &mut close),
                    1 => self.settings_page_behavior(p, input, x, y, w, &mut close),
                    2 => self.settings_page_organize(p, input, x, y, w, pattern, mode, target, &mut close, &mut open_instead),
                    10 => self.settings_page_portal(p, input, x, y, w, &mut open_modal_from_settings),
                    3 => self.settings_page_saved_layouts(p, input, x, y, w),
                    6 => self.settings_page_rollup(p, input, x, y, w),
                    7 => self.settings_page_pages(p, input, x, y, w, &mut close, &mut open_modal_from_settings),
                    9 => self.settings_page_displays(p, x, y, w),
                    8 => self.settings_page_peek(p, input, x, y, w),
                    #[cfg(windows)]
                    11 => self.settings_page_updates(p, x, y, w),
                    _ => self.settings_page_help(p, input, x, y, w),
                }
            }
        }
        // The open dropdown list, on top of everything in the dialog.
        if let Some((id, anchor, options, current)) = self.dropdown_list.take() {
            let count = options.len().min(8);
            let first = self
                .dropdown_scroll
                .min(options.len().saturating_sub(count));
            let rect = Bounds {
                x: anchor.x,
                y: anchor.y + anchor.h + 2.,
                w: anchor.w,
                h: count as f32 * 30.,
            };
            round(
                Bounds {
                    x: rect.x + 3.,
                    y: rect.y + 5.,
                    ..rect
                },
                7.,
                alpha(BLACK, 0.3),
            );
            round(rect, 6., Color::new(0.10, 0.14, 0.20, 1.));
            outline(rect, alpha(WHITE, 0.12));
            for (i, option) in options.iter().enumerate().skip(first).take(count) {
                let row = Bounds {
                    x: rect.x + 3.,
                    y: rect.y + (i - first) as f32 * 30. + 2.,
                    w: rect.w - 6.,
                    h: 26.,
                };
                if row.contains(p.mouse.x, p.mouse.y) {
                    round(row, 4., Color::new(0.22, 0.32, 0.45, 1.));
                } else if i == current {
                    round(row, 4., alpha(WHITE, 0.06));
                }
                p.text(
                    &p.fit(option, 14., row.w - 16.),
                    row.x + 9.,
                    row.y + 4.,
                    14.,
                    TEXT,
                );
            }
            self.dropdown_hit = Some((id, rect, options.len(), first));
        }
        self.dropdown_suppress = None;
        if let Some(next) = open_instead.take() {
            dialog = next;
            self.edit_focus = 0;
            self.select_all = false;
        }
        if !settings {
            if p.button(
                "Cancel",
                Bounds {
                    x: b.x + b.w - 214.,
                    y: b.y + b.h - 66.,
                    w: 84.,
                    h: 38.,
                },
                false,
            ) {
                close = true
            }
            let empty = matches!(&dialog,Dialog::NewGroup{value,..}|Dialog::Rename{value,..}|Dialog::Snapshot{value}|Dialog::NewPage{value}|Dialog::RenamePage{value,..}|Dialog::Path{value,..}|Dialog::BindFolder{value,..}|Dialog::EditRule{pattern:value,..} if value.trim().is_empty());
            if !empty
                && (p.button(
                    if matches!(dialog, Dialog::Remove { .. }) {
                        "Remove"
                    } else if matches!(dialog, Dialog::OrphanPins { .. }) {
                        "Remove pins"
                    } else if matches!(dialog, Dialog::NewPage { .. }) {
                        "Create"
                    } else if matches!(dialog, Dialog::RenamePage { .. }) {
                        "Rename"
                    } else {
                        "Save"
                    },
                    Bounds {
                        x: b.x + b.w - 118.,
                        y: b.y + b.h - 66.,
                        w: 90.,
                        h: 38.,
                    },
                    true,
                ) || input.key(KeyCode::Enter))
            {
                apply = true;
            }
        }
        // A size range must be two sizes, smaller first; keep the pop-up open.
        if apply
            && matches!(&dialog, Dialog::EditRule { mode: 9, pattern, .. } if size_range(pattern).is_none())
        {
            self.notify(
                "Enter two byte sizes, such as 100,200, with the smaller first.",
                input.time,
            );
            apply = false;
        }
        if apply {
            let needs_remember = !matches!(&dialog, Dialog::Remove { .. } | Dialog::Path { .. });
            if needs_remember {
                self.remember();
            }
            match &dialog {
                Dialog::EditRule {
                    index,
                    mode,
                    pattern,
                    target,
                } => {
                    let choices = self.rule_destinations();
                    match (self.state.rules.get_mut(*index), choices.get(*target)) {
                        (Some(rule), Some((group, tab, _))) => {
                            // Same slot and on/off state, so its priority is kept.
                            rule.mode = *mode;
                            rule.pattern = pattern.trim().into();
                            rule.target = *group;
                            rule.target_tab = Some(*tab);
                            self.notify(format!("Rule {} saved", index + 1), input.time);
                        }
                        _ => {
                            self.undo.pop();
                            self.notify("That rule or its box no longer exists.", input.time);
                        }
                    }
                }
                Dialog::NewGroup { bounds, value } => {
                    let id = self.state.add_group(value.trim(), *bounds);
                    self.selected = Some((id, 0));
                    self.notify("Group created", input.time);
                }
                Dialog::Rename { group, value } => {
                    if let Some(g) = self.state.group_mut(*group) {
                        g.tab_mut().name = value.trim().into()
                    }
                }
                Dialog::NewPage { value } => match self.state.create_page(value) {
                    Ok(_) => {
                        self.selected = None;
                        self.settings_offset = self.state.pages.len().saturating_sub(4);
                        if let Some(h) = self.host.as_mut() {
                            h.last_sync = -10.;
                        }
                        self.notify("Page created", input.time);
                    }
                    Err(reason) => {
                        self.undo.pop();
                        self.notify(reason, input.time);
                    }
                },
                Dialog::RenamePage { index, value } => {
                    match self.state.rename_page(*index, value) {
                        Ok(()) => self.notify("Page renamed", input.time),
                        Err(reason) => {
                            self.undo.pop();
                            self.notify(reason, input.time);
                        }
                    }
                }
                Dialog::Snapshot { value } => {
                    self.state.snapshot(value.trim());
                    self.notify("Layout saved", input.time);
                }
                Dialog::Path { portal, value } => self.import_path(
                    Path::new(value.trim().trim_matches('"')),
                    *portal,
                    input.time,
                ),
                Dialog::BindFolder { group, value } => {
                    let is_native = self.native_env.chrome_only
                        && self.native_env.general_workspace
                        && !self.automation;
                    let total_regular_tabs: usize = self
                        .state
                        .groups
                        .iter()
                        .flat_map(|g| &g.tabs)
                        .filter(|t| t.folder.is_none())
                        .count();
                    let target_is_regular = self
                        .state
                        .group(*group)
                        .is_some_and(|g| g.tab().folder.is_none());
                    if is_native && target_is_regular && total_regular_tabs <= 1 {
                        self.undo.pop();
                        self.notify(
                            "Keep at least one regular Desktop Box before linking this tab to a folder.",
                            input.time,
                        );
                    } else {
                        let path = Path::new(value.trim().trim_matches('"'));
                        match self.state.bind_group_folder(*group, path) {
                            Ok(()) => self.notify("Folder linked; files remain on disk.", input.time),
                            Err(e) => {
                                self.undo.pop();
                                self.notify(e, input.time);
                            }
                        }
                    }
                }
                Dialog::Remove { group } => self.request_remove_group(*group, input.time),
                Dialog::OrphanPins { entries } => {
                    // Explicit confirmation only. Never remove a source file,
                    // never silently prune the user's Workspace at startup.
                    if !self.save_allowed ||
                        self.root.join("native-inbox-recovery.json").exists() {
                        self.notify("Workspace is locked for recovery; missing pins preserved",
                            input.time);
                    }else{
                        let snapshot=std::fs::read(&self.save_path);
                        match snapshot {
                            Ok(bytes)=>{
                                let stamp=std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map_or(0,|d|d.as_millis());
                                let backup=self.save_path.with_file_name(format!(
                                    "workspace-before-missing-pin-cleanup-{stamp}.json"));
                                let copied=(||->std::io::Result<()>{
                                    use std::io::Write;
                                    let mut file=std::fs::OpenOptions::new()
                                        .create_new(true).write(true).open(&backup)?;
                                    file.write_all(&bytes)?;
                                    file.sync_all()
                                })();
                                if copied.is_err() {
                                    self.notify("Could not back up Workspace; pins unchanged",
                                        input.time);
                                }else{
                                    let ids=entries.iter().filter(|(_,_,_,path)|
                                        !std::path::Path::new(path).exists())
                                        .map(|(g,t,p,_)|(*g,*t,*p))
                                        .collect::<std::collections::BTreeSet<_>>();
                                    let mut removed=0usize;
                                    for g in &mut self.state.groups{
                                        for t in &mut g.tabs{
                                            let old=t.items.len();
                                            t.items.retain(|pin|!ids.contains(&(g.id,t.id,pin.id)));
                                            removed+=old-t.items.len();
                                        }
                                    }
                                    self.notify(format!(
                                        "{removed} missing virtual pin(s) removed; files retained; Workspace checkpoint saved"),
                                        input.time);
                                }
                            }
                            Err(_)=>self.notify(
                                "Could not read original Workspace; pins unchanged",input.time),
                        }
                    }
                }
                _ => {}
            }
            close = true;
        }
        if let Some(next_modal) = open_modal_from_settings.take() {
            self.dropdown_list = None;
            self.dropdown_open = None;
            self.dropdown_suppress = None;
            self.settings_drag_offset = None;
            self.settings_mouse_was_down = false;
            self.shortcut_recording = false;
            self.settings_return = Some(dialog);
            self.dialog = Some(next_modal);
            return;
        }
        if minimize && own_win {
            self.settings_minimize_requested = true;
            self.settings_drag_offset = None;
            self.settings_mouse_was_down = false;
            self.shortcut_recording = false;
            self.dialog = Some(dialog);
        } else if minimize {
            self.minimized_settings = Some(dialog);
            self.settings_restore_armed = false;
            self.settings_drag_offset = None;
            self.settings_mouse_was_down = false;
            self.shortcut_recording = false;
            if self.host.is_some() {
                host::capture_pointer(false);
                host::settings_deactivate();
            }
        } else if !close {
            self.dialog = Some(dialog);
        } else if matches!(dialog, Dialog::EditRule { .. }) {
            // The rule pop-up returns to the rules page it was opened from.
            self.dialog = Some(Dialog::Settings {
                page: 2,
                pattern: String::new(),
                mode: 0,
                target: 0,
            });
            self.settings_bring_to_front = true;
        } else if settings {
            self.settings_drag_offset = None;
            self.settings_mouse_was_down = false;
            self.shortcut_recording = false;
            self.settings_return = None;
            if self.host.is_some() && !own_win {
                host::capture_pointer(false);
            }
            if self.color_pick_target.is_some() {
                if !host::begin_color_pick() {
                    let target = self.color_pick_target.take().unwrap_or(0);
                    self.dialog = Some(Dialog::Settings {
                        page: 0,
                        pattern: String::new(),
                        mode: 0,
                        target,
                    });
                    self.settings_bring_to_front = true;
                    self.notify("Screen pixel picker is unavailable right now", input.time);
                }
            }
        } else if let Some(ret) = self.settings_return.take() {
            self.dialog = Some(ret);
            self.settings_bring_to_front = true;
        }
    }
}
