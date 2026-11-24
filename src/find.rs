//! Background text search across all pages.

use crate::doc::Document;
use crate::doc::text::FindOptions;
use crate::render::{TextCache, page_text};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::Arc;

pub struct Search {
    pub query: String,
    pub opts: FindOptions,
    /// Matches per page as character ranges; `None` until the page has been searched.
    pub results: Vec<Option<Vec<(usize, usize)>>>,
    /// The highlighted match: page and index into that page's matches.
    pub current: Option<(usize, usize)>,
    pub searched: usize,
    rx: Receiver<(usize, Vec<(usize, usize)>)>,
    cancel: Arc<AtomicBool>,
}

impl Search {
    pub fn start(
        doc: Arc<dyn Document>,
        cache: TextCache,
        query: &str,
        opts: FindOptions,
        from_page: usize,
        ctx: egui::Context,
    ) -> Search {
        let n = doc.page_count();
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let q = query.to_string();
        std::thread::Builder::new()
            .name("search".into())
            .spawn(move || {
                for i in 0..n {
                    if c.load(Ordering::Relaxed) {
                        return;
                    }
                    let page = (from_page + i) % n;
                    let text = page_text(&*doc, &cache, page);
                    if tx.send((page, text.find(&q, opts))).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            })
            .ok();
        Search { query: query.to_string(), opts, results: vec![None; n], current: None, searched: 0, rx, cancel }
    }

    /// Collects finished pages. Returns true if anything arrived.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok((page, hits)) = self.rx.try_recv() {
            self.results[page] = Some(hits);
            self.searched += 1;
            any = true;
        }
        any
    }

    pub fn finished(&self) -> bool {
        self.searched >= self.results.len()
    }

    pub fn total(&self) -> usize {
        self.results.iter().flatten().map(|v| v.len()).sum()
    }

    /// 1-based position of the current match among all matches found so far.
    pub fn current_index(&self) -> Option<usize> {
        let (p, i) = self.current?;
        let before: usize = self.results[..p].iter().flatten().map(|v| v.len()).sum();
        Some(before + i + 1)
    }

    /// Moves to the next (or previous) match starting from `page`, wrapping around.
    pub fn step(&mut self, forward: bool, page: usize) -> Option<(usize, usize)> {
        let n = self.results.len();
        if n == 0 {
            return None;
        }
        if let Some((p, i)) = self.current {
            let count = self.results[p].as_ref().map(|v| v.len()).unwrap_or(0);
            if forward && i + 1 < count {
                self.current = Some((p, i + 1));
                return self.current;
            }
            if !forward && i > 0 {
                self.current = Some((p, i - 1));
                return self.current;
            }
        }
        let start = self.current.map(|(p, _)| p).unwrap_or(page);
        let first_try = self.current.is_none();
        for k in 0..=n {
            let off = if first_try && k == 0 {
                0
            } else if k == 0 {
                continue;
            } else {
                k
            };
            let p = if forward { (start + off) % n } else { (start + n - off % n) % n };
            if let Some(Some(v)) = self.results.get(p)
                && !v.is_empty()
            {
                self.current = Some((p, if forward { 0 } else { v.len() - 1 }));
                return self.current;
            }
        }
        None
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
