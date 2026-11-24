//! The application window: header bar, find bar, sidebar, dialogs and keyboard handling.

use crate::config::{Bookmark, DocState, DocStore, Settings, Sizing};
use crate::doc::{self, Document, LinkTarget, OpenError};
use crate::icons::{self, Icon};
use crate::sidebar::{self, Sidebar};
use crate::theme::{Mode, Theme};
use crate::view::{Position, View};
use egui::{Align, Align2, Color32, Frame, Key, Layout, Margin, RichText, Ui, ViewportCommand, vec2};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Instant, SystemTime};

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Open,
    OpenPath(PathBuf),
    NewWindow,
    SaveCopy,
    Print,
    Properties,
    Close,
    Reload,
    OpenFolder,
    ToggleSidebar,
    Find,
    FindNext,
    FindPrev,
    Escape,
    Copy,
    SelectAll,
    CopyText(String),
    Follow(LinkTarget),
    ZoomIn,
    ZoomOut,
    Zoom(f32),
    Sizing(Sizing),
    ToggleContinuous,
    ToggleDual,
    ToggleOddLeft,
    ToggleInvert,
    Rotate(bool),
    Fullscreen,
    Presentation,
    NextPage,
    PrevPage,
    FirstPage,
    LastPage,
    GoToPage(usize),
    Back,
    Forward,
    Scroll(f32, f32),
    Screen(bool),
    AddBookmark,
    FocusPageEntry,
    Theme(Mode),
    EditTheme,
    Shortcuts,
    About,
}

enum Dialog {
    Password { path: PathBuf, text: String, wrong: bool },
    Error(String),
    Properties,
    Shortcuts,
    About,
}

struct OpenDoc {
    path: PathBuf,
    password: Option<String>,
    view: View,
    bookmarks: Vec<Bookmark>,
    title: String,
    mtime: Option<SystemTime>,
}

/// View settings carried over when a document is reloaded.
struct Keep {
    pos: Position,
    continuous: bool,
    dual: bool,
    odd_left: bool,
    sizing: Sizing,
    zoom: f32,
    rotation: u32,
    invert: bool,
}

struct Loading {
    path: PathBuf,
    password: Option<String>,
    rx: Receiver<Result<Arc<dyn Document>, OpenError>>,
    keep: Option<Keep>,
}

pub struct Startup {
    pub file: Option<PathBuf>,
    pub page: Option<String>,
    pub find: Option<String>,
    pub fullscreen: bool,
    pub presentation: bool,
}

pub struct App {
    settings: Settings,
    theme: Theme,
    doc: Option<OpenDoc>,
    loading: Option<Loading>,
    sidebar: Sidebar,
    dialog: Option<Dialog>,
    find_open: bool,
    find_query: String,
    find_case: bool,
    find_whole: bool,
    find_focus: bool,
    find_changed: Option<Instant>,
    page_entry: String,
    page_entry_id: egui::Id,
    focus_page_entry: bool,
    fullscreen: bool,
    presentation: bool,
    toast: Option<(String, Instant)>,
    file_rx: Option<Receiver<Option<PathBuf>>>,
    save_rx: Option<Receiver<Option<PathBuf>>>,
    print_rx: Option<Receiver<Result<(), String>>>,
    sent_title: String,
    last_mtime_check: Instant,
    startup: Option<Startup>,
    window: (f32, f32, bool),
    recent: Vec<PathBuf>,
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub fn open_external(target: &str) {
    let _ = std::process::Command::new("xdg-open")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn spawn_window(path: Option<&Path>) {
    if let Ok(exe) = std::env::current_exe() {
        let mut c = std::process::Command::new(exe);
        if let Some(p) = path {
            c.arg(p);
        }
        let _ = c.spawn();
    }
}

fn show_in_folder(path: &Path) {
    let uri = format!("file://{}", path.display());
    let ok = zbus::blocking::Connection::session()
        .and_then(|c| {
            c.call_method(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                Some("org.freedesktop.FileManager1"),
                "ShowItems",
                &(vec![uri.as_str()], ""),
            )
        })
        .is_ok();
    if !ok && let Some(dir) = path.parent() {
        open_external(&dir.to_string_lossy());
    }
}

fn paper_name(w_mm: f32, h_mm: f32) -> Option<&'static str> {
    const SIZES: &[(&str, f32, f32)] = &[
        ("A3", 297.0, 420.0),
        ("A4", 210.0, 297.0),
        ("A5", 148.0, 210.0),
        ("B5", 176.0, 250.0),
        ("US Letter", 215.9, 279.4),
        ("US Legal", 215.9, 355.6),
        ("Tabloid", 279.4, 431.8),
    ];
    let (a, b) = (w_mm.min(h_mm), w_mm.max(h_mm));
    SIZES.iter().find(|(_, w, h)| (a - w).abs() < 2.0 && (b - h).abs() < 2.0).map(|s| s.0)
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, startup: Startup) -> App {
        let ctx = &cc.egui_ctx;
        ctx.options_mut(|o| {
            o.zoom_with_keyboard = false;
        });
        let settings = Settings::load();
        let mut theme = Theme::load();
        theme.update(ctx);
        let mut app = App {
            window: (settings.window_width, settings.window_height, settings.window_maximized),
            sidebar: Sidebar::new(),
            settings,
            theme,
            doc: None,
            loading: None,
            dialog: None,
            find_open: false,
            find_query: String::new(),
            find_case: false,
            find_whole: false,
            find_focus: false,
            find_changed: None,
            page_entry: String::new(),
            page_entry_id: egui::Id::unique("page-entry"),
            focus_page_entry: false,
            fullscreen: false,
            presentation: false,
            toast: None,
            file_rx: None,
            save_rx: None,
            print_rx: None,
            sent_title: String::new(),
            last_mtime_check: Instant::now(),
            startup: None,
            recent: DocStore::load().recent(12),
        };
        if let Some(f) = &startup.file {
            app.open(ctx, f.clone(), None, None);
        }
        app.fullscreen = startup.fullscreen;
        if startup.fullscreen {
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
        }
        app.startup = Some(startup);
        app
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    // ---- documents ----

    fn open(&mut self, ctx: &egui::Context, path: PathBuf, password: Option<String>, keep: Option<Keep>) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let (tx, rx) = channel();
        let p = path.clone();
        let pw = password.clone();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let r = doc::open(&p, pw.as_deref());
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        self.loading = Some(Loading { path, password, rx, keep });
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let Some(l) = &self.loading else {
            return;
        };
        let Ok(result) = l.rx.try_recv() else {
            return;
        };
        let l = self.loading.take().unwrap();
        match result {
            Ok(d) => self.install(ctx, l.path, l.password, d, l.keep),
            Err(OpenError::NeedsPassword) => {
                self.dialog = Some(Dialog::Password { path: l.path, text: String::new(), wrong: false })
            }
            Err(OpenError::WrongPassword) => {
                self.dialog = Some(Dialog::Password { path: l.path, text: String::new(), wrong: true })
            }
            Err(e) => self.dialog = Some(Dialog::Error(format!("Unable to open “{}”\n\n{e}", file_name(&l.path)))),
        }
    }

    fn install(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        password: Option<String>,
        d: Arc<dyn Document>,
        keep: Option<Keep>,
    ) {
        self.save_doc_state();
        let mut view = View::new(d.clone(), ctx, self.settings.render_threads);
        let s = &self.settings;
        view.page_gap = self.theme.file.metrics.page_gap;
        view.invert = s.invert_colors;
        let mut bookmarks = Vec::new();
        let reloading = keep.is_some();
        if let Some(k) = keep {
            view.continuous = k.continuous;
            view.dual = k.dual;
            view.odd_left = k.odd_left;
            view.sizing = k.sizing;
            view.zoom = k.zoom;
            view.rotation = k.rotation;
            view.invert = k.invert;
            view.go_to_position(k.pos);
            if let Some(o) = &self.doc {
                bookmarks = o.bookmarks.clone();
            }
        } else {
            let state = DocStore::load().get(&path);
            view.continuous = state.as_ref().and_then(|s| s.continuous).unwrap_or(s.continuous);
            view.dual = state.as_ref().and_then(|s| s.dual_page).unwrap_or(s.dual_page);
            view.odd_left = s.dual_odd_left;
            view.sizing = state.as_ref().and_then(|s| s.sizing).unwrap_or(s.sizing);
            view.zoom = state.as_ref().and_then(|s| s.zoom).unwrap_or(s.zoom);
            if let Some(st) = &state {
                view.rotation = st.rotation % 360;
                bookmarks = st.bookmarks.clone();
                if s.restore_position {
                    view.go_to_position(Position { page: st.page, fraction: st.page_offset });
                }
                if let Some(v) = st.sidebar_visible {
                    self.settings.sidebar_visible = v;
                }
            }
        }
        let title = d.title().unwrap_or_else(|| file_name(&path));
        if !reloading {
            self.sidebar.reset();
            if let Some(st) = self.startup.take() {
                if let Some(label) = &st.page
                    && let Some(p) = Self::parse_page(&*d, label)
                {
                    view.go_to_page(p);
                }
                if st.presentation {
                    self.presentation = true;
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
                }
                if let Some(q) = st.find {
                    self.find_open = true;
                    self.find_query = q;
                    self.find_changed = Some(Instant::now() - std::time::Duration::from_secs(1));
                }
            }
        }
        if let Some(old) = &self.doc
            && reloading
            && let Some(s) = &old.view.search
        {
            self.find_query = s.query.clone();
            self.find_changed = Some(Instant::now() - std::time::Duration::from_secs(1));
        }
        self.doc = Some(OpenDoc { mtime: mtime(&path), path: path.clone(), password, view, bookmarks, title });
        if !reloading {
            DocStore::save_one(&path, self.doc_state().unwrap_or_default());
            self.recent = DocStore::load().recent(12);
        }
    }

    fn doc_state(&self) -> Option<DocState> {
        let d = self.doc.as_ref()?;
        let v = &d.view;
        let pos = v.position();
        Some(DocState {
            page: pos.page,
            page_offset: pos.fraction,
            sizing: Some(v.sizing),
            zoom: Some(v.zoom),
            rotation: v.rotation,
            continuous: Some(v.continuous),
            dual_page: Some(v.dual),
            sidebar_visible: Some(self.settings.sidebar_visible),
            bookmarks: d.bookmarks.clone(),
            last_opened: 0,
        })
    }

    fn save_doc_state(&self) {
        if let (Some(d), Some(s)) = (&self.doc, self.doc_state()) {
            DocStore::save_one(&d.path, s);
        }
    }

    fn reload(&mut self, ctx: &egui::Context) {
        let Some(d) = &self.doc else {
            return;
        };
        let v = &d.view;
        let keep = Keep {
            pos: v.position(),
            continuous: v.continuous,
            dual: v.dual,
            odd_left: v.odd_left,
            sizing: v.sizing,
            zoom: v.zoom,
            rotation: v.rotation,
            invert: v.invert,
        };
        let path = d.path.clone();
        let pw = d.password.clone();
        self.open(ctx, path, pw, Some(keep));
    }

    fn check_file_changed(&mut self, ctx: &egui::Context) {
        if !self.settings.auto_reload || self.loading.is_some() || self.last_mtime_check.elapsed().as_secs_f32() < 1.0 {
            return;
        }
        self.last_mtime_check = Instant::now();
        let Some(d) = &mut self.doc else {
            return;
        };
        let m = mtime(&d.path);
        if m.is_some() && m != d.mtime {
            d.mtime = m;
            self.reload(ctx);
        }
    }

    fn parse_page(doc: &dyn Document, s: &str) -> Option<usize> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let n = doc.page_count();
        if let Some(p) = (0..n).find(|&p| doc.page_label(p).is_some_and(|l| l == s)) {
            return Some(p);
        }
        if let Some(p) = (0..n).find(|&p| doc.page_label(p).is_some_and(|l| l.eq_ignore_ascii_case(s))) {
            return Some(p);
        }
        s.parse::<usize>().ok().filter(|v| *v >= 1).map(|v| (v - 1).min(n - 1))
    }

    // ---- commands ----

    fn apply(&mut self, ctx: &egui::Context, cmd: Cmd) {
        match cmd {
            Cmd::Open => {
                let (tx, rx) = channel();
                let dir = self.doc.as_ref().and_then(|d| d.path.parent().map(Path::to_path_buf));
                std::thread::spawn(move || {
                    let mut dlg = rfd::FileDialog::new()
                        .set_title("Open Document")
                        .add_filter("All Documents", doc::EXTENSIONS)
                        .add_filter("PDF", &["pdf"])
                        .add_filter("DjVu", &["djvu", "djv"])
                        .add_filter("Comic Books", &["cbz", "cbt"])
                        .add_filter("Images", &["png", "jpg", "jpeg", "tif", "tiff", "gif", "webp", "bmp"]);
                    if let Some(d) = dir {
                        dlg = dlg.set_directory(d);
                    }
                    let _ = tx.send(dlg.pick_file());
                });
                self.file_rx = Some(rx);
            }
            Cmd::OpenPath(p) => {
                if self.doc.is_some() && self.settings.restore_position {
                    self.save_doc_state();
                }
                self.open(ctx, p, None, None);
            }
            Cmd::NewWindow => spawn_window(None),
            Cmd::SaveCopy => {
                let Some(d) = &self.doc else { return };
                let (tx, rx) = channel();
                let name = file_name(&d.path);
                let dir = d.path.parent().map(Path::to_path_buf);
                std::thread::spawn(move || {
                    let mut dlg = rfd::FileDialog::new().set_title("Save a Copy").set_file_name(name);
                    if let Some(d) = dir {
                        dlg = dlg.set_directory(d);
                    }
                    let _ = tx.send(dlg.save_file());
                });
                self.save_rx = Some(rx);
            }
            Cmd::Print => {
                let Some(d) = &self.doc else { return };
                let doc = d.view.doc.clone();
                let title = d.title.clone();
                let (tx, rx) = channel();
                let ctx2 = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(crate::print::print(&*doc, &title));
                    ctx2.request_repaint();
                });
                self.print_rx = Some(rx);
                self.toast("Preparing to print…");
            }
            Cmd::Properties => {
                if self.doc.is_some() {
                    self.dialog = Some(Dialog::Properties);
                }
            }
            Cmd::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
            Cmd::Reload => self.reload(ctx),
            Cmd::OpenFolder => {
                if let Some(d) = &self.doc {
                    let p = d.path.clone();
                    std::thread::spawn(move || show_in_folder(&p));
                }
            }
            Cmd::ToggleSidebar => self.settings.sidebar_visible = !self.settings.sidebar_visible,
            Cmd::Find => {
                if self.doc.is_some() {
                    self.find_open = true;
                    self.find_focus = true;
                    if let Some(d) = &self.doc {
                        let sel = d.view.selected_text();
                        if !sel.trim().is_empty() && sel.len() < 200 && !sel.contains('\n') {
                            self.find_query = sel.trim().to_string();
                            self.find_changed = Some(Instant::now() - std::time::Duration::from_secs(1));
                        }
                    }
                }
            }
            Cmd::FindNext | Cmd::FindPrev => {
                let forward = cmd == Cmd::FindNext;
                if !self.find_open && !self.find_query.is_empty() {
                    self.find_open = true;
                }
                if self.find_changed.is_some() {
                    self.start_search(ctx);
                }
                if let Some(d) = &mut self.doc {
                    let page = d.view.current_page;
                    if let Some(s) = &mut d.view.search
                        && let Some((p, i)) = s.step(forward, page)
                    {
                        let hits = s.results[p].clone().unwrap_or_default();
                        let text = d.view.page_text_now(p);
                        if let Some((a, b)) = hits.get(i)
                            && let Some(r) = text.range_rects(*a, *b).first()
                        {
                            d.view.reveal(p, *r);
                        }
                    }
                }
            }
            Cmd::Escape => {
                if self.presentation {
                    self.presentation = false;
                    if !self.fullscreen {
                        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                    }
                } else if self.find_open {
                    self.find_open = false;
                    if let Some(d) = &mut self.doc {
                        d.view.search = None;
                    }
                } else if self.doc.as_ref().is_some_and(|d| d.view.selection.is_some()) {
                    if let Some(d) = &mut self.doc {
                        d.view.selection = None;
                    }
                } else if self.fullscreen {
                    self.fullscreen = false;
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                }
            }
            Cmd::Copy => {
                if let Some(d) = &self.doc {
                    let t = d.view.selected_text();
                    if !t.is_empty() {
                        ctx.copy_text(t);
                    }
                }
            }
            Cmd::CopyText(t) => ctx.copy_text(t),
            Cmd::SelectAll => {
                if let Some(d) = &mut self.doc {
                    d.view.select_all();
                }
            }
            Cmd::Follow(t) => {
                if let Some(d) = &mut self.doc {
                    d.view.follow(&t);
                }
            }
            Cmd::ZoomIn | Cmd::ZoomOut | Cmd::Zoom(_) | Cmd::Sizing(_) => {
                if let Some(d) = &mut self.doc {
                    match cmd {
                        Cmd::ZoomIn => d.view.zoom_in(),
                        Cmd::ZoomOut => d.view.zoom_out(),
                        Cmd::Zoom(z) => d.view.set_zoom(z, None),
                        Cmd::Sizing(s) => {
                            d.view.sizing = s;
                            d.view.relayout();
                        }
                        _ => {}
                    }
                    self.settings.sizing = d.view.sizing;
                    self.settings.zoom = d.view.zoom;
                }
            }
            Cmd::ToggleContinuous => {
                if let Some(d) = &mut self.doc {
                    d.view.continuous = !d.view.continuous;
                    self.settings.continuous = d.view.continuous;
                    d.view.relayout();
                }
            }
            Cmd::ToggleDual => {
                if let Some(d) = &mut self.doc {
                    d.view.dual = !d.view.dual;
                    self.settings.dual_page = d.view.dual;
                    d.view.relayout();
                }
            }
            Cmd::ToggleOddLeft => {
                if let Some(d) = &mut self.doc {
                    d.view.odd_left = !d.view.odd_left;
                    self.settings.dual_odd_left = d.view.odd_left;
                    d.view.relayout();
                }
            }
            Cmd::ToggleInvert => {
                self.settings.invert_colors = !self.settings.invert_colors;
                if let Some(d) = &mut self.doc {
                    d.view.set_invert(self.settings.invert_colors);
                }
            }
            Cmd::Rotate(cw) => {
                if let Some(d) = &mut self.doc {
                    d.view.rotate(cw);
                }
            }
            Cmd::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.fullscreen));
            }
            Cmd::Presentation => {
                if self.doc.is_some() {
                    self.presentation = !self.presentation;
                    if let Some(d) = &mut self.doc {
                        d.view.presentation_blank = None;
                    }
                    ctx.send_viewport_cmd(ViewportCommand::Fullscreen(self.presentation || self.fullscreen));
                }
            }
            Cmd::NextPage | Cmd::PrevPage | Cmd::FirstPage | Cmd::LastPage | Cmd::GoToPage(_) => {
                if let Some(d) = &mut self.doc {
                    let v = &mut d.view;
                    match cmd {
                        Cmd::NextPage if self.presentation => v.current_page = (v.current_page + 1).min(v.page_count() - 1),
                        Cmd::PrevPage if self.presentation => v.current_page = v.current_page.saturating_sub(1),
                        Cmd::NextPage => v.next_page(),
                        Cmd::PrevPage => v.prev_page(),
                        Cmd::FirstPage => {
                            v.push_history();
                            v.first_page();
                        }
                        Cmd::LastPage => {
                            v.push_history();
                            v.last_page();
                        }
                        Cmd::GoToPage(p) => {
                            v.push_history();
                            v.go_to_page(p);
                        }
                        _ => {}
                    }
                }
            }
            Cmd::Back => {
                if let Some(d) = &mut self.doc {
                    d.view.history_back();
                }
            }
            Cmd::Forward => {
                if let Some(d) = &mut self.doc {
                    d.view.history_forward();
                }
            }
            Cmd::Scroll(x, y) => {
                if let Some(d) = &mut self.doc {
                    d.view.scroll_by(vec2(x, y));
                }
            }
            Cmd::Screen(down) => {
                if let Some(d) = &mut self.doc {
                    d.view.scroll_screen(down);
                }
            }
            Cmd::AddBookmark => {
                if let Some(d) = &mut self.doc {
                    let page = d.view.current_page;
                    if !d.bookmarks.iter().any(|b| b.page == page) {
                        let label = d.view.doc.page_label(page).unwrap_or_else(|| (page + 1).to_string());
                        d.bookmarks.push(Bookmark { page, title: format!("Page {label}") });
                        d.bookmarks.sort_by_key(|b| b.page);
                        self.save_doc_state();
                        self.toast(format!("Bookmarked page {label}"));
                    }
                }
            }
            Cmd::FocusPageEntry => self.focus_page_entry = true,
            Cmd::Theme(m) => {
                self.theme.set_mode(m);
            }
            Cmd::EditTheme => open_external(&self.theme.path().to_string_lossy()),
            Cmd::Shortcuts => self.dialog = Some(Dialog::Shortcuts),
            Cmd::About => self.dialog = Some(Dialog::About),
        }
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        self.find_changed = None;
        let Some(d) = &mut self.doc else { return };
        if self.find_query.is_empty() {
            d.view.search = None;
            return;
        }
        let opts = doc::text::FindOptions { case_sensitive: self.find_case, whole_words: self.find_whole };
        d.view.search = Some(crate::find::Search::start(
            d.view.doc.clone(),
            d.view.text.cache.clone(),
            &self.find_query,
            opts,
            d.view.current_page,
            ctx.clone(),
        ));
    }

    // ---- keyboard ----

    fn keyboard(&mut self, ctx: &egui::Context) -> Vec<Cmd> {
        let typing = ctx.egui_wants_keyboard_input();
        let events = ctx.input(|i| i.events.clone());
        let mut out = Vec::new();
        for e in events {
            let egui::Event::Key { key, pressed: true, modifiers: m, .. } = e else {
                continue;
            };
            let ctrl = m.command && !m.alt;
            let plain = !m.command && !m.alt;
            let cmd = if self.presentation {
                match key {
                    Key::ArrowRight | Key::ArrowDown | Key::PageDown | Key::Space | Key::N | Key::Enter if plain => {
                        if let Some(d) = &mut self.doc {
                            d.view.presentation_blank = None;
                        }
                        Some(Cmd::NextPage)
                    }
                    Key::ArrowLeft | Key::ArrowUp | Key::PageUp | Key::Backspace | Key::P if plain => {
                        if let Some(d) = &mut self.doc {
                            d.view.presentation_blank = None;
                        }
                        Some(Cmd::PrevPage)
                    }
                    Key::Home => Some(Cmd::FirstPage),
                    Key::End => Some(Cmd::LastPage),
                    Key::B | Key::Period => {
                        if let Some(d) = &mut self.doc {
                            d.view.presentation_blank =
                                if d.view.presentation_blank.is_some() { None } else { Some(Color32::BLACK) };
                        }
                        None
                    }
                    Key::W | Key::Comma => {
                        if let Some(d) = &mut self.doc {
                            d.view.presentation_blank =
                                if d.view.presentation_blank.is_some() { None } else { Some(Color32::WHITE) };
                        }
                        None
                    }
                    Key::Escape | Key::F5 => Some(Cmd::Escape),
                    _ => None,
                }
            } else {
                match key {
                    Key::O if ctrl => Some(Cmd::Open),
                    Key::N if ctrl => Some(Cmd::NewWindow),
                    Key::S if ctrl => Some(Cmd::SaveCopy),
                    Key::P if ctrl => Some(Cmd::Print),
                    Key::W if ctrl => Some(Cmd::Close),
                    Key::Q if ctrl => Some(Cmd::Close),
                    Key::R if ctrl => Some(Cmd::Reload),
                    Key::F if ctrl => Some(Cmd::Find),
                    Key::G if ctrl && m.shift => Some(Cmd::FindPrev),
                    Key::G if ctrl => Some(Cmd::FindNext),
                    Key::F3 if m.shift => Some(Cmd::FindPrev),
                    Key::F3 => Some(Cmd::FindNext),
                    Key::C if ctrl && !typing => Some(Cmd::Copy),
                    Key::A if ctrl && !typing => Some(Cmd::SelectAll),
                    Key::L if ctrl => Some(Cmd::FocusPageEntry),
                    Key::D if ctrl => Some(Cmd::AddBookmark),
                    Key::I if ctrl => Some(Cmd::ToggleInvert),
                    Key::Plus | Key::Equals if ctrl => Some(Cmd::ZoomIn),
                    Key::Minus if ctrl => Some(Cmd::ZoomOut),
                    Key::Num0 if ctrl => Some(Cmd::Sizing(Sizing::Automatic)),
                    Key::ArrowLeft if ctrl => Some(Cmd::Rotate(false)),
                    Key::ArrowRight if ctrl => Some(Cmd::Rotate(true)),
                    Key::Home if ctrl => Some(Cmd::FirstPage),
                    Key::End if ctrl => Some(Cmd::LastPage),
                    Key::PageDown if ctrl => Some(Cmd::NextPage),
                    Key::PageUp if ctrl => Some(Cmd::PrevPage),
                    Key::ArrowLeft if m.alt => Some(Cmd::Back),
                    Key::ArrowRight if m.alt => Some(Cmd::Forward),
                    Key::Enter if m.alt => Some(Cmd::Properties),
                    Key::F1 => Some(Cmd::Shortcuts),
                    Key::F5 => Some(Cmd::Presentation),
                    Key::F9 => Some(Cmd::ToggleSidebar),
                    Key::F11 => Some(Cmd::Fullscreen),
                    Key::Escape => Some(Cmd::Escape),
                    _ if typing || !plain => None,
                    Key::Slash => Some(Cmd::Find),
                    Key::Questionmark => Some(Cmd::Shortcuts),
                    Key::Plus | Key::Equals => Some(Cmd::ZoomIn),
                    Key::Minus => Some(Cmd::ZoomOut),
                    Key::F => Some(Cmd::Sizing(Sizing::FitPage)),
                    Key::W => Some(Cmd::Sizing(Sizing::FitWidth)),
                    Key::A => Some(Cmd::Sizing(Sizing::Automatic)),
                    Key::C => Some(Cmd::ToggleContinuous),
                    Key::D => Some(Cmd::ToggleDual),
                    Key::O => Some(Cmd::ToggleOddLeft),
                    Key::N => Some(Cmd::NextPage),
                    Key::P => Some(Cmd::PrevPage),
                    Key::ArrowDown | Key::J => Some(Cmd::Scroll(0.0, 60.0)),
                    Key::ArrowUp | Key::K => Some(Cmd::Scroll(0.0, -60.0)),
                    Key::ArrowLeft | Key::H => Some(Cmd::Scroll(-60.0, 0.0)),
                    Key::ArrowRight | Key::L => Some(Cmd::Scroll(60.0, 0.0)),
                    Key::PageDown => Some(Cmd::Screen(true)),
                    Key::PageUp | Key::Backspace => Some(Cmd::Screen(false)),
                    Key::Space if m.shift => Some(Cmd::Screen(false)),
                    Key::Space => Some(Cmd::Screen(true)),
                    Key::Home => Some(Cmd::FirstPage),
                    Key::End => Some(Cmd::LastPage),
                    _ => None,
                }
            };
            // Shift+Space arrives with shift set, which `plain` allows.
            if let Some(c) = cmd {
                out.push(c);
            }
        }
        out
    }

    // ---- UI pieces ----

    fn header(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let c = self.theme.colors;
        let h = self.theme.file.metrics.header_height;
        egui::Panel::top("header")
            .exact_size(h)
            .show_separator_line(false)
            .frame(Frame::new().fill(c.surface).inner_margin(Margin::symmetric(8, 0)))
            .show(ui, |ui| {
                let full = ui.max_rect();
                ui.painter().hline(full.x_range(), full.bottom() - 0.5, egui::Stroke::new(1.0, c.border));
                let mut left_end = full.left();
                let mut right_start = full.right();
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let has_doc = self.doc.is_some();
                    if icons::button(ui, Icon::Sidebar, "Side panel (F9)", has_doc, self.settings.sidebar_visible).clicked() {
                        cmds.push(Cmd::ToggleSidebar);
                    }
                    ui.add_space(6.0);
                    if let Some(d) = &self.doc {
                        let v = &d.view;
                        let n = v.page_count();
                        if icons::button(ui, Icon::Prev, "Previous page", v.current_page > 0, false).clicked() {
                            cmds.push(Cmd::PrevPage);
                        }
                        let label = v.doc.page_label(v.current_page).unwrap_or_else(|| (v.current_page + 1).to_string());
                        let focused = ui.memory(|m| m.has_focus(self.page_entry_id));
                        if !focused {
                            self.page_entry = label.clone();
                        }
                        let w = ui.fonts_mut(|f| {
                            f.layout_no_wrap(format!("{}", n.max(1000)), egui::TextStyle::Body.resolve(ui.style()), c.text).size().x
                        }) + 16.0;
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.page_entry)
                                .id(self.page_entry_id)
                                .desired_width(w)
                                .align(Align2::CENTER_CENTER),
                        )
                        .accessible_name("Page number");
                        if self.focus_page_entry {
                            self.focus_page_entry = false;
                            r.request_focus();
                            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), self.page_entry_id).unwrap_or_default();
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(self.page_entry.chars().count()),
                            )));
                            state.store(ui.ctx(), self.page_entry_id);
                        }
                        if r.lost_focus()
                            && ui.input(|i| i.key_pressed(Key::Enter))
                            && let Some(p) = Self::parse_page(&*v.doc, &self.page_entry)
                        {
                            cmds.push(Cmd::GoToPage(p));
                        }
                        let has_labels = v.doc.page_label(v.current_page).is_some_and(|l| l != (v.current_page + 1).to_string());
                        let of = if has_labels { format!("({} of {n})", v.current_page + 1) } else { format!("of {n}") };
                        ui.label(RichText::new(of).color(c.text_dim));
                        if icons::button(ui, Icon::Next, "Next page", v.current_page + 1 < n, false).clicked() {
                            cmds.push(Cmd::NextPage);
                        }
                    }
                    left_end = ui.min_rect().right();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let menu = icons::button(ui, Icon::Menu, "Main menu", true, false);
                        egui::Popup::menu(&menu).width(240.0).show(|ui| self.main_menu(ui, cmds));
                        if self.doc.is_some() {
                            if icons::button(ui, Icon::Search, "Find (Ctrl+F)", true, self.find_open).clicked() {
                                if self.find_open {
                                    cmds.push(Cmd::Escape);
                                    self.find_open = false;
                                } else {
                                    cmds.push(Cmd::Find);
                                }
                            }
                            ui.add_space(6.0);
                            self.zoom_control(ui, cmds);
                        }
                        right_start = ui.min_rect().left();
                    });
                });
                // Title, centered in the free space between the two button groups.
                let title = match (&self.doc, &self.loading) {
                    (_, Some(l)) => format!("Loading {}…", file_name(&l.path)),
                    (Some(d), _) => d.title.clone(),
                    (None, None) => "Document Viewer".to_string(),
                };
                let avail = (right_start - left_end - 24.0).max(0.0);
                if avail > 40.0 {
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let galley = ui.fonts_mut(|f| {
                        let mut job = egui::text::LayoutJob::simple_singleline(title, font, c.text);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(avail);
                        f.layout_job(job)
                    });
                    let mut x = full.center().x - galley.size().x / 2.0;
                    x = x.clamp(left_end + 12.0, (right_start - 12.0 - galley.size().x).max(left_end + 12.0));
                    ui.painter().galley(egui::pos2(x, full.center().y - galley.size().y / 2.0), galley, c.text);
                }
            });
    }

    fn zoom_control(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let Some(d) = &self.doc else { return };
        if icons::button(ui, Icon::Plus, "Zoom in (Ctrl+Plus)", true, false).clicked() {
            cmds.push(Cmd::ZoomIn);
        }
        let z = d.view.effective_zoom();
        let label = match d.view.sizing {
            Sizing::FitPage => "Fit Page".to_string(),
            Sizing::FitWidth => "Fit Width".to_string(),
            _ => format!("{:.0}%", z * 100.0),
        };
        let r = ui.add(egui::Button::new(label).min_size(vec2(84.0, ui.spacing().interact_size.y)));
        egui::Popup::menu(&r).show(|ui| {
            let sizing = d.view.sizing;
            for (s, name, key) in [
                (Sizing::Automatic, "Automatic", "A"),
                (Sizing::FitPage, "Fit Page", "F"),
                (Sizing::FitWidth, "Fit Width", "W"),
            ] {
                if ui.add(egui::Button::selectable(sizing == s, name).shortcut_text(key)).clicked() {
                    cmds.push(Cmd::Sizing(s));
                    ui.close();
                }
            }
            ui.separator();
            for z in [0.5, std::f32::consts::FRAC_1_SQRT_2, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 8.0] {
                if ui.button(format!("{:.0}%", z * 100.0)).clicked() {
                    cmds.push(Cmd::Zoom(z));
                    ui.close();
                }
            }
        });
        if icons::button(ui, Icon::Minus, "Zoom out (Ctrl+Minus)", true, false).clicked() {
            cmds.push(Cmd::ZoomOut);
        }
    }

    fn item(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool, cmd: Cmd, cmds: &mut Vec<Cmd>) {
        let mut b = egui::Button::new(label);
        if !shortcut.is_empty() {
            b = b.shortcut_text(shortcut);
        }
        if ui.add_enabled(enabled, b).clicked() {
            cmds.push(cmd);
            ui.close();
        }
    }

    fn check(ui: &mut Ui, label: &str, shortcut: &str, on: bool, enabled: bool, cmd: Cmd, cmds: &mut Vec<Cmd>) {
        let mut b = egui::Button::selectable(on, label);
        if !shortcut.is_empty() {
            b = b.shortcut_text(shortcut);
        }
        if ui.add_enabled(enabled, b).clicked() {
            cmds.push(cmd);
            ui.close();
        }
    }

    fn main_menu(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let has = self.doc.is_some();
        Self::item(ui, "Open…", "Ctrl+O", true, Cmd::Open, cmds);
        ui.add_enabled_ui(!self.recent.is_empty(), |ui| {
            ui.menu_button("Open Recent", |ui| {
                for p in self.recent.clone() {
                    let r = ui.button(file_name(&p)).on_hover_text(p.display().to_string());
                    if r.clicked() {
                        cmds.push(Cmd::OpenPath(p));
                        ui.close();
                    }
                }
            });
        });
        Self::item(ui, "New Window", "Ctrl+N", true, Cmd::NewWindow, cmds);
        ui.separator();
        Self::item(ui, "Save a Copy…", "Ctrl+S", has, Cmd::SaveCopy, cmds);
        Self::item(ui, "Print…", "Ctrl+P", has, Cmd::Print, cmds);
        Self::item(ui, "Reload", "Ctrl+R", has, Cmd::Reload, cmds);
        Self::item(ui, "Open Containing Folder", "", has, Cmd::OpenFolder, cmds);
        Self::item(ui, "Properties", "Alt+Enter", has, Cmd::Properties, cmds);
        ui.separator();
        ui.add_enabled_ui(has, |ui| {
            ui.menu_button("View", |ui| {
                let (cont, dual, odd) =
                    self.doc.as_ref().map(|d| (d.view.continuous, d.view.dual, d.view.odd_left)).unwrap_or_default();
                Self::check(ui, "Continuous", "C", cont, true, Cmd::ToggleContinuous, cmds);
                Self::check(ui, "Dual Pages", "D", dual, true, Cmd::ToggleDual, cmds);
                Self::check(ui, "Odd Pages Left", "O", odd, dual, Cmd::ToggleOddLeft, cmds);
                ui.separator();
                Self::check(ui, "Side Panel", "F9", self.settings.sidebar_visible, true, Cmd::ToggleSidebar, cmds);
                Self::check(ui, "Inverted Colors", "Ctrl+I", self.settings.invert_colors, true, Cmd::ToggleInvert, cmds);
                ui.separator();
                Self::item(ui, "Rotate Left", "Ctrl+Left", true, Cmd::Rotate(false), cmds);
                Self::item(ui, "Rotate Right", "Ctrl+Right", true, Cmd::Rotate(true), cmds);
                ui.separator();
                Self::check(ui, "Fullscreen", "F11", self.fullscreen, true, Cmd::Fullscreen, cmds);
                Self::item(ui, "Presentation", "F5", true, Cmd::Presentation, cmds);
            });
        });
        ui.menu_button("Appearance", |ui| {
            let mode = self.theme.file.mode;
            Self::check(ui, "Follow System", "", mode == Mode::System, true, Cmd::Theme(Mode::System), cmds);
            Self::check(ui, "Light", "", mode == Mode::Light, true, Cmd::Theme(Mode::Light), cmds);
            Self::check(ui, "Dark", "", mode == Mode::Dark, true, Cmd::Theme(Mode::Dark), cmds);
            ui.separator();
            Self::item(ui, "Edit Theme File…", "", true, Cmd::EditTheme, cmds);
        });
        ui.separator();
        Self::item(ui, "Keyboard Shortcuts", "F1", true, Cmd::Shortcuts, cmds);
        Self::item(ui, "About Document Viewer", "", true, Cmd::About, cmds);
    }

    fn find_bar(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let c = self.theme.colors;
        egui::Panel::top("findbar")
            .show_separator_line(false)
            .frame(Frame::new().fill(c.surface).inner_margin(Margin::symmetric(10, 6)))
            .show(ui, |ui| {
                let full = ui.max_rect();
                ui.painter().hline(full.x_range(), full.bottom() + 5.5, egui::Stroke::new(1.0, c.border));
                ui.horizontal(|ui| {
                    let id = egui::Id::unique("find-entry");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.find_query)
                            .id(id)
                            .hint_text("Find in document")
                            .desired_width(260.0)
                            .return_key(None),
                    )
                    .accessible_name("Find in document");
                    if self.find_focus {
                        self.find_focus = false;
                        r.request_focus();
                    }
                    if r.changed() {
                        self.find_changed = Some(Instant::now());
                    }
                    if r.has_focus() {
                        let (enter, shift) = ui.input(|i| (i.key_pressed(Key::Enter), i.modifiers.shift));
                        if enter {
                            cmds.push(if shift { Cmd::FindPrev } else { Cmd::FindNext });
                        }
                    }
                    if icons::button(ui, Icon::Up, "Previous match (Shift+Enter)", true, false).clicked() {
                        cmds.push(Cmd::FindPrev);
                    }
                    if icons::button(ui, Icon::Down, "Next match (Enter)", true, false).clicked() {
                        cmds.push(Cmd::FindNext);
                    }
                    if ui.checkbox(&mut self.find_case, "Match case").changed() {
                        self.find_changed = Some(Instant::now() - std::time::Duration::from_secs(1));
                    }
                    if ui.checkbox(&mut self.find_whole, "Whole words").changed() {
                        self.find_changed = Some(Instant::now() - std::time::Duration::from_secs(1));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icons::button(ui, Icon::Close, "Close (Esc)", true, false).clicked() {
                            self.find_open = false;
                            if let Some(d) = &mut self.doc {
                                d.view.search = None;
                            }
                        }
                        if let Some(d) = &self.doc
                            && let Some(s) = &d.view.search
                        {
                            let total = s.total();
                            let text = if total == 0 && s.finished() {
                                "No results".to_string()
                            } else if let Some(i) = s.current_index() {
                                format!("{i} of {total}{}", if s.finished() { "" } else { "+" })
                            } else {
                                format!("{total}{} found", if s.finished() { "" } else { "+" })
                            };
                            ui.label(RichText::new(text).color(c.text_dim));
                            if !s.finished() {
                                ui.spinner();
                            }
                        }
                    });
                });
            });
    }

    fn context_menu(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let Some(d) = &self.doc else { return };
        let v = &d.view;
        if let Some(l) = &v.context_link {
            match &l.target {
                LinkTarget::Uri(u) => {
                    Self::item(ui, "Open Link", "", true, Cmd::Follow(l.target.clone()), cmds);
                    Self::item(ui, "Copy Link Address", "", true, Cmd::CopyText(u.clone()), cmds);
                }
                LinkTarget::Page { page, .. } => {
                    let label = v.doc.page_label(*page).unwrap_or_else(|| (page + 1).to_string());
                    Self::item(ui, &format!("Go to Page {label}"), "", true, Cmd::Follow(l.target.clone()), cmds);
                }
            }
            ui.separator();
        }
        Self::item(ui, "Copy", "Ctrl+C", v.has_selection(), Cmd::Copy, cmds);
        Self::item(ui, "Select All", "Ctrl+A", true, Cmd::SelectAll, cmds);
        ui.separator();
        Self::item(ui, "Previous Page", "P", v.current_page > 0, Cmd::PrevPage, cmds);
        Self::item(ui, "Next Page", "N", v.current_page + 1 < v.page_count(), Cmd::NextPage, cmds);
        Self::item(ui, "Back", "Alt+Left", v.can_go_back(), Cmd::Back, cmds);
        Self::item(ui, "Forward", "Alt+Right", v.can_go_forward(), Cmd::Forward, cmds);
        ui.separator();
        Self::item(ui, "Zoom In", "Ctrl+Plus", true, Cmd::ZoomIn, cmds);
        Self::item(ui, "Zoom Out", "Ctrl+Minus", true, Cmd::ZoomOut, cmds);
        Self::check(ui, "Fit Page", "F", v.sizing == Sizing::FitPage, true, Cmd::Sizing(Sizing::FitPage), cmds);
        Self::check(ui, "Fit Width", "W", v.sizing == Sizing::FitWidth, true, Cmd::Sizing(Sizing::FitWidth), cmds);
        ui.menu_button("Rotate", |ui| {
            Self::item(ui, "Rotate Left", "Ctrl+Left", true, Cmd::Rotate(false), cmds);
            Self::item(ui, "Rotate Right", "Ctrl+Right", true, Cmd::Rotate(true), cmds);
        });
        ui.separator();
        Self::item(ui, "Add Bookmark", "Ctrl+D", true, Cmd::AddBookmark, cmds);
        Self::item(ui, "Presentation", "F5", true, Cmd::Presentation, cmds);
        Self::item(ui, "Reload", "Ctrl+R", true, Cmd::Reload, cmds);
        Self::item(ui, "Open Containing Folder", "", true, Cmd::OpenFolder, cmds);
        Self::item(ui, "Properties", "Alt+Enter", true, Cmd::Properties, cmds);
    }

    fn empty_state(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let c = self.theme.colors;
        ui.painter().rect_filled(ui.max_rect(), 0.0, c.background);
        if self.loading.is_some() {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            return;
        }
        ui.vertical_centered(|ui| {
            let h = ui.available_height();
            let content_h = 140.0 + self.recent.len().min(8) as f32 * 34.0;
            ui.add_space(((h - content_h) / 2.0).max(24.0));
            ui.label(RichText::new("Document Viewer").heading().strong());
            ui.add_space(4.0);
            ui.label(RichText::new("PDF, DjVu, DVI, TIFF, images and comic books").color(c.text_dim));
            ui.add_space(18.0);
            let b = egui::Button::new(RichText::new("Open a Document…").color(c.accent_text)).fill(c.accent);
            if ui.add_sized(vec2(220.0, 36.0), b).clicked() {
                cmds.push(Cmd::Open);
            }
            ui.add_space(8.0);
            ui.label(RichText::new("or drop a file here").small().color(c.text_dim));
            if !self.recent.is_empty() {
                ui.add_space(28.0);
                ui.label(RichText::new("Recent").strong().color(c.text_dim));
                ui.add_space(4.0);
                for p in self.recent.iter().take(8) {
                    let r = ui
                        .add_sized(vec2(360.0, 30.0), egui::Button::new(file_name(p)).truncate().frame_when_inactive(false))
                        .on_hover_text(p.display().to_string());
                    if r.clicked() {
                        cmds.push(Cmd::OpenPath(p.clone()));
                    }
                }
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        let Some(dialog) = &mut self.dialog else { return };
        let c = self.theme.colors;
        let mut close = false;
        let mut retry: Option<(PathBuf, String)> = None;
        let modal = egui::Modal::new(egui::Id::unique("dialog")).backdrop_color(Color32::from_black_alpha(100));
        let r = match dialog {
            Dialog::Password { path, text, wrong } => modal.show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(RichText::new("Password Required").heading());
                ui.add_space(6.0);
                ui.label(format!("“{}” is password protected.", file_name(path)));
                if *wrong {
                    ui.label(RichText::new("The password was incorrect.").color(c.error));
                }
                ui.add_space(6.0);
                let r = ui
                    .add(egui::TextEdit::singleline(text).password(true).desired_width(f32::INFINITY))
                    .accessible_name("Password");
                r.request_focus();
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let unlock = egui::Button::new(RichText::new("Unlock").color(c.accent_text)).fill(c.accent);
                    if ui.add(unlock).clicked() || enter {
                        retry = Some((path.clone(), text.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            }),
            Dialog::Error(msg) => modal.show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new("Error").heading());
                ui.add_space(6.0);
                ui.label(msg.as_str());
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            }),
            Dialog::Properties => {
                let rows = self.properties_rows();
                modal.show(ctx, |ui| {
                    ui.set_width(460.0);
                    ui.label(RichText::new("Properties").heading());
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        egui::Grid::new("props").num_columns(2).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                            for (k, v) in rows {
                                ui.label(RichText::new(k).color(c.text_dim));
                                ui.add(egui::Label::new(v).wrap());
                                ui.end_row();
                            }
                        });
                    });
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                })
            }
            Dialog::Shortcuts => modal.show(ctx, |ui| {
                ui.set_width(560.0);
                ui.label(RichText::new("Keyboard Shortcuts").heading());
                ui.add_space(8.0);
                egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                    for (section, items) in SHORTCUTS {
                        ui.add_space(6.0);
                        ui.label(RichText::new(*section).strong().color(c.accent));
                        egui::Grid::new(section).num_columns(2).spacing(vec2(24.0, 4.0)).show(ui, |ui| {
                            for (k, d) in *items {
                                ui.label(RichText::new(*k).monospace());
                                ui.label(*d);
                                ui.end_row();
                            }
                        });
                    }
                });
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            }),
            Dialog::About => modal.show(ctx, |ui| {
                ui.set_width(360.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Document Viewer").heading().strong());
                    ui.label(RichText::new(env!("CARGO_PKG_VERSION")).color(c.text_dim));
                    ui.add_space(8.0);
                    ui.label("A document viewer for PDF, DjVu, DVI, TIFF, images and comic books, written in Rust.");
                    ui.add_space(6.0);
                    ui.label(RichText::new("GPL-3.0-or-later").small().color(c.text_dim));
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            }),
        };
        if r.should_close() {
            close = true;
        }
        if let Some((path, pw)) = retry {
            self.dialog = None;
            self.open(ctx, path, Some(pw), None);
            return;
        }
        if close {
            self.dialog = None;
        }
        let _ = cmds;
    }

    fn properties_rows(&self) -> Vec<(String, String)> {
        let Some(d) = &self.doc else { return Vec::new() };
        let doc = &d.view.doc;
        let mut rows = vec![
            ("File".to_string(), file_name(&d.path)),
            ("Location".to_string(), d.path.parent().map(|p| p.display().to_string()).unwrap_or_default()),
        ];
        if let Ok(m) = std::fs::metadata(&d.path) {
            let b = m.len() as f64;
            let size = if b > 1e6 { format!("{:.1} MB", b / 1e6) } else { format!("{:.0} kB", b / 1e3) };
            rows.push(("Size".into(), size));
        }
        rows.push(("Pages".into(), doc.page_count().to_string()));
        let (w, h) = doc.page_size(d.view.current_page);
        let (wmm, hmm) = (w / 72.0 * 25.4, h / 72.0 * 25.4);
        let mut ps = format!("{wmm:.0} × {hmm:.0} mm");
        if let Some(n) = paper_name(wmm, hmm) {
            ps.push_str(&format!(" ({n}, {})", if w > h { "landscape" } else { "portrait" }));
        }
        rows.push(("Page size".into(), ps));
        for (k, v) in doc.properties() {
            if k != "Title" || d.title != v {
                rows.push((k, v));
            } else {
                rows.insert(0, (k, v));
            }
        }
        rows
    }

    fn poll_background(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        if let Some(rx) = &self.file_rx
            && let Ok(r) = rx.try_recv()
        {
            self.file_rx = None;
            if let Some(p) = r {
                cmds.push(Cmd::OpenPath(p));
            }
        }
        if let Some(rx) = &self.save_rx
            && let Ok(r) = rx.try_recv()
        {
            self.save_rx = None;
            if let (Some(dest), Some(d)) = (r, &self.doc) {
                match std::fs::copy(&d.path, &dest) {
                    Ok(_) => self.toast(format!("Saved {}", file_name(&dest))),
                    Err(e) => self.dialog = Some(Dialog::Error(format!("Could not save “{}”\n\n{e}", file_name(&dest)))),
                }
            }
        }
        if let Some(rx) = &self.print_rx
            && let Ok(r) = rx.try_recv()
        {
            self.print_rx = None;
            match r {
                Ok(()) => self.toast = None,
                Err(e) => self.dialog = Some(Dialog::Error(format!("Printing failed\n\n{e}"))),
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        for (i, p) in dropped.into_iter().enumerate() {
            if i == 0 {
                cmds.push(Cmd::OpenPath(p));
            } else {
                spawn_window(Some(&p));
            }
        }
        if let Some(d) = &mut self.doc
            && let Some(u) = d.view.open_uri.take()
        {
            let lower = u.to_ascii_lowercase();
            let external = lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("mailto:") || lower.contains("://");
            if external {
                open_external(&u);
            } else {
                let base = d.path.parent().unwrap_or(Path::new("."));
                let target = base.join(u.trim_start_matches("file:"));
                if target.extension().is_some_and(|e| doc::EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str())) {
                    spawn_window(Some(&target));
                } else {
                    open_external(&target.to_string_lossy());
                }
            }
        }
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((msg, t)) = &self.toast else { return };
        if t.elapsed().as_secs_f32() > 3.0 && self.print_rx.is_none() {
            self.toast = None;
            return;
        }
        let c = self.theme.colors;
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .interactable(false)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(c.surface)
                    .stroke(egui::Stroke::new(1.0, c.border))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.label(msg.as_str());
                    });
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Document",
        &[
            ("Ctrl+O", "Open a document"),
            ("Ctrl+N", "Open a new window"),
            ("Ctrl+S", "Save a copy"),
            ("Ctrl+P", "Print"),
            ("Ctrl+R", "Reload"),
            ("Alt+Enter", "Properties"),
            ("Ctrl+W", "Close the window"),
        ],
    ),
    (
        "Moving around",
        &[
            ("Arrows, h j k l", "Scroll"),
            ("Page Up / Page Down, Space", "Scroll a screen"),
            ("N / P, Ctrl+Page Down / Up", "Next / previous page"),
            ("Home / End", "First / last page"),
            ("Ctrl+L", "Go to a page"),
            ("Alt+Left / Alt+Right", "Back / forward in history"),
        ],
    ),
    (
        "Find and select",
        &[
            ("Ctrl+F, /", "Find"),
            ("Enter, F3, Ctrl+G", "Next match"),
            ("Shift+Enter, Shift+F3", "Previous match"),
            ("Ctrl+C", "Copy the selection"),
            ("Ctrl+A", "Select all"),
            ("Double / triple click", "Select a word / line"),
            ("Ctrl+D", "Bookmark the current page"),
        ],
    ),
    (
        "View",
        &[
            ("Ctrl+Plus / Ctrl+Minus, Ctrl+wheel", "Zoom in / out"),
            ("Ctrl+0, A", "Automatic size"),
            ("F / W", "Fit page / fit width"),
            ("C / D / O", "Continuous / dual pages / odd pages left"),
            ("Ctrl+Left / Ctrl+Right", "Rotate"),
            ("Ctrl+I", "Invert colors"),
            ("F9", "Side panel"),
            ("F11", "Fullscreen"),
            ("F5", "Presentation (B / W blank the screen)"),
        ],
    ),
];

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.theme.update(&ctx);
        self.poll_loading(&ctx);
        self.check_file_changed(&ctx);
        let mut cmds = Vec::new();
        self.poll_background(&ctx, &mut cmds);
        if self.dialog.is_none() {
            cmds.extend(self.keyboard(&ctx));
        }
        if let Some(t) = self.find_changed
            && t.elapsed().as_millis() > 250
        {
            self.start_search(&ctx);
            cmds.push(Cmd::FindNext);
        } else if self.find_changed.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        ctx.input(|i| {
            let v = i.viewport();
            if let Some(r) = v.inner_rect
                && !v.fullscreen.unwrap_or(false)
            {
                let max = v.maximized.unwrap_or(false);
                if !max {
                    self.window = (r.width(), r.height(), false);
                } else {
                    self.window.2 = true;
                }
            }
        });

        let c = self.theme.colors;
        if self.presentation
            && let Some(d) = self.doc.as_mut()
        {
            let r = egui::CentralPanel::no_frame().show(ui, |ui| d.view.show_presentation(ui)).inner;
            if r.clicked() {
                let left = r.interact_pointer_pos().is_some_and(|p| p.x < r.rect.center().x / 2.0);
                d.view.presentation_blank = None;
                cmds.push(if left { Cmd::PrevPage } else { Cmd::NextPage });
            }
            if r.secondary_clicked() {
                cmds.push(Cmd::PrevPage);
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        } else {
            if !self.fullscreen {
                self.header(ui, &mut cmds);
            }
            if self.find_open && self.doc.is_some() {
                self.find_bar(ui, &mut cmds);
            }
            if self.doc.is_some() && self.settings.sidebar_visible && !self.fullscreen {
                let mut open = true;
                let resp = egui::Panel::left("sidebar")
                    .resizable(true)
                    .default_size(self.settings.sidebar_width)
                    .size_range(140.0..=520.0)
                    .frame(Frame::new().fill(c.surface).inner_margin(Margin::symmetric(8, 0)))
                    .show_collapsible(ui, &mut open, |ui| {
                        let d = self.doc.as_mut().unwrap();
                        self.sidebar.show(ui, &mut d.view, &d.bookmarks, &c)
                    });
                if !open {
                    self.settings.sidebar_visible = false;
                }
                if let Some(r) = resp {
                    self.settings.sidebar_width = r.response.rect.width();
                    if let Some(a) = r.inner {
                        let d = self.doc.as_mut().unwrap();
                        match a {
                            sidebar::Action::Follow(t) => cmds.push(Cmd::Follow(t)),
                            sidebar::Action::GoTo(p) => cmds.push(Cmd::GoToPage(p)),
                            sidebar::Action::AddBookmark => cmds.push(Cmd::AddBookmark),
                            sidebar::Action::RemoveBookmark(i) => {
                                if i < d.bookmarks.len() {
                                    d.bookmarks.remove(i);
                                }
                                self.save_doc_state();
                            }
                            sidebar::Action::RenameBookmark(i, t) => {
                                if let Some(b) = d.bookmarks.get_mut(i)
                                    && !t.trim().is_empty()
                                {
                                    b.title = t.trim().to_string();
                                }
                                self.save_doc_state();
                            }
                        }
                    }
                }
            }
            if self.doc.is_some() {
                egui::CentralPanel::no_frame().show(ui, |ui| {
                    let r = {
                        let d = self.doc.as_mut().unwrap();
                        d.view.show(ui, &c)
                    };
                    r.context_menu(|ui| {
                        ui.set_min_width(230.0);
                        self.context_menu(ui, &mut cmds);
                    });
                    if self.loading.is_some() {
                        egui::Area::new(egui::Id::unique("loading"))
                            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
                            .show(ui.ctx(), |ui| {
                                ui.spinner();
                            });
                    }
                });
            } else {
                egui::CentralPanel::no_frame().show(ui, |ui| self.empty_state(ui, &mut cmds));
            }
        }

        self.dialogs(&ctx, &mut cmds);
        self.toast_ui(&ctx);
        if let Some(e) = &self.theme.error {
            let e = e.clone();
            egui::Area::new(egui::Id::unique("theme-error")).anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -12.0)).show(&ctx, |ui| {
                Frame::new().fill(c.surface).stroke(egui::Stroke::new(1.0, c.error)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                    ui.label(RichText::new(e).color(c.error));
                });
            });
        }
        for cmd in cmds {
            self.apply(&ctx, cmd);
        }

        let title = match &self.doc {
            Some(d) => format!("{} — Document Viewer", d.title),
            None => "Document Viewer".to_string(),
        };
        if title != self.sent_title {
            self.sent_title = title.clone();
            ctx.send_viewport_cmd(ViewportCommand::Title(title));
        }
        if self.loading.is_some() || self.file_rx.is_some() || self.save_rx.is_some() || self.print_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.settings.auto_reload && self.doc.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.theme.colors.canvas.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_doc_state();
        let (w, h, max) = self.window;
        self.settings.window_width = w;
        self.settings.window_height = h;
        self.settings.window_maximized = max;
        if let Some(d) = &self.doc {
            self.settings.sizing = d.view.sizing;
            self.settings.zoom = d.view.zoom;
        }
        self.settings.save();
    }
}
