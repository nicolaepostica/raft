//! Book page: cover, metadata, description and actions.

use serde_json::json;

use crate::{RaftUi, language_name};

const SAVE: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);

/// A large icon button with a hover text; `name` is its accessible label.
fn icon_button(
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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
    resp.on_hover_text(hint).on_disabled_hover_text(hint)
}

pub fn book_page(app: &mut RaftUi, ui: &mut egui::Ui, id: &str) {
    let Some(book) = app.session.book(id).cloned() else {
        ui.label("Cartea nu mai există în catalog.");
        return;
    };
    let editing = matches!(app.session.route(), raft_doc::Route::Edit { .. });
    ui.horizontal(|ui| {
        if ui.button("Înapoi la bibliotecă").clicked() {
            app.act("view.library", json!({}));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // One spot: the pencil opens the form, which turns it into the save icon.
            if editing {
                let changed = crate::edit::has_changes(app, &book);
                let hint = if changed {
                    "Salvează (Ctrl+S)"
                } else {
                    "Salvează: nu ai modificat nimic"
                };
                let save = ui.input_mut(|i| i.consume_shortcut(&SAVE));
                if icon_button(ui, "💾", "Salvează", hint, changed).clicked() || save {
                    crate::edit::save(app, &book);
                }
            } else if icon_button(
                ui,
                "✏",
                "Corectează metadatele",
                "Corectează metadatele",
                true,
            )
            .clicked()
            {
                app.act("view.edit", json!({ "id": book.id }));
            }
        });
    });
    // Saving leaves the form; do not draw it again this frame from the old book.
    let editing = matches!(app.session.route(), raft_doc::Route::Edit { .. });
    ui.add_space(8.0);
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
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
                if editing {
                    ui.add_space(8.0);
                    if ui
                        .button("Deschide în aplicația externă")
                        .on_hover_text("Ca să verifici datele în carte")
                        .clicked()
                    {
                        app.act("book.open_external", json!({ "id": book.id }));
                    }
                }
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                if editing {
                    crate::edit::form(app, ui, &book);
                    return;
                }
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
                let edited = app.session.edited_fields(&book.id).len();
                if edited > 0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "Corectată manual: {edited} câmp(uri); restul vin din catalog"
                        ))
                        .small()
                        .color(egui::Color32::from_rgb(70, 140, 230)),
                    );
                }
                let missing: Vec<&str> =
                    book.missing().into_iter().map(crate::edit::label).collect();
                if !missing.is_empty() || book.reviewed {
                    ui.horizontal_wrapped(|ui| {
                        let text = if missing.is_empty() {
                            "Nu lipsește nimic".to_owned()
                        } else {
                            format!("Lipsește: {}", missing.join(", ").to_lowercase())
                        };
                        let text = egui::RichText::new(text);
                        if book.is_incomplete() {
                            ui.label(text.color(ui.visuals().warn_fg_color));
                        } else {
                            ui.label(text.weak());
                        }
                        let mut reviewed = book.reviewed;
                        if ui
                            .checkbox(&mut reviewed, "verificată")
                            .on_hover_text(
                                "Ce lipsește nu se poate afla; cartea nu mai apare la „date incomplete”",
                            )
                            .changed()
                        {
                            app.act("book.review", json!({ "id": book.id, "on": reviewed }));
                        }
                    });
                }
                if let Some(p) = app.session.progress(&book.id).cloned() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let text = match (p.page, p.block()) {
                            (Some(page), _) => {
                                format!("Ai ajuns la pagina {page} ({:.0}%)", p.percent)
                            }
                            (None, Some(_)) => format!("Ai citit {:.0}% din carte", p.percent),
                            (None, None) => "Deschisă în aplicația externă".to_owned(),
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
