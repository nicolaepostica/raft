//! The command registry: every user-visible action, with its menu, shortcut and params.

use raft_doc::{Format, Progress, Route, Sort};
use serde_json::{Value, json};

use crate::params::{opt_bool, opt_i64, opt_str, str};
use crate::{EngineError, Session};

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
        params: r#"{"min": int|null, "max": int|null}"#,
        coalesce: true,
        ..spec("filter.years", "Filtru: ani", filter_years)
    },
    CommandSpec {
        params: r#"{"sort": "relevance"|"title"|"year_desc"|"year_asc"|"recently_added"}"#,
        ..spec("sort.set", "Sortare", sort_set)
    },
    CommandSpec {
        menu: Some("View/Library"),
        shortcut: key("Escape"),
        enabled: not_library,
        ..spec("view.library", "Biblioteca", view_library)
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
        params: r#"{"page": int}"#,
        enabled: in_reader,
        coalesce: true,
        ..spec("reader.goto", "Mergi la pagina", reader_goto)
    },
    CommandSpec {
        shortcut: key("ArrowRight"),
        enabled: in_reader,
        coalesce: true,
        ..spec("reader.next", "Pagina următoare", |s, _| reader_step(s, 1))
    },
    CommandSpec {
        shortcut: key("ArrowLeft"),
        enabled: in_reader,
        coalesce: true,
        ..spec("reader.prev", "Pagina anterioară", |s, _| {
            reader_step(s, -1)
        })
    },
    CommandSpec {
        params: r#"{"id": string?} (default: current book)"#,
        ..spec("progress.reset", "Șterge progresul", progress_reset)
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

fn view_library(s: &mut Session, _: &Value) -> Result<Value, EngineError> {
    s.state.route = match &s.state.route {
        Route::Reader { id } => Route::Book { id: id.clone() },
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
