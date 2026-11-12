use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use pdf_batch_core::QueueStore;
use pdf_batch_print::*;

/// Minimal valid PDF: `pages` pages, each `w`x`h` points with a black box.
fn make_pdf(path: &Path, pages: usize, w: u32, h: u32) {
    let mut objs: Vec<String> = Vec::new();
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i * 2)).collect();
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages));
    for i in 0..pages {
        let content = "0 0 0 rg 10 10 50 50 re f";
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>", 4 + i * 2));
        objs.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    let mut out = String::from("%PDF-1.7\n");
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out += &format!("{} 0 obj\n{o}\nendobj\n", i + 1);
    }
    let xref = out.len();
    out += &format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1);
    for o in offsets {
        out += &format!("{o:010} 00000 n \n");
    }
    out += &format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1);
    std::fs::write(path, out).unwrap();
}

#[test]
fn layout_fits_and_rotates_like_v1() {
    // Portrait page on portrait paper: no rotation, centred.
    let (rot, (l, t, r, b)) = layout((1275, 1650), (4800, 6600));
    assert!(!rot);
    assert!(r - l <= 4800 && b - t <= 6600 && l >= 0 && t >= 0);
    // Landscape page on portrait paper: rotated to use more area.
    let (rot, _) = layout((1650, 1275), (4800, 6600));
    assert!(rot);
}

#[test]
fn rotate_90_moves_pixels_counter_clockwise() {
    // 2x1 image: [A, B] -> 1x2 image: [B; A]
    let r = Raster { width: 2, height: 1, bgra: vec![1, 1, 1, 1, 2, 2, 2, 2] };
    let o = r.rotate_90();
    assert_eq!((o.width, o.height), (1, 2));
    assert_eq!(o.bgra, vec![2, 2, 2, 2, 1, 1, 1, 1]);
}

#[test]
fn wait_for_job_outcomes() {
    let stop = AtomicBool::new(false);
    let ms = Duration::from_millis(1);
    let mut seq = vec![JobState::Status(0), JobState::Status(8), JobState::Gone].into_iter();
    assert!(wait_for_job(7, &stop, Duration::from_secs(5), ms, || Ok(seq.next().unwrap())).is_ok());
    assert!(wait_for_job(7, &stop, Duration::from_secs(5), ms, || Ok(JobState::Status(4096))).is_ok());
    let e = wait_for_job(7, &stop, Duration::from_secs(5), ms, || Ok(JobState::Status(2))).unwrap_err();
    assert!(matches!(e, PrintError::Uncertain(m) if m.contains("error de impresión")));
    let e = wait_for_job(7, &stop, Duration::from_millis(20), ms, || Ok(JobState::Status(0))).unwrap_err();
    assert!(e.to_string().contains("tiempo de espera"));
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(wait_for_job(7, &stop, Duration::from_secs(5), ms, || Ok(JobState::Gone)).unwrap_err().to_string().contains("Detenido"));
}

fn pdfium_available() -> bool {
    if std::env::var("PDFIUM_DIR").is_err() {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/pdfium/bin");
        std::env::set_var("PDFIUM_DIR", d);
    }
    pdfium().is_ok()
}

#[test]
fn renders_pages_at_150_dpi_on_white() {
    assert!(pdfium_available(), "pdfium.dll no encontrado (vendor/pdfium/bin o PDFIUM_DIR)");
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.pdf");
    make_pdf(&p, 3, 612, 792);
    let doc = Document::open(pdfium().unwrap(), &p).unwrap();
    assert_eq!(doc.page_count(), 3);
    let r = doc.render(0).unwrap();
    assert_eq!((r.width, r.height), (1275, 1650));
    // Top-left is white background; bottom-left box area is black.
    assert_eq!(&r.bgra[0..3], &[255, 255, 255]);
    let y = 1650 - 30 * 150 / 72;
    let x = 30 * 150 / 72;
    let i = ((y * 1275 + x) * 4) as usize;
    assert_eq!(&r.bgra[i..i + 3], &[0, 0, 0]);
}

#[test]
fn batch_prints_in_order_and_stops_safely_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::open(dir.path().join("q.sqlite")).unwrap();
    let srcs: Vec<PathBuf> = (0..4)
        .map(|i| {
            let p = dir.path().join(format!("{i}.pdf"));
            make_pdf(&p, 2, 612, 792);
            p
        })
        .collect();
    let ids = store.add(&srcs).unwrap();
    let (tx, rx) = mpsc::channel();
    let pause = AtomicBool::new(false);
    let mut calls = 0;
    let mut fake: Box<PrintFn> = Box::new(|_p, _pr, start, cb, _s| {
        calls += 1;
        (cb.on_job)(100 + calls, 2).unwrap();
        for page in start..=2 {
            (cb.on_progress)(page, 2).unwrap();
        }
        if calls == 3 {
            return Err(PrintError::Uncertain("atasco".into()));
        }
        Ok(())
    });
    let out = run_batch(&store, "Fake", 10, &pause, &tx, &mut *fake);
    drop(fake);
    assert_eq!(out, BatchOutcome::Error("atasco".into()));
    let items = store.list_items().unwrap();
    let states: Vec<&str> = items.iter().map(|i| i.state.as_str()).collect();
    assert_eq!(states, ["sent", "sent", "uncertain", "pending"]);
    assert_eq!(items[0].job_id.as_deref(), Some("101"));
    assert_eq!(items[2].last_page, 2);
    let evs: Vec<BatchEvent> = rx.try_iter().collect();
    assert!(evs.contains(&BatchEvent::Batch(2, 10)));
    // Nothing more can start until the operator resolves the uncertain item.
    let mut never: Box<PrintFn> = Box::new(|_, _, _, _, _| panic!("should not print"));
    assert!(matches!(run_batch(&store, "Fake", 10, &pause, &tx, &mut *never), BatchOutcome::Finished { completed: 0, .. }));
    let _ = ids;
}

#[test]
fn pause_takes_effect_between_documents() {
    let dir = tempfile::tempdir().unwrap();
    let store = QueueStore::open(dir.path().join("q.sqlite")).unwrap();
    let p = dir.path().join("a.pdf");
    make_pdf(&p, 1, 612, 792);
    store.add(&[&p, &p, &p]).unwrap();
    let (tx, _rx) = mpsc::channel();
    let pause = AtomicBool::new(false);
    let mut fake: Box<PrintFn> = Box::new(|_, _, _, cb, _| {
        (cb.on_job)(1, 1).unwrap();
        (cb.on_progress)(1, 1).unwrap();
        pause.store(true, std::sync::atomic::Ordering::SeqCst); // user presses Pause mid-document
        Ok(())
    });
    let out = run_batch(&store, "Fake", 3, &pause, &tx, &mut *fake);
    assert_eq!(out, BatchOutcome::Paused { completed: 1, target: 3 });
}

#[test]
fn lists_installed_printers() {
    // Every Windows install has at least "Microsoft Print to PDF" unless removed.
    let printers = list_printers();
    eprintln!("impresoras: {printers:?}, predeterminada: {:?}", default_printer());
}

#[test]
fn embedded_dll_is_extracted_once_and_tampering_is_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let bytes: &[u8] = b"MZ fake dll bytes";
    let p = extract_pdfium(bytes, dir.path()).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), bytes);
    let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
    assert_eq!(extract_pdfium(bytes, dir.path()).unwrap(), p);
    assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), mtime, "rewrote a valid copy");
    std::fs::write(&p, b"MZ malicious").unwrap();
    assert_eq!(extract_pdfium(bytes, dir.path()).unwrap(), p);
    assert_eq!(std::fs::read(&p).unwrap(), bytes);
    assert!(extract_pdfium(b"", dir.path()).is_err());
    // No temp files left behind.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
