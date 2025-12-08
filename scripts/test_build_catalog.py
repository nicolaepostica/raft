"""Tests of build-catalog's metadata without AI: python3 scripts/test_build_catalog.py"""
import importlib.machinery
import importlib.util
import os
import unittest
from unittest import mock

_HERE = os.path.dirname(os.path.abspath(__file__))
_loader = importlib.machinery.SourceFileLoader('build_catalog', os.path.join(_HERE, 'build-catalog'))
_spec = importlib.util.spec_from_loader('build_catalog', _loader)
bc = importlib.util.module_from_spec(_spec)
_loader.exec_module(bc)


class LocalMetadataTest(unittest.TestCase):
    def meta(self, rel, meta=None, text='', info=None, pages=None):
        info = info or {'authors': [], 'language': None, 'genres': []}
        with mock.patch.object(bc, 'file_info', return_value=info), \
                mock.patch.object(bc, 'title_from_pages', return_value=pages):
            return bc.local_metadata('/x/' + rel, rel, meta or {}, text)

    def test_a_renamed_file_keeps_its_name_unless_the_inside_title_is_the_same(self):
        self.assertEqual(self.meta('Clean_Code_2008.pdf', {'Title': 'Clean Code (for John)'})['title'],
                         'Clean Code')
        self.assertEqual(self.meta('Android_Studio_4_1_Essentials_2020.epub',
                                   {'Title': 'Android Studio 4.1 Essentials'})['title'],
                         'Android Studio 4.1 Essentials')
        self.assertEqual(self.meta('Clean_Code_2008.pdf')['year'], 2008)

    def test_an_unnamed_file_takes_the_title_inside_or_on_the_first_page(self):
        self.assertEqual(self.meta('download_3.pdf', {'Title': 'Kafka Streams in Action'})['title'],
                         'Kafka Streams in Action')
        self.assertEqual(self.meta('download_3.pdf', {'Title': 'Microsoft Word - doc1.docx'},
                                   pages='Kafka Streams')['title'], 'Kafka Streams')
        self.assertEqual(self.meta('scan_0001.pdf', pages=None)['title'], 'scan 0001')
        # A name that says something is kept.
        self.assertEqual(self.meta('Clean Code - Robert Martin.pdf', pages='Prentice Hall')['title'],
                         'Clean Code Robert Martin')

    def test_series_volume_is_in_the_title(self):
        self.assertEqual(self.meta('01. Тёмный Лекарь 1.fb2', {'Title': 'Темный Лекарь', 'SeriesNumber': '1'})
                         ['title'], 'Темный Лекарь Том 1')

    def test_authors_language_isbn_tags(self):
        m = self.meta('x.epub', {'Author': 'Smyth, Neil'},
                      text='ISBN 978-1-61729-959-9 ' + 'word ' * 100,
                      info={'authors': [], 'language': 'en-US', 'genres': ['sf_fantasy', 'unknown']})
        self.assertEqual(m['authors'], ['Neil Smyth'])
        self.assertEqual(m['language'], 'en')
        self.assertEqual(m['isbn'], '9781617299599')
        self.assertEqual(m['tags'], ['fantasy'])


class UnchangedTest(unittest.TestCase):
    def test_a_book_without_a_cover_is_not_redone_but_a_lost_cover_is(self):
        import tempfile
        with tempfile.TemporaryDirectory() as root:
            os.makedirs(os.path.join(root, 'covers'))
            open(os.path.join(root, 'covers', 'a.webp'), 'wb').close()
            st = mock.Mock(st_size=10, st_mtime=5.0)
            self.assertTrue(bc.unchanged(('a', 10, 5.0, None), st, root))
            self.assertTrue(bc.unchanged(('a', 10, 5.0, 'covers/a.webp'), st, root))
            self.assertFalse(bc.unchanged(('a', 10, 5.0, 'covers/gone.webp'), st, root))
            self.assertFalse(bc.unchanged(('a', 11, 5.0, None), st, root))
            self.assertFalse(bc.unchanged(('a', 10, 6.0, None), st, root))
            self.assertFalse(bc.unchanged(None, st, root))


class SetCategoryTest(unittest.TestCase):
    def test_every_book_under_the_folder_and_nothing_else(self):
        import json
        import sqlite3
        db = sqlite3.connect(':memory:')
        db.executescript(bc.SCHEMA)
        for bid, path in (('a', 'Mag/1995/x.pdf'), ('b', 'Mag/y.pdf'), ('c', 'Magazine/z.pdf'), ('d', 'w.pdf')):
            db.execute("INSERT INTO books(id, path, categories) VALUES(?, ?, '[\"other\"]')", (bid, path))
            db.execute("INSERT INTO books_fts(id, categories) VALUES(?, 'other')", (bid,))
        self.assertEqual(bc.set_category(db, 'diy-home', ['Mag/']), 2)
        cats = dict(db.execute('SELECT id, categories FROM books'))
        self.assertEqual({k: json.loads(v) for k, v in cats.items()},
                         {'a': ['diy-home'], 'b': ['diy-home'], 'c': ['other'], 'd': ['other']})
        self.assertEqual(db.execute("SELECT categories FROM books_fts WHERE id='a'").fetchone()[0], 'diy-home')
        with self.assertRaises(SystemExit):
            bc.set_category(db, 'no-such', ['Mag'])
        with self.assertRaises(SystemExit):
            bc.set_category(db, 'diy-home', ['Typo'])


class LanguageCategoriesTest(unittest.TestCase):
    def test_other_languages_is_put_right(self):
        lc = bc.language_categories
        # Next to the category that covers the language: dropped.
        self.assertEqual(lc(['mobile', 'other-languages'], ['android', 'java']), ['mobile'])
        self.assertEqual(lc(['frontend', 'other-languages'], ['angular', 'typescript']), ['frontend'])
        # English textbooks: natural languages.
        self.assertEqual(lc(['other-languages', 'languages-learning'], ['english', 'grammar']),
                         ['languages-learning'])
        # A language with its own category: replaced.
        self.assertEqual(lc(['other-languages', 'architecture'], ['kotlin', 'functional-programming']),
                         ['java-kotlin', 'architecture'])
        # A language without one keeps it, alone or next to another category.
        self.assertEqual(lc(['other-languages', 'mobile'], ['swift', 'ios']), ['other-languages', 'mobile'])
        self.assertEqual(lc(['other-languages', 'c-cpp'], ['visual-basic', 'win32']),
                         ['other-languages', 'c-cpp'])
        self.assertEqual(lc(['other-languages'], ['perl', 'xml']), ['other-languages'])
        self.assertEqual(lc(['python', 'data-ai'], ['pandas']), ['python', 'data-ai'])


class HelpersTest(unittest.TestCase):
    def test_split_authors(self):
        self.assertEqual(bc.split_authors('A. Smith; B. Jones'), ['A. Smith', 'B. Jones'])
        self.assertEqual(bc.split_authors('Ann Smith, Bob Jones'), ['Ann Smith', 'Bob Jones'])
        self.assertEqual(bc.split_authors('Lim, Greg'), ['Greg Lim'])
        self.assertEqual(bc.split_authors(''), [])

    def test_isbn_checksums(self):
        self.assertEqual(bc.find_isbn('ISBN: 5-7836-0106-3'), '5783601063')
        self.assertIsNone(bc.find_isbn('ISBN 978-1-61729-959-0'))   # wrong check digit
        self.assertIsNone(bc.find_isbn('no number here'))

    def test_language(self):
        self.assertEqual(bc.language_code('rus'), 'ru')
        self.assertIsNone(bc.language_code('und-xyz1'))
        self.assertEqual(bc.guess_language('Привет мир ' * 40), 'ru')
        self.assertEqual(bc.guess_language('Hello world ' * 40), 'en')
        self.assertEqual(bc.guess_language('Învățați programarea în științe ' * 20), 'ro')
        self.assertIsNone(bc.guess_language('short'))

    def test_mojibake_and_weak_names(self):
        self.assertTrue(bc.mojibake('Áèëë Áåäæåê'))
        self.assertFalse(bc.mojibake('Bill Bejeck'))
        self.assertTrue(bc.weak_name('download 3'))
        self.assertFalse(bc.weak_name('Clean Code'))


if __name__ == '__main__':
    unittest.main()
