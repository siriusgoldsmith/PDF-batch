//! Durable, ordered PDF dispatch journal. `Sent` never means physically printed.
//!
//! Port of `core.py` (PDFLote v1.4). The on-disk schema is kept compatible so an
//! existing `queue.sqlite` opens unchanged; one table (`owned_snapshots`) is added.
//!
//! Fix for the "Vaciar carpeta del lote se queda pegado" bug: removal no longer
//! calls `canonicalize()` on every historical source path once per row (O(n·m)
//! filesystem calls, very slow on network/USB paths) nor re-hashes legacy copies.
//! Ownership of copies is recorded at `add` time (and migrated once on open), and
//! the protected set is compared as normalised strings.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: i64,
    pub position: i64,
    pub source: String,
    pub name: String,
    pub snapshot: String,
    pub sha256: String,
    pub state: String,
    pub job_id: Option<String>,
    pub last_page: i64,
    pub total_pages: Option<i64>,
    pub error: Option<String>,
    pub printer: Option<String>,
    pub start_page: i64,
    pub queue_outcome: String,
    pub physical_status: String,
    pub physical_confirmed_at: Option<String>,
}

impl Item {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Item {
            id: r.get("id")?,
            position: r.get("position")?,
            source: r.get("source")?,
            name: r.get("name")?,
            snapshot: r.get("snapshot")?,
            sha256: r.get("sha256")?,
            state: r.get("state")?,
            job_id: r.get("job_id")?,
            last_page: r.get("last_page")?,
            total_pages: r.get("total_pages")?,
            error: r.get("error")?,
            printer: r.get("printer")?,
            start_page: r.get("start_page")?,
            queue_outcome: r.get("queue_outcome")?,
            physical_status: r.get("physical_status")?,
            physical_confirmed_at: r.get("physical_confirmed_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: i64,
    pub item_id: i64,
    pub action: String,
    pub detail: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Retry,
    Sent,
    Skip,
}

#[derive(Clone)]
pub struct QueueStore {
    db_path: PathBuf,
    documents: PathBuf,
}

/// Strip the `\\?\` verbatim prefix Windows `canonicalize` adds.
fn plain(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}

/// Lexical key used for "is this path protected?" comparisons. No filesystem access.
fn key(p: &str) -> String {
    let s = p.strip_prefix(r"\\?\").unwrap_or(p);
    let s = s.replace('/', "\\");
    if cfg!(windows) { s.to_lowercase() } else { s }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn is_uuid_pdf(path: &Path) -> bool {
    path.extension().map_or(false, |e| e == "pdf")
        && path.file_stem().and_then(|s| s.to_str()).map_or(false, |s| {
            s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

fn is_symlink(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

impl QueueStore {
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_path = db_path.as_ref();
        let parent = db_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let parent = plain(parent.canonicalize()?);
        let db_path = parent.join(db_path.file_name().ok_or_else(|| Error::Invalid("Ruta de base de datos inválida".into()))?);
        let documents = parent.join("documents");
        if !documents.exists() {
            fs::create_dir(&documents)?;
        }
        let store = QueueStore { db_path, documents };
        store.tx(|db| {
            db.execute_batch(
                "CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY AUTOINCREMENT, position INTEGER NOT NULL,
                source TEXT NOT NULL, name TEXT NOT NULL, snapshot TEXT NOT NULL,
                sha256 TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending'
                CHECK(state IN ('pending','in_flight','sent','uncertain','skipped')),
                job_id TEXT, last_page INTEGER NOT NULL DEFAULT 0,
                total_pages INTEGER, error TEXT, printer TEXT,
                start_page INTEGER NOT NULL DEFAULT 1 CHECK(start_page >= 1));",
            )?;
            let cols: HashSet<String> = db
                .prepare("PRAGMA table_info(items)")?
                .query_map([], |r| r.get::<_, String>("name"))?
                .collect::<rusqlite::Result<_>>()?;
            for (name, def) in [
                ("queue_outcome", "TEXT NOT NULL DEFAULT 'unrecorded' CHECK(queue_outcome IN ('unrecorded','completed','review'))"),
                ("physical_status", "TEXT NOT NULL DEFAULT 'unconfirmed' CHECK(physical_status IN ('unconfirmed','confirmed','incomplete'))"),
                ("physical_confirmed_at", "TEXT"),
            ] {
                if !cols.contains(name) {
                    db.execute_batch(&format!("ALTER TABLE items ADD COLUMN {name} {def}"))?;
                }
            }
            db.execute_batch(
                "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY, item_id INTEGER NOT NULL, action TEXT NOT NULL,
                detail TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT
                (strftime('%Y-%m-%dT%H:%M:%fZ','now')));",
            )?;
            let fresh: bool = db
                .query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name='owned_snapshots'", [], |_| Ok(()))
                .optional()?
                .is_none();
            if fresh {
                db.execute_batch(
                    "CREATE TABLE owned_snapshots (item_id INTEGER PRIMARY KEY, path TEXT NOT NULL);",
                )?;
                store.migrate_ownership(db)?;
            }
            Ok(())
        })?;
        Ok(store)
    }

    /// One-time migration from the Python journal: record which copies the app owns.
    /// Legacy (v1.3) add events lack the snapshot path, so those copies are verified
    /// by hash exactly once here instead of on every clear.
    fn migrate_ownership(&self, db: &Transaction) -> Result<()> {
        let mut adds: Vec<(i64, Value)> = Vec::new();
        {
            let mut st = db.prepare("SELECT item_id, detail FROM events WHERE action='add'")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            for r in rows {
                let (id, d) = r?;
                adds.push((id, serde_json::from_str(&d).unwrap_or(Value::Null)));
            }
        }
        for (item_id, detail) in adds {
            if let Some(snap) = detail.get("snapshot").and_then(Value::as_str) {
                db.execute("INSERT OR REPLACE INTO owned_snapshots VALUES(?,?)", params![item_id, snap])?;
                continue;
            }
            let row: Option<(String, String, String)> = db
                .query_row("SELECT source, sha256, snapshot FROM items WHERE id=?", [item_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?;
            let Some((source, sha, snapshot)) = row else { continue };
            if detail.get("source").and_then(Value::as_str) != Some(&source)
                || detail.get("sha256").and_then(Value::as_str) != Some(&sha)
            {
                continue;
            }
            let path = PathBuf::from(&snapshot);
            if is_uuid_pdf(&path)
                && path.parent() == Some(self.documents.as_path())
                && !is_symlink(&path)
                && sha256_file(&path).ok().as_deref() == Some(sha.as_str())
            {
                db.execute("INSERT OR REPLACE INTO owned_snapshots VALUES(?,?)", params![item_id, snapshot])?;
            }
        }
        Ok(())
    }

    pub fn documents(&self) -> &Path {
        &self.documents
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Open a connection and run `f` inside `BEGIN IMMEDIATE`, with `synchronous=FULL`.
    pub fn tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let mut conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(Duration::from_secs(30))?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?; // dropped tx rolls back on error
        tx.commit()?;
        Ok(out)
    }

    fn event(db: &Transaction, item_id: i64, action: &str, detail: Value) -> Result<()> {
        db.execute(
            "INSERT INTO events(item_id,action,detail) VALUES(?,?,?)",
            params![item_id, action, detail.to_string()],
        )?;
        Ok(())
    }

    fn item(db: &Transaction, id: i64) -> Result<Item> {
        db.query_row("SELECT * FROM items WHERE id=?", [id], Item::from_row)
            .optional()?
            .ok_or_else(|| Error::Invalid("Unknown item".into()))
    }

    fn first(db: &Transaction) -> Result<Option<Item>> {
        Ok(db
            .query_row(
                "SELECT * FROM items WHERE state NOT IN ('sent','skipped') ORDER BY position,id LIMIT 1",
                [],
                Item::from_row,
            )
            .optional()?)
    }

    fn ensure_idle(db: &Transaction) -> Result<()> {
        let busy = db
            .query_row("SELECT 1 FROM items WHERE state='in_flight' LIMIT 1", [], |_| Ok(()))
            .optional()?;
        if busy.is_some() {
            return invalid("Cannot edit queue while an item is in flight");
        }
        Ok(())
    }

    pub fn events(&self, item_id: Option<i64>) -> Result<Vec<Event>> {
        self.tx(|db| {
            let map = |r: &Row| {
                Ok(Event { id: r.get(0)?, item_id: r.get(1)?, action: r.get(2)?, detail: r.get(3)?, created_at: r.get(4)? })
            };
            let sql = "SELECT id,item_id,action,detail,created_at FROM events";
            let v = match item_id {
                None => db.prepare(&format!("{sql} ORDER BY id"))?.query_map([], map)?.collect::<rusqlite::Result<_>>()?,
                Some(i) => db
                    .prepare(&format!("{sql} WHERE item_id=? ORDER BY id"))?
                    .query_map([i], map)?
                    .collect::<rusqlite::Result<_>>()?,
            };
            Ok(v)
        })
    }

    pub fn list_items(&self) -> Result<Vec<Item>> {
        self.tx(|db| {
            Ok(db
                .prepare("SELECT * FROM items ORDER BY position,id")?
                .query_map([], Item::from_row)?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn add<P: AsRef<Path>>(&self, paths: &[P]) -> Result<Vec<i64>> {
        let mut created: Vec<PathBuf> = Vec::new();
        let result = self.tx(|db| {
            let mut position: i64 = db.query_row("SELECT COALESCE(MAX(position),0) FROM items", [], |r| r.get(0))?;
            let mut ids = Vec::new();
            for source in paths {
                let source = plain(source.as_ref().canonicalize()?);
                if source.extension().map(|e| e.to_ascii_lowercase()) != Some("pdf".into()) {
                    return invalid("Only .pdf files are accepted");
                }
                let mut sig = [0u8; 5];
                let n = File::open(&source)?.read(&mut sig)?;
                if n < 5 || &sig != b"%PDF-" {
                    return invalid(format!("Invalid PDF signature: {}", source.display()));
                }
                let snapshot = self.documents.join(format!("{}.pdf", uuid::Uuid::new_v4().simple()));
                let mut dst = OpenOptions::new().write(true).create_new(true).open(&snapshot)?;
                created.push(snapshot.clone());
                let mut src = File::open(&source)?;
                let mut h = Sha256::new();
                let mut buf = vec![0u8; 1024 * 1024];
                loop {
                    let n = src.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    h.update(&buf[..n]);
                    dst.write_all(&buf[..n])?;
                }
                dst.flush()?;
                dst.sync_all()?;
                let digest = format!("{:x}", h.finalize());
                position += 1;
                let src_s = source.to_string_lossy().into_owned();
                let snap_s = snapshot.to_string_lossy().into_owned();
                let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                db.execute(
                    "INSERT INTO items(position,source,name,snapshot,sha256) VALUES(?,?,?,?,?)",
                    params![position, src_s, name, snap_s, digest],
                )?;
                let id = db.last_insert_rowid();
                db.execute("INSERT INTO owned_snapshots VALUES(?,?)", params![id, snap_s])?;
                Self::event(db, id, "add", json!({"source": src_s, "snapshot": snap_s, "sha256": digest}))?;
                ids.push(id);
            }
            Ok(ids)
        });
        if result.is_err() {
            for s in &created {
                let _ = fs::remove_file(s);
            }
        }
        result
    }

    pub fn pending(&self) -> Result<Option<Item>> {
        self.tx(|db| Ok(Self::first(db)?.filter(|i| i.state == "pending")))
    }

    pub fn claim(&self, item_id: i64, printer: &str) -> Result<Item> {
        let mut problem: Option<String> = None;
        let item = self.tx(|db| {
            match Self::first(db)? {
                Some(r) if r.id == item_id && r.state == "pending" => {
                    db.execute(
                        "UPDATE items SET queue_outcome='unrecorded',physical_status='unconfirmed',physical_confirmed_at=NULL WHERE id=?",
                        [item_id],
                    )?;
                    match sha256_file(Path::new(&r.snapshot)) {
                        Ok(d) if d == r.sha256 => {}
                        Ok(_) => problem = Some("Snapshot SHA256 mismatch".into()),
                        Err(e) => problem = Some(format!("Snapshot unavailable: {e}")),
                    }
                    if let Some(p) = &problem {
                        db.execute("UPDATE items SET state='uncertain',error=? WHERE id=?", params![p, item_id])?;
                        Self::event(db, item_id, "integrity_error", json!({"error": p}))?;
                    } else {
                        db.execute("UPDATE items SET state='in_flight',printer=?,error=NULL WHERE id=?", params![printer, item_id])?;
                        Self::event(db, item_id, "claim", json!({"printer": printer, "start_page": r.start_page}))?;
                    }
                    Self::item(db, item_id)
                }
                _ => invalid("Only the first pending item can be claimed"),
            }
        })?;
        match problem {
            Some(p) => Err(Error::Invalid(p)),
            None => Ok(item),
        }
    }

    pub fn progress(&self, item_id: i64, last_page: i64, total_pages: Option<i64>, job_id: Option<&str>) -> Result<()> {
        self.tx(|db| {
            let row = Self::item(db, item_id)?;
            if row.state != "in_flight" {
                return invalid("Progress requires in_flight state");
            }
            if last_page < row.last_page || last_page < 0 {
                return invalid("last_page must be a non-regressing integer");
            }
            if let Some(t) = total_pages.or(row.total_pages) {
                if t < 1 || last_page > t {
                    return invalid("Invalid total_pages or last_page exceeds total");
                }
            }
            db.execute(
                "UPDATE items SET last_page=?,total_pages=COALESCE(?,total_pages),job_id=COALESCE(?,job_id) WHERE id=?",
                params![last_page, total_pages, job_id, item_id],
            )?;
            Self::event(db, item_id, "progress", json!({"last_page": last_page, "total_pages": total_pages, "job_id": job_id}))
        })
    }

    pub fn mark_sent(&self, item_id: i64) -> Result<()> {
        self.tx(|db| {
            if Self::item(db, item_id)?.state != "in_flight" {
                return invalid("Completion requires in_flight state");
            }
            db.execute("UPDATE items SET state='sent',queue_outcome='completed',error=NULL WHERE id=?", [item_id])?;
            Self::event(db, item_id, "sent", json!({}))
        })
    }

    pub fn fail(&self, item_id: i64, error: &str) -> Result<()> {
        self.tx(|db| {
            if Self::item(db, item_id)?.state != "in_flight" {
                return invalid("Failure requires in_flight state");
            }
            db.execute(
                "UPDATE items SET state='uncertain',queue_outcome='review',error=? WHERE id=?",
                params![error, item_id],
            )?;
            Self::event(db, item_id, "fail", json!({"error": error}))
        })
    }

    /// Call once at application startup, never while a worker is running.
    pub fn recover(&self) -> Result<usize> {
        self.tx(|db| {
            let ids: Vec<i64> = db
                .prepare("SELECT id FROM items WHERE state='in_flight'")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            db.execute(
                "UPDATE items SET state='uncertain',queue_outcome='review',error='Interrupted while in flight; operator review required' WHERE state='in_flight'",
                [],
            )?;
            for id in &ids {
                Self::event(db, *id, "recover", json!({}))?;
            }
            Ok(ids.len())
        })
    }

    /// Record operator-observed paper output, never infer Windows completion.
    pub fn confirm_physical(&self, item_id: i64, printed: bool) -> Result<()> {
        self.tx(|db| {
            Self::ensure_idle(db)?;
            let row = Self::item(db, item_id)?;
            if row.state != "sent" && row.state != "uncertain" {
                return invalid("Physical confirmation requires sent or uncertain state");
            }
            db.execute(
                "UPDATE items SET state=?,physical_status=?,physical_confirmed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),error=? WHERE id=?",
                params![
                    if printed { "sent" } else { "uncertain" },
                    if printed { "confirmed" } else { "incomplete" },
                    if printed { None } else { Some("Impresión física incompleta. Revise las hojas y resuelva antes de continuar.") },
                    item_id
                ],
            )?;
            Self::event(
                db,
                item_id,
                if printed { "physical_confirmed" } else { "physical_incomplete" },
                json!({"previous": row}),
            )
        })
    }

    pub fn resolve(&self, item_id: i64, action: Resolution, start_page: i64) -> Result<()> {
        if start_page < 1 {
            return invalid("start_page must be a positive integer");
        }
        self.tx(|db| {
            if Self::item(db, item_id)?.state != "uncertain" {
                return invalid("Only uncertain items can be resolved");
            }
            let name = match action {
                Resolution::Retry => {
                    db.execute(
                        "UPDATE items SET state='pending',start_page=?,last_page=?,job_id=NULL,error=NULL,
                         queue_outcome='unrecorded',physical_status='unconfirmed',physical_confirmed_at=NULL WHERE id=?",
                        params![start_page, start_page - 1, item_id],
                    )?;
                    "resolve_retry"
                }
                Resolution::Sent => {
                    db.execute(
                        "UPDATE items SET state='sent',error=NULL,physical_status='confirmed',
                         physical_confirmed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
                        [item_id],
                    )?;
                    "resolve_sent"
                }
                Resolution::Skip => {
                    db.execute(
                        "UPDATE items SET state='skipped',error=NULL,physical_status='unconfirmed',physical_confirmed_at=NULL WHERE id=?",
                        [item_id],
                    )?;
                    "resolve_skip"
                }
            };
            Self::event(db, item_id, name, json!({"start_page": start_page}))
        })
    }

    /// Legacy API: removes pending rows but keeps their copies as audit evidence.
    pub fn delete_pending(&self, ids: &[i64]) -> Result<usize> {
        let ids = dedup(ids);
        self.tx(|db| {
            Self::ensure_idle(db)?;
            let rows = ids.iter().map(|&i| Self::item(db, i)).collect::<Result<Vec<_>>>()?;
            if rows.iter().any(|r| r.state != "pending") {
                return invalid("Only pending items can be deleted");
            }
            for r in &rows {
                Self::event(db, r.id, "delete", json!({"snapshot": r.snapshot}))?;
                db.execute("DELETE FROM items WHERE id=?", [r.id])?;
            }
            Ok(rows.len())
        })
    }

    /// Explicitly remove selected rows, retaining their complete audit history.
    pub fn delete_items(&self, ids: &[i64]) -> Result<usize> {
        if ids.iter().any(|&i| i < 1) {
            return invalid("Cada identificador debe ser un entero positivo.");
        }
        self.remove_items(Some(dedup(ids)), "delete")
    }

    /// Remove the entire idle queue only after all uncertain work is resolved.
    pub fn clear_items(&self) -> Result<usize> {
        self.remove_items(None, "clear")
    }

    fn remove_items(&self, ids: Option<Vec<i64>>, action: &str) -> Result<usize> {
        let (rows, owned, protected) = self.tx(|db| {
            if Self::ensure_idle(db).is_err() {
                return invalid("No se puede eliminar mientras hay una impresión en curso.");
            }
            let rows: Vec<Item> = match &ids {
                None => db.prepare("SELECT * FROM items ORDER BY position,id")?.query_map([], Item::from_row)?.collect::<rusqlite::Result<_>>()?,
                Some(ids) => {
                    let mut v = Vec::new();
                    for &i in ids {
                        match db.query_row("SELECT * FROM items WHERE id=?", [i], Item::from_row).optional()? {
                            Some(r) => v.push(r),
                            None => return invalid("El PDF seleccionado no existe en la cola."),
                        }
                    }
                    v
                }
            };
            if rows.iter().any(|r| r.state == "uncertain" || r.state == "in_flight") {
                return invalid("Debe resolver los PDF con resultado incierto antes de eliminarlos.");
            }
            // Sources remain protected even after their queue rows were removed.
            let mut protected: HashSet<String> = HashSet::new();
            for s in db.prepare("SELECT source FROM items")?.query_map([], |r| r.get::<_, String>(0))? {
                protected.insert(key(&s?));
            }
            for d in db.prepare("SELECT detail FROM events WHERE action='add'")?.query_map([], |r| r.get::<_, String>(0))? {
                if let Ok(v) = serde_json::from_str::<Value>(&d?) {
                    if let Some(s) = v.get("source").and_then(Value::as_str) {
                        protected.insert(key(s));
                    }
                }
            }
            let mut owned: HashMap<i64, String> = HashMap::new();
            for r in db.prepare("SELECT item_id, path FROM owned_snapshots")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
                let (i, p) = r?;
                owned.insert(i, p);
            }
            for r in &rows {
                Self::event(db, r.id, action, json!({"previous": r}))?;
                db.execute("DELETE FROM items WHERE id=?", [r.id])?;
            }
            for s in db.prepare("SELECT snapshot FROM items")?.query_map([], |r| r.get::<_, String>(0))? {
                protected.insert(key(&s?));
            }
            Ok((rows, owned, protected))
        })?;
        // Committed. File cleanup is best effort and never fails the operation.
        let docs_ok = !is_symlink(&self.documents)
            && self.documents.canonicalize().map(plain).ok().as_deref() == Some(self.documents.as_path());
        if docs_ok {
            for r in &rows {
                self.unlink_owned(r, &owned, &protected);
            }
        }
        Ok(rows.len())
    }

    /// Untrusted DB paths are never deletion authority: only a regular file directly
    /// in `documents/`, with a UUID name, recorded by `add`, and not protected.
    fn unlink_owned(&self, row: &Item, owned: &HashMap<i64, String>, protected: &HashSet<String>) {
        if owned.get(&row.id) != Some(&row.snapshot) || row.snapshot.contains('\0') {
            return;
        }
        let path = Path::new(&row.snapshot);
        if path.parent() != Some(self.documents.as_path()) || !is_uuid_pdf(path) {
            return;
        }
        if protected.contains(&key(&row.snapshot)) {
            return;
        }
        match fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_file() => {
                let _ = fs::remove_file(path);
            }
            _ => {}
        }
    }

    pub fn move_item(&self, item_id: i64, delta: i64) -> Result<bool> {
        self.tx(|db| {
            Self::ensure_idle(db)?;
            if Self::item(db, item_id)?.state != "pending" {
                return invalid("Only pending items can be moved");
            }
            let rows: Vec<Item> = db.prepare("SELECT * FROM items ORDER BY position,id")?.query_map([], Item::from_row)?.collect::<rusqlite::Result<_>>()?;
            let index = rows.iter().position(|r| r.id == item_id).unwrap();
            let (mut low, mut high) = (index, index);
            while low > 0 && rows[low - 1].state == "pending" {
                low -= 1;
            }
            while high + 1 < rows.len() && rows[high + 1].state == "pending" {
                high += 1;
            }
            let target = (index as i64 + delta).clamp(low as i64, high as i64) as usize;
            if target == index {
                return Ok(false);
            }
            let positions: Vec<i64> = rows[low..=high].iter().map(|r| r.position).collect();
            let mut seg: Vec<i64> = rows[low..=high].iter().map(|r| r.id).collect();
            let moved = seg.remove(index - low);
            seg.insert(target - low, moved);
            for (p, id) in positions.iter().zip(&seg) {
                db.execute("UPDATE items SET position=? WHERE id=?", params![p, id])?;
            }
            Self::event(db, item_id, "move", json!({"delta": delta, "position": positions[target - low]}))?;
            Ok(true)
        })
    }
}

fn dedup(ids: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::new();
    ids.iter().copied().filter(|i| seen.insert(*i)).collect()
}

impl QueueStore {
    /// After the data folder was moved from `old` to `new`, point stored copy paths
    /// at the new location. Audit events keep their original text untouched.
    pub fn rebase(&self, old: &Path, new: &Path) -> Result<usize> {
        let (old, new) = (old.to_string_lossy().into_owned(), new.to_string_lossy().into_owned());
        self.tx(|db| {
            let mut n = 0;
            for table_col in [("items", "snapshot"), ("owned_snapshots", "path")] {
                let (t, c) = table_col;
                n += db.execute(
                    &format!("UPDATE {t} SET {c} = ?2 || substr({c}, length(?1) + 1) WHERE substr({c}, 1, length(?1)) = ?1"),
                    params![old, new],
                )?;
            }
            Ok(n)
        })
    }
}

/// Page to show the operator when reviewing an interruption: the selected item's
/// last recorded page, else the nearest earlier `sent` item's. Read-only.
pub fn recovery_reference(items: &[Item], item_id: i64) -> Option<(Item, i64)> {
    let mut sorted: Vec<&Item> = items.iter().collect();
    sorted.sort_by_key(|i| (i.position, i.id));
    let mut previous = None;
    for item in sorted {
        let mut page = item.last_page.max(0);
        if let Some(t) = item.total_pages.filter(|t| *t > 0) {
            page = page.min(t);
        }
        let reference = (page > 0).then(|| (item.clone(), page));
        if item.id == item_id {
            return reference.or(previous);
        }
        if item.state == "sent" && reference.is_some() {
            previous = reference;
        }
    }
    None
}
