"""Backend contract tests; no physical printer is required."""
import importlib
import sys
import unittest
from types import SimpleNamespace
from unittest.mock import patch


class DiscoveryTests(unittest.TestCase):
    def test_discovery_is_lazy_and_uses_windows_enumeration(self):
        backend = importlib.import_module('printer_backend')
        spooler = SimpleNamespace(
            PRINTER_ENUM_LOCAL=2, PRINTER_ENUM_CONNECTIONS=4,
            EnumPrinters=lambda flags: [(0, '', 'Oficina', ''), (0, '', 'PDF', '')],
            GetDefaultPrinter=lambda: 'Oficina')
        with patch.dict(sys.modules, win32print=spooler):
            self.assertEqual(backend.list_printers(), ['Oficina', 'PDF'])
            self.assertEqual(backend.default_printer(), 'Oficina')


class WinError(Exception):
    def __init__(self, code):
        self.winerror = code
        super().__init__(code, 'GetJob', 'spooler failure')


class WaitTests(unittest.TestCase):
    def test_only_invalid_or_missing_job_is_accepted_as_queue_disappearance(self):
        import printer_backend as backend
        from threading import Event
        for code in (87, 1168, 5, 1722, 1801):
            def get_job(*args):
                raise WinError(code)
            spooler = SimpleNamespace(GetJob=get_job)
            if code in (87, 1168):
                backend._wait_for_job(spooler, 1, 42, Event(), 1)
            else:
                with self.assertRaises(WinError):
                    backend._wait_for_job(spooler, 1, 42, Event(), 1)

    def test_hard_error_flags_override_completion(self):
        import printer_backend as backend
        from threading import Event
        for flag in (2, 4, 32, 64, 512, 1024):
            with self.subTest(flag=flag):
                spooler = SimpleNamespace(GetJob=lambda *args: {'Status': flag | 4096})
                with self.assertRaisesRegex(backend.PrintUncertain, '42.*cola'):
                    backend._wait_for_job(spooler, 1, 42, Event(), 1)

    def test_paused_job_times_out_and_stop_never_claims_success(self):
        import printer_backend as backend
        from threading import Event
        spooler = SimpleNamespace(GetJob=lambda *args: {'Status': 1})
        ticks = iter([0, 0, 2])
        with self.assertRaisesRegex(backend.PrintUncertain, 'tiempo.*cola'):
            backend._wait_for_job(spooler, 1, 42, Event(), 1,
                                  clock=lambda: next(ticks), sleep=lambda s: None)
        stop = Event()
        stop.set()
        spooler.GetJob = lambda *args: self.fail('Must stop before querying queue')
        with self.assertRaisesRegex(backend.PrintUncertain, 'Detenido.*cola'):
            backend._wait_for_job(spooler, 1, 42, stop, 1)

    def test_waits_until_spooler_reports_complete_or_printed(self):
        import printer_backend as backend
        from threading import Event
        for terminal in (128, 4096):
            states = iter([{'Status': 16}, {'Status': terminal}])
            spooler = SimpleNamespace(GetJob=lambda *args: next(states))
            backend._wait_for_job(spooler, 'handle', 42, Event(), 2,
                                  sleep=lambda seconds: None)
            with self.assertRaises(StopIteration):
                next(states)


class GeometryTests(unittest.TestCase):
    def test_fit_rotates_landscape_and_centers_inside_printable_not_physical_origin(self):
        import printer_backend as backend
        caps = {8: 1000, 10: 1400, 110: 1200, 111: 1600, 112: 100, 113: 100}
        dc = SimpleNamespace(GetDeviceCaps=caps.__getitem__)
        self.assertEqual(backend._layout((2000, 1000), dc), (True, (150, 0, 850, 1400)))
        self.assertEqual(backend._layout((1000, 2000), dc), (False, (150, 0, 850, 1400)))
        caps[8] = 1200  # Broken driver's HORZRES may exceed physical sheet remainder.
        self.assertEqual(backend._layout((100, 100), dc), (False, (0, 150, 1100, 1250)))


class FakeImage:
    size = (400, 800)
    def transpose(self, method):
        return self
    def close(self):
        pass


class PrintTests(unittest.TestCase):
    def setUp(self):
        import printer_backend as backend
        from threading import Event
        self.backend = backend
        self.events = []
        self.stop = Event()
        self.status = 4096
        self.health = 0
        self.devmode = SimpleNamespace(Copies=7, Fields=0)
        self.caps = {8: 1000, 10: 1400, 110: 1200, 111: 1600, 112: 100, 113: 100}
        record = self.events.append
        dc = SimpleNamespace(
            CreatePrinterDC=lambda name: record(('dc', name)),
            GetSafeHdc=lambda: 99, GetDeviceCaps=self.caps.__getitem__,
            StartDoc=lambda name: (record(('start', name)) or 42),
            StartPage=lambda: record('start_page'), EndPage=lambda: record('end_page'),
            EndDoc=lambda: record('end_doc'), AbortDoc=lambda: record('abort'),
            DeleteDC=lambda: record('delete_dc'))
        self.dc = dc
        self.doc = SimpleNamespace(page_count=3, needs_pass=False,
                                   close=lambda: record('close_pdf'))
        self.spooler = SimpleNamespace(
            OpenPrinter=lambda name: (record(('open', name)) or 'handle'),
            GetPrinter=lambda handle, level: {'Status': self.health, 'pDevMode': self.devmode},
            ClosePrinter=lambda handle: record('close_printer'),
            GetJob=lambda handle, job, level: (record(('get_job', job)) or {'Status': self.status}))
        self.deps = SimpleNamespace(
            spooler=self.spooler, ui=SimpleNamespace(CreateDC=lambda: dc),
            gui=SimpleNamespace(ResetDC=lambda hdc, mode: record(('copies', mode.Copies))),
            fitz=SimpleNamespace(open=lambda path: self.doc),
            Image=SimpleNamespace(Transpose=SimpleNamespace(ROTATE_90=2)),
            ImageWin=SimpleNamespace(Dib=lambda image: SimpleNamespace(
                draw=lambda hdc, box: record(('draw', hdc, box)))))
        def render(doc, page, deps):
            record(('render', page))
            return FakeImage()
        self.render_patch = patch.object(backend, '_render_page', side_effect=render, create=True)
        self.render_patch.start()
        self.addCleanup(self.render_patch.stop)

    def run_print(self, **kwargs):
        with patch.object(self.backend, '_load_dependencies', return_value=self.deps, create=True):
            return self.backend.print_document(
                'lote.pdf', 'Oficina', kwargs.pop('start_page', 2),
                kwargs.pop('on_progress', lambda page, total: self.events.append(('progress', page, total))),
                kwargs.pop('on_job', lambda job, total: self.events.append(('job', job, total))),
                self.stop, **kwargs)

    def test_render_or_callback_failure_aborts_only_current_open_document(self):
        for stage in ('render', 'on_job', 'on_progress', 'end_doc'):
            with self.subTest(stage=stage):
                self.events.clear()
                def fail(*args):
                    raise OSError('driver broke')
                kwargs = {}
                if stage == 'render':
                    failure = patch.object(self.backend, '_render_page', side_effect=fail, create=True)
                elif stage == 'end_doc':
                    failure = patch.object(self.dc, 'EndDoc', side_effect=fail)
                else:
                    kwargs[stage] = fail
                    failure = patch.object(self.dc, 'StartPage', wraps=self.dc.StartPage)
                with failure:
                    with self.assertRaisesRegex(self.backend.PrintUncertain, 'cola.*reintentar') as caught:
                        self.run_print(**kwargs)
                self.assertIsInstance(caught.exception.__cause__, OSError)
                self.assertIn('abort', self.events)
                self.assertEqual(self.events[-3:], ['delete_dc', 'close_printer', 'close_pdf'])

    def test_stop_before_or_during_render_does_not_abort_or_submit_more_pages(self):
        for stage in ('before', 'render', 'progress', 'poll'):
            with self.subTest(stage=stage):
                self.events.clear()
                self.stop.clear()
                kwargs = {}
                if stage == 'before':
                    self.stop.set()
                elif stage == 'render':
                    self.backend._render_page.side_effect = lambda *args: (self.stop.set() or FakeImage())
                elif stage == 'progress':
                    kwargs['on_progress'] = lambda *args: self.stop.set()
                else:
                    self.dc.EndDoc = lambda: (self.events.append('end_doc'), self.stop.set())
                with self.assertRaises(self.backend.PrintUncertain):
                    self.run_print(**kwargs)
                self.assertNotIn('abort', self.events)
                self.assertNotIn(('get_job', 42), self.events)
                if stage == 'before':
                    self.assertEqual(self.events, [])
                if stage == 'render':
                    self.assertNotIn('start_page', self.events)
                if stage == 'progress':
                    self.assertNotIn(('render', 2), self.events)
                self.backend._render_page.side_effect = lambda *args: FakeImage()

    def test_unhealthy_printer_blocks_submission_before_startdoc(self):
        for flag in (1, 2, 4, 8, 16, 64, 128, 2048, 4096, 262144, 1048576, 2097152, 4194304):
            with self.subTest(flag=flag):
                self.events.clear()
                self.health = flag
                with self.assertRaisesRegex(self.backend.PrintUncertain, 'impresora.*cola'):
                    self.run_print()
                self.assertNotIn(('start', 'PDFLote - lote.pdf'), self.events)
                self.assertNotIn('abort', self.events)
                self.assertEqual(self.events[-2:], ['close_printer', 'close_pdf'])

    def test_resumed_document_callbacks_and_completion_order(self):
        self.assertIsNone(self.run_print())
        self.assertEqual(self.events, [
            ('open', 'Oficina'), ('dc', 'Oficina'), ('copies', 1),
            ('start', 'PDFLote - lote.pdf'), ('job', 42, 3),
            ('render', 1), 'start_page', ('draw', 99, (150, 0, 850, 1400)),
            'end_page', ('progress', 2, 3),
            ('render', 2), 'start_page', ('draw', 99, (150, 0, 850, 1400)),
            'end_page', ('progress', 3, 3), 'end_doc', ('get_job', 42),
            'delete_dc', 'close_printer', 'close_pdf'])
        self.assertEqual(self.devmode.Copies, 1)
        self.assertTrue(self.devmode.Fields & 0x100)


class RasterTests(unittest.TestCase):
    def test_real_pdf_is_rendered_rgb_150dpi_with_white_background(self):
        import printer_backend as backend
        try:
            import fitz
            from PIL import Image
        except ImportError:
            self.skipTest('PyMuPDF/Pillow no instalados en este intérprete')
        deps = SimpleNamespace(fitz=fitz, Image=Image)
        with fitz.open() as doc:
            page = doc.new_page(width=72, height=144)
            page.draw_rect(fitz.Rect(0, 0, 36, 72), color=None, fill=(1, 0, 0))
            with backend._render_page(doc, 0, deps) as image:
                self.assertEqual(image.size, (150, 300))
                self.assertEqual(image.mode, 'RGB')
                self.assertEqual(image.getpixel((140, 290)), (255, 255, 255))
                self.assertEqual(image.getpixel((20, 20)), (255, 0, 0))


if __name__ == '__main__':
    unittest.main()
