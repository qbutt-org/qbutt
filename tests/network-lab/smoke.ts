import { randomBytes } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { createLab, requireCondition, startSeed, verifyPayload, waitFor } from "../lab";
import { startProxy } from "./proxy";

interface PathStatus {
    busy: boolean;
    pinned: boolean;
    open: boolean;
    processId: number;
    generation: number;
    nodes: { name: string; type: string }[];
}

const pathsMode = process.env.QBUTT_LAB_PATHS === "1";
const lab = await createLab(pathsMode ? "paths" : "network");
let failure: unknown;
let seed: Awaited<ReturnType<typeof startSeed>> | undefined;
let proxy: Awaited<ReturnType<typeof startProxy>> | undefined;
let tracker: ReturnType<typeof Bun.serve> | undefined;
try {
    seed = await startSeed(lab.python, lab.fixtures, "v1", lab.root);
    const trackerRequests: { remoteAddress: string | undefined; event: string | null; infoHashPresent: boolean }[] = [];
    tracker = Bun.serve({
        hostname: "127.0.0.1", port: 0,
        fetch(request, server) {
            const url = new URL(request.url);
            requireCondition(url.pathname === "/announce", "Unexpected tracker path");
            trackerRequests.push({
                remoteAddress: server.requestIP(request)?.address,
                event: url.searchParams.get("event"),
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
            await lab.request("qbuttPaths/open", pathRequest);
            const opened = await waitFor("qbutt-net open", readPath, status => !status.busy);
            requireCondition(opened.open && opened.pinned, "qbutt-net did not open the controlled path");
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
        const unauthenticated = await fetch(`${lab.origin}/api/v2/qbuttPaths/status`, {
            headers: { Origin: lab.origin, Referer: `${lab.origin}/` }, signal: AbortSignal.timeout(5000),
        });
        requireCondition(unauthenticated.status === 403, "Paths status admitted unauthenticated access");
        for (const method of ["open", "stop"]) {
            let rejected = false;
            try { await lab.request(`qbuttPaths/${method}`); }
            catch (error) { rejected = String(error).includes("HTTP 405"); }
            requireCondition(rejected, `GET ${method} was not rejected`);
        }
        await writeFile(configPath, JSON.stringify({ proxies: [{
            name: "fixture", type: "socks5", server: proxy.host, port: proxy.port,
            username: credentials.username, password: credentials.password, udp: false,
        }] }));
        await lab.request("qbuttPaths/list", { configPath });
        const listed = await waitFor("qbutt-net node listing", readPath, status => !status.busy);
        requireCondition(listed.nodes.length === 1 && listed.nodes[0]!.name === "fixture", "Controlled node listing failed");
        const admissionHash = await lab.add("v1", join(lab.root, "admission"));
        await lab.request("qbuttPaths/open", pathRequest);
        const rejected = await readPath();
        requireCondition(!rejected.pinned && !rejected.open, "Native-to-Pinned admitted with a retained torrent");
        await lab.request("torrents/delete", { hashes: admissionHash, deleteFiles: "false" });
        await waitFor("admission job removal", () => lab.json<unknown[]>("torrents/info"), torrents => torrents.length === 0);
        await lab.checkpoint({ check: "initial-path-transition-requires-empty-session", nodes: listed.nodes });
    }
    await configure();
    if (pathsMode) {
        const preferences = await lab.json<Record<string, unknown>>("app/preferences");
        requireCondition(preferences.proxy_username === "" && preferences.proxy_password === "",
            "Runtime SOCKS credentials leaked into app/preferences");
        const statusText = JSON.stringify(await readPath());
        requireCondition(!statusText.includes(credentials.username) && !statusText.includes(credentials.password),
            "Path status leaked adapter credentials");
        await lab.checkpoint({ check: "path-api-auth-methods-and-redaction" });
    }
    const destination = join(lab.root, "downloads");
    const hash = await lab.add("v1", destination);
    if (pathsMode) {
        await lab.request("qbuttPaths/native", {});
        const rejected = await readPath();
        requireCondition(rejected.pinned && rejected.open, "Pinned-to-Native admitted with a retained torrent");
        await lab.checkpoint({ check: "native-transition-requires-empty-session" });
    }
    await lab.request("torrents/addTrackers", { hash, urls: `http://127.0.0.2:${tracker.port}/announce` });
    await lab.request("torrents/start", { hashes: hash });
    await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.2:${seed.port}` });
    await waitFor("private tracker through SOCKS", async () => trackerRequests.length, count => count > 0);
    await waitFor("partial proxied download", () => lab.info(hash), info => info.completed > 65536 && info.progress < 1);
    requireCondition(proxy.stats.authenticatedConnections >= 2 && proxy.stats.downloadStreamBytes > 65536,
        "Peer/tracker proxy counters did not prove payload through SOCKS");
    requireCondition(trackerRequests.every(item => item.remoteAddress === "127.0.0.1" && item.infoHashPresent),
        "Tracker received unexpected source or missing infohash");
    const beforeDeath = { ...proxy.stats };
    let deadGeneration = 0;
    if (pathsMode) {
        const active = await readPath();
        requireCondition(active.open && active.processId > 0, "No owned qbutt-net child to terminate");
        deadGeneration = active.generation;
        // This PID came from the fresh authenticated lab profile, never from a
        // process-name search that could select the user's application.
        process.kill(active.processId, "SIGKILL");
        const blocked = await waitFor("qbutt-net death", readPath, status => !status.busy && !status.open && status.processId === 0);
        requireCondition(blocked.pinned, "Child death silently enabled Native");
        await waitFor("child peer sockets drained", async () => proxy!.stats.activeConnections, count => count === 0);
    }
    else {
        await proxy.close();
        requireCondition(proxy.stats.activeConnections === 0, "Proxy shutdown left live sockets");
    }
    await Bun.sleep(3000); // Drain data already delivered to the native disk queue.
    const drained = await lab.info(hash);
    await Bun.sleep(3000);
    const afterDeath = await lab.info(hash);
    requireCondition(afterDeath.completed === drained.completed && afterDeath.progress < 1,
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
    await configure();
    if (pathsMode)
        requireCondition((await readPath()).generation > deadGeneration, "qbutt-net retry reused stale generation");
    await lab.request("torrents/start", { hashes: hash });
    // A new controlled endpoint avoids the native retry cooldown of the dead
    // endpoint. The old relay has zero connections before this peer is added.
    await lab.request("torrents/addPeers", { hashes: hash, peers: `127.0.0.3:${seed.port}` });
    await waitFor("download after relay restart", () => lab.info(hash), info => info.progress === 1);
    await lab.request("torrents/stop", { hashes: hash });
    await waitFor("verified stop", () => lab.info(hash), info => info.state === "stoppedUP");
    const verifiedBytes = await verifyPayload(destination, lab.manifest.payload);
    await lab.checkpoint({ check: "relay-restart-download", verifiedBytes, exactSizes: true, relay: { ...proxy.stats } });
    if (pathsMode) {
        await lab.request("qbuttPaths/stop", {});
        const stopped = await readPath();
        requireCondition(stopped.pinned && !stopped.open && stopped.processId === 0, "Path stop failed to retain blocked mode");
        await lab.request("torrents/delete", { hashes: hash, deleteFiles: "false" });
        await waitFor("final job removal", () => lab.json<unknown[]>("torrents/info"), torrents => torrents.length === 0);
        await lab.request("qbuttPaths/native", {});
        requireCondition(!(await readPath()).pinned, "Empty session did not return to Native");
        requireCondition(await verifyPayload(destination, lab.manifest.payload) === verifiedBytes, "Path shutdown removed payload");
        await lab.checkpoint({ check: "path-stop-and-empty-session-native", verifiedBytes });
        const hiddenHash = lab.manifest.torrents.find(torrent => torrent.name === "v1-64k")!.infoHashV1!;
        const metadata = await lab.request("torrents/fetchMetadata", { source: `magnet:?xt=urn:btih:${hiddenHash}` });
        requireCondition(metadata.status === 202, "Hidden metadata fixture was not queued");
        requireCondition((await lab.json<unknown[]>("torrents/info")).length === 0, "Metadata job unexpectedly became a visible torrent");
        await lab.request("qbuttPaths/open", pathRequest);
        requireCondition(!(await readPath()).pinned, "Path transition ignored a hidden metadata job");
        await lab.checkpoint({ check: "hidden-metadata-blocks-native-to-pinned" });
    }
    await lab.shutdown();
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
