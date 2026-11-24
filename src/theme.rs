//! Colors, sizes and fonts, loaded from `~/.config/document-viewer/theme.toml` and reloaded
//! whenever the file changes.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Margin, Shadow, Stroke, Theme as EguiTheme};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Palette {
    /// Main window background.
    pub background: Option<String>,
    /// Header bar, sidebar, find bar.
    pub surface: Option<String>,
    /// Behind the pages.
    pub canvas: Option<String>,
    pub text: Option<String>,
    pub text_dim: Option<String>,
    pub accent: Option<String>,
    /// Text on top of accent-colored backgrounds.
    pub accent_text: Option<String>,
    pub border: Option<String>,
    /// Buttons and entries.
    pub control: Option<String>,
    pub control_hover: Option<String>,
    pub selection: Option<String>,
    pub search_match: Option<String>,
    pub search_current: Option<String>,
    pub link_hover: Option<String>,
    pub page_border: Option<String>,
    pub page_shadow: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Metrics {
    pub font_size: f32,
    /// Path to a TTF/OTF file to use for the interface, or empty for the built-in font.
    pub font: String,
    /// Fonts used for characters the main font lacks (CJK, symbols). Missing files are skipped.
    pub fallback_fonts: Vec<String>,
    pub corner_radius: u8,
    pub spacing: f32,
    pub header_height: f32,
    pub page_gap: f32,
    pub page_shadow: f32,
    pub scrollbar_width: f32,
    pub sidebar_width: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeFile {
    pub mode: Mode,
    pub light: Palette,
    pub dark: Palette,
    pub metrics: Metrics,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            font_size: 14.0,
            font: String::new(),
            fallback_fonts: [
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
                "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/adobe-source-han-sans/SourceHanSans.ttc",
            ]
            .map(String::from)
            .to_vec(),
            corner_radius: 6,
            spacing: 6.0,
            header_height: 46.0,
            page_gap: 14.0,
            page_shadow: 8.0,
            scrollbar_width: 8.0,
            sidebar_width: 220.0,
        }
    }
}

impl Palette {
    pub fn light() -> Self {
        Palette {
            background: Some("#fafafa".into()),
            surface: Some("#ebebed".into()),
            canvas: Some("#d8d8dc".into()),
            text: Some("#1f1f24".into()),
            text_dim: Some("#6b6b73".into()),
            accent: Some("#3a76d8".into()),
            accent_text: Some("#ffffff".into()),
            border: Some("#cfcfd4".into()),
            control: Some("#dedee2".into()),
            control_hover: Some("#d2d2d8".into()),
            selection: Some("#3a76d860".into()),
            search_match: Some("#f5c21180".into()),
            search_current: Some("#f57c11b0".into()),
            link_hover: Some("#3a76d830".into()),
            page_border: Some("#00000020".into()),
            page_shadow: Some("#00000030".into()),
            error: Some("#c01c28".into()),
        }
    }

    pub fn dark() -> Self {
        Palette {
            background: Some("#242428".into()),
            surface: Some("#2e2e33".into()),
            canvas: Some("#1a1a1d".into()),
            text: Some("#e8e8ec".into()),
            text_dim: Some("#9a9aa3".into()),
            accent: Some("#78aeed".into()),
            accent_text: Some("#101014".into()),
            border: Some("#3d3d44".into()),
            control: Some("#3a3a40".into()),
            control_hover: Some("#46464d".into()),
            selection: Some("#78aeed60".into()),
            search_match: Some("#f5c21170".into()),
            search_current: Some("#f57c11a0".into()),
            link_hover: Some("#78aeed30".into()),
            page_border: Some("#00000060".into()),
            page_shadow: Some("#00000070".into()),
            error: Some("#ff7b7b".into()),
        }
    }
}

impl Default for ThemeFile {
    fn default() -> Self {
        ThemeFile { mode: Mode::System, light: Palette::light(), dark: Palette::dark(), metrics: Metrics::default() }
    }
}

/// Parses `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<Color32> {
    let h = s.trim().strip_prefix('#')?;
    let v = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
    match h.len() {
        3 => Some(Color32::from_rgb(v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17)),
        6 => Some(Color32::from_rgb(v(0, 2)?, v(2, 2)?, v(4, 2)?)),
        8 => Some(Color32::from_rgba_unmultiplied(v(0, 2)?, v(2, 2)?, v(4, 2)?, v(6, 2)?)),
        _ => None,
    }
}

/// Resolved colors for the active palette.
#[derive(Clone, Copy, Debug)]
pub struct Colors {
    pub dark: bool,
    pub background: Color32,
    pub surface: Color32,
    pub canvas: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub border: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub selection: Color32,
    pub search_match: Color32,
    pub search_current: Color32,
    pub link_hover: Color32,
    pub page_border: Color32,
    pub page_shadow: Color32,
    pub error: Color32,
}

impl Colors {
    fn resolve(p: &Palette, fallback: &Palette, dark: bool) -> Colors {
        let c = |a: &Option<String>, b: &Option<String>| {
            a.as_deref().and_then(parse_color).or_else(|| b.as_deref().and_then(parse_color)).unwrap_or(Color32::MAGENTA)
        };
        Colors {
            dark,
            background: c(&p.background, &fallback.background),
            surface: c(&p.surface, &fallback.surface),
            canvas: c(&p.canvas, &fallback.canvas),
            text: c(&p.text, &fallback.text),
            text_dim: c(&p.text_dim, &fallback.text_dim),
            accent: c(&p.accent, &fallback.accent),
            accent_text: c(&p.accent_text, &fallback.accent_text),
            border: c(&p.border, &fallback.border),
            control: c(&p.control, &fallback.control),
            control_hover: c(&p.control_hover, &fallback.control_hover),
            selection: c(&p.selection, &fallback.selection),
            search_match: c(&p.search_match, &fallback.search_match),
            search_current: c(&p.search_current, &fallback.search_current),
            link_hover: c(&p.link_hover, &fallback.link_hover),
            page_border: c(&p.page_border, &fallback.page_border),
            page_shadow: c(&p.page_shadow, &fallback.page_shadow),
            error: c(&p.error, &fallback.error),
        }
    }
}

pub struct Theme {
    pub file: ThemeFile,
    pub colors: Colors,
    pub error: Option<String>,
    path: PathBuf,
    mtime: Option<SystemTime>,
    last_check: f64,
    applied_dark: Option<bool>,
    applied_font: Option<(String, Vec<String>)>,
}

const HEADER: &str = "\
# Document Viewer theme.
# Changes are applied as soon as the file is saved.
#
# mode: \"system\" follows the desktop's light/dark preference, or force \"light\" / \"dark\".
# Colors are #rgb, #rrggbb or #rrggbbaa. Delete a line to get the default back.

";

pub fn path() -> PathBuf {
    crate::config::config_dir().join("theme.toml")
}

impl Theme {
    pub fn load() -> Theme {
        let path = path();
        if !path.exists() {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let body = toml::to_string_pretty(&ThemeFile::default()).unwrap_or_default();
            let _ = std::fs::write(&path, format!("{HEADER}{body}"));
        }
        let mut t = Theme {
            file: ThemeFile::default(),
            colors: Colors::resolve(&Palette::light(), &Palette::light(), false),
            error: None,
            path,
            mtime: None,
            last_check: -1.0,
            applied_dark: None,
            applied_font: None,
        };
        t.read();
        t
    }

    fn read(&mut self) {
        self.mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        match std::fs::read_to_string(&self.path) {
            Ok(s) => match toml::from_str::<ThemeFile>(&s) {
                Ok(f) => {
                    self.file = f;
                    self.error = None;
                }
                Err(e) => self.error = Some(format!("theme.toml: {}", e.message())),
            },
            Err(_) => self.file = ThemeFile::default(),
        }
        self.applied_dark = None;
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.file.mode = mode;
        if let Ok(s) = std::fs::read_to_string(&self.path)
            && let Ok(mut doc) = s.parse::<toml::Table>()
        {
            doc.insert("mode".into(), toml::Value::String(format!("{mode:?}").to_lowercase()));
            let mut out = String::new();
            for line in s.lines() {
                if line.starts_with('#') || line.trim().is_empty() {
                    out.push_str(line);
                    out.push('\n');
                } else {
                    break;
                }
            }
            out.push_str(&toml::to_string_pretty(&doc).unwrap_or_default());
            let _ = std::fs::write(&self.path, out);
            self.mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        }
        self.applied_dark = None;
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Re-reads the file if it changed, and pushes the theme into egui when needed.
    pub fn update(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now - self.last_check > 1.0 {
            self.last_check = now;
            let m = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
            if m != self.mtime {
                self.read();
            }
        }
        let dark = match self.file.mode {
            Mode::Light => false,
            Mode::Dark => true,
            Mode::System => ctx.system_theme().map(|t| t == EguiTheme::Dark).unwrap_or(true),
        };
        if self.applied_dark != Some(dark) {
            self.applied_dark = Some(dark);
            let (p, fb) = if dark { (&self.file.dark, Palette::dark()) } else { (&self.file.light, Palette::light()) };
            self.colors = Colors::resolve(p, &fb, dark);
            self.apply(ctx);
        }
        let fonts = (self.file.metrics.font.clone(), self.file.metrics.fallback_fonts.clone());
        if self.applied_font.as_ref() != Some(&fonts) {
            self.applied_font = Some(fonts);
            self.apply_font(ctx);
        }
    }

    fn apply_font(&self, ctx: &egui::Context) {
        let mut fonts = FontDefinitions::default();
        let path = self.file.metrics.font.trim();
        if !path.is_empty() {
            let path = crate::config::expand_tilde(path);
            if let Ok(bytes) = std::fs::read(&path) {
                fonts.font_data.insert("user".into(), Arc::new(FontData::from_owned(bytes)));
                fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "user".into());
            }
        }
        for (i, p) in self.file.metrics.fallback_fonts.iter().enumerate() {
            let p = crate::config::expand_tilde(p.trim());
            if let Ok(bytes) = std::fs::read(&p) {
                let name = format!("fallback-{i}");
                fonts.font_data.insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
                for fam in [FontFamily::Proportional, FontFamily::Monospace] {
                    fonts.families.entry(fam).or_default().push(name.clone());
                }
            }
        }
        ctx.set_fonts(fonts);
    }

    fn apply(&self, ctx: &egui::Context) {
        let c = &self.colors;
        let m = &self.file.metrics;
        let r = CornerRadius::same(m.corner_radius);
        let mut v = if c.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.override_text_color = Some(c.text);
        v.weak_text_color = Some(c.text_dim);
        v.panel_fill = c.surface;
        v.window_fill = c.background;
        v.window_stroke = Stroke::new(1.0, c.border);
        v.window_corner_radius = CornerRadius::same(m.corner_radius.saturating_add(4));
        v.menu_corner_radius = CornerRadius::same(m.corner_radius.saturating_add(2));
        v.window_shadow = Shadow { offset: [0, 4], blur: 18, spread: 0, color: c.page_shadow };
        v.popup_shadow = Shadow { offset: [0, 3], blur: 12, spread: 0, color: c.page_shadow };
        v.extreme_bg_color = c.background;
        v.text_edit_bg_color = Some(c.background);
        v.faint_bg_color = c.control;
        v.code_bg_color = c.control;
        v.hyperlink_color = c.accent;
        v.error_fg_color = c.error;
        v.selection.bg_fill = c.selection;
        v.selection.stroke = Stroke::new(1.0, c.accent);
        v.text_cursor.stroke = Stroke::new(2.0, c.accent);
        v.indent_has_left_vline = false;
        v.collapsing_header_frame = false;
        let w = &mut v.widgets;
        for (wv, fill) in [
            (&mut w.noninteractive, c.surface),
            (&mut w.inactive, c.control),
            (&mut w.hovered, c.control_hover),
            (&mut w.active, c.control_hover),
            (&mut w.open, c.control_hover),
        ] {
            wv.corner_radius = r;
            wv.bg_fill = fill;
            wv.weak_bg_fill = fill;
            wv.fg_stroke = Stroke::new(1.0, c.text);
            wv.bg_stroke = Stroke::NONE;
            wv.expansion = 0.0;
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, c.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, c.text);
        w.inactive.weak_bg_fill = Color32::TRANSPARENT;
        w.hovered.bg_stroke = Stroke::NONE;
        w.active.bg_stroke = Stroke::new(1.0, c.accent);
        w.open.weak_bg_fill = c.control_hover;

        let size = m.font_size.clamp(8.0, 32.0);
        let spacing = m.spacing.clamp(0.0, 24.0);
        let theme = if c.dark { EguiTheme::Dark } else { EguiTheme::Light };
        ctx.set_visuals_of(theme, v);
        ctx.style_mut_of(theme, |s| {
            use egui::{FontId, TextStyle};
            s.text_styles = [
                (TextStyle::Small, FontId::proportional(size * 0.8)),
                (TextStyle::Body, FontId::proportional(size)),
                (TextStyle::Button, FontId::proportional(size)),
                (TextStyle::Heading, FontId::proportional(size * 1.35)),
                (TextStyle::Monospace, FontId::monospace(size * 0.95)),
            ]
            .into();
            s.spacing.item_spacing = egui::vec2(spacing, spacing);
            s.spacing.button_padding = egui::vec2(spacing * 1.4, spacing * 0.7);
            s.spacing.menu_margin = Margin::same((spacing) as i8);
            s.spacing.window_margin = Margin::same((spacing * 2.5) as i8);
            s.spacing.interact_size.y = size + spacing * 1.6;
            s.spacing.scroll.bar_width = m.scrollbar_width;
            s.spacing.scroll.floating = true;
            s.spacing.menu_width = 220.0;
        });
        ctx.set_theme(theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff"), Some(Color32::WHITE));
        assert_eq!(parse_color("#102030"), Some(Color32::from_rgb(16, 32, 48)));
        assert_eq!(parse_color("#ff000080"), Some(Color32::from_rgba_unmultiplied(255, 0, 0, 128)));
        assert_eq!(parse_color("red"), None);
    }

    #[test]
    fn default_file_round_trips() {
        let s = toml::to_string_pretty(&ThemeFile::default()).unwrap();
        let back: ThemeFile = toml::from_str(&s).unwrap();
        assert_eq!(back, ThemeFile::default());
        let partial: ThemeFile = toml::from_str("mode = \"dark\"\n[dark]\naccent = \"#ff0000\"\n").unwrap();
        assert_eq!(partial.mode, Mode::Dark);
        assert_eq!(partial.dark.accent.as_deref(), Some("#ff0000"));
        assert_eq!(partial.dark.background, None);
    }
}
