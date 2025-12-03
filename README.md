# Raft

A desktop app for a personal book library: search, filters, reading PDF, DjVu, EPUB and FB2,
reading progress, and hand corrections of the catalog's metadata. Written in Rust with egui.

The library it was built for holds about 2500 books (PDF, FB2, EPUB, DjVu), mostly in English
and Russian, in one folder with a SQLite catalog next to them.

## What it does

- **Library:** a cover grid with search over title, author, tags, description and book text
  (full-text, diacritics- and case-insensitive), filters by language, format, years (a
  two-handle slider), category and data marks, five sort orders, and "continue reading".
- **Book page:** metadata, description, categories; icons under the cover to read, open in the
  system's app, show in the folder, change the cover (an image file, a dropped image or a page
  of the book) and delete the book (to the trash, after a confirmation).
- **Readers:** PDF and DjVu pages rendered with `pdftoppm` / `ddjvu`, all in one column
  that scrolls to the end of the book (only the pages in view are rendered); EPUB and FB2 as
  reflowable text with a table of contents and adjustable letter size. ↑/↓ scroll, Page
  Up/Down and Space move by a screen, Home/End go to the ends. Zoom (25 % … 400 %, kept
  between runs) with − / % / + in the reader's bar, Ctrl+= / Ctrl+− / Ctrl+0 or Ctrl+wheel:
  the page's width for PDF/DjVu (scrolling sideways past 100 %), the letter size for
  EPUB/FB2. The position is saved.
- **Metadata corrections:** every field can be corrected by hand; corrections live in the app's
  own database, so rebuilding the catalog never loses them. Books are marked *edited*,
  *incomplete* (missing title, authors, year, language, categories or description) or
  *reviewed*, and can be filtered by those marks and processed one after another.
- **Duplicates:** books with the same ISBN (ISBN-10 and -13 alike), or the same title, year
  and volume, are marked *duplicate?* and can be filtered ("Possible duplicates"); the book
  page lists the other copies with format, size, pages and file name, so the extra one can
  be deleted, or the book marked "not a duplicate".
- **Many books at once:** a tick box at the top right of each cover (on hover, then on every
  cover once one is ticked; while books are ticked a click on a cover ticks it too, Esc
  clears). The bar above the grid selects every result or none and moves the ticked books
  to the trash, after a confirmation listing them.
- **Several library folders:** add more folders (another disk, an external drive) with
  File → Settings (Ctrl+,) → Add a folder; each has its own `catalog.db` made by
  `build-catalog DIR`. Their books are shown together. In the settings every folder can be
  renamed, switched off (its books are hidden, it stays
  in the list), pointed to a new place after a move (progress and corrections stay) or
  removed; a folder on an unplugged disk shows as unavailable and loads again when it is back.
- **Interface:** Romanian, English or Russian, light or dark theme (both in File → Settings);
  undo/redo for every change; keyboard shortcuts (Backspace / Alt+← go back, Ctrl+S saves,
  Ctrl+F searches, Ctrl+, opens the settings); the Shortcuts tab of the settings lists
  every key, with a search.

## Running

```bash
cargo run --release -p raft -- --library /backup/books
```

| Option | Meaning |
|---|---|
| `--library DIR` | a library folder with `catalog.db` and `covers/`, added to the saved ones (on the first run the default is `/backup/books`, or `$RAFT_LIBRARY`) |
| `--data FILE` | the app's own database: progress, corrections, settings (default `~/.local/share/raft/raft.db`) |
| `--control PORT` | a local JSON control channel on `127.0.0.1:PORT` (commands, state, screenshots) |

`RAFT_THEME=light|dark` overrides the saved theme for one run. Rendering pages needs
`poppler-utils` (`pdftoppm`) and `djvulibre` (`ddjvu`); moving books to the trash needs `gio`.

`assets/raft.desktop` and `assets/icon.png` are a desktop entry and icon for launchers.

## The library and its catalog

| What | Where |
|---|---|
| The books, one flat folder, named `Title_Year.ext` | `/backup/books/` |
| The catalog (SQLite + full-text search) | `/backup/books/catalog.db` |
| The same catalog as JSON | `/backup/books/catalog.json` |
| Covers, 400 px wide, `<id>.webp` | `/backup/books/covers/` |
| Script that renames books and removes duplicates | `scripts/rename-books` |
| Script that builds the catalog | `scripts/build-catalog` |

A book's id is the md5 of its file, so renaming or moving a file keeps its cover, progress and
corrections.

### Adding new books

```bash
cp new_book.pdf /backup/books/
rename-books --apply          # rename to Title_Year, delete identical duplicates
build-catalog --json          # add only what is new or moved, drop what was deleted
```

`rename-books` and `build-catalog` need `poppler-utils` (pdfinfo, pdftotext, pdftoppm),
`djvulibre`, `tesseract` + `tesseract-langpack-rus` (OCR for scanned books), Python 3 + Pillow,
and the `claude` CLI (AI metadata, with the haiku model).

## Development

The workspace is split into layers; a crate depends only on lower ones:

| Crate | Holds |
|---|---|
| `raft-doc` | the data model (books, queries, corrections) |
| `raft-search` | search, filters, facet counts |
| `raft-catalog`, `raft-store` | reading `catalog.json` / `catalog.db`; the app's own `raft.db` |
| `raft-render`, `raft-text` | page rendering; EPUB/FB2 parsing |
| `raft-engine` | the session, the command registry, undo/redo |
| `raft-ui` | the egui interface, translations, theme |
| `raft` | the desktop app |

Every user action is a command in `raft-engine` (`cargo run -p raft -- --control 7878`, then
send `{"method": "commands"}` to list them). The gate before a commit:

```bash
cargo xtask ci     # fmt, clippy -D warnings, tests, layer checks, wasm build
```

The colours live in `assets/theme.toml`; `python3 assets/icon.py` redraws the icon from it.

## Licence

MIT OR Apache-2.0.
