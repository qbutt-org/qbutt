// Read-only diagnostics for cross-origin frames in the isolated browser lab.
export async function debuggerSocket(debugUrl: string): Promise<WebSocket> {
    const response = await fetch(`${debugUrl}/json/version`, { signal: AbortSignal.timeout(1000) });
    if (!response.ok) throw new Error(`The isolated browser debugger returned HTTP ${response.status}.`);
    const version = await response.json() as { webSocketDebuggerUrl: string };
    const socket = new WebSocket(version.webSocketDebuggerUrl);
    await new Promise<void>((resolve, reject) => {
        const finish = (error?: Error) => {
            clearTimeout(timer);
            socket.removeEventListener("open", opened);
            socket.removeEventListener("error", failed);
            socket.removeEventListener("close", failed);
            if (error) { socket.close(); reject(error); }
            else resolve();
        };
        const opened = () => finish();
        const failed = () => finish(new Error("The isolated browser debugger connection failed."));
        const timer = setTimeout(failed, 1000);
        socket.addEventListener("open", opened, { once: true });
        socket.addEventListener("error", failed, { once: true });
        socket.addEventListener("close", failed, { once: true });
    });
    return socket;
}

export async function observePlayer(debugUrl: string, expectedUrl: string) {
    const socket = await debuggerSocket(debugUrl);
    let nextId = 0;
    const pending = new Map<number, { finish(value: any): void; timer: ReturnType<typeof setTimeout> }>();
    const contexts = new Map<string, Set<number>>();
    const observations = new Set<string>();
    let closed = false;
    socket.addEventListener("close", () => { closed = true; });

    function send(method: string, params: object, sessionId?: string): Promise<any> {
        if (closed) return Promise.resolve(undefined);
        return new Promise((resolve) => {
            const id = ++nextId;
            const timer = setTimeout(() => { pending.delete(id); resolve(undefined); }, 1000);
            pending.set(id, { finish: resolve, timer });
            socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
        });
    }

    socket.addEventListener("message", (event) => {
        const message = JSON.parse(String(event.data));
        if (message.id) {
            const request = pending.get(message.id);
            if (request) { clearTimeout(request.timer); pending.delete(message.id); request.finish(message.result); }
            return;
        }
        if (message.method === "Target.attachedToTarget") {
            const session = message.params.sessionId;
            void send("Runtime.enable", {}, session);
            void send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true }, session);
        }
        const session = message.sessionId ?? "";
        if (message.method === "Runtime.executionContextCreated") {
            const context = message.params.context;
            if (context.auxData?.isDefault && context.origin === new URL(expectedUrl).origin) {
                const ids = contexts.get(session) ?? new Set<number>();
                ids.add(context.id);
                contexts.set(session, ids);
            }
        }
        if (message.method === "Runtime.executionContextsCleared") contexts.delete(session);
        if (message.method === "Target.detachedFromTarget") contexts.delete(message.params.sessionId);
    });
    await send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true });
    const poll = (async () => {
        while (!closed) {
            for (const [session, ids] of contexts) for (const contextId of ids) {
                const value = await send("Runtime.evaluate", { contextId, returnByValue: true,
                    expression: `JSON.stringify(location.href===${JSON.stringify(expectedUrl)}?{
                        owned:window.name==='qbutt:player',framed:window!==window.top,visible:document.visibilityState,
                        fetchObserved:!window.fetch.toString().includes('[native code]'),
                        media:[...document.querySelectorAll('video,audio')].map(m=>({ready:m.readyState,paused:m.paused,time:m.currentTime,
                            duration:Number.isFinite(m.duration)?m.duration:null,error:m.error?.code,
                            source:m.currentSrc.startsWith('blob:')?'blob':m.currentSrc?new URL(m.currentSrc).origin:''})),
                        manifests:performance.getEntriesByType('resource').filter(r=>/\\.(mpd|m3u8)(\\?|$)/.test(r.name)).length,
                        buttons:[...document.querySelectorAll('button')].slice(0,8).map(b=>({class:b.className,text:b.innerText?.slice(0,30)}))
                    }:null)`,
                }, session);
                if (value?.result?.value && value.result.value !== "null" && observations.size < 50) observations.add(value.result.value);
            }
            await Bun.sleep(300);
        }
    })();

    return async () => {
        closed = true;
        await poll;
        socket.close();
        for (const request of pending.values()) { clearTimeout(request.timer); request.finish(undefined); }
        pending.clear();
        return [...observations].map((value) => JSON.parse(value));
    };
}
