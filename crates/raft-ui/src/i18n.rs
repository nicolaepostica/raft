//! Interface languages. The Romanian text is the key; English and Russian are
//! looked up in [`TABLE`]. A test checks that every `t(...)` / `tf(...)` key in
//! the sources has both translations.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;

/// An interface language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ro,
    En,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Ro, Lang::En, Lang::Ru];

    pub fn code(self) -> &'static str {
        match self {
            Lang::Ro => "ro",
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Self::ALL.into_iter().find(|l| l.code() == code)
    }

    /// The language's own name, as shown in the menu.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Ro => "Română",
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }
}

thread_local! {
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::Ro) };
}

/// Sets the language for this thread's frames (the UI thread).
pub fn set(lang: Lang) {
    CURRENT.with(|c| c.set(lang));
}

pub fn current() -> Lang {
    CURRENT.with(Cell::get)
}

fn index() -> &'static HashMap<&'static str, (&'static str, &'static str)> {
    static INDEX: OnceLock<HashMap<&'static str, (&'static str, &'static str)>> = OnceLock::new();
    INDEX.get_or_init(|| TABLE.iter().map(|(ro, en, ru)| (*ro, (*en, *ru))).collect())
}

/// `ro` in the current language (itself when untranslated).
pub fn t(ro: &'static str) -> &'static str {
    tr(ro, current())
}

/// `ro` in `lang`.
pub fn tr(ro: &'static str, lang: Lang) -> &'static str {
    match (lang, index().get(ro)) {
        (Lang::En, Some((en, _))) => en,
        (Lang::Ru, Some((_, ru))) => ru,
        _ => ro,
    }
}

/// A translated template with `{0}`, `{1}` … replaced by `args`.
pub fn tf(ro: &'static str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = t(ro).to_owned();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &a.to_string());
    }
    out
}

/// A runtime string (category name, command label) translated when known.
pub fn t_dyn(ro: &str) -> String {
    match (current(), index().get_key_value(ro)) {
        (Lang::En, Some((_, (en, _)))) => (*en).to_owned(),
        (Lang::Ru, Some((_, (_, ru)))) => (*ru).to_owned(),
        _ => ro.to_owned(),
    }
}

/// (Romanian, English, Russian).
pub const TABLE: &[(&str, &str, &str)] = &[
    // Menus and command labels.
    ("Fișier", "File", "Файл"),
    ("Editare", "Edit", "Правка"),
    ("Vizualizare", "View", "Вид"),
    ("Limba interfeței", "Interface language", "Язык интерфейса"),
    ("Setări", "Settings", "Настройки"),
    ("Anulează", "Undo", "Отменить"),
    ("Refă", "Redo", "Повторить"),
    ("Caută", "Search", "Найти"),
    (
        "Șterge căutarea și filtrele",
        "Clear search and filters",
        "Сбросить поиск и фильтры",
    ),
    ("Filtru: categorie", "Filter: category", "Фильтр: категория"),
    (
        "Doar categoria principală",
        "Main category only",
        "Только основная категория",
    ),
    ("Filtru: limbă", "Filter: language", "Фильтр: язык"),
    ("Filtru: format", "Filter: format", "Фильтр: формат"),
    ("Filtru: marcaj", "Filter: mark", "Фильтр: отметка"),
    (
        "Filtru: câmp lipsă",
        "Filter: missing field",
        "Фильтр: пустое поле",
    ),
    ("Filtru: ani", "Filter: years", "Фильтр: годы"),
    ("Sortare", "Sort", "Сортировка"),
    (
        "Temă albă / întunecată",
        "Light / dark theme",
        "Светлая / тёмная тема",
    ),
    ("Înapoi", "Back", "Назад"),
    ("Pagina cărții", "Book page", "Страница книги"),
    ("Citește", "Read", "Читать"),
    (
        "Deschide în aplicația externă",
        "Open in external app",
        "Открыть во внешней программе",
    ),
    ("Arată în dosar", "Show in folder", "Показать в папке"),
    ("Șterge cartea", "Delete book", "Удалить книгу"),
    ("Schimbă coperta", "Change cover", "Сменить обложку"),
    ("Mergi la pagina", "Go to page", "Перейти к странице"),
    ("Pagina următoare", "Next page", "Следующая страница"),
    ("Pagina anterioară", "Previous page", "Предыдущая страница"),
    ("Poziția în carte", "Position in book", "Место в книге"),
    ("Șterge progresul", "Clear progress", "Сбросить прогресс"),
    ("Corectează metadatele", "Edit metadata", "Исправить данные"),
    (
        "Marchează ca verificată",
        "Mark as reviewed",
        "Отметить как проверенную",
    ),
    (
        "Revino la datele din catalog",
        "Back to catalog data",
        "Вернуть данные каталога",
    ),
    (
        "Schimbă limba interfeței",
        "Change interface language",
        "Сменить язык интерфейса",
    ),
    (
        "Adaugă un folder cu cărți",
        "Add a book folder",
        "Добавить папку с книгами",
    ),
    (
        "Scoate un folder din bibliotecă",
        "Remove a folder from the library",
        "Убрать папку из библиотеки",
    ),
    ("Filtru: sursă", "Filter: source", "Фильтр: источник"),
    ("Sursă", "Source", "Источник"),
    ("Folder", "Folder", "Папка"),
    ("Foldere ({0})", "Folders ({0})", "Папки ({0})"),
    ("Foldere", "Folders", "Папки"),
    (
        "Alte foldere ({0})",
        "Other folders ({0})",
        "Другие папки ({0})",
    ),
    (
        "Alte categorii ({0})",
        "Other categories ({0})",
        "Другие категории ({0})",
    ),
    (
        "Arată cărțile din folder",
        "Show the books in the folder",
        "Показать книги из папки",
    ),
    ("Filtru: folder", "Filter: folder", "Фильтр: папка"),
    (
        "Adaugă un folder cu cărți (fără catalog.db se face unul, fără AI)",
        "Add a book folder (one without a catalog.db gets one, without AI)",
        "Добавить папку с книгами (без catalog.db он создаётся, без ИИ)",
    ),
    (
        "Scoate folderul din bibliotecă (fișierele rămân pe disc)",
        "Remove the folder from the library (the files stay on disk)",
        "Убрать папку из библиотеки (файлы останутся на диске)",
    ),
    (
        "Activează / dezactivează un folder",
        "Enable / disable a folder",
        "Включить / выключить папку",
    ),
    (
        "Modifică un folder cu cărți",
        "Change a book folder",
        "Изменить папку с книгами",
    ),
    // Settings window.
    ("Aspect", "Appearance", "Внешний вид"),
    ("Temă", "Theme", "Тема"),
    ("Albă", "Light", "Светлая"),
    ("Întunecată", "Dark", "Тёмная"),
    ("Surse de cărți", "Book sources", "Источники книг"),
    (
        "Folderele cu cărți. Pentru un folder fără catalog.db se face unul, fără AI: titlul și autorul din metadate sau din numele fișierului, coperta din prima pagină. Un folder dezactivat rămâne în listă, dar cărțile lui nu se arată.",
        "The book folders. A folder without a catalog.db gets one, made without AI: title and author from the metadata or the file name, the cover from the first page. A disabled folder stays in the list, but its books are not shown.",
        "Папки с книгами. Для папки без catalog.db он создаётся без ИИ: название и автор из метаданных или имени файла, обложка — первая страница. Выключенная папка остаётся в списке, но её книги не показываются.",
    ),
    (
        "se face catalogul…",
        "building the catalog…",
        "создаётся каталог…",
    ),
    // Book files opened from outside the library.
    ("Deschide un fișier…", "Open a file…", "Открыть файл…"),
    ("Deschide o carte", "Open a book", "Открыть книгу"),
    (
        "Scoate din fișierele deschise",
        "Remove from the opened files",
        "Убрать из открытых файлов",
    ),
    (
        "Scoate din fișierele deschise (fișierul rămâne pe disc)",
        "Remove from the opened files (the file stays on disk)",
        "Убрать из открытых файлов (файл остаётся на диске)",
    ),
    (
        "Fișier deschis din afara bibliotecii: se citește și își păstrează progresul, dar nu apare în căutare.",
        "A file opened from outside the library: it is read and keeps its progress, but does not show in search.",
        "Файл открыт не из библиотеки: читается и сохраняет прогресс, но не появляется в поиске.",
    ),
    ("fișier", "file", "файл"),
    (
        "catalog: {0} din {1}",
        "catalog: {0} of {1}",
        "каталог: {0} из {1}",
    ),
    ("Actualizează", "Update", "Обновить"),
    (
        "Adaugă în catalog cărțile noi din folder și le scoate pe cele șterse (fără AI)",
        "Adds the folder's new books to the catalog and drops the deleted ones (without AI)",
        "Добавляет в каталог новые книги из папки и убирает удалённые (без ИИ)",
    ),
    (
        "Actualizează catalogul unui folder",
        "Update a folder's catalog",
        "Обновить каталог папки",
    ),
    (
        "Se face catalogul pentru {0}…",
        "Building the catalog of {0}…",
        "Создаётся каталог для {0}…",
    ),
    (
        "Se face catalogul pentru {0}: {1} din {2} cărți",
        "Building the catalog of {0}: {1} of {2} books",
        "Каталог для {0}: {1} из {2} книг",
    ),
    ("Niciun folder încă.", "No folders yet.", "Папок пока нет."),
    ("Adaugă un folder…", "Add a folder…", "Добавить папку…"),
    (
        "Activează sau dezactivează folderul",
        "Enable or disable the folder",
        "Включить или выключить папку",
    ),
    ("{0} cărți", "{0} books", "книг: {0}"),
    ("dezactivat", "disabled", "выключена"),
    ("Activ: {0}", "Enabled: {0}", "Включена: {0}"),
    ("indisponibil", "unavailable", "недоступна"),
    ("Reîncearcă", "Try again", "Повторить"),
    ("Schimbă folderul…", "Change folder…", "Сменить папку…"),
    (
        "Pentru un folder mutat: cărțile își păstrează progresul și corecturile",
        "For a moved folder: the books keep their progress and corrections",
        "Для перемещённой папки: книги сохранят прогресс и исправления",
    ),
    ("Scoate", "Remove", "Убрать"),
    ("General", "General", "Общие"),
    ("Zoom", "Zoom", "Масштаб"),
    ("Ctrl + rotița", "Ctrl + wheel", "Ctrl + колесо"),
    (
        "Mărește / micșorează cartea",
        "Zoom the book in / out",
        "Увеличить / уменьшить книгу",
    ),
    ("Mărește", "Zoom in", "Увеличить"),
    ("Micșorează", "Zoom out", "Уменьшить"),
    ("Mărime normală", "Actual size", "Обычный размер"),
    (
        "Micșorează (Ctrl+−, Ctrl+rotița)",
        "Zoom out (Ctrl+−, Ctrl+wheel)",
        "Уменьшить (Ctrl+−, Ctrl+колесо)",
    ),
    (
        "Mărește (Ctrl+=, Ctrl+rotița)",
        "Zoom in (Ctrl+=, Ctrl+wheel)",
        "Увеличить (Ctrl+=, Ctrl+колесо)",
    ),
    (
        "Mărime normală (Ctrl+0)",
        "Actual size (Ctrl+0)",
        "Обычный размер (Ctrl+0)",
    ),
    ("Scurtături", "Shortcuts", "Сочетания клавиш"),
    ("Comenzi", "Commands", "Команды"),
    ("În interfață", "In the interface", "В интерфейсе"),
    (
        "Caută o comandă sau o tastă",
        "Search for a command or a key",
        "Найти команду или клавишу",
    ),
    (
        "Nicio scurtătură găsită.",
        "No shortcut found.",
        "Сочетания не найдены.",
    ),
    ("Spațiu", "Space", "Пробел"),
    (
        "Mută cursorul în căutare",
        "Move the cursor to the search",
        "Перейти к поиску",
    ),
    (
        "Salvează corecturile (în formularul de corectare)",
        "Save the corrections (in the correction form)",
        "Сохранить исправления (в форме исправления)",
    ),
    (
        "Închide fereastra de setări sau dialogul deschis",
        "Close the settings window or the open dialog",
        "Закрыть окно настроек или открытый диалог",
    ),
    (
        "Derulează cartea puțin în jos / în sus",
        "Scroll the book a little down / up",
        "Прокрутить книгу немного вниз / вверх",
    ),
    (
        "Un ecran mai jos în carte",
        "One screen down in the book",
        "На экран ниже в книге",
    ),
    (
        "Un ecran mai sus în carte",
        "One screen up in the book",
        "На экран выше в книге",
    ),
    (
        "Un ecran mai jos / mai sus (cititorul de text)",
        "One screen down / up (text reader)",
        "На экран ниже / выше (чтение текста)",
    ),
    (
        "Începutul / sfârșitul cărții",
        "Start / end of the book",
        "Начало / конец книги",
    ),
    (
        "Alege folderul bibliotecii",
        "Choose the library folder",
        "Выберите папку библиотеки",
    ),
    ("Alege coperta", "Choose the cover", "Выберите обложку"),
    // Library.
    ("Filtre", "Filters", "Фильтры"),
    ("Șterge filtrele", "Clear filters", "Сбросить фильтры"),
    ("Limbă", "Language", "Язык"),
    ("Format", "Format", "Формат"),
    ("Ani", "Years", "Годы"),
    ("de la", "from", "с"),
    ("până la", "to", "по"),
    ("orice an", "any year", "любой год"),
    ("Categorii", "Categories", "Категории"),
    (
        "doar categoria principală",
        "main category only",
        "только основная категория",
    ),
    (
        "Ignoră categoriile secundare (de ex. „career”, atribuită prea des)",
        "Ignores secondary categories (e.g. “career”, given too often)",
        "Не учитывает второстепенные категории (например «career», её ставят слишком часто)",
    ),
    ("Ani: {0}–{1}", "Years: {0}–{1}", "Годы: {0}–{1}"),
    (
        "Trage capetele ca să alegi intervalul de ani",
        "Drag the ends to choose the years",
        "Перетащите концы, чтобы выбрать годы",
    ),
    ("Date", "Data", "Данные"),
    ("Corectate manual", "Edited by hand", "Исправленные вручную"),
    ("Date incomplete", "Incomplete data", "Неполные данные"),
    ("Verificate", "Reviewed", "Проверенные"),
    (
        "Cărți în care ai corectat cel puțin un câmp",
        "Books where you corrected at least one field",
        "Книги, в которых вы исправили хотя бы одно поле",
    ),
    (
        "Fără titlu, autori, an, limbă, categorii sau descriere și nemarcate ca verificate",
        "Missing title, authors, year, language, categories or description, and not reviewed",
        "Без названия, авторов, года, языка, категорий или описания и не отмеченные как проверенные",
    ),
    (
        "Marcate de tine ca verificate",
        "Marked as reviewed by you",
        "Отмеченные вами как проверенные",
    ),
    ("Lipsește câmpul…", "Missing field…", "Пустое поле…"),
    ("Continuă lectura", "Continue reading", "Продолжить чтение"),
    (
        "Continuă lectura ({0})",
        "Continue reading ({0})",
        "Продолжить чтение ({0})",
    ),
    ("Bibliotecă", "Library", "Библиотека"),
    (
        "Nu ai început încă nicio carte. Cărțile deschise apar aici.",
        "You have not started any book yet. Books you open show up here.",
        "Вы ещё не начали ни одной книги. Открытые книги появятся здесь.",
    ),
    (
        "Nicio carte nu se potrivește căutării.",
        "No book matches the search.",
        "Ни одна книга не подходит под поиск.",
    ),
    (
        "Caută după titlu, autor, etichetă sau conținut…  (Ctrl+F)",
        "Search by title, author, tag or content…  (Ctrl+F)",
        "Поиск по названию, автору, тегу или тексту…  (Ctrl+F)",
    ),
    ("Șterge căutarea", "Clear search", "Очистить поиск"),
    (
        "Caută cărțile autorului",
        "Search the author's books",
        "Искать книги автора",
    ),
    ("Relevanță", "Relevance", "Релевантность"),
    ("Titlu", "Title", "Название"),
    (
        "An (noi întâi)",
        "Year (newest first)",
        "Год (сначала новые)",
    ),
    (
        "An (vechi întâi)",
        "Year (oldest first)",
        "Год (сначала старые)",
    ),
    ("Adăugate recent", "Recently added", "Недавно добавленные"),
    ("corectată", "edited", "исправлена"),
    ("dublură?", "duplicate?", "дубль?"),
    ("{0} selectate", "{0} selected", "Выбрано: {0}"),
    (
        "Selectează toate ({0})",
        "Select all ({0})",
        "Выбрать все ({0})",
    ),
    ("Deselectează", "Select none", "Снять выбор"),
    (
        "Mută în coș ({0})",
        "Move to the trash ({0})",
        "В корзину ({0})",
    ),
    ("Selectează: {0}", "Select: {0}", "Выбрать: {0}"),
    (
        "Ștergi {0} cărți?",
        "Delete {0} books?",
        "Удалить книги ({0})?",
    ),
    (
        "Fișierele se mută în coșul de gunoi, iar cărțile dispar din bibliotecă. Le poți recupera din coș; corecturile și progresul rămân în raft.db.",
        "The files move to the trash and the books leave the library. You can restore them from the trash; corrections and progress stay in raft.db.",
        "Файлы перемещаются в корзину, а книги исчезают из библиотеки. Их можно восстановить из корзины; исправления и прогресс остаются в raft.db.",
    ),
    (
        "{0} cărți mutate în coș",
        "{0} books moved to the trash",
        "В корзину перемещено книг: {0}",
    ),
    (
        "{0} cărți mutate în coș; {1} nu au putut fi mutate (rămân selectate)",
        "{0} books moved to the trash; {1} could not be moved (they stay selected)",
        "В корзину перемещено книг: {0}; не удалось переместить: {1} (они остались выбранными)",
    ),
    ("Selectează o carte", "Select a book", "Выбрать книгу"),
    (
        "Selectează toate rezultatele",
        "Select all results",
        "Выбрать все результаты",
    ),
    ("Deselectează tot", "Select none", "Снять выбор"),
    (
        "Șterge cărțile selectate",
        "Delete the selected books",
        "Удалить выбранные книги",
    ),
    ("Dubluri posibile", "Possible duplicates", "Возможные дубли"),
    (
        "Cărți cu același ISBN, sau cu același titlu, an și volum, ca alta din bibliotecă",
        "Books with the same ISBN, or the same title, year and volume, as another one in the library",
        "Книги с тем же ISBN или с тем же названием, годом и томом, что и другая книга библиотеки",
    ),
    (
        "Posibile dubluri ale acestei cărți:",
        "Possible duplicates of this book:",
        "Возможные дубли этой книги:",
    ),
    ("nu e dublură", "not a duplicate", "не дубль"),
    ("Nu e dublură", "Not a duplicate", "Не дубль"),
    (
        "Cartea nu mai apare la „dubluri posibile”",
        "The book no longer shows under “possible duplicates”",
        "Книга больше не показывается среди «возможных дублей»",
    ),
    ("{0} pag.", "{0} p.", "{0} с."),
    (
        "Marchează: nu e dublură",
        "Mark: not a duplicate",
        "Отметить: не дубль",
    ),
    ("incompletă", "incomplete", "неполная"),
    ("{0} din {1} cărți", "{0} of {1} books", "{0} из {1} книг"),
    ("necunoscută", "unknown", "неизвестен"),
    // Book page.
    (
        "Continuă de la pagina {0}",
        "Continue from page {0}",
        "Продолжить со страницы {0}",
    ),
    (
        "Continuă lectura ({0}%)",
        "Continue reading ({0}%)",
        "Продолжить чтение ({0}%)",
    ),
    (
        "Arată fișierul în managerul de fișiere",
        "Show the file in the file manager",
        "Показать файл в файловом менеджере",
    ),
    ("Copertă", "Cover", "Обложка"),
    (
        "Schimbă coperta (sau trage o imagine peste ea)",
        "Change the cover (or drop an image on it)",
        "Сменить обложку (или перетащите на неё картинку)",
    ),
    (
        "Alege o imagine…",
        "Choose an image…",
        "Выбрать изображение…",
    ),
    (
        "alegerea fișierelor nu e disponibilă aici",
        "choosing files is not available here",
        "выбор файлов здесь недоступен",
    ),
    (
        "Prima pagină a cărții",
        "The book's first page",
        "Первая страница книги",
    ),
    (
        "Pagina {0}, unde ai rămas",
        "Page {0}, where you stopped",
        "Страница {0}, где вы остановились",
    ),
    (
        "Coperta din catalog",
        "The catalog's cover",
        "Обложка из каталога",
    ),
    (
        "Poți și să tragi o imagine peste copertă.",
        "You can also drop an image on the cover.",
        "Можно и перетащить картинку на обложку.",
    ),
    (
        "Șterge cartea: fișierul merge în coșul de gunoi",
        "Delete the book: the file goes to the trash",
        "Удалить книгу: файл попадёт в корзину",
    ),
    (
        "Lasă imaginea aici\nca să devină copertă",
        "Drop the image here\nto make it the cover",
        "Отпустите картинку здесь,\nчтобы сделать её обложкой",
    ),
    (
        "Cartea nu mai există în catalog.",
        "The book is no longer in the catalog.",
        "Книги больше нет в каталоге.",
    ),
    (
        "Înapoi la pagina cărții",
        "Back to the book page",
        "Назад к странице книги",
    ),
    (
        "Înapoi la bibliotecă",
        "Back to the library",
        "Назад в библиотеку",
    ),
    ("Salvează", "Save", "Сохранить"),
    ("Salvează (Ctrl+S)", "Save (Ctrl+S)", "Сохранить (Ctrl+S)"),
    (
        "Renunță la modificări",
        "Discard changes",
        "Отменить изменения",
    ),
    (
        "Închide editarea",
        "Close editing",
        "Закрыть редактирование",
    ),
    ("Renunță", "Cancel", "Отмена"),
    ("fără copertă", "no cover", "нет обложки"),
    ("An", "Year", "Год"),
    ("Pagini", "Pages", "Страниц"),
    ("Editură", "Publisher", "Издательство"),
    ("ISBN", "ISBN", "ISBN"),
    ("Mărime", "Size", "Размер"),
    (
        "Arată cărțile din categorie",
        "Show the category's books",
        "Показать книги категории",
    ),
    (
        "Corectată manual: {0} câmp(uri); restul vin din catalog",
        "Edited by hand: {0} field(s); the rest comes from the catalog",
        "Исправлено вручную полей: {0}; остальное из каталога",
    ),
    (
        "Nu lipsește nimic",
        "Nothing is missing",
        "Ничего не пропущено",
    ),
    ("Lipsește: {0}", "Missing: {0}", "Не хватает: {0}"),
    ("verificată", "reviewed", "проверена"),
    (
        "Ce lipsește nu se poate afla; cartea nu mai apare la „date incomplete”",
        "What is missing cannot be found; the book leaves “incomplete data”",
        "Недостающее узнать нельзя; книга уходит из «неполных данных»",
    ),
    (
        "Ai ajuns la pagina {0} ({1}%)",
        "You reached page {0} ({1}%)",
        "Вы на странице {0} ({1}%)",
    ),
    (
        "Ai citit {0}% din carte",
        "You read {0}% of the book",
        "Прочитано {0}% книги",
    ),
    (
        "Deschisă în aplicația externă",
        "Opened in an external app",
        "Открыта во внешней программе",
    ),
    // Delete dialog.
    ("Ștergi cartea?", "Delete the book?", "Удалить книгу?"),
    (
        "Fișierul „{0}” se mută în coșul de gunoi, iar cartea dispare din bibliotecă. Îl poți recupera din coș; corecturile și progresul rămân în raft.db.",
        "The file “{0}” goes to the trash and the book leaves the library. You can restore it from the trash; corrections and progress stay in raft.db.",
        "Файл «{0}» попадёт в корзину, а книга исчезнет из библиотеки. Его можно восстановить из корзины; исправления и прогресс останутся в raft.db.",
    ),
    ("Mută în coș", "Move to trash", "В корзину"),
    (
        "„{0}” a fost mutată în coș",
        "“{0}” was moved to the trash",
        "«{0}» перемещена в корзину",
    ),
    // Edit form.
    ("Autori", "Authors", "Авторы"),
    ("Etichete", "Tags", "Теги"),
    ("Descriere", "Description", "Описание"),
    ("Verificată", "Reviewed", "Проверена"),
    ("lipsește", "missing", "пусто"),
    ("corectat", "edited", "исправлено"),
    ("În catalog: {0}", "In the catalog: {0}", "В каталоге: {0}"),
    ("din catalog", "from catalog", "из каталога"),
    (
        "Renunță la corectură și folosește valoarea din catalog",
        "Drop the correction and use the catalog's value",
        "Убрать исправление и взять значение из каталога",
    ),
    (
        "Corecturile se salvează separat de catalog și rămân după build-catalog --refresh-ai.",
        "Corrections are saved apart from the catalog and survive build-catalog --refresh-ai.",
        "Исправления хранятся отдельно от каталога и переживут build-catalog --refresh-ai.",
    ),
    (
        "Verificată: ce lipsește nu se poate afla (nu mai apare la „date incomplete”)",
        "Reviewed: what is missing cannot be found (leaves “incomplete data”)",
        "Проверена: недостающее узнать нельзя (уходит из «неполных данных»)",
    ),
    ("Următoarea", "Next", "Следующая"),
    (
        "Salvează și următoarea",
        "Save and next",
        "Сохранить и дальше",
    ),
    (
        "Cartea {0} din {1} din rezultatele bibliotecii",
        "Book {0} of {1} in the library's results",
        "Книга {0} из {1} в результатах библиотеки",
    ),
    ("Revino la catalog", "Back to catalog", "Вернуть каталог"),
    (
        "Șterge toate corecturile acestei cărți",
        "Remove all corrections of this book",
        "Удалить все исправления этой книги",
    ),
    (
        "titlul din carte, netradus",
        "the title in the book, untranslated",
        "название из книги, без перевода",
    ),
    (
        "separați prin virgulă",
        "separated by commas",
        "через запятую",
    ),
    ("gol = necunoscut", "empty = unknown", "пусто = неизвестно"),
    ("ru, en, ro…", "ru, en, ro…", "ru, en, ro…"),
    (
        "separate prin virgulă",
        "separated by commas",
        "через запятую",
    ),
    // Readers.
    ("Anterioara", "Previous", "Назад"),
    ("Săgeată stânga", "Left arrow", "Стрелка влево"),
    ("Pagina", "Page", "Страница"),
    ("din {0}", "of {0}", "из {0}"),
    ("Săgeată dreapta", "Right arrow", "Стрелка вправо"),
    ("Toată pagina", "Whole page", "Вся страница"),
    (
        "Se randează pagina {0}…",
        "Rendering page {0}…",
        "Отрисовка страницы {0}…",
    ),
    (
        "Pagina nu a putut fi afișată: {0}",
        "The page could not be shown: {0}",
        "Не удалось показать страницу: {0}",
    ),
    (
        "Se pregătește cartea…",
        "Preparing the book…",
        "Книга готовится…",
    ),
    (
        "Cartea nu a putut fi citită: {0}",
        "The book could not be read: {0}",
        "Не удалось прочитать книгу: {0}",
    ),
    ("Capitolul {0}", "Chapter {0}", "Глава {0}"),
    ("Cuprins", "Contents", "Оглавление"),
    // Categories (the catalog's fixed list).
    (
        "Alte limbaje de programare",
        "Other programming languages",
        "Другие языки программирования",
    ),
    (
        "Arhitectură, design patterns, clean code",
        "Architecture, design patterns, clean code",
        "Архитектура, паттерны, чистый код",
    ),
    ("Testare, QA", "Testing, QA", "Тестирование, QA"),
    ("Rețele", "Networking", "Сети"),
    (
        "Securitate, hacking",
        "Security, hacking",
        "Безопасность, хакинг",
    ),
    ("Baze de date, SQL", "Databases, SQL", "Базы данных, SQL"),
    (
        "Algoritmi, structuri de date, informatică",
        "Algorithms, data structures, computer science",
        "Алгоритмы, структуры данных, информатика",
    ),
    ("Dezvoltare de jocuri", "Game development", "Разработка игр"),
    (
        "Agile, Scrum, management de proiect",
        "Agile, Scrum, project management",
        "Agile, Scrum, управление проектами",
    ),
    (
        "Carieră în IT, interviuri, meseria de programator",
        "IT career, interviews, the programmer's trade",
        "Карьера в IT, собеседования, профессия программиста",
    ),
    (
        "Electronică, Arduino, Raspberry Pi, IoT",
        "Electronics, Arduino, Raspberry Pi, IoT",
        "Электроника, Arduino, Raspberry Pi, IoT",
    ),
    (
        "Științe exacte, inginerie",
        "Exact sciences, engineering",
        "Точные науки, инженерия",
    ),
    (
        "Învățarea limbilor străine",
        "Learning foreign languages",
        "Изучение иностранных языков",
    ),
    (
        "Dezvoltare personală, psihologie",
        "Personal development, psychology",
        "Саморазвитие, психология",
    ),
    (
        "Business, economie, finanțe",
        "Business, economics, finance",
        "Бизнес, экономика, финансы",
    ),
    (
        "Istorie, biografii",
        "History, biographies",
        "История, биографии",
    ),
    (
        "Bricolaj, renovare, casă",
        "DIY, renovation, home",
        "Своими руками, ремонт, дом",
    ),
    (
        "Medicină, sănătate",
        "Medicine, health",
        "Медицина, здоровье",
    ),
    ("Beletristică", "Fiction", "Художественная литература"),
    ("Fantasy", "Fantasy", "Фэнтези"),
    ("LitRPG", "LitRPG", "ЛитРПГ"),
    ("Science-fiction", "Science fiction", "Научная фантастика"),
    (
        "Istorie alternativă, popadanți",
        "Alternate history, isekai",
        "Альтернативная история, попаданцы",
    ),
    ("Altele", "Other", "Другое"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key passed to `t("…")` / `tf("…"` in the interface sources.
    fn used_keys() -> Vec<String> {
        let sources = [
            include_str!("lib.rs"),
            include_str!("book.rs"),
            include_str!("edit.rs"),
            include_str!("library.rs"),
            include_str!("reader.rs"),
            include_str!("settings.rs"),
            include_str!("text_reader.rs"),
        ];
        let mut keys = Vec::new();
        for src in sources {
            for marker in ["t(", "tf("] {
                for (i, _) in src.match_indices(marker) {
                    // Skip identifiers ending in t/tf (`list(`, `Sort(`…).
                    let before = src.get(..i).and_then(|s| s.chars().last());
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    // rustfmt may put a long key on the next line.
                    let rest = src.get(i + marker.len()..).unwrap_or_default().trim_start();
                    let Some(rest) = rest.strip_prefix('"') else {
                        continue;
                    };
                    let mut key = String::new();
                    let mut chars = rest.chars();
                    while let Some(c) = chars.next() {
                        match c {
                            '"' => break,
                            '\\' => match chars.next() {
                                Some('n') => key.push('\n'),
                                Some(other) => key.push(other),
                                None => {}
                            },
                            c => key.push(c),
                        }
                    }
                    keys.push(key);
                }
            }
        }
        keys
    }

    #[test]
    fn every_used_key_is_translated() {
        let keys = used_keys();
        assert!(keys.len() > 100, "found only {} keys", keys.len());
        let missing: Vec<&String> = keys
            .iter()
            .filter(|k| !index().contains_key(k.as_str()))
            .collect();
        assert!(missing.is_empty(), "untranslated: {missing:?}");
    }

    #[test]
    fn table_has_no_duplicates_and_placeholders_match() {
        let mut seen = std::collections::HashSet::new();
        for (ro, en, ru) in TABLE {
            assert!(seen.insert(*ro), "duplicate key {ro:?}");
            for i in 0..4 {
                let p = format!("{{{i}}}");
                assert_eq!(ro.contains(&p), en.contains(&p), "{ro:?} / {en:?}");
                assert_eq!(ro.contains(&p), ru.contains(&p), "{ro:?} / {ru:?}");
            }
        }
    }

    #[test]
    fn looks_up_and_fills_templates() {
        set(Lang::En);
        assert_eq!(t("Filtre"), "Filters");
        assert_eq!(tf("{0} din {1} cărți", &[&3, &10]), "3 of 10 books");
        assert_eq!(t_dyn("Beletristică"), "Fiction");
        assert_eq!(t_dyn("Python"), "Python");
        set(Lang::Ru);
        assert_eq!(t("Filtre"), "Фильтры");
        set(Lang::Ro);
        assert_eq!(t("Filtre"), "Filtre");
        assert_eq!(Lang::from_code("ru"), Some(Lang::Ru));
        assert_eq!(Lang::from_code("de"), None);
    }
}
