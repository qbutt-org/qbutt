import assert from "node:assert/strict";
import { Database } from "bun:sqlite";
import { cp, lstat, mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { labAppearanceSettings } from "../appearance";
import { generateFixtures, sha256 } from "../fixtures/generate";
import { allowLabNetwork } from "../windows-firewall";

const executable = process.env.QBUTT_QT_ACCEPTANCE_EXE;
const service = process.env.QBUTT_PROFILE_DRIVER;
const python = process.env.QBUTT_LAB_PYTHON;
assert(executable && service && python, "Set QBUTT_QT_ACCEPTANCE_EXE, QBUTT_PROFILE_DRIVER and QBUTT_LAB_PYTHON");
const bundle = resolve(process.env.QBUTT_QT_ACCEPTANCE_BUNDLE ?? dirname(executable));
const root = await mkdtemp(join(await realpath(tmpdir()), "qbutt-profile-qt-"));
const profile = join(root, "profile");
const screenshots = join(root, "screenshots");
const pathKey = Object.keys(process.env).find(key => key.toLowerCase() === "path") ?? "PATH";
const environment = { ...process.env, QT_QPA_PLATFORM: "offscreen", QT_SCALE_FACTOR: "1", QT_PLUGIN_PATH: bundle,
    [pathKey]: `${bundle};${process.env[pathKey] ?? ""}` };
let child: ReturnType<typeof Bun.spawn> | undefined;
const evidence: Record<string, unknown> = { suite: "profile-import-qt", status: "running", checks: [] };

async function snapshot(directory: string): Promise<Record<string, unknown>> {
    const files: Record<string, unknown> = {};
    async function visit(relative: string) {
        for (const entry of await readdir(join(directory, relative), { withFileTypes: true })) {
            const path = join(relative, entry.name);
            assert(!entry.isSymbolicLink(), "Source snapshot encountered a link");
            if (entry.isDirectory()) await visit(path);
            else {
                const info = await lstat(join(directory, path));
                files[path] = { sha256: sha256(await readFile(join(directory, path))), size: info.size, modified: info.mtimeMs };
            }
        }
    }
    await visit("");
    return files;
}

try {
    await allowLabNetwork([executable]);
    evidence.executableSha256 = sha256(await readFile(executable));
    evidence.serviceSha256 = sha256(await readFile(service));
    const fixtures = await generateFixtures(python, join(root, "fixtures"));
    child = Bun.spawn([service, "seed-profile", root, "source", "db", fixtures, "v1,v2,hybrid"], {
        cwd: bundle, env: environment, windowsHide: true, stdout: "pipe", stderr: "pipe", timeout: 90000,
    });
    const [code, output, error] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
    await writeFile(join(root, "source-generation.log"), output + error);
    assert.equal(code, 0, `Native fixture generator failed; inspect ${root}`);
    const source = JSON.parse(output.trim().split("\n").at(-1)!) as { settings: string; data: string };
    for (const kind of ["v1", "v2", "hybrid"])
        await cp(join(fixtures, "seed"), join(root, "source-payload", kind), { recursive: true });
    await writeFile(join(root, "source-payload", "unknown.keep"), "No imported policy owns this file.\n");
    for (const kind of ["schema", "metadata"]) {
        const directory = join(root, `${kind}-data`);
        await cp(source.data, directory, { recursive: true });
        const db = new Database(join(directory, "torrents.db"));
        try {
            const changed = db.run(kind === "schema" ? "UPDATE meta SET value = '999' WHERE name = 'version'"
                : "UPDATE torrents SET metadata = NULL WHERE rowid = (SELECT min(rowid) FROM torrents)");
            assert.equal(changed.changes, 1, `Failed to prepare the ${kind} refusal fixture`);
            db.exec("PRAGMA wal_checkpoint(TRUNCATE)");
        }
        finally { db.close(); }
    }
    const sourceRoots = ["source", "source-payload", "schema-data", "metadata-data"];
    const before = new Map(await Promise.all(sourceRoots.map(async name => [name, await snapshot(join(root, name))] as const)));
    const config = join(profile, "qbutt", "config");
    await mkdir(config, { recursive: true });
    await mkdir(screenshots);
    await writeFile(join(config, "qbutt.ini"), [
        "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
        "Session\\InterfaceAddress=127.0.0.1", "Session\\AddTorrentStopped=true", "Session\\ResumeDataStorageType=SQLite",
        "Session\\AddExtensionToIncompleteFiles=false", "Session\\UseUnwantedFolder=false", "Session\\QueueingSystemEnabled=false",
        "[Network]", "PortForwardingEnabled=false", "[GUI]", "Notifications\\Enabled=false",
        "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false", "Connection\\ResolvePeerCountries=false",
        "Connection\\ResolvePeerHostNames=false", "General\\ExitConfirm=false", "General\\CloseToTray=false",
        "General\\MinimizeToTray=false", "General\\SystrayEnabled=false", "WebUI\\Enabled=false",
        ...labAppearanceSettings("functional"), "",
    ].join("\n"));
    for (const phase of ["prepare", "recheck", "acknowledge", "replay"]) {
        const spec = join(root, `${phase}-spec.json`);
        const evidencePath = join(root, `${phase}-evidence.json`);
        await writeFile(spec, JSON.stringify({ schema: 1, mode: "profile-import", phase, evidencePath, screenshots,
            fixtureRoot: root, sourceSettings: source.settings, validData: source.data,
            schemaData: join(root, "schema-data"), metadataData: join(root, "metadata-data"),
            receipt: join(root, "acknowledgement.json") }, null, 2));
        child = Bun.spawn([executable, `--profile=${profile}`, "--no-splash"], {
            cwd: bundle, env: { ...environment, QBUTT_QT_ACCEPTANCE_SPEC: spec }, windowsHide: true, timeout: 120000,
            stdout: Bun.file(join(root, `${phase}-stdout.log`)), stderr: Bun.file(join(root, `${phase}-stderr.log`)),
        });
        const exitCode = await child.exited;
        const phaseEvidence = JSON.parse(await readFile(evidencePath, "utf8"));
        assert.equal(exitCode, 0, `Profile UI ${phase} exited ${exitCode}: ${phaseEvidence.error ?? "see process logs"}; inspect ${root}`);
        assert.equal(phaseEvidence.status, "passed");
        assert(phaseEvidence.checks.some((check: { name: string }) => check.name === `import-${phase === "prepare" ? "prepared" : phase}`),
            "The Qt driver did not run the requested import phase");
        for (const name of sourceRoots)
            assert.deepEqual(await snapshot(join(root, name)), before.get(name), `${phase} changed source ${name}`);
        (evidence.checks as unknown[]).push({ phase, sourceProfilesAndPayloadUnchanged: true, checks: phaseEvidence.checks });
        console.log(JSON.stringify({ phase, status: "passed", evidence: evidencePath }));
    }
    evidence.status = "passed";
}
catch (error) {
    evidence.status = "failed";
    evidence.error = error instanceof Error ? error.message : String(error);
    process.exitCode = 1;
}
finally {
    if (child?.exitCode === null) { child.kill(); await child.exited; }
    // Keep only compact evidence and screenshots; never clean outside this lab.
    try {
        for (const entry of await readdir(root, { withFileTypes: true })) {
            if (!entry.isDirectory() || entry.name === "screenshots") continue;
            const target = resolve(root, entry.name);
            assert(!entry.isSymbolicLink() && dirname(target) === root && await realpath(target) === target,
                "Cleanup escaped the generated profile fixture");
            await rm(target, { recursive: true, force: true });
        }
    }
    catch (error) {
        evidence.status = "failed";
        evidence.cleanupError = error instanceof Error ? error.message : String(error);
        process.exitCode = 1;
    }
    await writeFile(join(root, "evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
    console.log(JSON.stringify({ status: evidence.status, evidence: join(root, "evidence.json") }));
}
