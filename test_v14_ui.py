"""v1.4 UX contract. Real Tk, real PDFs, no physical printer calls."""
from pathlib import Path
import tempfile
import tkinter as tk
import unittest
from unittest.mock import patch

from app import App


class V14InterfaceTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        self.base=Path(self.tmp.name)
        self.root=tk.Tk()
        self.app=App(self.root,self.base/'data')
        self.root.update()

    def tearDown(self):
        self.app.set_busy(False)
        self.root.update_idletasks()
        self.app.close()
        self.tmp.cleanup()

    def make_pdfs(self,count):
        import fitz
        pdf=self.base/'documento.pdf'
        with fitz.open() as doc:
            doc.new_page();doc.save(pdf)
        return self.app.store.add([pdf]*count)

    def test_custom_batch_is_editable_and_honors_seven_without_repeating(self):
        import time
        self.make_pdfs(9)
        self.assertEqual(str(self.app.batch_box['state']),'normal','Debe permitir escribir un valor personalizado')
        self.app.batch_box.delete(0,'end');self.app.batch_box.insert(0,'7')
        self.app.printer.set('Prueba')
        calls=[]
        def printer(path,printer,start,progress,job,stop):
            calls.append(path);job(len(calls),1);progress(1,1)
        def wait():
            deadline=time.monotonic()+20
            while self.app.busy and time.monotonic()<deadline:self.root.update();time.sleep(.01)
            self.assertFalse(self.app.busy)
        with patch('app.messagebox.askyesno',return_value=True),patch('app.printing.print_document',side_effect=printer):
            self.app.start();wait()
            self.assertEqual(len(calls),7)
            self.assertEqual(self.app.batch_count.get(),'Lote: 7 / 7 PDFs finalizados según Windows')
            self.app.start();wait()
        self.assertEqual(len(calls),9)
        self.assertEqual(len(set(calls)),9)
        self.assertTrue(all(r['physical_status']=='unconfirmed' for r in self.app.store.list_items()))
        self.assertEqual(str(self.app.batch_box['state']),'normal')

    def test_all_primary_controls_fit_small_desktop_without_clipping(self):
        self.assertLessEqual(self.root.minsize()[0],960)
        self.assertLessEqual(self.root.minsize()[1],640)
        self.root.geometry('960x640');self.root.update()
        self.assertEqual((self.root.winfo_width(),self.root.winfo_height()),(960,640))
        self.assertIn('v1.4',self.root.title())
        controls=[self.app.start_button,self.app.pause_button,self.app.resolve_button,self.app.delete_button,
                  self.app.clear_button,self.app.printer_box,self.app.batch_box,self.app.signature]
        for control in controls:
            with self.subTest(control=control):
                self.assertTrue(control.winfo_ismapped())
                self.assertGreaterEqual(control.winfo_height(),control.winfo_reqheight())
                self.assertGreaterEqual(control.winfo_width(),control.winfo_reqwidth())
                self.assertLessEqual(control.winfo_rooty()+control.winfo_height(),self.root.winfo_rooty()+640)
                self.assertLessEqual(control.winfo_rootx()+control.winfo_width(),self.root.winfo_rootx()+960)
        self.assertGreaterEqual(self.app.tree.winfo_height(),160)
        self.assertTrue(hasattr(self.app,'batch_hint'))
        self.assertIn('cantidad',self.app.batch_hint['text'])

    def test_invalid_custom_values_never_dispatch_or_change_journal(self):
        self.make_pdfs(2);self.app.printer.set('Prueba')
        before=self.app.store.list_items()
        for value in ('','0','-1','1.5','abc','1e2','+2','１２','1,000'):
            with self.subTest(value=value):
                self.app.batch_size.set(value)
                with patch('app.messagebox.showerror') as errors,patch('app.messagebox.askyesno') as confirm,patch('app.printing.print_document') as printer:
                    self.app.start()
                    errors.assert_called_once();confirm.assert_not_called();printer.assert_not_called()
                self.assertFalse(self.app.busy)
                self.assertEqual(self.app.store.list_items(),before)

    def test_very_large_custom_value_clamps_to_remaining_documents_before_confirmation(self):
        self.make_pdfs(2);self.app.printer.set('Prueba')
        self.app.batch_size.set(' 999999999999999999999999999 ')
        with patch('app.messagebox.askyesno',return_value=False) as confirm,patch('app.messagebox.showerror') as error:
            self.app.start()
            error.assert_not_called()
            self.assertIn('próximos 2 PDFs',confirm.call_args.args[1])
        self.assertFalse(self.app.busy)
        self.assertTrue(all(r['state']=='pending' for r in self.app.store.list_items()))

    def test_pause_explains_wait_and_locks_printer_settings(self):
        self.app.set_busy(True)
        self.assertEqual(str(self.app.settings_button['state']),'disabled')
        self.app.pause_button.invoke()
        self.assertTrue(self.app.stop.is_set())
        self.assertIn('Pausa solicitada',self.app.status.get())
        self.assertEqual(str(self.app.pause_button['state']),'disabled')
        self.app.set_busy(False)
        self.assertEqual(str(self.app.settings_button['state']),'normal')

    def test_recovery_popup_fits_small_desktop_with_preview_and_actions(self):
        self.root.geometry('960x640');self.root.update()
        item_id=self.make_pdfs(1)[0]
        self.app.store.claim(item_id,'Prueba');self.app.store.progress(item_id,1,total_pages=1)
        self.app.store.fail(item_id,'Corte de prueba')
        self.app.refresh();self.app.tree.selection_set(str(item_id));self.app.resolve();self.root.update()
        win=self.app.recovery_window
        self.assertLessEqual(win.winfo_width(),960)
        self.assertLessEqual(win.winfo_height(),640)
        self.assertIsNotNone(win.preview_image)
        def widgets(parent):
            for child in parent.winfo_children():
                yield child
                yield from widgets(child)
        for widget in widgets(win):
            if widget.winfo_class()=='TButton':
                self.assertGreaterEqual(widget.winfo_width(),widget.winfo_reqwidth())
                self.assertGreaterEqual(widget.winfo_height(),widget.winfo_reqheight())
                self.assertLessEqual(widget.winfo_rooty()+widget.winfo_height(),win.winfo_rooty()+win.winfo_height())

    def test_active_document_is_not_presented_as_an_interruption(self):
        item_id=self.make_pdfs(1)[0]
        self.app.store.claim(item_id,'Prueba');self.app.refresh()
        self.assertIn('1 en proceso',self.app.summary.get())
        self.assertIn('0 por revisar',self.app.summary.get())

    def test_no_routine_physical_supervision_in_table_buttons_or_summary(self):
        self.assertEqual(tuple(self.app.tree['columns']),('order','name','state','automatic','progress'))
        def widgets(parent):
            for child in parent.winfo_children():
                yield child
                yield from widgets(child)
        texts=[str(w['text']) for w in widgets(self.root) if 'text' in w.keys()]
        self.assertFalse(any(t in ('Confirmar impreso','No impreso / incompleto','Salida física') for t in texts))
        self.assertNotIn('confirmados',self.app.summary.get())
        self.assertFalse(hasattr(self.app,'confirm_physical'))


if __name__=='__main__':unittest.main()
