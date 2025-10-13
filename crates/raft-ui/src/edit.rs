//! Metadata correction form on the book page.
//!
//! The form only edits a local draft; "Salvează" sends the changed fields in one
//! `book.edit` command (one undo step). Corrections are stored in `raft.db`.

use std::collections::BTreeSet;

use raft_doc::Book;
use raft_doc::edits::{Field, field_value};
use serde_json::{Map, Value, json};

use crate::RaftUi;

/// The form's editable copy of a book's metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub book_id: String,
    pub title: String,
    pub authors: String,
    pub year: String,
    pub language: String,
    pub publisher: String,
    pub isbn: String,
    pub tags: String,
    pub description: String,
    pub categories: BTreeSet<String>,
    pub reviewed: bool,
}

impl Draft {
    pub fn from_book(b: &Book) -> Self {
        Self {
            book_id: b.id.clone(),
            title: b.title.clone(),
            authors: b.authors.join(", "),
            year: b.year.map(|y| y.to_string()).unwrap_or_default(),
            language: b.language.clone(),
            publisher: b.publisher.clone().unwrap_or_default(),
            isbn: b.isbn.clone().unwrap_or_default(),
            tags: b.tags.join(", "),
            description: b.description.clone(),
            categories: b.categories.iter().cloned().collect(),
            reviewed: b.reviewed,
        }
    }

    /// Copies one field from `other`, keeping the other unsaved changes.
    pub fn take(&mut self, other: &Draft, field: Field) {
        match field {
            Field::Title => self.title.clone_from(&other.title),
            Field::Authors => self.authors.clone_from(&other.authors),
            Field::Year => self.year.clone_from(&other.year),
            Field::Language => self.language.clone_from(&other.language),
            Field::Publisher => self.publisher.clone_from(&other.publisher),
            Field::Isbn => self.isbn.clone_from(&other.isbn),
            Field::Categories => self.categories.clone_from(&other.categories),
            Field::Tags => self.tags.clone_from(&other.tags),
            Field::Description => self.description.clone_from(&other.description),
            Field::Reviewed => self.reviewed = other.reviewed,
        }
    }

    /// The fields that differ from `book`, as `book.edit` params.
    pub fn changes(&self, book: &Book) -> Map<String, Value> {
        let current = Self::from_book(book);
        let mut out = Map::new();
        let mut text = |field: Field, mine: &str, theirs: &str| {
            if mine.trim() != theirs.trim() {
                out.insert(field.slug().to_owned(), json!(mine.trim()));
            }
        };
        text(Field::Title, &self.title, &current.title);
        text(Field::Authors, &self.authors, &current.authors);
        text(Field::Year, &self.year, &current.year);
        text(Field::Language, &self.language, &current.language);
        text(Field::Publisher, &self.publisher, &current.publisher);
        text(Field::Isbn, &self.isbn, &current.isbn);
        text(Field::Tags, &self.tags, &current.tags);
        text(Field::Description, &self.description, &current.description);
        if self.reviewed != current.reviewed {
            out.insert(Field::Reviewed.slug().to_owned(), json!(self.reviewed));
        }
        if self.categories != current.categories {
            // Keep the book's order for categories it already had (the first is the main one).
            let mut list: Vec<&String> = book
                .categories
                .iter()
                .filter(|c| self.categories.contains(*c))
                .collect();
            list.extend(
                self.categories
                    .iter()
                    .filter(|c| !book.categories.contains(c)),
            );
            out.insert(Field::Categories.slug().to_owned(), json!(list));
        }
        out
    }
}

pub fn label(field: Field) -> &'static str {
    match field {
        Field::Title => "Titlu",
        Field::Authors => "Autori",
        Field::Year => "An",
        Field::Language => "Limbă",
        Field::Publisher => "Editură",
        Field::Isbn => "ISBN",
        Field::Categories => "Categorii",
        Field::Tags => "Etichete",
        Field::Description => "Descriere",
        Field::Reviewed => "Verificată",
    }
}

/// Field label, with a marker and a per-field reset when the field is corrected.
fn field_label(
    app: &mut RaftUi,
    ui: &mut egui::Ui,
    book: &Book,
    field: Field,
    reset: &mut Option<Field>,
) -> egui::Response {
    let edited = app.session.edited_fields(&book.id).contains(&field);
    ui.horizontal(|ui| {
        let resp = ui.label(label(field));
        if book.lacks(field) {
            let text = egui::RichText::new("lipsește").small();
            if Field::REQUIRED.contains(&field) {
                ui.label(text.color(ui.visuals().error_fg_color));
            } else {
                ui.label(text.weak());
            }
        }
        if edited {
            let original = app
                .session
                .original(&book.id)
                .map(|b| field_value(b, field))
                .unwrap_or_default();
            ui.label(
                egui::RichText::new("corectat")
                    .small()
                    .color(ui.visuals().warn_fg_color),
            )
            .on_hover_text(format!("În catalog: {original}"));
            if ui
                .small_button("din catalog")
                .on_hover_text("Renunță la corectură și folosește valoarea din catalog")
                .clicked()
            {
                app.act(
                    "book.edit.reset",
                    json!({ "id": book.id, "field": field.slug() }),
                );
                *reset = Some(field);
            }
        }
        resp
    })
    .inner
}

/// The result after `id` in the library's current results, with `id`'s 1-based
/// position and the result count. Computed before saving, so the next book is
/// found even when the save takes the current one out of the results.
fn next_result(app: &RaftUi, id: &str) -> Option<(String, usize, usize)> {
    let mut results = app.session.results();
    let pos = results.position(|b| b.id == id)?;
    let next = results.next()?.id.clone();
    Some((next, pos + 1, app.session.result_count()))
}

/// Draws the form for the book of the `Edit` route.
pub fn form(app: &mut RaftUi, ui: &mut egui::Ui, book: &Book) {
    let mut draft = match app.edit.clone().filter(|d| d.book_id == book.id) {
        Some(d) => d,
        None => Draft::from_book(book),
    };
    ui.heading("Corectează metadatele");
    ui.label(
        egui::RichText::new(
            "Corecturile se salvează separat de catalog și rămân după build-catalog --refresh-ai.",
        )
        .small()
        .weak(),
    );
    ui.add_space(6.0);
    ui.checkbox(
        &mut draft.reviewed,
        "Verificată: ce lipsește nu se poate afla (nu mai apare la „date incomplete”)",
    );
    let changes = draft.changes(book);
    let next = next_result(app, &book.id);
    let mut close = false;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!changes.is_empty(), egui::Button::new("Salvează"))
            .clicked()
        {
            let id = book.id.clone();
            if app
                .run("book.edit", json!({ "id": id, "fields": changes }))
                .is_ok()
            {
                close = true;
            }
        }
        if let Some((next, pos, total)) = &next {
            let text = if changes.is_empty() {
                "Următoarea"
            } else {
                "Salvează și următoarea"
            };
            if ui
                .button(text)
                .on_hover_text(format!(
                    "Cartea {pos} din {total} din rezultatele bibliotecii"
                ))
                .clicked()
            {
                let saved = changes.is_empty()
                    || app
                        .run("book.edit", json!({ "id": book.id, "fields": changes }))
                        .is_ok();
                if saved {
                    app.act("view.edit", json!({ "id": next }));
                    close = true;
                }
            }
        }
        if ui.button("Renunță").clicked() {
            app.act("view.book", json!({ "id": book.id }));
            close = true;
        }
        if !app.session.edited_fields(&book.id).is_empty()
            && ui
                .button("Revino la catalog")
                .on_hover_text("Șterge toate corecturile acestei cărți")
                .clicked()
        {
            app.act("book.edit.reset", json!({ "id": book.id }));
            app.act("view.book", json!({ "id": book.id }));
            close = true;
        }
    });
    if let Some(status) = &app.session.status {
        ui.colored_label(ui.visuals().warn_fg_color, status);
    }
    ui.add_space(6.0);
    let mut reset = None;
    let width = ui.available_width().min(640.0);
    let single = |ui: &mut egui::Ui, text: &mut String, lbl: &egui::Response, hint: &str| {
        ui.add(
            egui::TextEdit::singleline(text)
                .hint_text(hint)
                .desired_width(width),
        )
        .labelled_by(lbl.id);
    };
    let l = field_label(app, ui, book, Field::Title, &mut reset);
    single(ui, &mut draft.title, &l, "titlul din carte, netradus");
    let l = field_label(app, ui, book, Field::Authors, &mut reset);
    single(ui, &mut draft.authors, &l, "separați prin virgulă");
    let l = field_label(app, ui, book, Field::Year, &mut reset);
    single(ui, &mut draft.year, &l, "gol = necunoscut");
    let l = field_label(app, ui, book, Field::Language, &mut reset);
    single(ui, &mut draft.language, &l, "ru, en, ro…");
    let l = field_label(app, ui, book, Field::Publisher, &mut reset);
    single(ui, &mut draft.publisher, &l, "");
    let l = field_label(app, ui, book, Field::Isbn, &mut reset);
    single(ui, &mut draft.isbn, &l, "");
    let l = field_label(app, ui, book, Field::Tags, &mut reset);
    single(ui, &mut draft.tags, &l, "separate prin virgulă");
    let l = field_label(app, ui, book, Field::Description, &mut reset);
    ui.add(
        egui::TextEdit::multiline(&mut draft.description)
            .desired_width(width)
            .desired_rows(4),
    )
    .labelled_by(l.id);
    field_label(app, ui, book, Field::Categories, &mut reset);
    let categories = app.session.catalog().categories.clone();
    egui::Grid::new("edit-categories")
        .num_columns(3)
        .show(ui, |ui| {
            for (i, c) in categories.iter().enumerate() {
                let mut on = draft.categories.contains(&c.slug);
                if ui.checkbox(&mut on, &c.name).changed() {
                    if on {
                        draft.categories.insert(c.slug.clone());
                    } else {
                        draft.categories.remove(&c.slug);
                    }
                }
                if i % 3 == 2 {
                    ui.end_row();
                }
            }
        });
    if let Some(field) = reset
        && let Some(fresh) = app.session.book(&book.id).map(Draft::from_book)
    {
        draft.take(&fresh, field);
    }
    app.edit = if close { None } else { Some(draft) };
}
