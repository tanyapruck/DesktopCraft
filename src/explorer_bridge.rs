//! Explorer-first architecture laboratory (no injection or layout changes).
//! The existing Shell Desktop View can own real Desktop icon selection; it
//! cannot own arbitrary pinned non-Desktop files, independent Tab contents,
//! or multi-page visibility without a separate, carefully designed adapter.
use crate::{model::State, shell::ViewItem};
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub boxes: usize,
    pub pages: usize,
    pub regular_tabs: usize,
    pub linked_folder_tabs: usize,
    pub pinned_desktop_files: usize,
    pub pinned_non_desktop_files: usize,
    pub inactive_tab_pins: usize,
    pub inactive_page_pins: usize,
    pub desktop_icons_not_pinned: usize,
}

impl Coverage {
    pub fn to_json(&self) -> Value {
        json!({
            "boxes":self.boxes,
            "pages":self.pages,
            "regular_tabs":self.regular_tabs,
            "linked_folder_tabs":self.linked_folder_tabs,
            "pins_corresponding_to_explorer_desktop_icons":self.pinned_desktop_files,
            "pins_outside_explorer_desktop_view":self.pinned_non_desktop_files,
            "pins_in_inactive_tabs":self.inactive_tab_pins,
            "pins_on_other_desktopcraft_pages":self.inactive_page_pins,
            "explorer_desktop_icons_not_pinned":self.desktop_icons_not_pinned,
            "no_changes_to_icon_positions_or_files":true,
            "design_implication":"Explorer Desktop View cannot directly host non-Desktop pins or independently hide Box tabs/pages; separate Shell folder views, visual coordination and undoable layout restoration need a guarded prototype"
        })
    }
}

/// Pure, read-only analysis, safe to run against user workspace data. No paths
/// or file names are printed in the report; only counts are surfaced.
pub fn analyze(state: &State, explorer_items: &[ViewItem]) -> Coverage {
    let explorer: HashSet<String> = explorer_items
        .iter()
        .map(|i| i.path.to_lowercase())
        .collect();
    let mut pinned = HashSet::new();
    let mut report = Coverage {
        boxes: state.groups.len(),
        pages: state.pages.len(),
        ..Coverage::default()
    };
    for g in &state.groups {
        for (tab_index, tab) in g.tabs.iter().enumerate() {
            if tab.folder.is_some() {
                report.linked_folder_tabs += 1;
                continue;
            }
            report.regular_tabs += 1;
            for item in &tab.items {
                let key = item.path.to_lowercase();
                pinned.insert(key.clone());
                if explorer.contains(&key) {
                    report.pinned_desktop_files += 1;
                } else {
                    report.pinned_non_desktop_files += 1;
                }
                if tab_index != g.active {
                    report.inactive_tab_pins += 1;
                }
                if g.page != state.page {
                    report.inactive_page_pins += 1;
                }
            }
        }
    }
    report.desktop_icons_not_pinned = explorer.difference(&pinned).count();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Bounds, Item};
    #[test]
    fn audit_separates_explorer_icons_and_virtual_tab_page_needs() {
        let mut state = State::empty();
        let g = state.add_group(
            "Games",
            Bounds {
                x: 10.,
                y: 10.,
                w: 250.,
                h: 200.,
            },
        );
        state.group_mut(g).unwrap().tab_mut().items.push(Item {
            id: 10,
            name: "game.lnk".into(),
            path: "C:/Desktop/game.lnk".into(),
            kind: "lnk".into(),
            size: 0,
            modified: 0,
            label: None,
        });
        state.add_tab(g).unwrap();
        state.group_mut(g).unwrap().tab_mut().items.push(Item {
            id: 11,
            name: "external.exe".into(),
            path: "D:/Apps/external.exe".into(),
            kind: "exe".into(),
            size: 0,
            modified: 0,
            label: None,
        });
        let desktop = vec![
            ViewItem {
                path: "c:/desktop/GAME.LNK".into(),
                label: "game".into(),
                pos: (0, 0),
            },
            ViewItem {
                path: "C:/Desktop/misc.txt".into(),
                label: "misc".into(),
                pos: (100, 100),
            },
        ];
        let result = analyze(&state, &desktop);
        assert_eq!(result.pinned_desktop_files, 1);
        assert_eq!(result.pinned_non_desktop_files, 1);
        assert_eq!(result.desktop_icons_not_pinned, 1);
        assert_eq!(result.inactive_tab_pins, 1);
        assert_eq!(result.boxes, 1);
        assert_eq!(result.regular_tabs, 2);
        assert_eq!(result.pages, 1);
    }
}
