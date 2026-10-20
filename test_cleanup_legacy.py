import json
from pathlib import Path
import tempfile
import unittest
from core import QueueStore


class LegacyCleanupTests(unittest.TestCase):
    def test_clear_removes_verified_v13_legacy_copies_without_snapshot_in_add_event(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            source=root/'original.pdf';source.write_bytes(b'%PDF-1.7\nlegacy')
            store=QueueStore(root/'data'/'queue.sqlite3')
            item_id=store.add([source])[0]
            snapshot=Path(store.list_items()[0]['snapshot'])
            with store._connection() as db:
                event=db.execute("SELECT * FROM events WHERE action='add'").fetchone()
                detail=json.loads(event['detail']);detail.pop('snapshot')
                db.execute('UPDATE events SET detail=? WHERE id=?',(json.dumps(detail),event['id']))
            reopened=QueueStore(store.db_path)
            self.assertEqual(reopened.clear_items(),1)
            self.assertFalse(snapshot.exists(),'Vaciar debe limpiar también las copias de la versión anterior')
            self.assertTrue(source.exists())


if __name__=='__main__':unittest.main()
