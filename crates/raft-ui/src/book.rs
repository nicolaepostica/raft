//! Book page: cover, metadata, description and actions.

use serde_json::json;

use crate::{RaftUi, language_name};

const SAVE: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);

/// The book's actions as icons, under the cover (same in both modes).
fn actions(app: &mut RaftUi, ui: &mut egui::Ui, book: &raft_doc::Book) {
    let id = book.id.clone();
    let progress = app.session.progress(&id).cloned();
    ui.horizontal(|ui| {
        let read = match progress.as_ref() {
            Some(p) if p.page.is_some_and(|n| n > 1) => {
                format!("Continuă de la pagina {}", p.page.unwrap_or(1))
            }
            Some(p) if p.block().is_some() => format!("Continuă lectura ({:.0}%)", p.percent),
            _ => "Citește".to_owned(),
        };
        if crate::icon_button(ui, "📖", "Citește", &read, true).clicked() {
            app.act("book.read", json!({ "id": id }));
        }
        if crate::icon_button(
            ui,
            "↗",
            "Deschide în aplicația externă",
            "Deschide în aplicația externă",
            true,
        )
        .clicked()
        {
            app.act("book.open_external", json!({ "id": id }));
        }
        if crate::icon_button(
            ui,
            "📂",
            "Arată în dosar",
            "Arată fișierul în managerul de fișiere",
            true,
        )
        .clicked()
        {
            app.act("book.show_in_folder", json!({ "id": id }));
        }
        let picking = app.picking.as_ref().is_some_and(|(b, _)| *b == id);
        let cover = crate::icon_button(
            ui,
            "🖼",
            "Copertă",
            "Schimbă coperta (sau trage o imagine peste ea)",
            !picking,
        );
        egui::Popup::menu(&cover).show(|ui| {
            if ui.button("Alege o imagine…").clicked() {
                match app.files.pick_image() {
                    Some(rx) => app.picking = Some((id.clone(), rx)),
                    None => {
                        app.session.status =
                            Some("alegerea fișierelor nu e disponibilă aici".into());
                    }
                }
            }
            if book.format.is_paged() {
                if ui.button("Prima pagină a cărții").clicked() {
                    app.act("book.cover.set", json!({ "id": id, "page": 1 }));
                }
                if let Some(n) = progress.as_ref().and_then(|p| p.page).filter(|n| *n > 1)
                    && ui.button(format!("Pagina {n}, unde ai rămas")).clicked()
                {
                    app.act("book.cover.set", json!({ "id": id, "page": n }));
                }
            }
            if app.files.has_custom_cover(book) && ui.button("Coperta din catalog").clicked() {
                app.act("book.cover.set", json!({ "id": id, "catalog": true }));
            }
            ui.label(
                egui::RichText::new("Poți și să tragi o imagine peste copertă.")
                    .small()
                    .weak(),
            );
        });
        ui.add_space(8.0);
        if crate::icon_button(
            ui,
            "🗑",
            "Șterge cartea",
            "Șterge cartea: fișierul merge în coșul de gunoi",
            true,
        )
        .clicked()
        {
            app.confirm_delete = Some(id.clone());
        }
    });
}

/// An image file dropped on the window becomes the cover; while one is dragged
/// over the window, the cover shows where it goes.
fn drop_cover(app: &mut RaftUi, ui: &mut egui::Ui, book: &raft_doc::Book, rect: egui::Rect) {
    let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
    if hovering {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, egui::Color32::from_black_alpha(170));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Lasă imaginea aici\nca să devină copertă",
            egui::FontId::proportional(18.0),
            egui::Color32::WHITE,
        );
    }
    let dropped = ui.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_string_lossy().into_owned())
            .find(|p| !p.is_empty())
    });
    if let Some(path) = dropped {
        app.act("book.cover.set", json!({ "id": book.id, "path": path }));
    }
}

pub fn book_page(app: &mut RaftUi, ui: &mut egui::Ui, id: &str) {
    let Some(book) = app.session.book(id).cloned() else {
        ui.label("Cartea nu mai există în catalog.");
        return;
    };
    let editing = matches!(app.session.route(), raft_doc::Route::Edit { .. });
    ui.horizontal(|ui| {
        let hint = if editing {
            "Înapoi la pagina cărții"
        } else {
            "Înapoi la bibliotecă"
        };
        crate::back_button(app, ui, hint);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // One spot: the pencil opens the form and becomes "cancel"; once
            // something changed, save takes its place and cancel moves next to it.
            if editing {
                let changed = crate::edit::has_changes(app, &book);
                let save = ui.input_mut(|i| i.consume_shortcut(&SAVE));
                if changed
                    && (crate::icon_button(ui, "💾", "Salvează", "Salvează (Ctrl+S)", true)
                        .clicked()
                        || save)
                {
                    crate::edit::save(app, &book);
                }
                let hint = if changed {
                    "Renunță la modificări"
                } else {
                    "Închide editarea"
                };
                if crate::icon_button(ui, "✖", "Renunță", hint, true).clicked() {
                    crate::edit::cancel(app, &book);
                }
            } else if crate::icon_button(
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
                let rev = app.session.cover_rev(&book.id);
                let cover = match app.covers.get(ui.ctx(), app.files.as_ref(), &book, rev) {
                    Some(t) => {
                        let size = t.size_vec2();
                        let scale = (300.0 / size.x).min(420.0 / size.y);
                        ui.add(egui::Image::new((t.id(), size * scale)))
                    }
                    None => ui.add_sized([300.0, 420.0], egui::Label::new("fără copertă")),
                };
                ui.painter().rect_stroke(
                    cover.rect,
                    0.0,
                    ui.visuals().widgets.noninteractive.bg_stroke,
                    egui::StrokeKind::Outside,
                );
                drop_cover(app, ui, &book, cover.rect);
                ui.add_space(8.0);
                actions(app, ui, &book);
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                if editing {
                    crate::edit::form(app, ui, &book);
                    return;
                }
                ui.heading(book.display_title());
                if !book.authors.is_empty() {
                    ui.label(egui::RichText::new(book.authors_line()).size(18.0));
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
                let edited = app.session.edited_fields(&book.id).len();
                if edited > 0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "Corectată manual: {edited} câmp(uri); restul vin din catalog"
                        ))
                        .small()
                        .color(crate::theme::edited()),
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
