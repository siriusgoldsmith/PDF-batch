"""Durable, ordered PDF dispatch journal. 'sent' never means physically printed."""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import uuid
from contextlib import contextmanager


class QueueStore:
    def __init__(self, db_path):
        self.db_path = str(Path(db_path).resolve())
        Path(self.db_path).parent.mkdir(parents=True, exist_ok=True)
        self.documents = Path(self.db_path).parent / 'documents'
        self.documents.mkdir(exist_ok=True)
        with self._connection() as db:
            db.execute('''CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY AUTOINCREMENT, position INTEGER NOT NULL,
                source TEXT NOT NULL, name TEXT NOT NULL, snapshot TEXT NOT NULL,
                sha256 TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending'
                CHECK(state IN ('pending','in_flight','sent','uncertain','skipped')),
                job_id TEXT, last_page INTEGER NOT NULL DEFAULT 0,
                total_pages INTEGER, error TEXT, printer TEXT,
                start_page INTEGER NOT NULL DEFAULT 1 CHECK(start_page >= 1))''')
            columns = {row['name'] for row in db.execute('PRAGMA table_info(items)')}
            for name, definition in (
                ('queue_outcome', "TEXT NOT NULL DEFAULT 'unrecorded' CHECK(queue_outcome IN ('unrecorded','completed','review'))"),
                ('physical_status', "TEXT NOT NULL DEFAULT 'unconfirmed' CHECK(physical_status IN ('unconfirmed','confirmed','incomplete'))"),
                ('physical_confirmed_at', 'TEXT'),
            ):
                if name not in columns:
                    db.execute(f'ALTER TABLE items ADD COLUMN {name} {definition}')
            db.execute('''CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY, item_id INTEGER NOT NULL, action TEXT NOT NULL,
                detail TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT
                (strftime('%Y-%m-%dT%H:%M:%fZ','now')))''')

    @staticmethod
    def _event(db, item_id, action, **detail):
        db.execute('INSERT INTO events(item_id,action,detail) VALUES(?,?,?)',
                   (item_id, action, json.dumps(detail, ensure_ascii=False)))

    def events(self, item_id=None):
        with self._connection() as db:
            if item_id is None:
                rows = db.execute('SELECT * FROM events ORDER BY id')
            else:
                rows = db.execute('SELECT * FROM events WHERE item_id=? ORDER BY id', (item_id,))
            return [dict(row) for row in rows]

    @contextmanager
    def _connection(self):
        db = sqlite3.connect(self.db_path, timeout=30)
        db.row_factory = sqlite3.Row
        db.execute('PRAGMA synchronous=FULL')
        try:
            db.execute('BEGIN IMMEDIATE')
            yield db
            db.commit()
        except BaseException:
            db.rollback()
            raise
        finally:
            db.close()

    def add(self, paths):
        created = []
        ids = []
        try:
            with self._connection() as db:
                position = db.execute('SELECT COALESCE(MAX(position),0) FROM items').fetchone()[0]
                for source in paths:
                    source = Path(source).resolve()
                    if source.suffix.lower() != '.pdf':
                        raise ValueError('Only .pdf files are accepted')
                    with source.open('rb') as check:
                        if check.read(5) != b'%PDF-':
                            raise ValueError('Invalid PDF signature: ' + str(source))
                    snapshot = self.documents / (uuid.uuid4().hex + '.pdf')
                    created.append(snapshot)
                    digest = hashlib.sha256()
                    with source.open('rb') as src, snapshot.open('xb') as dst:
                        while chunk := src.read(1024 * 1024):
                            digest.update(chunk)
                            dst.write(chunk)
                        dst.flush()
                        os.fsync(dst.fileno())
                    position += 1
                    cursor = db.execute('''INSERT INTO items(position,source,name,snapshot,sha256)
                        VALUES(?,?,?,?,?)''', (position, str(source), source.name, str(snapshot), digest.hexdigest()))
                    ids.append(cursor.lastrowid)
                    self._event(db, cursor.lastrowid, 'add', source=str(source),
                                snapshot=str(snapshot), sha256=digest.hexdigest())
            return ids
        except BaseException:
            for snapshot in created:
                snapshot.unlink(missing_ok=True)
            raise

    @staticmethod
    def _first(db):
        return db.execute("SELECT * FROM items WHERE state NOT IN ('sent','skipped') ORDER BY position,id LIMIT 1").fetchone()

    @staticmethod
    def _item(db, item_id):
        row = db.execute('SELECT * FROM items WHERE id=?', (item_id,)).fetchone()
        if row is None:
            raise ValueError('Unknown item')
        return row

    def pending(self):
        with self._connection() as db:
            row = self._first(db)
            return dict(row) if row is not None and row['state'] == 'pending' else None

    def claim(self, item_id, printer):
        problem = None
        with self._connection() as db:
            row = self._first(db)
            if row is None or row['id'] != item_id or row['state'] != 'pending':
                raise ValueError('Only the first pending item can be claimed')
            db.execute("""UPDATE items SET queue_outcome='unrecorded',physical_status='unconfirmed',
                       physical_confirmed_at=NULL WHERE id=?""", (item_id,))
            try:
                digest = hashlib.sha256()
                with open(row['snapshot'], 'rb') as snapshot:
                    while chunk := snapshot.read(1024 * 1024):
                        digest.update(chunk)
                if digest.hexdigest() != row['sha256']:
                    problem = 'Snapshot SHA256 mismatch'
            except OSError as exc:
                problem = 'Snapshot unavailable: ' + str(exc)
            if problem:
                db.execute("UPDATE items SET state='uncertain',error=? WHERE id=?", (problem, item_id))
                self._event(db, item_id, 'integrity_error', error=problem)
            else:
                db.execute("UPDATE items SET state='in_flight',printer=?,error=NULL WHERE id=?", (printer, item_id))
                self._event(db, item_id, 'claim', printer=printer, start_page=row['start_page'])
            result = dict(self._item(db, item_id))
        if problem:
            raise ValueError(problem)
        return result

    def progress(self, item_id, last_page, total_pages=None, job_id=None):
        with self._connection() as db:
            row = self._item(db, item_id)
            if row['state'] != 'in_flight':
                raise ValueError('Progress requires in_flight state')
            total = row['total_pages'] if total_pages is None else total_pages
            if type(last_page) is not int or last_page < row['last_page'] or last_page < 0:
                raise ValueError('last_page must be a non-regressing integer')
            if total is not None and (type(total) is not int or total < 1 or last_page > total):
                raise ValueError('Invalid total_pages or last_page exceeds total')
            db.execute('''UPDATE items SET last_page=?,total_pages=COALESCE(?,total_pages),
                       job_id=COALESCE(?,job_id) WHERE id=?''', (last_page, total_pages, job_id, item_id))
            self._event(db, item_id, 'progress', last_page=last_page, total_pages=total_pages, job_id=job_id)

    def mark_sent(self, item_id):
        with self._connection() as db:
            if self._item(db, item_id)['state'] != 'in_flight':
                raise ValueError('Completion requires in_flight state')
            db.execute("UPDATE items SET state='sent',queue_outcome='completed',error=NULL WHERE id=?", (item_id,))
            self._event(db, item_id, 'sent')

    def fail(self, item_id, error):
        with self._connection() as db:
            if self._item(db, item_id)['state'] != 'in_flight':
                raise ValueError('Failure requires in_flight state')
            db.execute("UPDATE items SET state='uncertain',queue_outcome='review',error=? WHERE id=?", (str(error), item_id))
            self._event(db, item_id, 'fail', error=str(error))

    def recover(self):
        """Call once at application startup, never while a worker is running."""
        with self._connection() as db:
            ids = [r[0] for r in db.execute("SELECT id FROM items WHERE state='in_flight'")]
            db.execute("UPDATE items SET state='uncertain',queue_outcome='review',error='Interrupted while in flight; operator review required' WHERE state='in_flight'")
            for item_id in ids:
                self._event(db, item_id, 'recover')
            return len(ids)

    def confirm_physical(self, item_id, printed: bool):
        """Record operator-observed paper output, never infer Windows completion."""
        if type(printed) is not bool:
            raise ValueError('printed must be a bool')
        with self._connection() as db:
            self._ensure_idle(db)
            row = self._item(db, item_id)
            if row['state'] not in ('sent', 'uncertain'):
                raise ValueError('Physical confirmation requires sent or uncertain state')
            db.execute("""UPDATE items SET state=?,physical_status=?,
                       physical_confirmed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                       error=? WHERE id=?""",
                       ('sent' if printed else 'uncertain',
                        'confirmed' if printed else 'incomplete',
                        None if printed else 'Impresión física incompleta. Revise las hojas y resuelva antes de continuar.',
                        item_id))
            self._event(db, item_id, 'physical_confirmed' if printed else 'physical_incomplete',
                        previous=dict(row))

    def resolve(self, item_id, action, start_page=1):
        if action not in ('retry', 'sent', 'skip'):
            raise ValueError('Unknown resolution')
        if type(start_page) is not int or start_page < 1:
            raise ValueError('start_page must be a positive integer')
        with self._connection() as db:
            if self._item(db, item_id)['state'] != 'uncertain':
                raise ValueError('Only uncertain items can be resolved')
            if action == 'retry':
                db.execute("""UPDATE items SET state='pending',start_page=?,last_page=?,job_id=NULL,error=NULL,
                           queue_outcome='unrecorded',physical_status='unconfirmed',physical_confirmed_at=NULL WHERE id=?""",
                           (start_page, start_page - 1, item_id))
            elif action == 'sent':
                db.execute("""UPDATE items SET state='sent',error=NULL,physical_status='confirmed',
                           physical_confirmed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?""",
                           (item_id,))
            else:
                db.execute("""UPDATE items SET state='skipped',error=NULL,
                           physical_status='unconfirmed',physical_confirmed_at=NULL WHERE id=?""", (item_id,))
            self._event(db, item_id, 'resolve_' + action, start_page=start_page)

    @staticmethod
    def _ensure_idle(db):
        if db.execute("SELECT 1 FROM items WHERE state='in_flight' LIMIT 1").fetchone():
            raise ValueError('Cannot edit queue while an item is in flight')

    def delete_pending(self, ids):
        ids = list(dict.fromkeys(ids))
        with self._connection() as db:
            self._ensure_idle(db)
            rows = [self._item(db, item_id) for item_id in ids]
            if any(row['state'] != 'pending' for row in rows):
                raise ValueError('Only pending items can be deleted')
            for row in rows:
                self._event(db, row['id'], 'delete', snapshot=row['snapshot'])
                db.execute('DELETE FROM items WHERE id=?', (row['id'],))
        # Keep snapshots as audit evidence; originals are never touched.
        return len(ids)

    def delete_items(self, ids):
        """Explicitly remove selected rows, retaining their complete audit history."""
        try:
            ids = list(ids)
        except TypeError:
            raise ValueError('Debe indicar una lista de identificadores de PDF.') from None
        if any(type(item_id) is not int or item_id < 1 for item_id in ids):
            raise ValueError('Cada identificador debe ser un entero positivo.')
        return self._remove_items(list(dict.fromkeys(ids)), 'delete')

    def clear_items(self):
        """Remove the entire idle queue only after all uncertain work is resolved."""
        return self._remove_items(None, 'clear')

    def _remove_items(self, ids, action):
        with self._connection() as db:
            try:
                self._ensure_idle(db)
            except ValueError:
                raise ValueError('No se puede eliminar mientras hay una impresión en curso.') from None
            rows = []
            if ids is None:
                rows = list(db.execute('SELECT * FROM items ORDER BY position,id'))
            else:
                for item_id in ids:
                    row = db.execute('SELECT * FROM items WHERE id=?', (item_id,)).fetchone()
                    if row is None:
                        raise ValueError('El PDF seleccionado no existe en la cola.')
                    rows.append(row)
            if any(row['state'] in ('uncertain', 'in_flight') for row in rows):
                raise ValueError('Debe resolver los PDF con resultado incierto antes de eliminarlos.')
            # Sources remain protected even after their queue rows were removed.
            protected = {r['source'] for r in db.execute('SELECT source FROM items')}
            owned = {}
            legacy = {}
            for event in db.execute("SELECT item_id,detail FROM events WHERE action='add'"):
                detail = json.loads(event['detail'])
                if detail.get('source'):
                    protected.add(detail['source'])
                if detail.get('snapshot'):
                    owned[event['item_id']] = detail['snapshot']
                else:
                    legacy[event['item_id']] = detail
            for row in rows:
                self._event(db, row['id'], action, previous=dict(row))
                db.execute('DELETE FROM items WHERE id=?', (row['id'],))
            protected.update(r['snapshot'] for r in db.execute('SELECT snapshot FROM items'))
        for row in rows:
            detail=legacy.get(row['id'])
            if detail and detail.get('source')==row['source'] and detail.get('sha256')==row['sha256']:
                # Original v1.3 add events did not record the generated filename.
                # Admit only a direct UUID copy whose bytes still match its add audit.
                try:
                    path=Path(row['snapshot'])
                    valid_name=path.suffix=='.pdf' and len(path.stem)==32 and uuid.UUID(hex=path.stem).hex==path.stem
                    if valid_name and path.parent==self.documents and not path.is_symlink() and not self.documents.is_symlink():
                        digest=hashlib.sha256()
                        with path.open('rb') as stream:
                            while chunk:=stream.read(1024*1024):digest.update(chunk)
                        if digest.hexdigest()==row['sha256']:owned[row['id']]=str(path)
                except (OSError,ValueError,RuntimeError):
                    pass
            self._unlink_owned_snapshot(row, owned, protected)
        return len(rows)

    def _unlink_owned_snapshot(self, row, owned, protected):
        """Best-effort cleanup; untrusted DB paths are never deletion authority."""
        try:
            path = Path(row['snapshot'])
            root = self.documents
            # Only add()'s recorded copies qualify; unknown legacy paths are retained.
            if owned.get(row['id']) != str(path):
                return
            if root.is_symlink() or root.resolve() != root or path.parent != root:
                return
            if path.is_symlink() or not path.is_file():
                return
            if path.resolve() in {Path(p).resolve() for p in protected}:
                return
            path.unlink(missing_ok=True)
        except (OSError, ValueError, RuntimeError):
            # The row deletion is already committed; a locked or missing copy must
            # not make callers believe the queue transaction failed.
            pass

    def move(self, item_id, delta):
        if type(delta) is not int:
            raise ValueError('delta must be an integer')
        with self._connection() as db:
            self._ensure_idle(db)
            row = self._item(db, item_id)
            if row['state'] != 'pending':
                raise ValueError('Only pending items can be moved')
            # Reorder only the contiguous pending segment; never cross barriers.
            rows = list(db.execute('SELECT * FROM items ORDER BY position,id'))
            index = next(i for i, r in enumerate(rows) if r['id'] == item_id)
            low = high = index
            while low > 0 and rows[low - 1]['state'] == 'pending':
                low -= 1
            while high + 1 < len(rows) and rows[high + 1]['state'] == 'pending':
                high += 1
            target = max(low, min(high, index + delta))
            if target == index:
                return False
            positions = [r['position'] for r in rows[low:high + 1]]
            segment = [r['id'] for r in rows[low:high + 1]]
            segment.insert(target - low, segment.pop(index - low))
            db.executemany('UPDATE items SET position=? WHERE id=?', zip(positions, segment))
            self._event(db, item_id, 'move', delta=delta, position=positions[target - low])
            return True

    def list_items(self):
        with self._connection() as db:
            return [dict(row) for row in db.execute('SELECT * FROM items ORDER BY position,id')]
