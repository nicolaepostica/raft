# Catalogul: `/backup/books/catalog.db`

Construit de `scripts/build-catalog`. Fișier SQLite, citit direct de aplicație.

## Identificarea cărților

`id` = **md5 al conținutului fișierului**. Redenumirea sau mutarea unei cărți nu schimbă `id`-ul, deci nu se pierd coperta, metadatele sau (în viitor) progresul de citire. La rerulare, scriptul:
- sare peste fișierele cu aceeași cale, mărime, `mtime` și copertă;
- pentru o carte cunoscută dar mutată sau redenumită actualizează doar `path`;
- șterge din catalog (și coperta) cărțile care nu mai există pe disc.

## Tabele

### `books`

| Coloană | Tip | Note |
|---|---|---|
| `id` | TEXT PK | md5 |
| `path` | TEXT | relativ la `/backup/books` (acum = `filename`, folderul e plat) |
| `filename` | TEXT | |
| `format` | TEXT | `pdf`, `epub`, `djvu`, `fb2` |
| `size` | INTEGER | octeți |
| `pages` | INTEGER | doar PDF și DjVu; NULL pentru EPUB/FB2 |
| `title` | TEXT | în limba și alfabetul cărții, netradus |
| `authors` | TEXT | listă JSON |
| `year` | INTEGER | anul ediției (la traduceri anul traducerii); NULL la 18 de cărți |
| `language` | TEXT | ISO 639-1 (`en`, `ru`, `ro`...) |
| `publisher` | TEXT | |
| `isbn` | TEXT | |
| `categories` | TEXT | listă JSON, 1–3 slug-uri din tabelul `categories`, cea mai relevantă prima |
| `tags` | TEXT | listă JSON, 2–8 cuvinte cheie în engleză, cu litere mici |
| `description` | TEXT | 2–3 propoziții în română |
| `cover` | TEXT | `covers/<id>.webp` sau NULL (8 cărți fără copertă) |
| `mtime` | REAL | |
| `added_at`, `updated_at` | TEXT | ISO 8601 |

### `books_fts` (FTS5)

Coloane: `id` (neindexat), `title`, `authors`, `categories`, `tags`, `description`, `content`.
`content` = textul primelor ~10 pagini (max 6000 de caractere), deci căutarea găsește și după conținut.
Tokenizer `unicode61 remove_diacritics 2`.

### `categories`

`slug`, `name` (nume afișat în română). 32 de categorii fixe, definite în `CATEGORIES` din `build-catalog`:

python, web-backend, frontend, mobile, java-kotlin, csharp-dotnet, c-cpp, go-rust, other-languages, odoo, architecture, testing, devops, cloud, networking, security, databases, cs-algorithms, data-ai, gamedev, design-ux, agile-management, career, electronics-iot, science-engineering, office, languages-learning, self-development, business, history-bio, fiction, other.

Distribuția actuală (o carte poate fi în mai multe): python 733, data-ai 444, architecture 379, web-backend 322, frontend 260, career 206, cs-algorithms 196, other-languages 189, devops 169, databases 134, testing 125...

## Exemple de interogări

```sql
-- căutare full-text, ordonată după relevanță
SELECT b.* FROM books_fts f JOIN books b ON b.id = f.id
WHERE books_fts MATCH 'kotlin coroutines' ORDER BY rank LIMIT 50;

-- căutare doar în titlu
SELECT id FROM books_fts WHERE books_fts MATCH 'title: django';

-- filtrare după categorie (oriunde în listă / doar categoria principală)
SELECT * FROM books, json_each(books.categories) c WHERE c.value = 'devops';
SELECT * FROM books WHERE json_extract(categories, '$[0]') = 'devops';

-- filtre combinate
SELECT * FROM books WHERE language = 'ru' AND year >= 2020 AND format = 'pdf' ORDER BY year DESC;

-- numărul de cărți pe categorie (pentru fațetele din interfață)
SELECT c.value, count(*) FROM books, json_each(books.categories) c GROUP BY 1 ORDER BY 2 DESC;
```

## `catalog.json`

Generat cu `build-catalog --json`. Structură: `{"categories": {slug: nume}, "books": [ {…coloanele din books, cu authors/categories/tags ca liste…} ]}`. Nu conține textul pentru căutare (`content`).

## Probleme cunoscute

- **„career” e atribuită prea des (206).** AI-ul o pune ca a doua categorie și la cărți care nu sunt despre carieră. Variante: în interfață se filtrează după prima categorie, sau se rulează `build-catalog --refresh-ai` cu un prompt mai strict.
- **8 cărți fără copertă:** `pdftoppm` n-a putut randa prima pagină, iar `derevjannyj-venec_-tom-2.epub` e un fișier stricat (conține o eroare HTTP).
- **18 de cărți fără an** (multe sunt FB2 fără an în metadate).
- **Metadatele vin de la AI (haiku)** și pot avea mici greșeli, de exemplu la numele autorului. Aplicația ar trebui să permită editarea manuală, salvată separat ca să nu fie suprascrisă de `--refresh-ai`.
