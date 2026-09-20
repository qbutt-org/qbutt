import assert from "node:assert/strict";
import { mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { dirname, join, resolve, sep } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, startSeed, verifyPayload, waitFor, type TorrentFile } from "../lab";
import { snapshot } from "./staging-checks";

interface RepairStatus {
    id: string;
    state: string;
    error?: string;
    analysis?: { files: { native_index: number; path: string; selected: boolean;
        expected_size: number; actual_size: number; verified_bytes: number }[] };
}

const lab = await createLab("repair-mappings");
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
let failure: unknown;

async function analyze(hash: string): Promise<RepairStatus> {
    return waitFor("repair mapping admission", async () => {
        await lab.request("qbuttRepair/analyze", { hash });
        const result = await waitFor("physical mapping analysis", () => lab.json<RepairStatus>("qbuttRepair/status"),
            status => status.state === "analyzed" || status.state === "failed");
        if (result.state === "failed") {
            await lab.request("qbuttRepair/cancel", { id: result.id });
            assert(result.error?.startsWith("Wait for"), `Repair refused mapping: ${result.error}`);
        }
        return result;
    }, result => result.state === "analyzed", 15000);
}

try {
    await lab.start();
    await lab.request("app/setPreferences", {
        json: JSON.stringify({ locale: "en", incomplete_files_ext: true, use_unwanted_folder: true }),
    });
    await lab.shutdown();
    await lab.start();
    for (const scenario of ["v1", "v2", "hybrid", "v1-autotmm"]) {
        const format = scenario.split("-")[0]!;
        const autoTMM = scenario.endsWith("-autotmm");
        const directory = join(lab.root, scenario);
        const category = "repair/child";
        const saveBase = join(directory, "completed");
        const downloadBase = join(directory, "incomplete");
        const savePath = autoTMM ? join(saveBase, category) : saveBase;
        const downloadPath = autoTMM ? join(downloadBase, category) : downloadBase;
        await mkdir(savePath, { recursive: true });
        await mkdir(downloadPath, { recursive: true });
        if (autoTMM) {
            await lab.request("app/setPreferences", { json: JSON.stringify({ save_path: saveBase,
                temp_path: downloadBase, temp_path_enabled: true,
                save_path_changed_tmm_enabled: true, category_changed_tmm_enabled: true }) });
            await lab.request("torrents/createCategory", { category });
        }
        const fixture = lab.manifest.torrents.find(item => item.name === format)!;
        const hash = fixture.infoHashV2?.slice(0, 40) ?? fixture.infoHashV1!;
        const add = new FormData();
        add.set("torrents", Bun.file(join(lab.fixtures, fixture.file)));
        if (autoTMM)
            add.set("category", category);
        else {
            add.set("savepath", savePath);
            add.set("downloadPath", downloadPath);
        }
        add.set("stopped", "true");
        add.set("autoTMM", String(autoTMM));
        add.set("contentLayout", "Original");
        await lab.request("torrents/add", add);
        await waitFor("mapping torrent admitted", () => lab.json<unknown[]>(`torrents/info?hashes=${hash}`), items => items.length === 1);
        await waitFor("mapping torrent added", () => lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`), files => files.length === 5);
        const files = await lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`);
        const ignored = files.filter(file => file.name.endsWith("/skip.bin") || file.name.endsWith("/empty.bin"));
        await lab.request("torrents/filePrio", { hash, id: ignored.map(file => file.index).join("|"), priority: "0" });
        const mapping = await analyze(hash);
        assert(mapping.analysis?.files.length === 5);
        await lab.request("qbuttRepair/cancel", { id: mapping.id });

        const physical = new Map<string, string>();
        for (const file of mapping.analysis.files) {
            const logical = fixture.files.find(item => item.index === file.native_index)!;
            assert(resolve(file.path).startsWith(`${resolve(downloadPath)}${sep}`), "Mapping escaped active download root");
            assert(file.selected === !ignored.some(item => item.name === logical.path), "Analysis lost selected-file priorities");
            assert(file.path.endsWith(logical.size ? ".!qB" : "empty.bin"), "Analysis ignored the native incomplete name");
            if (!file.selected)
                assert(file.path.replaceAll("\\", "/").includes("/.unwanted/"), "Ignored file mapping lost unwanted directory");
            physical.set(logical.path, file.path);
            if (logical.size === 0 || logical.path.includes("Юникод")) {
                await rm(file.path, { force: true });
                continue;
            }
            await mkdir(dirname(file.path), { recursive: true });
            let bytes = await readFile(join(lab.fixtures, "seed", logical.path));
            if (logical.path.endsWith("alpha.bin")) {
                bytes[100] = bytes[100]! ^ 0xff;
                bytes = Buffer.concat([bytes, Buffer.alloc(8193, 0xa5)]);
            }
            if (logical.path.endsWith("beta.bin"))
                bytes = bytes.subarray(0, 10001);
            if (logical.path.endsWith("skip.bin"))
                bytes = Buffer.concat([bytes, Buffer.from("ignored tail must survive managed apply")]);
            await writeFile(file.path, bytes);
        }
        const unknown = join(downloadPath, "notes.txt");
        await writeFile(unknown, "Unknown data must survive repair and normal storage moves.\n");
        const before = await snapshot(directory);
        const skippedPath = physical.get("bundle/skip.bin")!;
        const skippedBefore = sha256(await readFile(skippedPath));
        const skippedEmpty = physical.get("bundle/empty.bin")!;

        const preview = await analyze(hash);
        assert.deepEqual(await snapshot(directory), before, "Analysis wrote candidate files");
        if (autoTMM) {
            const categoriesBefore = await lab.json<Record<string, unknown>>("torrents/categories");
            for (const editedCategory of [category, "repair"]) {
                await assert.rejects(lab.request("torrents/editCategory", {
                    category: editedCategory, savePath: join(directory, "changed-category"),
                }), /HTTP 409/, "Repair admitted a category or inherited parent-root change");
            }
            await assert.rejects(lab.request("torrents/setCategory", { hashes: hash, category: "repair" }), /HTTP 409/);
            await lab.request("torrents/removeCategories", { categories: "repair" });
            assert.deepEqual(await lab.json("torrents/categories"), categoriesBefore,
                "Repair admitted deletion of the active category or its parent");
            await lab.request("app/setPreferences", { json: JSON.stringify({ save_path: join(directory, "changed-default"),
                temp_path: join(directory, "changed-temporary"), temp_path_enabled: false }) });
            const preferences = await lab.json<{ save_path: string; temp_path: string; temp_path_enabled: boolean }>("app/preferences");
            assert.equal(resolve(preferences.save_path), saveBase, "Repair admitted a default save-root change");
            assert.equal(resolve(preferences.temp_path), downloadBase, "Repair admitted a default download-root change");
            assert(preferences.temp_path_enabled, "Repair admitted disabling the default download root");
        }
        await lab.request("torrents/filePrio", { hash, id: String(files[0]!.index), priority: "0" });
        await lab.request("torrents/setDownloadPath", { id: hash, path: savePath });
        await lab.request("torrents/renameFile", { hash, oldPath: "bundle/alpha.bin", newPath: "bundle/changed.bin" });
        await lab.request("app/setPreferences", { json: JSON.stringify({ incomplete_files_ext: false, use_unwanted_folder: false }) });
        const frozenFiles = await lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`);
        assert.deepEqual(frozenFiles.map(file => [file.name, file.priority]),
            files.map(file => [file.name, ignored.some(item => item.index === file.index) ? 0 : file.priority]),
            "Repair admitted changed file priorities or names");
        const preferences = await lab.json<{ incomplete_files_ext: boolean; use_unwanted_folder: boolean }>("app/preferences");
        assert(preferences.incomplete_files_ext && preferences.use_unwanted_folder,
            "Repair admitted a global physical-name change");
        const properties = await lab.json<{ download_path: string; save_path: string }>(`torrents/properties?hash=${hash}`);
        assert.equal(resolve(properties.download_path), downloadPath, "Repair admitted a storage-root change");
        assert.equal(resolve(properties.save_path), savePath);
        await lab.request("qbuttRepair/cancel", { id: preview.id });
        assert.deepEqual(await snapshot(directory), before, "Cancelling preview reconciled or modified the layout");
        await lab.shutdown();
        await lab.start();
        await waitFor("stopped mapping restored", () => lab.info(hash), info => info.state.startsWith("stopped"));
        assert.deepEqual(await snapshot(directory), before, "Preview cancellation/restart changed candidate files");
        if (autoTMM) {
            const [info] = await lab.json<{ auto_tmm: boolean; category: string }[]>(`torrents/info?hashes=${hash}`);
            assert(info?.auto_tmm && info.category === category, "Repair cancellation or restart disabled AutoTMM");
        }

        const operation = await analyze(hash);
        assert.deepEqual(operation.analysis!.files.map(file => [file.path, file.selected]),
            mapping.analysis.files.map(file => [file.path, file.selected]), "Restart lost native mappings or selection");
        await lab.request("qbuttRepair/apply", { id: operation.id, consent: "true" });
        const checked = await waitFor("selected managed repair", () => lab.json<RepairStatus>("qbuttRepair/status"),
            status => status.state === "checked" || status.state === "failed");
        assert.equal(checked.state, "checked", checked.error);
        assert.equal((await stat(physical.get("bundle/alpha.bin")!)).size,
            lab.manifest.payload.find(file => file.path === "bundle/alpha.bin")!.size, "Selected oversized file was not truncated");
        assert.equal(sha256(await readFile(skippedPath)), skippedBefore, "Managed repair modified ignored file or tail");
        await assert.rejects(stat(skippedEmpty), { code: "ENOENT" }, "Managed repair created ignored zero-length file");
        assert.equal((await readFile(unknown, "utf8")), "Unknown data must survive repair and normal storage moves.\n");
        await lab.request("qbuttRepair/cancel", { id: operation.id });
        await lab.checkpoint({ check: "physical-selected-repair", format, autoTMM, temporaryNames: true, separateDirectories: true,
            cancelAndRestartReadOnly: true, frozenNamesPrioritiesRoot: true, ignoredFileAndTailUnchanged: true,
            ignoredEmptyAbsent: true });

        let completedPath = savePath;
        if (autoTMM) {
            completedPath = join(directory, "completed-relocated");
            await lab.request("torrents/editCategory", { category, savePath: completedPath,
                downloadPathEnabled: "true", downloadPath });
            const properties = await lab.json<{ save_path: string }>(`torrents/properties?hash=${hash}`);
            assert.equal(resolve(properties.save_path), completedPath, "Category ownership was not released after repair");
        }
        seed = await startSeed(lab.python, lab.fixtures, format, lab.root, { label: scenario });
        await lab.request("torrents/start", { hashes: hash });
        await lab.request("torrents/addPeers", { hashes: hash, peers: `${seed.host}:${seed.port}` });
        await waitFor("selected download and final move", () => lab.info(hash), info => info.progress === 1 && info.state.endsWith("UP"));
        await lab.request("torrents/stop", { hashes: hash });
        await waitFor("selected download stopped", () => lab.info(hash), info => info.state === "stoppedUP");
        const selected = lab.manifest.payload.filter(file => !ignored.some(item => item.name === file.path));
        const verifiedBytes = await verifyPayload(completedPath, selected);
        assert.equal((await readFile(unknown, "utf8")), "Unknown data must survive repair and normal storage moves.\n",
            "Normal move deleted an unknown file");
        await lab.shutdown();
        await lab.start();
        await waitFor("completed mapping restored", () => lab.info(hash), info => info.state === "stoppedUP");
        assert.equal(await verifyPayload(completedPath, selected), verifiedBytes);
        if (autoTMM) {
            const [info] = await lab.json<{ auto_tmm: boolean; category: string }[]>(`torrents/info?hashes=${hash}`);
            assert(info?.auto_tmm && info.category === category, "Repair or native relocation disabled AutoTMM");
        }
        const restoredFiles = await lab.json<TorrentFile[]>(`torrents/files?hash=${hash}`);
        assert(restoredFiles.filter(file => ignored.some(item => item.name === file.name)).every(file => file.priority === 0));
        await lab.checkpoint({ check: "selected-resume-and-normal-layout", format, autoTMM, verifiedBytes, exactSizes: true,
            completedSuffixRemoved: true, completedDirectory: true, restartVerified: true,
            ignoredDownloadScope: "Ordinary libtorrent may write shared v1 boundary bytes; managed apply above remains read-only for ignored files." });
        await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
        await waitFor("mapping torrent removed", () => lab.json<unknown[]>("torrents/info"), items => items.length === 0);
        const stats = await seed.stop();
        seed = undefined;
        assert.equal(stats.downloadPayloadBytes, 0, "Fixture seed unexpectedly downloaded payload");
    }
}
catch (error) { failure = error; }
finally {
    try { await lab.shutdown(); }
    catch (error) { failure ??= error; }
    if (seed) {
        try { await seed.stop(); }
        catch (error) { failure ??= error; }
    }
    await lab.finish(failure);
    for (const name of ["fixtures", "profile", "v1", "v2", "hybrid", "v1-autotmm"]) {
        const target = resolve(lab.root, name);
        assert(target.startsWith(`${resolve(lab.root)}${sep}`), "Cleanup escaped owned fixture");
        await rm(target, { recursive: true, force: true });
    }
}
if (failure)
    throw failure;
