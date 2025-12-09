//! Raft engine (layer 5): the session, the command registry, undo history and the
//! panic guard. Every user-visible action is a command run through [`Session::run`].

mod commands;
pub mod control;
pub mod mcp;
mod params;

use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};

use raft_doc::edits::{self, Field, Override};
use raft_doc::{Book, BookId, Catalog, Progress, Query, Route, State};
use raft_search::Index;
use serde_json::Value;

pub use commands::{CommandSpec, Shortcut, registry};
pub use raft_doc::Source;
pub use raft_search::Facets;

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

/// Where a new cover comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverSource {
    /// An image file on disk (PNG, JPEG, WebP).
    File(String),
    /// A page of the book (paged formats), 1-based.
    Page(u32),
    /// Drop the custom cover and use the catalog's again.
    Catalog,
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
    /// Shows the book's file in the system's file manager.
    fn show_in_folder(&self, _book: &Book) -> Result<(), String> {
        Err("showing files is not available here".into())
    }
    /// Moves the book's file to the trash and records the removal, so the book
    /// stays hidden until the catalog is rebuilt without it.
    fn remove_book(&self, _book: &Book) -> Result<(), String> {
        Err("removing books is not available here".into())
    }
    /// Saves a preference (`theme` = `light` | `dark`).
    fn save_setting(&self, _key: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }
    /// Opens a library folder (its `catalog.db`) for content search and returns
    /// its books tagged with their source. Does not remember it: see `save_source`.
    fn open_source(&self, _root: &str) -> Result<Catalog, String> {
        Err("adding library folders is not available here".into())
    }
    /// Whether the folder is there but has no catalog yet, so it must be built
    /// first (`start_index`).
    fn needs_index(&self, _root: &str) -> bool {
        false
    }
    /// Starts building (or bringing up to date) the folder's catalog in the
    /// background, without AI; does nothing when a build of it already runs.
    fn start_index(&self, _root: &str) -> Result<(), String> {
        Err("building catalogs is not available here".into())
    }
    /// How the folder's build goes; `Done` / `Failed` are reported once, then
    /// `None` (also when no build was started).
    fn index_state(&self, _root: &str) -> Option<IndexState> {
        None
    }
    /// Reads a book file from outside the library: its id (the md5 of the
    /// file, like the catalog's), size and page count.
    fn open_file(&self, _path: &str) -> Result<Book, String> {
        Err("opening files is not available here".into())
    }
    /// Remembers a book file opened from outside the library (most recent first).
    fn save_file(&self, _book: &Book) -> Result<(), String> {
        Ok(())
    }
    /// Forgets a book file opened from outside the library (the file stays).
    fn forget_file(&self, _id: &str) -> Result<(), String> {
        Ok(())
    }
    /// Closes an open library folder (its files are not touched).
    fn close_source(&self, _root: &str) -> Result<(), String> {
        Ok(())
    }
    /// Remembers a library folder, or updates its name and switch.
    fn save_source(&self, _source: &Source) -> Result<(), String> {
        Err("saving library folders is not available here".into())
    }
    /// Forgets a library folder.
    fn forget_source(&self, _root: &str) -> Result<(), String> {
        Err("removing library folders is not available here".into())
    }
    /// A remembered folder moved to `to` (it keeps its place in the list).
    fn move_source(&self, _from: &str, _to: &str) -> Result<(), String> {
        Err("moving library folders is not available here".into())
    }
    /// Stores a custom cover for the book (kept apart from the catalog's covers).
    fn set_cover(&self, _book: &Book, _source: &CoverSource) -> Result<(), String> {
        Err("changing covers is not available here".into())
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

/// Whether a library folder's books are in the session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "reason")]
pub enum SourceStatus {
    Loaded,
    /// Switched off by the user.
    Disabled,
    /// Switched on but could not be opened (unplugged disk, a failed catalog build).
    Unavailable(String),
}

/// How far a folder's catalog build has got (books processed of those to do).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct IndexProgress {
    pub done: usize,
    pub total: usize,
}

/// A catalog build started by [`Backend::start_index`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexState {
    Running(IndexProgress),
    Done,
    Failed(String),
}

/// A library folder and its state in this session.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SourceEntry {
    #[serde(flatten)]
    pub source: Source,
    pub status: SourceStatus,
    /// Its catalog is being built (its books arrive when the build ends).
    pub indexing: Option<IndexProgress>,
    /// Its books, as loaded (before merging and corrections).
    #[serde(skip)]
    books: Catalog,
}

impl SourceEntry {
    pub fn loaded(&self) -> bool {
        self.status == SourceStatus::Loaded
    }
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
    /// Bumped each time a book's cover changes, so views reload it.
    cover_revs: BTreeMap<BookId, u32>,
    /// Dark theme instead of the light one (a preference, not undone).
    dark: bool,
    /// Interface language code (`ro`, `en`, `ru`).
    language: String,
    /// Library folders the books come from, in the order they were added.
    sources: Vec<SourceEntry>,
    /// Book files opened from outside the library, most recent first: readable,
    /// never in the search results.
    files: Vec<Book>,
    /// Books that belong to no configured folder (tests, the web build).
    loose: Catalog,
    /// The settings window is open (a view state, not undone).
    settings: bool,
    /// Its tab: one of `commands::SETTINGS_TABS`.
    settings_tab: &'static str,
    /// The library screen's tab: one of `commands::LIBRARY_TABS` (a view state, not undone).
    library_tab: &'static str,
    /// Reader zoom: page width (PDF, DjVu) or letter size (EPUB, FB2), 1 = default.
    zoom: f32,
    /// Books ticked in the library, for actions on many at once (a view state,
    /// not undone; books that leave the catalog leave it too).
    selected: std::collections::BTreeSet<BookId>,
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
            loose: catalog.clone(),
            settings: false,
            settings_tab: commands::SETTINGS_TABS[0],
            library_tab: commands::LIBRARY_TABS[0],
            zoom: 1.0,
            selected: Default::default(),
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
            cover_revs: BTreeMap::new(),
            dark: false,
            language: "ro".to_owned(),
            sources: Vec::new(),
            files: Vec::new(),
        };
        raft_search::mark_duplicates(&mut s.catalog.books);
        s.refresh();
        s
    }

    /// Loads the stored metadata corrections (at startup; not an undo step).
    pub fn with_overrides(mut self, overrides: Vec<Override>) -> Self {
        let before = self.state.overrides.clone();
        // Kept even for books not loaded now: their folder may be added later.
        for o in overrides {
            self.state
                .overrides
                .entry(o.book_id)
                .or_default()
                .insert(o.field, o.value);
        }
        self.apply_overrides(&before);
        self
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Starts with the saved theme.
    pub fn with_dark(mut self, dark: bool) -> Self {
        self.dark = dark;
        self
    }

    pub fn dark(&self) -> bool {
        self.dark
    }

    /// Starts with the saved interface language (unknown codes are ignored).
    pub fn with_language(mut self, code: &str) -> Self {
        if commands::LANGUAGES.contains(&code) {
            self.language = code.to_owned();
        }
        self
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// Changes each time the book's cover is replaced (0 = never in this session).
    pub fn cover_rev(&self, id: &str) -> u32 {
        self.cover_revs.get(id).copied().unwrap_or(0)
    }

    pub(crate) fn bump_cover(&mut self, id: &str) {
        *self.cover_revs.entry(id.to_owned()).or_default() += 1;
    }

    /// Drops a book from the session (after its file was removed).
    pub(crate) fn forget_book(&mut self, id: &str) {
        self.forget_books(&[id.to_owned()]);
    }

    /// Drops books from the session (after their files were removed), rebuilding once.
    pub(crate) fn forget_books(&mut self, ids: &[BookId]) {
        let gone: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        self.loose.books.retain(|b| !gone.contains(b.id.as_str()));
        for e in &mut self.sources {
            e.books.books.retain(|b| !gone.contains(b.id.as_str()));
        }
        self.rebuild();
    }

    /// The books ticked in the library.
    pub fn selected(&self) -> &std::collections::BTreeSet<BookId> {
        &self.selected
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selected.contains(id)
    }

    /// The library folders, in the order they were added, with their state.
    pub fn sources(&self) -> &[SourceEntry] {
        &self.sources
    }

    pub(crate) fn source(&self, root: &str) -> Option<&SourceEntry> {
        self.sources.iter().find(|e| e.source.path == root)
    }

    pub(crate) fn source_mut(&mut self, root: &str) -> Option<&mut SourceEntry> {
        self.sources.iter_mut().find(|e| e.source.path == root)
    }

    /// The folders whose books are loaded.
    pub fn loaded_sources(&self) -> impl Iterator<Item = &Source> {
        self.sources
            .iter()
            .filter(|e| e.loaded())
            .map(|e| &e.source)
    }

    /// How many of the library's books come from this folder.
    pub fn source_books(&self, root: &str) -> usize {
        self.catalog
            .books
            .iter()
            .filter(|b| b.source == root)
            .count()
    }

    /// A subfolder as shown: its path inside its library folder (`Author/Series`),
    /// with the library folder's name when several are loaded.
    pub fn folder_label(&self, dir: &str) -> String {
        let source = self.sources.iter().filter(|e| e.loaded()).find(|e| {
            dir.strip_prefix(e.source.path.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        });
        match source {
            Some(e) => {
                let rest = dir.get(e.source.path.len() + 1..).unwrap_or(dir);
                if self.loaded_sources().count() > 1 {
                    format!("{rest} · {}", e.source.display_name())
                } else {
                    rest.to_owned()
                }
            }
            None => dir.to_owned(),
        }
    }

    /// The name shown for a book's folder.
    pub fn source_name<'a>(&'a self, root: &'a str) -> &'a str {
        self.source(root).map_or(root, |e| e.source.display_name())
    }

    /// Starts with these folders, whose books are already in the catalog given
    /// to `new` (tests, the web build); the books of disabled ones are dropped.
    pub fn with_sources<S: Into<Source>>(mut self, sources: Vec<S>) -> Self {
        for source in sources {
            let source = source.into();
            let (mine, rest) = std::mem::take(&mut self.loose.books)
                .into_iter()
                .partition(|b| b.source == source.path);
            self.loose.books = rest;
            let status = if source.enabled {
                SourceStatus::Loaded
            } else {
                SourceStatus::Disabled
            };
            self.sources.push(SourceEntry {
                source,
                status,
                indexing: None,
                books: Catalog {
                    books: mine,
                    categories: Vec::new(),
                },
            });
        }
        self.rebuild();
        self
    }

    /// Starts with the saved folders, opening the enabled ones through the
    /// backend (or building the catalog of one that has none yet); one that
    /// cannot be opened stays in the list as unavailable.
    pub fn load_sources(mut self, sources: Vec<Source>) -> Self {
        for source in sources {
            let (status, opened) = if source.enabled {
                match self.open_or_index(&source.path) {
                    Ok(c) => (SourceStatus::Loaded, c),
                    Err(e) => (SourceStatus::Unavailable(e), Some(Catalog::default())),
                }
            } else {
                (SourceStatus::Disabled, Some(Catalog::default()))
            };
            self.sources.push(SourceEntry {
                source,
                status,
                indexing: opened.is_none().then(IndexProgress::default),
                books: opened.unwrap_or_default(),
            });
        }
        self.rebuild();
        self
    }

    /// Opens a folder's catalog, or starts building it when the folder has
    /// none yet (`Ok(None)`: its books arrive through `poll_indexing`).
    pub(crate) fn open_or_index(&self, root: &str) -> Result<Option<Catalog>, String> {
        if self.backend.needs_index(root) {
            self.backend.start_index(root)?;
            return Ok(None);
        }
        self.backend.open_source(root).map(Some)
    }

    /// Adds a folder with its books (already tagged with its path), or with
    /// none while its catalog is being built (`None`).
    pub(crate) fn add_source(&mut self, source: Source, catalog: Option<Catalog>) {
        self.sources.push(SourceEntry {
            source,
            status: SourceStatus::Loaded,
            indexing: catalog.is_none().then(IndexProgress::default),
            books: catalog.unwrap_or_default(),
        });
        self.rebuild();
    }

    /// Whether some folder's catalog is being built.
    pub fn indexing(&self) -> bool {
        self.sources.iter().any(|e| e.indexing.is_some())
    }

    /// Follows the catalog builds (call it every frame): a finished folder is
    /// opened again with its new books. Returns whether a build still runs.
    pub fn poll_indexing(&mut self) -> bool {
        let roots: Vec<String> = self
            .sources
            .iter()
            .filter(|e| e.indexing.is_some())
            .map(|e| e.source.path.clone())
            .collect();
        for root in roots {
            match self.backend.index_state(&root) {
                Some(IndexState::Running(p)) => {
                    if let Some(e) = self.source_mut(&root) {
                        e.indexing = Some(p);
                    }
                }
                // A build the backend no longer knows is taken as finished.
                Some(IndexState::Done) | None => self.index_finished(&root, None),
                Some(IndexState::Failed(e)) => self.index_finished(&root, Some(e)),
            }
        }
        self.indexing()
    }

    /// A folder's build ended (`error` when it failed): reloads its books.
    fn index_finished(&mut self, root: &str, error: Option<String>) {
        let Some(entry) = self.source_mut(root) else {
            return;
        };
        entry.indexing = None;
        let enabled = entry.source.enabled;
        let had_books = !entry.books.books.is_empty();
        if let Some(e) = error {
            let msg = format!("building the catalog of {root} failed: {e}");
            if enabled && !had_books {
                self.set_source_books(
                    root,
                    SourceStatus::Unavailable(msg.clone()),
                    Catalog::default(),
                );
            }
            self.status = Some(msg);
            return;
        }
        if !enabled {
            return;
        }
        let _ = self.backend.close_source(root);
        match self.backend.open_source(root) {
            Ok(books) => self.set_source_books(root, SourceStatus::Loaded, books),
            Err(e) => {
                self.set_source_books(
                    root,
                    SourceStatus::Unavailable(e.clone()),
                    Catalog::default(),
                );
                self.status = Some(e);
            }
        }
    }

    /// Marks a folder as being indexed.
    pub(crate) fn set_indexing(&mut self, root: &str) {
        if let Some(e) = self.source_mut(root) {
            e.indexing = Some(IndexProgress::default());
        }
    }

    /// Sets a folder's books and state (empty books when it is not loaded).
    pub(crate) fn set_source_books(&mut self, root: &str, status: SourceStatus, books: Catalog) {
        if let Some(e) = self.source_mut(root) {
            e.status = status;
            e.books = books;
        }
        if self.source(root).is_some_and(|e| !e.loaded()) {
            self.unshow_source(root);
        }
        self.rebuild();
    }

    /// Drops a folder and its books.
    pub(crate) fn drop_source(&mut self, root: &str) {
        self.sources.retain(|e| e.source.path != root);
        self.unshow_source(root);
        self.rebuild();
    }

    /// Leaves a folder that is no longer shown: its filter, and its book if open.
    fn unshow_source(&mut self, root: &str) {
        self.state.query.filters.sources.remove(root);
        let inside = format!("{root}/");
        self.state
            .query
            .filters
            .folders
            .retain(|d| !d.starts_with(&inside));
        if self.current_book().is_some_and(|b| b.source == root) {
            self.state.route = Route::Library;
        }
    }

    /// The settings window is open.
    pub fn settings_open(&self) -> bool {
        self.settings
    }

    pub(crate) fn set_settings_open(&mut self, open: bool) {
        self.settings = open;
    }

    /// The reader zoom (1 = the page fits the width / the default letter size).
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Starts with the saved reader zoom (out of range or invalid = 1).
    pub fn with_zoom(mut self, zoom: f32) -> Self {
        if zoom.is_finite() && (commands::MIN_ZOOM..=commands::MAX_ZOOM).contains(&zoom) {
            self.zoom = zoom;
        }
        self
    }

    /// The settings window's tab (`general` or `shortcuts`).
    pub fn settings_tab(&self) -> &'static str {
        self.settings_tab
    }

    /// The library screen's tab (`library`: search and filters, or `reading`: started books).
    pub fn library_tab(&self) -> &'static str {
        self.library_tab
    }

    /// Recomputes `base` from the loaded folders, then the corrected catalog,
    /// positions and index. A book in two folders keeps its first loaded copy.
    fn rebuild(&mut self) {
        let mut base = self.loose.clone();
        for e in self.sources.iter().filter(|e| e.loaded()) {
            base.merge(e.books.clone());
        }
        self.base = base;
        self.catalog = Catalog {
            books: self
                .base
                .books
                .iter()
                .map(|b| edits::edited(b, &self.state.overrides))
                .collect(),
            categories: self.base.categories.clone(),
        };
        raft_search::mark_duplicates(&mut self.catalog.books);
        self.positions = self
            .catalog
            .books
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id.clone(), i))
            .collect();
        self.index = Index::new(&self.catalog);
        let positions = &self.positions;
        self.selected.retain(|id| positions.contains_key(id));
        self.refresh();
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
            .map(|f| f.keys().copied().filter(|f| !f.is_mark()).collect())
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

    /// A book of the library, or a file opened from outside it.
    pub fn book(&self, id: &str) -> Option<&Book> {
        self.positions
            .get(id)
            .and_then(|&i| self.catalog.books.get(i))
            .or_else(|| self.files.iter().find(|b| b.id == id))
    }

    /// Starts with the book files opened earlier from outside the library.
    pub fn with_files(mut self, files: Vec<Book>) -> Self {
        self.files = files;
        self.files.truncate(commands::MAX_FILES);
        self
    }

    /// Book files opened from outside the library, most recent first.
    pub fn files(&self) -> &[Book] {
        &self.files
    }

    /// Whether the book is a file opened from outside the library (and not in it).
    pub fn is_outside(&self, id: &str) -> bool {
        !self.positions.contains_key(id) && self.files.iter().any(|b| b.id == id)
    }

    pub fn progress(&self, id: &str) -> Option<&Progress> {
        self.state.progress.get(id)
    }

    /// Books with saved progress, most recently read first.
    pub fn continue_reading(&self, limit: usize) -> Vec<&Book> {
        let mut p: Vec<&Progress> = self.state.progress.values().collect();
        p.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
        p.into_iter()
            .filter_map(|p| self.book(&p.book_id))
            .take(limit)
            .collect()
    }

    /// The next volume of `id`'s series: the lowest volume number above its own.
    pub fn next_volume(&self, id: &str) -> Option<&Book> {
        let book = self.book(id)?;
        let (series, index) = (book.series.as_ref()?, book.series_index?);
        self.catalog
            .books
            .iter()
            .filter(|b| b.series.as_ref() == Some(series))
            .filter_map(|b| Some((b.series_index?, b)))
            .filter(|(n, _)| *n > index)
            .min_by_key(|(n, b)| (*n, b.title.clone()))
            .map(|(_, b)| b)
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
            // A corrected ISBN, title or year, or a "not a duplicate" mark.
            raft_search::mark_duplicates(&mut self.catalog.books);
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
    /// A page of the books found, in their order, with the fields that tell them apart
    /// (`results` on the control channel).
    pub fn results_page(&self, offset: usize, limit: usize) -> Value {
        let books: Vec<Value> = self
            .results()
            .skip(offset)
            .take(limit)
            .map(|b| {
                serde_json::json!({
                    "id": b.id,
                    "title": b.title,
                    "authors": b.authors,
                    "year": b.year,
                    "format": b.format,
                    "language": b.language,
                    "pages": b.pages,
                    "series": b.series,
                    "series_index": b.series_index,
                    "categories": b.categories,
                    "file": b.file_path(),
                })
            })
            .collect();
        serde_json::json!({ "total": self.results.len(), "offset": offset, "books": books })
    }

    pub fn snapshot(&self) -> Value {
        serde_json::json!({
            "route": self.state.route,
            "query": self.state.query,
            "results": self.results.len(),
            "first_results": self.results().take(10).map(|b| &b.id).collect::<Vec<_>>(),
            "progress": self.state.progress.len(),
            "edited_books": self.catalog.books.iter().filter(|b| b.edited).count(),
            "incomplete_books": self.catalog.books.iter().filter(|b| b.is_incomplete()).count(),
            "can_undo": self.can_undo(),
            "can_redo": self.can_redo(),
            "status": self.status,
            "theme": if self.dark { "dark" } else { "light" },
            "language": self.language,
            "sources": self.sources,
            "settings": self.settings,
            "settings_tab": self.settings_tab,
            "library_tab": self.library_tab,
            "zoom": self.zoom,
            "selected": self.selected,
            "files": self.files.iter().map(|b| serde_json::json!({"id": b.id, "path": b.file_path(), "title": b.title})).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests;
