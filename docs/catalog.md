# The catalog: `/backup/books/catalog.db`

Built by `scripts/build-catalog`. A SQLite file the app reads directly (read-only).

## How books are identified

`id` = **md5 of the file's content**. Renaming or moving a book does not change its `id`, so its
cover, metadata, reading progress and hand corrections are kept. When run again, the script:
- skips files with the same path, size, `mtime` and cover;
- for a known book that was moved or renamed, updates only `path`;
- removes from the catalog (with the cover) the books that are no longer on disk.

## Tables

### `books`

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | md5 |
| `path` | TEXT | relative to `/backup/books` (now = `filename`, the folder is flat) |
| `filename` | TEXT | |
| `format` | TEXT | `pdf`, `epub`, `djvu`, `fb2` |
| `size` | INTEGER | bytes |
| `pages` | INTEGER | PDF and DjVu only; NULL for EPUB/FB2 |
| `title` | TEXT | in the book's language and alphabet, untranslated |
| `authors` | TEXT | JSON list |
| `year` | INTEGER | year of the edition (for translations, the translation's year); NULL for 18 books |
| `language` | TEXT | ISO 639-1 (`en`, `ru`, `ro`...) |
| `publisher` | TEXT | |
| `isbn` | TEXT | |
| `categories` | TEXT | JSON list, 1–3 slugs from the `categories` table, the most relevant first |
| `tags` | TEXT | JSON list, 2–8 lower-case English keywords |
| `description` | TEXT | 2–3 sentences in Romanian |
| `cover` | TEXT | `covers/<id>.webp` or NULL (8 books without a cover) |
| `mtime` | REAL | |
| `added_at`, `updated_at` | TEXT | ISO 8601 |

### `books_fts` (FTS5)

Columns: `id` (not indexed), `title`, `authors`, `categories`, `tags`, `description`, `content`.
`content` = the text of the first ~10 pages (at most 6000 characters), so search also finds books
by their content. Addresses of the sites and Telegram channels the books were downloaded from
(www.it-ebooks.info, allitebooks, searchfloor, OceanofPDF, @it_boooks...) are stripped from it;
`build-catalog --clean-only` strips them from an existing catalog.
Tokenizer `unicode61 remove_diacritics 2`.

### `categories`

`slug`, `name` (display name in Romanian; the app translates it to English and Russian).
32 fixed categories, defined in `CATEGORIES` in `build-catalog`:

python, web-backend, frontend, mobile, java-kotlin, csharp-dotnet, c-cpp, go-rust, other-languages,
odoo, architecture, testing, devops, cloud, networking, security, databases, cs-algorithms, data-ai,
gamedev, design-ux, agile-management, career, electronics-iot, science-engineering, office,
languages-learning, self-development, business, history-bio, fiction, other.

Distribution when the catalog was built (a book can be in several): python 733, data-ai 444,
architecture 379, web-backend 322, frontend 260, career 206, cs-algorithms 196, other-languages 189,
devops 169, databases 134, testing 125...

## Example queries

```sql
-- full-text search, ordered by relevance
SELECT b.* FROM books_fts f JOIN books b ON b.id = f.id
WHERE books_fts MATCH 'kotlin coroutines' ORDER BY rank LIMIT 50;

-- search in the title only
SELECT id FROM books_fts WHERE books_fts MATCH 'title: django';

-- filter by category (anywhere in the list / main category only)
SELECT * FROM books, json_each(books.categories) c WHERE c.value = 'devops';
SELECT * FROM books WHERE json_extract(categories, '$[0]') = 'devops';

-- combined filters
SELECT * FROM books WHERE language = 'ru' AND year >= 2020 AND format = 'pdf' ORDER BY year DESC;

-- books per category (the interface's facets)
SELECT c.value, count(*) FROM books, json_each(books.categories) c GROUP BY 1 ORDER BY 2 DESC;
```

## `catalog.json`

Made with `build-catalog --json`. Structure: `{"categories": {slug: name}, "books": [ {…the
columns of books, with authors/categories/tags as lists…} ]}`. It does not hold the search text
(`content`).

## The app's own data: `raft.db`

Kept apart from the catalog (default `~/.local/share/raft/raft.db`), so rebuilding the catalog
loses nothing:

| Table | Holds |
|---|---|
| `progress` | reading position per book (page, or text block) and percent |
| `overrides` | hand corrections: `(book_id, field, value)`; the field `reviewed` marks a book as checked |
| `removed` | books deleted from the app (their file went to the trash); hidden until the catalog is rebuilt without them |
| `settings` | preferences: `theme`, `language` |

Custom covers are PNG files in `covers/<id>.png` next to `raft.db`.

## Known problems

- **"career" is given too often (206).** The AI adds it as a second category to books that are
  not about careers. The interface can filter on the main category only; or run
  `build-catalog --refresh-ai` with a stricter prompt.
- **8 books without a cover:** `pdftoppm` could not render the first page, and
  `derevjannyj-venec_-tom-2.epub` is a broken file (it holds an HTTP error page).
- **18 books without a year** (many are FB2 files with no year in their metadata).
- **The metadata comes from the AI (haiku)** and can have small mistakes, e.g. in an author's
  name. Fix them in the app (pencil icon on the book page); the corrections survive
  `--refresh-ai`.
- **Authors taken from PDF metadata** are sometimes the producing program or download site
  ("Creator: www.it-ebooks.info", "Windows User"); the app leaves such entries out.
