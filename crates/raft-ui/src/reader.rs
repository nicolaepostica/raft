//! Paged reader (PDF, DjVu): shows one rendered page at a time.

use serde_json::json;

use crate::RaftUi;

#[derive(Default)]
pub struct ReaderView;

pub fn reader(app: &mut RaftUi, ui: &mut egui::Ui, id: &str) {
    let Some(book) = app.session.book(id).cloned() else {
        ui.label("Cartea nu mai există în catalog.");
        return;
    };
    let page = app.session.progress(id).and_then(|p| p.page).unwrap_or(1);
    ui.horizontal(|ui| {
        if ui.button("Înapoi").clicked() {
            app.act("view.library", json!({}));
        }
        ui.strong(book.display_title());
        ui.label(format!("pagina {page} / {}", book.pages.unwrap_or(0)));
    });
}
