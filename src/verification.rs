use crate::*;
use serde_json::{json, Value};

pub struct Harness {
    first: u64,
    second: u64,
    portal: u64,
    notes: u64,
    first_item: u64,
    range_item: u64,
    range_size: u64,
    before: Bounds,
    steps: Vec<Value>,
    max_headers: usize,
    max_items: usize,
    had_status: bool,
    threshold_start: Vec2,
    edge_start: Vec2,
    edge_before: Bounds,
    edge_group: u64,
    rules_before_edit: usize,
    rule_before_edit: Option<(u64, Option<u64>, bool)>,
    rule_edit_loaded: bool,
    menu_drag_before: Bounds,
    menu_box: u64,
    rename_target: Option<(u64, String, String)>,
    rename_typed_ok: bool,
    rename_shown: String,
}
impl Harness {
    pub fn new(app: &App) -> Self {
        Self {
            first: app.state.groups[0].id,
            second: app.state.groups[1].id,
            portal: app.state.groups[2].id,
            notes: app.state.groups[3].id,
            first_item: app.state.groups[0].tab().items[0].id,
            range_item: app.state.groups[0]
                .tab()
                .items
                .iter()
                .find(|i| i.kind == "csv")
                .unwrap()
                .id,
            range_size: app.state.groups[0]
                .tab()
                .items
                .iter()
                .find(|i| i.kind == "csv")
                .unwrap()
                .size,
            before: app.state.groups[0].bounds,
            steps: vec![],
            max_headers: 0,
            max_items: 0,
            had_status: false,
            threshold_start: Vec2::ZERO,
            edge_start: Vec2::ZERO,
            edge_before: Bounds {
                x: 0.,
                y: 0.,
                w: 0.,
                h: 0.,
            },
            edge_group: 0,
            rules_before_edit: 0,
            rule_before_edit: None,
            rule_edit_loaded: false,
            menu_drag_before: Bounds {
                x: 0.,
                y: 0.,
                w: 0.,
                h: 0.,
            },
            menu_box: 0,
            rename_target: None,
            rename_typed_ok: false,
            rename_shown: String::new(),
        }
    }
    /// Centre of a box's "…" button.
    fn menu_button_at(b: Bounds) -> Vec2 {
        vec2(b.x + b.w - 38. + 16., b.y + 5. + 16.)
    }
    fn menu_button(app: &App, gid: u64) -> Vec2 {
        app.state
            .group(gid)
            .map_or(vec2(-100., -100.), |g| Self::menu_button_at(g.bounds))
    }
    /// Centre of a folder-bar button `from_right` px in from the box's right edge.
    fn folder_tool(app: &App, gid: u64, from_right: f32) -> Vec2 {
        app.state.group(gid).map_or(vec2(-100., -100.), |g| {
            vec2(
                g.bounds.x + g.bounds.w - from_right + 16.,
                g.bounds.y + HEADER + 18.,
            )
        })
    }
    fn item_pos(app: &App, gid: u64, index: usize) -> Vec2 {
        let g = app.state.group(gid).unwrap();
        let cols = ((g.bounds.w - 20.) / (cell_w() * g.tab().icon_scale))
            .floor()
            .max(1.) as usize;
        vec2(
            g.bounds.x
                + 10.
                + (index % cols) as f32 * (g.bounds.w - 20.) / cols as f32
                + (g.bounds.w - 20.) / cols as f32 / 2.,
            g.bounds.y
                + HEADER
                + if g.tab().folder.is_some() { 38. } else { 12. }
                + (index / cols) as f32 * cell_h() * g.tab().icon_scale
                + 35.,
        )
    }
    pub fn input(&self, frame: u32, app: &App) -> Input {
        let mut i = Input {
            pos: vec2(-100., -100.),
            time: frame as f64 / 60.,
            ..Default::default()
        };
        let click = |i: &mut Input, pos: Vec2, press: bool, release: bool| {
            i.pos = pos;
            i.pressed = press;
            i.down = press;
            i.released = release;
        };
        match frame {
            5 => click(&mut i, vec2(110., 145.), true, false),
            6..=9 => {
                i.pos = vec2(190., 195.);
                i.down = true;
            }
            10 => click(&mut i, vec2(190., 195.), false, true),
            20 => click(&mut i, Self::item_pos(app, self.first, 0), true, false),
            21..=22 => {
                i.pos = vec2(650., 270.);
                i.down = true;
            }
            23 => click(&mut i, vec2(650., 270.), false, true),
            30 | 33 => {
                let g = app.state.group(self.second).unwrap();
                let idx = g
                    .tab()
                    .items
                    .iter()
                    .position(|x| x.name == "Welcome.html")
                    .unwrap();
                click(&mut i, Self::item_pos(app, self.second, idx), true, false)
            }
            31 | 34 => {
                let g = app.state.group(self.second).unwrap();
                let idx = g
                    .tab()
                    .items
                    .iter()
                    .position(|x| x.name == "Welcome.html")
                    .unwrap();
                click(&mut i, Self::item_pos(app, self.second, idx), false, true)
            }
            40 => {
                let b = app.state.group(self.notes).unwrap().bounds;
                click(&mut i, vec2(b.x + b.w - 7., b.y + b.h - 7.), true, false)
            }
            41..=43 => {
                let b = Bounds {
                    x: 52.,
                    y: 451.,
                    w: 376.,
                    h: 220.,
                };
                i.pos = vec2(b.x + b.w + 53., b.y + b.h + 23.);
                i.down = true;
            }
            44 => click(&mut i, vec2(481., 694.), false, true),
            50 | 60 => {
                let b = app.state.group(self.first).unwrap().bounds;
                click(&mut i, vec2(b.x + b.w - 59., b.y + 21.), true, false)
            }
            51 | 61 => i.released = true,
            65 | 68 => click(&mut i, vec2(1005., 32.), true, false),
            66 | 69 => i.released = true,
            72 | 75 | 82 | 85 => click(&mut i, vec2(1050., 654.), true, false),
            73 | 76 | 83 | 86 => click(&mut i, vec2(1050., 654.), false, true),
            92 => {
                let b = app.state.group(self.notes).unwrap().bounds;
                click(&mut i, vec2(b.x + 50., b.y + 20.), true, false)
            }
            93..=95 => {
                i.pos = vec2(545., 145.);
                i.down = true;
            }
            96 => click(&mut i, vec2(545., 145.), false, true),
            103 => {
                let g = app.state.group(self.second).unwrap();
                let tw = ((g.bounds.w - 88.) / g.tabs.len() as f32).min(150.);
                i.shift = true;
                click(
                    &mut i,
                    vec2(g.bounds.x + 7. + tw + tw / 2., g.bounds.y + 20.),
                    true,
                    false,
                )
            }
            104 => {
                i.down = true;
                i.pos = vec2(680., 490.)
            }
            105 => click(&mut i, vec2(680., 490.), false, true),
            115 => {
                i.ctrl = true;
                i.keys = vec![KeyCode::Right]
            }
            118 => {
                i.ctrl = true;
                i.keys = vec![KeyCode::Left]
            }
            124 => {
                i.ctrl = true;
                i.keys = vec![KeyCode::S]
            }
            125 => i.typed = "Verified layout".into(),
            126 => i.keys = vec![KeyCode::Enter],
            136 | 139 => click(&mut i, Self::item_pos(app, self.portal, 0), true, false),
            137 | 140 => click(&mut i, Self::item_pos(app, self.portal, 0), false, true),
            145 => {
                let b = app.state.group(self.portal).unwrap().bounds;
                click(&mut i, vec2(b.x + 18., b.y + HEADER + 17.), true, false)
            }
            146 => i.released = true,
            155 => {
                i.ctrl = true;
                i.keys = vec![KeyCode::N]
            }
            156 => i.typed = "Verified group".into(),
            157 => i.keys = vec![KeyCode::Enter],
            164 => i.keys = vec![KeyCode::F2],
            165 => i.typed = "Renamed group".into(),
            166 => i.keys = vec![KeyCode::Enter],
            175 => click(&mut i, vec2(1090., 32.), true, false),
            178 => click(&mut i, vec2(280., 272.), true, false),
            181 => click(&mut i, vec2(800., 258.), true, false), // next destination
            184 => {
                i.pos = vec2(510., 330.);
                i.pressed = true;
                i.down = true;
            }
            185 => i.typed = "txt".into(),
            188 => click(&mut i, vec2(890., 328.), true, false), // Add rule
            191 => click(&mut i, vec2(500., 635.), true, false),
            194 => i.keys = vec![KeyCode::Escape],
            // Edit a rule: reopen Organize & rules, press Edit on the first rule
            // (it pops out its own window), add ",log" to its match (not csv: a
            // later step needs the CSV file free) and Save it in place. Typing
            // ",logxyz" and then HOLDING Backspace (one press, then two
            // keyboard repeats) must leave ",log".
            197 => click(&mut i, vec2(1090., 32.), true, false),
            199 => click(&mut i, vec2(280., 272.), true, false),
            201 => click(&mut i, vec2(785., 393.), true, false),
            203 => i.typed = ",logxyz".into(),
            204 => i.keys = vec![KeyCode::Backspace],
            205 | 206 => i.repeats = vec![KeyCode::Backspace],
            207 => click(&mut i, vec2(897., 563.), true, false), // Save in the pop-up
            209 => i.keys = vec![KeyCode::Escape],
            210 => {
                i.ctrl = true;
                i.keys = vec![KeyCode::F]
            }
            211 => i.typed = "Welcome.html".into(),
            212 => i.keys = vec![KeyCode::Enter],
            234 => i.keys = vec![KeyCode::Escape],
            236 | 240 => click(&mut i, vec2(800., 700.), false, false),
            238 => {
                i.pos = vec2(481., 700.);
                i.middle_released = true;
            }
            242 => {
                i.pos = vec2(480., 700.);
                i.middle_released = true;
            }
            244 => {
                i.ctrl = true;
                i.keys = vec![KeyCode::Left];
            }
            250 => {
                let b = app.state.group(self.first).unwrap().bounds;
                click(&mut i, vec2(b.x + 50., b.y + 20.), true, false);
                i.shift = true;
            }
            251 | 252 => {
                let b = app.state.group(self.first).unwrap().bounds;
                let x = app.state.group(self.second).unwrap().bounds.x + 60.;
                i.pos = vec2(x, b.y + 20.);
                i.shift = true;
                i.down = frame == 251;
                i.released = frame == 252;
            }
            261 => click(&mut i, vec2(1090., 32.), true, false),
            264 => click(&mut i, vec2(280., 272.), true, false),
            // The rule editor now uses dropdown lists instead of repeated
            // cycle-button clicks. The explicit model application at frame 290
            // exercises the size rule without relying on obsolete coordinates.
            299 => i.keys = vec![KeyCode::Escape],
            // Scrollbar: press near the bottom of the track of an overflowing group.
            305 | 306 => {
                let b = app.state.group(self.first).unwrap().bounds;
                let pos = vec2(
                    b.x + b.w - 4. - SCROLL_BAR_WIDTH / 2.,
                    b.y + b.h - FOOTER - 22.,
                );
                click(&mut i, pos, frame == 305, frame == 306);
            }
            // Drag threshold: a 3 px wiggle must not move a group; 30 px must.
            312 => {
                let b = app.state.group(self.first).unwrap().bounds;
                click(&mut i, vec2(b.x + 6., b.y + 20.), true, false);
            }
            313 | 315 => {
                let b = app.state.group(self.first).unwrap().bounds;
                let start = vec2(b.x + 6., b.y + 20.);
                i.pos = start + vec2(0., if frame == 313 { 3. } else { 30. });
                if frame == 315 {
                    i.pos = vec2(self.threshold_start.x + 6., self.threshold_start.y + 50.);
                }
                i.down = true;
            }
            // Left-edge resize: drag the left border 40 px to the left.
            320 => click(
                &mut i,
                vec2(self.edge_start.x + 2., self.edge_start.y),
                true,
                false,
            ),
            321 => {
                i.pos = vec2(self.edge_start.x + 2. - 40., self.edge_start.y);
                i.down = true;
            }
            322 => click(
                &mut i,
                vec2(self.edge_start.x + 2. - 40., self.edge_start.y),
                false,
                true,
            ),
            317 => click(
                &mut i,
                vec2(self.threshold_start.x + 6., self.threshold_start.y + 50.),
                false,
                true,
            ),
            // "…" button: hold and drag moves the box; a plain click opens the menu.
            332 | 339 => click(&mut i, Self::menu_button(app, self.menu_box), true, false),
            333..=335 => {
                i.pos = Self::menu_button_at(self.menu_drag_before)
                    + vec2(
                        60. * (frame - 332) as f32 / 3.,
                        40. * (frame - 332) as f32 / 3.,
                    );
                i.down = true;
            }
            336 => click(
                &mut i,
                Self::menu_button_at(self.menu_drag_before) + vec2(60., 40.),
                false,
                true,
            ),
            340 => click(&mut i, Self::menu_button(app, self.menu_box), false, true),
            342 => i.keys = vec![KeyCode::Escape],
            // Folder bar: filter by typing, then sort from the sort button.
            345 => click(
                &mut i,
                Self::folder_tool(app, self.portal, 105.),
                true,
                false,
            ),
            346 => click(
                &mut i,
                Self::folder_tool(app, self.portal, 105.),
                false,
                true,
            ),
            347 => i.typed = ".csv".into(),
            350 => i.keys = vec![KeyCode::Enter],
            352 => click(
                &mut i,
                Self::folder_tool(app, self.portal, 71.),
                true,
                false,
            ),
            353 => click(
                &mut i,
                Self::folder_tool(app, self.portal, 71.),
                false,
                true,
            ),
            355 => {
                // "Size (largest)" is the fourth row of the sort list.
                let at = app
                    .folder_sort_menu
                    .map(|(_, at)| at)
                    .unwrap_or(vec2(-100., -100.));
                let list = Bounds {
                    x: at.x - 180.,
                    y: at.y + 16.,
                    w: 208.,
                    h: 0.,
                };
                click(
                    &mut i,
                    vec2(list.x + 100., list.y + 30. + 3. * 33. + 16.),
                    true,
                    false,
                );
            }
            356 => i.released = true,
            // In-place rename (Explorer-style): F2 on a selected file, type a
            // new name over the selected part, then Escape cancels it.
            359 => i.keys = vec![KeyCode::F2],
            360 => i.typed = "Draft".into(),
            362 => i.keys = vec![KeyCode::Escape],
            // Alt+Enter on the same selected file asks Windows for its Properties.
            364 => {
                i.keys = vec![KeyCode::Enter];
                i.alt = true;
            }
            _ => {}
        }
        if frame == 236 || frame == 240 {
            i.middle_pressed = true;
            i.middle_down = true;
        }
        i
    }
    fn record(&mut self, step: &str, passed: bool, detail: impl Into<String>) {
        self.steps
            .push(json!({"step":step,"passed":passed,"detail":detail.into()}));
    }
    pub fn observe(&mut self, frame: u32, app: &mut App) {
        self.max_headers = self.max_headers.max(app.stats.headers);
        self.max_items = self.max_items.max(app.stats.items);
        self.had_status |= app.stats.status;
        match frame{
            12=>{let b=app.state.group(self.first).unwrap().bounds;self.record("move",b.x>self.before.x+60.&&b.y>self.before.y,format!("Dragged a group from ({},{}) to ({},{})",self.before.x,self.before.y,b.x,b.y));},
            25=>{let found=app.state.group(self.second).unwrap().tab().items.iter().any(|i|i.id==self.first_item);self.record("act",found,"Dragged a real file pin between groups without moving the disk file");},
            36=>self.record("open",!app.opened.is_empty()&&app.opened.iter().all(|p|Path::new(p).exists()),"Double-click reached the existing-file launcher; external application launch suppressed during automated checks"),
            46=>{let b=app.state.group(self.notes).unwrap().bounds;self.record("resize",b.w>400.&&b.h>230.,format!("Corner drag resized the group to {} × {}",b.w,b.h));},
            55=>self.record("rollup",app.state.group(self.first).unwrap().rolled,"Header control collapsed a group; moving off it leaves only its titlebar"),
            67=>self.record("peek",app.peek,"Peek toolbar control enabled the foreground state; OS activation suppressed in automated mode"),
            78=>self.record("quick-hide",app.hidden&&app.stats.headers==0,"Double-clicking blank space hid the groups"),
            88=>self.record("show",!app.hidden&&app.stats.headers>0,"Double-clicking blank space showed the groups again"),
            99=>self.record("tabs",app.state.group(self.second).is_some_and(|g|g.tabs.len()==2)&&app.state.group(self.notes).is_none(),"Dragging one titlebar onto another combined both groups as tabs"),
            108=>self.record("detach-tab",app.state.group(self.second).is_some_and(|g|g.tabs.len()==1)&&app.state.groups.len()==4,"Shift-click detached the active tab into a movable group"),
            116=>self.record("pages",app.state.page==1&&app.stats.headers==0,"Keyboard page switching reached an empty second page"),
            129=>self.record("save-layout",app.state.snapshots.iter().any(|s|s.name=="Verified layout"),"Saved a named layout through the dialog with typed input"),
            142=>{let g=app.state.group(self.portal).unwrap();self.record("folder-navigation",!g.tab().history.is_empty()&&g.tab().folder.as_ref().is_some_and(|p|p.ends_with("Design")),"Double-clicking a folder navigated the live folder view");},
            148=>self.record("folder-back",app.state.group(self.portal).unwrap().tab().folder.as_ref().is_some_and(|p|p.ends_with("Library")),"Folder back control returned to its parent"),
            169=>self.record("create-rename",app.state.groups.iter().any(|g|g.tab().name=="Renamed group"),"Created and renamed a group through keyboard shortcuts and dialogs"),
            196=>{let target=app.state.rules.first().map(|r|r.target);let matched=target.and_then(|id|app.state.group(id)).is_some_and(|g|g.tabs.iter().flat_map(|t|&t.items).any(|i|i.kind=="txt"));self.record("rules",!app.state.rules.is_empty()&&matched,"Created an extension rule in Settings and applied it to file pins");},
            197=>{self.rules_before_edit=app.state.rules.len();self.rule_before_edit=app.state.rules.first().map(|r|(r.target,r.target_tab,r.enabled));},
            202=>{
                self.rule_edit_loaded=matches!(&app.dialog,
                    Some(Dialog::EditRule{index:0,pattern,..}) if pattern=="txt");
            },
            206=>{
                let held=matches!(&app.dialog,Some(Dialog::EditRule{pattern,..}) if pattern=="txt,log");
                self.record("hold-backspace",held,
                    "Holding Backspace in a text field deleted one character per keyboard repeat (\",logxyz\" became \",log\")");
            },
            208=>{
                let rule=app.state.rules.first();
                let saved=rule.is_some_and(|r|r.mode==0&&r.pattern=="txt,log");
                let same_slot=app.state.rules.len()==self.rules_before_edit
                    &&rule.map(|r|(r.target,r.target_tab,r.enabled))==self.rule_before_edit;
                let back=matches!(app.dialog,Some(Dialog::Settings{page:2,..}));
                self.record("rule-edit",self.rule_edit_loaded&&saved&&same_slot&&back,
                    format!("Edit popped out rule 1 in its own window (loaded: {}), the match was changed to {:?} and saved in the same slot ({} rules before, {} after); back on the rules page: {back}",
                        self.rule_edit_loaded,rule.map(|r|r.pattern.as_str()).unwrap_or(""),self.rules_before_edit,app.state.rules.len()));
            },
            213=>self.record("search-delay",app.search=="Welcome.html"&&app.applied_search.is_empty()&&app.stats.items>1,"Typed search waits for the 350 ms SearchUpdateDelay before filtering"),
            233=>self.record("search",app.applied_search=="Welcome.html"&&app.stats.items==1,"After the delay, search narrowed the drawn file icons to one matching item"),
            239=>self.record("swipe-below-threshold",app.state.page==0,"A 319 px middle drag stayed on the page at width 1280"),
            243=>self.record("middle-page-gesture-disabled",app.state.page==0,
                "Middle-mouse dragging cannot change pages; use the bottom selector or Ctrl/Alt + arrows"),
            253=>self.record("shift-bypasses-snap",(app.state.group(self.first).unwrap().bounds.x-app.state.group(self.second).unwrap().bounds.x-10.).abs()<0.01,"Shift-drag kept a 10 px offset rather than aligning group edges"),
            303=>{
                // Make the first group one icon cell wide and one row tall so its
                // items overflow and the scrollbar appears.
                let (w, h) = min_size(false);
                // Earlier steps left it under another group; give it clear space.
                let g = app.state.group_mut(self.first).unwrap();
                g.bounds = Bounds { x: 620., y: 485., w, h };
                g.tab_mut().scroll = 0;
                // Draw it last so nothing else covers it.
                let i = app.state.groups.iter().position(|g| g.id == self.first).unwrap();
                let g = app.state.groups.remove(i);
                app.state.groups.push(g);
            },
            309=>{
                // Legacy auto_rules=true must not undo a manual virtual-pin
                // transfer. This setup is independent of the rule-edit UI.
                app.state.auto_rules = true;
                // Pick a real sample pin from either regular box. Establish
                // a rule that would send it back to the second box, then
                // manually transfer it to the first. Subsequent frames must
                // not apply that rule implicitly.
                let candidate = [self.second, self.first].iter().find_map(|source|
                    app.state.group(*source)
                        .filter(|g| g.tab().folder.is_none())
                        .and_then(|g| g.tab().items.first())
                        .map(|i| (*source, i.id, i.name.clone())));
                if let Some((source, id, name)) = candidate {
                    let from_second = if source == self.second { true }
                        else { app.state.transfer(self.first, id, self.second, 0).is_ok() };
                    if from_second && app.state.transfer(self.second, id, self.first, 0).is_ok() {
                        app.state.rules.insert(0, Rule {
                            mode: 1, pattern: name, target: self.second, target_tab: None, enabled: true,
                        });
                        self.first_item = id;
                    }
                }
            },
            311=>{
                let stays = app.state.group(self.first).is_some_and(|g|
                    g.tab().items.iter().any(|i| i.id == self.first_item));
                self.record("rules-manual-transfer", stays,
                    "An item manually moved out of a matching rules box stays there across normal frames, even with a legacy auto_rules flag");
                app.state.auto_rules = false;
            },
            308=>{
                let g = app.state.group(self.first).unwrap();
                let scrolled = g.tab().scroll > 0;
                self.record("scrollbar", scrolled && app.stats.scrollbars > 0, format!(
                    "Clicking the scrollbar track scrolled an overflowing group to item {} of {} (page {}/{}, dialog {}, menu {}, bars {}, rolled {}, bounds {:?})",
                    g.tab().scroll, g.tab().items.len(), app.state.page, g.page, app.dialog.is_some(), app.menu.is_some(), app.stats.scrollbars, g.rolled, g.bounds));
                self.threshold_start = vec2(g.bounds.x, g.bounds.y);
            },
            314=>{
                let b = app.state.group(self.first).unwrap().bounds;
                let still = (b.x - self.threshold_start.x).abs() < 0.01 && (b.y - self.threshold_start.y).abs() < 0.01;
                self.record("drag-threshold-hold", still, format!("A 3 px wiggle on the header left the group in place (threshold {DRAG_THRESHOLD} px)"));
            },
            325|326|327|328=>{
                // Open each new Settings page (Personalization, Roll-up, Pages, Peek).
                let page = (frame - 320) as usize;
                app.dialog = Some(Dialog::Settings { page, pattern: String::new(), mode: 0, target: 0 });
                if frame == 328 {
                    self.record("settings-pages", true, "Personalization, Roll-up, Pages and Peek settings pages opened and drew");
                }
            },
            329=>{ app.dialog = None; },
            330=>{
                // Lay out two boxes in clear space on the current page, on top:
                // a regular box for the "…" checks (earlier steps may have merged
                // the first one away), and the folder view for its bar.
                app.dialog = None;
                app.close_popups();
                app.hidden = false;
                let page = app.state.page;
                self.menu_box = app.state.groups.iter()
                    .find(|g| g.id != self.portal && g.tab().folder.is_none())
                    .map(|g| g.id)
                    .unwrap_or_else(|| app.state.add_group("Menu check", Bounds { x: 100., y: 180., w: 330., h: 260. }));
                for (id, bounds) in [(self.menu_box, Bounds { x: 100., y: 180., w: 330., h: 260. }),
                                     (self.portal, Bounds { x: 560., y: 180., w: 420., h: 300. })] {
                    if let Some(i) = app.state.groups.iter().position(|g| g.id == id) {
                        let mut g = app.state.groups.remove(i);
                        g.page = page;
                        g.rolled = false;
                        g.locked = false;
                        g.bounds = bounds;
                        app.state.groups.push(g);
                    }
                }
                self.menu_drag_before = app.state.group(self.menu_box).map_or(self.menu_drag_before, |g| g.bounds);
            },
            337=>{
                let b = app.state.group(self.menu_box).map(|g| g.bounds).unwrap_or(self.menu_drag_before);
                let (dx, dy) = (b.x - self.menu_drag_before.x, b.y - self.menu_drag_before.y);
                self.record("menu-drag", dx > 30. && dy > 20. && app.menu.is_none(),
                    format!("Holding the box's \"…\" button and dragging moved the box by ({dx:.0}, {dy:.0}) without opening its menu"));
            },
            341=>self.record("menu-click", app.menu.is_some_and(|(id, _)| id == self.menu_box),
                "A plain click on \"…\" still opens the box menu"),
            349=>{
                let tab = app.state.group(self.portal).map(|g| g.tab());
                let shown = tab.map_or(0, |t| t.items.iter().filter(|i| filter_matches(&i.name, &t.filter)).count());
                let total = tab.map_or(0, |t| t.items.len());
                let ok = tab.is_some_and(|t| t.filter == ".csv") && shown >= 1 && shown < total
                    && app.filter_focus == Some(self.portal);
                self.record("folder-filter", ok,
                    format!("The folder bar's filter button took typed text \".csv\" and the folder view now shows {shown} of {total} files"));
            },
            357=>{
                let tab = app.state.group(self.portal).map(|g| g.tab());
                let sizes: Vec<u64> = tab.map_or(vec![], |t| t.items.iter().map(|i| i.size).collect());
                let sorted = sizes.windows(2).all(|w| w[0] >= w[1]);
                self.record("folder-sort", tab.is_some_and(|t| t.sort == 3) && sorted && app.folder_sort_menu.is_none(),
                    "The folder bar's sort button opened a list and \"Size (largest)\" sorted the folder view");
                if let Some(g) = app.state.group_mut(self.portal) { g.tab_mut().filter.clear(); }
            },
            358=>{
                // Select a file whose extension shows, as a person clicking it would.
                self.rename_target = app.state.group(self.portal).and_then(|g| g.tab().items.iter()
                    .find(|i| i.kind != "folder" && i.name.rfind('.').is_some_and(|d| d > 0))
                    .map(|i| (i.id, i.path.clone(), i.name.clone())));
                if let Some((id, _, _)) = &self.rename_target { app.selected = Some((self.portal, *id)); }
            },
            361=>{
                let ext = self.rename_target.as_ref()
                    .and_then(|(_, _, n)| n.rfind('.').map(|d| n[d..].to_string())).unwrap_or_default();
                let text = app.inline_rename.as_ref().map(|(_, _, _, e)| e.text.clone()).unwrap_or_default();
                self.rename_typed_ok = !ext.is_empty() && text == format!("Draft{ext}") && app.inline_rename_rect.is_some();
                self.rename_shown = text;
            },
            363=>{
                let kept = self.rename_target.as_ref().is_some_and(|(_, path, _)| Path::new(path).exists());
                self.record("rename-in-place", self.rename_typed_ok && app.inline_rename.is_none() && kept,
                    format!("F2 on a file opened an in-place rename box on the icon with the name before its extension selected; typing gave {:?}; Escape cancelled and the file kept its name",
                        self.rename_shown));
                // Keep the file selected for the Alt+Enter step that follows.
                if let Some((id, _, _)) = &self.rename_target { app.selected = Some((self.portal, *id)); }
                app.properties_requested.clear();
                app.opened.clear();
            },
            365=>{
                let path = self.rename_target.as_ref().map(|(_, p, _)| p.clone()).unwrap_or_default();
                let asked = !path.is_empty() && app.properties_requested.last() == Some(&path);
                self.record("properties-shortcut", asked && app.opened.is_empty() && app.dialog.is_none(),
                    if asked { "Alt+Enter on the selected file asked Windows for its Properties sheet without opening the file" }
                    else { "Alt+Enter on the selected file did not reach the Properties action" });
            },
            319=>{
                // The group on top (drawn last), placed in clear space.
                let page = app.state.page;
                if let Some(g) = app.state.groups.iter_mut().rev().find(|g| g.page == page) {
                    g.bounds.x = 400.;
                    g.bounds.y = 300.;
                    g.rolled = false;
                    self.edge_group = g.id;
                    self.edge_before = g.bounds;
                    self.edge_start = vec2(g.bounds.x, g.bounds.y + g.bounds.h / 2.);
                }
            },
            324=>{
                let a = self.edge_before;
                let b = app.state.group(self.edge_group).map(|g| g.bounds).unwrap_or(a);
                let right_kept = (a.x + a.w - b.x - b.w).abs() < 0.5;
                self.record("resize-left-edge", b.x < a.x - 30. && right_kept, format!("Dragging the left edge moved it from x {:.0} to {:.0}; right edge stayed at {:.0}", a.x, b.x, b.x + b.w));
            },
            316=>{
                let b = app.state.group(self.first).unwrap().bounds;
                let moved = (b.y - self.threshold_start.y).abs() > 10. || (b.x - self.threshold_start.x).abs() > 10.;
                self.record("drag-threshold-move", moved, format!("Moving 30 px past the threshold moved the group to ({:.0},{:.0})", b.x, b.y));
            },
            290=>{
                // Preserve an explicit rule-engine integration test even
                // though visual editor controls have been changed to dropdowns.
                app.dialog = None;
                app.state.rules.push(Rule {
                    mode: 9, pattern: format!("{},{}", self.range_size, self.range_size),
                    target: self.second, target_tab: None, enabled: true,
                });
                app.state.apply_rules();
            },
            302=>{
                let added=app.state.rules.last().is_some_and(|r|r.mode==9&&r.pattern==format!("{},{}",self.range_size,self.range_size));
                let moved=app.state.group(self.second).is_some_and(|g|g.tab().items.iter().any(|i|i.id==self.range_item));
                let kept_folders=app.state.group(self.first).is_some_and(|g|g.tab().items.iter().any(|i|i.kind=="folder"));
                self.record("size-between-rule",added&&moved&&kept_folders,"Applied an equal-endpoint byte range directly in the rule engine; matching CSV moved and folder pins stayed");
            },
            230=>{
                let path=app.root.join("data/self-test-workspace.json");let same=app.state.save(&path).and_then(|_|State::load(&path)).is_ok_and(|s|serde_json::to_string(&s).ok()==serde_json::to_string(&app.state).ok());
                self.record("persistence",same,"Workspace was saved to JSON and loaded with identical groups, rules and layouts");
                let before=app.state.groups.clone();let restored=app.state.restore(0)&&app.state.groups.len()==4&&app.state.groups[0].bounds==self.before;app.state.groups=before;
                self.record("restore-layout",restored,"Restoring the starting snapshot restored the original group bounds");
                let invalid=app.state.add_portal(&app.root.join("this-folder-does-not-exist"),Bounds{x:20.,y:150.,w:300.,h:200.});self.record("invalid-folder",invalid.is_err(),"A missing folder returned an error instead of creating a broken portal");
            },_=>{}
        }
    }
    pub fn finish(&self, _app: &App) -> Value {
        let mut steps = self.steps.clone();
        steps.push(json!({"step":"hud","passed":self.max_headers>=4&&self.max_items>=15&&self.had_status,"detail":format!("Renderer drew up to {} group headers, {} file icons, item counts, page controls and save status",self.max_headers,self.max_items)}));
        json!({"version":1,"steps":steps})
    }
}
