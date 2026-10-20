"""Recovery references are controller evidence, never physical confirmation."""
from pathlib import Path
import tempfile
import unittest

import fitz
from PIL import Image

import recovery_preview


def row(item_id, last_page=0, state='uncertain', **extra):
    return dict(id=item_id, position=item_id, name=f'{item_id}.pdf',
                snapshot=f'{item_id}.pdf', state=state, last_page=last_page,
                total_pages=10, start_page=1, **extra)


class ReferenceTests(unittest.TestCase):
    def test_selected_progress_is_the_reference(self):
        selected = row(2, 4)
        self.assertEqual(recovery_preview.recovery_reference([row(1, 8, 'sent'), selected], 2),
                         {'item': selected, 'page': 4})

    def test_zero_progress_uses_nearest_prior_sent_with_progress(self):
        previous = row(2, 6, 'sent')
        items = [row(1, 9, 'sent'), previous, row(3, 7, 'skipped'),
                 row(4, 0, 'sent'), row(5, 3), row(6), row(7, 8, 'sent')]
        self.assertEqual(recovery_preview.recovery_reference(items, 6),
                         {'item': previous, 'page': 6})

    def test_invalid_progress_never_invents_a_page(self):
        for value in (0, -1, None, '', 'oops', 1.5, True, float('inf')):
            with self.subTest(value=value):
                self.assertIsNone(recovery_preview.recovery_reference(
                    [row(1, value), row(2, 7, 'sent')], 1))
        self.assertIsNone(recovery_preview.recovery_reference([], 1))
        self.assertIsNone(recovery_preview.recovery_reference([row(1, 5, 'sent')], 99))

    def test_positive_total_caps_progress_without_mutating_rows(self):
        for total, expected in ((3, 3), (0, 8), (-1, 8), (None, 8), ('bad', 8)):
            with self.subTest(total=total):
                selected = row(1, 8)
                selected['total_pages'] = total
                result = recovery_preview.recovery_reference([selected], 1)
                self.assertEqual(result, {'item': selected, 'page': expected})
                self.assertEqual(selected['last_page'], 8)
                self.assertIsNot(result['item'], selected)

    def test_queue_position_not_input_order_determines_predecessor(self):
        previous = row(90, 5, 'sent')
        previous['position'] = 1
        selected = row(10)
        selected['position'] = 2
        future = row(2, 7, 'sent')
        future['position'] = 3
        self.assertEqual(recovery_preview.recovery_reference([future, selected, previous], 10),
                         {'item': previous, 'page': 5})

    def test_no_eligible_predecessor_means_no_reference(self):
        items = [row(1, 4, 'skipped'), row(2, 3, 'uncertain'),
                 row(3, 0, 'sent'), row(4), row(5, 8, 'sent')]
        self.assertIsNone(recovery_preview.recovery_reference(items, 4))

    def test_predecessor_cap_applies_to_invalid_selected_progress(self):
        previous = row(1, 9, 'sent')
        previous['total_pages'] = 4
        selected = row(2, None)
        selected['start_page'] = 6
        self.assertEqual(recovery_preview.recovery_reference([selected, previous], 2),
                         {'item': previous, 'page': 4})


class RenderTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.pdf = self.root / 'snapshot.pdf'
        with fitz.open() as document:
            for number, color in enumerate(((1, 0, 0), (0, 1, 0), (0, 0, 1)), 1):
                page = document.new_page(width=220, height=250)
                page.draw_rect(fitz.Rect(10, 20, 80, 100), color=color, fill=color)
                page.insert_text((20, 160), f'Unique content page {number}')
                if number == 2:
                    page.set_rotation(90)
            document.save(self.pdf)
        self.reference = {'item': {'snapshot': str(self.pdf)}, 'page': 3}

    def test_renders_actual_one_based_snapshot_page_as_white_rgb_without_writes(self):
        before = self.pdf.read_bytes()
        files = set(self.root.iterdir())
        image = recovery_preview.render_reference(self.reference)
        self.assertIsInstance(image, Image.Image)
        self.assertEqual(image.mode, 'RGB')
        self.assertEqual(image.size, (440, 500))
        self.assertEqual(image.getpixel((50, 60)), (0, 0, 255))
        self.assertEqual(image.getpixel((430, 490)), (255, 255, 255))
        self.assertEqual(self.pdf.read_bytes(), before)
        self.assertEqual(set(self.root.iterdir()), files)
        # The returned image is independent of the closed PDF handle.
        self.pdf.unlink()
        self.assertEqual(image.getpixel((50, 60)), (0, 0, 255))

    def test_rotation_and_custom_bounds_preserve_real_page_orientation(self):
        self.reference['page'] = 2
        image = recovery_preview.render_reference(self.reference, (250, 220))
        self.assertEqual(image.size, (250, 220))
        # Original upper-left green block rotates clockwise into the upper-right.
        self.assertEqual(image.getpixel((200, 30)), (0, 255, 0))
        self.assertEqual(image.getpixel((30, 200)), (255, 255, 255))
        for bounds in ((113, 77), (27, 81), (1, 1), (440, 500)):
            with self.subTest(bounds=bounds):
                image = recovery_preview.render_reference(self.reference, bounds)
                self.assertLessEqual(image.width, bounds[0])
                self.assertLessEqual(image.height, bounds[1])
                self.assertLessEqual(abs(image.width - image.height * 250 / 220), 2)

    def test_invalid_page_numbers_raise_value_error_instead_of_selecting_another_page(self):
        for page in (0, -1, 4, None, '2', 1.5, True):
            with self.subTest(page=page):
                self.reference['page'] = page
                with self.assertRaises(ValueError):
                    recovery_preview.render_reference(self.reference)

    def test_invalid_bounds_raise_value_error(self):
        for bounds in ((0, 500), (-1, 500), (440, 0), (1.5, 20),
                       (True, 20), (20,), (20, 30, 40), None, 'bad'):
            with self.subTest(bounds=bounds):
                with self.assertRaises(ValueError):
                    recovery_preview.render_reference(self.reference, bounds)

    def test_absent_or_malformed_reference_raises_value_error(self):
        for reference in (None, {}, {'page': 1}, {'item': {}, 'page': 1},
                          {'item': None, 'page': 1}):
            with self.subTest(reference=reference):
                with self.assertRaises(ValueError):
                    recovery_preview.render_reference(reference)

    def test_missing_snapshot_propagates_file_error(self):
        self.pdf.unlink()
        with self.assertRaises(fitz.FileNotFoundError):
            recovery_preview.render_reference(self.reference)

    def test_corrupt_snapshot_propagates_pdf_error(self):
        self.pdf.write_bytes(b'not a PDF')
        with self.assertRaises(fitz.FileDataError):
            recovery_preview.render_reference(self.reference)


if __name__ == '__main__':
    unittest.main()
