# Intel Arc RIFE model

`rife426_arc_s0p5.py` is the development-only, deterministic source for the Intel OpenVINO model stored at `framegen/models/openvino/rife_v2/rife_v4.26.onnx`.

The script does not install or invoke the player runtime. Its Python packages are pinned in the PEP 723 header and are resolved only when a developer explicitly runs it through `uv`.

## Reproduce

Extract `rife_v2/rife_v4.26.onnx` from the pinned source archive, then run:

```powershell
uv run .\tools\framegen-models\rife426_arc_s0p5.py rewrite `
  --source C:\path\to\regular\rife_v4.26.onnx `
  --output .\framegen\models\openvino\rife_v2\rife_v4.26.onnx
```

The command rejects every source except SHA-256 `af25762dfec02a4bbb949decea63988b01fa56c46c0ff9dc66ac8e2f12cbb661` and rejects its own output unless it is byte-for-byte SHA-256 `3a958289369a5645339b2fbd5f3a228bc82075866c2b4723e4ca485fda72fc6b`.

## Verify

```powershell
uv run .\tools\framegen-models\rife426_arc_s0p5.py verify `
  --source C:\path\to\regular\rife_v4.26.onnx `
  --target .\framegen\models\openvino\rife_v2\rife_v4.26.onnx
```

Verification performs the full ONNX checker and strict shape inference, reproduces and compares the complete serialized graph, checks that all 158 learned source initializers remain byte-identical, and compares the final graph with an independently implemented full-resolution warp at odd 641x359, 1280x720 and 1920x1080 shapes.

## Graph contract

- Input: `input`, `float32[batch, 7, height, width]`. Channels are RGB endpoint A, RGB endpoint B and the timestep plane.
- Output: `output`, `float32[batch, 3, height, width]`.
- Motion/refinement runs after a half-pixel resize to `floor(height / 2) x floor(width / 2)`.
- Flow is resized back and scaled in pixel units. Mask logits are resized before sigmoid.
- The final border-bilinear warp and blend use the original full-resolution endpoints, so output height and width always match the input, including odd dimensions.

This internal motion scale is the only graph variant. Outer playback `process_scale` remains `1`; applying another downscale would duplicate the optimization and lose endpoint detail.

## Provenance and licenses

- Source archive: [vs-mlrt external RIFE 4.26 model](https://github.com/AmusementClub/vs-mlrt/releases/download/external-models/rife_v4.26.7z), SHA-256 `dfdabd84a2a3db773f87604b8cc255e94a6a72f13550d910ccd3b4ee2606cd4f`.
- Source ONNX: 22,748,049 bytes, SHA-256 `af25762dfec02a4bbb949decea63988b01fa56c46c0ff9dc66ac8e2f12cbb661`.
- Upstream model: [Practical-RIFE 4.26](https://github.com/hzwer/Practical-RIFE/commit/0fa2b515e0a067a04a82a52c8dad077cff5eb3c5), published 2024-09-21.
- ONNX export provenance: [vs-rife commit 3488617](https://github.com/HolyWu/vs-rife/commit/3488617283db7c428a83ba4a19382285da698b6a).
- Derived ONNX: 22,751,073 bytes, SHA-256 `3a958289369a5645339b2fbd5f3a228bc82075866c2b4723e4ca485fda72fc6b`.

No learned weights are changed: the rewrite preserves every source initializer and adds only 26 shape/grid constants plus the full-resolution flow, warp and blend graph.

Practical-RIFE states that its linked trained models use the project MIT license. Exact repository copies are `tools/framegen-manager/notices/Practical-RIFE-MIT.txt` and `tools/framegen-manager/notices/vs-rife-MIT.txt`; the installed runtime keeps the same two filenames in its `notices` directory.
