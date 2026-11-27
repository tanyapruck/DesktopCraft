//! Transactional developer-only real Explorer IFolderView Native Box layout.
//! No folder moves or replicated icons. Persisted user Workspace untouched.
//! A recovery journal is created BEFORE any real Explorer icon coordinate
//! mutation and is only removed after every original coordinate is restored.
mod journal_archive;
use journal_archive::*;
mod move_planning;
use move_planning::*;
mod inventory;
use inventory::*;
mod recovery;
use recovery::*;
mod preflight;
use preflight::*;
mod grid;
use grid::*;
mod layout_planning;
use layout_planning::*;
mod transaction_init;
mod transaction_inventory;
mod transaction_checkpoint;
mod transaction_reflow;
mod transaction_drag;
mod transaction_restore;
pub use journal_archive::archive_stale_recovery_journal;
pub use journal_archive::recover_and_archive_stale_journal;
pub(crate) use move_planning::split_displaced_hidden_park;
pub(crate) use inventory::InventoryReconcileOutcome;
pub(crate) use inventory::reconcile_desktop_inventory;
pub(crate) use inventory::adopt_spacing_positions;
pub use recovery::recovery_status_or_restore;
pub use recovery::automatic_recovery_status_or_restore;
pub(crate) use recovery::automatic_recovery_after_monitor_extension;
pub use preflight::native_desktop_preflight;
pub(crate) use grid::DesktopCoordinates;
pub(crate) use grid::cell_for_point;
pub(crate) use layout_planning::offscreen_icon_count;
pub(crate) use layout_planning::current_screens;

use crate::{host,model::{Bounds, State},native_explorer_guard::{desktop_owner,validate_layout_snapshot},native_live_membership::NativeTwoBoxMembership,
    native_box_placement::{self,Rect,NativePagePlan,PlacementSnapshot},shell::{DesktopView,ViewItem}};
use std::{collections::{BTreeMap,BTreeSet},path::PathBuf,time::Duration};
use serde_json::{json,Value};

#[derive(Debug)]
pub struct LiveLayoutReport {
    pub hidden_indices:Vec<usize>,
    pub expected_native_order:Vec<ViewItem>,
    /// (Box id, first visible row, scroll positions, visible rows).
    pub page_status:Vec<(u64,usize,usize,usize)>,
    pub visible_paths:Vec<String>,
    pub observed_after:Vec<ViewItem>,
    /// Milliseconds per reflow step, for the diagnostic log.
    pub timing:String,
    /// True when the in-process ListView batch skipped redundant pre/post COM polls.
    pub fast_batch_applied:bool,
}

/// Records how long each step of a reflow took (diagnostic log only).
struct StepTimer{last:std::time::Instant,steps:Vec<(&'static str,u128)>}
impl StepTimer{
    fn new()->Self{Self{last:std::time::Instant::now(),steps:Vec::new()}}
    fn mark(&mut self,step:&'static str){
        let now=std::time::Instant::now();
        self.steps.push((step,(now-self.last).as_millis()));
        self.last=now;
    }
    fn summary(&self)->String{
        self.steps.iter().map(|(step,ms)|format!("{step}={ms}")).collect::<Vec<_>>().join(" ")
    }
}

pub struct LiveLayoutTransaction {
    original:Vec<ViewItem>,
    expected:Vec<ViewItem>,
    moved:Vec<(String,(i32,i32))>,
    /// Loose Desktop icons temporarily nudged onto nearby free grid cells while
    /// an active Box or Folder Portal covers their original Desktop spot.
    nudged_loose:Vec<(String,(i32,i32))>,
    pages:BTreeMap<u64,usize>,
    journal:PathBuf,
    initial_monitor_signature:String,
    snap_to_grid: bool,
    /// User's Explorer Align-to-Grid setting before Native Desktop started.
    /// Restored on clean exit and on manual/crash recovery.
    original_snap_to_grid: bool,
    /// Observed primary-monitor Desktop grid origin (before disabling snap)
    /// used to software-snap loose Desktop icons back onto the Desktop grid.
    desktop_grid_origin: Option<(i32, i32)>,
    /// Current on-screen parking point where hidden Box icons are stacked.
    hidden_park: Option<(i32, i32)>,
    /// Case-folded paths currently parked together at `hidden_park`.
    parked_paths: BTreeSet<String>,
    pending_scaled_recheck_at: Option<std::time::Instant>,
    scaled_targets: BTreeMap<String, (i32, i32)>,
    /// True after a prior `try_native_batch` generation settled cleanly under Explorer's Shell view.
    native_batch_shell_verified: bool,
    /// Pre-drop `(path, pos)` for physically dragged icons kept in place until reflow commits.
    pending_drop_pre_positions: Vec<(String, (i32, i32))>,
    count_changes:usize,
    restored:bool,
}

type Move=(String,(i32,i32));
pub(crate) type NativePlacementPlan = (Vec<NativePagePlan>, Vec<usize>, Option<(i32,i32)>, Vec<Move>);

impl Drop for LiveLayoutTransaction {
    fn drop(&mut self){
        if !self.restored {
            if let Some(view)=DesktopView::acquire(){
                let _=self.restore(&view);
            }
        }
    }
}

#[cfg(test)]
mod nudge_and_recovery_tests {
    use super::*;

    #[test]
    fn live_cell_repair_keeps_the_acknowledged_assignment_and_skips_other_moves() {
        let icon = |path: &str, pos| ViewItem { path: path.into(), label: path.into(), pos };
        let expected = [icon("Box", (150, 150)), icon("Parked", (300, 300)), icon("Loose", (400, 400))];
        let placed = vec![("Box".into(), (150, 150)), ("Parked".into(), (300, 300))];
        let parked = BTreeSet::from(["parked".into()]);
        let active = [Rect { x: 100, y: 100, w: 150, h: 150 }];
        let mut current = [icon("Box", (155, 158)), icon("Parked", (305, 308)), icon("Loose", (405, 408))];
        assert_eq!(nearby_box_cell_repairs(&expected, &current, &placed, &parked, &active), ["Box"]);
        current[0].pos = (159, 158);
        assert!(nearby_box_cell_repairs(&expected, &current, &placed, &parked, &active).is_empty());
        current[0].pos = (155, 158);
        current[2].pos = (150, 150);
        assert!(nearby_box_cell_repairs(&expected, &current, &placed, &parked, &active).is_empty());
    }
    use crate::model::Item;

    fn pin(id: u64, path: &str, name: &str) -> Item {
        Item { id, path: path.into(), name: name.into(), kind: "lnk".into(), size: 16, modified: 1, label: None }
    }

    fn mon(key: &str, x: i32, y: i32, dpi: u32, primary: bool) -> host::Monitor {
        host::Monitor { key: key.into(), rect: [x, y, 2560, 1440], work: [x, y, 2560, 1380], dpi, primary }
    }

    #[test]
    fn active_box_and_portal_nudge_covered_loose_icons_on_main_and_scaled_monitors() {
        let mut state = State::empty();
        let box_id = state.add_group("Apps", Bounds { x: 80., y: 100., w: 300., h: 260. });
        state.group_mut(box_id).unwrap().tab_mut().items.push(pin(101, "C:/d/InBox.lnk", "InBox"));
        let keep_box = state.add_group("Keep", Bounds { x: 1800., y: 900., w: 300., h: 260. });
        state.group_mut(keep_box).unwrap().tab_mut().items.push(pin(102, "C:/d/KeepPin.lnk", "KeepPin"));
        let portal_id = state.add_group("Downloads", Bounds { x: 2660., y: 120., w: 320., h: 280. });
        let portal_tab = state.group_mut(portal_id).unwrap().tab_mut();
        portal_tab.folder = Some("C:/Users/test/Downloads".into());

        let icons = [
            ViewItem { path: "C:/d/InBox.lnk".into(), label: "InBox".into(), pos: (11, 2) },
            ViewItem { path: "C:/d/KeepPin.lnk".into(), label: "KeepPin".into(), pos: (1811, 902) },
            ViewItem { path: "C:/d/Loose1.lnk".into(), label: "Loose1".into(), pos: (86, 109) },
            ViewItem { path: "C:/d/Loose2.lnk".into(), label: "Loose2".into(), pos: (161, 109) },
            ViewItem { path: "C:/d/SideLoose.lnk".into(), label: "SideLoose".into(), pos: (2668, 135) },
            ViewItem { path: "C:/d/Clear.lnk".into(), label: "Clear".into(), pos: (11, 400) },
        ];
        let member = NativeTwoBoxMembership::prepare_workspace(1, &state, &icons).unwrap();
        let monitors = [mon("main", 0, 0, 96, true), mon("side", 2560, -1, 120, false)];
        let screens = [Rect { x: 0, y: 0, w: 2560, h: 1440 }, Rect { x: 2560, y: -1, w: 2560, h: 1440 }];
        let coordinates = DesktopCoordinates { origin: (0, 0), scale: 1.0 };

        let (plans, _, _, nudged) = plan_layout(
            &member, &state, &icons, &icons, (75, 107), &screens, &monitors, coordinates, false, Some((11, 2)), |_| 0,
        ).unwrap();
        assert_eq!(plans[0].moves[0].to, (102, 112));
        assert_eq!(nudged.len(), 3);
        let box_rect = coordinates.screen_rect(state.group(box_id).unwrap().bounds);
        let portal_rect = coordinates.screen_rect(state.group(portal_id).unwrap().bounds);
        for (path, dest) in &nudged {
            let cell = if path.contains("SideLoose") { (94, 134) } else { (75, 107) };
            assert!(!native_box_placement::cell_overlaps_rect(*dest, cell, box_rect), "{path} overlapped box at {dest:?}");
            assert!(!native_box_placement::cell_overlaps_rect(*dest, cell, portal_rect), "{path} overlapped portal at {dest:?}");
            assert!(!native_box_placement::cells_overlap(*dest, (11, 400), cell), "{path} overlapped Clear at {dest:?}");
        }
        // Moving Box onto page 2 or rolling up un-covers Loose1/Loose2 so they return.
        let mut rolled = state.clone();
        rolled.group_mut(box_id).unwrap().rolled = true;
        let rolled_member = NativeTwoBoxMembership::prepare_workspace(2, &rolled, &icons).unwrap();
        let (_, _, _, rolled_nudged) = plan_layout(
            &rolled_member, &rolled, &icons, &icons, (75, 107), &screens, &monitors, coordinates, false, Some((11, 2)), |_| 0,
        ).unwrap();
        assert!(!rolled_nudged.iter().any(|(p, _)| p.contains("Loose1") || p.contains("Loose2")));
        assert!(rolled_nudged.iter().any(|(p, _)| p.contains("SideLoose")));

        let batches = plan_free_moves(
            &[], &nudged, &[("C:/d/InBox.lnk".into(), (102, 112))], &[], &[],
            &[
                ViewItem { path: "C:/d/InBox.lnk".into(), label: "InBox".into(), pos: (11, 2) },
                ViewItem { path: "C:/d/KeepPin.lnk".into(), label: "KeepPin".into(), pos: (1811, 902) },
                ViewItem { path: "C:/d/Loose1.lnk".into(), label: "Loose1".into(), pos: nudged[0].1 },
                ViewItem { path: "C:/d/Loose2.lnk".into(), label: "Loose2".into(), pos: nudged[1].1 },
                ViewItem { path: "C:/d/SideLoose.lnk".into(), label: "SideLoose".into(), pos: nudged[2].1 },
                ViewItem { path: "C:/d/Clear.lnk".into(), label: "Clear".into(), pos: (11, 400) },
            ],
            |p| icons.iter().find(|i| i.path == p).map(|i| i.pos),
        ).unwrap();
        assert!(batches[0].contains(&("C:/d/Loose1.lnk".into(), (86, 109))));
        assert!(batches[0].contains(&("C:/d/Loose2.lnk".into(), (161, 109))));
        assert!(batches[0].contains(&("C:/d/SideLoose.lnk".into(), (2668, 135))));
    }

    #[test]
    fn recovery_accepts_partial_restore_and_user_nudged_loose_drag_while_refusing_box_move() {
        let journal = json!({
            "auto_restore_requires_exact_ack": true,
            "last_ack_generation": 2,
            "original_icons": [
                {"path": "BoxA", "label": "BoxA", "x": 11, "y": 2},
                {"path": "BoxB", "label": "BoxB", "x": 86, "y": 2},
                {"path": "NudgedL", "label": "NudgedL", "x": 161, "y": 109},
                {"path": "UserMovedNudged", "label": "UserMovedNudged", "x": 236, "y": 109}
            ],
            "last_verified_positions": [
                {"path": "BoxA", "label": "BoxA", "x": 400, "y": 400},
                {"path": "BoxB", "label": "BoxB", "x": 475, "y": 400},
                {"path": "NudgedL", "label": "NudgedL", "x": 11, "y": 216},
                {"path": "UserMovedNudged", "label": "UserMovedNudged", "x": 86, "y": 216}
            ],
            "nudged_loose_positions": [
                {"path": "NudgedL", "x": 11, "y": 216},
                {"path": "UserMovedNudged", "x": 86, "y": 216}
            ]
        });
        let icon = |path: &str, pos| ViewItem { path: path.into(), label: path.into(), pos };
        // Partial restoration crash: BoxA returned to (11,2), BoxB stays at ACK (475,400),
        // NudgedL stays at exact ACK nudge (11,216), UserMovedNudged was dragged to (600,600).
        let sets = verified_recovery_plan(&journal, &[
            icon("BoxA", (11, 2)),
            icon("BoxB", (475, 400)),
            icon("NudgedL", (11, 216)),
            icon("UserMovedNudged", (600, 600)),
        ]).unwrap();
        assert_eq!(sets.placed, BTreeSet::from(["boxa".into(), "boxb".into()]));
        assert_eq!(sets.nudged_restore, BTreeSet::from(["nudgedl".into()]));
        assert!(!sets.nudged_restore.contains("usermovednudged"));

        // If BoxA was manually dragged inside another Box at (430, 400), refuse auto-overwrite.
        assert!(verified_recovery_plan(&journal, &[
            icon("BoxA", (430, 400)),
            icon("BoxB", (475, 400)),
            icon("NudgedL", (11, 216)),
            icon("UserMovedNudged", (600, 600)),
        ]).is_err());
    }

    #[test]
    fn order_rollback_batches_and_early_settle_work_deterministically() {
        let icon = |path: &str, pos| ViewItem { path: path.into(), label: path.into(), pos };
        let current = [icon("a", (100, 100)), icon("b", (200, 200))];
        // `a` returns to (200, 200) where `b` sits; `b` returns to (300, 300). `b` moves first.
        let batches = order_rollback_batches(
            &current,
            &[("a".into(), (200, 200)), ("b".into(), (300, 300))],
        );
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0], vec![("b".into(), (300, 300))]);
        assert_eq!(batches[1], vec![("a".into(), (200, 200))]);

        let mut checks = 0;
        let out = wait_until_settled(false, &[1, 500, 500], || {
            checks += 1;
            (checks == 1, current.to_vec())
        });
        assert_eq!(checks, 1);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn immediate_settle_read_retries_when_positions_have_not_landed() {
        let mut checks = 0;
        let out = wait_until_settled(false, &[0, 1], || {
            checks += 1;
            let pos = if checks == 1 { (10, 20) } else { (40, 50) };
            (pos == (40, 50), vec![ViewItem { path: "a".into(), label: "A".into(), pos }])
        });
        assert_eq!(checks, 2);
        assert_eq!(out[0].pos, (40, 50));
    }

    #[test]
    fn reconcile_inventory_adopts_added_and_deleted_icons_while_guarding_placed_icons() {
        let icon = |path: &str, pos| ViewItem { path: path.into(), label: path.into(), pos };
        let mut original = vec![
            icon("BoxPin", (11, 2)),
            icon("Loose1", (86, 2)),
            icon("Loose2", (161, 2)),
            icon("Loose3", (236, 2)),
        ];
        let mut expected = vec![
            icon("BoxPin", (500, 500)),
            icon("Loose1", (86, 2)),
            icon("Loose2", (161, 2)),
            icon("Loose3", (236, 2)),
        ];
        let mut placed = vec![("BoxPin".into(), (500, 500))];
        let mut nudged = vec![("Loose1".into(), (86, 2))];

        // Add two new files and delete Loose2 while moving Loose1 manually.
        let live = vec![
            icon("BoxPin", (500, 500)),
            icon("Loose1", (300, 300)),
            icon("Loose3", (236, 2)),
            icon("NewA", (550, 520)),
            icon("NewB", (625, 520)),
        ];
        let out = reconcile_desktop_inventory(
            &mut original, &mut expected, &mut placed, &mut nudged, &live,
        ).unwrap();
        assert_eq!(out.added.len(), 2);
        assert_eq!(out.removed, vec!["Loose2"]);
        assert_eq!(out.moved_loose, vec![("Loose1".into(), (300, 300))]);
        assert!(nudged.is_empty());
        assert_eq!(original.len(), 5);
        assert_eq!(expected.len(), 5);
        assert!(original.iter().any(|i| i.path == "NewA" && i.pos == (550, 520)));

        // Disappearing >= 50% at once is still rejected as a catastrophic shell glitch.
        let wiped = vec![icon("BoxPin", (500, 500)), icon("Loose1", (300, 300))];
        assert!(reconcile_desktop_inventory(
            &mut original.clone(), &mut expected.clone(), &mut placed.clone(), &mut nudged.clone(), &wiped,
        ).is_err());

        // App-placed Box icon moving outside transaction is still rejected.
        let mut tampered = live.clone();
        tampered[0].pos = (501, 500);
        assert!(reconcile_desktop_inventory(
            &mut original, &mut expected, &mut placed, &mut nudged, &tampered,
        ).is_err());
    }

    #[test]
    fn box_to_desktop_drop_targets_preserve_snapped_non_overlapping_cells_on_main_and_scaled() {
        let monitors = [mon("main", 0, 0, 96, true), mon("side", 2560, -1, 120, false)];
        let active_boxes = [Rect { x: 80, y: 100, w: 300, h: 260 }];
        let expected = [
            ViewItem { path: "C:/d/A1.lnk".into(), label: "A1".into(), pos: (80, 140) },
            ViewItem { path: "C:/d/A2.lnk".into(), label: "A2".into(), pos: (155, 140) },
            ViewItem { path: "C:/d/ExistingLoose.lnk".into(), label: "Existing".into(), pos: (536, 430) },
        ];
        let placed = [("C:/d/A1.lnk".into(), (80, 140)), ("C:/d/A2.lnk".into(), (155, 140))];
        // Multi-icon BoxToDesktop drop near (540, 435) on 100% monitor (occupied by ExistingLoose at (536, 430))
        // and near (2670, 140) on 125% monitor -> both get distinct snapped non-overlapping cells.
        let resolved = compute_loose_drop_targets(
            &[("C:/d/A1.lnk".into(), (540, 435)), ("C:/d/A2.lnk".into(), (2670, 140))],
            true,
            false,
            Some((11, 2)),
            (75, 107),
            &monitors,
            &expected,
            &placed,
            &active_boxes,
            None,
        );
        assert_eq!(resolved.len(), 2);
        let p1 = resolved[0].1;
        let p2 = resolved[1].1;
        assert_ne!(p1, (536, 430));
        assert_eq!((p1.0 - 11).rem_euclid(75), 0);
        assert_eq!((p1.1 - 2).rem_euclid(107), 0);
        assert_eq!((p2.0 - 2574).rem_euclid(94), 0);
        assert_eq!((p2.1 - 1).rem_euclid(134), 0);
    }

    #[test]
    fn adopt_spacing_positions_updates_live_coords_and_preserves_box_originals() {
        let icon = |path: &str, pos| ViewItem { path: path.into(), label: path.into(), pos };
        let mut original = vec![
            icon("BoxPin", (11, 2)),
            icon("Nudged", (86, 2)),
            icon("Loose", (161, 2)),
        ];
        let mut expected = vec![
            icon("BoxPin", (422, 412)),
            icon("Nudged", (11, 109)),
            icon("Loose", (161, 2)),
        ];
        let mut moved = vec![("BoxPin".into(), (422, 412))];
        let mut nudged = vec![("Nudged".into(), (11, 109))];
        let zoomed_live = vec![
            icon("BoxPin", (450, 440)),
            icon("Nudged", (11, 140)),
            icon("Loose", (200, 2)),
        ];
        adopt_spacing_positions(
            &mut original,
            &mut expected,
            &mut moved,
            &mut nudged,
            &zoomed_live,
        )
        .unwrap();
        assert_eq!(original[0].pos, (11, 2));
        assert_eq!(original[1].pos, (86, 2));
        assert_eq!(original[2].pos, (200, 2));
        assert_eq!(expected[0].pos, (450, 440));
        assert_eq!(moved[0].1, (450, 440));
        assert_eq!(nudged[0].1, (11, 140));
    }

    #[test]
    fn top_edge_box_places_row0_icons_strictly_below_floating_header() {
        let mut state = State::empty();
        let box_id = state.add_group("Games", Bounds { x: 1284., y: 1., w: 467., h: 637. });
        let icons = [ViewItem {
            path: "C:/d/Game1.lnk".into(),
            label: "Game1".into(),
            pos: (11, 2),
        }];
        state.group_mut(box_id).unwrap().tab_mut().items.push(Item {
            id: 1,
            name: "Game1".into(),
            path: icons[0].path.clone(),
            kind: "lnk".into(),
            size: 16,
            modified: 0,
            label: None,
        });
        let member = NativeTwoBoxMembership::prepare_workspace(1, &state, &icons).unwrap();
        let screens = [Rect { x: 0, y: 0, w: 2560, h: 1440 }];
        let monitors = [mon("main", 0, 0, 96, true)];
        let coordinates = DesktopCoordinates { origin: (0, 0), scale: 1.0 };
        let (plans, _, _, _) = plan_layout(
            &member, &state, &icons, &icons, (75, 107), &screens, &monitors, coordinates,
            false, Some((11, 2)), |_| 0,
        ).unwrap();
        assert_eq!(plans[0].moves.len(), 1);
        assert!(plans[0].moves[0].to.1 >= crate::model::HEADER as i32 + 10);
    }

    #[test]
    fn can_attempt_fast_native_batch_requires_verified_shell_and_clean_free_moves() {
        assert!(can_attempt_fast_native_batch(true, false, false, false, true, true));
        assert!(!can_attempt_fast_native_batch(false, false, false, false, true, true));
        assert!(!can_attempt_fast_native_batch(true, true, false, false, true, true));
        assert!(!can_attempt_fast_native_batch(true, false, true, false, true, true));
        assert!(!can_attempt_fast_native_batch(true, false, false, true, true, true));
        assert!(!can_attempt_fast_native_batch(true, false, false, false, false, true));
        assert!(!can_attempt_fast_native_batch(true, false, false, false, true, false));
    }

    #[test]
    fn same_move_set_ignores_order_but_detects_pos_or_path_changes() {
        let a = vec![("C:/d/A.lnk".into(), (10, 20)), ("C:/d/B.lnk".into(), (30, 40))];
        let reordered_case = vec![("c:/d/b.lnk".into(), (30, 40)), ("C:/D/a.lnk".into(), (10, 20))];
        let shifted = vec![("C:/d/A.lnk".into(), (10, 21)), ("C:/d/B.lnk".into(), (30, 40))];
        let shorter = vec![("C:/d/A.lnk".into(), (10, 20))];
        assert!(same_move_set(&a, &reordered_case));
        assert!(!same_move_set(&a, &shifted));
        assert!(!same_move_set(&a, &shorter));
        assert!(same_move_set(&[], &[]));
    }

    #[test]
    fn plan_layout_supports_all_boxes_rolled_up_and_parks_all_their_icons() {
        let mut s = State::empty();
        let b1 = s.add_group("B1", Bounds { x: 100., y: 100., w: 300., h: 240. });
        let b2 = s.add_group("B2", Bounds { x: 460., y: 100., w: 300., h: 240. });
        let b3 = s.add_group("B3", Bounds { x: 820., y: 100., w: 300., h: 240. });
        let icons = vec![
            ViewItem { path: "C:/d/I1.lnk".into(), label: "I1".into(), pos: (11, 2) },
            ViewItem { path: "C:/d/I2.lnk".into(), label: "I2".into(), pos: (86, 2) },
            ViewItem { path: "C:/d/I3.lnk".into(), label: "I3".into(), pos: (161, 2) },
            ViewItem { path: "C:/d/Loose.lnk".into(), label: "Loose".into(), pos: (11, 430) },
        ];
        for (idx, gid) in [b1, b2, b3].into_iter().enumerate() {
            s.group_mut(gid).unwrap().tab_mut().items.push(Item {
                id: 1 + idx as u64,
                name: format!("I{}", idx + 1),
                path: icons[idx].path.clone(),
                kind: "lnk".into(),
                size: 16,
                modified: 0,
                label: None,
            });
            s.group_mut(gid).unwrap().rolled = true;
        }
        let member = NativeTwoBoxMembership::prepare_workspace(1, &s, &icons).unwrap();
        let screens = [Rect { x: 0, y: 0, w: 2560, h: 1440 }];
        let monitors = [mon("main", 0, 0, 96, true)];
        let coordinates = DesktopCoordinates { origin: (0, 0), scale: 1.0 };
        let (plans, hidden, hidden_park, nudged) = plan_layout(
            &member, &s, &icons, &icons, (75, 107), &screens, &monitors, coordinates,
            false, Some((11, 2)), |_| 0,
        ).unwrap();
        assert!(plans.is_empty());
        assert_eq!(hidden, vec![0, 1, 2]);
        let park = hidden_park.expect("all rolled-up Box icons must receive an on-screen park point");
        assert!(nudged.is_empty());

        let old = vec![
            ("C:/d/I1.lnk".into(), (122, 112)),
            ("C:/d/I2.lnk".into(), (482, 112)),
            ("C:/d/I3.lnk".into(), (842, 112)),
        ];
        let parked = vec![
            ("C:/d/I1.lnk".into(), park),
            ("C:/d/I2.lnk".into(), park),
            ("C:/d/I3.lnk".into(), park),
        ];
        let now = vec![
            ViewItem { path: "C:/d/I1.lnk".into(), label: "I1".into(), pos: (122, 112) },
            ViewItem { path: "C:/d/I2.lnk".into(), label: "I2".into(), pos: (482, 112) },
            ViewItem { path: "C:/d/I3.lnk".into(), label: "I3".into(), pos: (842, 112) },
            icons[3].clone(),
        ];
        let steps = plan_free_moves(
            &old, &[], &[], &parked, &[], &now,
            |p| icons.iter().find(|i| i.path.eq_ignore_ascii_case(p)).map(|i| i.pos),
        ).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].len(), 3);
    }

    #[test]
    fn displaced_hidden_park_icons_do_not_block_loose_to_box_drop_and_keep_visible_guard() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "dc-park-drop-test-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let loose_file = temp_dir.join("desktopcraft.exe");
        std::fs::write(&loose_file, b"mz").unwrap();
        let loose_path = loose_file.to_string_lossy().into_owned();

        let park = (2469, 1247);
        let mut s = State::empty();
        let docs = s.add_group("Documents", Bounds { x: 805., y: 180., w: 467., h: 569. });
        let vis_path = "C:/d/VisibleDoc.lnk".to_string();
        s.group_mut(docs).unwrap().tab_mut().items.push(pin(1, &vis_path, "VisibleDoc"));

        let mut expected = vec![
            ViewItem { path: vis_path.clone(), label: "VisibleDoc".into(), pos: (827, 192) },
            ViewItem { path: loose_path.clone(), label: "desktopcraft.exe".into(), pos: (1815, 679) },
        ];
        let mut original = vec![
            ViewItem { path: vis_path.clone(), label: "VisibleDoc".into(), pos: (15, 196) },
            ViewItem { path: loose_path.clone(), label: "desktopcraft.exe".into(), pos: (1815, 679) },
        ];
        let mut placed = vec![(vis_path.clone(), (827, 192))];
        let mut parked_paths = BTreeSet::new();
        for i in 0..31 {
            let p = format!("C:/d/Parked{i:02}.lnk");
            parked_paths.insert(p.to_lowercase());
            placed.push((p.clone(), park));
            expected.push(ViewItem { path: p.clone(), label: format!("Parked{i:02}"), pos: park });
            original.push(ViewItem { path: p, label: format!("Parked{i:02}"), pos: (15 + i * 75, 400) });
        }

        // Simulate user dragging `desktopcraft.exe` into `Documents` while Explorer unstacks all 31 parked icons.
        let mut observed = expected.clone();
        observed[1].pos = (1100, 243);
        for (i, item) in observed[2..].iter_mut().enumerate() {
            item.pos = (2469 - (i as i32 / 10) * 75, 1268 - (i as i32 % 10) * 97);
        }

        // Raw drop count is 32 (> 8), which previously aborted the drop and crashed the worker.
        let raw_err = crate::native_pin_drop::physical_multi_drop_identity(&expected, &observed, 8).unwrap_err();
        assert!(raw_err.contains("32 > 8"), "unexpected raw err: {raw_err}");

        let eligible = std::collections::HashSet::from([
            vis_path.to_lowercase(),
            loose_path.to_lowercase(),
        ]);
        let (displaced, filtered) = split_displaced_hidden_park(
            &expected,
            &observed,
            &parked_paths,
            Some(park),
            Some(&eligible),
        );
        assert_eq!(displaced.len(), 31);
        let cleaned_moved = crate::native_pin_drop::physical_multi_drop_identity(&expected, &filtered, 8).unwrap();
        assert_eq!(cleaned_moved, vec![loose_path.clone()]);

        let coords = DesktopCoordinates { origin: (0, 0), scale: 1.0 };
        let monitors = [mon("main", 0, 0, 96, true)];
        let prepared = crate::native_pin_drop::prepare_native_multi_drop_with_monitors(
            &s,
            &expected,
            &filtered,
            &eligible,
            (1830, 700),
            (1100, 250),
            (75, 97),
            coords,
            &monitors,
        ).unwrap();
        assert!(matches!(
            prepared.kind,
            crate::native_pin_drop::NativeMultiDropKind::DesktopToBox { to, .. } if to == docs
        ));

        // Rollback/idle reconcile accepts displaced hidden-park icons when filtered...
        let mut orig_c = original.clone();
        let mut exp_c = expected.clone();
        let mut placed_c = placed.clone();
        let mut nudged_c = Vec::new();
        let (_, rollback_filtered) = split_displaced_hidden_park(
            &expected, &observed, &parked_paths, Some(park), None,
        );
        let outcome = reconcile_desktop_inventory(
            &mut orig_c, &mut exp_c, &mut placed_c, &mut nudged_c, &rollback_filtered,
        ).unwrap();
        assert_eq!(outcome.moved_loose.len(), 1);

        // ...while a genuine unexpected move of a visible Box icon still fails closed.
        let mut visible_tampered = observed.clone();
        visible_tampered[0].pos = (900, 300);
        let (_, tampered_filtered) = split_displaced_hidden_park(
            &expected, &visible_tampered, &parked_paths, Some(park), Some(&eligible),
        );
        let guard_err = reconcile_desktop_inventory(
            &mut original, &mut expected, &mut placed, &mut nudged_c, &tampered_filtered,
        ).unwrap_err();
        assert!(guard_err.contains("app-placed Explorer icon moved"), "unexpected guard err: {guard_err}");

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn direct_inflight_drop_moves_from_observed_position_without_bounce_and_preserves_original() {
        let icon_path = "C:/d/Dragged.lnk".to_string();
        let original = vec![
            ViewItem { path: "C:/d/Stay.lnk".into(), label: "Stay".into(), pos: (11, 2) },
            ViewItem { path: icon_path.clone(), label: "Dragged".into(), pos: (1815, 679) },
        ];
        let mut expected = vec![
            ViewItem { path: "C:/d/Stay.lnk".into(), label: "Stay".into(), pos: (827, 192) },
            ViewItem { path: icon_path.clone(), label: "Dragged".into(), pos: (1815, 679) },
        ];
        let mut moved = vec![("C:/d/Stay.lnk".into(), (827, 192))];
        let observed_drop = vec![
            ViewItem { path: "C:/d/Stay.lnk".into(), label: "Stay".into(), pos: (827, 192) },
            ViewItem { path: icon_path.clone(), label: "Dragged".into(), pos: (1100, 243) },
        ];
        let mut tx = LiveLayoutTransaction {
            original: original.clone(),
            expected: expected.clone(),
            moved: moved.clone(),
            nudged_loose: Vec::new(),
            pages: BTreeMap::new(),
            journal: PathBuf::from("C:/nonexistent/recovery.json"),
            initial_monitor_signature: "sig".into(),
            snap_to_grid: false,
            original_snap_to_grid: false,
            desktop_grid_origin: None,
            hidden_park: None,
            parked_paths: BTreeSet::new(),
            pending_scaled_recheck_at: None,
            scaled_targets: BTreeMap::new(),
            native_batch_shell_verified: true,
            pending_drop_pre_positions: Vec::new(),
            count_changes: 1,
            restored: true,
        };
        tx.accept_inflight_drop_positions(&observed_drop, std::slice::from_ref(&icon_path)).unwrap();
        // Original baseline must stay untouched for clean exit rollback.
        assert_eq!(tx.original[1].pos, (1815, 679));
        // Expected tracks observed drop position so no bounce-back occurs.
        assert_eq!(tx.expected[1].pos, (1100, 243));
        assert_eq!(tx.pending_drop_pre_positions, vec![(icon_path.clone(), (1815, 679))]);

        let visible_placement = vec![
            ("C:/d/Stay.lnk".into(), (827, 192)),
            (icon_path.clone(), (902, 192)),
        ];
        let steps = plan_free_moves(
            &tx.moved,
            &[],
            &visible_placement,
            &[],
            &[],
            &tx.expected,
            |p| tx.original.iter().find(|i| i.path.eq_ignore_ascii_case(p)).map(|i| i.pos),
        ).unwrap();
        assert_eq!(steps, vec![vec![(icon_path.clone(), (902, 192))]]);

        expected[1].pos = (902, 192);
        moved.push((icon_path, (902, 192)));
        assert!(positions_settled(&expected, &original, &moved, &[], &BTreeSet::new()));
    }
}
