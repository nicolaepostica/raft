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

/// Renders every page as a small blank PNG, at once.
struct BlankPages;

impl crate::Files for BlankPages {
    fn cover(&self, _: &Book) -> Option<Vec<u8>> {
        None
    }
    fn page(&self, _: &Book, _: u32, width: u32) -> Option<Result<Vec<u8>, String>> {
        let img = image::RgbImage::from_pixel(width, width * 3 / 2, image::Rgb([250, 250, 250]));
        let mut png = Vec::new();
        Some(
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .map(|()| png)
                .map_err(|e| e.to_string()),
        )
    }
    fn text(&self, _: &Book) -> Option<Result<std::sync::Arc<raft_text::TextBook>, String>> {
        None
    }
}

#[test]
fn paged_reader_scrolls_through_the_whole_book() {
    let mut ui = app();
    ui.files = Box::new(BlankPages);
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(|ui, app: &mut RaftUi| app.show(ui), ui);
    h.state_mut()
        .run("book.read", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    let page = |h: &Harness<'_, RaftUi>| h.state().session.progress("a").and_then(|p| p.page);
    assert_eq!(page(&h), Some(1));
    // Down a few lines stays on the first (tall) page; End reaches the last one.
    h.key_press(egui::Key::ArrowDown);
    h.run();
    assert_eq!(page(&h), Some(1));
    h.key_press(egui::Key::End);
    h.run();
    assert_eq!(page(&h), Some(10));
    h.key_press(egui::Key::Home);
    h.run();
    assert_eq!(page(&h), Some(1));
    // Screens down move the reading point into the next pages.
    for _ in 0..4 {
        h.key_press(egui::Key::PageDown);
        h.run();
    }
    let reached = page(&h).unwrap_or(0);
    assert!((2..10).contains(&reached), "page {reached}");
    // A page chosen elsewhere is scrolled to and kept.
    h.state_mut()
        .run("reader.goto", serde_json::json!({"page": 7}))
        .unwrap();
    h.run();
    h.run();
    assert_eq!(page(&h), Some(7));
    h.get_by_label("Următoarea").click();
    h.run();
    h.run();
    assert_eq!(page(&h), Some(8));

    // Zoom: buttons and keys; the page stays the same.
    let zoom = |h: &Harness<'_, RaftUi>| (h.state().session.zoom() * 100.0).round() as i32;
    h.get_by_label("+").click();
    h.run();
    assert_eq!(zoom(&h), 110);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    h.run();
    h.run();
    assert_eq!(zoom(&h), 125);
    assert_eq!(page(&h), Some(8));
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Minus);
    h.run();
    h.get_by_label("−").click();
    h.run();
    assert_eq!(zoom(&h), 100);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Minus);
    h.run();
    h.get_by_label("90%").click();
    h.run();
    h.run();
    assert_eq!(zoom(&h), 100);
    assert_eq!(page(&h), Some(8));
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
    fn save_source(&self, _: &raft_doc::Source) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn ticked_books_go_to_the_trash_together() {
    let catalog = app().session.catalog().clone();
    let ui = RaftUi::new(
        Session::new(catalog, vec![], Box::new(Trash)),
        Box::new(NoFiles),
    );
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 750.0))
        .build_ui_state(|ui, app: &mut RaftUi| app.show(ui), ui);
    h.run();
    assert!(h.query_by_label_contains("selectate").is_none());
    h.get_by_label("Selectează: Rust in Action").click();
    h.run();
    h.get_by_label("1 selectate");
    // Esc clears the selection.
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.state().session.selected().is_empty());
    h.get_by_label("Selectează: Rust in Action").click();
    h.run();
    h.get_by_label("Selectează toate (2)").click();
    h.run();
    h.get_by_label("2 selectate");
    h.get_by_label("Mută în coș (2)").click();
    h.run();
    h.get_by_label("Ștergi 2 cărți?");
    h.get_by_label("Renunță").click();
    h.run();
    assert_eq!(h.state().session.catalog().books.len(), 2);
    h.get_by_label("Mută în coș (2)").click();
    h.run();
    h.get_by_label("Mută în coș").click();
    h.run();
    assert!(h.state().session.catalog().books.is_empty());
    assert!(h.state().session.selected().is_empty());
    h.get_by_label("2 cărți mutate în coș");
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
fn book_actions_are_icons_under_the_cover_in_both_modes() {
    let mut h = harness();
    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    for name in [
        "Citește",
        "Deschide în aplicația externă",
        "Arată în dosar",
        "Copertă",
        "Șterge cartea",
    ] {
        h.get_by_label(name);
    }
    h.get_by_label("Copertă").click();
    h.run();
    h.get_by_label("Prima pagină a cărții");
    h.get_by_label("Alege o imagine…").click();
    h.run();
    // NoFiles cannot choose files: the status bar says so.
    h.get_by_label("alegerea fișierelor nu e disponibilă aici");
    h.state_mut()
        .run("view.edit", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("Arată în dosar");
    h.get_by_label("Citește").click();
    h.run();
    assert_eq!(h.state().session.route(), &Route::Reader { id: "a".into() });
}

#[test]
fn edit_icon_turns_into_cancel_then_save_and_cancel() {
    let mut h = harness();
    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("Corectează metadatele").click();
    h.run();
    assert!(h.query_by_label("Salvează").is_none());
    h.get_by_label("Renunță");
    h.get_by_label("Titlu").click();
    h.run();
    h.get_by_label("Titlu").type_text("?");
    h.run();
    h.get_by_label("Salvează");
    h.get_by_label("Renunță").click();
    h.run();
    let s = &h.state().session;
    assert_eq!(s.route(), &Route::Book { id: "a".into() });
    assert_eq!(
        s.book("a").map(|b| b.title.as_str()),
        Some("Rust in Action")
    );
    assert!(h.state().edit.is_none());
    assert!(h.query_by_label("Renunță").is_none());
}

#[test]
fn interface_language_changes_from_file_settings() {
    let mut h = harness();
    h.run();
    h.get_by_label("Fișier").click();
    h.run();
    h.get_by_label("Setări Ctrl+,").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Limba interfeței")
        .click();
    h.run();
    h.get_by_label("English").click();
    h.run();
    assert_eq!(h.state().session.language(), "en");
    h.get_by_label("Filters");
    h.get_by_label("2 of 2 books");
    h.get_by_label("Fiction  (1)");
    h.state_mut()
        .run("view.language", serde_json::json!({"language": "ru"}))
        .unwrap();
    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("Читать");
    h.get_by_label("Год");
    h.state_mut()
        .run("view.language", serde_json::json!({"language": "ro"}))
        .unwrap();
    h.run();
    h.get_by_label("Citește");
}

#[test]
fn duplicates_are_filtered_compared_and_dismissed() {
    let mut catalog = app().session.catalog().clone();
    for b in &mut catalog.books {
        b.isbn = Some("978-0-306-40615-7".into());
        b.size = 3 * 1_048_576;
    }
    let ui = RaftUi::new(
        Session::new(catalog, vec![], Box::new(NullBackend)),
        Box::new(NoFiles),
    );
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 750.0))
        .build_ui_state(|ui, app: &mut RaftUi| app.show(ui), ui);
    h.run();
    h.get_by_label("Dubluri posibile  (2)").click();
    h.run();
    assert_eq!(h.state().session.result_count(), 2);
    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("Posibile dubluri ale acestei cărți:");
    h.get_by_label_contains("FB2 · 3.0 MB");
    h.get_by_label("Война и мир").click();
    h.run();
    assert_eq!(h.state().session.route(), &Route::Book { id: "b".into() });
    h.get_by_label("nu e dublură").click();
    h.run();
    assert!(
        h.state()
            .session
            .book("a")
            .is_some_and(|b| b.duplicates.is_empty())
    );
    assert!(h.state().session.book("b").is_some_and(|b| b.not_duplicate));
    // Still shown, so the mark can be taken back.
    h.get_by_label("nu e dublură");
    assert!(
        h.query_by_label("Posibile dubluri ale acestei cărți:")
            .is_none()
    );
}

#[test]
fn subfolders_are_a_filter_and_a_link_on_the_book_page() {
    let mut catalog = app().session.catalog().clone();
    for (b, path) in catalog
        .books
        .iter_mut()
        .zip(["Rust/a.pdf", "Tolstoy/b.fb2"])
    {
        b.source = "/backup/books".into();
        b.path = path.into();
    }
    let session =
        Session::new(catalog, vec![], Box::new(NullBackend)).with_sources(vec!["/backup/books"]);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(
            |ui, app: &mut RaftUi| app.show(ui),
            RaftUi::new(session, Box::new(NoFiles)),
        );
    h.run();
    h.get_by_label("Foldere (2)").click();
    h.run();
    h.get_by_label("Tolstoy  (1)").click();
    h.run();
    assert_eq!(h.state().session.result_count(), 1);
    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.run();
    h.get_by_label("Rust").click();
    h.run();
    assert_eq!(h.state().session.route(), &Route::Library);
    assert_eq!(
        h.state()
            .session
            .query()
            .filters
            .folders
            .iter()
            .collect::<Vec<_>>(),
        ["/backup/books/Rust"]
    );
}

#[test]
fn library_folders_are_not_in_the_filters() {
    let mut h = harness();
    h.run();
    h.get_by_label("Filtre");
    assert!(h.query_by_label("Surse").is_none());
    // They are added from the settings window.
    h.state_mut()
        .run("view.settings", serde_json::json!({"open": true}))
        .unwrap();
    h.run();
    h.get_by_label("Adaugă un folder…").click();
    h.run();
    h.get_by_label("alegerea fișierelor nu e disponibilă aici");
}

#[test]
fn settings_window_shows_and_switches_the_folders() {
    let mut catalog = app().session.catalog().clone();
    for b in &mut catalog.books {
        b.source = "/backup/books".into();
    }
    if let Some(b) = catalog.books.get_mut(1) {
        b.source = "/run/media/disk/books2".into();
    }
    let session = Session::new(catalog, vec![], Box::new(Trash))
        .with_sources(vec!["/backup/books", "/run/media/disk/books2"]);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 750.0))
        .build_ui_state(
            |ui, app: &mut RaftUi| app.show(ui),
            RaftUi::new(session, Box::new(NoFiles)),
        );
    h.run();
    h.get_by_label("Fișier").click();
    h.run();
    h.get_by_label("Setări Ctrl+,").click();
    h.run();
    assert!(h.state().session.settings_open());
    h.get_by_label("Surse de cărți");
    assert_eq!(h.query_all_by_label("1 cărți").count(), 2);
    h.get_by_label("Activ: books2").click();
    h.run();
    assert_eq!(h.state().session.catalog().books.len(), 1);
    h.get_by_label("dezactivat");
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "/backup/books")
        .click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "/backup/books")
        .type_text("Raftul mare");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    assert_eq!(
        h.state().session.source_name("/backup/books"),
        "Raftul mare"
    );
    h.get_by_label("Întunecată").click();
    h.run();
    assert!(h.state().session.dark());
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!h.state().session.settings_open());
}

#[test]
fn settings_shortcuts_tab_lists_and_searches_the_keys() {
    let mut h = harness();
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Comma);
    h.run();
    assert!(h.state().session.settings_open());
    h.get_by_label("Scurtături").click();
    h.run();
    assert_eq!(h.state().session.settings_tab(), "shortcuts");
    h.get_by_label("Ctrl+,");
    h.get_by_label("Ctrl+Shift+T");
    h.get_by_label("Mută cursorul în căutare");
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Caută")
        .click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Caută")
        .type_text("temă");
    h.run();
    h.get_by_label("Ctrl+Shift+T");
    assert!(h.query_by_label("Ctrl+,").is_none());
    assert!(h.query_by_label("Mută cursorul în căutare").is_none());
    h.get_by_label("General").click();
    h.run();
    h.get_by_label("Surse de cărți");
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

#[test]
fn round_x_clears_the_search() {
    let mut h = harness();
    h.run();
    assert!(h.query_by_label("Șterge căutarea").is_none());
    h.state_mut()
        .run("search.set", serde_json::json!({"text": "Rust"}))
        .unwrap();
    h.run();
    h.run();
    h.get_by_label("1 din 2 cărți");
    h.get_by_label("Șterge căutarea").click();
    h.run();
    h.run();
    assert_eq!(h.state().session.query().text, "");
    assert_eq!(h.state().search, "");
    h.get_by_label("2 din 2 cărți");
    assert!(h.query_by_label("Șterge căutarea").is_none());
}

#[test]
fn continue_reading_has_its_own_tab() {
    let mut h = harness();
    h.run();
    // Nothing started: the library tab shows the grid and the filters, no reading strip.
    h.get_by_label("Continuă lectura (0)");
    h.get_by_label("Filtre");
    h.get_by_label("Selectează: Rust in Action");
    h.get_by_label("Continuă lectura (0)").click();
    h.run();
    assert_eq!(h.state().session.library_tab(), "reading");
    h.get_by_label("Nu ai început încă nicio carte. Cărțile deschise apar aici.");
    assert!(h.query_by_label("Filtre").is_none());

    h.state_mut()
        .run("view.book", serde_json::json!({"id": "a"}))
        .unwrap();
    h.state_mut()
        .run("book.read", serde_json::json!({}))
        .unwrap();
    h.state_mut()
        .run("reader.next", serde_json::json!({}))
        .unwrap();
    h.state_mut()
        .run("view.library", serde_json::json!({}))
        .unwrap();
    h.state_mut()
        .run("view.library", serde_json::json!({}))
        .unwrap();
    h.run();
    h.run();
    // Back from the book returns to the reading tab, which now lists it alone.
    h.get_by_label("Continuă lectura (1)");
    h.get_by_label("Selectează: Rust in Action");
    assert!(h.query_by_label("Selectează: Война и мир").is_none());

    // Searching switches back to the library tab.
    h.state_mut()
        .run("search.set", serde_json::json!({"text": "Rust"}))
        .unwrap();
    h.run();
    assert_eq!(h.state().session.library_tab(), "library");
    h.get_by_label("Bibliotecă").click();
    h.run();
    h.get_by_label("Filtre");
}
