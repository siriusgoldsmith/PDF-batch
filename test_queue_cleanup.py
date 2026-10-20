"""Explicit queue cleanup removes app copies, never original PDFs."""
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from core import QueueStore


class QueueCleanupTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.db = self.root / 'journal' / 'queue.sqlite'
        self.store = QueueStore(self.db)

    def add(self, count=1):
        sources = []
        for i in range(count):
            source = self.root / f'{i}.pdf'
            source.write_bytes(b'%PDF-1.7\noriginal')
            sources.append(source)
        return self.store.add(sources)

    def test_delete_deduplicates_all_resolved_states_and_preserves_full_audit(self):
        ids = self.add(3)
        self.store.claim(ids[0], 'Impresora')
        self.store.progress(ids[0], 2, 5, 'job-1')
        self.store.mark_sent(ids[0])
        self.store.confirm_physical(ids[0], True)
        self.store.claim(ids[1], 'Impresora')
        self.store.fail(ids[1], 'sin conexión')
        self.store.resolve(ids[1], 'skip')
        before = self.store.list_items()
        events = self.store.events()
        self.assertEqual(self.store.delete_items(ids + ids), 3)
        reopened = QueueStore(self.db)
        self.assertEqual(reopened.list_items(), [])
        self.assertEqual(reopened.events()[:len(events)], events)
        for row in before:
            event = reopened.events(row['id'])[-1]
            self.assertEqual(event['action'], 'delete')
            self.assertEqual(json.loads(event['detail'])['previous'], row)
            self.assertEqual(Path(row['source']).read_bytes(), b'%PDF-1.7\noriginal')
        self.assertGreater(self.add()[0], max(ids))

    def test_delete_rejects_invalid_ids_atomically_in_spanish(self):
        first, second = self.add(2)
        before = self.store.list_items(), self.store.events()
        for invalid in (9999, None, '1', True, 1.0, [], -1):
            with self.subTest(invalid=invalid):
                with self.assertRaisesRegex(ValueError, 'identificador|existe'):
                    self.store.delete_items([second, invalid])
                self.assertEqual((self.store.list_items(), self.store.events()), before)
                self.assertTrue(all(Path(r['snapshot']).exists() for r in before[0]))

    def test_delete_rejects_unresolved_selection_but_allows_other_idle_rows(self):
        first, second = self.add(2)
        self.store.claim(first, 'P')
        self.store.fail(first, 'offline')
        before = self.store.list_items(), self.store.events()
        with self.assertRaisesRegex(ValueError, 'resolver|incierto'):
            self.store.delete_items([second, first])
        self.assertEqual((self.store.list_items(), self.store.events()), before)
        self.assertEqual(self.store.delete_items([second]), 1)
        self.assertEqual(self.store.list_items()[0]['id'], first)

    def test_delete_requires_global_idle_even_for_empty_selection(self):
        first, second = self.add(2)
        self.store.claim(first, 'P')
        before = self.store.list_items(), self.store.events()
        for ids in ([second], [first], []):
            with self.subTest(ids=ids), self.assertRaisesRegex(ValueError, 'curso'):
                self.store.delete_items(ids)
            self.assertEqual((self.store.list_items(), self.store.events()), before)

    def test_clear_is_atomic_with_unresolved_or_busy_rows_then_removes_all(self):
        first, second = self.add(2)
        self.store.claim(first, 'P')
        for state in ('in_flight', 'uncertain'):
            before = self.store.list_items(), self.store.events()
            with self.assertRaisesRegex(ValueError, 'curso|resolver|incierto'):
                self.store.clear_items()
            self.assertEqual((self.store.list_items(), self.store.events()), before)
            if state == 'in_flight':
                self.store.fail(first, 'offline')
        self.store.resolve(first, 'skip')
        before = self.store.list_items()
        self.assertEqual(self.store.clear_items(), 2)
        self.assertEqual(QueueStore(self.db).list_items(), [])
        for row in before:
            event = self.store.events(row['id'])[-1]
            self.assertEqual(event['action'], 'clear')
            self.assertEqual(json.loads(event['detail'])['previous'], row)
        self.assertEqual(self.store.clear_items(), 0)
        self.assertEqual(self.store.delete_items([]), 0)

    def test_explicit_cleanup_unlinks_copies_after_commit_and_tolerates_missing(self):
        ids = self.add(3)
        rows = self.store.list_items()
        Path(rows[0]['snapshot']).unlink()
        real_unlink = Path.unlink
        observed = []

        def check_committed(path, *args, **kwargs):
            observed.append(path)
            self.assertEqual(QueueStore(self.db).list_items(), [])
            self.assertEqual(len([e for e in self.store.events() if e['action'] == 'clear']), 3)
            return real_unlink(path, *args, **kwargs)

        with patch.object(Path, 'unlink', check_committed):
            self.assertEqual(self.store.clear_items(), 3)
        self.assertTrue(observed)
        self.assertTrue(all(not Path(r['snapshot']).exists() for r in rows))
        self.assertTrue(all(Path(r['source']).exists() for r in rows))
        item_id = self.add()[0]
        snapshot = Path(self.store.list_items()[0]['snapshot'])
        self.assertEqual(self.store.delete_items([item_id]), 1)
        self.assertFalse(snapshot.exists())

    def test_malicious_snapshot_paths_never_delete_originals_or_arbitrary_files(self):
        cases = ('outside', 'traversal', 'source', 'other_source', 'historical_source',
                 'symlink', 'nested_symlink', 'arbitrary_inside', 'shared_snapshot', 'nul')
        for case in cases:
            with self.subTest(case=case):
                store = QueueStore(self.root / ('case_' + case) / 'queue.sqlite')
                source = self.root / f'case_{case}.pdf'
                source.write_bytes(b'%PDF-1.7\noriginal')
                selected, other = store.add([source, source])
                outside = self.root / f'{case}-outside.pdf'
                outside.write_bytes(b'valuable')
                rows = store.list_items()
                if case == 'outside':
                    malicious = outside
                elif case == 'traversal':
                    malicious = store.documents / '..' / '..' / outside.name
                elif case == 'source':
                    malicious = source
                elif case in ('other_source', 'historical_source'):
                    # Even a UUID-shaped source inside documents is not an app copy.
                    malicious = store.documents / ('a' * 32 + '.pdf')
                    malicious.write_bytes(b'%PDF-1.7\noriginal inside')
                    protected = store.add([malicious])[0]
                    if case == 'historical_source':
                        store.delete_pending([protected])
                elif case == 'symlink':
                    malicious = store.documents / ('b' * 32 + '.pdf')
                    malicious.symlink_to(outside)
                    if not malicious.is_symlink():
                        self.skipTest('Host cannot create inspectable Windows symlinks; exercised on Linux')
                elif case == 'nested_symlink':
                    link = store.documents / 'link'
                    link.symlink_to(self.root, target_is_directory=True)
                    if not link.is_symlink():
                        self.skipTest('Host cannot create inspectable Windows symlinks; exercised on Linux')
                    malicious = link / outside.name
                elif case == 'arbitrary_inside':
                    malicious = store.documents / 'valuable.pdf'
                    malicious.write_bytes(b'valuable')
                elif case == 'shared_snapshot':
                    malicious = Path(rows[1]['snapshot'])
                else:
                    malicious = 'bad\x00path'
                with store._connection() as db:
                    db.execute('UPDATE items SET snapshot=? WHERE id=?', (str(malicious), selected))
                self.assertEqual(store.delete_items([selected]), 1)
                self.assertTrue(source.exists())
                self.assertTrue(outside.exists())
                self.assertTrue(Path(rows[1]['snapshot']).exists())
                if case not in ('nul',):
                    self.assertTrue(Path(malicious).exists())

    def test_cleanup_permission_error_returns_committed_count_and_continues(self):
        ids = self.add(2)
        rows = self.store.list_items()
        locked = Path(rows[0]['snapshot'])
        real_unlink = Path.unlink

        def unlink(path, *args, **kwargs):
            if path == locked:
                raise PermissionError('archivo ocupado')
            return real_unlink(path, *args, **kwargs)

        with patch.object(Path, 'unlink', unlink):
            self.assertEqual(self.store.delete_items(ids), 2)
        self.assertTrue(locked.exists())
        self.assertFalse(Path(rows[1]['snapshot']).exists())
        self.assertEqual(QueueStore(self.db).list_items(), [])
        self.assertEqual(len([e for e in self.store.events() if e['action'] == 'delete']), 2)

    def test_audit_failure_rolls_back_all_rows_and_leaves_copies(self):
        ids = self.add(2)
        before = self.store.list_items(), self.store.events()
        with self.store._connection() as db:
            db.execute(f'''CREATE TRIGGER reject_delete BEFORE INSERT ON events
                       WHEN NEW.item_id={ids[1]} AND NEW.action IN ('delete','clear')
                       BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END''')
        for operation in (lambda: self.store.delete_items(ids), self.store.clear_items):
            with self.assertRaises(sqlite3.IntegrityError):
                operation()
            self.assertEqual((self.store.list_items(), self.store.events()), before)
            self.assertTrue(all(Path(r['snapshot']).exists() for r in before[0]))

    def test_replaced_owned_snapshot_symlink_and_redirected_documents_are_retained(self):
        item_id = self.add()[0]
        row = self.store.list_items()[0]
        snapshot = Path(row['snapshot'])
        snapshot.unlink()
        snapshot.symlink_to(row['source'])
        if not snapshot.is_symlink():
            self.skipTest('Host cannot create inspectable Windows symlinks; exercised on Linux')
        self.assertEqual(self.store.delete_items([item_id]), 1)
        self.assertTrue(snapshot.is_symlink())
        self.assertTrue(Path(row['source']).exists())
        item_id = self.add()[0]
        row = self.store.list_items()[0]
        moved = self.store.documents.with_name('moved')
        self.store.documents.rename(moved)
        self.store.documents.symlink_to(moved, target_is_directory=True)
        self.assertEqual(self.store.clear_items(), 1)
        self.assertTrue(Path(row['snapshot']).exists())

    def test_owned_copy_used_as_another_source_is_preserved_on_clear(self):
        self.add()
        original_copy = Path(self.store.list_items()[0]['snapshot'])
        self.store.add([original_copy])
        self.assertEqual(self.store.clear_items(), 2)
        self.assertTrue(original_copy.exists())

    def test_unknown_uuid_file_inside_documents_is_not_an_owned_copy(self):
        item_id = self.add()[0]
        arbitrary = self.store.documents / ('c' * 32 + '.pdf')
        arbitrary.write_bytes(b'valuable')
        with self.store._connection() as db:
            db.execute('UPDATE items SET snapshot=? WHERE id=?', (str(arbitrary), item_id))
        self.assertEqual(self.store.clear_items(), 1)
        self.assertEqual(arbitrary.read_bytes(), b'valuable')

    def test_legacy_delete_pending_still_retains_snapshot(self):
        item_id = self.add()[0]
        snapshot = Path(self.store.list_items()[0]['snapshot'])
        self.assertEqual(self.store.delete_pending([item_id]), 1)
        self.assertTrue(snapshot.exists())

    def test_delete_rejects_non_iterable_selection_in_spanish(self):
        self.add()
        before = self.store.list_items(), self.store.events()
        for ids in (None, 42):
            with self.subTest(ids=ids), self.assertRaisesRegex(ValueError, 'identificador'):
                self.store.delete_items(ids)
            self.assertEqual((self.store.list_items(), self.store.events()), before)


if __name__ == '__main__':
    unittest.main()
