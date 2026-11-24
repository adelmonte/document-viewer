use document_viewer::app::{App, Startup};
use document_viewer::config::{APP_ID, Settings};
use std::path::PathBuf;

const USAGE: &str = "\
Usage: document-viewer [OPTIONS] [FILE...]

Options:
  -p, --page-label=LABEL   Open at the page with this label (or number)
  -i, --page-index=N       Open at page number N
  -l, --find=TEXT          Search for TEXT
  -f, --fullscreen         Start in fullscreen
  -s, --presentation       Start a presentation
  -h, --help               Show this help
  -V, --version            Show the version

Extra files open in separate windows.";

fn decode_uri(s: &str) -> PathBuf {
    let Some(rest) = s.strip_prefix("file://") else {
        return PathBuf::from(s);
    };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(out))
}

fn icon() -> Option<egui::IconData> {
    let png = include_bytes!("../data/document-viewer.png");
    let img = image::load_from_memory(png).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

fn main() {
    let mut startup = Startup { file: None, page: None, find: None, fullscreen: false, presentation: false };
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut value = || inline.clone().or_else(|| args.next());
        match flag.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            "-V" | "--version" => {
                println!("document-viewer {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "-p" | "--page-label" | "-i" | "--page-index" => startup.page = value(),
            "-l" | "--find" => startup.find = value(),
            "-f" | "--fullscreen" => startup.fullscreen = true,
            "-s" | "--presentation" => startup.presentation = true,
            "--" => files.extend(args.by_ref().map(|s| decode_uri(&s))),
            f if f.starts_with('-') && f.len() > 1 => {
                eprintln!("document-viewer: unknown option {f}\n\n{USAGE}");
                std::process::exit(2);
            }
            _ => files.push(decode_uri(&a)),
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for f in files.iter().skip(1) {
            let _ = std::process::Command::new(&exe).arg(f).spawn();
        }
    }
    startup.file = files.into_iter().next();

    let s = Settings::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Document Viewer")
        .with_app_id(APP_ID)
        .with_inner_size(egui::vec2(s.window_width.max(320.0), s.window_height.max(240.0)))
        .with_min_inner_size(egui::vec2(320.0, 240.0))
        .with_maximized(s.window_maximized)
        .with_drag_and_drop(true);
    if let Some(i) = icon() {
        viewport = viewport.with_icon(i);
    }
    let options = eframe::NativeOptions { viewport, persist_window: false, ..Default::default() };
    if let Err(e) = eframe::run_native(APP_ID, options, Box::new(move |cc| Ok(Box::new(App::new(cc, startup))))) {
        eprintln!("document-viewer: {e}");
        std::process::exit(1);
    }
}
