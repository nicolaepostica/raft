//! Raft data model (layer 1): plain data with serde, no logic beyond small helpers.

pub mod edits;
pub mod sanitize;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Book identifier: md5 of the file content, stable across renames.
pub type BookId = String;

/// File format of a book.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Pdf,
    Epub,
    Djvu,
    Fb2,
    #[default]
    Other,
}

impl Format {
    /// Parses the catalog's `format` column; unknown values become [`Format::Other`].
    pub fn from_slug(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "pdf" => Self::Pdf,
            "epub" => Self::Epub,
            "djvu" => Self::Djvu,
            "fb2" => Self::Fb2,
            _ => Self::Other,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Epub => "epub",
            Self::Djvu => "djvu",
            Self::Fb2 => "fb2",
            Self::Other => "other",
        }
    }

    /// Formats whose pages can be rendered as images (paged reader).
    pub fn is_paged(self) -> bool {
        matches!(self, Self::Pdf | Self::Djvu)
    }

    pub const ALL: [Format; 5] = [Self::Pdf, Self::Epub, Self::Djvu, Self::Fb2, Self::Other];
}

/// One book from the catalog.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub id: BookId,
    /// Relative to the library root.
    pub path: String,
    pub filename: String,
    pub format: Format,
    pub size: u64,
    pub pages: Option<u32>,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    /// ISO 639-1, empty when unknown.
    pub language: String,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    /// Category slugs, most relevant first.
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub description: String,
    /// Relative cover path (`covers/<id>.webp`).
    pub cover: Option<String>,
    pub added_at: Option<String>,
}

impl Book {
    /// Title to show: the catalog title, or the file name when the title is empty.
    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            &self.filename
        } else {
            &self.title
        }
    }

    pub fn authors_line(&self) -> String {
        self.authors.join(", ")
    }

    pub fn primary_category(&self) -> Option<&str> {
        self.categories.first().map(String::as_str)
    }
}

/// A category: slug and display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub slug: String,
    pub name: String,
}

/// The whole read-only catalog.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub books: Vec<Book>,
    pub categories: Vec<Category>,
}

impl Catalog {
    pub fn book(&self, id: &str) -> Option<&Book> {
        self.books.iter().find(|b| b.id == id)
    }

    pub fn category_name<'a>(&'a self, slug: &'a str) -> &'a str {
        self.categories
            .iter()
            .find(|c| c.slug == slug)
            .map_or(slug, |c| c.name.as_str())
    }
}

/// Facet filters. Empty sets mean "no restriction".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filters {
    pub categories: BTreeSet<String>,
    /// Match categories only on the first (most relevant) one.
    pub primary_category_only: bool,
    pub languages: BTreeSet<String>,
    pub formats: BTreeSet<Format>,
    pub year_min: Option<i32>,
    pub year_max: Option<i32>,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
            && self.languages.is_empty()
            && self.formats.is_empty()
            && self.year_min.is_none()
            && self.year_max.is_none()
    }
}

/// Result ordering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    #[default]
    Relevance,
    Title,
    YearDesc,
    YearAsc,
    RecentlyAdded,
}

impl Sort {
    pub const ALL: [Sort; 5] = [
        Self::Relevance,
        Self::Title,
        Self::YearDesc,
        Self::YearAsc,
        Self::RecentlyAdded,
    ];
}

/// What the library view shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    pub text: String,
    pub filters: Filters,
    pub sort: Sort,
}

/// Reading progress of one book.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub book_id: BookId,
    /// 1-based page for paged formats.
    pub page: Option<u32>,
    /// Format-specific position (e.g. EPUB CFI or text offset).
    pub position: Option<String>,
    /// 0.0 ..= 100.0
    pub percent: f32,
    /// Unix seconds.
    pub updated_at: i64,
}

/// Which screen is shown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "screen", rename_all = "snake_case")]
pub enum Route {
    #[default]
    Library,
    Book {
        id: BookId,
    },
    Reader {
        id: BookId,
    },
    /// The metadata correction form of a book.
    Edit {
        id: BookId,
    },
}

/// The user's state that commands change and history can restore.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub route: Route,
    pub query: Query,
    pub progress: BTreeMap<BookId, Progress>,
    /// Manual metadata corrections.
    pub overrides: edits::Overrides,
}
