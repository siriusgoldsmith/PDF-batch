use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pdf_batch_core::{Error, QueueStore, Resolution};
use serde_json::Value;

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
    store: QueueStore,
}

fn fx() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let db = root.join("journal").join("queue.sqlite");
    let store = QueueStore::open(&db).unwrap();
    Fx { _tmp: tmp, root, db, store }
}

impl Fx {
    fn add(&self, n: usize) -> Vec<i64> {
        let srcs: Vec<PathBuf> = (0..n)
            .map(|i| {
                let p = self.root.join(format!("{i}.pdf"));
                fs::write(&p, b"%PDF-1.7\noriginal").unwrap();
                p
            })
            .collect();
        self.store.add(&srcs).unwrap()
    }
}

fn msg(e: Error) -> String {
    e.to_string()
}

#[test]
fn add_preserves_order_and_snapshots() {
    let f = fx();
    let ids = f.add(250);
    let items = f.store.list_items().unwrap();
    assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), ids);
    assert!(items.iter().all(|i| Path::new(&i.snapshot).exists() && i.state == "pending"));
}

#[test]
fn invalid_batch_rolls_back_rows_and_snapshots() {
    let f = fx();
    let good = f.root.join("a.pdf");
    fs::write(&good, b"%PDF-1.7").unwrap();
    let bad = f.root.join("b.pdf");
    fs::write(&bad, b"nope").unwrap();
    assert!(f.store.add(&[good, bad]).is_err());
    assert!(f.store.list_items().unwrap().is_empty());
    assert_eq!(fs::read_dir(f.store.documents()).unwrap().count(), 0);
}

#[test]
fn lifecycle_claim_progress_sent_and_recovery() {
    let f = fx();
    let ids = f.add(2);
    assert!(f.store.claim(ids[1], "P").is_err());
    f.store.claim(ids[0], "P").unwrap();
    f.store.progress(ids[0], 2, Some(5), Some("job")).unwrap();
    assert!(f.store.progress(ids[0], 1, None, None).is_err());
    assert!(f.store.progress(ids[0], 6, None, None).is_err());
    f.store.mark_sent(ids[0]).unwrap();
    f.store.claim(ids[1], "P").unwrap();
    let reopened = QueueStore::open(&f.db).unwrap();
    assert_eq!(reopened.recover().unwrap(), 1);
    let it = &reopened.list_items().unwrap()[1];
    assert_eq!((it.state.as_str(), it.queue_outcome.as_str()), ("uncertain", "review"));
    reopened.resolve(ids[1], Resolution::Retry, 3).unwrap();
    let it = &reopened.list_items().unwrap()[1];
    assert_eq!((it.state.as_str(), it.start_page, it.last_page), ("pending", 3, 2));
}

#[test]
fn tampered_snapshot_blocks_dispatch() {
    let f = fx();
    let id = f.add(1)[0];
    fs::write(&f.store.list_items().unwrap()[0].snapshot, b"%PDF-tampered").unwrap();
    assert!(msg(f.store.claim(id, "P").unwrap_err()).contains("SHA256"));
    assert_eq!(f.store.list_items().unwrap()[0].state, "uncertain");
}

#[test]
fn move_stays_within_pending_segment() {
    let f = fx();
    let ids = f.add(3);
    assert!(f.store.move_item(ids[2], -5).unwrap());
    let order: Vec<i64> = f.store.list_items().unwrap().iter().map(|i| i.id).collect();
    assert_eq!(order, vec![ids[2], ids[0], ids[1]]);
    assert!(!f.store.move_item(ids[2], -1).unwrap());
}

#[test]
fn clear_is_atomic_with_busy_or_uncertain_then_removes_all() {
    let f = fx();
    let ids = f.add(2);
    f.store.claim(ids[0], "P").unwrap();
    assert!(msg(f.store.clear_items().unwrap_err()).contains("curso"));
    f.store.fail(ids[0], "offline").unwrap();
    assert!(msg(f.store.clear_items().unwrap_err()).contains("resolver"));
    f.store.resolve(ids[0], Resolution::Skip, 1).unwrap();
    let before = f.store.list_items().unwrap();
    assert_eq!(f.store.clear_items().unwrap(), 2);
    assert!(QueueStore::open(&f.db).unwrap().list_items().unwrap().is_empty());
    for row in &before {
        let ev = f.store.events(Some(row.id)).unwrap().pop().unwrap();
        assert_eq!(ev.action, "clear");
        let prev: Value = serde_json::from_str(&ev.detail).unwrap();
        assert_eq!(prev["previous"], serde_json::to_value(row).unwrap());
        assert!(!Path::new(&row.snapshot).exists());
        assert!(Path::new(&row.source).exists());
    }
    assert_eq!(f.store.clear_items().unwrap(), 0);
}

#[test]
fn delete_rejects_unknown_ids_atomically() {
    let f = fx();
    let ids = f.add(2);
    let before = (f.store.list_items().unwrap(), f.store.events(None).unwrap());
    for bad in [9999, -1, 0] {
        assert!(f.store.delete_items(&[ids[1], bad]).is_err());
        assert_eq!((f.store.list_items().unwrap(), f.store.events(None).unwrap()), before);
    }
    assert_eq!(f.store.delete_items(&[ids[0], ids[0]]).unwrap(), 1);
}

#[test]
fn malicious_snapshot_paths_never_delete_other_files() {
    let f = fx();
    let ids = f.add(2);
    let outside = f.root.join("valuable.pdf");
    fs::write(&outside, b"valuable").unwrap();
    let inside = f.store.documents().join(format!("{}.pdf", "c".repeat(32)));
    fs::write(&inside, b"valuable").unwrap();
    let other_snap = f.store.list_items().unwrap()[1].snapshot.clone();
    for target in [outside.to_string_lossy().into_owned(), inside.to_string_lossy().into_owned(), other_snap.clone()] {
        let id = f.add(1)[0];
        f.store
            .tx(|db| Ok(db.execute("UPDATE items SET snapshot=? WHERE id=?", rusqlite::params![target, id])?))
            .unwrap();
        assert_eq!(f.store.delete_items(&[id]).unwrap(), 1);
        assert!(Path::new(&target).exists(), "{target} was deleted");
    }
    let _ = ids;
}

#[test]
fn owned_copy_used_as_another_source_is_preserved() {
    let f = fx();
    f.add(1);
    let copy = PathBuf::from(&f.store.list_items().unwrap()[0].snapshot);
    f.store.add(&[&copy]).unwrap();
    assert_eq!(f.store.clear_items().unwrap(), 2);
    assert!(copy.exists());
}

#[test]
fn delete_pending_retains_snapshot() {
    let f = fx();
    let id = f.add(1)[0];
    let snap = f.store.list_items().unwrap()[0].snapshot.clone();
    assert_eq!(f.store.delete_pending(&[id]).unwrap(), 1);
    assert!(Path::new(&snap).exists());
}

/// Regression: "Vaciar carpeta del lote se queda pegado". With many rows and a
/// large history of sources on unreachable paths, clearing must stay fast.
#[test]
fn clear_large_batch_with_unreachable_sources_is_fast() {
    let f = fx();
    f.add(500);
    // Simulate history whose originals lived on a disconnected network share.
    f.store
        .tx(|db| {
            for i in 0..2000 {
                db.execute(
                    "INSERT INTO events(item_id,action,detail) VALUES(?, 'add', ?)",
                    rusqlite::params![100_000 + i, format!(r#"{{"source":"\\\\servidor-caido\\lote\\{i}.pdf"}}"#)],
                )?;
            }
            db.execute_batch(r"UPDATE items SET source='\\servidor-caido\lote\' || id || '.pdf'")?;
            Ok(())
        })
        .unwrap();
    let t = Instant::now();
    assert_eq!(f.store.clear_items().unwrap(), 500);
    let took = t.elapsed();
    assert!(took.as_secs_f64() < 2.0, "clear took {took:?}");
    assert_eq!(fs::read_dir(f.store.documents()).unwrap().count(), 0);
}

#[test]
fn opens_python_v14_database_and_migrates_ownership() {
    let f = fx();
    let id = f.add(1)[0];
    let snap = f.store.list_items().unwrap()[0].snapshot.clone();
    // Recreate the state a Python-made journal would have: no owned_snapshots table.
    f.store.tx(|db| Ok(db.execute_batch("DROP TABLE owned_snapshots")?)).unwrap();
    let reopened = QueueStore::open(&f.db).unwrap();
    assert_eq!(reopened.delete_items(&[id]).unwrap(), 1);
    assert!(!Path::new(&snap).exists());
}

#[test]
fn recovery_reference_falls_back_to_previous_sent_page() {
    let f = fx();
    let ids = f.add(2);
    f.store.claim(ids[0], "P").unwrap();
    f.store.progress(ids[0], 3, Some(3), None).unwrap();
    f.store.mark_sent(ids[0]).unwrap();
    f.store.claim(ids[1], "P").unwrap();
    f.store.fail(ids[1], "x").unwrap();
    let items = f.store.list_items().unwrap();
    let (it, page) = pdf_batch_core::recovery_reference(&items, ids[1]).unwrap();
    assert_eq!((it.id, page), (ids[0], 3));
}

#[test]
fn rebase_moves_snapshot_paths_and_keeps_ownership() {
    let f = fx();
    let id = f.add(1)[0];
    let old_dir = f.db.parent().unwrap().to_path_buf();
    let canon_old = f.store.documents().parent().unwrap().to_path_buf();
    let new_dir = f.root.join("PDF-batch");
    drop(f.store);
    fs::rename(&old_dir, &new_dir).unwrap();
    let store = QueueStore::open(new_dir.join("queue.sqlite")).unwrap();
    store.rebase(&canon_old, store.documents().parent().unwrap()).unwrap();
    let snap = store.list_items().unwrap()[0].snapshot.clone();
    assert!(Path::new(&snap).exists(), "{snap}");
    assert_eq!(store.delete_items(&[id]).unwrap(), 1);
    assert!(!Path::new(&snap).exists());
}
