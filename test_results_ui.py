"""v1.4 shows Windows results, without exposing legacy physical review."""
import tempfile
import time
import tkinter as tk
from pathlib import Path
import unittest
from unittest.mock import patch


class ResultInterfaceTests(unittest.TestCase):
    def assert_no_physical_controls(self, app):
        self.assertEqual(tuple(app.tree['columns']),
                         ('order', 'name', 'state', 'automatic', 'progress'))
        for name in ('confirm_button', 'incomplete_button', 'confirm_physical'):
            self.assertFalse(hasattr(app, name), name)
        def descendants(widget):
            for child in widget.winfo_children():
                yield child
                yield from descendants(child)
        texts = [str(widget['text']) for widget in descendants(app.root)
                 if 'text' in widget.keys()]
        self.assertNotIn('Confirmar impreso', texts)
        self.assertNotIn('No impreso / incompleto', texts)
        headings = [app.tree.heading(column, 'text') for column in app.tree['columns']]
        self.assertNotIn('Salida física', headings)

    def test_five_columns_without_routine_physical_confirmation_controls(self):
        from app import App
        with tempfile.TemporaryDirectory() as tmp:
            root = tk.Tk()
            app = App(root, Path(tmp))
            try:
                root.update()
                self.assert_no_physical_controls(app)
                app.set_busy(True)
                self.assert_no_physical_controls(app)
                app.set_busy(False)
                self.assert_no_physical_controls(app)
            finally:
                app.set_busy(False)
                app.close()

    def test_legacy_physical_statuses_and_journal_survive_refresh_and_restart(self):
        from app import App
        from core import QueueStore
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            pdf = folder / 'prueba.pdf'
            pdf.write_bytes(b'%PDF-1.7\ntest\n%%EOF')
            data = folder / 'data'
            store = QueueStore(data / 'queue.sqlite3')
            unconfirmed, confirmed, incomplete, following = store.add([pdf] * 4)
            for item_id in (unconfirmed, confirmed, incomplete):
                store.claim(item_id, 'Prueba')
                store.mark_sent(item_id)
            store.confirm_physical(confirmed, True)
            store.confirm_physical(incomplete, False)
            before_items = store.list_items()
            before_events = store.events()
            self.assertEqual([row['physical_status'] for row in before_items],
                             ['unconfirmed', 'confirmed', 'incomplete', 'unconfirmed'])
            for _ in range(2):
                root = tk.Tk()
                app = App(root, data)
                try:
                    with patch.object(app.store, 'confirm_physical') as confirm:
                        app.refresh()
                        root.update()
                        self.assert_no_physical_controls(app)
                        for item_id in (unconfirmed, confirmed, incomplete):
                            app.tree.selection_set(str(item_id))
                            app.refresh()
                            values = tuple(app.tree.item(str(item_id))['values'])
                            self.assertEqual(len(values), 5)
                            self.assertEqual(values[3], 'Finalizado según Windows')
                            self.assertFalse(set(values) & {
                                'Sin confirmar', 'Impreso y confirmado',
                                'No impreso / incompleto', 'unconfirmed',
                                'confirmed', 'incomplete'})
                            self.assertFalse(set(app.tree.item(str(item_id))['tags']) &
                                             {'confirmed', 'incomplete'})
                        self.assertIsNone(app.store.pending(),
                                          'Legacy incomplete work must still block the following PDF')
                        app.printer.set('Prueba')
                        with patch('app.printing.print_document') as printer, \
                             patch('app.messagebox.showwarning'), \
                             patch('app.messagebox.showinfo'), \
                             patch('app.messagebox.showerror'), \
                             patch('app.messagebox.askyesno', return_value=False):
                            app.start()
                            root.update()
                            printer.assert_not_called()
                        confirm.assert_not_called()
                        self.assertEqual(app.store.list_items(), before_items)
                        self.assertEqual(app.store.events(), before_events)
                        self.assertEqual(app.store.list_items()[-1]['id'], following)
                finally:
                    app.close()
            reopened = QueueStore(data / 'queue.sqlite3')
            self.assertEqual(reopened.list_items(), before_items)
            self.assertEqual(reopened.events(), before_events)

    def test_windows_completion_never_automatically_confirms_paper(self):
        import fitz
        from app import App
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            pdf = folder / 'prueba.pdf'
            with fitz.open() as doc:
                doc.new_page()
                doc.save(pdf)
            root = tk.Tk()
            app = App(root, folder / 'data')
            try:
                item_id = app.store.add([pdf])[0]
                app.printer.set('Prueba')
                def printer(path, printer, start, progress, job, stop):
                    job(42, 1)
                    progress(1, 1)
                with patch('app.printing.print_document', side_effect=printer) as dispatch, \
                     patch('app.messagebox.askyesno', return_value=True), \
                     patch.object(app.store, 'confirm_physical') as confirm:
                    app.start()
                    deadline = time.monotonic() + 15
                    while app.busy and time.monotonic() < deadline:
                        root.update()
                        time.sleep(.01)
                    self.assertFalse(app.busy)
                    dispatch.assert_called_once()
                    confirm.assert_not_called()
                row = app.store.list_items()[0]
                self.assertEqual(row['state'], 'sent')
                self.assertEqual(row['queue_outcome'], 'completed')
                self.assertEqual(row['physical_status'], 'unconfirmed')
                self.assertIsNone(row['physical_confirmed_at'])
                self.assertFalse(any(event['action'].startswith('physical_')
                                     for event in app.store.events(item_id)))
                self.assert_no_physical_controls(app)
                values = tuple(app.tree.item(str(item_id))['values'])
                self.assertEqual(len(values), 5)
                self.assertEqual(values[3], 'Finalizado según Windows')
            finally:
                app.close()


if __name__ == '__main__':
    unittest.main()
