//! Background page rendering. The UI states each frame which tiles it wants, most important
//! first; worker threads take from the front of that list and send pixels back.

use crate::doc::{Document, PageText, RgbaImage};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TileKind {
    /// The whole page.
    Page,
    /// Part of the page, at the grid position given by x/y.
    Tile,
    /// A small whole-page image for the sidebar.
    Thumb,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TileKey {
    pub page: u32,
    pub kind: TileKind,
    /// Device pixels per point, times 1000.
    pub scale: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub invert: bool,
}

impl TileKey {
    pub fn scale(&self) -> f32 {
        self.scale as f32 / 1000.0
    }
}

pub struct Rendered {
    pub key: TileKey,
    pub image: Option<RgbaImage>,
}

#[derive(Default)]
struct Queue {
    wanted: Vec<TileKey>,
    in_flight: HashSet<TileKey>,
    stop: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    cv: Condvar,
}

pub struct RenderPool {
    shared: Arc<Shared>,
    pub results: Receiver<Rendered>,
    threads: Vec<JoinHandle<()>>,
}

impl RenderPool {
    pub fn new(doc: Arc<dyn Document>, threads: usize, ctx: egui::Context) -> RenderPool {
        let shared = Arc::new(Shared { queue: Mutex::new(Queue::default()), cv: Condvar::new() });
        let (tx, rx) = channel();
        let n = if threads == 0 {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 4)
        } else {
            threads
        };
        let threads = (0..n)
            .map(|i| {
                let shared = shared.clone();
                let doc = doc.clone();
                let tx: Sender<Rendered> = tx.clone();
                let ctx = ctx.clone();
                std::thread::Builder::new()
                    .name(format!("render-{i}"))
                    .spawn(move || worker(doc, shared, tx, ctx))
                    .expect("spawn render thread")
            })
            .collect();
        RenderPool { shared, results: rx, threads }
    }

    /// Replaces the list of wanted tiles. Tiles no longer listed are dropped unless already rendering.
    pub fn set_wanted(&self, wanted: Vec<TileKey>) {
        let mut q = self.shared.queue.lock().unwrap();
        let in_flight = &q.in_flight;
        let mut seen = HashSet::new();
        let list: Vec<TileKey> = wanted.into_iter().filter(|k| !in_flight.contains(k) && seen.insert(*k)).collect();
        if list != q.wanted {
            q.wanted = list;
            self.shared.cv.notify_all();
        }
    }
}

impl Drop for RenderPool {
    fn drop(&mut self) {
        self.shared.queue.lock().unwrap().stop = true;
        self.shared.cv.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn worker(doc: Arc<dyn Document>, shared: Arc<Shared>, tx: Sender<Rendered>, ctx: egui::Context) {
    let mut renderer = doc.renderer();
    loop {
        let key = {
            let mut q = shared.queue.lock().unwrap();
            loop {
                if q.stop {
                    return;
                }
                if !q.wanted.is_empty() {
                    let k = q.wanted.remove(0);
                    q.in_flight.insert(k);
                    break k;
                }
                q = shared.cv.wait(q).unwrap();
            }
        };
        let region = match key.kind {
            TileKind::Tile => Some((key.x, key.y, key.w, key.h)),
            _ => None,
        };
        let image = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            renderer.render(key.page as usize, key.scale(), region)
        }))
        .ok()
        .flatten()
        .map(|mut img| {
            if key.invert {
                img.invert();
            }
            img
        });
        shared.queue.lock().unwrap().in_flight.remove(&key);
        if tx.send(Rendered { key, image }).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

pub type TextCache = Arc<Mutex<HashMap<usize, Arc<PageText>>>>;

/// Extracts page text in the background so hovering and selecting feel instant.
pub struct TextWorker {
    tx: Option<Sender<usize>>,
    pending: Arc<Mutex<HashSet<usize>>>,
    pub cache: TextCache,
    thread: Option<JoinHandle<()>>,
}

pub fn page_text(doc: &dyn Document, cache: &TextCache, page: usize) -> Arc<PageText> {
    if let Some(t) = cache.lock().unwrap().get(&page) {
        return t.clone();
    }
    let t = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| doc.text(page)))
        .ok()
        .flatten()
        .unwrap_or_default();
    let t = Arc::new(t);
    cache.lock().unwrap().insert(page, t.clone());
    t
}

impl TextWorker {
    pub fn new(doc: Arc<dyn Document>, ctx: egui::Context) -> TextWorker {
        let (tx, rx) = channel::<usize>();
        let cache: TextCache = Default::default();
        let pending: Arc<Mutex<HashSet<usize>>> = Default::default();
        let c = cache.clone();
        let p = pending.clone();
        let thread = std::thread::Builder::new()
            .name("text".into())
            .spawn(move || {
                while let Ok(page) = rx.recv() {
                    if !p.lock().unwrap().contains(&page) {
                        continue;
                    }
                    page_text(&*doc, &c, page);
                    p.lock().unwrap().remove(&page);
                    ctx.request_repaint();
                }
            })
            .ok();
        TextWorker { tx: Some(tx), pending, cache, thread }
    }

    pub fn get(&self, page: usize) -> Option<Arc<PageText>> {
        self.cache.lock().unwrap().get(&page).cloned()
    }

    pub fn request(&self, page: usize) {
        if self.cache.lock().unwrap().contains_key(&page) {
            return;
        }
        if self.pending.lock().unwrap().insert(page)
            && let Some(tx) = &self.tx
        {
            let _ = tx.send(page);
        }
    }
}

impl Drop for TextWorker {
    fn drop(&mut self) {
        // Pending requests are dropped with the channel; the thread exits on its own.
        self.pending.lock().unwrap().clear();
        self.tx = None;
        self.thread.take();
    }
}
