use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use raft_doc::{Book, Catalog, Category, Format, Route};
use raft_engine::{NullBackend, Session};

use crate::{NoFiles, RaftUi};

fn app() -> RaftUi {
    let catalog = Catalog {
        books: vec![
            Book {
                id: "a".into(),
                title: "Rust in Action".into(),
                authors: vec!["Tim McNamara".into()],
                format: Format::Pdf,
                pages: Some(10),
                categories: vec!["go-rust".into()],
                language: "en".into(),
                year: Some(2021),
                description: "O carte despre Rust.".into(),
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
    };
    RaftUi::new(
        Session::new(catalog, vec![], Box::new(NullBackend)),
        Box::new(NoFiles),
    )
}

fn harness() -> Harness<'static, RaftUi> {
    Harness::builder()
        .with_size(egui::vec2(1100.0, 750.0))
        .build_ui_state(|ui, app: &mut RaftUi| app.show(ui), app())
}

#[test]
fn library_shows_books_and_filters() {
    let mut h = harness();
    h.run();
    h.get_by_label("2 din 2 cărți");
    h.get_by_label("Русский  (1)").click();
    h.run();
    h.get_by_label("1 din 2 cărți");
    assert_eq!(h.state().session.query().filters.languages.len(), 1);
}

#[test]
fn clicking_a_card_opens_the_book_page() {
    let mut h = harness();
    h.run();
    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("O carte despre Rust.");
    h.get_by_label("Citește").click();
    h.run();
    assert_eq!(h.state().session.route(), &Route::Reader { id: "a".into() });
}

#[test]
fn reader_without_renderer_shows_error_and_navigates() {
    let mut h = harness();
    h.state_mut()
        .run("book.read", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("din 10");
    h.get_by_label("Următoarea").click();
    h.run();
    assert_eq!(
        h.state().session.progress("a").and_then(|p| p.page),
        Some(2)
    );
    h.get_by_label_contains("Pagina nu a putut fi afișată");
}

proptest::proptest! {
    #[test]
    fn decode_image_never_panics(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4096)) {
        let _ = crate::decode_image(&bytes);
    }

    #[test]
    fn decode_image_never_panics_on_png_like_input(tail in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2048)) {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend(tail);
        let _ = crate::decode_image(&bytes);
    }
}

#[test]
fn correcting_metadata_from_the_book_page() {
    let mut h = harness();
    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("Corectează metadatele").click();
    h.run();
    h.get_by_label("Titlu").click();
    h.run();
    h.get_by_label("Titlu").type_text(" (ediția a 2-a)");
    h.run();
    h.get_by_label("Salvează").click();
    h.run();
    let s = &h.state().session;
    assert_eq!(
        s.book("a").map(|b| b.title.as_str()),
        Some("Rust in Action (ediția a 2-a)")
    );
    assert_eq!(s.edited_fields("a"), vec![raft_doc::edits::Field::Title]);
    assert_eq!(s.route(), &Route::Book { id: "a".into() });
    h.get_by_label("Corectată manual: 1 câmp(uri); restul vin din catalog");
    h.state_mut()
        .run("edit.undo", serde_json::Value::Null)
        .unwrap();
    h.run();
    assert_eq!(
        h.state().session.book("a").map(|b| b.title.as_str()),
        Some("Rust in Action")
    );
}

#[test]
fn processing_incomplete_books_one_after_another() {
    let mut h = harness();
    h.run();
    // "Война и мир" lacks authors, year and description.
    h.get_by_label("Date incomplete  (1)").click();
    h.run();
    h.get_by_label("1 din 2 cărți");
    h.state_mut()
        .run("filter.clear", serde_json::json!({}))
        .unwrap();
    h.state_mut()
        .run("view.edit", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("Titlu").click();
    h.run();
    h.get_by_label("Titlu").type_text("!");
    h.run();
    h.get_by_label("Salvează și următoarea").click();
    h.run();
    assert_eq!(h.state().session.route(), &Route::Edit { id: "b".into() });
    assert!(h.state().session.book("a").is_some_and(|b| b.edited));
    h.get_by_label("Autori").click();
    h.run();
    h.get_by_label("Autori").type_text("Лев Толстой");
    h.run();
    h.get_by_label("Verificată: ce lipsește nu se poate afla (nu mai apare la „date incomplete”)")
        .click();
    h.run();
    h.get_by_label("Salvează").click();
    h.run();
    let b = h.state().session.book("b").cloned().unwrap();
    assert_eq!(b.authors, vec!["Лев Толстой"]);
    assert!(b.reviewed && b.edited && !b.is_incomplete());
    h.get_by_label("verificată");
}

#[test]
fn resetting_one_field_refreshes_only_that_field_of_the_draft() {
    let mut h = harness();
    h.state_mut()
        .run(
            "book.edit",
            serde_json::json!({"id": "a", "fields": {"title": "Corectat"}}),
        )
        .unwrap();
    h.state_mut()
        .run("view.edit", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("An").click();
    h.run();
    h.get_by_label("An").type_text("9");
    h.run();
    h.get_by_label("din catalog").click();
    h.run();
    h.run();
    let d = h.state().edit.clone().unwrap();
    assert_eq!(d.title, "Rust in Action");
    assert_eq!(d.year, "20219");
}

#[test]
fn back_icon_and_keys_go_up_one_level() {
    let mut h = harness();
    h.state_mut()
        .run("view.edit", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    // Backspace while typing edits the text, it does not navigate.
    h.get_by_label("Titlu").click();
    h.run();
    h.key_press(egui::Key::Backspace);
    h.run();
    assert_eq!(h.state().session.route(), &Route::Edit { id: "a".into() });
    h.get_by_label("Înapoi").click();
    h.run();
    assert_eq!(h.state().session.route(), &Route::Book { id: "a".into() });
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
    h.run();
    assert_eq!(h.state().session.route(), &Route::Library);
    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.key_press(egui::Key::Backspace);
    h.run();
    assert_eq!(h.state().session.route(), &Route::Library);
}

#[test]
fn year_slider_drags_the_nearest_handle() {
    let mut h = harness();
    h.state_mut()
        .run(
            "book.edit",
            serde_json::json!({"id": "b", "fields": {"year": 1869}}),
        )
        .unwrap();
    h.run();
    // The track spans the rect minus 8 px at each end.
    let at_in =
        |r: egui::Rect, t: f32| egui::pos2(r.left() + 8.0 + t * (r.width() - 16.0), r.center().y);
    let r = h.get_by_label("Ani: 1869–2021").rect();
    let at = |t: f32| at_in(r, t);
    h.drag_at(at(1.0));
    h.run();
    h.hover_at(at(0.5));
    h.run();
    h.drop_at(at(0.5));
    h.run();
    let f = h.state().session.query().filters.clone();
    assert_eq!(f.year_min, None);
    assert_eq!(f.year_max, Some(1945));
    assert_eq!(h.state().session.result_count(), 1);
    // Back to the end: no upper limit. The slider moved up (one language less).
    let r = h.get_by_label("Ani: 1869–1945").rect();
    let at = |t: f32| at_in(r, t);
    h.drag_at(at(0.5));
    h.run();
    h.hover_at(at(1.0));
    h.run();
    h.drop_at(at(1.0));
    h.run();
    assert_eq!(h.state().session.query().filters.year_max, None);
    assert_eq!(h.state().session.result_count(), 2);
}

struct Trash;

impl raft_engine::Backend for Trash {
    fn content_search(&self, _: &str, _: usize) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
    fn save_progress(&self, _: &raft_doc::Progress) -> Result<(), String> {
        Ok(())
    }
    fn delete_progress(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn open_external(&self, _: &Book) -> Result<(), String> {
        Ok(())
    }
    fn now(&self) -> i64 {
        0
    }
    fn remove_book(&self, _: &Book) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn deleting_a_book_asks_first() {
    let catalog = app().session.catalog().clone();
    let ui = RaftUi::new(
        Session::new(catalog, vec![], Box::new(Trash)),
        Box::new(NoFiles),
    );
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 750.0))
        .build_ui_state(|ui, app: &mut RaftUi| app.show(ui), ui);
    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("Șterge cartea").click();
    h.run();
    h.get_by_label("Ștergi cartea?");
    // Esc closes the dialog and does not go back.
    h.key_press(egui::Key::Escape);
    h.run();
    h.run();
    assert!(h.query_by_label("Ștergi cartea?").is_none());
    assert_eq!(h.state().session.route(), &Route::Book { id: "a".into() });
    h.get_by_label("Șterge cartea").click();
    h.run();
    h.get_by_label("Renunță").click();
    h.run();
    assert!(h.state().session.book("a").is_some());
    h.get_by_label("Șterge cartea").click();
    h.run();
    h.get_by_label("Mută în coș").click();
    h.run();
    assert!(h.state().session.book("a").is_none());
    assert_eq!(h.state().session.route(), &Route::Library);
    h.get_by_label("1 din 1 cărți");
}

#[test]
fn draft_changes_only_contain_edited_fields() {
    let book = Book {
        id: "a".into(),
        title: "T".into(),
        categories: vec!["python".into(), "career".into()],
        ..Book::default()
    };
    let mut d = crate::edit::Draft::from_book(&book);
    assert!(d.changes(&book).is_empty());
    d.year = "2020".into();
    d.categories.remove("career");
    d.categories.insert("devops".into());
    let c = d.changes(&book);
    assert_eq!(c.len(), 2);
    assert_eq!(c["year"], "2020");
    assert_eq!(c["categories"], serde_json::json!(["python", "devops"]));
}

const SMALL_FB2: &str = r#"<FictionBook><body>
<section><title><p>Том первый</p></title><p>Ну, князь. Генуя и Лукка — поместья фамилии Бонапарте.</p></section>
<section><title><p>Том второй</p></title><p>В начале 1806 года Николай Ростов вернулся в отпуск.</p></section>
</body></FictionBook>"#;

struct TextFiles;

impl crate::Files for TextFiles {
    fn cover(&self, _: &Book) -> Option<Vec<u8>> {
        None
    }
    fn page(&self, _: &Book, _: u32, _: u32) -> Option<Result<Vec<u8>, String>> {
        None
    }
    fn text(&self, book: &Book) -> Option<Result<std::sync::Arc<raft_text::TextBook>, String>> {
        Some(
            raft_text::parse(SMALL_FB2.as_bytes(), book.format)
                .map(std::sync::Arc::new)
                .map_err(|e| e.to_string()),
        )
    }
}

#[test]
fn text_reader_shows_the_book_and_jumps_by_chapter() {
    let mut ui = app();
    ui.files = Box::new(TextFiles);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 750.0))
        .build_ui_state(|ui, app: &mut RaftUi| app.show(ui), ui);
    h.state_mut()
        .run("book.read", serde_json::json!({"id": "b"}))
        .unwrap();
    h.run();
    h.get_by_label("Cuprins").click();
    h.run();
    h.get_by_label("Том второй").click();
    h.run();
    let p = h.state().session.progress("b").cloned().unwrap();
    assert_eq!(p.block(), Some(2));
    assert_eq!(h.state().session.route(), &Route::Reader { id: "b".into() });
}
