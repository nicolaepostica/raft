//! Native implementations of the engine backend and the UI file host.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use raft_doc::edits::{Field, Override};
use raft_doc::{Book, BookId, Catalog, Progress, Source};
use raft_engine::{Backend, CoverSource, IndexState};
use raft_store::{CatalogDb, UserDb};
use raft_text::TextBook;
use raft_ui::Files;

/// Largest cover file read, in bytes.
const MAX_COVER_BYTES: u64 = 16 * 1024 * 1024;

pub struct NativeBackend {
    /// The open library folders and their catalogs.
    catalogs: Mutex<Vec<(String, CatalogDb)>>,
    user: UserDb,
    /// Custom covers, next to `raft.db`.
    covers: PathBuf,
    /// Catalog builds of folders that had none (or were asked to update).
    jobs: crate::index::Jobs,
}

impl NativeBackend {
    pub fn open(data: &Path) -> Result<Self> {
        if let Some(dir) = data.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let user = UserDb::open(data).with_context(|| format!("opening {}", data.display()))?;
        Ok(Self {
            catalogs: Mutex::default(),
            user,
            covers: covers_dir(data),
            jobs: crate::index::Jobs::new(crate::index::find_script()),
        })
    }

    /// The saved library folders, in the order they were added.
    pub fn saved_sources(&self) -> Result<Vec<Source>> {
        Ok(self.user.load_sources()?)
    }

    fn catalogs(&self) -> std::sync::MutexGuard<'_, Vec<(String, CatalogDb)>> {
        self.catalogs.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn load_progress(&self) -> Result<Vec<Progress>> {
        Ok(self.user.load_progress()?)
    }

    pub fn load_overrides(&self) -> Result<Vec<Override>> {
        Ok(self.user.load_overrides()?)
    }

    /// The saved interface language, if any.
    pub fn load_language(&self) -> Result<Option<String>> {
        Ok(self.user.setting("language")?)
    }

    /// The saved reader zoom, if any (1 = default).
    pub fn load_zoom(&self) -> Result<Option<f32>> {
        Ok(self.user.setting("zoom")?.and_then(|z| z.parse().ok()))
    }

    /// The saved theme: true for dark.
    pub fn load_dark(&self) -> Result<bool> {
        Ok(self.user.setting("theme")?.as_deref() == Some("dark"))
    }
}

/// The book's file: its path joined to its library folder.
pub fn book_file(book: &Book) -> Option<PathBuf> {
    if book.source.is_empty() {
        return None;
    }
    resolve(Path::new(&book.source), &book.path)
}

/// Joins a catalog path to the library root, refusing paths that escape it.
pub fn resolve(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    if rel.as_os_str().is_empty() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(root.join(rel))
}

impl Backend for NativeBackend {
    fn content_search(&self, text: &str, limit: usize) -> Result<Vec<BookId>, String> {
        let mut hits = Vec::new();
        for (_, db) in self.catalogs().iter() {
            hits.extend(db.content_search(text, limit).map_err(|e| e.to_string())?);
        }
        Ok(hits)
    }

    fn open_source(&self, root: &str) -> Result<Catalog, String> {
        let dir = Path::new(root);
        if !dir.is_absolute() {
            return Err(format!("`{root}` is not an absolute folder path"));
        }
        if !dir.is_dir() {
            return Err(format!("folder not found: {root}"));
        }
        let path = dir.join("catalog.db");
        if !path.is_file() {
            return Err(format!(
                "{root} has no catalog.db: run `build-catalog {root}` first"
            ));
        }
        let db = CatalogDb::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut catalog = db.load().map_err(|e| format!("{}: {e}", path.display()))?;
        let removed: std::collections::HashSet<BookId> = self
            .user
            .load_removed()
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        catalog.books.retain(|b| !removed.contains(&b.id));
        for b in &mut catalog.books {
            b.source = root.to_owned();
        }
        let mut catalogs = self.catalogs();
        catalogs.retain(|(r, _)| r != root);
        catalogs.push((root.to_owned(), db));
        Ok(catalog)
    }

    fn needs_index(&self, root: &str) -> bool {
        let dir = Path::new(root);
        dir.is_absolute() && dir.is_dir() && !dir.join("catalog.db").is_file()
    }

    fn start_index(&self, root: &str) -> Result<(), String> {
        let dir = Path::new(root);
        if !dir.is_absolute() || !dir.is_dir() {
            return Err(format!("folder not found: {root}"));
        }
        self.jobs.start(root)
    }

    fn index_state(&self, root: &str) -> Option<IndexState> {
        self.jobs.state(root)
    }

    fn close_source(&self, root: &str) -> Result<(), String> {
        self.catalogs().retain(|(r, _)| r != root);
        Ok(())
    }

    fn save_source(&self, source: &Source) -> Result<(), String> {
        self.user
            .save_source(source, self.now())
            .map_err(|e| e.to_string())
    }

    fn forget_source(&self, root: &str) -> Result<(), String> {
        self.user.delete_source(root).map_err(|e| e.to_string())
    }

    fn move_source(&self, from: &str, to: &str) -> Result<(), String> {
        self.user.move_source(from, to).map_err(|e| e.to_string())
    }

    fn save_progress(&self, p: &Progress) -> Result<(), String> {
        self.user.save_progress(p).map_err(|e| e.to_string())
    }

    fn delete_progress(&self, id: &str) -> Result<(), String> {
        self.user.delete_progress(id).map_err(|e| e.to_string())
    }

    fn open_external(&self, book: &Book) -> Result<(), String> {
        let path = book_file(book).ok_or("the book's path is not valid")?;
        if !path.is_file() {
            return Err(format!("file not found: {}", path.display()));
        }
        std::process::Command::new("xdg-open")
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("xdg-open failed: {e}"))
    }

    fn show_in_folder(&self, book: &Book) -> Result<(), String> {
        let path = book_file(book).ok_or("the book's path is not valid")?;
        if !path.is_file() {
            return Err(format!("file not found: {}", path.display()));
        }
        // The file manager selects the file (FileManager1); else open its folder.
        let uri = file_uri(&path);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        std::thread::spawn(move || {
            let shown = quiet("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.freedesktop.FileManager1",
                    "--object-path",
                    "/org/freedesktop/FileManager1",
                    "--method",
                    "org.freedesktop.FileManager1.ShowItems",
                    &format!("['{uri}']"),
                    "",
                ])
                .status()
                .is_ok_and(|s| s.success());
            if !shown {
                let _ = quiet("xdg-open").arg(&dir).status();
            }
        });
        Ok(())
    }

    fn remove_book(&self, book: &Book) -> Result<(), String> {
        let path = book_file(book).ok_or("the book's path is not valid")?;
        // Recorded first: a book whose file is gone must not come back on restart.
        self.user
            .save_removed(&book.id, &book.path, self.now())
            .map_err(|e| e.to_string())?;
        if path.exists() {
            let out = quiet("gio")
                .arg("trash")
                .arg(&path)
                .stderr(std::process::Stdio::piped())
                .output();
            let failed = match out {
                Ok(o) if o.status.success() => None,
                Ok(o) => Some(String::from_utf8_lossy(&o.stderr).trim().to_owned()),
                Err(e) => Some(format!("gio: {e}")),
            };
            if let Some(err) = failed {
                let _ = self.user.delete_removed(&book.id);
                return Err(format!("could not move the file to the trash: {err}"));
            }
        }
        Ok(())
    }

    fn save_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.user
            .save_setting(key, value)
            .map_err(|e| e.to_string())
    }

    fn set_cover(&self, book: &Book, source: &CoverSource) -> Result<(), String> {
        let target = custom_cover(&self.covers, &book.id).ok_or("the book id is not valid")?;
        let bytes = match source {
            CoverSource::Catalog => {
                return match std::fs::remove_file(&target) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Err("the book has no custom cover".into())
                    }
                    Err(e) => Err(e.to_string()),
                };
            }
            CoverSource::File(path) => read_capped(Path::new(path), MAX_COVER_INPUT)?,
            CoverSource::Page(page) => {
                let file = book_file(book).ok_or("the book's path is not valid")?;
                raft_render::render_page(&file, book.format, *page, COVER_W * 2)
                    .map_err(|e| e.to_string())?
                    .0
            }
        };
        save_cover(&bytes, &target)
    }

    fn save_override(&self, o: &Override) -> Result<(), String> {
        self.user.save_override(o).map_err(|e| e.to_string())
    }

    fn delete_override(&self, id: &str, field: Field) -> Result<(), String> {
        self.user
            .delete_override(id, field)
            .map_err(|e| e.to_string())
    }

    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
    }
}

/// Largest image accepted as a cover source, in bytes.
const MAX_COVER_INPUT: usize = 32 * 1024 * 1024;
/// Custom covers are scaled down to fit this box.
const COVER_W: u32 = 600;
const COVER_H: u32 = 900;

/// The folder of custom covers for the database at `data`.
pub fn covers_dir(data: &Path) -> PathBuf {
    data.parent().unwrap_or(Path::new(".")).join("covers")
}

/// `<dir>/<id>.png`, refusing ids that are not plain names.
pub fn custom_cover(dir: &Path, id: &str) -> Option<PathBuf> {
    let plain = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric());
    plain.then(|| dir.join(format!("{id}.png")))
}

/// Decodes an image (with limits), scales it down to the cover box and writes
/// it as PNG, atomically.
pub fn save_cover(bytes: &[u8], target: &Path) -> Result<(), String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader
        .decode()
        .map_err(|e| format!("not an image this app can read: {e}"))?;
    let img = if img.width() > COVER_W || img.height() > COVER_H {
        img.resize(COVER_W, COVER_H, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let dir = target.parent().ok_or("bad cover path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = target.with_extension("png.tmp");
    img.to_rgba8()
        .save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, target).map_err(|e| e.to_string())
}

/// Runs a zenity file chooser in the background; the chosen path (or `None`)
/// arrives on the channel.
fn zenity(args: Vec<String>) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("raft-pick".into())
        .spawn(move || {
            let out = std::process::Command::new("zenity")
                .args(&args)
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output();
            let path = out
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .filter(|p| !p.is_empty());
            let _ = tx.send(path);
        })
        .ok()?;
    Some(rx)
}

/// A command with no input and no output (it must not touch the terminal).
fn quiet(program: &str) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    c
}

/// `file://` URI of an absolute path, percent-encoded.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(b) {
            uri.push(char::from(*b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

type PageKey = (BookId, u32, u32);
type PageResult = Result<Vec<u8>, String>;

/// Most rendered pages kept in memory.
const PAGE_CACHE: usize = 12;
/// Most renders running at once.
const MAX_RENDERS: usize = 3;

#[derive(Default)]
struct Pages {
    done: Vec<(PageKey, PageResult)>,
    running: Vec<PageKey>,
}

/// Most parsed text books kept in memory.
const TEXT_CACHE: usize = 3;

type TextResult = Result<Arc<TextBook>, String>;

#[derive(Default)]
struct Texts {
    done: Vec<(BookId, TextResult)>,
    running: Vec<BookId>,
}

pub struct NativeFiles {
    covers: PathBuf,
    pages: Arc<Mutex<Pages>>,
    texts: Arc<Mutex<Texts>>,
}

impl NativeFiles {
    pub fn new(data: &Path) -> Self {
        Self {
            covers: covers_dir(data),
            pages: Arc::default(),
            texts: Arc::default(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Pages> {
        self.pages.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Files for NativeFiles {
    fn text(&self, book: &Book) -> Option<TextResult> {
        let mut texts = self.texts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, r)) = texts.done.iter().find(|(id, _)| *id == book.id) {
            return Some(r.clone());
        }
        if texts.running.contains(&book.id) {
            return None;
        }
        let Some(path) = book_file(book) else {
            return Some(Err("the book's path is not valid".into()));
        };
        texts.running.push(book.id.clone());
        drop(texts);
        let (id, format, shared) = (book.id.clone(), book.format, Arc::clone(&self.texts));
        let own_id = id.clone();
        let spawned = std::thread::Builder::new()
            .name("raft-text".into())
            .spawn(move || {
                let result = read_capped(&path, raft_text::MAX_INPUT)
                    .and_then(|bytes| raft_text::parse(&bytes, format).map_err(|e| e.to_string()))
                    .map(Arc::new);
                let mut texts = shared.lock().unwrap_or_else(|e| e.into_inner());
                texts.running.retain(|k| *k != id);
                texts.done.push((id, result));
                if texts.done.len() > TEXT_CACHE {
                    drop(texts.done.remove(0));
                }
            });
        if let Err(e) = spawned {
            self.texts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .running
                .retain(|k| *k != own_id);
            return Some(Err(format!("could not start reading: {e}")));
        }
        None
    }

    fn cover(&self, book: &Book) -> Option<Vec<u8>> {
        let custom = custom_cover(&self.covers, &book.id).filter(|p| p.is_file());
        let path = match custom {
            Some(p) => p,
            None => resolve(Path::new(&book.source), book.cover.as_deref()?)
                .filter(|_| !book.source.is_empty())?,
        };
        let meta = std::fs::metadata(&path).ok()?;
        if meta.len() > MAX_COVER_BYTES {
            return None;
        }
        std::fs::read(path).ok()
    }

    fn has_custom_cover(&self, book: &Book) -> bool {
        custom_cover(&self.covers, &book.id).is_some_and(|p| p.is_file())
    }

    fn pick_image(&self, title: &str) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
        zenity(vec![
            "--file-selection".into(),
            format!("--title={title}"),
            "--file-filter=* | *.png *.jpg *.jpeg *.webp *.PNG *.JPG *.JPEG *.WEBP".into(),
        ])
    }

    fn pick_folder(&self, title: &str) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
        zenity(vec![
            "--file-selection".into(),
            "--directory".into(),
            format!("--title={title}"),
        ])
    }
    fn page(&self, book: &Book, page: u32, width: u32) -> Option<Result<Vec<u8>, String>> {
        let key: PageKey = (book.id.clone(), page, width);
        let mut pages = self.lock();
        if let Some((_, r)) = pages.done.iter().find(|(k, _)| *k == key) {
            return Some(r.clone());
        }
        if pages.running.contains(&key) || pages.running.len() >= MAX_RENDERS {
            return None;
        }
        let Some(path) = book_file(book) else {
            return Some(Err("the book's path is not valid".into()));
        };
        pages.running.push(key.clone());
        drop(pages);
        let own_key = key.clone();
        let shared = Arc::clone(&self.pages);
        let format = book.format;
        let spawned = std::thread::Builder::new()
            .name("raft-render".into())
            .spawn(move || {
                let result = raft_render::render_page(&path, format, page, width)
                    .map(|(bytes, _)| bytes)
                    .map_err(|e| e.to_string());
                let mut pages = shared.lock().unwrap_or_else(|e| e.into_inner());
                pages.running.retain(|k| *k != key);
                pages.done.push((key, result));
                if pages.done.len() > PAGE_CACHE {
                    drop(pages.done.remove(0));
                }
            });
        if let Err(e) = spawned {
            self.lock().running.retain(|k| *k != own_key);
            return Some(Err(format!("could not start rendering: {e}")));
        }
        None
    }
}

/// Reads a whole file, refusing ones larger than `max` bytes.
fn read_capped(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if len > max as u64 {
        return Err(format!("the file is too large ({len} bytes)"));
    }
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_covers_are_scaled_and_ids_checked() {
        let dir = std::env::temp_dir().join(format!("raft-cover-test-{}", std::process::id()));
        assert_eq!(custom_cover(&dir, "../x"), None);
        assert_eq!(custom_cover(&dir, ""), None);
        let target = custom_cover(&dir, "abc123").unwrap();
        let big = image::RgbaImage::from_pixel(1200, 1200, image::Rgba([200, 10, 10, 255]));
        let mut png = Vec::new();
        big.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        save_cover(&png, &target).unwrap();
        let saved = image::open(&target).unwrap();
        assert_eq!((saved.width(), saved.height()), (600, 600));
        assert!(save_cover(b"not an image", &target).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A library folder with a catalog.db holding one book, `id`, whose text has `word`.
    fn library(dir: &Path, id: &str, word: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let conn = rusqlite::Connection::open(dir.join("catalog.db")).unwrap();
        conn.execute_batch(&format!(
            r#"CREATE TABLE books(id TEXT PRIMARY KEY, path TEXT, filename TEXT, format TEXT,
                 size INTEGER, pages INTEGER, title TEXT, authors TEXT, year INTEGER,
                 language TEXT, publisher TEXT, isbn TEXT, categories TEXT, tags TEXT,
                 description TEXT, cover TEXT, mtime REAL, added_at TEXT, updated_at TEXT);
               CREATE VIRTUAL TABLE books_fts USING fts5(id UNINDEXED, title, authors,
                 categories, tags, description, content);
               CREATE TABLE categories(slug TEXT PRIMARY KEY, name TEXT);
               INSERT INTO books(id, path, filename, format, title)
                 VALUES ('{id}', '{id}.pdf', '{id}.pdf', 'pdf', 'Book {id}');
               INSERT INTO books_fts(id, title, content) VALUES ('{id}', 'Book {id}', '{word}');"#
        ))
        .unwrap();
        std::fs::write(dir.join(format!("{id}.pdf")), b"%PDF").unwrap();
    }

    #[test]
    fn library_folders_open_search_and_close() {
        let tmp = std::env::temp_dir().join(format!("raft-sources-{}", std::process::id()));
        let (one, two) = (tmp.join("one"), tmp.join("two"));
        library(&one, "aaa", "coroutines");
        library(&two, "bbb", "monads");
        let backend = NativeBackend::open(&tmp.join("data/raft.db")).unwrap();
        let root = |p: &Path| p.to_string_lossy().into_owned();
        let c1 = backend.open_source(&root(&one)).unwrap();
        let c2 = backend.open_source(&root(&two)).unwrap();
        assert_eq!(c2.books.first().map(|b| b.source.clone()), Some(root(&two)));
        let book = c2.books.first().cloned().unwrap();
        assert_eq!(book_file(&book), Some(two.join("bbb.pdf")));
        assert_eq!(backend.content_search("monads", 10).unwrap(), vec!["bbb"]);
        assert_eq!(
            backend.content_search("coroutines", 10).unwrap(),
            vec!["aaa"]
        );
        // Opening is not remembering.
        assert!(backend.saved_sources().unwrap().is_empty());
        backend.save_source(&Source::new(root(&one))).unwrap();
        backend.save_source(&Source::new(root(&two))).unwrap();
        // Opening again does not search the folder twice.
        backend.open_source(&root(&two)).unwrap();
        assert_eq!(backend.content_search("monads", 10).unwrap(), vec!["bbb"]);
        assert!(
            backend.open_source(&root(&tmp)).is_err(),
            "no catalog.db there"
        );
        assert!(backend.open_source("relative/dir").is_err());
        backend.close_source(&root(&one)).unwrap();
        assert!(backend.content_search("coroutines", 10).unwrap().is_empty());
        backend.move_source(&root(&one), "/mnt/one").unwrap();
        backend.forget_source(&root(&two)).unwrap();
        assert_eq!(
            backend.saved_sources().unwrap(),
            vec![Source::new("/mnt/one")]
        );
        assert_eq!(c1.books.len(), 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn file_uris_are_encoded() {
        assert_eq!(
            file_uri(Path::new("/b/Ёж 1's.pdf")),
            "file:///b/%D0%81%D0%B6%201%27s.pdf"
        );
    }

    #[test]
    fn resolve_refuses_escapes() {
        let root = Path::new("/lib");
        assert_eq!(resolve(root, "a.pdf"), Some(PathBuf::from("/lib/a.pdf")));
        assert_eq!(
            resolve(root, "covers/x.webp"),
            Some(PathBuf::from("/lib/covers/x.webp"))
        );
        assert_eq!(resolve(root, "../etc/passwd"), None);
        assert_eq!(resolve(root, "/etc/passwd"), None);
        assert_eq!(resolve(root, ""), None);
    }
}
