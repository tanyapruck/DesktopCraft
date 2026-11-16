//! Local AI integration: one typed engine for CLI, JSON and MCP.
//! The running DesktopCraft instance remains the sole workspace writer.
//! Mutating requests from agents always require a native Windows user consent dialog.
use crate::{
    host,
    model::{Bounds, Item, Rule, MAX_RULES},
    App,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object", "properties":properties, "required": required, "additionalProperties":false})
}
fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"inputSchema":schema(properties,required)})
}
pub fn tools() -> Vec<Value> {
    vec![
        tool("desktopcraft_status","Read active workspace, page and box counts; no file contents.",json!({}),&[]),
        tool("desktopcraft_list_pages","List page IDs and names.",json!({}),&[]),
        tool("desktopcraft_list_boxes","List boxes and all their tabs with IDs, counts and linked-folder flags.",json!({}),&[]),
        tool("desktopcraft_list_pins","List virtual pin IDs and labels. Include full paths only when explicitly needed.",
            json!({"group_id":{"type":"integer"},"tab_id":{"type":"integer"},"include_paths":{"type":"boolean"}}),&[]),
        tool("desktopcraft_list_folder_paths","List linked folder paths; never read folder file contents.",json!({}),&[]),
        tool("desktopcraft_list_rules","Read manual sorting rules and their tab destinations.",json!({}),&[]),
        tool("desktopcraft_list_layouts","List available saved workspace snapshots.",json!({}),&[]),
        tool("desktopcraft_preview_rules","Simulate manual rules over virtual pins without changing assignments; first matching rule wins.",json!({}),&[]),
        tool("desktopcraft_create_box","Create a virtual box on a page; requires approval in DesktopCraft.",
            json!({"name":{"type":"string"},"page":{"type":"integer"}}),&["name"]),
        tool("desktopcraft_rename_box","Rename a box's active tab; requires approval.",
            json!({"group_id":{"type":"integer"},"name":{"type":"string"}}),&["group_id","name"]),
        tool("desktopcraft_create_page","Create a named page; requires approval.",
            json!({"name":{"type":"string"}}),&["name"]),
        tool("desktopcraft_rename_page","Rename an existing page by zero-based page index; requires approval.",
            json!({"page":{"type":"integer"},"name":{"type":"string"}}),&["page","name"]),
        tool("desktopcraft_switch_page","Switch to an existing zero-based page index; requires approval.",
            json!({"page":{"type":"integer"}}),&["page"]),
        tool("desktopcraft_pin_file","Virtually pin an existing file to a regular tab, without moving it; requires approval.",
            json!({"path":{"type":"string"},"group_id":{"type":"integer"},"tab_id":{"type":"integer"}}),&["path","group_id","tab_id"]),
        tool("desktopcraft_move_pin","Move a virtual pin between specific tabs; never move its file; requires approval.",
            json!({"source_group":{"type":"integer"},"source_tab":{"type":"integer"},"item_id":{"type":"integer"},
                "target_group":{"type":"integer"},"target_tab":{"type":"integer"}}),
            &["source_group","source_tab","item_id","target_group","target_tab"]),
        tool("desktopcraft_move_pins","Move several virtual pins, from any regular tabs, into one regular tab with a single approval that lists every icon; never moves files. Prefer this over repeated move_pin calls.",
            json!({"item_ids":{"type":"array","items":{"type":"integer"},"minItems":1,"maxItems":MAX_BATCH_PINS},
                "target_group":{"type":"integer"},"target_tab":{"type":"integer"}}),
            &["item_ids","target_group","target_tab"]),
        tool("desktopcraft_add_rule","Add a manual sorting rule for a regular tab; modes 0-10 (10=any folder); requires approval.",
            json!({"mode":{"type":"integer","minimum":0,"maximum":10},"pattern":{"type":"string"},
                "group_id":{"type":"integer"},"tab_id":{"type":"integer"}}),
            &["mode","pattern","group_id","tab_id"]),
        tool("desktopcraft_apply_rules","Apply existing rules to virtual pins only, not filesystem files; requires approval.",json!({}),&[]),
        tool("desktopcraft_save_layout","Store a named snapshot of virtual boxes/tabs/pages; requires approval.",
            json!({"name":{"type":"string"}}),&["name"]),
        tool("desktopcraft_undo_layout","Undo the last in-memory workspace change, not real filesystem operations; requires approval.",json!({}),&[]),
        tool("desktopcraft_rename_tab","Rename a specific tab, including an inactive tab; requires approval.",
            json!({"group_id":{"type":"integer"},"tab_id":{"type":"integer"},"name":{"type":"string"}}),
            &["group_id","tab_id","name"]),
        tool("desktopcraft_activate_tab","Switch a box to a particular tab; requires approval.",
            json!({"group_id":{"type":"integer"},"tab_id":{"type":"integer"}}),&["group_id","tab_id"]),
        tool("desktopcraft_unpin_file","Remove a virtual pin without touching the file; requires approval.",
            json!({"group_id":{"type":"integer"},"tab_id":{"type":"integer"},"item_id":{"type":"integer"}}),
            &["group_id","tab_id","item_id"]),
        tool("desktopcraft_move_box_page","Move a box (with all tabs) to an existing page; requires approval.",
            json!({"group_id":{"type":"integer"},"page":{"type":"integer"}}),&["group_id","page"]),
        tool("desktopcraft_merge_boxes","Combine two unlocked boxes as tabs, keeping rule destinations; requires approval.",
            json!({"source_group":{"type":"integer"},"target_group":{"type":"integer"}}),
            &["source_group","target_group"]),
        tool("desktopcraft_detach_tab","Separate a tab into its own box, keeping rule destinations; requires approval.",
            json!({"group_id":{"type":"integer"},"tab_id":{"type":"integer"}}),&["group_id","tab_id"]),
        tool("desktopcraft_link_folder","Link a local folder to an empty regular tab (view only); requires approval.",
            json!({"group_id":{"type":"integer"},"tab_id":{"type":"integer"},"path":{"type":"string"}}),
            &["group_id","tab_id","path"]),
        tool("desktopcraft_unlink_folder","Remove a folder link without deleting its files; requires approval.",
            json!({"group_id":{"type":"integer"},"tab_id":{"type":"integer"}}),&["group_id","tab_id"]),
    ]
}
fn mutation(name: &str) -> bool {
    matches!(
        name,
        "desktopcraft_create_box"
            | "desktopcraft_rename_box"
            | "desktopcraft_create_page"
            | "desktopcraft_rename_page"
            | "desktopcraft_switch_page"
            | "desktopcraft_pin_file"
            | "desktopcraft_move_pin"
            | "desktopcraft_move_pins"
            | "desktopcraft_add_rule"
            | "desktopcraft_apply_rules"
            | "desktopcraft_save_layout"
            | "desktopcraft_undo_layout"
            | "desktopcraft_rename_tab"
            | "desktopcraft_activate_tab"
            | "desktopcraft_unpin_file"
            | "desktopcraft_move_box_page"
            | "desktopcraft_merge_boxes"
            | "desktopcraft_detach_tab"
            | "desktopcraft_link_folder"
            | "desktopcraft_unlink_folder"
    )
}
fn known(name: &str) -> bool {
    tools().iter().any(|tool| tool["name"] == name)
}
fn num(args: &Value, field: &str) -> Result<u64, String> {
    args.get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{field} must be a nonnegative integer"))
}
fn text_arg<'a>(args: &'a Value, field: &str, limit: usize) -> Result<&'a str, String> {
    let value = args
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field} is required"))?
        .trim();
    if value.is_empty() || value.chars().count() > limit || value.chars().any(char::is_control) {
        return Err(format!("{field} must be 1-{limit} printable characters"));
    }
    Ok(value)
}
fn find_regular(app: &App, gid: u64, tid: u64) -> Result<(), String> {
    let group = app.state.group(gid).ok_or("Box ID not found")?;
    let tab = group
        .tabs
        .iter()
        .find(|t| t.id == tid)
        .ok_or("Tab ID not found in box")?;
    if tab.folder.is_some() {
        return Err("AI virtual actions cannot target linked Folder Views".into());
    }
    Ok(())
}
/// Most pins one batch move may carry (and one approval may list).
const MAX_BATCH_PINS: usize = 100;
/// A pin requested by ID, with where it sits now.
struct PinAt {
    group: u64,
    tab: u64,
    id: u64,
    name: String,
    tab_name: String,
}
/// Find every requested pin in a regular tab, in request order.
fn locate_pins(app: &App, args: &Value) -> Result<Vec<PinAt>, String> {
    let ids = args
        .get("item_ids")
        .and_then(Value::as_array)
        .ok_or("item_ids must be a list of pin IDs")?;
    if ids.is_empty() || ids.len() > MAX_BATCH_PINS {
        return Err(format!("item_ids must list 1-{MAX_BATCH_PINS} pins"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut found = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id.as_u64().ok_or("item_ids must be nonnegative integers")?;
        if !seen.insert(id) {
            return Err(format!("Pin {id} is listed twice"));
        }
        let hit = app
            .state
            .groups
            .iter()
            .flat_map(|g| {
                g.tabs
                    .iter()
                    .filter(|t| t.folder.is_none())
                    .map(move |t| (g.id, t))
            })
            .find_map(|(gid, t)| {
                t.items.iter().find(|i| i.id == id).map(|i| PinAt {
                    group: gid,
                    tab: t.id,
                    id,
                    name: i.display().to_string(),
                    tab_name: t.name.clone(),
                })
            });
        found.push(hit.ok_or_else(|| format!("Pin {id} was not found in a regular tab"))?);
    }
    Ok(found)
}
/// Move located pins into one regular tab as a single undoable change.
/// Pins whose file is already shown in the target are simply taken off
/// their old tab, the same outcome as a one-by-one move.
fn move_located_pins(app: &mut App, pins: &[PinAt], tg: u64, tt: u64) -> Result<usize, String> {
    let moving: Vec<&PinAt> = pins
        .iter()
        .filter(|p| (p.group, p.tab) != (tg, tt))
        .collect();
    if moving.is_empty() {
        return Ok(0);
    }
    let mut sources: Vec<(u64, u64)> = Vec::new();
    for p in &moving {
        if !sources.contains(&(p.group, p.tab)) {
            sources.push((p.group, p.tab));
        }
    }
    app.remember();
    let outcome = (|| -> Result<(), String> {
        for (sg, st) in sources {
            let dest_paths: std::collections::HashSet<String> = app
                .state
                .group(tg)
                .and_then(|g| g.tabs.iter().find(|t| t.id == tt))
                .map(|t| t.items.iter().map(|i| i.path.to_lowercase()).collect())
                .ok_or("Destination tab missing")?;
            let source = app
                .state
                .group_mut(sg)
                .and_then(|g| g.tabs.iter_mut().find(|t| t.id == st))
                .ok_or("Source tab missing")?;
            let mut ids = Vec::new();
            let mut already_shown = Vec::new();
            for p in moving.iter().filter(|p| (p.group, p.tab) == (sg, st)) {
                // A same-file pin moved earlier in this batch has already removed it.
                if let Some(item) = source.items.iter().find(|i| i.id == p.id) {
                    if dest_paths.contains(&item.path.to_lowercase()) {
                        already_shown.push(p.id)
                    } else {
                        ids.push(p.id)
                    }
                }
            }
            source.items.retain(|i| !already_shown.contains(&i.id));
            if !ids.is_empty() {
                app.state.transfer_many_between_tabs(sg, st, &ids, tg, tt)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        // All or nothing: put the workspace back exactly as it was.
        if let Some(previous) = app.undo.pop() {
            app.state = previous;
        }
        return Err(error);
    }
    Ok(moving.len())
}
/// Check a request before asking the person, so they never approve
/// something that would then fail.
fn precheck(app: &App, name: &str, args: &Value) -> Result<(), String> {
    match name {
        "desktopcraft_move_pins" => {
            locate_pins(app, args)?;
            find_regular(app, num(args, "target_group")?, num(args, "target_tab")?)
        }
        "desktopcraft_move_pin" => {
            let (sg, st, item) = (
                num(args, "source_group")?,
                num(args, "source_tab")?,
                num(args, "item_id")?,
            );
            find_regular(app, sg, st)?;
            find_regular(app, num(args, "target_group")?, num(args, "target_tab")?)?;
            pin_name(app, sg, st, item)
                .map(|_| ())
                .ok_or_else(|| "Pin ID not found in source tab".into())
        }
        _ => Ok(()),
    }
}
fn pin_name(app: &App, gid: u64, tid: u64, item: u64) -> Option<String> {
    app.state
        .group(gid)?
        .tabs
        .iter()
        .find(|t| t.id == tid)?
        .items
        .iter()
        .find(|i| i.id == item)
        .map(|i| i.display().to_string())
}
fn page_index(args: &Value) -> Result<usize, String> {
    usize::try_from(num(args, "page")?).map_err(|_| "Page index is too large".into())
}
fn result(ok: bool, data: Value) -> Value {
    if ok {
        json!({"ok":true,"data":data})
    } else {
        json!({"ok":false,"error":data})
    }
}

/// Pure, testable command dispatch. `approved` can only become true through
/// a native user confirmation in process_request, never via agent-supplied JSON.
pub fn execute(app: &mut App, name: &str, args: &Value, approved: bool) -> Result<Value, String> {
    if !known(name) {
        return Err("Unknown DesktopCraft tool".into());
    }
    if !args.is_object() {
        return Err("Tool arguments must be a JSON object".into());
    }
    if mutation(name) && !approved {
        return Err("Action requires confirmation in DesktopCraft".into());
    }
    match name {
        "desktopcraft_status" => Ok(json!({
            "application":"DesktopCraft","workspace_version":app.state.version,
            "page":app.state.page,"pages":app.state.pages.len(),
            "boxes":app.state.groups.len(),"rules":app.state.rules.len(),
            "mode":if app.host.is_some() {"desktop"} else {"windowed"},
            "file_operations_permitted":false
        })),
        "desktopcraft_list_pages" => Ok(json!(app.state.pages.iter().enumerate().map(|(index,name)|json!({
            "index":index,"name":name,"boxes":app.state.groups.iter().filter(|g|g.page==index).count(),
            "selected":app.state.page==index
        })).collect::<Vec<_>>())),
        "desktopcraft_list_boxes" => Ok(json!(app.state.groups.iter().map(|g|json!({
            "group_id":g.id,"page":g.page,"active_tab":g.tab().id,"locked":g.locked,
            "bounds":{"x":g.bounds.x,"y":g.bounds.y,"width":g.bounds.w,"height":g.bounds.h},
            "tabs":g.tabs.iter().map(|t|json!({"tab_id":t.id,"name":t.name,
                "pin_count":t.items.len(),"linked_folder":t.folder.is_some()})).collect::<Vec<_>>()
        })).collect::<Vec<_>>())),
        "desktopcraft_list_pins" => {
            let gid=args.get("group_id").map(|_|num(args,"group_id")).transpose()?;
            let tid=args.get("tab_id").map(|_|num(args,"tab_id")).transpose()?;
            let paths=args.get("include_paths").and_then(Value::as_bool).unwrap_or(false);
            let mut items=Vec::new();
            let mut total=0usize;
            for g in &app.state.groups {
                if gid.is_some_and(|id|id!=g.id) {continue;}
                for t in &g.tabs {
                    if t.folder.is_some() || tid.is_some_and(|id|id!=t.id) {continue;}
                    for item in &t.items {
                        total+=1;
                        if items.len()>=300 {continue;}
                        let mut row=json!({"group_id":g.id,"tab_id":t.id,"item_id":item.id,
                            "name":item.display(),"type":item.kind});
                        if paths {row["path"]=json!(item.path);}
                        items.push(row);
                    }
                }
            }
            Ok(json!({"items":items,"total":total,"truncated":total>300,"paths_included":paths}))
        }
        "desktopcraft_list_folder_paths" => Ok(json!(app.state.groups.iter()
            .flat_map(|g|g.tabs.iter().filter_map(move |t|t.folder.as_ref().map(|p|
                json!({"group_id":g.id,"tab_id":t.id,"name":t.name,"path":p}))))
            .collect::<Vec<_>>())),
        "desktopcraft_list_layouts" => Ok(json!(app.state.snapshots.iter().enumerate().map(|(index,s)|
            json!({"index":index,"name":s.name,"pages":s.pages.len(),"boxes":s.groups.len(),"saved_at":s.time}))
            .collect::<Vec<_>>())),
        "desktopcraft_list_rules" => Ok(json!(app.state.rules.iter().enumerate().map(|(index,r)|json!({
            "index":index,"mode":r.mode,"pattern":r.pattern,
            "group_id":r.target,"tab_id":r.target_tab,"enabled":r.enabled,
            "destination_valid":app.state.rule_destination(r).is_some()
        })).collect::<Vec<_>>())),
        "desktopcraft_preview_rules" => {
            let destinations:Vec<_>=app.state.rules.iter().take(MAX_RULES)
                .filter_map(|r|app.state.rule_destination(r).map(|dest|(r,dest))).collect();
            let mut proposals=vec![];
            let mut total=0usize;
            for g in &app.state.groups {
                for t in &g.tabs {
                    if t.folder.is_some(){continue;}
                    for item in &t.items {
                        if let Some((gid,tid))=app.state.best_rule_destination(item) {
                            if gid!=g.id || tid!=t.id {
                                total+=1;
                                if proposals.len()<200 {
                                    let label=destinations.iter().find(|(r,dest)|
                                        *dest==(gid,tid) && r.matches(item))
                                        .map_or("",|(r,_)|r.pattern.as_str());
                                    proposals.push(json!({"item_id":item.id,"name":item.display(),
                                        "from_group":g.id,"from_tab":t.id,"to_group":gid,"to_tab":tid,
                                        "matched_rule":label,"disk_effect":"none"}));
                                }
                            }
                        }
                    }
                }
            }
            Ok(json!({"proposed_moves":proposals,"total":total,"truncated":total>200,
                "disk_effect":"none","applied":false}))
        }
        "desktopcraft_create_box" => {
            let name=text_arg(args,"name",60)?;
            let page=if args.get("page").is_some(){page_index(args)?}else{app.state.page};
            if page>=app.state.pages.len(){return Err("Page index not found".into());}
            let bounds=Bounds{x:70.,y:100.+(app.state.groups.len()%8) as f32*36.,
                w:330.,h:260.};
            app.remember();
            let previous=app.state.page;
            app.state.page=page;
            let id=app.state.add_group(name,bounds);
            app.state.page=previous;
            Ok(json!({"group_id":id,"page":page,"name":name}))
        }
        "desktopcraft_rename_box" => {
            let gid=num(args,"group_id")?;
            let name=text_arg(args,"name",60)?;
            if app.state.group(gid).is_none(){return Err("Box not found".into());}
            app.remember();
            let tab=app.state.group_mut(gid).unwrap().tab_mut();
            tab.name=name.into();
            Ok(json!({"group_id":gid,"tab_id":tab.id,"name":name}))
        }
        "desktopcraft_create_page" => {
            let name=text_arg(args,"name",60)?;
            app.remember();
            let index=app.state.create_page(name).map_err(str::to_owned)?;
            Ok(json!({"page":index,"name":name}))
        }
        "desktopcraft_rename_page" => {
            let page=page_index(args)?;
            let name=text_arg(args,"name",60)?;
            if page>=app.state.pages.len(){return Err("Page index not found".into());}
            app.remember();
            app.state.rename_page(page,name).map_err(str::to_owned)?;
            Ok(json!({"page":page,"name":name}))
        }
        "desktopcraft_switch_page" => {
            let page=page_index(args)?;
            if page>=app.state.pages.len(){return Err("Page index not found".into());}
            app.remember();
            app.state.page=page;
            app.selected=None;
            Ok(json!({"page":page,"name":app.state.pages[page]}))
        }
        "desktopcraft_pin_file" => {
            let gid=num(args,"group_id")?;
            let tid=num(args,"tab_id")?;
            find_regular(app,gid,tid)?;
            let path=text_arg(args,"path",2048)?;
            let file=Path::new(path);
            if !file.is_file(){return Err("Pin target must be an existing regular file".into());}
            if app.state.groups.iter().flat_map(|g|g.tabs.iter())
                .filter(|t|t.folder.is_none()).flat_map(|t|t.items.iter())
                .any(|item|item.path.eq_ignore_ascii_case(path)){
                return Err("File is already pinned. Use move_pin to reassign it".into());
            }
            let id=app.state.id();
            let item=Item::from_path(file,id).map_err(|e|e.to_string())?;
            app.remember();
            app.state.group_mut(gid).unwrap().tabs.iter_mut().find(|t|t.id==tid).unwrap().items.push(item);
            Ok(json!({"group_id":gid,"tab_id":tid,"item_id":id,"disk_effect":"none"}))
        }
        "desktopcraft_move_pin" => {
            let sg=num(args,"source_group")?;
            let st=num(args,"source_tab")?;
            let item=num(args,"item_id")?;
            let tg=num(args,"target_group")?;
            let tt=num(args,"target_tab")?;
            find_regular(app,sg,st)?;
            find_regular(app,tg,tt)?;
            // Confirm source before adding an undo entry.
            if !app.state.group(sg).unwrap().tabs.iter().find(|t|t.id==st).unwrap()
                .items.iter().any(|i|i.id==item){return Err("Pin ID not found in source tab".into());}
            app.remember();
            let count=app.state.group(tg).unwrap().tabs.iter().find(|t|t.id==tt).unwrap().items.len();
            app.state.transfer_between_tabs(sg,st,item,tg,tt,count)?;
            Ok(json!({"item_id":item,"target_group":tg,"target_tab":tt,"disk_effect":"none"}))
        }
        "desktopcraft_move_pins" => {
            let pins=locate_pins(app,args)?;
            let tg=num(args,"target_group")?;
            let tt=num(args,"target_tab")?;
            find_regular(app,tg,tt)?;
            let moved=move_located_pins(app,&pins,tg,tt)?;
            Ok(json!({"moved":moved,"already_there":pins.len()-moved,
                "names":pins.iter().filter(|p|(p.group,p.tab)!=(tg,tt)).map(|p|p.name.as_str()).collect::<Vec<_>>(),
                "target_group":tg,"target_tab":tt,"disk_effect":"none"}))
        }
        "desktopcraft_add_rule" => {
            let mode=num(args,"mode")?;
            if mode>10 {return Err("Rule mode must be 0-10".into());}
            let pattern=if mode==10 { "folder" } else { text_arg(args,"pattern",128)? };
            let gid=num(args,"group_id")?;
            let tid=num(args,"tab_id")?;
            find_regular(app,gid,tid)?;
            if app.state.rules.len()>=MAX_RULES{return Err("Rule limit (50) reached".into());}
            app.remember();
            app.state.rules.push(Rule{mode:mode as u8,pattern:pattern.into(),
                target:gid,target_tab:Some(tid),enabled:true});
            Ok(json!({"rule_index":app.state.rules.len()-1,"group_id":gid,"tab_id":tid,
                "auto_applied":false}))
        }
        "desktopcraft_apply_rules" => {
            app.remember();
            let moved=app.state.apply_rules();
            Ok(json!({"moved_pins":moved,"disk_effect":"none"}))
        }
        "desktopcraft_save_layout" => {
            let name=text_arg(args,"name",60)?;
            app.remember();
            app.state.snapshot(name);
            Ok(json!({"name":name,"snapshots":app.state.snapshots.len()}))
        }
        "desktopcraft_undo_layout" => {
            let previous=app.undo.pop().ok_or("No layout history to undo")?;
            app.state=previous;
            app.selected=None;
            Ok(json!({"undone":true,"disk_effect":"none"}))
        }
        "desktopcraft_rename_tab" => {
            let gid=num(args,"group_id")?;
            let tid=num(args,"tab_id")?;
            let name=text_arg(args,"name",60)?;
            let position=app.state.group(gid).and_then(|g|g.tabs.iter().position(|t|t.id==tid))
                .ok_or("Tab not found in box")?;
            app.remember();
            app.state.group_mut(gid).unwrap().tabs[position].name=name.into();
            Ok(json!({"group_id":gid,"tab_id":tid,"name":name}))
        }
        "desktopcraft_activate_tab" => {
            let gid=num(args,"group_id")?;
            let tid=num(args,"tab_id")?;
            let position=app.state.group(gid).and_then(|g|g.tabs.iter().position(|t|t.id==tid))
                .ok_or("Tab not found in box")?;
            app.remember();
            app.state.group_mut(gid).unwrap().active=position;
            app.selected=Some((gid,0));
            Ok(json!({"group_id":gid,"active_tab":tid}))
        }
        "desktopcraft_unpin_file" => {
            let gid=num(args,"group_id")?;
            let tid=num(args,"tab_id")?;
            let iid=num(args,"item_id")?;
            find_regular(app,gid,tid)?;
            let tab=app.state.group(gid).unwrap().tabs.iter().find(|t|t.id==tid).unwrap();
            if !tab.items.iter().any(|item|item.id==iid){return Err("Pin ID not found".into());}
            app.remember();
            app.state.group_mut(gid).unwrap().tabs.iter_mut().find(|t|t.id==tid).unwrap()
                .items.retain(|item|item.id!=iid);
            app.selected=None;
            Ok(json!({"unpinned":iid,"disk_effect":"none"}))
        }
        "desktopcraft_move_box_page" => {
            let gid=num(args,"group_id")?;
            let page=page_index(args)?;
            if page>=app.state.pages.len(){return Err("Page not found".into());}
            if app.state.group(gid).is_none(){return Err("Box not found".into());}
            app.remember();
            app.state.group_mut(gid).unwrap().page=page;
            app.selected=None;
            Ok(json!({"group_id":gid,"page":page}))
        }
        "desktopcraft_merge_boxes" => {
            let source=num(args,"source_group")?;
            let target=num(args,"target_group")?;
            if source==target {return Err("A box cannot merge into itself".into());}
            if !app.state.groups.iter().any(|g|g.id==source)
                || !app.state.groups.iter().any(|g|g.id==target) {return Err("Box not found".into());}
            if app.state.group(source).unwrap().locked||app.state.group(target).unwrap().locked {
                return Err("Unlock both boxes before merging".into());
            }
            app.remember();
            if !app.state.merge(source,target){return Err("Could not merge boxes".into());}
            app.selected=Some((target,0));
            Ok(json!({"group_id":target,"tabs":app.state.group(target).unwrap().tabs.len()}))
        }
        "desktopcraft_detach_tab" => {
            let gid=num(args,"group_id")?;
            let tid=num(args,"tab_id")?;
            let group=app.state.group(gid).ok_or("Box not found")?;
            if group.tabs.len()<2 {return Err("The box contains only one tab".into());}
            if group.locked {return Err("Unlock the box before detaching".into());}
            let position=group.tabs.iter().position(|t|t.id==tid).ok_or("Tab not found")?;
            let bounds=Bounds{x:group.bounds.x+35.,y:group.bounds.y+35.,..group.bounds};
            let page=group.page;
            app.remember();
            let id=app.state.detach(gid,position,bounds).ok_or("Could not detach tab")?;
            app.state.group_mut(id).unwrap().page=page;
            app.selected=Some((id,0));
            Ok(json!({"new_group_id":id,"tab_id":tid,"page":page}))
        }
        "desktopcraft_link_folder" => {
            let gid=num(args,"group_id")?;
            let tid=num(args,"tab_id")?;
            let path=text_arg(args,"path",2048)?;
            let folder=Path::new(path);
            if !folder.is_dir(){return Err("Folder path does not exist".into());}
            let group=app.state.group(gid).ok_or("Box not found")?;
            let tab=group.tabs.iter().find(|t|t.id==tid).ok_or("Tab not found")?;
            if tab.folder.is_none() && !tab.items.is_empty() {
                return Err("A tab with virtual pins cannot be linked to a folder".into());
            }
            let old=tab.clone();
            app.remember();
            let tab=app.state.group_mut(gid).unwrap().tabs.iter_mut().find(|t|t.id==tid).unwrap();
            if let Err(e)=tab.navigate(path){*tab=old;app.undo.pop();return Err(e);}
            Ok(json!({"group_id":gid,"tab_id":tid,"path":path,"disk_effect":"none"}))
        }
        "desktopcraft_unlink_folder" => {
            let gid=num(args,"group_id")?;
            let tid=num(args,"tab_id")?;
            let group=app.state.group(gid).ok_or("Box not found")?;
            let tab=group.tabs.iter().find(|t|t.id==tid).ok_or("Tab not found")?;
            if tab.folder.is_none(){return Err("Tab is not linked to a folder".into());}
            app.remember();
            let tab=app.state.group_mut(gid).unwrap().tabs.iter_mut().find(|t|t.id==tid).unwrap();
            tab.folder=None;
            tab.history.clear();
            tab.items.clear();
            tab.scroll=0;
            Ok(json!({"group_id":gid,"tab_id":tid,"unlinked":true,"disk_effect":"none"}))
        }
        _=>Err("Unknown tool".into())
    }
}

/// The agent's text is untrusted, never interpolate raw filesystem paths in
/// confirmation dialogs or debug logs. An explicit native click is mandatory.
fn approval_message(app: &App, name: &str, args: &Value) -> String {
    let operation = name
        .strip_prefix("desktopcraft_")
        .unwrap_or(name)
        .replace('_', " ");
    let safe = |text: &str, max: usize| -> String {
        text.chars().filter(|c| !c.is_control()).take(max).collect()
    };
    let tab_label = |id: u64| {
        app.state
            .groups
            .iter()
            .flat_map(|g| g.tabs.iter())
            .find(|t| t.id == id)
            .map_or("Unknown tab".to_string(), |t| safe(&t.name, 70))
    };
    // Batch moves name every icon, so the person sees exactly what changes.
    if name == "desktopcraft_move_pins" {
        if let (Ok(pins), Some(tt)) = (
            locate_pins(app, args),
            args.get("target_tab").and_then(Value::as_u64),
        ) {
            const SHOWN: usize = 25;
            let target = tab_label(tt);
            let mut lines: Vec<String> = pins
                .iter()
                .take(SHOWN)
                .map(|p| {
                    format!(
                        "•  {}   (from {})",
                        safe(&p.name, 60),
                        safe(&p.tab_name, 40)
                    )
                })
                .collect();
            if pins.len() > SHOWN {
                lines.push(format!("…and {} more", pins.len() - SHOWN));
            }
            return format!("Allow AI to move {} icon{} into \"{target}\"?\n\n{}\n\nThis changes DesktopCraft workspace only. No files will be moved, deleted or overwritten. Undo can put them back.\n\nChoose No to cancel.",
                pins.len(),if pins.len()==1 {""} else {"s"},lines.join("\n"));
        }
    }
    let mut details = Vec::new();
    if let (Some(sg), Some(st), Some(item)) = (
        args.get("source_group").and_then(Value::as_u64),
        args.get("source_tab").and_then(Value::as_u64),
        args.get("item_id").and_then(Value::as_u64),
    ) {
        if let Some(icon) = pin_name(app, sg, st, item) {
            details.push(format!("Icon: {}", safe(&icon, 80)));
        }
    } else if let (Some(g), Some(t), Some(item)) = (
        args.get("group_id").and_then(Value::as_u64),
        args.get("tab_id").and_then(Value::as_u64),
        args.get("item_id").and_then(Value::as_u64),
    ) {
        if let Some(icon) = pin_name(app, g, t, item) {
            details.push(format!("Icon: {}", safe(&icon, 80)));
        }
    }
    if let Some(name) = args.get("name").and_then(Value::as_str) {
        details.push(format!("Name: {}", safe(name, 80)));
    }
    if let Some(path) = args.get("path").and_then(Value::as_str) {
        details.push(format!("File: {}", safe(path, 240)));
    }
    for key in ["group_id", "source_group", "target_group"] {
        if let Some(id) = args.get(key).and_then(Value::as_u64) {
            // A box with several tabs is named by all of them, not only the
            // tab that happens to be showing.
            let label = app.state.group(id).map_or("Unknown box".to_string(), |g| {
                g.tabs
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" / ")
            });
            let box_name = match key {
                "source_group" => "from box",
                "target_group" => "to box",
                _ => "box",
            };
            details.push(format!("{box_name}: {} (#{id})", safe(&label, 90)));
        }
    }
    for key in ["tab_id", "source_tab", "target_tab"] {
        if let Some(id) = args.get(key).and_then(Value::as_u64) {
            let tab_name = match key {
                "source_tab" => "from tab",
                "target_tab" => "to tab",
                _ => "tab",
            };
            details.push(format!("{tab_name}: {} (#{id})", tab_label(id)));
        }
    }
    if let Some(pattern) = args.get("pattern").and_then(Value::as_str) {
        details.push(format!("Rule match: {}", safe(pattern, 120)));
    }
    if let Some(page) = args.get("page").and_then(Value::as_u64) {
        details.push(format!("Page: {}", page.saturating_add(1)));
    }
    format!("Allow AI to {operation}?\n\n{}\n\nThis changes DesktopCraft workspace only. No files will be moved, deleted or overwritten.\n\nChoose No to cancel.",
        details.join("\n"))
}
fn request_dir() -> std::path::PathBuf {
    host::data_dir().join("agent-bridge")
}
fn token_ok(token: &str) -> bool {
    !token.is_empty() && token.len() <= 80 && token.bytes().all(|c| c.is_ascii_digit() || c == b'-')
}
fn request_path(token: &str) -> std::path::PathBuf {
    request_dir().join(format!("{token}.request"))
}
fn response_path(token: &str) -> std::path::PathBuf {
    request_dir().join(format!("{token}.response"))
}
/// Executed ONLY by the active UI process, which owns state and confirmation.
pub fn process_request(app: &mut App, token: &str, time: f64) {
    if !token_ok(token) {
        return;
    }
    let path = request_path(token);
    let body = fs::read(&path).ok().filter(|bytes| bytes.len() <= 16384);
    let _ = fs::remove_file(&path);
    let answer = (|| -> Result<Value, String> {
        let body = body.ok_or("Missing or oversized request")?;
        let request: Value = serde_json::from_slice(&body).map_err(|_| "Invalid tool JSON")?;
        let name = request
            .get("name")
            .and_then(Value::as_str)
            .ok_or("Tool name missing")?;
        let args = request.get("arguments").ok_or("Tool arguments missing")?;
        if !known(name) {
            return Err("Unknown tool".into());
        }
        if !args.is_object() {
            return Err("Tool arguments must be a JSON object".into());
        }
        // Never ask the person to approve a request that would then fail.
        if mutation(name) {
            precheck(app, name, args)?;
        }
        // Default-No native confirmation: an agent cannot self-authorize.
        let approved = !mutation(name) || host::ask_default_no(&approval_message(app, name, args));
        let output = execute(app, name, args, approved)?;
        if mutation(name) {
            // Fast-sync changed state while preserving the normal save scheduler.
            if let Some(h) = app.host.as_mut() {
                h.last_sync = -10.;
            }
            app.save(time);
        }
        Ok(output)
    })();
    let response = match answer {
        Ok(data) => result(true, data),
        Err(e) => result(false, json!(e)),
    };
    if let Ok(bytes) = serde_json::to_vec(&response) {
        if bytes.len() <= 512 * 1024 {
            let _ = fs::write(response_path(token), bytes);
        }
    }
}
/// One response per request, no second desktop process and no direct JSON
/// workspace writes. A UI confirmation can take time, so allow two minutes.
pub fn dispatch(name: &str, arguments: &Value) -> Value {
    if !known(name) {
        return result(false, json!("Unknown tool"));
    }
    if !host::instance_running() {
        return result(
            false,
            json!("Open DesktopCraft first; no running workspace instance was found"),
        );
    }
    let body = json!({"name":name,"arguments":arguments});
    let bytes = match serde_json::to_vec(&body) {
        Ok(b) if b.len() <= 16384 => b,
        _ => return result(false, json!("Tool request exceeds the 16 KiB limit")),
    };
    let token = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let dir = request_dir();
    if fs::create_dir_all(&dir).is_err() {
        return result(false, json!("Cannot open local agent mailbox"));
    }
    let request = request_path(&token);
    let response = response_path(&token);
    if fs::write(&request, bytes).is_err() {
        return result(false, json!("Cannot send agent request"));
    }
    if host::send_command(&format!("ai-tool {token}")).is_err() {
        let _ = fs::remove_file(&request);
        return result(false, json!("Cannot contact DesktopCraft"));
    }
    for _ in 0..2400 {
        if let Ok(bytes) = fs::read(&response) {
            let _ = fs::remove_file(&response);
            return serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| result(false, json!("Invalid tool response")));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = fs::remove_file(&request);
    result(
        false,
        json!("DesktopCraft did not respond within two minutes"),
    )
}
pub fn cli(name: &str, arguments: &str) -> i32 {
    let result = serde_json::from_str(arguments)
        .map(|args: Value| dispatch(name, &args))
        .unwrap_or_else(|_| result(false, json!("Invalid JSON arguments")));
    println!("{}", result);
    if result["ok"] == true {
        0
    } else {
        1
    }
}
fn mcp_reply(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let output = match method {
        "initialize" => {
            json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{"listChanged":false}},
            "serverInfo":{"name":"desktopcraft","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools":tools()}),
        "tools/call" => {
            let params = &message["params"];
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let response = dispatch(name, &args);
            json!({"content":[{"type":"text","text":response.to_string()}],
                "isError":response["ok"]!=true})
        }
        _ => {
            return Some(json!({"jsonrpc":"2.0","id":id,
            "error":{"code":-32601,"message":"Method not found"}}))
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":output}))
}
/// MCP JSON-RPC stdio transport. All debugging stays off stdout.
pub fn serve_mcp() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.len() > 64 * 1024 {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => mcp_reply(&msg),
            Err(_) => Some(json!({"jsonrpc":"2.0","id":null,
                "error":{"code":-32700,"message":"Parse error"}})),
        };
        if let Some(response) = response {
            if writeln!(stdout, "{}", response).is_err() {
                break;
            }
            if stdout.flush().is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        let dir =
            std::env::temp_dir().join(format!("desktopcraft-agent-tests-{}", std::process::id()));
        App::new(dir, true, false)
    }
    #[test]
    fn tools_are_discoverable_and_have_json_schemas() {
        let all = tools();
        assert!(all.len() >= 15);
        for t in &all {
            assert_eq!(t["inputSchema"]["type"], "object");
        }
        assert!(
            mcp_reply(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap()["result"]
                ["tools"]
                .as_array()
                .unwrap()
                .len()
                >= 15
        );
        assert!(mcp_reply(&json!({"method":"notifications/initialized"})).is_none());
    }
    #[test]
    fn agent_mutations_rejected_without_verified_user_approval() {
        let mut app = app();
        let before = app.state.groups.len();
        assert!(execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"Games"}),
            false
        )
        .is_err());
        assert_eq!(app.state.groups.len(), before);
        assert!(execute(&mut app, "desktopcraft_status", &json!({}), false).is_ok());
    }
    #[test]
    fn create_and_move_virtual_pins_with_tab_ids() {
        let mut app = app();
        let a = execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"Work"}),
            true,
        )
        .unwrap();
        let b = execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"Games"}),
            true,
        )
        .unwrap();
        let (a, b) = (
            a["group_id"].as_u64().unwrap(),
            b["group_id"].as_u64().unwrap(),
        );
        let src = app.state.group(a).unwrap().tab().id;
        let dst = app.state.group(b).unwrap().tab().id;
        let file = std::env::temp_dir().join(format!(
            "desktopcraft-agent-test-file-{}.txt",
            std::process::id()
        ));
        fs::write(&file, b"fixture").unwrap();
        let pin = execute(
            &mut app,
            "desktopcraft_pin_file",
            &json!({"path":file,"group_id":a,"tab_id":src}),
            true,
        )
        .unwrap();
        let item = pin["item_id"].as_u64().unwrap();
        execute(
            &mut app,
            "desktopcraft_move_pin",
            &json!({"source_group":a,"source_tab":src,
            "item_id":item,"target_group":b,"target_tab":dst}),
            true,
        )
        .unwrap();
        assert!(app.state.group(a).unwrap().tab().items.is_empty());
        assert_eq!(app.state.group(b).unwrap().tab().items.len(), 1);
        assert_eq!(fs::read(&file).unwrap(), b"fixture");
        let _ = fs::remove_file(file);
    }
    #[test]
    fn rule_preview_does_not_apply_until_explicit_action() {
        let mut app = app();
        let a = execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"A"}),
            true,
        )
        .unwrap();
        let b = execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"B"}),
            true,
        )
        .unwrap();
        let (a, b) = (
            a["group_id"].as_u64().unwrap(),
            b["group_id"].as_u64().unwrap(),
        );
        let src = app.state.group(a).unwrap().tab().id;
        let dest = app.state.group(b).unwrap().tab().id;
        let id = app.state.id();
        app.state.group_mut(a).unwrap().tab_mut().items.push(Item {
            id,
            name: "note.txt".into(),
            path: "C:/fixture/note.txt".into(),
            kind: "txt".into(),
            size: 0,
            modified: 0,
            label: None,
        });
        execute(
            &mut app,
            "desktopcraft_add_rule",
            &json!({"mode":0,"pattern":"txt","group_id":b,"tab_id":dest}),
            true,
        )
        .unwrap();
        let preview = execute(&mut app, "desktopcraft_preview_rules", &json!({}), false).unwrap();
        assert_eq!(preview["total"], 1);
        assert_eq!(app.state.group(a).unwrap().tab().items.len(), 1);
        execute(&mut app, "desktopcraft_apply_rules", &json!({}), true).unwrap();
        assert!(app.state.group(a).unwrap().tab().items.is_empty());
        assert_eq!(app.state.group(b).unwrap().tab().items.len(), 1);
        assert_eq!(src, app.state.group(a).unwrap().tab().id);
    }
    #[test]
    fn combined_boxes_tab_management_and_page_actions_are_stable() {
        let mut app = app();
        let x = execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"X"}),
            true,
        )
        .unwrap();
        let y = execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"Y"}),
            true,
        )
        .unwrap();
        let (x, y) = (
            x["group_id"].as_u64().unwrap(),
            y["group_id"].as_u64().unwrap(),
        );
        let tab = app.state.group(x).unwrap().tab().id;
        let expected = app.state.pages.len();
        let page = execute(
            &mut app,
            "desktopcraft_create_page",
            &json!({"name":"Other"}),
            true,
        )
        .unwrap();
        assert_eq!(page["page"], expected);
        execute(
            &mut app,
            "desktopcraft_rename_page",
            &json!({"page":expected,"name":"Personal"}),
            true,
        )
        .unwrap();
        execute(
            &mut app,
            "desktopcraft_move_box_page",
            &json!({"group_id":x,"page":expected}),
            true,
        )
        .unwrap();
        assert_eq!(app.state.group(x).unwrap().page, expected);
        execute(
            &mut app,
            "desktopcraft_merge_boxes",
            &json!({"source_group":x,"target_group":y}),
            true,
        )
        .unwrap();
        execute(
            &mut app,
            "desktopcraft_rename_tab",
            &json!({"group_id":y,"tab_id":tab,"name":"Renamed"}),
            true,
        )
        .unwrap();
        execute(
            &mut app,
            "desktopcraft_activate_tab",
            &json!({"group_id":y,"tab_id":tab}),
            true,
        )
        .unwrap();
        assert_eq!(app.state.group(y).unwrap().tab().name, "Renamed");
        let split = execute(
            &mut app,
            "desktopcraft_detach_tab",
            &json!({"group_id":y,"tab_id":tab}),
            true,
        )
        .unwrap();
        assert_eq!(
            app.state
                .group(split["new_group_id"].as_u64().unwrap())
                .unwrap()
                .tab()
                .id,
            tab
        );
        assert_eq!(app.state.pages[expected], "Personal");
    }
    #[test]
    fn linked_folder_tool_never_moves_or_deletes_any_file() {
        let mut app = app();
        let a = execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"Linked"}),
            true,
        )
        .unwrap();
        let gid = a["group_id"].as_u64().unwrap();
        let tid = app.state.group(gid).unwrap().tab().id;
        let folder = std::env::temp_dir().join(format!(
            "desktopcraft-agent-folder-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&folder).unwrap();
        let f = folder.join("precious.txt");
        fs::write(&f, b"never touch").unwrap();
        execute(
            &mut app,
            "desktopcraft_link_folder",
            &json!({"group_id":gid,"tab_id":tid,"path":folder}),
            true,
        )
        .unwrap();
        assert_eq!(app.state.group(gid).unwrap().tab().items.len(), 1);
        assert!(execute(
            &mut app,
            "desktopcraft_unpin_file",
            &json!({"group_id":gid,"tab_id":tid,"item_id":1}),
            true
        )
        .is_err());
        execute(
            &mut app,
            "desktopcraft_unlink_folder",
            &json!({"group_id":gid,"tab_id":tid}),
            true,
        )
        .unwrap();
        assert!(app.state.group(gid).unwrap().tab().folder.is_none());
        assert_eq!(fs::read(&f).unwrap(), b"never touch");
        fs::remove_file(f).unwrap();
        fs::remove_dir(folder).unwrap();
    }
    /// A box with a regular tab holding fake pins (no disk files involved).
    fn box_with(app: &mut App, name: &str, pins: &[&str]) -> (u64, u64, Vec<u64>) {
        let made = execute(app, "desktopcraft_create_box", &json!({"name":name}), true).unwrap();
        let gid = made["group_id"].as_u64().unwrap();
        let tid = app.state.group(gid).unwrap().tab().id;
        let mut ids = vec![];
        for pin in pins {
            let id = app.state.id();
            app.state
                .group_mut(gid)
                .unwrap()
                .tab_mut()
                .items
                .push(Item {
                    id,
                    name: (*pin).into(),
                    path: format!("C:/Desktop/{pin}"),
                    kind: "lnk".into(),
                    size: 0,
                    modified: 0,
                    label: None,
                });
            ids.push(id);
        }
        (gid, tid, ids)
    }
    #[test]
    fn batch_move_takes_pins_from_several_tabs_in_one_step() {
        let mut app = app();
        let (sg, st, s) = box_with(
            &mut app,
            "Shortcuts",
            &["Steam", "Palworld", "Discord", "Scorn"],
        );
        let (ag, at, a) = box_with(&mut app, "Archives", &["Mega Man X8.exe", "notes.zip"]);
        let (gg, gt, _) = box_with(&mut app, "Games", &[]);
        let undo_before = app.undo.len();
        let ids = json!([s[1], s[3], a[0]]);
        let request = json!({"item_ids":ids,"target_group":gg,"target_tab":gt});
        // The approval names each icon and where it comes from.
        let message = approval_message(&app, "desktopcraft_move_pins", &request);
        assert!(message.contains("move 3 icons into \"Games\""));
        assert!(message.contains("Palworld   (from Shortcuts)"));
        assert!(message.contains("Mega Man X8.exe   (from Archives)"));
        assert!(execute(&mut app, "desktopcraft_move_pins", &request, false).is_err());
        let out = execute(&mut app, "desktopcraft_move_pins", &request, true).unwrap();
        assert_eq!(out["moved"], 3);
        let names = |g: u64, t: u64, app: &App| {
            app.state
                .group(g)
                .unwrap()
                .tabs
                .iter()
                .find(|x| x.id == t)
                .unwrap()
                .items
                .iter()
                .map(|i| i.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(gg, gt, &app),
            ["Palworld", "Scorn", "Mega Man X8.exe"]
        );
        assert_eq!(names(sg, st, &app), ["Steam", "Discord"]);
        assert_eq!(names(ag, at, &app), ["notes.zip"]);
        // One undo step brings every icon back.
        assert_eq!(app.undo.len(), undo_before + 1);
        execute(&mut app, "desktopcraft_undo_layout", &json!({}), true).unwrap();
        assert_eq!(names(sg, st, &app).len(), 4);
        assert!(names(gg, gt, &app).is_empty());
    }
    #[test]
    fn batch_move_is_all_or_nothing_and_checked_before_asking() {
        let mut app = app();
        let (_, _, s) = box_with(&mut app, "Shortcuts", &["Palworld"]);
        let (gg, gt, _) = box_with(&mut app, "Games", &[]);
        let bad = json!({"item_ids":[s[0],999_999],"target_group":gg,"target_tab":gt});
        assert!(precheck(&app, "desktopcraft_move_pins", &bad).is_err());
        assert!(execute(&mut app, "desktopcraft_move_pins", &bad, true).is_err());
        assert!(app.state.group(gg).unwrap().tab().items.is_empty());
        let twice = json!({"item_ids":[s[0],s[0]],"target_group":gg,"target_tab":gt});
        assert!(precheck(&app, "desktopcraft_move_pins", &twice).is_err());
        let empty = json!({"item_ids":[],"target_group":gg,"target_tab":gt});
        assert!(precheck(&app, "desktopcraft_move_pins", &empty).is_err());
        let good = json!({"item_ids":[s[0]],"target_group":gg,"target_tab":gt});
        assert!(precheck(&app, "desktopcraft_move_pins", &good).is_ok());
        // Already in the target: nothing to do, nothing lost.
        execute(&mut app, "desktopcraft_move_pins", &good, true).unwrap();
        let again = execute(&mut app, "desktopcraft_move_pins", &good, true).unwrap();
        assert_eq!(again["moved"], 0);
        assert_eq!(app.state.group(gg).unwrap().tab().items.len(), 1);
    }
    #[test]
    fn single_move_approval_names_the_icon_and_the_target_tab() {
        let mut app = app();
        let (sg, st, s) = box_with(&mut app, "Shortcuts", &["Palworld"]);
        let (gg, _, _) = box_with(&mut app, "Documents", &[]);
        let games = app.state.id();
        app.state
            .group_mut(gg)
            .unwrap()
            .tabs
            .push(crate::model::Tab::new(games, "Games"));
        let message = approval_message(
            &app,
            "desktopcraft_move_pin",
            &json!({"source_group":sg,
            "source_tab":st,"item_id":s[0],"target_group":gg,"target_tab":games}),
        );
        assert!(message.contains("Icon: Palworld"));
        assert!(message.contains("to box: Documents / Games"));
        assert!(message.contains("to tab: Games"));
    }
    #[test]
    fn rejects_folder_targets_and_unknown_ids() {
        let mut app = app();
        let a = execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"A"}),
            true,
        )
        .unwrap();
        let id = a["group_id"].as_u64().unwrap();
        assert!(execute(
            &mut app,
            "desktopcraft_move_pin",
            &json!({"source_group":id,
            "source_tab":999,"item_id":1,"target_group":id,"target_tab":999}),
            true
        )
        .is_err());
        assert!(execute(
            &mut app,
            "desktopcraft_create_box",
            &json!({"name":"\n"}),
            true
        )
        .is_err());
    }
}
