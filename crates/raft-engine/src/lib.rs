//! Raft engine (layer 5): the session, the command registry, undo history and the
//! panic guard. Every user-visible action is a command run through [`Session::run`].

mod commands;
pub mod control;
mod params;

use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};

use raft_doc::edits::{self, Field, Override};
use raft_doc::{Book, BookId, Catalog, Progress, Query, Route, State};
use raft_search::{Facets, Index};
use serde_json::Value;

pub use commands::{CommandSpec, Shortcut, registry};

/// Errors a command can return.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("command is not available: {0}")]
    Disabled(String),
    #[error("bad parameters: {0}")]
    BadParams(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Backend(String),
    #[error("internal error (the change was rolled back): {0}")]
    Panic(String),
}

/// Platform services the engine needs; the native app implements them.
pub trait Backend: Send {
    /// Full-text search in book content; returns book ids, best first.
    fn content_search(&self, text: &str, limit: usize) -> Result<Vec<BookId>, String>;
    fn save_progress(&self, progress: &Progress) -> Result<(), String>;
    fn delete_progress(&self, book_id: &str) -> Result<(), String>;
    /// Opens the book in the system's default application.
    fn open_external(&self, book: &Book) -> Result<(), String>;
    /// Current time, unix seconds.
    fn now(&self) -> i64;
    /// Stores a metadata correction (insert or replace).
    fn save_override(&self, _o: &Override) -> Result<(), String> {
        Err("saving corrections is not available here".into())
    }
    /// Removes a stored metadata correction.
    fn delete_override(&self, _book_id: &str, _field: Field) -> Result<(), String> {
        Err("saving corrections is not available here".into())
    }
}

/// Backend that does nothing; used in tests and the web build.
#[derive(Debug, Default)]
pub struct NullBackend;

impl Backend for NullBackend {
    fn content_search(&self, _: &str, _: usize) -> Result<Vec<BookId>, String> {
        Ok(Vec::new())
    }
    fn save_progress(&self, _: &Progress) -> Result<(), String> {
        Ok(())
    }
    fn delete_progress(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn open_external(&self, _: &Book) -> Result<(), String> {
        Err("opening files is not available here".into())
    }
    fn now(&self) -> i64 {
        0
    }
    fn save_override(&self, _: &Override) -> Result<(), String> {
        Ok(())
    }
    fn delete_override(&self, _: &str, _: Field) -> Result<(), String> {
        Ok(())
    }
}

/// Most undo steps kept.
pub const MAX_HISTORY: usize = 200;
/// Most content-search hits merged into results.
pub const CONTENT_LIMIT: usize = 200;

#[derive(Debug, Default)]
struct History {
    undo: Vec<State>,
    redo: Vec<State>,
    /// Id of the last recorded command, for coalescing (page turns).
    last: Option<&'static str>,
}

/// One open library: catalog, user state, history and cached results.
pub struct Session {
    /// The catalog as loaded, without corrections.
    base: Catalog,
    /// The catalog with the corrections applied: what everything else sees.
    catalog: Catalog,
    positions: HashMap<BookId, usize>,
    index: Index,
    state: State,
    history: History,
    backend: Box<dyn Backend>,
    results: Vec<usize>,
    facets: Facets,
    content: (String, Vec<BookId>),
    /// Last error or notice for the status bar.
    pub status: Option<String>,
}

impl Session {
    pub fn new(catalog: Catalog, progress: Vec<Progress>, backend: Box<dyn Backend>) -> Self {
        let index = Index::new(&catalog);
        let progress: BTreeMap<BookId, Progress> = progress
            .into_iter()
            .map(|p| (p.book_id.clone(), p))
            .collect();
        let positions = catalog
            .books
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id.clone(), i))
            .collect();
        let mut s = Self {
            base: catalog.clone(),
            catalog,
            positions,
            index,
            state: State {
                progress,
                ..State::default()
            },
            history: History::default(),
            backend,
            results: Vec::new(),
            facets: Facets::default(),
            content: (String::new(), Vec::new()),
            status: None,
        };
        s.refresh();
        s
    }

    /// Loads the stored metadata corrections (at startup; not an undo step).
    pub fn with_overrides(mut self, overrides: Vec<Override>) -> Self {
        let before = self.state.overrides.clone();
        for o in overrides {
            if self.positions.contains_key(&o.book_id) {
                self.state
                    .overrides
                    .entry(o.book_id)
                    .or_default()
                    .insert(o.field, o.value);
            }
        }
        self.apply_overrides(&before);
        self
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The book as in the catalog, without corrections.
    pub fn original(&self, id: &str) -> Option<&Book> {
        self.positions.get(id).and_then(|&i| self.base.books.get(i))
    }

    /// Fields of a book that were corrected by hand.
    pub fn edited_fields(&self, id: &str) -> Vec<Field> {
        self.state
            .overrides
            .get(id)
            .map(|f| f.keys().copied().collect())
            .unwrap_or_default()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn query(&self) -> &Query {
        &self.state.query
    }

    pub fn route(&self) -> &Route {
        &self.state.route
    }

    /// Books matching the current query, in display order.
    pub fn results(&self) -> impl Iterator<Item = &Book> {
        self.results
            .iter()
            .filter_map(|&i| self.catalog.books.get(i))
    }

    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    pub fn facets(&self) -> &Facets {
        &self.facets
    }

    pub fn book(&self, id: &str) -> Option<&Book> {
        self.positions
            .get(id)
            .and_then(|&i| self.catalog.books.get(i))
    }

    pub fn progress(&self, id: &str) -> Option<&Progress> {
        self.state.progress.get(id)
    }

    /// Books with saved progress, most recently read first.
    pub fn continue_reading(&self, limit: usize) -> Vec<&Book> {
        let mut p: Vec<&Progress> = self.state.progress.values().collect();
        p.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
        p.into_iter()
            .filter_map(|p| self.catalog.book(&p.book_id))
            .take(limit)
            .collect()
    }

    /// The book of the current route (book page or reader).
    pub fn current_book(&self) -> Option<&Book> {
        match &self.state.route {
            Route::Book { id } | Route::Reader { id } | Route::Edit { id } => self.book(id),
            Route::Library => None,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    /// Whether `id` can run now; `Err` holds the reason.
    pub fn enabled(&self, id: &str) -> Result<(), String> {
        let spec = commands::find(id).ok_or_else(|| format!("unknown command `{id}`"))?;
        (spec.enabled)(self)
    }

    /// Runs a command. On error or panic, the state is rolled back.
    pub fn run(&mut self, id: &str, params: &Value) -> Result<Value, EngineError> {
        let spec = commands::find(id).ok_or_else(|| EngineError::UnknownCommand(id.to_owned()))?;
        (spec.enabled)(self).map_err(EngineError::Disabled)?;
        let before = self.state.clone();
        let outcome = catch_unwind(AssertUnwindSafe(|| (spec.run)(self, params)));
        let result = match outcome {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_owned())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "panic".to_owned());
                Err(EngineError::Panic(msg))
            }
        };
        match result {
            Ok(value) => {
                if spec.record && self.state != before {
                    self.record(spec.id, before.clone());
                }
                // Cleared first, so errors from saving stay visible.
                self.status = None;
                self.after_change(&before);
                Ok(value)
            }
            Err(e) => {
                self.state = before;
                self.refresh();
                self.status = Some(e.to_string());
                Err(e)
            }
        }
    }

    fn record(&mut self, id: &'static str, before: State) {
        let coalesce = commands::find(id).is_some_and(|s| s.coalesce);
        if !(coalesce && self.history.last == Some(id) && self.history.redo.is_empty()) {
            self.history.undo.push(before);
            if self.history.undo.len() > MAX_HISTORY {
                self.history.undo.remove(0);
            }
        }
        self.history.redo.clear();
        self.history.last = Some(id);
    }

    fn undo(&mut self) -> Result<(), EngineError> {
        let prev = self
            .history
            .undo
            .pop()
            .ok_or_else(|| EngineError::Disabled("nothing to undo".into()))?;
        let cur = std::mem::replace(&mut self.state, prev);
        self.history.redo.push(cur);
        self.history.last = None;
        Ok(())
    }

    fn redo(&mut self) -> Result<(), EngineError> {
        let next = self
            .history
            .redo
            .pop()
            .ok_or_else(|| EngineError::Disabled("nothing to redo".into()))?;
        let cur = std::mem::replace(&mut self.state, next);
        self.history.undo.push(cur);
        self.history.last = None;
        Ok(())
    }

    /// Persists progress changes and refreshes results when the query changed.
    fn after_change(&mut self, before: &State) {
        if before.progress != self.state.progress {
            let mut errors = Vec::new();
            for (id, p) in &self.state.progress {
                if before.progress.get(id) != Some(p)
                    && let Err(e) = self.backend.save_progress(p)
                {
                    errors.push(e);
                }
            }
            for id in before.progress.keys() {
                if !self.state.progress.contains_key(id)
                    && let Err(e) = self.backend.delete_progress(id)
                {
                    errors.push(e);
                }
            }
            if let Some(e) = errors.into_iter().next() {
                self.status = Some(format!("progress not saved: {e}"));
            }
        }
        let edited = before.overrides != self.state.overrides;
        if edited {
            self.persist_overrides(&before.overrides);
            self.apply_overrides(&before.overrides);
        }
        if edited || before.query != self.state.query {
            self.refresh();
        }
    }

    /// Writes the corrections that changed since `before` to the backend.
    fn persist_overrides(&mut self, before: &edits::Overrides) {
        let mut errors = Vec::new();
        for (id, fields) in &self.state.overrides {
            for (field, value) in fields {
                let old = before.get(id).and_then(|f| f.get(field));
                if old != Some(value) {
                    let o = Override {
                        book_id: id.clone(),
                        field: *field,
                        value: value.clone(),
                    };
                    if let Err(e) = self.backend.save_override(&o) {
                        errors.push(e);
                    }
                }
            }
        }
        for (id, fields) in before {
            for field in fields.keys() {
                let still = self
                    .state
                    .overrides
                    .get(id)
                    .is_some_and(|f| f.contains_key(field));
                if !still && let Err(e) = self.backend.delete_override(id, *field) {
                    errors.push(e);
                }
            }
        }
        if let Some(e) = errors.into_iter().next() {
            self.status = Some(format!("correction not saved: {e}"));
        }
    }

    /// Rebuilds the corrected books that changed since `before`, then the index.
    fn apply_overrides(&mut self, before: &edits::Overrides) {
        let mut ids: Vec<&BookId> = self.state.overrides.keys().collect();
        ids.extend(before.keys());
        ids.sort_unstable();
        ids.dedup();
        let mut changed = false;
        for id in ids {
            if before.get(id) == self.state.overrides.get(id) {
                continue;
            }
            if let Some(&i) = self.positions.get(id)
                && let (Some(base), Some(slot)) =
                    (self.base.books.get(i), self.catalog.books.get_mut(i))
            {
                *slot = edits::edited(base, &self.state.overrides);
                changed = true;
            }
        }
        if changed {
            self.index = Index::new(&self.catalog);
        }
    }

    fn refresh(&mut self) {
        let text = self.state.query.text.trim().to_owned();
        if self.content.0 != text {
            let hits = if text.is_empty() {
                Ok(Vec::new())
            } else {
                self.backend.content_search(&text, CONTENT_LIMIT)
            };
            self.content = (text, hits.unwrap_or_default());
        }
        let q = &self.state.query;
        self.results = raft_search::search(&self.catalog, &self.index, q, &self.content.1);
        self.facets = raft_search::facets(&self.catalog, &self.index, q, &self.content.1);
    }

    /// JSON summary for the control channel and tests.
    pub fn snapshot(&self) -> Value {
        serde_json::json!({
            "route": self.state.route,
            "query": self.state.query,
            "results": self.results.len(),
            "first_results": self.results().take(10).map(|b| &b.id).collect::<Vec<_>>(),
            "progress": self.state.progress.len(),
            "edited_books": self.state.overrides.len(),
            "can_undo": self.can_undo(),
            "can_redo": self.can_redo(),
            "status": self.status,
        })
    }
}

#[cfg(test)]
mod tests;
