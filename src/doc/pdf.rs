//! PDF backend built on hayro.

use super::text::TextBuilder;
use super::*;
use hayro::hayro_interpret::font::Glyph;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache,
    InterpreterSettings, SoftMask, interpret_page,
};
use hayro::hayro_syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier};
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::kurbo::{Affine, BezPath, Point};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings};
use std::collections::HashMap;

pub struct PdfDocument {
    pdf: Pdf,
    sizes: Vec<(f32, f32)>,
    page_ids: HashMap<ObjectIdentifier, usize>,
    labels: Vec<Option<String>>,
}

pub fn decode_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if bytes.len() >= 3 && bytes[..3] == [0xEF, 0xBB, 0xBF] {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    bytes.iter().map(|&b| pdfdoc_char(b)).collect()
}

fn pdfdoc_char(b: u8) -> char {
    match b {
        0x80 => '•',
        0x81 => '†',
        0x82 => '‡',
        0x83 => '…',
        0x84 => '—',
        0x85 => '–',
        0x86 => 'ƒ',
        0x87 => '⁄',
        0x88 => '‹',
        0x89 => '›',
        0x8A => '−',
        0x8B => '‰',
        0x8C => '„',
        0x8D => '“',
        0x8E => '”',
        0x8F => '‘',
        0x90 => '’',
        0x91 => '‚',
        0x92 => '™',
        0x93 => 'ﬁ',
        0x94 => 'ﬂ',
        0x95 => 'Ł',
        0x96 => 'Œ',
        0x97 => 'Š',
        0x98 => 'Ÿ',
        0x99 => 'Ž',
        0x9A => 'ı',
        0x9B => 'ł',
        0x9C => 'œ',
        0x9D => 'š',
        0x9E => 'ž',
        0xA0 => '€',
        _ => b as char,
    }
}

fn obj_string(o: &Object<'_>) -> Option<String> {
    match o {
        Object::String(s) => Some(decode_text(s.as_bytes())),
        Object::Name(n) => Some(n.as_str().to_string()),
        _ => None,
    }
}

fn roman(mut n: usize, upper: bool) -> String {
    const T: &[(usize, &str)] = &[
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for &(v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    if upper { s.to_uppercase() } else { s }
}

fn letters(n: usize, upper: bool) -> String {
    let n = n.max(1);
    let c = (b'a' + ((n - 1) % 26) as u8) as char;
    let s: String = std::iter::repeat_n(c, (n - 1) / 26 + 1).collect();
    if upper { s.to_uppercase() } else { s }
}

impl PdfDocument {
    pub fn open(data: Vec<u8>, password: Option<&str>) -> Result<Self, OpenError> {
        let pdf = match Pdf::new_with_password(data, password.unwrap_or("")) {
            Ok(p) => p,
            Err(LoadPdfError::Decryption(hayro::hayro_syntax::DecryptionError::PasswordProtected)) => {
                return Err(if password.is_some() { OpenError::WrongPassword } else { OpenError::NeedsPassword });
            }
            Err(LoadPdfError::Decryption(e)) => return Err(OpenError::Invalid(format!("Unsupported encryption: {e:?}"))),
            Err(LoadPdfError::Invalid) => return Err(OpenError::Invalid("The PDF file is damaged".into())),
        };
        let pages = pdf.pages();
        let sizes = pages.iter().map(|p| p.render_dimensions()).collect();
        let page_ids = pages.iter().enumerate().filter_map(|(i, p)| p.raw().obj_id().map(|id| (id, i))).collect();
        let mut doc = PdfDocument { pdf, sizes, page_ids, labels: Vec::new() };
        doc.labels = doc.read_labels();
        Ok(doc)
    }

    fn catalog(&self) -> Option<Dict<'_>> {
        self.pdf.xref().get::<Dict>(self.pdf.xref().root_id())
    }

    fn resolve<'a>(&'a self, m: MaybeRef<Object<'a>>) -> Option<Object<'a>> {
        match m {
            MaybeRef::Ref(r) => self.pdf.xref().get::<Object>(r.into()),
            MaybeRef::NotRef(o) => Some(o),
        }
    }

    fn read_labels(&self) -> Vec<Option<String>> {
        let n = self.sizes.len();
        let Some(tree) = self.catalog().and_then(|c| c.get::<Dict>(b"PageLabels")) else {
            return Vec::new();
        };
        let mut ranges: Vec<(usize, Dict<'_>)> = Vec::new();
        self.collect_number_tree(&tree, &mut ranges, 0);
        if ranges.is_empty() {
            return Vec::new();
        }
        ranges.sort_by_key(|r| r.0);
        let mut out = vec![None; n];
        for (i, (start, d)) in ranges.iter().enumerate() {
            let end = ranges.get(i + 1).map(|r| r.0).unwrap_or(n).min(n);
            let style = d.get::<Name>(b"S").map(|s| s.as_str().to_string());
            let prefix = d.get::<Object>(b"P").and_then(|o| obj_string(&o)).unwrap_or_default();
            let first = d.get::<i64>(b"St").unwrap_or(1).max(1) as usize;
            for p in *start..end {
                let v = first + (p - start);
                let num = match style.as_deref() {
                    Some("D") => v.to_string(),
                    Some("R") => roman(v, true),
                    Some("r") => roman(v, false),
                    Some("A") => letters(v, true),
                    Some("a") => letters(v, false),
                    _ => String::new(),
                };
                out[p] = Some(format!("{prefix}{num}"));
            }
        }
        out
    }

    fn collect_number_tree<'a>(&'a self, node: &Dict<'a>, out: &mut Vec<(usize, Dict<'a>)>, depth: u32) {
        if depth > 32 {
            return;
        }
        if let Some(nums) = node.get::<Array>(b"Nums") {
            let items: Vec<_> = nums.raw_iter().filter_map(|m| self.resolve(m)).collect();
            for pair in items.chunks(2) {
                if let [Object::Number(k), Object::Dict(d)] = pair {
                    out.push((k.as_i64().max(0) as usize, d.clone()));
                }
            }
        }
        if let Some(kids) = node.get::<Array>(b"Kids") {
            for kid in kids.iter::<Dict>() {
                self.collect_number_tree(&kid, out, depth + 1);
            }
        }
    }

    fn lookup_name_tree<'a>(&'a self, node: &Dict<'a>, key: &[u8], depth: u32) -> Option<Object<'a>> {
        if depth > 32 {
            return None;
        }
        if let Some(names) = node.get::<Array>(b"Names") {
            let mut it = names.raw_iter();
            while let (Some(k), Some(v)) = (it.next(), it.next()) {
                if let Some(Object::String(s)) = self.resolve(k)
                    && s.as_bytes() == key
                {
                    return self.resolve(v);
                }
            }
        }
        if let Some(kids) = node.get::<Array>(b"Kids") {
            for kid in kids.iter::<Dict>() {
                if let Some(limits) = kid.get::<Array>(b"Limits") {
                    let l: Vec<_> = limits.raw_iter().filter_map(|m| self.resolve(m)).collect();
                    if let [Object::String(lo), Object::String(hi)] = &l[..]
                        && (key < lo.as_bytes() || key > hi.as_bytes())
                    {
                        continue;
                    }
                }
                if let Some(v) = self.lookup_name_tree(&kid, key, depth + 1) {
                    return Some(v);
                }
            }
        }
        None
    }

    fn named_dest<'a>(&'a self, name: &[u8]) -> Option<Object<'a>> {
        let cat = self.catalog()?;
        if let Some(dests) = cat.get::<Dict>(b"Dests")
            && let Some(o) = dests.get::<Object>(name)
        {
            return Some(o);
        }
        let tree = cat.get::<Dict>(b"Names")?.get::<Dict>(b"Dests")?;
        self.lookup_name_tree(&tree, name, 0)
    }

    fn resolve_dest(&self, dest: Object<'_>, depth: u32) -> Option<LinkTarget> {
        if depth > 4 {
            return None;
        }
        match dest {
            Object::Array(arr) => {
                let mut it = arr.raw_iter();
                let first = it.next()?;
                let page = match first {
                    MaybeRef::Ref(r) => *self.page_ids.get(&r.into())?,
                    MaybeRef::NotRef(Object::Number(n)) => n.as_i64().max(0) as usize,
                    _ => return None,
                };
                if page >= self.sizes.len() {
                    return None;
                }
                let kind = it.next().and_then(|m| self.resolve(m));
                let nums: Vec<Option<f64>> = it
                    .map(|m| match self.resolve(m) {
                        Some(Object::Number(n)) => Some(n.as_f64()),
                        _ => None,
                    })
                    .collect();
                let top = match kind {
                    Some(Object::Name(n)) => match n.as_str() {
                        "XYZ" => nums.get(1).copied().flatten().map(|t| (nums.first().copied().flatten().unwrap_or(0.0), t)),
                        "FitH" | "FitBH" => nums.first().copied().flatten().map(|t| (0.0, t)),
                        "FitR" => nums.get(3).copied().flatten().map(|t| (nums.first().copied().flatten().unwrap_or(0.0), t)),
                        _ => None,
                    },
                    _ => None,
                };
                let y = top.map(|(x, t)| {
                    let p = &self.pdf.pages()[page];
                    let tr = p.initial_transform(true).to_kurbo();
                    (tr * Point::new(x, t)).y as f32
                });
                Some(LinkTarget::Page { page, y })
            }
            Object::Dict(d) => self.resolve_dest(d.get::<Object>(b"D")?, depth + 1),
            Object::String(s) => self.resolve_dest(self.named_dest(s.as_bytes())?, depth + 1),
            Object::Name(n) => self.resolve_dest(self.named_dest(n.as_ref())?, depth + 1),
            _ => None,
        }
    }

    fn resolve_action(&self, action: &Dict<'_>) -> Option<LinkTarget> {
        match action.get::<Name>(b"S")?.as_str() {
            "GoTo" => self.resolve_dest(action.get::<Object>(b"D")?, 0),
            "URI" => action.get::<Object>(b"URI").and_then(|o| obj_string(&o)).map(LinkTarget::Uri),
            "Launch" => {
                let f = action.get::<Object>(b"F")?;
                let s = match f {
                    Object::Dict(d) => d.get::<Object>(b"F").and_then(|o| obj_string(&o))?,
                    o => obj_string(&o)?,
                };
                Some(LinkTarget::Uri(s))
            }
            "Named" => {
                let n = action.get::<Name>(b"N")?;
                let last = self.sizes.len().saturating_sub(1);
                let page = match n.as_str() {
                    "FirstPage" => 0,
                    "LastPage" => last,
                    _ => return None,
                };
                Some(LinkTarget::Page { page, y: None })
            }
            _ => None,
        }
    }

    fn outline_level(&self, first: Option<Dict<'_>>, depth: u32, budget: &mut usize) -> Vec<OutlineNode> {
        let mut out = Vec::new();
        let mut cur = first;
        while let Some(item) = cur {
            if *budget == 0 || depth > 64 {
                break;
            }
            *budget -= 1;
            let title = item.get::<Object>(b"Title").and_then(|o| obj_string(&o)).unwrap_or_default();
            let target = item
                .get::<Object>(b"Dest")
                .and_then(|d| self.resolve_dest(d, 0))
                .or_else(|| item.get::<Dict>(b"A").and_then(|a| self.resolve_action(&a)));
            let children = self.outline_level(item.get::<Dict>(b"First"), depth + 1, budget);
            out.push(OutlineNode { title: title.trim().to_string(), target, children });
            cur = item.get::<Dict>(b"Next");
        }
        out
    }

    fn page(&self, i: usize) -> &Page<'_> {
        &self.pdf.pages()[i]
    }
}

fn format_date(d: &hayro::hayro_syntax::object::DateTime) -> String {
    format!("{:04}-{:02}-{:02} {:02}:{:02}", d.year, d.month, d.day, d.hour, d.minute)
}

impl Document for PdfDocument {
    fn kind(&self) -> &'static str {
        "PDF"
    }

    fn page_count(&self) -> usize {
        self.sizes.len()
    }

    fn page_size(&self, page: usize) -> (f32, f32) {
        self.sizes[page]
    }

    fn page_label(&self, page: usize) -> Option<String> {
        self.labels.get(page).cloned().flatten()
    }

    fn renderer(&self) -> Box<dyn PageRenderer + '_> {
        Box::new(PdfRenderer { doc: self, cache: RenderCache::new(), settings: InterpreterSettings::default() })
    }

    fn supports_regions(&self) -> bool {
        true
    }

    fn text(&self, page: usize) -> Option<PageText> {
        let p = self.page(page);
        let (w, h) = self.sizes[page];
        let cache = InterpreterCache::new();
        let transform = p.initial_transform(true).to_kurbo();
        let mut ctx = Context::new(
            transform,
            hayro::kurbo::Rect::new(0.0, 0.0, w as f64, h as f64),
            &cache,
            p.xref(),
            InterpreterSettings::default(),
        );
        let mut dev = TextDevice { builder: TextBuilder::default() };
        interpret_page(p, &mut ctx, &mut dev);
        Some(dev.builder.finish())
    }

    fn links(&self, page: usize) -> Vec<Link> {
        let p = self.page(page);
        let Some(annots) = p.raw().get::<Array>(b"Annots") else {
            return Vec::new();
        };
        let tr = p.initial_transform(true).to_kurbo();
        let mut out = Vec::new();
        for a in annots.iter::<Dict>() {
            if a.get::<Name>(b"Subtype").is_none_or(|s| s.as_str() != "Link") {
                continue;
            }
            let Some(r) = a.get::<hayro::hayro_syntax::object::Rect>(b"Rect") else {
                continue;
            };
            let target = a
                .get::<Dict>(b"A")
                .and_then(|act| self.resolve_action(&act))
                .or_else(|| a.get::<Object>(b"Dest").and_then(|d| self.resolve_dest(d, 0)));
            let Some(target) = target else {
                continue;
            };
            let p0 = tr * Point::new(r.x0, r.y0);
            let p1 = tr * Point::new(r.x1, r.y1);
            out.push(Link { rect: RectF::new(p0.x as f32, p0.y as f32, p1.x as f32, p1.y as f32), target });
        }
        out
    }

    fn outline(&self) -> Vec<OutlineNode> {
        let first = self.catalog().and_then(|c| c.get::<Dict>(b"Outlines")).and_then(|o| o.get::<Dict>(b"First"));
        let mut budget = 20_000;
        self.outline_level(first, 0, &mut budget)
    }

    fn title(&self) -> Option<String> {
        self.pdf.metadata().title.as_ref().map(|t| decode_text(t)).filter(|t| !t.trim().is_empty())
    }

    fn properties(&self) -> Vec<(String, String)> {
        let m = self.pdf.metadata();
        let mut v = Vec::new();
        let mut put = |k: &str, val: &Option<Box<[u8]>>| {
            if let Some(b) = val {
                let s = decode_text(b);
                if !s.trim().is_empty() {
                    v.push((k.to_string(), s));
                }
            }
        };
        put("Title", &m.title);
        put("Author", &m.author);
        put("Subject", &m.subject);
        put("Keywords", &m.keywords);
        put("Creator", &m.creator);
        put("Producer", &m.producer);
        if let Some(d) = &m.creation_date {
            v.push(("Created".into(), format_date(d)));
        }
        if let Some(d) = &m.modification_date {
            v.push(("Modified".into(), format_date(d)));
        }
        v.push(("Format".into(), format!("PDF {:?}", self.pdf.version()).replace("Pdf", "").replace('_', ".")));
        v
    }

    fn pdf_bytes(&self) -> Option<&[u8]> {
        Some(self.pdf.data().as_ref())
    }
}

struct PdfRenderer<'a> {
    doc: &'a PdfDocument,
    cache: RenderCache<'a>,
    settings: InterpreterSettings,
}

impl PageRenderer for PdfRenderer<'_> {
    fn render(&mut self, page: usize, scale: f32, region: Option<(u32, u32, u32, u32)>) -> Option<RgbaImage> {
        let p = self.doc.page(page);
        let (pw, ph) = self.doc.sizes[page];
        let full_w = (pw * scale).round().max(1.0) as u32;
        let full_h = (ph * scale).round().max(1.0) as u32;
        let (x, y, w, h) = region.unwrap_or((0, 0, full_w, full_h));
        if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
            return None;
        }
        let mut ctx = RenderContext::new(w as u16, h as u16);
        let transform = Affine::translate((-(x as f64), -(y as f64)))
            * Affine::scale(scale as f64)
            * p.initial_transform(true).to_kurbo();
        hayro::render_into(p, &self.cache, &self.settings, &RenderSettings::default(), &mut ctx, transform);
        ctx.flush();
        let mut pixmap = Pixmap::new(w as u16, h as u16);
        ctx.render_with(
            &mut pixmap,
            &mut Resources::default(),
            RasterizerSettings { target_init: TargetInit::Clear(WHITE), ..Default::default() },
        );
        Some(RgbaImage { width: w, height: h, data: pixmap.data_as_u8_slice().to_vec() })
    }
}

struct TextDevice {
    builder: TextBuilder,
}

impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(
        &mut self,
        run: &hayro::hayro_interpret::font::GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        _: &DrawMode,
    ) {
        for g in run.glyphs() {
            let Some(u) = g.as_unicode() else {
                continue;
            };
            let s = match u {
                hayro::hayro_interpret::hayro_cmap::BfString::Char(c) => c.to_string(),
                hayro::hayro_interpret::hayro_cmap::BfString::String(s) => s,
            };
            if s.is_empty() {
                continue;
            }
            let adv = match &**g {
                Glyph::Outline(o) => o.advance_width(),
                Glyph::Type3(t) => t.advance_width(),
            }
            .unwrap_or(500.0) as f64;
            let t = props.transform * g.transform();
            let pts = [
                t * Point::new(0.0, -200.0),
                t * Point::new(adv, -200.0),
                t * Point::new(0.0, 800.0),
                t * Point::new(adv, 800.0),
            ];
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for p in pts {
                x0 = x0.min(p.x);
                y0 = y0.min(p.y);
                x1 = x1.max(p.x);
                y1 = y1.max(p.y);
            }
            let origin = t * Point::new(0.0, 0.0);
            let size = (t * Point::new(0.0, 1000.0) - origin).hypot();
            if !x0.is_finite() || size <= 0.0 {
                continue;
            }
            self.builder.push(&s, RectF::new(x0 as f32, y0 as f32, x1 as f32, y1 as f32), origin.y as f32, size as f32);
        }
    }
}
