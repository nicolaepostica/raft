//! SQLite storage (layer 3, native only).
//!
//! - [`CatalogDb`]: `catalog.db` built by `build-catalog`, opened read-only.
//! - [`UserDb`]: `raft.db`, the app's own data (reading progress), kept apart so
//!   regenerating the catalog never loses it. Books are linked by `book_id` (md5).

use std::path::Path;

use raft_catalog::{BookParts, book_from_parts, json_list};
use raft_doc::{BookId, Catalog, Category, Progress};
use rusqlite::{Connection, OpenFlags, params};

/// Most books loaded from the catalog.
pub const MAX_BOOKS: usize = raft_catalog::MAX_BOOKS;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
}

/// The read-only catalog database.
pub struct CatalogDb {
    conn: Connection,
}

impl CatalogDb {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        Ok(Self { conn })
    }

    /// Wraps an existing connection (tests).
    pub fn from_connection(conn: Connection) -> Self {
        Self { conn }
    }

    /// Loads every book and category.
    pub fn load(&self) -> Result<Catalog, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, filename, format, size, pages, title, authors, year, language,
                    publisher, isbn, categories, tags, description, cover, added_at
             FROM books",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(BookParts {
                id: r.get(0)?,
                path: r.get(1)?,
                filename: r.get(2)?,
                format: r.get(3)?,
                size: r
                    .get::<_, Option<i64>>(4)?
                    .and_then(|n| u64::try_from(n).ok()),
                pages: r
                    .get::<_, Option<i64>>(5)?
                    .and_then(|n| u32::try_from(n).ok()),
                title: r.get(6)?,
                authors: json_list(&r.get::<_, Option<String>>(7)?.unwrap_or_default()),
                year: r
                    .get::<_, Option<i64>>(8)?
                    .and_then(|n| i32::try_from(n).ok()),
                language: r.get(9)?,
                publisher: r.get(10)?,
                isbn: r.get(11)?,
                categories: json_list(&r.get::<_, Option<String>>(12)?.unwrap_or_default()),
                tags: json_list(&r.get::<_, Option<String>>(13)?.unwrap_or_default()),
                description: r.get(14)?,
                cover: r.get(15)?,
                added_at: r.get(16)?,
            })
        })?;
        let mut books = Vec::new();
        for row in rows.take(MAX_BOOKS) {
            // A row with a wrongly typed column is skipped, not fatal.
            if let Some(book) = row.ok().and_then(book_from_parts) {
                books.push(book);
            }
        }
        let mut stmt = self.conn.prepare("SELECT slug, name FROM categories")?;
        let categories = stmt
            .query_map([], |r| {
                Ok(Category {
                    slug: r.get(0)?,
                    name: r.get(1)?,
                })
            })?
            .filter_map(Result::ok)
            .take(raft_catalog::MAX_CATEGORIES)
            .collect();
        Ok(Catalog { books, categories })
    }

    /// Full-text search over title, authors, tags, description and the first pages.
    pub fn content_search(&self, text: &str, limit: usize) -> Result<Vec<BookId>, StoreError> {
        let Some(fts) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare_cached(
            "SELECT id FROM books_fts WHERE books_fts MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let ids = stmt
            .query_map(params![fts, limit], |r| r.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();
        Ok(ids)
    }
}

/// Builds a safe FTS5 query: every word quoted, as a prefix, all required.
pub fn fts_query(text: &str) -> Option<String> {
    let words = raft_search::query_words(text);
    if words.is_empty() {
        return None;
    }
    Some(
        words
            .iter()
            .map(|w| format!("\"{}\"*", w.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The app's own database.
pub struct UserDb {
    conn: Connection,
}

impl UserDb {
    /// Opens (creating if needed) and migrates `raft.db`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS progress(
                 book_id TEXT PRIMARY KEY,
                 position TEXT,
                 page INTEGER,
                 percent REAL NOT NULL DEFAULT 0,
                 updated_at INTEGER NOT NULL
             );",
        )?;
        Ok(Self { conn })
    }

    pub fn load_progress(&self) -> Result<Vec<Progress>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT book_id, position, page, percent, updated_at FROM progress")?;
        let rows = stmt
            .query_map([], |r| {
                let percent: f64 = r.get(3)?;
                Ok(Progress {
                    book_id: r.get(0)?,
                    position: r.get(1)?,
                    page: r
                        .get::<_, Option<i64>>(2)?
                        .and_then(|n| u32::try_from(n).ok()),
                    percent: if percent.is_finite() {
                        percent.clamp(0.0, 100.0) as f32
                    } else {
                        0.0
                    },
                    updated_at: r.get(4)?,
                })
            })?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn save_progress(&self, p: &Progress) -> Result<(), StoreError> {
        let percent = if p.percent.is_finite() {
            f64::from(p.percent)
        } else {
            0.0
        };
        self.conn.execute(
            "INSERT INTO progress(book_id, position, page, percent, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(book_id) DO UPDATE SET position = ?2, page = ?3, percent = ?4,
                 updated_at = ?5",
            params![p.book_id, p.position, p.page, percent, p.updated_at],
        )?;
        Ok(())
    }

    pub fn delete_progress(&self, book_id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM progress WHERE book_id = ?1", params![book_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn catalog_db() -> CatalogDb {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"CREATE TABLE books(id TEXT PRIMARY KEY, path TEXT, filename TEXT, format TEXT,
                 size INTEGER, pages INTEGER, title TEXT, authors TEXT, year INTEGER,
                 language TEXT, publisher TEXT, isbn TEXT, categories TEXT, tags TEXT,
                 description TEXT, cover TEXT, mtime REAL, added_at TEXT, updated_at TEXT);
               CREATE VIRTUAL TABLE books_fts USING fts5(id UNINDEXED, title, authors,
                 categories, tags, description, content,
                 tokenize="unicode61 remove_diacritics 2");
               CREATE TABLE categories(slug TEXT PRIMARY KEY, name TEXT);
               INSERT INTO categories VALUES ('python', 'Python');
               INSERT INTO books(id, path, filename, format, size, pages, title, authors, year,
                 language, categories, tags, description, cover)
                 VALUES ('a', 'A_2020.pdf', 'A_2020.pdf', 'pdf', 5, 12, 'Asyncio', '["Ann"]',
                   2020, 'en', '["python"]', '["async"]', 'Despre asyncio.', 'covers/a.webp'),
                        ('b', 'B.fb2', 'B.fb2', 'fb2', 'not a number', NULL, 'Б', 'garbage',
                   NULL, 'ru', NULL, NULL, NULL, NULL);
               INSERT INTO books_fts(id, title, content) VALUES ('a', 'Asyncio', 'event loop coroutines'),
                 ('b', 'Б', 'Ёлка и ёжик');"#,
        )
        .unwrap();
        CatalogDb::from_connection(conn)
    }

    #[test]
    fn loads_books_and_tolerates_bad_columns() {
        let c = catalog_db().load().unwrap();
        assert_eq!(c.books.len(), 2);
        let a = c.book("a").unwrap();
        assert_eq!(a.authors, vec!["Ann"]);
        assert_eq!(a.pages, Some(12));
        let b = c.book("b").unwrap();
        assert!(b.authors.is_empty());
        assert_eq!(b.size, 0);
        assert_eq!(c.categories.len(), 1);
    }

    #[test]
    fn content_search_by_prefix_and_without_diacritics() {
        let db = catalog_db();
        assert_eq!(db.content_search("corout", 10).unwrap(), vec!["a"]);
        assert_eq!(db.content_search("ежик", 10).unwrap(), vec!["b"]);
        assert!(db.content_search("   ", 10).unwrap().is_empty());
    }

    #[test]
    fn progress_round_trip() {
        let db = UserDb::in_memory().unwrap();
        let mut p = Progress {
            book_id: "a".into(),
            page: Some(3),
            position: None,
            percent: 25.0,
            updated_at: 10,
        };
        db.save_progress(&p).unwrap();
        p.page = Some(4);
        p.percent = f32::NAN;
        db.save_progress(&p).unwrap();
        let all = db.load_progress().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].page, Some(4));
        assert!(all[0].percent.abs() < f32::EPSILON);
        db.delete_progress("a").unwrap();
        assert!(db.load_progress().unwrap().is_empty());
    }

    proptest! {
        #[test]
        fn fts_queries_never_fail(text in ".{0,200}") {
            let db = catalog_db();
            prop_assert!(db.content_search(&text, 50).is_ok());
        }
    }

    #[test]
    fn real_catalog_if_present() {
        let path = Path::new("/backup/books/catalog.db");
        if !path.exists() {
            return;
        }
        let db = CatalogDb::open(path).unwrap();
        let c = db.load().unwrap();
        assert!(c.books.len() > 100);
        assert!(!db.content_search("python", 20).unwrap().is_empty());
    }
}
