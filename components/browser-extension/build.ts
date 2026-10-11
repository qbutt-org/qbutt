import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join, resolve } from "node:path";
import { CHROMIUM_EXTENSION_ID, FIREFOX_EXTENSION_ID } from "../content/browser/protocol.ts";

const destination = resolve(process.argv[2] ?? join(import.meta.dir, "dist"));
const manifest = JSON.parse(await readFile(join(import.meta.dir, "manifest.json"), "utf8"));
const id = [...createHash("sha256").update(Buffer.from(manifest.key, "base64")).digest().subarray(0, 16)]
    .map((value) => String.fromCharCode(97 + (value >> 4), 97 + (value & 15))).join("");
if (id !== CHROMIUM_EXTENSION_ID) throw new Error("Extension manifest key and native host ID differ");

for (const browser of ["chromium", "firefox"]) {
    const outdir = join(destination, browser);
    await mkdir(outdir, { recursive: true });
    const build = await Bun.build({
        entrypoints: ["background", "sniffer", "content", "parser"].map((name) => join(import.meta.dir, "src", `${name}.ts`)),
        outdir,
        target: "browser",
        format: "iife",
        minify: false,
        splitting: false,
    });
    if (!build.success) throw new AggregateError(build.logs, "Browser extension build failed");
    const output = structuredClone(manifest);
    if (browser === "firefox") {
        delete output.key;
        output.background = { scripts: ["background.js"] };
        output.browser_specific_settings = { gecko: { id: FIREFOX_EXTENSION_ID, strict_min_version: "128.0", data_collection_permissions: { required: ["none"] } } };
    }
    await writeFile(join(outdir, "manifest.json"), JSON.stringify(output, null, 4) + "\n");
    for (const file of ["LICENSE", "NOTICE", "parser.html"]) await cp(join(import.meta.dir, file), join(outdir, file));
}
console.log(`Browser connection built: ${destination}`);
