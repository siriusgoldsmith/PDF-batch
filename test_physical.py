"""Physical paper confirmation is independent of Windows queue completion."""
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from contextlib import closing
from datetime import datetime, timezone

from core import QueueStore


class PhysicalTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.db = self.root / 'queue.sqlite'
        self.store = QueueStore(self.db)

    def add(self, count=1):
        paths = []
        for i in range(count):
            path = self.root / f'{i}.pdf'
            path.write_bytes(b'%PDF-1.7\nbody')
            paths.append(path)
        return self.store.add(paths)

    def row(self, item_id):
        return next(r for r in QueueStore(self.db).list_items() if r['id'] == item_id)

    def test_defaults_and_windows_completion_never_confirm_paper(self):
        item_id = self.add()[0]
        row = self.row(item_id)
        self.assertEqual(row['queue_outcome'], 'unrecorded')
        self.assertEqual(row['physical_status'], 'unconfirmed')
        self.assertIsNone(row['physical_confirmed_at'])
        self.store.claim(item_id, 'P')
        self.store.mark_sent(item_id)
        row = self.row(item_id)
        self.assertEqual(row['queue_outcome'], 'completed')
        self.assertEqual(row['physical_status'], 'unconfirmed')
        self.assertIsNone(row['physical_confirmed_at'])

    def test_real_legacy_database_migrates_idempotently_without_inventing_results(self):
        legacy = self.root / 'legacy.sqlite'
        with closing(sqlite3.connect(legacy)) as db, db:
            db.execute('''CREATE TABLE items (
                id INTEGER PRIMARY KEY AUTOINCREMENT, position INTEGER NOT NULL,
                source TEXT NOT NULL, name TEXT NOT NULL, snapshot TEXT NOT NULL,
                sha256 TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending'
                CHECK(state IN ('pending','in_flight','sent','uncertain','skipped')),
                job_id TEXT, last_page INTEGER NOT NULL DEFAULT 0,
                total_pages INTEGER, error TEXT, printer TEXT,
                start_page INTEGER NOT NULL DEFAULT 1 CHECK(start_page >= 1))''')
            for i, state in enumerate(('pending', 'in_flight', 'sent', 'uncertain', 'skipped'), 1):
                db.execute('INSERT INTO items(position,source,name,snapshot,sha256,state) VALUES(?,?,?,?,?,?)',
                           (i, 'old.pdf', 'old.pdf', 'old.pdf', 'hash', state))
        QueueStore(legacy)
        rows = QueueStore(legacy).list_items()
        self.assertEqual([r['state'] for r in rows], ['pending', 'in_flight', 'sent', 'uncertain', 'skipped'])
        for row in rows:
            self.assertEqual(row['queue_outcome'], 'unrecorded')
            self.assertEqual(row['physical_status'], 'unconfirmed')
            self.assertIsNone(row['physical_confirmed_at'])

    def test_failed_and_interrupted_attempts_record_review_without_paper_success(self):
        item_id = self.add()[0]
        self.store.claim(item_id, 'P')
        self.store.fail(item_id, 'offline')
        row = self.row(item_id)
        self.assertEqual(row['queue_outcome'], 'review')
        self.assertEqual(row['physical_status'], 'unconfirmed')
        self.assertIsNone(row['physical_confirmed_at'])
        self.store.resolve(item_id, 'retry')
        self.store.claim(item_id, 'P')
        # A fresh attempt must not borrow the prior failure's result.
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute("UPDATE items SET queue_outcome='unrecorded' WHERE id=?", (item_id,))
        self.assertEqual(self.store.recover(), 1)
        row = self.row(item_id)
        self.assertEqual(row['queue_outcome'], 'review')
        self.assertEqual(row['physical_status'], 'unconfirmed')
        self.assertIsNone(row['physical_confirmed_at'])

    def test_manual_confirmation_is_durable_and_preserves_queue_evidence(self):
        item_id = self.add()[0]
        self.store.claim(item_id, 'P')
        self.store.fail(item_id, 'offline')
        before = datetime.now(timezone.utc)
        self.store.confirm_physical(item_id, True)
        row = self.row(item_id)
        self.assertEqual((row['state'], row['physical_status'], row['queue_outcome']),
                         ('sent', 'confirmed', 'review'))
        stamp = datetime.fromisoformat(row['physical_confirmed_at'].replace('Z', '+00:00'))
        self.assertEqual(stamp.utcoffset().total_seconds(), 0)
        self.assertGreaterEqual(stamp, before.replace(microsecond=(before.microsecond // 1000) * 1000))
        self.assertIsNone(row['error'])
        event = QueueStore(self.db).events(item_id)[-1]
        self.assertEqual(event['action'], 'physical_confirmed')
        previous = json.loads(event['detail'])['previous']
        self.assertEqual(previous['state'], 'uncertain')
        self.assertEqual(previous['queue_outcome'], 'review')
        self.assertEqual(previous['physical_status'], 'unconfirmed')
        self.assertIsNone(previous['physical_confirmed_at'])

    def test_incomplete_revises_confirmation_and_blocks_later_pending_until_resolved(self):
        first, second = self.add(2)
        self.store.claim(first, 'P')
        self.store.mark_sent(first)
        self.store.confirm_physical(first, True)
        prior = self.row(first)
        self.store.confirm_physical(first, False)
        row = self.row(first)
        self.assertEqual((row['state'], row['physical_status'], row['queue_outcome']),
                         ('uncertain', 'incomplete', 'completed'))
        self.assertTrue(row['physical_confirmed_at'].endswith('Z'))
        self.assertIn('revis', row['error'].lower())
        self.assertIsNone(self.store.pending())
        with self.assertRaises(ValueError):
            self.store.claim(second, 'P')
        event = self.store.events(first)[-1]
        self.assertEqual(event['action'], 'physical_incomplete')
        self.assertEqual(json.loads(event['detail'])['previous'], prior)
        self.store.confirm_physical(first, True)
        self.assertEqual(self.store.pending()['id'], second)
        self.assertEqual(self.row(first)['queue_outcome'], 'completed')

    def test_confirmation_rejects_pending_inflight_skipped_unknown_and_global_busy(self):
        first, second = self.add(2)
        for printed in (True, False):
            with self.subTest(state='pending', printed=printed), self.assertRaises(ValueError):
                self.store.confirm_physical(first, printed)
        self.store.claim(first, 'P')
        with self.assertRaises(ValueError):
            self.store.confirm_physical(first, True)
        self.store.mark_sent(first)
        self.store.claim(second, 'P')
        before = self.store.list_items(), self.store.events()
        with self.assertRaises(ValueError):
            self.store.confirm_physical(first, False)
        self.assertEqual((self.store.list_items(), self.store.events()), before)
        self.store.fail(second, 'offline')
        self.store.resolve(second, 'skip')
        for item_id in (second, 9999):
            with self.subTest(item_id=item_id), self.assertRaises(ValueError):
                self.store.confirm_physical(item_id, True)

    def test_confirmation_requires_strict_bool_without_mutation(self):
        item_id = self.add()[0]
        self.store.claim(item_id, 'P')
        self.store.mark_sent(item_id)
        before = self.store.list_items(), self.store.events()
        for invalid in (None, 0, 1, '', 'false', [], {}, 1.0):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                self.store.confirm_physical(item_id, invalid)
            self.assertEqual((self.store.list_items(), self.store.events()), before)

    def test_resolve_sent_explicitly_confirms_paper_without_inventing_queue_completion(self):
        item_id = self.add()[0]
        self.store.claim(item_id, 'P')
        self.store.fail(item_id, 'offline')
        self.store.resolve(item_id, 'sent')
        row = self.row(item_id)
        self.assertEqual((row['state'], row['physical_status'], row['queue_outcome']),
                         ('sent', 'confirmed', 'review'))
        self.assertTrue(row['physical_confirmed_at'].endswith('Z'))

    def test_retry_and_skip_clear_physical_confirmation_without_dispatch(self):
        for action, expected in (('retry', 'pending'), ('skip', 'skipped')):
            with self.subTest(action=action):
                item_id = self.add()[0]
                self.store.claim(item_id, 'P')
                self.store.mark_sent(item_id)
                self.store.confirm_physical(item_id, False)
                self.store.resolve(item_id, action, start_page=3)
                row = self.row(item_id)
                self.assertEqual(row['state'], expected)
                self.assertEqual(row['physical_status'], 'unconfirmed')
                self.assertIsNone(row['physical_confirmed_at'])
                if action == 'retry':
                    self.assertEqual(row['queue_outcome'], 'unrecorded')
                    self.assertEqual(row['start_page'], 3)
                    self.assertEqual(row['last_page'], 2)
                    self.assertIsNone(row['job_id'])
                    # Explicitly retire it so the next subtest has no pending work.
                    self.store.claim(item_id, 'P')
                    self.store.mark_sent(item_id)

    def test_claim_clears_stale_attempt_confirmation_and_queue_result(self):
        item_id = self.add()[0]
        # Simulate a stale imported pending attempt; claim is a defensive boundary.
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute("""UPDATE items SET queue_outcome='completed',physical_status='confirmed',
                       physical_confirmed_at='2020-01-01T00:00:00Z' WHERE id=?""", (item_id,))
        row = self.store.claim(item_id, 'P')
        self.assertEqual(row['queue_outcome'], 'unrecorded')
        self.assertEqual(row['physical_status'], 'unconfirmed')
        self.assertIsNone(row['physical_confirmed_at'])
        self.assertEqual(self.row(item_id), row)

    def test_confirmation_and_audit_are_one_transaction(self):
        item_id = self.add()[0]
        self.store.claim(item_id, 'P')
        self.store.mark_sent(item_id)
        before = self.store.list_items(), self.store.events()
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute("""CREATE TRIGGER reject_confirmation BEFORE INSERT ON events
                       WHEN NEW.action LIKE 'physical_%'
                       BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END""")
        with self.assertRaises(sqlite3.IntegrityError):
            self.store.confirm_physical(item_id, False)
        self.assertEqual((self.store.list_items(), self.store.events()), before)

    def test_manual_confirmation_preserves_unrecorded_legacy_outcome(self):
        item_id = self.add()[0]
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute("UPDATE items SET state='sent' WHERE id=?", (item_id,))
        self.store.confirm_physical(item_id, True)
        self.assertEqual(self.row(item_id)['queue_outcome'], 'unrecorded')
        self.assertEqual(self.row(item_id)['physical_status'], 'confirmed')


if __name__ == '__main__':
    unittest.main()
