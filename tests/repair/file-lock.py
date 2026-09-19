"""Hold a real Windows sharing conflict on one file in an owned qbutt lab."""

import ctypes
from ctypes import wintypes
import json
import pathlib
import sys
import tempfile


root = pathlib.Path(sys.argv[1]).resolve()
path = pathlib.Path(sys.argv[2]).resolve()
temporary = pathlib.Path(tempfile.gettempdir()).resolve()
if (root.parent != temporary or not root.name.startswith("qbutt-storage-faults-")
        or not path.is_relative_to(root) or not path.is_file()):
    raise RuntimeError("An existing file inside the owned storage-fault lab is required")

kernel = ctypes.WinDLL("kernel32", use_last_error=True)
kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
                              wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
kernel.CreateFileW.restype = wintypes.HANDLE
kernel.CloseHandle.argtypes = [wintypes.HANDLE]
handle = kernel.CreateFileW(str(path), 0x80000000, 1, None, 3, 0x80, None)
if handle == ctypes.c_void_p(-1).value:
    raise ctypes.WinError(ctypes.get_last_error())
try:
    print(json.dumps({"ready": True}), flush=True)
    sys.stdin.readline()
finally:
    kernel.CloseHandle(handle)
