import { cp, lstat, mkdir, mkdtemp, rename, rm } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";

import { mediaTool } from "../components/content/media-runtime";

const { values } = parseArgs({ options: { destination: { type: "string" }, "app-dir": { type: "string" } } });
if (!values.destination || !values["app-dir"])
    throw new Error("Usage: bun scripts/prepare-media-lab.ts --app-dir <portable bundle> --destination <new external lab media directory>");

const destination = resolve(values.destination);
const appDir = resolve(values["app-dir"]);
const sourceRelative = relative(resolve(import.meta.dir, ".."), destination);
if (!isAbsolute(sourceRelative) && sourceRelative !== ".." && !sourceRelative.startsWith(`..${sep}`))
    throw new Error("Lab media output must be outside the source tree.");
if (await lstat(destination).catch(error => {
    if (error.code === "ENOENT") return null;
    throw error;
})) throw new Error("Lab media output already exists. Choose a new directory.");

await mkdir(dirname(destination), { recursive: true });
const temporary = await mkdtemp(join(dirname(destination), ".qbutt-lab-media-"));
const stage = join(temporary, "media");
// Preparation must exercise the production pins, even when launched from a lab.
delete process.env.QBUTT_LAB_PATHS;
delete process.env.QBUTT_MEDIA_RUNTIME_DIR;
try {
    await mediaTool(appDir, "mpv", AbortSignal.timeout(30_000));
    await cp(join(appDir, "media"), stage, { recursive: true });
    await rename(stage, destination);
    console.log(`Pinned lab media tools: ${destination}`);
}
finally { await rm(temporary, { recursive: true, force: true }); }
