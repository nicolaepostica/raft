//! Parses `catalog.json` (produced by `build-catalog --json`) into a [`Catalog`].
//!
//! The input is untrusted: sizes and counts are capped, malformed records are skipped
//! and reported instead of failing the whole catalog.

use std::collections::BTreeMap;

use raft_doc::sanitize::{BookParts, book_from_parts, cap, string_list};
pub use raft_doc::sanitize::{MAX_LIST, MAX_STR, json_list};
use raft_doc::{Book, Catalog, Category};
use serde::Deserialize;

/// Largest accepted catalog file.
pub const MAX_BYTES: usize = 256 * 1024 * 1024;
/// Largest accepted number of books.
pub const MAX_BOOKS: usize = 200_000;
/// Most categories accepted.
pub const MAX_CATEGORIES: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("catalog is too large: {0} bytes (max {MAX_BYTES})")]
    TooLarge(usize),
    #[error("catalog has too many books: {0} (max {MAX_BOOKS})")]
    TooManyBooks(usize),
    #[error("catalog JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

/// Parse result: the catalog plus how many book records were skipped as malformed.
#[derive(Debug, Default)]
pub struct Parsed {
    pub catalog: Catalog,
    pub skipped: usize,
}

#[derive(Deserialize)]
struct RawCatalog {
    #[serde(default)]
    categories: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    books: Vec<serde_json::Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawBook {
    id: Option<String>,
    path: Option<String>,
    filename: Option<String>,
    format: Option<String>,
    size: Option<serde_json::Value>,
    pages: Option<serde_json::Value>,
    title: Option<String>,
    authors: Option<serde_json::Value>,
    year: Option<serde_json::Value>,
    language: Option<String>,
    publisher: Option<String>,
    isbn: Option<String>,
    categories: Option<serde_json::Value>,
    tags: Option<serde_json::Value>,
    description: Option<String>,
    cover: Option<String>,
    added_at: Option<String>,
}

/// Parses catalog JSON bytes.
pub fn parse(bytes: &[u8]) -> Result<Parsed, CatalogError> {
    if bytes.len() > MAX_BYTES {
        return Err(CatalogError::TooLarge(bytes.len()));
    }
    let raw: RawCatalog = serde_json::from_slice(bytes)?;
    if raw.books.len() > MAX_BOOKS {
        return Err(CatalogError::TooManyBooks(raw.books.len()));
    }
    let categories = raw
        .categories
        .into_iter()
        .take(MAX_CATEGORIES)
        .map(|(slug, name)| Category {
            name: name
                .as_str()
                .map_or_else(|| slug.clone(), |s| cap(s.to_owned())),
            slug: cap(slug),
        })
        .collect();
    let mut parsed = Parsed {
        catalog: Catalog {
            books: Vec::with_capacity(raw.books.len()),
            categories,
        },
        skipped: 0,
    };
    for value in raw.books {
        match serde_json::from_value::<RawBook>(value)
            .ok()
            .and_then(book_from_raw)
        {
            Some(book) => parsed.catalog.books.push(book),
            None => parsed.skipped += 1,
        }
    }
    Ok(parsed)
}

fn book_from_raw(raw: RawBook) -> Option<Book> {
    book_from_parts(BookParts {
        id: raw.id,
        path: raw.path,
        filename: raw.filename,
        format: raw.format,
        size: raw.size.as_ref().and_then(serde_json::Value::as_u64),
        pages: raw.pages.as_ref().and_then(as_u32),
        title: raw.title,
        authors: string_list(raw.authors.as_ref()),
        year: raw.year.as_ref().and_then(as_i32),
        language: raw.language,
        publisher: raw.publisher,
        isbn: raw.isbn,
        categories: string_list(raw.categories.as_ref()),
        tags: string_list(raw.tags.as_ref()),
        description: raw.description,
        cover: raw.cover,
        added_at: raw.added_at,
    })
}

fn as_u32(v: &serde_json::Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

fn as_i32(v: &serde_json::Value) -> Option<i32> {
    v.as_i64().and_then(|n| i32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raft_doc::Format;
    use proptest::prelude::*;

    const SAMPLE: &str = r#"{
      "categories": {"python": "Python", "devops": "DevOps"},
      "books": [
        {"id": "a1", "path": "X_2020.pdf", "filename": "X_2020.pdf", "format": "pdf",
         "size": 10, "pages": 3, "title": "X", "authors": ["Ann", "Bob"], "year": 2020,
         "language": "en", "categories": ["python"], "tags": ["t"], "description": "d",
         "cover": "covers/a1.webp"},
        {"id": "b2", "format": "fb2", "title": "", "filename": "y.fb2", "year": null,
         "pages": null, "language": null, "authors": "not a list"},
        {"title": "no id"},
        42
      ]
    }"#;

    #[test]
    fn parses_sample_and_skips_bad_records() {
        let p = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(p.skipped, 2);
        assert_eq!(p.catalog.books.len(), 2);
        let a = &p.catalog.books[0];
        assert_eq!(a.authors, vec!["Ann", "Bob"]);
        assert_eq!(a.format, Format::Pdf);
        assert_eq!(a.pages, Some(3));
        let b = &p.catalog.books[1];
        assert_eq!(b.display_title(), "y.fb2");
        assert!(b.authors.is_empty());
        assert_eq!(b.year, None);
        assert_eq!(p.catalog.category_name("devops"), "DevOps");
    }

    #[test]
    fn json_list_handles_sqlite_strings() {
        assert_eq!(json_list(r#"["go", "rust"]"#), vec!["go", "rust"]);
        assert!(json_list("garbage").is_empty());
        assert!(json_list(r#"{"a":1}"#).is_empty());
    }

    #[test]
    fn cap_respects_char_boundaries() {
        let s = "я".repeat(MAX_STR);
        let c = cap(s);
        assert!(c.len() <= MAX_STR);
        assert!(c.chars().all(|ch| ch == 'я'));
    }

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = parse(&bytes);
        }

        #[test]
        fn never_panics_on_arbitrary_json_books(
            id in proptest::option::of(".{0,40}"),
            title in proptest::option::of(".{0,200}"),
            year in proptest::option::of(any::<i64>()),
            pages in proptest::option::of(any::<i64>()),
            authors in proptest::collection::vec(".{0,30}", 0..50),
        ) {
            let book = serde_json::json!({
                "id": id, "title": title, "year": year, "pages": pages, "authors": authors,
            });
            let doc = serde_json::json!({"books": [book]});
            let p = parse(doc.to_string().as_bytes()).unwrap();
            prop_assert_eq!(p.catalog.books.len() + p.skipped, 1);
            for b in &p.catalog.books {
                prop_assert!(b.authors.len() <= MAX_LIST);
            }
        }
    }

    #[test]
    fn parses_real_catalog_if_present() {
        let Ok(bytes) = std::fs::read("/backup/books/catalog.json") else {
            return;
        };
        let p = parse(&bytes).unwrap();
        assert!(p.catalog.books.len() > 100);
        assert_eq!(p.skipped, 0);
    }
}
