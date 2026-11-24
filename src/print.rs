//! Printing through the XDG desktop portal (falling back to `lp`), plus a small PDF writer
//! so non-PDF documents can be printed as page images.

use crate::doc::Document;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.Print";

fn token() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("dv{}_{}", std::process::id(), n % 1_000_000_000)
}

/// Calls a portal method and waits for its Request's Response signal.
fn portal_request(
    conn: &Connection,
    method: &str,
    build: impl FnOnce(&str) -> Result<zbus::Message, zbus::Error>,
) -> Result<HashMap<String, OwnedValue>, String> {
    let tok = token();
    let sender = conn.unique_name().ok_or("no D-Bus name")?.as_str().trim_start_matches(':').replace('.', "_");
    let req_path = format!("/org/freedesktop/portal/desktop/request/{sender}/{tok}");
    let req = Proxy::new(conn, DEST, req_path.as_str(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
    let mut signals = req.receive_signal("Response").map_err(|e| e.to_string())?;
    let reply = build(&tok).map_err(|e| format!("{method}: {e}"))?;
    let _: OwnedObjectPath = reply.body().deserialize().map_err(|e| e.to_string())?;
    let msg = signals.next().ok_or("portal closed")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize().map_err(|e| e.to_string())?;
    match code {
        0 => Ok(results),
        1 => Err("cancelled".into()),
        _ => Err("the print portal failed".into()),
    }
}

fn portal_print(pdf: &Path, title: &str) -> Result<(), String> {
    let conn = Connection::session().map_err(|e| e.to_string())?;
    let proxy = Proxy::new(&conn, DEST, PATH, IFACE).map_err(|e| e.to_string())?;
    let empty: HashMap<&str, Value> = HashMap::new();
    let prep = portal_request(&conn, "PreparePrint", |tok| {
        let mut opts: HashMap<&str, Value> = HashMap::new();
        opts.insert("handle_token", Value::from(tok.to_string()));
        opts.insert("modal", Value::from(true));
        proxy.call_method("PreparePrint", &("", title, &empty, &empty, opts))
    })?;
    let print_token: u32 = prep.get("token").and_then(|v| u32::try_from(v.clone()).ok()).unwrap_or(0);
    let file = std::fs::File::open(pdf).map_err(|e| e.to_string())?;
    portal_request(&conn, "Print", |tok| {
        let mut opts: HashMap<&str, Value> = HashMap::new();
        opts.insert("handle_token", Value::from(tok.to_string()));
        opts.insert("token", Value::from(print_token));
        proxy.call_method("Print", &("", title, Fd::from(&file), opts))
    })?;
    Ok(())
}

fn lp_print(pdf: &Path) -> Result<(), String> {
    let out = std::process::Command::new("lp").arg(pdf).output().map_err(|e| format!("lp: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

/// Writes the document as a PDF (the original bytes for PDFs, page images otherwise).
pub fn export_pdf(doc: &dyn Document, out: &Path, dpi: f32) -> std::io::Result<()> {
    if let Some(bytes) = doc.pdf_bytes() {
        return std::fs::write(out, bytes);
    }
    let mut w = PdfWriter::default();
    let mut r = doc.renderer();
    for p in 0..doc.page_count() {
        let (pw, ph) = doc.page_size(p);
        if let Some(img) = r.render(p, dpi / 72.0, None) {
            w.add_page(pw, ph, img.width, img.height, &img.to_straight());
        }
    }
    std::fs::write(out, w.finish())
}

/// Prints a document. Runs synchronously, so call it from a background thread.
pub fn print(doc: &dyn Document, title: &str) -> Result<(), String> {
    let tmp: PathBuf = std::env::temp_dir().join(format!("document-viewer-print-{}.pdf", std::process::id()));
    export_pdf(doc, &tmp, 200.0).map_err(|e| e.to_string())?;
    let result = match portal_print(&tmp, title) {
        Ok(()) => Ok(()),
        Err(e) if e == "cancelled" => Ok(()),
        Err(portal_err) => lp_print(&tmp).map_err(|lp_err| format!("{portal_err}; {lp_err}")),
    };
    let _ = std::fs::remove_file(&tmp);
    result
}

#[derive(Default)]
struct PdfWriter {
    objects: Vec<Vec<u8>>,
    pages: Vec<usize>,
}

impl PdfWriter {
    fn add(&mut self, body: Vec<u8>) -> usize {
        self.objects.push(body);
        self.objects.len() + 2
    }

    fn add_page(&mut self, pw: f32, ph: f32, iw: u32, ih: u32, rgba: &[u8]) {
        let mut rgb = Vec::with_capacity((iw * ih * 3) as usize);
        for px in rgba.chunks_exact(4) {
            let a = px[3] as u32;
            for c in &px[..3] {
                rgb.push(((*c as u32 * a + 255 * (255 - a)) / 255) as u8);
            }
        }
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        let _ = enc.write_all(&rgb);
        let data = enc.finish().unwrap_or_default();
        let mut img = format!(
            "<< /Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            data.len()
        )
        .into_bytes();
        img.extend_from_slice(&data);
        img.extend_from_slice(b"\nendstream");
        let img_id = self.add(img);
        let content = format!("q {pw} 0 0 {ph} 0 0 cm /Im0 Do Q");
        let content_id =
            self.add(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes());
        let page_id = self.add(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw} {ph}] /Resources << /XObject << /Im0 {img_id} 0 R >> >> /Contents {content_id} 0 R >>"
            )
            .into_bytes(),
        );
        self.pages.push(page_id);
    }

    fn finish(self) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::new();
        let kids: Vec<String> = self.pages.iter().map(|p| format!("{p} 0 R")).collect();
        let head = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), self.pages.len()).into_bytes(),
        ];
        for (i, body) in head.iter().chain(self.objects.iter()).enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
        for o in &offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes(),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_readable_pdf() {
        let mut w = PdfWriter::default();
        w.add_page(200.0, 100.0, 2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]);
        w.add_page(100.0, 200.0, 1, 1, &[0, 255, 0, 255]);
        let bytes = w.finish();
        let doc = crate::doc::pdf::PdfDocument::open(bytes, None).unwrap();
        use crate::doc::Document;
        assert_eq!(doc.page_count(), 2);
        assert_eq!(doc.page_size(1), (100.0, 200.0));
        let img = doc.renderer().render(0, 1.0, None).unwrap();
        assert!(img.data[..4][0] > 200 && img.data[..4][2] < 50, "{:?}", &img.data[..4]);
    }
}
