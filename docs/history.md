# History of the library (November 2026)

## Where the books came from

| Source | What was taken | Result |
|---|---|---|
| `/backup/books` (initially) | 294 books | renamed to `Title_Year`, duplicates deleted |
| `/run/media/black/free/Books` + `Books/files` + the series "Библиотека Программиста", "Для профессионалов" | every IT book (1474) | 1272 moved, 202 identical copies deleted |
| `/run/media/black/free/Books` (second round, 22.11) | the remaining books, non-IT ones included (54) | 49 moved, 5 identical copies deleted |
| `/run/media/black/free/pdf` | IT books only (18) | moved; the fiction stayed there |
| `/run/media/black/free/pdf` (second round, 22.11) | everything else: 450 FB2 (fiction, series) + 24 PDF | all moved; FB2 names from their metadata (title + volume + year); 24 duplicate volumes of "Имя нам Легион" removed (the larger variant was kept) |
| `/run/media/black/data/books` | every book (735) | 568 moved, 164 identical copies deleted |

## Rules applied

- **Names:** `<title>_<year>.<ext>`, the title in the book's language (untranslated), at most
  200 bytes.
- **Identical duplicates** (md5): a single copy is kept.
- **The same book in several formats:** the PDF is kept, the EPUB and DjVu are deleted.
- **Same title and year, different content** (`_2`, `_3`): the variant without a suffix is kept
  (checked that it opens).
- **Left on the disks on purpose:** companion material (code, "(+code)", "(+CD)", `Exemples/`,
  zip files), video courses, files that are not books (invoices, READMEs, modem manuals, the
  hash-named PDFs in `iot/`) and the non-IT books in `free/Books` and `free/pdf`.

## Logs (to check or undo)

- `/backup/books/.rename_log.tsv`: every rename (`old<TAB>new`) and deletion
  (`old<TAB><deleted: duplicate of …>`; older entries say `DELETED<TAB>file`)
- `/backup/books/.move_log.tsv`: every move (`MV<TAB>source<TAB>new name`) and deleted duplicate
  (`DUP<TAB>source<TAB>the kept copy`)
- `/run/media/black/free/Books/.rename_log.tsv`, `/run/media/black/free/Books/files/.rename_log.tsv`
- `/backup/books/.rename_ignore`: files `rename-books` leaves alone

## Bugs fixed in the scripts

- `rename-books`: titles were translated into Romanian, so the prompt was switched to English
  with a ban on translating.
- `rename-books`: an error on names that were too long (Errno 36); names are now cut by bytes,
  and one error no longer stops the run.
- `rename-books`: files that already had `_YYYY` in their name were skipped; the `--all` option
  was added.
- `rename-books`: `_2` was added when the new name was the current name.
- `build-catalog`: `database is locked` when running in parallel; the worker threads no longer
  touch the database.

## OCR for scanned books (22.11.2026)

155 books (PDF/DjVu without a text layer) got their metadata from OCR (tesseract, rus+eng, the
first 8 pages):
- 145 got the title, authors, year and description from their real text in the catalog;
- ~60 were renamed: transliterated names, wrong years, and "Модернизация и ремонт ПК для
  чайников" was in fact "C# для чайников";
- 6 duplicates were deleted: 3 pairs of a scan + a variant with text, MS SQL ×2,
  Введение в программирование ×2, Схемотехника DjVu + PDF.

From then on, `rename-books` and `build-catalog` run OCR automatically when a book has no text
(`ocr()` in `rename-books`).
**Note:** `build-catalog --refresh-ai` redoes the metadata; with OCR built in the result should
be similar. Corrections made in the app are kept (they live in `raft.db`).

## Download-site addresses (23.11.2026)

The page footers of many PDFs carry the site or Telegram channel they came from. They were
stripped from the indexed text of 451 books (`build-catalog --clean-only`); the PDF files were
not changed.
