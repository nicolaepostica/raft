//! Search, filters and facet counts over an in-memory [`Catalog`] (layer 2).
//!
//! Text matching is case- and diacritics-insensitive: every query word must occur
//! (as a substring) in the title, authors, tags, categories or description.
//! Books found only by the full-text content search (passed in as `content_hits`)
//! are added after the metadata matches.

use std::collections::{BTreeMap, HashMap};

use raft_doc::edits::Field;
use raft_doc::{Book, Catalog, Filters, Format, Mark, Query, Sort};
use unicode_normalization::UnicodeNormalization;

/// Longest query text considered, in chars.
pub const MAX_QUERY_CHARS: usize = 256;
/// Most query words considered.
pub const MAX_QUERY_WORDS: usize = 16;

/// Lowercases, removes diacritics (NFD + drop combining marks) and maps `ё` to `е`.
pub fn normalize(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c == 'ё' { 'е' } else { c })
        .collect()
}

fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF)
}

/// Splits a query into normalized words.
pub fn query_words(text: &str) -> Vec<String> {
    let text: String = text.chars().take(MAX_QUERY_CHARS).collect();
    normalize(&text)
        .split(|c: char| !c.is_alphanumeric() && c != '+' && c != '#')
        .filter(|w| !w.is_empty())
        .take(MAX_QUERY_WORDS)
        .map(str::to_owned)
        .collect()
}

/// Most books in one duplicate group; a larger group is a false match (a series
/// sharing a title) and is ignored.
pub const MAX_DUPLICATES: usize = 8;

/// Sets `Book::duplicates`: books with the same ISBN (ISBN-10 and -13 compared as
/// one), or the same title, year and volume (the volume read from the title or the
/// file name: "Том 3", "Vol. 2", "Part IV"…). Books marked `not_duplicate` are left
/// out. Linear in the number of books.
pub fn mark_duplicates(books: &mut [Book]) {
    let n = books.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while let Some(&p) = parent.get(i) {
            if p == i {
                break;
            }
            let grand = parent.get(p).copied().unwrap_or(p);
            if let Some(slot) = parent.get_mut(i) {
                *slot = grand;
            }
            i = grand;
        }
        i
    }
    let mut first: HashMap<String, usize> = HashMap::new();
    for (i, b) in books.iter().enumerate() {
        if b.not_duplicate {
            continue;
        }
        let keys = [
            b.isbn
                .as_deref()
                .and_then(isbn13)
                .map(|k| format!("isbn:{k}")),
            title_key(b),
        ];
        for key in keys.into_iter().flatten() {
            match first.get(&key) {
                Some(&j) => {
                    let (a, c) = (root(&mut parent, i), root(&mut parent, j));
                    if let Some(slot) = parent.get_mut(a) {
                        *slot = c;
                    }
                }
                None => {
                    first.insert(key, i);
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut found: Vec<Vec<String>> = vec![Vec::new(); n];
    for members in groups.values() {
        if members.len() < 2 || members.len() > MAX_DUPLICATES {
            continue;
        }
        for &i in members {
            let others = members
                .iter()
                .filter(|&&j| j != i)
                .filter_map(|&j| books.get(j).map(|b| b.id.clone()))
                .collect();
            if let Some(slot) = found.get_mut(i) {
                *slot = others;
            }
        }
    }
    for (b, d) in books.iter_mut().zip(found) {
        b.duplicates = d;
    }
}

/// An ISBN as 13 digits (an ISBN-10 is converted), or `None` if it is not one.
pub fn isbn13(isbn: &str) -> Option<String> {
    let chars: Vec<char> = isbn
        .chars()
        .take(64)
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    match chars.len() {
        13 if chars.iter().all(char::is_ascii_digit) => Some(chars.into_iter().collect()),
        10 if chars.iter().take(9).all(char::is_ascii_digit) => {
            let body: String = std::iter::once("978".to_owned())
                .chain(chars.iter().take(9).map(char::to_string))
                .collect();
            let sum: u32 = body
                .chars()
                .filter_map(|c| c.to_digit(10))
                .enumerate()
                .map(|(i, d)| if i % 2 == 0 { d } else { d * 3 })
                .sum();
            Some(format!("{body}{}", (10 - sum % 10) % 10))
        }
        _ => None,
    }
}

/// Title, year and volume, normalized; `None` without a title or a year.
fn title_key(b: &Book) -> Option<String> {
    let year = b.year?;
    let words = |s: &str| -> Vec<String> {
        normalize(s)
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let title = words(&b.title).join(" ");
    if title.chars().count() < 3 {
        return None;
    }
    let volume = volume(&words(&b.title)).or_else(|| volume(&words(&b.filename)));
    Some(format!(
        "title:{title}|{year}|{}",
        volume.unwrap_or_default()
    ))
}

/// The volume number after "том", "книга", "часть", "vol", "part", "book"…
fn volume(words: &[String]) -> Option<String> {
    const MARKERS: [&str; 10] = [
        "том",
        "т",
        "книга",
        "часть",
        "выпуск",
        "vol",
        "volume",
        "part",
        "book",
        "tome",
    ];
    words.windows(2).find_map(|w| match w {
        [m, v] if MARKERS.contains(&m.as_str()) && is_number(v) => Some(v.clone()),
        _ => None,
    })
}

/// Digits or a Roman numeral.
fn is_number(w: &str) -> bool {
    !w.is_empty()
        && (w.chars().all(|c| c.is_ascii_digit())
            || w.chars().all(|c| matches!(c, 'i' | 'v' | 'x' | 'l' | 'c')))
}

/// Normalized fields of one book, ready for matching.
#[derive(Debug, Clone)]
struct Entry {
    title: String,
    authors: String,
    tags: String,
    description: String,
}

/// Precomputed search index; build once per catalog.
#[derive(Debug, Clone, Default)]
pub struct Index {
    entries: Vec<Entry>,
}

impl Index {
    pub fn new(catalog: &Catalog) -> Self {
        let entries = catalog
            .books
            .iter()
            .map(|b| Entry {
                title: normalize(&format!("{} {}", b.title, b.filename)),
                authors: normalize(&b.authors.join(" ")),
                tags: normalize(&format!("{} {}", b.tags.join(" "), b.categories.join(" "))),
                description: normalize(&b.description),
            })
            .collect();
        Self { entries }
    }

    /// Relevance of book `i` for `words`, or `None` if some word does not match.
    fn score(&self, i: usize, words: &[String]) -> Option<u32> {
        let e = self.entries.get(i)?;
        let mut total = 0;
        for w in words {
            let mut s = 0;
            if e.title.contains(w.as_str()) {
                s += 8;
            }
            if e.authors.contains(w.as_str()) {
                s += 5;
            }
            if e.tags.contains(w.as_str()) {
                s += 3;
            }
            if e.description.contains(w.as_str()) {
                s += 1;
            }
            if s == 0 {
                return None;
            }
            total += s;
        }
        Some(total)
    }
}

/// Which facet to leave out when counting (so a facet's own options stay visible).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Skip {
    None,
    Category,
    Language,
    Format,
    /// Marks and missing fields.
    Marks,
    Source,
}

fn passes(book: &Book, f: &Filters, skip: Skip) -> bool {
    if skip != Skip::Source && !f.sources.is_empty() && !f.sources.contains(&book.source) {
        return false;
    }
    if skip != Skip::Marks {
        if !f.marks.iter().all(|m| book.has(*m)) {
            return false;
        }
        if !f.missing.is_empty() && !f.missing.iter().any(|field| book.lacks(*field)) {
            return false;
        }
    }
    if skip != Skip::Category && !f.categories.is_empty() {
        let ok = if f.primary_category_only {
            book.primary_category()
                .is_some_and(|c| f.categories.contains(c))
        } else {
            book.categories.iter().any(|c| f.categories.contains(c))
        };
        if !ok {
            return false;
        }
    }
    if skip != Skip::Language && !f.languages.is_empty() && !f.languages.contains(&book.language) {
        return false;
    }
    if skip != Skip::Format && !f.formats.is_empty() && !f.formats.contains(&book.format) {
        return false;
    }
    if f.year_min.is_some() || f.year_max.is_some() {
        let Some(y) = book.year else {
            return false;
        };
        if f.year_min.is_some_and(|m| y < m) || f.year_max.is_some_and(|m| y > m) {
            return false;
        }
    }
    true
}

/// Text relevance for every book: `None` = no match. Empty text matches everything with 0.
fn text_scores(
    catalog: &Catalog,
    index: &Index,
    text: &str,
    content_hits: &[String],
) -> Vec<Option<u32>> {
    let words = query_words(text);
    if words.is_empty() {
        return vec![Some(0); catalog.books.len()];
    }
    let content: HashMap<&str, usize> = content_hits
        .iter()
        .enumerate()
        .map(|(rank, id)| (id.as_str(), rank))
        .collect();
    catalog
        .books
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let meta = index.score(i, &words).map(|s| s + 100);
            // Content-only matches rank below every metadata match, in FTS order.
            let by_content = content.get(b.id.as_str()).map(|rank| {
                let r = u32::try_from(*rank).unwrap_or(u32::MAX);
                99u32.saturating_sub(r.min(99))
            });
            match (meta, by_content) {
                (Some(m), Some(c)) => Some(m + c / 10),
                (m, c) => m.or(c),
            }
        })
        .collect()
}

/// Indices into `catalog.books` matching `query`, ordered by `query.sort`.
pub fn search(
    catalog: &Catalog,
    index: &Index,
    query: &Query,
    content_hits: &[String],
) -> Vec<usize> {
    let scores = text_scores(catalog, index, &query.text, content_hits);
    let mut hits: Vec<(usize, u32)> = catalog
        .books
        .iter()
        .zip(&scores)
        .enumerate()
        .filter_map(|(i, (b, s))| {
            s.filter(|_| passes(b, &query.filters, Skip::None))
                .map(|s| (i, s))
        })
        .collect();
    let book = |i: usize| catalog.books.get(i);
    let title = |i: usize| {
        book(i)
            .map(|b| normalize(b.display_title()))
            .unwrap_or_default()
    };
    let year = |i: usize| book(i).and_then(|b| b.year);
    match query.sort {
        Sort::Relevance if !query_words(&query.text).is_empty() => {
            hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| title(a.0).cmp(&title(b.0))));
        }
        Sort::Relevance | Sort::Title => hits.sort_by_cached_key(|h| title(h.0)),
        Sort::YearDesc => hits.sort_by_key(|h| (std::cmp::Reverse(year(h.0)), title(h.0))),
        Sort::YearAsc => hits.sort_by_key(|h| (year(h.0).is_none(), year(h.0), title(h.0))),
        Sort::RecentlyAdded => {
            hits.sort_by_key(|h| std::cmp::Reverse(book(h.0).and_then(|b| b.added_at.clone())))
        }
    }
    hits.into_iter().map(|(i, _)| i).collect()
}

/// Number of matching books per option of each facet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facets {
    /// (slug, count), most frequent first.
    pub categories: Vec<(String, usize)>,
    pub languages: Vec<(String, usize)>,
    pub formats: Vec<(Format, usize)>,
    /// Year range among the matching books (ignoring the year filter itself).
    pub years: Option<(i32, i32)>,
    /// Books with each mark (ignoring the mark and missing-field filters).
    pub marks: Vec<(Mark, usize)>,
    /// Books lacking each field, in form order (same filters as `marks`).
    pub missing: Vec<(Field, usize)>,
    /// Books per library folder (ignoring the source filter).
    pub sources: Vec<(String, usize)>,
}

/// Facet counts for `query`: each facet is counted with every other filter applied.
pub fn facets(catalog: &Catalog, index: &Index, query: &Query, content_hits: &[String]) -> Facets {
    let scores = text_scores(catalog, index, &query.text, content_hits);
    let f = &query.filters;
    let mut cats: BTreeMap<String, usize> = BTreeMap::new();
    let mut langs: BTreeMap<String, usize> = BTreeMap::new();
    let mut formats: BTreeMap<Format, usize> = BTreeMap::new();
    let mut years: Option<(i32, i32)> = None;
    let mut marks: BTreeMap<Mark, usize> = BTreeMap::new();
    let mut missing: BTreeMap<Field, usize> = BTreeMap::new();
    let mut sources: BTreeMap<String, usize> = BTreeMap::new();
    let no_years = Filters {
        year_min: None,
        year_max: None,
        ..f.clone()
    };
    for (b, s) in catalog.books.iter().zip(&scores) {
        if s.is_none() {
            continue;
        }
        if passes(b, f, Skip::Category) {
            if f.primary_category_only {
                if let Some(c) = b.primary_category() {
                    *cats.entry(c.to_owned()).or_default() += 1;
                }
            } else {
                for c in &b.categories {
                    *cats.entry(c.clone()).or_default() += 1;
                }
            }
        }
        if passes(b, f, Skip::Language) && !b.language.is_empty() {
            *langs.entry(b.language.clone()).or_default() += 1;
        }
        if passes(b, f, Skip::Format) {
            *formats.entry(b.format).or_default() += 1;
        }
        if passes(b, f, Skip::Source) {
            *sources.entry(b.source.clone()).or_default() += 1;
        }
        if passes(b, f, Skip::Marks) {
            for m in Mark::ALL.into_iter().filter(|m| b.has(*m)) {
                *marks.entry(m).or_default() += 1;
            }
            for field in b.missing() {
                *missing.entry(field).or_default() += 1;
            }
        }
        if let Some(y) = b.year
            && passes(b, &no_years, Skip::None)
        {
            years = Some(years.map_or((y, y), |(lo, hi)| (lo.min(y), hi.max(y))));
        }
    }
    let by_count = |m: BTreeMap<String, usize>| {
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    };
    let mut formats: Vec<_> = formats.into_iter().collect();
    formats.sort_by_key(|a| std::cmp::Reverse(a.1));
    Facets {
        categories: by_count(cats),
        languages: by_count(langs),
        formats,
        years,
        marks: marks.into_iter().collect(),
        missing: missing.into_iter().collect(),
        sources: sources.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn book(
        id: &str,
        title: &str,
        cats: &[&str],
        lang: &str,
        year: Option<i32>,
        f: Format,
    ) -> Book {
        Book {
            id: id.into(),
            title: title.into(),
            filename: format!("{id}.{}", f.slug()),
            categories: cats.iter().map(|s| (*s).to_owned()).collect(),
            language: lang.into(),
            year,
            format: f,
            ..Book::default()
        }
    }

    fn sample() -> Catalog {
        Catalog {
            books: vec![
                book(
                    "1",
                    "Fluent Python",
                    &["python"],
                    "en",
                    Some(2015),
                    Format::Pdf,
                ),
                book(
                    "2",
                    "Изучаем Python",
                    &["python", "career"],
                    "ru",
                    Some(2019),
                    Format::Pdf,
                ),
                book(
                    "3",
                    "Kubernetes in Action",
                    &["devops", "cloud"],
                    "en",
                    Some(2018),
                    Format::Epub,
                ),
                book("4", "Ёжик в тумане", &["fiction"], "ru", None, Format::Fb2),
            ],
            categories: vec![],
        }
    }

    fn q(text: &str) -> Query {
        Query {
            text: text.into(),
            ..Query::default()
        }
    }

    #[test]
    fn filters_and_counts_sources() {
        let mut c = sample();
        c.books[0].source = "/a".into();
        c.books[1].source = "/b".into();
        c.books[2].source = "/b".into();
        c.books[3].source = "/a".into();
        let i = Index::new(&c);
        let mut query = Query::default();
        query.filters.sources.insert("/b".into());
        assert_eq!(search(&c, &i, &query, &[]), vec![2, 1]);
        let f = facets(&c, &i, &query, &[]);
        assert_eq!(f.sources, vec![("/a".into(), 2), ("/b".into(), 2)]);
        assert!(!query.filters.is_empty());
        let mut merged = Catalog::default();
        merged.merge(c.clone());
        merged.merge(c);
        assert_eq!(merged.books.len(), 4);
    }

    fn dup_book(id: &str, title: &str, year: Option<i32>, isbn: Option<&str>, file: &str) -> Book {
        Book {
            id: id.into(),
            title: title.into(),
            year,
            isbn: isbn.map(str::to_owned),
            filename: file.into(),
            ..Book::default()
        }
    }

    #[test]
    fn duplicates_by_isbn_and_by_title_year_and_volume() {
        let mut books = vec![
            dup_book(
                "a",
                "JavaScript: сильные стороны",
                Some(2012),
                Some("978-5-459-01263-7"),
                "a.pdf",
            ),
            dup_book(
                "b",
                "JavaScript: Сильные стороны",
                Some(2012),
                Some("9785459012637"),
                "b.pdf",
            ),
            // ISBN-10 of the same book as c.
            dup_book(
                "c",
                "Effective C++",
                Some(2006),
                Some("5-94074-304-8"),
                "c.djvu",
            ),
            dup_book(
                "d",
                "Эффективное использование C++",
                Some(2006),
                Some("978-5-94074-304-0"),
                "d.pdf",
            ),
            // Same title and year, told apart by the volume in the file name.
            dup_book(
                "e",
                "Имя нам Легион",
                Some(2025),
                None,
                "Имя_нам_Легион_Том_3_2025.fb2",
            ),
            dup_book(
                "f",
                "Имя нам Легион",
                Some(2025),
                None,
                "Имя_нам_Легион_Том_8_2025.fb2",
            ),
            dup_book(
                "g",
                "Чистый код. Создание, анализ и рефакторинг",
                Some(2013),
                None,
                "g.pdf",
            ),
            dup_book(
                "h",
                "Чистый код: создание, анализ и рефакторинг",
                Some(2013),
                Some("978-5-496-00487-9"),
                "h.pdf",
            ),
            // No year: never matched by title.
            dup_book("i", "Untitled", None, None, "i.pdf"),
            dup_book("j", "Untitled", None, None, "j.pdf"),
        ];
        mark_duplicates(&mut books);
        fn dups(books: &[Book], id: &str) -> Vec<String> {
            books
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.duplicates.clone())
                .unwrap_or_default()
        }
        assert_eq!(dups(&books, "a"), ["b"]);
        assert_eq!(dups(&books, "b"), ["a"]);
        assert_eq!(dups(&books, "c"), ["d"]);
        assert!(dups(&books, "e").is_empty());
        assert!(dups(&books, "f").is_empty());
        assert_eq!(dups(&books, "g"), ["h"]);
        assert!(dups(&books, "i").is_empty());
        assert!(
            books
                .iter()
                .find(|b| b.id == "a")
                .is_some_and(|b| b.has(Mark::Duplicate))
        );

        // Marked by hand as not a duplicate: out of every group.
        if let Some(b) = books.iter_mut().find(|b| b.id == "b") {
            b.not_duplicate = true;
        }
        mark_duplicates(&mut books);
        assert!(dups(&books, "a").is_empty());
        assert!(dups(&books, "b").is_empty());
    }

    #[test]
    fn isbn_forms() {
        assert_eq!(isbn13("5-94074-304-8").as_deref(), Some("9785940743040"));
        assert_eq!(isbn13("0-306-40615-2").as_deref(), Some("9780306406157"));
        assert_eq!(
            isbn13("978-0-306-40615-7").as_deref(),
            Some("9780306406157")
        );
        assert_eq!(isbn13("123"), None);
        assert_eq!(isbn13("978-0-306-40615-X"), None);
    }

    proptest::proptest! {
        #[test]
        fn duplicate_detection_never_panics(
            titles in proptest::collection::vec(".{0,40}", 0..30),
            isbns in proptest::collection::vec(proptest::option::of("[0-9Xx -]{0,20}"), 0..30),
        ) {
            let mut books: Vec<Book> = titles
                .iter()
                .enumerate()
                .map(|(i, t)| Book {
                    id: i.to_string(),
                    title: t.clone(),
                    year: Some(2000 + (i % 3) as i32),
                    isbn: isbns.get(i).cloned().flatten(),
                    filename: format!("{t}.pdf"),
                    ..Book::default()
                })
                .collect();
            mark_duplicates(&mut books);
            for b in &books {
                proptest::prop_assert!(!b.duplicates.contains(&b.id));
                proptest::prop_assert!(b.duplicates.len() < MAX_DUPLICATES);
            }
        }
    }

    #[test]
    fn filters_and_counts_marks() {
        let mut c = sample();
        for b in &mut c.books {
            b.authors = vec!["A".into()];
            b.description = "d".into();
        }
        c.books[1].edited = true;
        c.books[3].reviewed = true;
        let i = Index::new(&c);
        let mut query = Query::default();
        query.filters.marks.insert(Mark::Incomplete);
        // Only book 4 lacks the year, and it was reviewed.
        assert!(search(&c, &i, &query, &[]).is_empty());
        c.books[3].reviewed = false;
        assert_eq!(search(&c, &i, &query, &[]), vec![3]);
        query.filters.marks.insert(Mark::Edited);
        assert!(search(&c, &i, &query, &[]).is_empty());
        let f = facets(&c, &i, &query, &[]);
        assert_eq!(f.marks, vec![(Mark::Edited, 1), (Mark::Incomplete, 1)]);
        assert!(f.missing.contains(&(Field::Year, 1)));
        assert!(f.missing.contains(&(Field::Isbn, 4)));
        let mut query = Query::default();
        query.filters.missing.insert(Field::Year);
        assert_eq!(search(&c, &i, &query, &[]), vec![3]);
        assert!(!query.filters.is_empty());
    }

    #[test]
    fn normalizes_case_and_diacritics() {
        assert_eq!(normalize("Ștefan Ëжик Ё"), "stefan eжик е");
        assert_eq!(normalize("Йога"), "иога");
    }

    #[test]
    fn text_search_matches_all_words() {
        let c = sample();
        let i = Index::new(&c);
        assert_eq!(search(&c, &i, &q("python"), &[]), vec![0, 1]);
        assert_eq!(search(&c, &i, &q("fluent PYTHON"), &[]), vec![0]);
        assert_eq!(search(&c, &i, &q("ежик"), &[]), vec![3]);
        assert!(search(&c, &i, &q("python kubernetes"), &[]).is_empty());
    }

    #[test]
    fn content_hits_come_after_metadata_hits() {
        let c = sample();
        let i = Index::new(&c);
        let r = search(&c, &i, &q("python"), &["3".to_owned()]);
        assert_eq!(r, vec![0, 1, 2]);
    }

    #[test]
    fn filters_and_facets() {
        let c = sample();
        let i = Index::new(&c);
        let mut query = q("");
        query.filters.languages.insert("ru".into());
        assert_eq!(search(&c, &i, &query, &[]), vec![3, 1]);
        let f = facets(&c, &i, &query, &[]);
        // Language facet ignores its own filter.
        assert_eq!(f.languages, vec![("en".into(), 2), ("ru".into(), 2)]);
        assert_eq!(
            f.categories,
            vec![
                ("career".into(), 1),
                ("fiction".into(), 1),
                ("python".into(), 1)
            ]
        );
        assert_eq!(f.years, Some((2019, 2019)));

        query.filters.year_min = Some(2016);
        assert_eq!(search(&c, &i, &query, &[]), vec![1]);
    }

    #[test]
    fn primary_category_only() {
        let c = sample();
        let i = Index::new(&c);
        let mut query = q("");
        query.filters.categories.insert("career".into());
        assert_eq!(search(&c, &i, &query, &[]).len(), 1);
        query.filters.primary_category_only = true;
        assert!(search(&c, &i, &query, &[]).is_empty());
    }

    #[test]
    fn sorts_by_year() {
        let c = sample();
        let i = Index::new(&c);
        let mut query = q("");
        query.sort = Sort::YearDesc;
        assert_eq!(search(&c, &i, &query, &[]), vec![1, 2, 0, 3]);
        query.sort = Sort::YearAsc;
        assert_eq!(search(&c, &i, &query, &[]), vec![0, 2, 1, 3]);
    }

    proptest! {
        #[test]
        fn search_never_panics(text in ".{0,400}", year in proptest::option::of(any::<i32>())) {
            let c = sample();
            let i = Index::new(&c);
            let mut query = q(&text);
            query.filters.year_min = year;
            let r = search(&c, &i, &query, &["4".into(), "zz".into()]);
            prop_assert!(r.iter().all(|&ix| ix < c.books.len()));
            let _ = facets(&c, &i, &query, &[]);
        }
    }
}
