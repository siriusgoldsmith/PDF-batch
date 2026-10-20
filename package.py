"""Package explicit allowlist only; no PDF snapshots, databases or personal data."""
from pathlib import Path
import hashlib
import zipfile
base=Path(__file__).resolve().parent
exe=base/'dist'/'PDFLote.exe'
assert exe.read_bytes()[:2]==b'MZ'
sha=hashlib.sha256(exe.read_bytes()).hexdigest()
(base/'SHA256.txt').write_text(f'{sha}  PDFLote.exe\n',encoding='ascii')
files=['LEEME.txt','COPYING','THIRD_PARTY.txt','SHA256.txt','qa-final.json','qa-gui.json','qa-frozen.json']
sources=['desktop_ui.py','test_v14_ui.py','verify_build.py','package.py','test_cleanup_legacy.py','app.py','recovery_preview.py','test_recovery_preview.py','test_queue_cleanup.py','test_v13_ui.py','make_icon.py','instance_lock.py','test_instance_lock.py','test_packaging.py','test_physical.py','test_results_ui.py','PDFLote.spec','core.py','printer_backend.py','native_dc.py','diagnostic.py','test_core.py','test_printer_backend.py','test_native_dc.py','test_ui.py','qa_gui.py','requirements.txt','build-windows.bat']
output=base/'dist'/'PDFLote-Windows-x64-v1.4.zip'
with zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED,compresslevel=9) as z:
    z.write(exe,'PDFLote/PDFLote.exe')
    for name in files:z.write(base/name,'PDFLote/'+name)
    for name in sources:z.write(base/name,'PDFLote/fuentes/'+name)
    for path in (base/'assets').iterdir():
        if path.is_file():z.write(path,'PDFLote/fuentes/assets/'+path.name)
    for path in (base/'runtime').glob('*.dll'):z.write(path,'PDFLote/fuentes/runtime/'+path.name)
    for path in (base/'licenses').iterdir():
        if path.is_file():z.write(path,'PDFLote/licenses/'+path.name)
with zipfile.ZipFile(output) as z:
    assert z.testzip() is None
    assert not any(Path(n).suffix.lower() in ['.pdf','.sqlite3','.db'] for n in z.namelist())
print(output)
print('bytes',output.stat().st_size)
print('exe_sha256',sha)
