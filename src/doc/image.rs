//! Raster backends: single images, multi-page TIFF and (via `comics`) image archives.

use super::*;
use std::io::Cursor;
use std::sync::Mutex;

/// Points per image pixel for images without a resolution.
pub const PX_TO_PT: f32 = 1.0;

pub enum Source {
    Single(Arc<RgbaImage>),
    Tiff(Vec<u8>),
    Archive(Vec<Vec<u8>>),
}

pub struct ImageDocument {
    kind: &'static str,
    source: Source,
    sizes: Vec<(f32, f32)>,
    names: Vec<String>,
}

fn decode_bytes(bytes: &[u8]) -> Option<RgbaImage> {
    let img = ::image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.decode().ok()?;
    Some(RgbaImage::from_dynamic(img))
}

fn expand_bits(data: &[u8], w: usize, h: usize, bits: u8, channels: usize) -> Vec<u8> {
    let row_bits = w * channels * bits as usize;
    let stride = row_bits.div_ceil(8);
    let max = (1u32 << bits) - 1;
    let mut out = Vec::with_capacity(w * h * channels);
    for y in 0..h {
        let row = &data[(y * stride).min(data.len())..((y + 1) * stride).min(data.len())];
        for i in 0..w * channels {
            let bit = i * bits as usize;
            let byte = row.get(bit / 8).copied().unwrap_or(0) as u32;
            let shift = 8 - bits as usize - (bit % 8);
            let v = (byte >> shift) & max;
            out.push((v * 255 / max) as u8);
        }
    }
    out
}

fn tiff_page(data: &[u8], index: usize) -> Option<(RgbaImage, f32)> {
    use tiff::ColorType as C;
    use tiff::decoder::{Decoder, DecodingResult as R, Limits};
    use tiff::tags::Tag;
    let mut dec = Decoder::new(Cursor::new(data)).ok()?.with_limits(Limits::unlimited());
    dec.seek_to_image(index).ok()?;
    let (w, h) = dec.dimensions().ok()?;
    let ct = dec.colortype().ok()?;
    let photometric = dec.find_tag_unsigned::<u16>(Tag::PhotometricInterpretation).ok().flatten().unwrap_or(1);
    let dpi = dec
        .find_tag(Tag::XResolution)
        .ok()
        .flatten()
        .and_then(|v| v.into_f64().ok())
        .filter(|d| *d > 1.0)
        .map(|d| {
            let unit = dec.find_tag_unsigned::<u16>(Tag::ResolutionUnit).ok().flatten().unwrap_or(2);
            if unit == 3 { d * 2.54 } else { d }
        });
    let palette = dec.find_tag_unsigned_vec::<u16>(Tag::ColorMap).ok().flatten();
    let img = dec.read_image().ok()?;
    let (w, h) = (w as usize, h as usize);
    let to8: Vec<u8> = match img {
        R::U8(v) => v,
        R::U16(v) => v.iter().map(|x| (x >> 8) as u8).collect(),
        R::U32(v) => v.iter().map(|x| (x >> 24) as u8).collect(),
        R::F32(v) => v.iter().map(|x| (x.clamp(0.0, 1.0) * 255.0) as u8).collect(),
        _ => return None,
    };
    let (channels, bits) = match ct {
        C::Gray(b) => (1, b),
        C::GrayA(b) => (2, b),
        C::RGB(b) | C::YCbCr(b) => (3, b),
        C::RGBA(b) => (4, b),
        C::CMYK(b) => (4, b),
        C::Palette(b) => (1, b),
        _ => return None,
    };
    let samples = if bits < 8 {
        if matches!(ct, C::Palette(_)) {
            // Keep raw indices for palette lookup.
            let row_bits = w * bits as usize;
            let stride = row_bits.div_ceil(8);
            let mut out = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..w {
                    let bit = x * bits as usize;
                    let byte = to8.get(y * stride + bit / 8).copied().unwrap_or(0);
                    out.push((byte >> (8 - bits as usize - bit % 8)) & ((1 << bits) - 1));
                }
            }
            out
        } else {
            expand_bits(&to8, w, h, bits, channels)
        }
    } else {
        to8
    };
    let mut rgba = Vec::with_capacity(w * h * 4);
    let n = w * h;
    match ct {
        C::Gray(_) => {
            let inv = photometric == 0;
            for &g in samples.iter().take(n) {
                let g = if inv { 255 - g } else { g };
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
        }
        C::GrayA(_) => {
            for p in samples.chunks_exact(2).take(n) {
                rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        C::RGB(_) | C::YCbCr(_) => {
            for p in samples.chunks_exact(3).take(n) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        C::RGBA(_) => rgba = samples,
        C::CMYK(_) => {
            for p in samples.chunks_exact(4).take(n) {
                let k = 255 - p[3] as u32;
                let f = |c: u8| ((255 - c as u32) * k / 255) as u8;
                rgba.extend_from_slice(&[f(p[0]), f(p[1]), f(p[2]), 255]);
            }
        }
        C::Palette(_) => {
            let pal = palette?;
            let count = pal.len() / 3;
            for &i in samples.iter().take(n) {
                let i = (i as usize).min(count.saturating_sub(1));
                let c = |k: usize| (pal.get(k * count + i).copied().unwrap_or(0) >> 8) as u8;
                rgba.extend_from_slice(&[c(0), c(1), c(2), 255]);
            }
        }
        _ => return None,
    }
    rgba.resize(n * 4, 255);
    let scale = dpi.map(|d| 72.0 / d as f32).unwrap_or(PX_TO_PT);
    Some((RgbaImage::from_straight(w as u32, h as u32, rgba), scale))
}

fn tiff_info(data: &[u8]) -> Option<Vec<(f32, f32)>> {
    use tiff::decoder::{Decoder, Limits};
    use tiff::tags::Tag;
    let mut dec = Decoder::new(Cursor::new(data)).ok()?.with_limits(Limits::unlimited());
    let mut sizes = Vec::new();
    loop {
        let (w, h) = dec.dimensions().ok()?;
        let dpi = dec.find_tag(Tag::XResolution).ok().flatten().and_then(|v| v.into_f64().ok()).filter(|d| *d > 1.0);
        let unit = dec.find_tag_unsigned::<u16>(Tag::ResolutionUnit).ok().flatten().unwrap_or(2);
        let scale = dpi.map(|d| 72.0 / (if unit == 3 { d * 2.54 } else { d }) as f32).unwrap_or(PX_TO_PT);
        sizes.push((w as f32 * scale, h as f32 * scale));
        if !dec.more_images() || dec.next_image().is_err() {
            break;
        }
    }
    Some(sizes)
}

impl ImageDocument {
    pub fn open_single(data: Vec<u8>) -> Result<Self, OpenError> {
        let img = decode_bytes(&data).ok_or_else(|| OpenError::Invalid("The image could not be decoded".into()))?;
        let sizes = vec![(img.width as f32 * PX_TO_PT, img.height as f32 * PX_TO_PT)];
        Ok(ImageDocument { kind: "Image", source: Source::Single(Arc::new(img)), sizes, names: Vec::new() })
    }

    pub fn open_tiff(data: Vec<u8>) -> Result<Self, OpenError> {
        let sizes = tiff_info(&data).ok_or_else(|| OpenError::Invalid("The TIFF file could not be read".into()))?;
        Ok(ImageDocument { kind: "TIFF", source: Source::Tiff(data), sizes, names: Vec::new() })
    }

    pub fn from_archive(entries: Vec<(String, Vec<u8>)>) -> Result<Self, OpenError> {
        let mut names = Vec::new();
        let mut pages = Vec::new();
        let mut sizes = Vec::new();
        for (name, bytes) in entries {
            let Ok(reader) = ::image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format() else {
                continue;
            };
            let Ok((w, h)) = reader.into_dimensions() else {
                continue;
            };
            sizes.push((w as f32 * PX_TO_PT, h as f32 * PX_TO_PT));
            names.push(name);
            pages.push(bytes);
        }
        if pages.is_empty() {
            return Err(OpenError::Invalid("The archive contains no images".into()));
        }
        Ok(ImageDocument { kind: "Comic book", source: Source::Archive(pages), sizes, names })
    }

    fn full_page(&self, page: usize) -> Option<Arc<RgbaImage>> {
        match &self.source {
            Source::Single(img) => Some(img.clone()),
            Source::Tiff(data) => tiff_page(data, page).map(|(i, _)| Arc::new(i)),
            Source::Archive(pages) => decode_bytes(pages.get(page)?).map(Arc::new),
        }
    }
}

impl Document for ImageDocument {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn page_count(&self) -> usize {
        self.sizes.len()
    }

    fn page_size(&self, page: usize) -> (f32, f32) {
        self.sizes[page]
    }

    fn page_label(&self, page: usize) -> Option<String> {
        self.names.get(page).map(|n| n.rsplit('/').next().unwrap_or(n).to_string())
    }

    fn renderer(&self) -> Box<dyn PageRenderer + '_> {
        Box::new(ImageRenderer { doc: self, last: Mutex::new(None) })
    }

    fn properties(&self) -> Vec<(String, String)> {
        let mut v = vec![("Format".to_string(), self.kind.to_string())];
        if let Source::Single(img) = &self.source {
            v.push(("Dimensions".into(), format!("{} × {} pixels", img.width, img.height)));
        }
        v
    }
}

struct ImageRenderer<'a> {
    doc: &'a ImageDocument,
    last: Mutex<Option<(usize, Arc<RgbaImage>)>>,
}

impl PageRenderer for ImageRenderer<'_> {
    fn render(&mut self, page: usize, scale: f32, region: Option<(u32, u32, u32, u32)>) -> Option<RgbaImage> {
        let src = {
            let mut last = self.last.lock().unwrap();
            match &*last {
                Some((p, img)) if *p == page => img.clone(),
                _ => {
                    let img = self.doc.full_page(page)?;
                    *last = Some((page, img.clone()));
                    img
                }
            }
        };
        let (pw, ph) = self.doc.sizes[page];
        let w = (pw * scale).round().max(1.0) as u32;
        let h = (ph * scale).round().max(1.0) as u32;
        let out = src.resized(w, h);
        Some(match region {
            Some((x, y, rw, rh)) => out.crop(x, y, rw, rh),
            None => out,
        })
    }
}
