//! SQLite storage (layer 3, native only).
//!
//! - [`CatalogDb`]: `catalog.db` built by `build-catalog`, opened read-only.
//! - [`UserDb`]: `raft.db`, the app's own data (reading progress), kept apart so
//!   regenerating the catalog never loses it. Books are linked by `book_id` (md5).

use std::path::Path;

use raft_doc::edits::{Field, Override};
use raft_doc::sanitize::{BookParts, book_from_parts, json_list};
use raft_doc::{Book, BookId, Catalog, Category, Progress, Source};
use rusqlite::{Connection, OpenFlags, params};

/// Most books loaded from the catalog.
pub const MAX_BOOKS: usize = 200_000;

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
        // Columns are read loosely: a wrongly typed value becomes "missing".
        let rows = stmt.query_map([], |r| {
            let text = |i: usize| -> Option<String> {
                r.get_ref(i).ok()?.as_str().ok().map(str::to_owned)
            };
            let int = |i: usize| -> Option<i64> { r.get_ref(i).ok()?.as_i64().ok() };
            let list = |i: usize| json_list(&text(i).unwrap_or_default());
            Ok(BookParts {
                id: text(0),
                path: text(1),
                filename: text(2),
                format: text(3),
                size: int(4).and_then(|n| u64::try_from(n).ok()),
                pages: int(5).and_then(|n| u32::try_from(n).ok()),
                title: text(6),
                authors: list(7),
                year: int(8).and_then(|n| i32::try_from(n).ok()),
                language: text(9),
                publisher: text(10),
                isbn: text(11),
                categories: list(12),
                tags: list(13),
                description: text(14),
                cover: text(15),
                added_at: text(16),
            })
        })?;
        let mut books = Vec::new();
        for row in rows.take(MAX_BOOKS) {
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
            .take(1024)
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
///
/// FTS5 folds Latin diacritics only, so each word is searched both as typed
/// (`ёжик`) and normalized (`ежик`).
pub fn fts_query(text: &str) -> Option<String> {
    let text: String = text.chars().take(raft_search::MAX_QUERY_CHARS).collect();
    let terms: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(raft_search::MAX_QUERY_WORDS)
        .map(|raw| {
            let norm: String = raft_search::normalize(raw)
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            if norm.is_empty() || norm == raw {
                format!("\"{raw}\"*")
            } else {
                format!("(\"{raw}\"* OR \"{norm}\"*)")
            }
        })
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
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
             );
             CREATE TABLE IF NOT EXISTS overrides(
                 book_id TEXT NOT NULL,
                 field TEXT NOT NULL,
                 value TEXT NOT NULL,
                 updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                 PRIMARY KEY (book_id, field)
             );
             CREATE TABLE IF NOT EXISTS sources(
                 path TEXT PRIMARY KEY,
                 added_at INTEGER NOT NULL,
                 name TEXT NOT NULL DEFAULT '',
                 enabled INTEGER NOT NULL DEFAULT 1
             );
             CREATE TABLE IF NOT EXISTS settings(
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS files(
                 book_id TEXT PRIMARY KEY,
                 path TEXT NOT NULL,
                 title TEXT NOT NULL,
                 size INTEGER NOT NULL DEFAULT 0,
                 pages INTEGER,
                 opened_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS removed(
                 book_id TEXT PRIMARY KEY,
                 path TEXT NOT NULL,
                 removed_at INTEGER NOT NULL
             );",
        )?;
        // Databases made before sources had a name and an on/off switch.
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('sources')")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        if !columns.iter().any(|c| c == "name") {
            conn.execute_batch("ALTER TABLE sources ADD COLUMN name TEXT NOT NULL DEFAULT ''")?;
        }
        if !columns.iter().any(|c| c == "enabled") {
            conn.execute_batch(
                "ALTER TABLE sources ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1",
            )?;
        }
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

    /// Every stored metadata correction; rows with an unknown field are skipped.
    pub fn load_overrides(&self) -> Result<Vec<Override>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT book_id, field, value FROM overrides ORDER BY book_id, field")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .filter_map(Result::ok)
            .filter_map(|(book_id, field, value)| {
                Some(Override {
                    book_id,
                    field: Field::from_slug(&field)?,
                    value,
                })
            })
            .collect();
        Ok(rows)
    }

    pub fn save_override(&self, o: &Override) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO overrides(book_id, field, value, updated_at)
             VALUES (?1, ?2, ?3, strftime('%s', 'now'))
             ON CONFLICT(book_id, field) DO UPDATE SET value = ?3, updated_at = strftime('%s', 'now')",
            params![o.book_id, o.field.slug(), o.value],
        )?;
        Ok(())
    }

    pub fn delete_override(&self, book_id: &str, field: Field) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM overrides WHERE book_id = ?1 AND field = ?2",
            params![book_id, field.slug()],
        )?;
        Ok(())
    }

    /// Library folders, in the order they were added.
    pub fn load_sources(&self) -> Result<Vec<Source>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, name, enabled FROM sources ORDER BY added_at, rowid")?;
        let rows = stmt.query_map([], |r| {
            Ok(Source {
                path: r.get(0)?,
                name: r.get(1)?,
                enabled: r.get::<_, i64>(2)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Remembers a library folder, or updates its name and switch (it keeps its place).
    pub fn save_source(&self, source: &Source, at: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO sources(path, added_at, name, enabled) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET name = excluded.name, enabled = excluded.enabled",
            params![source.path, at, source.name, i64::from(source.enabled)],
        )?;
        Ok(())
    }

    /// Points a remembered folder to a new path, keeping its place in the list.
    pub fn move_source(&self, from: &str, to: &str) -> Result<(), StoreError> {
        self.conn
            .execute("UPDATE sources SET path = ?2 WHERE path = ?1", [from, to])?;
        Ok(())
    }

    pub fn delete_source(&self, path: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM sources WHERE path = ?1", [path])?;
        Ok(())
    }

    /// Book files opened from outside the library, most recently opened first.
    pub fn load_files(&self) -> Result<Vec<Book>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT book_id, path, title, size, pages FROM files ORDER BY opened_at DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })?;
        let mut books = Vec::new();
        for row in rows {
            let (id, path, title, size, pages) = row?;
            let pages = pages.and_then(|p| u32::try_from(p).ok());
            // A row that no longer makes a book (edited by hand) is skipped.
            if let Some(mut b) = Book::outside(&path, id, u64::try_from(size).unwrap_or(0), pages) {
                if !title.trim().is_empty() {
                    b.title = title;
                }
                books.push(b);
            }
        }
        Ok(books)
    }

    /// Remembers a book file opened from outside the library (again: moves it first).
    pub fn save_file(&self, book: &Book, at: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO files(book_id, path, title, size, pages, opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                book.id,
                book.file_path(),
                book.title,
                i64::try_from(book.size).unwrap_or(i64::MAX),
                book.pages.map(i64::from),
                at
            ],
        )?;
        Ok(())
    }

    pub fn delete_file(&self, book_id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM files WHERE book_id = ?1", [book_id])?;
        Ok(())
    }

    /// A saved preference (`theme` …), if set.
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn save_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)",
            [key, value],
        )?;
        Ok(())
    }

    /// Records a book removed from the library (its file went to the trash).
    pub fn save_removed(&self, book_id: &str, path: &str, at: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO removed(book_id, path, removed_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![book_id, path, at],
        )?;
        Ok(())
    }

    pub fn delete_removed(&self, book_id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM removed WHERE book_id = ?1", [book_id])?;
        Ok(())
    }

    /// Ids of the removed books, hidden even while the catalog still lists them.
    pub fn load_removed(&self) -> Result<Vec<BookId>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT book_id FROM removed")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
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
        assert_eq!(db.content_search("Ёжик", 10).unwrap(), vec!["b"]);
        assert_eq!(db.content_search("asyncio", 10).unwrap(), vec!["a"]);
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

    #[test]
    fn sources_round_trip_in_order() {
        let db = UserDb::in_memory().unwrap();
        db.save_source(&Source::new("/backup/books"), 5).unwrap();
        db.save_source(&Source::new("/disk2"), 7).unwrap();
        let renamed = Source {
            name: "Old disk".into(),
            enabled: false,
            ..Source::new("/backup/books")
        };
        db.save_source(&renamed, 9).unwrap();
        assert_eq!(
            db.load_sources().unwrap(),
            vec![renamed.clone(), Source::new("/disk2")]
        );
        db.move_source("/backup/books", "/mnt/books").unwrap();
        assert_eq!(
            db.load_sources().unwrap().first().map(|s| s.path.as_str()),
            Some("/mnt/books")
        );
        db.delete_source("/mnt/books").unwrap();
        assert_eq!(db.load_sources().unwrap(), vec![Source::new("/disk2")]);
    }

    #[test]
    fn old_sources_table_gets_the_new_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sources(path TEXT PRIMARY KEY, added_at INTEGER NOT NULL);
             INSERT INTO sources VALUES ('/backup/books', 1);",
        )
        .unwrap();
        let db = UserDb::init(conn).unwrap();
        assert_eq!(
            db.load_sources().unwrap(),
            vec![Source::new("/backup/books")]
        );
    }

    #[test]
    fn opened_files_round_trip() {
        let db = UserDb::in_memory().unwrap();
        let a = Book::outside("/home/u/Clean_Code_2008.pdf", "a".into(), 10, Some(400)).unwrap();
        let mut b = Book::outside("/tmp/x.epub", "b".into(), 5, None).unwrap();
        b.title = "Dune".into();
        db.save_file(&a, 1).unwrap();
        db.save_file(&b, 2).unwrap();
        let ids = |db: &UserDb| {
            db.load_files()
                .unwrap()
                .into_iter()
                .map(|b| b.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&db), ["b", "a"]);
        db.save_file(&a, 3).unwrap();
        assert_eq!(ids(&db), ["a", "b"]);
        let loaded = db.load_files().unwrap();
        assert_eq!(loaded.first(), Some(&a));
        assert_eq!(loaded.get(1).map(|b| b.title.as_str()), Some("Dune"));
        db.conn
            .execute(
                "INSERT INTO files VALUES ('c', 'not/absolute.pdf', '', 0, NULL, 4)",
                [],
            )
            .unwrap();
        assert_eq!(ids(&db), ["a", "b"]);
        db.delete_file("a").unwrap();
        assert_eq!(ids(&db), ["b"]);
    }

    #[test]
    fn settings_round_trip() {
        let db = UserDb::in_memory().unwrap();
        assert_eq!(db.setting("theme").unwrap(), None);
        db.save_setting("theme", "dark").unwrap();
        db.save_setting("theme", "light").unwrap();
        assert_eq!(db.setting("theme").unwrap().as_deref(), Some("light"));
    }

    #[test]
    fn removed_books_round_trip() {
        let db = UserDb::in_memory().unwrap();
        db.save_removed("a", "x/a.pdf", 1).unwrap();
        db.save_removed("a", "x/a.pdf", 2).unwrap();
        db.save_removed("b", "b.pdf", 3).unwrap();
        let mut ids = db.load_removed().unwrap();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
        db.delete_removed("a").unwrap();
        assert_eq!(db.load_removed().unwrap(), vec!["b"]);
    }

    #[test]
    fn overrides_round_trip_and_skip_unknown_fields() {
        let db = UserDb::in_memory().unwrap();
        let o = Override {
            book_id: "a".into(),
            field: Field::Language,
            value: "ru".into(),
        };
        db.save_override(&o).unwrap();
        db.save_override(&Override {
            value: "uk".into(),
            ..o.clone()
        })
        .unwrap();
        db.conn
            .execute(
                "INSERT INTO overrides(book_id, field, value) VALUES ('a', 'colour', 'x')",
                [],
            )
            .unwrap();
        let all = db.load_overrides().unwrap();
        assert_eq!(
            all,
            vec![Override {
                value: "uk".into(),
                ..o
            }]
        );
        db.delete_override("a", Field::Language).unwrap();
        assert!(db.load_overrides().unwrap().is_empty());
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
