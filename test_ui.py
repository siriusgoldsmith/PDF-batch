import tempfile
import tkinter as tk
import unittest
from pathlib import Path

class InterfaceTests(unittest.TestCase):
    def test_window_has_ordered_queue_and_resume_controls(self):
        from app import App
        with tempfile.TemporaryDirectory() as tmp:
            root = tk.Tk()
            try:
                app = App(root, Path(tmp))
                root.update()
                self.assertEqual(tuple(app.tree['columns']), ('order', 'name', 'state', 'automatic', 'progress'))
                self.assertIn('v1.4', root.title())
                self.assertEqual(root.minsize(), (960, 640))
                for name in ('confirm_button', 'incomplete_button', 'confirm_physical'):
                    self.assertFalse(hasattr(app, name), name)
                self.assertEqual(app.start_button['text'], 'Iniciar / continuar')
                self.assertEqual(app.resolve_button['text'], 'Resolver interrupción')
                self.assertFalse(app.busy)
            finally:
                app.close()

    def test_retry_from_page_three_updates_real_journal_without_regression(self):
        import app as module
        import fitz
        import time
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as tmp:
            pdf=Path(tmp)/'tres.pdf'
            doc=fitz.open()
            for _ in range(3):doc.new_page()
            doc.save(pdf);doc.close()
            root=tk.Tk()
            try:
                app=module.App(root,Path(tmp)/'data')
                item_id=app.store.add([pdf])[0]
                app.store.claim(item_id,'Prueba')
                app.store.progress(item_id,2,total_pages=3,job_id=10)
                app.store.fail(item_id,'corte de prueba')
                app.store.resolve(item_id,'retry',start_page=3)
                app.printer.set('Prueba')
                calls=[]
                def fake_print(path,printer,start_page,on_progress,on_job,stop_event):
                    calls.append(start_page)
                    on_job(11,3)
                    on_progress(3,3)
                with patch.object(module.printing,'print_document',side_effect=fake_print), patch.object(module.messagebox,'askyesno',return_value=True), patch.object(module.messagebox,'showerror') as errors:
                    app.start()
                    deadline=time.monotonic()+8
                    while app.busy and time.monotonic()<deadline:
                        root.update();time.sleep(.03)
                    self.assertFalse(app.busy)
                    self.assertEqual(calls,[3])
                    self.assertEqual(app.store.list_items()[0]['state'],'sent')
                    errors.assert_not_called()
            finally:app.close()

    def test_201_documents_stop_at_101_and_resume_without_repeating_first_100(self):
        import app as module
        import fitz
        import time
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp)
            doc=fitz.open();doc.new_page();payload=doc.tobytes();doc.close()
            paths=[]
            for n in range(201):
                p=folder/f'{n:03d}.pdf';p.write_bytes(payload);paths.append(p)
            data=folder/'data'
            root=tk.Tk();app=module.App(root,data)
            ids=app.store.add(paths)
            app.printer.set('Prueba')
            calls=[]
            def printer(path,printer,start,on_progress,on_job,stop):
                calls.append(path)
                on_job(len(calls),1)
                if len(calls)==101:raise RuntimeError('Fallo deliberado en documento 101')
                on_progress(1,1)
            def wait():
                deadline=time.monotonic()+60
                while app.busy and time.monotonic()<deadline:
                    root.update();time.sleep(.01)
                self.assertFalse(app.busy)
            with patch.object(module.printing,'print_document',side_effect=printer),patch.object(module.messagebox,'askyesno',return_value=True),patch.object(module.messagebox,'showerror'):
                try:
                    app.start();wait()
                    rows=app.store.list_items()
                    self.assertEqual(sum(r['state']=='sent' for r in rows),100)
                    self.assertEqual(rows[100]['state'],'uncertain')
                    self.assertIsNone(app.store.pending())
                    app.close()
                    root=tk.Tk();app=module.App(root,data)
                    self.assertIsNone(app.store.pending())
                    app.store.resolve(ids[100],'sent')
                    app.printer.set('Prueba')
                    app.start();wait()
                    self.assertEqual(len(calls),201)
                    self.assertEqual(len(set(calls)),201)
                    self.assertTrue(all(r['state']=='sent' for r in app.store.list_items()))
                finally:app.close()

if __name__ == '__main__':
    unittest.main()
