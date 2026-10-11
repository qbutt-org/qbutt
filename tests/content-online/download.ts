import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, readdir, rm, stat } from "node:fs/promises";
import { join, resolve, sep } from "node:path";

import type { Event, Variant } from "../../components/content/contracts";
import type { DownloadJob } from "../../components/content/downloads";

type Request = (method: string, params?: Record<string, unknown>) => Promise<Event[]>;

export async function verifyLiveDownload(request: Request, lab: string, variant: Variant): Promise<Record<string, unknown>> {
    const events = await request("download", { variant, title: "Bounded selected episode" });
    const job = events.find(event => event.event === "job")?.data as DownloadJob | undefined;
    assert(job?.id, "The actual helper did not enqueue the exact variant");
    const partial = join(lab, "downloads", `.qbutt-${job.id}.part`);
    const staging = join(lab, "profile", "downloads", job.id);
    const deadline = Date.now() + 65000;
    let manifestAt = 0;
    let bytes = 0;
    let references = 0;
    let cancelled = false;
    let transferSeconds = 0;
    const jobs = async () => {
        const reply = await request("jobs");
        return reply.find(event => event.event === "jobs")?.data as DownloadJob[];
    };

    try {
        while (Date.now() < deadline) {
            const current = (await jobs()).find(item => item.id === job.id)!;
            if (current.status === "error")
                throw new Error(`Actual selected download failed: ${current.message}`);
            if (current.status === "complete") {
                throw new Error("The bounded check finished the episode before cancellation; no partial-download proof.");
            }
            const files = await readdir(staging).catch(() => []);
            const manifest = files.find(file => file.endsWith(".mpd"));
            if (manifest) {
                const body = await readFile(join(staging, manifest), "utf8");
                references = (body.match(/http:\/\/127\.0\.0\.1:\d+\/_qbutt\//g) || []).length;
                if (!manifestAt && references)
                    manifestAt = Date.now();
            }
            bytes = await stat(partial).then(info => info.size).catch(() => 0);
            if (bytes >= 4096 && manifestAt)
                break;
            if (manifestAt && Date.now() - manifestAt >= 8000)
                throw new Error("The guarded DASH transfer produced no reviewable partial payload within eight seconds.");
            await Bun.sleep(20);
        }
        assert(manifestAt && references >= 2 && bytes >= 4096,
            "No actual DASH manifest and muxed partial bytes were observed; a timeout is not a pass");
        await request("cancelDownload", { id: job.id });
        cancelled = true;
        transferSeconds = (Date.now() - manifestAt) / 1000;
        assert.equal((await jobs()).find(item => item.id === job.id)?.status, "cancelled");

        // A small public free-software license exercises the next real queue job
        // without fetching another commercial episode.
        const legalUrl = "https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt";
        const legal = await fetch(legalUrl, { signal: AbortSignal.timeout(15000) });
        assert(legal.ok, "The public legal priority fixture was unavailable");
        const expected = Buffer.from(await legal.arrayBuffer());
        assert(expected.includes(Buffer.from("GNU LESSER GENERAL PUBLIC LICENSE")));
        const priorityEvents = await request("download", { title: "Legal priority fixture", url: legalUrl });
        const priority = priorityEvents.find(event => event.event === "job")?.data as DownloadJob | undefined;
        assert(priority?.id);
        const priorityDeadline = Date.now() + 30000;
        let completed: DownloadJob | undefined;
        while (Date.now() < priorityDeadline) {
            const current = (await jobs()).find(item => item.id === priority.id)!;
            if (current.status === "error")
                throw new Error(`The next legal queue job failed: ${current.message}`);
            if (current.status === "complete") {
                completed = current;
                break;
            }
            await Bun.sleep(20);
        }
        assert(completed?.path, "Cancellation did not release the queue for the next legal job");
        const actual = await readFile(completed.path);
        const sha256 = (value: Buffer) => createHash("sha256").update(value).digest("hex");
        assert.equal(sha256(actual), sha256(expected));
        assert.equal(await stat(partial).then(() => true).catch(() => false), false);
        assert.equal(await stat(staging).then(() => true).catch(() => false), false);

        const child = Bun.spawn(["powershell.exe", "-NoProfile", "-Command",
            `$owned = Get-CimInstance Win32_Process -Filter \"Name='ffmpeg.exe' OR Name='ffprobe.exe'\" | Where-Object { $_.CommandLine -and $_.CommandLine.Contains('${job.id}') }; @($owned).Count`], {
            windowsHide: true, stdout: "pipe", stderr: "pipe",
        });
        const [output, error, exit] = await Promise.all([
            new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited,
        ]);
        assert.equal(exit, 0, error);
        assert.equal(output.trim(), "0", "The cancelled download left an owned media process");
        return { status: "passed", variantId: variant.id, episode: variant.episode,
            translation: variant.translation, player: variant.player, partialBytesObserved: bytes,
            secondsAfterManifest: transferSeconds, guardedDashReferences: references,
            cancelled: true, partAndStagingRemoved: true, ownedMediaProcesses: 0,
            nextLegalJob: { bytes: actual.length, sha256: sha256(actual) } };
    }
    finally {
        if (!cancelled)
            await request("cancelDownload", { id: job.id });
        const completed = (await jobs()).find(item => item.id === job.id);
        if (completed?.path && resolve(completed.path).startsWith(resolve(lab, "downloads") + sep))
            await rm(completed.path);
    }
}
