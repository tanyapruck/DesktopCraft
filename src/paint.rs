use crate::model::*;
use macroquad::prelude::*;
pub const TEXT: Color = Color::new(0.94, 0.97, 1.0, 1.0);
pub const MUTED: Color = Color::new(0.68, 0.76, 0.84, 1.0);
pub const ACCENT: Color = Color::new(0.48, 0.79, 0.97, 1.0);
pub const SURFACE: Color = Color::new(0.065, 0.10, 0.15, 0.98);
pub const COLORS: [Color; 6] = [
    Color::new(0.34, 0.68, 0.93, 1.),
    Color::new(0.48, 0.78, 0.68, 1.),
    Color::new(0.65, 0.58, 0.87, 1.),
    Color::new(0.93, 0.69, 0.42, 1.),
    Color::new(0.89, 0.51, 0.61, 1.),
    Color::new(0.49, 0.77, 0.84, 1.),
];
/// A preset group colour as a visible background (dark enough for white text).
pub fn preset_color(i: usize) -> [f32; 3] {
    let c = COLORS[i % 6];
    [c.r * 0.42, c.g * 0.42, c.b * 0.45]
}
/// The colour icons are tinted with for a group: its custom colour at full
/// brightness, or its preset colour.
pub fn tint_color(g: &Group) -> Color {
    match g.custom_color {
        Some(c) => {
            let (h, s, _) = rgb_to_hsv(c);
            let [r, gg, b] = hsv_to_rgb(h, s.max(0.35), 1.0);
            Color::new(r, gg, b, 1.)
        }
        None => COLORS[g.color % 6],
    }
}
pub fn round(b: Bounds, r: f32, c: Color) {
    let r = r.min(b.w / 2.).min(b.h / 2.).max(0.);
    draw_rectangle(b.x + r, b.y, b.w - 2. * r, b.h, c);
    draw_rectangle(b.x, b.y + r, b.w, b.h - 2. * r, c);
    for (x, y) in [
        (b.x + r, b.y + r),
        (b.x + b.w - r, b.y + r),
        (b.x + r, b.y + b.h - r),
        (b.x + b.w - r, b.y + b.h - r),
    ] {
        draw_circle(x, y, r, c);
    }
}
pub fn outline(b: Bounds, color: Color) {
    draw_rectangle_lines(b.x, b.y, b.w, b.h, 1., color)
}
pub fn alpha(mut c: Color, a: f32) -> Color {
    c.a = a;
    c
}
struct IconWorker {
    requests: std::sync::mpsc::Sender<String>,
    results: std::sync::mpsc::Receiver<(String, Option<(u32, u32, Vec<u8>)>)>,
}
impl IconWorker {
    fn start() -> Option<Self> {
        let (requests, receive_requests) = std::sync::mpsc::channel::<String>();
        let (send_results, results) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("desktop-icon-decoder".into())
            .spawn(move || {
                // COM objects must be created and released in their owning apartment.
                crate::shell::init();
                for path in receive_requests {
                    let start = std::time::Instant::now();
                    let pixels = crate::shell::image(&path, 64);
                    crate::perf::record("icon_decode_worker_ms", start.elapsed());
                    if send_results.send((path, pixels)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { requests, results })
    }
}
pub struct Painter {
    pub font: Option<Font>,
    pub mouse: Vec2,
    pub click: bool,
    pub enabled: bool,
    pub width: f32,
    pub height: f32,
    /// Workspace rectangles drawn this frame, with their corner radius. In desktop
    /// mode the window is clipped to them, so everything else stays Explorer's.
    pub regions: std::cell::RefCell<Vec<(Bounds, f32)>>,
    /// Added to recorded regions while drawing in a translated view.
    pub offset: Vec2,
    /// Explorer's own icons and thumbnails, by path (None when unavailable).
    pub icons: std::cell::RefCell<std::collections::HashMap<String, Option<Texture2D>>>,
    wrapped_labels: std::cell::RefCell<
        std::collections::HashMap<
            String,
            std::collections::HashMap<(u32, u32), std::rc::Rc<[String]>>,
        >,
    >,
    label_cache_count: std::cell::Cell<usize>,
    pub icon_budget: std::cell::Cell<u32>,
    pending_icons: std::cell::RefCell<std::collections::HashSet<String>>,
    icon_worker: Option<IconWorker>,
    pub real_icons: bool,
}
impl Painter {
    pub fn new(font: Option<Font>, real_icons: bool) -> Self {
        Self {
            font,
            mouse: vec2(-100., -100.),
            click: false,
            enabled: true,
            width: 1280.,
            height: 800.,
            regions: Default::default(),
            offset: Vec2::ZERO,
            icons: Default::default(),
            wrapped_labels: Default::default(),
            label_cache_count: std::cell::Cell::new(0),
            icon_budget: std::cell::Cell::new(0),
            pending_icons: Default::default(),
            icon_worker: real_icons.then(IconWorker::start).flatten(),
            real_icons,
        }
    }
    pub fn push_region(&self, b: Bounds, radius: f32) {
        self.regions.borrow_mut().push((
            Bounds {
                x: b.x + self.offset.x,
                y: b.y + self.offset.y,
                ..b
            },
            radius,
        ));
    }
    /// Upload a small number of decoded icons on the graphics thread.
    /// The COM worker never touches GPU resources.
    pub fn collect_icons(&self) {
        let Some(worker) = &self.icon_worker else {
            return;
        };
        // GPU texture creation is synchronous. Even with decoding on a worker,
        // uploading four icons in one frame can produce a noticeable hitch.
        // Bound the number of uploads and the cumulative frame time while
        // gradually populating the cache across frames.
        let started = std::time::Instant::now();
        for _ in 0..2 {
            if started.elapsed() >= std::time::Duration::from_millis(4) {
                break;
            }
            let Ok((path, pixels)) = worker.results.try_recv() else {
                break;
            };
            if !self.pending_icons.borrow_mut().remove(&path) {
                continue;
            }
            let start = std::time::Instant::now();
            let texture = pixels.and_then(|(w, h, rgba)| {
                if w > u16::MAX as u32 || h > u16::MAX as u32 {
                    return None;
                }
                let t = Texture2D::from_rgba8(w as u16, h as u16, &rgba);
                t.set_filter(FilterMode::Linear);
                Some(t)
            });
            crate::perf::record("icon_upload_ms", start.elapsed());
            self.icons.borrow_mut().insert(path, texture);
        }
        crate::perf::record("icon_collect_frame_ms", started.elapsed());
    }
    /// Queue icon decoding without waiting for Windows shell extraction.
    fn real_icon(&self, path: &str) -> Option<Texture2D> {
        if !self.real_icons || path.is_empty() {
            return None;
        }
        if let Some(cached) = self.icons.borrow().get(path) {
            return cached.clone();
        }
        if self.pending_icons.borrow().contains(path) || self.icon_budget.get() == 0 {
            return None;
        }
        self.icon_budget.set(self.icon_budget.get() - 1);
        if let Some(worker) = &self.icon_worker {
            let queued = path.to_owned();
            if worker.requests.send(queued.clone()).is_ok() {
                self.pending_icons.borrow_mut().insert(queued);
                return None;
            }
        }
        // If the background worker could not start, retain the original
        // fallback rather than silently losing all icons.
        let start = std::time::Instant::now();
        let texture = crate::shell::image(path, 64).map(|(w, h, rgba)| {
            let t = Texture2D::from_rgba8(w as u16, h as u16, &rgba);
            t.set_filter(FilterMode::Linear);
            t
        });
        crate::perf::record("icon_decode_fallback_ms", start.elapsed());
        self.icons
            .borrow_mut()
            .insert(path.to_owned(), texture.clone());
        texture
    }
    /// Cache label layout across frames. Key includes exact font size and
    /// available width, so DPI changes and renamed files cannot reuse it.
    pub fn wrap_label_cached(&self, label: &str, size: f32, width: f32) -> std::rc::Rc<[String]> {
        // A borrowed &str lookup avoids allocating a String on every cache hit.
        // Rc lets drawing share the wrapped lines without cloning every word.
        let dimensions = (size.to_bits(), width.to_bits());
        if let Some(lines) = self
            .wrapped_labels
            .borrow()
            .get(label)
            .and_then(|sizes| sizes.get(&dimensions))
        {
            return lines.clone();
        }
        let lines: std::rc::Rc<[String]> =
            crate::desktop::wrap_label(self, label, size, width).into();
        let mut cache = self.wrapped_labels.borrow_mut();
        if self.label_cache_count.get() >= 4096 {
            cache.clear();
            self.label_cache_count.set(0);
        }
        cache
            .entry(label.to_owned())
            .or_default()
            .insert(dimensions, lines.clone());
        self.label_cache_count.set(self.label_cache_count.get() + 1);
        lines
    }
    pub fn profile_cache_counts(&self) {
        if !crate::perf::enabled() {
            return;
        }
        crate::perf::record_value("icon_cache_count", self.icons.borrow().len() as f64);
        crate::perf::record_value(
            "icon_pending_count",
            self.pending_icons.borrow().len() as f64,
        );
        crate::perf::record_value("label_cache_count", self.label_cache_count.get() as f64);
    }
    pub fn forget_icon(&self, path: &str) {
        self.pending_icons.borrow_mut().remove(path);
        self.icons.borrow_mut().remove(path);
    }
    pub fn text(&self, s: &str, x: f32, y: f32, size: f32, c: Color) {
        draw_text_ex(
            s,
            x,
            y + size,
            TextParams {
                font: self.font.as_ref(),
                font_size: size as u16,
                color: c,
                ..Default::default()
            },
        );
    }
    pub fn measure(&self, s: &str, size: f32) -> f32 {
        measure_text(s, self.font.as_ref(), size as u16, 1.).width
    }
    pub fn fit(&self, s: &str, size: f32, width: f32) -> String {
        if self.measure(s, size) <= width {
            return s.into();
        }
        // One reusable candidate string; avoid allocating a new formatted
        // string for every measured character of every label, every frame.
        let mut result = String::with_capacity(s.len().saturating_add('…'.len_utf8()));
        for c in s.chars() {
            let before = result.len();
            result.push(c);
            result.push('…');
            let fits = self.measure(&result, size) <= width;
            result.pop();
            if !fits && !crate::desktop::is_combining_mark(c) {
                result.truncate(before);
                break;
            }
        }
        result.push('…');
        result
    }
    /// Like `fit`, but keeps the END of the text ("…xlsx,csv,md"): a text
    /// field being typed into shows where the typing happens.
    pub fn fit_tail(&self, s: &str, size: f32, width: f32) -> String {
        if self.measure(s, size) <= width {
            return s.into();
        }
        let chars: Vec<char> = s.chars().collect();
        // Grow the kept tail one character at a time while "…" + tail fits,
        // never starting on a combining mark (it belongs to the character before).
        let mut keep = 0;
        let mut candidate = String::with_capacity(s.len() + '…'.len_utf8());
        for n in 1..=chars.len() {
            candidate.clear();
            candidate.push('…');
            candidate.extend(&chars[chars.len() - n..]);
            if self.measure(&candidate, size) > width {
                break;
            }
            keep = n;
        }
        while keep > 0 && crate::desktop::is_combining_mark(chars[chars.len() - keep]) {
            keep -= 1;
        }
        let mut result = String::from('…');
        result.extend(&chars[chars.len() - keep..]);
        result
    }
    pub fn label(&self, s: &str, b: Bounds, size: f32, c: Color) {
        let s = self.fit(s, size, b.w - 8.);
        self.text(
            &s,
            b.x + (b.w - self.measure(&s, size)) / 2.,
            b.y + (b.h - size) / 2. - 2.,
            size,
            c,
        )
    }
    pub fn hover(&self, b: Bounds) -> bool {
        self.enabled && b.contains(self.mouse.x, self.mouse.y)
    }
    pub fn button(&self, s: &str, b: Bounds, primary: bool) -> bool {
        let hover = self.hover(b);
        round(
            b,
            6.,
            if primary {
                if hover {
                    Color::new(0.49, 0.80, 1., 1.)
                } else {
                    ACCENT
                }
            } else if hover {
                Color::new(0.22, 0.30, 0.39, 1.)
            } else {
                Color::new(0.14, 0.20, 0.28, 0.96)
            },
        );
        self.label(
            s,
            b,
            15.,
            if primary {
                Color::new(0.05, 0.12, 0.19, 1.)
            } else {
                TEXT
            },
        );
        hover && self.click
    }
    pub fn checkbox(&self, label: &str, b: Bounds, on: bool) -> bool {
        let boxb = Bounds {
            x: b.x,
            y: b.y + 7.,
            w: 20.,
            h: 20.,
        };
        round(
            boxb,
            4.,
            if on {
                ACCENT
            } else {
                Color::new(0.20, 0.28, 0.37, 1.)
            },
        );
        if on {
            draw_line(b.x + 5., b.y + 17., b.x + 9., b.y + 21., 2., SURFACE);
            draw_line(b.x + 9., b.y + 21., b.x + 16., b.y + 12., 2., SURFACE)
        }
        self.text(label, b.x + 32., b.y + 6., 15., TEXT);
        self.hover(b) && self.click
    }
    /// Group pins and loose desktop icons share one rendering path; loose
    /// icons never need an owned Item or cloned file names each frame.
    pub fn icon(&self, item: &Item, x: f32, y: f32, size: f32, tint: Option<Color>) {
        self.icon_path(&item.path, &item.kind, x, y, size, tint);
    }
    pub fn icon_path(
        &self,
        path: &str,
        kind: &str,
        x: f32,
        y: f32,
        size: f32,
        tint: Option<Color>,
    ) {
        let fade = tint.map_or(1.0, |t| t.a);
        let tinted = tint.filter(|t| !(t.r >= 0.999 && t.g >= 0.999 && t.b >= 0.999));
        if let Some(texture) = self.real_icon(path) {
            let (w, h) = (texture.width(), texture.height());
            let k = size / w.max(h);
            // Tinting shifts the icon toward the group colour, keeping its shape.
            let color = match tinted {
                Some(t) => Color::new(
                    0.35 + 0.65 * t.r,
                    0.35 + 0.65 * t.g,
                    0.35 + 0.65 * t.b,
                    fade,
                ),
                None => Color::new(1., 1., 1., fade),
            };
            draw_texture_ex(
                &texture,
                x + (size - w * k) / 2.,
                y + (size - h * k) / 2.,
                color,
                DrawTextureParams {
                    dest_size: Some(vec2(w * k, h * k)),
                    ..Default::default()
                },
            );
            return;
        }
        // Borrowing the raw path extension avoids allocating a lowercase
        // String for loose desktop icons; compare common extensions without
        // introducing case-sensitive differences in fallback rendering.
        let kind_is = |expected: &str| kind.eq_ignore_ascii_case(expected);
        let mut c = tinted.unwrap_or_else(|| {
            if kind_is("folder") {
                Color::new(0.95, 0.73, 0.33, 1.)
            } else if ["csv", "xlsx", "xls"].iter().any(|x| kind_is(x)) {
                COLORS[1]
            } else if ["png", "jpg", "jpeg", "svg", "webp"]
                .iter()
                .any(|x| kind_is(x))
            {
                COLORS[2]
            } else if kind_is("pdf") {
                COLORS[4]
            } else if ["exe", "lnk", "url"].iter().any(|x| kind_is(x)) {
                COLORS[5]
            } else {
                COLORS[0]
            }
        });
        c.a = fade;
        if kind_is("folder") {
            round(
                Bounds {
                    x,
                    y: y + size * 0.13,
                    w: size * 0.45,
                    h: size * 0.35,
                },
                3.,
                alpha(c, c.a * 0.75),
            );
            round(
                Bounds {
                    x,
                    y: y + size * 0.25,
                    w: size,
                    h: size * 0.64,
                },
                4.,
                alpha(c, c.a * 0.75),
            );
            c.r = (c.r * 1.04).min(1.);
            round(
                Bounds {
                    x: x - 1.,
                    y: y + size * 0.37,
                    w: size + 2.,
                    h: size * 0.53,
                },
                4.,
                c,
            );
        } else {
            round(
                Bounds {
                    x: x + size * 0.12,
                    y: y + 2.,
                    w: size * 0.73,
                    h: size * 0.94,
                },
                4.,
                alpha(c, 0.18 * c.a),
            );
            let doc = Bounds {
                x: x + size * 0.14,
                y: y + 2.,
                w: size * 0.70,
                h: size * 0.92,
            };
            draw_rectangle_lines(doc.x, doc.y, doc.w, doc.h, 1.6, c);
            draw_line(
                x + size * 0.59,
                y + 2.,
                x + size * 0.59,
                y + size * 0.26,
                1.5,
                c,
            );
            draw_line(
                x + size * 0.59,
                y + size * 0.26,
                x + size * 0.84,
                y + size * 0.26,
                1.5,
                c,
            );
            let ext = if kind.is_empty() {
                "FILE".to_string()
            } else {
                kind.chars().take(4).collect::<String>().to_uppercase()
            };
            self.label(
                &ext,
                Bounds {
                    x: x - 4.,
                    y: y + size * 0.40,
                    w: size + 8.,
                    h: 18.,
                },
                (size * 0.24).max(9.),
                c,
            );
        }
    }
    pub fn wallpaper(&self, theme: usize) {
        let w = self.width;
        let h = self.height;
        let (top, bottom) = match theme % 3 {
            1 => (
                Color::new(0.17, 0.10, 0.23, 1.),
                Color::new(0.39, 0.26, 0.32, 1.),
            ),
            2 => (
                Color::new(0.05, 0.16, 0.16, 1.),
                Color::new(0.21, 0.39, 0.35, 1.),
            ),
            _ => (
                Color::new(0.055, 0.13, 0.23, 1.),
                Color::new(0.21, 0.35, 0.46, 1.),
            ),
        };
        for y in (0..h as i32).step_by(4) {
            let t = y as f32 / h;
            draw_rectangle(
                0.,
                y as f32,
                w,
                4.,
                Color::new(
                    top.r + (bottom.r - top.r) * t,
                    top.g + (bottom.g - top.g) * t,
                    top.b + (bottom.b - top.b) * t,
                    1.,
                ),
            );
        }
        // A neutral, procedurally drawn landscape. No original art or brand assets.
        for layer in 0..5 {
            let base = h * (0.50 + layer as f32 * 0.10);
            let c = Color::new(
                top.r * 0.8 + layer as f32 * 0.007,
                top.g * 0.85 + layer as f32 * 0.01,
                top.b * 0.9 + layer as f32 * 0.01,
                1.,
            );
            for x in (0..w as i32).step_by(8) {
                let ridge = |x: f32| {
                    base + (x * 0.003 + layer as f32 * 1.2).sin() * h * 0.12
                        + (x * 0.008 + layer as f32 * 2.).sin() * h * 0.025
                };
                let a = vec2(x as f32, ridge(x as f32));
                let b = vec2(x as f32 + 8., ridge(x as f32 + 8.));
                draw_triangle(a, b, vec2(x as f32, h), c);
                draw_triangle(b, vec2(x as f32 + 8., h), vec2(x as f32, h), c);
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Header(u64),
    Tab(u64, usize),
    TabNext(u64),
    Roll(u64),
    Menu(u64),
    /// Resize handle: group id and edges (1 left, 2 right, 4 top, 8 bottom).
    Resize(u64, u8),
    Item(u64, u64),
    Back(u64),
    Open(u64),
    /// Folder bar: the filter (magnifier) button, the filter text field, and
    /// the sort button.
    FolderFilter(u64),
    FilterField(u64),
    FolderSort(u64),
    Body(u64),
    /// A desktop icon outside any group (desktop mode), by index.
    Loose(usize),
    Scroll {
        group: u64,
        track_y: f32,
        track_h: f32,
        thumb_y: f32,
        thumb_h: f32,
        total: usize,
        page: usize,
        cols: usize,
    },
}
#[derive(Clone)]
pub struct Hit {
    pub bounds: Bounds,
    pub target: Target,
}
#[derive(Default)]
pub struct DrawStats {
    pub headers: usize,
    pub items: usize,
    pub status: bool,
    pub scrollbars: usize,
}
/// Settings that change how every group looks and behaves.
pub struct GroupStyle {
    pub titlebar: u8,
    pub hide_scrollbars: bool,
    pub rollup: bool,
    /// The rolled group currently opened (after the delay, or by a click).
    pub revealed: Option<u64>,
    /// The Folder View whose filter is receiving typed text.
    pub filter_focus: Option<u64>,
}
pub fn group(
    p: &Painter,
    g: &Group,
    selected: Option<(u64, u64)>,
    multi_selected: &[(u64, u64, u64)],
    hidden: bool,
    query: &str,
    peek: bool,
    blend: bool,
    tint: bool,
    style: &GroupStyle,
    hits: &mut Vec<Hit>,
    stats: &mut DrawStats,
) {
    if hidden && !g.keep_visible && !peek {
        return;
    }
    let b = g.bounds;
    let hovered = b.contains(p.mouse.x, p.mouse.y);
    let selected_group = selected.is_some_and(|(id, _)| id == g.id);
    // Roll-up (manager "Rollup"): a rolled group opens on mouseover at once,
    // after the delay, or on click, as chosen.
    let rolled = g.rolled && style.rollup;
    // Opening is gated to title/tab hover by App::update_rollup_reveal.
    // Keep the expanded body interactive until the pointer leaves the box.
    let revealed = style.revealed == Some(g.id);
    let expanded = !rolled || revealed;
    let h = if expanded { b.h } else { HEADER };
    let col = COLORS[g.color % 6];
    let fade = if blend && !hovered && !peek { 0.42 } else { 1. };
    p.push_region(Bounds { h, ..b }, GROUP_CORNER_RADIUS);
    // Translucent custom-color panel with a subtle title band and border.
    let panel = match g.custom_color {
        Some([r, gg, bb]) => Color::new(r, gg, bb, g.opacity.min(0.95) * fade),
        None => Color::new(
            col.r * 0.07,
            col.g * 0.07,
            col.b * 0.08,
            (g.opacity * 0.86).min(0.95) * fade,
        ),
    };
    round(Bounds { h, ..b }, GROUP_CORNER_RADIUS, panel);
    // Titlebars (manager "Titlebars"): always, on mouseover, or never.
    let show_title = match style.titlebar {
        0 => true,
        1 => hovered || selected_group,
        _ => false,
    };
    let band = Color::new(0., 0., 0., if show_title { 0.20 } else { 0. } * fade);
    round(Bounds { h: HEADER, ..b }, GROUP_CORNER_RADIUS, band);
    if expanded && show_title {
        draw_rectangle(
            b.x,
            b.y + HEADER - GROUP_CORNER_RADIUS,
            b.w,
            GROUP_CORNER_RADIUS,
            band,
        );
        draw_line(
            b.x + 10.,
            b.y + HEADER,
            b.x + b.w - 10.,
            b.y + HEADER,
            1.,
            alpha(WHITE, 0.07 * fade),
        );
    }
    let fade_title = if show_title { fade } else { 0. };
    draw_rectangle_lines(
        b.x,
        b.y,
        b.w,
        h,
        1.,
        alpha(
            WHITE,
            if selected_group {
                0.30
            } else if hovered {
                0.16
            } else {
                0.08
            } * fade,
        ),
    );
    hits.push(Hit {
        bounds: Bounds {
            x: b.x,
            y: b.y,
            w: b.w,
            h,
        },
        target: Target::Body(g.id),
    });
    hits.push(Hit {
        bounds: Bounds {
            x: b.x,
            y: b.y,
            w: b.w - 82.,
            h: HEADER,
        },
        target: Target::Header(g.id),
    });
    stats.headers += 1;
    let tab = g.tab();
    if g.tabs.len() > 1 {
        let (visible_tabs, next) = g.header_tabs();
        for (i, tb) in visible_tabs {
            let t = &g.tabs[i];
            if i == g.active {
                round(tb, 4., alpha(WHITE, 0.12 * fade_title));
            }
            p.label(&t.name, tb, 14., alpha(TEXT, fade_title));
            hits.push(Hit {
                bounds: tb,
                target: Target::Tab(g.id, i),
            });
        }
        if let Some(next) = next {
            if p.hover(next) {
                round(next, 4., alpha(WHITE, 0.12));
            }
            p.text(">", next.x + 7., next.y + 5., 15., alpha(TEXT, fade));
            hits.push(Hit {
                bounds: next,
                target: Target::TabNext(g.id),
            });
        }
    } else {
        // Centered title, like the original's fence label.
        // Never draw a long title under roll-up / ellipsis controls.
        let title_area = (b.w - 104.).max(24.);
        let title = p.fit(&tab.name, 15., title_area);
        let tw = p.measure(&title, 15.);
        let tx = b.x + 8. + (title_area - tw).max(0.) / 2.;
        if show_title {
            p.text(
                &title,
                tx + 1.,
                b.y + 12.,
                15.,
                Color::new(0., 0., 0., 0.6 * fade),
            );
            p.text(&title, tx, b.y + 11., 15., alpha(WHITE, fade));
        }
    }
    let roll = Bounds {
        x: b.x + b.w - 75.,
        y: b.y + 5.,
        w: 32.,
        h: 32.,
    };
    let menu = Bounds {
        x: b.x + b.w - 38.,
        y: b.y + 5.,
        w: 32.,
        h: 32.,
    };
    // Header controls appear while the pointer is over the group (they still
    // respond to clicks otherwise).
    let controls = hovered || selected_group;
    if controls {
        round(
            Bounds {
                x: roll.x - 4.,
                w: b.x + b.w - roll.x + 2.,
                ..roll
            },
            5.,
            Color::new(0., 0., 0., 0.35 * fade),
        );
    }
    if p.hover(roll) {
        round(roll, 5., alpha(WHITE, 0.12));
    }
    if p.hover(menu) {
        round(menu, 5., alpha(WHITE, 0.12));
    }
    let bfade = if controls { fade } else { 0. };
    // Roll-up turned off in Settings: no roll button.
    let rfade = if style.rollup { bfade } else { 0. };
    let cy = roll.y + 16.;
    let cx = roll.x + 16.;
    let d = if g.rolled { 1. } else { -1. };
    let bfade_menu = bfade;
    let bfade = rfade;
    draw_line(
        cx - 4.,
        cy - d * 2.,
        cx,
        cy + d * 2.,
        1.5,
        alpha(TEXT, bfade),
    );
    draw_line(
        cx,
        cy + d * 2.,
        cx + 4.,
        cy - d * 2.,
        1.5,
        alpha(TEXT, bfade),
    );
    for i in 0..3 {
        draw_circle(
            menu.x + 10. + i as f32 * 5.,
            menu.y + 16.,
            1.3,
            alpha(TEXT, bfade_menu),
        );
    }
    if style.rollup {
        hits.push(Hit {
            bounds: roll,
            target: Target::Roll(g.id),
        });
    }
    hits.push(Hit {
        bounds: menu,
        target: Target::Menu(g.id),
    });
    if !expanded {
        return;
    }
    let folder = tab.folder.is_some();
    let content_y = b.y + HEADER + if folder { 38. } else { 12. };
    if let Some(path) = &tab.folder {
        // Vector arrows avoid missing-glyph squares on some Windows fonts.
        let arrow_x = b.x + 20.;
        let arrow_y = b.y + HEADER + 17.;
        if tab.history.is_empty() {
            draw_line(
                arrow_x,
                arrow_y + 7.,
                arrow_x,
                arrow_y - 6.,
                1.8,
                alpha(MUTED, fade),
            );
            draw_line(
                arrow_x - 5.,
                arrow_y - 1.,
                arrow_x,
                arrow_y - 6.,
                1.8,
                alpha(MUTED, fade),
            );
            draw_line(
                arrow_x + 5.,
                arrow_y - 1.,
                arrow_x,
                arrow_y - 6.,
                1.8,
                alpha(MUTED, fade),
            );
        } else {
            draw_line(
                arrow_x + 6.,
                arrow_y,
                arrow_x - 6.,
                arrow_y,
                1.8,
                alpha(MUTED, fade),
            );
            draw_line(
                arrow_x - 1.,
                arrow_y - 5.,
                arrow_x - 6.,
                arrow_y,
                1.8,
                alpha(MUTED, fade),
            );
            draw_line(
                arrow_x - 1.,
                arrow_y + 5.,
                arrow_x - 6.,
                arrow_y,
                1.8,
                alpha(MUTED, fade),
            );
        }
        let display = std::path::Path::new(path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        // Filter and sort sit on the same row as the open-folder link, when
        // the box is wide enough for them.
        let tools = b.w >= 150.;
        let filtering = style.filter_focus == Some(g.id) || !tab.filter.is_empty();
        let text_w = if tools { b.w - 150. } else { b.w - 90. };
        if tools && filtering {
            let field = Bounds {
                x: b.x + 36.,
                y: b.y + HEADER + 5.,
                w: (b.w - 146.).max(20.),
                h: 26.,
            };
            let focused = style.filter_focus == Some(g.id);
            round(field, 5., Color::new(0.04, 0.075, 0.12, 0.92 * fade));
            outline(
                field,
                if focused {
                    ACCENT
                } else {
                    Color::new(0.24, 0.31, 0.40, fade)
                },
            );
            let shown = if tab.filter.is_empty() {
                "Filter: name or .ext"
            } else {
                tab.filter.as_str()
            };
            let text = p.fit(shown, 12., field.w - 14.);
            p.text(
                &text,
                field.x + 7.,
                field.y + 6.,
                12.,
                alpha(if tab.filter.is_empty() { MUTED } else { TEXT }, fade),
            );
            if focused {
                let cx = if tab.filter.is_empty() {
                    field.x + 7.
                } else {
                    (field.x + 8. + p.measure(&text, 12.)).min(field.x + field.w - 4.)
                };
                draw_line(cx, field.y + 5., cx, field.y + 21., 1., ACCENT);
            }
            hits.push(Hit {
                bounds: field,
                target: Target::FilterField(g.id),
            });
        } else {
            p.text(
                &p.fit(&format!("Folder / {display}"), 12., text_w),
                b.x + 38.,
                b.y + HEADER + 10.,
                12.,
                alpha(MUTED, fade),
            );
        }
        if tools {
            // Filter: a magnifier, lit while a filter is in use.
            let fb = Bounds {
                x: b.x + b.w - 105.,
                y: b.y + HEADER + 2.,
                w: 32.,
                h: 32.,
            };
            if filtering {
                round(fb, 5., alpha(ACCENT, 0.22 * fade));
            } else if p.hover(fb) {
                round(fb, 5., alpha(WHITE, 0.12));
            }
            let colour = alpha(if filtering { ACCENT } else { MUTED }, fade);
            draw_circle_lines(fb.x + 14., fb.y + 14., 5.5, 1.6, colour);
            draw_line(fb.x + 18., fb.y + 18., fb.x + 23., fb.y + 23., 1.8, colour);
            hits.push(Hit {
                bounds: fb,
                target: Target::FolderFilter(g.id),
            });
            // Sort: three bars getting shorter, with a small down arrow.
            let sb = Bounds {
                x: b.x + b.w - 71.,
                y: b.y + HEADER + 2.,
                w: 32.,
                h: 32.,
            };
            if p.hover(sb) {
                round(sb, 5., alpha(WHITE, 0.12));
            }
            let muted = alpha(MUTED, fade);
            for (i, len) in [12., 9., 6.].iter().enumerate() {
                let y = sb.y + 10. + i as f32 * 5.;
                draw_line(sb.x + 7., y, sb.x + 7. + len, y, 1.6, muted);
            }
            draw_line(sb.x + 24., sb.y + 9., sb.x + 24., sb.y + 22., 1.6, muted);
            draw_line(sb.x + 20.5, sb.y + 18.5, sb.x + 24., sb.y + 22., 1.6, muted);
            draw_line(sb.x + 27.5, sb.y + 18.5, sb.x + 24., sb.y + 22., 1.6, muted);
            hits.push(Hit {
                bounds: sb,
                target: Target::FolderSort(g.id),
            });
        }
        let ox = b.x + b.w - 29.;
        let oy = b.y + HEADER + 12.;
        draw_line(ox, oy + 12., ox + 12., oy, 1.8, alpha(MUTED, fade));
        draw_line(ox + 4., oy, ox + 12., oy, 1.8, alpha(MUTED, fade));
        draw_line(ox + 12., oy, ox + 12., oy + 8., 1.8, alpha(MUTED, fade));
        hits.push(Hit {
            bounds: Bounds {
                x: b.x + 4.,
                y: b.y + HEADER + 2.,
                w: 30.,
                h: 32.,
            },
            target: Target::Back(g.id),
        });
        hits.push(Hit {
            bounds: Bounds {
                x: b.x + b.w - 37.,
                y: b.y + HEADER + 2.,
                w: 32.,
                h: 32.,
            },
            target: Target::Open(g.id),
        });
    }
    let items: Vec<_> = tab
        .items
        .iter()
        .filter(|i| query.is_empty() || i.name.to_lowercase().contains(&query.to_lowercase()))
        .filter(|i| tab.folder.is_none() || crate::model::filter_matches(&i.name, &tab.filter))
        .collect();
    if items.is_empty() && tab.folder.is_some() && !tab.filter.trim().is_empty() {
        p.text(
            &p.fit("No files match this filter", 12., b.w - 28.),
            b.x + 14.,
            content_y + 8.,
            12.,
            alpha(MUTED, fade),
        );
    }
    let bottom = b.y + b.h - FOOTER;
    let available = (bottom - content_y).max(0.);
    let cw = cell_w() * tab.icon_scale;
    let ch = cell_h() * tab.icon_scale;
    let cols = if tab.list {
        1
    } else {
        ((b.w - 20.) / cw).floor().max(1.) as usize
    };
    let rows = if tab.list {
        (available / 36.).floor() as usize
    } else {
        (available / ch).floor().max(1.) as usize
    };
    let count = rows * cols;
    let last_row = items.len().div_ceil(cols).saturating_sub(rows) * cols;
    let start = tab.scroll.min(last_row);
    for (index, item) in items.iter().skip(start).take(count).enumerate() {
        let cell = if tab.list {
            Bounds {
                x: b.x + 10.,
                y: content_y + index as f32 * 36.,
                w: b.w - 20.,
                h: 34.,
            }
        } else {
            Bounds {
                x: b.x + 10. + (index % cols) as f32 * (b.w - 20.) / cols as f32,
                y: content_y + (index / cols) as f32 * ch,
                w: (b.w - 20.) / cols as f32,
                h: ch,
            }
        };
        if cell.y + cell.h > bottom + 3. {
            continue;
        }
        // Explorer-style highlight: translucent blue when selected, faint on hover.
        let shape = Bounds {
            x: cell.x + 2.,
            y: cell.y + 1.,
            w: cell.w - 4.,
            h: cell.h - 2.,
        };
        if multi_selected.contains(&(g.id, tab.id, item.id))
            || (multi_selected.is_empty() && selected == Some((g.id, item.id)))
        {
            round(shape, 3., Color::new(0.35, 0.6, 0.95, 0.35 * fade));
            outline(shape, Color::new(0.6, 0.8, 1.0, 0.7 * fade));
        } else if p.hover(cell) {
            round(shape, 3., alpha(WHITE, 0.12));
        }
        // Tint with the group's colour; a white "tint" only fades the icon.
        let ic = if tint {
            Some(alpha(tint_color(g), fade))
        } else if fade < 1. {
            Some(Color::new(1., 1., 1., fade))
        } else {
            None
        };
        if tab.list {
            p.icon(item, cell.x + 5., cell.y + 3., 26., ic);
            p.text(
                &p.fit(item.display(), 14., cell.w - 135.),
                cell.x + 42.,
                cell.y + 6.,
                14.,
                alpha(TEXT, fade),
            );
            let detail = if item.kind == "folder" {
                "Folder".into()
            } else {
                format!("{} KB", item.size.div_ceil(1024))
            };
            p.text(
                &detail,
                cell.x + cell.w - 78.,
                cell.y + 8.,
                11.,
                alpha(MUTED, fade),
            );
        } else {
            // Same size and label style as desktop icons: 48 px (medium icons),
            // white label with a dark shadow, wrapped to two lines.
            let size = (48. * tab.icon_scale)
                .min(cell.w - 12.)
                .min(cell.h - 36.)
                .max(16.);
            p.icon(item, cell.x + (cell.w - size) / 2., cell.y + 4., size, ic);
            for (n, line) in p
                .wrap_label_cached(item.display(), 12., cell.w - 6.)
                .iter()
                .enumerate()
            {
                let w = p.measure(line, 12.);
                let (x, y) = (
                    cell.x + (cell.w - w) / 2.,
                    cell.y + size + 8. + n as f32 * 15.,
                );
                p.text(
                    line,
                    x + 1.,
                    y + 1.,
                    12.,
                    Color::new(0., 0., 0., 0.85 * fade),
                );
                p.text(line, x, y, 12., alpha(WHITE, fade));
            }
        }
        hits.push(Hit {
            bounds: cell,
            target: Target::Item(g.id, item.id),
        });
        stats.items += 1;
    }
    if items.is_empty() {
        p.label(
            if query.is_empty() {
                if folder {
                    "This folder is empty"
                } else {
                    "Drop files here"
                }
            } else {
                "No matching files"
            },
            Bounds {
                x: b.x + 10.,
                y: content_y + 10.,
                w: b.w - 20.,
                h: 45.,
            },
            14.,
            alpha(MUTED, fade),
        );
    }
    // No item-count footer (the original shows none); the count is in the
    // title's tooltip-free design, so nothing is drawn here.
    if items.len() > count && count > 0 {
        // Draw a low-contrast track and a stronger draggable thumb.
        // Inactive bars are subtle until the pointer hovers nearby.
        let total_rows = items.len().div_ceil(cols);
        let page_rows = (count / cols).max(1);
        let track = Bounds {
            x: b.x + b.w - 4. - SCROLL_BAR_WIDTH,
            y: content_y,
            w: SCROLL_BAR_WIDTH,
            h: (bottom - content_y).max(0.),
        };
        let (start_y, len) = scroll_thumb(total_rows, page_rows, start / cols, track.h);
        let thumb = Bounds {
            y: track.y + start_y,
            h: len,
            ..track
        };
        let hit = Bounds {
            x: track.x + track.w - SCROLL_HIT_WIDTH + 2.,
            w: SCROLL_HIT_WIDTH,
            ..track
        };
        let over_bar = p.hover(hit);
        let over_thumb = over_bar && p.hover(thumb);
        let rest = if over_bar { 1.0 } else { SCROLL_REST_OPACITY };
        let base = Color::new(
            0.07 + col.r * 0.10,
            0.11 + col.g * 0.09,
            0.17 + col.b * 0.09,
            1.,
        );
        // Personalization "Hide scrollbars until mouseover".
        let shown = if style.hide_scrollbars && !hovered {
            0.
        } else {
            1.
        };
        round(
            track,
            2.,
            alpha(
                base,
                if over_bar { 0.35 } else { 0.30 } * rest * fade * shown,
            ),
        );
        round(
            thumb,
            2.,
            alpha(
                TEXT,
                if over_bar && !over_thumb { 0.85 } else { 1.0 } * rest * fade * shown,
            ),
        );
        stats.scrollbars += 1;
        hits.push(Hit {
            bounds: hit,
            target: Target::Scroll {
                group: g.id,
                track_y: track.y,
                track_h: track.h,
                thumb_y: thumb.y,
                thumb_h: thumb.h,
                total: total_rows,
                page: page_rows,
                cols,
            },
        });
    }
    if !g.locked {
        // The resize grip only shows while the pointer is over the group.
        for i in 0..3 {
            // Reveal the resize hint only when approaching the corner itself.
            if !(p.mouse.x >= b.x + b.w - 48.
                && p.mouse.x <= b.x + b.w + 8.
                && p.mouse.y >= b.y + b.h - 48.
                && p.mouse.y <= b.y + b.h + 8.)
            {
                break;
            }
            let offset = i as f32 * 4.;
            draw_line(
                b.x + b.w - 6. - offset,
                b.y + b.h - 5.,
                b.x + b.w - 5.,
                b.y + b.h - 6. - offset,
                1.,
                alpha(MUTED, fade * 0.6),
            );
        }
        // Resize from every edge and corner, like the original ("click & drag the
        // edge"): 5 px edge strips, 14 px corners (the lower-right corner keeps
        // its larger 20 px handle). Pushed last so they win at the border.
        let e = 5.;
        let c = 14.;
        for (bounds, edges) in [
            (
                Bounds {
                    x: b.x,
                    y: b.y + c,
                    w: e,
                    h: b.h - 2. * c,
                },
                1u8,
            ),
            (
                Bounds {
                    x: b.x + b.w - e,
                    y: b.y + c,
                    w: e,
                    h: b.h - 2. * c,
                },
                2,
            ),
            (
                Bounds {
                    x: b.x + c,
                    y: b.y,
                    w: b.w - 2. * c,
                    h: 4.,
                },
                4,
            ),
            (
                Bounds {
                    x: b.x + c,
                    y: b.y + b.h - e,
                    w: b.w - 2. * c,
                    h: e,
                },
                8,
            ),
            (
                Bounds {
                    x: b.x,
                    y: b.y,
                    w: c,
                    h: c,
                },
                1 | 4,
            ),
            (
                Bounds {
                    x: b.x + b.w - c,
                    y: b.y,
                    w: c,
                    h: c,
                },
                2 | 4,
            ),
            (
                Bounds {
                    x: b.x,
                    y: b.y + b.h - c,
                    w: c,
                    h: c,
                },
                1 | 8,
            ),
            (
                Bounds {
                    x: b.x + b.w - 18.,
                    y: b.y + b.h - 18.,
                    w: 20.,
                    h: 20.,
                },
                2 | 8,
            ),
        ] {
            hits.push(Hit {
                bounds,
                target: Target::Resize(g.id, edges),
            });
        }
    }
}

/// The cursor that shows which way a resize handle moves.
pub fn resize_cursor(edges: u8) -> macroquad::miniquad::CursorIcon {
    use macroquad::miniquad::CursorIcon;
    match edges {
        1 | 2 => CursorIcon::EWResize,
        4 | 8 => CursorIcon::NSResize,
        5 | 10 => CursorIcon::NWSEResize,
        _ => CursorIcon::NESWResize,
    }
}
