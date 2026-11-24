use document_viewer::doc::text::{FindOptions, SelectionStyle};
use document_viewer::doc::{self, LinkTarget, OpenError, RgbaImage};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn non_white(img: &RgbaImage) -> usize {
    img.data.chunks_exact(4).filter(|p| p[0] < 250 || p[1] < 250 || p[2] < 250).count()
}

#[test]
fn pdf_backend() {
    let d = doc::open(&fixture("sample.pdf"), None).unwrap();
    assert_eq!(d.kind(), "PDF");
    assert_eq!(d.page_count(), 3);
    assert_eq!(d.page_size(0), (612.0, 792.0));
    assert_eq!(d.title().as_deref(), Some("Sample Title"));
    assert!(d.properties().iter().any(|(k, v)| k == "Author" && v == "Test Author"));
    assert!(d.pdf_bytes().is_some());

    let mut r = d.renderer();
    let img = r.render(0, 0.5, None).unwrap();
    assert_eq!((img.width, img.height), (306, 396));
    assert!(non_white(&img) > 100);
    let tile = r.render(0, 2.0, Some((0, 0, 256, 256))).unwrap();
    assert_eq!((tile.width, tile.height), (256, 256));

    let text = d.text(1).unwrap();
    assert!(text.string().contains("findme"), "{}", text.string());
    let hits = text.find("FINDME", FindOptions::default());
    assert_eq!(hits.len(), 1);
    let rect = text.range_rects(hits[0].0, hits[0].1)[0];
    assert!(rect.y0 > 100.0 && rect.y1 < 160.0, "find rect in top-left coordinates: {rect:?}");
    assert_eq!(text.find("FINDME", FindOptions { case_sensitive: true, whole_words: false }).len(), 0);
    assert!(d.text(0).unwrap().find("findme", FindOptions::default()).is_empty());

    let t0 = d.text(0).unwrap();
    let (a, b) = t0.range_between(SelectionStyle::Line, (80.0, 90.0), (80.0, 90.0)).unwrap();
    assert!(t0.range_text(a, b).contains("Hello World page 1"), "{:?}", t0.range_text(a, b));

    let links = d.links(0);
    let ext = links.iter().find(|l| l.target == LinkTarget::Uri("https://example.org/".into())).expect("uri link");
    assert!(ext.rect.y0 > 170.0 && ext.rect.y1 < 220.0, "{:?}", ext.rect);
    assert!(links.iter().any(|l| matches!(l.target, LinkTarget::Page { page: 2, .. })), "{links:?}");

    let outline = d.outline();
    assert_eq!(outline.len(), 2);
    assert_eq!(outline[0].title, "Chapter 1");
    assert_eq!(outline[0].children[0].title, "Section 1.1");
    assert_eq!(d.page_label(0).as_deref(), Some("i"));
}

#[test]
fn gzipped_pdf() {
    let d = doc::open(&fixture("sample.pdf.gz"), None).unwrap();
    assert_eq!(d.page_count(), 3);
}

#[test]
fn comics_backend() {
    let d = doc::open(&fixture("sample.cbz"), None).unwrap();
    assert_eq!(d.page_count(), 3, "junk and non-image entries skipped");
    assert_eq!(d.page_size(0), (300.0, 400.0), "pages sorted by name");
    assert_eq!(d.page_size(2), (400.0, 400.0));
    let img = d.renderer().render(1, 0.5, None).unwrap();
    assert_eq!((img.width, img.height), (175, 200));
    assert!(non_white(&img) > 50);
}

#[test]
fn tiff_backend() {
    let d = doc::open(&fixture("sample.tiff"), None).unwrap();
    assert_eq!(d.page_count(), 2);
    assert_eq!(d.page_size(0), (300.0, 400.0));
    assert_eq!(d.page_size(1), (350.0, 400.0));
    let img = d.renderer().render(1, 1.0, None).unwrap();
    assert_eq!((img.width, img.height), (350, 400));
    assert!(non_white(&img) > 100);
}

#[test]
fn djvu_backend() {
    let d = doc::open(&fixture("sample.djvu"), None).unwrap();
    assert_eq!(d.page_count(), 2);
    let (w, h) = d.page_size(0);
    assert!((w - 216.0).abs() < 1.0 && (h - 288.0).abs() < 1.0, "{w}x{h}");
    let img = d.renderer().render(0, 1.0, None).unwrap();
    assert_eq!((img.width, img.height), (216, 288));
    assert!(non_white(&img) > 100);
    let tile = d.renderer().render(0, 1.0, Some((10, 10, 50, 50))).unwrap();
    assert_eq!((tile.width, tile.height), (50, 50));
}

#[test]
fn unsupported_type() {
    let p = std::env::temp_dir().join(format!("document-viewer-plain-{}.txt", std::process::id()));
    std::fs::write(&p, "just text, nothing else").unwrap();
    assert!(matches!(doc::open(&p, None), Err(OpenError::Unsupported(_))));
    let _ = std::fs::remove_file(p);
}
