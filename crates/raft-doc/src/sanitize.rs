//! Normalization of loosely typed book records (shared by the JSON and SQLite loaders).

use crate::{Book, Format};

/// Longest kept string field, in bytes (longer ones are cut at a char boundary).
pub const MAX_STR: usize = 8 * 1024;
/// Most items kept in a list field (authors, tags, categories).
pub const MAX_LIST: usize = 32;

/// Builds a book from loose fields (also used by the SQLite loader).
pub fn book_from_parts(parts: BookParts) -> Option<Book> {
    let id = cap(parts.id?);
    if id.trim().is_empty() {
        return None;
    }
    let path = cap(parts.path.unwrap_or_default());
    let filename = cap(parts.filename.unwrap_or_else(|| path.clone()));
    Some(Book {
        id,
        format: Format::from_slug(&parts.format.unwrap_or_default()),
        size: parts.size.unwrap_or(0),
        pages: parts.pages,
        title: cap(parts.title.unwrap_or_default()),
        authors: clean_authors(parts.authors),
        year: parts.year,
        language: cap(parts.language.unwrap_or_default()),
        publisher: parts.publisher.map(cap),
        isbn: parts.isbn.map(cap),
        categories: parts.categories,
        tags: parts.tags,
        description: cap(parts.description.unwrap_or_default()),
        cover: parts.cover.map(cap).filter(|c| !c.is_empty()),
        added_at: parts.added_at.map(cap),
        path,
        filename,
        edited: false,
        reviewed: false,
    })
}

/// Sites that books were downloaded from; never an author.
const DOWNLOAD_SITES: [&str; 6] = [
    "it-ebooks",
    "allitebooks",
    "alldatasheet",
    "freebookspot",
    "ebook3000",
    "bookzz",
];

/// Placeholder names that PDF tools write when nobody filled in the author.
const PLACEHOLDERS: [&str; 6] = [
    "user",
    "windows user",
    "unknown",
    "admin",
    "administrator",
    "owner",
];

/// Whether an "author" taken from file metadata is really the producing
/// program, a download site or a placeholder.
pub fn is_junk_author(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    lower.is_empty()
        // PDF "Creator" is the program or site that made the file.
        || lower.starts_with("creator:")
        || lower.starts_with("producer:")
        || lower.starts_with("provided by")
        || lower.starts_with("www.")
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || DOWNLOAD_SITES.iter().any(|s| lower.contains(s))
        || PLACEHOLDERS.contains(&lower.as_str())
}

/// Authors without download sites, producing programs and placeholders.
pub fn clean_authors(authors: Vec<String>) -> Vec<String> {
    authors.into_iter().filter(|a| !is_junk_author(a)).collect()
}

/// Loose, already-typed fields of a book record.
#[derive(Debug, Default)]
pub struct BookParts {
    pub id: Option<String>,
    pub path: Option<String>,
    pub filename: Option<String>,
    pub format: Option<String>,
    pub size: Option<u64>,
    pub pages: Option<u32>,
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub cover: Option<String>,
    pub added_at: Option<String>,
}

/// Accepts a JSON list of strings, or a string holding a JSON list (the SQLite form).
pub fn string_list(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.as_str())
            .filter(|s| !s.trim().is_empty())
            .take(MAX_LIST)
            .map(|s| cap(s.to_owned()))
            .collect(),
        Some(serde_json::Value::String(s)) => json_list(s),
        _ => Vec::new(),
    }
}

/// Parses a string column that holds a JSON list (`'["a", "b"]'`).
pub fn json_list(s: &str) -> Vec<String> {
    if s.len() > MAX_STR * 4 {
        return Vec::new();
    }
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(v @ serde_json::Value::Array(_)) => string_list(Some(&v)),
        _ => Vec::new(),
    }
}

/// Cuts a string to [`MAX_STR`] bytes at a char boundary.
pub fn cap(mut s: String) -> String {
    if s.len() > MAX_STR {
        let mut end = MAX_STR;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_authors_are_dropped() {
        let authors = [
            "Creator:         www.it-ebooks.info",
            "Creator:         LaTeX with hyperref",
            "Provided By ALLDATASHEET.COM(FREE DATASHEET DOWNLOAD SITE)",
            "www.allitebooks.com",
            "Windows User",
            "Unknown",
            "Magnus Lie Hetland",
            "GoalKicker.com",
            "Сергей Тарасов",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            clean_authors(authors),
            vec!["Magnus Lie Hetland", "GoalKicker.com", "Сергей Тарасов"]
        );
        assert!(!is_junk_author("Userman Smith"));
    }
}
