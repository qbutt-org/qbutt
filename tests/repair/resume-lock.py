"""Hold an actual native resume-storage write conflict in an owned temp lab."""

import ctypes
from ctypes import wintypes
import json
import pathlib
import sqlite3
import sys
import tempfile

backend, root_arg, path_arg = sys.argv[1:]
root = pathlib.Path(root_arg).resolve()
path = pathlib.Path(path_arg).resolve()
if (root.parent != pathlib.Path(tempfile.gettempdir()).resolve()
        or not root.name.startswith("qbutt-staging-receipt-") or not path.is_relative_to(root) or not path.is_file()):
    raise RuntimeError("An existing storage file inside the owned temporary lab is required")
if backend == "SQLite":
    database = sqlite3.connect(path, timeout=5)
    try:
        database.execute("BEGIN IMMEDIATE")
        print(json.dumps({"ready": True, "backend": backend}), flush=True)
        sys.stdin.readline()
    finally:
        database.rollback()
        database.close()
elif backend == "Legacy":
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
                                  wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.CreateFileW(str(path), 0x80000000, 1, None, 3, 0x80, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        print(json.dumps({"ready": True, "backend": backend}), flush=True)
        sys.stdin.readline()
    finally:
        kernel.CloseHandle(handle)
else:
    raise ValueError("Backend must be Legacy or SQLite")
