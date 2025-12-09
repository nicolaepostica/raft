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
- **Books from outside the library:** File → Open a file… (Ctrl+Shift+O), a book file
  dropped on the window, `raft BOOK` or "Open with" in the file manager read a PDF, DjVu, EPUB
  or FB2 without adding it to the library. It keeps its progress, shows in Continue reading
  with a "file" mark (a PDF/DjVu gets its first page as cover) and never in the search
  results; a file that is a library book (same md5) opens as that book.
- **Several library folders:** add more folders (another disk, an external drive) with
  File → Settings (Ctrl+,) → Add a folder; each has its own `catalog.db` made by
  `build-catalog DIR`. A folder without one gets it from the app: `build-catalog DIR --no-ai`
  runs in the background (progress in the settings and the status bar) and takes the title,
  authors, year, language and ISBN from the files' metadata, the text of the first pages
  (the largest font of a PDF's first pages when neither the metadata nor the file name says
  anything) and the file names; the cover is the first page (PDF, DjVu) or the book's own
  picture (EPUB, FB2). "Update" in the settings runs it again for new or deleted books
  (books in subfolders too, e.g. one folder per author; the files are not
  renamed or moved; the subfolders are a "Folders" filter and a link on the book page). Their
  books are shown together. In the settings every folder can be
  renamed, switched off (its books are hidden, it stays
  in the list), pointed to a new place after a move (progress and corrections stay) or
  removed; a folder on an unplugged disk shows as unavailable and loads again when it is back.
- **Interface:** Romanian, English or Russian, light or dark theme (both in File → Settings);
  undo/redo for every change; keyboard shortcuts (Backspace / Alt+← go back, Ctrl+S saves,
  Ctrl+F searches, Ctrl+, opens the settings); the Shortcuts tab of the settings lists
  every key, with a search.

## Install

Linux x86_64, from [Releases](../../releases):

```
raft_<version>_amd64.deb                 Ubuntu 22.04+, Debian 12+
raft-<version>-linux-x86_64.tar.gz       any distribution with glibc 2.35+
SHA256SUMS.txt
```

- **.deb:** `sudo apt install ./raft_<version>_amd64.deb` installs `raft`, `build-catalog`,
  `rename-books`, the menu entry and the icon, and pulls in poppler-utils, djvulibre and
  Python with Pillow.
- **.tar.gz:** unpack it anywhere and run `./raft`; `build-catalog` and `rename-books` are
  next to it. Install `poppler-utils`, `djvulibre` and `python3-pillow` from the
  distribution. `raft.desktop` and `raft.png` go to `~/.local/share/applications/` and
  `~/.local/share/icons/hicolor/256x256/apps/` for a menu entry.

Windows and macOS are not supported: opening files, the trash and the image picker use
`xdg-open`, `gio` and `zenity`.

## Running

```bash
cargo run --release -p raft -- --library /backup/books
```

| Option | Meaning |
|---|---|
| `BOOK` | a PDF, DjVu, EPUB or FB2 file to read at start; it need not be in the library |
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

`build-catalog --no-ai` makes the catalog without the `claude` CLI (what the app runs for a
new folder); `build-catalog --refresh-ai` adds the AI metadata later.

`rename-books` and `build-catalog` need `poppler-utils` (pdfinfo, pdftotext, pdftoppm, pdftohtml),
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

## Release

Every push runs CI (`cargo xtask ci`); when it passes on `main` and the version in
`Cargo.toml` has no release yet, the Release workflow builds the packages and publishes them,
with the version's section of [CHANGELOG.md](CHANGELOG.md) as the notes. Details in
[packaging/RELEASING.md](packaging/RELEASING.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Third-party material built into the binary is
listed in [NOTICE](NOTICE) and, with full licence texts, in `THIRD-PARTY.txt` in every package.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above,
without any additional terms or conditions.
