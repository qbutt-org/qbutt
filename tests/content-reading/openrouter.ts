import { mkdir, readFile, writeFile } from "node:fs/promises";
import { isAbsolute, join, resolve } from "node:path";

import { labAppearanceSettings } from "../appearance";

const [application, seed, evidence] = process.argv.slice(2);
if (!application || !seed || !evidence || ![application, seed, evidence].every(isAbsolute)) {
    throw new Error("Pass absolute acceptance executable, native settings seed and fresh evidence directory.");
}

await mkdir(evidence);
const runs = [];
for (const language of ["en", "ru"]) {
    for (const appearance of ["light", "dark"]) {
        const profile = join(evidence, `${language}-${appearance}`);
        const seedProcess = Bun.spawn([seed, "--seed-reader", profile], {
            stdout: Bun.file(join(evidence, `${language}-${appearance}-seed.log`)),
            stderr: "pipe",
            windowsHide: true,
        });
        if (await seedProcess.exited !== 0) {
            throw new Error(`Native settings seed failed: ${await new Response(seedProcess.stderr).text()}`);
        }
        await writeFile(join(profile, "qBittorrent.ini"), labAppearanceSettings("product").join("\n"));
        const child = Bun.spawn([
            application, "--reader-openrouter", "--lang", language, "--profile", profile,
            ...(appearance === "dark" ? ["--dark"] : []),
        ], {
            env: {
                ...process.env,
                ProgramFiles: profile,
                LOCALAPPDATA: profile,
                OPENROUTER_API_KEY: "generated-env-key-must-not-be-used",
                QT_QPA_PLATFORM: "offscreen",
                QT_QUICK_BACKEND: "software",
                QT_QPA_FONTDIR: "C:/Windows/Fonts",
                QT_FORCE_STDERR_LOGGING: "1",
            },
            stdout: Bun.file(join(profile, "stdout.log")),
            stderr: Bun.file(join(profile, "stderr.log")),
            windowsHide: true,
        });
        const timeout = setTimeout(() => child.kill(), 120_000);
        const exitCode = await child.exited;
        clearTimeout(timeout);
        const log = await readFile(join(profile, "stderr.log"), "utf8");
        console.log(`${language} ${appearance}: ${log}`);
        runs.push({ language, appearance, exitCode });
        if (exitCode !== 0 || !log.includes("PASS reader_openrouter")) {
            throw new Error(`Native OpenRouter reader acceptance failed with exit ${exitCode}.`);
        }
    }
}
await writeFile(join(evidence, "receipt.json"), JSON.stringify({
    executable: resolve(application),
    sha256: new Bun.CryptoHasher("sha256").update(await Bun.file(application).arrayBuffer()).digest("hex"),
    runs,
}, null, 2));
