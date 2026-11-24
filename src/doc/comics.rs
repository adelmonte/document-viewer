//! Comic book archives (CBZ/ZIP and CBT/TAR) of page images.

use super::image::ImageDocument;
use super::*;
use std::io::{Cursor, Read};

const IMAGE_EXT: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "tga", "pnm", "ppm", "pgm", "pbm", "qoi"];

fn is_image(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.split('/').any(|c| c.starts_with('.') || c == "__macosx") {
        return false;
    }
    lower.rsplit_once('.').is_some_and(|(_, ext)| IMAGE_EXT.contains(&ext))
}

fn read_zip(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, OpenError> {
    let mut z = zip::ZipArchive::new(Cursor::new(data)).map_err(|e| OpenError::Invalid(e.to_string()))?;
    let mut out = Vec::new();
    for i in 0..z.len() {
        let Ok(mut f) = z.by_index(i) else {
            continue;
        };
        let Ok(name) = f.name().map(|n| n.into_owned()) else {
            continue;
        };
        if !f.is_file() || !is_image(&name) {
            continue;
        }
        let mut buf = Vec::with_capacity(f.size() as usize);
        if f.read_to_end(&mut buf).is_ok() {
            out.push((name, buf));
        }
    }
    Ok(out)
}

fn read_tar(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, OpenError> {
    let mut a = tar::Archive::new(Cursor::new(data));
    let mut out = Vec::new();
    for entry in a.entries().map_err(|e| OpenError::Invalid(e.to_string()))? {
        let Ok(mut e) = entry else {
            continue;
        };
        if !e.header().entry_type().is_file() {
            continue;
        }
        let Some(name) = e.path().ok().map(|p| p.to_string_lossy().into_owned()) else {
            continue;
        };
        if !is_image(&name) {
            continue;
        }
        let mut buf = Vec::new();
        if e.read_to_end(&mut buf).is_ok() {
            out.push((name, buf));
        }
    }
    Ok(out)
}

pub fn open(data: Vec<u8>) -> Result<ImageDocument, OpenError> {
    let mut entries = if data.starts_with(b"PK") { read_zip(&data)? } else { read_tar(&data)? };
    entries.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    ImageDocument::from_archive(entries)
}
