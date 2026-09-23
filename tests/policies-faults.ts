import assert from "node:assert/strict";
import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { createLab, startSeed, verifyPayload, waitFor } from "./lab";

interface Claim { key: string; status: string }
interface Repair { id: string; state: string; error?: string }

const selected = process.env.QBUTT_POLICY_CASE;
assert(!selected || ["before_claim", "claimed", "dispatched", "acknowledged", "queued", "journal_write"].includes(selected), "Unknown policy fault scenario");
for (const point of ["before_claim", "claimed", "dispatched", "acknowledged"].filter(point => !selected || point === selected)) {
    const lab = await createLab(`policies-fault-${point}`);
    let failure: unknown;
    try {
        const destination = join(lab.root, "target");
        await cp(join(lab.fixtures, "seed"), destination, { recursive: true });
        await writeFile(join(destination, "unknown.dat"), "unknown payload");
        await lab.start();
        const hash = await lab.add("v1", destination);
        await lab.request("torrents/recheck", { hashes: hash });
        await waitFor("native check complete", () => lab.info(hash), item => item.progress === 1 && item.state === "stoppedUP");
        await lab.shutdown();
        if (point === "acknowledged")
            await lab.markCompletionPreview(hash);
        process.env.QBUTT_COMPLETION_FAULT = point;
        await lab.start();
        await lab.request("qbuttPolicies/configure", { configuration: JSON.stringify({ enabled: true, allow_delete_data: false,
            rules: [{ id: "once", enabled: true, match: {}, actions: ["remove_torrent"] }] }) });
        if (point === "acknowledged") {
            await waitFor("imported torrent initialized and stopped", () => lab.info(hash), item => item.progress === 1 && item.state === "stoppedUP");
            await lab.request("qbuttPolicies/acknowledge", { hash, consent: "true" });
        }
        await waitFor("real native crash point", async () => lab.exitCode, code => code !== null && code !== undefined);
        assert(lab.exitCode === 198, `Expected completion fault exit 198 at ${point}; got ${lab.exitCode}`);
        await assert.rejects(lab.shutdown(), /exited 198/);
        delete process.env.QBUTT_COMPLETION_FAULT;
        const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
        await lab.start();
        if (point === "before_claim" || point === "acknowledged") {
            await waitFor("unclaimed completion resumes after restart", () => lab.json<unknown[]>(`torrents/info?hashes=${hash}`), items => !items.length);
            await waitFor("claim dispatched after restart", () => lab.json<Claim[]>("qbuttPolicies/journal"), entries => entries.length === 1 && entries[0]!.status === "dispatched");
        }
        else {
            await Bun.sleep(2000);
            const journal = await lab.json<Claim[]>("qbuttPolicies/journal");
            assert(journal.length === 1 && journal[0]!.status === "claimed", "Interrupted action replayed or lost its durable claim");
            if (point === "claimed")
                assert((await lab.info(hash)).progress === 1, "Claim-before-action crash removed the task on restart");
        }
        const originalJournal = await lab.json<Claim[]>("qbuttPolicies/journal");
        await lab.shutdown();
        await lab.start();
        await Bun.sleep(1400);
        assert.deepEqual(await lab.json<Claim[]>("qbuttPolicies/journal"), originalJournal);
        await lab.shutdown();
        assert(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes);
        assert((await readFile(join(destination, "unknown.dat"), "utf8")) === "unknown payload");
        await lab.checkpoint({ point, check: "crash-and-two-restarts-preserve-idempotency-and-payload", verifiedBytes, journal: originalJournal });
    }
    catch (error) { failure = error; }
    finally {
        delete process.env.QBUTT_COMPLETION_FAULT;
        try { await lab.shutdown(); }
        catch (error) { failure ??= error; }
        if (!failure) {
            for (const name of ["fixtures", "profile", "target"]) {
                const target = resolve(lab.root, name);
                assert.equal(dirname(target), resolve(lab.root), "Cleanup escaped the owned policy fixture");
                try { await rm(target, { recursive: true, force: true }); }
                catch (error) { failure ??= error; }
            }
        }
        await lab.finish(failure);
    }
    if (failure)
        throw failure;
}

// Enter the queued application handler, admit repair while it is live,
// then let the Exit action reach its guard.
// No shutdown, reboot, suspend or hibernate action is configured or called.
if (!selected || selected === "queued") {
    const point = "queued";
    const lab = await createLab(`policies-${point}`);
    let failure: unknown;
    try {
        const configPath = join(lab.root, "profile", "qbutt", "config", "qbutt.ini");
        const config = await readFile(configPath, "utf8");
        await writeFile(configPath, config.replace("[Preferences]\n", "[Preferences]\nGeneral\\Locale=en\nDownloads\\AutoShutDownqBTOnCompletion=true\n"));
        const gate = join(lab.root, "completion-gate");
        process.env.QBUTT_COMPLETION_GATE = gate;
        process.env.QBUTT_COMPLETION_GATE_POINT = point;
        await lab.start();
        const destination = join(lab.root, "download");
        const trigger = await lab.add("v1-64k", destination);
        await lab.request("torrents/start", { hashes: trigger });
        await waitFor("unfinished trigger", () => lab.info(trigger), info => info.state === "stalledDL");
        const candidate = join(lab.root, "candidate");
        await cp(join(lab.fixtures, "seed"), candidate, { recursive: true });
        const repairHash = await lab.add("v2", candidate);
        await lab.request("torrents/recheck", { hashes: repairHash });
        await waitFor("repair candidate ready", () => lab.info(repairHash), info => info.progress === 1 && info.state === "stoppedUP");
        const seed = await startSeed(lab.python, lab.fixtures, "v1-64k", lab.root);
        try {
            await lab.request("torrents/addPeers", { hashes: trigger, peers: `${seed.host}:${seed.port}` });
            await waitFor("completion entered actual Qt gate", () => Bun.file(`${gate}.entered`).exists(), exists => exists);
            assert((await readFile(`${gate}.entered`, "utf8")) === point);
            const operation = await (await lab.request("qbuttRepair/analyze", { hash: repairHash })).json() as Repair;
            await waitFor("repair owns data during completion loop", () => lab.json<Repair>("qbuttRepair/status"), value => value.state === "analyzed");
            await writeFile(gate, "release Qt loop");
            await Bun.sleep(1800);
            assert(lab.exitCode === null, `${point} completion bypassed repair ownership`);
            const held = await lab.json<Repair>("qbuttRepair/status");
            assert(held.id === operation.id && held.state === "analyzed");
            await lab.request("qbuttRepair/cancel", { id: operation.id });
            await lab.shutdown();
            const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
            assert(await verifyPayload(candidate, lab.manifest.payload) === verifiedBytes);
            await lab.checkpoint({ point, check: "queued-completion-rechecks-live-maintenance-owner", verifiedBytes });
        }
        finally { await seed.stop(); }
    }
    catch (error) { failure = error; }
    finally {
        delete process.env.QBUTT_COMPLETION_GATE;
        delete process.env.QBUTT_COMPLETION_GATE_POINT;
        try { await lab.shutdown(); }
        catch (error) { failure ??= error; }
        if (!failure) {
            for (const name of ["fixtures", "profile", "download", "candidate"]) {
                const target = resolve(lab.root, name);
                assert.equal(dirname(target), resolve(lab.root), "Cleanup escaped the owned policy fixture");
                try { await rm(target, { recursive: true, force: true }); }
                catch (error) { failure ??= error; }
            }
        }
        await lab.finish(failure);
    }
    if (failure)
        throw failure;
}


if (!selected || selected === "journal_write") {
    const lab = await createLab("policies-journal-write");
    let failure: unknown;
    let holder: ReturnType<typeof Bun.spawn> | undefined;
    try {
        const destination = join(lab.root, "target");
        await cp(join(lab.fixtures, "seed"), destination, { recursive: true });
        await lab.start();
        const hash = await lab.add("v1", destination);
        await lab.request("torrents/recheck", { hashes: hash });
        await waitFor("write-fault candidate checked", () => lab.info(hash), item => item.progress === 1 && item.state === "stoppedUP");
        await lab.shutdown();
        const journalDirectory = join(lab.root, "profile", "qbutt", "data", "completion");
        await mkdir(journalDirectory, { recursive: true });
        const journalPath = join(journalDirectory, "journal.json");
        await writeFile(journalPath, "[]");
        holder = Bun.spawn([lab.python, join(import.meta.dir, "fixtures", "hold-file.py"), journalPath],
            { stdin: "pipe", stdout: "pipe", stderr: "pipe", windowsHide: true });
        const first = await holder.stdout.getReader().read();
        assert(new TextDecoder().decode(first.value).trim() === "held", "Native replacement guard did not open");
        await lab.start();
        await lab.request("qbuttPolicies/configure", { configuration: JSON.stringify({ enabled: true, allow_delete_data: false,
            rules: [{ id: "needs-durable-reason", enabled: true, match: {}, actions: ["remove_torrent"] }] }) });
        await waitFor("journal replacement failure surfaced", () => lab.json<{ error?: string }>("qbuttPolicies/configuration"), config => !!config.error);
        assert((await lab.info(hash)).progress === 1, "Failed journal write removed native task");
        assert((await readFile(journalPath, "utf8")) === "[]", "Failed replacement changed previous journal");
        assert((await lab.json<{ preview_required: boolean }[]>("qbuttPolicies/preview"))[0]!.preview_required);
        const laterDestination = join(lab.root, "later-target");
        await cp(join(lab.fixtures, "seed"), laterDestination, { recursive: true });
        const laterHash = await lab.add("v2", laterDestination);
        await lab.request("torrents/setShareLimits", { hashes: laterHash, ratioLimit: "0", seedingTimeLimit: "-1",
            inactiveSeedingTimeLimit: "-1", shareLimitAction: "1" });
        await lab.request("torrents/recheck", { hashes: laterHash });
        await waitFor("later completion initialized during journal failure", () => lab.info(laterHash), item => item.progress === 1 && item.state === "stoppedUP");
        await Bun.sleep(2200);
        assert((await lab.info(laterHash)).progress === 1, "Journal failure allowed a later native share-limit removal");
        await lab.request("torrents/delete", { hashes: laterHash, deleteFiles: "false" });
        await waitFor("manual removal remains available during policy error", () => lab.json<unknown[]>(`torrents/info?hashes=${laterHash}`), items => !items.length);
        holder.stdin.end();
        assert(await holder.exited === 0, await new Response(holder.stderr).text());
        holder = undefined;
        await lab.shutdown();
        const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
        assert(await verifyPayload(laterDestination, lab.manifest.payload) === verifiedBytes);
        await lab.start();
        await waitFor("write-fault recovery candidate initialized", () => lab.info(hash), item => item.state === "stoppedUP");
        assert((await lab.json<{ preview_required: boolean }[]>("qbuttPolicies/preview"))[0]!.preview_required);
        await lab.request("qbuttPolicies/acknowledge", { hash, consent: "true" });
        await waitFor("acknowledged storage recovery applies rule", () => lab.json<unknown[]>(`torrents/info?hashes=${hash}`), items => !items.length);
        await lab.shutdown();
        assert(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes);
        await lab.checkpoint({ check: "real-journal-replacement-failure-holds-action-and-requires-persisted-preview", verifiedBytes });
    }
    catch (error) { failure = error; }
    finally {
        if (holder) {
            holder.stdin.end();
            await holder.exited;
        }
        try { await lab.shutdown(); }
        catch (error) { failure ??= error; }
        if (!failure) {
            for (const name of ["fixtures", "profile", "target", "later-target"]) {
                const target = resolve(lab.root, name);
                assert.equal(dirname(target), resolve(lab.root), "Cleanup escaped the owned policy fixture");
                try { await rm(target, { recursive: true, force: true }); }
                catch (error) { failure ??= error; }
            }
        }
        await lab.finish(failure);
    }
    if (failure)
        throw failure;
}
