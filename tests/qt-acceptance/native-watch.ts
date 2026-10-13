import assert from "node:assert/strict";
import { Database } from "bun:sqlite";
import { mkdir, mkdtemp, readFile, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { labAppearanceSettings } from "../appearance";
import { assertQtRuntime } from "../qt-runtime";

const source = process.env.QBUTT_QT_ACCEPTANCE_EXE;
const qplay = process.env.QBUTT_LAB_QPLAY;
assert(source, "Set QBUTT_QT_ACCEPTANCE_EXE to the combined Qt acceptance build");
assert(qplay, "Set QBUTT_LAB_QPLAY to the verified qplay executable");
assert(process.platform === "win32", "Main Watch acceptance requires Windows");
const executable = resolve(source);
const bundle = resolve(process.env.QBUTT_QT_ACCEPTANCE_BUNDLE ?? dirname(executable));
assertQtRuntime(bundle);
assert((await stat(qplay)).isFile(), "qplay executable is missing");

const selections = {
    aniliberty: { query: "Frieren", workId: "aniliberty:9542", variantSuffix: ":480" },
    anitokyo: { query: "Psyren", workId: "anitokyo:9981", episode: "1", translation: "Дублированный" },
    animespirit: { query: "Magical Explorer", workId: "animespirit:16461", episode: "2",
        translation: "Субтитры AnimeJoy" },
    vkvideo: { query: "Sintel Blender", workId: "vkvideo:-76820671_456239057",
        variantSuffix: ":video", contentKind: "video" },
} as const;
const sourceId = process.argv[2]?.replace(/^--source=/, "") ?? "aniliberty";
assert(sourceId in selections, `Unknown native video source: ${sourceId}`);
const selection = selections[sourceId as keyof typeof selections];

const root = await mkdtemp(join(tmpdir(), "qbutt-main-watch-"));
const profile = join(root, "profile");
const qplayProfile = join(root, "qplay");
await mkdir(join(profile, "qbutt", "config"), { recursive: true });
await mkdir(join(qplayProfile, "config"), { recursive: true });
await writeFile(join(qplayProfile, "config", "mpv.conf"), "vo=null\nao=null\nosc=no\nhwdec=no\nkeep-open=yes\n");
await writeFile(join(profile, "qbutt", "config", "qbutt.ini"), [
    "[LegalNotice]",
    "Accepted=true",
    "[BitTorrent]",
    "Session\\DHTEnabled=false",
    "Session\\LSDEnabled=false",
    "Session\\PeXEnabled=false",
    "Session\\Port=0",
    "Session\\InterfaceAddress=127.0.0.1",
    "[Network]",
    "PortForwardingEnabled=false",
    "[Core]",
    "AutoOpenTorrentFiles=false",
    "[GUI]",
    "Notifications\\Enabled=false",
    "[Preferences]",
    "General\\Locale=en",
    "Advanced\\updateCheck=false",
    "WebUI\\Enabled=false",
    "Search\\SearchEnabled=true",
    ...labAppearanceSettings("functional"),
    "",
].join("\n"));
const evidencePath = join(root, "evidence.json");
const specPath = join(root, "spec.json");
await writeFile(specPath, JSON.stringify({ mode: "native-watch", evidencePath,
    screenshots: join(root, "screenshots"), source: sourceId, ...selection }));
const application = Bun.spawn([executable, `--profile=${profile}`], {
    cwd: bundle,
    windowsHide: true,
    stdin: "ignore",
    timeout: 160_000,
    env: {
        ...process.env,
        QT_QPA_PLATFORM: "offscreen",
        QT_QUICK_BACKEND: "software",
        QBUTT_QT_ACCEPTANCE_SPEC: specPath,
        QBUTT_LAB_PATHS: "1",
        QBUTT_LAB_QPLAY_PROFILE: qplayProfile,
    },
    stdout: Bun.file(join(root, "stdout.log")),
    stderr: Bun.file(join(root, "stderr.log")),
});
const exit = await application.exited;
const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
assert.equal(exit, 0, `Main Watch failed; inspect ${root}`);
assert.equal(evidence.status, "passed", JSON.stringify(evidence));
const check = evidence.checks[0];
assert.equal(check.name, "native-watch");
assert.equal(check.source, sourceId);
assert.equal(check.workId, selection.workId);
assert.equal(check.identity, `video:${check.variantId}`);
assert(check.descriptorIsHttps || check.descriptorIsLocal);
assert.equal(check.contentKind, "contentKind" in selection ? selection.contentKind : "anime");
assert(check.trackCount > 0);
const database = new Database(join(check.libraryPath, "content-library.sqlite"), { readonly: true });
try {
    assert.equal((database.query("PRAGMA integrity_check").get() as { integrity_check: string }).integrity_check, "ok");
    const rows = database.query("SELECT entry FROM library WHERE json_extract(entry, '$.last_opened') IS NOT NULL").all() as { entry: string }[];
    assert.equal(rows.length, 1);
    const entry = JSON.parse(rows[0]!.entry);
    const publicationId = selection.workId.split(":")[1];
    const publication = entry.group.publications.find((work: { reference: { id: string } }) =>
        work.reference.id === publicationId);
    assert(publication);
    assert.equal(publication.variants.length, check.variantCount);
    assert.deepEqual(entry.preferences.selected, entry.last_opened.variant);
    const opened = publication.variants.find((variant: { reference: unknown }) =>
        JSON.stringify(variant.reference) === JSON.stringify(entry.last_opened.variant));
    assert(opened);
    if ("episode" in selection) {
        assert.equal(opened.episode, selection.episode);
    }
    if ("translation" in selection) {
        assert.equal(opened.translation, selection.translation);
    }
    if (sourceId === "aniliberty") {
        const selector = entry.last_opened.variant.selector.episode;
        assert(selector);
        assert.equal(check.variantId, `aniliberty:9542:episode:${selector.id}:${selector.quality.slice(1)}`);
    }
    if (sourceId === "vkvideo") {
        assert.equal(check.variantId, `${selection.workId}:video`);
        assert.equal(publication.url, `https://vkvideo.ru/video${publicationId}`);
        assert.equal(entry.last_opened.variant.selector, "vk_video");
        assert(!rows[0]!.entry.includes("headers") && !rows[0]!.entry.includes(".mp4"));
    }
} finally {
    database.close();
}
console.log(JSON.stringify({ status: "passed", root, evidencePath, exit }));
