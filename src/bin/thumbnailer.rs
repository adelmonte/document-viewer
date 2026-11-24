//! Thumbnailer for file managers: `document-viewer-thumbnailer [-s SIZE] INPUT OUTPUT.png`.

use document_viewer::doc;
use std::path::PathBuf;

fn input_path(s: &str) -> PathBuf {
    match s.strip_prefix("file://") {
        Some(rest) => {
            let mut out = Vec::new();
            let b = rest.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'%'
                    && i + 2 < b.len()
                    && let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
                {
                    out.push(v);
                    i += 3;
                } else {
                    out.push(b[i]);
                    i += 1;
                }
            }
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(out))
        }
        None => PathBuf::from(s),
    }
}

fn main() {
    let mut size = 256u32;
    let mut pos = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-s" | "--size" => size = args.next().and_then(|v| v.parse().ok()).unwrap_or(size),
            "-h" | "--help" => {
                println!("Usage: document-viewer-thumbnailer [-s SIZE] INPUT OUTPUT");
                return;
            }
            _ => pos.push(a),
        }
    }
    let [input, output] = &pos[..] else {
        eprintln!("Usage: document-viewer-thumbnailer [-s SIZE] INPUT OUTPUT");
        std::process::exit(2);
    };
    let d = match doc::open(&input_path(input), None) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("document-viewer-thumbnailer: {e}");
            std::process::exit(1);
        }
    };
    let (w, h) = d.page_size(0);
    let scale = size as f32 / w.max(h).max(1.0);
    let Some(img) = d.renderer().render(0, scale, None) else {
        eprintln!("document-viewer-thumbnailer: could not render the first page");
        std::process::exit(1);
    };
    let buf = image::RgbaImage::from_raw(img.width, img.height, img.to_straight()).unwrap();
    if let Err(e) = buf.save_with_format(output, image::ImageFormat::Png) {
        eprintln!("document-viewer-thumbnailer: {e}");
        std::process::exit(1);
    }
}
