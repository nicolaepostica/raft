# Raft

Aplicație pentru biblioteca personală de cărți: căutare, filtrare, deschiderea cărții și salvarea progresului de citire.

**Starea actuală (22.11.2026):** biblioteca este curățată și catalogată. Aplicația propriu-zisă încă nu există.

## Ce există deja

| Ce | Unde |
|---|---|
| Toate cărțile (~2500), un singur folder, nume `Titlu_An.ext` | `/backup/books/` |
| Catalogul (SQLite + căutare full-text) | `/backup/books/catalog.db` |
| Același catalog în JSON | `/backup/books/catalog.json` |
| Coperți 400px, `<id>.webp` | `/backup/books/covers/` |
| Scriptul de redenumire și deduplicare | `scripts/rename-books` (link în `~/.local/bin`) |
| Scriptul care construiește catalogul | `scripts/build-catalog` (link în `~/.local/bin`) |

Biblioteca: ~2500 de cărți (PDF, FB2, EPUB, DjVu), în principal în engleză și rusă.

## Adăugarea cărților noi

```bash
cp carte_noua.pdf /backup/books/
rename-books --apply          # redenumește în Titlu_An, șterge duplicatele identice
build-catalog --json          # adaugă în catalog doar ce e nou sau mutat, scoate ce s-a șters
```

Pentru `rename-books` și `build-catalog` sunt necesare: `poppler-utils` (pdfinfo, pdftotext, pdftoppm), `djvulibre`, `tesseract` + `tesseract-langpack-rus` (OCR pentru cărțile scanate), Python 3 + Pillow și CLI-ul `claude` (pentru metadatele AI, cu modelul haiku).

## Documentație

- [docs/catalog.md](docs/catalog.md): schema `catalog.db`, categoriile, exemple de interogări
- [docs/plan.md](docs/plan.md): planul aplicației, funcții, variante tehnice, întrebări deschise
- [docs/history.md](docs/history.md): ce s-a făcut cu biblioteca, de unde au venit cărțile, jurnalele
