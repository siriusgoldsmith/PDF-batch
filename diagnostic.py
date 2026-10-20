"""Non-printing diagnostic for the real Windows EXE and its v1.4 UI."""
import json
from pathlib import Path
import tempfile
import time
import tkinter as tk


def run(report_path):
    import fitz
    import printer_backend
    from app import App
    import app as module
    deps=printer_backend._load_dependencies()
    import importlib
    import datetime
    import pywintypes
    timezone=importlib.import_module('win32timezone')
    stamp=pywintypes.Time(datetime.datetime(2026,1,1,tzinfo=timezone.TimeZoneInfo.utc()))
    assert stamp.utcoffset()==datetime.timedelta(0)
    output=Path(report_path).resolve()
    with tempfile.TemporaryDirectory() as tmp:
        base=Path(tmp)
        pdf=base/'Documento de control.pdf'
        with fitz.open() as doc:
            page=doc.new_page()
            page.insert_text((50,65),'PDF Lote / Documento de control',fontsize=22)
            page.insert_text((50,110),'Pagina 1 - Referencia para recuperar una interrupcion',fontsize=12)
            page.draw_rect(fitz.Rect(50,145,545,215),color=(.1,.25,.4),fill=(.9,.94,.98))
            page.insert_text((70,180),'Comprueba el papel antes de decidir donde continuar.',fontsize=12)
            for y in range(260,700,40):page.draw_line((50,y),(545,y),color=(.8,.84,.88))
            doc.save(pdf)
        with fitz.open(pdf) as doc:
            image=printer_backend._render_page(doc,0,deps)
            assert image.width>0 and image.height>0
            image.close()
        root=tk.Tk()
        app=None
        try:
            app=App(root,base/'data')
            root.after_cancel(app.recovery_timer)
            ids=app.store.add([pdf]*250)
            for item_id in ids[:3]:
                app.store.claim(item_id,'Diagnóstico (sin impresión)')
                app.store.progress(item_id,1,total_pages=1,job_id='diagnostic')
                app.store.mark_sent(item_id)
            # Legacy observations remain in history, never impose routine supervision.
            app.store.confirm_physical(ids[1],True)
            saved_history=app.store.list_items()[1]
            app.refresh();root.update()
            assert tuple(app.tree['columns'])==('order','name','state','automatic','progress')
            assert app.tree.set(str(ids[0]),'automatic')=='Finalizado según Windows'
            assert not hasattr(app,'confirm_physical') and not hasattr(app,'confirm_button')
            assert len(app.tree.get_children())==250
            assert app.app_icon is not None
            assert app.signature['text']=='Creado por siriusgoldsmith · GitHub'
            assert tuple(app.batch_box['values'])==('20','50','100','Todos')
            assert str(app.batch_box['state'])=='normal'
            app.batch_size.set('35');app.printer.set('Prueba interna (no imprime)')
            calls=[]
            def simulated_print(path,printer,start,on_progress,on_job,stop):
                calls.append(path);on_job(len(calls),1);on_progress(1,1)
            original_print=printer_backend.print_document
            original_confirm=module.messagebox.askyesno
            printer_backend.print_document=simulated_print
            module.messagebox.askyesno=lambda *args,**kwargs: True
            try:
                app.start()
                deadline=time.monotonic()+45
                while app.busy and time.monotonic()<deadline:root.update();time.sleep(.01)
                assert not app.busy and len(calls)==35
                assert app.batch_count.get()=='Lote: 35 / 35 PDFs finalizados según Windows'
                assert sum(r['state']=='sent' for r in app.store.list_items())==38
                assert app.store.list_items()[1]==saved_history
                assert app.store.list_items()[38]['state']=='pending'
            finally:
                printer_backend.print_document=original_print
                module.messagebox.askyesno=original_confirm
            from PIL import ImageGrab
            root.update();ImageGrab.grab().save(output.with_suffix('.png'))
            root.geometry('960x640');root.update()
            for widget in (app.start_button,app.pause_button,app.batch_box,app.delete_button,app.clear_button,app.signature):
                assert widget.winfo_height()>=widget.winfo_reqheight()
                assert widget.winfo_rooty()+widget.winfo_height()<=root.winfo_rooty()+640
            ImageGrab.grab().save(output.with_name(output.stem+'-compact.png'))
            item_id=ids[38]
            app.store.claim(item_id,'Diagnóstico (sin impresión)')
            app.store.progress(item_id,1,total_pages=1,job_id='diagnostic-interruption')
            app.store.fail(item_id,'Interrupción de prueba: no se imprimió papel.')
            app.refresh();assert app.show_interruption();root.update()
            assert app.recovery_window.preview_image is not None
            assert 'Página 1' in app.recovery_window.preview_caption.get()
            assert app.recovery_window.winfo_width()<=960 and app.recovery_window.winfo_height()<=640
            ImageGrab.grab().save(output.with_name(output.stem+'-recovery.png'))
            from core import QueueStore
            cleanup=QueueStore(base/'cleanup'/'queue.sqlite3')
            first,second=cleanup.add([pdf,pdf])
            assert cleanup.delete_items([first])==1 and cleanup.clear_items()==1
            assert not list(cleanup.documents.glob('*.pdf')) and pdf.exists()
            report={'version':'1.4','frozen':bool(getattr(__import__('sys'),'frozen',False)),
                    'windows_dependencies_loaded':True,'win32timezone_loaded':True,'pywin32_timezone_conversion':True,
                    'rendered_real_pdf':True,'visible_queue_rows':250,'recovery_preview_rendered':True,
                    'batch_sizes':[20,50,100,'Todos'],'custom_batch_tested':35,'custom_boundary_and_no_repeat':True,
                    'physical_column_removed':True,'routine_confirmation_removed':True,'legacy_history_preserved':True,
                    'compact_window_tested':[960,640],'icon_and_signature':True,'cleanup_originals_preserved':True,
                    'printer_transport':'injected simulation; no spooler submission','physical_printing_tested':False}
            output.write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
            app.close()
        except BaseException:
            if app:app.set_busy(False)
            try:root.destroy()
            except Exception:pass
            raise
