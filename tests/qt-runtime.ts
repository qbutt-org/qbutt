import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { join } from "node:path";

const required = [
    "Qt6Core.dll", "Qt6Gui.dll", "Qt6Network.dll", "Qt6Sql.dll",
    "Qt6Svg.dll", "Qt6Widgets.dll", "Qt6Xml.dll",
    "platforms/qoffscreen.dll", "platforms/qwindows.dll", "qt.conf",
];

export function assertQtRuntime(directory: string): void {
    if (process.platform !== "win32")
        return;
    const missing = required.filter(file => !existsSync(join(directory, file)));
    assert(missing.length === 0,
        `Incomplete Qt runtime at ${directory}: missing ${missing.join(", ")}. `
        + "Use the deployed portable bundle from scripts/build-windows.ps1 before launching the native lab.");
}
