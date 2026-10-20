import tempfile
from pathlib import Path
import unittest

class InstanceLockTests(unittest.TestCase):
    def test_same_database_cannot_be_owned_twice_and_releases_on_close(self):
        from instance_lock import InstanceLock, AlreadyRunning
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'queue.lock'
            first=InstanceLock(path)
            try:
                with self.assertRaises(AlreadyRunning):InstanceLock(path)
            finally:first.close()
            second=InstanceLock(path)
            second.close()

    def test_other_process_is_blocked_and_crash_releases_lock(self):
        import subprocess
        import sys
        from instance_lock import InstanceLock
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'queue.lock'
            first=InstanceLock(path)
            try:
                probe="from instance_lock import InstanceLock,AlreadyRunning; import sys\ntry: InstanceLock(sys.argv[1])\nexcept AlreadyRunning: sys.exit(23)\n"
                result=subprocess.run([sys.executable,'-c',probe,str(path)],capture_output=True,timeout=15)
                self.assertEqual(result.returncode,23,result.stderr)
            finally:first.close()
            crash="from instance_lock import InstanceLock; import sys,os; lock=InstanceLock(sys.argv[1]); os._exit(0)"
            result=subprocess.run([sys.executable,'-c',crash,str(path)],capture_output=True,timeout=15)
            self.assertEqual(result.returncode,0,result.stderr)
            second=InstanceLock(path);second.close()

if __name__=='__main__':unittest.main()
