//! Library screen: two tabs, the library (filters panel, search bar, book grid) and
//! "continue reading" (the started books, most recently read first).

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
        let others = |n: usize, checked: bool| checked.then(|| tf("Alte limbi ({0})", &[&n]));
        checked_above(
            ui,
            &facets.languages,
            &filters.languages,
            usize::MAX,
            ("other-languages-list", true, others),
            |ui, lang, n, mut on| {
                if ui
                    .checkbox(&mut on, format!("{}  ({n})", language_name(lang)))
                    .changed()
                {
                    app.act("filter.language.toggle", json!({ "language": lang }));
                }
            },
        );
        ui.add_space(6.0);

        ui.strong(t("Format"));
        let others = |n: usize, checked: bool| checked.then(|| tf("Alte formate ({0})", &[&n]));
        checked_above(
            ui,
            &facets.formats,
            &filters.formats,
            usize::MAX,
            ("other-formats", true, others),
            |ui, format, n, mut on| {
                let label = format!("{}  ({n})", format.slug().to_uppercase());
                if ui.checkbox(&mut on, label).changed() {
                    app.act("filter.format.toggle", json!({ "format": format.slug() }));
                }
            },
        );
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
            .on_hover_text(t(
                "Ignoră categoriile secundare (de ex. „career”, atribuită prea des)",
            ))
            .changed()
        {
            app.act("filter.primary_only", json!({ "on": primary }));
        }
        categories(app, ui, &facets, &filters);
        folders(app, ui, &facets, &filters);
    });
}

/// The categories: the checked ones in sight under the heading, the others below them,
/// in a list of their own once something is checked ("Alte categorii (N)", open at first,
/// closed at will).
fn categories(
    app: &mut RaftUi,
    ui: &mut egui::Ui,
    facets: &raft_engine::Facets,
    filters: &raft_doc::Filters,
) {
    let others = |n: usize, checked: bool| checked.then(|| tf("Alte categorii ({0})", &[&n]));
    checked_above(
        ui,
        &facets.categories,
        &filters.categories,
        TOP_CATEGORIES,
        ("other-categories", true, others),
        |ui, slug, n, mut on| {
            let name = crate::i18n::t_dyn(app.session.catalog().category_name(slug));
            if ui.checkbox(&mut on, format!("{name}  ({n})")).changed() {
                app.act("filter.category.toggle", json!({ "slug": slug }));
            }
        },
    );
}

/// A facet section: the checked entries in sight (see `selected_first`), the others below
/// them in a collapsible list titled `title(count, anything checked)`, or as a plain list
/// when that gives `None`. `list` = (egui id of the collapsible list, open at first, title).
fn checked_above<T: Clone + Ord>(
    ui: &mut egui::Ui,
    items: &[(T, usize)],
    selected: &std::collections::BTreeSet<T>,
    top: usize,
    list: (&str, bool, impl Fn(usize, bool) -> Option<String>),
    mut row: impl FnMut(&mut egui::Ui, &T, usize, bool),
) {
    let (id, open, title) = list;
    let all = selected_first(items, selected, top);
    let (checked, rest) = all.split_at(selected.len().min(all.len()));
    for (k, n) in checked {
        row(ui, k, *n, true);
    }
    if rest.is_empty() {
        return;
    }
    match title(rest.len(), !checked.is_empty()) {
        Some(title) => {
            egui::CollapsingHeader::new(title)
                .id_salt(id)
                .default_open(open)
                .show(ui, |ui| {
                    for (k, n) in rest {
                        row(ui, k, *n, false);
                    }
                });
        }
        _ => {
            for (k, n) in rest {
                row(ui, k, *n, false);
            }
        }
    }
}

/// A facet list with the checked entries first (in the facet's order), so they need no
/// searching for: then up to `top` of the others. A checked entry the facet no longer
/// counts is listed with 0, so it can still be unchecked.
fn selected_first<T: Clone + Ord>(
    items: &[(T, usize)],
    selected: &std::collections::BTreeSet<T>,
    top: usize,
) -> Vec<(T, usize)> {
    let mut out: Vec<(T, usize)> = items
        .iter()
        .filter(|(k, _)| selected.contains(k))
        .cloned()
        .collect();
    for s in selected {
        if !items.iter().any(|(k, _)| k == s) {
            out.push((s.clone(), 0));
        }
    }
    out.extend(
        items
            .iter()
            .filter(|(k, _)| !selected.contains(k))
            .take(top)
            .cloned(),
    );
    out
}

/// The "Folders" section: the subfolders of the library folders (one per author or
/// series, say), with their book counts; shown only when there are some.
fn folders(
    app: &mut RaftUi,
    ui: &mut egui::Ui,
    facets: &raft_engine::Facets,
    filters: &raft_doc::Filters,
) {
    if facets.folders.is_empty() && filters.folders.is_empty() {
        return;
    }
    ui.add_space(6.0);
    // The checked folders stay in sight, above the list they were picked from (which can
    // hold a hundred series, so it starts closed); a checked folder the facet no longer
    // counts shows with 0.
    if !filters.folders.is_empty() {
        ui.strong(t("Foldere"));
    }
    let title = |n: usize, checked: bool| {
        Some(if checked {
            tf("Alte foldere ({0})", &[&n])
        } else {
            tf("Foldere ({0})", &[&n])
        })
    };
    checked_above(
        ui,
        &facets.folders,
        &filters.folders,
        usize::MAX,
        ("folders", false, title),
        |ui, dir, n, mut on| {
            let label = format!("{}  ({n})", app.session.folder_label(dir));
            if ui
                .checkbox(&mut on, label)
                .on_hover_text(dir.as_str())
                .changed()
            {
                app.act("filter.folder.toggle", json!({ "path": dir }));
            }
        },
    );
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
        Mark::Duplicate => t("Dubluri posibile"),
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
            Mark::Incomplete => t(
                "Fără titlu, autori, an, limbă, categorii sau descriere și nemarcate ca verificate",
            ),
            Mark::Reviewed => t("Marcate de tine ca verificate"),
            Mark::Duplicate => t(
                "Cărți cu același ISBN, sau cu același titlu, an și volum, ca alta din bibliotecă",
            ),
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
    egui::CollapsingHeader::new(t("Lipsește câmpul…"))
        .id_salt("missing-fields")
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

/// The tab strip above the library screen.
pub fn tabs(app: &mut RaftUi, ui: &mut egui::Ui) {
    let current = app.session.library_tab();
    let reading = app.session.continue_reading(usize::MAX).len();
    ui.horizontal(|ui| {
        let tabs = [
            ("library", "view.tab.library", t("Bibliotecă").to_owned()),
            (
                "reading",
                "view.tab.reading",
                tf("Continuă lectura ({0})", &[&reading]),
            ),
        ];
        for (tab, command, label) in tabs {
            if ui.selectable_label(tab == current, label).clicked() && tab != current {
                app.act(command, json!({}));
            }
        }
    });
}

pub fn library(app: &mut RaftUi, ui: &mut egui::Ui) {
    search_bar(app, ui);
    ui.add_space(8.0);
    let results: Vec<Book> = app.session.results().cloned().collect();
    if !app.session.selected().is_empty() {
        selection_bar(app, ui, results.len());
    }
    if results.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| ui.label(t("Nicio carte nu se potrivește căutării.")));
        return;
    }
    grid(app, ui, "grid", &results);
}

/// The "continue reading" tab: every started book, most recently read first.
pub fn reading(app: &mut RaftUi, ui: &mut egui::Ui) {
    let books: Vec<Book> = app
        .session
        .continue_reading(usize::MAX)
        .into_iter()
        .cloned()
        .collect();
    ui.add_space(8.0);
    if books.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(t(
                "Nu ai început încă nicio carte. Cărțile deschise apar aici.",
            ))
        });
        return;
    }
    grid(app, ui, "reading-grid", &books);
}

/// Book cards in rows as wide as the panel; only the visible rows are drawn.
fn grid(app: &mut RaftUi, ui: &mut egui::Ui, id: &str, books: &[Book]) {
    let cols = (((ui.available_width() + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
    let rows = books.len().div_ceil(cols);
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .show_rows(ui, CARD_H + GAP, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for book in books.iter().skip(row * cols).take(cols) {
                        card(app, ui, book);
                    }
                });
                ui.add_space(GAP);
            }
        });
}

/// Shown while books are ticked: how many, select all / none, move to the trash.
fn selection_bar(app: &mut RaftUi, ui: &mut egui::Ui, results: usize) {
    let n = app.session.selected().len();
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(tf("{0} selectate", &[&n]));
                ui.separator();
                if ui
                    .button(tf("Selectează toate ({0})", &[&results]))
                    .clicked()
                {
                    app.act("select.all", json!({}));
                }
                if ui.button(t("Deselectează")).on_hover_text("Esc").clicked() {
                    app.act("select.clear", json!({}));
                }
                ui.separator();
                let delete = egui::Button::new(
                    egui::RichText::new(tf("Mută în coș ({0})", &[&n]))
                        .color(ui.visuals().error_fg_color),
                );
                if ui.add(delete).clicked() {
                    app.confirm_bulk_delete = true;
                }
            });
        });
    ui.add_space(6.0);
}

fn search_bar(app: &mut RaftUi, ui: &mut egui::Ui) {
    // Right to left: the sort list takes the width its label needs, the field the rest.
    let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
    let layout = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(row, layout, |ui| {
        let current = app.session.query().sort;
        egui::ComboBox::from_id_salt("sort")
            .selected_text(sort_label(current))
            .show_ui(ui, |ui| {
                for s in Sort::ALL {
                    if ui.selectable_label(s == current, sort_label(s)).clicked() {
                        app.act("sort.set", json!({ "sort": sort_slug(s) }));
                    }
                }
            })
            .response
            .on_hover_text(t("Sortare"));
        let edit = egui::TextEdit::singleline(&mut app.search)
            .hint_text(t(
                "Caută după titlu, autor, etichetă sau conținut…  (Ctrl+F)",
            ))
            // Room on the right for the clear button.
            .margin(egui::Margin {
                left: 4,
                right: CLEAR_D as i8 + 8,
                top: 2,
                bottom: 2,
            })
            .desired_width(ui.available_width().max(120.0));
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
        // The "×" resets what the grid shows: the text first, else the filters (a folder or
        // a category chosen from a book's page leaves the bar empty but the list narrowed).
        let filtered = !app.session.query().filters.is_empty();
        if !app.search.is_empty() {
            if clear_button(ui, resp.rect, t("Șterge căutarea")).clicked() {
                app.search.clear();
                app.act("search.set", json!({ "text": "" }));
            }
        } else if filtered && clear_button(ui, resp.rect, t("Șterge filtrele")).clicked() {
            app.act("filter.clear", json!({}));
        }
    });
}

/// Diameter of the clear-search button.
const CLEAR_D: f32 = 18.0;

/// A round "×" button drawn inside the right end of the search field `field`.
fn clear_button(ui: &mut egui::Ui, field: egui::Rect, label: &str) -> egui::Response {
    let center = egui::pos2(field.right() - 6.0 - CLEAR_D / 2.0, field.center().y);
    let rect = egui::Rect::from_center_size(center, egui::vec2(CLEAR_D, CLEAR_D));
    let resp = ui
        .interact(
            rect,
            ui.scope_id().with("clear_search"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let resp = resp.on_hover_text(label);
    let visuals = ui.visuals();
    let color = if resp.hovered() {
        visuals.strong_text_color()
    } else {
        visuals.weak_text_color()
    };
    let painter = ui.painter();
    let r = CLEAR_D / 2.0 - 1.0;
    if resp.hovered() {
        painter.circle_filled(center, r, visuals.widgets.hovered.weak_bg_fill);
    }
    painter.circle_stroke(center, r, egui::Stroke::new(1.2, color));
    let a = r * 0.42;
    let stroke = egui::Stroke::new(1.5, color);
    painter.line_segment(
        [center + egui::vec2(-a, -a), center + egui::vec2(a, a)],
        stroke,
    );
    painter.line_segment(
        [center + egui::vec2(-a, a), center + egui::vec2(a, -a)],
        stroke,
    );
    resp
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
    let outside = app.session.is_outside(&book.id);
    if outside {
        y += 2.0 + badge(&painter, at(y), t("fișier"), crate::theme::color("accent"));
    }
    if book.edited {
        y += 2.0 + badge(&painter, at(y), t("corectată"), crate::theme::edited());
    }
    if book.is_incomplete() && !outside {
        y += 2.0 + badge(&painter, at(y), t("incompletă"), crate::theme::incomplete());
    }
    if !book.duplicates.is_empty() {
        badge(&painter, at(y), t("dublură?"), crate::theme::duplicate());
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
    // The tick box, top right: on hover, and on every card once one is ticked.
    let selected = app.session.is_selected(&book.id);
    let selecting = !app.session.selected().is_empty();
    let tick_rect = egui::Rect::from_min_size(
        egui::pos2(cover_rect.right() - 30.0, cover_rect.top() + 6.0),
        egui::vec2(24.0, 24.0),
    );
    // A file from outside the library cannot be ticked (nothing to delete in bulk).
    let tick = ui.interact(
        tick_rect,
        ui.scope_id().with(("tick", &book.id)),
        if outside {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        },
    );
    let label = tf("Selectează: {0}", &[&book.display_title()]);
    tick.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, selected, &label));
    if !outside && (selected || selecting || resp.hovered() || tick.hovered()) {
        let fill = if selected {
            visuals.selection.bg_fill
        } else {
            visuals.extreme_bg_color.gamma_multiply(0.9)
        };
        painter.rect_filled(tick_rect, 5.0, fill);
        painter.rect_stroke(
            tick_rect,
            5.0,
            egui::Stroke::new(1.5, visuals.strong_text_color()),
            egui::StrokeKind::Inside,
        );
        if selected {
            painter.text(
                tick_rect.center(),
                egui::Align2::CENTER_CENTER,
                "✔",
                egui::FontId::proportional(16.0),
                visuals.selection.stroke.color,
            );
        }
        if selected {
            painter.rect_stroke(
                cover_rect,
                6.0,
                egui::Stroke::new(3.0, visuals.selection.bg_fill),
                egui::StrokeKind::Inside,
            );
        }
    }
    let resp = resp.on_hover_text(book.display_title());
    if !outside && (tick.clicked() || (selecting && resp.clicked())) {
        // While books are ticked, a click on a card ticks it too.
        app.act("select.toggle", json!({ "id": book.id }));
    } else if resp.clicked() {
        app.act("view.book", json!({ "id": book.id }));
    }
}

#[cfg(test)]
mod tests {
    use super::selected_first;

    #[test]
    fn checked_entries_come_first_and_never_vanish() {
        let items: Vec<(String, usize)> =
            [("python", 9), ("fiction", 7), ("diy-home", 3), ("odoo", 1)]
                .iter()
                .map(|(k, n)| ((*k).to_owned(), *n))
                .collect();
        let sel = ["odoo", "diy-home", "gone"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let list = selected_first(&items, &sel, 1);
        let got: Vec<(&str, usize)> = list.iter().map(|(k, n)| (k.as_str(), *n)).collect();
        assert_eq!(
            got,
            [("diy-home", 3), ("odoo", 1), ("gone", 0), ("python", 9)]
        );
        let none = selected_first(&items, &Default::default(), 2);
        assert_eq!(none.len(), 2);
    }
}
