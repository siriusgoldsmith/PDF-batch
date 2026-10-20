"""PDF Lote: ordered, durable, conservative Windows printing."""
import ctypes
import json
import os
from pathlib import Path
import queue
import sys
import tempfile
import threading
import tkinter as tk
from tkinter import ttk, filedialog, messagebox, simpledialog
from core import QueueStore
import printer_backend as printing

STATES = {'pending':'Pendiente', 'in_flight':'En proceso', 'uncertain':'Revisar interrupción', 'error':'Error', 'sent':'Finalizado', 'skipped':'Omitido'}
AUTOMATIC = {'unrecorded':'Sin resultado registrado', 'completed':'Finalizado según Windows', 'review':'Error / por revisar'}


class App:
    def __init__(self, root, data_dir=None):
        self.root = root
        self.data_dir = Path(data_dir or Path(os.environ.get('LOCALAPPDATA', str(Path.home()))) / 'PDFLote')
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.store = QueueStore(self.data_dir / 'queue.sqlite3')
        self.store.recover()
        self.busy = False
        self.stop = threading.Event()
        self.messages = queue.Queue()
        from desktop_ui import build_ui
        build_ui(self)
        self.refresh_printers(); self.refresh(); self.set_busy(False)
        root.protocol('WM_DELETE_WINDOW',self.close)
        self.timer=root.after(150,self.poll)
        self.recovery_timer=root.after(400,self.show_interruption)

    def refresh_printers(self):
        try:
            printers=printing.list_printers(); self.printer_box['values']=printers
            if self.printer.get() not in printers:
                default=printing.default_printer()
                self.printer.set(default if default in printers else (printers[0] if printers else ''))
            if not printers: self.status.set('No hay impresoras disponibles. Instala una en Windows y pulsa Actualizar.')
        except Exception as exc: self.status.set('No se pudo consultar impresoras: '+str(exc))

    def refresh(self):
        selected=set(self.tree.selection()); self.tree.delete(*self.tree.get_children())
        items=self.store.list_items()
        for r in items:
            total=r.get('total_pages') or '?'; page=r.get('last_page') or 0
            self.tree.insert('', 'end', iid=str(r['id']),values=(r['position'],r['name'],STATES.get(r['state'],r['state']),AUTOMATIC.get(r.get('queue_outcome','unrecorded'),'Sin resultado registrado'),f'{page} / {total}'),tags=(r['state'],))
        for item in selected:
            if self.tree.exists(item):self.tree.selection_add(item)
        sent=sum(r['state']=='sent' for r in items); uncertain=sum(r['state'] in ('uncertain','error') for r in items)
        active=sum(r['state']=='in_flight' for r in items)
        pending=sum(r['state']=='pending' for r in items)
        self.summary.set(f'{len(items)} PDFs · {pending} pendientes · {sent} finalizados · {active} en proceso · {uncertain} por revisar')

    def selection_info(self,event=None):
        ids=self.tree.selection()
        if not ids:return
        r=next((r for r in self.store.list_items() if str(r['id'])==ids[0]),None)
        if r:self.details.set(f"{r['source']}" + (f" | {r['error']}" if r.get('error') else ''))

    def set_busy(self,value):
        self.busy=value
        for b in self.edit_buttons+[self.start_button,self.resolve_button,self.refresh_button,self.settings_button]: b.configure(state='disabled' if value else 'normal')
        self.printer_box.configure(state='disabled' if value else 'readonly')
        self.batch_box.configure(state='disabled' if value else 'normal')
        self.pause_button.configure(state='normal' if value else 'disabled')

    def request_pause(self):
        if not self.busy:return
        self.stop.set()
        self.pause_button.configure(state='disabled')
        self.status.set('Pausa solicitada. Se terminará el PDF actual antes de detener el lote.')

    def add_files(self):
        if self.busy:return
        paths=filedialog.askopenfilenames(title='Añadir PDFs: revisa luego el orden en la lista',filetypes=[('PDF','*.pdf')])
        if paths:self.add_paths(paths)

    def add_folder(self):
        if self.busy:return
        folder=filedialog.askdirectory(title='Añadir PDFs de una carpeta (orden por nombre, sin subcarpetas)')
        if folder:
            paths=sorted((str(p) for p in Path(folder).iterdir() if p.is_file() and p.suffix.lower()=='.pdf'),key=str.casefold)
            if not paths:messagebox.showinfo('Carpeta vacía','No hay archivos PDF en esta carpeta.')
            else:self.add_paths(paths)

    def add_paths(self,paths):
        self.set_busy(True); self.status.set('Validando PDFs y guardando copias del lote…')
        def work():
            try:
                import fitz
                for path in paths:
                    with fitz.open(path) as doc:
                        if doc.needs_pass:raise ValueError(f'PDF protegido con contraseña: {Path(path).name}')
                        if not doc.is_pdf or doc.page_count<1:raise ValueError(f'PDF inválido o vacío: {Path(path).name}')
                        for n in range(doc.page_count):doc.load_page(n)
                self.store.add(paths)
                self.messages.put(('done',f'Se añadieron {len(paths)} PDFs al final de la lista.'))
            except Exception as exc:self.messages.put(('error',str(exc)))
        threading.Thread(target=work,daemon=True).start()

    def selected_id(self):
        ids=self.tree.selection()
        if len(ids)!=1:raise ValueError('Selecciona un solo documento.')
        return int(ids[0])

    def move(self,delta):
        if self.busy:return
        try:self.store.move(self.selected_id(),delta); self.refresh()
        except Exception as exc:messagebox.showerror('No se pudo mover',str(exc))

    def remove(self):
        if self.busy:return
        ids=self.tree.selection()
        if not ids:return
        if not messagebox.askyesno('Eliminar PDF del lote',f'¿Eliminar {len(ids)} PDF(s) seleccionados de la lista y sus copias locales?\n\nLos archivos originales y el historial se conservan. No se pueden eliminar interrupciones sin resolver.'):return
        try:
            count=self.store.delete_items([int(i) for i in ids]); self.refresh()
            self.status.set(f'{count} PDF(s) eliminados del lote. Originales conservados.')
        except Exception as exc:messagebox.showerror('No se pudo quitar',str(exc))

    def clear_folder(self):
        if self.busy:return
        if not self.store.list_items():return
        if not messagebox.askyesno('Vaciar carpeta del lote','¿Vaciar la lista y eliminar sus copias locales de PDF?\n\nNO se borrará la carpeta de origen ni sus archivos. El historial se conserva. Resuelve primero cualquier interrupción.'):return
        try:
            count=self.store.clear_items();self.refresh()
            self.batch_count.set('Lote: sin iniciar');self.batch_progress.configure(value=0)
            self.status.set(f'Carpeta del lote vaciada: {count} PDF(s). Originales conservados.')
        except Exception as exc:messagebox.showerror('No se pudo vaciar',str(exc))

    def start(self):
        if self.busy:return
        if not self.printer.get():messagebox.showerror('Sin impresora','Selecciona una impresora.');return
        if not self.store.pending():
            messagebox.showinfo('Sin pendientes disponibles','El lote terminó o hay una interrupción que debes resolver antes de continuar.');return
        choice=self.batch_size.get().strip()
        try:
            limit=None if choice.casefold()=='todos' else int(choice) if choice.isascii() and choice.isdecimal() else 0
            if limit is not None and limit<1:raise ValueError('nonpositive')
        except ValueError:
            messagebox.showerror('Cantidad no válida','Escribe una cantidad entera mayor que cero (por ejemplo, 35) o selecciona Todos.')
            self.batch_box.focus_set();self.batch_box.selection_range(0,'end');return
        available=0
        for row in self.store.list_items():
            if row['state'] in ('sent','skipped'):continue
            if row['state']!='pending':break
            available+=1
        target=available if limit is None else min(limit,available)
        if not messagebox.askyesno('Confirmar impresión',f'¿Imprimir los próximos {target} PDFs en «{self.printer.get()}»?\n\nUna copia por documento, ajustada al área imprimible. Papel, bandeja, color y dúplex: valores predeterminados del controlador.\n\nRevisa los ajustes y el orden antes de aceptar. No envíes el mismo lote desde otra aplicación.'):return
        printer=self.printer.get(); self.stop.clear(); self.set_busy(True)
        self.batch_count.set(f'Lote: 0 / {target} PDFs finalizados según Windows')
        self.batch_progress.configure(maximum=max(target,1),value=0)
        def work():
            completed=0
            try:
                while not self.stop.is_set() and completed<target:
                    item=self.store.pending()
                    if not item:break
                    item_id=item['id']
                    claimed=False
                    try:
                        self.store.claim(item_id,printer)
                        claimed=True
                        self.messages.put(('status',f"Enviando #{item['position']}: {item['name']}"))
                        # Pause applies BETWEEN documents: never cut a document on user pause.
                        document_stop=threading.Event()
                        def on_progress(page,total):
                            self.store.progress(item_id,page,total_pages=total)
                            self.messages.put(('refresh',''))
                        printing.print_document(item['snapshot'],printer,item.get('start_page',1),
                            on_progress,
                            lambda j,t:self.store.progress(item_id,item.get('start_page',1)-1,total_pages=t,job_id=j),document_stop)
                        self.store.mark_sent(item_id)
                        completed+=1
                        self.messages.put(('batch',(completed,target)))
                        self.messages.put(('refresh',''))
                    except Exception as exc:
                        if claimed:
                            try:self.store.fail(item_id,str(exc))
                            except Exception as save_exc:raise RuntimeError(f'{exc}; además no se pudo guardar el estado: {save_exc}')
                        raise
                outcome='Lote pausado' if self.stop.is_set() else 'Lote terminado'
                self.messages.put(('done',f'{outcome}: {completed} / {target} PDFs finalizados según Windows. Pulsa Iniciar / continuar para el siguiente lote.'))
            except Exception as exc:self.messages.put(('error',str(exc)))
        threading.Thread(target=work,daemon=True).start()


    def resolve(self):
        if self.busy:return
        previous=getattr(self,'recovery_window',None)
        if previous is not None and previous.winfo_exists():
            previous.lift();previous.focus_force();return
        try:
            item_id=self.selected_id(); item=next(r for r in self.store.list_items() if r['id']==item_id)
            if item['state'] not in ('uncertain','error'):raise ValueError('Este documento no tiene una interrupción pendiente de resolver.')
        except Exception as exc:messagebox.showerror('Seleccionar interrupción',str(exc));return
        width=min(1060,max(900,self.root.winfo_width()-40))
        height=min(700,max(600,self.root.winfo_height()-40))
        panel_width=width//2-32
        win=tk.Toplevel(self.root); win.title('Revisar interrupción · elegir dónde continuar');win.geometry(f'{width}x{height}');win.minsize(width,height);win.transient(self.root);win.grab_set()
        self.recovery_window=win
        win.recovery_item_id=item_id
        preview=ttk.Frame(win,padding=16);preview.pack(side='left',fill='both',expand=True)
        ttk.Label(preview,text='Última página esperada',font=('Segoe UI',14,'bold')).pack(anchor='w')
        win.preview_caption=tk.StringVar(value='Sin páginas enviadas registradas.')
        ttk.Label(preview,textvariable=win.preview_caption,wraplength=panel_width).pack(anchor='w',pady=8)
        win.preview_image=None
        try:
            from recovery_preview import recovery_reference,render_reference
            from PIL import ImageTk
            ref=recovery_reference(self.store.list_items(),item_id)
            if ref:
                with render_reference(ref,max_size=(panel_width,height-210)) as image:
                    win.preview_image=ImageTk.PhotoImage(image,master=win)
                ttk.Label(preview,image=win.preview_image).pack(expand=True)
                win.preview_caption.set(f"{ref['item']['name']} · Página {ref['page']}\nReferencia enviada al controlador; NO confirma salida física.")
            else:
                ttk.Label(preview,text='No hay una última página registrada para mostrar.\nRevisa físicamente el papel y la cola de Windows.',wraplength=430).pack(expand=True)
        except Exception as exc:
            win.preview_caption.set('No se pudo abrir la vista previa: '+str(exc))
        ttk.Label(preview,text='Compara esta página con la última hoja de papel que salió.\nSi imprimes a doble cara, comprueba ambos lados.',foreground='#92400e',wraplength=440).pack(anchor='w',pady=10)
        body=ttk.Frame(win,padding=16,width=width//2);body.pack(side='right',fill='both',expand=True)
        ttk.Label(body,text=item['name'],font=('Segoe UI',13,'bold'),wraplength=panel_width).pack(anchor='w')
        text=(f"Impresora: {item.get('printer') or 'sin registrar'}\nTrabajo Windows: {item.get('job_id') or 'no registrado'}\nPágina registrada: {item.get('last_page') or 0} / {item.get('total_pages') or '?'}\n\nCompara el preview con el papel. Antes de reintentar, espera o cancela el trabajo anterior en Windows: podría seguir imprimiéndose.")
        ttk.Label(body,text=text,wraplength=panel_width).pack(anchor='w',pady=12)
        page=tk.StringVar(value='1')
        line=ttk.Frame(body);line.pack(fill='x',pady=6)
        ttk.Label(line,text='Primera página que falta:').pack(side='left');ttk.Entry(line,textvariable=page,width=8).pack(side='left',padx=8)
        def apply(action,from_first=False):
            try:
                first=int(page.get()) if action=='retry' and not from_first else 1
                if first<1 or (item.get('total_pages') and first>item['total_pages']):raise ValueError('Página fuera del documento.')
                if not messagebox.askyesno('Confirmar revisión manual','¿Confirmas que revisaste las hojas y la cola de Windows?\nEl programa no cancela trabajos anteriores automáticamente.',parent=win):return
                self.store.resolve(item_id,action,start_page=first);win.destroy();self.refresh()
            except Exception as exc:messagebox.showerror('No se pudo resolver',str(exc),parent=win)
        ttk.Button(body,text='Reanudar desde este PDF (página 1)',command=lambda:apply('retry',True)).pack(fill='x',pady=3)
        for label,action in [('Reanudar desde la página indicada','retry'),('Continuar con el siguiente PDF (este salió completo)','sent'),('Omitir este PDF sin confirmar impresión','skip')]:
            ttk.Button(body,text=label,command=lambda a=action:apply(a)).pack(fill='x',pady=3)
        ttk.Label(body,text='Después pulsa Iniciar / continuar. Cerrar esta ventana conserva el bloqueo.',foreground='#475569',wraplength=panel_width).pack(anchor='w',pady=8)
        win.lift();win.focus_force()

    def show_interruption(self):
        if self.busy:return False
        item=next((r for r in self.store.list_items() if r['state'] in ('uncertain','error')),None)
        if item:
            self.tree.selection_set(str(item['id']));self.tree.see(str(item['id']))
            self.resolve()
            return True
        return False

    def poll(self):
        try:
            while True:
                kind,text=self.messages.get_nowait()
                if kind=='batch':
                    completed,target=text
                    self.batch_count.set(f'Lote: {completed} / {target} PDFs finalizados según Windows')
                    self.batch_progress.configure(value=completed)
                if kind in ('status','done','error'):self.status.set(text)
                if kind in ('refresh','done','error'):self.refresh()
                if kind in ('done','error'):self.set_busy(False)
                if kind=='error' and not self.show_interruption():
                    messagebox.showerror('Proceso detenido de forma segura',text+'\n\nNo se reintentará automáticamente.')
        except queue.Empty:pass
        self.timer=self.root.after(150,self.poll)

    def export(self):
        path=filedialog.asksaveasfilename(title='Exportar registro',defaultextension='.json',initialfile='informe-pdf-lote.json',filetypes=[('JSON','*.json')])
        if path:
            try:Path(path).write_text(json.dumps(self.store.list_items(),ensure_ascii=False,indent=2),encoding='utf-8');self.status.set('Informe exportado: '+path)
            except Exception as exc:messagebox.showerror('Exportar',str(exc))

    def printer_settings(self):
        try:os.startfile('ms-settings:printers')
        except Exception as exc:messagebox.showerror('Ajustes Windows',str(exc))

    def open_data(self):
        try:os.startfile(str(self.data_dir))
        except Exception as exc:messagebox.showerror('Carpeta del lote',str(exc))

    def close(self):
        if self.busy:
            self.stop.set()
            messagebox.showinfo('Pausa solicitada','Espera a que termine el documento o la operación actual. Luego podrás cerrar. Si Windows fuerza un cierre, el documento activo quedará por revisar al volver a abrir.')
            return
        self.root.after_cancel(self.timer)
        self.root.after_cancel(self.recovery_timer)
        self.root.destroy()


def main():
    if len(sys.argv)==3 and sys.argv[1]=='--diagnostico':
        import diagnostic
        diagnostic.run(sys.argv[2])
        return
    mutex=None
    if sys.platform=='win32':
        kernel=ctypes.WinDLL('kernel32',use_last_error=True)
        kernel.CreateMutexW.argtypes=[ctypes.c_void_p,ctypes.c_bool,ctypes.c_wchar_p]
        kernel.CreateMutexW.restype=ctypes.c_void_p
        mutex=kernel.CreateMutexW(None,False,'Local\\PDFLote-Ordered-v1')
        if not mutex:raise ctypes.WinError(ctypes.get_last_error())
        if ctypes.get_last_error()==183:
            messagebox.showwarning('PDF Lote ya está abierto','Usa la ventana existente para evitar dos procesos enviando el mismo lote.');return
    from instance_lock import InstanceLock, AlreadyRunning
    data_dir=Path(os.environ.get('LOCALAPPDATA',str(Path.home()))) / 'PDFLote'
    try:
        lifetime_lock=InstanceLock(data_dir/'queue.lock')
    except AlreadyRunning as exc:
        messagebox.showwarning('Lote ya abierto',str(exc))
        return
    root=tk.Tk()
    try:
        app=App(root,data_dir)
        root.mainloop()
    finally:
        lifetime_lock.close()
        if mutex:
            kernel.CloseHandle.argtypes=[ctypes.c_void_p];kernel.CloseHandle(mutex)

if __name__=='__main__':
    try:main()
    except Exception as exc:
        messagebox.showerror('PDF Lote no pudo iniciar',str(exc))
        raise
