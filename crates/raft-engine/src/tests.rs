use std::sync::{Arc, Mutex};

use proptest::prelude::*;
use raft_doc::edits::{Field, Override};
use raft_doc::{Book, Catalog, Category, Format, Progress, ReadState, Route};
use serde_json::{Value, json};

use crate::{Backend, EngineError, NullBackend, Session, registry};

/// (book id, field, value; None = deleted)
type StoredEdit = (String, Field, Option<String>);

#[derive(Default, Clone)]
struct Recorder {
    saved: Arc<Mutex<Vec<Progress>>>,
    opened: Arc<Mutex<Vec<String>>>,
    overrides: Arc<Mutex<Vec<StoredEdit>>>,
    fail_overrides: bool,
    /// Ids passed to `remove_book` / `show_in_folder`.
    removed: Arc<Mutex<Vec<String>>>,
    shown: Arc<Mutex<Vec<String>>>,
    covers: Arc<Mutex<Vec<(String, crate::CoverSource)>>>,
    /// The remembered folders, as `save_source` / `move_source` / `forget_source` left them.
    sources: Arc<Mutex<Vec<raft_doc::Source>>>,
    /// Folders `start_index` was called for, and the build state it reports.
    indexed: Arc<Mutex<Vec<String>>>,
    index: Arc<Mutex<Option<crate::IndexState>>>,
    /// The catalog of "/fresh" and "/broken" has been built.
    built: Arc<Mutex<bool>>,
    /// Book files remembered by `save_file` (ids), most recent first.
    files: Arc<Mutex<Vec<String>>>,
}

impl Recorder {
    fn set_index(&self, state: crate::IndexState) {
        *self.index.lock().unwrap() = Some(state);
    }
}

impl Backend for Recorder {
    fn content_search(&self, text: &str, _: usize) -> Result<Vec<String>, String> {
        Ok(if text == "inside" {
            vec!["b".into()]
        } else {
            vec![]
        })
    }
    fn save_progress(&self, p: &Progress) -> Result<(), String> {
        self.saved
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(p.clone());
        Ok(())
    }
    fn delete_progress(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn open_external(&self, b: &Book) -> Result<(), String> {
        self.opened
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(b.id.clone());
        Ok(())
    }
    fn now(&self) -> i64 {
        1_700_000_000
    }
    fn save_override(&self, o: &Override) -> Result<(), String> {
        if self.fail_overrides {
            return Err("disk full".into());
        }
        self.overrides
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((o.book_id.clone(), o.field, Some(o.value.clone())));
        Ok(())
    }
    fn delete_override(&self, id: &str, field: Field) -> Result<(), String> {
        self.overrides
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id.to_owned(), field, None));
        Ok(())
    }
    fn remove_book(&self, b: &Book) -> Result<(), String> {
        if b.id == "b" {
            return Err("trash is full".into());
        }
        self.removed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(b.id.clone());
        Ok(())
    }
    fn set_cover(&self, b: &Book, source: &crate::CoverSource) -> Result<(), String> {
        self.covers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((b.id.clone(), source.clone()));
        Ok(())
    }
    fn save_source(&self, src: &raft_doc::Source) -> Result<(), String> {
        let mut all = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        match all.iter_mut().find(|s| s.path == src.path) {
            Some(s) => *s = src.clone(),
            None => all.push(src.clone()),
        }
        Ok(())
    }
    fn forget_source(&self, root: &str) -> Result<(), String> {
        let mut all = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|s| s.path != root);
        Ok(())
    }
    fn move_source(&self, from: &str, to: &str) -> Result<(), String> {
        let mut all = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        for s in all.iter_mut().filter(|s| s.path == from) {
            s.path = to.into();
        }
        Ok(())
    }
    fn open_file(&self, path: &str) -> Result<Book, String> {
        // The id is the file's stem in lower case ("a" is the library's book).
        let stem = path
            .rsplit('/')
            .next()
            .and_then(|n| n.split(['.', '_']).next())
            .unwrap_or_default()
            .to_lowercase();
        let pages = path.ends_with(".pdf").then_some(12);
        Book::outside(path, stem, 100, pages).ok_or_else(|| format!("not a book file: {path}"))
    }
    fn save_file(&self, b: &Book) -> Result<(), String> {
        let mut all = self.files.lock().unwrap();
        all.retain(|id| *id != b.id);
        all.insert(0, b.id.clone());
        Ok(())
    }
    fn forget_file(&self, id: &str) -> Result<(), String> {
        self.files.lock().unwrap().retain(|f| f != id);
        Ok(())
    }
    fn needs_index(&self, root: &str) -> bool {
        (root == "/fresh" || root == "/broken") && !*self.built.lock().unwrap()
    }
    fn start_index(&self, root: &str) -> Result<(), String> {
        self.indexed.lock().unwrap().push(root.into());
        self.set_index(crate::IndexState::Running(Default::default()));
        Ok(())
    }
    fn index_state(&self, _: &str) -> Option<crate::IndexState> {
        let mut state = self.index.lock().unwrap();
        match state.clone() {
            Some(crate::IndexState::Running(p)) => Some(crate::IndexState::Running(p)),
            _ => state.take(),
        }
    }
    fn open_source(&self, root: &str) -> Result<Catalog, String> {
        if root == "/fresh" && *self.built.lock().unwrap() {
            return Ok(Catalog {
                books: vec![Book {
                    id: "f".into(),
                    title: "Fresh Book".into(),
                    source: root.into(),
                    format: Format::Epub,
                    ..Book::default()
                }],
                categories: vec![],
            });
        }
        if root != "/disk2" && root != "/disk3" {
            return Err(format!("no catalog.db in {root}"));
        }
        let book = |id: &str, title: &str| Book {
            id: id.into(),
            title: title.into(),
            source: root.into(),
            format: Format::Pdf,
            ..Book::default()
        };
        Ok(Catalog {
            // "a" is the same file as the first library's: it keeps its first copy.
            books: vec![book("a", "Rust in Action (copy)"), book("c", "Clean Code")],
            categories: vec![],
        })
    }
    fn close_source(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn show_in_folder(&self, b: &Book) -> Result<(), String> {
        self.shown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(b.id.clone());
        Ok(())
    }
}

fn catalog() -> Catalog {
    Catalog {
        books: vec![
            Book {
                id: "a".into(),
                title: "Rust in Action".into(),
                format: Format::Pdf,
                pages: Some(10),
                categories: vec!["go-rust".into()],
                language: "en".into(),
                year: Some(2021),
                ..Book::default()
            },
            Book {
                id: "b".into(),
                title: "Война и мир".into(),
                format: Format::Fb2,
                categories: vec!["fiction".into()],
                language: "ru".into(),
                ..Book::default()
            },
        ],
        categories: vec![
            Category {
                slug: "go-rust".into(),
                name: "Go, Rust".into(),
            },
            Category {
                slug: "fiction".into(),
                name: "Beletristică".into(),
            },
        ],
    }
}

fn session() -> (Session, Recorder) {
    let r = Recorder::default();
    (Session::new(catalog(), vec![], Box::new(r.clone())), r)
}

#[test]
fn registry_ids_are_unique_and_shortcuts_distinct() {
    let mut ids: Vec<_> = registry().iter().map(|c| c.id).collect();
    ids.sort_unstable();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n);
    let mut keys: Vec<_> = registry()
        .iter()
        .flat_map(|c| c.shortcut.into_iter().chain(c.also.iter().copied()))
        .map(|s| (s.key, s.command, s.shift, s.alt))
        .collect();
    keys.sort_unstable();
    let n = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), n);
}

#[test]
fn search_filters_and_undo() {
    let (mut s, _) = session();
    assert_eq!(s.result_count(), 2);
    s.run("search.set", &json!({"text": "rust"})).unwrap();
    assert_eq!(s.result_count(), 1);
    s.run("search.set", &json!({"text": "inside"})).unwrap();
    assert_eq!(s.results().next().map(|b| b.id.as_str()), Some("b"));
    s.run("filter.language.toggle", &json!({"language": "en"}))
        .unwrap();
    assert_eq!(s.result_count(), 0);
    s.run("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.result_count(), 1);
    // Both search.set runs coalesced into one step.
    s.run("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.result_count(), 2);
    assert!(s.run("edit.undo", &Value::Null).is_err());
    s.run("edit.redo", &Value::Null).unwrap();
    assert_eq!(s.query().text, "inside");
}

#[test]
fn reader_saves_progress_and_clamps_pages() {
    let (mut s, rec) = session();
    s.run("view.book", &json!({"id": "a"})).unwrap();
    s.run("book.read", &Value::Null).unwrap();
    assert_eq!(s.route(), &Route::Reader { id: "a".into() });
    s.run("reader.next", &Value::Null).unwrap();
    s.run("reader.goto", &json!({"page": 999})).unwrap();
    let p = s.progress("a").unwrap();
    assert_eq!(p.page, Some(10));
    assert!((p.percent - 100.0).abs() < f32::EPSILON);
    assert!(!rec.saved.lock().unwrap().is_empty());
    s.run("view.library", &Value::Null).unwrap();
    assert_eq!(s.route(), &Route::Book { id: "a".into() });
    assert_eq!(s.continue_reading(5).len(), 1);
}

#[test]
fn text_books_open_in_the_text_reader_and_save_their_position() {
    let (mut s, rec) = session();
    let r = s.run("book.read", &json!({"id": "b"})).unwrap();
    assert_eq!(r["reader"], "text");
    assert_eq!(s.route(), &Route::Reader { id: "b".into() });
    assert!(s.enabled("reader.next").is_err());
    assert!(s.run("reader.goto", &json!({"page": 3})).is_err());
    s.run("reader.position", &json!({"block": 50, "total": 101}))
        .unwrap();
    s.run("reader.position", &json!({"block": 75, "total": 101}))
        .unwrap();
    let p = s.progress("b").unwrap();
    assert_eq!(p.block(), Some(75));
    assert!((p.percent - 75.0).abs() < 0.01);
    assert!(rec.saved.lock().unwrap().len() >= 2);
    assert!(
        s.run("reader.position", &json!({"block": 1, "total": 0}))
            .is_err()
    );
    // Reopening starts at the saved block; page books still refuse positions.
    s.run("view.library", &Value::Null).unwrap();
    let r = s.run("book.read", &json!({"id": "b"})).unwrap();
    assert_eq!(r["block"], 75);
    s.run("book.read", &json!({"id": "a"})).unwrap();
    assert!(
        s.run("reader.position", &json!({"block": 1, "total": 5}))
            .is_err()
    );
    // Opening externally stays available.
    s.run("book.open_external", &json!({"id": "b"})).unwrap();
    assert_eq!(rec.opened.lock().unwrap().as_slice(), ["b"]);
}

#[test]
fn errors_roll_back_and_report() {
    let (mut s, _) = session();
    assert!(matches!(
        s.run("nope", &Value::Null),
        Err(EngineError::UnknownCommand(_))
    ));
    assert!(matches!(
        s.run("view.book", &json!({"id": "zzz"})),
        Err(EngineError::NotFound(_))
    ));
    assert!(matches!(
        s.run("reader.next", &Value::Null),
        Err(EngineError::Disabled(_))
    ));
    assert!(matches!(
        s.run("filter.years", &json!({"min": 2020, "max": 1990})),
        Err(EngineError::BadParams(_))
    ));
    assert!(s.status.is_some());
    assert_eq!(s.route(), &Route::Library);
}

#[test]
fn null_backend_works() {
    let mut s = Session::new(catalog(), vec![], Box::new(NullBackend));
    assert!(s.run("book.open_external", &json!({"id": "b"})).is_err());
}

#[test]
fn editing_metadata_changes_search_persists_and_undoes() {
    let (mut s, rec) = session();
    s.run(
        "book.edit",
        &json!({"id": "a", "fields": {"title": "Ржавчина в деле", "language": "ru",
            "authors": ["Тим Макнамара"], "year": null}}),
    )
    .unwrap();
    let b = s.book("a").unwrap();
    assert_eq!(b.title, "Ржавчина в деле");
    assert_eq!(b.language, "ru");
    assert_eq!(b.year, None);
    s.run("search.set", &json!({"text": "ржавчина"})).unwrap();
    assert_eq!(s.result_count(), 1);
    assert_eq!(rec.overrides.lock().unwrap().len(), 4);
    assert_eq!(s.edited_fields("a").len(), 4);

    s.run("search.set", &json!({"text": ""})).unwrap();
    s.run("edit.undo", &Value::Null).unwrap();
    s.run("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.book("a").unwrap().title, "Rust in Action");
    assert!(s.edited_fields("a").is_empty());
    // Undo deleted the four stored corrections.
    assert_eq!(
        rec.overrides
            .lock()
            .unwrap()
            .iter()
            .filter(|o| o.2.is_none())
            .count(),
        4
    );
}

#[test]
fn marks_filter_and_reviewing_survives_reset() {
    let (mut s, rec) = session();
    // Both books lack authors and a description.
    s.run("filter.mark.toggle", &json!({"mark": "incomplete"}))
        .unwrap();
    assert_eq!(s.result_count(), 2);
    s.run(
        "book.edit",
        &json!({"id": "a", "fields": {"authors": "Tim McNamara", "description": "Rust."}}),
    )
    .unwrap();
    assert_eq!(s.result_count(), 1);
    assert!(s.book("a").unwrap().edited);
    s.run("book.review", &json!({"id": "b"})).unwrap();
    assert_eq!(s.result_count(), 0);
    assert!(s.book("b").unwrap().reviewed && !s.book("b").unwrap().edited);
    assert!(s.edited_fields("b").is_empty());
    assert!(rec.overrides.lock().unwrap().contains(&(
        "b".into(),
        Field::Reviewed,
        Some("1".into())
    )));
    // Resetting the corrections keeps the reviewed mark; only corrections are reset.
    assert!(s.run("book.edit.reset", &json!({"id": "b"})).is_err());
    s.run("book.edit", &json!({"id": "b", "fields": {"year": 1869}}))
        .unwrap();
    s.run("book.edit.reset", &json!({"id": "b"})).unwrap();
    assert!(s.book("b").unwrap().reviewed);
    s.run(
        "book.edit",
        &json!({"id": "b", "fields": {"reviewed": false}}),
    )
    .unwrap();
    assert_eq!(s.result_count(), 1);
    s.run("filter.mark.toggle", &json!({"mark": "incomplete"}))
        .unwrap();
    s.run("filter.mark.toggle", &json!({"mark": "edited"}))
        .unwrap();
    assert_eq!(s.results().next().map(|b| b.id.as_str()), Some("a"));
    s.run("filter.clear", &json!({})).unwrap();
    s.run("filter.missing.toggle", &json!({"field": "year"}))
        .unwrap();
    assert_eq!(s.results().next().map(|b| b.id.as_str()), Some("b"));
    assert_eq!(s.snapshot()["incomplete_books"], 1);
    for bad in [
        ("filter.mark.toggle", json!({"mark": "nope"})),
        ("filter.missing.toggle", json!({"field": "reviewed"})),
        (
            "book.edit",
            json!({"id": "a", "fields": {"reviewed": "yes"}}),
        ),
        ("book.review", json!({"id": "zzz"})),
    ] {
        assert!(s.run(bad.0, &bad.1).is_err(), "{bad:?}");
    }
}

#[test]
fn deleting_a_book_needs_confirmation_and_forgets_it() {
    let (mut s, rec) = session();
    s.run("view.book", &json!({"id": "a"})).unwrap();
    s.run("book.show_in_folder", &json!({})).unwrap();
    assert_eq!(*rec.shown.lock().unwrap(), vec!["a"]);
    assert!(s.run("book.delete", &json!({"id": "a"})).is_err());
    assert!(
        s.run("book.delete", &json!({"id": "a", "confirm": false}))
            .is_err()
    );
    assert!(s.book("a").is_some());
    // A failed trash keeps the book.
    assert!(
        s.run("book.delete", &json!({"id": "b", "confirm": true}))
            .is_err()
    );
    assert!(s.book("b").is_some());
    s.run("book.delete", &json!({"confirm": true})).unwrap();
    assert_eq!(*rec.removed.lock().unwrap(), vec!["a"]);
    assert!(s.book("a").is_none());
    assert_eq!(s.route(), &Route::Library);
    assert_eq!(s.result_count(), 1);
    assert_eq!(s.book("b").map(|b| b.id.as_str()), Some("b"));
    // Undo does not bring it back.
    let _ = s.run("edit.undo", &Value::Null);
    assert!(s.book("a").is_none());
    assert!(
        s.run("book.delete", &json!({"id": "a", "confirm": true}))
            .is_err()
    );
}

#[test]
fn covers_change_from_one_source_and_bump_the_revision() {
    use crate::CoverSource;
    let (mut s, rec) = session();
    assert_eq!(s.cover_rev("a"), 0);
    s.run("book.cover.set", &json!({"id": "a", "page": 3}))
        .unwrap();
    s.run(
        "book.cover.set",
        &json!({"id": "b", "path": " /tmp/c.png "}),
    )
    .unwrap();
    s.run("book.cover.set", &json!({"id": "a", "catalog": true}))
        .unwrap();
    assert_eq!(s.cover_rev("a"), 2);
    assert_eq!(
        *rec.covers.lock().unwrap(),
        vec![
            ("a".into(), CoverSource::Page(3)),
            ("b".into(), CoverSource::File("/tmp/c.png".into())),
            ("a".into(), CoverSource::Catalog),
        ]
    );
    for bad in [
        json!({"id": "a"}),
        json!({"id": "a", "page": 0}),
        json!({"id": "a", "page": 11}),
        json!({"id": "b", "page": 1}),
        json!({"id": "a", "page": 1, "path": "/x.png"}),
        json!({"id": "a", "path": "  "}),
    ] {
        assert!(s.run("book.cover.set", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.cover_rev("a"), 2);
}

#[test]
fn interface_language_changes_and_is_validated() {
    let (mut s, _) = session();
    assert_eq!(s.language(), "ro");
    s.run("view.language", &json!({"language": "ru"})).unwrap();
    assert_eq!(s.language(), "ru");
    assert!(s.run("view.language", &json!({"language": "de"})).is_err());
    assert!(s.run("view.language", &json!({})).is_err());
    assert!(s.run("edit.undo", &Value::Null).is_err());
    assert_eq!(s.snapshot()["language"], "ru");
    let (s, _) = session();
    assert_eq!(s.with_language("xx").language(), "ro");
}

#[test]
fn theme_switches_and_is_not_undone() {
    let (mut s, _) = session();
    assert!(!s.dark());
    s.run("view.theme", &Value::Null).unwrap();
    assert!(s.dark());
    assert!(s.run("edit.undo", &Value::Null).is_err());
    s.run("view.theme", &json!({"theme": "light"})).unwrap();
    assert!(!s.dark());
    assert!(s.run("view.theme", &json!({"theme": "blue"})).is_err());
    assert_eq!(s.snapshot()["theme"], "light");
}

#[test]
fn library_folders_are_added_filtered_and_removed() {
    let (s, _) = session();
    let mut s = s.with_sources(vec!["/backup/books"]);
    s.run(
        "book.edit",
        &json!({"id": "a", "fields": {"title": "Corrected"}}),
    )
    .unwrap();
    assert!(s.run("source.add", &json!({"path": "/nowhere"})).is_err());
    let r = s.run("source.add", &json!({"path": "/disk2/"})).unwrap();
    assert_eq!(r["books"], 1);
    let paths = |s: &Session| -> Vec<String> {
        s.sources().iter().map(|e| e.source.path.clone()).collect()
    };
    assert_eq!(paths(&s), ["/backup/books", "/disk2"]);
    assert_eq!(s.result_count(), 3);
    // Corrections still apply after the catalog is rebuilt.
    assert_eq!(s.book("a").unwrap().title, "Corrected");
    assert!(s.run("source.add", &json!({"path": "/disk2"})).is_err());
    s.run("filter.source.toggle", &json!({"path": "/disk2"}))
        .unwrap();
    assert_eq!(s.results().next().map(|b| b.id.as_str()), Some("c"));
    s.run("view.book", &json!({"id": "c"})).unwrap();
    s.run("source.remove", &json!({"path": "/disk2"})).unwrap();
    assert!(s.book("c").is_none());
    assert_eq!(s.route(), &Route::Library);
    assert!(s.query().filters.sources.is_empty());
    assert_eq!(s.result_count(), 2);
    assert!(s.run("source.remove", &json!({"path": "/disk2"})).is_err());
    assert!(s.run("source.add", &json!({"path": "  "})).is_err());
}

#[test]
fn a_folder_without_a_catalog_gets_one_built() {
    let r = Recorder::default();
    let mut s = Session::new(catalog(), vec![], Box::new(r.clone()));
    let out = s.run("source.add", &json!({"path": "/fresh"})).unwrap();
    assert_eq!(out["indexing"], true);
    assert_eq!(*r.indexed.lock().unwrap(), ["/fresh"]);
    assert_eq!(r.sources.lock().unwrap().len(), 1);
    let entry = |s: &Session| s.source("/fresh").cloned().unwrap();
    assert!(entry(&s).loaded() && entry(&s).indexing.is_some());

    let p = crate::IndexProgress { done: 3, total: 10 };
    r.set_index(crate::IndexState::Running(p));
    assert!(s.poll_indexing());
    assert_eq!(entry(&s).indexing, Some(p));
    assert!(s.book("f").is_none());

    *r.built.lock().unwrap() = true;
    r.set_index(crate::IndexState::Done);
    assert!(!s.poll_indexing());
    assert_eq!(entry(&s).indexing, None);
    assert_eq!(s.book("f").map(|b| b.title.as_str()), Some("Fresh Book"));
    assert_eq!(s.source_books("/fresh"), 1);

    // Updating it again: a failed build keeps the books it had.
    s.run("source.reindex", &json!({"path": "/fresh"})).unwrap();
    assert_eq!(r.indexed.lock().unwrap().len(), 2);
    r.set_index(crate::IndexState::Failed("no python3".into()));
    assert!(!s.poll_indexing());
    assert!(s.book("f").is_some());
    assert!(
        s.status
            .as_deref()
            .is_some_and(|m| m.contains("no python3"))
    );
    assert!(
        s.run("source.reindex", &json!({"path": "/elsewhere"}))
            .is_err()
    );
}

#[test]
fn a_book_file_outside_the_library_is_read_without_joining_it() {
    let (mut s, r) = session();
    let results = s.result_count();
    let out = s
        .run("file.open", &json!({"path": "/home/u/Dune_1965.epub"}))
        .unwrap();
    assert_eq!(
        (out["in_library"].clone(), out["reader"].clone()),
        (json!(false), json!("text"))
    );
    assert_eq!(s.route(), &Route::Reader { id: "dune".into() });
    assert_eq!(s.current_book().map(|b| b.title.as_str()), Some("Dune"));
    assert!(s.is_outside("dune"));
    assert_eq!(s.result_count(), results, "not in the search results");
    assert_eq!(*r.files.lock().unwrap(), ["dune"]);
    assert!(
        r.covers.lock().unwrap().is_empty(),
        "a text book keeps its own cover"
    );

    // A PDF gets its first page as cover and shows up in Continue reading.
    let out = s
        .run("file.open", &json!({"path": "/tmp/Scan.pdf"}))
        .unwrap();
    assert_eq!(out["page"], 1);
    assert_eq!(*r.files.lock().unwrap(), ["scan", "dune"]);
    assert_eq!(
        r.covers.lock().unwrap().clone(),
        [("scan".to_owned(), crate::CoverSource::Page(1))]
    );
    s.run("reader.goto", &json!({"page": 7})).unwrap();
    assert_eq!(s.progress("scan").and_then(|p| p.page), Some(7));
    assert!(s.continue_reading(10).iter().any(|b| b.id == "scan"));
    // Opened again: no new cover, back on top.
    s.run("file.open", &json!({"path": "/home/u/Dune_1965.epub"}))
        .unwrap();
    assert_eq!(r.covers.lock().unwrap().len(), 1);
    assert_eq!(s.files().first().map(|b| b.id.as_str()), Some("dune"));

    // Library actions are refused on it.
    for (cmd, params) in [
        ("book.edit", json!({"id": "dune", "fields": {"title": "X"}})),
        ("view.edit", json!({"id": "dune"})),
        ("book.delete", json!({"id": "dune", "confirm": true})),
    ] {
        assert!(
            matches!(s.run(cmd, &params), Err(EngineError::Disabled(_))),
            "{cmd}"
        );
    }

    // A file that is a library book opens as that book.
    let out = s
        .run("file.open", &json!({"path": "/elsewhere/a.pdf"}))
        .unwrap();
    assert_eq!(out["in_library"], true);
    assert_eq!(
        s.current_book().map(|b| b.title.as_str()),
        Some("Rust in Action")
    );
    assert_eq!(s.files().len(), 2);

    s.run("book.read", &json!({"id": "dune"})).unwrap();
    s.run("file.forget", &json!({"id": "dune"})).unwrap();
    assert_eq!(s.route(), &Route::Library);
    assert!(s.book("dune").is_none());
    assert_eq!(*r.files.lock().unwrap(), ["scan"]);
    assert!(s.run("file.forget", &json!({"id": "dune"})).is_err());
    assert!(
        s.run("file.open", &json!({"path": "/x/notes.txt"}))
            .is_err()
    );
    assert!(s.run("file.open", &json!({})).is_err());
}

#[test]
fn a_failed_first_build_leaves_the_folder_unavailable() {
    let r = Recorder::default();
    let mut s = Session::new(catalog(), vec![], Box::new(r.clone()));
    s.run("source.add", &json!({"path": "/broken"})).unwrap();
    r.set_index(crate::IndexState::Failed("build-catalog not found".into()));
    s.poll_indexing();
    let entry = s.source("/broken").cloned().unwrap();
    assert!(
        matches!(entry.status, crate::SourceStatus::Unavailable(ref m) if m.contains("not found"))
    );
    assert_eq!(entry.indexing, None);
    // Trying it again starts another build.
    s.run(
        "source.enable",
        &json!({"path": "/broken", "enabled": true}),
    )
    .unwrap();
    assert_eq!(r.indexed.lock().unwrap().len(), 2);
    assert!(s.source("/broken").is_some_and(|e| e.indexing.is_some()));
}

#[test]
fn library_folders_are_disabled_renamed_and_moved() {
    let r = Recorder::default();
    let mut books = catalog();
    for b in &mut books.books {
        b.source = "/backup/books".into();
    }
    let mut s =
        Session::new(books, vec![], Box::new(r.clone())).with_sources(vec!["/backup/books"]);
    let titles = |s: &Session| -> Vec<String> {
        let mut t: Vec<String> = s.catalog().books.iter().map(|b| b.title.clone()).collect();
        t.sort();
        t
    };
    s.run("source.add", &json!({"path": "/disk2"})).unwrap();
    assert_eq!(titles(&s), ["Clean Code", "Rust in Action", "Война и мир"]);
    s.run("filter.source.toggle", &json!({"path": "/backup/books"}))
        .unwrap();
    s.run("view.book", &json!({"id": "b"})).unwrap();

    // Off: its books go, and the copy of "a" in the other folder shows instead.
    s.run(
        "source.enable",
        &json!({"path": "/backup/books", "enabled": false}),
    )
    .unwrap();
    assert_eq!(titles(&s), ["Clean Code", "Rust in Action (copy)"]);
    assert_eq!(s.route(), &Route::Library);
    assert!(s.query().filters.sources.is_empty());
    assert_eq!(s.sources().len(), 2);
    assert_eq!(
        s.sources().first().map(|e| e.status.clone()),
        Some(crate::SourceStatus::Disabled)
    );
    // Only the added folder was saved by this session; the switch saved the other.
    let saved = r.sources.lock().unwrap().clone();
    assert!(
        saved
            .iter()
            .any(|s| s.path == "/backup/books" && !s.enabled)
    );

    // On again, but the folder cannot be opened now: it stays on, unavailable.
    let e = s.run(
        "source.enable",
        &json!({"path": "/backup/books", "enabled": true}),
    );
    assert!(e.is_err());
    assert!(matches!(
        s.sources().first().map(|e| &e.status),
        Some(crate::SourceStatus::Unavailable(_))
    ));
    assert!(s.sources().first().is_some_and(|e| e.source.enabled));

    // Moved to a folder that opens: loaded again, at its old place in the list.
    assert!(
        s.run(
            "source.edit",
            &json!({"path": "/backup/books", "new_path": "/disk2"})
        )
        .is_err(),
        "already in the library"
    );
    assert!(
        s.run(
            "source.edit",
            &json!({"path": "/backup/books", "new_path": "/nowhere"})
        )
        .is_err()
    );
    s.run(
        "source.edit",
        &json!({"path": "/backup/books", "new_path": "/disk3/", "name": "  Old disk "}),
    )
    .unwrap();
    let first = s.sources().first().cloned().unwrap();
    assert_eq!(first.source.path, "/disk3");
    assert_eq!(first.source.display_name(), "Old disk");
    assert!(first.loaded());
    assert_eq!(s.source_name("/disk3"), "Old disk");
    assert_eq!(s.source_name("/disk2"), "disk2");
    assert_eq!(s.source_books("/disk3"), 2);
    let saved = r.sources.lock().unwrap().clone();
    assert!(
        saved
            .iter()
            .any(|s| s.path == "/disk3" && s.name == "Old disk")
    );
    assert!(!saved.iter().any(|s| s.path == "/backup/books"));

    assert!(
        s.run(
            "source.edit",
            &json!({"path": "/disk2", "name": "x".repeat(201)})
        )
        .is_err()
    );
    assert!(s.run("source.enable", &json!({"path": "/disk2"})).is_err());
    assert!(
        s.run(
            "source.enable",
            &json!({"path": "/nowhere", "enabled": true})
        )
        .is_err()
    );
    s.run("source.remove", &json!({"path": "/disk3"})).unwrap();
    assert_eq!(r.sources.lock().unwrap().len(), 1);
    assert_eq!(s.sources().len(), 1);
}

#[test]
fn series_filter_and_hand_corrections() {
    let mut books = catalog();
    for (b, volume) in books.books.iter_mut().zip([2, 1]) {
        b.series = Some("Бард".into());
        b.series_index = Some(volume);
    }
    let first = books.books.first().map(|b| b.id.clone()).unwrap();
    let mut s = Session::new(books, vec![], Box::new(Recorder::default()));
    assert!(
        s.run("filter.series.toggle", &json!({"series": "Нет"}))
            .is_err()
    );
    s.run("filter.series.toggle", &json!({"series": "Бард"}))
        .unwrap();
    assert_eq!(s.facets().series, vec![("Бард".to_owned(), 2)]);
    // Volume 1 first.
    assert_ne!(
        s.results().next().map(|b| b.id.clone()),
        Some(first.clone())
    );
    let second = s.results().next().map(|b| b.id.clone()).unwrap();
    assert_eq!(
        s.next_volume(&second).map(|b| b.id.clone()),
        Some(first.clone())
    );
    assert!(s.next_volume(&first).is_none());
    // Series and volume can be corrected by hand; a bad volume is refused.
    s.run(
        "book.edit",
        &json!({"id": first, "fields": {"series": "Бард II", "series_index": "7"}}),
    )
    .unwrap();
    let b = s.book(&first).unwrap();
    assert_eq!(
        (b.series.as_deref(), b.series_index),
        (Some("Бард II"), Some(7))
    );
    assert!(
        s.run(
            "book.edit",
            &json!({"id": first, "fields": {"series_index": 10_000}})
        )
        .is_err()
    );
}

#[test]
fn subfolders_filter_and_follow_their_library_folder() {
    let r = Recorder::default();
    let mut books = catalog();
    for (b, path) in books
        .books
        .iter_mut()
        .zip(["Rust/a.pdf", "Tolstoy/War/b.fb2"])
    {
        b.source = "/backup/books".into();
        b.path = path.into();
    }
    let mut s =
        Session::new(books, vec![], Box::new(r.clone())).with_sources(vec!["/backup/books"]);
    assert_eq!(s.folder_label("/backup/books/Tolstoy/War"), "Tolstoy/War");
    assert!(
        s.run(
            "filter.folder.toggle",
            &json!({"path": "/backup/books/Nope"})
        )
        .is_err()
    );
    s.run(
        "filter.folder.toggle",
        &json!({"path": "/backup/books/Tolstoy/War"}),
    )
    .unwrap();
    assert_eq!(s.result_count(), 1);
    assert_eq!(s.facets().folders.len(), 2);
    // A moved library folder takes its subfolder filters along.
    s.run(
        "source.edit",
        &json!({"path": "/backup/books", "new_path": "/disk3"}),
    )
    .unwrap();
    assert!(s.query().filters.folders.contains("/disk3/Tolstoy/War"));
    // A removed one drops them.
    s.run("source.remove", &json!({"path": "/disk3"})).unwrap();
    assert!(s.query().filters.folders.is_empty());
}

#[test]
fn duplicates_are_found_corrected_and_dismissed() {
    let mut books = catalog();
    for b in &mut books.books {
        b.isbn = Some("978-0-306-40615-7".into());
    }
    let r = Recorder::default();
    let mut s = Session::new(books, vec![], Box::new(r.clone()));
    assert_eq!(
        s.book("a").map(|b| b.duplicates.clone()),
        Some(vec!["b".into()])
    );
    s.run("filter.mark.toggle", &json!({"mark": "duplicate"}))
        .unwrap();
    assert_eq!(s.result_count(), 2);
    // A corrected ISBN separates them.
    s.run(
        "book.edit",
        &json!({"id": "b", "fields": {"isbn": "978-1-4028-9462-6"}}),
    )
    .unwrap();
    assert_eq!(s.result_count(), 0);
    s.run("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.result_count(), 2);
    // "Not a duplicate" is a mark, saved, undoable, and not a correction.
    s.run("book.not_duplicate", &json!({"id": "a"})).unwrap();
    assert_eq!(s.result_count(), 0);
    assert!(s.book("a").is_some_and(|b| b.not_duplicate && !b.edited));
    assert!(s.edited_fields("a").is_empty());
    assert!(
        r.overrides
            .lock()
            .unwrap()
            .iter()
            .any(|(id, f, v)| id == "a" && *f == Field::NotDuplicate && v.as_deref() == Some("1"))
    );
    s.run("edit.undo", &Value::Null).unwrap();
    assert!(s.book("a").is_some_and(|b| !b.not_duplicate));
    assert_eq!(s.result_count(), 2);
}

#[test]
fn selected_books_are_deleted_together() {
    let (mut s, r) = session();
    assert!(
        s.run("books.delete", &json!({"confirm": true})).is_err(),
        "nothing selected"
    );
    s.run("select.toggle", &json!({"id": "a"})).unwrap();
    s.run("select.toggle", &json!({"id": "a"})).unwrap();
    assert!(s.selected().is_empty());
    assert!(s.run("select.toggle", &json!({"id": "zzz"})).is_err());
    s.run("select.all", &Value::Null).unwrap();
    assert_eq!(s.selected().len(), 2);
    assert!(
        s.run("edit.undo", &Value::Null).is_err(),
        "selection is not undone"
    );
    assert!(
        s.run("books.delete", &Value::Null).is_err(),
        "needs confirm"
    );
    // "b" cannot go to the trash (the recorder refuses it): "a" goes, "b" stays selected.
    let out = s.run("books.delete", &json!({"confirm": true})).unwrap();
    assert_eq!(out["removed"], 1);
    assert_eq!(out["failed"].as_array().map(Vec::len), Some(1));
    assert!(s.book("a").is_none());
    assert_eq!(r.removed.lock().unwrap().as_slice(), ["a"]);
    assert_eq!(s.selected().iter().collect::<Vec<_>>(), ["b"]);
    assert!(s.run("books.delete", &json!({"confirm": true})).is_err());
    s.run("select.clear", &Value::Null).unwrap();
    assert!(s.run("select.clear", &Value::Null).is_err());
    assert!(
        s.run("books.delete", &json!({"ids": "a", "confirm": true}))
            .is_err()
    );
}

#[test]
fn reader_zoom_steps_clamps_and_is_not_undone() {
    let (mut s, _) = session();
    assert!(
        s.run("reader.zoom_in", &Value::Null).is_err(),
        "only in the reader"
    );
    s.run("book.read", &json!({"id": "a"})).unwrap();
    s.run("reader.zoom_in", &Value::Null).unwrap();
    assert!((s.zoom() - 1.1).abs() < 1e-6);
    s.run("reader.zoom_out", &Value::Null).unwrap();
    s.run("reader.zoom_out", &Value::Null).unwrap();
    assert!((s.zoom() - 0.9).abs() < 1e-6);
    s.run("reader.zoom", &json!({"zoom": 1.3})).unwrap();
    s.run("reader.zoom_in", &Value::Null).unwrap();
    assert!((s.zoom() - 1.5).abs() < 1e-6);
    s.run("reader.zoom", &json!({"zoom": 99})).unwrap();
    assert!((s.zoom() - 4.0).abs() < 1e-6);
    assert!(s.run("reader.zoom_in", &Value::Null).is_err());
    assert!(s.run("reader.zoom", &json!({"zoom": "big"})).is_err());
    assert!(s.run("reader.zoom", &json!({})).is_err());
    s.run("reader.zoom_reset", &Value::Null).unwrap();
    assert!((s.zoom() - 1.0).abs() < 1e-6);
    assert_eq!(s.snapshot()["zoom"], 1.0);
    // Zoom is a preference: undo goes back over the reading, not the zoom.
    s.run("reader.zoom", &json!({"zoom": 2})).unwrap();
    s.run("edit.undo", &Value::Null).unwrap();
    assert!((s.zoom() - 2.0).abs() < 1e-6);
    let s = Session::new(catalog(), vec![], Box::new(NullBackend)).with_zoom(f32::NAN);
    assert!((s.zoom() - 1.0).abs() < 1e-6);
}

#[test]
fn settings_window_opens_and_closes_without_undo() {
    let (mut s, _) = session();
    s.run("view.settings", &Value::Null).unwrap();
    assert!(s.settings_open());
    assert_eq!(s.snapshot()["settings"], true);
    assert!(s.run("edit.undo", &Value::Null).is_err());
    s.run("view.settings", &json!({"open": false})).unwrap();
    assert!(!s.settings_open());
    assert_eq!(s.settings_tab(), "general");
    s.run("view.settings", &json!({"tab": "shortcuts"}))
        .unwrap();
    assert!(s.settings_open());
    assert_eq!(s.settings_tab(), "shortcuts");
    assert!(s.run("view.settings", &json!({"tab": "colours"})).is_err());
    assert_eq!(s.settings_tab(), "shortcuts");
}

#[test]
fn edits_are_validated_and_reset() {
    let (mut s, _) = session();
    let bad = [
        json!({"id": "a", "fields": {"nope": "x"}}),
        json!({"id": "a", "fields": {"year": "soon"}}),
        json!({"id": "a", "fields": {"year": 99999}}),
        json!({"id": "a", "fields": {"language": "Russian"}}),
        json!({"id": "a", "fields": {"categories": ["no-such-category"]}}),
        json!({"id": "a", "fields": {"title": "   "}}),
        json!({"id": "a", "fields": {}}),
        json!({"id": "a", "fields": {"title": "x".repeat(20_000)}}),
        json!({"id": "zzz", "fields": {"title": "x"}}),
    ];
    for p in bad {
        assert!(s.run("book.edit", &p).is_err(), "{p}");
    }
    // Comma-separated lists are accepted; a value equal to the catalog's is no override.
    s.run(
        "book.edit",
        &json!({"id": "a", "fields": {"tags": "rust, systems", "title": "Rust in Action"}}),
    )
    .unwrap();
    assert_eq!(s.book("a").unwrap().tags, vec!["rust", "systems"]);
    assert_eq!(s.edited_fields("a"), vec![Field::Tags]);
    s.run(
        "book.edit",
        &json!({"id": "a", "fields": {"categories": ["fiction", "go-rust"]}}),
    )
    .unwrap();
    s.run("book.edit.reset", &json!({"id": "a", "field": "tags"}))
        .unwrap();
    assert_eq!(s.edited_fields("a"), vec![Field::Categories]);
    s.run("book.edit.reset", &json!({"id": "a"})).unwrap();
    assert!(s.edited_fields("a").is_empty());
    assert!(s.run("book.edit.reset", &json!({"id": "a"})).is_err());
}

#[test]
fn stored_overrides_load_and_save_errors_reach_the_status() {
    let rec = Recorder {
        fail_overrides: true,
        ..Recorder::default()
    };
    let stored = vec![Override {
        book_id: "b".into(),
        field: Field::Year,
        value: "1869".into(),
    }];
    let mut s = Session::new(catalog(), vec![], Box::new(rec)).with_overrides(stored);
    assert_eq!(s.book("b").unwrap().year, Some(1869));
    s.run(
        "book.edit",
        &json!({"id": "b", "fields": {"title": "Война и мир (1869)"}}),
    )
    .unwrap();
    assert!(
        s.status
            .as_deref()
            .unwrap_or_default()
            .contains("disk full")
    );
}

proptest! {
    #[test]
    fn arbitrary_commands_never_panic(
        cmds in proptest::collection::vec((0usize..64, any::<i64>(), ".{0,20}", any::<bool>()), 0..40)
    ) {
        let (mut s, _) = session();
        let reg = registry();
        for (i, n, text, b) in cmds {
            let Some(spec) = reg.get(i % reg.len()) else { continue };
            let params = json!({
                "text": text, "id": if b { "a" } else { "b" }, "slug": text, "language": text,
                "format": text, "min": n, "max": n, "page": n, "sort": text, "on": b,
                "block": n, "total": n, "field": text, "fields": { "title": text, "year": n, "authors": [text], "language": text, "tags": text },
            });
            let _ = s.run(spec.id, &params);
        }
    }
}

#[test]
fn library_tabs_switch_without_undo_and_search_shows_the_library() {
    let (mut s, _) = session();
    assert_eq!(s.library_tab(), "library");
    s.run("view.book", &json!({"id": "a"})).unwrap();
    s.run("view.tab.reading", &Value::Null).unwrap();
    assert_eq!(s.route(), &Route::Library);
    assert_eq!(s.library_tab(), "reading");
    assert_eq!(s.snapshot()["library_tab"], "reading");
    s.run("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.library_tab(), "reading");
    s.run("search.set", &json!({"text": ""})).unwrap();
    assert_eq!(s.library_tab(), "reading");
    s.run("search.set", &json!({"text": "rust"})).unwrap();
    assert_eq!(s.library_tab(), "library");
    s.run("view.tab.reading", &Value::Null).unwrap();
    s.run("view.tab.library", &Value::Null).unwrap();
    assert_eq!(s.library_tab(), "library");
}

#[test]
fn results_come_a_page_at_a_time_for_the_control_channel() {
    let (s, _) = session();
    let all = s.results_page(0, 100);
    let total = all["total"].as_u64().unwrap() as usize;
    assert!(total >= 2);
    let books = all["books"].as_array().unwrap();
    assert_eq!(books.len(), total);
    let first = books.first().unwrap();
    assert_eq!(first["id"], json!(s.results().next().unwrap().id));
    assert!(first["title"].is_string() && first["file"].is_string());
    assert!(first.get("description").is_none());
    let page = s.results_page(1, 1);
    assert_eq!(page["books"].as_array().unwrap().len(), 1);
    assert_eq!(page["books"][0]["id"], books[1]["id"]);
    assert_eq!(s.results_page(total, 10)["books"], json!([]));
}

#[test]
fn statuses_favourites_and_ratings_filter_persist_and_undo() {
    let (mut s, rec) = session();
    // Opening a book with no state makes it one being read; a set state stays.
    s.run("book.read", &json!({"id": "a"})).unwrap();
    assert_eq!(s.book("a").unwrap().status, Some(ReadState::Reading));
    assert!(rec.overrides.lock().unwrap().contains(&(
        "a".into(),
        Field::Status,
        Some("reading".into())
    )));
    s.run("book.status", &json!({"id": "b", "state": "to_read"}))
        .unwrap();
    s.run("book.read", &json!({"id": "b"})).unwrap();
    assert_eq!(s.book("b").unwrap().status, Some(ReadState::ToRead));
    // Not a correction: the book is not "edited" and a reset keeps it.
    assert!(!s.book("b").unwrap().edited && s.edited_fields("b").is_empty());
    assert_eq!(
        s.facets().states,
        vec![(ReadState::ToRead, 1), (ReadState::Reading, 1)]
    );
    s.run("filter.status.toggle", &json!({"state": "reading"}))
        .unwrap();
    assert_eq!(s.results().next().map(|b| b.id.as_str()), Some("a"));
    assert_eq!(s.result_count(), 1);
    s.run("filter.clear", &json!({})).unwrap();

    s.run("book.favorite", &json!({"id": "b"})).unwrap();
    s.run("book.rating", &json!({"id": "b", "stars": 4}))
        .unwrap();
    let b = s.book("b").unwrap();
    assert!(b.favorite && b.rating == 4);
    s.run("filter.mark.toggle", &json!({"mark": "favorite"}))
        .unwrap();
    assert_eq!(s.result_count(), 1);
    s.run("edit.undo", &Value::Null).unwrap();
    s.run("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.book("b").unwrap().rating, 0);
    assert!(
        rec.overrides
            .lock()
            .unwrap()
            .contains(&("b".into(), Field::Rating, None))
    );

    // In bulk over the selection, undone in one step.
    s.run("select.all", &Value::Null).unwrap();
    let out = s.run("books.status", &json!({"state": "read"})).unwrap();
    assert_eq!(out["changed"], 2);
    assert!(
        ["a", "b"]
            .iter()
            .all(|id| s.book(id).unwrap().status == Some(ReadState::Read))
    );
    s.run("edit.undo", &Value::Null).unwrap();
    assert_eq!(s.book("a").unwrap().status, Some(ReadState::Reading));
    s.run("books.favorite", &json!({"ids": ["a"], "on": true}))
        .unwrap();
    assert!(s.book("a").unwrap().favorite);
    s.run("books.status", &json!({"ids": ["a"], "state": null}))
        .unwrap();
    assert_eq!(s.book("a").unwrap().status, None);
    s.run(
        "book.edit",
        &json!({"id": "a", "fields": {"status": "abandoned", "rating": 2}}),
    )
    .unwrap();
    assert_eq!(s.book("a").unwrap().status, Some(ReadState::Abandoned));
    assert_eq!(s.book("a").unwrap().rating, 2);

    for bad in [
        ("book.status", json!({"id": "a", "state": "someday"})),
        ("book.status", json!({"id": "a"})),
        ("book.rating", json!({"id": "a", "stars": 6})),
        ("book.rating", json!({"id": "a", "stars": -1})),
        ("books.status", json!({"ids": ["zzz"], "state": "read"})),
        ("books.favorite", json!({"ids": ["a"]})),
        ("filter.status.toggle", json!({"state": "nope"})),
        (
            "book.edit",
            json!({"id": "a", "fields": {"rating": "five"}}),
        ),
    ] {
        assert!(s.run(bad.0, &bad.1).is_err(), "{bad:?}");
    }
}
