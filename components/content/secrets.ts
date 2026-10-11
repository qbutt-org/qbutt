import { spawn } from "node:child_process";

// DPAPI binds stored source credentials to the current Windows user. Secrets
// travel only through the child's private standard streams, never command args.
async function dpapi(value: string, decrypt: boolean): Promise<string> {
    if (process.platform !== "win32")
        throw new Error("Protected source credentials require Windows.");

    const operation = decrypt ? "Unprotect" : "Protect";
    const input = decrypt
        ? "[Convert]::FromBase64String([Console]::In.ReadToEnd())"
        : "[Text.Encoding]::UTF8.GetBytes([Console]::In.ReadToEnd())";
    const result = decrypt
        ? "[Text.Encoding]::UTF8.GetString($result)"
        : "[Convert]::ToBase64String($result)";
    const script = `
        $ErrorActionPreference = 'Stop'
        [Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
        [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
        Add-Type -AssemblyName System.Security
        $bytes = ${input}
        $result = [Security.Cryptography.ProtectedData]::${operation}(
            $bytes, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
        [Console]::Out.Write(${result})
    `;

    return new Promise((resolve, reject) => {
        const child = spawn("powershell.exe", [
            "-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand",
            Buffer.from(script, "utf16le").toString("base64"),
        ], { windowsHide: true, stdio: ["pipe", "pipe", "pipe"] });
        const output: Buffer[] = [];
        const timer = setTimeout(() => child.kill(), 15_000);

        child.stdout.on("data", chunk => output.push(chunk));
        child.stderr.resume();
        child.on("error", reject);
        child.on("close", code => {
            clearTimeout(timer);

            if (code !== 0)
                reject(new Error("Windows could not unlock the saved source credentials."));
            else
                resolve(Buffer.concat(output).toString("utf8"));
        });
        child.stdin.end(value);
    });
}

export async function protectSettings(settings: Record<string, string>): Promise<string> {
    return dpapi(JSON.stringify(settings), false);
}

export async function unprotectSettings(value: string): Promise<Record<string, string>> {
    return JSON.parse(await dpapi(value, true));
}
