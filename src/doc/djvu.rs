//! DjVu backend built on djvu-rs.

use super::text::TextBuilder;
use super::*;
use djvu_rs::djvu_render::{RenderRect, RenderRequest};
use djvu_rs::text::{TextZone, TextZoneKind};

pub struct DjvuDocument {
    doc: djvu_rs::Document,
    /// Display size in pixels and dpi for each page.
    pages: Vec<(u32, u32, f32)>,
}

impl DjvuDocument {
    pub fn open(data: Vec<u8>) -> Result<Self, OpenError> {
        let doc = djvu_rs::Document::from_bytes(data).map_err(|e| OpenError::Invalid(e.to_string()))?;
        let mut pages = Vec::new();
        for i in 0..doc.page_count() {
            let p = doc.page(i).map_err(|e| OpenError::Invalid(e.to_string()))?;
            let dpi = if p.dpi() == 0 { 300.0 } else { p.dpi() as f32 };
            pages.push((p.display_width().max(1), p.display_height().max(1), dpi));
        }
        Ok(DjvuDocument { doc, pages })
    }

    fn bookmark_target(&self, url: &str) -> Option<LinkTarget> {
        let s = url.strip_prefix('#')?;
        if let Ok(n) = s.trim().parse::<usize>() {
            return Some(LinkTarget::Page { page: n.saturating_sub(1).min(self.pages.len() - 1), y: None });
        }
        None
    }

    fn convert(&self, b: &[djvu_rs::DjVuBookmark]) -> Vec<OutlineNode> {
        b.iter()
            .map(|b| OutlineNode {
                title: b.title.clone(),
                target: self.bookmark_target(&b.url),
                children: self.convert(&b.children),
            })
            .collect()
    }
}

fn collect_words(z: &TextZone, scale: f32, out: &mut TextBuilder) {
    match z.kind {
        TextZoneKind::Word | TextZoneKind::Character if z.children.is_empty() || z.kind == TextZoneKind::Word => {
            let r = &z.rect;
            let rect = RectF::new(
                r.x as f32 * scale,
                r.y as f32 * scale,
                (r.x + r.width) as f32 * scale,
                (r.y + r.height) as f32 * scale,
            );
            let text = z.text.trim();
            if !text.is_empty() {
                out.push(text, rect, rect.y1, rect.height());
            }
        }
        _ => {
            for c in &z.children {
                collect_words(c, scale, out);
            }
        }
    }
}

impl Document for DjvuDocument {
    fn kind(&self) -> &'static str {
        "DjVu"
    }

    fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn page_size(&self, page: usize) -> (f32, f32) {
        let (w, h, dpi) = self.pages[page];
        (w as f32 * 72.0 / dpi, h as f32 * 72.0 / dpi)
    }

    fn renderer(&self) -> Box<dyn PageRenderer + '_> {
        Box::new(DjvuRenderer { doc: self })
    }

    fn supports_regions(&self) -> bool {
        true
    }

    fn text(&self, page: usize) -> Option<PageText> {
        let p = self.doc.page(page).ok()?;
        let layer = p.text_layer().ok()??;
        let (_, _, dpi) = self.pages[page];
        let mut b = TextBuilder::default();
        for z in &layer.zones {
            collect_words(z, 72.0 / dpi, &mut b);
        }
        Some(b.finish())
    }

    fn outline(&self) -> Vec<OutlineNode> {
        self.doc.bookmarks().map(|b| self.convert(&b)).unwrap_or_default()
    }

    fn properties(&self) -> Vec<(String, String)> {
        let mut v = vec![("Format".to_string(), "DjVu".to_string())];
        if let Some((_, _, dpi)) = self.pages.first() {
            v.push(("Resolution".into(), format!("{dpi} dpi")));
        }
        v
    }
}

struct DjvuRenderer<'a> {
    doc: &'a DjvuDocument,
}

impl PageRenderer for DjvuRenderer<'_> {
    fn render(&mut self, page: usize, scale: f32, region: Option<(u32, u32, u32, u32)>) -> Option<RgbaImage> {
        let p = self.doc.doc.page(page).ok()?;
        let (pw, ph) = self.doc.page_size(page);
        let w = (pw * scale).round().max(1.0) as u32;
        let h = (ph * scale).round().max(1.0) as u32;
        let opts = p.options_for_size(w, h);
        let px = match region {
            Some((x, y, rw, rh)) => p
                .render_request(&RenderRequest::new(opts).region(RenderRect { x, y, width: rw, height: rh }))
                .ok()?,
            None => p.render_with(&opts).ok()?,
        };
        Some(RgbaImage::from_straight(px.width, px.height, px.data))
    }
}
