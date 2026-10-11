"""Prepare the exact playback graph outside the player process.

The compiler stays hidden and dies with its worker. Playback never runs it.
"""

from __future__ import annotations

import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys

sys.dont_write_bytecode = True

import vapoursynth as vs


class CompilationRequired(Exception):
    pass


def capture_compiler(*args, **kwargs):
    raise CompilationRequired


def run_hidden(args, **kwargs):
    return subprocess.run(args, creationflags=subprocess.CREATE_NO_WINDOW, **kwargs)


class JobBasicLimits(ctypes.Structure):
    _fields_ = [
        ("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
        ("flags", wintypes.DWORD),
        ("minimum_working_set", ctypes.c_size_t),
        ("maximum_working_set", ctypes.c_size_t),
        ("active_processes", wintypes.DWORD),
        ("affinity", ctypes.c_size_t),
        ("priority", wintypes.DWORD), ("scheduling", wintypes.DWORD),
    ]


class JobExtendedLimits(ctypes.Structure):
    _fields_ = [
        ("basic", JobBasicLimits),
        ("io_counters", ctypes.c_uint64 * 6),
        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
        ("peak_process_memory", ctypes.c_size_t), ("peak_job_memory", ctypes.c_size_t),
    ]


def terminate_compiler_with_worker():
    """Closing mpv cancels its worker and the compiler, releasing the GPU lock."""
    api = ctypes.WinDLL("kernel32", use_last_error=True)
    api.CreateJobObjectW.argtypes = (ctypes.c_void_p, wintypes.LPCWSTR)
    api.CreateJobObjectW.restype = wintypes.HANDLE
    api.SetInformationJobObject.argtypes = (
        wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD,
    )
    api.SetInformationJobObject.restype = wintypes.BOOL
    api.GetCurrentProcess.restype = wintypes.HANDLE
    api.AssignProcessToJobObject.argtypes = (wintypes.HANDLE, wintypes.HANDLE)
    api.AssignProcessToJobObject.restype = wintypes.BOOL
    api.CloseHandle.argtypes = (wintypes.HANDLE,)
    handle = api.CreateJobObjectW(None, None)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    limits = JobExtendedLimits()
    limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not api.SetInformationJobObject(handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)) \
            or not api.AssignProcessToJobObject(handle, api.GetCurrentProcess()):
        error = ctypes.get_last_error()
        api.CloseHandle(handle)
        raise ctypes.WinError(error)
    # Keep this non-inheritable handle until process exit. Its closure kills
    # any surviving descendants; closing it here would also terminate us.
    return handle


def prepare_engine(config, build=False):
    if config["backend"] != "trt":
        raise ValueError("only TensorRT engines need preparation")
    with os.add_dll_directory(os.path.join(config["plugin_dir"], "vsmlrt-cuda")):
        source = vs.core.std.BlankClip(
            width=config["output_width"],
            height=config["output_height"],
            format=vs.YUV420P8,
            length=8,
            fpsnum=config["source_num"],
            fpsden=config["source_den"],
        )
        try:
            runpy.run_path(str(Path(__file__).with_name("framegen.vpy")), init_globals={
                "video_in": source,
                "user_data": json.dumps(config),
                "engine_builder": run_hidden if build else capture_compiler,
            })
        except CompilationRequired:
            return {"ready": False}
        finally:
            vs.clear_outputs()
        return {"ready": True}


if __name__ == "__main__":
    job_handle = terminate_compiler_with_worker()
    print(json.dumps(prepare_engine(json.loads(sys.argv[1]), build="--build" in sys.argv[2:])))
