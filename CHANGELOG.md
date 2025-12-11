# Changelog

All notable changes to Raft. The format follows [Keep a Changelog](https://keepachangelog.com/),
and versions follow [Semantic Versioning](https://semver.org/). A version's date is the date of
the commit its tag points to.

## [Unreleased]

### Added

- **Reading states, favourites and ratings:** mark a book as to read, reading, read or
  abandoned, as a favourite, and give it 1 to 5 stars, on its page or for all the selected
  books at once; filter by state and favourites (with counts); a badge and a heart on the
  cover. Opening a book marks it as being read; at the end of a book the reader offers to mark
  it as read.
- **Packages:** an `.rpm` (Fedora, RHEL, openSUSE) and an AppImage next to the `.deb` and
  the `.tar.gz`.

## [0.1.0] — 2026-11-27

The first release: Linux x86_64 (`.deb` and `.tar.gz`).

### Added

- **Library:** a cover grid with full-text search over titles, authors, tags, descriptions and
  the books' text (case- and diacritics-insensitive); filters by language, format, years,
  category, series, folder and data marks, with the checked ones listed first; five sort
  orders; "continue reading".
- **Book page:** metadata, description, categories, series with a link to the next volume,
  authors and folders as links; read, open in the system's app, show in the folder, change
  the cover, move to the trash.
- **Readers:** PDF and DjVu pages (rendered by `pdftoppm` / `ddjvu`) in one scrolling column,
  EPUB and FB2 as reflowable text with a table of contents; zoom; the position is saved.
- **Metadata corrections** by hand, kept in the app's own database across catalog rebuilds,
  with *edited*, *incomplete* and *reviewed* marks; possible duplicates (same ISBN, or the
  same title, year and volume).
- **Many books at once:** tick covers, then select all or move the ticked ones to the trash.
- **Books from outside the library:** File → Open a file…, drag and drop, `raft BOOK`.
- **Several library folders**, each with its own `catalog.db`; a folder without one is
  catalogued in the background by `build-catalog DIR --no-ai`.
- **Interface** in Romanian, English and Russian, light and dark themes, undo/redo for every
  change, keyboard shortcuts listed in the settings, a local JSON control channel
  (`--control PORT`).
- **Scripts:** `build-catalog` (catalog, covers, series from FB2/EPUB, optional AI metadata
  through the `claude` CLI) and `rename-books` (names files `Title_Year.ext`, removes exact
  duplicates).

[0.1.0]: https://github.com/nicolaepostica/raft/releases/tag/v0.1.0
