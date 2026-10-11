import { cp, lstat, mkdir, readdir, realpath } from "node:fs/promises";
import { basename, isAbsolute, join, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";

import lock from "../upstream-lock.json";
import { verifiedArtifact, type ArtifactPin } from "../components/content/product-artifacts";

const { values } = parseArgs({ options: {
    destination: { type: "string" }, cache: { type: "string" },
} });
if (!values.destination || !values.cache)
    throw new Error("Usage: bun scripts/package-products.ts --destination <portable bundle> --cache <cache>");
const destination = resolve(values.destination);
const repository = resolve(import.meta.dir, "..");
const withinSource = relative(repository, destination);
if (!isAbsolute(withinSource) && withinSource !== ".." && !withinSource.startsWith(`..${sep}`))
    throw new Error("Component output must be outside the source tree.");
const cache = resolve(values.cache);
const signal = AbortSignal.timeout(20 * 60_000);
const products = lock.products as Record<string, {
    commit?: string; protocolVersion: number;
    artifacts: { setup?: ArtifactPin; portable?: ArtifactPin; runtimeSources?: ArtifactPin };
}>;

for (const [name, product] of Object.entries(products)) {
    if (!/^[0-9a-f]{40}$/.test(product.commit ?? "") || product.protocolVersion !== 1
        || !product.artifacts.setup
        || (name === "qplay" && (!product.artifacts.portable || !product.artifacts.runtimeSources)))
        throw new Error(`The ${name} source and release pins are incomplete.`);
}
await mkdir(cache, { recursive: true });
await mkdir(join(destination, "components"), { recursive: true });

const tar = join(process.env.SystemRoot ?? "C:/Windows", "System32", "tar.exe");
for (const [name, product] of Object.entries(products)) {
    for (const kind of ["setup", "portable", "runtimeSources"] as const) {
        const pin = product.artifacts[kind];
        if (!pin || (name === "qcatch" && (kind === "setup" || kind === "portable"))) continue;
        const filename = basename(new URL(pin.url).pathname);
        const downloaded = join(cache, filename);
        await verifiedArtifact(pin, downloaded, signal);
        if (kind === "setup") {
            await cp(downloaded, join(destination, "components", `${name}-setup.exe`));
        }
        else if (kind === "portable") {
            const output = join(destination, name);
            if (await Bun.file(join(output, `${name}.exe`)).exists())
                throw new Error("Component packaging requires a clean bundle.");
            const listing = Bun.spawn([tar, "-tf", downloaded], { stdout: "pipe", stderr: "inherit", windowsHide: true });
            const entries = (await new Response(listing.stdout).text()).trimEnd().split(/\r?\n/);
            if (await listing.exited !== 0 || !entries.length || entries.some(entry =>
                !entry || entry.includes("\\") || entry.includes(":") || entry.startsWith("/")
                || entry.split("/").some(part => part === ".." || part === ".")))
                throw new Error("The qplay archive contains an unsafe path.");
            await mkdir(output);
            const extraction = Bun.spawn([tar, "-xf", downloaded, "-C", await realpath(output)], { stdout: "inherit", stderr: "inherit", windowsHide: true });
            if (await extraction.exited !== 0) throw new Error("The qplay archive could not be extracted.");
            const pending = [output];
            while (pending.length) {
                const directory = pending.pop()!;
                for (const entry of await readdir(directory)) {
                    const path = join(directory, entry);
                    const info = await lstat(path);
                    if (info.isSymbolicLink() || !(info.isFile() || info.isDirectory()))
                        throw new Error("The qplay archive contains a non-regular entry.");
                    if (info.isDirectory()) pending.push(path);
                }
            }
            if (!await Bun.file(join(output, "qplay.exe")).exists()
                || !await Bun.file(join(output, "qplay.portable")).exists())
                throw new Error("The qplay portable archive is incomplete.");
        }
    }
}
console.log(`Pinned qplay packaged; runtime sources verified in cache: ${destination}`);
