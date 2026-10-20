"""Small ctypes GDI printer DC wrapper; no MFC/VC++ runtime required."""
import ctypes
from ctypes import wintypes

class DOCINFOW(ctypes.Structure):
    _fields_=[('cbSize',ctypes.c_int),('lpszDocName',wintypes.LPCWSTR),('lpszOutput',wintypes.LPCWSTR),('lpszDatatype',wintypes.LPCWSTR),('fwType',wintypes.DWORD)]

gdi=ctypes.WinDLL('gdi32',use_last_error=True)
gdi.CreateDCW.argtypes=[wintypes.LPCWSTR,wintypes.LPCWSTR,wintypes.LPCWSTR,ctypes.c_void_p]
gdi.CreateDCW.restype=ctypes.c_void_p
gdi.GetDeviceCaps.argtypes=[ctypes.c_void_p,ctypes.c_int]
gdi.GetDeviceCaps.restype=ctypes.c_int
gdi.StartDocW.argtypes=[ctypes.c_void_p,ctypes.POINTER(DOCINFOW)]
gdi.StartDocW.restype=ctypes.c_int
for name in ('StartPage','EndPage','EndDoc','AbortDoc','DeleteDC'):
    fn=getattr(gdi,name);fn.argtypes=[ctypes.c_void_p];fn.restype=ctypes.c_int

def checked(result,operation):
    if result<=0:
        code=ctypes.get_last_error()
        raise OSError(code,f'{operation}: fallo GDI de Windows (código {code}).')
    return result

class CreateDC:
    def __init__(self):self.handle=None
    def CreatePrinterDC(self,printer):
        self.handle=gdi.CreateDCW('WINSPOOL',printer,None,None)
        if not self.handle:raise ctypes.WinError(ctypes.get_last_error())
    def GetSafeHdc(self):return self.handle
    def GetDeviceCaps(self,index):return gdi.GetDeviceCaps(self.handle,index)
    def StartDoc(self,name):
        info=DOCINFOW(ctypes.sizeof(DOCINFOW),name,None,None,0)
        return checked(gdi.StartDocW(self.handle,ctypes.byref(info)),'StartDoc')
    def StartPage(self):return checked(gdi.StartPage(self.handle),'StartPage')
    def EndPage(self):return checked(gdi.EndPage(self.handle),'EndPage')
    def EndDoc(self):return checked(gdi.EndDoc(self.handle),'EndDoc')
    def AbortDoc(self):return checked(gdi.AbortDoc(self.handle),'AbortDoc')
    def DeleteDC(self):
        if self.handle:
            checked(gdi.DeleteDC(self.handle),'DeleteDC');self.handle=None
