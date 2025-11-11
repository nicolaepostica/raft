//! The command registry: every user-visible action, with its menu, shortcut and params.

use raft_doc::edits::{self, Field};
use raft_doc::{Format, Mark, Progress, Route, Sort};
use serde_json::{Value, json};

use crate::params::{opt_bool, opt_i64, opt_str, str};
use crate::{CoverSource, EngineError, Session};

/// Keyboard shortcut; `key` is an egui key name (`"Z"`, `"ArrowRight"`, `"PageDown"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub key: &'static str,
    /// Ctrl on Linux/Windows, Cmd on macOS.
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}

const fn key(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        command: false,
        shift: false,
        alt: false,
    })
}

const fn ctrl(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        command: true,
        shift: false,
        alt: false,
    })
}

const fn ctrl_shift(key: &'static str) -> Option<Shortcut> {
    Some(Shortcut {
        key,
        command: true,
        shift: true,
        alt: false,
    })
}

type Enabled = fn(&Session) -> Result<(), String>;
type Run = fn(&mut Session, &Value) -> Result<Value, EngineError>;

/// One registered command.
#[derive(Clone, Copy)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu path, `"View/Library"`; `None` = not in a menu.
    pub menu: Option<&'static str>,
    pub shortcut: Option<Shortcut>,
    /// More shortcuts for the same command (not shown in menus).
    pub also: &'static [Shortcut],
    /// Human-readable description of the JSON params.
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
    /// Whether the change goes into undo history.
    pub record: bool,
    /// Consecutive runs merge into one undo step (page turns).
    pub coalesce: bool,
}

impl std::fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandSpec").field("id", &self.id).finish()
    }
}

fn always(_: &Session) -> Result<(), String> {
    Ok(())
}

fn has_undo(s: &Session) -> Result<(), String> {
    if s.can_undo() {
        Ok(())
    } else {
        Err("nothing to undo".into())
    }
}

fn has_redo(s: &Session) -> Result<(), String> {
    if s.can_redo() {
        Ok(())
    } else {
        Err("nothing to redo".into())
    }
}

fn in_reader(s: &Session) -> Result<(), String> {
    match s.route() {
        Route::Reader { .. } => Ok(()),
        _ => Err("no book is open in the reader".into()),
    }
}

fn in_paged_reader(s: &Session) -> Result<(), String> {
    in_reader(s)?;
    match s.current_book() {
        Some(b) if b.format.is_paged() => Ok(()),
        _ => Err("the open book has no pages; it scrolls".into()),
    }
}

fn in_text_reader(s: &Session) -> Result<(), String> {
    in_reader(s)?;
    match s.current_book() {
        Some(b) if b.format.is_text() => Ok(()),
        _ => Err("the open book is not a text book".into()),
    }
}

fn not_library(s: &Session) -> Result<(), String> {
    match s.route() {
        Route::Library => Err("already in the library".into()),
        _ => Ok(()),
    }
}

fn has_filters(s: &Session) -> Result<(), String> {
    let q = s.query();
    if q.filters.is_empty() && q.text.is_empty() {
        Err("no search or filters set".into())
    } else {
        Ok(())
    }
}

const fn spec(id: &'static str, label: &'static str, run: Run) -> CommandSpec {
    CommandSpec {
        id,
        label,
        menu: None,
        shortcut: None,
        also: &[],
        params: "{}",
        enabled: always,
        run,
        record: true,
        coalesce: false,
    }
}

static REGISTRY: &[CommandSpec] = &[
    CommandSpec {
        menu: Some("Edit/Undo"),
        shortcut: ctrl("Z"),
        enabled: has_undo,
        record: false,
        ..spec("edit.undo", "Anulează", |s, _| {
            s.undo().map(|()| Value::Null)
        })
    },
    CommandSpec {
        menu: Some("Edit/Redo"),
        shortcut: ctrl_shift("Z"),
        enabled: has_redo,
        record: false,
        ..spec("edit.redo", "Refă", |s, _| s.redo().map(|()| Value::Null))
    },
    CommandSpec {
        params: r#"{"text": string}"#,
        coalesce: true,
        ..spec("search.set", "Caută", search_set)
    },
    CommandSpec {
        menu: Some("View/Clear search and filters"),
        shortcut: ctrl_shift("X"),
        enabled: has_filters,
        ..spec("filter.clear", "Șterge căutarea și filtrele", filter_clear)
    },
    CommandSpec {
        params: r#"{"slug": string}"#,
        ..spec(
            "filter.category.toggle",
            "Filtru: categorie",
            toggle_category,
        )
    },
    CommandSpec {
        params: r#"{"on": bool}"#,
        ..spec(
            "filter.primary_only",
            "Doar categoria principală",
            primary_only,
        )
    },
    CommandSpec {
        params: r#"{"language": string}"#,
        ..spec("filter.language.toggle", "Filtru: limbă", toggle_language)
    },
    CommandSpec {
        params: r#"{"format": "pdf"|"epub"|"djvu"|"fb2"|"other"}"#,
        ..spec("filter.format.toggle", "Filtru: format", toggle_format)
    },
    CommandSpec {
        params: r#"{"mark": "edited"|"incomplete"|"reviewed"}"#,
        ..spec("filter.mark.toggle", "Filtru: marcaj", toggle_mark)
    },
    CommandSpec {
        params: r#"{"field": "title"|"authors"|"year"|"language"|"publisher"|"isbn"|"categories"|"tags"|"description"}"#,
        ..spec(
            "filter.missing.toggle",
            "Filtru: câmp lipsă",
            toggle_missing,
        )
    },
    CommandSpec {
        params: r#"{"min": int|null, "max": int|null}"#,
        coalesce: true,
        ..spec("filter.years", "Filtru: ani", filter_years)
    },
    CommandSpec {
        params: r#"{"sort": "relevance"|"title"|"year_desc"|"year_asc"|"recently_added"}"#,
        ..spec("sort.set", "Sortare", sort_set)
    },
    CommandSpec {
        menu: Some("View/Theme"),
        shortcut: ctrl_shift("T"),
        params: r#"{"theme": "light"|"dark"} (default: switch)"#,
        record: false,
        ..spec("view.theme", "Temă albă / întunecată", view_theme)
    },
    CommandSpec {
        params: r#"{"language": "ro"|"en"|"ru"}"#,
        record: false,
        ..spec("view.language", "Schimbă limba interfeței", view_language)
    },
    CommandSpec {
        menu: Some("File/Add library folder"),
        params: r#"{"path": string} (a folder with catalog.db, made by build-catalog DIR)"#,
        record: false,
        ..spec("source.add", "Adaugă un folder cu cărți", source_add)
    },
    CommandSpec {
        params: r#"{"path": string} (the folder's files are not touched)"#,
        record: false,
        ..spec("source.remove", "Scoate un folder din bibliotecă", source_remove)
    },
    CommandSpec {
        params: r#"{"path": string}"#,
        ..spec("filter.source.toggle", "Filtru: sursă", toggle_source)
    },
    CommandSpec {
        menu: Some("View/Back"),
        shortcut: key("Backspace"),
        also: &[
            Shortcut {
                key: "ArrowLeft",
                command: false,
                shift: false,
                alt: true,
            },
            Shortcut {
                key: "Escape",
                command: false,
                shift: false,
                alt: false,
            },
        ],
        enabled: not_library,
        ..spec("view.library", "Înapoi", view_library)
    },
    CommandSpec {
        params: r#"{"id": string}"#,
        ..spec("view.book", "Pagina cărții", view_book)
    },
    CommandSpec {
        params: r#"{"id": string?} (default: current book)"#,
        ..spec("book.read", "Citește", book_read)
    },
    CommandSpec {
        menu: Some("File/Open in external app"),
        shortcut: ctrl("O"),
        params: r#"{"id": string?} (default: current book)"#,
        enabled: |s| {
            s.current_book()
                .map(|_| ())
                .ok_or_else(|| "no book selected".into())
        },
        record: false,
        ..spec(
            "book.open_external",
            "Deschide în aplicația externă",
            open_external,
        )
    },
    CommandSpec {
        params: r#"{"id": string?} (default: current book)"#,
        record: false,
        ..spec("book.show_in_folder", "Arată în dosar", show_in_folder)
    },
    CommandSpec {
        params: r#"{"id": string?, "confirm": true} — moves the file to the trash; not undoable"#,
        record: false,
        ..spec("book.delete", "Șterge cartea", book_delete)
    },
    CommandSpec {
        params: r#"{"id": string?, "path": string} | {"id": string?, "page": int} | {"id": string?, "catalog": true} — one source"#,
        record: false,
        ..spec("book.cover.set", "Schimbă coperta", cover_set)
    },
    CommandSpec {
        params: r#"{"page": int}"#,
        enabled: in_paged_reader,
        coalesce: true,
        ..spec("reader.goto", "Mergi la pagina", reader_goto)
    },
    CommandSpec {
        shortcut: key("ArrowRight"),
        enabled: in_paged_reader,
        coalesce: true,
        ..spec("reader.next", "Pagina următoare", |s, _| reader_step(s, 1))
    },
    CommandSpec {
        shortcut: key("ArrowLeft"),
        enabled: in_paged_reader,
        coalesce: true,
        ..spec("reader.prev", "Pagina anterioară", |s, _| {
            reader_step(s, -1)
        })
    },
    CommandSpec {
        params: r#"{"block": int, "total": int} (index of the first visible block)"#,
        enabled: in_text_reader,
        coalesce: true,
        ..spec("reader.position", "Poziția în carte", reader_position)
    },
    CommandSpec {
        params: r#"{"id": string?} (default: current book)"#,
        ..spec("progress.reset", "Șterge progresul", progress_reset)
    },
    CommandSpec {
        params: r#"{"id": string?} (default: current book)"#,
        ..spec("view.edit", "Corectează metadatele", view_edit)
    },
    CommandSpec {
        params: r#"{"id": string?, "fields": {"title"|"authors"|"year"|"language"|"publisher"|"isbn"|"categories"|"tags"|"description": value}} — lists as arrays or comma-separated text, year as int, null or ""#,
        ..spec("book.edit", "Corectează metadatele", book_edit)
    },
    CommandSpec {
        params: r#"{"id": string?, "on": bool?} (default: toggle) — missing fields are known to be unknowable"#,
        ..spec("book.review", "Marchează ca verificată", book_review)
    },
    CommandSpec {
        params: r#"{"id": string?, "field": string?} (no field: every correction of the book, the reviewed mark stays)"#,
        ..spec(
            "book.edit.reset",
            "Revino la datele din catalog",
            book_edit_reset,
        )
    },
];

/// All registered commands.
pub fn registry() -> &'static [CommandSpec] {
    REGISTRY
}

pub(crate) fn find(id: &str) -> Option<&'static CommandSpec> {
    REGISTRY.iter().find(|c| c.id == id)
}

fn search_set(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let text = str(p, "text")?;
    s.state.query.text = text.chars().take(raft_search::MAX_QUERY_CHARS).collect();
    if s.state.route != Route::Library {
        s.state.route = Route::Library;
    }
    s.refresh();
    Ok(json!({ "results": s.results.len() }))
}

fn filter_clear(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    let sort = s.state.query.sort;
    s.state.query = raft_doc::Query {
        sort,
        ..raft_doc::Query::default()
    };
    Ok(Value::Null)
}

fn toggle<T: Ord>(set: &mut std::collections::BTreeSet<T>, v: T) {
    if !set.remove(&v) {
        set.insert(v);
    }
}

fn toggle_category(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let slug = str(p, "slug")?;
    if !s.catalog.categories.iter().any(|c| c.slug == slug)
        && !s.state.query.filters.categories.contains(slug)
    {
        return Err(EngineError::NotFound(format!("category `{slug}`")));
    }
    toggle(&mut s.state.query.filters.categories, slug.to_owned());
    Ok(Value::Null)
}

fn primary_only(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let on = opt_bool(p, "on")?.unwrap_or(!s.state.query.filters.primary_category_only);
    s.state.query.filters.primary_category_only = on;
    Ok(Value::Null)
}

fn toggle_language(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let lang = str(p, "language")?;
    if lang.len() > 16 {
        return Err(EngineError::BadParams("`language` is too long".into()));
    }
    toggle(&mut s.state.query.filters.languages, lang.to_owned());
    Ok(Value::Null)
}

fn toggle_format(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let slug = str(p, "format")?;
    let format = Format::ALL
        .into_iter()
        .find(|f| f.slug() == slug)
        .ok_or_else(|| EngineError::BadParams(format!("unknown format `{slug}`")))?;
    toggle(&mut s.state.query.filters.formats, format);
    Ok(Value::Null)
}

fn toggle_mark(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let slug = str(p, "mark")?;
    let mark = Mark::from_slug(slug)
        .ok_or_else(|| EngineError::BadParams(format!("unknown mark `{slug}`")))?;
    toggle(&mut s.state.query.filters.marks, mark);
    Ok(Value::Null)
}

fn toggle_missing(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let slug = str(p, "field")?;
    let field = Field::from_slug(slug)
        .filter(|f| *f != Field::Reviewed)
        .ok_or_else(|| EngineError::BadParams(format!("unknown field `{slug}`")))?;
    toggle(&mut s.state.query.filters.missing, field);
    Ok(Value::Null)
}

fn year(p: &Value, key: &str) -> Result<Option<i32>, EngineError> {
    opt_i64(p, key)?
        .map(|y| {
            i32::try_from(y)
                .ok()
                .filter(|y| (-5000..=10000).contains(y))
                .ok_or_else(|| EngineError::BadParams(format!("`{key}` is out of range")))
        })
        .transpose()
}

fn filter_years(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let (min, max) = (year(p, "min")?, year(p, "max")?);
    if let (Some(a), Some(b)) = (min, max)
        && a > b
    {
        return Err(EngineError::BadParams("`min` is greater than `max`".into()));
    }
    s.state.query.filters.year_min = min;
    s.state.query.filters.year_max = max;
    Ok(Value::Null)
}

fn sort_set(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let v = p
        .get("sort")
        .cloned()
        .ok_or_else(|| EngineError::BadParams("missing `sort`".into()))?;
    let sort: Sort =
        serde_json::from_value(v).map_err(|e| EngineError::BadParams(format!("`sort`: {e}")))?;
    s.state.query.sort = sort;
    Ok(Value::Null)
}

/// Longest accepted folder path, in bytes.
const MAX_PATH: usize = 4096;

fn folder_param(p: &Value) -> Result<String, EngineError> {
    let path = str(p, "path")?.trim().trim_end_matches('/');
    if path.is_empty() || path.len() > MAX_PATH || path.chars().any(char::is_control) {
        return Err(bad("`path` must be a folder path"));
    }
    Ok(path.to_owned())
}

fn source_add(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let root = folder_param(p)?;
    if s.sources.contains(&root) {
        return Err(bad(format!("`{root}` is already in the library")));
    }
    let catalog = s.backend.open_source(&root).map_err(EngineError::Backend)?;
    let before = s.catalog.books.len();
    s.add_source(root.clone(), catalog);
    let added = s.catalog.books.len() - before;
    Ok(json!({ "source": root, "books": added }))
}

fn source_remove(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let root = folder_param(p)?;
    if !s.sources.contains(&root) {
        return Err(EngineError::NotFound(format!("source `{root}`")));
    }
    s.backend.close_source(&root).map_err(EngineError::Backend)?;
    if s.current_book().is_some_and(|b| b.source == root) {
        s.state.route = Route::Library;
    }
    s.drop_source(&root);
    Ok(Value::Null)
}

fn toggle_source(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let root = folder_param(p)?;
    if !s.sources.contains(&root) && !s.state.query.filters.sources.contains(&root) {
        return Err(EngineError::NotFound(format!("source `{root}`")));
    }
    toggle(&mut s.state.query.filters.sources, root);
    Ok(Value::Null)
}

/// Interface languages the UI has translations for.
pub const LANGUAGES: [&str; 3] = ["ro", "en", "ru"];

fn view_language(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let code = str(p, "language")?;
    if !LANGUAGES.contains(&code) {
        return Err(bad(format!(
            "unknown interface language `{code}` (ro, en or ru)"
        )));
    }
    s.backend
        .save_setting("language", code)
        .map_err(|e| EngineError::Backend(format!("language not saved: {e}")))?;
    s.language = code.to_owned();
    Ok(json!({ "language": code }))
}

fn view_theme(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let dark = match opt_str(p, "theme")? {
        None => !s.dark,
        Some("dark") => true,
        Some("light") => false,
        Some(other) => return Err(bad(format!("unknown theme `{other}` (light or dark)"))),
    };
    let name = if dark { "dark" } else { "light" };
    s.backend
        .save_setting("theme", name)
        .map_err(|e| EngineError::Backend(format!("theme not saved: {e}")))?;
    s.dark = dark;
    Ok(json!({ "theme": name }))
}

fn view_library(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    s.state.route = match &s.state.route {
        Route::Reader { id } | Route::Edit { id } => Route::Book { id: id.clone() },
        _ => Route::Library,
    };
    Ok(Value::Null)
}

fn book_id(s: &Session, p: &Value) -> Result<String, EngineError> {
    let id = match opt_str(p, "id")? {
        Some(id) => id.to_owned(),
        None => s
            .current_book()
            .map(|b| b.id.clone())
            .ok_or_else(|| EngineError::BadParams("missing `id`".into()))?,
    };
    if s.catalog.book(&id).is_none() {
        return Err(EngineError::NotFound(format!("book `{id}`")));
    }
    Ok(id)
}

fn view_book(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, &json!({ "id": str(p, "id")? }))?;
    s.state.route = Route::Book { id };
    Ok(Value::Null)
}

fn book_read(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    let book = s
        .catalog
        .book(&id)
        .ok_or_else(|| EngineError::NotFound(format!("book `{id}`")))?;
    if book.format.is_paged() {
        let page = s.state.progress.get(&id).and_then(|p| p.page).unwrap_or(1);
        s.state.route = Route::Reader { id: id.clone() };
        let page = set_page(s, &id, i64::from(page))?;
        Ok(json!({ "reader": "paged", "page": page }))
    } else if book.format.is_text() {
        s.state.route = Route::Reader { id: id.clone() };
        let block = s
            .state
            .progress
            .get(&id)
            .and_then(Progress::block)
            .unwrap_or(0);
        Ok(json!({ "reader": "text", "block": block }))
    } else {
        s.backend
            .open_external(book)
            .map_err(EngineError::Backend)?;
        let now = s.backend.now();
        s.state
            .progress
            .entry(id.clone())
            .and_modify(|p| p.updated_at = now)
            .or_insert(Progress {
                book_id: id,
                page: None,
                position: None,
                percent: 0.0,
                updated_at: now,
            });
        Ok(json!({ "reader": "external" }))
    }
}

fn open_external(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    let book = s
        .catalog
        .book(&id)
        .ok_or_else(|| EngineError::NotFound(format!("book `{id}`")))?;
    s.backend
        .open_external(book)
        .map_err(EngineError::Backend)?;
    Ok(Value::Null)
}

fn reader_book(s: &Session) -> Result<String, EngineError> {
    match &s.state.route {
        Route::Reader { id } => Ok(id.clone()),
        _ => Err(EngineError::Disabled(
            "no book is open in the reader".into(),
        )),
    }
}

/// Moves the reader to `page` (clamped to the book) and records progress.
fn set_page(s: &mut Session, id: &str, page: i64) -> Result<u32, EngineError> {
    let pages = s.catalog.book(id).and_then(|b| b.pages).unwrap_or(0);
    let max = i64::from(pages.max(1));
    let page = u32::try_from(page.clamp(1, max)).unwrap_or(1);
    let percent = if pages > 0 {
        (f64::from(page) / f64::from(pages) * 100.0) as f32
    } else {
        0.0
    };
    let now = s.backend.now();
    s.state.progress.insert(
        id.to_owned(),
        Progress {
            book_id: id.to_owned(),
            page: Some(page),
            position: None,
            percent,
            updated_at: now,
        },
    );
    Ok(page)
}

fn reader_goto(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = reader_book(s)?;
    let page =
        opt_i64(p, "page")?.ok_or_else(|| EngineError::BadParams("missing `page`".into()))?;
    let page = set_page(s, &id, page)?;
    Ok(json!({ "page": page }))
}

fn reader_step(s: &mut Session, delta: i64) -> Result<Value, EngineError> {
    let id = reader_book(s)?;
    let cur = s.state.progress.get(&id).and_then(|p| p.page).unwrap_or(1);
    let page = set_page(s, &id, i64::from(cur) + delta)?;
    Ok(json!({ "page": page }))
}

fn progress_reset(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    if s.state.progress.remove(&id).is_none() {
        return Err(EngineError::NotFound(format!("no progress for `{id}`")));
    }
    Ok(Value::Null)
}

/// Longest accepted text field (title, publisher …), in bytes.
const MAX_SHORT: usize = 1024;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams(msg.into())
}

/// Checks a value for `field` and returns its canonical form.
fn canonical(s: &Session, field: Field, v: &Value) -> Result<String, EngineError> {
    let name = field.slug();
    let text = |max: usize| -> Result<String, EngineError> {
        let t = match v {
            Value::String(t) => t.trim().to_owned(),
            Value::Null => String::new(),
            _ => return Err(bad(format!("`{name}` must be text"))),
        };
        if t.len() > max {
            return Err(bad(format!("`{name}` is too long (max {max} bytes)")));
        }
        if t.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
            return Err(bad(format!("`{name}` contains control characters")));
        }
        Ok(t)
    };
    let list = || -> Result<Vec<String>, EngineError> {
        let items: Vec<String> = match v {
            Value::Array(items) => items
                .iter()
                .map(|i| i.as_str().map(|t| t.trim().to_owned()))
                .collect::<Option<_>>()
                .ok_or_else(|| bad(format!("`{name}` must be a list of text")))?,
            Value::String(t) => t.split([',', ';']).map(|p| p.trim().to_owned()).collect(),
            Value::Null => Vec::new(),
            _ => return Err(bad(format!("`{name}` must be a list"))),
        };
        let items: Vec<String> = items.into_iter().filter(|i| !i.is_empty()).collect();
        if items.len() > raft_doc::sanitize::MAX_LIST {
            return Err(bad(format!("`{name}` has too many items")));
        }
        if items.iter().any(|i| i.len() > MAX_SHORT) {
            return Err(bad(format!("an item of `{name}` is too long")));
        }
        Ok(items)
    };
    let json_list =
        |items: Vec<String>| serde_json::to_string(&items).map_err(|e| bad(e.to_string()));
    match field {
        Field::Reviewed => match v {
            Value::Bool(true) => Ok("1".to_owned()),
            Value::Bool(false) | Value::Null => Ok(String::new()),
            _ => Err(bad("`reviewed` must be a boolean")),
        },
        Field::Title => {
            let t = text(MAX_SHORT)?;
            if t.is_empty() {
                return Err(bad(
                    "the title cannot be empty (reset it to use the catalog's)",
                ));
            }
            Ok(t)
        }
        Field::Publisher | Field::Isbn => text(MAX_SHORT),
        Field::Description => text(edits::MAX_VALUE),
        Field::Language => {
            let t = text(8)?.to_lowercase();
            if !(t.is_empty()
                || (2..=3).contains(&t.len()) && t.chars().all(|c| c.is_ascii_lowercase()))
            {
                return Err(bad(
                    "`language` must be an ISO 639 code such as ru, en or ro",
                ));
            }
            Ok(t)
        }
        Field::Year => {
            let y = match v {
                Value::Null => return Ok(String::new()),
                Value::String(t) if t.trim().is_empty() => return Ok(String::new()),
                Value::String(t) => t
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| bad("`year` must be a number"))?,
                Value::Number(n) => n
                    .as_i64()
                    .ok_or_else(|| bad("`year` must be a whole number"))?,
                _ => return Err(bad("`year` must be a number")),
            };
            if !(1000..=2100).contains(&y) {
                return Err(bad("`year` must be between 1000 and 2100"));
            }
            Ok(y.to_string())
        }
        Field::Authors | Field::Tags => json_list(list()?),
        Field::Categories => {
            let items = list()?;
            if let Some(unknown) = items
                .iter()
                .find(|c| !s.catalog.categories.iter().any(|k| &k.slug == *c))
            {
                return Err(EngineError::NotFound(format!("category `{unknown}`")));
            }
            json_list(items)
        }
    }
}

fn view_edit(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    s.state.route = Route::Edit { id };
    Ok(Value::Null)
}

fn book_edit(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    let fields = p
        .get("fields")
        .and_then(Value::as_object)
        .ok_or_else(|| bad("missing `fields` (an object of field: value)"))?;
    if fields.is_empty() {
        return Err(bad("`fields` is empty"));
    }
    let mut changes = Vec::new();
    for (name, v) in fields {
        let field = Field::from_slug(name).ok_or_else(|| bad(format!("unknown field `{name}`")))?;
        changes.push((field, canonical(s, field, v)?));
    }
    let original = s
        .original(&id)
        .cloned()
        .ok_or_else(|| EngineError::NotFound(format!("book `{id}`")))?;
    let entry = s.state.overrides.entry(id.clone()).or_default();
    for (field, value) in changes {
        // A value equal to the catalog's is not a correction.
        if edits::field_value(&original, field) == value {
            entry.remove(&field);
        } else {
            entry.insert(field, value);
        }
    }
    if entry.is_empty() {
        s.state.overrides.remove(&id);
    }
    if s.state.route == (Route::Edit { id: id.clone() }) {
        s.state.route = Route::Book { id: id.clone() };
    }
    Ok(json!({ "edited": s.edited_fields(&id).iter().map(|f| f.slug()).collect::<Vec<_>>() }))
}

fn book_edit_reset(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    let field = opt_str(p, "field")?
        .map(|f| Field::from_slug(f).ok_or_else(|| bad(format!("unknown field `{f}`"))))
        .transpose()?;
    let fields = s
        .state
        .overrides
        .get_mut(&id)
        .ok_or_else(|| EngineError::NotFound(format!("no corrections for `{id}`")))?;
    match field {
        Some(f) => {
            if fields.remove(&f).is_none() {
                return Err(EngineError::NotFound(format!(
                    "`{}` is not corrected",
                    f.slug()
                )));
            }
        }
        None => {
            if fields.keys().all(|f| *f == Field::Reviewed) {
                return Err(EngineError::NotFound(format!("no corrections for `{id}`")));
            }
            fields.retain(|f, _| *f == Field::Reviewed);
        }
    }
    if fields.is_empty() {
        s.state.overrides.remove(&id);
    }
    Ok(Value::Null)
}

fn book_review(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    let current = s.book(&id).is_some_and(|b| b.reviewed);
    let on = opt_bool(p, "on")?.unwrap_or(!current);
    let fields = s.state.overrides.entry(id.clone()).or_default();
    if on {
        fields.insert(Field::Reviewed, "1".to_owned());
    } else {
        fields.remove(&Field::Reviewed);
    }
    if fields.is_empty() {
        s.state.overrides.remove(&id);
    }
    Ok(json!({ "reviewed": on }))
}

fn show_in_folder(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    let book = s
        .book(&id)
        .cloned()
        .ok_or_else(|| EngineError::NotFound(format!("book `{id}`")))?;
    s.backend
        .show_in_folder(&book)
        .map_err(EngineError::Backend)?;
    Ok(Value::Null)
}

fn cover_set(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    let book = s
        .book(&id)
        .cloned()
        .ok_or_else(|| EngineError::NotFound(format!("book `{id}`")))?;
    let path = opt_str(p, "path")?.map(str::trim).filter(|p| !p.is_empty());
    let page = opt_i64(p, "page")?;
    let catalog = opt_bool(p, "catalog")? == Some(true);
    let source = match (path, page, catalog) {
        (Some(path), None, false) => CoverSource::File(path.to_owned()),
        (None, Some(n), false) => {
            if !book.format.is_paged() {
                return Err(bad(
                    "only PDF and DjVu books have pages to take a cover from",
                ));
            }
            let max = book.pages.map_or(100_000, i64::from);
            let n = u32::try_from(n)
                .ok()
                .filter(|n| *n >= 1 && i64::from(*n) <= max)
                .ok_or_else(|| bad(format!("`page` must be between 1 and {max}")))?;
            CoverSource::Page(n)
        }
        (None, None, true) => CoverSource::Catalog,
        _ => return Err(bad("give exactly one of `path`, `page` or `catalog: true`")),
    };
    s.backend
        .set_cover(&book, &source)
        .map_err(EngineError::Backend)?;
    s.bump_cover(&id);
    Ok(json!({ "cover_rev": s.cover_rev(&id) }))
}

fn book_delete(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = book_id(s, p)?;
    if opt_bool(p, "confirm")? != Some(true) {
        return Err(bad(
            "deleting moves the file to the trash; pass `confirm: true`",
        ));
    }
    let book = s
        .book(&id)
        .cloned()
        .ok_or_else(|| EngineError::NotFound(format!("book `{id}`")))?;
    s.backend.remove_book(&book).map_err(EngineError::Backend)?;
    if s.current_book().is_some_and(|b| b.id == id) {
        s.state.route = Route::Library;
    }
    s.forget_book(&id);
    Ok(json!({ "removed": id, "title": book.display_title() }))
}

/// Most blocks a text book may report.
const MAX_TEXT_BLOCKS: i64 = 10_000_000;

fn reader_position(s: &mut Session, p: &Value) -> Result<Value, EngineError> {
    let id = reader_book(s)?;
    let total = opt_i64(p, "total")?.ok_or_else(|| bad("missing `total`"))?;
    let block = opt_i64(p, "block")?.ok_or_else(|| bad("missing `block`"))?;
    if !(1..=MAX_TEXT_BLOCKS).contains(&total) {
        return Err(bad("`total` is out of range"));
    }
    let block = block.clamp(0, total - 1);
    let percent = if total > 1 {
        block as f64 / (total - 1) as f64 * 100.0
    } else {
        100.0
    };
    let position = format!("block:{block}");
    if s.state
        .progress
        .get(&id)
        .and_then(|p| p.position.as_deref())
        == Some(position.as_str())
    {
        return Ok(json!({ "block": block }));
    }
    let now = s.backend.now();
    s.state.progress.insert(
        id.clone(),
        Progress {
            book_id: id,
            page: None,
            position: Some(position),
            percent: percent as f32,
            updated_at: now,
        },
    );
    Ok(json!({ "block": block }))
}
