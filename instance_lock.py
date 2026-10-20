"""Cross-session lifetime lock for one local PDF queue; OS releases on crash."""
import os
from pathlib import Path

class AlreadyRunning(RuntimeError):pass

class InstanceLock:
    def __init__(self,path):
        path=Path(path);path.parent.mkdir(parents=True,exist_ok=True)
        self.handle=None
        if os.name=='nt':
            import ctypes
            self.kernel=ctypes.WinDLL('kernel32',use_last_error=True)
            fn=self.kernel.CreateFileW
            fn.argtypes=[ctypes.c_wchar_p,ctypes.c_ulong,ctypes.c_ulong,ctypes.c_void_p,ctypes.c_ulong,ctypes.c_ulong,ctypes.c_void_p]
            fn.restype=ctypes.c_void_p
            handle=fn(str(path),0xC0000000,0,None,4,0x80,None)
            if handle==ctypes.c_void_p(-1).value:
                code=ctypes.get_last_error()
                if code in (32,33):raise AlreadyRunning('Este lote ya está abierto en otra ventana o sesión de Windows.')
                raise ctypes.WinError(code)
            self.handle=handle
        else:
            import fcntl
            handle=path.open('a+b')
            try:fcntl.flock(handle.fileno(),fcntl.LOCK_EX|fcntl.LOCK_NB)
            except BlockingIOError:
                handle.close();raise AlreadyRunning('Este lote ya está abierto en otro proceso.')
            self.handle=handle
    def close(self):
        if self.handle is None:return
        if os.name=='nt':
            import ctypes
            self.kernel.CloseHandle.argtypes=[ctypes.c_void_p]
            self.kernel.CloseHandle(self.handle)
        else:self.handle.close()
        self.handle=None
