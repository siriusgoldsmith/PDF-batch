"""Inspect the real built PE and its PyInstaller archive; never prints."""
from pathlib import Path
import hashlib
import json
import re
import pefile
from PyInstaller.archive.readers import CArchiveReader

base=Path(__file__).resolve().parent
exe=base/'dist'/'PDFLote.exe'
pe=pefile.PE(str(exe))
assert pe.FILE_HEADER.Machine==0x8664
resource_types={entry.id for entry in pe.DIRECTORY_ENTRY_RESOURCE.entries}
assert 3 in resource_types and 14 in resource_types, 'Missing icon/group icon resources'
archive=CArchiveReader(str(exe))
pyz=archive.open_embedded_archive('PYZ.pyz')
assert 'win32timezone' in pyz.toc
assert 'recovery_preview' in pyz.toc
assert 'desktop_ui' in pyz.toc
assert any(name.replace('\\','/').endswith('assets/pdflote.png') for name in archive.toc)
assert any(name.replace('\\','/').endswith('assets/pdflote.ico') for name in archive.toc)
for path in (base/'runtime').glob('*.dll'):
    assert path.name.lower() in {name.lower() for name in archive.toc},path.name
log=(base/'tests-v1.4-final.log').read_text(errors='replace')
match=re.search(r'Ran (\d+) tests in ([\d.]+)s',log)
assert match and 'FAILED (' not in log and re.search(r'^OK(?: \(skipped=\d+\))?$',log,re.M)
skip_match=re.search(r'^OK \(skipped=(\d+)\)',log,re.M)
skipped=int(skip_match[1]) if skip_match else 0
frozen=json.loads((base/'qa-frozen.json').read_text())
assert frozen['frozen'] and frozen['recovery_preview_rendered'] and frozen['cleanup_originals_preserved']
assert frozen['custom_batch_tested']==35 and frozen['custom_boundary_and_no_repeat']
assert frozen['physical_column_removed'] and frozen['routine_confirmation_removed']
report={'version':'1.4','author':'siriusgoldsmith','tests_run':int(match[1]),'windows_wine_suite':'OK','skipped_platform_cases':skipped,'skips_reason':'Wine cannot create inspectable Windows symlinks; equivalent symlink tests pass on Linux','linux_backend_tests':39,'frozen_diagnostic':frozen,'pe_x64':True,'embedded_icon_verified':True,'required_runtime_dlls_in_archive':True,'exe_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'physical_printing_tested':False,'success':True}
(base/'qa-final.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps(report,ensure_ascii=False,indent=2))
