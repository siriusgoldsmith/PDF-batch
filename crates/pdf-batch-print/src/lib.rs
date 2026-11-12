//! PDF rasterisation (PDFium) and Windows GDI printing. Port of `printer_backend.py`.
//!
//! Returning `Ok` from [`print_document`] does NOT prove physical printing: it only
//! means the spooler reports the job completed or removed it after `EndDoc`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use pdfium_render::prelude::*;

mod batch;
mod embedded;
pub use embedded::{extract as extract_pdfium, set_embedded_pdfium};
#[cfg(windows)]
mod gdi;

pub use batch::{run_batch, BatchEvent, BatchOutcome, PrintFn};
#[cfg(windows)]
pub use batch::windows_printer;

/// Raster resolution used for every page (same as v1.x).
pub const DPI: f32 = 150.0;

#[derive(Debug, thiserror::Error)]
pub enum PrintError {
    /// The job must be reconciled manually by the operator before any retry.
    #[error("{0}")]
    Uncertain(String),
    /// Failed before anything was submitted; nothing reached the spooler.
    #[error("{0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, PrintError>;

pub fn stopped() -> PrintError {
    PrintError::Uncertain(
        "Detenido por el usuario. Revise la cola antes de reintentar; el trabajo puede seguir \
         imprimiéndose. No se canceló ningún trabajo de la cola."
            .into(),
    )
}

pub fn check_stop(stop: &AtomicBool) -> Result<()> {
    if stop.load(Ordering::SeqCst) {
        Err(stopped())
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------- PDFium

static PDFIUM: OnceLock<std::result::Result<Pdfium, String>> = OnceLock::new();

/// Directory searched for `pdfium.dll`: next to the executable, else `PDFIUM_DIR`.
fn pdfium_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            v.push(d.to_path_buf());
        }
    }
    if let Ok(d) = std::env::var("PDFIUM_DIR") {
        v.push(PathBuf::from(d));
    }
    v
}

pub fn pdfium() -> Result<&'static Pdfium> {
    PDFIUM
        .get_or_init(|| {
            let mut last = String::from("no se encontró pdfium.dll");
            if let Some(e) = embedded::EMBEDDED.get() {
                match embedded::extract(e.bytes, &e.dir) {
                    Ok(path) => match Pdfium::bind_to_library(&path) {
                        Ok(b) => return Ok(Pdfium::new(b)),
                        Err(err) => last = format!("{err:?}"),
                    },
                    Err(err) => last = err,
                }
            }
            for dir in pdfium_dirs() {
                match Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir)) {
                    Ok(b) => return Ok(Pdfium::new(b)),
                    Err(e) => last = format!("{e:?}"),
                }
            }
            Err(last)
        })
        .as_ref()
        .map_err(|e| PrintError::Failed(format!("No se pudo cargar PDFium: {e}")))
}

/// A rendered page: top-down BGRA pixels, as GDI expects for 32-bpp DIBs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Raster {
    /// Rotate 90° counter-clockwise (PIL `ROTATE_90`, as v1.x did).
    pub fn rotate_90(&self) -> Raster {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out = vec![0u8; self.bgra.len()];
        for y in 0..h {
            for x in 0..w {
                let (nx, ny) = (y, w - 1 - x);
                let src = (y * w + x) * 4;
                let dst = (ny * h + nx) * 4;
                out[dst..dst + 4].copy_from_slice(&self.bgra[src..src + 4]);
            }
        }
        Raster { width: self.height, height: self.width, bgra: out }
    }
}

pub struct Document<'a> {
    doc: PdfDocument<'a>,
}

impl<'a> Document<'a> {
    pub fn open(pdfium: &'a Pdfium, path: &Path) -> Result<Self> {
        pdfium
            .load_pdf_from_file(path, None)
            .map(|doc| Document { doc })
            .map_err(|e| PrintError::Failed(format!("No se pudo abrir el PDF: {e:?}")))
    }

    pub fn page_count(&self) -> u32 {
        self.doc.pages().len() as u32
    }

    /// Render a 0-based page scaled to fit `max` pixels (for on-screen previews).
    pub fn render_fit(&self, index: u32, max: (u32, u32)) -> std::result::Result<Raster, String> {
        let page = self.doc.pages().get(index as _).map_err(|e| format!("{e:?}"))?;
        let (pw, ph) = (page.width().value.max(1.0), page.height().value.max(1.0));
        let s = (max.0 as f32 / pw).min(max.1 as f32 / ph);
        let cfg = PdfRenderConfig::new()
            .set_target_width(((pw * s) as i32).max(1))
            .set_target_height(((ph * s) as i32).max(1))
            .set_clear_color(PdfColor::WHITE)
            .render_form_data(true);
        let bmp = page.render_with_config(&cfg).map_err(|e| format!("{e:?}"))?;
        Ok(Raster { width: bmp.width() as u32, height: bmp.height() as u32, bgra: bmp.as_raw_bytes().to_vec() })
    }

    /// True if the PDF is encrypted with a user password (cannot be opened).
    pub fn page_ok(&self, index: u32) -> bool {
        self.doc.pages().get(index as _).is_ok()
    }

    /// Render a 0-based page at [`DPI`], transparent content composited onto white.
    pub fn render(&self, index: u32) -> std::result::Result<Raster, String> {
        let page = self.doc.pages().get(index as _).map_err(|e| format!("{e:?}"))?;
        let w = ((page.width().value * DPI / 72.0).round() as i32).max(1);
        let h = ((page.height().value * DPI / 72.0).round() as i32).max(1);
        let cfg = PdfRenderConfig::new()
            .set_target_width(w)
            .set_target_height(h)
            .set_clear_color(PdfColor::WHITE)
            .render_form_data(true)
            .rotate_if_landscape(PdfPageRenderRotation::None, false);
        let bmp = page.render_with_config(&cfg).map_err(|e| format!("{e:?}"))?;
        let (bw, bh) = (bmp.width() as u32, bmp.height() as u32);
        let bgra = bmp.as_raw_bytes().to_vec();
        Ok(Raster { width: bw, height: bh, bgra })
    }
}

// ---------------------------------------------------------------- layout

/// Fit an image into the printable area, rotating if that makes it larger.
/// Returns `(rotate, (left, top, right, bottom))` in printable-area device units.
pub fn layout(image: (u32, u32), printable: (i32, i32)) -> (bool, (i32, i32, i32, i32)) {
    let (width, height) = (printable.0 as f64, printable.1 as f64);
    let (mut iw, mut ih) = (image.0 as f64, image.1 as f64);
    let normal = (width / iw).min(height / ih);
    let turned = (width / ih).min(height / iw);
    let rotate = turned > normal;
    if rotate {
        std::mem::swap(&mut iw, &mut ih);
    }
    let scale = normal.max(turned);
    let dw = printable.0.min((iw * scale).round() as i32);
    let dh = printable.1.min((ih * scale).round() as i32);
    let (left, top) = ((printable.0 - dw) / 2, (printable.1 - dh) / 2);
    (rotate, (left, top, left + dw, top + dh))
}

// ---------------------------------------------------------------- spooler wait

/// Spooler job state as seen by [`wait_for_job`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// `GetJob` failed with ERROR_INVALID_PARAMETER / ERROR_NOT_FOUND.
    Gone,
    Status(u32),
}

pub const JOB_ERRORS: u32 = 2 | 4 | 32 | 64 | 512 | 1024;
const JOB_DONE: u32 = 128 | 4096; // PRINTED | COMPLETE

/// Only call AFTER EndDoc. A missing job is queue completion, not paper proof.
pub fn wait_for_job(
    job_id: u32,
    stop: &AtomicBool,
    timeout: Duration,
    poll: Duration,
    mut get: impl FnMut() -> std::result::Result<JobState, String>,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        check_stop(stop)?;
        if Instant::now() >= deadline {
            return Err(PrintError::Uncertain(format!(
                "Trabajo {job_id}: tiempo de espera agotado. Revise la cola y la impresora antes de \
                 reintentar; no se canceló el trabajo."
            )));
        }
        match get().map_err(|e| PrintError::Uncertain(format!("Trabajo {job_id}: no se pudo consultar la cola: {e}")))? {
            JobState::Gone => return Ok(()),
            JobState::Status(s) if s & JOB_ERRORS != 0 => {
                return Err(PrintError::Uncertain(format!(
                    "Trabajo {job_id}: error de impresión (estado {s}). Revise la cola y la impresora \
                     antes de reintentar; puede haber páginas impresas."
                )))
            }
            JobState::Status(s) if s & JOB_DONE != 0 => return Ok(()),
            JobState::Status(_) => std::thread::sleep(poll),
        }
    }
}

// ---------------------------------------------------------------- public API

pub struct Callbacks<'a> {
    /// `(job_id, total_pages)` right after StartDoc.
    pub on_job: &'a mut dyn FnMut(u32, u32) -> std::result::Result<(), String>,
    /// `(page_number_1_based, total_pages)` after each EndPage.
    pub on_progress: &'a mut dyn FnMut(u32, u32) -> std::result::Result<(), String>,
}

/// Submit pages from `start_page` (1-based) and wait for queue completion/removal.
#[cfg(windows)]
pub fn print_document(
    path: &Path,
    printer: &str,
    start_page: u32,
    cb: Callbacks,
    stop: &AtomicBool,
    timeout: Duration,
) -> Result<()> {
    gdi::print_document(path, printer, start_page, cb, stop, timeout)
}

#[cfg(windows)]
pub use gdi::{default_printer, list_printers};
