import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { createSocket } from "node:dgram";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { createServer, type AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { allowLabNetwork } from "../windows-firewall";
import { startProxy } from "./proxy";

const executable = resolve(process.argv[2] ?? process.env.QBUTT_SSL_UTP_EXE ?? "ssl-utp-integration.exe");
const go = process.env.QBUTT_GO ?? Bun.which("go");
const openssl = process.env.QBUTT_OPENSSL ?? Bun.which("openssl");
assert(go && openssl, "Set QBUTT_GO and QBUTT_OPENSSL, or place both tools on PATH");
await allowLabNetwork([executable, process.execPath]);
const root = await mkdtemp(join(tmpdir(), "qbutt-ssl-utp-"));
const certificates = join(root, "certificates");
const payloadRoot = join(root, "payload");
let reservation = createSocket("udp4");
let tcpReservation = createServer();
const canary = createSocket("udp4");
let nativeDatagrams = 0;
canary.on("message", () => { nativeDatagrams++; });
let proxy: Awaited<ReturnType<typeof startProxy>> | undefined;
let driver: ReturnType<typeof Bun.spawn> | undefined;

async function run(command: string[], timeoutMs: number) {
    const child = Bun.spawn(command, { stdout: "pipe", stderr: "pipe", windowsHide: true });
    const timer = setTimeout(() => child.kill(), timeoutMs);
    try {
        const [exit, stdout, stderr] = await Promise.all([
            child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
        ]);
        assert.equal(exit, 0, `Fixture tool failed: ${stderr}`);
        return stdout;
    }
    finally { clearTimeout(timer); }
}

try {
    await mkdir(certificates);
    const certificate = JSON.parse(await run([go, "run", join(import.meta.dir, "ssl-utp-certificates.go"), certificates], 60000));
    assert.match(certificate.fingerprint, /^[0-9a-f]{64}$/);
    await run([openssl, "genpkey", "-genparam", "-algorithm", "DH", "-pkeyopt", "group:ffdhe2048",
        "-out", join(certificates, "dh.pem")], 10000);
    let seedPort: number;
    for (let attempt = 0; ; attempt++) {
        try {
            await new Promise<void>((done, fail) => {
                tcpReservation.once("error", fail);
                tcpReservation.listen(0, "127.0.0.1", done);
            });
            seedPort = (tcpReservation.address() as AddressInfo).port;
            await new Promise<void>((done, fail) => {
                reservation.once("error", fail);
                reservation.bind(seedPort, "127.0.0.1", done);
            });
            break;
        }
        catch (error) {
            if (attempt === 15 || !["EACCES", "EADDRINUSE"].includes((error as NodeJS.ErrnoException).code ?? ""))
                throw error;
            if (tcpReservation.listening)
                await new Promise<void>((done, fail) => tcpReservation.close(error => error ? fail(error) : done()));
            try { await new Promise<void>(done => reservation.close(done)); }
            catch (error) { if ((error as NodeJS.ErrnoException).code !== "ERR_SOCKET_DGRAM_NOT_RUNNING") throw error; }
            tcpReservation = createServer();
            reservation = createSocket("udp4");
        }
    }
    await new Promise<void>((done, fail) => {
        canary.once("error", fail);
        canary.bind(seedPort, "127.0.0.10", done);
    });
    await new Promise<void>((done, fail) => {
        const timer = setTimeout(() => fail(new Error("Direct UDP canary did not receive its positive control")), 3000);
        canary.once("message", () => { clearTimeout(timer); done(); });
        reservation.send(Buffer.from("owned-canary-control"), seedPort, "127.0.0.10", error => {
            if (error) { clearTimeout(timer); fail(error); }
        });
    });
    nativeDatagrams = 0;
    const username = randomBytes(16).toString("hex");
    const password = randomBytes(24).toString("hex");
    proxy = await startProxy({ username, password, udp: true, targets: [
        { host: "127.0.0.10", port: seedPort, connectHost: "127.0.0.1", connectPort: seedPort },
    ] });
    await new Promise<void>(done => reservation.close(done));
    await new Promise<void>((done, fail) => tcpReservation.close(error => error ? fail(error) : done()));
    driver = Bun.spawn([executable, payloadRoot, String(seedPort), String(proxy.port), username, password, certificates],
        { stdout: "pipe", stderr: "pipe", windowsHide: true });
    const timer = setTimeout(() => driver?.kill(), 90000);
    let exit: number;
    let stdout: string;
    let stderr: string;
    try {
        [exit, stdout, stderr] = await Promise.all([
            driver.exited, new Response(driver.stdout).text(), new Response(driver.stderr).text(),
        ]);
    }
    finally { clearTimeout(timer); }
    await writeFile(join(root, "driver.stdout.log"), stdout);
    await writeFile(join(root, "driver.stderr.log"), stderr);
    assert.equal(exit, 0, `SSL-uTP driver failed: ${stderr}; proxy=${JSON.stringify(proxy.stats)}`);
    const result = JSON.parse(stdout);
    assert(result.passed && result.verifiedBytes === 512 * 1024 && result.socketType === "utp_ssl");
    assert(result.generation1VerifiedBytes > 0 && result.generation2VerifiedBytes > 0);
    assert.equal(result.generation1VerifiedBytes + result.generation2VerifiedBytes, result.verifiedBytes);
    assert.equal(result.sslUtpConnections, 2);
    assert(result.retiredGeneration === 1 && result.activeGeneration === 2 && result.staleGenerationRejected);
    const actual = await readFile(join(payloadRoot, "download", "ssl-utp.bin"));
    assert.deepEqual(actual, await readFile(join(payloadRoot, "seed", "ssl-utp.bin")));
    assert.equal(nativeDatagrams, 0, "SSL-uTP bypassed the managed SOCKS UDP route");
    assert.equal(proxy.stats.authenticatedConnections, 2, "Expected one authenticated UDP association per generation");
    assert.equal(proxy.stats.deniedConnections, 0);
    assert.equal(proxy.stats.uploadStreamBytes + proxy.stats.downloadStreamBytes, 0, "SSL-uTP used the TCP payload relay");
    assert(proxy.stats.uploadDatagramBytes > 0 && proxy.stats.downloadDatagramBytes > actual.length);
    await proxy.close();
    assert.equal(proxy.stats.activeConnections, 0);
    const evidence = { ...result, payloadSha256: createHash("sha256").update(actual).digest("hex"),
        driverSha256: createHash("sha256").update(await readFile(executable)).digest("hex"),
        certificateFingerprint: certificate.fingerprint, nativeDatagrams, proxy: proxy.stats,
        scope: "Generated CA and exact payload over controlled IPv4 SOCKS UDP; outgoing SSL-uTP only" };
    await writeFile(join(root, "evidence.json"), JSON.stringify(evidence, null, 2) + "\n");
    console.log(JSON.stringify({ passed: true, evidencePath: join(root, "evidence.json"), ...evidence }));
}
catch (error) {
    console.error(`Preserved SSL-uTP diagnostics: ${root}`);
    throw error;
}
finally {
    if (driver && driver.exitCode === null) { driver.kill(); await driver.exited; }
    await proxy?.close();
    if (tcpReservation.listening)
        await new Promise<void>((done, fail) => tcpReservation.close(error => error ? fail(error) : done()));
    for (const socket of [reservation, canary]) {
        try { await new Promise<void>(done => socket.close(done)); }
        catch (error) { if ((error as NodeJS.ErrnoException).code !== "ERR_SOCKET_DGRAM_NOT_RUNNING") throw error; }
    }
    const resolvedRoot = await realpath(root);
    for (const path of [certificates, payloadRoot]) {
        try {
            assert.equal(dirname(await realpath(path)), resolvedRoot, "Fixture cleanup escaped its temporary root");
            await rm(path, { recursive: true });
        }
        catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
    }
}
