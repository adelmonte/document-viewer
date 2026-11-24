//! Settings (`config.toml`) and remembered per-document state (`documents.toml`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const APP_ID: &str = "document-viewer";

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_ID)
}

pub fn state_dir() -> PathBuf {
    dirs::state_dir().or_else(dirs::data_local_dir).unwrap_or_else(|| PathBuf::from(".")).join(APP_ID)
}

pub fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(p),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Sizing {
    /// Fit the width, but never zoom past 100% for narrow windows' sake.
    #[default]
    Automatic,
    FitWidth,
    FitPage,
    Free,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub continuous: bool,
    pub dual_page: bool,
    pub dual_odd_left: bool,
    pub sizing: Sizing,
    pub zoom: f32,
    pub invert_colors: bool,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    pub restore_position: bool,
    pub auto_reload: bool,
    pub render_threads: usize,
    pub window_width: f32,
    pub window_height: f32,
    pub window_maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            continuous: true,
            dual_page: false,
            dual_odd_left: false,
            sizing: Sizing::Automatic,
            zoom: 1.0,
            invert_colors: false,
            sidebar_visible: false,
            sidebar_width: 220.0,
            restore_position: true,
            auto_reload: true,
            render_threads: 0,
            window_width: 900.0,
            window_height: 1000.0,
            window_maximized: false,
        }
    }
}

fn write_atomic(path: &Path, body: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let body = format!(
            "# Document Viewer settings. The theme lives in theme.toml next to this file.\n\n{}",
            toml::to_string_pretty(self).unwrap_or_default()
        );
        write_atomic(&Self::path(), &body);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bookmark {
    pub page: usize,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DocState {
    pub page: usize,
    /// Fraction of the page height scrolled past, for restoring the exact spot.
    pub page_offset: f32,
    pub sizing: Option<Sizing>,
    pub zoom: Option<f32>,
    pub rotation: u32,
    pub continuous: Option<bool>,
    pub dual_page: Option<bool>,
    pub sidebar_visible: Option<bool>,
    pub bookmarks: Vec<Bookmark>,
    pub last_opened: u64,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DocStore {
    pub documents: BTreeMap<String, DocState>,
}

const MAX_REMEMBERED: usize = 500;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl DocStore {
    fn path() -> PathBuf {
        state_dir().join("documents.toml")
    }

    pub fn load() -> DocStore {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn get(&self, path: &Path) -> Option<DocState> {
        self.documents.get(&path.to_string_lossy().into_owned()).cloned()
    }

    pub fn put(&mut self, path: &Path, mut state: DocState) {
        state.last_opened = now();
        self.documents.insert(path.to_string_lossy().into_owned(), state);
        if self.documents.len() > MAX_REMEMBERED {
            let mut by_age: Vec<(u64, String)> = self.documents.iter().map(|(k, v)| (v.last_opened, k.clone())).collect();
            by_age.sort();
            for (_, k) in by_age.into_iter().take(self.documents.len() - MAX_REMEMBERED) {
                self.documents.remove(&k);
            }
        }
    }

    /// Re-reads the file and merges in `path`'s state so other windows' changes survive.
    pub fn save_one(path: &Path, state: DocState) {
        let mut store = DocStore::load();
        store.put(path, state);
        write_atomic(&Self::path(), &toml::to_string(&store).unwrap_or_default());
    }

    /// Recently opened documents that still exist, newest first.
    pub fn recent(&self, n: usize) -> Vec<PathBuf> {
        let mut v: Vec<(&u64, &String)> = self.documents.iter().map(|(k, s)| (&s.last_opened, k)).collect();
        v.sort_by(|a, b| b.0.cmp(a.0));
        v.into_iter().map(|(_, k)| PathBuf::from(k)).filter(|p| p.exists()).take(n).collect()
    }

    pub fn forget_all() {
        let _ = std::fs::remove_file(Self::path());
    }
}
