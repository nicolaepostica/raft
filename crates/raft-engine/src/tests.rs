use std::sync::{Arc, Mutex};

use proptest::prelude::*;
use raft_doc::{Book, Catalog, Category, Format, Progress, Route};
use serde_json::{Value, json};

use crate::{Backend, EngineError, NullBackend, Session, registry};

#[derive(Default, Clone)]
struct Recorder {
    saved: Arc<Mutex<Vec<Progress>>>,
    opened: Arc<Mutex<Vec<String>>>,
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
        .filter_map(|c| c.shortcut)
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
fn non_paged_books_open_externally() {
    let (mut s, rec) = session();
    let r = s.run("book.read", &json!({"id": "b"})).unwrap();
    assert_eq!(r["reader"], "external");
    assert_eq!(rec.opened.lock().unwrap().as_slice(), ["b"]);
    assert!(s.progress("b").is_some());
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
    assert!(s.run("book.read", &json!({"id": "b"})).is_err());
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
            });
            let _ = s.run(spec.id, &params);
        }
    }
}
