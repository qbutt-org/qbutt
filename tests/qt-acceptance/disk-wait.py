"""Hold a real Windows file operation, without replacing libtorrent disk I/O."""

import ctypes
from ctypes import wintypes
import json
from pathlib import Path
import sys
import tempfile
import threading
import time


class Overlapped(ctypes.Structure):
    _fields_ = [("Internal", ctypes.c_size_t), ("InternalHigh", ctypes.c_size_t),
                ("Offset", wintypes.DWORD), ("OffsetHigh", wintypes.DWORD),
                ("hEvent", wintypes.HANDLE)]


kernel = ctypes.WinDLL("kernel32", use_last_error=True)
kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                              ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
kernel.CreateFileW.restype = wintypes.HANDLE
kernel.CreateEventW.argtypes = [ctypes.c_void_p, wintypes.BOOL, wintypes.BOOL, wintypes.LPCWSTR]
kernel.CreateEventW.restype = wintypes.HANDLE
kernel.DeviceIoControl.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.c_void_p,
                                  wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD,
                                  ctypes.POINTER(wintypes.DWORD), ctypes.POINTER(Overlapped)]
kernel.DeviceIoControl.restype = wintypes.BOOL
kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
kernel.WaitForSingleObject.restype = wintypes.DWORD
kernel.GetOverlappedResult.argtypes = [wintypes.HANDLE, ctypes.POINTER(Overlapped),
                                     ctypes.POINTER(wintypes.DWORD), wintypes.BOOL]
kernel.GetOverlappedResult.restype = wintypes.BOOL
kernel.CancelIoEx.argtypes = [wintypes.HANDLE, ctypes.POINTER(Overlapped)]
kernel.CloseHandle.argtypes = [wintypes.HANDLE]
FSCTL_REQUEST_OPLOCK_LEVEL_1 = 0x00090000


class FileWait:
    def __init__(self, path):
        self.file = None
        self.requested = False
        self.operation = Overlapped()
        self.operation.hEvent = kernel.CreateEventW(None, True, False, None)
        if not self.operation.hEvent:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            # Shared access permits the native writer after the oplock is released.
            self.file = kernel.CreateFileW(str(path), 0xC0000000, 7, None, 3, 0x40000080, None)
            if self.file == ctypes.c_void_p(-1).value:
                self.file = None
                raise ctypes.WinError(ctypes.get_last_error())
            returned = wintypes.DWORD()
            result = kernel.DeviceIoControl(self.file, FSCTL_REQUEST_OPLOCK_LEVEL_1, None, 0, None, 0,
                                            ctypes.byref(returned), ctypes.byref(self.operation))
            if result or ctypes.get_last_error() != 997:  # ERROR_IO_PENDING = granted.
                raise RuntimeError(f"Level 1 oplock was not granted: {ctypes.get_last_error()}")
            self.requested = True
        except BaseException:
            self.close()
            raise

    def broken(self):
        result = kernel.WaitForSingleObject(self.operation.hEvent, 0)
        if result == 258:  # WAIT_TIMEOUT
            return False
        if result != 0:
            raise ctypes.WinError(ctypes.get_last_error())
        transferred = wintypes.DWORD()
        if not kernel.GetOverlappedResult(self.file, ctypes.byref(self.operation),
                                         ctypes.byref(transferred), False):
            raise ctypes.WinError(ctypes.get_last_error())
        return True

    def close(self):
        if self.file:
            if self.requested:
                kernel.CancelIoEx(self.file, ctypes.byref(self.operation))
            kernel.CloseHandle(self.file)  # Closing acknowledges the break and unblocks native I/O.
            self.file = None
            # Cancellation is asynchronous; keep OVERLAPPED and its event alive until completion.
            if self.requested and kernel.WaitForSingleObject(self.operation.hEvent, 5000) != 0:
                raise TimeoutError("Oplock cancellation did not complete")
        if self.operation.hEvent:
            kernel.CloseHandle(self.operation.hEvent)
            self.operation.hEvent = None


def publish(path, state):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps({"state": state}), encoding="utf-8")
    temporary.replace(path)


def hold(path, evidence, release):
    wait = FileWait(path)
    try:
        publish(evidence, "held")
        deadline = time.monotonic() + 120
        broken = False
        while not release.exists():
            if not broken and wait.broken():
                broken = True
                publish(evidence, "blocked")
            if time.monotonic() >= deadline:
                raise TimeoutError("Fixture did not release its file oplock within 120 seconds")
            time.sleep(0.02)
    finally:
        wait.close()
    publish(evidence, "released")


def probe():
    with tempfile.TemporaryDirectory(prefix="qbutt-oplock-probe-") as directory:
        path = Path(directory) / "generated.bin"
        path.write_bytes(b"before")
        wait = FileWait(path)
        finished = threading.Event()
        errors = []

        def write():
            try:
                path.write_bytes(b"after")
            except BaseException as error:
                errors.append(str(error))
            finally:
                finished.set()

        worker = threading.Thread(target=write, daemon=True)
        try:
            worker.start()
            deadline = time.monotonic() + 3
            while not wait.broken():
                if time.monotonic() >= deadline:
                    raise TimeoutError("Native writer did not break the oplock")
                time.sleep(0.01)
            assert not finished.wait(0.2), "Writer escaped the unreleased oplock"
        finally:
            wait.close()
            worker.join(3)
        assert finished.is_set() and not errors, f"Writer did not resume: {errors}"
        assert path.read_bytes() == b"after"
        # Failure cleanup must also finish when no competing operation ever broke the lock.
        pending = FileWait(path)
        pending.close()
        print(json.dumps({"passed": True, "actualWriterBlocked": True,
                          "releasedPayloadExact": True, "unbrokenCancellationCompleted": True}))


if __name__ == "__main__":
    if sys.argv[1:] == ["--probe"]:
        probe()
    else:
        assert len(sys.argv) == 4, "Expected target file, evidence file, release marker"
        hold(*(Path(argument) for argument in sys.argv[1:]))
