import assert from "node:assert/strict";
import { createHash, pbkdf2Sync, randomBytes } from "node:crypto";
import { existsSync } from "node:fs";
import { appendFile, cp, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve, sep } from "node:path";

import { labAppearanceSettings } from "../appearance";
import { generatePayload } from "../fixtures/generate";
import { encode } from "../network-lab/bencode";

assert.equal(process.platform, "linux", "Read-only repair acceptance requires Linux");
const executable = process.env.QBUTT_QT_ACCEPTANCE_EXE;
assert(executable, "Set QBUTT_QT_ACCEPTANCE_EXE to the native acceptance driver");
const root = await mkdtemp(join(await realpath(tmpdir()), "qbutt-repair-readonly-"));
const profile = join(root, "profile");
const destination = join(root, "destination");
const existingDestination = join(root, "existing");
const screenshots = join(root, "screenshots");
const specPath = join(root, "spec.json");
const runtime = join(root, "runtime");
let application: ReturnType<typeof Bun.spawn> | undefined;
try {
    const payload = await generatePayload(root);
    const sourceRoot = join(root, "seed");
    const ignoredTail = Buffer.from("ignored tail must survive");
    const unknown = Buffer.from("unrelated user data must survive");
    await cp(sourceRoot, destination, { recursive: true });
    await cp(sourceRoot, existingDestination, { recursive: true });
    await appendFile(join(destination, "bundle", "skip.bin"), ignoredTail);
    await writeFile(join(destination, "unknown.keep"), unknown);
    await mkdir(screenshots);
    await mkdir(runtime, { mode: 0o700 });

    const bytes = Buffer.concat(await Promise.all(payload.map(file => readFile(join(sourceRoot, file.path)))));
    const pieces: Buffer[] = [];
    for (let offset = 0; offset < bytes.length; offset += 16384)
        pieces.push(createHash("sha1").update(bytes.subarray(offset, offset + 16384)).digest());
    const torrentPath = join(root, "fixture.torrent");
    await writeFile(torrentPath, encode({ info: {
        name: Buffer.from("bundle"), private: 1, "piece length": 16384, pieces: Buffer.concat(pieces),
        files: payload.map(file => ({ length: file.size, path: file.path.split("/").slice(1).map(part => Buffer.from(part)) })),
    } }));

    const reservation = Bun.listen({ hostname: "127.0.0.1", port: 0, socket: { data() {} } });
    const port = reservation.port;
    reservation.stop(true);
    const password = randomBytes(32).toString("hex");
    const salt = randomBytes(16);
    const passwordHash = `${salt.toString("base64")}:${pbkdf2Sync(password, salt, 100000, 64, "sha512").toString("base64")}`;
    const config = join(profile, "qbutt", "config");
    await mkdir(config, { recursive: true });
    await writeFile(join(config, "qbutt.conf"), [
        "[BitTorrent]", "Session\\DHTEnabled=false", "Session\\LSDEnabled=false", "Session\\PeXEnabled=false",
        "Session\\InterfaceAddress=127.0.0.1", "Session\\AddTorrentStopped=true", "Session\\QueueingSystemEnabled=false",
        "Session\\AddExtensionToIncompleteFiles=false", "Session\\UseUnwantedFolder=false",
        "Session\\AddTrackersEnabled=false", "Session\\AddTrackersFromURLEnabled=false",
        "[Network]", "PortForwardingEnabled=false", "[Core]", "AutoOpenTorrentFiles=false",
        "[GUI]", "Notifications\\Enabled=false",
        "[Preferences]", "General\\Locale=en", "Advanced\\updateCheck=false",
        "Connection\\ResolvePeerCountries=false", "Connection\\ResolvePeerHostNames=false",
        "Downloads\\AutoRemoveCompletedTorrents=false", "Search\\SearchEnabled=false",
        "WebUI\\Enabled=true", "WebUI\\Address=127.0.0.1", `WebUI\\Port=${port}`,
        "WebUI\\Username=lab", `WebUI\\Password_PBKDF2=@ByteArray(${passwordHash})`,
        "WebUI\\LocalHostAuth=true", "WebUI\\UseUPnP=false", "WebUI\\ServerDomains=127.0.0.1",
        "WebUI\\HostHeaderValidation=true", "WebUI\\CSRFProtection=true",
        ...labAppearanceSettings("functional"), "",
    ].join("\n"));
    const evidencePath = join(root, "evidence.json");
    await writeFile(specPath, JSON.stringify({ mode: "repair-readonly", evidencePath, profile, screenshots,
        torrentPath, sourceRoot, destination, existingDestination,
        expectedVerifiedBytes: payload.reduce((sum, file) => sum + file.size, 0),
        apiOrigin: `http://127.0.0.1:${port}`, apiPassword: password,
    }, null, 2));

    application = Bun.spawn(["timeout", "--kill-after=5s", "90s", "dbus-run-session", "--", "xvfb-run",
        "--auto-servernum", "--server-args=-screen 0 1440x900x24", resolve(executable), `--profile=${profile}`], {
        cwd: dirname(resolve(executable)),
        env: { ...process.env, QBUTT_QT_ACCEPTANCE_SPEC: specPath, QBUTT_QT_ACCEPTANCE_NATIVE: "1",
            QT_QPA_PLATFORM: "xcb", QT_QUICK_BACKEND: "software", QT_SCALE_FACTOR: "1",
            XDG_RUNTIME_DIR: runtime, XDG_CONFIG_HOME: join(root, "xdg-config"),
            XDG_CACHE_HOME: join(root, "xdg-cache"), XDG_DATA_HOME: join(root, "xdg-data"),
            DISPLAY: undefined, WAYLAND_DISPLAY: undefined, DBUS_SESSION_BUS_ADDRESS: undefined,
        },
        stdout: Bun.file(join(root, "stdout.log")), stderr: Bun.file(join(root, "stderr.log")),
    });
    assert.equal(await application.exited, 0, `Native read-only repair failed; inspect ${root}`);
    const evidence = JSON.parse(await readFile(evidencePath, "utf8"));
    assert.equal(evidence.status, "passed");
    for (const name of ["existing-files", "repair-readonly-cancel", "repair-readonly-dialog", "repair-readonly-api"])
        assert(evidence.checks.some((check: { name: string }) => check.name === name), `Missing ${name} evidence`);
    for (const file of payload) {
        const original = await readFile(join(sourceRoot, file.path));
        assert.equal(createHash("sha256").update(original).digest("hex"), file.sha256, "Analysis changed source payload");
        const expected = file.path.endsWith("/skip.bin") ? Buffer.concat([original, ignoredTail]) : original;
        assert.deepEqual(await readFile(join(destination, file.path)), expected, "Application teardown changed target payload");
    }
    assert.deepEqual(await readFile(join(destination, "unknown.keep")), unknown, "Application teardown changed unrelated data");
    console.log(JSON.stringify({ status: "passed", evidencePath, screenshots, checks: evidence.checks }, null, 2));
}
finally {
    if (application?.exitCode === null) {
        application.kill("SIGTERM");
        await application.exited;
    }
    for (const directory of [profile, destination, existingDestination, join(root, "seed"), runtime,
        join(root, "xdg-config"), join(root, "xdg-cache"), join(root, "xdg-data")]) {
        if (!existsSync(directory))
            continue;
        const target = await realpath(directory);
        assert(target.startsWith(root + sep), "Repair fixture cleanup escaped its temporary root");
        await rm(target, { recursive: true, force: true });
    }
    await rm(specPath, { force: true });
}
