import { execFile } from "node:child_process";
import { lstat } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";

import lock from "../../upstream-lock.json";
import { ContentError } from "./errors";
import { verifiedArtifact, type ArtifactPin } from "./product-artifacts";

type Product = "qplay" | "qcatch";

export function validateProductVersion(product: Product, info: unknown): void {
    const expected = lock.products[product];
    const value = info as { product?: string; version?: string; protocolVersion?: number };
    const version = /^(\d+)\.(\d+)\.(\d+)$/.exec(value?.version ?? "")?.slice(1).map(Number);
    const minimum = expected.version.split(".").map(Number);
    const older = !version || version.some((part, index) => part < minimum[index]!
        && version.slice(0, index).every((previous, previousIndex) => previous === minimum[previousIndex]));
    if (value?.product !== product || value.protocolVersion !== expected.protocolVersion || older)
        throw new ContentError(`${product}_incompatible`,
            `This version of ${product} is not compatible. Install ${product} ${expected.version} or newer.`);
}

export async function productExecutable(product: Product, appDir: string): Promise<string | undefined> {
    if (product === "qplay") {
        const portable = join(appDir, "qplay");
        if (await Bun.file(join(portable, "qplay.portable")).exists())
            return join(portable, "qplay.exe");
    }
    if (process.platform !== "win32") return;
    try {
        const { stdout } = await promisify(execFile)("reg.exe", ["query",
            `HKCU\\Software\\qbutt-org\\${product}`, "/v", "InstallDir", "/reg:64"], {
            windowsHide: true, timeout: 5_000, maxBuffer: 16 * 1024,
        });
        const directory = /^\s*InstallDir\s+REG_SZ\s+(.+)$/m.exec(stdout)?.[1]?.trim();
        if (!directory) return;
        const executable = join(directory, `${product}.exe`);
        if ((await lstat(executable)).isFile()) return executable;
    }
    catch { /* The optional product is not installed. */ }
}

export async function connectQcatch(appDir: string, browser: "edge" | "chrome", signal: AbortSignal) {
    let executable = await productExecutable("qcatch", appDir);
    let compatible = false;
    if (executable) {
        try {
            const { stdout } = await promisify(execFile)(executable, ["--version-json"], {
                windowsHide: true, timeout: 10_000, maxBuffer: 16 * 1024, signal,
            });
            validateProductVersion("qcatch", JSON.parse(stdout));
            compatible = true;
        }
        catch { signal.throwIfAborted(); }
    }
    if (!compatible) {
        const product = lock.products.qcatch;
        const pin = (product.artifacts as { setup?: ArtifactPin }).setup;
        if (!pin) throw new ContentError("qcatch_unavailable", "The qcatch release pin is missing. Reinstall qbutt and try again.");
        const setup = join(process.env.LOCALAPPDATA ?? appDir, "qbutt", "cache", "components", `qcatch-${product.version}-setup.exe`);
        await verifiedArtifact(pin, setup, signal);
        signal.throwIfAborted();
        // An installer must finish its own transaction even if this request is canceled.
        await promisify(execFile)(setup, ["/CURRENTUSER", "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"], {
            windowsHide: true, timeout: 180_000, maxBuffer: 16 * 1024,
        }).catch(() => {
            throw new ContentError("qcatch_unavailable", "qcatch installation did not complete. Install qcatch and try again.");
        });
        executable = await productExecutable("qcatch", appDir);
        if (!executable) throw new ContentError("qcatch_unavailable", "qcatch installation did not complete. Install qcatch and try again.");
        const { stdout } = await promisify(execFile)(executable, ["--version-json"], {
            windowsHide: true, timeout: 10_000, maxBuffer: 16 * 1024, signal,
        });
        validateProductVersion("qcatch", JSON.parse(stdout));
    }
    const { stdout } = await promisify(execFile)(executable!, ["--connect-browser", browser], {
        windowsHide: true, timeout: 30_000, maxBuffer: 64 * 1024, signal,
    }).catch(() => {
        signal.throwIfAborted();
        throw new ContentError("qcatch_unavailable", "qcatch could not connect this browser. Reinstall qcatch and try again.");
    });
    const result = JSON.parse(stdout);
    if (result.browser !== browser || typeof result.extensionDirectory !== "string"
        || result.extensionId !== "jnmnnglmejlbjnjmmjampnhfncpcbffh" || result.nativeHost !== "org.qbutt.qcatch")
        throw new ContentError("qcatch_unavailable", "qcatch returned an incomplete browser setup. Reinstall qcatch and try again.");
    return result as { browser: string; extensionDirectory: string };
}
