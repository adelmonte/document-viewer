//! The side panel: page thumbnails, document outline and user bookmarks.

use crate::config::Bookmark;
use crate::doc::{LinkTarget, OutlineNode};
use crate::theme::Colors;
use crate::view::View;
use egui::{Align, Id, Rect, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Thumbnails,
    Outline,
    Bookmarks,
}

pub enum Action {
    Follow(LinkTarget),
    GoTo(usize),
    AddBookmark,
    RemoveBookmark(usize),
    RenameBookmark(usize, String),
}

pub struct Sidebar {
    pub tab: Tab,
    outline: Option<std::sync::Arc<Vec<OutlineNode>>>,
    shown_page: Option<usize>,
    filter: String,
    renaming: Option<(usize, String)>,
    expand_all: Option<bool>,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

impl Sidebar {
    pub fn new() -> Sidebar {
        Sidebar { tab: Tab::Thumbnails, outline: None, shown_page: None, filter: String::new(), renaming: None, expand_all: None }
    }

    /// Forgets the cached outline (after loading another document).
    pub fn reset(&mut self) {
        self.outline = None;
        self.shown_page = None;
        self.filter.clear();
        self.renaming = None;
    }

    pub fn has_outline(&mut self, view: &View) -> bool {
        !self.outline(view).is_empty()
    }

    fn outline(&mut self, view: &View) -> std::sync::Arc<Vec<OutlineNode>> {
        self.outline.get_or_insert_with(|| std::sync::Arc::new(view.doc.outline())).clone()
    }

    pub fn show(&mut self, ui: &mut Ui, view: &mut View, bookmarks: &[Bookmark], colors: &Colors) -> Option<Action> {
        let mut action = None;
        let has_outline = self.has_outline(view);
        if self.tab == Tab::Outline && !has_outline {
            self.tab = Tab::Thumbnails;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let tabs: &[(Tab, &str)] = if has_outline {
                &[(Tab::Thumbnails, "Pages"), (Tab::Outline, "Outline"), (Tab::Bookmarks, "Bookmarks")]
            } else {
                &[(Tab::Thumbnails, "Pages"), (Tab::Bookmarks, "Bookmarks")]
            };
            let w = (ui.available_width() - 2.0 * (tabs.len() as f32 - 1.0)) / tabs.len() as f32;
            for (t, label) in tabs {
                let selected = self.tab == *t;
                let text = if selected { RichText::new(*label).color(colors.accent_text) } else { RichText::new(*label) };
                let b = egui::Button::new(text).fill(if selected { colors.accent } else { colors.control }).frame(true);
                if ui.add_sized(vec2(w, ui.spacing().interact_size.y), b).clicked() {
                    self.tab = *t;
                }
            }
        });
        ui.add_space(4.0);
        match self.tab {
            Tab::Thumbnails => self.thumbnails(ui, view, colors, &mut action),
            Tab::Outline => self.outline_tab(ui, view, colors, &mut action),
            Tab::Bookmarks => self.bookmarks(ui, view, bookmarks, colors, &mut action),
        }
        action
    }

    fn thumbnails(&mut self, ui: &mut Ui, view: &mut View, colors: &Colors, action: &mut Option<Action>) {
        let n = view.page_count();
        let width = (ui.available_width() - 28.0).clamp(40.0, 260.0);
        let label_h = ui.text_style_height(&egui::TextStyle::Small) + 6.0;
        let gap = 14.0;
        let mut tops = Vec::with_capacity(n + 1);
        let mut y = 6.0;
        for p in 0..n {
            tops.push(y);
            let (w, h) = view.rotated_page_size(p);
            y += width * h / w.max(1.0) + label_h + gap;
        }
        tops.push(y);
        let total = y;
        let current = view.current_page;
        let scroll_to = self.shown_page != Some(current);
        self.shown_page = Some(current);
        let ppp = ui.ctx().pixels_per_point();

        ScrollArea::vertical().auto_shrink([false, false]).id_salt("thumbs").show_viewport(ui, |ui, viewport| {
            ui.set_height(total);
            let origin = ui.max_rect().min;
            let x = (ui.max_rect().width() - width) / 2.0;
            let first = tops.partition_point(|t| *t < viewport.top()).saturating_sub(1);
            for p in first..n {
                let top = tops[p];
                if top > viewport.bottom() {
                    break;
                }
                let (w, h) = view.rotated_page_size(p);
                let th = width * h / w.max(1.0);
                let rect = Rect::from_min_size(origin + vec2(x, top), vec2(width, th));
                let item = rect.union(Rect::from_min_size(rect.left_bottom(), vec2(width, label_h)));
                let resp = ui.interact(item.expand(4.0), Id::unique(("thumb", p)), Sense::click());
                let painter = ui.painter();
                if resp.hovered() {
                    painter.rect_filled(item.expand(5.0), 6.0, colors.control_hover);
                }
                painter.rect_filled(rect, 0.0, if view.invert { egui::Color32::BLACK } else { egui::Color32::WHITE });
                if let Some(tex) = view.thumbnail(p, width * ppp) {
                    view.paint_page_image(painter, tex, rect);
                }
                let selected = p == current;
                let stroke = if selected { Stroke::new(2.5, colors.accent) } else { Stroke::new(1.0, colors.page_border) };
                painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Outside);
                let label = view.doc.page_label(p).unwrap_or_else(|| (p + 1).to_string());
                painter.text(
                    pos2(rect.center().x, rect.bottom() + 4.0),
                    egui::Align2::CENTER_TOP,
                    label,
                    egui::TextStyle::Small.resolve(ui.style()),
                    if selected { colors.accent } else { colors.text_dim },
                );
                if resp.clicked() {
                    *action = Some(Action::GoTo(p));
                    self.shown_page = Some(p);
                }
            }
            if scroll_to && let (Some(t), Some(b)) = (tops.get(current), tops.get(current + 1)) {
                let r = Rect::from_min_max(origin + vec2(x, *t), origin + vec2(x + width, *b));
                if !viewport.translate(origin.to_vec2()).contains_rect(r) {
                    ui.scroll_to_rect(r, Some(Align::Center));
                }
            }
        });
    }

    fn outline_tab(&mut self, ui: &mut Ui, view: &mut View, colors: &Colors, action: &mut Option<Action>) {
        ui.horizontal(|ui| {
            let w = ui.available_width() - 2.0 * (ui.spacing().interact_size.y + ui.spacing().item_spacing.x);
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter outline").desired_width(w.max(40.0)))
                .accessible_name("Filter outline");
            if ui.small_button("+").on_hover_text("Expand all").clicked() {
                self.expand_all = Some(true);
            }
            if ui.small_button("-").on_hover_text("Collapse all").clicked() {
                self.expand_all = Some(false);
            }
        });
        ui.add_space(4.0);
        let filter = self.filter.to_lowercase();
        let expand = self.expand_all.take();
        let current = view.current_page;
        let outline = self.outline(view);
        ScrollArea::vertical().auto_shrink([false, false]).id_salt("outline").show(ui, |ui| {
            if filter.is_empty() {
                for (i, node) in outline.iter().enumerate() {
                    outline_node(ui, node, Id::unique("outline").with(i), expand, current, colors, action);
                }
            } else {
                let mut flat = Vec::new();
                flatten(&outline, &filter, &mut flat);
                for node in flat {
                    if ui.selectable_label(false, &node.title).clicked()
                        && let Some(t) = &node.target
                    {
                        *action = Some(Action::Follow(t.clone()));
                    }
                }
            }
        });
    }

    fn bookmarks(&mut self, ui: &mut Ui, view: &View, bookmarks: &[Bookmark], colors: &Colors, action: &mut Option<Action>) {
        if ui.add_sized(vec2(ui.available_width(), ui.spacing().interact_size.y), egui::Button::new("Add bookmark")).clicked() {
            *action = Some(Action::AddBookmark);
        }
        ui.add_space(4.0);
        if bookmarks.is_empty() {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No bookmarks").color(colors.text_dim));
                ui.label(RichText::new("Ctrl+D bookmarks the current page").small().color(colors.text_dim));
            });
            return;
        }
        ScrollArea::vertical().auto_shrink([false, false]).id_salt("bookmarks").show(ui, |ui| {
            for (i, b) in bookmarks.iter().enumerate() {
                if let Some((idx, text)) = &mut self.renaming
                    && *idx == i
                {
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY)).accessible_name("Bookmark name");
                    r.request_focus();
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) || r.lost_focus() {
                        *action = Some(Action::RenameBookmark(i, text.clone()));
                        self.renaming = None;
                    } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.renaming = None;
                    }
                    continue;
                }
                let label = view.doc.page_label(b.page).unwrap_or_else(|| (b.page + 1).to_string());
                let r = ui.horizontal(|ui| {
                    let r = ui.selectable_label(b.page == view.current_page, &b.title);
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(label).color(colors.text_dim));
                    });
                    r
                });
                let r = r.inner;
                if r.clicked() {
                    *action = Some(Action::GoTo(b.page));
                }
                r.context_menu(|ui| {
                    if ui.button("Open Bookmark").clicked() {
                        *action = Some(Action::GoTo(b.page));
                        ui.close();
                    }
                    if ui.button("Rename Bookmark").clicked() {
                        self.renaming = Some((i, b.title.clone()));
                        ui.close();
                    }
                    if ui.button("Delete Bookmark").clicked() {
                        *action = Some(Action::RemoveBookmark(i));
                        ui.close();
                    }
                });
            }
        });
    }
}

fn flatten<'a>(nodes: &'a [OutlineNode], filter: &str, out: &mut Vec<&'a OutlineNode>) {
    for n in nodes {
        if n.title.to_lowercase().contains(filter) {
            out.push(n);
        }
        flatten(&n.children, filter, out);
    }
}

fn node_page(n: &OutlineNode) -> Option<usize> {
    match &n.target {
        Some(LinkTarget::Page { page, .. }) => Some(*page),
        _ => None,
    }
}

fn outline_node(
    ui: &mut Ui,
    node: &OutlineNode,
    id: Id,
    expand: Option<bool>,
    current: usize,
    colors: &Colors,
    action: &mut Option<Action>,
) {
    let page_text = |ui: &mut Ui| {
        if let Some(p) = node_page(node) {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new((p + 1).to_string()).small().color(colors.text_dim));
            });
        }
    };
    let here = node_page(node) == Some(current);
    let title = if here { RichText::new(&node.title).color(colors.accent) } else { RichText::new(&node.title) };
    if node.children.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().icon_width + ui.spacing().item_spacing.x);
            let r = ui.add(egui::Label::new(title).sense(Sense::click()).truncate());
            if r.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked()
                && let Some(t) = &node.target
            {
                *action = Some(Action::Follow(t.clone()));
            }
            page_text(ui);
        });
        return;
    }
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
    if let Some(e) = expand {
        state.set_open(e);
    }
    state
        .show_header(ui, |ui| {
            let r = ui.add(egui::Label::new(title).sense(Sense::click()).truncate());
            if r.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked()
                && let Some(t) = &node.target
            {
                *action = Some(Action::Follow(t.clone()));
            }
            page_text(ui);
        })
        .body(|ui| {
            for (i, c) in node.children.iter().enumerate() {
                outline_node(ui, c, id.with(i), expand, current, colors, action);
            }
        });
}
