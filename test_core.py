import hashlib
from pathlib import Path
import tempfile
import unittest

from core import QueueStore


class QueueTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.db = self.root / 'journal' / 'queue.sqlite'
        self.store = QueueStore(self.db)

    def pdf(self, name, content=b'%PDF-1.7\nbody\n%%EOF'):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def test_add_250_preserves_order_duplicates_unicode_and_snapshots(self):
        paths = [self.pdf(f'ñ carpeta/{i:03}.pdf', f'%PDF-1.7\n{i}'.encode()) for i in range(250)]
        paths.append(paths[0])
        self.store.add(paths)
        rows = QueueStore(self.db).list_items()
        self.assertEqual([r['source'] for r in rows], list(map(str, paths)))
        self.assertEqual([r['position'] for r in rows], list(range(1, 252)))
        self.assertEqual(len({r['snapshot'] for r in rows}), 251)
        for row, source in zip(rows, paths):
            self.assertEqual(row['sha256'], hashlib.sha256(source.read_bytes()).hexdigest())
            self.assertEqual(Path(row['snapshot']).read_bytes(), source.read_bytes())
            self.assertEqual(row['state'], 'pending')
            self.assertEqual(row['start_page'], 1)
        paths[0].unlink()
        self.assertTrue(Path(rows[0]['snapshot']).is_file())

    def test_invalid_batch_rolls_back_rows_and_snapshots(self):
        self.store.add([self.pdf('existing.pdf')])
        before = set(self.store.documents.iterdir())
        for invalid in [self.pdf('bad.pdf', b'not pdf'), self.pdf('wrong.txt'), self.root / 'absent.pdf']:
            with self.assertRaises((ValueError, OSError)):
                self.store.add([self.pdf('valid.pdf'), invalid])
            self.assertEqual(len(self.store.list_items()), 1)
            self.assertEqual(set(self.store.documents.iterdir()), before)

    def test_claim_progress_sent_are_durable_and_ordered(self):
        ids = self.store.add([self.pdf('a.pdf'), self.pdf('b.pdf')])
        self.assertEqual(self.store.pending()['id'], ids[0])
        with self.assertRaises(ValueError):
            self.store.claim(ids[1], 'Printer')
        self.store.claim(ids[0], 'Printer')
        self.assertIsNone(self.store.pending())
        self.store.progress(ids[0], 3, total_pages=10, job_id=42)
        row = QueueStore(self.db).list_items()[0]
        self.assertEqual((row['state'], row['printer'], row['last_page'], row['total_pages'], row['job_id']),
                         ('in_flight', 'Printer', 3, 10, '42'))
        self.store.mark_sent(ids[0])
        self.assertEqual(self.store.pending()['id'], ids[1])
        self.assertEqual(self.store.list_items()[0]['state'], 'sent')

    def test_error_101_restart_never_resends_prior_jobs(self):
        ids = self.store.add([self.pdf(f'{i}.pdf') for i in range(250)])
        dispatched = []
        for item_id in ids[:100]:
            self.store.claim(item_id, 'P')
            dispatched.append(item_id)
            self.store.mark_sent(item_id)
        self.store.claim(ids[100], 'P')
        self.store.progress(ids[100], 4, 12, 'job101')
        self.store.fail(ids[100], 'connection lost')
        store = QueueStore(self.db)
        self.assertEqual(store.recover(), 0)
        self.assertIsNone(store.pending())
        self.assertEqual(store.list_items()[100]['last_page'], 4)
        store.resolve(ids[100], 'retry', start_page=5)
        self.assertEqual(store.pending()['start_page'], 5)
        store.claim(ids[100], 'P')
        store.mark_sent(ids[100])
        self.assertEqual(store.pending()['id'], ids[101])
        self.assertTrue(all(r['state'] == 'sent' for r in store.list_items()[:101]))
        self.assertEqual(len(set(dispatched)), 100)

    def test_crash_recovery_requires_explicit_resolution(self):
        ids = self.store.add([self.pdf('a.pdf'), self.pdf('b.pdf')])
        self.store.claim(ids[0], 'P')
        store = QueueStore(self.db)
        self.assertEqual(store.recover(), 1)
        self.assertEqual(store.recover(), 0)
        self.assertIsNone(store.pending())
        self.assertEqual(store.list_items()[0]['state'], 'uncertain')
        with self.assertRaises(ValueError):
            store.resolve(ids[0], 'retry', 0)
        with self.assertRaises(ValueError):
            store.resolve(ids[0], 'bogus')
        store.resolve(ids[0], 'sent')
        store.claim(ids[1], 'P')
        store.fail(ids[1], 'interrupted')
        store.resolve(ids[1], 'skip')
        self.assertIsNone(store.pending())
        self.assertEqual(store.list_items()[1]['state'], 'skipped')

    def test_tampered_or_missing_snapshot_blocks_dispatch_durably(self):
        for missing in (False, True):
            with self.subTest(missing=missing):
                store = QueueStore(self.root / str(missing) / 'q.sqlite')
                item_id = store.add([self.pdf('source.pdf')])[0]
                snapshot = Path(store.pending()['snapshot'])
                if missing:
                    snapshot.unlink()
                else:
                    snapshot.write_bytes(b'%PDF-1.7\nmodified')
                with self.assertRaises(ValueError):
                    store.claim(item_id, 'P')
                self.assertEqual(store.list_items()[0]['state'], 'uncertain')
                self.assertIsNone(store.pending())

    def test_audit_records_lifecycle_with_deleted_rows_retained(self):
        item_id = self.store.add([self.pdf('audit.pdf')])[0]
        self.store.claim(item_id, 'P')
        self.store.progress(item_id, 1, 2, 'job')
        self.store.recover()
        self.store.resolve(item_id, 'retry', 2)
        self.store.claim(item_id, 'P')
        self.store.fail(item_id, 'offline')
        self.store.resolve(item_id, 'sent')
        events = self.store.events(item_id)
        self.assertEqual([e['action'] for e in events],
                         ['add', 'claim', 'progress', 'recover', 'resolve_retry', 'claim', 'fail', 'resolve_sent'])
        self.assertTrue(all(e['created_at'] and e['item_id'] == item_id for e in events))
        self.assertIn('offline', str(events))

    def test_safe_pending_move_delete_and_stable_audit_ids(self):
        ids = self.store.add([self.pdf(f'{i}.pdf') for i in range(4)])
        self.assertTrue(self.store.move(ids[2], -1))
        self.assertEqual([r['id'] for r in self.store.list_items()], [ids[0], ids[2], ids[1], ids[3]])
        self.store.claim(ids[0], 'P')
        with self.assertRaises(ValueError):
            self.store.delete_pending([ids[3]])
        with self.assertRaises(ValueError):
            self.store.move(ids[2], 1)
        self.store.fail(ids[0], 'error')
        with self.assertRaises(ValueError):
            self.store.delete_pending([ids[0], ids[1]])
        with self.assertRaises(ValueError):
            self.store.move(ids[0], 1)
        # Pending rows cannot be moved ahead of an uncertainty barrier.
        self.store.move(ids[1], -100)
        self.assertEqual(self.store.list_items()[0]['id'], ids[0])
        self.assertIsNone(self.store.pending())
        self.assertEqual(self.store.delete_pending(ids[1:]), 3)
        self.assertEqual(len(self.store.list_items()), 1)
        self.assertEqual(self.store.events(ids[3])[-1]['action'], 'delete')
        new_id = self.store.add([self.pdf('new.pdf')])[0]
        self.assertGreater(new_id, max(ids))

    def test_progress_rejects_invalid_or_regressing_checkpoints(self):
        item_id = self.store.add([self.pdf('pages.pdf')])[0]
        with self.assertRaises(ValueError):
            self.store.progress(item_id, 1)
        self.store.claim(item_id, 'P')
        self.store.progress(item_id, 3, 10)
        for last, total in [(2, None), (-1, None), (11, None), (3.5, None), (3, 2), (3, 0)]:
            with self.subTest(last=last, total=total), self.assertRaises(ValueError):
                self.store.progress(item_id, last, total)
        self.assertEqual(self.store.list_items()[0]['last_page'], 3)

    def test_concurrent_claim_has_exactly_one_winner(self):
        from concurrent.futures import ThreadPoolExecutor
        item_id = self.store.add([self.pdf('race.pdf')])[0]
        def attempt(_):
            try:
                self.store.claim(item_id, 'P')
                return True
            except ValueError:
                return False
        with ThreadPoolExecutor(max_workers=8) as pool:
            outcomes = list(pool.map(attempt, range(8)))
        self.assertEqual(outcomes.count(True), 1)
        self.assertEqual(QueueStore(self.db).list_items()[0]['state'], 'in_flight')

    def test_snapshot_is_fsynced_before_add_returns(self):
        import os
        from unittest.mock import patch
        with patch('core.os.fsync', wraps=os.fsync) as sync:
            self.store.add([self.pdf('durable.pdf')])
        self.assertGreaterEqual(sync.call_count, 1)

    def test_database_failure_rolls_back_entire_add_and_snapshot_files(self):
        import sqlite3
        from contextlib import closing
        with closing(sqlite3.connect(self.db)) as db:
            with db:
                db.execute("CREATE TRIGGER reject_second BEFORE INSERT ON items WHEN NEW.name='reject.pdf' BEGIN SELECT RAISE(ABORT, 'forced insert error'); END")
        with self.assertRaises(sqlite3.IntegrityError):
            self.store.add([self.pdf('ok.pdf'), self.pdf('reject.pdf')])
        self.assertEqual(self.store.list_items(), [])
        self.assertEqual(self.store.events(), [])
        self.assertEqual(list(self.store.documents.iterdir()), [])


if __name__ == '__main__':
    unittest.main()
