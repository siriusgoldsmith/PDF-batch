"""Real GUI + real PDFs smoke; NEVER sends anything to a printer."""
import json
from pathlib import Path
import tempfile
import tkinter as tk
import fitz
from PIL import ImageGrab
from app import App

with tempfile.TemporaryDirectory() as tmp:
    folder=Path(tmp)
    paths=[]
    for n in range(250):
        path=folder/f'{n+1:03d} — Documento de prueba.pdf'
        with fitz.open() as doc:
            page=doc.new_page();page.insert_text((60,70),f'PDF Lote - documento {n+1}')
            doc.save(path)
        paths.append(path)
    root=tk.Tk()
    app=App(root,folder/'data')
    ids=app.store.add(paths)
    app.refresh();root.update()
    assert len(app.tree.get_children())==250
    assert [r['name'] for r in app.store.list_items()]==[p.name for p in paths]
    app.store.claim(ids[0],'Prueba sin hardware')
    app.store.progress(ids[0],1,total_pages=1,job_id=777)
    assert app.store.recover()==1
    assert app.store.pending() is None
    app.refresh();root.update()
    ImageGrab.grab().save('qa-interface.png')
    app.store.resolve(ids[0],'sent')
    assert app.store.pending()['id']==ids[1]
    app.close()
    report={'real_pdfs':250,'visible_rows':250,'ordered':True,'restart_interruption_blocks_next':True,'manual_resolution_continues_at_second':True,'hardware_printing_tested':False}
    Path('qa-gui.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps(report))
