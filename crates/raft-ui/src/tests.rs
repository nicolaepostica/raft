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
            Category { slug: "go-rust".into(), name: "Go, Rust".into() },
            Category { slug: "fiction".into(), name: "Beletristică".into() },
        ],
    };
    RaftUi::new(Session::new(catalog, vec![], Box::new(NullBackend)), Box::new(NoFiles))
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
    h.state_mut().run("view.book", serde_json::json!({"id": "a"})).unwrap();
    h.run();
    h.get_by_label("O carte despre Rust.");
    h.get_by_label("Citește").click();
    h.run();
    assert_eq!(h.state().session.route(), &Route::Reader { id: "a".into() });
}
