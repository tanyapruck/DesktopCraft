use crate::*;
impl App {
    pub(super) fn text_edit(&mut self, value: &mut String, input: &Input) {
        if input.ctrl && input.key(KeyCode::A) {
            self.select_all = true;
        }
        if input.ctrl && input.key(KeyCode::C) {
            macroquad::miniquad::window::clipboard_set(value);
        }
        let paste = if input.ctrl && input.key(KeyCode::V) {
            macroquad::miniquad::window::clipboard_get().unwrap_or_default()
        } else {
            String::new()
        };
        let text = if !paste.is_empty() {
            paste
        } else if input.ctrl {
            String::new()
        } else {
            input.typed.clone()
        };
        if !text.is_empty() {
            if self.select_all {
                value.clear();
                self.select_all = false
            }
            value.push_str(&text.replace(['\n', '\r'], ""));
        }
        // Holding Backspace keeps deleting at the keyboard's repeat rate.
        if input.key_or_repeat(KeyCode::Backspace) {
            if self.select_all {
                value.clear();
                self.select_all = false
            } else {
                value.pop();
            }
        }
    }
    fn text_field(
        &mut self,
        p: &Painter,
        input: &Input,
        label: &str,
        value: &mut String,
        b: Bounds,
        id: usize,
    ) {
        p.text(label, b.x, b.y - 24., 13., MUTED);
        round(b, 5., Color::new(0.04, 0.075, 0.12, 1.));
        outline(
            b,
            if self.edit_focus == id {
                ACCENT
            } else {
                Color::new(0.24, 0.31, 0.40, 1.)
            },
        );
        if input.pressed && b.contains(input.pos.x, input.pos.y) {
            self.edit_focus = id;
            self.select_all = false;
        }
        if self.edit_focus == id {
            self.text_edit(value, input);
        }
        // While typing, a long value scrolls to show its end (where the caret
        // is); otherwise it shows its start.
        let text = if self.edit_focus == id {
            p.fit_tail(value, 16., b.w - 24.)
        } else {
            p.fit(value, 16., b.w - 24.)
        };
        p.text(&text, b.x + 12., b.y + 9., 16., TEXT);
        if self.edit_focus == id && (input.time * 2.).fract() < 0.65 {
            let x = (b.x + 12. + p.measure(&text, 16.)).min(b.x + b.w - 8.);
            draw_line(x, b.y + 10., x, b.y + 29., 1., ACCENT);
        }
    }
    /// A dropdown list: shows the current choice with ▾; clicking opens the list
    /// below it (drawn on top at the end of the dialog). Returns a new choice.
    fn dropdown(
        &mut self,
        p: &Painter,
        id: u32,
        b: Bounds,
        options: &[String],
        current: usize,
    ) -> Option<usize> {
        let pick = match self.dropdown_pick {
            Some((pid, i)) if pid == id => {
                self.dropdown_pick = None;
                Some(i)
            }
            _ => None,
        };
        let label = options.get(current).cloned().unwrap_or_default();
        let open = self.dropdown_open == Some(id);
        round(
            b,
            6.,
            if p.hover(b) || open {
                Color::new(0.22, 0.30, 0.39, 1.)
            } else {
                Color::new(0.14, 0.20, 0.28, 0.96)
            },
        );
        p.text(
            &p.fit(&label, 15., b.w - 44.),
            b.x + 12.,
            b.y + (b.h - 15.) / 2. - 2.,
            15.,
            TEXT,
        );
        // ASCII arrow marker avoids missing-glyph squares on some Windows fonts.
        p.text(
            "v",
            b.x + b.w - 23.,
            b.y + (b.h - 15.) / 2. - 2.,
            15.,
            MUTED,
        );
        if p.hover(b) && p.click && self.dropdown_suppress != Some(id) {
            self.dropdown_open = if open { None } else { Some(id) };
            if !open {
                self.dropdown_scroll = current.saturating_sub(3);
            }
        }
        if self.dropdown_open == Some(id) {
            self.dropdown_list = Some((id, b, options.to_vec(), current));
        }
        pick
    }
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
        let mut b = if settings {
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
        if settings {
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
                w: b.w - 105.,
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
                draw_line(
                    b.x + 194.,
                    b.y + 76.,
                    b.x + 194.,
                    b.y + b.h - 28.,
                    1.,
                    alpha(WHITE, 0.08),
                );
                // Group related controls by task: each action lives on one
                // settings page, avoiding duplicate Organize / Apply buttons.
                for (i, (label, id)) in [
                    ("Appearance", 0),
                    ("Behavior", 1),
                    ("Organize & rules", 2),
                    ("Folder paths", 10),
                    ("Roll-up", 6),
                    ("Pages", 7),
                    ("Displays", 9),
                    ("Peek", 8),
                    ("Saved layouts", 3),
                    ("Help", 4),
                ]
                .iter()
                .enumerate()
                {
                    if p.button(
                        label,
                        Bounds {
                            x: b.x + 18.,
                            y: b.y + 88. + i as f32 * 45.,
                            w: 160.,
                            h: 36.,
                        },
                        *page == *id,
                    ) {
                        *page = *id;
                        self.edit_focus = 0;
                        self.shortcut_recording = false;
                        self.settings_offset = 0;
                    }
                }
                let x = b.x + 222.;
                let y = b.y + 90.;
                let w = b.w - 250.;
                match *page {
                    0 | 5 => {
                        // Appearance (with the original manager's Personalization).
                        p.text("Appearance", x, y - 4., 23., TEXT);
                        let names: Vec<(Option<u64>, String)> =
                            std::iter::once((None, "All groups".to_string()))
                                .chain(
                                    self.state
                                        .groups
                                        .iter()
                                        .map(|g| (Some(g.id), g.tab().name.clone())),
                                )
                                .collect();
                        *target = (*target).min(names.len() - 1);
                        let which = names[*target].0;
                        let selected = which
                            .and_then(|id| self.state.group(id))
                            .or(self.state.groups.first());
                        let (custom, preset, opacity) = selected
                            .map_or((None, 0, 0.78), |g| (g.custom_color, g.color, g.opacity));
                        // Colour presets: a clearly visible colour each.
                        p.text("Group colour", x, y + 78., 14., MUTED);
                        for (i, c) in COLORS.iter().enumerate() {
                            let bb = Bounds {
                                x: x + 110. + i as f32 * 40.,
                                y: y + 72.,
                                w: 28.,
                                h: 28.,
                            };
                            round(bb, 6., *c);
                            if i == preset && custom == Some(preset_color(i)) {
                                outline(
                                    Bounds {
                                        x: bb.x - 3.,
                                        y: bb.y - 3.,
                                        w: 34.,
                                        h: 34.,
                                    },
                                    TEXT,
                                );
                            }
                            if input.pressed && bb.contains(input.pos.x, input.pos.y) {
                                for g in self
                                    .state
                                    .groups
                                    .iter_mut()
                                    .filter(|g| which.is_none_or(|id| id == g.id))
                                {
                                    g.color = i;
                                    g.custom_color = Some(preset_color(i));
                                }
                            }
                        }
                        if p.button(
                            "Default",
                            Bounds {
                                x: x + 110. + 6. * 40.,
                                y: y + 72.,
                                w: 80.,
                                h: 28.,
                            },
                            false,
                        ) {
                            for g in self
                                .state
                                .groups
                                .iter_mut()
                                .filter(|g| which.is_none_or(|id| id == g.id))
                            {
                                g.custom_color = None;
                                g.opacity = 0.78;
                            }
                        }
                        // Use the actual preset colour for editable HSV values. The
                        // old near-black default made Hue/Saturation appear broken
                        // until Brightness was adjusted first.
                        let base = custom.unwrap_or_else(|| preset_color(preset % 6));
                        let (mut hue, mut sat, mut val) = rgb_to_hsv(base);
                        let mut opacity = opacity;
                        let mut colour_changed = false;
                        let mut opacity_changed = false;
                        // True hue spectrum, rather than a generic grey slider.
                        p.text("Spectrum", x, y + 118., 14., MUTED);
                        let spectrum = Bounds {
                            x: x + 120.,
                            y: y + 119.,
                            w: w - 168.,
                            h: 15.,
                        };
                        let bands = 120;
                        for i in 0..bands {
                            let t = i as f32 / bands as f32;
                            let [r, g, b] = hsv_to_rgb(t, 1., 1.);
                            draw_rectangle(
                                spectrum.x + t * spectrum.w,
                                spectrum.y,
                                spectrum.w / bands as f32 + 0.5,
                                spectrum.h,
                                Color::new(r, g, b, 1.),
                            );
                        }
                        draw_rectangle_lines(
                            spectrum.x - 1.,
                            spectrum.y - 1.,
                            spectrum.w + 2.,
                            spectrum.h + 2.,
                            1.,
                            MUTED,
                        );
                        draw_circle(
                            spectrum.x + hue * spectrum.w,
                            spectrum.y + spectrum.h / 2.,
                            6.,
                            TEXT,
                        );
                        let hue_hit = Bounds {
                            x: spectrum.x - 7.,
                            y: spectrum.y - 12.,
                            w: spectrum.w + 14.,
                            h: 38.,
                        };
                        if input.down && hue_hit.contains(input.pos.x, input.pos.y) {
                            hue = ((input.pos.x - spectrum.x) / spectrum.w).clamp(0., 0.99999);
                            colour_changed = true;
                        }
                        for (i, (label, value)) in [
                            ("Brightness", &mut val),
                            ("Saturation", &mut sat),
                            ("Opacity", &mut opacity),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if let Some(v) = slider(
                                p,
                                input,
                                label,
                                *value,
                                x,
                                y + 148. + i as f32 * 27.,
                                w - 40.,
                            ) {
                                *value = v;
                                if i == 2 {
                                    opacity_changed = true;
                                } else {
                                    colour_changed = true;
                                }
                            }
                        }
                        let screen_rgb = hsv_to_rgb(hue, sat, val);
                        let hex = format!(
                            "#{:02X}{:02X}{:02X}",
                            (screen_rgb[0] * 255.).round() as u8,
                            (screen_rgb[1] * 255.).round() as u8,
                            (screen_rgb[2] * 255.).round() as u8
                        );
                        round(
                            Bounds {
                                x: x + 260.,
                                y: y + 247.,
                                w: 24.,
                                h: 22.,
                            },
                            4.,
                            Color::new(screen_rgb[0], screen_rgb[1], screen_rgb[2], 1.),
                        );
                        p.text(&hex, x + 295., y + 249., 13., TEXT);
                        if self.host.is_some() {
                            if p.button(
                                "Pick screen pixel",
                                Bounds {
                                    x,
                                    y: y + 242.,
                                    w: 180.,
                                    h: 29.,
                                },
                                false,
                            ) {
                                self.color_pick_target = Some(*target);
                                close = true;
                            }
                        }
                        if colour_changed || opacity_changed {
                            let rgb = hsv_to_rgb(hue, sat, val);
                            for g in self
                                .state
                                .groups
                                .iter_mut()
                                .filter(|g| which.is_none_or(|id| id == g.id))
                            {
                                if colour_changed {
                                    g.custom_color = Some(rgb);
                                }
                                if opacity_changed {
                                    g.opacity = opacity.clamp(0.15, 1.0);
                                }
                            }
                        }
                        p.text("Personalization", x, y + 282., 17., TEXT);
                        p.text("Titlebars", x, y + 329., 14., MUTED);
                        for (i, label) in ["Always visible", "On mouseover", "Never"]
                            .iter()
                            .enumerate()
                        {
                            if p.button(
                                label,
                                Bounds {
                                    x: x + 110. + i as f32 * 140.,
                                    y: y + 320.,
                                    w: 132.,
                                    h: 32.,
                                },
                                self.state.prefs.titlebar == i as u8,
                            ) {
                                self.state.prefs.titlebar = i as u8;
                            }
                        }
                        if p.checkbox(
                            "Tint icons with the group colour",
                            Bounds {
                                x,
                                y: y + 367.,
                                w,
                                h: 32.,
                            },
                            self.state.tint,
                        ) {
                            self.state.tint = !self.state.tint;
                        }
                        if p.checkbox(
                            "Hide scrollbars until mouseover",
                            Bounds {
                                x,
                                y: y + 404.,
                                w,
                                h: 32.,
                            },
                            self.state.prefs.hide_scrollbars,
                        ) {
                            self.state.prefs.hide_scrollbars = !self.state.prefs.hide_scrollbars;
                        }
                        if p.checkbox(
                            "Blend groups into the wallpaper until mouseover",
                            Bounds {
                                x,
                                y: y + 441.,
                                w,
                                h: 32.,
                            },
                            self.state.blend,
                        ) {
                            self.state.blend = !self.state.blend;
                        }
                        let on_desktop = self.host.is_some();
                        if p.button(
                            if on_desktop {
                                "Change wallpaper (Windows settings)"
                            } else {
                                "Change wallpaper"
                            },
                            Bounds {
                                x,
                                y: y + 490.,
                                w: 300.,
                                h: 34.,
                            },
                            false,
                        ) {
                            if on_desktop {
                                // The desktop's wallpaper is Windows' own.
                                let _ = platform::open("ms-settings:personalization-background");
                            } else {
                                self.state.wallpaper = (self.state.wallpaper + 1) % 3;
                            }
                        }
                        // Group chooser last, so its open list draws over the rest.
                        p.text("Group", x, y + 40., 14., MUTED);
                        let labels: Vec<String> = names.iter().map(|n| n.1.clone()).collect();
                        if let Some(i) = self.dropdown(
                            p,
                            1,
                            Bounds {
                                x: x + 110.,
                                y: y + 32.,
                                w: 330.,
                                h: 32.,
                            },
                            &labels,
                            *target,
                        ) {
                            *target = i;
                        }
                    }
                    1 => {
                        p.text("A calmer desktop", x, y, 23., TEXT);
                        if p.checkbox(
                            "Snap to other groups (hold Shift to bypass)",
                            Bounds {
                                x,
                                y: y + 60.,
                                w,
                                h: 36.,
                            },
                            self.state.snap,
                        ) {
                            self.state.snap = !self.state.snap
                        }
                        if p.checkbox(
                            "Double-click blank space to hide groups",
                            Bounds {
                                x,
                                y: y + 108.,
                                w,
                                h: 36.,
                            },
                            self.state.quick_hide,
                        ) {
                            self.state.quick_hide = !self.state.quick_hide
                        }
                        p.text("After moving a file between tabs", x, y + 158., 14., TEXT);
                        if p.button(
                            "Stay on source tab",
                            Bounds {
                                x,
                                y: y + 181.,
                                w: 226.,
                                h: 34.,
                            },
                            !self.state.prefs.switch_tab_after_drop,
                        ) {
                            self.state.prefs.switch_tab_after_drop = false;
                        }
                        if p.button(
                            "Switch to destination",
                            Bounds {
                                x: x + 238.,
                                y: y + 181.,
                                w: 226.,
                                h: 34.,
                            },
                            self.state.prefs.switch_tab_after_drop,
                        ) {
                            self.state.prefs.switch_tab_after_drop = true;
                        }
                        if self.host.is_some() {
                            p.text("On your Windows desktop", x, y + 226., 17., TEXT);
                            let on = host::autostart(None);
                            if p.checkbox(
                                "Start DesktopCraft when I sign in to Windows",
                                Bounds {
                                    x,
                                    y: y + 256.,
                                    w,
                                    h: 36.,
                                },
                                on,
                            ) {
                                host::autostart(Some(!on));
                            }
                            p.text(
                                "Right-click the desktop for \"New group here\". Double-click empty desktop to hide or show everything.",
                                x,
                                y + 316.,
                                12.,
                                MUTED,
                            );
                            p.text(
                                "Use Help to open diagnostics or exit DesktopCraft.",
                                x,
                                y + 366.,
                                13.,
                                MUTED,
                            );
                        } else {
                            p.text("Desktop window", x, y + 226., 17., TEXT);
                            p.text(
                                "Use a borderless workspace behind other windows.",
                                x,
                                y + 260.,
                                14.,
                                MUTED,
                            );
                            p.text(
                                "Press Escape to return to a normal window.",
                                x,
                                y + 286.,
                                14.,
                                MUTED,
                            );
                            if p.button(
                                if self.desktop {
                                    "Return to normal window"
                                } else {
                                    "Use desktop window"
                                },
                                Bounds {
                                    x,
                                    y: y + 328.,
                                    w: 230.,
                                    h: 38.,
                                },
                                false,
                            ) {
                                self.desktop = !self.desktop;
                                if !self.automation {
                                    platform::desktop(self.desktop, &mut self.previous)
                                }
                                close = true;
                            }
                            p.text(
                                "Customize the Peek shortcut on the Peek settings page.",
                                x,
                                y + 399.,
                                14.,
                                TEXT,
                            );
                            p.text(
                                "Press the chosen shortcut again, or Escape, to leave Peek.",
                                x,
                                y + 427.,
                                14.,
                                MUTED,
                            );
                        }
                    }
                    2 => {
                        p.text("Put files in their place", x, y, 23., TEXT);
                        p.text(
                            "Rules organize pins. The first matching rule wins.",
                            x,
                            y + 39.,
                            14.,
                            MUTED,
                        );
                        let modes = RULE_MODES;

                        let mode_options: Vec<String> =
                            modes.iter().map(|label| label.to_string()).collect();
                        if let Some(index) = self.dropdown(
                            p,
                            20,
                            Bounds {
                                x,
                                y: y + 76.,
                                w: 260.,
                                h: 34.,
                            },
                            &mode_options,
                            (*mode as usize).min(mode_options.len() - 1),
                        ) {
                            *mode = index as u8;
                        }
                        if *mode == 10 {
                            *pattern = "folder".into();
                        }
                        if *mode != 10 {
                            self.text_field(
                                p,
                                input,
                                rule_field_label(*mode),
                                pattern,
                                // As wide as the row allows beside Add rule, so long
                                // extension lists stay readable.
                                Bounds {
                                    x,
                                    y: y + 145.,
                                    w: w - 222.,
                                    h: 38.,
                                },
                                0,
                            );
                        } else {
                            p.text("All real folders on the Desktop", x, y + 155., 14., MUTED);
                        }
                        // Each regular tab is a distinct rule destination.
                        let choices = self.rule_destinations();
                        if !choices.is_empty() {
                            *target = (*target).min(choices.len() - 1);
                            let group_options: Vec<String> = choices
                                .iter()
                                .map(|(_, _, name)| format!("To: {name}"))
                                .collect();
                            if let Some(index) = self.dropdown(
                                p,
                                21,
                                Bounds {
                                    x: x + 276.,
                                    y: y + 76.,
                                    w: w - 276.,
                                    h: 34.,
                                },
                                &group_options,
                                *target,
                            ) {
                                *target = index;
                            }
                            if p.button(
                                "Add rule",
                                Bounds {
                                    x: x + w - 212.,
                                    y: y + 145.,
                                    w: 110.,
                                    h: 38.,
                                },
                                true,
                            ) {
                                if pattern.trim().is_empty() {
                                    self.notify("Enter a match for this rule.", input.time)
                                } else if *mode == 9 && size_range(pattern).is_none() {
                                    self.notify("Enter two byte sizes, such as 100,200, with the smaller first.", input.time)
                                } else if self.state.rules.len() >= MAX_RULES {
                                    self.notify(
                                        "All 50 rule slots are in use. Remove a rule first.",
                                        input.time,
                                    )
                                } else {
                                    self.state.rules.push(Rule {
                                        mode: *mode,
                                        pattern: pattern.trim().into(),
                                        target: choices[*target].0,
                                        target_tab: Some(choices[*target].1),
                                        enabled: true,
                                    });
                                    pattern.clear();
                                }
                            }
                        }
                        let mut remove = None;
                        let mut reorder = None;
                        let mut edit = None;
                        self.settings_offset = self
                            .settings_offset
                            .min(self.state.rules.len().saturating_sub(5));
                        for i in self.settings_offset
                            ..self.state.rules.len().min(self.settings_offset + 5)
                        {
                            let rowy = y + 211. + (i - self.settings_offset) as f32 * 43.;
                            let rule = &self.state.rules[i];
                            let dest = self
                                .state
                                .rule_destination(rule)
                                .and_then(|(gid, tid)| {
                                    self.state
                                        .group(gid)
                                        .and_then(|g| g.tabs.iter().find(|t| t.id == tid))
                                        .map(|t| t.name.as_str())
                                })
                                .unwrap_or("Missing tab");
                            let label = format!(
                                "{}: {}  to  {}",
                                modes.get(rule.mode as usize).unwrap_or(&"Rule"),
                                rule.pattern,
                                dest
                            );
                            p.text(&p.fit(&label, 14., w - 302.), x + 3., rowy + 9., 14., TEXT);
                            if p.button(
                                "Edit",
                                Bounds {
                                    x: x + w - 290.,
                                    y: rowy + 2.,
                                    w: 56.,
                                    h: 30.,
                                },
                                false,
                            ) {
                                edit = Some(i);
                            }
                            // A binary rule state needs one visible toggle, not
                            // a dropdown with only Enabled / Disabled choices.
                            if p.button(
                                if rule.enabled { "Enabled" } else { "Disabled" },
                                Bounds {
                                    x: x + w - 228.,
                                    y: rowy + 2.,
                                    w: 115.,
                                    h: 30.,
                                },
                                rule.enabled,
                            ) {
                                self.state.rules[i].enabled = !rule.enabled;
                            }
                            if p.button(
                                "Up",
                                Bounds {
                                    x: x + w - 100.,
                                    y: rowy + 2.,
                                    w: 38.,
                                    h: 30.,
                                },
                                false,
                            ) {
                                reorder = Some(i)
                            }
                            if p.button(
                                "×",
                                Bounds {
                                    x: x + w - 49.,
                                    y: rowy + 2.,
                                    w: 38.,
                                    h: 30.,
                                },
                                false,
                            ) {
                                remove = Some(i)
                            }
                        }
                        if let Some(i) = edit {
                            // Edit pops out its own window (like renaming a box),
                            // with room to show the whole rule.
                            open_instead = Some(self.rule_editor(i));
                        }
                        if let Some(i) = remove {
                            self.state.rules.remove(i);
                        }
                        if let Some(i) = reorder {
                            if i > 0 {
                                self.state.rules.swap(i, i - 1);
                            }
                        }
                        if self.state.rules.is_empty() {
                            p.text(
                                "No rules yet. Add one above to get started.",
                                x,
                                y + 225.,
                                14.,
                                MUTED,
                            );
                        }
                        // Action row and rule-list paging have distinct vertical
                        // space; neither the count nor the navigation buttons
                        // can overlap the Organize button at narrower DPIs.
                        if p.button(
                            "Apply rules now",
                            Bounds {
                                x,
                                y: y + 444.,
                                w: 180.,
                                h: 38.,
                            },
                            false,
                        ) {
                            let count = if self.host.is_some() {
                                self.sort_desktop_now()
                            } else {
                                self.state.apply_rules()
                            };
                            self.notify(format!("Applied rules to {count} item(s)"), input.time);
                        }
                        if self.host.is_some()
                            && p.button(
                                "Organize desktop now",
                                Bounds {
                                    x: x + 196.,
                                    y: y + 444.,
                                    w: 210.,
                                    h: 38.,
                                },
                                false,
                            )
                        {
                            self.auto_organize();
                            self.notify(
                                "Created desktop category groups (files remain on Desktop)",
                                input.time,
                            );
                            close = true;
                        }

                        if p.button(
                            "‹",
                            Bounds {
                                x: x,
                                y: y + 498.,
                                w: 32.,
                                h: 38.,
                            },
                            false,
                        ) {
                            self.settings_offset = self.settings_offset.saturating_sub(5);
                        }
                        if p.button(
                            "›",
                            Bounds {
                                x: x + 44.,
                                y: y + 498.,
                                w: 32.,
                                h: 38.,
                            },
                            false,
                        ) {
                            self.settings_offset = (self.settings_offset + 5)
                                .min(self.state.rules.len().saturating_sub(5));
                        }
                        p.text(
                            &format!("{} rules", self.state.rules.len()),
                            x + 96.,
                            y + 505.,
                            13.,
                            MUTED,
                        );
                    }
                    10 => {
                        p.text("Folder paths", x, y - 4., 23., TEXT);
                        p.text(
                            "View and update the folders linked to your boxes.",
                            x,
                            y + 32.,
                            14.,
                            MUTED,
                        );
                        let entries: Vec<(u64, usize, String, String)> = self
                            .state
                            .groups
                            .iter()
                            .flat_map(|g| {
                                g.tabs.iter().enumerate().filter_map(move |(ti, tab)| {
                                    tab.folder
                                        .as_ref()
                                        .map(|path| (g.id, ti, tab.name.clone(), path.clone()))
                                })
                            })
                            .collect();
                        p.text(
                            &format!("Linked folders ({})", entries.len()),
                            x,
                            y + 90.,
                            16.,
                            TEXT,
                        );
                        let visible = 5usize;
                        let max_offset = entries.len().saturating_sub(visible);
                        self.settings_offset = self.settings_offset.min(max_offset);
                        let mut unlink = None;
                        let mut open_path = None;
                        let mut change = None;
                        for (offset, (gid, ti, name, path)) in entries
                            .iter()
                            .enumerate()
                            .skip(self.settings_offset)
                            .take(visible)
                        {
                            let yy = y + 125. + (offset - self.settings_offset) as f32 * 67.;
                            p.text(&p.fit(name, 14., w - 242.), x, yy, 14., TEXT);
                            p.text(&p.fit(path, 11., w - 242.), x, yy + 22., 11., MUTED);
                            if p.button(
                                "Open",
                                Bounds {
                                    x: x + w - 232.,
                                    y: yy,
                                    w: 64.,
                                    h: 32.,
                                },
                                false,
                            ) {
                                open_path = Some(path.clone());
                            }
                            if p.button(
                                "Change",
                                Bounds {
                                    x: x + w - 160.,
                                    y: yy,
                                    w: 80.,
                                    h: 32.,
                                },
                                false,
                            ) {
                                change = Some((*gid, *ti, path.clone()));
                            }
                            if p.button(
                                "Unlink",
                                Bounds {
                                    x: x + w - 72.,
                                    y: yy,
                                    w: 72.,
                                    h: 32.,
                                },
                                false,
                            ) {
                                unlink = Some((*gid, *ti));
                            }
                        }
                        if let Some(path) = open_path {
                            if let Err(e) = platform::open(&path) {
                                self.notify(e, input.time);
                            }
                        }
                        if let Some((gid, ti, path)) = change {
                            if let Some(g) = self.state.group_mut(gid) {
                                g.active = ti;
                            }
                            self.dialog = Some(Dialog::BindFolder {
                                group: gid,
                                value: path,
                            });
                            return;
                        }
                        if let Some((gid, ti)) = unlink {
                            self.remember();
                            if let Some(tab) =
                                self.state.group_mut(gid).and_then(|g| g.tabs.get_mut(ti))
                            {
                                tab.folder = None;
                                tab.history.clear();
                                tab.items.clear();
                                tab.scroll = 0;
                            }
                            self.selected = None;
                            self.notify("Folder link removed; files remain on disk", input.time);
                        }
                        if entries.is_empty() {
                            p.text(
                                "No linked folders. Connect one through a box menu.",
                                x,
                                y + 146.,
                                14.,
                                MUTED,
                            );
                        }
                        if p.button(
                            "Previous",
                            Bounds {
                                x,
                                y: y + 480.,
                                w: 118.,
                                h: 34.,
                            },
                            false,
                        ) {
                            self.settings_offset = self.settings_offset.saturating_sub(visible);
                        }
                        if p.button(
                            "Next",
                            Bounds {
                                x: x + 132.,
                                y: y + 480.,
                                w: 118.,
                                h: 34.,
                            },
                            false,
                        ) {
                            self.settings_offset = (self.settings_offset + visible).min(max_offset);
                        }
                        p.text(
                            &format!(
                                "{}–{} of {}",
                                if entries.is_empty() {
                                    0
                                } else {
                                    self.settings_offset + 1
                                },
                                (self.settings_offset + visible).min(entries.len()),
                                entries.len()
                            ),
                            x + 275.,
                            y + 490.,
                            12.,
                            MUTED,
                        );
                    }
                    3 => {
                        p.text("Keep a layout you like", x, y, 23., TEXT);
                        p.text(
                            "Restore groups, tabs and pages from an earlier layout.",
                            x,
                            y + 39.,
                            14.,
                            MUTED,
                        );
                        if p.button(
                            "Save current layout",
                            Bounds {
                                x,
                                y: y + 78.,
                                w: 190.,
                                h: 38.,
                            },
                            true,
                        ) {
                            let name = format!("Layout {}", self.state.snapshots.len() + 1);
                            self.state.snapshot(&name);
                            self.notify("Layout saved", input.time);
                        }
                        let mut restore = None;
                        let mut remove = None;
                        self.settings_offset = self
                            .settings_offset
                            .min(self.state.snapshots.len().saturating_sub(5));
                        for (i, s) in self
                            .state
                            .snapshots
                            .iter()
                            .enumerate()
                            .rev()
                            .skip(self.settings_offset)
                            .take(5)
                        {
                            let row = self.state.snapshots.len() - 1 - i - self.settings_offset;
                            let yy = y + 145. + row as f32 * 64.;
                            round(
                                Bounds {
                                    x,
                                    y: yy,
                                    w,
                                    h: 54.,
                                },
                                6.,
                                Color::new(0.11, 0.17, 0.24, 1.),
                            );
                            p.text(&p.fit(&s.name, 15., w - 210.), x + 13., yy + 8., 15., TEXT);
                            p.text(
                                &format!("{} groups  ·  {} pages", s.groups.len(), s.pages.len()),
                                x + 13.,
                                yy + 29.,
                                11.,
                                MUTED,
                            );
                            if p.button(
                                "Restore",
                                Bounds {
                                    x: x + w - 150.,
                                    y: yy + 10.,
                                    w: 90.,
                                    h: 32.,
                                },
                                false,
                            ) {
                                restore = Some(i)
                            }
                            if p.button(
                                "×",
                                Bounds {
                                    x: x + w - 46.,
                                    y: yy + 10.,
                                    w: 32.,
                                    h: 32.,
                                },
                                false,
                            ) {
                                remove = Some(i)
                            }
                        }
                        if let Some(i) = restore {
                            self.remember();
                            self.state.restore(i);
                            self.selected = None;
                            self.notify(
                                "Layout restored. Ctrl + Z undoes this change.",
                                input.time,
                            );
                        }
                        if let Some(i) = remove {
                            self.state.snapshots.remove(i);
                        }
                        if p.button(
                            "‹ Newer",
                            Bounds {
                                x,
                                y: y + 488.,
                                w: 100.,
                                h: 34.,
                            },
                            false,
                        ) {
                            self.settings_offset = self.settings_offset.saturating_sub(5);
                        }
                        if p.button(
                            "Older ›",
                            Bounds {
                                x: x + 112.,
                                y: y + 488.,
                                w: 100.,
                                h: 34.,
                            },
                            false,
                        ) {
                            self.settings_offset = (self.settings_offset + 5)
                                .min(self.state.snapshots.len().saturating_sub(5));
                        }
                    }
                    6 => {
                        p.text("Roll-up", x, y, 23., TEXT);
                        p.text(
                            "Collapse a group into its title to keep icons tucked away.",
                            x,
                            y + 36.,
                            14.,
                            MUTED,
                        );
                        if p.checkbox(
                            "Enabled",
                            Bounds {
                                x,
                                y: y + 72.,
                                w,
                                h: 34.,
                            },
                            self.state.prefs.rollup,
                        ) {
                            self.state.prefs.rollup = !self.state.prefs.rollup;
                        }
                        if p.checkbox(
                            "Double-click a group's title to roll it up or down",
                            Bounds {
                                x,
                                y: y + 112.,
                                w,
                                h: 34.,
                            },
                            self.state.prefs.rollup_double_click,
                        ) {
                            self.state.prefs.rollup_double_click =
                                !self.state.prefs.rollup_double_click;
                        }
                        p.text("A rolled-up group opens", x, y + 162., 15., TEXT);
                        let modes: Vec<String> = [
                            "On mouse over, immediately",
                            "On mouse over, after a short delay",
                            "When its title is clicked",
                        ]
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                        if let Some(choice) = self.dropdown(
                            p,
                            2,
                            Bounds {
                                x,
                                y: y + 188.,
                                w: 330.,
                                h: 32.,
                            },
                            &modes,
                            self.state.prefs.rollup_open.min(2) as usize,
                        ) {
                            self.state.prefs.rollup_open = choice as u8;
                        }
                        if p.checkbox(
                            "Roll up groups dragged to the top edge of a screen",
                            Bounds {
                                x,
                                y: y + 238.,
                                w,
                                h: 34.,
                            },
                            self.state.prefs.rollup_dock,
                        ) {
                            self.state.prefs.rollup_dock = !self.state.prefs.rollup_dock;
                        }
                    }
                    7 => {
                        p.text("Pages", x, y, 23., TEXT);
                        p.text(
                            "Keep separate screens of groups and switch between them.",
                            x,
                            y + 36.,
                            14.,
                            MUTED,
                        );
                        if p.checkbox(
                            "Enable pages",
                            Bounds {
                                x,
                                y: y + 65.,
                                w,
                                h: 34.,
                            },
                            self.state.prefs.pages,
                        ) {
                            self.state.prefs.pages = !self.state.prefs.pages;
                        }
                        p.text(
                            "Switch with the bottom page selector or Ctrl / Alt + Left / Right.",
                            x,
                            y + 110.,
                            13.,
                            MUTED,
                        );
                        p.text(
                            "Only + creates a new page, after confirmation.",
                            x,
                            y + 137.,
                            13.,
                            MUTED,
                        );
                        p.text(
                            "Manage pages · rename any page, delete only empty pages",
                            x,
                            y + 177.,
                            14.,
                            TEXT,
                        );
                        let count = self.state.pages.len();
                        self.settings_offset = self.settings_offset.min(count.saturating_sub(4));
                        let mut deleting = None;
                        let mut renaming = None;
                        for index in self.settings_offset..count.min(self.settings_offset + 4) {
                            let row_y = y + 209. + (index - self.settings_offset) as f32 * 47.;
                            let assigned =
                                self.state.groups.iter().filter(|g| g.page == index).count();
                            let label = format!(
                                "{}  {}  ({} boxes)",
                                index + 1,
                                self.state.pages[index],
                                assigned
                            );
                            if p.button(
                                &p.fit(&label, 14., 324.),
                                Bounds {
                                    x,
                                    y: row_y,
                                    w: 330.,
                                    h: 34.,
                                },
                                self.state.page == index,
                            ) {
                                self.state.page = index;
                                self.selected = None;
                                if let Some(h) = self.host.as_mut() {
                                    h.last_sync = -10.;
                                }
                            }
                            if p.button(
                                "Rename",
                                Bounds {
                                    x: x + 338.,
                                    y: row_y,
                                    w: 83.,
                                    h: 34.,
                                },
                                false,
                            ) {
                                renaming = Some(index);
                            }
                            if assigned == 0 && count > 1 {
                                if p.button(
                                    "Delete",
                                    Bounds {
                                        x: x + 431.,
                                        y: row_y,
                                        w: 84.,
                                        h: 34.,
                                    },
                                    false,
                                ) {
                                    deleting = Some(index);
                                }
                            } else {
                                p.text(
                                    if count <= 1 { "Required" } else { "In use" },
                                    x + 439.,
                                    row_y + 9.,
                                    12.,
                                    MUTED,
                                );
                            }
                        }
                        if let Some(index) = renaming {
                            self.dialog = Some(Dialog::RenamePage {
                                index,
                                value: self.state.pages[index].clone(),
                            });
                            self.edit_focus = 0;
                            self.select_all = true;
                            close = true;
                        }
                        if let Some(index) = deleting {
                            let page_name =
                                self.state.pages.get(index).cloned().unwrap_or_default();
                            let approve = !self.automation && host::ask_default_no(&format!(
                                "Delete empty page '{}' (page {})?\n\nOnly the page will be removed. Your files remain unchanged.\n\nChoose No to keep it.",
                                page_name, index + 1));
                            if approve {
                                self.remember();
                                match self.state.delete_empty_page(index) {
                                    Ok(()) => {
                                        self.notify(
                                            "Empty page deleted; other page numbers adjusted",
                                            input.time,
                                        );
                                        if let Some(h) = self.host.as_mut() {
                                            h.last_sync = -10.;
                                        }
                                    }
                                    Err(reason) => self.notify(reason, input.time),
                                }
                            }
                        }
                        if p.button(
                            "+ New page",
                            Bounds {
                                x,
                                y: y + 430.,
                                w: 142.,
                                h: 34.,
                            },
                            false,
                        ) {
                            self.request_new_page();
                            close = true;
                        }
                        if p.button(
                            "‹",
                            Bounds {
                                x: x + 166.,
                                y: y + 430.,
                                w: 36.,
                                h: 34.,
                            },
                            false,
                        ) {
                            self.settings_offset = self.settings_offset.saturating_sub(4);
                        }
                        if p.button(
                            "›",
                            Bounds {
                                x: x + 210.,
                                y: y + 430.,
                                w: 36.,
                                h: 34.,
                            },
                            false,
                        ) {
                            self.settings_offset = (self.settings_offset + 4)
                                .min(self.state.pages.len().saturating_sub(4));
                        }
                    }
                    9 => {
                        p.text("Displays", x, y, 23., TEXT);
                        p.text(
                            "Choose which monitor hosts DesktopCraft' panels and controls.",
                            x,
                            y + 36.,
                            14.,
                            MUTED,
                        );
                        p.text(
                            "This does not change Windows' primary monitor or move your files.",
                            x,
                            y + 60.,
                            13.,
                            MUTED,
                        );
                        if let Some(h) = &self.host {
                            let monitors = h.monitors.clone();
                            let selected_key = self.state.prefs.workspace_monitor.clone();
                            let actual_primary = monitors
                                .iter()
                                .find(|m| m.primary)
                                .map(|m| m.key.as_str())
                                .unwrap_or("");
                            if p.button(
                                "Auto-detect Windows primary monitor",
                                Bounds {
                                    x,
                                    y: y + 99.,
                                    w: 400.,
                                    h: 38.,
                                },
                                selected_key.is_empty(),
                            ) {
                                self.state.prefs.workspace_monitor.clear();
                                self.settings_origin = None;
                            }
                            for (index, monitor) in monitors.iter().take(7).enumerate() {
                                let display = format!(
                                    "Display {}  |  {} × {}  |  {}{}",
                                    index + 1,
                                    monitor.rect[2],
                                    monitor.rect[3],
                                    monitor.key,
                                    if monitor.primary {
                                        "  (Windows primary)"
                                    } else {
                                        ""
                                    }
                                );
                                if p.button(
                                    &p.fit(&display, 14., w - 18.),
                                    Bounds {
                                        x,
                                        y: y + 150. + index as f32 * 43.,
                                        w: w - 18.,
                                        h: 36.,
                                    },
                                    selected_key == monitor.key,
                                ) {
                                    self.state.prefs.workspace_monitor = monitor.key.clone();
                                    self.settings_origin = None;
                                }
                            }
                            if selected_key.is_empty() {
                                p.text(
                                    &format!("Currently following {}", actual_primary),
                                    x,
                                    y + 479.,
                                    12.,
                                    MUTED,
                                );
                            } else if !monitors.iter().any(|m| m.key == selected_key) {
                                p.text("Saved display is disconnected; using Windows primary until it returns.", x, y + 479., 12., MUTED);
                            }
                        } else {
                            p.text("Available in Windows desktop mode. Display selection is automatic here.", x, y + 115., 14., MUTED);
                        }
                    }
                    8 => {
                        p.text("Peek", x, y, 23., TEXT);
                        p.text(
                            "Bring your groups above all windows for a moment.",
                            x,
                            y + 36.,
                            14.,
                            MUTED,
                        );
                        if p.checkbox(
                            "Show groups with hotkeys",
                            Bounds {
                                x,
                                y: y + 72.,
                                w,
                                h: 34.,
                            },
                            self.state.prefs.peek_hotkeys,
                        ) {
                            self.state.prefs.peek_hotkeys = !self.state.prefs.peek_hotkeys;
                        }
                        p.text("Peek shortcut", x, y + 122., 14., TEXT);
                        p.text(&self.state.prefs.peek_shortcut, x + 9., y + 181., 17., TEXT);
                        if p.button(
                            if self.shortcut_recording {
                                "Stop recording"
                            } else {
                                "Record shortcut"
                            },
                            Bounds {
                                x: x + 178.,
                                y: y + 174.,
                                w: 164.,
                                h: 42.,
                            },
                            false,
                        ) {
                            self.shortcut_recording = !self.shortcut_recording;
                            self.shortcut_candidate = None;
                            self.shortcut_recording_rejected = false;
                        }
                        if p.button(
                            "Reset default",
                            Bounds {
                                x: x + 352.,
                                y: y + 174.,
                                w: 138.,
                                h: 42.,
                            },
                            false,
                        ) {
                            self.state.prefs.peek_shortcut = default_peek_shortcut();
                            self.shortcut_recording = false;
                            self.shortcut_candidate = None;
                            self.shortcut_recording_rejected = false;
                        }
                        if self.shortcut_recording {
                            let held = platform::peek_pressed_keys();
                            if held.len() > 3 {
                                // A four-key press is not a valid recording.
                                self.shortcut_recording_rejected = true;
                                self.shortcut_candidate = None;
                            } else if held.len() >= 2 && !self.shortcut_recording_rejected {
                                if let Some(combo) = platform::describe_peek_keys(held.clone()) {
                                    // Keep the longest simultaneous chord seen.
                                    // Releasing one key from Z+X+C must not
                                    // silently overwrite it with just X+Z.
                                    let old_count = self
                                        .shortcut_candidate
                                        .as_ref()
                                        .map_or(0, |candidate| candidate.split('+').count());
                                    if held.len() >= old_count {
                                        self.shortcut_candidate = Some(combo);
                                    }
                                }
                            } else if held.is_empty() {
                                if self.shortcut_recording_rejected {
                                    self.shortcut_candidate = None;
                                    self.shortcut_recording_rejected = false;
                                } else if let Some(combo) = self.shortcut_candidate.take() {
                                    self.state.prefs.peek_shortcut = combo;
                                    self.shortcut_recording = false;
                                }
                            }
                            let status = if self.shortcut_recording_rejected {
                                "Too many keys. Release and try 2–3 keys."
                            } else {
                                self.shortcut_candidate
                                    .as_deref()
                                    .unwrap_or("Hold 2–3 keys")
                            };
                            p.text(
                                &p.fit(&format!("{status}  Release keys to save.",), 14., w),
                                x,
                                y + 238.,
                                14.,
                                ACCENT,
                            );
                        } else {
                            p.text(
                                "Record any 2–3 keys, then release. Hold them again for Peek.",
                                x,
                                y + 238.,
                                13.,
                                MUTED,
                            );
                        }
                        p.text(
                            "Release the shortcut to return to your desktop.",
                            x,
                            y + 311.,
                            13.,
                            MUTED,
                        );
                    }
                    _ => {
                        p.text("Your files, within reach", x, y, 23., TEXT);
                        for (i, (key, desc)) in [
                            (
                                "Drag title / box edge",
                                "Move or resize; snap to neighbor edges",
                            ),
                            ("Hold Shift while dragging", "Temporarily disable snapping"),
                            (
                                "Hover a rolled group's title",
                                "Reveal; move into the shown body to use it",
                            ),
                            ("Double-click its title", "Roll up or expand the box"),
                            ("Left-click ...", "Group menu and DesktopCraft settings"),
                            ("Box menu (...) ", "Manage tabs, folders and appearance"),
                            (
                                "Drag a header onto another",
                                "Combine as tabs; Shift-click to detach",
                            ),
                            ("Double-click a file", "Open with its default application"),
                            ("Ctrl + N / Ctrl + O", "New group / add a file"),
                            ("Ctrl + S / Ctrl + Z", "Save layout / undo"),
                            (
                                "Ctrl/Alt + Left / Right",
                                "Switch existing pages; add using +",
                            ),
                            ("Organize & rules", "Assign files by matching a rule"),
                            (
                                "Spectrum / Pick screen pixel",
                                "Sample any visible app; right-click cancels",
                            ),
                        ]
                        .iter()
                        .enumerate()
                        {
                            let yy = y + 48. + i as f32 * 24.;
                            p.text(key, x, yy, 12., ACCENT);
                            p.text(&p.fit(desc, 12., w - 240.), x + 235., yy, 12., TEXT);
                        }
                        p.text(
                            &format!("DesktopCraft v{}", env!("CARGO_PKG_VERSION")),
                            x,
                            y + 366.,
                            14.,
                            MUTED,
                        );
                        p.text("Diagnostics", x, y + 398., 15., TEXT);

                        if p.button(
                            "Open diagnostic logs",
                            Bounds {
                                x,
                                y: y + 442.,
                                w: 210.,
                                h: 37.,
                            },
                            false,
                        ) {
                            host::event(
                                "INFO",
                                "diagnostics.open_logs",
                                "log folder requested from settings",
                            );
                            if let Err(error) = platform::open(&host::log_dir().to_string_lossy()) {
                                self.notify(error, input.time);
                            }
                        }

                        if p.button(
                            "Exit DesktopCraft",
                            Bounds {
                                x,
                                y: y + 517.,
                                w: 210.,
                                h: 37.,
                            },
                            false,
                        ) {
                            if let Some(h) = self.host.as_mut() {
                                h.quit = true;
                            } else if !self.automation {
                                macroquad::miniquad::window::request_quit();
                            }
                            close = true;
                        }
                    }
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
            self.remember();
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
                        _ => self.notify("That rule or its box no longer exists.", input.time),
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
                    Err(reason) => self.notify(reason, input.time),
                },
                Dialog::RenamePage { index, value } => {
                    match self.state.rename_page(*index, value) {
                        Ok(()) => self.notify("Page renamed", input.time),
                        Err(reason) => self.notify(reason, input.time),
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
                    let path = Path::new(value.trim().trim_matches('"'));
                    match self.state.bind_group_folder(*group, path) {
                        Ok(()) => self.notify("Folder linked; files remain on disk.", input.time),
                        Err(e) => self.notify(e, input.time),
                    }
                }
                Dialog::Remove { group } => {
                    self.state.groups.retain(|g| g.id != *group);
                    self.selected = None;
                    self.notify("Group removed; its files are kept", input.time);
                }
                _ => {}
            }
            close = true;
        }
        if minimize {
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
        } else if settings {
            self.settings_drag_offset = None;
            self.settings_mouse_was_down = false;
            self.shortcut_recording = false;
            if self.host.is_some() {
                host::capture_pointer(false);
            }
            if self.color_pick_target.is_some() {
                host::begin_color_pick();
            }
        }
    }
}

/// Sorting rule kinds, in `Rule::mode` order.
pub(crate) const RULE_MODES: [&str; 11] = [
    "File extensions",
    "Name contains",
    "Name starts with",
    "Name ends with (no extension)",
    "Path contains",
    "Name does not contain",
    "Size at least (bytes)",
    "Size below (bytes)",
    "Modified within (days)",
    "Size between (bytes)",
    "Folder (any name)",
];
/// The label above a rule's match field, for its kind.
fn rule_field_label(mode: u8) -> &'static str {
    match mode {
        3 => "Ending before the extension",
        6 | 7 => "File size in bytes",
        8 => "Days",
        9 => "Minimum,maximum bytes (inclusive)",
        _ => "Match (example: txt,csv)",
    }
}
/// Break text into lines no wider than `width`, preferring to break after a
/// comma or space (rule lists), and inside a word only when it must.
pub(crate) fn wrap_text(p: &Painter, text: &str, size: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for token in text.split_inclusive([',', ' ']) {
        let joined = format!("{line}{token}");
        if p.measure(&joined, size) <= width {
            line = joined;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        for c in token.chars() {
            line.push(c);
            if p.measure(&line, size) > width && line.chars().count() > 1 {
                line.pop();
                lines.push(std::mem::take(&mut line));
                line.push(c);
            }
        }
    }
    lines.push(line);
    lines
}
impl App {
    /// Every regular tab a rule can send icons to: (box, tab, label).
    pub(crate) fn rule_destinations(&self) -> Vec<(u64, u64, String)> {
        self.state
            .groups
            .iter()
            .flat_map(|g| {
                g.tabs
                    .iter()
                    .filter(|tab| tab.folder.is_none())
                    .map(move |tab| {
                        let label = if g.tabs.len() > 1 {
                            format!("{} / {}", g.tabs[0].name, tab.name)
                        } else {
                            tab.name.clone()
                        };
                        (g.id, tab.id, label)
                    })
            })
            .collect()
    }
    /// The pop-up editor for rule `index`, filled in with its current values.
    pub(crate) fn rule_editor(&self, index: usize) -> Dialog {
        let rule = &self.state.rules[index];
        let dest = self.state.rule_destination(rule);
        let target = self
            .rule_destinations()
            .iter()
            .position(|(g, t, _)| dest == Some((*g, *t)))
            .unwrap_or(0);
        Dialog::EditRule {
            index,
            mode: rule.mode.min(RULE_MODES.len() as u8 - 1),
            pattern: rule.pattern.clone(),
            target,
        }
    }
    /// A multi-line text box: the whole value wraps onto as many lines as fit,
    /// and while typing the last lines (where the caret is) stay in view.
    fn text_area(
        &mut self,
        p: &Painter,
        input: &Input,
        label: &str,
        value: &mut String,
        b: Bounds,
    ) {
        p.text(label, b.x, b.y - 24., 13., MUTED);
        round(b, 5., Color::new(0.04, 0.075, 0.12, 1.));
        let focused = self.edit_focus == 0;
        outline(
            b,
            if focused {
                ACCENT
            } else {
                Color::new(0.24, 0.31, 0.40, 1.)
            },
        );
        if input.pressed && b.contains(input.pos.x, input.pos.y) {
            self.edit_focus = 0;
            self.select_all = false;
        }
        if focused {
            self.text_edit(value, input);
        }
        let (size, line_h) = (16., 24.);
        let lines = wrap_text(p, value, size, b.w - 24.);
        let fit = (((b.h - 16.) / line_h).floor() as usize).max(1);
        let first = lines.len().saturating_sub(fit);
        for (row, line) in lines.iter().skip(first).enumerate() {
            p.text(line, b.x + 12., b.y + 9. + row as f32 * line_h, size, TEXT);
        }
        if first > 0 {
            // Plain text, not an arrow glyph some Windows fonts lack.
            let note = format!(
                "{first} more line{} above",
                if first == 1 { "" } else { "s" }
            );
            p.text(
                &note,
                b.x + b.w - p.measure(&note, 12.),
                b.y - 22.,
                12.,
                MUTED,
            );
        }
        if focused && (input.time * 2.).fract() < 0.65 {
            let last = lines.last().map(String::as_str).unwrap_or("");
            let row = lines.len() - 1 - first;
            let cx = (b.x + 12. + p.measure(last, size)).min(b.x + b.w - 8.);
            let cy = b.y + 10. + row as f32 * line_h;
            draw_line(cx, cy, cx, cy + 19., 1., ACCENT);
        }
    }
}
/// A labelled 0..1 slider; returns the new value while it is being dragged.
fn slider(
    p: &Painter,
    input: &Input,
    label: &str,
    value: f32,
    x: f32,
    y: f32,
    w: f32,
) -> Option<f32> {
    p.text(label, x, y + 4., 14., TEXT);
    let track = Bounds {
        x: x + 120.,
        y: y + 12.,
        w: w - 120.,
        h: 5.,
    };
    round(track, 2., Color::new(0.2, 0.28, 0.38, 1.));
    round(
        Bounds {
            w: (track.w * value).max(1.),
            ..track
        },
        2.,
        ACCENT,
    );
    draw_circle(track.x + track.w * value, track.y + 2.5, 7., TEXT);
    let grab = Bounds {
        x: track.x - 6.,
        y: track.y - 12.,
        w: track.w + 12.,
        h: 30.,
    };
    (input.down && grab.contains(input.pos.x, input.pos.y))
        .then(|| ((input.pos.x - track.x) / track.w).clamp(0., 1.))
}
