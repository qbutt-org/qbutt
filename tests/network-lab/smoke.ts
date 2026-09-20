import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { sha256 } from "../fixtures/generate";
import { createLab, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface PathStatus {
    busy: boolean;
    pinned: boolean;
    open: boolean;
    processId: number;
    generation: number;
    mode: "pinned" | "mixed" | "tunnels";
    nodes: { name: string; type: string }[];
    paths: { pathId: string; generation: number; proxyName: string; open: boolean }[];
}

const pathsMode = process.env.QBUTT_LAB_PATHS === "1";
const lab = await createLab(pathsMode ? "paths" : "network");
let failure: unknown;
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
let proxy: Awaited<ReturnType<typeof startProxy>> | undefined;
let tracker: ReturnType<typeof Bun.serve> | undefined;
try {
    seed = await startSeed(lab.python, lab.fixtures, "v1", lab.root);
    const trackerRequests: { remoteAddress: string | undefined; infoHashPresent: boolean }[] = [];
    tracker = Bun.serve({
        hostname: "127.0.0.1", port: 0,
        fetch(request, server) {
            const url = new URL(request.url);
            assert(url.pathname === "/announce", "Unexpected tracker path");
            trackerRequests.push({
                remoteAddress: server.requestIP(request)?.address,
                infoHashPresent: url.searchParams.has("info_hash"),
            });
            return new Response("d8:intervali60e5:peers0:e", { headers: { "Content-Type": "text/plain" } });
        },
    });
    // Nothing listens on 127.0.0.2. Only this explicit proxy map reaches the seed/tracker.
    const targets = [
        { host: "127.0.0.2", port: seed.port, connectHost: seed.host, connectPort: seed.port },
        { host: "127.0.0.3", port: seed.port, connectHost: seed.host, connectPort: seed.port },
        { host: "127.0.0.2", port: tracker.port!, connectHost: "127.0.0.1", connectPort: tracker.port! },
    ];
    const credentials = { username: randomBytes(16).toString("hex"), password: randomBytes(24).toString("hex") };
    proxy = await startProxy({ ...credentials, targets });
    await lab.start();
    const configPath = join(lab.root, "controlled-node.json");
    const pathRequest = {
        configPath, proxyName: "fixture", interfaceName: "Loopback Pseudo-Interface 1",
    };
    const readPath = () => lab.json<PathStatus>("qbuttPaths/status");
    const configure = async () => {
        if (pathsMode) {
            const current = await readPath();
            if (current.paths.some(path => path.proxyName === pathRequest.proxyName && path.open))
                return;
            await lab.request("qbuttPaths/open", pathRequest);
            const opened = await waitFor("qbutt-net open", readPath, status => !status.busy);
            assert(opened.open && opened.pinned, "qbutt-net did not open the controlled path");
        }
        else {
            await lab.request("app/setPreferences", { json: JSON.stringify({
                proxy_type: "SOCKS5", proxy_ip: proxy!.host, proxy_port: proxy!.port,
                proxy_auth_enabled: true, proxy_username: credentials.username, proxy_password: credentials.password,
                proxy_hostname_lookup: true, proxy_bittorrent: true, proxy_peer_connections: true,
                proxy_rss: false, proxy_misc: false, bittorrent_protocol: 1,
            }) });
        }
    };
    if (pathsMode) {
        const childExecutable = join(dirname(process.env.QBUTT_LAB_EXE!), "qbutt-net.exe");
        await lab.checkpoint({ check: "qbutt-net-binary", sha256: sha256(await readFile(childExecutable)) });
        const unauthenticated = await fetch(`${lab.origin}/api/v2/qbuttPaths/status`, {
            headers: { Origin: lab.origin, Referer: `${lab.origin}/` }, signal: AbortSignal.timeout(5000),
        });
        assert(unauthenticated.status === 403, "Paths status admitted unauthenticated access");
        for (const method of ["open", "stop"]) {
            let rejected = false;
            try { await lab.request(`qbuttPaths/${method}`); }
            catch (error) { rejected = String(error).includes("HTTP 405"); }
            assert(rejected, `GET ${method} was not rejected`);
        }
        await writeFile(configPath, JSON.stringify({ proxies: ["fixture", "replacement"].map(name => ({
            name, type: "socks5", server: proxy.host, port: proxy.port,
            username: credentials.username, password: credentials.password, udp: false,
        })) }));
        await lab.request("qbuttPaths/list", { configPath });
        const listed = await waitFor("qbutt-net node listing", readPath, status => !status.busy);
        assert(listed.nodes.length === 2 && listed.nodes[0]!.name === "fixture", "Controlled node listing failed");
        const admissionHash = await lab.add("v1", join(lab.root, "admission"));
        await lab.request("qbuttPaths/open", pathRequest);
        const opened = await waitFor("path added to retained session", readPath, status => !status.busy);
        assert(opened.pinned && opened.open && opened.mode === "pinned"
            && opened.paths.some(path => path.proxyName === "fixture" && path.open),
            "A retained torrent did not accept the new managed route");
        const retained = await lab.json<{ hash: string }[]>(`torrents/info?hashes=${admissionHash}`);
        assert(retained.length === 1 && retained[0]!.hash === admissionHash,
            "Adding a path replaced or removed the live torrent session");
        await lab.request("torrents/delete", { hashes: admissionHash, deleteFiles: "false" });
        await waitFor("admission job removal", () => lab.json<unknown[]>("torrents/info"), torrents => torrents.length === 0);
        await lab.checkpoint({ check: "path-added-with-retained-session", generation: opened.generation, nodes: listed.nodes });
    }
    await configure();
    if (pathsMode) {
        const preferences = await lab.json<Record<string, unknown>>("app/preferences");
        assert(preferences.proxy_username === "" && preferences.proxy_password === "",
            "Runtime SOCKS credentials leaked into app/preferences");
        const statusText = JSON.stringify(await readPath());
        assert(!statusText.includes(credentials.username) && !statusText.includes(credentials.password),
            "Path status leaked adapter credentials");
        await lab.checkpoint({ check: "path-api-auth-methods-and-redaction" });
    }
    const destination = join(lab.root, "downloads");
    const hash = await lab.add("v1", destination);
    await lab.request("torrents/addTrackers", { hash, urls: `http://127.0.0.2:${tracker.port}/announce` });
    await lab.request("torrents/start", { hashes: hash });
    await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.2:${seed.port}` });
    await waitFor("private tracker through SOCKS", async () => trackerRequests.length, count => count > 0);
    await waitFor("partial proxied download", () => lab.info(hash), info => info.completed > 65536 && info.progress < 1);
    assert(proxy.stats.authenticatedConnections >= 2 && proxy.stats.downloadStreamBytes > 65536,
        "Peer/tracker proxy counters did not prove payload through SOCKS");
    assert(trackerRequests.every(item => item.remoteAddress === "127.0.0.1" && item.infoHashPresent),
        "Tracker received unexpected source or missing infohash");
    const beforeDeath = { ...proxy.stats };
    let deadGeneration = 0;
    if (pathsMode) {
        const active = await readPath();
        assert(active.open && active.processId > 0, "No owned qbutt-net child to terminate");
        deadGeneration = active.generation;
        // This PID came from the fresh authenticated lab profile, never from a
        // process-name search that could select the user's application.
        process.kill(active.processId, "SIGKILL");
        const blocked = await waitFor("qbutt-net death", readPath, status => !status.busy && !status.open && status.processId === 0);
        assert(blocked.pinned, "Child death silently enabled Native");
        await waitFor("child peer sockets drained", async () => proxy!.stats.activeConnections, count => count === 0);
    }
    else {
        await proxy.close();
        assert(proxy.stats.activeConnections === 0, "Proxy shutdown left live sockets");
    }
    await Bun.sleep(3000); // Drain data already delivered to the native disk queue.
    const drained = await lab.info(hash);
    await Bun.sleep(3000);
    const afterDeath = await lab.info(hash);
    assert(afterDeath.completed === drained.completed && afterDeath.progress < 1,
        "Client completed new payload after relay death and drain");
    await lab.checkpoint({
        check: pathsMode ? "qbutt-net-child-death-blocks-path" : "single-authenticated-socks-tcp-and-death", privateTrackerRequests: trackerRequests.length,
        trackerSources: [...new Set(trackerRequests.map(item => item.remoteAddress))],
        relayBeforeDeath: beforeDeath, completedAfterDrain: drained.completed,
        completedAfterObservation: afterDeath.completed,
        scope: "Controlled TCP peer and HTTP tracker only; no UDP, DNS, public egress or Tunnels only claim",
    });

    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("stop after relay death", () => lab.info(hash), info => info.state === "stoppedDL");
    if (!pathsMode)
        proxy = await startProxy({ ...credentials, targets });
    else
        pathRequest.proxyName = "replacement";
    await configure();
    if (pathsMode)
        assert((await readPath()).generation > deadGeneration, "qbutt-net retry reused stale generation");
    await lab.request("torrents/start", { hashes: hash });
    // A new controlled endpoint avoids the native retry cooldown of the dead
    // endpoint. The old relay has zero connections before this peer is added.
    await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.3:${seed.port}` });
    await waitFor("download after relay restart", () => lab.info(hash), info => info.progress === 1);
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("verified stop", () => lab.info(hash), info => info.state === "stoppedUP");
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    await lab.checkpoint({ check: "relay-restart-download", selectedNode: pathsMode ? pathRequest.proxyName : undefined,
        verifiedBytes, exactSizes: true, relay: { ...proxy.stats } });
    if (pathsMode) {
        await lab.request("qbuttPaths/stop", {});
        const stopped = await waitFor("path stop", readPath,
            status => !status.busy && !status.open && status.processId === 0);
        assert(stopped.pinned, "Path stop failed to retain blocked mode");
        await lab.request("qbuttPaths/native", {});
        assert(!(await readPath()).pinned, "Retained session did not return to Native");
        assert((await lab.json<{ hash: string }[]>(`torrents/info?hashes=${hash}`)).length === 1,
            "Returning to Native replaced or removed the live torrent session");
        assert(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes, "Path shutdown changed payload");
        await lab.checkpoint({ check: "path-stop-and-retained-session-native", verifiedBytes });
        await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
        await waitFor("final job removal", () => lab.json<unknown[]>("torrents/info"), torrents => torrents.length === 0);
    }
    await lab.shutdown();
    assert(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes, "Shutdown changed verified payload");
}
catch (error) {
    failure = error;
    try { await lab.shutdown(); }
    catch (shutdownError) { console.error(String(shutdownError)); }
}
finally {
    await proxy?.close();
    tracker?.stop(true);
    await seed?.stop();
}
await lab.finish(failure);
if (failure)
    throw failure;
