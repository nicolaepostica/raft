# Istoricul bibliotecii (noiembrie 2026)

## De unde au venit cărțile

| Sursă | Ce s-a luat | Rezultat |
|---|---|---|
| `/backup/books` (inițial) | 294 de cărți | redenumite în `Titlu_An`, duplicate șterse |
| `/run/media/black/free/Books` + `Books/files` + seriile „Библиотека Программиста”, „Для профессионалов” | toate cărțile de IT (1474) | 1272 mutate, 202 copii identice șterse |
| `/run/media/black/free/Books` (a doua rundă, 22.11) | restul cărților, și cele care nu sunt de IT (54) | 49 mutate, 5 copii identice șterse |
| `/run/media/black/free/pdf` | doar cărțile de IT (18) | mutate; beletristica a rămas acolo |
| `/run/media/black/free/pdf` (a doua rundă, 22.11) | tot restul: 450 FB2 (beletristică, serii) + 24 PDF | mutate toate; numele FB2 din metadate (titlu + volum + an); 24 de volume duplicate din „Имя нам Легион” eliminate (s-a păstrat varianta mai mare) |
| `/run/media/black/data/books` | toate cărțile (735) | 568 mutate, 164 copii identice șterse |

## Reguli aplicate

- **Nume:** `<titlu>_<an>.<ext>`, titlul în limba cărții (netradus), maximum 200 de octeți.
- **Duplicate identice** (md5): se păstrează un singur exemplar.
- **Aceeași carte în mai multe formate:** se păstrează PDF-ul, iar EPUB-ul și DjVu-ul se șterg.
- **Același titlu și an cu conținut diferit** (`_2`, `_3`): se păstrează varianta fără sufix (verificată că se deschide).
- **Ce a rămas pe discuri, intenționat:** materialele însoțitoare (cod, „(+code)”, „(+CD)”, `Exemples/`, zip-uri), cursurile video, fișierele care nu sunt cărți (facturi, README, manuale de modem, PDF-urile cu nume de tip hash din `iot/`) și cărțile care nu sunt de IT din `free/Books` și `free/pdf`.

## Jurnale (pentru verificare sau anulare)

- `/backup/books/.rename_log.tsv`: fiecare redenumire (`vechi<TAB>nou`) și ștergere (`DELETED<TAB>fișier`)
- `/backup/books/.move_log.tsv`: fiecare mutare (`MV<TAB>sursă<TAB>nume nou`) și duplicat șters (`DUP<TAB>sursă<TAB>exemplarul păstrat`)
- `/run/media/black/free/Books/.rename_log.tsv`, `/run/media/black/free/Books/files/.rename_log.tsv`
- `/backup/books/.rename_ignore`: fișiere pe care `rename-books` nu le atinge

## Bug-uri reparate în scripturi

- `rename-books`: titlurile erau traduse în română, de aceea promptul a fost trecut în engleză cu interdicția de a traduce.
- `rename-books`: eroare la nume prea lungi (Errno 36); acum se trunchiază după octeți, iar o eroare nu mai oprește rularea.
- `rename-books`: fișierele care aveau deja `_AAAA` în nume erau sărite; s-a adăugat opțiunea `--all`.
- `rename-books`: se adăuga `_2` când noul nume era chiar numele actual.
- `build-catalog`: `database is locked` la rularea în paralel; acum firele nu mai ating baza de date.

## OCR pentru cărțile scanate (22.11.2026)

155 de cărți (PDF/DjVu fără strat de text) au primit metadatele din OCR (tesseract, rus+eng, primele 8 pagini):
- 145 au primit în catalog titlul, autorii, anul și descrierea din textul real;
- ~60 au fost redenumite: nume transliterate, ani greșiți, iar „Модернизация и ремонт ПК для чайников” era de fapt „C# для чайников”;
- 6 duplicate au fost șterse: 3 perechi scanat + varianta cu text, MS SQL ×2, Введение в программирование ×2, Схемотехника DjVu + PDF.

De acum, `rename-books` și `build-catalog` fac OCR automat când o carte nu are text (`ocr()` în `rename-books`).
**Atenție:** `build-catalog --refresh-ai` reface metadatele; cu OCR-ul integrat rezultatul ar trebui să fie similar, dar corecturile manuale se pierd.
