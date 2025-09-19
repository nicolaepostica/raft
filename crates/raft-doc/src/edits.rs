//! Manual corrections of catalog metadata ("overrides").
//!
//! They live apart from the catalog (in `raft.db`), so regenerating the catalog
//! never loses them; a book is matched by its id (md5 of the content).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sanitize::{MAX_LIST, MAX_STR, cap};
use crate::{Book, BookId};

/// A metadata field that can be corrected by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Title,
    Authors,
    Year,
    Language,
    Publisher,
    Isbn,
    Categories,
    Tags,
    Description,
}

impl Field {
    pub const ALL: [Field; 9] = [
        Self::Title,
        Self::Authors,
        Self::Year,
        Self::Language,
        Self::Publisher,
        Self::Isbn,
        Self::Categories,
        Self::Tags,
        Self::Description,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Authors => "authors",
            Self::Year => "year",
            Self::Language => "language",
            Self::Publisher => "publisher",
            Self::Isbn => "isbn",
            Self::Categories => "categories",
            Self::Tags => "tags",
            Self::Description => "description",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.slug() == s)
    }

    /// Fields whose value is a list (stored as a JSON array of strings).
    pub fn is_list(self) -> bool {
        matches!(self, Self::Authors | Self::Categories | Self::Tags)
    }
}

/// One correction as stored in `raft.db`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Override {
    pub book_id: BookId,
    pub field: Field,
    /// Canonical value: text; a JSON array of strings for lists; digits or "" for the year.
    pub value: String,
}

/// Corrections of every book: book id → field → canonical value.
pub type Overrides = BTreeMap<BookId, BTreeMap<Field, String>>;

/// The canonical value of `field` in `book`.
pub fn field_value(book: &Book, field: Field) -> String {
    let list = |v: &[String]| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_owned());
    match field {
        Field::Title => book.title.clone(),
        Field::Authors => list(&book.authors),
        Field::Year => book.year.map(|y| y.to_string()).unwrap_or_default(),
        Field::Language => book.language.clone(),
        Field::Publisher => book.publisher.clone().unwrap_or_default(),
        Field::Isbn => book.isbn.clone().unwrap_or_default(),
        Field::Categories => list(&book.categories),
        Field::Tags => list(&book.tags),
        Field::Description => book.description.clone(),
    }
}

/// Applies one canonical value to a book. Values that do not parse are ignored,
/// so a damaged `raft.db` row can never break the catalog.
pub fn apply(book: &mut Book, field: Field, value: &str) {
    let list = || -> Option<Vec<String>> {
        let v: Vec<String> = serde_json::from_str(value).ok()?;
        Some(
            v.into_iter()
                .map(|s| cap(s.trim().to_owned()))
                .filter(|s| !s.is_empty())
                .take(MAX_LIST)
                .collect(),
        )
    };
    let text = || cap(value.trim().to_owned());
    let opt = || Some(text()).filter(|s| !s.is_empty());
    match field {
        Field::Title => book.title = text(),
        Field::Authors => {
            if let Some(v) = list() {
                book.authors = v;
            }
        }
        Field::Year => {
            if value.trim().is_empty() {
                book.year = None;
            } else if let Ok(y) = value.trim().parse::<i32>() {
                book.year = Some(y);
            }
        }
        Field::Language => book.language = text(),
        Field::Publisher => book.publisher = opt(),
        Field::Isbn => book.isbn = opt(),
        Field::Categories => {
            if let Some(v) = list() {
                book.categories = v;
            }
        }
        Field::Tags => {
            if let Some(v) = list() {
                book.tags = v;
            }
        }
        Field::Description => book.description = text(),
    }
}

/// `book` with all its corrections applied.
pub fn edited(book: &Book, overrides: &Overrides) -> Book {
    let mut b = book.clone();
    if let Some(fields) = overrides.get(&book.id) {
        for (field, value) in fields {
            apply(&mut b, *field, value);
        }
    }
    b
}

/// Longest accepted value, in bytes (descriptions are the longest field).
pub const MAX_VALUE: usize = MAX_STR;

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Book {
        Book {
            id: "x".into(),
            title: "Applied Deep Learning".into(),
            authors: vec!["Umberto Michelucci".into()],
            language: "ro".into(),
            year: Some(2020),
            ..Book::default()
        }
    }

    #[test]
    fn applies_and_round_trips_every_field() {
        let mut b = book();
        apply(&mut b, Field::Language, "ru");
        apply(&mut b, Field::Authors, r#"["Умберто Микелуччи", " "]"#);
        apply(&mut b, Field::Year, "");
        apply(&mut b, Field::Publisher, "БХВ-Петербург");
        assert_eq!(b.language, "ru");
        assert_eq!(b.authors, vec!["Умберто Микелуччи"]);
        assert_eq!(b.year, None);
        for f in Field::ALL {
            let v = field_value(&b, f);
            let mut c = book();
            apply(&mut c, f, &v);
            assert_eq!(field_value(&c, f), v, "{f:?}");
        }
    }

    #[test]
    fn bad_values_are_ignored() {
        let mut b = book();
        apply(&mut b, Field::Authors, "not json");
        apply(&mut b, Field::Year, "soon");
        assert_eq!(b.authors, vec!["Umberto Michelucci"]);
        assert_eq!(b.year, Some(2020));
    }

    #[test]
    fn edited_uses_only_that_books_overrides() {
        let mut o = Overrides::new();
        o.entry("x".into())
            .or_default()
            .insert(Field::Title, "Прикладное".into());
        o.entry("y".into())
            .or_default()
            .insert(Field::Title, "Other".into());
        assert_eq!(edited(&book(), &o).title, "Прикладное");
        assert_eq!(
            edited(&book(), &Overrides::new()).title,
            "Applied Deep Learning"
        );
        assert_eq!(Field::from_slug("isbn"), Some(Field::Isbn));
        assert_eq!(Field::from_slug("nope"), None);
    }
}
