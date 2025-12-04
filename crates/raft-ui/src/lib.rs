//! Raft egui interface (layer 6).
//!
//! The UI never changes the session directly: every action goes through
//! [`RaftUi::run`], which runs a registered command.

mod book;
mod covers;
mod edit;
pub mod i18n;
mod library;
mod reader;
mod settings;
mod text_reader;
pub mod theme;

use i18n::{t, tf};

use std::sync::Arc;

use raft_doc::{Book, BookId, Route};
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
    /// Whether the book has a cover chosen by the user.
    fn has_custom_cover(&self, _book: &Book) -> bool {
        false
    }
    /// Opens a file chooser for an image; the path (or `None` if cancelled)
    /// arrives on the channel. `None` when choosing files is not possible.
    fn pick_image(&self, _title: &str) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
        None
    }
    /// Opens a folder chooser, like [`Files::pick_image`].
    fn pick_folder(&self, _title: &str) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
        None
    }
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
    /// Open metadata correction form, if any.
    edit: Option<edit::Draft>,
    search: String,
    focus_search: bool,
    /// The book whose deletion waits for confirmation.
    confirm_delete: Option<BookId>,
    /// The "move the selected books to the trash?" dialog is open.
    confirm_bulk_delete: bool,
    /// The theme last installed in egui (`Some(dark)`).
    themed: Option<bool>,
    /// A cover file chooser that is open, for this book.
    picking: Option<(Pick, std::sync::mpsc::Receiver<Option<String>>)>,
    /// Folder names being typed in the settings window, by folder path.
    source_names: std::collections::BTreeMap<String, String>,
    /// What is typed in the settings' shortcut search.
    shortcut_filter: String,
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
            edit: None,
            search,
            focus_search: false,
            confirm_delete: None,
            confirm_bulk_delete: false,
            picking: None,
            source_names: Default::default(),
            shortcut_filter: String::new(),
            themed: None,
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
        i18n::set(i18n::Lang::from_code(self.session.language()).unwrap_or(i18n::Lang::Ro));
        let dark = self.session.dark();
        if self.themed != Some(dark) {
            // Ctrl +/−/0 zoom the book (commands), not the whole interface.
            ui.ctx().options_mut(|o| o.zoom_with_keyboard = false);
            theme::apply(ui.ctx(), dark);
            self.themed = Some(dark);
        }
        if self.session.poll_indexing() {
            // A folder's catalog is being built: follow its progress.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.shortcuts(ui.ctx());
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        match self.session.route().clone() {
            Route::Library => {
                egui::Panel::top("library-tabs").show(ui, |ui| library::tabs(self, ui));
                if self.session.library_tab() == "reading" {
                    egui::CentralPanel::default().show(ui, |ui| library::reading(self, ui));
                } else {
                    egui::Panel::left("filters")
                        .default_size(250.0)
                        .show(ui, |ui| library::filters(self, ui));
                    egui::CentralPanel::default().show(ui, |ui| library::library(self, ui));
                }
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
        settings::window(self, ui.ctx());
        self.delete_dialog(ui.ctx());
        self.bulk_delete_dialog(ui.ctx());
        self.poll_picker(ui.ctx());
    }

    /// Applies what was chosen in an open file chooser, once it arrives.
    fn poll_picker(&mut self, ctx: &egui::Context) {
        let Some((pick, rx)) = &self.picking else {
            return;
        };
        match rx.try_recv() {
            Ok(Some(path)) => {
                let pick = pick.clone();
                self.picking = None;
                match pick {
                    Pick::Cover(id) => {
                        self.act("book.cover.set", json!({ "id": id, "path": path }));
                    }
                    Pick::Folder => self.act("source.add", json!({ "path": path })),
                    Pick::Move(from) => {
                        self.act("source.edit", json!({ "path": from, "new_path": path }));
                    }
                }
            }
            Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => self.picking = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
    }

    /// Asks before the selected books' files go to the trash.
    fn bulk_delete_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_bulk_delete {
            return;
        }
        let books: Vec<Book> = self
            .session
            .selected()
            .iter()
            .filter_map(|id| self.session.book(id).cloned())
            .collect();
        if books.is_empty() {
            self.confirm_bulk_delete = false;
            return;
        }
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("confirm-bulk-delete")).show(ctx, |ui| {
            ui.set_max_width(520.0);
            ui.heading(tf("Ștergi {0} cărți?", &[&books.len()]));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for b in &books {
                    ui.label(format!("• {}  ({})", b.display_title(), b.filename));
                }
            });
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(t(
                    "Fișierele se mută în coșul de gunoi, iar cărțile dispar din bibliotecă. Le poți recupera din coș; corecturile și progresul rămân în raft.db.",
                ))
                .wrap(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(t("Mută în coș")).clicked() {
                    // An error is already in the status bar.
                    if let Ok(out) = self.run("books.delete", json!({ "confirm": true })) {
                        let removed = out.get("removed").and_then(Value::as_u64).unwrap_or(0);
                        let failed = out
                            .get("failed")
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len);
                        self.session.status = Some(if failed == 0 {
                            tf("{0} cărți mutate în coș", &[&removed])
                        } else {
                            tf(
                                "{0} cărți mutate în coș; {1} nu au putut fi mutate (rămân selectate)",
                                &[&removed, &failed],
                            )
                        });
                        self.edit = None;
                    }
                    close = true;
                }
                if ui.button(t("Renunță")).clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.confirm_bulk_delete = false;
        }
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
        let modal = egui::Modal::new(egui::Id::unique("confirm-delete")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(t("Ștergi cartea?"));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(book.display_title()).strong());
            ui.add(
                egui::Label::new(tf(
                    "Fișierul „{0}” se mută în coșul de gunoi, iar cartea dispare din bibliotecă. Îl poți recupera din coș; corecturile și progresul rămân în raft.db.",
                    &[&book.filename],
                ))
                .wrap(),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(t("Mută în coș")).clicked() {
                    let ok = self
                        .run("book.delete", json!({ "id": id, "confirm": true }))
                        .is_ok();
                    if ok {
                        self.session.status =
                            Some(tf("„{0}” a fost mutată în coș", &[&book.display_title()]));
                        self.edit = None;
                    }
                    close = true;
                }
                if ui.button(t("Renunță")).clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.confirm_delete = None;
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.confirm_delete.is_some() || self.confirm_bulk_delete {
            // Esc closes the dialog instead of going back.
            return;
        }
        if !self.session.selected().is_empty()
            && !self.session.settings_open()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.act("select.clear", json!({}));
        }
        if self.session.settings_open()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.act("view.settings", json!({ "open": false }));
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
                ("File", t("Fișier")),
                ("Edit", t("Editare")),
                ("View", t("Vizualizare")),
            ] {
                ui.menu_button(title, |ui| {
                    for spec in registry() {
                        let Some(path) = spec.menu else { continue };
                        if path.split('/').next() != Some(menu) {
                            continue;
                        }
                        self.menu_item(ui, spec);
                    }
                });
            }
        });
    }

    /// One menu entry for a command: label, shortcut, enabled state.
    fn menu_item(&mut self, ui: &mut egui::Ui, spec: &raft_engine::CommandSpec) {
        let mut button = egui::Button::new(i18n::t_dyn(spec.label));
        if let Some(sc) = spec.shortcut.and_then(keyboard_shortcut) {
            button = button.shortcut_text(shortcut_text(ui.ctx(), &sc));
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

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let total = self.session.catalog().books.len();
            ui.label(tf(
                "{0} din {1} cărți",
                &[&self.session.result_count(), &total],
            ));
            for entry in self.session.sources() {
                if let Some(p) = entry.indexing {
                    ui.separator();
                    ui.spinner();
                    ui.label(indexing_label(entry.source.display_name(), p));
                }
            }
            if let Some(status) = &self.session.status {
                ui.separator();
                ui.colored_label(ui.visuals().warn_fg_color, status);
            }
        });
    }
}

/// "Catalog of X: 3 of 10 books", while a folder's catalog is being built.
pub(crate) fn indexing_label(name: &str, p: raft_engine::IndexProgress) -> String {
    if p.total == 0 {
        tf("Se face catalogul pentru {0}…", &[&name])
    } else {
        tf(
            "Se face catalogul pentru {0}: {1} din {2} cărți",
            &[&name, &p.done, &p.total],
        )
    }
}

/// What an open file chooser is for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Pick {
    /// A new cover for this book.
    Cover(BookId),
    /// A library folder to add.
    Folder,
    /// The new place of the library folder at this path.
    Move(String),
}

/// Opens the folder chooser for a new library folder.
pub(crate) fn add_folder(app: &mut RaftUi) {
    pick_folder(app, Pick::Folder);
}

/// Opens the folder chooser, for a new folder or a moved one.
pub(crate) fn pick_folder(app: &mut RaftUi, pick: Pick) {
    match app.files.pick_folder(t("Alege folderul bibliotecii")) {
        Some(rx) => app.picking = Some((pick, rx)),
        None => {
            app.session.status = Some(t("alegerea fișierelor nu e disponibilă aici").into());
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

/// The reader's zoom: smaller, the zoom in percent (a click resets it), larger.
pub(crate) fn zoom_controls(app: &mut RaftUi, ui: &mut egui::Ui) {
    let zoom = app.session.zoom();
    if ui
        .add_enabled(
            app.session.enabled("reader.zoom_out").is_ok(),
            egui::Button::new("−"),
        )
        .on_hover_text(t("Micșorează (Ctrl+−, Ctrl+rotița)"))
        .clicked()
    {
        app.act("reader.zoom_out", json!({}));
    }
    if ui
        .button(format!("{:.0}%", zoom * 100.0))
        .on_hover_text(t("Mărime normală (Ctrl+0)"))
        .clicked()
    {
        app.act("reader.zoom_reset", json!({}));
    }
    if ui
        .add_enabled(
            app.session.enabled("reader.zoom_in").is_ok(),
            egui::Button::new("+"),
        )
        .on_hover_text(t("Mărește (Ctrl+=, Ctrl+rotița)"))
        .clicked()
    {
        app.act("reader.zoom_in", json!({}));
    }
}

/// Ctrl + mouse wheel (or a pinch) over the reader zooms the book.
pub(crate) fn wheel_zoom(app: &mut RaftUi, ui: &egui::Ui) {
    let over = ui.rect_contains_pointer(ui.max_rect());
    let delta = ui.input(|i| i.zoom_delta());
    if over && (delta - 1.0).abs() > 0.001 {
        let zoom = app.session.zoom() * delta;
        app.act("reader.zoom", json!({ "zoom": zoom }));
    }
}

/// The back icon (Backspace, Alt+Left or Esc).
pub(crate) fn back_button(app: &mut RaftUi, ui: &mut egui::Ui, hint: &str) {
    let hint = format!("{hint} (Backspace, Alt+←, Esc)");
    if icon_button(ui, "⬅", t("Înapoi"), &hint, true).clicked() {
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

/// A shortcut as menus show it; punctuation keys by their sign (`Ctrl+,`, not `Ctrl+Comma`).
fn shortcut_text(ctx: &egui::Context, sc: &egui::KeyboardShortcut) -> String {
    let text = ctx.format_shortcut(sc);
    let (name, sign) = (sc.logical_key.name(), sc.logical_key.symbol_or_name());
    match text.strip_suffix(name) {
        Some(mods) if sign.is_ascii() => format!("{mods}{sign}"),
        _ => text,
    }
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
        "" => t("necunoscută"),
        other => other,
    }
}

#[cfg(test)]
mod tests;
