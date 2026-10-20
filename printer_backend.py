"""Windows GDI PDF printing. Imports stay lazy so the GUI can load elsewhere."""


import time
from pathlib import Path
from types import SimpleNamespace


def _load_dependencies():
    import fitz
    import win32print
    import native_dc as win32ui  # GDI via ctypes; avoids a non-portable MFC DLL dependency.
    import win32gui
    from PIL import Image, ImageWin
    return SimpleNamespace(fitz=fitz, spooler=win32print, ui=win32ui,
                           gui=win32gui, Image=Image, ImageWin=ImageWin)


def print_document(path, printer, start_page, on_progress, on_job, stop_event,
                   timeout=600) -> None:
    """Submit pages (1-based resume) and wait for queue completion/removal.

    Return does NOT prove physical printing. Callbacks run synchronously:
    on_job(job_id, total_pages) immediately after StartDoc and
    on_progress(page_number, total_pages) after each EndPage.
    """
    _check_stop(stop_event)
    deps = _load_dependencies()
    doc = deps.fitz.open(str(path))
    handle = None
    dc = None
    started = False
    ended = False
    try:
        handle = deps.spooler.OpenPrinter(printer)
        info = deps.spooler.GetPrinter(handle, 2)
        unhealthy = 1 | 2 | 4 | 8 | 16 | 64 | 128 | 2048 | 4096 | 262144 | 1048576 | 2097152 | 4194304
        if info.get('Status',0) & unhealthy:
            raise PrintUncertain('La impresora informa un bloqueo o error; revise la impresora y la cola antes de continuar.')
        dc = deps.ui.CreateDC()
        dc.CreatePrinterDC(printer)
        mode = info['pDevMode']
        mode.Copies = 1
        mode.Fields |= 0x100  # DM_COPIES; preserve other default device settings.
        deps.gui.ResetDC(dc.GetSafeHdc(), mode)
        _check_stop(stop_event)
        job_id = dc.StartDoc('PDFLote - ' + Path(path).name)
        started = True
        on_job(job_id, doc.page_count)
        for page in range(start_page - 1, doc.page_count):
            _check_stop(stop_event)
            image = _render_page(doc, page, deps)
            try:
                _check_stop(stop_event)
                rotate, box = _layout(image.size, dc)
                if rotate:
                    rotated = image.transpose(deps.Image.Transpose.ROTATE_90)
                    image.close()
                    image = rotated
                dc.StartPage()
                deps.ImageWin.Dib(image).draw(dc.GetSafeHdc(), box)
                dc.EndPage()
            finally:
                image.close()
            on_progress(page + 1, doc.page_count)
        _check_stop(stop_event)
        dc.EndDoc()
        ended = True
        _wait_for_job(deps.spooler, handle, job_id, stop_event, timeout)
    except PrintUncertain:
        # A stop request is not authorization to cancel a submitted job.
        raise
    except Exception as exc:
        if started and not ended:
            _best_effort(dc.AbortDoc)
            raise PrintUncertain(
                f'Trabajo {job_id}: fallo al enviar el PDF: {exc}. '
                'Revise la cola y las páginas físicas antes de reintentar; '
                'puede haber páginas impresas.') from exc
        raise
    finally:
        if dc is not None:
            _best_effort(dc.DeleteDC)
        if handle is not None:
            _best_effort(lambda: deps.spooler.ClosePrinter(handle))
        _best_effort(doc.close)


def _best_effort(action):
    try:
        action()
    except Exception:
        pass



class PrintUncertain(RuntimeError):
    """The job must be reconciled manually before any retry."""


_JOB_ERRORS = 2 | 4 | 32 | 64 | 512 | 1024


def _check_stop(stop_event):
    if stop_event.is_set():
        raise PrintUncertain(
            'Detenido por el usuario. Revise la cola antes de reintentar; '
            'el trabajo puede seguir imprimiéndose. No se canceló ningún trabajo de la cola.')


def _wait_for_job(spooler, handle, job_id, stop_event, timeout,
                  *, sleep=time.sleep, clock=time.monotonic):
    """Only call AFTER EndDoc. A missing job is queue completion, not paper proof."""
    deadline = clock() + timeout
    while True:
        _check_stop(stop_event)
        if clock() >= deadline:
            raise PrintUncertain(
                f'Trabajo {job_id}: tiempo de espera agotado. Revise la cola '
                'y la impresora antes de reintentar; no se canceló el trabajo.')
        try:
            job = spooler.GetJob(handle, job_id, 1)
        except Exception as exc:
            code = getattr(exc, 'winerror', None)
            if code is None and exc.args:
                code = exc.args[0]
            if code in (87, 1168):
                return  # EndDoc has succeeded; disappearance is not physical proof.
            raise
        if job['Status'] & _JOB_ERRORS:
            raise PrintUncertain(
                f'Trabajo {job_id}: error de impresión (estado {job["Status"]}). '
                'Revise la cola y la impresora antes de reintentar; puede haber páginas impresas.')
        if job['Status'] & (128 | 4096):  # PRINTED | COMPLETE
            return
        sleep(0.25)


def _render_page(doc, page_number, deps):
    # alpha=False composites transparent PDF content onto white in MuPDF.
    pix = doc[page_number].get_pixmap(dpi=150, colorspace=deps.fitz.csRGB, alpha=False)
    return deps.Image.frombytes('RGB', (pix.width, pix.height), pix.samples)


def _layout(size, dc):
    # MM_TEXT coordinates start at the printable origin, NOT the paper origin.
    width = min(dc.GetDeviceCaps(8), dc.GetDeviceCaps(110) - dc.GetDeviceCaps(112))
    height = min(dc.GetDeviceCaps(10), dc.GetDeviceCaps(111) - dc.GetDeviceCaps(113))
    iw, ih = size
    normal = min(width / iw, height / ih)
    turned = min(width / ih, height / iw)
    rotate = turned > normal
    if rotate:
        iw, ih = ih, iw
    scale = max(normal, turned)
    dw, dh = min(width, round(iw * scale)), min(height, round(ih * scale))
    left, top = (width - dw) // 2, (height - dh) // 2
    return rotate, (left, top, left + dw, top + dh)


def list_printers() -> list[str]:
    import win32print
    return [row[2] for row in win32print.EnumPrinters(
        win32print.PRINTER_ENUM_LOCAL | win32print.PRINTER_ENUM_CONNECTIONS)]


def default_printer() -> str:
    import win32print
    return win32print.GetDefaultPrinter()
