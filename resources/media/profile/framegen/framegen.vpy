"""Native vs-mlrt frame interpolation graph used by scripts/framegen.lua.

All paths and playback parameters come from mpv's length-quoted ``user-data``
JSON.  This script deliberately performs no package discovery or downloads.
"""

from __future__ import annotations

from collections import Counter
import ctypes
from ctypes import wintypes
from fractions import Fraction
import importlib.util
import json
import math
import os
import sys
import threading
from types import SimpleNamespace
from uuid import UUID

# The installed runtime is immutable and manager status rejects undeclared
# files. VSScript embeds Python, so process environment is not a reliable place
# to suppress bytecode writes before loading the standalone vsmlrt wrapper.
sys.dont_write_bytecode = True

import vapoursynth as vs


core = vs.core


GPU_LOCK_STATE_ATTRIBUTE = "_mpv_config_framegen_gpu_lock_state"
OPENVINO_DLL_SEARCH_FLAGS = 0x00000100 | 0x00001000


class OpenVinoAvailableDevices(ctypes.Structure):
    _fields_ = [
        ("devices", ctypes.POINTER(ctypes.c_char_p)),
        ("size", ctypes.c_size_t),
    ]


def positive_int(config: dict, name: str) -> int:
    value = config.get(name)
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise ValueError(f"{name} must be a positive integer")
    return value


def required_path(config: dict, name: str, kind: str) -> str:
    value = config.get(name)
    if not isinstance(value, str) or not os.path.isabs(value):
        raise ValueError(f"{name} must be an absolute path")

    exists = os.path.isfile(value) if kind == "file" else os.path.isdir(value)
    if not exists:
        raise FileNotFoundError(f"{name} is missing: {value}")
    return value


def device_binding(config: dict) -> tuple[int, str]:
    device_id = config.get("device_id")
    if isinstance(device_id, bool) or not isinstance(device_id, int) or device_id < 0:
        raise ValueError("device_id must be a non-negative integer")
    gpu_id = config.get("gpu_id")
    if not isinstance(gpu_id, str) or not gpu_id:
        raise ValueError("gpu_id must identify the selected GPU")
    return device_id, gpu_id


def framegen_gpu_lock_state() -> dict:
    """Return the process-wide refcount for overlapping VSScript graphs."""
    state = sys.__dict__.get(GPU_LOCK_STATE_ATTRIBUTE)
    if state is None:
        state = sys.__dict__.setdefault(
            GPU_LOCK_STATE_ATTRIBUTE,
            {
                "guard": threading.RLock(),
                "api": None,
                "handle": None,
                "leases": 0,
            },
        )
    return state


def load_framegen_gpu_lock_api():
    if os.name != "nt":
        raise RuntimeError("the frame generation runtime requires Windows")

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateFileW.argtypes = (
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        ctypes.c_void_p,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.HANDLE,
    )
    kernel32.CreateFileW.restype = wintypes.HANDLE
    kernel32.CloseHandle.argtypes = (wintypes.HANDLE,)
    kernel32.CloseHandle.restype = wintypes.BOOL
    return kernel32


def open_framegen_gpu_lock(kernel32):
    """Acquire the cross-process lock once, without blocking mpv's UI thread."""

    runtime_root = os.environ.get("QBUTT_MEDIA_RUNTIME")
    if not runtime_root:
        raise RuntimeError("QBUTT_MEDIA_RUNTIME is unavailable; the framegen GPU lock cannot be resolved")
    lock_path = os.path.join(os.path.dirname(runtime_root), "locks", "framegen-gpu.lock")
    os.makedirs(os.path.dirname(lock_path), exist_ok=True)

    generic_read_write = 0x80000000 | 0x40000000
    share_none = 0
    open_always = 4
    file_attribute_normal = 0x80
    invalid_handle = wintypes.HANDLE(-1).value
    handle = kernel32.CreateFileW(
        lock_path,
        generic_read_write,
        share_none,
        None,
        open_always,
        file_attribute_normal,
        None,
    )
    if handle == invalid_handle:
        error_code = ctypes.get_last_error()
        if error_code == 32:
            raise RuntimeError("framegen GPU is busy")
        raise ctypes.WinError(error_code)
    return handle


class FramegenGpuLockLease:
    __slots__ = ("_state", "_win_error", "_get_last_error")

    def __init__(self, state: dict):
        self._state = state
        # VSScript clears script globals before invoking destruction callbacks.
        self._win_error = ctypes.WinError
        self._get_last_error = ctypes.get_last_error

    def close(self) -> None:
        state = self._state
        if state is None:
            return
        with state["guard"]:
            if state["leases"] <= 0 or state["handle"] is None:
                raise RuntimeError("framegen GPU lock lease is not active")
            if state["leases"] > 1:
                state["leases"] -= 1
            else:
                if not state["api"].CloseHandle(state["handle"]):
                    raise self._win_error(self._get_last_error())
                state["handle"] = None
                state["leases"] = 0
            self._state = None


def acquire_framegen_gpu_lock(
    already_owned: bool,
    bypassed: bool,
) -> FramegenGpuLockLease | None:
    if already_owned and bypassed:
        raise ValueError("gpu_lock_owned and gpu_lock_bypassed cannot both be true")
    if already_owned or bypassed:
        return None

    state = framegen_gpu_lock_state()
    with state["guard"]:
        if state["api"] is None:
            state["api"] = load_framegen_gpu_lock_api()
        if state["handle"] is None:
            state["handle"] = open_framegen_gpu_lock(state["api"])
        state["leases"] += 1
        return FramegenGpuLockLease(state)


def validate_trt_binding(device_id: int, gpu_id: str) -> None:
    if not gpu_id.lower().startswith("gpu-"):
        raise ValueError("TensorRT gpu_id must be an NVIDIA GPU UUID")
    try:
        expected_uuid = UUID(gpu_id[4:])
    except ValueError as error:
        raise ValueError("TensorRT gpu_id must be an NVIDIA GPU UUID") from error
    properties = core.trt.DeviceProperties(device_id)
    raw_uuid = properties.get("uuid")
    if not isinstance(raw_uuid, list) or len(raw_uuid) != 16:
        raise RuntimeError("TensorRT did not report a valid GPU UUID")
    actual_uuid = UUID(bytes=bytes(int(value) & 0xFF for value in raw_uuid))
    if actual_uuid != expected_uuid:
        raise RuntimeError("TensorRT GPU does not match the installed runtime device")


def canonical_openvino_uuid(value, *, prefixed: bool) -> str:
    if isinstance(value, bytes):
        try:
            value = value.decode("ascii")
        except UnicodeDecodeError as error:
            raise ValueError("OpenVINO DEVICE_UUID must be ASCII") from error
    if not isinstance(value, str):
        raise ValueError("OpenVINO DEVICE_UUID must be text")
    if prefixed:
        if not value.startswith("uuid:"):
            raise ValueError("OpenVINO gpu_id must use the uuid: prefix")
        value = value[5:]
    value = value.lower()
    if (
        len(value) != 32
        or any(character not in "0123456789abcdef" for character in value)
        or int(value, 16) == 0
    ):
        raise ValueError("OpenVINO DEVICE_UUID must contain 16 non-zero bytes")
    return f"uuid:{value}"


def openvino_gpu_device_name(value) -> str | None:
    if isinstance(value, bytes):
        try:
            value = value.decode("ascii")
        except UnicodeDecodeError as error:
            raise ValueError("OpenVINO device name must be ASCII") from error
    if not isinstance(value, str):
        raise ValueError("OpenVINO device name must be text")
    if value == "GPU":
        return value
    suffix = value[4:] if value.startswith("GPU.") else ""
    if suffix and all(character in "0123456789" for character in suffix):
        return value
    return None


def select_openvino_device(gpu_id: str, devices) -> str:
    expected_uuid = canonical_openvino_uuid(gpu_id, prefixed=True)
    matches = []
    for raw_name, raw_uuid in devices:
        name = openvino_gpu_device_name(raw_name)
        if name is None:
            continue
        if canonical_openvino_uuid(raw_uuid, prefixed=False) == expected_uuid:
            matches.append(name)
    if len(matches) != 1:
        raise RuntimeError(
            f"OpenVINO DEVICE_UUID matched {len(matches)} GPU devices"
        )
    return matches[0]


def check_openvino_status(api, status: int, operation: str) -> None:
    if status == 0:
        return
    message = api.ov_get_last_err_msg()
    details = message.decode("utf-8", errors="replace") if message else "no details"
    raise RuntimeError(f"{operation} failed ({status}): {details}")


def query_openvino_gpu_devices(openvino_c_path: str) -> list[tuple[str, bytes]]:
    if os.name != "nt":
        raise RuntimeError("the OpenVINO frame generation runtime requires Windows")
    api = ctypes.WinDLL(openvino_c_path, winmode=OPENVINO_DLL_SEARCH_FLAGS)
    api.ov_core_create.argtypes = (ctypes.POINTER(ctypes.c_void_p),)
    api.ov_core_create.restype = ctypes.c_int
    api.ov_core_free.argtypes = (ctypes.c_void_p,)
    api.ov_core_free.restype = None
    api.ov_core_get_available_devices.argtypes = (
        ctypes.c_void_p,
        ctypes.POINTER(OpenVinoAvailableDevices),
    )
    api.ov_core_get_available_devices.restype = ctypes.c_int
    api.ov_available_devices_free.argtypes = (
        ctypes.POINTER(OpenVinoAvailableDevices),
    )
    api.ov_available_devices_free.restype = None
    api.ov_core_get_property.argtypes = (
        ctypes.c_void_p,
        ctypes.c_char_p,
        ctypes.c_char_p,
        ctypes.POINTER(ctypes.c_void_p),
    )
    api.ov_core_get_property.restype = ctypes.c_int
    api.ov_free.argtypes = (ctypes.c_void_p,)
    api.ov_free.restype = None
    api.ov_get_last_err_msg.argtypes = ()
    api.ov_get_last_err_msg.restype = ctypes.c_char_p

    core_handle = ctypes.c_void_p()
    available_devices = OpenVinoAvailableDevices()
    devices_loaded = False
    try:
        check_openvino_status(
            api,
            api.ov_core_create(ctypes.byref(core_handle)),
            "OpenVINO core creation",
        )
        check_openvino_status(
            api,
            api.ov_core_get_available_devices(
                core_handle,
                ctypes.byref(available_devices),
            ),
            "OpenVINO device enumeration",
        )
        devices_loaded = True
        result = []
        for index in range(available_devices.size):
            raw_name = available_devices.devices[index]
            if raw_name is None:
                raise RuntimeError("OpenVINO returned an empty device name")
            name = openvino_gpu_device_name(raw_name)
            if name is None:
                continue

            property_value = ctypes.c_void_p()
            try:
                check_openvino_status(
                    api,
                    api.ov_core_get_property(
                        core_handle,
                        raw_name,
                        b"DEVICE_UUID",
                        ctypes.byref(property_value),
                    ),
                    f"OpenVINO DEVICE_UUID query for {name}",
                )
                if not property_value.value:
                    raise RuntimeError(f"OpenVINO returned no DEVICE_UUID for {name}")
                result.append((name, ctypes.string_at(property_value.value)))
            finally:
                if property_value.value:
                    api.ov_free(property_value)
        return result
    finally:
        if devices_loaded:
            api.ov_available_devices_free(ctypes.byref(available_devices))
        if core_handle.value:
            api.ov_core_free(core_handle)


def timeline_position(n: int, source_rate: Fraction, target: Fraction) -> tuple[int, int, int]:
    """Return the source index and exact fractional remainder for output frame n."""
    position_num = source_rate.numerator * target.denominator
    position_den = source_rate.denominator * target.numerator
    index, remainder = divmod(n * position_num, position_den)
    return index, remainder, position_den


def histogram_distance(left: list[int], right: list[int]) -> float:
    """Bhattacharyya distance between equally sized 64-bin luma histograms."""
    if len(left) != 64 or len(right) != 64:
        raise ValueError("luma histograms must contain exactly 64 bins")
    sample_count = sum(left)
    if sample_count <= 0 or sample_count != sum(right):
        raise ValueError("luma histograms must contain the same non-zero sample count")
    coefficient = sum(math.sqrt(a * b) for a, b in zip(left, right)) / sample_count
    return math.sqrt(max(0.0, 1.0 - coefficient))


def frame_histogram(frame: vs.VideoFrame) -> list[int]:
    counts = Counter(memoryview(frame[0]).cast("B"))
    if counts and max(counts) >= 64:
        raise RuntimeError("luma histogram frame was not quantized to 64 bins")
    return [counts.get(index, 0) for index in range(64)]


def convert_to_rgb(
    clip: vs.VideoNode,
    matrix: str,
    color_range: str,
    model_format: int,
) -> vs.VideoNode:
    if clip.format.color_family == vs.RGB:
        return clip.resize.Bicubic(format=model_format)
    return clip.resize.Bicubic(
        format=model_format,
        matrix_in_s=matrix,
        range_in_s=color_range,
    )


def convert_from_rgb(
    clip: vs.VideoNode,
    source_format: vs.VideoFormat,
    matrix: str,
    color_range: str,
) -> vs.VideoNode:
    if source_format.color_family == vs.RGB:
        return clip.resize.Bicubic(format=source_format.id)
    return clip.resize.Bicubic(
        format=source_format.id,
        matrix_s=matrix,
        range_s=color_range,
    )


def mark_temporal_guards(
    clip: vs.VideoNode,
    scene_threshold: float,
    duplicate_threshold: float,
) -> vs.VideoNode:
    # Thresholds are calibrated at this exact width. Upscaling a rare smaller
    # input is preferable to silently changing the detector's distribution.
    analysis_width = 320
    analysis_height = max(2, round(clip.height * analysis_width / clip.width / 2) * 2)
    gray = clip.resize.Bicubic(
        width=analysis_width,
        height=analysis_height,
        format=vs.GRAYS,
        matrix_s="709",
    )
    next_gray = gray[1:] + gray[-1]
    difference = next_gray.std.PlaneStats(gray)
    # Match the frozen evaluator exactly: first apply VapourSynth's canonical
    # float-to-full-range-byte conversion, then collapse each four-byte bucket
    # into one of the 64 histogram bins. Combining both operations in Expr
    # changes boundary rounding for a small number of real frames.
    gray8 = gray.resize.Point(
        format=vs.GRAY8,
        range_s="full",
        dither_type="none",
    )
    histogram_frames = gray8.std.Lut(function=lambda x: x >> 2)
    next_histogram_frames = histogram_frames[1:] + histogram_frames[-1]

    def set_scene_prop(n: int, f: list[vs.VideoFrame]) -> vs.VideoFrame:
        output = f[0].copy()
        scene_score = histogram_distance(frame_histogram(f[1]), frame_histogram(f[2]))
        frame_difference = f[3].props.get("PlaneStatsDiff", 0.0)
        output.props["_SceneChangeNext"] = int(scene_score >= scene_threshold)
        output.props["_FramegenDuplicateNext"] = int(frame_difference < duplicate_threshold)
        return output

    return clip.std.ModifyFrame(
        clips=[clip, histogram_frames, next_histogram_frames, difference],
        selector=set_scene_prop,
    )


def exact_rate_rife(
    source: vs.VideoNode,
    target: Fraction,
    interpolate,
    model_format: int,
    mask_format: int,
    process_scale: float,
    scene_threshold: float,
    duplicate_threshold: float,
    matrix: str,
    color_range: str,
) -> vs.VideoNode:
    """Sample RIFE at exact timestamps and preserve coincident decoded frames."""
    source_rate = source.fps
    source_format = source.format
    output_length = min(
        math.ceil(source.num_frames * target / source_rate),
        2**31 - 1,
    )
    timeline = source.std.BlankClip(
        length=output_length,
        fpsnum=target.numerator,
        fpsden=target.denominator,
        keep=True,
    )

    def source_index(n: int) -> int:
        return timeline_position(n, source_rate, target)[0]

    rgb_source = convert_to_rgb(source, matrix, color_range, model_format)
    guarded_source = mark_temporal_guards(
        rgb_source,
        scene_threshold,
        duplicate_threshold,
    )

    def left_source(n: int) -> vs.VideoNode:
        return source[source_index(n)]

    source_left = core.std.FrameEval(timeline, left_source)
    source_left = source_left.std.AssumeFPS(
        fpsnum=target.numerator,
        fpsden=target.denominator,
    )

    def left_rgb(n: int) -> vs.VideoNode:
        return rgb_source[source_index(n)]

    def left_guard(n: int) -> vs.VideoNode:
        return guarded_source[source_index(n)]

    def right_rgb(n: int) -> vs.VideoNode:
        # mpv intentionally advertises a synthetic source length. Its bridge
        # turns this look-ahead into EOF during drain, so clamping against
        # ``num_frames`` would clamp to the synthetic INT_MAX/16 sentinel.
        return rgb_source[source_index(n) + 1]

    rgb_base = rgb_source.std.BlankClip(
        length=output_length,
        fpsnum=target.numerator,
        fpsden=target.denominator,
        keep=True,
    )
    rgb_left = core.std.FrameEval(rgb_base, left_rgb)
    guarded_left = core.std.FrameEval(rgb_base, left_guard)
    model_left = rgb_left
    model_right = core.std.FrameEval(rgb_base, right_rgb)

    if process_scale < 1.0:
        width = max(64, round(source.width * process_scale / 2) * 2)
        height = max(64, round(source.height * process_scale / 2) * 2)
        model_left = model_left.resize.Bicubic(width=width, height=height)
        model_right = model_right.resize.Bicubic(width=width, height=height)

    mask_base = model_left.std.BlankClip(format=mask_format, keep=True)

    def timepoint(n: int) -> vs.VideoNode:
        _, remainder, denominator = timeline_position(n, source_rate, target)
        fraction = remainder / denominator
        return mask_base.std.BlankClip(color=fraction, keep=True)

    mask = core.std.FrameEval(mask_base, timepoint)
    generated = interpolate(model_left, model_right, mask)
    if process_scale < 1.0:
        generated = generated.resize.Bicubic(width=source.width, height=source.height)
    generated = convert_from_rgb(generated, source_format, matrix, color_range)
    # Carry mpv's private image metadata as well as public color/SAR props.
    # The final duration below intentionally replaces the copied source duration.
    generated = generated.std.CopyFrameProps(prop_src=source_left)

    def choose_interpolated(n: int, f: vs.VideoFrame) -> vs.VideoNode:
        if f.props.get("_SceneChangeNext", False) or f.props.get(
            "_FramegenDuplicateNext", False
        ):
            return source_left
        return generated

    interpolated = core.std.FrameEval(
        timeline,
        choose_interpolated,
        prop_src=guarded_left,
    )

    def choose_timestamp(n: int) -> vs.VideoNode:
        # Exact decoded timestamps never evaluate the i+1-dependent model or
        # temporal props. This preserves the final real source frame; only an
        # unavailable fractional tail is dropped when mpv reports dynamic EOF.
        return source_left if timeline_position(n, source_rate, target)[1] == 0 else interpolated

    output = core.std.FrameEval(timeline, choose_timestamp)
    output = output.std.AssumeFPS(
        fpsnum=target.numerator,
        fpsden=target.denominator,
    )
    return output.std.SetFrameProps(
        _DurationNum=target.denominator,
        _DurationDen=target.numerator,
    )


def refuse_engine_build(*args, **kwargs):
    raise RuntimeError("TensorRT engine is not prepared; compilation is forbidden during playback")


def build_graph(
    source: vs.VideoNode,
    raw_config: str,
    engine_builder=None,
) -> tuple[vs.VideoNode, FramegenGpuLockLease | None]:
    try:
        config = json.loads(raw_config)
    except json.JSONDecodeError as error:
        raise ValueError("framegen user-data is not valid JSON") from error
    if not isinstance(config, dict) or config.get("schema_version") != 1:
        raise ValueError("unsupported framegen user-data schema")

    source_rate = Fraction(
        positive_int(config, "source_num"),
        positive_int(config, "source_den"),
    )
    target = Fraction(
        positive_int(config, "target_num"),
        positive_int(config, "target_den"),
    )
    if target <= source_rate:
        raise ValueError("target rate must be higher than source rate")
    if source.format is None:
        raise ValueError("framegen requires a constant source format")

    process_scale = config.get("process_scale", 1.0)
    if isinstance(process_scale, bool) or not isinstance(process_scale, (int, float)):
        raise ValueError("process_scale must be a number")
    process_scale = float(process_scale)
    if not 0.25 <= process_scale <= 1.0:
        raise ValueError("process_scale must be between 0.25 and 1.0")

    output_width = positive_int(config, "output_width")
    output_height = positive_int(config, "output_height")
    if output_width > source.width or output_height > source.height:
        raise ValueError("framegen output cannot exceed the decoded source")

    scene_threshold = config.get("scene_threshold")
    if isinstance(scene_threshold, bool) or not isinstance(scene_threshold, (int, float)):
        raise ValueError("scene_threshold must be a number")
    scene_threshold = float(scene_threshold)
    if not 0.0 <= scene_threshold <= 1.0:
        raise ValueError("scene_threshold must be between 0 and 1")

    duplicate_threshold = config.get("duplicate_threshold")
    if isinstance(duplicate_threshold, bool) or not isinstance(
        duplicate_threshold, (int, float)
    ):
        raise ValueError("duplicate_threshold must be a number")
    duplicate_threshold = float(duplicate_threshold)
    if not 0.0 <= duplicate_threshold < 1.0:
        raise ValueError("duplicate_threshold must be between 0 and 1")

    gpu_lock_owned = config.get("gpu_lock_owned", False)
    if not isinstance(gpu_lock_owned, bool):
        raise ValueError("gpu_lock_owned must be a boolean")
    gpu_lock_bypassed = config.get("gpu_lock_bypassed", False)
    if not isinstance(gpu_lock_bypassed, bool):
        raise ValueError("gpu_lock_bypassed must be a boolean")
    if gpu_lock_owned and gpu_lock_bypassed:
        raise ValueError("gpu_lock_owned and gpu_lock_bypassed cannot both be true")

    matrix = config.get("matrix", "709")
    if matrix not in {"709", "470bg", "240m", "2020ncl"}:
        raise ValueError(f"unsupported matrix: {matrix}")
    color_range = config.get("range", "limited")
    if color_range not in {"limited", "full"}:
        raise ValueError(f"unsupported color range: {color_range}")

    runtime_backend = config.get("backend")
    if runtime_backend not in {"trt", "openvino"}:
        raise ValueError("unsupported frame generation backend")
    if runtime_backend == "openvino" and process_scale != 1.0:
        raise ValueError("OpenVINO frame generation requires process_scale=1")
    plugin_dir = required_path(config, "plugin_dir", "dir")
    model_dir = required_path(config, "model_dir", "dir")
    engine_dir = required_path(config, "engine_dir", "dir")
    model_name = config.get("model")
    model_files = {
        "4.26": ("rife_v4.26.onnx", "v4_26"),
    }
    if model_name not in model_files:
        raise ValueError(f"unsupported RIFE model: {model_name}")
    model_filename, model_enum = model_files[model_name]
    model_path = os.path.join(model_dir, "rife_v2", model_filename)
    if not os.path.isfile(model_path):
        raise FileNotFoundError(f"RIFE model is missing: {model_path}")

    device_id, gpu_id = device_binding(config)
    if runtime_backend == "trt":
        plugin_path = os.path.join(plugin_dir, "vstrt.dll")
        wrapper_path = os.path.join(plugin_dir, "vsmlrt.py")
        trt_runtime_dir = os.path.join(plugin_dir, "vsmlrt-cuda")
        for name, path in (("vstrt plugin", plugin_path), ("vsmlrt wrapper", wrapper_path)):
            if not os.path.isfile(path):
                raise FileNotFoundError(f"{name} is missing: {path}")
        if not os.path.isdir(trt_runtime_dir):
            raise FileNotFoundError(f"TensorRT runtime is missing: {trt_runtime_dir}")
        core.std.LoadPlugin(path=plugin_path, altsearchpath=True)
        validate_trt_binding(device_id, gpu_id)
        spec = importlib.util.spec_from_file_location("vsmlrt", wrapper_path)
        if spec is None or spec.loader is None:
            raise ImportError(f"could not load vsmlrt wrapper: {wrapper_path}")
        vsmlrt = importlib.util.module_from_spec(spec)
        sys.modules["vsmlrt"] = vsmlrt
        spec.loader.exec_module(vsmlrt)
        vsmlrt.models_path = model_dir
        # Keep upstream cache identity and RIFE precision rules, but never let
        # a cache miss spawn a blocking compiler inside the playback graph.
        # Only the separate preparation worker may run the compiler.
        vsmlrt.subprocess = SimpleNamespace(run=engine_builder or refuse_engine_build)
        backend = vsmlrt.Backend.TRT(
            fp16=True,
            device_id=device_id,
            output_format=1,
            use_cuda_graph=True,
            num_streams=2,
            builder_optimization_level=5,
            engine_folder=engine_dir,
            # vsmlrt replaces the builder subprocess environment. Give it only
            # the validated, version-local TensorRT directory; playback does
            # not depend on the shell or persistent user PATH.
            custom_env={"PATH": trt_runtime_dir},
        )

        def interpolate(left, right, mask):
            return vsmlrt.RIFEMerge(
                clipa=left,
                clipb=right,
                mask=mask,
                model=getattr(vsmlrt.RIFEModel, model_enum),
                backend=backend,
                _implementation=2,
            )

        model_format = vs.RGBH
        mask_format = vs.GRAYH
    else:
        plugin_path = os.path.join(plugin_dir, "vsov.dll")
        openvino_c_path = os.path.join(plugin_dir, "vsov", "openvino_c.dll")
        for name, path in (
            ("vsov plugin", plugin_path),
            ("OpenVINO C runtime", openvino_c_path),
        ):
            if not os.path.isfile(path):
                raise FileNotFoundError(f"{name} is missing: {path}")
        openvino_device = select_openvino_device(
            gpu_id,
            query_openvino_gpu_devices(openvino_c_path),
        )
        core.std.LoadPlugin(path=plugin_path, altsearchpath=True)

        def interpolate(left, right, mask):
            return core.ov.Model(
                [left, right, mask],
                model_path,
                device=openvino_device,
                builtin=False,
                fp16=False,
                config=lambda: {
                    "NUM_STREAMS": 4,
                    "INFERENCE_PRECISION_HINT": "f16",
                    "CACHE_DIR": engine_dir,
                },
            )

        model_format = vs.RGBS
        mask_format = vs.GRAYS

    source = source.std.AssumeFPS(
        fpsnum=source_rate.numerator,
        fpsden=source_rate.denominator,
    )
    if source.width != output_width or source.height != output_height:
        source = source.resize.Bicubic(width=output_width, height=output_height)
    gpu_lock_lease = acquire_framegen_gpu_lock(gpu_lock_owned, gpu_lock_bypassed)
    try:
        output = exact_rate_rife(
            source=source,
            target=target,
            interpolate=interpolate,
            model_format=model_format,
            mask_format=mask_format,
            process_scale=process_scale,
            scene_threshold=scene_threshold,
            duplicate_threshold=duplicate_threshold,
            matrix=matrix,
            color_range=color_range,
        )
    except BaseException:
        if gpu_lock_lease is not None:
            gpu_lock_lease.close()
        raise
    return output, gpu_lock_lease


video_out, gpu_lock_lease = build_graph(
    video_in, user_data, engine_builder=globals().get("engine_builder")
)
try:
    video_out.set_output()
    if gpu_lock_lease is not None:
        # VSScript teardown must release the GPU even when Python callback
        # cycles retain this script's globals until a later collection.
        vs.register_on_destroy(gpu_lock_lease.close)
except BaseException:
    if gpu_lock_lease is not None:
        gpu_lock_lease.close()
    raise
