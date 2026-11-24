//! The page canvas: layout, tiled painting, scrolling, zooming, selection and links.

use crate::config::Sizing;
use crate::doc::text::SelectionStyle;
use crate::doc::{Document, Link, LinkTarget, PageText, RectF};
use crate::find::Search;
use crate::render::{RenderPool, TextWorker, TileKey, TileKind};
use crate::theme::Colors;
use egui::{
    Color32, ColorImage, CornerRadius, CursorIcon, Id, Mesh, PointerButton, Pos2, Rect, Response, Sense, Shape,
    Stroke, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Logical pixels per point at 100% zoom.
pub const BASE: f32 = 96.0 / 72.0;
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 64.0;
pub const ZOOM_STEPS: &[f32] = &[
    0.25, 0.333, 0.5, 0.667, 0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0,
];
const TILE: u32 = 512;
const MAX_WHOLE_PAGE_PIXELS: u32 = 1800 * 1800;
const MAX_TEXTURE_SIDE: f32 = 8192.0;
const PREVIEW_SIDE: f32 = 1400.0;
const TEXTURE_BUDGET: usize = 600 << 20;
const UPLOADS_PER_FRAME: usize = 24;

struct Cached {
    tex: TextureHandle,
    bytes: usize,
    used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelPoint {
    pub page: usize,
    /// Unrotated page coordinates in points.
    pub pt: Pos2,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub a: SelPoint,
    pub b: SelPoint,
    pub style: SelectionStyle,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub page: usize,
    /// How far into the page the top of the window is, as a fraction of its height.
    pub fraction: f32,
}

#[derive(Clone, Copy, Debug)]
enum Target {
    Page { page: usize, y: Option<f32> },
    Position(Position),
    Rect { page: usize, rect: RectF },
}

#[derive(Clone, Copy, Debug)]
struct Anchor {
    page: usize,
    /// Unrotated page point that must stay under `screen`.
    pt: Pos2,
    screen: Pos2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    Select,
    Pan,
}

#[derive(Default)]
struct Layout {
    scale: f32,
    rects: Vec<Rect>,
    size: Vec2,
    rows: Vec<Vec<usize>>,
}

pub struct View {
    pub doc: Arc<dyn Document>,
    pool: RenderPool,
    pub text: TextWorker,
    textures: HashMap<TileKey, Cached>,
    texture_bytes: usize,
    frame: u64,
    sizes: Vec<(f32, f32)>,

    pub continuous: bool,
    pub dual: bool,
    pub odd_left: bool,
    pub sizing: Sizing,
    pub zoom: f32,
    pub rotation: u32,
    pub invert: bool,
    pub page_gap: f32,

    offset: Vec2,
    viewport: Rect,
    layout: Layout,
    pub current_page: usize,
    target: Option<Target>,
    anchor: Option<Anchor>,
    drag: Drag,
    flip_acc: f32,

    pub selection: Option<Selection>,
    pub search: Option<Search>,
    links: HashMap<usize, Arc<Vec<Link>>>,
    hover_link: Option<Link>,
    /// Link under the pointer when the context menu was opened.
    pub context_link: Option<Link>,
    pub context_point: Option<SelPoint>,
    /// A URI the user clicked; the app opens it.
    pub open_uri: Option<String>,

    back: Vec<Position>,
    forward: Vec<Position>,

    thumb_wanted: Vec<TileKey>,
    pub presentation_blank: Option<Color32>,
}

fn quant(scale: f32) -> u32 {
    (scale * 1000.0).round().max(1.0) as u32
}

impl View {
    pub fn new(doc: Arc<dyn Document>, ctx: &egui::Context, threads: usize) -> View {
        let sizes = (0..doc.page_count()).map(|i| doc.page_size(i)).collect();
        View {
            pool: RenderPool::new(doc.clone(), threads, ctx.clone()),
            text: TextWorker::new(doc.clone(), ctx.clone()),
            doc,
            textures: HashMap::new(),
            texture_bytes: 0,
            frame: 0,
            sizes,
            continuous: true,
            dual: false,
            odd_left: false,
            sizing: Sizing::Automatic,
            zoom: 1.0,
            rotation: 0,
            invert: false,
            page_gap: 14.0,
            offset: Vec2::ZERO,
            viewport: Rect::NOTHING,
            layout: Layout::default(),
            current_page: 0,
            target: None,
            anchor: None,
            drag: Drag::None,
            flip_acc: 0.0,
            selection: None,
            search: None,
            links: HashMap::new(),
            hover_link: None,
            context_link: None,
            context_point: None,
            open_uri: None,
            back: Vec::new(),
            forward: Vec::new(),
            thumb_wanted: Vec::new(),
            presentation_blank: None,
        }
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    pub fn page_size(&self, page: usize) -> (f32, f32) {
        self.sizes[page]
    }

    fn rotated_size(&self, page: usize) -> (f32, f32) {
        let (w, h) = self.sizes[page];
        if self.rotation % 180 == 90 { (h, w) } else { (w, h) }
    }

    /// The effective zoom (1.0 = 100%), whatever the sizing mode.
    pub fn effective_zoom(&self) -> f32 {
        if self.layout.scale > 0.0 { self.layout.scale / BASE } else { self.zoom }
    }

    // ---- layout ----

    fn rows(&self) -> Vec<Vec<usize>> {
        let n = self.sizes.len();
        if !self.dual {
            return (0..n).map(|i| vec![i]).collect();
        }
        let mut rows = Vec::new();
        let mut i = 0;
        if !self.odd_left && n > 0 {
            rows.push(vec![0]);
            i = 1;
        }
        while i < n {
            if i + 1 < n {
                rows.push(vec![i, i + 1]);
            } else {
                rows.push(vec![i]);
            }
            i += 2;
        }
        rows
    }

    fn fit_scales(&self, rows: &[Vec<usize>], vw: f32, vh: f32) -> (f32, f32) {
        let gap = self.page_gap;
        let mut fit_w = f32::MAX;
        let mut fit_h = f32::MAX;
        let candidates: Vec<&Vec<usize>> = if self.continuous {
            rows.iter().collect()
        } else {
            rows.iter().filter(|r| r.contains(&self.current_page)).collect()
        };
        let cols = if self.dual { 2.0 } else { 1.0 };
        for r in candidates {
            let w: f32 = r.iter().map(|&p| self.rotated_size(p).0).sum::<f32>();
            let w = if self.dual && r.len() == 1 { w * 2.0 } else { w };
            let h = r.iter().map(|&p| self.rotated_size(p).1).fold(0.0, f32::max);
            fit_w = fit_w.min((vw - gap * (cols + 1.0)).max(20.0) / w.max(1.0));
            fit_h = fit_h.min((vh - gap * 2.0).max(20.0) / h.max(1.0));
        }
        if fit_w == f32::MAX {
            fit_w = BASE;
            fit_h = BASE;
        }
        (fit_w, fit_h)
    }

    fn compute_layout(&mut self, vw: f32, vh: f32) {
        let rows = self.rows();
        let (fit_w, fit_h) = self.fit_scales(&rows, vw, vh);
        let scale = match self.sizing {
            Sizing::FitWidth => fit_w,
            Sizing::FitPage => fit_w.min(fit_h),
            Sizing::Automatic => fit_w.min(BASE),
            Sizing::Free => self.zoom * BASE,
        }
        .clamp(MIN_ZOOM * BASE, MAX_ZOOM * BASE);
        if self.sizing != Sizing::Free {
            self.zoom = scale / BASE;
        }
        let gap = self.page_gap;
        let mut rects = vec![Rect::NOTHING; self.sizes.len()];
        let row_w = |r: &Vec<usize>| -> f32 {
            r.iter().map(|&p| self.rotated_size(p).0 * scale).sum::<f32>() + gap * (r.len() as f32 - 1.0)
        };
        let visible_rows: Vec<&Vec<usize>> =
            if self.continuous { rows.iter().collect() } else { rows.iter().filter(|r| r.contains(&self.current_page)).collect() };
        let half = if self.dual {
            visible_rows
                .iter()
                .flat_map(|r| r.iter())
                .map(|&p| self.rotated_size(p).0 * scale)
                .fold(0.0, f32::max)
        } else {
            0.0
        };
        let max_w = visible_rows.iter().map(|r| row_w(r)).fold(0.0, f32::max).max(if self.dual { half * 2.0 + gap } else { 0.0 });
        let content_w = vw.max(max_w + gap * 2.0);
        let mut y = gap;
        for r in &visible_rows {
            let row_h = r.iter().map(|&p| self.rotated_size(p).1 * scale).fold(0.0, f32::max);
            if self.dual {
                let center = content_w / 2.0;
                for (i, &p) in r.iter().enumerate() {
                    let (w, h) = self.rotated_size(p);
                    let (w, h) = (w * scale, h * scale);
                    let right = if r.len() == 1 { p % 2 == if self.odd_left { 1 } else { 0 } } else { i == 1 };
                    let x = if right { center + gap / 2.0 } else { center - gap / 2.0 - w };
                    rects[p] = Rect::from_min_size(pos2(x, y + (row_h - h) / 2.0), vec2(w, h));
                }
            } else {
                let p = r[0];
                let (w, h) = self.rotated_size(p);
                let (w, h) = (w * scale, h * scale);
                rects[p] = Rect::from_min_size(pos2((content_w - w) / 2.0, y + (row_h - h) / 2.0), vec2(w, h));
            }
            y += row_h + gap;
        }
        let mut content_h = y;
        if content_h < vh {
            let shift = (vh - content_h) / 2.0;
            for r in rects.iter_mut().filter(|r| r.is_positive()) {
                *r = r.translate(vec2(0.0, shift));
            }
            content_h = vh;
        }
        self.layout = Layout { scale, rects, size: vec2(content_w, content_h), rows };
    }

    fn clamp_offset(&mut self) {
        let max = (self.layout.size - self.viewport.size()).max(Vec2::ZERO);
        self.offset = self.offset.clamp(Vec2::ZERO, max);
    }

    fn row_of(&self, page: usize) -> usize {
        self.layout.rows.iter().position(|r| r.contains(&page)).unwrap_or(0)
    }

    // ---- coordinates ----

    /// Screen rectangle of a page (Rect::NOTHING when not laid out).
    pub fn page_rect(&self, page: usize) -> Rect {
        let r = self.layout.rects[page];
        if !r.is_positive() {
            return Rect::NOTHING;
        }
        r.translate(self.viewport.min.to_vec2() - self.offset)
    }

    fn page_to_screen(&self, page: usize, pt: Pos2) -> Pos2 {
        let r = self.page_rect(page);
        let (pw, ph) = self.sizes[page];
        let (x, y) = (pt.x, pt.y);
        let (rx, ry) = match self.rotation {
            90 => (ph - y, x),
            180 => (pw - x, ph - y),
            270 => (y, pw - x),
            _ => (x, y),
        };
        r.min + vec2(rx, ry) * self.layout.scale
    }

    fn screen_to_page(&self, page: usize, q: Pos2) -> Pos2 {
        let r = self.page_rect(page);
        let (pw, ph) = self.sizes[page];
        let v = (q - r.min) / self.layout.scale;
        let (rx, ry) = (v.x, v.y);
        let (x, y) = match self.rotation {
            90 => (ry, ph - rx),
            180 => (pw - rx, ph - ry),
            270 => (pw - ry, rx),
            _ => (rx, ry),
        };
        pos2(x, y)
    }

    fn rect_to_screen(&self, page: usize, r: &RectF) -> Rect {
        Rect::from_two_pos(self.page_to_screen(page, pos2(r.x0, r.y0)), self.page_to_screen(page, pos2(r.x1, r.y1)))
    }

    fn page_at(&self, q: Pos2) -> Option<usize> {
        self.visible_pages().into_iter().find(|&p| self.page_rect(p).contains(q))
    }

    /// The page nearest to a screen point (for selections dragged between pages).
    fn nearest_page(&self, q: Pos2) -> Option<usize> {
        self.visible_pages().into_iter().min_by(|&a, &b| {
            let da = self.page_rect(a).distance_sq_to_pos(q);
            let db = self.page_rect(b).distance_sq_to_pos(q);
            da.total_cmp(&db)
        })
    }

    pub fn visible_pages(&self) -> Vec<usize> {
        let vis = Rect::from_min_size(self.offset.to_pos2(), self.viewport.size());
        let rects = &self.layout.rects;
        if self.continuous {
            // Rows are laid out top to bottom, so binary search the first row reaching the window.
            let rows = &self.layout.rows;
            let first = rows.partition_point(|r| rects[*r.last().unwrap()].bottom().max(rects[r[0]].bottom()) < vis.top());
            let mut out = Vec::new();
            for r in &rows[first.min(rows.len())..] {
                if rects[r[0]].top() > vis.bottom() {
                    break;
                }
                for &p in r {
                    if rects[p].intersects(vis) {
                        out.push(p);
                    }
                }
            }
            out
        } else {
            (0..rects.len()).filter(|&p| rects[p].is_positive() && rects[p].intersects(vis)).collect()
        }
    }

    fn update_current_page(&mut self) {
        let vis = Rect::from_min_size(self.offset.to_pos2(), self.viewport.size());
        let mut best = (self.current_page, -1.0f32);
        for p in self.visible_pages() {
            let i = self.layout.rects[p].intersect(vis);
            let area = i.width().max(0.0) * i.height().max(0.0);
            if area > best.1 + 1.0 {
                best = (p, area);
            }
        }
        if best.1 >= 0.0 && self.continuous {
            self.current_page = best.0;
        }
    }

    // ---- navigation ----

    pub fn position(&self) -> Position {
        let r = self.layout.rects.get(self.current_page).copied().unwrap_or(Rect::NOTHING);
        let fraction =
            if r.is_positive() && self.continuous { ((self.offset.y - r.top()) / r.height()).clamp(-0.5, 1.0) } else { 0.0 };
        Position { page: self.current_page, fraction }
    }

    pub fn go_to_position(&mut self, pos: Position) {
        let page = pos.page.min(self.page_count().saturating_sub(1));
        self.current_page = page;
        self.target = Some(Target::Position(Position { page, fraction: pos.fraction }));
    }

    pub fn go_to_page(&mut self, page: usize) {
        self.go_to(page, None);
    }

    /// Jumps to a page, optionally to a point (in page coordinates) near its top.
    pub fn go_to(&mut self, page: usize, y: Option<f32>) {
        let page = page.min(self.page_count().saturating_sub(1));
        self.current_page = page;
        self.target = Some(Target::Page { page, y });
    }

    pub fn push_history(&mut self) {
        let p = self.position();
        if self.back.last() != Some(&p) {
            self.back.push(p);
            if self.back.len() > 100 {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn history_back(&mut self) {
        if let Some(p) = self.back.pop() {
            self.forward.push(self.position());
            self.go_to_position(p);
        }
    }

    pub fn history_forward(&mut self) {
        if let Some(p) = self.forward.pop() {
            self.back.push(self.position());
            self.go_to_position(p);
        }
    }

    pub fn follow(&mut self, target: &LinkTarget) {
        match target {
            LinkTarget::Page { page, y } => {
                self.push_history();
                self.go_to(*page, *y);
            }
            LinkTarget::Uri(u) => self.open_uri = Some(u.clone()),
        }
    }

    pub fn next_page(&mut self) {
        let row = self.row_of(self.current_page);
        if let Some(r) = self.layout.rows.get(row + 1) {
            self.go_to_page(r[0]);
        }
    }

    pub fn prev_page(&mut self) {
        let row = self.row_of(self.current_page);
        if row > 0 {
            self.go_to_page(self.layout.rows[row - 1][0]);
        }
    }

    pub fn first_page(&mut self) {
        self.go_to_page(0);
    }

    pub fn last_page(&mut self) {
        self.go_to_page(self.page_count().saturating_sub(1));
    }

    pub fn scroll_by(&mut self, d: Vec2) {
        self.offset += d;
        self.clamp_offset();
        if !self.continuous && d.y != 0.0 {
            self.flip_if_at_edge(d.y, true);
        }
    }

    /// Moves a window height down (or up), flipping pages in single-page mode.
    pub fn scroll_screen(&mut self, down: bool) {
        let h = (self.viewport.height() - 40.0).max(40.0);
        let before = self.offset;
        self.offset.y += if down { h } else { -h };
        self.clamp_offset();
        if !self.continuous && before == self.offset {
            if down {
                self.next_page();
            } else {
                self.prev_page();
                self.target = None;
                self.offset.y = f32::MAX;
            }
        }
    }

    fn flip_if_at_edge(&mut self, dy: f32, immediate: bool) {
        let max = (self.layout.size.y - self.viewport.height()).max(0.0);
        let at_bottom = self.offset.y >= max - 0.5;
        let at_top = self.offset.y <= 0.5;
        if (dy > 0.0 && at_bottom) || (dy < 0.0 && at_top) {
            self.flip_acc += dy;
        } else {
            self.flip_acc = 0.0;
        }
        let threshold = if immediate { 0.0 } else { 120.0 };
        if self.flip_acc > threshold && at_bottom {
            self.flip_acc = 0.0;
            let row = self.row_of(self.current_page);
            if row + 1 < self.layout.rows.len() {
                self.next_page();
            }
        } else if self.flip_acc < -threshold && at_top {
            self.flip_acc = 0.0;
            let row = self.row_of(self.current_page);
            if row > 0 {
                self.prev_page();
                self.target = None;
                self.offset.y = f32::MAX;
            }
        }
    }

    pub fn set_zoom(&mut self, zoom: f32, anchor: Option<Pos2>) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let screen = anchor.filter(|p| self.viewport.contains(*p)).unwrap_or(self.viewport.center());
        if let Some(page) = self.page_at(screen).or_else(|| self.nearest_page(screen)) {
            self.anchor = Some(Anchor { page, pt: self.screen_to_page(page, screen), screen });
        }
        self.sizing = Sizing::Free;
        self.zoom = zoom;
    }

    pub fn zoom_in(&mut self) {
        let z = self.effective_zoom();
        let next = ZOOM_STEPS.iter().copied().find(|s| *s > z * 1.01).unwrap_or(MAX_ZOOM);
        self.set_zoom(next, None);
    }

    pub fn zoom_out(&mut self) {
        let z = self.effective_zoom();
        let prev = ZOOM_STEPS.iter().rev().copied().find(|s| *s < z / 1.01).unwrap_or(MIN_ZOOM);
        self.set_zoom(prev, None);
    }

    /// Changes a layout setting while keeping the current page in view.
    pub fn relayout(&mut self) {
        let pos = self.position();
        self.target = Some(Target::Position(pos));
    }

    pub fn rotate(&mut self, clockwise: bool) {
        let pos = self.position();
        self.rotation = (self.rotation + if clockwise { 90 } else { 270 }) % 360;
        self.target = Some(Target::Page { page: pos.page, y: None });
    }

    pub fn set_invert(&mut self, invert: bool) {
        self.invert = invert;
    }

    pub fn reveal(&mut self, page: usize, rect: RectF) {
        self.current_page = page;
        self.target = Some(Target::Rect { page, rect });
    }

    // ---- text ----

    pub fn page_text_now(&self, page: usize) -> Arc<PageText> {
        crate::render::page_text(&*self.doc, &self.text.cache, page)
    }

    fn links_for(&mut self, page: usize) -> Arc<Vec<Link>> {
        if let Some(l) = self.links.get(&page) {
            return l.clone();
        }
        let l = Arc::new(self.doc.links(page));
        self.links.insert(page, l.clone());
        l
    }

    fn ordered_selection(&self) -> Option<(SelPoint, SelPoint, SelectionStyle)> {
        let s = self.selection?;
        let key = |p: &SelPoint| (p.page, p.pt.y, p.pt.x);
        let (a, b) = if key(&s.a) <= key(&s.b) { (s.a, s.b) } else { (s.b, s.a) };
        Some((a, b, s.style))
    }

    /// Selected character range on one page.
    fn selection_on(&self, page: usize, text: &PageText) -> Option<(usize, usize)> {
        let (a, b, style) = self.ordered_selection()?;
        if page < a.page || page > b.page {
            return None;
        }
        let (pw, ph) = self.sizes[page];
        let from = if page == a.page { (a.pt.x, a.pt.y) } else { (0.0, 0.0) };
        let to = if page == b.page { (b.pt.x, b.pt.y) } else { (pw, ph) };
        if a.page == b.page {
            return text.range_between(style, from, to);
        }
        let n = text.chars.len();
        let start = if page == a.page { text.range_between(style, from, from).map(|r| r.0).or(text.offset_at(from.0, from.1))? } else { 0 };
        let end = if page == b.page { text.range_between(style, to, to).map(|r| r.1).or(text.offset_at(to.0, to.1))? } else { n };
        (start < end).then_some((start, end))
    }

    pub fn selected_text(&self) -> String {
        let Some((a, b, _)) = self.ordered_selection() else {
            return String::new();
        };
        let mut out = String::new();
        for p in a.page..=b.page {
            let t = self.page_text_now(p);
            if let Some((s, e)) = self.selection_on(p, &t) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&t.range_text(s, e));
            }
        }
        out
    }

    pub fn has_selection(&self) -> bool {
        self.selection.is_some_and(|s| s.a != s.b || s.style != SelectionStyle::Glyph)
    }

    pub fn select_page(&mut self, page: usize) {
        let (pw, ph) = self.sizes[page];
        self.selection = Some(Selection {
            a: SelPoint { page, pt: pos2(0.0, 0.0) },
            b: SelPoint { page, pt: pos2(pw, ph) },
            style: SelectionStyle::Glyph,
        });
    }

    pub fn select_all(&mut self) {
        let last = self.page_count() - 1;
        let (pw, ph) = self.sizes[last];
        self.selection = Some(Selection {
            a: SelPoint { page: 0, pt: pos2(0.0, 0.0) },
            b: SelPoint { page: last, pt: pos2(pw, ph) },
            style: SelectionStyle::Glyph,
        });
    }

    // ---- textures ----

    fn receive(&mut self, ctx: &egui::Context) {
        for _ in 0..UPLOADS_PER_FRAME {
            let Ok(r) = self.pool.results.try_recv() else {
                return;
            };
            let Some(img) = r.image else {
                continue;
            };
            let size = [img.width as usize, img.height as usize];
            let color = ColorImage::from_rgba_premultiplied(size, &img.data);
            let opts = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
            let tex = ctx.load_texture(format!("p{}-{:?}", r.key.page, r.key.kind), color, opts);
            let bytes = img.data.len();
            self.texture_bytes += bytes;
            if let Some(old) = self.textures.insert(r.key, Cached { tex, bytes, used: self.frame }) {
                self.texture_bytes -= old.bytes;
            }
        }
        ctx.request_repaint();
    }

    fn evict(&mut self) {
        if self.texture_bytes <= TEXTURE_BUDGET {
            return;
        }
        let mut keys: Vec<(u64, TileKey)> =
            self.textures.iter().filter(|(_, c)| c.used + 1 < self.frame).map(|(k, c)| (c.used, *k)).collect();
        keys.sort_by_key(|k| k.0);
        for (_, k) in keys {
            if self.texture_bytes <= TEXTURE_BUDGET * 3 / 4 {
                break;
            }
            if let Some(c) = self.textures.remove(&k) {
                self.texture_bytes -= c.bytes;
            }
        }
    }

    fn touch(&mut self, key: &TileKey) -> Option<egui::TextureId> {
        let frame = self.frame;
        self.textures.get_mut(key).map(|c| {
            c.used = frame;
            c.tex.id()
        })
    }

    /// The best already-rendered whole-page image of a page, for drawing while tiles arrive.
    fn fallback(&mut self, page: usize) -> Option<(egui::TextureId, TileKey)> {
        let mut best: Option<TileKey> = None;
        for k in self.textures.keys() {
            if k.page as usize == page && k.kind != TileKind::Tile && k.invert == self.invert && best.is_none_or(|b| k.scale > b.scale) {
                best = Some(*k);
            }
        }
        let k = best?;
        self.touch(&k).map(|t| (t, k))
    }

    fn page_scale_for_whole(&self, page: usize, ds: f32) -> f32 {
        let (pw, ph) = self.sizes[page];
        ds.min(MAX_TEXTURE_SIDE / pw.max(ph).max(1.0))
    }

    fn page_key(&self, page: usize, ds: f32) -> TileKey {
        TileKey { page: page as u32, kind: TileKind::Page, scale: quant(ds), x: 0, y: 0, w: 0, h: 0, invert: self.invert }
    }

    fn needs_tiles(&self, page: usize, ds: f32) -> bool {
        let (pw, ph) = self.sizes[page];
        self.doc.supports_regions() && (pw * ds) * (ph * ds) > MAX_WHOLE_PAGE_PIXELS as f32
    }

    /// Tiles covering the visible part of a page, nearest to the window center first.
    fn tiles_for(&self, page: usize, ds: f32, clip: Rect) -> Vec<TileKey> {
        let (pw, ph) = self.sizes[page];
        let full_w = (pw * ds).round() as u32;
        let full_h = (ph * ds).round() as u32;
        let vis = self.page_rect(page).intersect(clip.expand(256.0));
        if !vis.is_positive() {
            return Vec::new();
        }
        let corners = [vis.left_top(), vis.right_top(), vis.left_bottom(), vis.right_bottom()].map(|c| self.screen_to_page(page, c));
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for c in corners {
            x0 = x0.min(c.x);
            y0 = y0.min(c.y);
            x1 = x1.max(c.x);
            y1 = y1.max(c.y);
        }
        let tx0 = ((x0 * ds).max(0.0) as u32) / TILE;
        let ty0 = ((y0 * ds).max(0.0) as u32) / TILE;
        let tx1 = ((x1 * ds).max(0.0) as u32).min(full_w.saturating_sub(1)) / TILE;
        let ty1 = ((y1 * ds).max(0.0) as u32).min(full_h.saturating_sub(1)) / TILE;
        let center = self.screen_to_page(page, clip.center());
        let mut out = Vec::new();
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let x = tx * TILE;
                let y = ty * TILE;
                out.push(TileKey {
                    page: page as u32,
                    kind: TileKind::Tile,
                    scale: quant(ds),
                    x,
                    y,
                    w: TILE.min(full_w - x),
                    h: TILE.min(full_h - y),
                    invert: self.invert,
                });
            }
        }
        out.sort_by(|a, b| {
            let d = |k: &TileKey| {
                let cx = (k.x + k.w / 2) as f32 / ds - center.x;
                let cy = (k.y + k.h / 2) as f32 / ds - center.y;
                cx * cx + cy * cy
            };
            d(a).total_cmp(&d(b))
        });
        out
    }

    fn paint_texture(&self, painter: &egui::Painter, page: usize, tex: egui::TextureId, pt_rect: RectF, tint: Color32) {
        let corners = [
            self.page_to_screen(page, pos2(pt_rect.x0, pt_rect.y0)),
            self.page_to_screen(page, pos2(pt_rect.x1, pt_rect.y0)),
            self.page_to_screen(page, pos2(pt_rect.x1, pt_rect.y1)),
            self.page_to_screen(page, pos2(pt_rect.x0, pt_rect.y1)),
        ];
        let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        let mut mesh = Mesh::with_texture(tex);
        for (p, uv) in corners.iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv, color: tint });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(Shape::mesh(mesh));
    }

    /// Requests a sidebar thumbnail and returns whatever image of the page is available.
    pub fn thumbnail(&mut self, page: usize, width_px: f32) -> Option<egui::TextureId> {
        let (pw, ph) = self.sizes[page];
        let w = if self.rotation % 180 == 90 { ph } else { pw };
        let ds = width_px / w.max(1.0);
        let key = TileKey { page: page as u32, kind: TileKind::Thumb, scale: quant(ds), x: 0, y: 0, w: 0, h: 0, invert: self.invert };
        if let Some(t) = self.touch(&key) {
            return Some(t);
        }
        self.thumb_wanted.push(key);
        self.fallback(page).map(|f| f.0)
    }

    /// Paints a texture of a whole page (unrotated texture) into a rotated screen rectangle.
    pub fn paint_page_image(&self, painter: &egui::Painter, tex: egui::TextureId, rect: Rect) {
        let uv = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        let k = (self.rotation / 90) as usize;
        let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
        let mut mesh = Mesh::with_texture(tex);
        for i in 0..4 {
            mesh.vertices.push(egui::epaint::Vertex { pos: corners[(i + k) % 4], uv: uv[i], color: Color32::WHITE });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(Shape::mesh(mesh));
    }

    pub fn rotated_page_size(&self, page: usize) -> (f32, f32) {
        self.rotated_size(page)
    }

    // ---- frame ----

    fn apply_target(&mut self) {
        let Some(t) = self.target.take() else {
            return;
        };
        let gap = self.page_gap;
        match t {
            Target::Page { page, y } => {
                let r = self.layout.rects[page];
                if !r.is_positive() {
                    return;
                }
                match y {
                    Some(y) => {
                        let p = self.page_to_screen(page, pos2(0.0, y)) - self.viewport.min.to_vec2() + self.offset;
                        self.offset.y = p.y - gap;
                    }
                    None => self.offset.y = r.top() - gap,
                }
                if r.width() <= self.viewport.width() {
                    self.offset.x = r.center().x - self.viewport.width() / 2.0;
                } else {
                    self.offset.x = r.left() - gap;
                }
            }
            Target::Position(p) => {
                let r = self.layout.rects[p.page];
                if r.is_positive() {
                    self.offset.y = if self.continuous { r.top() + p.fraction * r.height() } else { 0.0 };
                    self.offset.x = (self.layout.size.x - self.viewport.width()) / 2.0;
                }
            }
            Target::Rect { page, rect } => {
                let s = self.rect_to_screen(page, &rect).translate(self.offset - self.viewport.min.to_vec2());
                let vis = Rect::from_min_size(self.offset.to_pos2(), self.viewport.size()).shrink(40.0);
                if !vis.contains_rect(s) {
                    self.offset.y = s.center().y - self.viewport.height() / 2.0;
                    if s.width() < self.viewport.width() && (s.left() < vis.left() || s.right() > vis.right()) {
                        self.offset.x = s.center().x - self.viewport.width() / 2.0;
                    }
                }
            }
        }
        self.clamp_offset();
    }

    fn apply_anchor(&mut self) {
        let Some(a) = self.anchor.take() else {
            return;
        };
        if !self.layout.rects[a.page].is_positive() {
            return;
        }
        let now = self.page_to_screen(a.page, a.pt);
        self.offset += now - a.screen;
        self.clamp_offset();
    }

    fn handle_scroll_input(&mut self, ui: &Ui, hovered: bool) {
        if !hovered {
            return;
        }
        let (zoom_delta, scroll, pointer) =
            ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta(), i.pointer.hover_pos()));
        if zoom_delta != 1.0 {
            let z = self.effective_zoom() * zoom_delta;
            self.set_zoom(z, pointer);
        }
        if scroll != Vec2::ZERO {
            self.offset -= scroll;
            self.clamp_offset();
            if !self.continuous {
                self.flip_if_at_edge(-scroll.y, false);
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, colors: &Colors) -> Response {
        self.frame += 1;
        let ctx = ui.ctx().clone();
        self.receive(&ctx);
        if let Some(s) = &mut self.search {
            s.poll();
        }

        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag()).accessible_name("Document");
        let size_changed = rect.size() != self.viewport.size();
        let keep = if size_changed && self.viewport.is_positive() && self.target.is_none() && self.anchor.is_none() {
            Some(self.position())
        } else {
            None
        };
        self.viewport = rect;

        let hovered = resp.hovered() || resp.contains_pointer();
        if self.layout.scale > 0.0 {
            self.handle_scroll_input(ui, hovered);
        }
        self.compute_layout(rect.width(), rect.height());
        if let Some(p) = keep
            && self.sizing != Sizing::Free
        {
            self.target = Some(Target::Position(p));
        }
        self.apply_anchor();
        self.apply_target();
        self.clamp_offset();
        self.update_current_page();

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, colors.canvas);

        let ppp = ctx.pixels_per_point();
        let ds = self.layout.scale * ppp;
        let visible = self.visible_pages();
        let mut wanted: Vec<TileKey> = Vec::new();
        let mut later: Vec<TileKey> = Vec::new();
        let paper = if self.invert { Color32::BLACK } else { Color32::WHITE };

        for &p in &visible {
            let pr = self.page_rect(p);
            let shadow = egui::epaint::Shadow { offset: [0, 2], blur: 8, spread: 0, color: colors.page_shadow };
            painter.add(shadow.as_shape(pr, CornerRadius::ZERO));
            painter.rect_filled(pr, 0.0, paper);
            let (pw, ph) = self.sizes[p];
            let full = RectF::new(0.0, 0.0, pw, ph);
            if self.needs_tiles(p, ds) {
                let preview = self.page_scale_for_whole(p, PREVIEW_SIDE / pw.max(ph).max(1.0));
                let pk = self.page_key(p, preview);
                if let Some((tex, _)) = self.fallback(p) {
                    self.paint_texture(&painter, p, tex, full, Color32::WHITE);
                }
                if self.touch(&pk).is_none() {
                    wanted.push(pk);
                }
                for t in self.tiles_for(p, ds, rect) {
                    if let Some(tex) = self.touch(&t) {
                        let r = RectF::new(t.x as f32 / ds, t.y as f32 / ds, (t.x + t.w) as f32 / ds, (t.y + t.h) as f32 / ds);
                        self.paint_texture(&painter, p, tex, r, Color32::WHITE);
                    } else {
                        wanted.push(t);
                    }
                }
            } else {
                let key = self.page_key(p, self.page_scale_for_whole(p, ds));
                if let Some(tex) = self.touch(&key) {
                    self.paint_texture(&painter, p, tex, full, Color32::WHITE);
                } else {
                    if let Some((tex, _)) = self.fallback(p) {
                        self.paint_texture(&painter, p, tex, full, Color32::WHITE);
                    }
                    wanted.push(key);
                }
            }
            painter.rect_stroke(pr, 0.0, Stroke::new(1.0, colors.page_border), egui::StrokeKind::Outside);
            self.text.request(p);
        }

        // Prefetch the neighbors of the visible range.
        if let (Some(&first), Some(&last)) = (visible.first(), visible.last()) {
            for p in [last + 1, last + 2, first.wrapping_sub(1)] {
                if p < self.page_count() && self.layout.rects[p].is_positive() {
                    let s = if self.needs_tiles(p, ds) {
                        let (pw, ph) = self.sizes[p];
                        self.page_scale_for_whole(p, PREVIEW_SIDE / pw.max(ph).max(1.0))
                    } else {
                        self.page_scale_for_whole(p, ds)
                    };
                    let k = self.page_key(p, s);
                    if self.touch(&k).is_none() {
                        later.push(k);
                    }
                }
            }
        }

        self.paint_highlights(&painter, &visible, colors);
        self.handle_pointer(ui, &resp, &visible, colors, &painter);
        self.scrollbars(ui, rect, colors);

        wanted.append(&mut std::mem::take(&mut self.thumb_wanted));
        wanted.append(&mut later);
        self.pool.set_wanted(wanted);
        self.evict();
        resp
    }

    fn paint_highlights(&mut self, painter: &egui::Painter, visible: &[usize], colors: &Colors) {
        for &p in visible {
            let Some(text) = self.text.get(p) else {
                continue;
            };
            if let Some(s) = &self.search
                && let Some(Some(hits)) = s.results.get(p)
            {
                for (i, (a, b)) in hits.iter().enumerate() {
                    let current = s.current == Some((p, i));
                    let color = if current { colors.search_current } else { colors.search_match };
                    for r in text.range_rects(*a, *b) {
                        painter.rect_filled(self.rect_to_screen(p, &r).expand(1.0), 2.0, color);
                    }
                }
            }
            if let Some((a, b)) = self.selection_on(p, &text) {
                for r in text.range_rects(a, b) {
                    painter.rect_filled(self.rect_to_screen(p, &r), 0.0, colors.selection);
                }
            }
        }
    }

    fn handle_pointer(&mut self, ui: &Ui, resp: &Response, visible: &[usize], colors: &Colors, painter: &egui::Painter) {
        let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
        self.hover_link = None;
        if let Some(q) = pointer.filter(|q| self.viewport.contains(*q))
            && let Some(p) = self.page_at(q)
        {
            let pt = self.screen_to_page(p, q);
            let links = self.links_for(p);
            if let Some(l) = links.iter().find(|l| l.rect.contains(pt.x, pt.y)) {
                self.hover_link = Some(l.clone());
            }
        }

        let primary_down_pos = ui.input(|i| i.pointer.press_origin());
        if resp.drag_started_by(PointerButton::Primary) {
            if let Some(q) = primary_down_pos
                && let Some(p) = self.page_at(q)
            {
                let text = self.page_text_now(p);
                if text.is_empty() {
                    self.drag = Drag::Pan;
                } else {
                    let pt = self.screen_to_page(p, q);
                    let sp = SelPoint { page: p, pt };
                    self.selection = Some(Selection { a: sp, b: sp, style: SelectionStyle::Glyph });
                    self.drag = Drag::Select;
                }
            } else {
                self.drag = Drag::Pan;
            }
        }
        if resp.drag_started_by(PointerButton::Middle) {
            self.drag = Drag::Pan;
        }
        if resp.dragged() {
            match self.drag {
                Drag::Pan => {
                    self.offset -= resp.drag_delta();
                    self.clamp_offset();
                }
                Drag::Select => {
                    if let Some(q) = pointer
                        && let Some(p) = self.page_at(q).or_else(|| self.nearest_page(q))
                    {
                        let r = self.page_rect(p);
                        let qc = q.clamp(r.min, r.max);
                        let pt = self.screen_to_page(p, qc);
                        if p != self.selection.map(|s| s.b.page).unwrap_or(p) {
                            self.text.request(p);
                        }
                        if let Some(s) = &mut self.selection {
                            s.b = SelPoint { page: p, pt };
                        }
                        // Scroll while dragging past the edges.
                        let v = self.viewport;
                        let dy = if q.y < v.top() + 10.0 {
                            q.y - v.top() - 10.0
                        } else if q.y > v.bottom() - 10.0 {
                            q.y - v.bottom() + 10.0
                        } else {
                            0.0
                        };
                        if dy != 0.0 {
                            self.offset.y += dy * 0.5;
                            self.clamp_offset();
                            ui.ctx().request_repaint();
                        }
                    }
                }
                Drag::None => {}
            }
        }
        if resp.drag_stopped() {
            if self.drag == Drag::Select && !self.has_selection_text() {
                self.selection = None;
            }
            self.drag = Drag::None;
        }

        if resp.triple_clicked() {
            self.select_at(pointer, SelectionStyle::Line);
        } else if resp.double_clicked() {
            self.select_at(pointer, SelectionStyle::Word);
        } else if resp.clicked_by(PointerButton::Primary) {
            if let Some(l) = self.hover_link.clone() {
                self.follow(&l.target);
            } else {
                self.selection = None;
            }
        }
        if resp.secondary_clicked() {
            self.context_link = self.hover_link.clone();
            self.context_point = pointer.and_then(|q| self.page_at(q).map(|p| SelPoint { page: p, pt: self.screen_to_page(p, q) }));
        }

        // Cursor and link highlight.
        if self.drag == Drag::Pan {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if let Some(l) = &self.hover_link {
            let p = pointer.and_then(|q| self.page_at(q)).unwrap_or(0);
            painter.rect_filled(self.rect_to_screen(p, &l.rect), 2.0, colors.link_hover);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                let tip = match &l.target {
                    LinkTarget::Uri(u) => u.clone(),
                    LinkTarget::Page { page, .. } => {
                        let label = self.doc.page_label(*page).unwrap_or_else(|| (page + 1).to_string());
                        format!("Go to page {label}")
                    }
                };
                egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), Id::unique("link-tip"), egui::PopupAnchor::Pointer)
                    .gap(12.0)
                    .show(|ui| {
                        ui.label(tip);
                    });
            }
        } else if resp.hovered()
            && let Some(q) = pointer
            && let Some(p) = self.page_at(q)
            && let Some(text) = self.text.get(p)
        {
            let pt = self.screen_to_page(p, q);
            if self.drag == Drag::Select || text.char_at(pt.x, pt.y).is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::Text);
            }
        }
        let _ = visible;
    }

    fn has_selection_text(&self) -> bool {
        let Some((a, b, _)) = self.ordered_selection() else {
            return false;
        };
        (a.page..=b.page).any(|p| self.text.get(p).is_some_and(|t| self.selection_on(p, &t).is_some()))
    }

    fn select_at(&mut self, pointer: Option<Pos2>, style: SelectionStyle) {
        if let Some(q) = pointer
            && let Some(p) = self.page_at(q)
        {
            let pt = self.screen_to_page(p, q);
            let sp = SelPoint { page: p, pt };
            self.page_text_now(p);
            self.selection = Some(Selection { a: sp, b: sp, style });
            if !self.has_selection_text() {
                self.selection = None;
            }
        }
    }

    fn scrollbars(&mut self, ui: &Ui, rect: Rect, colors: &Colors) {
        let w = ui.style().spacing.scroll.bar_width.max(4.0);
        let content = self.layout.size;
        let view = rect.size();
        let painter = ui.painter_at(rect);
        if content.y > view.y + 0.5 {
            let track = Rect::from_min_max(pos2(rect.right() - w - 3.0, rect.top() + 3.0), pos2(rect.right() - 3.0, rect.bottom() - 3.0));
            let thumb_h = (view.y / content.y * track.height()).max(28.0).min(track.height());
            let range = content.y - view.y;
            let t = self.offset.y / range;
            let thumb = Rect::from_min_size(pos2(track.left(), track.top() + t * (track.height() - thumb_h)), vec2(w, thumb_h));
            let r = ui.interact(track.expand2(vec2(3.0, 0.0)), ui.scope_id().with("vscroll"), Sense::click_and_drag());
            if r.dragged() {
                self.offset.y += r.drag_delta().y * range / (track.height() - thumb_h).max(1.0);
                self.clamp_offset();
            } else if r.clicked()
                && let Some(p) = r.interact_pointer_pos()
            {
                self.offset.y += if p.y < thumb.top() { -view.y } else if p.y > thumb.bottom() { view.y } else { 0.0 };
                self.clamp_offset();
            }
            let a = if r.dragged() || r.hovered() { 200 } else { 110 };
            let c = colors.text_dim.gamma_multiply(a as f32 / 255.0);
            painter.rect_filled(thumb, w / 2.0, c);
        }
        if content.x > view.x + 0.5 {
            let track = Rect::from_min_max(pos2(rect.left() + 3.0, rect.bottom() - w - 3.0), pos2(rect.right() - w - 6.0, rect.bottom() - 3.0));
            let thumb_w = (view.x / content.x * track.width()).max(28.0).min(track.width());
            let range = content.x - view.x;
            let t = self.offset.x / range;
            let thumb = Rect::from_min_size(pos2(track.left() + t * (track.width() - thumb_w), track.top()), vec2(thumb_w, w));
            let r = ui.interact(track.expand2(vec2(0.0, 3.0)), ui.scope_id().with("hscroll"), Sense::click_and_drag());
            if r.dragged() {
                self.offset.x += r.drag_delta().x * range / (track.width() - thumb_w).max(1.0);
                self.clamp_offset();
            }
            let a = if r.dragged() || r.hovered() { 200 } else { 110 };
            painter.rect_filled(thumb, w / 2.0, colors.text_dim.gamma_multiply(a as f32 / 255.0));
        }
    }

    /// Full-screen slideshow of one page at a time.
    pub fn show_presentation(&mut self, ui: &mut Ui) -> Response {
        self.frame += 1;
        let ctx = ui.ctx().clone();
        self.receive(&ctx);
        let rect = ui.max_rect();
        let resp = ui.allocate_rect(rect, Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::BLACK);
        if let Some(c) = self.presentation_blank {
            painter.rect_filled(rect, 0.0, c);
            return resp;
        }
        let p = self.current_page.min(self.page_count() - 1);
        let (w, h) = self.rotated_size(p);
        let scale = (rect.width() / w).min(rect.height() / h);
        let size = vec2(w, h) * scale;
        let pr = Rect::from_center_size(rect.center(), size);
        let (pw, ph) = self.sizes[p];
        let ds = self.page_scale_for_whole(p, scale * ctx.pixels_per_point());
        let key = self.page_key(p, ds);
        let mut wanted = Vec::new();
        painter.rect_filled(pr, 0.0, if self.invert { Color32::BLACK } else { Color32::WHITE });
        if let Some(tex) = self.touch(&key) {
            self.paint_page_image(&painter, tex, pr);
        } else {
            if let Some((tex, _)) = self.fallback(p) {
                self.paint_page_image(&painter, tex, pr);
            }
            wanted.push(key);
        }
        for n in [p + 1, p + 2] {
            if n < self.page_count() {
                let (nw, nh) = self.rotated_size(n);
                let s = (rect.width() / nw).min(rect.height() / nh) * ctx.pixels_per_point();
                let k = self.page_key(n, self.page_scale_for_whole(n, s));
                if self.touch(&k).is_none() {
                    wanted.push(k);
                }
            }
        }
        let _ = (pw, ph);
        self.pool.set_wanted(wanted);
        self.evict();
        resp
    }
}
