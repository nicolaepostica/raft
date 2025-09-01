//! Book page: cover, metadata, description and actions.

use serde_json::json;

use crate::{RaftUi, language_name};

pub fn book_page(app: &mut RaftUi, ui: &mut egui::Ui, id: &str) {
    let Some(book) = app.session.book(id).cloned() else {
        ui.label("Cartea nu mai există în catalog.");
        return;
    };
    if ui.button("Înapoi la bibliotecă").clicked() {
        app.act("view.library", json!({}));
    }
    ui.add_space(8.0);
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal_top(|ui| {
            match app.covers.get(ui.ctx(), app.files.as_ref(), &book) {
                Some(t) => {
                    let size = t.size_vec2();
                    let scale = (300.0 / size.x).min(420.0 / size.y);
                    ui.add(egui::Image::new((t.id(), size * scale)));
                }
                None => {
                    ui.add_sized([300.0, 420.0], egui::Label::new("fără copertă"));
                }
            }
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.heading(book.display_title());
                if !book.authors.is_empty() {
                    ui.label(egui::RichText::new(book.authors_line()).size(16.0));
                }
                ui.add_space(8.0);
                egui::Grid::new("meta")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        let mut row = |k: &str, v: String| {
                            if !v.is_empty() {
                                ui.label(egui::RichText::new(k).weak());
                                ui.label(v);
                                ui.end_row();
                            }
                        };
                        row("An", book.year.map(|y| y.to_string()).unwrap_or_default());
                        row("Limbă", language_name(&book.language).to_owned());
                        row("Format", book.format.slug().to_uppercase());
                        row(
                            "Pagini",
                            book.pages.map(|p| p.to_string()).unwrap_or_default(),
                        );
                        row("Editură", book.publisher.clone().unwrap_or_default());
                        row("ISBN", book.isbn.clone().unwrap_or_default());
                        row("Fișier", book.filename.clone());
                        row(
                            "Mărime",
                            format!("{:.1} MB", book.size as f64 / 1_048_576.0),
                        );
                    });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for slug in &book.categories {
                        let name = app.session.catalog().category_name(slug).to_owned();
                        if ui
                            .button(name)
                            .on_hover_text("Arată cărțile din categorie")
                            .clicked()
                        {
                            app.act("filter.clear", json!({}));
                            app.act("filter.category.toggle", json!({ "slug": slug }));
                            app.act("view.library", json!({}));
                        }
                    }
                });
                if !book.tags.is_empty() {
                    ui.label(egui::RichText::new(book.tags.join(" · ")).small().weak());
                }
                ui.add_space(8.0);
                if !book.description.is_empty() {
                    ui.add(egui::Label::new(&book.description).wrap());
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    app.open_book_button(ui, &book);
                    if ui.button("Deschide în aplicația externă").clicked() {
                        app.act("book.open_external", json!({ "id": book.id }));
                    }
                });
                if let Some(p) = app.session.progress(&book.id).cloned() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let text = match p.page {
                            Some(page) => format!("Ai ajuns la pagina {page} ({:.0}%)", p.percent),
                            None => "Deschisă în aplicația externă".to_owned(),
                        };
                        ui.label(text);
                        if ui.small_button("Șterge progresul").clicked() {
                            app.act("progress.reset", json!({ "id": book.id }));
                        }
                    });
                }
            });
        });
    });
}
