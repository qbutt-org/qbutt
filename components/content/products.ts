import { execFile } from "node:child_process";
import { lstat } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";

import lock from "../../upstream-lock.json";
import { ContentError } from "./errors";

export function validateProductVersion(info: unknown): void {
    const expected = lock.products.qplay;
    const value = info as { product?: string; version?: string; protocolVersion?: number };
    const version = /^(\d+)\.(\d+)\.(\d+)$/.exec(value?.version ?? "")?.slice(1).map(Number);
    const minimum = expected.version.split(".").map(Number);
    const older = !version || version.some((part, index) => part < minimum[index]!
        && version.slice(0, index).every((previous, previousIndex) => previous === minimum[previousIndex]));
    if (value?.product !== "qplay" || value.protocolVersion !== expected.protocolVersion || older)
        throw new ContentError("qplay_incompatible",
            `This version of qplay is not compatible. Install qplay ${expected.version} or newer.`);
}

export async function productExecutable(appDir: string): Promise<string | undefined> {
    const portable = join(appDir, "qplay");
    if (await Bun.file(join(portable, "qplay.portable")).exists())
        return join(portable, "qplay.exe");
    if (process.platform !== "win32") return;
    try {
        const { stdout } = await promisify(execFile)("reg.exe", ["query",
            "HKCU\\Software\\qbutt-org\\qplay", "/v", "InstallDir", "/reg:64"], {
            windowsHide: true, timeout: 5_000, maxBuffer: 16 * 1024,
        });
        const directory = /^\s*InstallDir\s+REG_SZ\s+(.+)$/m.exec(stdout)?.[1]?.trim();
        if (!directory) return;
        const executable = join(directory, "qplay.exe");
        if ((await lstat(executable)).isFile()) return executable;
    }
    catch { /* The optional product is not installed. */ }
}
