use crate::*;

/// Direct Shell actions for keyboard shortcuts; right-click file menus are
/// entirely owned by Windows via native IContextMenu, never drawn here.
/// Some variants are only reached from tests now that Windows' own menu
/// performs those verbs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum FileAction {
    Open,
    OpenWith,
    Copy,
    Cut,
    PasteInto,
    Rename,
    Delete,
    Properties,
    Reveal,
}
/// Return only currently usable actions for the box ellipsis menu.
/// IDs deliberately match the action dispatcher; hidden items consume no
/// menu height or input hitbox.
fn visible_box_menu_indices(
    tab_count: usize,
    linked_folder: bool,
    selected_pin: bool,
) -> Vec<usize> {
    std::iter::once(0)
        .chain([15, 16])
        .chain(1..15)
        .filter(|i| match i {
            8 => tab_count > 1,
            9 => linked_folder || selected_pin,
            10 => !linked_folder && selected_pin,
            13 => linked_folder,
            _ => true,
        })
        .collect()
}

#[cfg(test)]
mod menu_visibility_tests {
    use super::*;
    #[test]
    fn empty_regular_box_shows_only_actionable_items() {
        let ids = visible_box_menu_indices(1, false, false);
        assert!(ids.contains(&14)); // bind a folder
        assert_eq!(ids.get(1), Some(&15)); // Add tab immediately below Rename group
        assert_eq!(ids.get(2), Some(&16)); // Paste files in active tab
        for i in [8, 9, 10, 13] {
            assert!(!ids.contains(&i));
        }
    }
    #[test]
    fn linked_folder_hides_unpin_but_shows_open_and_change_folder() {
        let ids = visible_box_menu_indices(1, true, false);
        assert!(ids.contains(&9) && ids.contains(&13) && ids.contains(&14));
        assert!(!ids.contains(&10));
    }
    #[test]
    fn selected_virtual_pin_enables_copy_and_unpin() {
        let ids = visible_box_menu_indices(2, false, true);
        assert!(ids.contains(&8) && ids.contains(&9) && ids.contains(&10));
        assert!(!ids.contains(&13));
    }
}
impl App {
    fn scroll_item_into_view(&mut self, id: u64, index: usize) {
        let Some(g) = self.state.group(id) else {
            return;
        };
        let tab = g.tab();
        let cols = if tab.list {
            1
        } else {
            ((g.bounds.w - 20.) / (cell_w() * tab.icon_scale))
                .floor()
                .max(1.) as usize
        };
        let content = g.bounds.h - HEADER - FOOTER - if tab.folder.is_some() { 38. } else { 12. };
        let rows = (content
            / if tab.list {
                36.
            } else {
                cell_h() * tab.icon_scale
            })
        .floor()
        .max(1.) as usize;
        let count = rows * cols;
        let next = if index < tab.scroll {
            index / cols * cols
        } else if index >= tab.scroll + count {
            (index / cols + 1).saturating_sub(rows) * cols
        } else {
            tab.scroll
        };
        self.state.group_mut(id).unwrap().tab_mut().scroll = next;
    }
    /// Close every pop-up (the "…" menu with its sub-lists, the folder sort
    /// list) and stop typing into a folder filter.
    pub(crate) fn close_popups(&mut self) {
        self.menu = None;
        self.file_menu = None;
        self.menu_page_open = false;
        self.menu_sort_open = false;
        self.menu_press = None;
        self.folder_sort_menu = None;
        self.filter_focus = None;
        self.popup_rects.clear();
    }
    /// Start renaming a loose Desktop icon without renaming a previously
    /// selected Box. Special shell items without a real path are not editable.
    pub(crate) fn start_inline_rename_loose(&mut self, path: &str) {
        if !Path::new(path).exists() {
            return;
        }
        let Some(l) = self
            .host
            .as_ref()
            .and_then(|h| h.loose.iter().find(|l| l.path.eq_ignore_ascii_case(path)))
            .cloned()
        else {
            return;
        };
        let filename = Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let edit = NameEdit::new(&l.label, &filename, Path::new(path).is_dir());
        self.close_popups();
        self.selected = None;
        self.multi_selected.clear();
        if let Some(h) = self.host.as_mut() {
            h.selected_loose = Some(path.to_string());
        }
        self.inline_rename = Some((0, 0, path.to_string(), edit));
    }
    /// Start renaming an icon in place, the way Explorer does (F2 or Rename in
    /// its right-click menu): the name, without a visible extension, starts selected.
    pub(crate) fn start_inline_rename(&mut self, group: u64, item: u64) {
        let Some(g) = self.state.group(group) else {
            return;
        };
        let Some((index, it)) = g.tab().items.iter().enumerate().find(|(_, i)| i.id == item) else {
            return;
        };
        let edit = NameEdit::new(it.display(), &it.name, it.kind == "folder");
        let path = it.path.clone();
        self.scroll_item_into_view(group, index);
        self.close_popups();
        self.selected = Some((group, item));
        self.multi_selected.clear();
        self.inline_rename = Some((group, item, path, edit));
    }
    /// Save an in-place rename (Enter, or clicking somewhere else, like Windows).
    pub(crate) fn commit_inline_rename(&mut self, time: f64) {
        let Some((_, _, path, edit)) = self.inline_rename.take() else {
            return;
        };
        self.inline_rename_rect = None;
        let Some(new_name) = edit.new_file_name() else {
            return;
        };
        if !self.rename_item(&path, &new_name, time) {
            return;
        }
        // Show the new name at once; Explorer's own label catches up later.
        let renamed = Path::new(&path)
            .with_file_name(&new_name)
            .to_string_lossy()
            .to_string();
        for g in &mut self.state.groups {
            for tab in &mut g.tabs {
                for it in &mut tab.items {
                    if it.path.eq_ignore_ascii_case(&renamed) && it.label.is_some() {
                        it.label = Some(edit.shown().to_string());
                    }
                }
            }
        }
    }
    /// Keys while renaming in place: typing edits the name, Enter saves,
    /// Escape cancels, and nothing else (like Delete on the icon) reacts.
    fn inline_rename_typing(&mut self, input: &Input) -> bool {
        let Some((group, item, _, _)) = self.inline_rename.as_ref() else {
            return false;
        };
        let still_there = self.dialog.is_none()
            && if *group == 0 {
                self.host.as_ref().is_some_and(|h| {
                    h.selected_loose.as_deref().is_some_and(|path| {
                        h.loose.iter().any(|l| l.path.eq_ignore_ascii_case(path))
                    })
                })
            } else {
                self.state
                    .group(*group)
                    .is_some_and(|g| g.tab().items.iter().any(|i| i.id == *item))
            };
        if !still_there {
            self.inline_rename = None;
            self.inline_rename_rect = None;
            return false;
        }
        if input.key(KeyCode::Escape) {
            self.inline_rename = None;
            self.inline_rename_rect = None;
            return true;
        }
        if input.key(KeyCode::Enter) || input.key(KeyCode::KpEnter) {
            self.commit_inline_rename(input.time);
            return true;
        }
        let paste = if input.ctrl && input.key(KeyCode::V) {
            macroquad::miniquad::window::clipboard_get().unwrap_or_default()
        } else {
            String::new()
        };
        let edit = &mut self.inline_rename.as_mut().unwrap().3;
        let mut refused = false;
        if input.ctrl && input.key(KeyCode::A) {
            edit.select_all();
        } else if input.ctrl && input.key(KeyCode::C) {
            macroquad::miniquad::window::clipboard_set(&edit.text);
        } else if !paste.is_empty() {
            refused = !edit.type_text(&paste.replace(['\r', '\n'], ""));
        } else if !input.ctrl && !input.typed.is_empty() {
            refused = !edit.type_text(&input.typed);
        }
        if input.key_or_repeat(KeyCode::Backspace) {
            edit.backspace();
        }
        if input.key_or_repeat(KeyCode::Delete) {
            edit.delete();
        }
        if input.key_or_repeat(KeyCode::Left) {
            edit.left();
        }
        if input.key_or_repeat(KeyCode::Right) {
            edit.right();
        }
        if input.key(KeyCode::Home) {
            edit.home();
        }
        if input.key(KeyCode::End) {
            edit.end();
        }
        if refused {
            self.notify(
                "A file name can't contain any of these characters: \\ / : * ? \" < > |",
                input.time,
            );
        }
        true
    }
    /// Draw the in-place rename box over the icon's label, Windows-style:
    /// white box, dark text, the selected part highlighted in blue.
    pub(super) fn draw_inline_rename(&mut self, p: &mut Painter, hits: &[Hit], time: f64) {
        self.inline_rename_rect = None;
        let Some((group, item, path, edit)) = self.inline_rename.as_ref() else {
            return;
        };
        let bounds = if *group == 0 {
            let index = self.host.as_ref().and_then(|h| {
                h.loose
                    .iter()
                    .position(|l| l.path.eq_ignore_ascii_case(path))
            });
            index
                .and_then(|index| {
                    hits.iter()
                        .rev()
                        .find(|h| h.target == Target::Loose(index))
                        .map(|h| h.bounds)
                })
                .map(|cell| {
                    let scale = self.host.as_ref().map_or(1., |h| h.scale);
                    let size = (48. / scale.max(1.)).min(cell.w - 16.).max(24.);
                    (12., cell.x - 6., cell.y + size + 5., cell.w + 12.)
                })
        } else {
            let cell = hits
                .iter()
                .rev()
                .find(|h| h.target == Target::Item(*group, *item))
                .map(|h| h.bounds);
            cell.and_then(|cell| {
                self.state.group(*group).map(|g| {
                    let tab = g.tab();
                    if tab.list {
                        (14., cell.x + 38., cell.y + 4., cell.w - 128.)
                    } else {
                        let size = (48. * tab.icon_scale)
                            .min(cell.w - 12.)
                            .min(cell.h - 36.)
                            .max(16.);
                        (12., cell.x - 6., cell.y + size + 5., cell.w + 12.)
                    }
                })
            })
        };
        let Some((font, x, y, w)) = bounds else {
            return;
        };
        let line_h = font + 4.;
        let mut lines = crate::dialogs::wrap_text(p, &edit.text, font, w - 8.);
        if lines.is_empty() {
            lines.push(String::new());
        }
        let b = Bounds {
            x,
            y,
            w,
            h: lines.len() as f32 * line_h + 6.,
        };
        draw_rectangle(b.x, b.y, b.w, b.h, WHITE);
        draw_rectangle_lines(b.x, b.y, b.w, b.h, 1., Color::new(0.0, 0.33, 0.6, 1.));
        let selected = edit.selected_chars();
        let dark = Color::new(0.05, 0.05, 0.05, 1.);
        let mut offset = 0;
        let mut caret_drawn = false;
        for (n, line) in lines.iter().enumerate() {
            let (lx, ly) = (b.x + 4., b.y + 3. + n as f32 * line_h);
            let count = line.chars().count();
            let in_line = selected.saturating_sub(offset).min(count);
            if in_line > 0 {
                // Selected part white on blue, then the rest right after it
                // (so a narrow "." beside the highlight stays visible).
                let picked: String = line.chars().take(in_line).collect();
                let rest: String = line.chars().skip(in_line).collect();
                let pw = p.measure(&picked, font);
                draw_rectangle(lx, ly - 1., pw, line_h, Color::new(0.0, 0.47, 0.84, 1.));
                p.text(&picked, lx, ly, font, WHITE);
                p.text(&rest, lx + pw, ly, font, dark);
            } else {
                p.text(line, lx, ly, font, dark);
            }
            if edit.selection == NameSelection::None
                && !caret_drawn
                && edit.caret >= offset
                && edit.caret <= offset + count
                && (time * 2.).fract() < 0.65
            {
                let before: String = line.chars().take(edit.caret - offset).collect();
                let cx = lx + p.measure(&before, font);
                draw_line(cx, ly - 1., cx, ly + font + 1., 1., dark);
                caret_drawn = true;
            }
            offset += count;
        }
        p.push_region(b, 0.);
        self.inline_rename_rect = Some(b);
    }
    /// Typing into a Folder View's filter: Enter keeps it, Escape clears it.
    fn filter_typing(&mut self, input: &Input) -> bool {
        let Some(id) = self.filter_focus else {
            return false;
        };
        let usable = self.dialog.is_none()
            && self
                .state
                .group(id)
                .is_some_and(|g| g.tab().folder.is_some());
        if !usable {
            self.filter_focus = None;
            return false;
        }
        let tab = self.state.group_mut(id).unwrap().tab_mut();
        if input.key(KeyCode::Escape) {
            tab.filter.clear();
            tab.scroll = 0;
            self.filter_focus = None;
            return true;
        }
        if input.key(KeyCode::Enter) || input.key(KeyCode::KpEnter) {
            self.filter_focus = None;
            return true;
        }
        let mut text = std::mem::take(&mut tab.filter);
        let before = text.clone();
        self.text_edit(&mut text, input);
        text.truncate(text.char_indices().nth(80).map_or(text.len(), |(i, _)| i));
        let tab = self.state.group_mut(id).unwrap().tab_mut();
        if text != before {
            tab.scroll = 0;
        }
        tab.filter = text;
        true
    }
    /// The Folder View sort list opened from the sort button in its folder bar.
    pub(super) fn folder_sort_list(&mut self, p: &mut Painter, input: &Input) {
        let Some((id, at)) = self.folder_sort_menu else {
            return;
        };
        let Some(current) = self.state.group(id).map(|g| g.tab().sort) else {
            self.folder_sort_menu = None;
            return;
        };
        let screen = self.menu_monitor_bounds(at);
        let h = 30. + 4. * 33. + 6.;
        let b = Bounds {
            x: (at.x - 180.).clamp(
                screen.x + 5.,
                (screen.x + screen.w - 213.).max(screen.x + 5.),
            ),
            y: (at.y + 16.).clamp(
                screen.y + 5.,
                (screen.y + screen.h - h - 5.).max(screen.y + 5.),
            ),
            w: 208.,
            h,
        };
        self.popup_rects.push(b);
        round(
            Bounds {
                x: b.x + 4.,
                y: b.y + 7.,
                ..b
            },
            8.,
            alpha(BLACK, 0.22),
        );
        round(b, 7., SURFACE);
        p.push_region(b, 7.);
        p.enabled = true;
        p.text("Sort files by", b.x + 13., b.y + 9., 12., MUTED);
        let mut choice = None;
        for (i, label) in ["Name (A–Z)", "File type", "Date (newest)", "Size (largest)"]
            .iter()
            .enumerate()
        {
            let row = Bounds {
                x: b.x + 5.,
                y: b.y + 30. + i as f32 * 33.,
                w: b.w - 10.,
                h: 32.,
            };
            if p.hover(row) {
                round(row, 4., Color::new(0.18, 0.26, 0.35, 1.));
                if input.pressed {
                    choice = Some(i as u8);
                }
            }
            let on = current == i as u8;
            if on {
                draw_line(
                    row.x + 9.,
                    row.y + 17.,
                    row.x + 14.,
                    row.y + 22.,
                    2.,
                    ACCENT,
                );
                draw_line(
                    row.x + 14.,
                    row.y + 22.,
                    row.x + 23.,
                    row.y + 10.,
                    2.,
                    ACCENT,
                );
            }
            p.text(
                label,
                row.x + 32.,
                row.y + 7.,
                14.,
                if on { ACCENT } else { TEXT },
            );
        }
        if let Some(sort) = choice {
            self.remember();
            if let Some(g) = self.state.group_mut(id) {
                g.tab_mut().sort = sort;
                g.tab_mut().sort_items();
                g.tab_mut().scroll = 0;
            }
            self.folder_sort_menu = None;
            self.notify(
                [
                    "Sorted by name",
                    "Sorted by type",
                    "Sorted by newest date",
                    "Sorted by largest size",
                ][sort as usize],
                input.time,
            );
        } else if input.pressed && !b.contains(input.pos.x, input.pos.y) {
            self.folder_sort_menu = None;
        }
    }
    pub(super) fn shortcuts(&mut self, input: &Input) {
        if self.inline_rename_typing(input) {
            return;
        }
        if self.filter_typing(input) {
            return;
        }
        if input.key(KeyCode::Escape) {
            self.folder_sort_menu = None;
            self.menu = None;
            self.search_focus = false;
            if matches!(self.dialog, Some(Dialog::EditRule { .. })) {
                // Escape leaves the rule pop-up for the rules page it came from.
                self.dialog = Some(Dialog::Settings {
                    page: 2,
                    pattern: String::new(),
                    mode: 0,
                    target: 0,
                });
            } else if self.dialog.is_some() {
                self.dialog = None;
            } else if self.peek {
                self.set_peek(false)
            } else if self.desktop {
                self.desktop = false;
                if !self.automation {
                    platform::desktop(false, &mut self.previous);
                }
            } else {
                self.search.clear();
            }
            self.drag = None;
            return;
        }
        // An unmodified-letter Peek chord (e.g. Z+X+C) must not also
        // trigger type-ahead, search or other box keyboard shortcuts.
        if self.dialog.is_some() || self.peek {
            return;
        }
        // Type-ahead selection behaves like Explorer. A focused box reacts to
        // ordinary letters (Latin or Thai) without opening global search.
        if !input.ctrl && !input.alt && !self.search_focus && !input.typed.is_empty() {
            if let Some(id) = self.group_id().filter(|id| {
                self.state
                    .group(*id)
                    .is_some_and(|g| g.page == self.state.page)
            }) {
                if self.type_group != Some(id) || input.time - self.typed_at > 1.2 {
                    self.typed_prefix.clear();
                }
                self.type_group = Some(id);
                self.typed_at = input.time;
                self.typed_prefix.push_str(&input.typed.to_lowercase());
                let matching = |prefix: &str| {
                    self.state.group(id).and_then(|g| {
                        g.tab()
                            .items
                            .iter()
                            .enumerate()
                            .find(|(_, item)| item.name.to_lowercase().starts_with(prefix))
                            .map(|(index, item)| (index, item.id))
                    })
                };
                let mut found = matching(&self.typed_prefix);
                if found.is_none() {
                    self.typed_prefix = input.typed.to_lowercase();
                    found = matching(&self.typed_prefix);
                }
                if let Some((index, item_id)) = found {
                    self.selected = Some((id, item_id));
                    self.scroll_item_into_view(id, index);
                }
                return;
            }
        }
        if input.ctrl && input.key(KeyCode::F) {
            self.search_focus = true;
            self.select_all = true;
            return;
        }
        if self.search_focus {
            let mut s = std::mem::take(&mut self.search);
            self.text_edit(&mut s, input);
            self.search = s;
            if input.key(KeyCode::Enter) {
                self.search_focus = false
            }
            return;
        }
        if input.ctrl && input.key(KeyCode::N) {
            self.create_group(self.fresh_bounds());
        }
        if input.ctrl && input.key(KeyCode::O) {
            if input.shift {
                self.pick_folder(input.time)
            } else {
                self.pick_file(input.time)
            }
        }
        if input.ctrl && input.key(KeyCode::S) {
            self.dialog = Some(Dialog::Snapshot {
                value: format!("Layout {}", self.state.snapshots.len() + 1),
            });
            self.select_all = true;
            self.edit_focus = 0;
        }
        if input.ctrl && input.key(KeyCode::Z) {
            if let Some(state) = self.undo.pop() {
                self.state = state;
                self.notify("Last change undone", input.time)
            }
        }
        // Only bottom page selector + Ctrl/Alt + arrows switch pages.
        // On the Windows desktop, keyboard input is polled only when its own
        // shell/our app has focus (no interference with other apps).
        if self.state.prefs.pages {
            if self.host.is_some() && !self.automation {
                match platform::page_hotkey() {
                    1 => self.switch_page(1),
                    -1 => self.switch_page(-1),
                    _ => {}
                }
            } else if (input.ctrl || input.alt) && !input.shift {
                if input.key(KeyCode::Right) {
                    self.switch_page(1);
                }
                if input.key(KeyCode::Left) {
                    self.switch_page(-1);
                }
            }
        }
        // Windows-standard Enter and Shift+F10 for loose icons and Box pins.
        // Settings and text fields retain their own keyboard handling.
        if self.dialog.is_none() && !self.search_focus && self.filter_focus.is_none() {
            let loose = self.host.as_ref().and_then(|h| h.selected_loose.clone());
            let pinned = self.selected.and_then(|(gid, iid)| {
                self.state.group(gid).and_then(|g| {
                    g.tab()
                        .items
                        .iter()
                        .find(|it| it.id == iid)
                        .map(|it| it.path.clone())
                })
            });
            // Alt+Enter opens the Windows Properties sheet, as on the desktop.
            if input.alt
                && !input.ctrl
                && (input.key(KeyCode::Enter) || input.key(KeyCode::KpEnter))
            {
                if let Some(path) = loose.clone().or(pinned.clone()) {
                    self.file_action(&path, FileAction::Properties, input.time);
                    return;
                }
            }
            if input.shift && input.key(KeyCode::F10) {
                if let Some(path) = loose.clone().or(pinned) {
                    self.close_popups();
                    if let Some(h) = self.host.as_mut() {
                        h.pending = Some(desktop::Pending::NativeFileMenu { path });
                    }
                    return;
                }
            }
            if !input.ctrl && !input.alt && input.key(KeyCode::Enter) {
                if let Some(path) = loose {
                    self.open(&path, input.time);
                    return;
                }
            }
        }
        // Ctrl+V: selected Box -> its active Tab, selected loose Desktop icon
        // or no selected Box -> the signed-in user's real Desktop folder.
        if self.dialog.is_none()
            && !self.search_focus
            && self.filter_focus.is_none()
            && input.ctrl
            && input.key(KeyCode::V)
        {
            let on_desktop = self
                .host
                .as_ref()
                .is_some_and(|h| h.selected_loose.is_some());
            let group = if on_desktop {
                None
            } else {
                self.selected.map(|(g, _)| g)
            };
            self.paste_file_clipboard(group, input.time);
            return;
        }
        // Real Windows file clipboard from both Box pins and loose icons.
        // Do not intercept Ctrl+C/X from Settings or an active text editor.
        if self.dialog.is_none()
            && !self.search_focus
            && self.filter_focus.is_none()
            && input.ctrl
            && (input.key(KeyCode::C) || input.key(KeyCode::X))
        {
            let loose = self.host.as_ref().and_then(|h| h.selected_loose.clone());
            let pinned = self.selected.and_then(|(gid, iid)| {
                self.state.group(gid).and_then(|g| {
                    g.tab()
                        .items
                        .iter()
                        .find(|i| i.id == iid)
                        .map(|i| i.path.clone())
                })
            });
            if let Some(path) = loose.or(pinned) {
                self.file_action(
                    &path,
                    if input.key(KeyCode::X) {
                        FileAction::Cut
                    } else {
                        FileAction::Copy
                    },
                    input.time,
                );
                return;
            }
        }
        // F2 on a selected icon renames it in place (as on the Windows desktop);
        // F2 on a box renames the box.
        let selected_icon = self.selected.filter(|(g, i)| {
            *i != 0
                && self
                    .state
                    .group(*g)
                    .is_some_and(|g| g.tab().items.iter().any(|it| it.id == *i))
        });
        if input.key(KeyCode::F2)
            && self.host.as_ref().is_some_and(|h| {
                h.selected_loose
                    .as_deref()
                    .is_some_and(|p| Path::new(p).exists())
            })
        {
            let path = self
                .host
                .as_ref()
                .and_then(|h| h.selected_loose.clone())
                .unwrap();
            self.start_inline_rename_loose(&path);
        } else if input.key(KeyCode::F2) && selected_icon.is_some() {
            let (group, item) = selected_icon.unwrap();
            self.start_inline_rename(group, item);
        } else if input.key(KeyCode::F2)
            && self
                .host
                .as_ref()
                .is_some_and(|h| h.selected_loose.is_some())
        {
            // Virtual shell item: do not accidentally rename a Box.
        } else if input.key(KeyCode::F2) {
            if let Some(id) = self.group_id() {
                self.dialog = Some(Dialog::Rename {
                    group: id,
                    value: self.state.group(id).unwrap().tab().name.clone(),
                });
                self.select_all = true;
                self.edit_focus = 0;
            }
        }
        if input.key(KeyCode::F1) {
            self.dialog = Some(Dialog::Settings {
                page: 4,
                pattern: String::new(),
                mode: 0,
                target: 0,
            });
        }
        // Desktop mode: Delete on a selected desktop icon sends it to the Recycle Bin.
        if input.key(KeyCode::Delete) {
            if let Some(path) = self.host.as_ref().and_then(|h| h.selected_loose.clone()) {
                if let Err(e) = shell::recycle(&path) {
                    self.notify(e, input.time);
                }
                if let Some(h) = self.host.as_mut() {
                    h.last_sync = -10.;
                    h.selected_loose = None;
                }
                return;
            }
        }
        if let Some((group, item)) = self.selected {
            if input.key(KeyCode::Delete) && self.host.is_some() {
                // Desktop mode: Delete sends the real item to the Recycle Bin,
                // exactly like on the desktop (Windows confirms as usual).
                let path = self
                    .state
                    .group(group)
                    .and_then(|g| g.tab().items.iter().find(|i| i.id == item))
                    .map(|i| i.path.clone());
                if let Some(path) = path {
                    if let Err(e) = shell::recycle(&path) {
                        self.notify(e, input.time);
                    }
                    if let Some(h) = self.host.as_mut() {
                        h.last_sync = -10.;
                    }
                }
            } else if input.key(KeyCode::Delete) {
                if self
                    .state
                    .group(group)
                    .is_some_and(|g| g.tab().folder.is_none())
                {
                    self.remember();
                    self.state
                        .group_mut(group)
                        .unwrap()
                        .tab_mut()
                        .items
                        .retain(|i| i.id != item);
                    self.notify("Pin removed from group", input.time);
                }
            }
            if input.key(KeyCode::Enter) {
                self.activate_item(group, item, input.time);
            }
            if input.shift
                && [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down]
                    .iter()
                    .any(|&k| input.key(k))
            {
                self.remember();
                if let Some(g) = self.state.group_mut(group) {
                    if !g.locked {
                        let d = if input.ctrl { 20. } else { 8. };
                        g.bounds.x += (if input.key(KeyCode::Right) { d } else { 0. })
                            - (if input.key(KeyCode::Left) { d } else { 0. });
                        g.bounds.y += (if input.key(KeyCode::Down) { d } else { 0. })
                            - (if input.key(KeyCode::Up) { d } else { 0. });
                    }
                }
            }
        }
        if input.key(KeyCode::Tab) {
            let groups: Vec<u64> = self
                .state
                .groups
                .iter()
                .filter(|g| g.page == self.state.page)
                .map(|g| g.id)
                .collect();
            if !groups.is_empty() {
                let idx = self
                    .group_id()
                    .and_then(|id| groups.iter().position(|x| *x == id))
                    .map_or(0, |i| (i + 1) % groups.len());
                self.selected = Some((groups[idx], 0));
            }
        }
        if !input.ctrl
            && !input.shift
            && [KeyCode::Left, KeyCode::Right, KeyCode::Up, KeyCode::Down]
                .iter()
                .any(|&k| input.key(k))
        {
            if let Some(id) = self.group_id() {
                let g = self.state.group(id).unwrap();
                if !g.tab().items.is_empty() {
                    let tab = g.tab();
                    let cols = if tab.list {
                        1
                    } else {
                        ((g.bounds.w - 20.) / (cell_w() * tab.icon_scale))
                            .floor()
                            .max(1.) as usize
                    };
                    let index = self
                        .selected
                        .and_then(|(_, item)| tab.items.iter().position(|x| x.id == item))
                        .unwrap_or(0);
                    let delta = if input.key(KeyCode::Right) {
                        1
                    } else if input.key(KeyCode::Left) {
                        -1
                    } else if input.key(KeyCode::Down) {
                        cols as i32
                    } else {
                        -(cols as i32)
                    };
                    let next = (index as i32 + delta).clamp(0, tab.items.len() as i32 - 1) as usize;
                    self.selected = Some((id, tab.items[next].id));
                    let available =
                        g.bounds.h - HEADER - FOOTER - if tab.folder.is_some() { 38. } else { 12. };
                    let rows = (available
                        / if tab.list {
                            36.
                        } else {
                            cell_h() * tab.icon_scale
                        })
                    .floor()
                    .max(1.) as usize;
                    let count = rows * cols;
                    let tab = self.state.group_mut(id).unwrap().tab_mut();
                    if next < tab.scroll {
                        tab.scroll = next / cols * cols;
                    } else if next >= tab.scroll + count {
                        tab.scroll = (next / cols + 1).saturating_sub(rows) * cols;
                    }
                }
            }
        }
    }
    pub(super) fn pointer(&mut self, input: &Input, hits: &[Hit]) {
        let hit = hits
            .iter()
            .rev()
            .find(|h| h.bounds.contains(input.pos.x, input.pos.y))
            .map(|h| h.target.clone());
        // In-place rename: a click inside its box keeps editing; a click
        // anywhere else saves it first (as Explorer does), then acts normally.
        if (input.pressed || input.right_pressed) && self.inline_rename.is_some() {
            if self
                .inline_rename_rect
                .is_some_and(|r| r.contains(input.pos.x, input.pos.y))
            {
                return;
            }
            self.commit_inline_rename(input.time);
        }
        // Desktop icons outside groups (desktop mode) handle their own clicks.
        if let Some(Target::Loose(i)) = hit {
            if self.drag.is_none() {
                if input.pressed || input.right_pressed {
                    self.loose_pointer(i, input);
                }
                return;
            }
            // A box item dragged over a loose icon must still receive its
            // release event instead of getting stuck in Drag::Item.
        }
        if input.pressed || input.right_pressed {
            if let Some(h) = self.host.as_mut() {
                h.selected_loose = None;
            }
        }
        if input.right_pressed {
            // DesktopCraft paints one Windows 11-style file menu while its
            // existing Shell API still handles real Windows file operations.
            if let Some(Target::Item(group, item)) = &hit {
                let path = self
                    .state
                    .group(*group)
                    .and_then(|g| g.tab().items.iter().find(|i| i.id == *item))
                    .map(|i| i.path.clone());
                self.close_popups();
                self.selected = Some((*group, *item));
                if let Some(path) = path {
                    self.file_menu = Some(ModernFilePopup {
                        path,
                        pos: input.pos,
                    });
                }
                return;
            }
        }
        // Right-clicks on a tab, header, and box body are intentionally inert.
        // Only the explicit ellipsis opens the group menu. File pins keep their
        // native Explorer right-click menu above.
        if input.right_pressed && hit.is_none() && self.host.is_none() {
            self.drag = Some(Drag::Create { start: input.pos });
        }
        // High-resolution Logitech free-spin wheels can send fractional
        // notches. Consume them smoothly without jumping dozens of rows or
        // switching pages for every tiny WM_MOUSEWHEEL packet.
        if input.scroll != 0. {
            let target = hit
                .as_ref()
                .map(target_group)
                .filter(|id| *id != 0 && self.state.group(*id).is_some());
            if self.wheel_target != target {
                self.wheel_target = target;
                self.wheel_remainder = 0.;
            }
            let delta = input.scroll.clamp(-8., 8.);
            self.wheel_remainder += delta;
            let notches = self.wheel_remainder.trunc() as i32;
            self.wheel_remainder -= notches as f32;
            if let Some(id) = target {
                let g = self.state.group_mut(id).unwrap();
                if input.ctrl {
                    g.tab_mut().icon_scale = (g.tab().icon_scale + delta * 0.1).clamp(0.7, 1.6);
                } else if notches != 0 {
                    let tab = g.tab();
                    let cols = if tab.list {
                        1
                    } else {
                        ((g.bounds.w - 20.) / (cell_w() * tab.icon_scale))
                            .floor()
                            .max(1.) as usize
                    };
                    let content =
                        g.bounds.h - HEADER - FOOTER - if tab.folder.is_some() { 38. } else { 12. };
                    let rows = (content
                        / if tab.list {
                            36.
                        } else {
                            cell_h() * tab.icon_scale
                        })
                    .floor()
                    .max(1.) as usize;
                    let next = scroll_rows(tab.scroll, tab.items.len(), rows * cols, cols, notches);
                    g.tab_mut().scroll = next;
                }
            }
        }
        if input.pressed {
            self.search_focus = false;
            // Clicking anywhere but the filter itself stops typing into it
            // (the filter stays applied until it is cleared).
            if !matches!(
                hit,
                Some(Target::FilterField(_)) | Some(Target::FolderFilter(_))
            ) {
                self.filter_focus = None;
            }
            let double_click = platform::double_click_interval();
            let double = self.last_click.as_ref().is_some_and(|(target, time, pos)| {
                Some(target) == hit.as_ref()
                    && input.time - time <= double_click
                    && pos.distance(input.pos) < DRAG_THRESHOLD
            });
            if let Some(ref h) = hit {
                self.last_click = if double {
                    None
                } else {
                    Some((h.clone(), input.time, input.pos))
                };
            } else {
                let blank = Target::Body(0);
                let dbl = self.last_click.as_ref().is_some_and(|(t, time, pos)| {
                    *t == blank
                        && input.time - time <= double_click
                        && pos.distance(input.pos) < DRAG_THRESHOLD
                });
                self.last_click = Some((blank, input.time, input.pos));
                // Window mode has its own blank canvas. In desktop mode only
                // the OS hook may toggle quick-hide, after checking Explorer's
                // actual desktop (never arbitrary application windows).
                if dbl && self.state.quick_hide && self.host.is_none() {
                    self.last_click = None;
                    self.hidden = !self.hidden;
                    self.drag = None;
                    return;
                }
            }
            match hit.clone() {
                Some(Target::Header(id)) => {
                    self.selected = Some((id, 0));
                    if double {
                        if input.ctrl {
                            if let Some(path) =
                                self.state.group(id).and_then(|g| g.tab().folder.clone())
                            {
                                self.open(&path, input.time)
                            }
                        } else if self.state.prefs.rollup && self.state.prefs.rollup_double_click {
                            self.remember();
                            let g = self.state.group_mut(id).unwrap();
                            g.rolled = !g.rolled;
                        }
                        return;
                    }
                    // Roll-up "open on click": a click on a rolled group's title opens it.
                    if self.state.prefs.rollup_open == 2
                        && self.state.group(id).is_some_and(|g| g.rolled)
                    {
                        self.revealed = Some(id);
                    }
                    let g = self.state.group(id).unwrap();
                    if !g.locked {
                        self.drag = Some(Drag::Group {
                            id,
                            start: input.pos,
                            before: g.bounds,
                        });
                        self.remember();
                    }
                }
                Some(Target::TabNext(id)) => {
                    if let Some(g) = self.state.group_mut(id) {
                        g.active = (g.active + 1) % g.tabs.len();
                        self.selected = Some((id, 0));
                    }
                }
                Some(Target::Tab(id, index)) => {
                    let tab_id = self.state.group(id).unwrap().tabs[index].id;
                    self.state.group_mut(id).unwrap().active = index;
                    self.selected = Some((id, 0));
                    if input.shift {
                        let bounds = self.state.group(id).unwrap().bounds;
                        if let Some(new) = self.state.detach(
                            id,
                            index,
                            Bounds {
                                x: bounds.x + 35.,
                                y: bounds.y + 55.,
                                ..bounds
                            },
                        ) {
                            self.drag = Some(Drag::Group {
                                id: new,
                                start: input.pos,
                                before: self.state.group(new).unwrap().bounds,
                            });
                        }
                    } else {
                        self.drag = Some(Drag::Tab {
                            group: id,
                            tab_id,
                            start: input.pos,
                        });
                    }
                }
                Some(Target::Roll(id)) => {
                    self.remember();
                    let g = self.state.group_mut(id).unwrap();
                    g.rolled = !g.rolled;
                }
                Some(Target::Menu(id)) => {
                    // Hold and drag the "…" button to move the box: when tabs
                    // fill the title bar it's the one handle always left. A
                    // plain click still opens the menu, on release.
                    self.selected = Some((id, 0));
                    let g = self.state.group(id).unwrap();
                    let movable = !g.locked;
                    if movable {
                        self.drag = Some(Drag::Group {
                            id,
                            start: input.pos,
                            before: g.bounds,
                        });
                        self.remember();
                    }
                    self.menu_press = Some((id, input.pos, movable));
                }
                Some(Target::FolderFilter(id)) => {
                    let active = self.filter_focus == Some(id)
                        || self
                            .state
                            .group(id)
                            .is_some_and(|g| !g.tab().filter.is_empty());
                    if active {
                        // A second click puts the filter away and shows every file.
                        if let Some(g) = self.state.group_mut(id) {
                            g.tab_mut().filter.clear();
                            g.tab_mut().scroll = 0;
                        }
                        self.filter_focus = None;
                    } else {
                        self.filter_focus = Some(id);
                        self.select_all = false;
                        self.selected = Some((id, 0));
                    }
                }
                Some(Target::FilterField(id)) => {
                    self.filter_focus = Some(id);
                    self.select_all = false;
                    self.selected = Some((id, 0));
                }
                Some(Target::FolderSort(id)) => {
                    self.selected = Some((id, 0));
                    self.folder_sort_menu = Some((id, input.pos));
                }
                Some(Target::Resize(id, edges)) => {
                    self.remember();
                    self.drag = Some(Drag::Resize {
                        id,
                        start: input.pos,
                        before: self.state.group(id).unwrap().bounds,
                        edges,
                    });
                }
                Some(Target::Item(group, item)) => {
                    let tab_id = self.state.group(group).map(|g| g.tab().id).unwrap_or(0);
                    let key = (group, tab_id, item);
                    if input.ctrl {
                        // Ctrl-click toggles individual pins in the selection.
                        if self.multi_selected.is_empty() {
                            if let Some((gid, iid)) = self.selected {
                                if gid == group && iid != 0 && iid != item {
                                    self.multi_selected.push((gid, tab_id, iid));
                                }
                            }
                        }
                        if self.multi_selected.contains(&key) {
                            self.multi_selected.retain(|entry| *entry != key);
                        } else {
                            self.multi_selected.push(key);
                        }
                    } else if input.shift {
                        // Shift-click selects a contiguous range in this tab.
                        let previous = self
                            .selected
                            .and_then(|(gid, iid)| (gid == group).then_some(iid));
                        let positions = self.state.group(group).map(|g| &g.tab().items);
                        if let Some(items) = positions {
                            let end = items.iter().position(|i| i.id == item);
                            let begin =
                                previous.and_then(|iid| items.iter().position(|i| i.id == iid));
                            if let (Some(a), Some(b)) = (begin, end) {
                                self.multi_selected = (a.min(b)..=a.max(b))
                                    .map(|idx| (group, tab_id, items[idx].id))
                                    .collect();
                            } else {
                                self.multi_selected = vec![key];
                            }
                        }
                    } else if !self.multi_selected.contains(&key) {
                        self.multi_selected.clear();
                    }
                    self.selected = if input.ctrl && self.multi_selected.is_empty() {
                        Some((group, 0))
                    } else {
                        Some((group, item))
                    };
                    if double && !input.ctrl && !input.shift {
                        self.multi_selected.clear();
                        self.activate_item(group, item, input.time);
                        self.drag = None;
                    } else if !input.ctrl && !input.shift {
                        self.drag = Some(Drag::Item {
                            group,
                            item,
                            start: input.pos,
                        });
                        if self.host.is_some() {
                            host::capture_pointer(true);
                        }
                    } else {
                        self.drag = None;
                    }
                }
                Some(Target::Back(id)) => {
                    let tab = self.state.group_mut(id).unwrap().tab_mut();
                    if tab.history.is_empty() {
                        if let Some(parent) = tab
                            .folder
                            .as_ref()
                            .and_then(|x| Path::new(x).parent())
                            .map(|x| x.to_string_lossy().to_string())
                        {
                            if let Err(e) = tab.navigate(&parent) {
                                self.notify(e, input.time)
                            }
                        }
                    } else if let Err(e) = tab.back() {
                        self.notify(e, input.time)
                    }
                }
                Some(Target::Open(id)) => {
                    if let Some(path) = self.state.group(id).and_then(|g| g.tab().folder.clone()) {
                        self.open(&path, input.time)
                    }
                }
                Some(Target::Scroll {
                    group,
                    track_y,
                    track_h,
                    thumb_y,
                    thumb_h,
                    total,
                    page,
                    cols,
                }) => {
                    // Grab the thumb where it was pressed; a press on the track
                    // centres the thumb there (and keeps following the pointer).
                    let grab = if input.pos.y >= thumb_y && input.pos.y <= thumb_y + thumb_h {
                        input.pos.y - thumb_y
                    } else {
                        thumb_h / 2.
                    };
                    let row = scroll_row_at(input.pos.y, track_y, track_h, total, page, grab);
                    if let Some(g) = self.state.group_mut(group) {
                        g.tab_mut().scroll = row * cols;
                    }
                    self.drag = Some(Drag::Scroll {
                        group,
                        track_y,
                        track_h,
                        total,
                        page,
                        cols,
                        grab,
                    });
                }
                Some(Target::Loose(_)) => {}
                Some(Target::Body(id)) => {
                    self.selected = Some((id, 0));
                    if double {
                        if let Some(path) =
                            self.state.group(id).and_then(|g| g.tab().folder.clone())
                        {
                            self.open(&path, input.time);
                        }
                    } else if let Some(tab) = self.state.group(id).map(|g| g.tab()) {
                        if tab.folder.is_none() {
                            self.multi_selected.clear();
                            self.drag = Some(Drag::BoxSelect {
                                group: id,
                                tab_id: tab.id,
                                start: input.pos,
                            });
                            if self.host.is_some() {
                                host::capture_pointer(true);
                            }
                        }
                    }
                }
                None => {
                    if input.pos.y > 124. && input.pos.y < self.height - 60. {
                        self.drag = Some(Drag::Create { start: input.pos });
                        self.selected = None;
                    }
                }
            }
        }
        if input.down || input.right_down || input.middle_down {
            match self.drag.clone() {
                Some(Drag::Group { id, start, before }) => 'moving: {
                    let delta = input.pos - start;
                    let g = self.state.group(id).unwrap();
                    // A press only becomes a move after DRAG_THRESHOLD px, so a
                    // click (or a slightly shaky double-click) never nudges a group.
                    if g.bounds == before && delta.length() <= DRAG_THRESHOLD {
                        break 'moving;
                    }
                    // Window mode keeps groups below its toolbar and above its page
                    // bar; on the desktop a group may go anywhere on the screens,
                    // right up to the top edge.
                    let (left, top, right, bottom) = if self.host.is_some() {
                        (0., 0., self.width - g.bounds.w, self.height - HEADER)
                    } else {
                        (
                            8.,
                            124.,
                            self.width - g.bounds.w - 8.,
                            self.height - HEADER - 54.,
                        )
                    };
                    let mut x = (before.x + delta.x).clamp(left, right.max(left));
                    let mut y = (before.y + delta.y).clamp(top, bottom.max(top));
                    if self.state.snap && !input.shift {
                        let snapped = self.state.snap_position(id, Bounds { x, y, ..g.bounds });
                        x = snapped.x;
                        y = snapped.y;
                    }
                    let g = self.state.group_mut(id).unwrap();
                    g.bounds.x = x;
                    g.bounds.y = y;
                    // Edge-drag page switching intentionally removed. A box
                    // can be moved to another page through its explicit menu.
                }
                Some(Drag::Scroll {
                    group,
                    track_y,
                    track_h,
                    total,
                    page,
                    cols,
                    grab,
                }) => {
                    let row = scroll_row_at(input.pos.y, track_y, track_h, total, page, grab);
                    if let Some(g) = self.state.group_mut(group) {
                        g.tab_mut().scroll = row * cols;
                    }
                }
                Some(Drag::Item { group, item, start }) if self.host.is_some() => {
                    // Keep Box -> Box and Box -> Desktop inside DesktopCraft.
                    // Those are changes to icon placement, NOT filesystem moves.
                    // Only start Windows OLE dragging when entering another app.
                    let over_group = self.state.groups.iter().any(|g| {
                        g.page == self.state.page && g.bounds.contains(input.pos.x, input.pos.y)
                    });
                    let source_tab = self.state.group(group).map(|g| g.tab().id);
                    let multi_count = self
                        .multi_selected
                        .iter()
                        .filter(|(gid, tid, _)| *gid == group && Some(*tid) == source_tab)
                        .count();
                    if multi_count <= 1 && !over_group && start.distance(input.pos) > DRAG_THRESHOLD
                    {
                        let (x, y) = host::cursor();
                        if host::over_other_app(x, y) {
                            let path = self
                                .state
                                .group(group)
                                .and_then(|g| g.tab().items.iter().find(|i| i.id == item))
                                .map(|i| i.path.clone());
                            if let (Some(path), Some(h)) = (path, self.host.as_mut()) {
                                h.pending = Some(desktop::Pending::Drag { path });
                            }
                            host::capture_pointer(false);
                            self.drag = None;
                        }
                    }
                }
                Some(Drag::Resize {
                    id,
                    start,
                    before,
                    edges,
                }) => {
                    let host = self.host.is_some();
                    let area = if host {
                        Bounds {
                            x: 0.,
                            y: 0.,
                            w: self.width,
                            h: self.height,
                        }
                    } else {
                        Bounds {
                            x: 8.,
                            y: 124.,
                            w: self.width - 16.,
                            h: self.height - 124. - 58.,
                        }
                    };
                    let min = min_size(self.state.group(id).unwrap().tab().folder.is_some());
                    let resized = resize_bounds(before, input.pos - start, edges, min, area);
                    let result = if self.state.snap && !input.shift {
                        self.state.snap_resize(id, resized, edges, min, area)
                    } else {
                        resized
                    };
                    self.state.group_mut(id).unwrap().bounds = result;
                }
                _ => {}
            }
        }
        if input.released || input.right_released || input.middle_released {
            if self.host.is_some() {
                host::capture_pointer(false);
            }
            // A "…" press that never moved its box is a click: open the menu.
            let menu_press = self.menu_press.take();
            let menu_moved = match (&self.drag, &menu_press) {
                (Some(Drag::Group { id, before, .. }), Some((pressed, _, _))) if id == pressed => {
                    self.state.group(*id).is_some_and(|g| g.bounds != *before)
                }
                _ => false,
            };
            if let Some((id, at, added_undo)) = menu_press {
                if !menu_moved && input.released && self.state.group(id).is_some() {
                    if added_undo {
                        self.undo.pop();
                    } // nothing changed
                    self.drag = None;
                    self.menu = Some((id, at));
                    self.menu_page_open = false;
                    self.menu_page_scroll = 0;
                    self.menu_sort_open = false;
                    return;
                }
            }
            match self.drag.take() {
                Some(Drag::BoxSelect {
                    group,
                    tab_id,
                    start,
                }) => {
                    if start.distance(input.pos) > DRAG_THRESHOLD {
                        let area = Bounds {
                            x: start.x.min(input.pos.x),
                            y: start.y.min(input.pos.y),
                            w: (start.x - input.pos.x).abs(),
                            h: (start.y - input.pos.y).abs(),
                        };
                        let still_active = self
                            .state
                            .group(group)
                            .is_some_and(|g| g.tab().id == tab_id && g.tab().folder.is_none());
                        if still_active {
                            self.multi_selected = hits
                                .iter()
                                .filter_map(|h| match h.target {
                                    Target::Item(gid, iid)
                                        if gid == group
                                            && area.x < h.bounds.x + h.bounds.w
                                            && area.x + area.w > h.bounds.x
                                            && area.y < h.bounds.y + h.bounds.h
                                            && area.y + area.h > h.bounds.y =>
                                    {
                                        Some((group, tab_id, iid))
                                    }
                                    _ => None,
                                })
                                .collect();
                            self.multi_selected.sort_unstable();
                            self.multi_selected.dedup();
                            self.selected = self
                                .multi_selected
                                .last()
                                .map(|(gid, _, iid)| (*gid, *iid))
                                .or(Some((group, 0)));
                        }
                    }
                }
                Some(Drag::Tab {
                    group,
                    tab_id,
                    start,
                }) => {
                    if start.distance(input.pos) > DRAG_THRESHOLD {
                        let target = self
                            .state
                            .group(group)
                            .and_then(|g| g.drop_tab_at(input.pos.x, input.pos.y));
                        if let Some(target) = target {
                            let from = self
                                .state
                                .group(group)
                                .and_then(|g| g.tabs.iter().position(|tab| tab.id == tab_id));
                            if from != Some(target) {
                                self.remember();
                                if self.state.reorder_tab(group, tab_id, target) {
                                    self.selected = Some((group, 0));
                                    self.notify("Tab order updated", input.time);
                                }
                            }
                        }
                    }
                }
                Some(Drag::Group { id, start, .. }) => {
                    if start.distance(input.pos) > DRAG_THRESHOLD {
                        self.dock_if_at_edge(id);
                        if let Some(h) = self.host.as_mut() {
                            h.last_sync = -10.;
                        }
                        self.last_click = None;
                        let target = self
                            .state
                            .groups
                            .iter()
                            .rev()
                            .find(|g| {
                                g.id != id
                                    && g.page == self.state.page
                                    && Bounds {
                                        h: HEADER,
                                        ..g.bounds
                                    }
                                    .contains(input.pos.x, input.pos.y)
                            })
                            .map(|g| g.id);
                        if let Some(target) = target {
                            if self.state.merge(id, target) {
                                self.selected = Some((target, 0));
                                self.notify(
                                    "Groups combined as tabs. Shift-click a tab to separate it.",
                                    input.time,
                                )
                            }
                        }
                    }
                }
                Some(Drag::Item { group, item, start }) => {
                    if start.distance(input.pos) > DRAG_THRESHOLD {
                        self.last_click = None;
                        // A tab strip is a drop target even when its tab is inactive.
                        let tab_target = hits.iter().rev().find_map(|h| {
                            if !h.bounds.contains(input.pos.x, input.pos.y) {
                                return None;
                            }
                            if let Target::Tab(gid, index) = &h.target {
                                Some((*gid, *index))
                            } else {
                                None
                            }
                        });
                        let target = self
                            .state
                            .groups
                            .iter()
                            .rev()
                            .find(|g| {
                                g.page == self.state.page
                                    && g.bounds.contains(input.pos.x, input.pos.y)
                            })
                            .map(|g| g.id);
                        if self.host.is_some() {
                            host::log(&format!(
                                "drag-release: src={group} item={item} target={target:?} at={:?}",
                                input.pos
                            ));
                        }
                        let source_tab = self.state.group(group).map(|g| g.tab().id);
                        let batch_ids: Vec<_> = self
                            .multi_selected
                            .iter()
                            .filter_map(|(gid, tid, iid)| {
                                (Some(*gid) == Some(group) && Some(*tid) == source_tab)
                                    .then_some(*iid)
                            })
                            .collect();
                        let batch = batch_ids.len() > 1 && batch_ids.contains(&item);
                        if let Some(target) = target {
                            if batch {
                                let destination_tab = self.state.group(target).and_then(|g| {
                                    tab_target
                                        .filter(|(gid, _)| *gid == target)
                                        .and_then(|(_, idx)| g.tabs.get(idx))
                                        .or_else(|| g.tabs.get(g.active))
                                        .map(|t| t.id)
                                });
                                if let (Some(src), Some(dst)) = (source_tab, destination_tab) {
                                    if src == dst && target == group {
                                        self.notify(
                                            "Selection unchanged: choose another tab or Box",
                                            input.time,
                                        );
                                    } else {
                                        self.remember();
                                        match self.state.transfer_many_between_tabs(
                                            group, src, &batch_ids, target, dst,
                                        ) {
                                            Ok(count) => {
                                                if self.state.prefs.switch_tab_after_drop {
                                                    if let Some(g) = self.state.group_mut(target) {
                                                        if let Some(i) =
                                                            g.tabs.iter().position(|t| t.id == dst)
                                                        {
                                                            g.active = i;
                                                        }
                                                    }
                                                    self.multi_selected = batch_ids
                                                        .iter()
                                                        .map(|id| (target, dst, *id))
                                                        .collect();
                                                    self.selected = Some((target, item));
                                                } else {
                                                    self.multi_selected.clear();
                                                    self.selected = Some((group, 0));
                                                }
                                                if let Some(h) = self.host.as_mut() {
                                                    h.last_sync = -10.;
                                                }
                                                self.notify(
                                                    format!(
                                                        "Moved {count} pins (disk files unchanged)"
                                                    ),
                                                    input.time,
                                                );
                                            }
                                            Err(e) => {
                                                self.undo.pop();
                                                self.notify(e, input.time);
                                            }
                                        }
                                    }
                                }
                            } else if self.host.is_some()
                                && self
                                    .state
                                    .group(target)
                                    .is_some_and(|g| g.tab().folder.is_some())
                            {
                                let path = self.state.group(group).and_then(|g| {
                                    g.tab()
                                        .items
                                        .iter()
                                        .find(|i| i.id == item)
                                        .map(|i| i.path.clone())
                                });
                                if let Some(path) = path {
                                    if let Err(e) =
                                        self.drop_path_into_box(&path, target, input.time)
                                    {
                                        self.notify(e, input.time);
                                    }
                                }
                            } else if self.host.is_some()
                                && self
                                    .state
                                    .group(group)
                                    .is_some_and(|g| g.tab().folder.is_some())
                            {
                                // A Folder View remains a view of the real folder.
                                // Pin its file to the normal box without moving it.
                                let path = self.state.group(group).and_then(|g| {
                                    g.tab()
                                        .items
                                        .iter()
                                        .find(|i| i.id == item)
                                        .map(|i| i.path.clone())
                                });
                                if let Some(path) = path {
                                    if let Err(e) =
                                        self.drop_path_into_box(&path, target, input.time)
                                    {
                                        self.notify(e, input.time);
                                    }
                                }
                            } else {
                                // Regular Box -> Box: only the virtual assignment
                                // changes; original file stays at the same path.
                                self.remember();
                                let source_tab = self.state.group(group).map(|g| g.tab().id);
                                let destination_tab = self.state.group(target).and_then(|g| {
                                    tab_target
                                        .filter(|(gid, _)| *gid == target)
                                        .and_then(|(_, idx)| g.tabs.get(idx))
                                        .or_else(|| g.tabs.get(g.active))
                                        .map(|t| t.id)
                                });
                                if let (Some(source_tab), Some(destination_tab)) =
                                    (source_tab, destination_tab)
                                {
                                    let index = self
                                        .state
                                        .group(target)
                                        .and_then(|g| {
                                            g.tabs.iter().find(|t| t.id == destination_tab)
                                        })
                                        .map_or(0, |t| t.items.len());
                                    match self.state.transfer_between_tabs(
                                        group,
                                        source_tab,
                                        item,
                                        target,
                                        destination_tab,
                                        index,
                                    ) {
                                        Ok(()) => {
                                            if self.state.prefs.switch_tab_after_drop {
                                                if let Some(g) = self.state.group_mut(target) {
                                                    if let Some(i) = g
                                                        .tabs
                                                        .iter()
                                                        .position(|t| t.id == destination_tab)
                                                    {
                                                        g.active = i;
                                                    }
                                                }
                                            }
                                            self.selected =
                                                if self.state.prefs.switch_tab_after_drop {
                                                    Some((target, item))
                                                } else {
                                                    Some((group, 0))
                                                };
                                            self.notify(
                                                "Pin moved to tab (file unchanged)",
                                                input.time,
                                            );
                                        }
                                        Err(e) => self.notify(e, input.time),
                                    }
                                }
                            }
                        } else if batch {
                            self.notify(
                                "Select another Box or Tab to move the selected pins",
                                input.time,
                            );
                        } else if self.host.is_some() {
                            let (x, y) = host::cursor();
                            if !host::over_other_app(x, y) {
                                let path = self.state.group(group).and_then(|g| {
                                    g.tab()
                                        .items
                                        .iter()
                                        .find(|i| i.id == item)
                                        .map(|i| i.path.clone())
                                });
                                let recycled = path.as_deref().is_some_and(|source| {
                                    self.try_recycle_drop(source, (x, y), input.time)
                                });
                                if !recycled {
                                    self.ungroup_to_desktop(group, item, (x, y), input.time);
                                }
                            }
                        }
                    }
                }
                Some(Drag::Create { start }) => {
                    if start.distance(input.pos) > 45. {
                        let b = Bounds {
                            x: start.x.min(input.pos.x),
                            y: start.y.min(input.pos.y),
                            w: (start.x - input.pos.x).abs().max(min_size(false).0),
                            h: (start.y - input.pos.y).abs().max(min_size(false).1),
                        };
                        self.create_group(b)
                    } else if input.right_released {
                        self.create_group(self.fresh_bounds());
                    }
                }
                _ => {}
            }
        }
    }
    /// Windows-backed actions on custom-drawn file icons. Unlike the classic
    /// Shell IContextMenu popup this menu works without an Explorer HWND owning
    /// the icon and explicitly routes each action to the Windows API.
    /// Ctrl+V or Box > Paste. Resolve intent from the current surface,
    /// never a stale fallback Box selection.
    pub(crate) fn paste_file_clipboard(&mut self, group: Option<u64>, time: f64) {
        if self.automation {
            self.notify(
                "File clipboard is only available in a Windows session",
                time,
            );
            return;
        }
        match shell::read_file_clipboard() {
            Ok(Some(files)) => self.paste_files(group, files, time),
            Ok(None) => self.notify("Clipboard has no files or folders", time),
            Err(e) => self.notify(e, time),
        }
    }
    /// Regular Box -> virtual pins; linked Folder View or bare Desktop ->
    /// actual Windows disk operation. A Cut must never silently only pin.
    pub(crate) fn paste_files(
        &mut self,
        group: Option<u64>,
        files: shell::FileClipboard,
        time: f64,
    ) {
        if files.paths.is_empty() {
            self.notify("Clipboard has no files", time);
            return;
        }
        let linked = group.and_then(|id| self.state.group(id).map(|g| g.tab().folder.clone()));
        match linked {
            Some(None) => {
                if files.cut {
                    self.notify("Cut files cannot move into a virtual Box. Copy them instead, or paste into a linked Folder View.",time);
                    return;
                }
                let id = group.unwrap();
                let mut staged = self.state.clone();
                match staged.pin_clipboard_paths(id, &files.paths) {
                    Ok(count) if count > 0 => {
                        self.remember();
                        self.state = staged;
                        self.selected = Some((id, 0));
                        self.multi_selected.clear();
                        if let Some(h) = self.host.as_mut() {
                            h.last_sync = -10.;
                        }
                        self.notify(
                            format!("Pinned {count} file(s) to this Box; files stay on disk"),
                            time,
                        );
                    }
                    Ok(_) => self.notify("Files are already pinned in this Box", time),
                    Err(e) => self.notify(e, time),
                }
            }
            Some(Some(folder)) => {
                if self.automation {
                    return;
                }
                match shell::paste_files_into_folder(&files, &PathBuf::from(&folder)) {
                    Ok(count) => {
                        if files.cut {
                            for path in &files.paths {
                                if !path.exists() {
                                    self.state.unassign(&path.to_string_lossy());
                                }
                            }
                        }
                        if let Some(g) = self.state.group_mut(group.unwrap()) {
                            if let Err(e) = g.tab_mut().refresh() {
                                self.notify(e, time);
                            }
                        }
                        if let Some(h) = self.host.as_mut() {
                            h.last_sync = -10.;
                        }
                        self.notify(
                            format!(
                                "{} {count} file(s) in linked folder",
                                if files.cut { "Moved" } else { "Copied" }
                            ),
                            time,
                        );
                    }
                    Err(e) => self.notify(e, time),
                }
            }
            None if group.is_some() => self.notify("Selected Box no longer exists", time),
            None => {
                if self.host.is_none() {
                    self.notify("Select a Box before pasting files", time);
                    return;
                }
                let folder = shell::desktop_folders().into_iter().next();
                if let Some(folder) = folder {
                    if self.automation {
                        return;
                    }
                    match shell::paste_files_into_folder(&files, &folder) {
                        Ok(count) => {
                            if files.cut {
                                for path in &files.paths {
                                    if !path.exists() {
                                        self.state.unassign(&path.to_string_lossy());
                                    }
                                }
                            }
                            if let Some(h) = self.host.as_mut() {
                                h.last_sync = -10.;
                            }
                            self.notify(
                                format!(
                                    "{} {count} file(s) onto Desktop",
                                    if files.cut { "Moved" } else { "Copied" }
                                ),
                                time,
                            );
                        }
                        Err(e) => self.notify(e, time),
                    }
                } else {
                    self.notify("Windows Desktop folder is unavailable", time);
                }
            }
        }
    }
    fn file_action(&mut self, path: &str, action: FileAction, time: f64) {
        // Shell-only operations on virtual namespace items are unsafe. A
        // non-filesystem icon is not a real disk file.
        if !Path::new(path).exists() {
            self.notify("This item is not a regular file or folder", time);
            return;
        }
        let box_target = self.selected.and_then(|(gid, iid)| {
            self.state.group(gid).and_then(|g| {
                g.tab()
                    .items
                    .iter()
                    .any(|it| it.id == iid && it.path.eq_ignore_ascii_case(path))
                    .then_some((gid, iid))
            })
        });
        match action {
            FileAction::PasteInto => {
                if !Path::new(path).is_dir() {
                    self.notify("Paste requires a real folder", time);
                    return;
                }
                if self.automation {
                    return;
                }
                match shell::read_file_clipboard() {
                    Ok(Some(files)) => {
                        match shell::paste_files_into_folder(&files, Path::new(path)) {
                            Ok(n) => {
                                if files.cut {
                                    for item in &files.paths {
                                        if !item.exists() {
                                            self.state.unassign(&item.to_string_lossy());
                                        }
                                    }
                                }
                                if let Some(h) = self.host.as_mut() {
                                    h.last_sync = -10.;
                                }
                                self.notify(format!("Pasted {n} file(s) into folder"), time);
                            }
                            Err(e) => self.notify(e, time),
                        }
                    }
                    Ok(None) => self.notify("Clipboard has no files", time),
                    Err(e) => self.notify(e, time),
                }
            }
            FileAction::Rename => {
                if let Some((gid, iid)) = box_target {
                    self.start_inline_rename(gid, iid);
                } else {
                    self.start_inline_rename_loose(path);
                }
            }
            FileAction::Open => self.open(path, time),
            FileAction::OpenWith => {
                if !self.automation {
                    if let Err(e) = shell::open_with(path) {
                        self.notify(e, time);
                    }
                }
            }
            FileAction::Copy | FileAction::Cut => {
                let paths: Vec<String> = box_target
                    .and_then(|(gid, _)| {
                        self.state.group(gid).map(|g| {
                            let tab_id = g.tab().id;
                            self.multi_selected
                                .iter()
                                .filter(|(g, t, _)| *g == gid && *t == tab_id)
                                .filter_map(|(_, _, iid)| {
                                    g.tab()
                                        .items
                                        .iter()
                                        .find(|i| i.id == *iid)
                                        .map(|i| i.path.clone())
                                })
                                .filter(|p| Path::new(p).exists())
                                .collect()
                        })
                    })
                    .filter(|items: &Vec<String>| items.len() > 1)
                    .or_else(|| {
                        self.host.as_ref().and_then(|h| {
                            let key = path.to_lowercase();
                            if !h.marquee_selected.contains(&key) {
                                return None;
                            }
                            let paths: Vec<String> = h
                                .loose
                                .iter()
                                .filter(|i| h.marquee_selected.contains(&i.key))
                                .map(|i| i.path.clone())
                                .filter(|p| Path::new(p).exists())
                                .collect();
                            (paths.len() > 1).then_some(paths)
                        })
                    })
                    .unwrap_or_else(|| vec![path.to_string()]);
                if !self.automation {
                    match shell::copy_files_to_clipboard(&paths, action == FileAction::Cut) {
                        Ok(()) => self.notify(
                            format!(
                                "{} {} file(s) to Windows clipboard",
                                if action == FileAction::Cut {
                                    "Cut"
                                } else {
                                    "Copied"
                                },
                                paths.len()
                            ),
                            time,
                        ),
                        Err(e) => self.notify(e, time),
                    }
                }
            }
            FileAction::Delete => {
                if !self.automation {
                    match shell::recycle(path) {
                        Ok(()) => {
                            if !Path::new(path).exists() {
                                self.state.unassign(path);
                                self.selected = None;
                                if let Some(h) = self.host.as_mut() {
                                    h.selected_loose = None;
                                    h.last_sync = -10.;
                                }
                            }
                        }
                        Err(e) => self.notify(e, time),
                    }
                }
            }
            FileAction::Properties => {
                self.properties_requested.push(path.to_string());
                if !self.automation {
                    if let Err(e) = shell::properties(path) {
                        self.notify(e, time);
                    }
                }
            }
            FileAction::Reveal => {
                if !self.automation {
                    if let Err(e) = shell::reveal_in_explorer(path) {
                        self.notify(e, time);
                    }
                }
            }
        }
    }

    /// One Windows 11-style file popup drawn by DesktopCraft. The supported
    /// commands still delegate to Windows Shell; unlike IContextMenu this
    /// popup never displays the old Windows 10-style menu by default.
    pub(super) fn modern_file_menu(&mut self, p: &mut Painter, input: &Input) {
        let Some(menu) = self.file_menu.clone() else {
            return;
        };
        let is_box_pin = self.selected.is_some_and(|(gid, iid)| {
            self.state.group(gid).is_some_and(|g| {
                g.tab().folder.is_none()
                    && g.tab()
                        .items
                        .iter()
                        .any(|i| i.id == iid && i.path.eq_ignore_ascii_case(&menu.path))
            })
        });
        let rect = self.menu_monitor_bounds(menu.pos);
        let height = if is_box_pin { 292. } else { 256. };
        let box_bounds = Bounds {
            x: menu
                .pos
                .x
                .clamp(rect.x + 6., (rect.x + rect.w - 298.).max(rect.x + 6.)),
            y: menu.pos.y.clamp(
                rect.y + 6.,
                (rect.y + rect.h - height - 6.).max(rect.y + 6.),
            ),
            w: 292.,
            h: height,
        };
        self.popup_rects.push(box_bounds);
        round(box_bounds, 10., SURFACE);
        p.push_region(box_bounds, 10.);
        p.enabled = true;
        p.text(
            "File actions",
            box_bounds.x + 12.,
            box_bounds.y + 9.,
            15.,
            TEXT,
        );
        let quick = [
            ("Cut", FileAction::Cut),
            ("Copy", FileAction::Copy),
            ("Rename", FileAction::Rename),
            ("Delete", FileAction::Delete),
        ];
        let mut chosen: Option<FileAction> = None;
        let mut more_options = false;
        let mut unpin = false;
        for (index, (label, action)) in quick.iter().enumerate() {
            let r = Bounds {
                x: box_bounds.x + 10. + index as f32 * 69.,
                y: box_bounds.y + 33.,
                w: 64.,
                h: 35.,
            };
            let hovered = r.contains(input.pos.x, input.pos.y);
            round(
                r,
                5.,
                if hovered {
                    Color::new(0.18, 0.27, 0.37, 1.)
                } else {
                    Color::new(0.14, 0.20, 0.27, 1.)
                },
            );
            p.label(label, r, 13., TEXT);
            if hovered && input.pressed {
                chosen = Some(*action);
            }
        }
        draw_line(
            box_bounds.x + 10.,
            box_bounds.y + 75.,
            box_bounds.x + box_bounds.w - 10.,
            box_bounds.y + 75.,
            1.,
            MUTED,
        );
        let entries = [
            ("Open", Some(FileAction::Open)),
            ("Open with...", Some(FileAction::OpenWith)),
            ("Show in File Explorer", Some(FileAction::Reveal)),
            ("Properties", Some(FileAction::Properties)),
            ("Show more options", None),
        ];
        for (index, (label, action)) in entries.iter().enumerate() {
            let r = Bounds {
                x: box_bounds.x + 8.,
                y: box_bounds.y + 83. + index as f32 * 32.,
                w: box_bounds.w - 16.,
                h: 30.,
            };
            let hovered = r.contains(input.pos.x, input.pos.y);
            if hovered {
                round(r, 5., Color::new(0.18, 0.27, 0.37, 1.));
            }
            p.text(label, r.x + 10., r.y + 7., 14., TEXT);
            if hovered && input.pressed {
                if let Some(action) = action {
                    chosen = Some(*action);
                } else {
                    more_options = true;
                }
            }
        }
        if is_box_pin {
            let row = Bounds {
                x: box_bounds.x + 8.,
                y: box_bounds.y + 246.,
                w: box_bounds.w - 16.,
                h: 32.,
            };
            if row.contains(input.pos.x, input.pos.y) {
                round(row, 5., Color::new(0.18, 0.27, 0.37, 1.));
                if input.pressed {
                    unpin = true;
                }
            }
            p.text(
                "Unpin from Box (keep file)",
                row.x + 10.,
                row.y + 8.,
                14.,
                TEXT,
            );
        }
        let outside = input.pressed && !box_bounds.contains(input.pos.x, input.pos.y);
        if chosen.is_some() || more_options || unpin || outside {
            self.file_menu = None;
        }
        if unpin {
            self.remember();
            self.state.unassign(&menu.path);
            self.selected = None;
            if let Some(h) = self.host.as_mut() {
                h.last_sync = -10.;
            }
            self.notify("Unpinned (original file unchanged)", input.time);
        } else if let Some(action) = chosen {
            self.file_action(&menu.path, action, input.time);
        } else if more_options {
            if let Some(h) = self.host.as_mut() {
                h.pending = Some(desktop::Pending::NativeFileMenu { path: menu.path });
            }
        }
    }

    pub(super) fn context_menu(&mut self, p: &mut Painter, input: &Input) {
        let Some((id, pos)) = self.menu else { return };
        let Some(g) = self.state.group(id) else {
            self.menu = None;
            return;
        };
        let rows = [
            "Rename group",
            "Add file…",
            "Sort by  ›",
            if g.tab().list {
                "Switch to icon view"
            } else {
                "Switch to list view"
            },
            "Change appearance…",
            if g.locked {
                "Unlock position"
            } else {
                "Lock position"
            },
            if g.keep_visible {
                "Include in quick-hide"
            } else {
                "Keep visible when hidden"
            },
            "Move to page  ›",
            "Separate active tab",
            if g.tab().folder.is_some() {
                "Copy this folder path"
            } else {
                "Copy selected file path"
            },
            "Unpin selected file (keep on disk)",
            "Remove group…",
            "DesktopCraft settings…",
            "Open linked folder",
            if g.tab().folder.is_some() {
                "Change linked folder path…"
            } else {
                "Bind folder path…"
            },
            "Add tab",
            "Paste files",
        ];
        let selected_pin = self.selected.is_some_and(|(gid, item)| {
            gid == id
                && item != 0
                && g.tab().folder.is_none()
                && g.tab().items.iter().any(|x| x.id == item)
        });
        // Only render actions that can actually be used by this box.
        // Stable action IDs are retained for the command dispatcher.
        let visible_rows: Vec<(usize, &str)> =
            visible_box_menu_indices(g.tabs.len(), g.tab().folder.is_some(), selected_pin)
                .into_iter()
                .map(|i| (i, rows[i]))
                .collect();
        // Clamp to the monitor containing the ellipsis, not the virtual
        // multi-monitor desktop; popup menus must not leak onto the next panel.
        let screen = self.menu_monitor_bounds(pos);
        let menu_h = visible_rows.len() as f32 * 33. + 12.;
        let b = Bounds {
            x: pos.x.clamp(
                screen.x + 5.,
                (screen.x + screen.w - 327.).max(screen.x + 5.),
            ),
            y: pos.y.clamp(
                screen.y + 5.,
                (screen.y + screen.h - menu_h - 5.).max(screen.y + 5.),
            ),
            w: 322.,
            h: menu_h,
        };
        self.popup_rects.push(b);
        round(
            Bounds {
                x: b.x + 4.,
                y: b.y + 7.,
                ..b
            },
            8.,
            alpha(BLACK, 0.22),
        );
        round(b, 7., SURFACE);
        p.push_region(b, 7.);
        p.enabled = true;
        let mut action = None;
        for (position, (i, label)) in visible_rows.iter().enumerate() {
            let i = *i;
            let row = Bounds {
                x: b.x + 5.,
                y: b.y + 6. + position as f32 * 33.,
                w: b.w - 10.,
                h: 32.,
            };
            if p.hover(row) {
                round(row, 4., Color::new(0.18, 0.26, 0.35, 1.));
                if i == 7 {
                    self.menu_page_open = true;
                    self.menu_sort_open = false;
                }
                if i == 2 {
                    self.menu_sort_open = true;
                    self.menu_page_open = false;
                }
                if input.pressed && i != 7 && i != 2 {
                    action = Some(i);
                }
            }
            p.text(
                &p.fit(label, 14., row.w - 24.),
                row.x + 11.,
                row.y + 6.,
                14.,
                if i == 11 { COLORS[4] } else { TEXT },
            );
        }
        // Same side-aware submenu pattern for sorting and moving between pages.
        let mut sort_bounds = None;
        let mut sort_choice = None;
        if self.menu_sort_open {
            let sb = Bounds {
                x: if b.x + b.w + 212. <= screen.x + screen.w {
                    b.x + b.w + 4.
                } else {
                    (b.x - 212.).max(screen.x + 5.)
                },
                y: (b.y
                    + visible_rows
                        .iter()
                        .position(|(original, _)| *original == 2)
                        .unwrap_or(3) as f32
                        * 33.)
                    .clamp(
                        screen.y + 5.,
                        (screen.y + screen.h - 4. * 33. - 16.).max(screen.y + 5.),
                    ),
                w: 208.,
                h: 4. * 33. + 10.,
            };
            sort_bounds = Some(sb);
            self.popup_rects.push(sb);
            round(sb, 7., SURFACE);
            p.push_region(sb, 7.);
            for (i, label) in ["Name (A–Z)", "File type", "Date (newest)", "Size (largest)"]
                .iter()
                .enumerate()
            {
                let row = Bounds {
                    x: sb.x + 5.,
                    y: sb.y + 5. + i as f32 * 33.,
                    w: sb.w - 10.,
                    h: 32.,
                };
                if p.hover(row) {
                    round(row, 4., Color::new(0.18, 0.26, 0.35, 1.));
                    if input.pressed {
                        sort_choice = Some(i as u8);
                    }
                }
                let current = self
                    .state
                    .group(id)
                    .is_some_and(|g| g.tab().sort == i as u8);
                // The old checkmark glyph was missing on some Windows fonts.
                // Draw a real vector tick instead of a Unicode fallback square.
                if current {
                    draw_line(
                        row.x + 9.,
                        row.y + 17.,
                        row.x + 14.,
                        row.y + 22.,
                        2.,
                        ACCENT,
                    );
                    draw_line(
                        row.x + 14.,
                        row.y + 22.,
                        row.x + 23.,
                        row.y + 10.,
                        2.,
                        ACCENT,
                    );
                }
                p.text(
                    label,
                    row.x + 32.,
                    row.y + 7.,
                    14.,
                    if current { ACCENT } else { TEXT },
                );
            }
        }
        // Move-to-page lists existing destinations only. Page creation is
        // intentionally restricted to the confirmed + action in the pager.
        let mut submenu_bounds = None;
        let mut page_choice = None;
        if self.menu_page_open {
            let total = self.state.pages.len();
            let visible = total.min(((self.height - 120.) / 33.).floor().max(3.) as usize);
            self.menu_page_scroll = self.menu_page_scroll.min(total.saturating_sub(visible));
            let sb = Bounds {
                x: if b.x + b.w + 208. <= screen.x + screen.w {
                    b.x + b.w + 4.
                } else {
                    (b.x - 208.).max(screen.x + 5.)
                },
                y: (b.y
                    + visible_rows
                        .iter()
                        .position(|(original, _)| *original == 7)
                        .unwrap_or(7) as f32
                        * 33.)
                    .clamp(
                        screen.y + 5.,
                        (screen.y + screen.h - visible as f32 * 33. - 14.).max(screen.y + 5.),
                    ),
                w: 204.,
                h: visible as f32 * 33. + 10.,
            };
            submenu_bounds = Some(sb);
            self.popup_rects.push(sb);
            if sb.contains(input.pos.x, input.pos.y) && input.scroll != 0. {
                if input.scroll < 0. {
                    self.menu_page_scroll =
                        (self.menu_page_scroll + 1).min(total.saturating_sub(visible));
                } else {
                    self.menu_page_scroll = self.menu_page_scroll.saturating_sub(1);
                }
            }
            round(sb, 7., SURFACE);
            p.push_region(sb, 7.);
            for offset in 0..visible {
                let i = self.menu_page_scroll + offset;
                let row = Bounds {
                    x: sb.x + 5.,
                    y: sb.y + 5. + offset as f32 * 33.,
                    w: sb.w - 10.,
                    h: 32.,
                };
                if p.hover(row) {
                    round(row, 4., Color::new(0.18, 0.26, 0.35, 1.));
                    if input.pressed {
                        page_choice = Some(i);
                    }
                }
                let label = format!("{}  {}", i + 1, self.state.pages[i]);
                p.text(
                    &p.fit(&label, 14., row.w - 18.),
                    row.x + 9.,
                    row.y + 7.,
                    14.,
                    if i == self.state.page { ACCENT } else { TEXT },
                );
            }
        }
        if input.pressed
            && !b.contains(input.pos.x, input.pos.y)
            && !submenu_bounds.is_some_and(|sb| sb.contains(input.pos.x, input.pos.y))
            && !sort_bounds.is_some_and(|sb| sb.contains(input.pos.x, input.pos.y))
        {
            self.menu = None;
            self.menu_page_open = false;
            return;
        }
        if let Some(sort) = sort_choice {
            self.remember();
            if let Some(g) = self.state.group_mut(id) {
                g.tab_mut().sort = sort;
                g.tab_mut().sort_items();
                g.tab_mut().scroll = 0;
            }
            self.menu = None;
            self.menu_sort_open = false;
            self.notify(
                [
                    "Sorted by name",
                    "Sorted by type",
                    "Sorted by newest date",
                    "Sorted by largest size",
                ][sort as usize],
                input.time,
            );
            return;
        }
        if let Some(page) = page_choice {
            self.remember();
            if page >= self.state.pages.len() {
                return;
            }
            if let Some(g) = self.state.group_mut(id) {
                g.page = page;
            }
            if let Some(h) = self.host.as_mut() {
                h.last_sync = -10.;
            }
            self.menu = None;
            self.menu_page_open = false;
            self.selected = None;
            self.notify("Group moved to selected page", input.time);
            return;
        }
        if let Some(action) = action {
            self.remember();
            self.menu = None;
            match action {
                16 => self.paste_file_clipboard(Some(id), input.time),
                15 => match self.state.add_tab(id) {
                    Ok(_) => {
                        self.selected = Some((id, 0));
                        self.multi_selected.clear();
                        if let Some(h) = self.host.as_mut() {
                            h.last_sync = -10.;
                        }
                        self.notify("Tab added to this Box", input.time);
                    }
                    Err(e) => {
                        self.undo.pop();
                        self.notify(e, input.time);
                    }
                },
                0 => {
                    self.dialog = Some(Dialog::Rename {
                        group: id,
                        value: self.state.group(id).unwrap().tab().name.clone(),
                    });
                    self.select_all = true;
                    self.edit_focus = 0;
                }
                1 => {
                    self.selected = Some((id, 0));
                    self.pick_file(input.time);
                }
                2 => {
                    let tab = self.state.group_mut(id).unwrap().tab_mut();
                    tab.sort = (tab.sort + 1) % 4;
                    tab.sort_items();
                    self.notify(
                        [
                            "Sorted by name",
                            "Sorted by type",
                            "Sorted by newest",
                            "Sorted by largest",
                        ][self.state.group(id).unwrap().tab().sort as usize],
                        input.time,
                    );
                }
                3 => {
                    let tab = self.state.group_mut(id).unwrap().tab_mut();
                    tab.list = !tab.list;
                    tab.scroll = 0;
                }
                4 => {
                    // Opens Appearance with this group already chosen.
                    let index = self
                        .state
                        .groups
                        .iter()
                        .position(|g| g.id == id)
                        .map_or(0, |i| i + 1);
                    self.dialog = Some(Dialog::Settings {
                        page: 0,
                        pattern: String::new(),
                        mode: 0,
                        target: index,
                    });
                }
                5 => {
                    let g = self.state.group_mut(id).unwrap();
                    g.locked = !g.locked;
                }
                6 => {
                    let g = self.state.group_mut(id).unwrap();
                    g.keep_visible = !g.keep_visible;
                }
                7 => {
                    if self.state.page + 1 >= self.state.pages.len() {
                        self.state
                            .pages
                            .push(format!("Space {}", self.state.pages.len() + 1));
                    }
                    let next = self.state.page + 1;
                    self.state.group_mut(id).unwrap().page = next;
                    self.notify("Group moved to the next page", input.time);
                }
                8 => {
                    let g = self.state.group(id).unwrap();
                    let b = g.bounds;
                    let active = g.active;
                    if self
                        .state
                        .detach(
                            id,
                            active,
                            Bounds {
                                x: b.x + 40.,
                                y: b.y + 60.,
                                ..b
                            },
                        )
                        .is_none()
                    {
                        self.notify(
                            "Combine two groups first by dragging one header onto another.",
                            input.time,
                        );
                    }
                }
                9 => {
                    let path = self
                        .state
                        .group(id)
                        .unwrap()
                        .tab()
                        .folder
                        .clone()
                        .or_else(|| {
                            self.selected.and_then(|(_, item)| {
                                self.state
                                    .group(id)
                                    .unwrap()
                                    .tab()
                                    .items
                                    .iter()
                                    .find(|i| i.id == item)
                                    .map(|i| i.path.clone())
                            })
                        });
                    if let Some(path) = path {
                        macroquad::miniquad::window::clipboard_set(&path);
                        self.notify("Path copied", input.time)
                    }
                }
                10 => {
                    if let Some((_, item)) = self.selected {
                        let tab = self.state.group_mut(id).unwrap().tab_mut();
                        if tab.folder.is_none() {
                            tab.items.retain(|i| i.id != item);
                            self.selected = None;
                            self.notify(
                                "Pin removed from this group; the file was not deleted",
                                input.time,
                            );
                        } else {
                            self.notify("Folder view items reflect the folder on disk.", input.time)
                        }
                    }
                }
                11 => self.dialog = Some(Dialog::Remove { group: id }),
                12 => {
                    self.dialog = Some(Dialog::Settings {
                        page: 1,
                        pattern: String::new(),
                        mode: 0,
                        target: 0,
                    })
                }
                13 => {
                    let location = self.state.group(id).and_then(|g| g.tab().folder.clone());
                    if let Some(path) = location {
                        if let Err(e) = platform::open(&path) {
                            self.notify(e, input.time);
                        }
                    }
                }
                14 => {
                    self.dialog = Some(Dialog::BindFolder {
                        group: id,
                        value: self
                            .state
                            .group(id)
                            .and_then(|g| g.tab().folder.clone())
                            .unwrap_or_default(),
                    })
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod windows_file_ux_tests {
    use super::*;
    fn fixture() -> (App, String, u64, u64) {
        let root = std::env::temp_dir().join(format!(
            "desktopcraft-file-menu-test-{}",
            std::process::id()
        ));
        let path = root.join("Draft.zip");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&path, b"fixture").unwrap();
        let mut app = App::new(root, true, true);
        app.state = State::empty();
        let group = app.state.add_group(
            "Archive",
            Bounds {
                x: 20.,
                y: 20.,
                w: 270.,
                h: 260.,
            },
        );
        let iid = 809;
        let path_string = path.to_string_lossy().to_string();
        app.state
            .group_mut(group)
            .unwrap()
            .tab_mut()
            .items
            .push(Item {
                id: iid,
                name: "Draft.zip".into(),
                path: path_string.clone(),
                kind: "zip".into(),
                size: 7,
                modified: 0,
                label: None,
            });
        let h = app.host.as_mut().unwrap();
        h.loose.push(desktop::Loose {
            path: path_string.clone(),
            key: path_string.to_lowercase(),
            label: "Draft.zip".into(),
            pos: (100, 120),
        });
        (app, path_string, group, iid)
    }
    #[test]
    fn f2_loose_file_wins_over_selected_box_and_escape_does_not_rename() {
        let (mut app, path, group, _) = fixture();
        app.selected = Some((group, 0));
        app.host.as_mut().unwrap().selected_loose = Some(path.clone());
        app.shortcuts(&Input {
            keys: vec![KeyCode::F2],
            time: 1.,
            ..Default::default()
        });
        assert!(app.dialog.is_none(), "F2 must not rename a Box");
        assert!(matches!(&app.inline_rename,Some((0,0,p,_)) if p==&path));
        assert_eq!(
            app.inline_rename.as_ref().unwrap().3.selection,
            NameSelection::Stem
        );
        app.shortcuts(&Input {
            typed: "Budget".into(),
            time: 2.,
            ..Default::default()
        });
        assert_eq!(app.inline_rename.as_ref().unwrap().3.text, "Budget.zip");
        app.shortcuts(&Input {
            keys: vec![KeyCode::Escape],
            time: 3.,
            ..Default::default()
        });
        assert!(app.inline_rename.is_none());
        assert!(Path::new(&path).exists(), "Esc must not alter disk files");
    }
    #[test]
    fn keyboard_context_menu_and_enter_work_on_loose_desktop_icon() {
        let (mut app, path, group, _) = fixture();
        app.selected = Some((group, 0));
        app.host.as_mut().unwrap().selected_loose = Some(path.clone());
        app.shortcuts(&Input {
            keys: vec![KeyCode::F10],
            shift: true,
            pos: vec2(200., 300.),
            time: 1.,
            ..Default::default()
        });
        assert!(matches!(app.host.as_ref().unwrap().pending.as_ref(),
            Some(desktop::Pending::NativeFileMenu{path:p}) if p==&path));
        // The host event loop will invoke Windows' native menu. Do not pop a
        // real Shell context menu during automated unit tests.
        app.host.as_mut().unwrap().pending = None;
        app.shortcuts(&Input {
            keys: vec![KeyCode::Enter],
            time: 3.,
            ..Default::default()
        });
        assert_eq!(app.opened.last(), Some(&path));
        assert!(app.dialog.is_none());
    }
    #[test]
    fn alt_enter_requests_windows_properties_for_pin_and_loose_icon() {
        let (mut app, path, group, item) = fixture();
        app.selected = Some((group, item));
        app.shortcuts(&Input {
            keys: vec![KeyCode::Enter],
            alt: true,
            time: 1.,
            ..Default::default()
        });
        assert_eq!(app.properties_requested.last(), Some(&path));
        assert!(app.opened.is_empty(), "Alt+Enter must not open the file");
        app.selected = None;
        app.properties_requested.clear();
        app.host.as_mut().unwrap().selected_loose = Some(path.clone());
        app.shortcuts(&Input {
            keys: vec![KeyCode::Enter],
            alt: true,
            time: 2.,
            ..Default::default()
        });
        assert_eq!(app.properties_requested.last(), Some(&path));
        assert!(app.opened.is_empty());
        assert!(app.dialog.is_none());
    }
    #[test]
    fn paste_into_virtual_box_is_not_a_disk_move_and_cut_is_rejected() {
        let (mut app, path, group, _) = fixture();
        let next = Path::new(&path).with_file_name("another.txt");
        std::fs::write(&next, b"safe").unwrap();
        let clip = shell::FileClipboard {
            paths: vec![next.clone()],
            cut: false,
        };
        app.paste_files(Some(group), clip.clone(), 1.);
        assert_eq!(app.state.group(group).unwrap().tab().items.len(), 2);
        assert!(next.exists(), "Pinning must not move the disk file");
        app.paste_files(
            Some(group),
            shell::FileClipboard {
                paths: vec![next.clone(), PathBuf::from("C:/does/not/exist")],
                cut: false,
            },
            2.,
        );
        assert_eq!(app.state.group(group).unwrap().tab().items.len(), 2);
        let two = Path::new(&path).with_file_name("not-moved.txt");
        std::fs::write(&two, b"still here").unwrap();
        app.paste_files(
            Some(group),
            shell::FileClipboard {
                paths: vec![two.clone()],
                cut: true,
            },
            3.,
        );
        assert_eq!(app.state.group(group).unwrap().tab().items.len(), 2);
        assert!(
            two.exists(),
            "Cut into virtual box must never move a real file"
        );
        assert!(app.toast.contains("Cut files cannot move"));
        assert!(
            app.undo.len() <= 1,
            "Errors should not append undo snapshots"
        );
    }
    #[test]
    fn right_click_box_pin_opens_desktopcraft_modern_menu() {
        let (mut app, path, group, item) = fixture();
        let hits = [Hit {
            target: Target::Item(group, item),
            bounds: Bounds {
                x: 100.,
                y: 100.,
                w: 80.,
                h: 70.,
            },
        }];
        app.pointer(
            &Input {
                pos: vec2(130., 130.),
                right_pressed: true,
                time: 1.,
                ..Default::default()
            },
            &hits,
        );
        assert!(matches!(app.file_menu.as_ref(),Some(m) if m.path==path));
        assert!(
            app.host.as_ref().unwrap().pending.is_none(),
            "Modern popup should not trigger the classic Shell menu"
        );
        assert!(app.dialog.is_none());
        assert_eq!(app.selected, Some((group, item)));
    }
    #[test]
    fn file_menu_routes_rename_to_correct_box_pin_or_loose_icon() {
        let (mut app, path, group, item) = fixture();
        app.selected = Some((group, item));
        app.file_action(&path, FileAction::Rename, 1.);
        assert!(matches!(&app.inline_rename,Some((g,i,p,_)) if *g==group&&*i==item&&p==&path));
        assert!(app.dialog.is_none());
        app.inline_rename = None;
        app.selected = None;
        app.host.as_mut().unwrap().selected_loose = Some(path.clone());
        app.file_action(&path, FileAction::Rename, 2.);
        assert!(matches!(&app.inline_rename,Some((0,0,p,_)) if p==&path));
        app.inline_rename = None;
        app.file_action(&path, FileAction::Open, 3.);
        assert_eq!(app.opened.last().unwrap(), &path);
    }
}
