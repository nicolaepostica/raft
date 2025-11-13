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
