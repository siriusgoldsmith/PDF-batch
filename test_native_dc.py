import sys
import unittest

@unittest.skipUnless(sys.platform=='win32','Windows GDI only')
class NativeDCTests(unittest.TestCase):
    def test_native_gdi_uses_pointer_sized_handles(self):
        import ctypes
        import native_dc
        self.assertEqual(native_dc.gdi.CreateDCW.restype,ctypes.c_void_p)
        dc=native_dc.CreateDC()
        dc.handle=native_dc.gdi.CreateDCW('DISPLAY',None,None,None)
        self.assertTrue(dc.GetSafeHdc())
        self.assertGreater(dc.GetDeviceCaps(8),0)
        dc.DeleteDC()
        self.assertIsNone(dc.handle)

if __name__=='__main__':unittest.main()
