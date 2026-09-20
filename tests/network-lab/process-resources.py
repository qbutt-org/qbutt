"""Bounded Windows process counters for one benchmark transfer window."""

import ctypes
from ctypes import wintypes
from datetime import datetime, timezone
import json
import os
import pathlib
import sys
import threading
import time


class IoCounters(ctypes.Structure):
    _fields_ = [(name, ctypes.c_ulonglong) for name in (
        "readOperations", "writeOperations", "otherOperations", "readBytes", "writeBytes", "otherBytes")]


class MemoryCounters(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("pageFaultCount", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in ("peakWorkingSet", "workingSet", "peakPagedPool", "pagedPool",
                                           "peakNonPagedPool", "nonPagedPool", "pagefile", "peakPagefile", "privateBytes")]


kernel = ctypes.WinDLL("kernel32", use_last_error=True)
psapi = ctypes.WinDLL("psapi", use_last_error=True)
kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel.OpenProcess.restype = wintypes.HANDLE
kernel.CloseHandle.argtypes = [wintypes.HANDLE]
kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
kernel.WaitForSingleObject.restype = wintypes.DWORD
kernel.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR,
                                            ctypes.POINTER(wintypes.DWORD)]
kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
kernel.GetProcessIoCounters.argtypes = [wintypes.HANDLE, ctypes.POINTER(IoCounters)]
psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(MemoryCounters), wintypes.DWORD]


def checked(ok):
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error())


def filetime(value):
    return (value.dwHighDateTime << 32) | value.dwLowDateTime


def times(handle):
    values = [wintypes.FILETIME() for _ in range(4)]
    checked(kernel.GetProcessTimes(handle, *(ctypes.byref(value) for value in values)))
    return [filetime(value) for value in values]


def emit(value):
    print(json.dumps(value), flush=True)


targets = json.loads(sys.argv[1])
if not (1 <= len(targets) <= 2 and len({target["role"] for target in targets}) == len(targets)
        and len({target["pid"] for target in targets}) == len(targets)):
    raise RuntimeError("Expected distinct app and optional qbutt-net targets")
handles = []
try:
    identities = []
    for target in targets:
        if target["role"] not in ("app", "qbutt-net") or type(target["pid"]) is not int or target["pid"] <= 0:
            raise RuntimeError("Invalid benchmark process identity")
        # Hold the opened object throughout the window: a recycled PID cannot
        # replace it. SYNCHRONIZE lets every snapshot reject an exited process.
        handle = kernel.OpenProcess(0x100000 | 0x0400 | 0x0010, False, target["pid"])
        checked(handle)
        handles.append(handle)
        image = ctypes.create_unicode_buffer(32768)
        length = wintypes.DWORD(len(image))
        checked(kernel.QueryFullProcessImageNameW(handle, 0, image, ctypes.byref(length)))
        if pathlib.Path(image.value).resolve() != pathlib.Path(target["executable"]).resolve():
            raise RuntimeError("Benchmark PID belongs to a different executable")
        identities.append({"role": target["role"], "pid": target["pid"], "executable": image.value,
                           "creationTime100ns": str(times(handle)[0])})

    def snapshot():
        captured = []
        for handle in handles:
            if kernel.WaitForSingleObject(handle, 0) != 258:  # WAIT_TIMEOUT: still running
                raise RuntimeError("A measured process exited during the resource window")
            _, _, kernel_time, user_time = times(handle)
            io = IoCounters()
            checked(kernel.GetProcessIoCounters(handle, ctypes.byref(io)))
            memory = MemoryCounters()
            memory.cb = ctypes.sizeof(memory)
            checked(psapi.GetProcessMemoryInfo(handle, ctypes.byref(memory), memory.cb))
            captured.append({"cpu100ns": kernel_time + user_time,
                             "workingSetBytes": memory.workingSet, "privateBytes": memory.privateBytes,
                             "io": {name: getattr(io, name) for name, _ in IoCounters._fields_}})
        return {"monotonicNs": time.perf_counter_ns(), "processes": captured}

    emit({"ready": True, "processes": identities})
    if sys.stdin.readline().strip() != "start":
        raise RuntimeError("Expected resource window start")
    started_at = datetime.now(timezone.utc).isoformat()
    samples = [snapshot()]
    stopped = threading.Event()
    errors = []

    def sample_memory():
        try:
            while not stopped.wait(0.25):
                if time.perf_counter_ns() - samples[0]["monotonicNs"] > 180_000_000_000:
                    raise RuntimeError("Resource window exceeded 180 seconds")
                samples.append(snapshot())
        except Exception as error:
            errors.append(error)

    sampler = threading.Thread(target=sample_memory, daemon=True)
    sampler.start()
    emit({"started": True})
    command = sys.stdin.readline().strip()
    stopped.set()
    sampler.join()
    if command != "stop":
        raise RuntimeError("Expected resource window stop")
    if errors:
        raise errors[0]
    samples.append(snapshot())
    elapsed_ms = (samples[-1]["monotonicNs"] - samples[0]["monotonicNs"]) / 1_000_000
    processes = []
    for index, identity in enumerate(identities):
        first = samples[0]["processes"][index]
        last = samples[-1]["processes"][index]
        cpu_ms = (last["cpu100ns"] - first["cpu100ns"]) / 10_000
        processes.append({**identity, "cpuMilliseconds": cpu_ms, "cpuOneCorePercent": cpu_ms / elapsed_ms * 100,
                          "sampledPeakWorkingSetBytes": max(sample["processes"][index]["workingSetBytes"] for sample in samples),
                          "sampledPeakPrivateBytes": max(sample["processes"][index]["privateBytes"] for sample in samples),
                          "processIo": {name: last["io"][name] - first["io"][name] for name, _ in IoCounters._fields_}})
    emit({"finished": True, "startedAt": started_at, "finishedAt": datetime.now(timezone.utc).isoformat(),
          "elapsedMilliseconds": elapsed_ms, "sampleIntervalMilliseconds": 250, "sampleCount": len(samples),
          "maximumSampleGapMilliseconds": max((right["monotonicNs"] - left["monotonicNs"]) / 1_000_000
                                               for left, right in zip(samples, samples[1:])),
          "logicalProcessors": os.cpu_count(), "processes": processes,
          "semantics": "CPU 100% is one logical core; memory peaks are sampled within this window; process I/O includes file, network and device I/O, not disk-only"})
finally:
    for handle in handles:
        kernel.CloseHandle(handle)
