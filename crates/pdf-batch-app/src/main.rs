#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! PDF-batch: ordered, durable, conservative Windows printing. Port of `app.py`.

mod app;
mod i18n;

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use pdf_batch_core::QueueStore;

pub const APP_NAME: &str = "PDF-batch";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cross-session lifetime lock: an exclusive (share-mode 0) handle that Windows
/// releases if the process dies. Same file and semantics as v1.x `instance_lock.py`.
fn lock(path: &Path) -> Result<File, String> {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let mut o = OpenOptions::new();
    o.read(true).write(true).create(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.share_mode(0);
    }
    o.open(path).map_err(|e| match e.raw_os_error() {
        Some(32) | Some(33) => i18n::t("alreadyOpen").to_string(),
        _ => e.to_string(),
    })
}

fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Move `%LOCALAPPDATA%\PDFLote` (v1.x) to `%LOCALAPPDATA%\PDF-batch` once.
/// Returns the data dir to use and, if moved, the old canonical path to rebase.
fn migrate(base: &Path) -> Result<(PathBuf, Option<PathBuf>), String> {
    let new = base.join(APP_NAME);
    let old = base.join("PDFLote");
    if new.exists() || !old.join("queue.sqlite3").exists() {
        return Ok((new, None));
    }
    // Holding v1.x's own lock proves PDFLote is not running right now.
    let guard = lock(&old.join("queue.lock"))
        .map_err(|_| i18n::t("v1Open").to_string())?;
    let canon = old.canonicalize().map_err(|e| e.to_string())?;
    let canon = PathBuf::from(canon.to_string_lossy().trim_start_matches(r"\\?\"));
    drop(guard);
    match std::fs::rename(&old, &new) {
        Ok(()) => Ok((new, Some(canon))),
        // Could not move (e.g. a file is in use): keep working in the old folder.
        Err(_) => Ok((old, None)),
    }
}

static PDFIUM_DLL: &[u8] = include_bytes!(env!("PDFIUM_EMBED"));

fn start() -> Result<(), String> {
    let (data_dir, moved_from) = migrate(&local_app_data())?;
    i18n::init(&data_dir);
    pdf_batch_print::set_embedded_pdfium(PDFIUM_DLL, data_dir.join("runtime"));
    let _lifetime = lock(&data_dir.join("queue.lock"))?;
    let store = QueueStore::open(data_dir.join("queue.sqlite3")).map_err(|e| e.to_string())?;
    if let Some(old) = moved_from {
        let new = store.documents().parent().unwrap().to_path_buf();
        store.rebase(&old, &new).map_err(|e| e.to_string())?;
    }
    store.recover().map_err(|e| e.to_string())?;

    let icon = image::load_from_memory(include_bytes!("icon.png")).ok().map(|i| {
        let i = i.to_rgba8();
        egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() }
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("PDF-batch v{VERSION} · siriusgoldsmith"))
        .with_inner_size([1100.0, 720.0])
        .with_min_inner_size([960.0, 640.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, store, data_dir)))),
    )
    .map_err(|e| e.to_string())
}

use eframe::egui;

fn main() {
    // Startup errors can happen before the data folder (and the saved choice) is known.
    i18n::init_from_windows();
    if let Err(e) = start() {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(i18n::t("startFailed"))
            .set_description(e)
            .show();
    }
}
