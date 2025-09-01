//! Library screen: filters panel, search bar, "continue reading" and the book grid.

use raft_doc::{Book, Format, Sort};
use serde_json::json;

use crate::{RaftUi, language_name};

const CARD_W: f32 = 150.0;
const COVER_H: f32 = 210.0;
const CARD_H: f32 = COVER_H + 70.0;
const GAP: f32 = 14.0;
/// Categories listed before "show more".
const TOP_CATEGORIES: usize = 40;

fn sort_label(s: Sort) -> &'static str {
    match s {
        Sort::Relevance => "Relevanță",
        Sort::Title => "Titlu",
        Sort::YearDesc => "An (noi întâi)",
        Sort::YearAsc => "An (vechi întâi)",
        Sort::RecentlyAdded => "Adăugate recent",
    }
}

fn sort_slug(s: Sort) -> &'static str {
    match s {
        Sort::Relevance => "relevance",
        Sort::Title => "title",
        Sort::YearDesc => "year_desc",
        Sort::YearAsc => "year_asc",
        Sort::RecentlyAdded => "recently_added",
    }
}

pub fn filters(app: &mut RaftUi, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let facets = app.session.facets().clone();
        let filters = app.session.query().filters.clone();

        ui.heading("Filtre");
        if ui
            .add_enabled(!filters.is_empty(), egui::Button::new("Șterge filtrele"))
            .clicked()
        {
            app.act("filter.clear", json!({}));
        }
        ui.add_space(6.0);

        ui.strong("Limbă");
        for (lang, n) in &facets.languages {
            let mut on = filters.languages.contains(lang);
            if ui
                .checkbox(&mut on, format!("{}  ({n})", language_name(lang)))
                .changed()
            {
                app.act("filter.language.toggle", json!({ "language": lang }));
            }
        }
        ui.add_space(6.0);

        ui.strong("Format");
        for (format, n) in &facets.formats {
            let mut on = filters.formats.contains(format);
            let label = format!("{}  ({n})", format.slug().to_uppercase());
            if ui.checkbox(&mut on, label).changed() {
                app.act("filter.format.toggle", json!({ "format": format.slug() }));
            }
        }
        ui.add_space(6.0);

        ui.strong("Ani");
        let (lo, hi) = facets.years.unwrap_or((1900, 2030));
        let mut min = filters.year_min.unwrap_or(lo);
        let mut max = filters.year_max.unwrap_or(hi);
        let changed = ui
            .horizontal(|ui| {
                let from = ui.label("de la");
                let a = ui
                    .add(egui::DragValue::new(&mut min).range(1000..=max))
                    .labelled_by(from.id);
                let to = ui.label("până la");
                let b = ui
                    .add(egui::DragValue::new(&mut max).range(min..=3000))
                    .labelled_by(to.id);
                a.changed() || b.changed()
            })
            .inner;
        if changed {
            app.act("filter.years", json!({ "min": min, "max": max }));
        }
        if (filters.year_min.is_some() || filters.year_max.is_some())
            && ui.small_button("orice an").clicked()
        {
            app.act("filter.years", json!({ "min": null, "max": null }));
        }
        ui.add_space(6.0);

        ui.strong("Categorii");
        let mut primary = filters.primary_category_only;
        if ui
            .checkbox(&mut primary, "doar categoria principală")
            .on_hover_text("Ignoră categoriile secundare (de ex. „career”, atribuită prea des)")
            .changed()
        {
            app.act("filter.primary_only", json!({ "on": primary }));
        }
        for (slug, n) in facets.categories.iter().take(TOP_CATEGORIES) {
            let mut on = filters.categories.contains(slug);
            let name = app.session.catalog().category_name(slug).to_owned();
            if ui.checkbox(&mut on, format!("{name}  ({n})")).changed() {
                app.act("filter.category.toggle", json!({ "slug": slug }));
            }
        }
    });
}

pub fn library(app: &mut RaftUi, ui: &mut egui::Ui) {
    search_bar(app, ui);
    ui.add_space(8.0);

    let show_continue =
        app.session.query().text.is_empty() && app.session.query().filters.is_empty();
    let recent: Vec<Book> = if show_continue {
        app.session
            .continue_reading(12)
            .into_iter()
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    if !recent.is_empty() {
        ui.strong("Continuă lectura");
        egui::ScrollArea::horizontal()
            .id_salt("continue")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for book in &recent {
                        card(app, ui, book);
                    }
                });
            });
        ui.separator();
    }

    let results: Vec<Book> = app.session.results().cloned().collect();
    if results.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| ui.label("Nicio carte nu se potrivește căutării."));
        return;
    }
    let cols = (((ui.available_width() + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
    let rows = results.len().div_ceil(cols);
    egui::ScrollArea::vertical()
        .id_salt("grid")
        .auto_shrink([false, false])
        .show_rows(ui, CARD_H + GAP, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for book in results.iter().skip(row * cols).take(cols) {
                        card(app, ui, book);
                    }
                });
                ui.add_space(GAP);
            }
        });
}

fn search_bar(app: &mut RaftUi, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let edit = egui::TextEdit::singleline(&mut app.search)
            .hint_text("Caută după titlu, autor, etichetă sau conținut…  (Ctrl+F)")
            .desired_width((ui.available_width() - 260.0).max(120.0));
        let resp = ui.add(edit);
        if app.focus_search {
            resp.request_focus();
            app.focus_search = false;
        }
        if resp.changed() {
            let text = app.search.clone();
            app.act("search.set", json!({ "text": text }));
        } else if !resp.has_focus() {
            app.search.clone_from(&app.session.query().text);
        }
        let current = app.session.query().sort;
        egui::ComboBox::from_label("Sortare")
            .selected_text(sort_label(current))
            .show_ui(ui, |ui| {
                for s in Sort::ALL {
                    if ui.selectable_label(s == current, sort_label(s)).clicked() {
                        app.act("sort.set", json!({ "sort": sort_slug(s) }));
                    }
                }
            });
    });
}

/// One book card; click opens the book page.
fn card(app: &mut RaftUi, ui: &mut egui::Ui, book: &Book) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(CARD_W, CARD_H), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let visuals = ui.visuals().clone();
    let cover_rect = egui::Rect::from_min_size(rect.min, egui::vec2(CARD_W, COVER_H));
    let painter = ui.painter_at(rect);
    let tex = app.covers.get(ui.ctx(), app.files.as_ref(), book);
    match tex {
        Some(t) => {
            let size = t.size_vec2();
            let scale = (CARD_W / size.x).min(COVER_H / size.y);
            let draw = egui::Rect::from_center_size(cover_rect.center(), size * scale);
            painter.image(
                t.id(),
                draw,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(cover_rect, 6.0, visuals.faint_bg_color);
            painter.text(
                cover_rect.center(),
                egui::Align2::CENTER_CENTER,
                book.format.slug().to_uppercase(),
                egui::FontId::proportional(18.0),
                visuals.weak_text_color(),
            );
        }
    }
    if let Some(p) = app.session.progress(&book.id) {
        let bar = egui::Rect::from_min_size(
            egui::pos2(cover_rect.left(), cover_rect.bottom() - 4.0),
            egui::vec2(CARD_W * (p.percent / 100.0).clamp(0.0, 1.0), 4.0),
        );
        painter.rect_filled(bar, 0.0, visuals.selection.bg_fill);
    }
    let text_rect =
        egui::Rect::from_min_max(egui::pos2(rect.left(), cover_rect.bottom() + 6.0), rect.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text_rect));
    child.add(egui::Label::new(egui::RichText::new(book.display_title()).strong()).truncate());
    let meta = match (book.authors.first(), book.year) {
        (Some(a), Some(y)) => format!("{a} · {y}"),
        (Some(a), None) => a.clone(),
        (None, Some(y)) => y.to_string(),
        (None, None) => String::new(),
    };
    child.add(egui::Label::new(egui::RichText::new(meta).small().weak()).truncate());
    if book.format != Format::Pdf {
        child.label(
            egui::RichText::new(book.format.slug().to_uppercase())
                .small()
                .weak(),
        );
    }
    if resp.hovered() {
        painter.rect_stroke(
            cover_rect,
            6.0,
            visuals.selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let resp = resp.on_hover_text(book.display_title());
    if resp.clicked() {
        app.act("view.book", json!({ "id": book.id }));
    }
}
