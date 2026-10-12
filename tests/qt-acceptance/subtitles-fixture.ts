import { createHash } from "node:crypto";
import { appendFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

interface Request {
    id: number;
    method: string;
    params: Record<string, any>;
}

const spec = JSON.parse(await readFile(process.env.QBUTT_SUBTITLE_FIXTURE_SPEC!, "utf8"));
const dataDir = process.argv[process.argv.indexOf("--data-dir") + 1];
const requests = new Map<number, Request>();
const cancelled = new Set<number>();
const attempts = new Map<string, number>();

function emit(id: number, event: string, data: unknown) {
    process.stdout.write(`${JSON.stringify({ id, event, data })}\n`);
}

async function record(event: string, request: Request, detail: Record<string, unknown> = {}) {
    await appendFile(spec.helperTrace, `${JSON.stringify({ event, id: request.id,
        identity: request.params.identity, ...detail })}\n`);
}

async function revision(request: Request, version: number, windowStart: number, invalid = false) {
    const identity = request.params.identity as string;
    const text = `${version === 1 ? "Первая версия" : "Вторая версия"} [${identity}] [track ${request.params.track.id}] ${windowStart}`;
    const contents = invalid ? "This is not a timed subtitle file.\n" : `1\n00:00:00,000 --> 00:05:00,000\n${text}\n\n`;
    const hash = createHash("sha256").update(contents).digest("hex");
    const document = createHash("sha256").update(identity).digest("hex");
    const directory = join(dataDir, "subtitles", document);
    await mkdir(directory, { recursive: true });
    const path = join(directory, `translated-${hash}.srt`);
    await writeFile(path, contents);
    emit(request.id, "subtitle", { identity, url: pathToFileURL(path).href,
        title: "Russian fixture", language: "ru", revision: hash, windowStart, windowEnd: windowStart + 120 });
}

async function subtitles(request: Request) {
    const { identity, track, position, startTime, targetLanguage, url } = request.params;
    const scenario = spec.scenarios.find((item: any) => identity === item.identity || identity === `${item.identity}:next`);
    const external = scenario?.externalSubtitle || scenario?.lateSubtitle;
    const expectedTrack = scenario?.mode === "action-retarget" && attempts.has(identity) ? 2 : 1;
    if (!scenario || !url.startsWith("file:") || targetLanguage !== "ru" || !Number.isFinite(startTime)
        || track.id !== expectedTrack || track.ffIndex !== (external ? 0 : expectedTrack + 1)
        || (external && track.url !== pathToFileURL(external).href)) {
        emit(request.id, "error", { message: "Native subtitle contract did not match actual mpv source track" });
        return;
    }
    requests.set(request.id, request);
    const attempt = (attempts.get(identity) || 0) + 1;
    attempts.set(identity, attempt);
    await record("request", request, { position, startTime, track, url });
    const windowStart = Math.floor(position / 60) * 60;
    if (scenario.mode === "unavailable" || scenario.mode === "already-target" || (scenario.mode === "retry" && attempt === 1)) {
        emit(request.id, "complete", { identity, status: scenario.mode === "already-target" ? "already-target" : "unavailable",
            windowStart, windowEnd: windowStart + 120, code: scenario.failureCode || "ai_authentication",
            message: scenario.failureCode === "ai_future" ? "Specific future AI failure." : "Исходная ошибка сервиса перевода." });
        return;
    }
    emit(request.id, "subtitleProgress", { phase: "connecting", provider: "codex" });
    await Bun.sleep(50);
    emit(request.id, "subtitleProgress", { phase: "translating", provider: "codex", model: "fixture-model" });
    const slow = ["pending-original", "seek", "delay", "navigate", "stop"].includes(scenario.mode)
        && position < 120 && !identity.endsWith(":next");
    await Bun.sleep(slow ? 3000 : 200);
    await revision(request, 1, windowStart, scenario.mode === "retry-load" && attempt === 1);
    await record(cancelled.has(request.id) ? "late" : "first", request);
    if (scenario.mode.startsWith("action-") || ["retry", "retry-load", "late-track"].includes(scenario.mode)) {
        emit(request.id, "complete", { identity, status: "ready", windowStart, windowEnd: windowStart + 120 });
        return;
    }
    await Bun.sleep(1500);
    await revision(request, 2, windowStart);
    await record(cancelled.has(request.id) ? "late" : "replacement", request);
    emit(request.id, "complete", { identity, status: "ready", windowStart, windowEnd: windowStart + 120 });
}

async function handle(request: Request) {
    if (request.method === "subtitles.ahead") {
        await subtitles(request);
    }
    else if (request.method === "cancel") {
        cancelled.add(request.params.id);
        const original = requests.get(request.params.id);
        if (original) await record("cancel", original);
        emit(request.id, "complete", { cancelled: true });
    }
    else {
        emit(request.id, "error", { message: `Unsupported subtitle fixture method: ${request.method}` });
    }
}

let input = "";
for await (const chunk of Bun.stdin.stream()) {
    input += new TextDecoder().decode(chunk);
    let newline: number;
    while ((newline = input.indexOf("\n")) >= 0) {
        const line = input.slice(0, newline);
        input = input.slice(newline + 1);
        if (line) {
            const request: Request = JSON.parse(line);
            void handle(request).catch(error => emit(request.id, "error", { message: String(error) }));
        }
    }
}
