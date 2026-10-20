"""Windows UI orchestration with real PDFs; printer boundary is injected, never prints."""
import tempfile
import time
import tkinter as tk
import unittest
from pathlib import Path
from unittest.mock import patch
import fitz
from app import App


class V13InterfaceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name)
        self.pdf = self.base / 'original.pdf'
        with fitz.open() as doc:
            for n in range(3):
                page = doc.new_page()
                page.insert_text((50, 70), f'HOJA DE CONTROL {n+1}')
            doc.save(self.pdf)
        self.root = tk.Tk()
        self.app = App(self.root, self.base / 'data')

    def tearDown(self):
        self.app.set_busy(False)
        self.app.close()
        self.tmp.cleanup()

    def wait(self):
        deadline = time.monotonic() + 30
        while self.app.busy and time.monotonic() < deadline:
            self.root.update()
            time.sleep(.01)
        self.assertFalse(self.app.busy)

    def test_batch_of_twenty_stops_and_next_run_does_not_repeat(self):
        self.app.store.add([self.pdf] * 23)
        self.assertTrue(hasattr(self.app, 'batch_size'), 'Falta selector de lotes 20/50/100')
        self.app.batch_size.set('20')
        self.app.printer.set('Prueba sin hardware')
        calls = []
        def printer(path, printer, start, progress, job, stop):
            calls.append(path)
            job(len(calls), 3)
            progress(3, 3)
        with patch('app.messagebox.askyesno', return_value=True), patch('app.printing.print_document', side_effect=printer):
            self.app.start(); self.wait()
            self.assertEqual(len(calls), 20)
            self.assertEqual(self.app.batch_count.get(), 'Lote: 20 / 20 PDFs finalizados según Windows')
            self.assertEqual(len([r for r in self.app.store.list_items() if r['state']=='pending']), 3)
            self.app.start(); self.wait()
        self.assertEqual(len(calls), 23)
        self.assertEqual(len(set(calls)), 23)
        self.assertEqual(self.app.batch_count.get(), 'Lote: 3 / 3 PDFs finalizados según Windows')
        self.assertTrue(all(r['physical_status']=='unconfirmed' for r in self.app.store.list_items()))
        self.assertTrue(all(r['physical_confirmed_at'] is None for r in self.app.store.list_items()))
        self.assertFalse(any(e['action'].startswith('physical_') for e in self.app.store.events()))

    def test_batch_selector_retains_presets_and_accepts_custom_integer(self):
        self.assertEqual(tuple(self.app.batch_box['values']), ('20', '50', '100', 'Todos'))
        self.assertEqual(str(self.app.batch_box['state']), 'normal')
        self.app.set_busy(True)
        self.assertEqual(str(self.app.batch_box['state']), 'disabled')
        self.app.set_busy(False)
        self.assertEqual(str(self.app.batch_box['state']), 'normal')
        self.app.store.add([self.pdf] * 4)
        self.app.batch_box.set('2')
        self.app.printer.set('Prueba')
        def printer(path, printer, start, progress, job, stop):
            job(1, 3)
            progress(3, 3)
        with patch('app.messagebox.askyesno', return_value=True), \
             patch('app.printing.print_document', side_effect=printer) as dispatch:
            self.app.start(); self.wait()
        self.assertEqual(dispatch.call_count, 2)
        self.assertEqual([r['state'] for r in self.app.store.list_items()],
                         ['sent', 'sent', 'pending', 'pending'])

    def test_resolve_opens_preview_and_does_not_print_on_cancel(self):
        item_id, following = self.app.store.add([self.pdf] * 2)
        self.app.store.claim(item_id, 'Prueba')
        self.app.store.progress(item_id, 2, total_pages=3)
        self.app.store.fail(item_id, 'Corte simulado')
        before_items = self.app.store.list_items()
        before_events = self.app.store.events()
        self.app.refresh(); self.app.tree.selection_set(str(item_id))
        with patch('app.printing.print_document') as printer:
            self.app.resolve(); self.root.update()
            self.assertTrue(hasattr(self.app, 'recovery_window'), 'Falta popup con preview')
            window = self.app.recovery_window
            self.assertTrue(window.winfo_exists())
            self.assertIsNotNone(window.preview_image)
            self.assertIn('Página 2', window.preview_caption.get())
            self.assertEqual(window.recovery_item_id, item_id)
            window.destroy(); self.root.update()
            printer.assert_not_called()
        self.assertEqual(self.app.store.list_items()[0]['state'], 'uncertain')
        self.assertIsNone(self.app.store.pending())
        self.app.printer.set('Prueba')
        with patch('app.printing.print_document') as printer, \
             patch('app.messagebox.showinfo'), \
             patch('app.messagebox.showerror'), \
             patch('app.messagebox.askyesno', return_value=False):
            self.app.start(); self.root.update()
            printer.assert_not_called()
        self.assertEqual(self.app.store.list_items(), before_items)
        self.assertEqual(self.app.store.events(), before_events)
        self.assertEqual(self.app.store.list_items()[1]['id'], following)

    def test_error_automatically_opens_recovery_popup(self):
        item_id=self.app.store.add([self.pdf])[0]
        self.app.store.claim(item_id,'Prueba')
        self.app.store.progress(item_id,1,total_pages=3)
        self.app.store.fail(item_id,'Error simulado')
        self.app.messages.put(('error','Error simulado'))
        with patch('app.messagebox.showerror'):
            self.app.poll();self.root.update()
        win=getattr(self.app,'recovery_window',None)
        self.assertIsNotNone(win,'El preview debe abrirse automáticamente')
        self.assertEqual(win.recovery_item_id,item_id)
        self.assertEqual(self.app.tree.selection(),(str(item_id),))

    def test_delete_and_empty_buttons_require_confirmation_and_preserve_originals(self):
        ids=self.app.store.add([self.pdf]*2)
        self.app.refresh();self.app.tree.selection_set(str(ids[0]))
        self.assertTrue(hasattr(self.app,'delete_button'),'Falta botón Eliminar PDF')
        self.assertTrue(hasattr(self.app,'clear_button'),'Falta botón Vaciar carpeta')
        with patch('app.messagebox.askyesno',return_value=False):
            self.app.delete_button.invoke();self.app.clear_button.invoke()
        self.assertEqual(len(self.app.store.list_items()),2)
        with patch('app.messagebox.askyesno',return_value=True):
            self.app.delete_button.invoke()
            self.assertEqual(len(self.app.store.list_items()),1)
            self.app.clear_button.invoke()
        self.assertEqual(self.app.store.list_items(),[])
        self.assertTrue(self.pdf.exists())
        self.assertEqual(list(self.app.store.documents.glob('*.pdf')),[])
        self.app.set_busy(True)
        self.assertEqual(str(self.app.delete_button['state']),'disabled')
        self.assertEqual(str(self.app.clear_button['state']),'disabled')

    def test_identity_has_icon_signature_and_bundle_assets(self):
        from PIL import Image
        self.assertTrue(hasattr(self.app,'app_icon'),'Falta icono de ventana')
        self.assertIsNotNone(self.app.app_icon)
        self.assertEqual(self.app.signature['text'],'Creado por siriusgoldsmith · GitHub')
        self.root.update()
        self.assertGreaterEqual(self.app.signature.winfo_height(),self.app.signature.winfo_reqheight())
        with Image.open(Path(__file__).parent/'assets'/'pdflote.ico') as icon:
            self.assertEqual(icon.format,'ICO')
            self.assertIn((256,256),icon.ico.sizes())
        spec=(Path(__file__).parent/'PDFLote.spec').read_text()
        self.assertIn("icon='assets/pdflote.ico'",spec)
        self.assertIn("('assets', 'assets')",spec)

    def test_fifty_and_hundred_limits_stop_at_exact_boundary(self):
        for limit in (50,100):
            with self.subTest(limit=limit):
                self.app.store.clear_items()
                self.app.store.add([self.pdf]*(limit+1))
                self.app.batch_size.set(str(limit));self.app.printer.set('Prueba')
                calls=[]
                def printer(path,printer,start,progress,job,stop):
                    calls.append(path);job(len(calls),3);progress(3,3)
                with patch('app.messagebox.askyesno',return_value=True),patch('app.printing.print_document',side_effect=printer):
                    self.app.start();self.wait()
                self.assertEqual(len(calls),limit)
                self.assertEqual(sum(r['state']=='pending' for r in self.app.store.list_items()),1)

    def test_restart_automatically_opens_preview_without_dispatch(self):
        item_id=self.app.store.add([self.pdf])[0]
        self.app.store.claim(item_id,'Prueba')
        self.app.store.progress(item_id,2,total_pages=3)
        self.app.close()
        self.root=tk.Tk();self.app=App(self.root,self.base/'data')
        deadline=time.monotonic()+3
        with patch('app.printing.print_document') as printer:
            while not getattr(self.app,'recovery_window',None) and time.monotonic()<deadline:
                self.root.update();time.sleep(.02)
            win=getattr(self.app,'recovery_window',None)
            self.assertIsNotNone(win)
            self.assertIsNotNone(win.preview_image)
            self.assertIn('Página 2',win.preview_caption.get())
            printer.assert_not_called()
        self.assertIsNone(self.app.store.pending())

    def test_error_does_not_count_failed_pdf_or_send_following_pdf(self):
        self.app.store.add([self.pdf]*3)
        self.app.batch_size.set('20');self.app.printer.set('Prueba')
        calls=[]
        def printer(path,printer,start,progress,job,stop):
            calls.append(path);job(len(calls),3)
            if len(calls)==2:
                progress(1,3);raise RuntimeError('Interrupción simulada')
            progress(3,3)
        with patch('app.messagebox.askyesno',return_value=True),patch('app.printing.print_document',side_effect=printer):
            self.app.start();self.wait()
        self.assertEqual(len(calls),2)
        self.assertEqual(self.app.batch_count.get(),'Lote: 1 / 3 PDFs finalizados según Windows')
        self.assertEqual([r['state'] for r in self.app.store.list_items()],['sent','uncertain','pending'])
        self.assertIsNotNone(self.app.recovery_window.preview_image)
        self.assertIn('Página 1',self.app.recovery_window.preview_caption.get())

    def test_popup_next_pdf_confirms_current_without_printing(self):
        first,second=self.app.store.add([self.pdf]*2)
        self.app.store.claim(first,'Prueba');self.app.store.progress(first,3,total_pages=3)
        self.app.store.fail(first,'Error de cola')
        self.app.refresh();self.app.tree.selection_set(str(first));self.app.resolve()
        def descendants(widget):
            for child in widget.winfo_children():
                yield child
                yield from descendants(child)
        button=next(w for w in descendants(self.app.recovery_window) if 'text' in w.keys() and str(w['text']).startswith('Continuar con el siguiente PDF'))
        with patch('app.messagebox.askyesno',return_value=True),patch('app.printing.print_document') as printer:
            button.invoke();printer.assert_not_called()
        row=self.app.store.list_items()[0]
        self.assertEqual(row['physical_status'],'confirmed')
        self.assertEqual(row['queue_outcome'],'review')
        self.assertEqual(self.app.store.pending()['id'],second)


if __name__ == '__main__':
    unittest.main()
