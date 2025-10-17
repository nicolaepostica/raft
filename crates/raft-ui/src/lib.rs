//! Raft egui interface (layer 6).
//!
//! The UI never changes the session directly: every action goes through
//! [`RaftUi::run`], which runs a registered command.

mod book;
mod covers;
mod edit;
mod library;
mod reader;
mod text_reader;

use std::sync::Arc;

use raft_doc::{Book, Route};
use raft_engine::{EngineError, Session, Shortcut, registry};
use raft_text::TextBook;
use serde_json::{Value, json};

pub use covers::decode_image;

/// Host services the UI needs (file access lives in the app).
pub trait Files {
    /// Raw bytes of the book's cover image, if any.
    fn cover(&self, book: &Book) -> Option<Vec<u8>>;
    /// Asks for a rendered page image; returns it once ready (non-blocking).
    /// `Some(Err)` when rendering failed.
    fn page(&self, book: &Book, page: u32, width_px: u32) -> Option<Result<Vec<u8>, String>>;
    /// Asks for an EPUB/FB2 book parsed as text; returns it once ready (non-blocking).
    fn text(&self, book: &Book) -> Option<Result<Arc<TextBook>, String>>;
}

/// Files host that has nothing (tests, web).
#[derive(Debug, Default)]
pub struct NoFiles;

impl Files for NoFiles {
    fn cover(&self, _: &Book) -> Option<Vec<u8>> {
        None
    }
    fn page(&self, _: &Book, _: u32, _: u32) -> Option<Result<Vec<u8>, String>> {
        Some(Err("page rendering is not available".into()))
    }
    fn text(&self, _: &Book) -> Option<Result<Arc<TextBook>, String>> {
        Some(Err("reading files is not available".into()))
    }
}

/// The whole interface state.
pub struct RaftUi {
    pub session: Session,
    files: Box<dyn Files>,
    covers: covers::Covers,
    reader: reader::ReaderView,
    text: Option<text_reader::TextView>,
    text_prefs: text_reader::TextPrefs,
    /// Open metadata correction form, if any.
    edit: Option<edit::Draft>,
    search: String,
    focus_search: bool,
    /// The book whose deletion waits for confirmation.
    confirm_delete: Option<BookId>,
}

impl RaftUi {
    pub fn new(session: Session, files: Box<dyn Files>) -> Self {
        let search = session.query().text.clone();
        Self {
            session,
            files,
            covers: covers::Covers::default(),
            reader: reader::ReaderView::default(),
            text: None,
            text_prefs: text_reader::TextPrefs::default(),
            edit: None,
            search,
            focus_search: false,
            confirm_delete: None,
        }
    }

    /// The single entry point for actions.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, EngineError> {
        self.session.run(id, &params)
    }

    /// Runs a command from a UI event; errors go to the status bar.
    fn act(&mut self, id: &str, params: Value) {
        // The error is already recorded in `session.status`.
        let _ = self.run(id, params);
    }

    /// Draws one frame.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.shortcuts(ui.ctx());
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        match self.session.route().clone() {
            Route::Library => {
                egui::Panel::left("filters")
                    .default_size(250.0)
                    .show(ui, |ui| library::filters(self, ui));
                egui::CentralPanel::default().show(ui, |ui| library::library(self, ui));
            }
            Route::Book { id } | Route::Edit { id } => {
                egui::CentralPanel::default().show(ui, |ui| book::book_page(self, ui, &id));
            }
            Route::Reader { id } => {
                let text = self.session.book(&id).is_some_and(|b| b.format.is_text());
                egui::CentralPanel::default().show(ui, |ui| {
                    if text {
                        text_reader::reader(self, ui, &id);
                    } else {
                        reader::reader(self, ui, &id);
                    }
                });
            }
        }
        self.delete_dialog(ui.ctx());
    }

    /// Asks before a book's file goes to the trash.
    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_delete.clone() else {
            return;
        };
        let Some(book) = self.session.book(&id).cloned() else {
            self.confirm_delete = None;
            return;
        };
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("confirm-delete")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading("Ștergi cartea?");
            ui.add_space(6.0);
            ui.label(egui::RichText::new(book.display_title()).strong());
            ui.add(
                egui::Label::new(format!(
                    "Fișierul „{}” se mută în coșul de gunoi, iar cartea dispare din bibliotecă. \
                     Îl poți recupera din coș; corecturile și progresul rămân în raft.db.",
                    book.filename
                ))
                .wrap(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Mută în coș").clicked() {
                    let ok = self
                        .run("book.delete", json!({ "id": id, "confirm": true }))
                        .is_ok();
                    if ok {
                        self.session.status =
                            Some(format!("„{}” a fost mutată în coș", book.display_title()));
                        self.edit = None;
                    }
                    close = true;
                }
                if ui.button("Renunță").clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.confirm_delete = None;
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.confirm_delete.is_some() {
            // Esc closes the dialog instead of going back.
            return;
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::F,
            ))
        }) {
            self.focus_search = true;
        }
        let typing = ctx.egui_wants_keyboard_input();
        for spec in registry() {
            for sc in spec.shortcut.iter().chain(spec.also) {
                let Some(shortcut) = keyboard_shortcut(*sc) else {
                    continue;
                };
                let plain = !sc.command && !sc.alt;
                if typing && plain {
                    continue;
                }
                if self.session.enabled(spec.id).is_ok()
                    && ctx.input_mut(|i| i.consume_shortcut(&shortcut))
                {
                    self.act(spec.id, Value::Null);
                    break;
                }
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            for (menu, title) in [
                ("File", "Fișier"),
                ("Edit", "Editare"),
                ("View", "Vizualizare"),
            ] {
                ui.menu_button(title, |ui| {
                    for spec in registry() {
                        let Some(path) = spec.menu else { continue };
                        if path.split('/').next() != Some(menu) {
                            continue;
                        }
                        let mut button = egui::Button::new(spec.label);
                        if let Some(sc) = spec.shortcut.and_then(keyboard_shortcut) {
                            button = button.shortcut_text(ui.ctx().format_shortcut(&sc));
                        }
                        let enabled = self.session.enabled(spec.id);
                        let resp = ui.add_enabled(enabled.is_ok(), button);
                        let resp = match &enabled {
                            Err(reason) => resp.on_disabled_hover_text(reason),
                            Ok(()) => resp,
                        };
                        if resp.clicked() {
                            self.act(spec.id, Value::Null);
                            ui.close();
                        }
                    }
                });
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let total = self.session.catalog().books.len();
            ui.label(format!(
                "{} din {} cărți",
                self.session.result_count(),
                total
            ));
            if let Some(status) = &self.session.status {
                ui.separator();
                ui.colored_label(ui.visuals().warn_fg_color, status);
            }
        });
    }

    fn open_book_button(&mut self, ui: &mut egui::Ui, book: &Book) {
        if ui.button("Citește").clicked() {
            self.act("book.read", json!({ "id": book.id }));
        }
    }
}

/// A large icon button with a hover text; `name` is its accessible label.
pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    icon: &str,
    name: &str,
    hint: &str,
    enabled: bool,
) -> egui::Response {
    let button =
        egui::Button::new(egui::RichText::new(icon).size(20.0)).min_size(egui::vec2(36.0, 30.0));
    let resp = ui.add_enabled(enabled, button);
    let name = name.to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &name));
    resp.on_hover_text(hint).on_disabled_hover_text(hint)
}

/// The back icon (Backspace, Alt+Left or Esc).
pub(crate) fn back_button(app: &mut RaftUi, ui: &mut egui::Ui, hint: &str) {
    let hint = format!("{hint} (Backspace, Alt+←, Esc)");
    if icon_button(ui, "⬅", "Înapoi", &hint, true).clicked() {
        app.act("view.library", serde_json::json!({}));
    }
}

/// Maps a command shortcut to egui's.
pub fn keyboard_shortcut(sc: Shortcut) -> Option<egui::KeyboardShortcut> {
    let key = egui::Key::from_name(sc.key)?;
    let mut m = egui::Modifiers::NONE;
    if sc.command {
        m |= egui::Modifiers::COMMAND;
    }
    if sc.shift {
        m |= egui::Modifiers::SHIFT;
    }
    if sc.alt {
        m |= egui::Modifiers::ALT;
    }
    Some(egui::KeyboardShortcut::new(m, key))
}

/// Display name of a language code.
pub fn language_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "ru" => "Русский",
        "ro" => "Română",
        "uk" => "Українська",
        "de" => "Deutsch",
        "fr" => "Français",
        "" => "necunoscută",
        other => other,
    }
}

#[cfg(test)]
mod tests;
