pub mod comics;
pub mod djvu;
pub mod dvi;
pub mod image;
pub mod pdf;
pub mod text;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use text::{PageText, TextChar};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl RectF {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        RectF { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) }
    }
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
    pub fn union(&self, o: &RectF) -> RectF {
        RectF { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    pub fn intersects(&self, o: &RectF) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
}

/// Premultiplied RGBA8 pixels.
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaImage {
    pub fn from_straight(width: u32, height: u32, mut data: Vec<u8>) -> Self {
        for px in data.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a != 255 {
                px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
                px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
                px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
            }
        }
        RgbaImage { width, height, data }
    }

    pub fn from_dynamic(img: ::image::DynamicImage) -> Self {
        let rgba = img.into_rgba8();
        let (w, h) = rgba.dimensions();
        Self::from_straight(w, h, rgba.into_raw())
    }

    pub fn resized(&self, width: u32, height: u32) -> RgbaImage {
        let width = width.max(1);
        let height = height.max(1);
        if width == self.width && height == self.height {
            return RgbaImage { width, height, data: self.data.clone() };
        }
        let buf = ::image::RgbaImage::from_raw(self.width, self.height, self.data.clone()).unwrap();
        let filter = if width < self.width { ::image::imageops::FilterType::Triangle } else { ::image::imageops::FilterType::CatmullRom };
        let out = ::image::imageops::resize(&buf, width, height, filter);
        RgbaImage { width, height, data: out.into_raw() }
    }

    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width - x).max(1);
        let h = h.min(self.height - y).max(1);
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            let start = ((row * self.width + x) * 4) as usize;
            data.extend_from_slice(&self.data[start..start + (w * 4) as usize]);
        }
        RgbaImage { width: w, height: h, data }
    }

    pub fn invert(&mut self) {
        for px in self.data.chunks_exact_mut(4) {
            let a = px[3];
            px[0] = a - px[0];
            px[1] = a - px[1];
            px[2] = a - px[2];
        }
    }

    pub fn to_straight(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        for px in out.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a != 0 && a != 255 {
                px[0] = ((px[0] as u32 * 255 + a / 2) / a).min(255) as u8;
                px[1] = ((px[1] as u32 * 255 + a / 2) / a).min(255) as u8;
                px[2] = ((px[2] as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    Page { page: usize, y: Option<f32> },
    Uri(String),
}

#[derive(Clone, Debug)]
pub struct Link {
    pub rect: RectF,
    pub target: LinkTarget,
}

#[derive(Clone, Debug)]
pub struct OutlineNode {
    pub title: String,
    pub target: Option<LinkTarget>,
    pub children: Vec<OutlineNode>,
}

/// Renders pages. One renderer is created per worker thread so backends can keep caches.
pub trait PageRenderer {
    /// Renders `page` at `scale` pixels per point. With `region` (x, y, w, h in pixels at that
    /// scale), only that part of the page is rendered.
    fn render(&mut self, page: usize, scale: f32, region: Option<(u32, u32, u32, u32)>) -> Option<RgbaImage>;
}

pub trait Document: Send + Sync {
    fn kind(&self) -> &'static str;
    fn page_count(&self) -> usize;
    /// Page size in points (1/72 inch), with any intrinsic rotation already applied.
    fn page_size(&self, page: usize) -> (f32, f32);
    fn page_label(&self, _page: usize) -> Option<String> {
        None
    }
    fn renderer(&self) -> Box<dyn PageRenderer + '_>;
    /// Whether rendering sub-regions is cheaper than rendering whole pages.
    fn supports_regions(&self) -> bool {
        false
    }
    fn text(&self, _page: usize) -> Option<PageText> {
        None
    }
    fn links(&self, _page: usize) -> Vec<Link> {
        Vec::new()
    }
    fn outline(&self) -> Vec<OutlineNode> {
        Vec::new()
    }
    fn properties(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    fn title(&self) -> Option<String> {
        None
    }
    /// The document as a PDF file, if it is one (used for printing).
    fn pdf_bytes(&self) -> Option<&[u8]> {
        None
    }
}

#[derive(Debug)]
pub enum OpenError {
    NeedsPassword,
    WrongPassword,
    Unsupported(String),
    Io(std::io::Error),
    Invalid(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::NeedsPassword => write!(f, "This document is password protected"),
            OpenError::WrongPassword => write!(f, "The password is incorrect"),
            OpenError::Unsupported(s) => write!(f, "Unsupported document type: {s}"),
            OpenError::Io(e) => write!(f, "{e}"),
            OpenError::Invalid(s) => write!(f, "{s}"),
        }
    }
}

impl From<std::io::Error> for OpenError {
    fn from(e: std::io::Error) -> Self {
        OpenError::Io(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Djvu,
    Dvi,
    Tiff,
    Image,
    Comics,
}

pub const EXTENSIONS: &[&str] = &[
    "pdf", "djvu", "djv", "dvi", "tif", "tiff", "png", "jpg", "jpeg", "gif", "webp", "bmp", "pnm", "pbm", "pgm", "ppm",
    "tga", "qoi", "ico", "cbz", "cbt", "zip", "tar", "gz", "bz2", "xz",
];

fn sniff(data: &[u8]) -> Option<Format> {
    if data.len() < 8 {
        return None;
    }
    let head = &data[..data.len().min(1024)];
    if head.windows(5).any(|w| w == b"%PDF-") {
        return Some(Format::Pdf);
    }
    if &data[..4] == b"AT&T" || &data[..4] == b"FORM" && data.len() > 12 && &data[8..11] == b"DJV" {
        return Some(Format::Djvu);
    }
    if data[0] == 0xF7 && data[1] == 2 {
        return Some(Format::Dvi);
    }
    if &data[..4] == b"II*\0" || &data[..4] == b"MM\0*" {
        return Some(Format::Tiff);
    }
    if &data[..4] == b"PK\x03\x04" {
        return Some(Format::Comics);
    }
    if data.len() > 262 && &data[257..262] == b"ustar" {
        return Some(Format::Comics);
    }
    if ::image::guess_format(data).is_ok() {
        return Some(Format::Image);
    }
    None
}

fn format_from_extension(path: &Path) -> Option<Format> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "pdf" => Format::Pdf,
        "djvu" | "djv" => Format::Djvu,
        "dvi" => Format::Dvi,
        "tif" | "tiff" => Format::Tiff,
        "cbz" | "cbt" | "zip" | "tar" => Format::Comics,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "pnm" | "pbm" | "pgm" | "ppm" | "tga" | "qoi" | "ico" => {
            Format::Image
        }
        _ => return None,
    })
}

/// Undoes gzip/bzip2/xz compression of a whole document (e.g. `paper.pdf.gz`).
fn decompress(path: &Path, data: Vec<u8>) -> std::io::Result<(Vec<u8>, PathBuf)> {
    use std::io::Read;
    let stem = path.with_extension("");
    if data.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(&data[..]).read_to_end(&mut out)?;
        return Ok((out, stem));
    }
    if data.starts_with(b"BZh") {
        let mut out = Vec::new();
        bzip2::read::MultiBzDecoder::new(&data[..]).read_to_end(&mut out)?;
        return Ok((out, stem));
    }
    if data.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0]) {
        let mut out = Vec::new();
        lzma_rust2::XzReader::new(&data[..], true).read_to_end(&mut out)?;
        return Ok((out, stem));
    }
    Ok((data, path.to_path_buf()))
}

pub fn open(path: &Path, password: Option<&str>) -> Result<Arc<dyn Document>, OpenError> {
    let raw = std::fs::read(path)?;
    let (data, effective) = decompress(path, raw)?;
    let format = sniff(&data)
        .or_else(|| format_from_extension(&effective))
        .ok_or_else(|| OpenError::Unsupported(effective.display().to_string()))?;
    let doc: Arc<dyn Document> = match format {
        Format::Pdf => Arc::new(pdf::PdfDocument::open(data, password)?),
        Format::Djvu => Arc::new(djvu::DjvuDocument::open(data)?),
        Format::Dvi => Arc::new(dvi::DviDocument::open(data, path)?),
        Format::Tiff => Arc::new(image::ImageDocument::open_tiff(data)?),
        Format::Image => Arc::new(image::ImageDocument::open_single(data)?),
        Format::Comics => Arc::new(comics::open(data)?),
    };
    if doc.page_count() == 0 {
        return Err(OpenError::Invalid("The document contains no pages".into()));
    }
    Ok(doc)
}

/// Compares two file names the way a human would (numbers compared by value).
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    bi.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}
