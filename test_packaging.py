"""Regression guard for pywin32's dynamically imported timezone support."""
import os
from pathlib import Path
import unittest


class PackagingTests(unittest.TestCase):
    def test_windows_build_includes_dynamic_timezone_dependency(self):
        base = Path(__file__).resolve().parent
        self.assertIn('--hidden-import win32timezone',
                      (base / 'build-windows.bat').read_text(encoding='utf-8'))
        self.assertIn("'win32timezone'", (base / 'PDFLote.spec').read_text(encoding='utf-8'))

    @unittest.skipUnless(os.environ.get('PDFLOTE_CHECK_FROZEN'), 'requires built Windows executable and PyInstaller')
    def test_frozen_archive_contains_timezone_module(self):
        from PyInstaller.archive.readers import CArchiveReader
        exe = Path(__file__).resolve().parent / 'dist' / 'PDFLote.exe'
        archive = CArchiveReader(str(exe))
        pyz = archive.open_embedded_archive('PYZ.pyz')
        self.assertIn('win32timezone', pyz.toc)


if __name__ == '__main__':
    unittest.main()
