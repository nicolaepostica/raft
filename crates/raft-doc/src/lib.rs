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

    /// Formats read as reflowable text (text reader).
    pub fn is_text(self) -> bool {
        matches!(self, Self::Epub | Self::Fb2)
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
    /// Some metadata was corrected by hand (set from the overrides, not the catalog).
    #[serde(default)]
    pub edited: bool,
    /// Checked by hand: its missing fields are known to be unknowable.
    #[serde(default)]
    pub reviewed: bool,
    /// The library folder (source) the book was loaded from; `path` and `cover`
    /// are relative to it.
    #[serde(default)]
    pub source: String,
    /// Other books that are probably the same work (same ISBN, or same title,
    /// year and volume): set by `raft_search::mark_duplicates`, not stored.
    #[serde(default)]
    pub duplicates: Vec<BookId>,
    /// Marked by hand as not a duplicate of the books it resembles.
    #[serde(default)]
    pub not_duplicate: bool,
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

    /// The subfolder of its library folder the file is in, as a full path
    /// (`/disk/Books/Author`); `None` for a file at the top of the folder.
    pub fn folder(&self) -> Option<String> {
        let (dir, _) = self.path.rsplit_once('/')?;
        if dir.is_empty() {
            return None;
        }
        Some(if self.source.is_empty() {
            dir.to_owned()
        } else {
            format!("{}/{dir}", self.source)
        })
    }

    pub fn primary_category(&self) -> Option<&str> {
        self.categories.first().map(String::as_str)
    }

    /// Whether `field` has no value.
    pub fn lacks(&self, field: edits::Field) -> bool {
        edits::field_value(self, field)
            .trim()
            .trim_matches(['[', ']'])
            .is_empty()
    }

    /// Metadata fields with no value, in form order.
    pub fn missing(&self) -> Vec<edits::Field> {
        edits::Field::ALL
            .into_iter()
            .filter(|f| self.lacks(*f))
            .collect()
    }

    /// Lacks a [required](edits::Field::REQUIRED) field and was not marked as reviewed.
    pub fn is_incomplete(&self) -> bool {
        !self.reviewed && edits::Field::REQUIRED.iter().any(|f| self.lacks(*f))
    }

    pub fn has(&self, mark: Mark) -> bool {
        match mark {
            Mark::Edited => self.edited,
            Mark::Incomplete => self.is_incomplete(),
            Mark::Reviewed => self.reviewed,
            Mark::Duplicate => !self.duplicates.is_empty(),
        }
    }
}

/// A state of a book's metadata that can be filtered on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    /// Corrected by hand.
    Edited,
    /// Lacks a required field and not reviewed yet.
    Incomplete,
    /// Marked as checked by hand.
    Reviewed,
    /// Probably the same work as another book of the library.
    Duplicate,
}

impl Mark {
    pub const ALL: [Mark; 4] = [
        Self::Edited,
        Self::Incomplete,
        Self::Reviewed,
        Self::Duplicate,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Edited => "edited",
            Self::Incomplete => "incomplete",
            Self::Reviewed => "reviewed",
            Self::Duplicate => "duplicate",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.slug() == s)
    }
}

/// A category: slug and display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub slug: String,
    pub name: String,
}

/// A library folder the books come from, as the user configured it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// Absolute folder path, without a trailing `/`; holds `catalog.db`.
    pub path: String,
    /// Name shown instead of the folder's own name (empty = the folder's name).
    #[serde(default)]
    pub name: String,
    /// Disabled folders stay in the list but their books are not loaded.
    pub enabled: bool,
}

impl Source {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            name: String::new(),
            enabled: true,
        }
    }

    /// The name to show: the given one, else the folder's last component.
    pub fn display_name(&self) -> &str {
        let name = self.name.trim();
        if !name.is_empty() {
            return name;
        }
        self.path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or(&self.path)
    }
}

impl From<&str> for Source {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
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

    /// Adds another source's books and categories. A book already present (the
    /// same file, by md5, in two folders) keeps its first copy.
    pub fn merge(&mut self, other: Catalog) {
        let known: std::collections::HashSet<BookId> =
            self.books.iter().map(|b| b.id.clone()).collect();
        self.books
            .extend(other.books.into_iter().filter(|b| !known.contains(&b.id)));
        for c in other.categories {
            if !self.categories.iter().any(|k| k.slug == c.slug) {
                self.categories.push(c);
            }
        }
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
    /// Books having every one of these marks.
    #[serde(default)]
    pub marks: BTreeSet<Mark>,
    /// Books lacking any of these fields.
    #[serde(default)]
    pub missing: BTreeSet<edits::Field>,
    /// Books from any of these library folders.
    #[serde(default)]
    pub sources: BTreeSet<String>,
    /// Books in any of these subfolders (full paths, see `Book::folder`).
    #[serde(default)]
    pub folders: BTreeSet<String>,
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
            && self.missing.is_empty()
            && self.sources.is_empty()
            && self.folders.is_empty()
            && self.categories.is_empty()
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

impl Progress {
    /// The block index of a text-reader position (`block:N`).
    pub fn block(&self) -> Option<usize> {
        self.position
            .as_deref()?
            .strip_prefix("block:")?
            .parse()
            .ok()
    }
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
