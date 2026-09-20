import assert from "node:assert/strict";
import { cp, open, readFile, readdir, rename, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createLab, verifyPayload, waitFor, type TorrentStatus } from "../lab";
import { sha256 } from "../fixtures/generate";
import { allowLabNetwork } from "../windows-firewall";

const driver = process.env.QBUTT_PROFILE_DRIVER;
assert(driver, "Set QBUTT_PROFILE_DRIVER to the standalone native service integration executable");
await allowLabNetwork([driver]);
const lab = await createLab("profile-import");

async function run(mode: string, target: string, args: string[] = [], expected = 0) {
    const child = Bun.spawn([driver!, mode, lab.root, target, ...args], { stdout: "pipe", stderr: "pipe", timeout: 90000, windowsHide: true });
    const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
    await writeFile(join(lab.root, `${target}-${mode}.log`), stdout + stderr);
    assert.equal(code, expected, `${target}/${mode}: ${stdout} ${stderr}`);
    return JSON.parse(stdout.trim().split("\n").at(-1)!) as Record<string, any>;
}

async function snapshot(path: string): Promise<Record<string, { hash: string; size: number; modified: number }>> {
    const result: Record<string, { hash: string; size: number; modified: number }> = {};
    async function visit(relative: string) {
        for (const entry of await readdir(join(path, relative), { withFileTypes: true })) {
            if (entry.name === "profile-import.lock") continue; // A crashed process's stale lock is expected to be retired.
            const name = join(relative, entry.name);
            assert(!entry.isSymbolicLink(), "Profile fixture contains a link");
            if (entry.isDirectory()) await visit(name);
            else {
                const info = await stat(join(path, name));
                result[name] = { hash: sha256(await readFile(join(path, name))), size: info.size, modified: info.mtimeMs };
            }
        }
    }
    await visit("");
    return result;
}

try {
    const sources: Record<string, string[]> = {};
    for (const backend of ["legacy", "db"]) {
        const name = `source-${backend}`;
        const generated = await run("seed-profile", name, [backend, lab.fixtures, "v1,v2,hybrid"]);
        for (const kind of ["v1", "v2", "hybrid"])
            await cp(join(lab.fixtures, "seed"), join(lab.root, `${name}-payload`, kind), { recursive: true });
        sources[backend] = [generated.settings, generated.data, lab.root];
    }
    const beforeSources = {
        legacy: await snapshot(join(lab.root, "source-legacy")),
        db: await snapshot(join(lab.root, "source-db")),
        legacyPayload: await snapshot(join(lab.root, "source-legacy-payload")),
        dbPayload: await snapshot(join(lab.root, "source-db-payload")),
    };

    for (const fault of ["no-consent", "bad-index", "protected", "overlap"])
        assert.equal((await run("prepare", `reject-${fault}`, [...sources.legacy!, fault], 2)).success, false);
    const payloadWriter = await open(join(lab.root, "source-legacy-payload", "v1", "bundle", "alpha.bin"), "r+");
    try { assert.equal((await run("prepare", "reject-active-writer", sources.legacy!, 2)).success, false); }
    finally { await payloadWriter.close(); }
    const databaseWriter = await open(join(lab.root, "source-db", "qbutt", "data", "torrents.db"), "r+");
    try { await run("prepare", "reject-active-source-db", sources.db!, 1); }
    finally { await databaseWriter.close(); }
    await lab.checkpoint({ check: "admission-negatives", passed: true, cases: ["no-consent", "bad-index", "protected", "overlap", "active-payload-writer", "active-source-database"] });

    for (const aliasExists of [true, false]) {
        const target = aliasExists ? "reject-alias" : "mapped-incomplete";
        const payload = join(lab.root, `${target}-payload`);
        await cp(join(lab.fixtures, "seed"), payload, { recursive: true });
        const original = join(payload, "bundle", "alpha.bin");
        if (aliasExists) await cp(original, original + ".!qB");
        else await rename(original, original + ".!qB");
        const before = await snapshot(payload);
        await run("prepare", target, [...sources.legacy!, "mapped-alias"], aliasExists ? 2 : 0);
        if (!aliasExists) {
            await run("recover", target);
            const inspected = await run("inspect", target);
            assert.equal(inspected.torrents.length, 1);
            assert.equal(inspected.torrents[0].renamedCount, 1);
        }
        assert.deepEqual(await snapshot(payload), before, "Mapping admission changed payload files");
    }
    for (const backend of ["legacy", "db"]) {
        const source = `relative-${backend}`;
        const generated = await run("seed-profile", source, [backend, lab.fixtures, "v1", "--relative-path"]);
        await cp(join(lab.fixtures, "seed"), join(lab.root, `${source}-payload`, "v1"), { recursive: true });
        await run("prepare", `reject-${source}`, [generated.settings, generated.data, ""], 1);
        await run("prepare", `accept-${source}`, [generated.settings, generated.data, lab.root]);
        await run("recover", `accept-${source}`);
        const inspected = await run("inspect", `accept-${source}`);
        assert.equal(inspected.torrents[0].savePath.replaceAll("\\", "/"), join(lab.root, `${source}-payload`, "v1").replaceAll("\\", "/"));
    }
    await lab.checkpoint({ check: "native-mapped-and-portable-paths", existingAliasRejected: true, incompleteMappingPreserved: true, bothBackendsResolveExplicitSourceBase: true });

    for (const sourceBackend of ["legacy", "db"]) {
        for (const destinationBackend of ["legacy", "db"]) {
            const target = `merge-${sourceBackend}-${destinationBackend}`;
            await run("seed-profile", target, [destinationBackend, lab.fixtures, "v1-64k"]);
            await cp(join(lab.fixtures, "seed"), join(lab.root, `${target}-payload`, "v1-64k"), { recursive: true });
            const config = join(lab.root, target, "qbutt", "config", "qbutt.ini");
            await writeFile(config, (await readFile(config, "utf8")).replace("MaxConnections=713", "MaxConnections=271") + "\n[Fixture]\nUnrelated=preserved\n");
            const originalSettings = await readFile(config);
            // Native SettingsStorage ignores a crash residue with no keys.
            await writeFile(join(lab.root, target, "qbutt", "config", "qbutt_new.ini"), "; interrupted save\n[Preferences]\n");
            const originalData = await snapshot(join(lab.root, target, "qbutt", "data"));
            await run("prepare", target, sources[sourceBackend]!);
            assert.deepEqual(await readFile(config), originalSettings, "Preparation changed destination settings");
            for (const [path, value] of Object.entries(originalData))
                assert.equal(sha256(await readFile(join(lab.root, target, "qbutt", "data", path))), value.hash);
            await run("recover", target);
            const inspected = await run("inspect", target);
            const imported = inspected.torrents.filter((item: any) => item.preview);
            assert.equal(inspected.torrents.length, 4);
            assert.equal(imported.length, 3);
            assert(imported.every((item: any) => item.stopped && !item.finished && !item.autoTMM && item.haveCount === 0
                && item.savePath === item.nativePath && !item.uploadMode && !item.shareMode && item.applyIPFilter));
            const settings = await readFile(config, "utf8");
            assert(settings.includes("MaxConnections=713") && settings.includes("Unrelated=preserved"));
            await run("recover", target); // Repeating startup must not replay the import.
            await lab.checkpoint({ check: "native-profile-merge", sourceBackend, destinationBackend, existing: 1, imported: 3, stopped: true, previewPolicy: true, unrelatedSettingsPreserved: true });
        }
    }

    // Enlarge only an unknown metadata fixture to make the real installation
    // boundary observable. No production fault hooks or injected exceptions.
    const crashTarget = "crash-recovery";
    await run("seed-profile", crashTarget, ["legacy", lab.fixtures, "v1-64k"]);
    await cp(join(lab.fixtures, "seed"), join(lab.root, `${crashTarget}-payload`, "v1-64k"), { recursive: true });
    const crashData = join(lab.root, crashTarget, "qbutt", "data");
    const paddingName = "000-migration-window.bin";
    await writeFile(join(crashData, "BT_backup", paddingName), Buffer.alloc(64 * 1024 * 1024, 0x71));
    const originalCrashData = await snapshot(crashData);
    const crashConfig = join(lab.root, crashTarget, "qbutt", "config", "qbutt.ini");
    const originalCrashSettings = await readFile(crashConfig);
    await run("prepare", crashTarget, sources.legacy!);
    const manifestPath = join(crashData, "profile-import", "transaction.json");
    const crashing = Bun.spawn([driver, "recover", lab.root, crashTarget], {
        stdout: Bun.file(join(lab.root, "crash-process.stdout.log")), stderr: Bun.file(join(lab.root, "crash-process.stderr.log")),
        windowsHide: true,
    });
    let interrupted: Record<string, any> | undefined;
    const deadline = Date.now() + 90000;
    while (crashing.exitCode === null && Date.now() < deadline) {
        const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
        if (manifest.state === "installing") {
            crashing.kill();
            await crashing.exited;
            interrupted = JSON.parse(await readFile(manifestPath, "utf8"));
            break;
        }
        await Bun.sleep(1);
    }
    if (crashing.exitCode === null) { crashing.kill(); await crashing.exited; }
    assert(interrupted?.state === "installing", "The process was not interrupted at the durable installation boundary");
    const partialCurrent = await snapshot(crashData);
    const backup = interrupted.backup as string;
    assert(backup.replaceAll("\\", "/").toLowerCase().startsWith(tmpdir().replaceAll("\\", "/").toLowerCase() + "/qbutt-profile-migration-"), "Unexpected backup path");
    const backupPadding = join(backup, "data", "BT_backup", paddingName);
    const preservedPadding = join(lab.root, "backup-padding-preserved.bin");
    await cp(backupPadding, preservedPadding);
    await rm(backupPadding); // Deliberately remove one generated backup file.
    await run("recover", crashTarget, [], 2);
    assert.deepEqual(await snapshot(crashData), partialCurrent, "Incomplete backup recovery removed current files");
    await cp(preservedPadding, backupPadding);
    await run("recover", crashTarget, [], 2);
    await run("recover", crashTarget);
    const restored = await snapshot(crashData);
    assert.deepEqual(Object.keys(restored).sort(), Object.keys(originalCrashData).sort());
    for (const [path, info] of Object.entries(originalCrashData))
        assert.equal(restored[path]!.hash, info.hash, "Crash rollback did not restore original metadata bytes");
    assert.deepEqual(await readFile(crashConfig), originalCrashSettings);
    await lab.checkpoint({ check: "forced-process-exit-and-incomplete-backup", installingJournalObserved: true, missingBackupRefusedBeforeRemoval: true, originalMetadataAndSettingsRestored: true });

    // Exercise the real Application startup hook and one existing native job.
    const existing = join(lab.root, "existing-payload");
    await cp(join(lab.fixtures, "seed"), existing, { recursive: true });
    await lab.start();
    const existingHash = await lab.add("v1-64k", existing);
    await lab.shutdown();
    await run("prepare", "profile", sources.db!);
    await lab.start();
    const all = await lab.json<TorrentStatus[]>("torrents/info");
    assert.equal(all.length, 4);
    assert(all.every(item => item.state.startsWith("stopped") || item.state.startsWith("checking")), "Imported job started without user action");
    const importedHashes = all.filter(item => item.hash !== existingHash).map(item => item.hash);
    await lab.request("torrents/addTags", { hashes: importedHashes.join("|"), tags: "import-policy" });
    await lab.request("qbuttPolicies/configure", { configuration: JSON.stringify({
        enabled: true, allow_delete_data: false,
        rules: [{ id: "import-completed", enabled: true, match: { tags: ["import-policy"] }, actions: ["remove_torrent"] }],
    }) });
    for (const torrent of all.filter(item => item.hash !== existingHash)) {
        await lab.request("torrents/recheck", { hashes: torrent.hash });
        await waitFor("imported native hash check", () => lab.info(torrent.hash), value => value.progress === 1 && value.state === "stoppedUP");
        const pieces = await lab.json<number[]>(`torrents/pieceStates?hash=${torrent.hash}`);
        assert(pieces.length > 0 && pieces.every(value => value === 2), "Imported data did not pass native piece validation");
    }
    await lab.shutdown();
    const afterNative = await run("inspect", "profile");
    assert.equal(afterNative.torrents.filter((item: any) => item.preview).length, 3, "Native resume lost first-import preview policy");
    await lab.start();
    assert.equal((await lab.json<TorrentStatus[]>("torrents/info")).length, 4, "Restart lost imported or original jobs");
    const previews = await waitFor("imported completion barriers ready", () => lab.json<{
        hash: string; ready: boolean; preview_required: boolean; rules: { rule: string }[];
    }[]>("qbuttPolicies/preview"), items => importedHashes.every(hash => items.some(item => item.hash === hash && item.ready)));
    for (const hash of importedHashes) {
        const preview = previews.find(item => item.hash === hash)!;
        assert(preview.preview_required && preview.rules.some(rule => rule.rule === "import-completed"),
            "Real imported resume did not hold a matching completion policy for review");
    }
    await Bun.sleep(1800);
    assert.equal((await lab.json<TorrentStatus[]>("torrents/info")).length, 4,
        "Completion policy removed an imported torrent without acknowledgement");
    assert.deepEqual(await lab.json<unknown[]>("qbuttPolicies/journal"), []);
    await lab.request("qbuttPolicies/acknowledge", { hash: importedHashes[0]!, consent: "true" });
    await waitFor("only the acknowledged imported torrent removed", () => lab.json<TorrentStatus[]>("torrents/info"),
        items => items.length === 3 && !items.some(item => item.hash === importedHashes[0]));
    const journal = await waitFor("imported completion claim persisted", () => lab.json<{ rule: string; status: string }[]>("qbuttPolicies/journal"),
        items => items.length === 1 && items[0]!.status === "dispatched");
    assert.equal(journal[0]!.rule, "import-completed");
    await lab.shutdown();
    await lab.checkpoint({ check: "real-native-startup-recheck-restart", existingHash, imported: 3, nativePiecesVerified: true, previewSurvivesNativeResume: true });
    await lab.start();
    await Bun.sleep(1800);
    assert.equal((await lab.json<TorrentStatus[]>("torrents/info")).length, 3);
    assert.deepEqual(await lab.json<unknown[]>("qbuttPolicies/journal"), journal, "Imported completion action replayed after restart");
    await lab.shutdown();
    await lab.checkpoint({ check: "actual-profile-import-completion-policy", imported: 3,
        matchingRulesHeldAcrossRestart: true, explicitlyAcknowledged: 1, remainingImported: 2, actionNotReplayed: true });

    assert.deepEqual(await snapshot(join(lab.root, "source-legacy")), beforeSources.legacy);
    assert.deepEqual(await snapshot(join(lab.root, "source-db")), beforeSources.db);
    assert.deepEqual(await snapshot(join(lab.root, "source-legacy-payload")), beforeSources.legacyPayload);
    assert.deepEqual(await snapshot(join(lab.root, "source-db-payload")), beforeSources.dbPayload);
    for (const source of ["source-legacy", "source-db"])
        for (const kind of ["v1", "v2", "hybrid"])
            await verifyPayload(join(lab.root, `${source}-payload`, kind), lab.manifest.payload);
    await lab.checkpoint({ check: "source-immutability", nativeProfilesAndPayloads: true, bytesAndModifiedTimesUnchanged: true });
    for (const entry of await readdir(lab.root, { withFileTypes: true })) {
        if (!entry.isDirectory() && entry.name !== "backup-padding-preserved.bin") continue;
        const target = resolve(lab.root, entry.name);
        assert(!entry.isSymbolicLink() && dirname(target) === resolve(lab.root), "Cleanup escaped the owned profile fixture");
        await rm(target, { recursive: true, force: true });
    }
    await lab.finish();
}
catch (error) {
    await lab.shutdown().catch(() => {});
    await lab.finish(error);
    throw error;
}
