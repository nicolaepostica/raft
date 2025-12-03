"""Tests of rename-books' series volume rules: python3 scripts/test_rename_books.py"""
import importlib.machinery
import importlib.util
import os
import unittest

_HERE = os.path.dirname(os.path.abspath(__file__))
_loader = importlib.machinery.SourceFileLoader('rename_books', os.path.join(_HERE, 'rename-books'))
_spec = importlib.util.spec_from_loader('rename_books', _loader)
rb = importlib.util.module_from_spec(_spec)
_loader.exec_module(rb)


class VolumeTest(unittest.TestCase):
    def name(self, title, filename, meta=None, ai=None):
        vol = rb.find_volume(meta or {}, ai, filename, title)
        return rb.sanitize(rb.volume_title(title, vol)) if vol else rb.sanitize(title)

    def test_fb2_series_number_wins(self):
        self.assertEqual(self.name('Темный Лекарь', '01. Тёмный Лекарь 1.fb2', {'SeriesNumber': '1'}),
                         'Темный_Лекарь_Том_1')

    def test_number_already_in_the_title_is_not_doubled(self):
        self.assertEqual(self.name('Темный Лекарь 1', 'x.fb2', {'SeriesNumber': '1'}), 'Темный_Лекарь_Том_1')
        self.assertEqual(self.name('Император Пограничья #2', 'x.fb2', {'SeriesNumber': '2'}),
                         'Император_Пограничья_Том_2')

    def test_the_word_of_the_book_is_kept(self):
        self.assertEqual(self.name('Император Пограничья. Книга 2', 'x.fb2'), 'Император_Пограничья_Книга_2')
        self.assertEqual(self.name('Магия - не для оборотней', '01. Магия - не для оборотней. Часть 1.fb2'),
                         'Магия_-_не_для_оборотней_Часть_1')

    def test_roman_numerals_and_latin_titles(self):
        self.assertEqual(self.name('Системный Кузнец VII', 'x.fb2', {'SeriesNumber': '7'}),
                         'Системный_Кузнец_Том_7')
        self.assertEqual(self.name('The Expanse', 'x.epub', ai={'volume': 3}), 'The_Expanse_Vol_3')

    def test_leading_and_trailing_number_in_the_file_name(self):
        self.assertEqual(self.name('Бард', '3. Бард 3.fb2'), 'Бард_Том_3')

    def test_not_a_series(self):
        self.assertEqual(self.name('Python Cookbook', 'Python_Cookbook.pdf'), 'Python_Cookbook')
        self.assertEqual(self.name('Fluent Python', 'x.pdf', ai={'volume': None}), 'Fluent_Python')
        # a number that is part of the title is not a volume
        self.assertEqual(self.name('1984', '1984.epub'), '1984')
        self.assertEqual(self.name('100 Go Mistakes', '100_Go_Mistakes_2022.pdf'), '100_Go_Mistakes')

    def test_bad_values_are_ignored(self):
        for v in ('0', '', 'abc', '1-2', '99999', None):
            self.assertIsNone(rb.volume_number(v))
        self.assertEqual(rb.volume_number('3.0'), 3)
        self.assertEqual(rb.volume_number('IV'), 4)


if __name__ == '__main__':
    unittest.main()
