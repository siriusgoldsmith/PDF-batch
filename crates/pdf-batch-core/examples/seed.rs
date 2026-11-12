//! QA helper: `cargo run -p pdf-batch-core --example seed -- <data_dir>` creates a
//! demo queue (sent, uncertain and pending items) for manual GUI checks.
use pdf_batch_core::QueueStore;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("data_dir"));
    let src = dir.join("_origen");
    std::fs::create_dir_all(&src).unwrap();
    let pdf = b"%PDF-1.7\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj 2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj 3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Contents 4 0 R>>endobj 4 0 obj<</Length 25>>stream\n0 0 0 rg 72 600 300 80 re f\nendstream endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
    let names = ["Factura 001.pdf", "Factura 002.pdf", "Contrato María.pdf", "Informe anual.pdf", "Anexo A.pdf", "Anexo B.pdf"];
    let paths: Vec<PathBuf> = names.iter().map(|n| { let p = src.join(n); std::fs::write(&p, pdf).unwrap(); p }).collect();
    let store = QueueStore::open(dir.join("queue.sqlite3")).unwrap();
    let ids = store.add(&paths).unwrap();
    for (i, id) in ids.iter().take(3).enumerate() {
        store.claim(*id, "Kyocera ECOSYS PA5500x KX SANDRA").unwrap();
        store.progress(*id, 1, Some(1), Some(&(40 + i).to_string())).unwrap();
        if i < 2 || std::env::var_os("SEED_NO_INTERRUPTION").is_some() { store.mark_sent(*id).unwrap() } else { store.fail(*id, "Trabajo 42: error de impresión (estado 2).").unwrap() }
    }
    println!("cola de demostración en {}", dir.display());
}
