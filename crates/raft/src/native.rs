//! Native implementations of the engine backend and the UI file host.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use raft_doc::edits::{Field, Override};
use raft_doc::{Book, BookId, Catalog, Progress};
use raft_engine::Backend;
use raft_store::{CatalogDb, UserDb};
use raft_ui::Files;

/// Largest cover file read, in bytes.
const MAX_COVER_BYTES: u64 = 16 * 1024 * 1024;

pub struct NativeBackend {
    library: PathBuf,
    catalog: CatalogDb,
    user: UserDb,
}

impl NativeBackend {
    pub fn open(library: &Path, data: &Path) -> Result<Self> {
        let db = library.join("catalog.db");
        let catalog = CatalogDb::open(&db).with_context(|| format!("opening {}", db.display()))?;
        if let Some(dir) = data.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let user = UserDb::open(data).with_context(|| format!("opening {}", data.display()))?;
        Ok(Self {
            library: library.to_owned(),
            catalog,
            user,
        })
    }

    pub fn load_catalog(&self) -> Result<Catalog> {
        Ok(self.catalog.load()?)
    }

    pub fn load_progress(&self) -> Result<Vec<Progress>> {
        Ok(self.user.load_progress()?)
    }

    pub fn load_overrides(&self) -> Result<Vec<Override>> {
        Ok(self.user.load_overrides()?)
    }
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
        self.catalog
            .content_search(text, limit)
            .map_err(|e| e.to_string())
    }

    fn save_progress(&self, p: &Progress) -> Result<(), String> {
        self.user.save_progress(p).map_err(|e| e.to_string())
    }

    fn delete_progress(&self, id: &str) -> Result<(), String> {
        self.user.delete_progress(id).map_err(|e| e.to_string())
    }

    fn open_external(&self, book: &Book) -> Result<(), String> {
        let path = resolve(&self.library, &book.path).ok_or("the book's path is not valid")?;
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

pub struct NativeFiles {
    library: PathBuf,
    pages: Arc<Mutex<Pages>>,
}

impl NativeFiles {
    pub fn new(library: &Path) -> Self {
        Self {
            library: library.to_owned(),
            pages: Arc::default(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Pages> {
        self.pages.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Files for NativeFiles {
    fn cover(&self, book: &Book) -> Option<Vec<u8>> {
        let path = resolve(&self.library, book.cover.as_deref()?)?;
        let meta = std::fs::metadata(&path).ok()?;
        if meta.len() > MAX_COVER_BYTES {
            return None;
        }
        std::fs::read(path).ok()
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
        let Some(path) = resolve(&self.library, &book.path) else {
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

#[cfg(test)]
mod tests {
    use super::*;

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
