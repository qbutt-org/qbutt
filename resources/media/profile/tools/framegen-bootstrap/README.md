# Framegen in-process bootstrap

`framegen-bootstrap.dll` is a small Windows mpv C-plugin loaded on demand by `scripts/framegen.lua`. It reads only the active runtime manifest under the `QBUTT_MEDIA_RUNTIME` directory supplied by qbutt Player, independently validates its canonical managed layout, preloads that exact `vsscript.dll`, and acknowledges the exact runtime through mpv `user-data`. Player keeps this frame generation directory inside the separate qbutt data profile.

It does not discover runtime paths from `PATH`, registry values, command arguments, or arbitrary script messages, and it does not change mpv's process-wide DLL search policy. The Lua request may select only the active VSScript path or the manager's one retained ABI-identical compatibility generation; the plugin revalidates either choice against the fixed manifest and marker. Backend plugins load their version-local dependencies through their own exact-path loaders. A different or foreign VapourSynth runtime cannot replace one already loaded in the process, so only that incompatible case fails closed and requires one honest reopen.

Rebuild recipe for the dependency-free x64 DLL:

```powershell
.\resources\media\profile\tools\framegen-bootstrap\package.ps1
```

The package script statically links the MSVC runtime and verifies the x64 PE contract itself: exports `mpv_open_cplugin`, `pfn_mpv_get_property`, `pfn_mpv_set_property_string`, and `pfn_mpv_free_node_contents`, with only the expected Windows system DLL dependencies.

