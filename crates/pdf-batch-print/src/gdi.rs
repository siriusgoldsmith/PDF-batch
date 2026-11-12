//! Win32 spooler + GDI. Mirrors the v1.x flow in `printer_backend.py`/`native_dc.py`.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, ERROR_NOT_FOUND, HANDLE};
use windows::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, GetDeviceCaps, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DEVMODEW,
    DIB_RGB_COLORS, DM_COPIES, GET_DEVICE_CAPS_INDEX, HDC, SRCCOPY,
};
use windows::Win32::Graphics::Printing::{
    ClosePrinter, DocumentPropertiesW, EnumPrintersW, GetDefaultPrinterW, GetJobW, GetPrinterW, OpenPrinterW,
    JOB_INFO_1W, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_2W, PRINTER_INFO_4W,
};
use windows::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};

use crate::{check_stop, layout, pdfium, wait_for_job, Callbacks, Document, JobState, PrintError, Result};

const HORZRES: i32 = 8;
const VERTRES: i32 = 10;
const PHYSICALWIDTH: i32 = 110;
const PHYSICALHEIGHT: i32 = 111;
const PHYSICALOFFSETX: i32 = 112;
const PHYSICALOFFSETY: i32 = 113;
const DM_IN_BUFFER: u32 = 8;
const DM_OUT_BUFFER: u32 = 2;
/// Printer status bits treated as "do not start": PAUSED, ERROR, PENDING_DELETION,
/// PAPER_JAM, PAPER_OUT, OFFLINE, IO_ACTIVE-less error group, NOT_AVAILABLE, etc.
const UNHEALTHY: u32 = 1 | 2 | 4 | 8 | 16 | 64 | 128 | 2048 | 4096 | 262144 | 1048576 | 2097152 | 4194304;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn win_err(op: &str) -> String {
    let code = unsafe { GetLastError() }.0;
    format!("{op}: fallo de Windows (código {code}).")
}

struct Printer(HANDLE);
impl Drop for Printer {
    fn drop(&mut self) {
        unsafe {
            let _ = ClosePrinter(self.0);
        }
    }
}

struct Dc(HDC);
impl Drop for Dc {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

fn open_printer(name: &[u16]) -> std::result::Result<Printer, String> {
    let mut h = HANDLE::default();
    unsafe { OpenPrinterW(PCWSTR(name.as_ptr()), &mut h, None) }.map_err(|e| format!("OpenPrinter: {e}"))?;
    Ok(Printer(h))
}

/// Call a Win32 "size query then fill" API into an 8-byte-aligned buffer.
fn query_buf(mut f: impl FnMut(Option<&mut [u8]>, &mut u32) -> windows::core::Result<()>) -> std::result::Result<Vec<u64>, String> {
    let mut needed = 0u32;
    if let Err(e) = f(None, &mut needed) {
        if e.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult() && needed == 0 {
            return Err(e.to_string());
        }
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, needed as usize) };
    f(Some(bytes), &mut needed).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn printer_status(h: &Printer) -> std::result::Result<u32, String> {
    let buf = query_buf(|b, n| unsafe { GetPrinterW(h.0, 2, b, n) })?;
    Ok(unsafe { (*(buf.as_ptr() as *const PRINTER_INFO_2W)).Status })
}

/// Driver default DEVMODE with Copies forced to 1 (other settings preserved).
fn devmode(h: &Printer, name: &[u16]) -> std::result::Result<Vec<u64>, String> {
    let size = unsafe { DocumentPropertiesW(None, h.0, PCWSTR(name.as_ptr()), None, None, 0) };
    if size <= 0 {
        return Err(win_err("DocumentProperties"));
    }
    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
    let dm = buf.as_mut_ptr() as *mut DEVMODEW;
    let r = unsafe { DocumentPropertiesW(None, h.0, PCWSTR(name.as_ptr()), Some(dm), None, DM_OUT_BUFFER) };
    if r < 0 {
        return Err(win_err("DocumentProperties"));
    }
    unsafe {
        (*dm).Anonymous1.Anonymous1.dmCopies = 1;
        (*dm).dmFields |= DM_COPIES;
        let _ = DocumentPropertiesW(None, h.0, PCWSTR(name.as_ptr()), Some(dm), Some(dm), DM_IN_BUFFER | DM_OUT_BUFFER);
    }
    Ok(buf)
}

fn job_state(h: &Printer, job_id: u32) -> std::result::Result<JobState, String> {
    let mut needed = 0u32;
    let first = unsafe { GetJobW(h.0, job_id, 1, None, &mut needed) };
    if !first.as_bool() {
        let code = unsafe { GetLastError() };
        if code == ERROR_INVALID_PARAMETER || code == ERROR_NOT_FOUND {
            return Ok(JobState::Gone);
        }
        if code != ERROR_INSUFFICIENT_BUFFER {
            return Err(format!("GetJob: código {}", code.0));
        }
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, needed as usize) };
    if !unsafe { GetJobW(h.0, job_id, 1, Some(bytes), &mut needed) }.as_bool() {
        let code = unsafe { GetLastError() };
        if code == ERROR_INVALID_PARAMETER || code == ERROR_NOT_FOUND {
            return Ok(JobState::Gone);
        }
        return Err(format!("GetJob: código {}", code.0));
    }
    Ok(JobState::Status(unsafe { (*(buf.as_ptr() as *const JOB_INFO_1W)).Status }))
}

fn caps(dc: HDC, i: i32) -> i32 {
    unsafe { GetDeviceCaps(dc, GET_DEVICE_CAPS_INDEX(i as u32)) }
}

fn gdi_ok(r: i32, op: &str) -> std::result::Result<i32, String> {
    if r <= 0 {
        Err(win_err(op))
    } else {
        Ok(r)
    }
}

pub fn print_document(
    path: &Path,
    printer: &str,
    start_page: u32,
    cb: Callbacks,
    stop: &AtomicBool,
    timeout: Duration,
) -> Result<()> {
    check_stop(stop)?;
    let pdfium = pdfium()?;
    let doc = Document::open(pdfium, path)?;
    let total = doc.page_count();
    let name = wide(printer);
    let fail = |e: String| PrintError::Failed(e);
    let handle = open_printer(&name).map_err(fail)?;
    if printer_status(&handle).map_err(fail)? & UNHEALTHY != 0 {
        return Err(PrintError::Uncertain(
            "La impresora informa un bloqueo o error; revise la impresora y la cola antes de continuar.".into(),
        ));
    }
    let mode = devmode(&handle, &name).map_err(fail)?;
    let driver = wide("WINSPOOL");
    let hdc = unsafe {
        CreateDCW(PCWSTR(driver.as_ptr()), PCWSTR(name.as_ptr()), PCWSTR::null(), Some(mode.as_ptr() as *const DEVMODEW))
    };
    if hdc.is_invalid() {
        return Err(fail(win_err("CreateDC")));
    }
    let dc = Dc(hdc);
    check_stop(stop)?;

    let title = wide(&format!(
        "PDF-batch - {}",
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    let info = DOCINFOW { cbSize: std::mem::size_of::<DOCINFOW>() as i32, lpszDocName: PCWSTR(title.as_ptr()), ..Default::default() };
    let job_id = gdi_ok(unsafe { StartDocW(dc.0, &info) }, "StartDoc").map_err(fail)? as u32;

    // Everything between StartDoc and EndDoc: on error abort and report uncertain.
    let mut submit = || -> std::result::Result<(), SubmitError> {
        (cb.on_job)(job_id, total).map_err(SubmitError::Other)?;
        let printable = (
            caps(dc.0, HORZRES).min(caps(dc.0, PHYSICALWIDTH) - caps(dc.0, PHYSICALOFFSETX)),
            caps(dc.0, VERTRES).min(caps(dc.0, PHYSICALHEIGHT) - caps(dc.0, PHYSICALOFFSETY)),
        );
        for page in start_page.saturating_sub(1)..total {
            check_stop(stop).map_err(SubmitError::Stop)?;
            let mut raster = doc.render(page).map_err(SubmitError::Other)?;
            check_stop(stop).map_err(SubmitError::Stop)?;
            let (rotate, (l, t, r, b)) = layout((raster.width, raster.height), printable);
            if rotate {
                raster = raster.rotate_90();
            }
            gdi_ok(unsafe { StartPage(dc.0) }, "StartPage").map_err(SubmitError::Other)?;
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: raster.width as i32,
                    biHeight: -(raster.height as i32), // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let lines = unsafe {
                StretchDIBits(
                    dc.0, l, t, r - l, b - t, 0, 0, raster.width as i32, raster.height as i32,
                    Some(raster.bgra.as_ptr() as *const _), &bmi, DIB_RGB_COLORS, SRCCOPY,
                )
            };
            if lines == 0 {
                return Err(SubmitError::Other(win_err("StretchDIBits")));
            }
            gdi_ok(unsafe { EndPage(dc.0) }, "EndPage").map_err(SubmitError::Other)?;
            (cb.on_progress)(page + 1, total).map_err(SubmitError::Other)?;
        }
        check_stop(stop).map_err(SubmitError::Stop)?;
        gdi_ok(unsafe { EndDoc(dc.0) }, "EndDoc").map_err(SubmitError::Other)?;
        Ok(())
    };
    match submit() {
        Ok(()) => {}
        // A stop request is not authorization to cancel a submitted job.
        Err(SubmitError::Stop(e)) => return Err(e),
        Err(SubmitError::Other(e)) => {
            unsafe {
                AbortDoc(dc.0);
            }
            return Err(PrintError::Uncertain(format!(
                "Trabajo {job_id}: fallo al enviar el PDF: {e}. Revise la cola y las páginas físicas antes \
                 de reintentar; puede haber páginas impresas."
            )));
        }
    }
    drop(dc);
    wait_for_job(job_id, stop, timeout, Duration::from_millis(250), || job_state(&handle, job_id))
}

enum SubmitError {
    Stop(PrintError),
    Other(String),
}

pub fn list_printers() -> Vec<String> {
    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let mut needed = 0u32;
    let mut count = 0u32;
    unsafe {
        let _ = EnumPrintersW(flags, PCWSTR::null(), 4, None, &mut needed, &mut count);
    }
    if needed == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    let bytes = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, needed as usize) };
    if unsafe { EnumPrintersW(flags, PCWSTR::null(), 4, Some(bytes), &mut needed, &mut count) }.is_err() {
        return Vec::new();
    }
    let infos = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const PRINTER_INFO_4W, count as usize) };
    infos.iter().filter_map(|i| unsafe { i.pPrinterName.to_string().ok() }).collect()
}

pub fn default_printer() -> Option<String> {
    let mut len = 0u32;
    unsafe {
        let _ = GetDefaultPrinterW(PWSTR::null(), &mut len);
    }
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    if !unsafe { GetDefaultPrinterW(PWSTR(buf.as_mut_ptr()), &mut len) }.as_bool() {
        return None;
    }
    String::from_utf16(&buf[..(len as usize).saturating_sub(1)]).ok()
}

