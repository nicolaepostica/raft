use std::sync::{Arc, Mutex};

use proptest::prelude::*;
use raft_doc::edits::{Field, Override};
use raft_doc::{Book, Catalog, Category, Format, Progress, Route};
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
