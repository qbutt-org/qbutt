# /// script
# requires-python = ">=3.12,<3.13"
# dependencies = [
#   "numpy==2.5.2",
#   "onnx==1.22.0",
#   "onnxruntime==1.28.0",
# ]
# ///
"""Reproduce and verify the Intel Arc RIFE 4.26 s0.5 ONNX graph."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import deque
from copy import deepcopy
from pathlib import Path
from typing import NoReturn

import numpy as np
import onnx
import onnxruntime as ort
from onnx import TensorProto, helper, numpy_helper

SOURCE_ARCHIVE_URL = (
    "https://github.com/AmusementClub/vs-mlrt/releases/download/"
    "external-models/rife_v4.26.7z"
)
SOURCE_ARCHIVE_SHA256 = (
    "dfdabd84a2a3db773f87604b8cc255e94a6a72f13550d910ccd3b4ee2606cd4f"
)
SOURCE_MODEL_SHA256 = "af25762dfec02a4bbb949decea63988b01fa56c46c0ff9dc66ac8e2f12cbb661"
TARGET_MODEL_SHA256 = "3a958289369a5645339b2fbd5f3a228bc82075866c2b4723e4ca485fda72fc6b"
PRACTICAL_RIFE_RELEASE_COMMIT = "0fa2b515e0a067a04a82a52c8dad077cff5eb3c5"
VS_RIFE_EXPORT_COMMIT = "3488617283db7c428a83ba4a19382285da698b6a"

CONTRACT_ID = "mpv-rife-v426-lowres-motion-fullres-warp-v1"
INPUT_SHAPE = ["batch", 7, "height", "width"]
OUTPUT_SHAPE = ["batch", 3, "height", "width"]
VERIFY_SHAPES = ((359, 641), (720, 1280), (1080, 1920))
MAX_ABSOLUTE_ERROR = 5.0e-4
MIN_PSNR_DB = 100.0


def fail(message: str) -> NoReturn:
    raise SystemExit(message)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(4 * 1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def require_hash(path: Path, expected: str, label: str) -> None:
    actual = sha256(path)
    if actual != expected:
        fail(f"{label} SHA-256 mismatch: expected {expected}, got {actual}")


def scalar(name: str, value: float, dtype: np.dtype) -> onnx.TensorProto:
    return numpy_helper.from_array(np.asarray(value, dtype=dtype), name=name)


def vector(
    name: str, values: list[int] | list[float], dtype: np.dtype
) -> onnx.TensorProto:
    return numpy_helper.from_array(np.asarray(values, dtype=dtype), name=name)


def prune_graph(model: onnx.ModelProto, output_names: list[str]) -> None:
    producers = {
        output: index
        for index, node in enumerate(model.graph.node)
        for output in node.output
        if output
    }
    required_nodes: set[int] = set()
    required_tensors = deque(output_names)
    while required_tensors:
        tensor = required_tensors.popleft()
        index = producers.get(tensor)
        if index is None or index in required_nodes:
            continue
        required_nodes.add(index)
        required_tensors.extend(name for name in model.graph.node[index].input if name)

    nodes = [
        deepcopy(node)
        for index, node in enumerate(model.graph.node)
        if index in required_nodes
    ]
    referenced = {name for node in nodes for name in node.input if name}
    initializers = [
        deepcopy(value) for value in model.graph.initializer if value.name in referenced
    ]

    del model.graph.node[:]
    model.graph.node.extend(nodes)
    del model.graph.initializer[:]
    model.graph.initializer.extend(initializers)
    del model.graph.value_info[:]


def rewrite(source: onnx.ModelProto) -> onnx.ModelProto:
    model = deepcopy(source)
    low_input = "/half/Input_output_0"
    for node in model.graph.node:
        for index, name in enumerate(node.input):
            if name == "input":
                node.input[index] = low_input

    model.graph.initializer.extend(
        [
            vector("/half/Resize_scales", [1.0, 1.0, 0.5, 0.5], np.float32),
            vector("/hr/Slice_rgb_starts", [0], np.int64),
            vector("/hr/Slice_rgb_ends", [6], np.int64),
            vector("/hr/Slice_rgb_axes", [1], np.int64),
            vector("/hr/Slice_rgb_steps", [1], np.int64),
            vector("/hr/Slice_spatial_starts", [0, 0], np.int64),
            vector("/hr/Slice_spatial_axes", [2, 3], np.int64),
            vector("/hr/Slice_spatial_steps", [1, 1], np.int64),
            scalar("/hr/Gather_b_index", 0, np.int64),
            scalar("/hr/Gather_h_index", 2, np.int64),
            scalar("/hr/Gather_w_index", 3, np.int64),
            vector("/hr/Channels_rgb", [3], np.int64),
            vector("/hr/Channels_flow", [4], np.int64),
            vector("/hr/Channels_mask", [1], np.int64),
            vector("/hr/Channels_xy", [2], np.int64),
            scalar("/hr/Two_i64", 2, np.int64),
            scalar("/hr/Zero_i64", 0, np.int64),
            scalar("/hr/One_i64", 1, np.int64),
            scalar("/hr/One_f32", 1.0, np.float32),
            scalar("/hr/Two_f32", 2.0, np.float32),
            vector("/hr/Reshape_x", [1, 1, -1], np.int64),
            vector("/hr/Reshape_y", [1, -1, 1], np.int64),
            vector("/hr/Reshape_flow_scale", [1, 4, 1, 1], np.int64),
            vector("/hr/Reshape_grid_scale", [1, 2, 1, 1], np.int64),
            vector("/hr/Unsqueeze_zero", [0], np.int64),
            vector("/hr/Unsqueeze_last", [3], np.int64),
        ]
    )
    model.graph.node.insert(
        0,
        helper.make_node(
            "Resize",
            ["input", "", "/half/Resize_scales"],
            [low_input],
            name="/half/Resize",
            mode="linear",
            coordinate_transformation_mode="half_pixel",
            nearest_mode="floor",
        ),
    )

    nodes: list[onnx.NodeProto] = []

    def add(op: str, inputs: list[str], output: str, name: str, **attrs: object) -> str:
        nodes.append(helper.make_node(op, inputs, [output], name=name, **attrs))
        return output

    full_shape = add("Shape", ["input"], "/hr/Full_shape", "/hr/Shape_full")
    low_shape = add("Shape", [low_input], "/hr/Half_shape", "/hr/Shape_half")
    full_b = add(
        "Gather",
        [full_shape, "/hr/Gather_b_index"],
        "/hr/Full_b",
        "/hr/Gather_full_b",
        axis=0,
    )
    full_h = add(
        "Gather",
        [full_shape, "/hr/Gather_h_index"],
        "/hr/Full_h",
        "/hr/Gather_full_h",
        axis=0,
    )
    full_w = add(
        "Gather",
        [full_shape, "/hr/Gather_w_index"],
        "/hr/Full_w",
        "/hr/Gather_full_w",
        axis=0,
    )
    low_h = add(
        "Gather",
        [low_shape, "/hr/Gather_h_index"],
        "/hr/Half_h",
        "/hr/Gather_half_h",
        axis=0,
    )
    low_w = add(
        "Gather",
        [low_shape, "/hr/Gather_w_index"],
        "/hr/Half_w",
        "/hr/Gather_half_w",
        axis=0,
    )

    full_b_v = add(
        "Unsqueeze",
        [full_b, "/hr/Unsqueeze_zero"],
        "/hr/Full_b_v",
        "/hr/Unsqueeze_full_b",
    )
    full_h_v = add(
        "Unsqueeze",
        [full_h, "/hr/Unsqueeze_zero"],
        "/hr/Full_h_v",
        "/hr/Unsqueeze_full_h",
    )
    full_w_v = add(
        "Unsqueeze",
        [full_w, "/hr/Unsqueeze_zero"],
        "/hr/Full_w_v",
        "/hr/Unsqueeze_full_w",
    )
    low_h_v = add(
        "Unsqueeze",
        [low_h, "/hr/Unsqueeze_zero"],
        "/hr/Half_h_v",
        "/hr/Unsqueeze_half_h",
    )
    low_w_v = add(
        "Unsqueeze",
        [low_w, "/hr/Unsqueeze_zero"],
        "/hr/Half_w_v",
        "/hr/Unsqueeze_half_w",
    )
    spatial_ends = add(
        "Concat",
        [low_h_v, low_w_v],
        "/hr/Half_spatial_ends",
        "/hr/Concat_half_spatial",
        axis=0,
    )
    flow_crop = add(
        "Slice",
        [
            "/Add_7_output_0",
            "/hr/Slice_spatial_starts",
            spatial_ends,
            "/hr/Slice_spatial_axes",
            "/hr/Slice_spatial_steps",
        ],
        "/hr/Flow_crop",
        "/hr/Slice_flow",
    )
    mask_crop = add(
        "Slice",
        [
            "/block4/Split_output_1",
            "/hr/Slice_spatial_starts",
            spatial_ends,
            "/hr/Slice_spatial_axes",
            "/hr/Slice_spatial_steps",
        ],
        "/hr/Mask_crop",
        "/hr/Slice_mask",
    )
    flow_sizes = add(
        "Concat",
        [full_b_v, "/hr/Channels_flow", full_h_v, full_w_v],
        "/hr/Flow_sizes",
        "/hr/Concat_flow_sizes",
        axis=0,
    )
    mask_sizes = add(
        "Concat",
        [full_b_v, "/hr/Channels_mask", full_h_v, full_w_v],
        "/hr/Mask_sizes",
        "/hr/Concat_mask_sizes",
        axis=0,
    )
    flow_up = add(
        "Resize",
        [flow_crop, "", "", flow_sizes],
        "/hr/Flow_up",
        "/hr/Resize_flow",
        mode="linear",
        coordinate_transformation_mode="half_pixel",
        nearest_mode="floor",
    )
    full_w_f = add(
        "Cast", [full_w], "/hr/Full_w_f", "/hr/Cast_full_w", to=TensorProto.FLOAT
    )
    full_h_f = add(
        "Cast", [full_h], "/hr/Full_h_f", "/hr/Cast_full_h", to=TensorProto.FLOAT
    )
    low_w_f = add(
        "Cast", [low_w], "/hr/Half_w_f", "/hr/Cast_half_w", to=TensorProto.FLOAT
    )
    low_h_f = add(
        "Cast", [low_h], "/hr/Half_h_f", "/hr/Cast_half_h", to=TensorProto.FLOAT
    )
    scale_x = add("Div", [full_w_f, low_w_f], "/hr/Scale_x", "/hr/Div_scale_x")
    scale_y = add("Div", [full_h_f, low_h_f], "/hr/Scale_y", "/hr/Div_scale_y")
    scale_x_v = add(
        "Unsqueeze",
        [scale_x, "/hr/Unsqueeze_zero"],
        "/hr/Scale_x_v",
        "/hr/Unsqueeze_scale_x",
    )
    scale_y_v = add(
        "Unsqueeze",
        [scale_y, "/hr/Unsqueeze_zero"],
        "/hr/Scale_y_v",
        "/hr/Unsqueeze_scale_y",
    )
    flow_scale_flat = add(
        "Concat",
        [scale_x_v, scale_y_v, scale_x_v, scale_y_v],
        "/hr/Flow_scale_flat",
        "/hr/Concat_flow_scale",
        axis=0,
    )
    flow_scale = add(
        "Reshape",
        [flow_scale_flat, "/hr/Reshape_flow_scale"],
        "/hr/Flow_scale",
        "/hr/Reshape_flow_scale_node",
    )
    flow_full = add("Mul", [flow_up, flow_scale], "/hr/Flow_full", "/hr/Mul_flow_scale")

    rgb6 = add(
        "Slice",
        [
            "input",
            "/hr/Slice_rgb_starts",
            "/hr/Slice_rgb_ends",
            "/hr/Slice_rgb_axes",
            "/hr/Slice_rgb_steps",
        ],
        "/hr/Rgb6",
        "/hr/Slice_rgb6",
    )
    two_b = add("Mul", [full_b, "/hr/Two_i64"], "/hr/Two_b", "/hr/Mul_two_b")
    two_b_v = add(
        "Unsqueeze", [two_b, "/hr/Unsqueeze_zero"], "/hr/Two_b_v", "/hr/Unsqueeze_two_b"
    )
    rgb_shape = add(
        "Concat",
        [two_b_v, "/hr/Channels_rgb", full_h_v, full_w_v],
        "/hr/Rgb_shape",
        "/hr/Concat_rgb_shape",
        axis=0,
    )
    rgb_pair = add("Reshape", [rgb6, rgb_shape], "/hr/Rgb_pair", "/hr/Reshape_rgb_pair")
    flow_pair_shape = add(
        "Concat",
        [two_b_v, "/hr/Channels_xy", full_h_v, full_w_v],
        "/hr/Flow_pair_shape",
        "/hr/Concat_flow_pair_shape",
        axis=0,
    )
    flow_pair = add(
        "Reshape",
        [flow_full, flow_pair_shape],
        "/hr/Flow_pair",
        "/hr/Reshape_flow_pair",
    )

    full_w_m1 = add(
        "Sub", [full_w, "/hr/One_i64"], "/hr/Full_w_m1", "/hr/Sub_full_w_m1"
    )
    full_h_m1 = add(
        "Sub", [full_h, "/hr/One_i64"], "/hr/Full_h_m1", "/hr/Sub_full_h_m1"
    )
    full_w_m1_f = add(
        "Cast",
        [full_w_m1],
        "/hr/Full_w_m1_f",
        "/hr/Cast_full_w_m1",
        to=TensorProto.FLOAT,
    )
    full_h_m1_f = add(
        "Cast",
        [full_h_m1],
        "/hr/Full_h_m1_f",
        "/hr/Cast_full_h_m1",
        to=TensorProto.FLOAT,
    )
    norm_x = add("Div", ["/hr/Two_f32", full_w_m1_f], "/hr/Norm_x", "/hr/Div_norm_x")
    norm_y = add("Div", ["/hr/Two_f32", full_h_m1_f], "/hr/Norm_y", "/hr/Div_norm_y")
    norm_x_v = add(
        "Unsqueeze",
        [norm_x, "/hr/Unsqueeze_zero"],
        "/hr/Norm_x_v",
        "/hr/Unsqueeze_norm_x",
    )
    norm_y_v = add(
        "Unsqueeze",
        [norm_y, "/hr/Unsqueeze_zero"],
        "/hr/Norm_y_v",
        "/hr/Unsqueeze_norm_y",
    )
    grid_scale_flat = add(
        "Concat",
        [norm_x_v, norm_y_v],
        "/hr/Grid_scale_flat",
        "/hr/Concat_grid_scale",
        axis=0,
    )
    grid_scale = add(
        "Reshape",
        [grid_scale_flat, "/hr/Reshape_grid_scale"],
        "/hr/Grid_scale",
        "/hr/Reshape_grid_scale_node",
    )
    flow_normalized = add(
        "Mul", [flow_pair, grid_scale], "/hr/Flow_normalized", "/hr/Mul_flow_normalized"
    )
    flow_nhwc = add(
        "Transpose",
        [flow_normalized],
        "/hr/Flow_nhwc",
        "/hr/Transpose_flow",
        perm=[0, 2, 3, 1],
    )

    range_x_i = add(
        "Range", ["/hr/Zero_i64", full_w, "/hr/One_i64"], "/hr/Range_x_i", "/hr/Range_x"
    )
    range_y_i = add(
        "Range", ["/hr/Zero_i64", full_h, "/hr/One_i64"], "/hr/Range_y_i", "/hr/Range_y"
    )
    range_x = add(
        "Cast", [range_x_i], "/hr/Range_x_f", "/hr/Cast_range_x", to=TensorProto.FLOAT
    )
    range_y = add(
        "Cast", [range_y_i], "/hr/Range_y_f", "/hr/Cast_range_y", to=TensorProto.FLOAT
    )
    base_x_01 = add("Mul", [range_x, norm_x], "/hr/Base_x_01", "/hr/Mul_base_x")
    base_y_01 = add("Mul", [range_y, norm_y], "/hr/Base_y_01", "/hr/Mul_base_y")
    base_x = add("Sub", [base_x_01, "/hr/One_f32"], "/hr/Base_x", "/hr/Sub_base_x")
    base_y = add("Sub", [base_y_01, "/hr/One_f32"], "/hr/Base_y", "/hr/Sub_base_y")
    base_x_3d = add(
        "Reshape", [base_x, "/hr/Reshape_x"], "/hr/Base_x_3d", "/hr/Reshape_base_x"
    )
    base_y_3d = add(
        "Reshape", [base_y, "/hr/Reshape_y"], "/hr/Base_y_3d", "/hr/Reshape_base_y"
    )
    grid_nhw = add(
        "Concat",
        [two_b_v, full_h_v, full_w_v],
        "/hr/Grid_nhw",
        "/hr/Concat_grid_nhw",
        axis=0,
    )
    base_x_nhw = add(
        "Expand", [base_x_3d, grid_nhw], "/hr/Base_x_nhw", "/hr/Expand_base_x"
    )
    base_y_nhw = add(
        "Expand", [base_y_3d, grid_nhw], "/hr/Base_y_nhw", "/hr/Expand_base_y"
    )
    base_x_4d = add(
        "Unsqueeze",
        [base_x_nhw, "/hr/Unsqueeze_last"],
        "/hr/Base_x_4d",
        "/hr/Unsqueeze_base_x",
    )
    base_y_4d = add(
        "Unsqueeze",
        [base_y_nhw, "/hr/Unsqueeze_last"],
        "/hr/Base_y_4d",
        "/hr/Unsqueeze_base_y",
    )
    base_grid = add(
        "Concat",
        [base_x_4d, base_y_4d],
        "/hr/Base_grid",
        "/hr/Concat_base_grid",
        axis=3,
    )
    sample_grid = add(
        "Add", [base_grid, flow_nhwc], "/hr/Sample_grid", "/hr/Add_sample_grid"
    )
    warped_pair = add(
        "GridSample",
        [rgb_pair, sample_grid],
        "/hr/Warped_pair",
        "/hr/GridSample_full",
        align_corners=1,
        mode="bilinear",
        padding_mode="border",
    )
    nodes.append(
        helper.make_node(
            "Split",
            [warped_pair],
            ["/hr/Warped_0", "/hr/Warped_1"],
            name="/hr/Split_warped",
            axis=0,
        )
    )
    mask_up = add(
        "Resize",
        [mask_crop, "", "", mask_sizes],
        "/hr/Mask_up",
        "/hr/Resize_mask_logit",
        mode="linear",
        coordinate_transformation_mode="half_pixel",
        nearest_mode="floor",
    )
    mask_probability = add(
        "Sigmoid", [mask_up], "/hr/Mask_probability", "/hr/Sigmoid_mask"
    )
    inverse_mask = add(
        "Sub",
        ["/hr/One_f32", mask_probability],
        "/hr/Inverse_mask",
        "/hr/Sub_inverse_mask",
    )
    weighted_0 = add(
        "Mul",
        ["/hr/Warped_0", mask_probability],
        "/hr/Weighted_0",
        "/hr/Mul_weighted_0",
    )
    weighted_1 = add(
        "Mul", ["/hr/Warped_1", inverse_mask], "/hr/Weighted_1", "/hr/Mul_weighted_1"
    )
    output = add("Add", [weighted_0, weighted_1], "output", "/hr/Add_output")

    model.graph.node.extend(nodes)
    prune_graph(model, [output])
    del model.graph.output[:]
    model.graph.output.extend(
        [helper.make_tensor_value_info(output, TensorProto.FLOAT, OUTPUT_SHAPE)]
    )
    model.graph.name = "rife_v4.26_halfmotion_0.5_fullwarp_logit"
    model.producer_name = "mpv-config RIFE half-motion prototype"
    model.producer_version = "1"
    model.doc_string = (
        "RIFE v4.26 motion/refinement at 0.5x linear resolution; "
        "final flow scaled to source coordinates; full-resolution endpoint warp; "
        "mask resized in logit domain."
    )
    helper.set_model_props(
        model,
        {
            "mpv.framegen.contract_id": CONTRACT_ID,
            "mpv.framegen.motion_scale": "0.5",
            "mpv.framegen.endpoint_warp": "full-resolution",
            "mpv.framegen.mask_resize_domain": "logit",
            "mpv.framegen.timestep": "input-plane",
        },
    )
    onnx.checker.check_model(model, full_check=True)
    return model


def value_shape(value: onnx.ValueInfoProto) -> list[int | str | None]:
    result: list[int | str | None] = []
    for dimension in value.type.tensor_type.shape.dim:
        if dimension.HasField("dim_value"):
            result.append(dimension.dim_value)
        elif dimension.HasField("dim_param"):
            result.append(dimension.dim_param)
        else:
            result.append(None)
    return result


def initializer_payload(value: onnx.TensorProto) -> bytes:
    clone = onnx.TensorProto()
    clone.CopyFrom(value)
    clone.name = ""
    return clone.SerializeToString()


def require_contract(model: onnx.ModelProto) -> None:
    if len(model.graph.input) != 1 or len(model.graph.output) != 1:
        fail("target model must have exactly one input and one output")
    input_value = model.graph.input[0]
    output_value = model.graph.output[0]
    actual = (
        input_value.name,
        input_value.type.tensor_type.elem_type,
        value_shape(input_value),
        output_value.name,
        output_value.type.tensor_type.elem_type,
        value_shape(output_value),
    )
    expected = (
        "input",
        TensorProto.FLOAT,
        INPUT_SHAPE,
        "output",
        TensorProto.FLOAT,
        OUTPUT_SHAPE,
    )
    if actual != expected:
        fail(f"target input/output contract mismatch: {actual!r}")


def internal_session(
    source: onnx.ModelProto, options: ort.SessionOptions
) -> ort.InferenceSession:
    model = deepcopy(source)
    flow = "/Add_7_output_0"
    mask = "/block4/Split_output_1"
    prune_graph(model, [flow, mask])
    del model.graph.output[:]
    model.graph.output.extend(
        [
            helper.make_tensor_value_info(
                flow,
                TensorProto.FLOAT,
                ["batch", 4, "padded_height", "padded_width"],
            ),
            helper.make_tensor_value_info(
                mask,
                TensorProto.FLOAT,
                ["batch", 1, "padded_height", "padded_width"],
            ),
        ]
    )
    onnx.checker.check_model(model, full_check=True)
    return ort.InferenceSession(
        model.SerializeToString(),
        sess_options=options,
        providers=["CPUExecutionProvider"],
    )


def deterministic_input(height: int, width: int) -> np.ndarray:
    x = np.linspace(0.0, 1.0, width, dtype=np.float32)[None, :]
    y = np.linspace(0.0, 1.0, height, dtype=np.float32)[:, None]
    first = np.stack(
        [
            np.broadcast_to(x, (height, width)),
            np.broadcast_to(y, (height, width)),
            np.broadcast_to((x + y) * 0.5, (height, width)),
        ]
    )
    second = np.roll(first, shift=(7, -11), axis=(1, 2)).copy()
    midpoint = np.full((1, height, width), 0.5, dtype=np.float32)
    return np.concatenate([first, second, midpoint], axis=0)[None]


def resize_half_pixel(
    value: np.ndarray,
    height: int,
    width: int,
    *,
    scales: tuple[float, float] | None = None,
) -> np.ndarray:
    source_height, source_width = value.shape[-2:]
    scale_y, scale_x = scales or (height / source_height, width / source_width)
    y = (np.arange(height, dtype=np.float32) + 0.5) / scale_y - 0.5
    x = (np.arange(width, dtype=np.float32) + 0.5) / scale_x - 0.5
    y0_raw = np.floor(y).astype(np.int32)
    x0_raw = np.floor(x).astype(np.int32)
    y_weight = (y - y0_raw).astype(np.float32).reshape(1, 1, height, 1)
    x_weight = (x - x0_raw).astype(np.float32).reshape(1, 1, 1, width)
    y0 = np.clip(y0_raw, 0, source_height - 1)
    x0 = np.clip(x0_raw, 0, source_width - 1)
    y1 = np.clip(y0_raw + 1, 0, source_height - 1)
    x1 = np.clip(x0_raw + 1, 0, source_width - 1)
    vertical = value[:, :, y0, :] * (1.0 - y_weight) + value[:, :, y1, :] * y_weight
    return vertical[:, :, :, x0] * (1.0 - x_weight) + vertical[:, :, :, x1] * x_weight


def warp_border(frame: np.ndarray, flow: np.ndarray) -> np.ndarray:
    channels, height, width = frame.shape
    sample_x = np.clip(
        flow[0] + np.arange(width, dtype=np.float32)[None, :],
        0.0,
        float(width - 1),
    )
    sample_y = np.clip(
        flow[1] + np.arange(height, dtype=np.float32)[:, None],
        0.0,
        float(height - 1),
    )
    x0 = np.floor(sample_x).astype(np.int32)
    y0 = np.floor(sample_y).astype(np.int32)
    x1 = np.minimum(x0 + 1, width - 1)
    y1 = np.minimum(y0 + 1, height - 1)
    x_weight = sample_x - x0
    y_weight = sample_y - y0
    output = np.empty((channels, height, width), dtype=np.float32)
    for channel in range(channels):
        plane = frame[channel]
        top = plane[y0, x0] * (1.0 - x_weight) + plane[y0, x1] * x_weight
        bottom = plane[y1, x0] * (1.0 - x_weight) + plane[y1, x1] * x_weight
        output[channel] = top * (1.0 - y_weight) + bottom * y_weight
    return output


def reference_output(
    full_input: np.ndarray,
    low_flow_padded: np.ndarray,
    low_mask_padded: np.ndarray,
    low_height: int,
    low_width: int,
) -> np.ndarray:
    _, _, full_height, full_width = full_input.shape
    flow = resize_half_pixel(
        low_flow_padded[:, :, :low_height, :low_width], full_height, full_width
    )
    flow *= np.asarray(
        [
            full_width / low_width,
            full_height / low_height,
            full_width / low_width,
            full_height / low_height,
        ],
        dtype=np.float32,
    ).reshape(1, 4, 1, 1)
    flow_pair = flow.reshape(2, 2, full_height, full_width)
    frames = full_input[:, :6].reshape(2, 3, full_height, full_width)
    warped = np.stack(
        [warp_border(frames[index], flow_pair[index]) for index in range(2)]
    )
    mask_logits = resize_half_pixel(
        low_mask_padded[:, :, :low_height, :low_width], full_height, full_width
    )
    mask = 1.0 / (1.0 + np.exp(-mask_logits))
    return warped[0:1] * mask + warped[1:2] * (1.0 - mask)


def semantic_rows(
    source: onnx.ModelProto,
    target_bytes: bytes,
) -> list[dict[str, object]]:
    options = ort.SessionOptions()
    options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    options.intra_op_num_threads = 4
    options.inter_op_num_threads = 1
    low_session = internal_session(source, options)
    target_session = ort.InferenceSession(
        target_bytes,
        sess_options=options,
        providers=["CPUExecutionProvider"],
    )
    rows: list[dict[str, object]] = []
    for height, width in VERIFY_SHAPES:
        full_input = deterministic_input(height, width)
        low_height, low_width = int(height * 0.5), int(width * 0.5)
        low_input = resize_half_pixel(
            full_input,
            low_height,
            low_width,
            scales=(0.5, 0.5),
        )
        low_flow, low_mask = low_session.run(None, {"input": low_input})
        expected = reference_output(
            full_input,
            low_flow,
            low_mask,
            low_height,
            low_width,
        )
        actual = target_session.run(None, {"input": full_input})[0]
        difference = np.abs(expected.astype(np.float64) - actual.astype(np.float64))
        mean_square = float(np.mean(np.square(difference)))
        row = {
            "input_hw": [height, width],
            "motion_hw": [low_height, low_width],
            "output_shape": list(actual.shape),
            "finite": bool(np.isfinite(actual).all()),
            "max_abs": float(difference.max()),
            "mean_abs": float(difference.mean()),
            "psnr_db": math.inf
            if mean_square == 0.0
            else -10.0 * math.log10(mean_square),
        }
        if (
            row["output_shape"] != [1, 3, height, width]
            or not row["finite"]
            or row["max_abs"] > MAX_ABSOLUTE_ERROR
            or row["psnr_db"] < MIN_PSNR_DB
        ):
            fail(f"semantic verification failed: {row}")
        rows.append(row)
    return rows


def rewrite_command(source_path: Path, output_path: Path) -> None:
    require_hash(source_path, SOURCE_MODEL_SHA256, "source model")
    model = rewrite(onnx.load(source_path))
    serialized = model.SerializeToString()
    actual = hashlib.sha256(serialized).hexdigest()
    if actual != TARGET_MODEL_SHA256:
        fail(
            f"rewritten model SHA-256 mismatch: expected {TARGET_MODEL_SHA256}, got {actual}"
        )
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(serialized)
    print(f"wrote {output_path}")
    print(f"sha256 {actual}")


def verify_command(source_path: Path, target_path: Path) -> None:
    require_hash(source_path, SOURCE_MODEL_SHA256, "source model")
    require_hash(target_path, TARGET_MODEL_SHA256, "target model")
    source = onnx.load(source_path)
    target = onnx.load(target_path)
    onnx.checker.check_model(target, full_check=True)
    inferred = onnx.shape_inference.infer_shapes(
        target, strict_mode=True, data_prop=True
    )
    onnx.checker.check_model(inferred, full_check=True)
    require_contract(target)

    reproduced = rewrite(source).SerializeToString()
    if reproduced != target_path.read_bytes():
        fail("target bytes differ from the deterministic rewrite result")

    source_initializers = {
        value.name: initializer_payload(value) for value in source.graph.initializer
    }
    target_initializers = {
        value.name: initializer_payload(value) for value in target.graph.initializer
    }
    changed_weights = sorted(
        name
        for name, payload in source_initializers.items()
        if target_initializers.get(name) != payload
    )
    removed_weights = sorted(source_initializers.keys() - target_initializers.keys())
    if changed_weights or removed_weights:
        fail(
            "source initializer integrity failed: "
            f"changed={changed_weights}, removed={removed_weights}"
        )

    report = {
        "provenance": {
            "source_archive_url": SOURCE_ARCHIVE_URL,
            "source_archive_sha256": SOURCE_ARCHIVE_SHA256,
            "practical_rife_release_commit": PRACTICAL_RIFE_RELEASE_COMMIT,
            "vs_rife_export_commit": VS_RIFE_EXPORT_COMMIT,
        },
        "source_sha256": SOURCE_MODEL_SHA256,
        "target_sha256": TARGET_MODEL_SHA256,
        "deterministic_rewrite": True,
        "onnx_full_check": True,
        "strict_shape_inference": True,
        "input": "input: float32[batch,7,height,width]",
        "output": "output: float32[batch,3,height,width]",
        "source_initializers_preserved": len(source_initializers),
        "shape_and_grid_initializers_added": len(target_initializers)
        - len(source_initializers),
        "semantic": semantic_rows(source, target_path.read_bytes()),
    }
    print(json.dumps(report, indent=2))


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    rewrite_parser = commands.add_parser("rewrite", help="build the pinned Arc graph")
    rewrite_parser.add_argument("--source", type=Path, required=True)
    rewrite_parser.add_argument("--output", type=Path, required=True)
    verify_parser = commands.add_parser(
        "verify", help="verify hashes, contract and semantics"
    )
    verify_parser.add_argument("--source", type=Path, required=True)
    verify_parser.add_argument("--target", type=Path, required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    if args.command == "rewrite":
        rewrite_command(args.source, args.output)
    else:
        verify_command(args.source, args.target)


if __name__ == "__main__":
    main()
