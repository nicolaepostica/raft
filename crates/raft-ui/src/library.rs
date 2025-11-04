//! Library screen: filters panel, search bar, "continue reading" and the book grid.

use raft_doc::edits::Field;
use raft_doc::{Book, Format, Mark, Sort};
use serde_json::json;

use crate::i18n::{t, tf};
use crate::{RaftUi, language_name};

const CARD_W: f32 = 150.0;
const COVER_H: f32 = 210.0;
const CARD_H: f32 = COVER_H + 70.0;
const GAP: f32 = 14.0;
/// Categories listed before "show more".
const TOP_CATEGORIES: usize = 40;

fn sort_label(s: Sort) -> &'static str {
    match s {
        Sort::Relevance => t("Relevanță"),
        Sort::Title => t("Titlu"),
        Sort::YearDesc => t("An (noi întâi)"),
        Sort::YearAsc => t("An (vechi întâi)"),
        Sort::RecentlyAdded => t("Adăugate recent"),
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

        ui.heading(t("Filtre"));
        if ui
            .add_enabled(!filters.is_empty(), egui::Button::new(t("Șterge filtrele")))
            .clicked()
        {
            app.act("filter.clear", json!({}));
        }
        ui.add_space(6.0);

        marks(app, ui, &facets, &filters);
        ui.add_space(6.0);

        ui.strong(t("Limbă"));
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

        ui.strong(t("Format"));
        for (format, n) in &facets.formats {
            let mut on = filters.formats.contains(format);
            let label = format!("{}  ({n})", format.slug().to_uppercase());
            if ui.checkbox(&mut on, label).changed() {
                app.act("filter.format.toggle", json!({ "format": format.slug() }));
            }
        }
        ui.add_space(6.0);

        ui.strong(t("Ani"));
        let (lo, hi) = facets.years.unwrap_or((1900, 2030));
        let mut min = filters.year_min.unwrap_or(lo).clamp(lo, hi);
        let mut max = filters.year_max.unwrap_or(hi).clamp(min, hi);
        let typed = ui
            .horizontal(|ui| {
                let from = ui.label(t("de la"));
                let a = ui
                    .add(egui::DragValue::new(&mut min).range(lo..=max))
                    .labelled_by(from.id);
                let to = ui.label(t("până la"));
                let b = ui
                    .add(egui::DragValue::new(&mut max).range(min..=hi))
                    .labelled_by(to.id);
                a.changed() || b.changed()
            })
            .inner;
        let slid = year_slider(ui, lo, hi, &mut min, &mut max);
        if slid || typed {
            // A handle at the end of the range means "no limit" on that side.
            let min = Some(min).filter(|m| *m > lo);
            let max = Some(max).filter(|m| *m < hi);
            app.act("filter.years", json!({ "min": min, "max": max }));
        }
        if (filters.year_min.is_some() || filters.year_max.is_some())
            && ui.small_button(t("orice an")).clicked()
        {
            app.act("filter.years", json!({ "min": null, "max": null }));
        }
        ui.add_space(6.0);

        ui.strong(t("Categorii"));
        let mut primary = filters.primary_category_only;
        if ui
            .checkbox(&mut primary, t("doar categoria principală"))
            .on_hover_text(t("Ignoră categoriile secundare (de ex. „career”, atribuită prea des)"))
            .changed()
        {
            app.act("filter.primary_only", json!({ "on": primary }));
        }
        for (slug, n) in facets.categories.iter().take(TOP_CATEGORIES) {
            let mut on = filters.categories.contains(slug);
            let name = crate::i18n::t_dyn(app.session.catalog().category_name(slug));
            if ui.checkbox(&mut on, format!("{name}  ({n})")).changed() {
                app.act("filter.category.toggle", json!({ "slug": slug }));
            }
        }
    });
}

/// A slider with two handles over `lo..=hi`; returns true when a handle moved.
/// Dragging moves the handle nearest to where the drag started; a click moves
/// the nearest handle there; with focus, ←/→ move the lower and ↓/↑ the upper.
fn year_slider(ui: &mut egui::Ui, lo: i32, hi: i32, min: &mut i32, max: &mut i32) -> bool {
    let width = ui.available_width().clamp(80.0, 200.0);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::click_and_drag());
    let (from, to) = (*min, *max);
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Slider, true, tf("Ani: {0}–{1}", &[&from, &to]))
    });
    let span = (hi - lo).max(1) as f32;
    let track = rect.shrink2(egui::vec2(8.0, 0.0));
    let x_of = |y: i32| track.left() + (y - lo) as f32 / span * track.width();
    let year_at = |x: f32| {
        let t = ((x - track.left()) / track.width()).clamp(0.0, 1.0);
        lo + (t * span).round() as i32
    };
    let (old_min, old_max) = (*min, *max);
    // Which handle is held: false = lower, true = upper.
    let held = resp.id.with("upper");
    if let Some(pos) = resp.interact_pointer_pos() {
        // Pick the handle on the press itself, before any drag or click is recognized.
        if ui.input(|i| i.pointer.primary_pressed()) {
            let upper = if *min == *max {
                pos.x > x_of(*max)
            } else {
                (pos.x - x_of(*max)).abs() < (pos.x - x_of(*min)).abs()
            };
            ui.data_mut(|d| d.insert_temp(held, upper));
        }
        let upper = ui.data(|d| d.get_temp::<bool>(held)).unwrap_or(false);
        let y = year_at(pos.x);
        if upper {
            *max = y.max(*min);
        } else {
            *min = y.min(*max);
        }
    }
    if resp.has_focus() {
        ui.input(|i| {
            let step = |k| i32::from(i.key_pressed(k));
            *min =
                (*min + step(egui::Key::ArrowRight) - step(egui::Key::ArrowLeft)).clamp(lo, *max);
            *max = (*max + step(egui::Key::ArrowUp) - step(egui::Key::ArrowDown)).clamp(*min, hi);
        });
    }
    let v = ui.visuals();
    let painter = ui.painter_at(rect.expand(2.0));
    let y = rect.center().y;
    painter.line_segment(
        [egui::pos2(track.left(), y), egui::pos2(track.right(), y)],
        egui::Stroke::new(4.0, v.widgets.inactive.bg_fill),
    );
    painter.line_segment(
        [egui::pos2(x_of(*min), y), egui::pos2(x_of(*max), y)],
        egui::Stroke::new(4.0, v.selection.bg_fill),
    );
    let stroke = if resp.has_focus() {
        v.selection.stroke
    } else {
        v.widgets.inactive.fg_stroke
    };
    for x in [x_of(*min), x_of(*max)] {
        painter.circle(egui::pos2(x, y), 7.0, v.widgets.active.bg_fill, stroke);
    }
    resp.on_hover_text(t("Trage capetele ca să alegi intervalul de ani"));
    (*min, *max) != (old_min, old_max)
}

fn mark_label(m: Mark) -> &'static str {
    match m {
        Mark::Edited => t("Corectate manual"),
        Mark::Incomplete => t("Date incomplete"),
        Mark::Reviewed => t("Verificate"),
    }
}

/// The "Date" section: marks, and the fields that are missing.
fn marks(
    app: &mut RaftUi,
    ui: &mut egui::Ui,
    facets: &raft_engine::Facets,
    filters: &raft_doc::Filters,
) {
    ui.strong(t("Date"));
    let count = |m: Mark| {
        facets
            .marks
            .iter()
            .find(|(k, _)| *k == m)
            .map_or(0, |(_, n)| *n)
    };
    for m in Mark::ALL {
        let mut on = filters.marks.contains(&m);
        let hint = match m {
            Mark::Edited => t("Cărți în care ai corectat cel puțin un câmp"),
            Mark::Incomplete => {
                t("Fără titlu, autori, an, limbă, categorii sau descriere și nemarcate ca verificate")
            }
            Mark::Reviewed => t("Marcate de tine ca verificate"),
        };
        if ui
            .checkbox(&mut on, format!("{}  ({})", mark_label(m), count(m)))
            .on_hover_text(hint)
            .changed()
        {
            app.act("filter.mark.toggle", json!({ "mark": m.slug() }));
        }
    }
    let open = !filters.missing.is_empty();
    egui::CollapsingHeader::new(t("Lipsește câmpul…")).id_salt("missing-fields")
        .default_open(open)
        .show(ui, |ui| {
            for (field, n) in &facets.missing {
                let mut on = filters.missing.contains(field);
                let name = crate::edit::label(*field);
                let name = if Field::REQUIRED.contains(field) {
                    egui::RichText::new(format!("{name}  ({n})"))
                } else {
                    egui::RichText::new(format!("{name}  ({n})")).weak()
                };
                if ui.checkbox(&mut on, name).changed() {
                    app.act("filter.missing.toggle", json!({ "field": field.slug() }));
                }
            }
        });
}

/// A small label in a filled rounded box, its top-left corner at `pos`; returns its height.
fn badge(painter: &egui::Painter, pos: egui::Pos2, text: &str, fill: egui::Color32) -> f32 {
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(crate::theme::small_size()),
        egui::Color32::WHITE,
    );
    let rect = egui::Rect::from_min_size(pos, galley.size() + egui::vec2(8.0, 2.0));
    painter.rect_filled(rect, 4.0, fill);
    painter.galley(
        rect.min + egui::vec2(4.0, 1.0),
        galley,
        egui::Color32::WHITE,
    );
    rect.height()
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
        ui.strong(t("Continuă lectura"));
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
        ui.vertical_centered(|ui| ui.label(t("Nicio carte nu se potrivește căutării.")));
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
            .hint_text(t("Caută după titlu, autor, etichetă sau conținut…  (Ctrl+F)"))
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
        egui::ComboBox::from_label(t("Sortare")).id_salt("sort")
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
    let rev = app.session.cover_rev(&book.id);
    let tex = app.covers.get(ui.ctx(), app.files.as_ref(), book, rev);
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
            // White covers would melt into the white page without an edge.
            painter.rect_stroke(
                draw,
                0.0,
                visuals.widgets.noninteractive.bg_stroke,
                egui::StrokeKind::Inside,
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
    let mut y = cover_rect.top() + 4.0;
    let at = |y: f32| egui::pos2(cover_rect.left() + 4.0, y);
    if book.edited {
        y += 2.0 + badge(&painter, at(y), t("corectată"), crate::theme::edited());
    }
    if book.is_incomplete() {
        badge(&painter, at(y), t("incompletă"), crate::theme::incomplete());
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
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
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
