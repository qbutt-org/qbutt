# qbutt interface icons

Common action, navigation and status icons use [Tabler Icons](https://github.com/tabler/tabler-icons/tree/bbed884d15354b5cebf2493371f20dc2d5e83eaf/icons/outline), version 3.49.0. Preserve upstream 24 × 24 geometry and 2-unit strokes; each SVG's `icon-tabler-*` class identifies its original icon. The exact revision is in `upstream-lock.json`, and the full MIT license is in `LICENSE-Tabler` beside these assets. A few torrent-specific glyphs retain their existing assets until a suitable library icon is selected.

Only colors are adapted for Qt SVG rendering. Neutral strokes are `#465669` in the root file and `#c8d4df` in `dark/`. Reserve the blue accent (`#009df7` in light mode) for the active part of an action; use semantic colors for status and destructive actions. Verify legibility at the displayed size, including 16 pixels.

The built-in theme selects `dark/<name>.svg` automatically. Add both variants to `icons.qrc` when creating a new interface icon. Brand and flag assets keep their own colors.
