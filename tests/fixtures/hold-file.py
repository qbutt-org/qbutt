"""Hold a generated Windows fixture against replacement until stdin closes."""

import ctypes
import sys

kernel = ctypes.WinDLL("kernel32", use_last_error=True)
kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_ulong, ctypes.c_ulong,
                              ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_void_p]
kernel.CreateFileW.restype = ctypes.c_void_p
kernel.CloseHandle.argtypes = [ctypes.c_void_p]
handle = kernel.CreateFileW(sys.argv[1], 0x80000000, 1, None, 3, 128, None)
if handle == ctypes.c_void_p(-1).value:
    raise ctypes.WinError(ctypes.get_last_error())
try:
    print("held", flush=True)
    sys.stdin.read()
finally:
    kernel.CloseHandle(handle)
