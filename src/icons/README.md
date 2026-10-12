# qbutt interface icons

Most qbutt interface glyphs use a 24 × 24 viewBox in a 32 × 32 SVG, with rounded 1.8-unit strokes. Keep the shape legible at 16 pixels. Neutral strokes are `#465669` in the root file and `#c8d4df` in `dark/`. Reserve the blue accent (`#009df7` in light mode) for the active part of an action; use semantic colors only for status and destructive actions.

`chevron-left.svg`, `chevron-right.svg`, `chevron-down.svg`, and `close.svg` are the outline `chevron-left`, `chevron-right`, `chevron-down`, and `x` icons from [Tabler Icons](https://github.com/tabler/tabler-icons/tree/bbed884d15354b5cebf2493371f20dc2d5e83eaf/icons/outline), version 3.49.0. Their upstream 24 × 24 geometry and 2-unit stroke are unchanged; only `currentColor` was replaced with the light or dark neutral stroke for Qt SVG rendering. The exact revision is in `upstream-lock.json`, and the full MIT license is in `LICENSE-Tabler` beside these assets.

The built-in theme selects `dark/<name>.svg` automatically. Add both variants to `icons.qrc` when creating a new interface icon. Brand and flag assets keep their own colors.
