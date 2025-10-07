//! Text reader (EPUB, FB2): one scrolling column, chapter list, font size.
//!
//! Only block heights are kept (measured once per width and font size); the
//! blocks in view are laid out each frame through egui's galley cache, so books
//! with tens of thousands of paragraphs scroll smoothly. The first visible block
//! is reported with `reader.position` (throttled), which saves the progress.

use std::sync::Arc;

use raft_text::{Block, Kind, TextBook};
use serde_json::json;

use crate::RaftUi;

/// Widest text column, in points.
const COLUMN: f32 = 760.0;
/// Seconds between position reports while scrolling.
const REPORT_EVERY: f64 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Jump {
    Block(usize),
    Offset(f32),
}

pub struct TextView {
    book_id: String,
    book: Arc<TextBook>,
    /// (width, font size) the heights were measured for.
    measured: (f32, f32),
    tops: Vec<f32>,
    heights: Vec<f32>,
    total: f32,
    jump: Option<Jump>,
    offset: f32,
    view_height: f32,
    reported: Option<usize>,
    last_report: f64,
    top: usize,
    /// Target of the last jump: the reading place until the user scrolls away
    /// (the scroll cannot always bring it to the top, e.g. near the end).
    pinned: Option<(usize, Option<f32>)>,
}

/// Reader preferences (not part of the document).
pub struct TextPrefs {
    pub font_size: f32,
}

impl Default for TextPrefs {
    fn default() -> Self {
        Self { font_size: 18.0 }
    }
}

impl TextView {
    fn new(book_id: String, book: Arc<TextBook>, start: Option<usize>) -> Self {
        Self {
            book_id,
            book,
            measured: (0.0, 0.0),
            tops: Vec::new(),
            heights: Vec::new(),
            total: 0.0,
            jump: start.filter(|&b| b > 0).map(Jump::Block),
            offset: 0.0,
            view_height: 0.0,
            reported: start,
            last_report: 0.0,
            top: start.unwrap_or(0),
            pinned: start.map(|b| (b, None)),
        }
    }

    /// Index of the chapter containing `block`.
    fn chapter_of(&self, block: usize) -> usize {
        self.book
            .chapters
            .iter()
            .rposition(|c| c.start <= block)
            .unwrap_or(0)
    }
}

/// Font, color and left indent of a block.
fn style(ui: &egui::Ui, block: &Block, size: f32) -> (egui::FontId, egui::Color32, f32) {
    let v = ui.visuals();
    match block {
        Block::Heading { level, .. } => (
            egui::FontId::proportional(size * (1.7 - 0.12 * f32::from(*level))),
            v.strong_text_color(),
            0.0,
        ),
        Block::Paragraph {
            kind: Kind::Pre, ..
        } => (egui::FontId::monospace(size * 0.8), v.text_color(), 16.0),
        Block::Paragraph {
            kind: Kind::Quote, ..
        } => (
            egui::FontId::proportional(size * 0.95),
            v.weak_text_color(),
            36.0,
        ),
        Block::Paragraph {
            kind: Kind::Verse, ..
        } => (egui::FontId::proportional(size), v.text_color(), 56.0),
        Block::Paragraph { .. } => (egui::FontId::proportional(size), v.text_color(), 0.0),
        Block::Break => (egui::FontId::proportional(size), v.weak_text_color(), 0.0),
    }
}

fn block_text(block: &Block) -> &str {
    match block {
        Block::Break => "*  *  *",
        b => b.text(),
    }
}

/// Space after a block.
fn gap(block: &Block, size: f32) -> f32 {
    match block {
        Block::Heading { .. } => size * 0.9,
        Block::Paragraph {
            kind: Kind::Verse, ..
        } => size * 0.15,
        _ => size * 0.55,
    }
}

fn measure(view: &mut TextView, ui: &egui::Ui, width: f32, size: f32) {
    view.tops.clear();
    view.heights.clear();
    let mut y = 0.0;
    for block in &view.book.blocks {
        let (font, color, indent) = style(ui, block, size);
        let galley = ui.painter().layout(
            block_text(block).to_owned(),
            font,
            color,
            (width - indent).max(50.0),
        );
        let h = galley.size().y + gap(block, size);
        view.tops.push(y);
        view.heights.push(h);
        y += h;
    }
    view.total = y;
    view.measured = (width, size);
}

/// First block whose bottom is below `y`.
fn block_at(view: &TextView, y: f32) -> usize {
    let i = view.tops.partition_point(|&t| t <= y);
    i.saturating_sub(1).min(view.tops.len().saturating_sub(1))
}

pub fn reader(app: &mut RaftUi, ui: &mut egui::Ui, id: &str) {
    let Some(book) = app.session.book(id).cloned() else {
        ui.label("Cartea nu mai există în catalog.");
        return;
    };
    // Load (in the background) and keep the parsed book.
    if app.text.as_ref().is_none_or(|v| v.book_id != id) {
        match app.files.text(&book) {
            None => {
                top_bar(app, ui, book.display_title(), id);
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.spinner();
                    ui.label("Se pregătește cartea…");
                });
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
                return;
            }
            Some(Err(e)) => {
                top_bar(app, ui, book.display_title(), id);
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(format!("Cartea nu a putut fi citită: {e}"));
                    if ui.button("Deschide în aplicația externă").clicked() {
                        app.act("book.open_external", json!({ "id": id }));
                    }
                });
                return;
            }
            Some(Ok(text)) => {
                let start = app.session.progress(id).and_then(raft_doc::Progress::block);
                app.text = Some(TextView::new(id.to_owned(), text, start));
            }
        }
    }
    // Follow position changes made elsewhere (undo, the control channel).
    let saved = app.session.progress(id).and_then(raft_doc::Progress::block);
    if let (Some(view), Some(block)) = (app.text.as_mut(), saved)
        && view.reported != Some(block)
    {
        view.jump = Some(Jump::Block(block));
        view.top = block;
        view.reported = Some(block);
        view.pinned = Some((block, None));
    }
    let title = book.display_title().to_owned();
    top_bar(app, ui, &title, id);
    ui.separator();
    keys(app, ui);

    let size = app.text_prefs.font_size;
    let width = (ui.available_width() - 24.0).clamp(200.0, COLUMN);
    let Some(view) = app.text.as_mut() else {
        return;
    };
    if view.measured != (width, size) {
        // Keep the reading place when the layout changes.
        let keep = view.top;
        measure(view, ui, width, size);
        if keep > 0 {
            view.jump = Some(Jump::Block(keep));
            view.pinned = Some((keep, None));
        }
    }
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("text", id.to_owned()))
        .auto_shrink([false, false]);
    if let Some(jump) = view.jump.take() {
        let y = match jump {
            Jump::Block(b) => view.tops.get(b).copied().unwrap_or(0.0),
            Jump::Offset(y) => y,
        };
        area = area.vertical_scroll_offset(y.clamp(0.0, (view.total - view.view_height).max(0.0)));
    }
    let output = area.show_viewport(ui, |ui, viewport| {
        ui.set_min_height(view.total);
        let left = ui.min_rect().left() + ((ui.available_width() - width) / 2.0).max(0.0);
        let origin_y = ui.min_rect().top();
        let mut i = block_at(view, viewport.min.y);
        while let (Some(&top), Some(block)) = (view.tops.get(i), view.book.blocks.get(i)) {
            if top > viewport.max.y {
                break;
            }
            let (font, color, indent) = style(ui, block, size);
            let galley = ui.painter().layout(
                block_text(block).to_owned(),
                font,
                color,
                (width - indent).max(50.0),
            );
            let x = if matches!(block, Block::Break) {
                left + (width - galley.size().x) / 2.0
            } else {
                left + indent
            };
            ui.painter()
                .galley(egui::pos2(x, origin_y + top), galley, color);
            i += 1;
        }
    });
    view.offset = output.state.clamped_offset().y;
    view.view_height = output.inner_rect.height();
    view.pinned = match view.pinned {
        Some((b, None)) => Some((b, Some(view.offset))),
        Some((b, Some(at))) if (at - view.offset).abs() < 1.0 => Some((b, Some(at))),
        _ => None,
    };
    view.top = view
        .pinned
        .map_or_else(|| block_at(view, view.offset + 1.0), |(b, _)| b);

    // Report the reading position (throttled; also right after a jump settles).
    let now = ui.input(|i| i.time);
    let total = view.book.blocks.len();
    if view.reported != Some(view.top) && now - view.last_report > REPORT_EVERY && total > 0 {
        view.reported = Some(view.top);
        view.last_report = now;
        let block = view.top;
        app.act("reader.position", json!({ "block": block, "total": total }));
    } else if view.reported != Some(view.top) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(REPORT_EVERY));
    }
}

fn top_bar(app: &mut RaftUi, ui: &mut egui::Ui, title: &str, id: &str) {
    ui.horizontal(|ui| {
        if ui.button("Înapoi").clicked() {
            app.act("view.library", json!({}));
        }
        ui.separator();
        if let Some(view) = app.text.as_ref().filter(|v| v.book_id == id) {
            let current = view.chapter_of(view.top);
            let name = |i: usize, t: &str| {
                if t.is_empty() {
                    format!("Capitolul {}", i + 1)
                } else {
                    t.to_owned()
                }
            };
            let chapters: Vec<(usize, String)> = view
                .book
                .chapters
                .iter()
                .enumerate()
                .map(|(i, c)| (c.start, name(i, &c.title)))
                .collect();
            let selected = chapters
                .get(current)
                .map(|c| c.1.clone())
                .unwrap_or_default();
            let mut goto = None;
            egui::ComboBox::from_label("Cuprins")
                .selected_text(egui::RichText::new(selected))
                .width(260.0)
                .height(500.0)
                .show_ui(ui, |ui| {
                    for (i, (start, name)) in chapters.iter().enumerate() {
                        if ui.selectable_label(i == current, name).clicked() {
                            goto = Some(*start);
                        }
                    }
                });
            let percent = if view.book.blocks.len() > 1 {
                view.top as f32 / (view.book.blocks.len() - 1) as f32 * 100.0
            } else {
                100.0
            };
            ui.label(format!("{percent:.0}%"));
            if let Some(start) = goto {
                let total = view.book.blocks.len();
                if let Some(v) = app.text.as_mut() {
                    v.jump = Some(Jump::Block(start));
                    v.top = start;
                    v.reported = Some(start);
                    v.pinned = Some((start, None));
                }
                app.act("reader.position", json!({ "block": start, "total": total }));
            }
        }
        ui.separator();
        if ui.button("A−").on_hover_text("Litere mai mici").clicked() {
            app.text_prefs.font_size = (app.text_prefs.font_size - 2.0).max(12.0);
        }
        if ui.button("A+").on_hover_text("Litere mai mari").clicked() {
            app.text_prefs.font_size = (app.text_prefs.font_size + 2.0).min(40.0);
        }
        ui.separator();
        ui.add(egui::Label::new(egui::RichText::new(title).strong()).truncate());
    });
}

/// Page keys scroll the text by a screen (they are not document changes).
fn keys(app: &mut RaftUi, ui: &egui::Ui) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let Some(view) = app.text.as_mut() else {
        return;
    };
    let page = (view.view_height * 0.9).max(40.0);
    let step = ui.input(|i| {
        if i.key_pressed(egui::Key::PageDown)
            || i.key_pressed(egui::Key::Space)
            || i.key_pressed(egui::Key::ArrowRight)
        {
            Some(page)
        } else if i.key_pressed(egui::Key::PageUp) || i.key_pressed(egui::Key::ArrowLeft) {
            Some(-page)
        } else if i.key_pressed(egui::Key::Home) {
            Some(f32::NEG_INFINITY)
        } else if i.key_pressed(egui::Key::End) {
            Some(f32::INFINITY)
        } else {
            None
        }
    });
    if let Some(s) = step {
        let y = (view.offset + s).clamp(0.0, (view.total - view.view_height).max(0.0));
        view.jump = Some(Jump::Offset(y));
    }
}
