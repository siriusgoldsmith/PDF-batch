"""Compact Windows desktop layout; queue, print setup and secondary tools."""
import sys
from pathlib import Path
import tkinter as tk
from tkinter import ttk


def build_ui(app):
    root=app.root
    root.title('PDF Lote v1.4 · siriusgoldsmith')
    root.geometry('1100x720')
    root.minsize(960,640)
    assets=Path(getattr(sys,'_MEIPASS',str(Path(__file__).resolve().parent)))/'assets'
    app.app_icon=tk.PhotoImage(file=str(assets/'pdflote.png'))
    root.iconphoto(True,app.app_icon)
    if sys.platform=='win32':root.iconbitmap(str(assets/'pdflote.ico'))
    root.configure(bg='#f3f5f8')
    style=ttk.Style(root)
    if 'clam' in style.theme_names():style.theme_use('clam')
    style.configure('.',font=('Segoe UI',10),background='#f3f5f8',foreground='#203349')
    style.configure('TButton',padding=(10,6),background='#ffffff',bordercolor='#cbd5e1')
    style.map('TButton',background=[('active','#e8eef5')],foreground=[('disabled','#8995a5')])
    style.configure('Primary.TButton',background='#173d60',foreground='white',font=('Segoe UI',10,'bold'))
    style.map('Primary.TButton',background=[('disabled','#d9e1e9'),('active','#24547f')],foreground=[('disabled','#75869a'),('!disabled','white')])
    style.configure('Danger.TButton',foreground='#a43232')
    style.configure('Treeview',rowheight=28,font=('Segoe UI',10),fieldbackground='#ffffff',background='#ffffff',bordercolor='#d5dde6')
    style.configure('Treeview.Heading',font=('Segoe UI',10,'bold'),padding=(6,8),background='#e8eef5',foreground='#203349')
    style.map('Treeview',background=[('selected','#cfe2f4')],foreground=[('selected','#102b46')])
    style.configure('TLabelframe',bordercolor='#d5dde6')
    style.configure('TLabelframe.Label',font=('Segoe UI',10,'bold'))
    style.configure('TCombobox',padding=(5,4))
    style.configure('TProgressbar',background='#316a9c',troughcolor='#e3eaf2',bordercolor='#e3eaf2')
    base=ttk.Frame(root,padding=16)
    base.pack(fill='both',expand=True)
    base.columnconfigure(0,weight=1)
    base.rowconfigure(3,weight=1)

    header=ttk.Frame(base);header.grid(row=0,column=0,sticky='ew',pady=(0,12))
    ttk.Label(header,text='PDF Lote',font=('Segoe UI',22,'bold')).pack(side='left')
    ttk.Label(header,text='v1.4  /  Impresión ordenada',foreground='#63748a').pack(side='left',padx=14,pady=(9,0))
    app.signature=ttk.Label(header,text='Creado por siriusgoldsmith · GitHub',foreground='#63748a',cursor='hand2')
    app.signature.pack(side='right',pady=(9,0))
    app.signature.bind('<Button-1>',lambda event:__import__('webbrowser').open('https://github.com/siriusgoldsmith'))

    toolbar=ttk.Frame(base);toolbar.grid(row=1,column=0,sticky='ew')
    app.edit_buttons=[]
    for text,action in [('Añadir PDFs',app.add_files),('Añadir carpeta',app.add_folder),('Subir ↑',lambda:app.move(-1)),('Bajar ↓',lambda:app.move(1))]:
        button=ttk.Button(toolbar,text=text,command=action)
        button.pack(side='left',padx=(0,6));app.edit_buttons.append(button)
    app.clear_button=ttk.Button(toolbar,text='Vaciar carpeta del lote',command=app.clear_folder,style='Danger.TButton')
    app.clear_button.pack(side='right');app.edit_buttons.append(app.clear_button)
    app.delete_button=ttk.Button(toolbar,text='Eliminar PDF',command=app.remove)
    app.delete_button.pack(side='right',padx=(0,6));app.edit_buttons.append(app.delete_button)

    app.summary=tk.StringVar()
    summary_bar=ttk.Frame(base);summary_bar.grid(row=2,column=0,sticky='ew',pady=(12,6))
    ttk.Label(summary_bar,textvariable=app.summary,font=('Segoe UI',10,'bold')).pack(side='left')
    ttk.Label(summary_bar,text='Orden de impresión: de arriba hacia abajo',foreground='#63748a').pack(side='right')

    table=ttk.Frame(base);table.grid(row=3,column=0,sticky='nsew')
    table.rowconfigure(0,weight=1);table.columnconfigure(0,weight=1)
    app.tree=ttk.Treeview(table,columns=('order','name','state','automatic','progress'),show='headings',selectmode='extended',height=6)
    for column,title,width in [('order','Orden',55),('name','Documento',360),('state','Estado',155),('automatic','Resultado Windows',200),('progress','Pág. enviadas',110)]:
        app.tree.heading(column,text=title)
        app.tree.column(column,width=width,minwidth=170 if column=='name' else width,stretch=(column=='name'))
    for tag,background,foreground in [('uncertain','#fff2d7','#87520d'),('error','#fff2d7','#87520d'),('in_flight','#e4f0fc','#174c7a'),('sent','#edf5f0','#2e6047'),('skipped','#f5f6f8','#738093')]:
        app.tree.tag_configure(tag,background=background,foreground=foreground)
    app.tree.grid(row=0,column=0,sticky='nsew')
    scrollbar=ttk.Scrollbar(table,orient='vertical',command=app.tree.yview);scrollbar.grid(row=0,column=1,sticky='ns')
    app.tree.configure(yscrollcommand=scrollbar.set)
    app.tree.bind('<<TreeviewSelect>>',app.selection_info)

    app.details=tk.StringVar(value='Añade PDFs y revisa el orden. Los originales se conservan.')
    app.details_label=ttk.Label(base,textvariable=app.details,foreground='#63748a',wraplength=900)
    app.details_label.grid(row=4,column=0,sticky='ew',pady=(6,8))

    setup=ttk.LabelFrame(base,text='Impresión',padding=(12,8))
    setup.grid(row=5,column=0,sticky='ew')
    setup.columnconfigure(1,weight=1)
    ttk.Label(setup,text='Impresora').grid(row=0,column=0,sticky='w',padx=(0,12))
    app.printer=tk.StringVar()
    app.printer_box=ttk.Combobox(setup,textvariable=app.printer,state='readonly',width=25)
    app.printer_box.grid(row=0,column=1,sticky='ew')
    app.refresh_button=ttk.Button(setup,text='Actualizar',command=app.refresh_printers)
    app.refresh_button.grid(row=0,column=2,padx=6)
    app.settings_button=ttk.Button(setup,text='Ajustes Windows',command=app.printer_settings)
    app.settings_button.grid(row=0,column=3)

    batch_row=ttk.Frame(setup);batch_row.grid(row=1,column=0,columnspan=4,sticky='ew',pady=(10,0))
    ttk.Label(batch_row,text='PDFs por lote').pack(side='left',padx=(0,12))
    app.batch_size=tk.StringVar(value='Todos')
    app.batch_box=ttk.Combobox(batch_row,textvariable=app.batch_size,values=('20','50','100','Todos'),state='normal',width=9)
    app.batch_box.pack(side='left')
    app.batch_hint=ttk.Label(batch_row,text='Escribe una cantidad o elige Todos',foreground='#63748a')
    app.batch_hint.pack(side='left',padx=12)
    app.start_button=ttk.Button(batch_row,text='Iniciar / continuar',command=app.start,style='Primary.TButton')
    app.start_button.pack(side='right')
    app.pause_button=ttk.Button(batch_row,text='Pausar tras documento',command=app.request_pause)
    app.pause_button.pack(side='right',padx=8)

    progress_row=ttk.Frame(setup);progress_row.grid(row=2,column=0,columnspan=4,sticky='ew',pady=(10,0))
    app.batch_count=tk.StringVar(value='Lote: sin iniciar')
    ttk.Label(progress_row,textvariable=app.batch_count).pack(side='left',padx=(0,16))
    app.batch_progress=ttk.Progressbar(progress_row,mode='determinate',maximum=1)
    app.batch_progress.pack(side='left',fill='x',expand=True)

    footer=ttk.Frame(base);footer.grid(row=6,column=0,sticky='ew',pady=(10,0))
    app.resolve_button=ttk.Button(footer,text='Resolver interrupción',command=app.resolve)
    app.resolve_button.pack(side='left')
    ttk.Label(footer,text='Solo si hay un error o corte',foreground='#63748a').pack(side='left',padx=10)
    ttk.Button(footer,text='Exportar informe',command=app.export).pack(side='right')
    ttk.Button(footer,text='Abrir registro local',command=app.open_data).pack(side='right',padx=6)
    app.status=tk.StringVar(value='Listo. El avance se guarda automáticamente; no necesitas confirmar cada PDF.')
    app.status_label=ttk.Label(base,textvariable=app.status,foreground='#475569',wraplength=900)
    app.status_label.grid(row=7,column=0,sticky='ew',pady=(8,0))
    def resize(event):
        if event.widget is root:
            width=max(300,event.width-40)
            app.details_label.configure(wraplength=width)
            app.status_label.configure(wraplength=width)
    root.bind('<Configure>',resize,add='+')
